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

//! ATR-normalized margin strategy for short-horizon binary outcome markets.

use std::{
    collections::{HashMap, VecDeque},
    fmt::Debug,
};

use nautilus_common::actor::DataActor;
use nautilus_model::{
    data::{CustomData, DataType, QuoteTick},
    enums::{OrderSide, TimeInForce},
    identifiers::{InstrumentId, Venue},
    instruments::InstrumentAny,
    types::{Price, Quantity},
};
use rust_decimal::Decimal;
use ustr::Ustr;

use super::{
    config::AtrMarginBinaryConfig,
    decision::{
        BarAccumulator, DirectionalBar, Thresholds, Verdict, Vote, atr_scale,
        break_even_win_rate, directional_atr, evaluate,
    },
};
use nautilus_polymarket::{common::consts::POLYMARKET_CLIENT_ID, data_types::PolymarketRtdsCryptoTwap};
use nautilus_trading::{
    nautilus_strategy,
    strategy::{Strategy, StrategyCore},
};

/// Reference observations retained, bounding memory on a long run.
const REFERENCE_HISTORY_CAP: usize = 4096;

/// Tolerance when matching a reference observation to a market's activation instant.
const BASELINE_MATCH_TOLERANCE_NS: u64 = 5_000_000_000;

/// One tradeable leg of an outcome pair.
#[derive(Debug, Clone)]
struct Leg {
    instrument_id: InstrumentId,
    price_precision: u8,
    size_precision: u8,
    bid: Option<Price>,
    ask: Option<Price>,
}

/// State for one outcome pair, keyed by the venue's event identifier.
#[derive(Debug, Clone)]
struct Window {
    up: Option<Leg>,
    down: Option<Leg>,
    activation_ns: u64,
    expiration_ns: u64,
    /// Reference value at the market's activation instant, the level the outcome resolves against.
    baseline: Option<Decimal>,
    /// Set once the activation instant has passed without a usable reference observation.
    baseline_unavailable: bool,
    ordered: bool,
}

impl Window {
    fn leg(&self, vote: Vote) -> Option<&Leg> {
        match vote {
            Vote::Up => self.up.as_ref(),
            Vote::Down => self.down.as_ref(),
        }
    }
}

/// Counters for every path that declines to trade.
///
/// A strategy that silently does nothing is indistinguishable from one whose feed has
/// stopped, so each decline increments a named counter rather than returning quietly.
#[derive(Debug, Default, Clone)]
pub(crate) struct DeclineCounts {
    /// Reference feed had not produced enough completed bars.
    pub(crate) no_atr: u64,
    /// Activation instant passed before a reference observation was available.
    pub(crate) no_baseline: u64,
    /// The leg the vote called for was not subscribed.
    pub(crate) no_leg: u64,
    /// The leg had no usable quote.
    pub(crate) no_quote: u64,
    /// Entry price fell outside the configured bounds.
    pub(crate) price_bounds: u64,
    /// Ratio landed in the no-trade band.
    pub(crate) undecided: u64,
    /// A non-thick-lead rule fired while restricted to thick-lead only.
    pub(crate) rule_filtered: u64,
    /// Market carried no event identifier, so its legs could not be paired.
    pub(crate) unpaired: u64,
}

/// ATR-normalized margin strategy.
///
/// Trades the proposition that as settlement approaches, the distance between the current
/// reference level and the settlement baseline becomes large relative to what remaining
/// volatility can traverse. The distance is normalized by a directional ATR of the same
/// feed the market resolves against, and only entries inside the configured price bounds
/// are taken.
pub(crate) struct AtrMarginBinary {
    pub(crate) core: StrategyCore,
    pub(crate) config: AtrMarginBinaryConfig,
    thresholds: Thresholds,
    reference_data_type: Option<DataType>,
    /// Reference observations as `(ts_event_ns, value)`, ascending.
    reference_history: VecDeque<(u64, Decimal)>,
    /// Bars of the reference feed, bucketed on absolute time.
    bars: BarAccumulator,
    windows: HashMap<Ustr, Window>,
    /// Orders submitted, for reconciliation against the decline counters.
    submitted: u64,
    declines: DeclineCounts,
}

