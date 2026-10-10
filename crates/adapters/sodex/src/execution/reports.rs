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
//! cancelled order as simply absent - which the engine cannot distinguish from an order it should
//! never have known about.
//!
//! History is also not one row per order but one row per state change, so the two lists are
//! reduced to each order's latest revision before anything is reported - see [`current_orders`].
//!
//! # A reported fill beats an inferred one, and says why
//!
//! With `/accounts/{wallet}/trades` typed, fills come from the venue rather than being inferred
//! from an order record. Three things are authoritative only on this path:
//!
//! - the venue's own `tradeID`, instead of a synthetic one;
//! - `isMaker`, so the liquidity side is known rather than assumed;
//! - `fee` together with `feeCoin`, which is the asset the fee was actually taken from - the
//!   **base** asset on a buy, deducted from what arrives.
//!
//! # Average fill price is derived, not reported
//!
//! The venue gives `executedQty` and `executedValue` (the quote-asset total), not an average
//! price. Dividing is the only way to get one, and it is only defined once something has filled -
//! so an unfilled order reports no average rather than a zero, which would read as "filled at
//! zero".

use std::{collections::BTreeMap, str::FromStr};

use nautilus_core::UnixNanos;
use nautilus_model::{
    enums::{
        LiquiditySide, OrderSide as NautilusSide, OrderStatus as NautilusStatus,
        OrderType as NautilusType, PositionSide, TimeInForce as NautilusTif,
    },
    identifiers::{AccountId, ClientOrderId, InstrumentId, PositionId, TradeId, VenueOrderId},
    reports::{fill::FillReport, order::OrderStatusReport, position::PositionStatusReport},
    types::{Currency, Money, Price, Quantity},
};
use rust_decimal::Decimal;

use crate::{
    common::{
        decimal::normalize_to,
        enums::{OrderSide, OrderStatus, OrderType, TimeInForce},
    },
    http::account_reads::{OrderRecord, PositionRecord, TradeRecord},
};

/// Why a venue order record cannot become a report.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ReportError {
    #[error("order {order_id} reports status {status:?}, which has no Nautilus equivalent")]
    UnmappableStatus { order_id: u64, status: OrderStatus },
    #[error("order {order_id} reports a fee in {coin:?}, which is not a currency the engine knows")]
    UnknownFeeCurrency { order_id: u64, coin: String },
    #[error("order {order_id} has an unparsable {field}: {value:?} ({reason})")]
    InvalidValue {
        order_id: u64,
        field: &'static str,
        value: String,
        reason: String,
    },
    #[error("position {position_id} has an unparsable {field}: {value:?} ({reason})")]
    InvalidPositionValue {
        position_id: u64,
        field: &'static str,
        value: String,
        reason: String,
    },
}

/// Builds a position status report from one venue position.
///
/// # Direction comes from the sign of `size`
///
/// `position_side` is `BOTH` on this venue whichever way the position runs - that is what one-way
/// mode reports, and it carries no direction. Both directions were observed on testnet instead: a
/// long read `"0.0002"` and a short `"-0.0002"`. Reading the side from `position_side` would report
/// every short as a long, so the sign is the only source used here.
///
/// Nautilus derives the signed quantity from the side plus an unsigned quantity, so the magnitude
/// is what gets passed.
///
/// # Errors
///
/// Returns [`ReportError::InvalidPositionValue`] if the size or entry price cannot be parsed.
pub fn position_status_report(
    record: &PositionRecord,
    account_id: AccountId,
    instrument_id: InstrumentId,
    size_precision: u8,
    ts_init: UnixNanos,
) -> Result<PositionStatusReport, ReportError> {
    let invalid =
        |field: &'static str, value: &str, reason: String| ReportError::InvalidPositionValue {
            position_id: record.id,
            field,
            value: value.to_string(),
            reason,
        };

    let signed = Decimal::from_str(&record.size)
        .map_err(|e| invalid("size", &record.size, e.to_string()))?;

    let side = if signed.is_zero() {
        PositionSide::Flat
    } else if signed.is_sign_positive() {
        PositionSide::Long
    } else {
        PositionSide::Short
    };

    let quantity = quantity_at(record.size.trim_start_matches('-'), size_precision)
        .map_err(|e| invalid("size", &record.size, e))?;

    // Zero on a position the venue has not closed into: reporting it as `None` rather than as an
    // average of zero keeps a caller from treating "no entry yet" as "entered at zero".
    let avg_px_open = Decimal::from_str(&record.avg_entry_price)
        .map_err(|e| invalid("avgEntryPrice", &record.avg_entry_price, e.to_string()))?;
    let avg_px_open = (!avg_px_open.is_zero()).then_some(avg_px_open);

    Ok(PositionStatusReport::new(
        account_id,
        instrument_id,
        side,
        quantity,
        UnixNanos::from(record.updated_at_ms * 1_000_000),
        ts_init,
        None,
        Some(PositionId::new(record.id.to_string())),
        avg_px_open,
    ))
}

/// Maps the venue's side onto Nautilus's.
#[must_use]
pub const fn map_side(side: OrderSide) -> NautilusSide {
    match side {
        OrderSide::Buy => NautilusSide::Buy,
        OrderSide::Sell => NautilusSide::Sell,
    }
}

/// Maps the venue's order type onto Nautilus's.
#[must_use]
pub const fn map_order_type(order_type: OrderType) -> NautilusType {
    match order_type {
        OrderType::Market => NautilusType::Market,
        OrderType::Limit => NautilusType::Limit,
    }
}

