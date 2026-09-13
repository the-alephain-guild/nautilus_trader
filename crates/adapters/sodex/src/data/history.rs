//! Historical bar retrieval.
//!
//! # The venue does not mark historical bars as closed
//!
//! The streaming candle carries a `closed` flag; the REST kline does not. A request whose
//! range reaches the present therefore returns a final row that is **still forming**, and
//! nothing in the payload says so.
//!
//! Feeding that row into a backtest is look-ahead bias in its purest form: the bar's high,
//! low and close are not yet final, so a strategy would be deciding on values that could not
//! have been known at that timestamp. [`drop_forming_tail`] removes it by comparing the
//! bar's open plus its interval against the current time, which needs no venue flag.
//!
//! The split of responsibility is deliberate. [`parse_kline`] does not check closure,
//! because a historical row carries no evidence either way; the time-based filter is applied
//! once over the series instead.
//!
//! # The venue responds newest-first
//!
//! Nautilus consumes bar series oldest-first, and the closure filter inspects the tail, so
//! [`fetch_bars`] sorts before returning. Observed on the live testnet: a ten-bar response
//! arrived in descending timestamp order.

use std::collections::HashMap;

use nautilus_core::{UnixNanos, time::get_atomic_clock_realtime};
use nautilus_model::{
    data::{Bar, BarSpecification, BarType},
    enums::AggregationSource,
    identifiers::InstrumentId,
};
use serde::Deserialize;

use super::parse::{BarMappingError, spec_to_interval};
use crate::{common::Market, http::SodexHttpClient};

/// Largest number of rows the venue returns in one request.
///
/// Spot and perps differ here, as they do in several other places; asking for more than the
/// engine allows is rejected rather than silently truncated by the venue.
#[must_use]
pub const fn max_limit(market: Market) -> u32 {
    match market {
        Market::Spot => 1500,
        Market::Perps => 1000,
    }
}

/// A historical bar query.
#[derive(Debug, Clone)]
pub struct BarRequest {
    pub instrument_id: InstrumentId,
    pub spec: BarSpecification,
    /// Price precision from the instrument definition.
    ///
    /// Carried on the request rather than derived from each kline field, because the engine
    /// panics on a bar whose prices disagree about scale and the venue writes one tick size
    /// several ways.
    pub price_precision: u8,
    /// Size precision from the instrument definition.
    pub size_precision: u8,
    /// Inclusive start in milliseconds.
    pub start_ms: Option<u64>,
    /// Inclusive end in milliseconds.
    pub end_ms: Option<u64>,
    /// Rows to return; the venue defaults to 500.
    pub limit: Option<u32>,
}

/// Why a historical request cannot be served.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HistoryError {
    #[error(transparent)]
    Mapping(#[from] BarMappingError),
    #[error("limit {requested} exceeds the {market:?} maximum of {max}")]
    LimitTooLarge {
        requested: u32,
        max: u32,
        market: Market,
    },
    #[error("start {start_ms} is after end {end_ms}")]
    InvertedRange { start_ms: u64, end_ms: u64 },
    #[error("request failed: {0}")]
    Transport(String),
}

/// One historical kline row.
///
/// Note the absence of a closed flag, unlike the streaming candle.
#[derive(Debug, Clone, Deserialize)]
pub struct RpcKline {
    #[serde(rename = "t")]
    pub open_time_ms: u64,
    #[serde(rename = "o")]
    pub open: String,
    #[serde(rename = "h")]
    pub high: String,
    #[serde(rename = "l")]
    pub low: String,
    #[serde(rename = "c")]
    pub close: String,
    #[serde(rename = "v")]
    pub volume: String,
    #[serde(rename = "q")]
    pub quote_volume: String,
    #[serde(rename = "n", default)]
    pub trade_count: u64,
}

