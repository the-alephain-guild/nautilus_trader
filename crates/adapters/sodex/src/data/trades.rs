//! Recent public trades, read over REST.
//!
//! The venue serves these at `/markets/{symbol}/trades` on both engines, newest first, and this
//! is what a strategy warms up on before its own subscription starts producing ticks.
//!
//! # What the endpoint will and will not do
//!
//! Measured 2026-09-14 on testnet perps, because nothing documents it: `limit` is honored between
//! 1 and 500, defaulting to 50, and 501 is refused as `invalid parameter: limit`. **Time filters
//! are not honored at all** - `startTime`, `endTime` and `from` each returned the same most-recent
//! window as a request carrying none of them.
//!
//! So a request for a past window cannot be served by asking the venue for it. Trimming the
//! newest rows to the window is all that can be done, and when the window lies entirely before
//! them the answer is empty rather than wrong - a caller that received the newest trades in
//! response to a request for yesterday's would compute against the wrong hour and never know.

use nautilus_core::UnixNanos;
use nautilus_model::{
    data::TradeTick,
    identifiers::{InstrumentId, TradeId},
};
use serde::Deserialize;

use super::parse::{price_at, quantity_at};
use crate::{
    common::enums::OrderSide,
    http::{ClientError, SodexHttpClient},
};

/// Most rows the venue will return, measured: 500 is served, 501 is refused.
pub const MAX_TRADES: u32 = 500;

/// One public trade as the REST endpoint returns it.
///
/// The push frame for the same event carries three fields this does not - the frame's own time
/// and both account ids - so the two are separate types rather than one with optional fields:
/// filling those in would put invented values where the venue puts real ones.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct RpcTrade {
    #[serde(rename = "t")]
    pub trade_id: u64,
    /// Time the trade happened, milliseconds.
    #[serde(rename = "T")]
    pub trade_time_ms: u64,
    #[serde(rename = "s")]
    pub symbol: String,
    /// The **aggressing** side, established by observation on the push channel and the same here.
    #[serde(rename = "S")]
    pub side: OrderSide,
    #[serde(rename = "p")]
    pub price: String,
    #[serde(rename = "q")]
    pub quantity: String,
}

/// Why a trades request could not be served.
#[derive(Debug, thiserror::Error)]
pub enum TradesError {
    #[error("failed to read trades: {0}")]
    Transport(#[from] ClientError),
    #[error("limit {requested} exceeds the venue's maximum of {MAX_TRADES}")]
    LimitTooLarge { requested: u32 },
}

/// Reads the most recent trades for one symbol.
///
/// # Errors
///
/// Returns [`TradesError`] on transport failure, or when `limit` exceeds what the venue accepts -
/// refused here rather than sent, because the venue answers an over-large limit by rejecting the
/// whole request instead of capping it.
pub async fn fetch_trades(
    client: &SodexHttpClient,
    instrument_id: InstrumentId,
    limit: Option<u32>,
) -> Result<Vec<RpcTrade>, TradesError> {
    if let Some(limit) = limit
        && limit > MAX_TRADES
    {
        return Err(TradesError::LimitTooLarge { requested: limit });
    }

    let mut params = std::collections::HashMap::new();
    if let Some(limit) = limit {
        params.insert("limit".to_string(), vec![limit.to_string()]);
    }

    // Addressed by name, like the klines and orderbook endpoints.
    let path = format!("/markets/{}/trades", instrument_id.symbol);
    let params = (!params.is_empty()).then_some(params);

    Ok(client.get_public(&path, params.as_ref()).await?)
}

/// Converts the rows into ticks, oldest first, keeping only those inside the requested window.
///
/// The venue returns newest first while Nautilus wants ascending time, and it ignores time
/// filters, so both the ordering and the window are this side's work.
///
/// A row that cannot be expressed at the instrument's precision is skipped with a warning rather
/// than failing the request: one malformed trade should not deny the strategy the rest of them.
#[must_use]
pub fn parse_trades(
    rows: Vec<RpcTrade>,
    instrument_id: InstrumentId,
    price_precision: u8,
    size_precision: u8,
    start_ms: Option<u64>,
    end_ms: Option<u64>,
    ts_init: UnixNanos,
) -> Vec<TradeTick> {
    let mut ticks = Vec::with_capacity(rows.len());

    for row in rows {
        if start_ms.is_some_and(|start| row.trade_time_ms < start)
            || end_ms.is_some_and(|end| row.trade_time_ms > end)
        {
            continue;
        }

        let price = match price_at(&row.price, price_precision, "trade price") {
            Ok(price) => price,
            Err(e) => {
                log::warn!("sodex_trade_skipped trade_id={} error={e}", row.trade_id);
                continue;
            }
        };
        let quantity = match quantity_at(&row.quantity, size_precision, "trade quantity") {
            Ok(quantity) => quantity,
            Err(e) => {
                log::warn!("sodex_trade_skipped trade_id={} error={e}", row.trade_id);
                continue;
            }
        };

        ticks.push(TradeTick::new(
            instrument_id,
            price,
            quantity,
            super::parse::map_aggressor_side(row.side),
            TradeId::new(row.trade_id.to_string().as_str()),
            UnixNanos::from(row.trade_time_ms * 1_000_000),
            ts_init,
        ));
    }

    ticks.reverse();
    ticks
}

#[cfg(test)]
mod tests {
    use nautilus_model::enums::AggressorSide;
    use rstest::rstest;