impl AtrMarginBinary {
    /// Creates a new [`AtrMarginBinary`] instance from config.
    #[must_use]
    pub(crate) fn from_config(config: AtrMarginBinaryConfig) -> Self {
        let config_bar_secs = config.atr_bar_secs;
        let config_atr_bars = config.atr_bars;
        let thresholds = Thresholds {
            lead_thick: config.k_lead_thick,
            lead_thin: config.k_lead_thin,
            gap_near: config.k_gap_near,
            gap_far: config.k_gap_far,
        };
        Self {
            core: StrategyCore::new(config.base.clone()),
            config,
            thresholds,
            reference_data_type: None,
            reference_history: VecDeque::new(),
            bars: BarAccumulator::new(config_bar_secs, config_atr_bars),
            windows: HashMap::new(),
            submitted: 0,
            declines: DeclineCounts::default(),
        }
    }

    /// Returns the counters for every path that declined to trade.
    #[must_use]
    pub(crate) const fn declines(&self) -> &DeclineCounts {
        &self.declines
    }

    /// Returns the number of orders submitted.
    #[must_use]
    pub(crate) const fn submitted(&self) -> u64 {
        self.submitted
    }

    /// Returns the completed reference bars.
    #[must_use]
    pub(crate) const fn bars(&self) -> &VecDeque<DirectionalBar> {
        self.bars.completed()
    }

    /// Folds one reference observation into the bar series and history.
    ///
    /// Bars are bucketed on absolute time so their boundaries line up with the venue's
    /// own minute boundaries rather than with whenever this strategy happened to start.
    pub(crate) fn ingest_reference(&mut self, ts_event_ns: u64, value: Decimal) {
        if !self.bars.push(ts_event_ns, value) {
            log::warn!(
                "Discarding out-of-order reference observation at ts={ts_event_ns}; \
                 total rejected={}",
                self.bars.out_of_order()
            );
            return;
        }
        self.reference_history.push_back((ts_event_ns, value));
        while self.reference_history.len() > REFERENCE_HISTORY_CAP {
            self.reference_history.pop_front();
        }
    }

    /// Returns the reference observation closest to `target_ns` within the match tolerance.
    fn reference_at(&self, target_ns: u64) -> Option<Decimal> {
        let mut best: Option<(u64, Decimal)> = None;
        for &(ts, value) in &self.reference_history {
            let delta = ts.abs_diff(target_ns);
            if delta > BASELINE_MATCH_TOLERANCE_NS {
                continue;
            }
            match best {
                Some((best_ts, _)) if best_ts.abs_diff(target_ns) <= delta => {}
                _ => best = Some((ts, value)),
            }
        }
        best.map(|(_, v)| v)
    }

    /// Returns the latest reference observation.
    fn latest_reference(&self) -> Option<Decimal> {
        self.reference_history.back().map(|&(_, v)| v)
    }

    /// Resolves pending baselines and drops markets that have expired.
    fn refresh_windows(&mut self, now_ns: u64) {
        let mut resolved: Vec<(Ustr, Decimal)> = Vec::new();
        let mut unavailable: Vec<Ustr> = Vec::new();
        let mut expired: Vec<Ustr> = Vec::new();

        for (event_id, window) in &self.windows {
            if now_ns > window.expiration_ns {
                expired.push(*event_id);
                continue;
            }
            if window.baseline.is_some() || window.baseline_unavailable {
                continue;
            }
            if let Some(value) = self.reference_at(window.activation_ns) {
                resolved.push((*event_id, value));
            } else if now_ns > window.activation_ns + BASELINE_MATCH_TOLERANCE_NS {
                // The activation instant is now outside the tolerance and no observation
                // covered it, so this market's baseline can never be established. Trading
                // it would require inferring the level the outcome resolves against.
                unavailable.push(*event_id);
            }
        }

        for (event_id, value) in resolved {
            if let Some(window) = self.windows.get_mut(&event_id) {
                window.baseline = Some(value);
                log::info!("Baseline resolved for event {event_id}: {value}");
            }
        }
        for event_id in unavailable {
            if let Some(window) = self.windows.get_mut(&event_id) {
                window.baseline_unavailable = true;
                self.declines.no_baseline += 1;
                log::warn!(
                    "Baseline unavailable for event {event_id}: activation instant not covered \
                     by the reference feed; market will not be traded"
                );
            }
        }
        for event_id in expired {
            self.windows.remove(&event_id);
        }
    }