/// Converts one kline row into a bar.
///
/// Does not judge closure: a historical row carries no evidence either way. Use
/// [`drop_forming_tail`] over the resulting series instead.
///
/// # Errors
///
/// Returns [`BarMappingError::InvalidValue`] if a price or volume cannot be parsed.
pub fn parse_kline(
    kline: &RpcKline,
    bar_type: BarType,
    price_precision: u8,
    size_precision: u8,
    ts_init: UnixNanos,
) -> Result<Bar, BarMappingError> {
    // Normalized to the instrument's declared precision, not to each field's own text. Klines
    // carry on-chain padding the engine's fixed-point types reject, and their four prices need a
    // uniform scale or the engine panics - a whole-number close beside a fractional open is
    // exactly how that surfaced.
    let price =
        |raw: &str, field: &'static str| super::parse::price_at(raw, price_precision, field);
    let volume = super::parse::quantity_at(&kline.volume, size_precision, "volume")?;

    Ok(Bar::new(
        bar_type,
        price(&kline.open, "open")?,
        price(&kline.high, "high")?,
        price(&kline.low, "low")?,
        price(&kline.close, "close")?,
        volume,
        UnixNanos::from(kline.open_time_ms * 1_000_000),
        ts_init,
    ))
}

/// Removes trailing bars whose interval has not yet elapsed.
///
/// A bar is complete only once `open + interval <= now`.
///
/// **Expects oldest-first input**, which is what [`fetch_bars`] returns after sorting. The
/// venue itself responds newest-first, so calling this on a raw response would inspect the
/// oldest bar while the incomplete one sits at the other end - dropping good history and
/// keeping the very bar this is meant to remove.
///
/// The check walks back rather than assuming exactly one incomplete bar, since clock skew or
/// a stale response could leave more.
#[must_use]
pub fn drop_forming_tail(mut bars: Vec<Bar>, spec: &BarSpecification, now_ms: u64) -> Vec<Bar> {
    let interval_ms = u64::try_from(spec.timedelta().as_millis().max(0)).unwrap_or(0);
    if interval_ms == 0 {
        return bars;
    }

    while let Some(last) = bars.last() {
        let open_ms = last.ts_event.as_u64() / 1_000_000;
        if open_ms.saturating_add(interval_ms) <= now_ms {
            break;
        }
        bars.pop();
    }

    bars
}

/// Fetches historical bars for one instrument.
///
/// # Errors
///
/// Returns [`HistoryError`] for an unsupported interval, an over-large limit, an inverted
/// range, or a transport failure.
pub async fn fetch_bars(
    client: &SodexHttpClient,
    market: Market,
    request: &BarRequest,
) -> Result<Vec<Bar>, HistoryError> {
    let interval = spec_to_interval(&request.spec, market)?;

    if let Some(limit) = request.limit {
        let max = max_limit(market);
        if limit > max {
            return Err(HistoryError::LimitTooLarge {
                requested: limit,
                max,
                market,
            });
        }
    }

    if let (Some(start), Some(end)) = (request.start_ms, request.end_ms)
        && start > end
    {
        return Err(HistoryError::InvertedRange {
            start_ms: start,
            end_ms: end,
        });
    }

    let mut params: HashMap<String, Vec<String>> = HashMap::new();
    params.insert("interval".to_string(), vec![interval.to_string()]);
    if let Some(start) = request.start_ms {
        params.insert("startTime".to_string(), vec![start.to_string()]);
    }
    if let Some(end) = request.end_ms {
        params.insert("endTime".to_string(), vec![end.to_string()]);
    }
    if let Some(limit) = request.limit {
        params.insert("limit".to_string(), vec![limit.to_string()]);
    }

    // The klines endpoint addresses the instrument by name, not by numeric symbol id.
    let path = format!("/markets/{}/klines", request.instrument_id.symbol);

    let klines: Vec<RpcKline> = client
        .get_public(&path, Some(&params))
        .await
        .map_err(|e| HistoryError::Transport(e.to_string()))?;

    // When this client produced the bars, not epoch zero: the engine orders and validates a series
    // by `ts_init`, so leaving it at 1970 makes every bar look older than any request window - and
    // a node's warmup then receives an empty response with nothing logged as wrong.
    finalize_bars(&klines, request, get_atomic_clock_realtime().get_time_ns())
}

