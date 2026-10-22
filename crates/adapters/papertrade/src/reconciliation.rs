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

//! Synchronizes source state before publishing reports; failed reads never imply flatness.

use std::{
    collections::{HashMap, HashSet},
    sync::atomic::Ordering,
};

use alloy_primitives::B256;
use anyhow::{Result, ensure};
use nautilus_model::{
    enums::{OrderStatus, OrderType, TimeInForce},
    identifiers::VenueOrderId,
    reports::OrderStatusReport,
    types::Quantity,
};

use crate::{
    chain,
    execution::{Inner, ReportBundle},
    reports,
    wire::AccountSnapshot,
};

pub(crate) async fn synchronize(inner: &Inner, snapshot: AccountSnapshot) -> Result<()> {
    let _serial = inner.sync.lock().await;
    snapshot.validate(inner.wallet)?;
    let previous = inner.snapshot.read().clone();
    if let Some(previous) = &previous {
        ensure!(
            snapshot.wallet_revision >= previous.wallet_revision
                && snapshot.queue_revision >= previous.queue_revision
                && snapshot.indexed_through_block >= previous.indexed_through_block
                && snapshot.indexed_through_timestamp >= previous.indexed_through_timestamp,
            "Account coverage regressed across source connections"
        );
    }
    let trading = inner.api.trading().await?;
    let unsettled = inner
        .journal
        .entries()?
        .iter()
        .any(|e| matches!(e.status, OrderStatus::Submitted | OrderStatus::Accepted));
    // A complete history stays valid while the wallet revision is unchanged;
    // queued or ambiguous intents still require a fresh recovery pass.
    let fresh_history = if inner.bundle.read().is_none()
        || previous
            .as_ref()
            .is_none_or(|old| old.wallet_revision != snapshot.wallet_revision)
        || unsettled
    {
        inner.api.history(inner.wallet).await?
    } else {
        Vec::new()
    };
    ensure!(
        !fresh_history.iter().any(|r| snapshot
            .open_positions
            .iter()
            .any(|p| p.position_id == r.position_id)),
        "Close history is ahead of the account snapshot"
    );
    inner
        .journal
        .remember(&snapshot.open_positions, &fresh_history)?;
    let history = inner.journal.closes()?;
    let observed = inner.journal.observed()?;
    ensure!(
        observed.iter().all(|old| snapshot
            .open_positions
            .iter()
            .any(|p| p.position_id == old.position_id)
            || history.iter().any(|r| r.position_id == old.position_id)),
        "Position disappeared without a terminal history record"
    );
    let markets: HashMap<_, _> = trading.instruments.iter().map(|m| (m.id, m)).collect();
    let mut entries = inner.journal.entries()?;
    let mut bundle = ReportBundle {
        complete: true,
        ..Default::default()
    };
    if snapshot.operations.iter().any(|o| {
        matches!(o.action.as_str(), "openPosition" | "closePositions")
            && !entries.iter().any(|e| e.intent_id == o.intent_id)
    }) {
        bundle.complete = false;
    }
    let now = inner.clock.get_time_ns();
    let account_id = inner.config.account_id;
    let mut by_position = HashMap::new();
    for p in &observed {
        by_position.insert(p.position_id, p.clone());
    }
    for close in &history {
        by_position
            .entry(close.position_id)
            .or_insert_with(|| reports::historical_position(close));
    }
    let pending: Vec<_> = entries
        .iter()
        .filter(|e| {
            e.fills.is_empty()
                && matches!(e.status, OrderStatus::Submitted | OrderStatus::Accepted)
                && !snapshot
                    .operations
                    .iter()
                    .any(|o| o.intent_id == e.intent_id)
                && !snapshot
                    .open_positions
                    .iter()
                    .any(|p| p.intent_id == Some(e.intent_id))
        })
        .collect();
    let start = pending
        .iter()
        .map(|e| e.scanned_through.saturating_add(1))
        .min();
    let pending_ids: Vec<_> = pending.iter().map(|e| e.intent_id).collect();
    let mut receipts: HashMap<B256, serde_json::Value> = HashMap::new();
    if let Some(start) = start {
        for receipt in inner
            .api
            .intent_receipts(&pending_ids, start, trading.indexed_through_block)
            .await?
        {
            let hash: B256 = serde_json::from_value(receipt["transactionHash"].clone())?;
            receipts.insert(hash, receipt);
        }
    }
    for entry in &mut entries {
        if !pending_ids.contains(&entry.intent_id) {
            continue;
        }
        if !entry.fills.is_empty()
            || !matches!(entry.status, OrderStatus::Submitted | OrderStatus::Accepted)
        {
            continue;
        }
        for receipt in receipts.values() {
            match chain::intent_result(receipt, entry.intent_id)? {
                Some(true) if entry.action == "openPosition" => {
                    let opened =
                        chain::recovered_open(receipt, entry.intent_id, inner.wallet, &entry.body)?;
                    inner.journal.remember(std::slice::from_ref(&opened), &[])?;
                    by_position.insert(opened.position_id, opened);
                }
                // A failed chain attempt does not revoke the signed intent; a
                // later relayer attempt can still execute it before its deadline.
                _ => {}
            }
        }
        entry.scanned_through = entry.scanned_through.max(trading.indexed_through_block);
        inner.journal.update(entry)?;
    }
    let mut open_clients = HashMap::new();
    let mut close_clients = HashMap::new();
    for entry in &mut entries {
        if let Some((_, outcome)) = snapshot
            .intent_outcomes
            .iter()
            .find(|(id, _)| *id == entry.intent_id)
        {
            let status = match outcome.as_str() {
                "failed" => Some(OrderStatus::Rejected),
                "expired" => Some(OrderStatus::Expired),
                "cancelled" => Some(OrderStatus::Canceled),
                "succeeded" | "unknown" => None,
                _ => anyhow::bail!("Unknown intent outcome"),
            };
            if let Some(status) = status {
                ensure!(
                    entry.fills.is_empty(),
                    "Intent outcome contradicts a known fill"
                );
                entry.status = status;
                inner.journal.update(entry)?;
            }
        }
        if entry.action == "openPosition" {
            let found = by_position
                .values()
                .find(|p| p.intent_id == Some(entry.intent_id))
                .or_else(|| {
                    entry.fills.first().and_then(|fill| {
                        by_position
                            .values()
                            .find(|p| Some(reports::position_id(p)) == fill.venue_position_id)
                    })
                });
            if let Some(position) = found {
                let market = markets
                    .get(&position.instrument_id)
                    .ok_or_else(|| anyhow::anyhow!("Missing market for source position"))?;
                let fill = reports::open_fill(
                    position,
                    market,
                    account_id,
                    Some(entry.command.client_order_id),
                    now,
                )?;
                ensure!(
                    !matches!(
                        entry.status,
                        OrderStatus::Rejected | OrderStatus::Canceled | OrderStatus::Expired
                    ),
                    "Venue position contradicts terminal intent status"
                );
                entry.status = OrderStatus::Filled;
                entry.fills = vec![fill];
                inner.journal.update(entry)?;
                open_clients.insert(position.position_id, entry.clone());
            }
        } else if let Some(target) = entry.target
            && let Some(record) = history.iter().find(|r| r.position_id == target)
        {
            let proved = if entry.fills.is_empty() {
                if let std::collections::hash_map::Entry::Vacant(slot) =
                    receipts.entry(record.tx_hash)
                {
                    match inner.api.receipt(record.tx_hash).await {
                        Ok(receipt) => {
                            slot.insert(receipt);
                        }
                        Err(e) => {
                            log::warn!(
                                "papertrade_close_receipt_unavailable intent_id={} error={e}",
                                entry.intent_id
                            );
                        }
                    }
                }
                if let Some(receipt) = receipts.get(&record.tx_hash) {
                    match chain::intent_result(receipt, entry.intent_id)? {
                        Some(true) => {
                            chain::verify_close(receipt, record, inner.wallet)?;
                            true
                        }
                        Some(false) | None => false,
                    }
                } else {
                    false
                }
            } else {
                true
            };
            if proved {
                let market = markets
                    .get(&record.instrument_id)
                    .ok_or_else(|| anyhow::anyhow!("Missing closed market"))?;
                let fill = reports::close_fill(
                    record,
                    market,
                    account_id,
                    Some(entry.command.client_order_id),
                    Some(VenueOrderId::from(format!("{:#x}", entry.intent_id))),
                    now,
                )?;
                ensure!(
                    !matches!(
                        entry.status,
                        OrderStatus::Rejected | OrderStatus::Canceled | OrderStatus::Expired
                    ),
                    "Confirmed close contradicts terminal intent"
                );
                entry.status = OrderStatus::Filled;
                entry.fills = vec![fill];
                inner.journal.update(entry)?;
                close_clients.insert(record.position_id, entry.clone());
            }
        }
        if entry.fills.is_empty() {
            let remote = snapshot
                .operations
                .iter()
                .any(|o| o.intent_id == entry.intent_id);
            if remote && entry.status == OrderStatus::Submitted {
                entry.status = OrderStatus::Accepted;
                inner.journal.update(entry)?;
            }
            if matches!(entry.status, OrderStatus::Submitted | OrderStatus::Accepted) && !remote {
                bundle.complete = false;
            }
            let ts = nautilus_core::UnixNanos::from(entry.command.ts_init.as_u64());
            let mut report = OrderStatusReport::new(
                account_id,
                entry.command.instrument_id,
                Some(entry.command.client_order_id),
                VenueOrderId::from(format!("{:#x}", entry.intent_id)),
                Some(entry.command.order_init.order_side),
                OrderType::Market,
                TimeInForce::Gtc,
                entry.status,
                entry.command.order_init.quantity,
                Quantity::zero(entry.command.order_init.quantity.precision),
                ts,
                now,
                now,
                None,
            );
            report.reduce_only = entry.command.order_init.reduce_only;
            bundle.orders.push(report);
        }
    }
    let mut fill_ids = HashSet::new();
    for position in by_position.values() {
        let market = markets
            .get(&position.instrument_id)
            .ok_or_else(|| anyhow::anyhow!("Missing market for historical position"))?;
        let fill = if let Some(entry) = open_clients.get(&position.position_id) {
            entry.fills[0].clone()
        } else {
            reports::open_fill(position, market, account_id, None, now)?
        };
        if fill_ids.insert(fill.trade_id) {
            bundle.orders.push(reports::order_report(&fill, now));
            bundle.fills.push(fill);
        }
    }
    for record in &history {
        let market = markets
            .get(&record.instrument_id)
            .ok_or_else(|| anyhow::anyhow!("Missing market for close history"))?;
        let fill = if let Some(entry) = close_clients.get(&record.position_id) {
            entry.fills[0].clone()
        } else {
            reports::close_fill(record, market, account_id, None, None, now)?
        };
        if fill_ids.insert(fill.trade_id) {
            bundle.orders.push(reports::order_report(&fill, now));
            bundle.fills.push(fill);
        }
    }
    for p in &snapshot.open_positions {
        let market = markets
            .get(&p.instrument_id)
            .ok_or_else(|| anyhow::anyhow!("Unknown open market"))?;
        bundle
            .positions
            .push(reports::position_report(p, market, account_id, now)?);
    }
    bundle.fills.sort_by_key(|f| (f.ts_event, f.trade_id));
    let mut info = nautilus_core::Params::default();
    info.insert(
        "papertrade_queued_raw".into(),
        serde_json::json!(snapshot.queued.to_string()),
    );
    info.insert(
        "papertrade_account_revision".into(),
        serde_json::json!(snapshot.account_revision),
    );
    let balances = reports::balances(&snapshot)?;
    inner.journal.checkpoint(&snapshot)?;
    inner.emitter.try_emit_account_state(
        balances,
        Vec::new(),
        true,
        nautilus_core::UnixNanos::from(snapshot.indexed_through_timestamp * 1_000_000_000),
        Some(info),
    )?;
    for report in &bundle.orders {
        let fills: Vec<_> = bundle
            .fills
            .iter()
            .filter(|f| f.venue_order_id == report.venue_order_id)
            .cloned()
            .collect();
        let economics: Vec<_> = fills
            .iter()
            .map(|f| (f.trade_id, f.last_qty, f.last_px, f.commission))
            .collect();
        let fingerprint = serde_json::to_string(&(
            report.order_status,
            report.quantity,
            report.filled_qty,
            report.ts_last,
            economics,
        ))?;
        if inner.sent_orders.lock().get(&report.venue_order_id) == Some(&fingerprint) {
            continue;
        }
        if let Some(entry) = entries.iter().find(|e| {
            Some(e.command.client_order_id) == report.client_order_id
                && e.command.order_init.quote_quantity
                && !e.fills.is_empty()
        }) && inner
            .normalized
            .lock()
            .insert(entry.command.client_order_id)
        {
            let order =
                nautilus_model::orders::OrderAny::try_from(entry.command.order_init.clone())?;
            inner.emitter.emit_order_updated(
                &order,
                report.venue_order_id,
                report.quantity,
                None,
                None,
                None,
                report.ts_last,
            );
        }
        inner.emitter.send_order_with_fills(report.clone(), fills);
        inner
            .sent_orders
            .lock()
            .insert(report.venue_order_id, fingerprint);
    }
    for position in &bundle.positions {
        inner.emitter.send_position_report(position.clone());
    }
    *inner.snapshot.write() = Some(snapshot);
    *inner.trading.write() = Some(trading);
    *inner.bundle.write() = Some(bundle);
    inner.live.store(true, Ordering::Release);
    Ok(())
}
