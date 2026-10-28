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

//! Closed external reference candles, with complete bounded daily windows.

use crate::{api::PapertradeApi, reports, wire::Market};
use anyhow::{Result, ensure};
use nautilus_core::UnixNanos;
use nautilus_model::{
    data::{Bar, BarType},
    enums::{BarAggregation, PriceType},
    instruments::Instrument,
    types::{Price, Quantity, fixed::FIXED_PRECISION},
};
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub const REFERENCE_DAY_MS: u64 = 86_400_000;
const MAX_DAILY_BARS: u64 = 5000;

/// A half-open range of completed UTC days, within the provider's retention bound.
#[derive(Debug, Clone, Copy)]
pub struct ReferenceDailyWindow {
    start_ms: u64,
    end_ms: u64,
}
impl ReferenceDailyWindow {
    /// Validate boundaries without claiming that the provider actually covers them.
    pub fn new(start_ms: u64, end_ms: u64, now_ms: u64) -> Result<Self> {
        let today = now_ms / REFERENCE_DAY_MS * REFERENCE_DAY_MS;
        ensure!(
            start_ms.is_multiple_of(REFERENCE_DAY_MS) && end_ms.is_multiple_of(REFERENCE_DAY_MS),
            "Daily window must use UTC boundaries"
        );
        ensure!(
            start_ms < end_ms && end_ms <= today,
            "Daily window must contain only closed days"
        );
        ensure!(
            (end_ms - start_ms) / REFERENCE_DAY_MS <= MAX_DAILY_BARS
                && start_ms >= today.saturating_sub(MAX_DAILY_BARS * REFERENCE_DAY_MS),
            "Daily window exceeds the reference retention boundary"
        );
        ensure!(
            now_ms.checked_mul(1_000_000).is_some(),
            "Daily source time exceeds Native nanoseconds"
        );
        Ok(Self { start_ms, end_ms })
    }
    #[must_use]
    pub fn start_ms(self) -> u64 {
        self.start_ms
    }
    #[must_use]
    pub fn end_ms(self) -> u64 {
        self.end_ms
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReferenceCandle {
    t: u64,
    #[serde(rename = "T")]
    end_ms: u64,
    s: String,
    i: String,
    o: String,
    h: String,
    l: String,
    c: String,
    v: String,
    #[serde(rename = "n")]
    _trade_count: u64,
}

pub(crate) fn validate_daily_bar_type(market: &Market, bar_type: BarType) -> Result<()> {
    let spec = bar_type.spec();
    ensure!(
        !market.terminal,
        "Terminal market cannot supply strategy reference bars"
    );
    ensure!(
        bar_type.instrument_id() == reports::instrument_id(market)
            && bar_type.is_externally_aggregated()
            && spec.aggregation == BarAggregation::Day
            && spec.step.get() == 1
            && spec.price_type == PriceType::Last,
        "Reference bars require this market's 1-DAY-LAST-EXTERNAL type"
    );
    Ok(())
}

fn price(value: &str, precision: u8) -> Result<Price> {
    let number = Decimal::from_str_exact(value)?;
    ensure!(
        number > Decimal::ZERO && number.round_dp(u32::from(precision)) == number,
        "Reference price does not fit the market precision"
    );
    Ok(Price::from_decimal_dp(number, precision)?)
}

/// Require every requested day exactly once; timestamps match the reference adapter's
/// convention of open time for `ts_event` and exclusive close boundary for `ts_init`.
pub fn decode_daily_bars(
    value: &Value,
    market: &Market,
    bar_type: BarType,
    window: ReferenceDailyWindow,
) -> Result<Vec<Bar>> {
    validate_daily_bar_type(market, bar_type)?;
    let rows = value
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("Reference candles must be an array"))?;
    let expected = (window.end_ms - window.start_ms) / REFERENCE_DAY_MS;
    ensure!(
        rows.len() == usize::try_from(expected)?,
        "Reference daily window is incomplete or overlong"
    );
    let instrument = reports::instrument(market, UnixNanos::from(window.end_ms * 1_000_000))?;
    let precision = instrument.price_precision();
    let mut ordered = BTreeMap::new();
    for row in rows {
        let candle: ReferenceCandle = serde_json::from_value(row.clone())?;
        ensure!(
            candle.s == market.symbol && candle.i == "1d",
            "Reference candle market or interval differs"
        );
        ensure!(
            candle.t.is_multiple_of(REFERENCE_DAY_MS)
                && candle.t >= window.start_ms
                && candle.t < window.end_ms
                && candle.end_ms
                    == candle
                        .t
                        .checked_add(REFERENCE_DAY_MS - 1)
                        .ok_or_else(|| anyhow::anyhow!("Reference close boundary overflow"))?,
            "Reference candle time range is invalid"
        );
        let volume = Decimal::from_str_exact(&candle.v)?;
        ensure!(
            volume >= Decimal::ZERO && volume.round_dp(u32::from(FIXED_PRECISION)) == volume,
            "Reference volume does not fit Native precision"
        );
        let bar = Bar::new_checked(
            bar_type,
            price(&candle.o, precision)?,
            price(&candle.h, precision)?,
            price(&candle.l, precision)?,
            price(&candle.c, precision)?,
            Quantity::from_decimal_dp(volume, FIXED_PRECISION)?,
            UnixNanos::from(candle.t * 1_000_000),
            UnixNanos::from((candle.end_ms + 1) * 1_000_000),
        )?;
        ensure!(
            ordered.insert(candle.t, bar).is_none(),
            "Reference daily window contains duplicate days"
        );
    }
    for (index, stamp) in ordered.keys().enumerate() {
        ensure!(
            *stamp == window.start_ms + u64::try_from(index)? * REFERENCE_DAY_MS,
            "Reference daily window contains a gap"
        );
    }
    Ok(ordered.into_values().collect())
}

/// Fetch reference OHLCV, never Papertrade fills or its BBO-mid settlement history.
pub async fn fetch_daily_bars(
    api: &PapertradeApi,
    market: &Market,
    bar_type: BarType,
    window: ReferenceDailyWindow,
) -> Result<Vec<Bar>> {
    validate_daily_bar_type(market, bar_type)?;
    let response=api.reference_request(&json!({"type":"candleSnapshot","req":{"coin":market.symbol,"interval":"1d","startTime":window.start_ms,"endTime":window.end_ms-1}})).await?;
    decode_daily_bars(&response, market, bar_type, window)
}