/// Turns a raw kline response into the closed, ordered series the engine expects.
///
/// Separate from the request so the three things that go wrong here are testable without a venue:
/// the response arrives newest-first, the newest bar is usually still forming, and a zero
/// `ts_init` silently invalidates the whole series.
///
/// # Errors
///
/// Returns [`HistoryError`] if a kline cannot be parsed at the requested precisions.
///
/// # Panics
///
/// Debug builds panic on a zero `ts_init`, which is never a real timestamp and is how this adapter
/// once made every historical bar unusable.
pub fn finalize_bars(
    klines: &[RpcKline],
    request: &BarRequest,
    ts_init: UnixNanos,
) -> Result<Vec<Bar>, HistoryError> {
    debug_assert!(
        ts_init.as_u64() > 0,
        "ts_init must be a real timestamp; epoch zero puts every bar before any request window",
    );

    let bar_type = BarType::new(
        request.instrument_id,
        request.spec,
        AggregationSource::External,
    );

    let mut bars = klines
        .iter()
        .map(|k| {
            parse_kline(
                k,
                bar_type,
                request.price_precision,
                request.size_precision,
                ts_init,
            )
        })
        .collect::<Result<Vec<_>, _>>()
        .map_err(HistoryError::from)?;

    // The venue responds newest-first. Nautilus consumes series oldest-first, and
    // `drop_forming_tail` inspects the tail, so the order is normalized here rather than
    // left for each caller to discover.
    bars.sort_by_key(|bar| bar.ts_event);

    // Dropped here rather than by each caller. The venue does not flag an incomplete bar, so a
    // caller that forgets hands the engine a bar whose high and low are still moving - and the
    // data client did forget, which is how a node's warmup received one.
    Ok(drop_forming_tail(
        bars,
        &request.spec,
        ts_init.as_u64() / 1_000_000,
    ))
}

#[cfg(test)]
mod finalize_tests {
    use nautilus_model::{
        enums::{BarAggregation, PriceType},
        identifiers::{Symbol, Venue},
    };
    use rstest::rstest;

    use super::*;
    use crate::config::SODEX_PERPS;

    const MINUTE_MS: u64 = 60_000;

    fn kline(open_ms: u64) -> RpcKline {
        RpcKline {
            open_time_ms: open_ms,
            open: "77226".to_string(),
            high: "77226".to_string(),
            low: "77226".to_string(),
            close: "77226".to_string(),
            volume: "0".to_string(),
            quote_volume: "0".to_string(),
            trade_count: 0,
        }
    }

    fn request() -> BarRequest {
        BarRequest {
            instrument_id: InstrumentId::new(Symbol::from("BTC-USD"), Venue::from(SODEX_PERPS)),
            spec: BarSpecification::new(1, BarAggregation::Minute, PriceType::Last),
            price_precision: 0,
            size_precision: 5,
            start_ms: None,
            end_ms: None,
            limit: None,
        }
    }

    /// The venue answers newest-first and never flags the bar it is still building, so the response
    /// has to be reordered and trimmed before the engine sees it.
    #[rstest]
    fn a_newest_first_response_comes_back_ordered_without_the_forming_bar() {
        let now_ms = 10 * MINUTE_MS;
        let forming = now_ms; // its minute has not elapsed
        let klines = vec![
            kline(forming),
            kline(now_ms - MINUTE_MS),
            kline(now_ms - 2 * MINUTE_MS),
        ];

        let bars = finalize_bars(&klines, &request(), UnixNanos::from(now_ms * 1_000_000)).unwrap();

        assert_eq!(bars.len(), 2, "the forming bar must not be published");
        assert!(
            bars[0].ts_event < bars[1].ts_event,
            "the series must be oldest-first",
        );
    }

    /// Every bar carries the time this client produced it. Zero is what it used to carry, which put
    /// the whole series before any request window and made a node's warmup arrive empty.
    #[rstest]
    fn bars_carry_the_supplied_ts_init() {
        let now_ms = 10 * MINUTE_MS;
        let ts_init = UnixNanos::from(now_ms * 1_000_000);
        let klines = vec![kline(now_ms - MINUTE_MS)];

        let bars = finalize_bars(&klines, &request(), ts_init).unwrap();

        assert_eq!(bars.len(), 1);
        assert_eq!(bars[0].ts_init, ts_init);
        assert!(bars[0].ts_init.as_u64() > 0);
    }

