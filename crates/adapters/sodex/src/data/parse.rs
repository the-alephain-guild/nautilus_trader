//! Conversion between SoDEX market data and Nautilus types.
//!
//! # Bar intervals do not map one to one
//!
//! Three separate restrictions narrow what can be requested, and all three are checked here
//! rather than left to fail at the venue or, worse, to silently return the wrong series:
//!
//! - **Nautilus has no monthly aggregation.** `BarAggregation` stops at `Week`, so the
//!   venue's `1M` interval has no representation and is refused.
//! - **Perps supports fewer intervals than spot.** The venue documents `3m`, `2h`, `6h`,
//!   `8h`, `12h` and `3d` as spot-only.
//! - **Only a fixed set of steps exists per unit.** A 2-minute bar is valid in Nautilus but
//!   has no venue interval, and rounding it to 1m or 3m would return a different series
//!   than was asked for.
//!
//! Nautilus applies its own constraint upstream - minute steps must divide 60 evenly - so
//! this layer only has to reject what Nautilus accepts but the venue does not serve.
//!
//! # The ticker is the only top of book available
//!
//! The venue publishes no dedicated quote channel. Its `ticker` frame carries best bid and
//! ask alongside 24-hour statistics, on its own cadence - the subscription acknowledgement
//! reports that as `pushInterval`, observed at 1000ms - rather than on every book change. A
//! quote built from it is therefore a periodic sample of the top of book, not a stream of
//! every change, and a strategy that assumes it sees every touch of the spread will be wrong.
//!
//! # Only closed bars are safe to emit
//!
//! The candle channel republishes the forming bar on every block. Emitting those downstream
//! would feed strategies an OHLC that can still move, making a live run incomparable to a
//! backtest built from completed bars. [`parse_bar`] refuses an unclosed candle rather than
//! leaving that filter to each caller.

use std::str::FromStr;

use nautilus_core::UnixNanos;
use nautilus_model::{
    data::{Bar, BarSpecification, BarType, QuoteTick, TradeTick},
    enums::{AggregationSource, AggressorSide, BarAggregation, PriceType},
    identifiers::{InstrumentId, TradeId},
    types::{Price, Quantity},
};

use crate::{
    common::{Market, decimal::normalize_to, enums::OrderSide},
    websocket::{Candle, Ticker, Trade},
};

/// Every interval the venue serves, with its Nautilus equivalent.
///
/// The `spot_only` flag carries the venue's documented restriction that the perps engine
/// supports a smaller set.
struct IntervalMapping {
    venue: &'static str,
    step: usize,
    aggregation: BarAggregation,
    spot_only: bool,
}

const INTERVALS: &[IntervalMapping] = &[
    IntervalMapping {
        venue: "1m",
        step: 1,
        aggregation: BarAggregation::Minute,
        spot_only: false,
    },
    IntervalMapping {
        venue: "3m",
        step: 3,
        aggregation: BarAggregation::Minute,
        spot_only: true,
    },
    IntervalMapping {
        venue: "5m",
        step: 5,
        aggregation: BarAggregation::Minute,
        spot_only: false,
    },
    IntervalMapping {
        venue: "15m",
        step: 15,
        aggregation: BarAggregation::Minute,
        spot_only: false,
    },
    IntervalMapping {
        venue: "30m",
        step: 30,
        aggregation: BarAggregation::Minute,
        spot_only: false,
    },
    IntervalMapping {
        venue: "1h",
        step: 1,
        aggregation: BarAggregation::Hour,
        spot_only: false,
    },
    IntervalMapping {
        venue: "4h",
        step: 4,
        aggregation: BarAggregation::Hour,
        spot_only: false,
    },
    IntervalMapping {
        venue: "6h",
        step: 6,
        aggregation: BarAggregation::Hour,
        spot_only: true,
    },
    IntervalMapping {
        venue: "8h",
        step: 8,
        aggregation: BarAggregation::Hour,
        spot_only: true,
    },
    IntervalMapping {
        venue: "12h",
        step: 12,
        aggregation: BarAggregation::Hour,
        spot_only: true,
    },
    IntervalMapping {
        venue: "1d",
        step: 1,
        aggregation: BarAggregation::Day,
        spot_only: false,
    },
    IntervalMapping {
        venue: "3d",
        step: 3,
        aggregation: BarAggregation::Day,
        spot_only: true,
    },
    IntervalMapping {
        venue: "1w",
        step: 1,
        aggregation: BarAggregation::Week,
        spot_only: false,
    },
    // `1M` is deliberately absent: Nautilus has no monthly aggregation to map it onto.
];