    /// Evaluates every market that has reached its decision point.
    fn evaluate_due(&mut self, now_ns: u64) -> anyhow::Result<()> {
        let Some((up_atr, down_atr)) = directional_atr(self.bars.completed(), self.config.atr_min_bars) else {
            self.declines.no_atr += 1;
            return Ok(());
        };
        let Some(current) = self.latest_reference() else {
            return Ok(());
        };

        let lower_ns = self
            .config
            .decide_before_expiry_secs
            .saturating_sub(self.config.decide_tolerance_secs)
            * 1_000_000_000;
        let upper_ns = (self.config.decide_before_expiry_secs + self.config.decide_tolerance_secs)
            * 1_000_000_000;

        let due: Vec<Ustr> = self
            .windows
            .iter()
            .filter(|(_, w)| !w.ordered && w.baseline.is_some())
            .filter(|(_, w)| {
                let remaining = w.expiration_ns.saturating_sub(now_ns);
                remaining >= lower_ns && remaining <= upper_ns
            })
            .map(|(id, _)| *id)
            .collect();

        for event_id in due {
            self.decide_one(event_id, current, up_atr, down_atr, now_ns)?;
        }
        Ok(())
    }

    fn decide_one(
        &mut self,
        event_id: Ustr,
        current: Decimal,
        up_atr: Decimal,
        down_atr: Decimal,
        now_ns: u64,
    ) -> anyhow::Result<()> {
        let Some(window) = self.windows.get(&event_id) else {
            return Ok(());
        };
        let Some(baseline) = window.baseline else {
            return Ok(());
        };
        let remaining_secs = (window.expiration_ns.saturating_sub(now_ns)) as f64 / 1e9;
        let scale = atr_scale(
            remaining_secs,
            self.config.atr_bar_secs as f64,
            self.config.scale_atr_by_remaining,
        );
        let decision = evaluate(
            baseline,
            current,
            up_atr,
            down_atr,
            scale,
            &self.thresholds,
        );
        let ratio_str = decision
            .ratio
            .map_or_else(|| "n/a".to_string(), |r| format!("{r:.3}"));

        let Some(vote) = decision.verdict.vote() else {
            match decision.verdict {
                Verdict::NoAtr => self.declines.no_atr += 1,
                _ => self.declines.undecided += 1,
            }
            log::debug!(
                "No entry for event {event_id}: verdict={} ratio={ratio_str}",
                decision.verdict.label()
            );
            return Ok(());
        };
        if self.config.thick_lead_only && !decision.verdict.is_thick_lead() {
            self.declines.rule_filtered += 1;
            log::debug!(
                "Entry filtered for event {event_id}: verdict={} restricted to thick-lead only",
                decision.verdict.label()
            );
            return Ok(());
        }
        let Some(leg) = window.leg(vote).cloned() else {
            self.declines.no_leg += 1;
            log::warn!(
                "No {vote:?} leg subscribed for event {event_id}; verdict={}",
                decision.verdict.label()
            );
            return Ok(());
        };
        // Resting at the near touch pays no spread but is not guaranteed to fill;
        // crossing fills immediately and pays the spread.
        let price = if self.config.post_only {
            leg.bid
        } else {
            leg.ask
        };
        let Some(price) = price else {
            self.declines.no_quote += 1;
            log::warn!("No usable quote for {} on event {event_id}", leg.instrument_id);
            return Ok(());
        };
        let price_f = price.as_f64();
        if price_f > self.config.max_entry_price || price_f < self.config.min_entry_price {
            self.declines.price_bounds += 1;
            log::info!(
                "Entry rejected for event {event_id}: price {price_f:.4} outside \
                 [{:.4}, {:.4}]; break-even win rate at this price is {:.1}%",
                self.config.min_entry_price,
                self.config.max_entry_price,
                break_even_win_rate(price_f) * 100.0
            );
            return Ok(());
        }

        let quantity = Quantity::new(self.config.trade_size.as_f64(), leg.size_precision);
        let limit_price = Price::new(price_f, leg.price_precision);
        let (tif, expire_time) = match self.config.order_expire_secs {
            Some(secs) => (
                Some(TimeInForce::Gtd),
                Some(
                    self.clock().timestamp_ns()
                        + nautilus_core::nanos::DurationNanos::try_from_secs(secs)?,
                ),
            ),
            None => (None, None),
        };
        let order = self.order().limit(
            leg.instrument_id,
            OrderSide::Buy,
            quantity,
            limit_price,
            tif,
            expire_time,
            Some(self.config.post_only),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        );
        self.submit_order(order, None, None, None)?;
        self.submitted += 1;
        if let Some(window) = self.windows.get_mut(&event_id) {
            window.ordered = true;
        }
        log::info!(
            "Entry submitted for event {event_id}: {vote:?} {} @ {limit_price} \
             verdict={} ratio={ratio_str} remaining={remaining_secs:.0}s",
            leg.instrument_id,
            decision.verdict.label()
        );
        Ok(())
    }

