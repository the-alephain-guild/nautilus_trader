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

//! Ticker snapshots, and the perps statistics that only arrive in them.
//!
//! `GET /markets/tickers` is where this venue keeps the funding rate, the mark price and the index
//! price. There is no stream for any of them, so they are polled - and a perps position cannot be
//! valued without the mark price, nor its carry cost known without the funding rate, which makes
//! this the difference between running perps and guessing at it.
//!
//! Two shapes from one path: the perps response carries `fundingRate`, `nextFundingTime`,
//! `indexPrice`, `markPrice` and `openInterest`, and the spot response carries none of the five.
//! They are optional here for the same reason the balance read needed it - a required field that
//! one engine never sends fails the whole read on that engine.
//!
//! The response lists only symbols that have traded, so an instrument's absence means no activity
//! rather than no such instrument. Callers are told which it is rather than receiving an empty
//! answer that reads like either.

use std::collections::HashMap;

use nautilus_core::UnixNanos;
use nautilus_model::{
    data::{FundingRateUpdate, IndexPriceUpdate, MarkPriceUpdate},
    identifiers::InstrumentId,
};
use rust_decimal::Decimal;
use serde::Deserialize;
use thiserror::Error;

use super::parse::price_at;
use crate::http::{ClientError, SodexHttpClient};

/// Failure reading or converting a ticker.
#[derive(Debug, Error)]
pub enum TickerError {
    #[error("request failed: {0}")]
    Transport(String),
    /// The venue lists only symbols that have traded, so this is a quiet market rather than a bad
    /// symbol - and saying so beats returning nothing, which reads like either.
    #[error("{instrument_id} is absent from the ticker list, which carries only traded symbols")]
    NotListed { instrument_id: InstrumentId },
    /// The field exists on perps and never on spot.
    #[error("{field} is absent from this ticker; it is a perpetuals-only statistic")]
    PerpsOnly { field: &'static str },
    #[error("{field} could not be read from {value:?}: {reason}")]
    InvalidValue {
        field: &'static str,
        value: String,
        reason: String,
    },
}

/// One ticker row, as the venue serves it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct RpcTicker {
    pub symbol: String,
    #[serde(rename = "lastPx")]
    pub last_price: String,
    #[serde(rename = "lastSz")]
    pub last_size: String,
    #[serde(rename = "bidPx")]
    pub bid_price: String,
    #[serde(rename = "bidSz")]
    pub bid_size: String,
    #[serde(rename = "askPx")]
    pub ask_price: String,
    #[serde(rename = "askSz")]
    pub ask_size: String,
    #[serde(rename = "openPx")]
    pub open_price: String,
    #[serde(rename = "highPx")]
    pub high_price: String,
    #[serde(rename = "lowPx")]
    pub low_price: String,
    pub volume: String,
    #[serde(rename = "quoteVolume")]
    pub quote_volume: String,
    pub vwap: String,
    /// Start of the statistics window, milliseconds.
    #[serde(rename = "openTime")]
    pub open_time_ms: u64,
    /// End of the statistics window, milliseconds. Also the freshest time this row describes.
    #[serde(rename = "closeTime")]
    pub close_time_ms: u64,
    /// Perps only: the current funding rate, as a fraction per interval.
    #[serde(rename = "fundingRate")]
    pub funding_rate: Option<String>,
    /// Perps only: when the next funding payment falls due, milliseconds.
    #[serde(rename = "nextFundingTime")]
    pub next_funding_time_ms: Option<u64>,
    /// Perps only: the price positions are marked at, which is not the last traded price.
    #[serde(rename = "markPrice")]
    pub mark_price: Option<String>,
    /// Perps only.
    #[serde(rename = "indexPrice")]
    pub index_price: Option<String>,
    /// Perps only.
    #[serde(rename = "openInterest")]
    pub open_interest: Option<String>,
}

/// Fetches tickers, for one symbol or for every traded one.
///
/// The venue honours `?symbol=`, so a per-instrument caller transfers one row rather than filtering
/// the whole list locally.
///
/// # Errors
///
/// Returns [`TickerError::Transport`] if the request fails.
pub async fn fetch_tickers(
    client: &SodexHttpClient,
    symbol: Option<&str>,
) -> Result<Vec<RpcTicker>, TickerError> {
    let mut params: HashMap<String, Vec<String>> = HashMap::new();
    if let Some(symbol) = symbol {
        params.insert("symbol".to_string(), vec![symbol.to_string()]);
    }
    let params = (!params.is_empty()).then_some(params);

    client
        .get_public("/markets/tickers", params.as_ref())
        .await
        .map_err(|e: ClientError| TickerError::Transport(e.to_string()))
}

