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

//! Stable physical-fill identities and verified aliases for pre-normalization caches.
use ahash::{AHashMap, AHashSet};
use anyhow::{Result, ensure};
use nautilus_core::UnixNanos;
use nautilus_model::{
    events::OrderFilled,
    identifiers::{TradeId, VenueOrderId},
    reports::FillReport,
};
use rust_decimal::Decimal;

const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const PRIME: u64 = 0x0100_0000_01b3;
// Multiplicative inverse of PRIME modulo 2^64, for validating old display-scale fingerprints.
const INVERSE: u64 = 0xce96_5057_aff6_957b;

fn extend(mut h: u64, bytes: &[u8]) -> u64 {
    for &byte in bytes {
        h = (h ^ u64::from(byte)).wrapping_mul(PRIME);
    }
    h
}
fn remove(mut h: u64, bytes: &[u8]) -> u64 {
    for &byte in bytes.iter().rev() {
        h = h.wrapping_mul(INVERSE) ^ u64::from(byte);
    }
    h
}

pub(crate) fn venue_millis(time: UnixNanos) -> u64 {
    let nanos = time.as_u64();
    nanos / 1_000_000 + u64::from(nanos % 1_000_000 >= 500_000)
}

pub(crate) fn is_venue_id(id: TradeId, oid: u64) -> bool {
    let text = id.as_str();
    let fingerprint = text.strip_prefix("HL-").unwrap_or(text);
    fingerprint.len() == 33
        && fingerprint.as_bytes()[..16]
            .iter()
            .all(u8::is_ascii_hexdigit)
        && fingerprint.ends_with(&format!("-{oid:016x}"))
}

pub(crate) struct FillIdentity<'a> {
    pub(crate) hash: &'a str,
    pub(crate) oid: u64,
    pub(crate) px: Decimal,
    pub(crate) sz: Decimal,
    pub(crate) time: u64,
    pub(crate) start: Decimal,
}
impl FillIdentity<'_> {
    pub(crate) fn trade_id(&self) -> TradeId {
        let mut h = extend(OFFSET, self.hash.as_bytes());
        h = extend(h, &self.oid.to_le_bytes());
        // Separate variable-width numeric fields; equivalent scales share an identity.
        for value in [self.px, self.sz] {
            h = extend(h, value.normalize().to_string().as_bytes());
            h = extend(h, &[0]);
        }
        h = extend(h, &self.time.to_le_bytes());
        h = extend(h, self.start.normalize().to_string().as_bytes());
        TradeId::new(format!("HL-{h:016x}-{:016x}", self.oid))
    }

    fn legacy_states(&self) -> AHashSet<u64> {
        let prefix = extend(
            extend(OFFSET, self.hash.as_bytes()),
            &self.oid.to_le_bytes(),
        );
        let mut states = AHashSet::new();
        let sizes = encodings(self.sz);
        for price in encodings(self.px) {
            let after_price = extend(prefix, price.as_bytes());
            for size in &sizes {
                states.insert(extend(after_price, size.as_bytes()));
            }
        }
        states
    }

    fn matches_legacy(&self, id: TradeId, states: &AHashSet<u64>) -> bool {
        let Ok(target) = u64::from_str_radix(&id.as_str()[..16], 16) else {
            return false;
        };
        encodings(self.start).iter().any(|start| {
            let after_time = remove(target, start.as_bytes());
            states.contains(&remove(after_time, &self.time.to_le_bytes()))
        })
    }
}

fn encodings(value: Decimal) -> Vec<String> {
    let normalized = value.normalize();
    let mut rows = Vec::new();

    for scale in normalized.scale()..=28 {
        let mut candidate = normalized;
        candidate.rescale(scale);
        // Large mantissas cannot represent every scale; never round to create an alias.
        if candidate.scale() != scale || candidate != value {
            continue;
        }
        let text = candidate.to_string();
        if value.is_zero() {
            rows.push(format!("-{text}"));
        }
        rows.push(text);
    }
    rows
}

/// Only retained, adapter-generated execution proofs can supply legacy aliases.
#[derive(Debug, Default)]
pub(crate) struct FillAliases {
    retained: Vec<OrderFilled>,
    by_order_time: AHashMap<(VenueOrderId, u64), Vec<usize>>,
    by_id: AHashMap<TradeId, usize>,
    resolved: AHashMap<TradeId, usize>,
    failure: Option<String>,
}
impl FillAliases {
    pub(crate) fn from_retained(retained: Vec<OrderFilled>) -> Self {
        let mut result = Self::default();
        for (index, fill) in retained.iter().enumerate() {
            result
                .by_order_time
                .entry((fill.venue_order_id, venue_millis(fill.ts_event)))
                .or_default()
                .push(index);
            result.by_id.insert(fill.trade_id, index);
        }
        result.retained = retained;
        result
    }

    pub(crate) fn check(&self) -> Result<()> {
        ensure!(
            self.failure.is_none(),
            "Fill identity reconciliation failed: {}",
            self.failure.as_deref().unwrap_or_default()
        );
        Ok(())
    }

    /// Returns true for an already retained execution, before WS dispatch updates quantities.
    pub(crate) fn resolve(
        &mut self,
        identity: &FillIdentity<'_>,
        report: &mut FillReport,
    ) -> Result<bool> {
        self.check()?;
        let result = self.resolve_inner(identity, report);
        if let Err(error) = &result {
            self.failure = Some(error.to_string());
        }
        result
    }