    /// Registers a binary outcome market and subscribes to its quotes.
    fn register_instrument(&mut self, instrument: &InstrumentAny) {
        let InstrumentAny::BinaryOption(binary) = instrument else {
            return;
        };
        let Some(event_id) = binary.event_id else {
            // Without an event identifier the up and down legs of the same market cannot
            // be paired, and a vote could be filled on the wrong side.
            self.declines.unpaired += 1;
            log::warn!(
                "Skipping {}: no event_id, legs cannot be paired",
                binary.id
            );
            return;
        };
        let Some(outcome) = binary.outcome else {
            log::warn!("Skipping {}: no outcome label", binary.id);
            return;
        };
        let expiration_ns = binary.expiration_ns.as_u64();
        let activation_ns = binary.activation_ns.as_u64();
        if expiration_ns == 0 || expiration_ns <= activation_ns {
            log::warn!(
                "Skipping {}: unusable activation/expiration pair ({activation_ns}, {expiration_ns})",
                binary.id
            );
            return;
        }
        let lower = outcome.to_lowercase();
        // The adapter re-publishes instrument definitions periodically. Re-registering
        // would reset the leg and discard quotes already received, so a leg that is
        // already present is left untouched.
        if let Some(window) = self.windows.get(&event_id) {
            let known = match lower.as_str() {
                "up" | "yes" => window.up.as_ref(),
                "down" | "no" => window.down.as_ref(),
                _ => None,
            };
            if known.is_some_and(|leg| leg.instrument_id == binary.id) {
                return;
            }
        }
        let leg = Leg {
            instrument_id: binary.id,
            price_precision: binary.price_precision,
            size_precision: binary.size_precision,
            bid: None,
            ask: None,
        };
        let window = self.windows.entry(event_id).or_insert_with(|| Window {
            up: None,
            down: None,
            activation_ns,
            expiration_ns,
            baseline: None,
            baseline_unavailable: false,
            ordered: false,
        });
        match lower.as_str() {
            "up" | "yes" => window.up = Some(leg),
            "down" | "no" => window.down = Some(leg),
            other => {
                log::warn!("Skipping {}: unrecognized outcome '{other}'", binary.id);
                return;
            }
        }
        self.subscribe_quotes(binary.id, None, None);
        log::info!(
            "Registered {instrument_id} as '{lower}' leg of event {event_id}, \
             expires at {expiration_ns}",
            instrument_id = binary.id
        );
    }
}

