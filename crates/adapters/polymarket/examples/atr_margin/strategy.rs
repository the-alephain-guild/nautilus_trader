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
    enums::{LiquiditySide, OrderSide, TimeInForce},
    events::{OrderFilled, PositionClosed, PositionOpened},
    identifiers::{InstrumentId, Venue},
    instruments::InstrumentAny,
    types::{Price, Quantity},
};
use rust_decimal::Decimal;
use ustr::Ustr;

use super::{
    config::AtrMarginBinaryConfig,
    journal::{DecisionJournal, DecisionRecord, FillRecord, HeartbeatRecord, SettlementRecord},
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

/// Interval between counter snapshots written to the journal.
const HEARTBEAT_INTERVAL_NS: u64 = 60_000_000_000;

/// Tolerance when matching an observation to a market's expiration instant.
///
/// Wider than the baseline tolerance: a settlement read slightly off the close is still
/// informative, and the record carries the offset so it can be filtered later.
const SETTLEMENT_MATCH_TOLERANCE_NS: u64 = 60_000_000_000;

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
    /// How many times this market has been evaluated.
    ///
    /// A refusal does not close the market to re-evaluation, so one market can produce
    /// several decision records; the index lets a consumer collapse them rather than
    /// treating them as independent observations.
    evaluations: u32,
    /// Side ordered, retained so the settlement record can state whether it won.
    ordered_vote: Option<Vote>,
    /// Price paid, retained for the realized profit per share.
    entry_price: Option<f64>,
    /// Guards against journalling the same market's settlement twice.
    settled: bool,
    /// Fills the venue reported on this market's legs, as `(px, qty, commission)`.
    fills: Vec<(f64, f64, f64)>,
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
    /// Maps each subscribed leg back to its market, so a fill can be attributed.
    leg_index: HashMap<InstrumentId, Ustr>,
    /// Fill events received, for the heartbeat.
    fills: u64,
    /// Reference observations accepted, for the heartbeat.
    observations: u64,
    /// Engine clock at the last heartbeat.
    last_heartbeat_ns: u64,
    /// Orders submitted, for reconciliation against the decline counters.
    submitted: u64,
    declines: DeclineCounts,
    journal: Option<DecisionJournal>,
}