/// Maps the venue's time-in-force onto Nautilus's, resolving `GTX` to post-only.
///
/// `GTX` has no Nautilus time-in-force of its own - Nautilus carries post-only as a flag on the
/// order rather than as a time-in-force - so it reports as `GTC`, which is what it is once the
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

/// Reduces the venue's two order lists to one record per order: the one describing where it stands
/// now.
///
/// # History is a revision log, not a list of orders
///
/// `/orders/history` answers with a row per state change, newest first, all under the order's one
/// `orderID`. A limit order that rested, was modified and was then cancelled appears three times -
/// `NEW` at the price it was placed at, `NEW` at the modified price, `CANCELED` - and every one of
/// those rows is a well-formed order record. Reporting each of them hands the engine several
/// reports for one order, of which it keeps the last per venue order id; with the venue's
/// newest-first ordering that is the oldest revision. Long-cancelled orders came back as accepted
/// that way, with their original `price * quantity` booked as locked balance, and every cancel the
/// engine then sent was answered `OrderNotFound`.
///
/// The latest revision is picked by `updatedAt`, not by position in the response, and a tie goes
/// to the later lifecycle stage: an order that crosses on arrival can be accepted and filled in the
/// same millisecond, and only the stage separates those two rows.
///
/// # Only the open list says an order is still working
///
/// An order whose latest revision reads `NEW` or `PARTIALLY_FILLED` but which `/orders` does not
/// hold is left out. Reporting it as working would claim a resting order the venue has just said
/// it does not have; leaving it out lets the next read, by which time history carries the terminal
/// row, settle it.
///
/// Orders are returned in venue order id order, which is the order they were placed in.
#[must_use]
pub fn current_orders<'a>(
    open: &'a [OrderRecord],
    history: &'a [OrderRecord],
) -> Vec<&'a OrderRecord> {
    let mut latest: BTreeMap<u64, &OrderRecord> = BTreeMap::new();

    for record in open.iter().chain(history) {
        latest
            .entry(record.order_id)
            .and_modify(|held| {
                if revision_rank(record) > revision_rank(held) {
                    *held = record;
                }
            })
            .or_insert(record);
    }

    latest
        .into_values()
        .filter(|record| {
            let working = matches!(
                record.status,
                OrderStatus::New | OrderStatus::PartiallyFilled
            );
            let listed_open = open.iter().any(|held| held.order_id == record.order_id);
            if working && !listed_open {
                log::warn!(
                    "sodex_order_report_skipped order_id={} status={} reason=not_on_open_list",
                    record.order_id,
                    record.status
                );
                return false;
            }
            true
        })
        .collect()
}

/// Orders an order's revisions: by update time, then by how far along its lifecycle each is.
const fn revision_rank(record: &OrderRecord) -> (u64, u8) {
    let stage = match record.status {
        OrderStatus::New => 0,
        OrderStatus::Triggered => 1,
        OrderStatus::PartiallyFilled => 2,
        OrderStatus::Filled
        | OrderStatus::Canceled
        | OrderStatus::Rejected
        | OrderStatus::Expired => 3,
    };
    (record.updated_at_ms, stage)
}

/// The average price actually achieved, or `None` when nothing has filled.
///
/// Derived from `executedValue / executedQty` because the venue reports no average, and kept as a
/// `Decimal` rather than hopping through a float - this is money.
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

    Ok(Some((value / filled).round_dp(u32::from(price_precision))))
}

fn price_at(raw: &str, precision: u8) -> Result<Price, String> {
    let normalized = normalize_to(raw, precision).map_err(|e| e.to_string())?;
    Price::from_str(&normalized)
}

fn quantity_at(raw: &str, precision: u8) -> Result<Quantity, String> {
    let normalized = normalize_to(raw, precision).map_err(|e| e.to_string())?;
    Quantity::from_str(&normalized)
}