/// Why a bar specification cannot be served.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BarMappingError {
    #[error("{step}-{aggregation:?} bars have no SoDEX interval")]
    NoSuchInterval {
        step: usize,
        aggregation: BarAggregation,
    },
    #[error("interval {interval} is spot-only; the perps engine does not serve it")]
    SpotOnly { interval: &'static str },
    #[error("unknown SoDEX interval {0:?}")]
    UnknownInterval(String),
    #[error("bar is still forming; only closed bars may be emitted")]
    BarNotClosed,
    #[error("invalid {field} {value:?}: {reason}")]
    InvalidValue {
        field: &'static str,
        value: String,
        reason: String,
    },
}

/// The Nautilus specification for a venue interval string.
///
/// # Errors
///
/// Returns [`BarMappingError::UnknownInterval`] for anything the venue does not serve,
/// including `1M`, which has no Nautilus aggregation.
pub fn interval_to_spec(interval: &str) -> Result<BarSpecification, BarMappingError> {
    INTERVALS
        .iter()
        .find(|m| m.venue == interval)
        .map(|m| BarSpecification::new(m.step, m.aggregation, PriceType::Last))
        .ok_or_else(|| BarMappingError::UnknownInterval(interval.to_string()))
}

/// The venue interval string for a Nautilus specification on a given engine.
///
/// # Errors
///
/// Returns [`BarMappingError::NoSuchInterval`] when no venue interval matches, or
/// [`BarMappingError::SpotOnly`] when the interval exists but the perps engine does not
/// serve it. Refusing here means a strategy learns at subscription time rather than by
/// silently receiving nothing.
pub fn spec_to_interval(
    spec: &BarSpecification,
    market: Market,
) -> Result<&'static str, BarMappingError> {
    let step = spec.step.get();
    let mapping = INTERVALS
        .iter()
        .find(|m| m.step == step && m.aggregation == spec.aggregation)
        .ok_or(BarMappingError::NoSuchInterval {
            step,
            aggregation: spec.aggregation,
        })?;

    if mapping.spot_only && market == Market::Perps {
        return Err(BarMappingError::SpotOnly {
            interval: mapping.venue,
        });
    }

    Ok(mapping.venue)
}

/// Builds a bar type for a venue interval.
///
/// The aggregation source is always [`AggregationSource::External`]: these bars are computed
/// by the venue, not aggregated locally from ticks.
///
/// # Errors
///
/// Returns [`BarMappingError`] if the interval has no Nautilus equivalent.
pub fn bar_type_for(
    instrument_id: InstrumentId,
    interval: &str,
) -> Result<BarType, BarMappingError> {
    Ok(BarType::new(
        instrument_id,
        interval_to_spec(interval)?,
        AggregationSource::External,
    ))
}

/// Converts a closed candle into a Nautilus bar.
///
/// # Errors
///
/// Returns [`BarMappingError::BarNotClosed`] for a still-forming candle, or
/// [`BarMappingError::InvalidValue`] if a price or volume cannot be parsed.
pub fn parse_bar(
    candle: &Candle,
    bar_type: BarType,
    price_precision: u8,
    size_precision: u8,
    ts_init: UnixNanos,
) -> Result<Bar, BarMappingError> {
    if !candle.is_final() {
        return Err(BarMappingError::BarNotClosed);
    }
    parse_completed_bar(candle, bar_type, price_precision, size_precision, ts_init)
}

