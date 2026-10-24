// -------------------------------------------------------------------------------------------------
//  Copyright (C) 2015-2026 Nautech Systems Pty Ltd. All rights reserved.
//  https://nautechsystems.io
//
//  Licensed under the GNU Lesser General Public License Version 3.0 (the "License");
//  You may not use this file except in compliance with the License.
//  You may obtain a copy of the License at https://www.gnu.org/licenses/lgpl-3.0.en.html
//
//  Unless required by applicable law or agreed to in writing, software
//  distributed under the License is distributed on an "AS IS" BASIS,
//  WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
//  See the License for the specific language governing permissions and
//  limitations under the License.
// -------------------------------------------------------------------------------------------------

//! Verified local fills used to recover order projections without inferring executions.
use ahash::{AHashMap, AHashSet};
use anyhow::{Result, ensure};
use nautilus_common::cache::Cache;
use nautilus_model::{
    events::{OrderEventAny, OrderFilled},
    identifiers::{AccountId, ClientId, InstrumentId, TradeId, Venue, VenueOrderId},
    orders::Order,
    reports::{FillReport, OrderStatusReport},
};

type FillKey = (InstrumentId, VenueOrderId, TradeId);

// Legacy typed fills converted millisecond venue time through floating-point
// nanoseconds. Compare their timestamps at the venue's actual resolution.
fn venue_millis(time: nautilus_core::UnixNanos) -> u64 {
    let nanos = time.as_u64();
    nanos / 1_000_000 + u64::from(nanos % 1_000_000 >= 500_000)
}

fn same_execution(a: &FillReport, b: &FillReport) -> bool {
    a.account_id == b.account_id
        && a.instrument_id == b.instrument_id
        && a.venue_order_id == b.venue_order_id
        && a.trade_id == b.trade_id
        && a.order_side == b.order_side
        && a.last_qty == b.last_qty
        && a.last_px == b.last_px
        && venue_millis(a.ts_event) == venue_millis(b.ts_event)
        && a.commission == b.commission
        && a.liquidity_side == b.liquidity_side
        && (a.client_order_id.is_none()
            || b.client_order_id.is_none()
            || a.client_order_id == b.client_order_id)
}

fn push_unique(
    rows: &mut Vec<FillReport>,
    index: &mut AHashMap<FillKey, usize>,
    row: FillReport,
) -> Result<()> {
    let key = (row.instrument_id, row.venue_order_id, row.trade_id);
    if let Some(i) = index.get(&key).copied() {
        ensure!(
            same_execution(&rows[i], &row),
            "Conflicting retained fill proof for {}",
            row.trade_id
        );
        if rows[i].client_order_id.is_none() {
            rows[i].client_order_id = row.client_order_id;
        }
    } else {
        index.insert(key, rows.len());
        rows.push(row);
    }
    Ok(())
}

pub(crate) fn retained_fills(
    cache: &Cache,
    client: ClientId,
    account: AccountId,
    venue: Venue,
) -> Result<Vec<OrderFilled>> {
    let mut candidates = Vec::new();
    for position in cache.positions(Some(&venue), None, None, Some(&account), None) {
        candidates.extend(position.events.iter().cloned());
    }
    for order in cache.orders(Some(&venue), None, None, None, None) {
        for event in order.events() {
            if let OrderEventAny::Filled(fill) = event {
                candidates.push(fill.clone());
            }
        }
    }
    let mut result = Vec::new();
    let mut index = AHashMap::new();
    let mut reports = Vec::new();
    for fill in candidates {
        if fill.account_id != account
            || fill.instrument_id.venue != venue
            || cache
                .client_id(&fill.client_order_id)
                .is_some_and(|id| *id != client)
        {
            continue;
        }
        // Inferred reconciliation IDs are not execution evidence.
        let Ok(oid) = fill.venue_order_id.as_str().parse::<u64>() else {
            continue;
        };
        let id = fill.trade_id.as_str();
        if id.len() != 33
            || !id.as_bytes()[..16].iter().all(|b| b.is_ascii_hexdigit())
            || !id.ends_with(&format!("-{oid:016x}"))
        {
            continue;
        }
        let Some(commission) = fill.commission else {
            continue;
        };
        let report = FillReport::new(
            fill.account_id,
            fill.instrument_id,
            fill.venue_order_id,
            fill.trade_id,
            fill.order_side,
            fill.last_qty,
            fill.last_px,
            commission,
            fill.liquidity_side,
            Some(fill.client_order_id),
            None,
            nautilus_core::UnixNanos::from(
                venue_millis(fill.ts_event)
                    .checked_mul(1_000_000)
                    .ok_or_else(|| anyhow::anyhow!("Retained fill timestamp overflow"))?,
            ),
            fill.ts_init,
            None,
        );
        let before = reports.len();
        push_unique(&mut reports, &mut index, report)?;
        if reports.len() > before {
            result.push(fill);
        }
    }
    Ok(result)
}

pub(crate) fn merge_retained_fills(
    fills: &mut Vec<FillReport>,
    retained: &[OrderFilled],
    seen: &AHashSet<VenueOrderId>,
) -> Result<()> {
    let mut rows = Vec::new();
    let mut index = AHashMap::new();
    for row in fills.drain(..) {
        push_unique(&mut rows, &mut index, row)?;
    }
    for fill in retained
        .iter()
        .filter(|fill| seen.contains(&fill.venue_order_id))
    {
        let commission = fill
            .commission
            .expect("verified retained fill has a commission");
        push_unique(
            &mut rows,
            &mut index,
            FillReport::new(
                fill.account_id,
                fill.instrument_id,
                fill.venue_order_id,
                fill.trade_id,
                fill.order_side,
                fill.last_qty,
                fill.last_px,
                commission,
                fill.liquidity_side,
                Some(fill.client_order_id),
                None,
                nautilus_core::UnixNanos::from(
                    venue_millis(fill.ts_event)
                        .checked_mul(1_000_000)
                        .ok_or_else(|| anyhow::anyhow!("Retained fill timestamp overflow"))?,
                ),
                fill.ts_init,
                None,
            ),
        )?;
    }
    *fills = rows;
    Ok(())
}

pub(crate) fn ensure_fill_coverage(
    orders: &[OrderStatusReport],
    fills: &[FillReport],
) -> Result<()> {
    for order in orders {
        let total: rust_decimal::Decimal = fills
            .iter()
            .filter(|fill| {
                fill.account_id == order.account_id
                    && fill.instrument_id == order.instrument_id
                    && fill.venue_order_id == order.venue_order_id
            })
            .map(|fill| fill.last_qty.as_decimal())
            .sum();
        ensure!(
            total >= order.filled_qty.as_decimal(),
            "Incomplete fill coverage for {}: proved {}, reported {}",
            order.venue_order_id,
            total,
            order.filled_qty
        );
    }
    Ok(())
}