    fn resolve_inner(
        &mut self,
        identity: &FillIdentity<'_>,
        report: &mut FillReport,
    ) -> Result<bool> {
        let canonical = identity.trade_id();
        let mut matches = Vec::new();

        if let Some(index) = self
            .resolved
            .get(&canonical)
            .or_else(|| self.by_id.get(&canonical))
        {
            matches.push(*index);
        }

        if let Some(candidates) = self
            .by_order_time
            .get(&(report.venue_order_id, identity.time))
        {
            let legacy: Vec<_> = candidates
                .iter()
                .copied()
                .filter(|i| !self.retained[*i].trade_id.as_str().starts_with("HL-"))
                .collect();

            if !legacy.is_empty() {
                let states = identity.legacy_states();
                for index in legacy {
                    if identity.matches_legacy(self.retained[index].trade_id, &states)
                        && !matches.contains(&index)
                    {
                        matches.push(index);
                    }
                }
            }
        }
        ensure!(
            matches.len() <= 1,
            "Ambiguous retained fill identity for {canonical}"
        );
        let Some(index) = matches.first().copied() else {
            report.trade_id = canonical;
            return Ok(false);
        };
        let fill = &self.retained[index];
        ensure!(
            fill.account_id == report.account_id
                && fill.instrument_id == report.instrument_id
                && fill.venue_order_id == report.venue_order_id
                && fill.order_side == report.order_side
                && fill.last_px == report.last_px
                && fill.last_qty == report.last_qty
                && fill.commission == Some(report.commission)
                && fill.liquidity_side == report.liquidity_side
                && venue_millis(fill.ts_event) == venue_millis(report.ts_event),
            "Conflicting retained execution for {}",
            fill.trade_id
        );
        report.trade_id = fill.trade_id;
        report.client_order_id = Some(fill.client_order_id);
        self.resolved.insert(canonical, index);
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nautilus_core::UUID4;
    use nautilus_model::{
        enums::{LiquiditySide, OrderSide, OrderType},
        identifiers::{AccountId, ClientOrderId, InstrumentId, StrategyId, TraderId},
        types::{Currency, Money, Price, Quantity},
    };
    use rstest::rstest;
    use rust_decimal_macros::dec;

    fn proof(id: &str) -> OrderFilled {
        let time = UnixNanos::from(1704470400000000000u64);
        OrderFilled::new(
            TraderId::from("TESTER-001"),
            StrategyId::from("ENTROPY-001"),
            InstrumentId::from("io:SNDK-USD-PERP.HYPERLIQUID"),
            ClientOrderId::from("ENTROPY-OPEN"),
            VenueOrderId::from("12345"),
            AccountId::from("HYPERLIQUID-001"),
            TradeId::from(id),
            OrderSide::Buy,
            OrderType::Limit,
            Quantity::from("0.10"),
            Price::from("50000.00"),
            Currency::USD(),
            LiquiditySide::Taker,
            UUID4::new(),
            time,
            time,
            false,
            None,
            Some(Money::from("0.10 USDC")),
            None,
        )
    }
    fn row(fill: &OrderFilled) -> FillReport {
        FillReport::new(
            fill.account_id,
            fill.instrument_id,
            fill.venue_order_id,
            fill.trade_id,
            fill.order_side,
            fill.last_qty,
            fill.last_px,
            fill.commission.unwrap(),
            fill.liquidity_side,
            None,
            None,
            fill.ts_event,
            fill.ts_init,
            None,
        )
    }
    fn identity() -> FillIdentity<'static> {
        FillIdentity {
            hash: "0xabc123",
            oid: 12345,
            px: dec!(50000.000),
            sz: dec!(0.10000),
            time: 1704470400000,
            start: dec!(0),
        }
    }

    #[rstest]
    fn legacy_proof_matches_independent_decimal_encodings_but_not_another_partial() {
        let fill = proof("a846ae6f557868e9-0000000000003039");
        let mut aliases = FillAliases::from_retained(vec![fill.clone()]);
        let mut report = row(&fill);
        report.trade_id = identity().trade_id();
        assert!(aliases.resolve(&identity(), &mut report).unwrap());
        assert_eq!(report.trade_id, fill.trade_id);
        assert_eq!(report.client_order_id, Some(fill.client_order_id));
        let other = FillIdentity {
            start: dec!(1),
            ..identity()
        };
        assert!(!aliases.resolve(&other, &mut report).unwrap());
        assert_ne!(report.trade_id, fill.trade_id);
    }

    #[rstest]
    fn conflicting_commission_latches_failure_instead_of_reusing_a_known_trade() {
        let fill = proof("a846ae6f557868e9-0000000000003039");
        let mut aliases = FillAliases::from_retained(vec![fill.clone()]);
        let mut report = row(&fill);
        report.commission = Money::from("0.20 USDC");
        assert!(
            aliases
                .resolve(&identity(), &mut report)
                .unwrap_err()
                .to_string()
                .contains("Conflicting retained execution")
        );
        assert!(aliases.check().is_err());
        assert!(aliases.resolve(&identity(), &mut row(&fill)).is_err());
    }

    #[rstest]
    fn two_legacy_ids_for_one_physical_execution_are_ambiguous() {
        // Same numbers, formerly encoded as 50000.0/0.1/0.0 and 50000/0.10/0.
        let first = proof("a846ae6f557868e9-0000000000003039");
        let second = proof("b1aff2171b2a1f29-0000000000003039");
        let mut aliases = FillAliases::from_retained(vec![first.clone(), second]);
        assert!(
            aliases
                .resolve(&identity(), &mut row(&first))
                .unwrap_err()
                .to_string()
                .contains("Ambiguous retained fill")
        );
        assert!(aliases.check().is_err());
    }
}