/// Converts a candle the caller has established is complete.
///
/// Observed on the live testnet: the venue republishes the forming bar on every block and
/// then simply starts the next one - over two full bar periods, no push ever carried
/// `closed`. A consumer that waited for that flag would receive nothing at all while its
/// connection looked perfectly healthy.
///
/// The flag is still authoritative when it is set. When it is not, a bar whose successor has
/// begun is complete by construction, and that is evidence the caller holds and this function
/// does not - hence the split. Use [`parse_bar`] wherever the flag is the only evidence
/// available.
///
/// # Errors
///
/// Returns [`BarMappingError::InvalidValue`] if a price or volume cannot be parsed.
pub fn parse_completed_bar(
    candle: &Candle,
    bar_type: BarType,
    price_precision: u8,
    size_precision: u8,
    ts_init: UnixNanos,
) -> Result<Bar, BarMappingError> {
    // The four prices must share one precision: the engine panics on a bar whose fields
    // disagree, because its Arrow encoding assumes a uniform scale. Deriving each from its own
    // text satisfies that only by luck - it held on an instrument quoting whole numbers and
    // failed on the first one quoting `"2465"` beside `"2464.9"`. The instrument's declared
    // precision is the only source that agrees across fields, as it is for quotes and trades.
    let volume = quantity_at(&candle.volume, size_precision, "volume")?;

    Ok(Bar::new(
        bar_type,
        price_at(&candle.open, price_precision, "open")?,
        price_at(&candle.high, price_precision, "high")?,
        price_at(&candle.low, price_precision, "low")?,
        price_at(&candle.close, price_precision, "close")?,
        volume,
        // The venue stamps bars in milliseconds; Nautilus works in nanoseconds. The bar's
        // event time is its open, which is what makes a series joinable across sources.
        UnixNanos::from(candle.open_time_ms * 1_000_000),
        ts_init,
    ))
}

/// The precision a value must carry to sit beside its counterpart in one tick.
///
/// Taken from the instrument rather than from the text, because the venue writes the same
/// tick size two ways - a bid of `"77378.5"` beside an ask of `"77379"` - and Nautilus
/// rejects a quote whose two sides disagree about precision.
pub(crate) fn price_at(
    raw: &str,
    precision: u8,
    field: &'static str,
) -> Result<Price, BarMappingError> {
    let invalid = |reason: String| BarMappingError::InvalidValue {
        field,
        value: raw.to_string(),
        reason,
    };
    let normalized = normalize_to(raw, precision).map_err(|e| invalid(e.to_string()))?;
    Price::from_str(&normalized).map_err(invalid)
}

pub(crate) fn quantity_at(
    raw: &str,
    precision: u8,
    field: &'static str,
) -> Result<Quantity, BarMappingError> {
    let invalid = |reason: String| BarMappingError::InvalidValue {
        field,
        value: raw.to_string(),
        reason,
    };
    let normalized = normalize_to(raw, precision).map_err(|e| invalid(e.to_string()))?;
    Quantity::from_str(&normalized).map_err(invalid)
}

/// Which side crossed the spread.
///
/// The venue's `S` names the aggressor. Established by observation rather than assumption:
/// across 22 mainnet trades matched against contemporaneous top of book, every `BUY` printed
/// at the ask and every `SELL` at the bid, which is a taker crossing in each case. Reading it
/// as the resting side would invert every order-flow measure built on this feed.
#[must_use]
pub const fn map_aggressor_side(side: OrderSide) -> AggressorSide {
    match side {
        OrderSide::Buy => AggressorSide::Buy,
        OrderSide::Sell => AggressorSide::Sell,
    }
}

/// Converts a public trade into a Nautilus tick.
///
/// # Errors
///
/// Returns [`BarMappingError::InvalidValue`] if the price or size cannot be parsed at the
/// instrument's precision.
pub fn parse_trade(
    trade: &Trade,
    instrument_id: InstrumentId,
    price_precision: u8,
    size_precision: u8,
    ts_init: UnixNanos,
) -> Result<TradeTick, BarMappingError> {
    Ok(TradeTick::new(
        instrument_id,
        price_at(&trade.price, price_precision, "trade price")?,
        quantity_at(&trade.quantity, size_precision, "trade quantity")?,
        map_aggressor_side(trade.side),
        TradeId::new(trade.trade_id.to_string().as_str()),
        // The trade's own time, not the frame's: `E` is when the venue produced the push and
        // moves with network conditions, while `T` is when the trade happened.
        UnixNanos::from(trade.trade_time_ms * 1_000_000),
        ts_init,
    ))
}