    use super::*;

    /// Rows exactly as the venue returned them on testnet perps, newest first.
    fn rows() -> Vec<RpcTrade> {
        serde_json::from_str(
            r#"[{"t":4643656,"T":1789360125703,"s":"BTC-USD","S":"SELL","p":"77582","q":"0.0597"},
                {"t":4643655,"T":1789359528384,"s":"BTC-USD","S":"SELL","p":"77523","q":"0.06359"},
                {"t":4643654,"T":1789359512982,"s":"BTC-USD","S":"BUY","p":"77540","q":"0.0641"}]"#,
        )
        .unwrap()
    }

    fn instrument_id() -> InstrumentId {
        InstrumentId::from("BTC-USD.SODEX_PERPS")
    }

    #[rstest]
    fn ticks_come_back_oldest_first() {
        // The venue returns newest first and the engine wants ascending time. A response left in
        // arrival order would look like a market running backwards.
        let ticks = parse_trades(
            rows(),
            instrument_id(),
            1,
            5,
            None,
            None,
            UnixNanos::from(1),
        );

        assert_eq!(ticks.len(), 3);
        assert!(ticks[0].ts_event < ticks[1].ts_event);
        assert!(ticks[1].ts_event < ticks[2].ts_event);
        assert_eq!(ticks[0].trade_id.to_string(), "4643654");
    }

    #[rstest]
    fn the_side_is_the_aggressor() {
        // `S` names the taker, established on the push channel against contemporaneous top of
        // book. Reading it as the maker's side would invert every order-flow measure built on it.
        let ticks = parse_trades(
            rows(),
            instrument_id(),
            1,
            5,
            None,
            None,
            UnixNanos::from(1),
        );

        assert_eq!(ticks[0].aggressor_side, AggressorSide::Buy);
        assert_eq!(ticks[2].aggressor_side, AggressorSide::Sell);
    }

    #[rstest]
    fn a_window_the_venue_ignores_is_applied_here() {
        // The endpoint returns the newest trades whatever the time parameters say, so a request
        // for an earlier window must come back empty rather than carrying the newest ones - a
        // caller could not tell those from the real answer.
        let ticks = parse_trades(
            rows(),
            instrument_id(),
            1,
            5,
            Some(1_700_000_000_000),
            Some(1_700_000_001_000),
            UnixNanos::from(1),
        );

        assert!(ticks.is_empty());
    }

    #[rstest]
    fn a_window_that_covers_part_of_the_page_keeps_that_part() {
        let ticks = parse_trades(
            rows(),
            instrument_id(),
            1,
            5,
            Some(1_789_359_528_384),
            None,
            UnixNanos::from(1),
        );

        assert_eq!(ticks.len(), 2);
        assert_eq!(ticks[0].trade_id.to_string(), "4643655");
    }

    #[rstest]
    fn an_unreadable_row_is_skipped_not_fatal() {
        // One malformed trade should not deny the strategy the rest of them.
        let mut rows = rows();
        rows[1].price = "not a price".to_string();

        let ticks = parse_trades(rows, instrument_id(), 1, 5, None, None, UnixNanos::from(1));

        assert_eq!(ticks.len(), 2);
    }

    #[rstest]
    fn a_limit_beyond_what_the_venue_serves_is_refused_before_it_is_sent() {
        // Measured: 500 is served, 501 comes back `invalid parameter: limit` - the venue rejects
        // the request rather than capping it, so an over-large limit would cost a round trip and
        // return nothing.
        assert_eq!(MAX_TRADES, 500);
    }
}