/// Builds a fill report from one venue trade.
///
/// Everything here is the venue's own: the trade id, the liquidity side, and the fee in the asset
/// it was actually charged in. That last point is why the fee currency is read from the response
/// rather than assumed to be the quote asset - a buy pays in the base asset, and recording it as
/// quote would misstate which balance moved.
///
/// # Errors
///
/// Returns [`ReportError::InvalidValue`] if a price, quantity or fee cannot be parsed, or
/// [`ReportError::UnknownFeeCurrency`] for a fee asset the engine does not know.
pub fn fill_report(
    trade: &TradeRecord,
    account_id: AccountId,
    instrument_id: InstrumentId,
    price_precision: u8,
    size_precision: u8,
    ts_init: UnixNanos,
) -> Result<FillReport, ReportError> {
    let invalid = |field: &'static str, value: &str, reason: String| ReportError::InvalidValue {
        order_id: trade.order_id,
        field,
        value: value.to_string(),
        reason,
    };

    let fee_currency =
        Currency::try_from_str(&trade.fee_coin).ok_or_else(|| ReportError::UnknownFeeCurrency {
            order_id: trade.order_id,
            coin: trade.fee_coin.clone(),
        })?;
    // Parsed as a `Decimal` first, not a float: the fee is fractions of a basis point on an
    // 18-decimal asset, and a float hop here is a rounding policy nobody chose.
    let normalized = normalize_to(&trade.fee, fee_currency.precision)
        .map_err(|e| invalid("fee", &trade.fee, e.to_string()))?;
    let commission =
        Decimal::from_str(&normalized).map_err(|e| invalid("fee", &trade.fee, e.to_string()))?;

    Ok(FillReport::new(
        account_id,
        instrument_id,
        VenueOrderId::new(trade.order_id.to_string()),
        TradeId::new(trade.trade_id.to_string().as_str()),
        map_side(trade.side),
        quantity_at(&trade.quantity, size_precision)
            .map_err(|e| invalid("quantity", &trade.quantity, e))?,
        price_at(&trade.price, price_precision).map_err(|e| invalid("price", &trade.price, e))?,
        Money::new(
            commission
                .try_into()
                .map_err(|e: rust_decimal::Error| invalid("fee", &trade.fee, e.to_string()))?,
            fee_currency,
        ),
        if trade.is_maker {
            LiquiditySide::Maker
        } else {
            LiquiditySide::Taker
        },
        Some(ClientOrderId::new(trade.cl_ord_id.as_str())),
        // The venue reports no position id. Spot has no positions, and on perps the engine's own
        // netting is the authority rather than a field that does not exist.
        None,
        UnixNanos::from(trade.time * 1_000_000),
        ts_init,
        None,
    ))
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
    use rstest::rstest;

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

    #[rstest]
    fn an_unfilled_order_reports_no_average_price() {
        // Not zero. A zero average reads as "filled at zero", which is the one misreading that
        // could make a strategy believe it was handed free inventory.
        let report = report(&record());

        assert_eq!(report.avg_px, None);
        assert_eq!(report.filled_qty.to_string(), "0.00000");
    }

    #[rstest]
    fn a_partial_fill_reports_the_average_the_venue_implies() {
        // The venue reports no average, only quantity and quote value, so it is derived - and
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

    #[rstest]
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

    #[rstest]
    fn post_only_reports_as_gtc_because_nautilus_carries_it_as_a_flag() {
        // `GTX` is the venue's post-only. Nautilus has no such time-in-force - it carries
        // post-only as a flag on the order - so the resting behavior is what survives into a
        // report. The flag itself is not recoverable, and inventing one would be worse.
        let mut post_only = record();
        post_only.time_in_force = TimeInForce::Gtx;

        assert_eq!(report(&post_only).time_in_force, NautilusTif::Gtc);
    }

    #[rstest]
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

    #[rstest]
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

    #[rstest]
    fn a_terminal_status_is_recognized_as_settled() {
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
    use rstest::rstest;

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

    #[rstest]
    fn an_order_that_filled_immediately_is_found_in_history_not_open() {
        // The case that makes searching only the open list dangerous: an accepted order that
        // filled at once never appears there, and calling it absent would report a completed
        // order as rejected.
        let open: Vec<OrderRecord> = Vec::new();
        let history = vec![record("O-1", OrderStatus::Filled)];

        let found = find(&open, &history, "O-1").expect("history must be searched too");

        assert_eq!(found.status, OrderStatus::Filled);
    }

    #[rstest]
    fn a_resting_order_is_found_on_the_open_list() {
        let open = vec![record("O-1", OrderStatus::New)];

        assert!(find(&open, &[], "O-1").is_some());
    }

    #[rstest]
    fn an_order_the_venue_never_saw_is_absent_from_both() {
        // Only then is a rejection an observation rather than an assumption.
        let open = vec![record("O-other", OrderStatus::New)];
        let history = vec![record("O-older", OrderStatus::Canceled)];

        assert!(find(&open, &history, "O-1").is_none());
    }

    #[rstest]
    fn matching_is_exact_rather_than_prefixed() {
        // Client order ids share prefixes by construction - a timestamped label and its cancel
        // label differ only at the end - so a prefix match could resolve one order's fate from
        // another's record.
        let history = vec![record("O-1-retry", OrderStatus::Filled)];

        assert!(find(&[], &history, "O-1").is_none());
    }
}

#[cfg(test)]
mod fill_tests {
    use nautilus_model::{
        enums::CurrencyType,
        identifiers::{Symbol, Venue},
        types::fixed::FIXED_PRECISION,
    };
    use rstest::rstest;

    use super::*;
    use crate::{config::SODEX_SPOT, http::account_reads::TradeRecord};

    /// The venue's own response for a real testnet fill, verbatim from
    /// `/accounts/{wallet}/trades`.
    fn trade() -> TradeRecord {
        TradeRecord {
            trade_id: 9_458_096,
            order_id: 1_290_012_932,
            cl_ord_id: "fillprobe-1789108466447".to_string(),
            symbol: "vBTC_vUSDC".to_string(),
            side: OrderSide::Buy,
            price: "77184".to_string(),
            quantity: "0.001".to_string(),
            fee: "0.00000065".to_string(),
            fee_coin: "vBTC".to_string(),
            is_maker: false,
            time: 1_789_108_467_221,
        }
    }

    /// Registers a venue coin the way the provider does: at the engine's full width, not at the
    /// precision the symbol listing reports for it.
    fn registered_coin(code: &str) -> Currency {
        let currency = Currency::new(code, FIXED_PRECISION, 0, code, CurrencyType::Crypto);
        let _ = Currency::register(currency, false);
        currency
    }

    /// The venue's own response for the flattening sell, verbatim.
    fn sell_trade() -> TradeRecord {
        TradeRecord {
            trade_id: 9_458_097,
            order_id: 1_290_029_653,
            cl_ord_id: "fillprobe-1789115999154".to_string(),
            symbol: "vBTC_vUSDC".to_string(),
            side: OrderSide::Sell,
            price: "77401".to_string(),
            quantity: "0.00099".to_string(),
            fee: "0.0498075435".to_string(),
            fee_coin: "vUSDC".to_string(),
            is_maker: false,
            time: 1_789_116_003_061,
        }
    }

    fn report(trade: &TradeRecord) -> FillReport {
        registered_coin("vBTC");
        registered_coin("vUSDC");
        fill_report(
            trade,
            AccountId::from("SODEX_SPOT-60366"),
            InstrumentId::new(Symbol::from("vBTC_vUSDC"), Venue::from(SODEX_SPOT)),
            2,
            5,
            UnixNanos::default(),
        )
        .expect("the venue's own fill must convert")
    }

    #[rstest]
    fn the_fee_keeps_the_asset_the_venue_charged_it_in() {
        // A buy's fee comes out of the **base** asset, not the quote: ordering 0.001 vBTC credits
        // 0.00099935, and the difference is this fee. Recording it as quote would misstate which
        // balance moved.
        let report = report(&trade());

        assert_eq!(report.commission.currency.code.as_str(), "vBTC");
        assert_eq!(report.commission.as_decimal(), Decimal::new(65, 8));
    }

    #[rstest]
    fn the_liquidity_side_is_the_venue_s_rather_than_a_guess() {
        // This is the whole advantage of a reported fill over an inferred one: `isMaker` is
        // stated, so nothing has to assume the conservative side.
        assert_eq!(report(&trade()).liquidity_side, LiquiditySide::Taker);

        let mut maker = trade();
        maker.is_maker = true;
        assert_eq!(report(&maker).liquidity_side, LiquiditySide::Maker);
    }

    #[rstest]
    fn the_venue_s_own_trade_id_is_carried() {
        // An inferred fill has to synthesize one. A reported fill must not, or two runs would
        // disagree about which trade they were talking about.
        let report = report(&trade());

        assert_eq!(report.trade_id.to_string(), "9458096");
        assert_eq!(report.venue_order_id.to_string(), "1290012932");
        assert_eq!(
            report.client_order_id.map(|id| id.to_string()),
            Some("fillprobe-1789108466447".to_string())
        );
    }

    #[rstest]
    fn the_fill_is_stamped_with_its_own_time() {
        assert_eq!(
            report(&trade()).ts_event.as_u64(),
            1_789_108_467_221 * 1_000_000
        );
    }

    #[rstest]
    fn a_sell_pays_its_fee_in_the_quote_asset() {
        // The other half of the rule, and observed rather than assumed from symmetry: the fee is
        // charged in the asset received, so a sell pays in quote where a buy pays in base.
        let report = report(&sell_trade());

        assert_eq!(report.commission.currency.code.as_str(), "vUSDC");
        assert_eq!(report.order_side, NautilusSide::Sell);
    }

    #[rstest]
    fn the_fee_is_carried_at_full_precision_not_the_listed_coin_precision() {
        // This rounded before. The symbol listing reports `quoteCoinPrecision: 6` for vUSDC, but
        // the venue's ledger carries ten places - fee `0.0498075435`, balance `999.3931824565`.
        // Registering the coin at six turned that fee into `0.049808`, overstating it and leaving
        // the recorded commission unable to reconcile against the balance it came out of.
        let report = report(&sell_trade());

        assert_eq!(
            report.commission.as_decimal(),
            Decimal::from_str("0.0498075435").unwrap(),
            "the venue's fee must survive verbatim"
        );
    }

    #[rstest]
    fn the_fee_equals_notional_times_the_rate_on_both_sides() {
        // Why the quote-denominated estimate used for *inferred* fills is exact: a sell's fee is
        // `notional * rate` outright, and a buy's base-denominated fee converted at the fill price
        // comes to the same number. Checked against both observed trades rather than assumed.
        let rate = Decimal::from_str("0.00065").unwrap();

        let sell = sell_trade();
        let sell_notional =
            Decimal::from_str(&sell.quantity).unwrap() * Decimal::from_str(&sell.price).unwrap();
        assert_eq!(sell_notional * rate, Decimal::from_str(&sell.fee).unwrap());

        let buy = trade();
        let buy_fee_in_quote =
            Decimal::from_str(&buy.fee).unwrap() * Decimal::from_str(&buy.price).unwrap();
        let buy_notional =
            Decimal::from_str(&buy.quantity).unwrap() * Decimal::from_str(&buy.price).unwrap();
        assert_eq!(buy_notional * rate, buy_fee_in_quote);
    }

    #[rstest]
    fn a_fee_in_an_unknown_asset_is_refused_rather_than_silently_dropped() {
        // The venue can list a coin this engine has never registered. Defaulting the currency
        // would book the fee against the wrong balance, so the fill is refused and named.
        let mut odd = trade();
        odd.fee_coin = "vNOTACOIN".to_string();

        let error = fill_report(
            &odd,
            AccountId::from("SODEX_SPOT-60366"),
            InstrumentId::new(Symbol::from("vBTC_vUSDC"), Venue::from(SODEX_SPOT)),
            2,
            5,
            UnixNanos::default(),
        )
        .expect_err("an unknown fee currency must not become a report");

        assert!(matches!(error, ReportError::UnknownFeeCurrency { .. }));
    }
}

#[cfg(test)]
mod position_tests {
    use nautilus_model::identifiers::{Symbol, Venue};
    use rstest::rstest;

    use super::*;
    use crate::config::SODEX_PERPS;

    /// The venue's own response for a real testnet long, verbatim from
    /// `/accounts/{wallet}/positions`.
    fn long() -> PositionRecord {
        PositionRecord {
            id: 2_586_243,
            symbol: "BTC-USD".to_string(),
            size: "0.0002".to_string(),
            avg_entry_price: "77280".to_string(),
            avg_close_price: "0".to_string(),
            position_side: "BOTH".to_string(),
            leverage: 20,
            margin_mode: "CROSS".to_string(),
            initial_margin: "0.7728".to_string(),
            max_size: "0.0002".to_string(),
            cum_open_cost: "15.456".to_string(),
            cum_closed_size: "0".to_string(),
            cum_trading_fee: "0.0061824".to_string(),
            realized_pnl: "-0.0061824".to_string(),
            active: true,
            is_taken_over: false,
            take_over_price: "0".to_string(),
            created_at_ms: 1_789_171_771_461,
            updated_at_ms: 1_789_171_771_461,
        }
    }

    /// The same account's short, which differs from the long only in the sign of `size`.
    fn short() -> PositionRecord {
        PositionRecord {
            id: 2_586_244,
            size: "-0.0002".to_string(),
            avg_entry_price: "77260".to_string(),
            initial_margin: "0.7726".to_string(),
            cum_open_cost: "15.452".to_string(),
            cum_trading_fee: "0.0061808".to_string(),
            realized_pnl: "-0.0061808".to_string(),
            created_at_ms: 1_789_175_122_512,
            updated_at_ms: 1_789_175_122_512,
            ..long()
        }
    }

    fn instrument() -> InstrumentId {
        InstrumentId::new(Symbol::from("BTC-USD"), Venue::from(SODEX_PERPS))
    }

    fn account() -> AccountId {
        AccountId::from("SODEX_PERPS-60366")
    }

    #[rstest]
    fn a_positive_size_reports_a_long() {
        let report =
            position_status_report(&long(), account(), instrument(), 5, UnixNanos::default())
                .unwrap();

        assert_eq!(report.position_side, PositionSide::Long);
        assert_eq!(report.quantity.to_string(), "0.00020");
        assert_eq!(
            report.signed_decimal_qty,
            Decimal::from_str("0.00020").unwrap()
        );
    }

    #[rstest]
    fn a_negative_size_reports_a_short_of_the_same_magnitude() {
        // The only field distinguishing the two on this venue. `position_side` reads `BOTH` on
        // both, so reading the side from it would report this short as a long.
        let report =
            position_status_report(&short(), account(), instrument(), 5, UnixNanos::default())
                .unwrap();

        assert_eq!(report.position_side, PositionSide::Short);
        assert_eq!(report.quantity.to_string(), "0.00020");
        assert_eq!(
            report.signed_decimal_qty,
            Decimal::from_str("-0.00020").unwrap()
        );
    }

    #[rstest]
    fn both_directions_report_the_venue_side_field_as_uninformative() {
        // Guards the reason the sign is used: if the venue ever starts distinguishing here, these
        // fixtures stop matching what is deployed and this test is the place that says so.
        assert_eq!(long().position_side, "BOTH");
        assert_eq!(short().position_side, "BOTH");
    }

    #[rstest]
    fn the_entry_price_and_venue_id_carry_through() {
        let report =
            position_status_report(&long(), account(), instrument(), 5, UnixNanos::default())
                .unwrap();

        assert_eq!(report.avg_px_open, Some(Decimal::from(77_280)));
        assert_eq!(
            report.venue_position_id.map(|id| id.to_string()),
            Some("2586243".to_string())
        );
    }

    #[rstest]
    fn the_report_timestamp_is_the_update_time_in_nanoseconds() {
        let record = long();
        let report =
            position_status_report(&record, account(), instrument(), 5, UnixNanos::default())
                .unwrap();

        assert_eq!(report.ts_last.as_u64(), record.updated_at_ms * 1_000_000);
    }

    #[rstest]
    fn a_zero_entry_price_reports_no_average_rather_than_zero() {
        // Zero would read as "entered at a price of nothing"; absence reads as "not known".
        let record = PositionRecord {
            avg_entry_price: "0".to_string(),
            ..long()
        };
        let report =
            position_status_report(&record, account(), instrument(), 5, UnixNanos::default())
                .unwrap();

        assert_eq!(report.avg_px_open, None);
    }

    #[rstest]
    fn an_unparsable_size_names_the_position_rather_than_an_order() {
        let record = PositionRecord {
            size: "not-a-number".to_string(),
            ..long()
        };
        let error =
            position_status_report(&record, account(), instrument(), 5, UnixNanos::default())
                .unwrap_err();

        assert!(matches!(
            error,
            ReportError::InvalidPositionValue {
                position_id: 2_586_243,
                field: "size",
                ..
            }
        ));
    }
}

#[cfg(test)]
mod revision_tests {
    use nautilus_model::{
        identifiers::{ClientId, Symbol, Venue},
        reports::mass_status::ExecutionMassStatus,
    };
    use rstest::rstest;

    use super::*;
    use crate::{config::SODEX_SPOT, http::account_reads::OpenOrders};

    /// The spot account's `/accounts/{wallet}/orders/history`, verbatim as read on 2026-10-10,
    /// while `/accounts/{wallet}/orders` and the account state both listed no open order.
    ///
    /// Seven orders in fourteen rows: one row per state change, newest first. The two `modprobe`
    /// orders rested, were modified from 40000 to 39000 and were then cancelled, so each appears
    /// three times; the plain `probe` orders rested and were cancelled; the `fillprobe` market
    /// orders filled on arrival and appear once.
    const HISTORY: &str = r#"[
        {"symbol":"vBTC_vUSDC","orderID":1290675050,"clOrdID":"modprobe-1789366562193","side":"BUY","type":"LIMIT","timeInForce":"GTC","price":"39000","origQty":"0.001","status":"CANCELED","executedQty":"0","executedValue":"0","marginFrozen":"0","createdAt":1789366566113,"updatedAt":1789366566894},
        {"symbol":"vBTC_vUSDC","orderID":1290675050,"clOrdID":"modprobe-1789366562193","side":"BUY","type":"LIMIT","timeInForce":"GTC","price":"39000","origQty":"0.001","status":"NEW","executedQty":"0","executedValue":"0","marginFrozen":"39","createdAt":1789366566113,"updatedAt":1789366566485},
        {"symbol":"vBTC_vUSDC","orderID":1290675050,"clOrdID":"modprobe-1789366562193","side":"BUY","type":"LIMIT","timeInForce":"GTC","price":"40000","origQty":"0.001","status":"NEW","executedQty":"0","executedValue":"0","marginFrozen":"40","createdAt":1789366566113,"updatedAt":1789366566113},
        {"symbol":"vBTC_vUSDC","orderID":1290674856,"clOrdID":"modprobe-1789366491054","side":"BUY","type":"LIMIT","timeInForce":"GTC","price":"39000","origQty":"0.001","status":"CANCELED","executedQty":"0","executedValue":"0","marginFrozen":"0","createdAt":1789366491841,"updatedAt":1789366531252},
        {"symbol":"vBTC_vUSDC","orderID":1290674856,"clOrdID":"modprobe-1789366491054","side":"BUY","type":"LIMIT","timeInForce":"GTC","price":"39000","origQty":"0.001","status":"NEW","executedQty":"0","executedValue":"0","marginFrozen":"39","createdAt":1789366491841,"updatedAt":1789366492268},
        {"symbol":"vBTC_vUSDC","orderID":1290674856,"clOrdID":"modprobe-1789366491054","side":"BUY","type":"LIMIT","timeInForce":"GTC","price":"40000","origQty":"0.001","status":"NEW","executedQty":"0","executedValue":"0","marginFrozen":"40","createdAt":1789366491841,"updatedAt":1789366491841},
        {"symbol":"vBTC_vUSDC","orderID":1290674093,"clOrdID":"probe-1789366226975","side":"BUY","type":"LIMIT","timeInForce":"GTC","price":"40002","origQty":"0.001","status":"CANCELED","executedQty":"0","executedValue":"0","marginFrozen":"0","createdAt":1789366227859,"updatedAt":1789366242504},
        {"symbol":"vBTC_vUSDC","orderID":1290674086,"clOrdID":"probe-1789366224603","side":"BUY","type":"LIMIT","timeInForce":"GTC","price":"40001","origQty":"0.001","status":"CANCELED","executedQty":"0","executedValue":"0","marginFrozen":"0","createdAt":1789366225689,"updatedAt":1789366242504},
        {"symbol":"vBTC_vUSDC","orderID":1290674093,"clOrdID":"probe-1789366226975","side":"BUY","type":"LIMIT","timeInForce":"GTC","price":"40002","origQty":"0.001","status":"NEW","executedQty":"0","executedValue":"0","marginFrozen":"40.002","createdAt":1789366227859,"updatedAt":1789366227859},
        {"symbol":"vBTC_vUSDC","orderID":1290674086,"clOrdID":"probe-1789366224603","side":"BUY","type":"LIMIT","timeInForce":"GTC","price":"40001","origQty":"0.001","status":"NEW","executedQty":"0","executedValue":"0","marginFrozen":"40.001","createdAt":1789366225689,"updatedAt":1789366225689},
        {"symbol":"vBTC_vUSDC","orderID":1290029653,"clOrdID":"fillprobe-1789115999154","side":"SELL","type":"MARKET","timeInForce":"IOC","price":"69661.35","origQty":"0.00099","status":"FILLED","executedQty":"0.00099","executedValue":"76.62699","marginFrozen":"0","createdAt":1789116003061,"updatedAt":1789116003061},
        {"symbol":"vBTC_vUSDC","orderID":1290012932,"clOrdID":"fillprobe-1789108466447","side":"BUY","type":"MARKET","timeInForce":"IOC","price":"84901.85","origQty":"0.001","status":"FILLED","executedQty":"0.001","executedValue":"77.184","marginFrozen":"0","createdAt":1789108467221,"updatedAt":1789108467221},
        {"symbol":"vBTC_vUSDC","orderID":1289807722,"clOrdID":"probe-1789040198677","side":"BUY","type":"LIMIT","timeInForce":"GTC","price":"40000","origQty":"0.001","status":"CANCELED","executedQty":"0","executedValue":"0","marginFrozen":"0","createdAt":1789040199583,"updatedAt":1789040199772},
        {"symbol":"vBTC_vUSDC","orderID":1289807722,"clOrdID":"probe-1789040198677","side":"BUY","type":"LIMIT","timeInForce":"GTC","price":"40000","origQty":"0.001","status":"NEW","executedQty":"0","executedValue":"0","marginFrozen":"40","createdAt":1789040199583,"updatedAt":1789040199583}
    ]"#;

    /// The open list as read alongside [`HISTORY`]: empty.
    const NO_OPEN_ORDERS: &str =
        r#"{"blockTime":1791641491424,"blockHeight":283520468,"orders":[]}"#;

    /// An open list holding one resting order and one partly filled order.
    ///
    /// Constructed in the venue's shape rather than captured: the account held nothing open when
    /// the history above was read. The partly filled order's history appears in
    /// [`HISTORY_WITH_LIVE_ORDERS`].
    const TWO_OPEN_ORDERS: &str = r#"{"blockTime":1791641491424,"blockHeight":283520468,"orders":[
        {"symbol":"vBTC_vUSDC","orderID":1290700001,"clOrdID":"resting-1","side":"BUY","type":"LIMIT","timeInForce":"GTC","price":"40000","origQty":"0.001","status":"NEW","executedQty":"0","executedValue":"0","marginFrozen":"40","createdAt":1791641400000,"updatedAt":1791641400000},
        {"symbol":"vBTC_vUSDC","orderID":1290700002,"clOrdID":"partial-open-1","side":"BUY","type":"LIMIT","timeInForce":"GTC","price":"40000","origQty":"0.001","status":"PARTIALLY_FILLED","executedQty":"0.0004","executedValue":"16","marginFrozen":"24","createdAt":1791641410000,"updatedAt":1791641420000}
    ]}"#;

    /// History rows for the orders on [`TWO_OPEN_ORDERS`], plus one order that partly filled and
    /// was then cancelled. Constructed in the venue's revision-log shape, newest first.
    const HISTORY_WITH_LIVE_ORDERS: &str = r#"[
        {"symbol":"vBTC_vUSDC","orderID":1290700003,"clOrdID":"partial-cancel-1","side":"BUY","type":"LIMIT","timeInForce":"GTC","price":"40000","origQty":"0.001","status":"CANCELED","executedQty":"0.0004","executedValue":"16","marginFrozen":"0","createdAt":1791641430000,"updatedAt":1791641450000},
        {"symbol":"vBTC_vUSDC","orderID":1290700003,"clOrdID":"partial-cancel-1","side":"BUY","type":"LIMIT","timeInForce":"GTC","price":"40000","origQty":"0.001","status":"PARTIALLY_FILLED","executedQty":"0.0004","executedValue":"16","marginFrozen":"24","createdAt":1791641430000,"updatedAt":1791641440000},
        {"symbol":"vBTC_vUSDC","orderID":1290700003,"clOrdID":"partial-cancel-1","side":"BUY","type":"LIMIT","timeInForce":"GTC","price":"40000","origQty":"0.001","status":"NEW","executedQty":"0","executedValue":"0","marginFrozen":"40","createdAt":1791641430000,"updatedAt":1791641430000},
        {"symbol":"vBTC_vUSDC","orderID":1290700002,"clOrdID":"partial-open-1","side":"BUY","type":"LIMIT","timeInForce":"GTC","price":"40000","origQty":"0.001","status":"PARTIALLY_FILLED","executedQty":"0.0004","executedValue":"16","marginFrozen":"24","createdAt":1791641410000,"updatedAt":1791641420000},
        {"symbol":"vBTC_vUSDC","orderID":1290700002,"clOrdID":"partial-open-1","side":"BUY","type":"LIMIT","timeInForce":"GTC","price":"40000","origQty":"0.001","status":"NEW","executedQty":"0","executedValue":"0","marginFrozen":"40","createdAt":1791641410000,"updatedAt":1791641410000},
        {"symbol":"vBTC_vUSDC","orderID":1290700001,"clOrdID":"resting-1","side":"BUY","type":"LIMIT","timeInForce":"GTC","price":"40000","origQty":"0.001","status":"NEW","executedQty":"0","executedValue":"0","marginFrozen":"40","createdAt":1791641400000,"updatedAt":1791641400000}
    ]"#;

    fn open(json: &str) -> Vec<OrderRecord> {
        serde_json::from_str::<OpenOrders>(json)
            .expect("the open-list fixture must parse")
            .orders
    }

    fn history(json: &str) -> Vec<OrderRecord> {
        serde_json::from_str(json).expect("the history fixture must parse")
    }

    /// Feeds the selected records to the engine's own mass status, which keeps one report per
    /// venue order id - the last one it is given - exactly as reconciliation receives them.
    fn reconcile(open: &[OrderRecord], history: &[OrderRecord]) -> ExecutionMassStatus {
        let reports = current_orders(open, history)
            .into_iter()
            .map(|record| {
                order_status_report(
                    record,
                    AccountId::from("SODEX_SPOT-60366"),
                    InstrumentId::new(Symbol::from("vBTC_vUSDC"), Venue::from(SODEX_SPOT)),
                    2,
                    5,
                    UnixNanos::default(),
                )
                .expect("every fixture record must convert")
            })
            .collect();

        let mut mass_status = ExecutionMassStatus::new(
            ClientId::from("SODEX_SPOT"),
            AccountId::from("SODEX_SPOT-60366"),
            Venue::from(SODEX_SPOT),
            UnixNanos::default(),
            None,
        );
        mass_status.add_order_reports(reports);
        mass_status
    }

    /// The status the engine will reconcile each order to, by client order id.
    fn status_of(mass_status: &ExecutionMassStatus, cl_ord_id: &str) -> NautilusStatus {
        mass_status
            .order_reports()
            .values()
            .find(|report| {
                report.client_order_id.map(|id| id.to_string()) == Some(cl_ord_id.into())
            })
            .unwrap_or_else(|| panic!("{cl_ord_id} must be reported"))
            .order_status
    }

    /// How many orders reconciliation will initialize as open, by the engine's own predicate.
    fn open_count(mass_status: &ExecutionMassStatus) -> usize {
        mass_status
            .order_reports()
            .values()
            .filter(|report| report.order_status.is_open())
            .count()
    }

    #[rstest]
    fn closed_history_reconciles_to_no_open_order() {
        // The incident: with nothing open at the venue, five long-closed orders came back as
        // accepted, and 200.003 vUSDC - the sum of their original `price * quantity` - as locked.
        let mass_status = reconcile(&open(NO_OPEN_ORDERS), &history(HISTORY));

        assert_eq!(mass_status.order_reports().len(), 7);
        assert_eq!(open_count(&mass_status), 0);
    }

    #[rstest]
    fn each_order_reconciles_to_its_final_state() {
        let mass_status = reconcile(&open(NO_OPEN_ORDERS), &history(HISTORY));

        for cancelled in [
            "modprobe-1789366562193",
            "modprobe-1789366491054",
            "probe-1789366226975",
            "probe-1789366224603",
            "probe-1789040198677",
        ] {
            assert_eq!(
                status_of(&mass_status, cancelled),
                NautilusStatus::Canceled,
                "{cancelled}"
            );
        }
        for filled in ["fillprobe-1789115999154", "fillprobe-1789108466447"] {
            assert_eq!(
                status_of(&mass_status, filled),
                NautilusStatus::Filled,
                "{filled}"
            );
        }
    }

    #[rstest]
    fn a_modified_order_reports_its_last_price() {
        // The last revision carries the modified price; the first carries the one it was placed
        // at. Reporting the first would describe an order the venue no longer has.
        let mass_status = reconcile(&open(NO_OPEN_ORDERS), &history(HISTORY));

        let report = mass_status
            .order_reports()
            .get(&VenueOrderId::new("1290675050"))
            .cloned()
            .expect("the modified order must be reported");
        assert_eq!(
            report.price.map(|price| price.to_string()),
            Some("39000.00".to_string())
        );
    }

    #[rstest]
    fn only_orders_on_the_open_list_reconcile_as_open() {
        let mass_status = reconcile(&open(TWO_OPEN_ORDERS), &history(HISTORY_WITH_LIVE_ORDERS));

        assert_eq!(mass_status.order_reports().len(), 3);
        assert_eq!(open_count(&mass_status), 2);
        assert_eq!(
            status_of(&mass_status, "resting-1"),
            NautilusStatus::Accepted
        );
        assert_eq!(
            status_of(&mass_status, "partial-open-1"),
            NautilusStatus::PartiallyFilled
        );
    }

    #[rstest]
    fn a_partial_fill_then_cancel_reconciles_as_cancelled_with_its_fill() {
        let mass_status = reconcile(&open(TWO_OPEN_ORDERS), &history(HISTORY_WITH_LIVE_ORDERS));

        let report = mass_status
            .order_reports()
            .get(&VenueOrderId::new("1290700003"))
            .cloned()
            .expect("the cancelled order must be reported");
        assert_eq!(report.order_status, NautilusStatus::Canceled);
        assert_eq!(report.filled_qty.to_string(), "0.00040");
        assert_eq!(report.avg_px, Some(Decimal::from(40_000)));
    }

    #[rstest]
    fn a_working_revision_absent_from_the_open_list_is_not_reported_open() {
        // History whose newest row for an order still reads `NEW`, while the open list does not
        // hold it. The open list is the venue's statement of what is working, so the order is
        // left out rather than reported as resting - which would book its notional as locked and
        // send a cancel the venue answers with `OrderNotFound`.
        let stale = history(HISTORY_WITH_LIVE_ORDERS);

        let mass_status = reconcile(&open(NO_OPEN_ORDERS), &stale);

        assert_eq!(open_count(&mass_status), 0);
        assert!(
            mass_status
                .order_reports()
                .get(&VenueOrderId::new("1290700001"))
                .is_none()
        );
        assert_eq!(
            status_of(&mass_status, "partial-cancel-1"),
            NautilusStatus::Canceled
        );
    }

    #[rstest]
    fn an_order_closed_between_the_two_reads_reports_closed() {
        // The open list is read first. An order cancelled before history is read is on both, and
        // the later revision is the one that holds.
        let mut cancelled = open(TWO_OPEN_ORDERS);
        cancelled.truncate(1);
        let mut later = cancelled[0].clone();
        later.status = OrderStatus::Canceled;
        later.updated_at_ms += 1;

        let mass_status = reconcile(&cancelled, &[later]);

        assert_eq!(
            status_of(&mass_status, "resting-1"),
            NautilusStatus::Canceled
        );
        assert_eq!(open_count(&mass_status), 0);
    }

    #[rstest]
    #[case::terminal_row_first(true)]
    #[case::terminal_row_last(false)]
    fn revisions_stamped_the_same_millisecond_resolve_to_the_later_stage(
        #[case] terminal_first: bool,
    ) {
        // A limit order that crosses on arrival can be accepted and filled within one
        // millisecond. Row order is not relied on to separate them; the lifecycle stage is.
        let mut placed = history(HISTORY_WITH_LIVE_ORDERS)
            .into_iter()
            .find(|record| record.cl_ord_id == "resting-1")
            .expect("fixture holds the resting order");
        placed.order_id = 1_290_700_009;
        placed.cl_ord_id = "crossed-1".to_string();
        let mut filled = placed.clone();
        filled.status = OrderStatus::Filled;
        filled.executed_qty = "0.001".to_string();
        filled.executed_value = "40".to_string();

        let rows = if terminal_first {
            vec![filled, placed]
        } else {
            vec![placed, filled]
        };

        let mass_status = reconcile(&[], &rows);

        assert_eq!(status_of(&mass_status, "crossed-1"), NautilusStatus::Filled);
    }
}