/// Converts a ticker frame into a Nautilus quote.
///
/// # Errors
///
/// Returns [`BarMappingError::InvalidValue`] if a price or size cannot be parsed at the
/// instrument's precision.
pub fn parse_quote(
    ticker: &Ticker,
    instrument_id: InstrumentId,
    price_precision: u8,
    size_precision: u8,
    ts_init: UnixNanos,
) -> Result<QuoteTick, BarMappingError> {
    Ok(QuoteTick::new(
        instrument_id,
        price_at(&ticker.bid_price, price_precision, "bid price")?,
        price_at(&ticker.ask_price, price_precision, "ask price")?,
        quantity_at(&ticker.bid_quantity, size_precision, "bid size")?,
        quantity_at(&ticker.ask_quantity, size_precision, "ask size")?,
        UnixNanos::from(ticker.event_time_ms * 1_000_000),
        ts_init,
    ))
}

#[cfg(test)]
mod tests {
    use nautilus_model::identifiers::{Symbol, Venue};
    use rstest::rstest;

    use super::*;
    use crate::config::{SODEX_PERPS, SODEX_SPOT};

    fn instrument(venue: &str) -> InstrumentId {
        InstrumentId::new(Symbol::from("vBTC_vUSDC"), Venue::from(venue))
    }

    fn closed_candle() -> Candle {
        Candle {
            open_time_ms: 1_767_972_900_000,
            update_time_ms: 1_767_972_960_000,
            symbol: "BTC-USD".to_string(),
            interval: "1m".to_string(),
            open: "91869".to_string(),
            high: "91982".to_string(),
            low: "91869".to_string(),
            close: "91976".to_string(),
            volume: "4.12298".to_string(),
            quote_volume: "379148.6798".to_string(),
            trade_count: 12,
            closed: true,
        }
    }

    #[rstest]
    fn common_intervals_round_trip_through_both_directions() {
        for interval in ["1m", "5m", "15m", "30m", "1h", "4h", "1d", "1w"] {
            let spec = interval_to_spec(interval).unwrap();
            assert_eq!(spec_to_interval(&spec, Market::Spot).unwrap(), interval);
            assert_eq!(spec_to_interval(&spec, Market::Perps).unwrap(), interval);
        }
    }

    #[rstest]
    fn spot_only_intervals_are_refused_on_perps() {
        // The venue documents these as unavailable on the perps engine. Refusing at
        // subscription time beats a subscription that silently never delivers.
        for interval in ["3m", "6h", "8h", "12h", "3d"] {
            let spec = interval_to_spec(interval).unwrap();
            assert_eq!(spec_to_interval(&spec, Market::Spot).unwrap(), interval);
            assert!(
                matches!(
                    spec_to_interval(&spec, Market::Perps),
                    Err(BarMappingError::SpotOnly { .. })
                ),
                "{interval} must be refused on perps"
            );
        }
    }

    #[rstest]
    fn monthly_bars_have_no_representation() {
        // Nautilus BarAggregation stops at Week. Mapping 1M onto Day or Week would return a
        // series that silently differs from what was asked for.
        assert!(matches!(
            interval_to_spec("1M"),
            Err(BarMappingError::UnknownInterval(_))
        ));
    }

    #[rstest]
    fn an_unsupported_step_is_refused_rather_than_rounded() {
        // 2-minute bars are valid in Nautilus but the venue does not serve them. Rounding
        // to 1m or 3m would hand the strategy a different series than it subscribed to.
        //
        // The step is deliberately 2 rather than something like 7: Nautilus requires minute
        // steps to divide 60 evenly, so a 7 is rejected before this layer is ever reached.
        // Testing with one would assert against the wrong layer's constraint.
        let spec = BarSpecification::new(2, BarAggregation::Minute, PriceType::Last);

        assert!(matches!(
            spec_to_interval(&spec, Market::Spot),
            Err(BarMappingError::NoSuchInterval { step: 2, .. })
        ));
    }