impl AtrMarginBinary {
    /// Creates a new [`AtrMarginBinary`] instance from config.
    #[must_use]
    pub(crate) fn from_config(config: AtrMarginBinaryConfig) -> Self {
        let config_bar_secs = config.atr_bar_secs;
        let config_atr_bars = config.atr_bars;
        let journal_path = config.journal_path.clone();
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
            leg_index: HashMap::new(),
            fills: 0,
            observations: 0,
            last_heartbeat_ns: 0,
            submitted: 0,
            declines: DeclineCounts::default(),
            journal: journal_path.map(DecisionJournal::open),
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
        self.observations += 1;
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

    /// Returns the observation nearest `target_ns` with its timestamp.
    ///
    /// Unlike the baseline lookup this uses a wide tolerance and reports the offset,
    /// so a settlement read from a distant observation stays identifiable afterwards
    /// rather than being silently dropped.
    fn nearest_reference(&self, target_ns: u64) -> Option<(u64, Decimal)> {
        self.reference_history
            .iter()
            .filter(|(ts, _)| ts.abs_diff(target_ns) <= SETTLEMENT_MATCH_TOLERANCE_NS)
            .min_by_key(|(ts, _)| ts.abs_diff(target_ns))
            .copied()
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

        let grace_ns = self.config.settle_grace_secs * 1_000_000_000;
        for (event_id, window) in &self.windows {
            if now_ns > window.expiration_ns + grace_ns {
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
            // Settle before dropping: the record is the only lasting trace of the market.
            self.settle_window(event_id, now_ns);
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

        // The upper bound is the target itself, not the target plus a tolerance: the
        // reference feed publishes irregularly, so admitting the whole band around the
        // target fires on the first observation to enter it, which is the earliest edge
        // rather than the target. The tolerance only extends the window backwards, so a
        // sparse feed still gets a chance to act.
        let upper_ns = self.config.decide_before_expiry_secs * 1_000_000_000;
        let lower_ns = self
            .config
            .decide_before_expiry_secs
            .saturating_sub(self.config.decide_tolerance_secs)
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

    /// Evaluates one market and journals the outcome, whether or not it ordered.
    ///
    /// Written with a single journalling exit rather than one per early return: a refusal
    /// path that forgets to record leaves no trace, and the absence of a line is
    /// indistinguishable from the evaluation never having happened.
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
        let expiration_ns = window.expiration_ns;
        let evaluation_index = window.evaluations + 1;
        let remaining_secs = (expiration_ns.saturating_sub(now_ns)) as f64 / 1e9;
        let scale = atr_scale(
            remaining_secs,
            self.config.atr_bar_secs as f64,
            self.config.scale_atr_by_remaining,
        );
        let decision = evaluate(baseline, current, up_atr, down_atr, scale, &self.thresholds);
        let ratio_str = decision
            .ratio
            .map_or_else(|| "n/a".to_string(), |r| format!("{r:.3}"));

        let mut action = "declined";
        let mut decline_reason: Option<&'static str> = None;
        let mut vote_label: Option<&'static str> = None;
        let mut leg_instrument_id: Option<String> = None;
        let mut entry_price: Option<f64> = None;
        let mut bid_f: Option<f64> = None;
        let mut ask_f: Option<f64> = None;
        let mut placed: Option<(Vote, f64)> = None;

        match decision.verdict.vote() {
            None => {
                if matches!(decision.verdict, Verdict::NoAtr) {
                    self.declines.no_atr += 1;
                    decline_reason = Some("no_atr");
                } else {
                    self.declines.undecided += 1;
                    decline_reason = Some("undecided");
                }
                log::debug!(
                    "No entry for event {event_id}: verdict={} ratio={ratio_str}",
                    decision.verdict.label()
                );
            }
            Some(vote) => {
                vote_label = Some(vote.label());
                let leg = window.leg(vote).cloned();
                if self.config.thick_lead_only && !decision.verdict.is_thick_lead() {
                    self.declines.rule_filtered += 1;
                    decline_reason = Some("rule_filtered");
                    log::debug!(
                        "Entry filtered for event {event_id}: verdict={} restricted to \
                         thick-lead only",
                        decision.verdict.label()
                    );
                } else if let Some(leg) = leg {
                    leg_instrument_id = Some(leg.instrument_id.to_string());
                    bid_f = leg.bid.map(|p| p.as_f64());
                    ask_f = leg.ask.map(|p| p.as_f64());
                    // Resting at the near touch pays no spread but is not guaranteed to
                    // fill; crossing fills at once and pays it.
                    let quote = if self.config.post_only { leg.bid } else { leg.ask };
                    match quote {
                        None => {
                            self.declines.no_quote += 1;
                            decline_reason = Some("no_quote");
                            log::warn!(
                                "No usable quote for {} on event {event_id}",
                                leg.instrument_id
                            );
                        }
                        Some(price) => {
                            let price_f = price.as_f64();
                            entry_price = Some(price_f);
                            if price_f > self.config.max_entry_price
                                || price_f < self.config.min_entry_price
                            {
                                self.declines.price_bounds += 1;
                                decline_reason = Some("price_bounds");
                                log::info!(
                                    "Entry rejected for event {event_id}: price \
                                     {price_f:.4} outside [{:.4}, {:.4}]; break-even win \
                                     rate at this price is {:.1}%",
                                    self.config.min_entry_price,
                                    self.config.max_entry_price,
                                    break_even_win_rate(price_f) * 100.0
                                );
                            } else {
                                let quantity = Quantity::new(
                                    self.config.trade_size.as_f64(),
                                    leg.size_precision,
                                );
                                let limit_price = Price::new(price_f, leg.price_precision);
                                let (tif, expire_time) = match self.config.order_expire_secs {
                                    Some(secs) => (
                                        Some(TimeInForce::Gtd),
                                        Some(
                                            self.clock().timestamp_ns()
                                                + nautilus_core::nanos::DurationNanos::try_from_secs(
                                                    secs,
                                                )?,
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
                                action = "submitted";
                                placed = Some((vote, price_f));
                                log::info!(
                                    "Entry submitted for event {event_id}: {vote:?} {} @ \
                                     {limit_price} verdict={} ratio={ratio_str} \
                                     remaining={remaining_secs:.0}s",
                                    leg.instrument_id,
                                    decision.verdict.label()
                                );
                            }
                        }
                    }
                } else {
                    self.declines.no_leg += 1;
                    decline_reason = Some("no_leg");
                    log::warn!(
                        "No {vote:?} leg subscribed for event {event_id}; verdict={}",
                        decision.verdict.label()
                    );
                }
            }
        }

        if let Some(window) = self.windows.get_mut(&event_id) {
            window.evaluations = evaluation_index;
        }
        let atr_bars = self.bars.completed().len();
        if let Some(journal) = self.journal.as_mut() {
            journal.write(&DecisionRecord {
                kind: "decision",
                ts_ns: now_ns,
                event_id: event_id.as_str(),
                expiration_ns,
                evaluation_index,
                remaining_secs,
                baseline,
                current,
                up_atr,
                down_atr,
                atr_scale: scale,
                atr_bars,
                ratio: decision.ratio,
                verdict: decision.verdict.label(),
                vote: vote_label,
                action,
                decline_reason,
                leg_instrument_id,
                entry_price,
                bid: bid_f,
                ask: ask_f,
                break_even_win_rate: entry_price.map(break_even_win_rate),
                trade_size: Some(self.config.trade_size.as_f64()),
                post_only: self.config.post_only,
            });
        }

        // Only a market that ordered is barred from re-evaluation; a refusal may become
        // an entry on a later observation inside the same decision window.
        if let Some((vote, price)) = placed
            && let Some(window) = self.windows.get_mut(&event_id)
        {
            window.ordered = true;
            window.ordered_vote = Some(vote);
            window.entry_price = Some(price);
        }
        Ok(())
    }

    /// Journals the outcome of a market that has passed expiration.
    ///
    /// The outcome is derived from the reference feed's own level at expiration rather
    /// than from a venue resolution message: for a sixty-second TWAP read at expiration
    /// that level is the quantity the market resolves against, and deriving it from the
    /// same feed the decision used keeps both sides of the comparison on one basis.
    fn settle_window(&mut self, event_id: Ustr, now_ns: u64) {
        let Some(window) = self.windows.get(&event_id) else {
            return;
        };
        if window.settled {
            return;
        }
        let baseline = window.baseline;
        let skip_reason = if baseline.is_none() {
            Some("no_baseline")
        } else if !window.ordered {
            Some("no_entry")
        } else {
            None
        };
        let expiration_ns = window.expiration_ns;
        let ordered = window.ordered;
        let evaluations = window.evaluations;
        let ordered_vote = window.ordered_vote;
        let entry_price = window.entry_price;
        let fills = window.fills.clone();

        let nearest = self.nearest_reference(expiration_ns);
        let settled_reference = nearest.map(|(_, value)| value);
        let settle_offset_secs = nearest
            .map(|(ts, _)| (ts.abs_diff(expiration_ns)) as f64 / 1e9);
        let up_won = match (settled_reference, baseline) {
            (Some(settled), Some(base)) => Some(u8::from(settled > base)),
            _ => None,
        };
        let won = match (up_won, ordered_vote) {
            (Some(up), Some(vote)) => Some(match vote {
                Vote::Up => up == 1,
                Vote::Down => up == 0,
            }),
            _ => None,
        };
        let pnl_per_share = match (won, entry_price) {
            (Some(won), Some(price)) => Some(if won { 1.0 - price } else { -price }),
            _ => None,
        };
        // Account profit from what actually filled. The venue cannot settle a binary
        // outcome at expiry itself, so it is derived here from the fills and the reference
        // outcome — the same outcome the per-share figure uses, applied to real quantity.
        let filled_qty: f64 = fills.iter().map(|f| f.1).sum();
        let commission_total: f64 = fills.iter().map(|f| f.2).sum();
        let avg_fill_px = if filled_qty > 0.0 {
            Some(fills.iter().map(|f| f.0 * f.1).sum::<f64>() / filled_qty)
        } else {
            None
        };
        let realized_pnl = match (won, avg_fill_px) {
            (Some(won), Some(px)) if filled_qty > 0.0 => {
                let payoff = if won { 1.0 } else { 0.0 };
                Some(filled_qty * (payoff - px) - commission_total)
            }
            _ => None,
        };

        if let Some(journal) = self.journal.as_mut() {
            journal.write(&SettlementRecord {
                kind: "settlement",
                ts_ns: now_ns,
                event_id: event_id.as_str(),
                expiration_ns,
                evaluations,
                baseline,
                skip_reason,
                settled_reference,
                settle_offset_secs,
                up_won,
                ordered,
                ordered_vote: ordered_vote.map(Vote::label),
                entry_price,
                won,
                pnl_per_share,
                filled_qty,
                avg_fill_px,
                commission_total,
                realized_pnl,
            });
        }
        if let Some(window) = self.windows.get_mut(&event_id) {
            window.settled = true;
        }
        // Legs of a settled market will not fill again.
        self.leg_index.retain(|_, ev| *ev != event_id);
        if ordered {
            log::info!(
                "Settled event {event_id}: up_won={:?} ordered={:?} entry={:?} won={:?} \
                 pnl_per_share={:?} filled_qty={filled_qty} realized_pnl={realized_pnl:?}",
                up_won,
                ordered_vote.map(Vote::label),
                entry_price,
                won,
                pnl_per_share
            );
        } else {
            log::debug!("Settled event {event_id} without an order: up_won={up_won:?}");
        }
    }

    /// Writes a counter snapshot when the heartbeat interval has elapsed.
    fn maybe_heartbeat(&mut self, now_ns: u64) {
        let interval_ns = HEARTBEAT_INTERVAL_NS;
        if self.last_heartbeat_ns != 0 && now_ns.saturating_sub(self.last_heartbeat_ns) < interval_ns
        {
            return;
        }
        self.last_heartbeat_ns = now_ns;
        let windows_tracked = self.windows.len();
        let windows_without_baseline = self
            .windows
            .values()
            .filter(|w| w.baseline_unavailable)
            .count();
        let atr_bars = self.bars.completed().len();
        let observations = self.observations;
        let out_of_order = self.bars.out_of_order();
        let submitted = self.submitted;
        let fills = self.fills;
        let arm = self.config.arm_label.clone();
        let d = self.declines.clone();
        if let Some(journal) = self.journal.as_mut() {
            journal.write(&HeartbeatRecord {
                kind: "heartbeat",
                ts_ns: now_ns,
                arm,
                fills,
                reference_observations: observations,
                out_of_order,
                atr_bars,
                windows_tracked,
                windows_without_baseline,
                submitted,
                declines_no_atr: d.no_atr,
                declines_no_baseline: d.no_baseline,
                declines_no_leg: d.no_leg,
                declines_no_quote: d.no_quote,
                declines_price_bounds: d.price_bounds,
                declines_undecided: d.undecided,
                declines_rule_filtered: d.rule_filtered,
                declines_unpaired: d.unpaired,
            });
        }
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
        // The adapter re-publishes definitions of markets that have already expired. Left
        // unchecked, an expired market re-enters as a fresh window with no baseline, is
        // settled again as soon as it is seen, and leaves a duplicate record each time.
        let now_ns = self.clock().timestamp_ns().as_u64();
        if expiration_ns <= now_ns {
            return;
        }
        let interval_ns = self.config.interval_secs.max(1) * 1_000_000_000;
        if expiration_ns <= interval_ns {
            log::warn!(
                "Skipping {}: expiration {expiration_ns} is not past one interval",
                binary.id
            );
            return;
        }
        // Derived, not read: the venue's activation field is the market's creation
        // instant, which for a recurring series precedes the interval it trades by
        // weeks, and using it would place the baseline lookup outside any observation.
        let activation_ns = expiration_ns - interval_ns;
        let venue_activation_ns = binary.activation_ns.as_u64();
        if venue_activation_ns != 0 {
            let skew_secs = venue_activation_ns.abs_diff(activation_ns) / 1_000_000_000;
            if skew_secs > self.config.interval_secs {
                log::debug!(
                    "Venue activation for {} is {skew_secs}s from the derived open; \
                     using the derived value",
                    binary.id
                );
            }
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
            evaluations: 0,
            ordered_vote: None,
            entry_price: None,
            settled: false,
            fills: Vec::new(),
        });
        match lower.as_str() {
            "up" | "yes" => window.up = Some(leg),
            "down" | "no" => window.down = Some(leg),
            other => {
                log::warn!("Skipping {}: unrecognized outcome '{other}'", binary.id);
                return;
            }
        }
        self.leg_index.insert(binary.id, event_id);
        self.subscribe_quotes(binary.id, None, None);
        // Trades feed the simulated exchange's matching engine: a resting bid is only
        // filled when a sell-side trade prints at or through it, and without trade data the
        // engine can only fill on a quote crossing, which a resting order almost never sees.
        self.subscribe_trades(binary.id, None, None);
        log::info!(
            "Registered {instrument_id} as '{lower}' leg of event {event_id}, \
             expires at {expiration_ns}",
            instrument_id = binary.id
        );
    }
}

nautilus_strategy!(AtrMarginBinary, {
    fn on_order_filled(&mut self, event: &OrderFilled) {
        self.fills += 1;
        let Some(event_id) = self.leg_index.get(&event.instrument_id).copied() else {
            log::warn!(
                "Fill on {} could not be attributed to a tracked market",
                event.instrument_id
            );
            return;
        };
        let px = event.last_px.as_f64();
        let qty = event.last_qty.as_f64();
        let commission = event.commission.map_or(0.0, |m| m.as_f64());
        let side = match event.order_side {
            OrderSide::Buy => "buy",
            OrderSide::Sell => "sell",
        };
        let liquidity = match event.liquidity_side {
            LiquiditySide::Maker => "maker",
            LiquiditySide::Taker => "taker",
            LiquiditySide::NoLiquiditySide => "none",
        };
        if let Some(window) = self.windows.get_mut(&event_id) {
            window.fills.push((px, qty, commission));
        }
        let ts_ns = event.ts_event.as_u64();
        if let Some(journal) = self.journal.as_mut() {
            journal.write(&FillRecord {
                kind: "fill",
                ts_ns,
                event_id: event_id.as_str(),
                instrument_id: event.instrument_id.to_string(),
                side,
                px,
                qty,
                liquidity_side: liquidity,
                commission,
            });
        }
        log::info!(
            "Filled {} {qty} @ {px} ({liquidity}) on event {event_id}, commission={commission}",
            event.instrument_id
        );
    }

    fn on_position_opened(&mut self, event: PositionOpened) {
        log::info!(
            "Position opened {} qty={} avg_px={}",
            event.instrument_id,
            event.quantity,
            event.avg_px_open
        );
    }

    fn on_position_closed(&mut self, event: PositionClosed) {
        log::info!(
            "Position closed {} realized={:?}",
            event.instrument_id,
            event.realized_pnl
        );
    }
});

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
        if let Some(journal) = self.journal.as_ref() {
            log::info!(
                "Journal {}: {} records written, {} failed",
                journal.path().display(),
                journal.written(),
                journal.failures()
            );
        }
        let d = self.declines().clone();
        log::info!(
            "Stopped: submitted={} fills={} bars={} declines[no_atr={} no_baseline={} \
             no_leg={} no_quote={} price_bounds={} undecided={} rule_filtered={} unpaired={}]",
            self.submitted(),
            self.fills,
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
        self.maybe_heartbeat(now_ns);
        self.evaluate_due(now_ns)
    }

    fn on_reset(&mut self) -> anyhow::Result<()> {
        self.reference_history.clear();
        self.bars.clear();
        self.windows.clear();
        self.submitted = 0;
        self.fills = 0;
        self.leg_index.clear();
        self.observations = 0;
        self.last_heartbeat_ns = 0;
        self.declines = DeclineCounts::default();
        Ok(())
    }
}