nautilus_strategy!(AtrMarginBinary);

impl Debug for AtrMarginBinary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct(stringify!(AtrMarginBinary))
            .field("reference_symbol", &self.config.reference_symbol)
            .field("decide_before_expiry_secs", &self.config.decide_before_expiry_secs)
            .field("windows", &self.windows.len())
            .field("bars", &self.bars.completed().len())
            .field("submitted", &self.submitted)
            .finish()
    }
}

impl DataActor for AtrMarginBinary {
    fn on_start(&mut self) -> anyhow::Result<()> {
        let mut metadata = nautilus_core::Params::new();
        metadata.insert(
            "symbol".to_string(),
            serde_json::Value::String(self.config.reference_symbol.clone()),
        );
        metadata.insert(
            "window_seconds".to_string(),
            serde_json::Value::from(self.config.reference_window_seconds),
        );
        let data_type = DataType::new("PolymarketRtdsCryptoTwap", Some(metadata), None);
        self.reference_data_type = Some(data_type.clone());
        // A custom DataType carries no venue, so the engine cannot infer which client
        // should receive the subscription: without an explicit client id it is registered
        // on the message bus and never reaches the adapter.
        self.subscribe_data(data_type, Some(*POLYMARKET_CLIENT_ID), None);
        self.subscribe_instruments(Venue::from("POLYMARKET"), None, None);
        log::info!(
            "Started: reference={} window={}s decide at T-{}s (+/-{}s)",
            self.config.reference_symbol,
            self.config.reference_window_seconds,
            self.config.decide_before_expiry_secs,
            self.config.decide_tolerance_secs
        );
        Ok(())
    }

    fn on_stop(&mut self) -> anyhow::Result<()> {
        if let Some(data_type) = self.reference_data_type.take() {
            self.unsubscribe_data(data_type, Some(*POLYMARKET_CLIENT_ID), None);
        }
        let d = self.declines().clone();
        log::info!(
            "Stopped: submitted={} bars={} declines[no_atr={} no_baseline={} no_leg={} \
             no_quote={} price_bounds={} undecided={} rule_filtered={} unpaired={}]",
            self.submitted(),
            self.bars().len(),
            d.no_atr,
            d.no_baseline,
            d.no_leg,
            d.no_quote,
            d.price_bounds,
            d.undecided,
            d.rule_filtered,
            d.unpaired
        );
        Ok(())
    }

    fn on_instrument(&mut self, instrument: &InstrumentAny) -> anyhow::Result<()> {
        self.register_instrument(instrument);
        Ok(())
    }

    fn on_quote(&mut self, quote: &QuoteTick) -> anyhow::Result<()> {
        for window in self.windows.values_mut() {
            for leg in [window.up.as_mut(), window.down.as_mut()].into_iter().flatten() {
                if leg.instrument_id == quote.instrument_id {
                    leg.bid = Some(quote.bid_price);
                    leg.ask = Some(quote.ask_price);
                    return Ok(());
                }
            }
        }
        Ok(())
    }

    fn on_data(&mut self, data: &CustomData) -> anyhow::Result<()> {
        let Some(twap) = data
            .data
            .as_any()
            .downcast_ref::<PolymarketRtdsCryptoTwap>()
        else {
            return Ok(());
        };
        if !twap
            .symbol
            .eq_ignore_ascii_case(&self.config.reference_symbol)
            || u64::from(twap.window_seconds) != self.config.reference_window_seconds
        {
            return Ok(());
        }
        self.ingest_reference(twap.ts_event.as_u64(), twap.value);
        let now_ns = self.clock().timestamp_ns().as_u64();
        self.refresh_windows(now_ns);
        self.evaluate_due(now_ns)
    }

    fn on_reset(&mut self) -> anyhow::Result<()> {
        self.reference_history.clear();
        self.bars.clear();
        self.windows.clear();
        self.submitted = 0;
        self.declines = DeclineCounts::default();
        Ok(())
    }
}