    #[rstest]
    fn bars_are_marked_as_externally_aggregated() {
        // These come from the venue, not from local tick aggregation; the distinction
        // affects how the engine treats them.
        let bar_type = bar_type_for(instrument(SODEX_SPOT), "1m").unwrap();

        assert_eq!(bar_type.aggregation_source(), AggregationSource::External);
    }

    #[rstest]
    fn forming_candles_are_refused() {
        // The channel republishes the open bar on every block. Emitting one would feed a
        // strategy an OHLC that can still change.
        let mut forming = closed_candle();
        forming.closed = false;
        let bar_type = bar_type_for(instrument(SODEX_SPOT), "1m").unwrap();

        assert_eq!(
            parse_bar(&forming, bar_type, 0, 5, UnixNanos::default()),
            Err(BarMappingError::BarNotClosed)
        );
    }

    #[rstest]
    fn closed_candle_becomes_a_bar_with_venue_prices_intact() {
        let bar_type = bar_type_for(instrument(SODEX_SPOT), "1m").unwrap();
        let bar = parse_bar(&closed_candle(), bar_type, 0, 5, UnixNanos::default()).unwrap();

        assert_eq!(bar.open.to_string(), "91869");
        assert_eq!(bar.high.to_string(), "91982");
        assert_eq!(bar.low.to_string(), "91869");
        assert_eq!(bar.close.to_string(), "91976");
        assert_eq!(bar.volume.to_string(), "4.12298");
    }

    #[rstest]
    fn mixed_decimal_places_across_one_candle_still_share_a_precision() {
        // The engine panics on a bar whose fields disagree about scale, and this venue writes
        // one tick size several ways within a single candle. Deriving precision from each
        // value's own text passed only while an instrument happened to quote whole numbers.
        let candle = Candle {
            open: "2465".to_string(),
            high: "2465.400000000000000000".to_string(),
            low: "2464.1".to_string(),
            close: "2464.900000000000000000".to_string(),
            ..closed_candle()
        };
        let bar_type = bar_type_for(instrument(SODEX_SPOT), "1m").unwrap();
        let bar = parse_bar(&candle, bar_type, 1, 5, UnixNanos::default()).unwrap();

        assert_eq!(bar.open.to_string(), "2465.0");
        assert_eq!(bar.high.to_string(), "2465.4");
        assert_eq!(bar.low.to_string(), "2464.1");
        assert_eq!(bar.close.to_string(), "2464.9");
        assert_eq!(bar.open.precision, 1);
        assert_eq!(bar.high.precision, 1);
        assert_eq!(bar.low.precision, 1);
        assert_eq!(bar.close.precision, 1);
    }

    #[rstest]
    fn bar_event_time_is_the_open_converted_to_nanoseconds() {
        // The venue stamps milliseconds. Using the update time instead of the open would
        // shift every bar forward by up to one interval.
        let candle = closed_candle();
        let bar_type = bar_type_for(instrument(SODEX_SPOT), "1m").unwrap();
        let bar = parse_bar(&candle, bar_type, 0, 5, UnixNanos::default()).unwrap();

        assert_eq!(bar.ts_event.as_u64(), candle.open_time_ms * 1_000_000);
        assert_ne!(bar.ts_event.as_u64(), candle.update_time_ms * 1_000_000);
    }

    #[rstest]
    fn the_same_interval_on_each_engine_yields_distinct_bar_types() {
        let spot = bar_type_for(instrument(SODEX_SPOT), "1m").unwrap();
        let perps = bar_type_for(instrument(SODEX_PERPS), "1m").unwrap();

        assert_ne!(spot, perps);
        assert_eq!(spot.spec(), perps.spec());
    }
}