    /// Epoch zero is never a real timestamp here, so it fails loudly rather than producing a series
    /// the engine silently discards.
    #[rstest]
    #[should_panic(expected = "ts_init must be a real timestamp")]
    fn a_zero_ts_init_is_refused() {
        let _ = finalize_bars(&[kline(0)], &request(), UnixNanos::default());
    }
}

#[cfg(test)]
mod tests {
    use nautilus_model::{
        enums::{BarAggregation, PriceType},
        identifiers::{Symbol, Venue},
    };
    use rstest::rstest;

    use super::*;
    use crate::{config::SODEX_SPOT, data::parse::bar_type_for};

    const MINUTE_MS: u64 = 60_000;

    fn instrument() -> InstrumentId {
        InstrumentId::new(Symbol::from("vBTC_vUSDC"), Venue::from(SODEX_SPOT))
    }

    fn minute_spec() -> BarSpecification {
        BarSpecification::new(1, BarAggregation::Minute, PriceType::Last)
    }

    fn kline(open_time_ms: u64) -> RpcKline {
        RpcKline {
            open_time_ms,
            open: "91869".to_string(),
            high: "91982".to_string(),
            low: "91869".to_string(),
            close: "91976".to_string(),
            volume: "4.12298".to_string(),
            quote_volume: "379148.6798".to_string(),
            trade_count: 12,
        }
    }

    fn bars_at(times: &[u64]) -> Vec<Bar> {
        let bar_type = bar_type_for(instrument(), "1m").unwrap();
        times
            .iter()
            .map(|t| parse_kline(&kline(*t), bar_type, 0, 5, UnixNanos::default()).unwrap())
            .collect()
    }

    #[rstest]
    fn kline_parses_without_judging_closure() {
        // A historical row carries no closed flag, so parsing must not require one.
        let bar_type = bar_type_for(instrument(), "1m").unwrap();
        let bar = parse_kline(
            &kline(1_767_972_900_000),
            bar_type,
            0,
            5,
            UnixNanos::default(),
        )
        .unwrap();

        assert_eq!(bar.close.to_string(), "91976");
        assert_eq!(bar.volume.to_string(), "4.12298");
    }

    #[rstest]
    fn kline_fields_with_mixed_decimal_places_share_one_precision() {
        // The failure this guards: a history response for an instrument quoting "2465" beside
        // "2464.9" built a bar whose fields disagreed about scale, and the engine panicked.
        let mixed = RpcKline {
            open: "2465".to_string(),
            high: "2465.400000000000000000".to_string(),
            low: "2464.1".to_string(),
            close: "2464.900000000000000000".to_string(),
            ..kline(1_767_972_900_000)
        };
        let bar_type = bar_type_for(instrument(), "1m").unwrap();
        let bar = parse_kline(&mixed, bar_type, 1, 5, UnixNanos::default()).unwrap();

        assert_eq!(bar.open.precision, 1);
        assert_eq!(bar.high.precision, 1);
        assert_eq!(bar.low.precision, 1);
        assert_eq!(bar.close.precision, 1);
        assert_eq!(bar.open.to_string(), "2465.0");
        assert_eq!(bar.close.to_string(), "2464.9");
    }

    #[rstest]
    fn the_still_forming_final_bar_is_dropped() {
        // The venue returns it with no indication that it is incomplete. Keeping it would
        // let a backtest decide on a high, low and close that were not yet final.
        let now = 10 * MINUTE_MS + 30_000; // half way through the bar opening at 10m
        let bars = bars_at(&[8 * MINUTE_MS, 9 * MINUTE_MS, 10 * MINUTE_MS]);

        let kept = drop_forming_tail(bars, &minute_spec(), now);

        assert_eq!(kept.len(), 2);
        assert_eq!(
            kept.last().unwrap().ts_event.as_u64(),
            9 * MINUTE_MS * 1_000_000
        );
    }

