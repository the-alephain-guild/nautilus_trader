//! Conversion from the venue's account reads into Nautilus reconciliation reports.
//!
//! Reconciliation is what lets a strategy run unattended: it is how the engine learns about an
//! order it did not place, a fill it missed while disconnected, and what the account actually
//! holds. Nautilus asks for that as reports, and this module turns the venue's answers into them.
//!
//! # An order's state is split across two endpoints
//!
//! `/orders` lists only what is still open; anything terminal has moved to `/orders/history`.
//! Reconciling therefore means reading both, and a report built from only the first would show a
//! cancelled order as simply absent — which the engine cannot distinguish from an order it should
//! never have known about.
//!
//! # Average fill price is derived, not reported
//!
//! The venue gives `executedQty` and `executedValue` (the quote-asset total), not an average
//! price. Dividing is the only way to get one, and it is only defined once something has filled —
//! so an unfilled order reports no average rather than a zero, which would read as "filled at
//! zero".

use nautilus_core::UnixNanos;
use nautilus_model::{
    enums::{OrderStatus as NautilusStatus, TimeInForce as NautilusTif},
    identifiers::{AccountId, ClientOrderId, InstrumentId, VenueOrderId},
    reports::order::OrderStatusReport,
    types::{Price, Quantity},
};
use rust_decimal::Decimal;
use std::str::FromStr;

use crate::{
    common::{
        decimal::normalize_to,
        enums::{OrderSide, OrderStatus, OrderType, TimeInForce},
    },
    http::account_reads::OrderRecord,
};

/// Why a venue order record cannot become a report.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ReportError {
    #[error("order {order_id} reports status {status:?}, which has no Nautilus equivalent")]
    UnmappableStatus { order_id: u64, status: OrderStatus },
    #[error("order {order_id} has an unparseable {field}: {value:?} ({reason})")]
    InvalidValue {
        order_id: u64,
        field: &'static str,
        value: String,
        reason: String,
    },
}

/// Maps the venue's side onto Nautilus's.
#[must_use]
pub const fn map_side(side: OrderSide) -> nautilus_model::enums::OrderSide {
    match side {
        OrderSide::Buy => nautilus_model::enums::OrderSide::Buy,
        OrderSide::Sell => nautilus_model::enums::OrderSide::Sell,
    }
}

/// Maps the venue's order type onto Nautilus's.
#[must_use]
pub const fn map_order_type(order_type: OrderType) -> nautilus_model::enums::OrderType {
    match order_type {
        OrderType::Market => nautilus_model::enums::OrderType::Market,
        OrderType::Limit => nautilus_model::enums::OrderType::Limit,
    }
}

/// Maps the venue's time-in-force onto Nautilus's, resolving `GTX` to post-only.
///
/// `GTX` has no Nautilus time-in-force of its own — Nautilus carries post-only as a flag on the
/// order rather than as a time-in-force — so it reports as `GTC`, which is what it is once the
/// post-only constraint has been applied at placement. The flag itself is not recoverable from a
/// report, and pretending otherwise would be worse than this.
///
/// Total, because the venue has exactly these four and all of them land somewhere.
#[must_use]
pub const fn map_time_in_force(tif: TimeInForce) -> NautilusTif {
    match tif {
        TimeInForce::Gtc | TimeInForce::Gtx => NautilusTif::Gtc,
        TimeInForce::Ioc => NautilusTif::Ioc,
        TimeInForce::Fok => NautilusTif::Fok,
    }
}

/// Builds an order status report from one venue record.
///
/// # Errors
///
/// Returns [`ReportError`] when a value cannot be expressed in Nautilus's types. Refusing a
/// single record is the right outcome: a report that guessed at a status would feed the engine a
/// false picture of an order it is about to act on.
pub fn order_status_report(
    record: &OrderRecord,
    account_id: AccountId,
    instrument_id: InstrumentId,
    price_precision: u8,
    size_precision: u8,
    ts_init: UnixNanos,
) -> Result<OrderStatusReport, ReportError> {
    let invalid = |field: &'static str, value: &str, reason: String| ReportError::InvalidValue {
        order_id: record.order_id,
        field,
        value: value.to_string(),
        reason,
    };

    let status = crate::execution::parse::map_order_status(record.status).ok_or(
        ReportError::UnmappableStatus {
            order_id: record.order_id,
            status: record.status,
        },
    )?;

    let quantity = quantity_at(&record.orig_qty, size_precision)
        .map_err(|e| invalid("origQty", &record.orig_qty, e))?;
    let filled_qty = quantity_at(&record.executed_qty, size_precision)
        .map_err(|e| invalid("executedQty", &record.executed_qty, e))?;

    let mut report = OrderStatusReport::new(
        account_id,
        instrument_id,
        Some(ClientOrderId::new(record.cl_ord_id.as_str())),
        VenueOrderId::new(record.order_id.to_string()),
        Some(map_side(record.side)),
        map_order_type(record.order_type),
        map_time_in_force(record.time_in_force),
        status,
        quantity,
        filled_qty,
        UnixNanos::from(record.created_at_ms * 1_000_000),
        UnixNanos::from(record.updated_at_ms * 1_000_000),
        ts_init,
        None,
    );

    // A market order carries no meaningful limit price; the venue reports "0" for it, and
    // passing that through would describe an order priced at zero.
    if record.order_type == OrderType::Limit {
        report.price = Some(
            price_at(&record.price, price_precision)
                .map_err(|e| invalid("price", &record.price, e))?,
        );
    }

    report.avg_px = average_fill_price(record, price_precision)
        .map_err(|e| invalid("executedValue", &record.executed_value, e))?;

    Ok(report)
}