/// Finds one instrument's ticker.
///
/// # Errors
///
/// Returns [`TickerError::NotListed`] when the instrument has not traded, which the venue expresses
/// by omitting it.
pub fn ticker_for(
    tickers: Vec<RpcTicker>,
    instrument_id: InstrumentId,
) -> Result<RpcTicker, TickerError> {
    let symbol = instrument_id.symbol.as_str();
    tickers
        .into_iter()
        .find(|t| t.symbol == symbol)
        .ok_or(TickerError::NotListed { instrument_id })
}

fn decimal_field(raw: &str, field: &'static str) -> Result<Decimal, TickerError> {
    raw.parse::<Decimal>()
        .map_err(|e| TickerError::InvalidValue {
            field,
            value: raw.to_string(),
            reason: e.to_string(),
        })
}

/// Converts the funding statistics of a perps ticker.
///
/// # Errors
///
/// Returns [`TickerError::PerpsOnly`] on a spot ticker, or [`TickerError::InvalidValue`] if the rate
/// cannot be read.
pub fn parse_funding_rate(
    ticker: &RpcTicker,
    instrument_id: InstrumentId,
    ts_init: UnixNanos,
) -> Result<FundingRateUpdate, TickerError> {
    let raw = ticker
        .funding_rate
        .as_deref()
        .ok_or(TickerError::PerpsOnly {
            field: "fundingRate",
        })?;

    Ok(FundingRateUpdate::new(
        instrument_id,
        decimal_field(raw, "fundingRate")?,
        // The interval is a property of the instrument, not of this row, so it is left to the
        // caller that holds the instrument rather than guessed from the next funding time.
        None,
        ticker
            .next_funding_time_ms
            .map(|ms| UnixNanos::from(ms * 1_000_000)),
        UnixNanos::from(ticker.close_time_ms * 1_000_000),
        ts_init,
    ))
}

/// Converts the mark price of a perps ticker.
///
/// # Errors
///
/// Returns [`TickerError::PerpsOnly`] on a spot ticker, or [`TickerError::InvalidValue`] if the
/// price cannot be read at `price_precision`.
pub fn parse_mark_price(
    ticker: &RpcTicker,
    instrument_id: InstrumentId,
    price_precision: u8,
    ts_init: UnixNanos,
) -> Result<MarkPriceUpdate, TickerError> {
    let raw = ticker
        .mark_price
        .as_deref()
        .ok_or(TickerError::PerpsOnly { field: "markPrice" })?;
    let price =
        price_at(raw, price_precision, "markPrice").map_err(|e| TickerError::InvalidValue {
            field: "markPrice",
            value: raw.to_string(),
            reason: e.to_string(),
        })?;

    Ok(MarkPriceUpdate::new(
        instrument_id,
        price,
        UnixNanos::from(ticker.close_time_ms * 1_000_000),
        ts_init,
    ))
}

/// Converts the index price of a perps ticker.
///
/// # Errors
///
/// Returns [`TickerError::PerpsOnly`] on a spot ticker, or [`TickerError::InvalidValue`] if the
/// price cannot be read at `price_precision`.
pub fn parse_index_price(
    ticker: &RpcTicker,
    instrument_id: InstrumentId,
    price_precision: u8,
    ts_init: UnixNanos,
) -> Result<IndexPriceUpdate, TickerError> {
    let raw = ticker
        .index_price
        .as_deref()
        .ok_or(TickerError::PerpsOnly {
            field: "indexPrice",
        })?;
    let price =
        price_at(raw, price_precision, "indexPrice").map_err(|e| TickerError::InvalidValue {
            field: "indexPrice",
            value: raw.to_string(),
            reason: e.to_string(),
        })?;

    Ok(IndexPriceUpdate::new(
        instrument_id,
        price,
        UnixNanos::from(ticker.close_time_ms * 1_000_000),
        ts_init,
    ))
}

#[cfg(test)]
mod tests {
    use nautilus_model::identifiers::{Symbol, Venue};
    use rstest::rstest;

    use super::*;
    use crate::config::{SODEX_PERPS, SODEX_SPOT};