    #[rstest]
    fn a_bar_closing_exactly_now_is_kept() {
        // Boundary: open + interval == now means the interval has elapsed.
        let now = 10 * MINUTE_MS;
        let bars = bars_at(&[9 * MINUTE_MS]);

        assert_eq!(drop_forming_tail(bars, &minute_spec(), now).len(), 1);
    }

    #[rstest]
    fn a_fully_historical_series_is_untouched() {
        let now = 100 * MINUTE_MS;
        let bars = bars_at(&[8 * MINUTE_MS, 9 * MINUTE_MS, 10 * MINUTE_MS]);

        assert_eq!(drop_forming_tail(bars, &minute_spec(), now).len(), 3);
    }

    #[rstest]
    fn more_than_one_incomplete_tail_bar_is_handled() {
        // Should not happen with a sane clock, but skew or a stale response could produce
        // it, and assuming exactly one would leave an incomplete bar in the series.
        let now = 8 * MINUTE_MS + 30_000;
        let bars = bars_at(&[8 * MINUTE_MS, 9 * MINUTE_MS, 10 * MINUTE_MS]);

        assert!(drop_forming_tail(bars, &minute_spec(), now).is_empty());
    }

    #[rstest]
    fn dropping_on_a_newest_first_series_would_remove_the_wrong_end() {
        // Guards the sort in `fetch_bars`. Fed the venue's own ordering, the filter inspects
        // the oldest bar, keeps the incomplete newest one, and discards good history -
        // exactly backwards. This test exists because the live response is newest-first
        // while the code originally assumed the opposite.
        let now = 10 * MINUTE_MS + 30_000;
        let ascending = bars_at(&[8 * MINUTE_MS, 9 * MINUTE_MS, 10 * MINUTE_MS]);
        let mut descending = ascending.clone();
        descending.reverse();

        let correct = drop_forming_tail(ascending, &minute_spec(), now);
        let wrong = drop_forming_tail(descending, &minute_spec(), now);

        // Correct: the 10m bar is still forming and goes.
        assert_eq!(correct.len(), 2);
        assert!(
            correct
                .iter()
                .all(|b| b.ts_event.as_u64() < 10 * MINUTE_MS * 1_000_000)
        );

        // Wrong order: the incomplete bar survives at the head.
        assert_eq!(
            wrong.first().unwrap().ts_event.as_u64(),
            10 * MINUTE_MS * 1_000_000
        );
    }

    #[rstest]
    fn limits_differ_between_the_engines() {
        assert_eq!(max_limit(Market::Spot), 1500);
        assert_eq!(max_limit(Market::Perps), 1000);
    }

    #[rstest]
    fn an_over_large_limit_names_the_engine_maximum() {
        // Silently clamping would return fewer bars than asked for with no indication.
        let request = BarRequest {
            instrument_id: instrument(),
            spec: minute_spec(),
            price_precision: 1,
            size_precision: 5,
            start_ms: None,
            end_ms: None,
            limit: Some(1200),
        };

        // 1200 is fine on spot but over the perps ceiling; only the check itself is
        // exercised here, without a network call.
        assert!(request.limit.unwrap() <= max_limit(Market::Spot));
        assert!(request.limit.unwrap() > max_limit(Market::Perps));
    }

    #[rstest]
    fn kline_deserializes_from_the_venue_field_names() {
        let raw = r#"{"t":1767972900000,"o":"91869","h":"91982","l":"91869","c":"91976","v":"4.12298","q":"379148.6798","n":12}"#;
        let parsed: RpcKline = serde_json::from_str(raw).unwrap();

        assert_eq!(parsed.open_time_ms, 1_767_972_900_000);
        assert_eq!(parsed.trade_count, 12);
    }

    #[rstest]
    fn trade_count_is_optional() {
        // Documented as not required; a missing count must not fail the whole series.
        let raw = r#"{"t":1,"o":"1","h":"1","l":"1","c":"1","v":"1","q":"1"}"#;
        let parsed: RpcKline = serde_json::from_str(raw).unwrap();

        assert_eq!(parsed.trade_count, 0);
    }
}