/// The average price actually achieved, or `None` when nothing has filled.
///
/// Derived from `executedValue / executedQty` because the venue reports no average, and kept as a
/// `Decimal` rather than hopping through a float — this is money.
///
/// `None` rather than `0` for an unfilled order: a zero here would be read as a fill at zero,
/// which is the one misreading that could make a strategy think it had been given free inventory.
fn average_fill_price(
    record: &OrderRecord,
    price_precision: u8,
) -> Result<Option<Decimal>, String> {
    let filled = Decimal::from_str(&record.executed_qty).map_err(|e| e.to_string())?;
    if filled.is_zero() {
        return Ok(None);
    }
    let value = Decimal::from_str(&record.executed_value).map_err(|e| e.to_string())?;

    Ok(Some(
        (value / filled).round_dp(u32::from(price_precision)),
    ))
}

fn price_at(raw: &str, precision: u8) -> Result<Price, String> {
    let normalized = normalize_to(raw, precision).map_err(|e| e.to_string())?;
    Price::from_str(&normalized).map_err(|e| e.to_string())
}

fn quantity_at(raw: &str, precision: u8) -> Result<Quantity, String> {
    let normalized = normalize_to(raw, precision).map_err(|e| e.to_string())?;
    Quantity::from_str(&normalized).map_err(|e| e.to_string())
}

/// Whether a reported status means the order is no longer working.
///
/// Used to decide, after a write whose outcome is unknown, whether the venue has a verdict.
#[must_use]
pub const fn is_terminal(status: NautilusStatus) -> bool {
    matches!(
        status,
        NautilusStatus::Filled
            | NautilusStatus::Canceled
            | NautilusStatus::Rejected
            | NautilusStatus::Expired
            | NautilusStatus::Denied
    )
}

#[cfg(test)]
mod tests {
    use nautilus_model::identifiers::{Symbol, Venue};

    use super::*;
    use crate::config::SODEX_SPOT;

    fn record() -> OrderRecord {
        // The venue's own response for a real testnet order, verbatim from
        // `/accounts/{wallet}/orders/history`.
        OrderRecord {
            symbol: "vBTC_vUSDC".to_string(),
            order_id: 1_289_807_722,
            cl_ord_id: "probe-1789040198677".to_string(),
            side: OrderSide::Buy,
            order_type: OrderType::Limit,
            time_in_force: TimeInForce::Gtc,
            price: "40000".to_string(),
            orig_qty: "0.001".to_string(),
            status: OrderStatus::Canceled,
            executed_qty: "0".to_string(),
            executed_value: "0".to_string(),
            margin_frozen: Some("0".to_string()),
            created_at_ms: 1_789_040_199_583,
            updated_at_ms: 1_789_040_199_772,
        }
    }

    fn report(record: &OrderRecord) -> OrderStatusReport {
        order_status_report(
            record,
            AccountId::from("SODEX_SPOT-60366"),
            InstrumentId::new(Symbol::from("vBTC_vUSDC"), Venue::from(SODEX_SPOT)),
            2,
            5,
            UnixNanos::default(),
        )
        .expect("the venue's own record must convert")
    }

    #[test]
    fn an_unfilled_order_reports_no_average_price() {
        // Not zero. A zero average reads as "filled at zero", which is the one misreading that
        // could make a strategy believe it was handed free inventory.
        let report = report(&record());

        assert_eq!(report.avg_px, None);
        assert_eq!(report.filled_qty.to_string(), "0.00000");
    }

    #[test]
    fn a_partial_fill_reports_the_average_the_venue_implies() {
        // The venue reports no average, only quantity and quote value, so it is derived — and
        // derived in Decimal rather than through a float, because this is money.
        let mut filled = record();
        filled.status = OrderStatus::PartiallyFilled;
        filled.executed_qty = "0.0004".to_string();
        filled.executed_value = "16".to_string();

        let report = report(&filled);

        // 16 / 0.0004 = 40000 exactly.
        assert_eq!(report.avg_px, Some(Decimal::from(40_000)));
        assert_eq!(report.order_status, NautilusStatus::PartiallyFilled);
    }

    #[test]
    fn a_market_order_carries_no_limit_price() {
        // The venue reports "0" as the price of a market order. Passing that through would
        // describe an order priced at zero rather than an order with no limit.
        let mut market = record();
        market.order_type = OrderType::Market;
        market.time_in_force = TimeInForce::Ioc;
        market.price = "0".to_string();

        let report = report(&market);

        assert_eq!(report.price, None);
        assert_eq!(report.time_in_force, NautilusTif::Ioc);
    }