    /// Verbatim from `/markets/tickers` on the perps engine.
    fn perps_ticker() -> RpcTicker {
        RpcTicker {
            symbol: "BTC-USD".to_string(),
            last_price: "77226".to_string(),
            last_size: "0.00875".to_string(),
            bid_price: "77279".to_string(),
            bid_size: "0.1294".to_string(),
            ask_price: "77295".to_string(),
            ask_size: "0.12937".to_string(),
            open_price: "77326".to_string(),
            high_price: "77425".to_string(),
            low_price: "77128".to_string(),
            volume: "11.54433".to_string(),
            quote_volume: "892219.9153".to_string(),
            vwap: "77286.4181204106258224".to_string(),
            open_time_ms: 1_789_164_521_654,
            close_time_ms: 1_789_250_921_654,
            funding_rate: Some("0.0000125".to_string()),
            next_funding_time_ms: Some(1_789_268_400_000),
            mark_price: Some("77284".to_string()),
            index_price: Some("77312".to_string()),
            open_interest: Some("185.4302".to_string()),
        }
    }

    /// The same path on the spot engine, which omits all five perps statistics.
    fn spot_ticker() -> RpcTicker {
        RpcTicker {
            symbol: "vBTC_vUSDC".to_string(),
            funding_rate: None,
            next_funding_time_ms: None,
            mark_price: None,
            index_price: None,
            open_interest: None,
            ..perps_ticker()
        }
    }

    fn perps_instrument() -> InstrumentId {
        InstrumentId::new(Symbol::from("BTC-USD"), Venue::from(SODEX_PERPS))
    }

    #[rstest]
    fn the_funding_rate_and_its_due_time_come_through() {
        let update =
            parse_funding_rate(&perps_ticker(), perps_instrument(), UnixNanos::from(1)).unwrap();

        assert_eq!(update.rate, "0.0000125".parse::<Decimal>().unwrap());
        assert_eq!(
            update.next_funding_ns,
            Some(UnixNanos::from(1_789_268_400_000_000_000))
        );
        assert_eq!(update.ts_event, UnixNanos::from(1_789_250_921_654_000_000));
    }

    /// The mark price is not the last traded price, and a position valued at the wrong one reports
    /// the wrong unrealized P&L and the wrong distance to liquidation.
    #[rstest]
    fn the_mark_price_is_its_own_value() {
        let ticker = perps_ticker();
        let update = parse_mark_price(&ticker, perps_instrument(), 0, UnixNanos::from(1)).unwrap();

        assert_eq!(update.value.as_f64(), 77284.0);
        assert_ne!(update.value.to_string(), ticker.last_price);
    }

    #[rstest]
    fn the_index_price_is_carried_separately() {
        let update =
            parse_index_price(&perps_ticker(), perps_instrument(), 0, UnixNanos::from(1)).unwrap();

        assert_eq!(update.value.as_f64(), 77312.0);
    }

    /// Spot omits the five perps statistics, so asking for one says which rather than returning a
    /// zero that would be indistinguishable from a real zero rate.
    #[rstest]
    fn a_spot_ticker_refuses_the_perps_statistics() {
        let instrument = InstrumentId::new(Symbol::from("vBTC_vUSDC"), Venue::from(SODEX_SPOT));
        let ticker = spot_ticker();

        for result in [
            parse_funding_rate(&ticker, instrument, UnixNanos::from(1)).err(),
            parse_mark_price(&ticker, instrument, 2, UnixNanos::from(1)).err(),
            parse_index_price(&ticker, instrument, 2, UnixNanos::from(1)).err(),
        ] {
            assert!(matches!(result, Some(TickerError::PerpsOnly { .. })));
        }
    }

    /// The venue lists only traded symbols, so a quiet instrument is reported as unlisted rather
    /// than as an empty success.
    #[rstest]
    fn an_untraded_instrument_is_named_rather_than_silently_missing() {
        let absent = InstrumentId::new(Symbol::from("DOGE-USD"), Venue::from(SODEX_PERPS));

        let error = ticker_for(vec![perps_ticker()], absent).unwrap_err();

        assert!(matches!(error, TickerError::NotListed { .. }));
    }

    #[rstest]
    fn the_requested_instrument_is_found_among_several() {
        let mut other = perps_ticker();
        other.symbol = "ETH-USD".to_string();

        let found = ticker_for(vec![other, perps_ticker()], perps_instrument()).unwrap();

        assert_eq!(found.symbol, "BTC-USD");
    }

    #[rstest]
    fn an_unreadable_rate_fails_rather_than_defaulting() {
        let mut ticker = perps_ticker();
        ticker.funding_rate = Some("not-a-rate".to_string());

        let error =
            parse_funding_rate(&ticker, perps_instrument(), UnixNanos::from(1)).unwrap_err();

        assert!(matches!(error, TickerError::InvalidValue { .. }));
    }
}