    #[test]
    fn post_only_reports_as_gtc_because_nautilus_carries_it_as_a_flag() {
        // `GTX` is the venue's post-only. Nautilus has no such time-in-force — it carries
        // post-only as a flag on the order — so the resting behaviour is what survives into a
        // report. The flag itself is not recoverable, and inventing one would be worse.
        let mut post_only = record();
        post_only.time_in_force = TimeInForce::Gtx;

        assert_eq!(report(&post_only).time_in_force, NautilusTif::Gtc);
    }

    #[test]
    fn identifiers_and_timestamps_come_from_the_venue() {
        let report = report(&record());

        assert_eq!(report.venue_order_id.to_string(), "1289807722");
        assert_eq!(
            report.client_order_id.map(|id| id.to_string()),
            Some("probe-1789040198677".to_string())
        );
        // Milliseconds to nanoseconds, and `ts_last` is the venue's update time rather than now:
        // reconciliation compares these against the engine's own view of when things happened.
        assert_eq!(report.ts_accepted.as_u64(), 1_789_040_199_583 * 1_000_000);
        assert_eq!(report.ts_last.as_u64(), 1_789_040_199_772 * 1_000_000);
    }

    #[test]
    fn a_status_nautilus_cannot_express_is_refused_rather_than_guessed() {
        // `TRIGGERED` is a perps stop state with no Nautilus equivalent. Mapping it to something
        // plausible would hand the engine a false picture of an order it is about to act on.
        let mut triggered = record();
        triggered.status = OrderStatus::Triggered;

        let error = order_status_report(
            &triggered,
            AccountId::from("SODEX_SPOT-60366"),
            InstrumentId::new(Symbol::from("vBTC_vUSDC"), Venue::from(SODEX_SPOT)),
            2,
            5,
            UnixNanos::default(),
        )
        .expect_err("an unmappable status must not become a report");

        assert!(matches!(error, ReportError::UnmappableStatus { .. }));
    }

    #[test]
    fn a_terminal_status_is_recognised_as_settled() {
        // Used to decide whether the venue has reached a verdict on a write whose outcome was
        // unknown, so the open states must not be mistaken for settled ones.
        assert!(is_terminal(NautilusStatus::Filled));
        assert!(is_terminal(NautilusStatus::Canceled));
        assert!(is_terminal(NautilusStatus::Rejected));
        assert!(!is_terminal(NautilusStatus::Accepted));
        assert!(!is_terminal(NautilusStatus::PartiallyFilled));
    }
}

#[cfg(test)]
mod lookup_tests {
    use super::*;
    use crate::http::account_reads::OrderRecord;

    fn record(cl_ord_id: &str, status: OrderStatus) -> OrderRecord {
        OrderRecord {
            symbol: "vBTC_vUSDC".to_string(),
            order_id: 1,
            cl_ord_id: cl_ord_id.to_string(),
            side: OrderSide::Buy,
            order_type: OrderType::Limit,
            time_in_force: TimeInForce::Gtc,
            price: "40000".to_string(),
            orig_qty: "0.001".to_string(),
            status,
            executed_qty: "0".to_string(),
            executed_value: "0".to_string(),
            margin_frozen: None,
            created_at_ms: 1,
            updated_at_ms: 2,
        }
    }

    /// Mirrors how an ambiguous submission is resolved: both lists, open first.
    fn find<'a>(
        open: &'a [OrderRecord],
        history: &'a [OrderRecord],
        wanted: &str,
    ) -> Option<&'a OrderRecord> {
        open.iter()
            .chain(history.iter())
            .find(|record| record.cl_ord_id == wanted)
    }

    #[test]
    fn an_order_that_filled_immediately_is_found_in_history_not_open() {
        // The case that makes searching only the open list dangerous: an accepted order that
        // filled at once never appears there, and calling it absent would report a completed
        // order as rejected.
        let open: Vec<OrderRecord> = Vec::new();
        let history = vec![record("O-1", OrderStatus::Filled)];

        let found = find(&open, &history, "O-1").expect("history must be searched too");

        assert_eq!(found.status, OrderStatus::Filled);
    }

    #[test]
    fn a_resting_order_is_found_on_the_open_list() {
        let open = vec![record("O-1", OrderStatus::New)];

        assert!(find(&open, &[], "O-1").is_some());
    }

    #[test]
    fn an_order_the_venue_never_saw_is_absent_from_both() {
        // Only then is a rejection an observation rather than an assumption.
        let open = vec![record("O-other", OrderStatus::New)];
        let history = vec![record("O-older", OrderStatus::Canceled)];

        assert!(find(&open, &history, "O-1").is_none());
    }

    #[test]
    fn matching_is_exact_rather_than_prefixed() {
        // Client order ids share prefixes by construction — a timestamped label and its cancel
        // label differ only at the end — so a prefix match could resolve one order's fate from
        // another's record.
        let history = vec![record("O-1-retry", OrderStatus::Filled)];

        assert!(find(&[], &history, "O-1").is_none());
    }
}
