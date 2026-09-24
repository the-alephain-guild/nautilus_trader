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

//! Decision kernel for the ATR-normalized margin strategy.
//!
//! Kept free of engine types so the rules can be exercised directly. A rule that can only
//! be reached through a running engine tends to be verified only where an outer guard
//! already decided the outcome, which leaves the inner branch untested.

use std::collections::VecDeque;

use rust_decimal::{Decimal, prelude::ToPrimitive};

/// Which side the rules favour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Vote {
    /// Buy the "up" outcome.
    Up,
    /// Buy the "down" outcome.
    Down,
}

impl Vote {
    /// Returns a stable label for logging and journalling.
    #[must_use]
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Up => "up",
            Self::Down => "down",
        }
    }
}

/// The rule that produced a vote, or why no vote was produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Verdict {
    /// Lead exceeds the downside ATR band: the result is held to be settled.
    LeadThick,
    /// Lead is thinner than the downside ATR band: a routine decline reverses it.
    LeadThin,
    /// Deficit is inside the upside ATR band: a routine advance recovers it.
    GapNear,
    /// Deficit exceeds the upside ATR band: recovery is held to be out of reach.
    GapFar,
    /// Ratio fell in the no-trade band between the thresholds.
    Undecided,
    /// The relevant ATR was zero or unavailable, so the ratio is undefined.
    NoAtr,
    /// Current and reference prices are exactly equal.
    Level,
}

impl Verdict {
    /// Returns the vote this verdict implies, if any.
    #[must_use]
    pub(crate) const fn vote(self) -> Option<Vote> {
        match self {
            Self::LeadThick | Self::GapNear => Some(Vote::Up),
            Self::LeadThin | Self::GapFar => Some(Vote::Down),
            Self::Undecided | Self::NoAtr | Self::Level => None,
        }
    }

    /// Returns whether this verdict came from the thick-lead rule.
    #[must_use]
    pub(crate) const fn is_thick_lead(self) -> bool {
        matches!(self, Self::LeadThick)
    }

    /// Returns a stable label for logging and metrics.
    #[must_use]
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::LeadThick => "lead_thick",
            Self::LeadThin => "lead_thin",
            Self::GapNear => "gap_near",
            Self::GapFar => "gap_far",
            Self::Undecided => "undecided",
            Self::NoAtr => "no_atr",
            Self::Level => "level",
        }
    }
}

/// Rule thresholds, expressed as multiples of the directional ATR.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Thresholds {
    /// Lead ratio at or above which the lead is safe.
    pub(crate) lead_thick: f64,
    /// Lead ratio at or below which the lead is fragile.
    pub(crate) lead_thin: f64,
    /// Gap ratio at or below which the deficit is recoverable.
    pub(crate) gap_near: f64,
    /// Gap ratio at or above which the deficit is unrecoverable.
    pub(crate) gap_far: f64,
}

impl Default for Thresholds {
    fn default() -> Self {
        Self {
            lead_thick: 1.05,
            lead_thin: 0.90,
            gap_near: 0.80,
            gap_far: 1.04,
        }
    }
}

/// Outcome of one evaluation, including the ratio that produced it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Decision {
    /// Which rule fired, or why none did.
    pub(crate) verdict: Verdict,
    /// Distance to the reference divided by the scaled directional ATR.
    ///
    /// `None` when the ratio is undefined.
    pub(crate) ratio: Option<f64>,
}

/// Evaluates the rules for one observation.
///
/// `reference` is the settlement baseline and `current` the latest observation of the
/// same feed. Both must come from the feed the market resolves against: any constant
/// offset cancels in `current - reference`, but an offset that moves between the two
/// read times does not, and it enters the ratio undiminished.
///
/// `atr_scale` multiplies the ATR before the comparison, to match the ATR's bar length
/// to the remaining horizon.
#[must_use]
pub(crate) fn evaluate(
    reference: Decimal,
    current: Decimal,
    up_atr: Decimal,
    down_atr: Decimal,
    atr_scale: f64,
    thresholds: &Thresholds,
) -> Decision {
    if current == reference {
        return Decision {
            verdict: Verdict::Level,
            ratio: None,
        };
    }
    let leading = current > reference;
    // A lead is threatened by a decline, so it is measured against the downside ATR.
    let atr = if leading { down_atr } else { up_atr };
    if atr <= Decimal::ZERO {
        return Decision {
            verdict: Verdict::NoAtr,
            ratio: None,
        };
    }
    let distance = if leading {
        current - reference
    } else {
        reference - current
    };
    let Some(distance_f) = distance.to_f64() else {
        return Decision {
            verdict: Verdict::NoAtr,
            ratio: None,
        };
    };
    let Some(atr_f) = atr.to_f64() else {
        return Decision {
            verdict: Verdict::NoAtr,
            ratio: None,
        };
    };
    let denom = atr_f * atr_scale;
    if denom <= 0.0 || !denom.is_finite() {
        return Decision {
            verdict: Verdict::NoAtr,
            ratio: None,
        };
    }
    let ratio = distance_f / denom;
    if !ratio.is_finite() {
        return Decision {
            verdict: Verdict::NoAtr,
            ratio: None,
        };
    }
    let verdict = if leading {
        if ratio >= thresholds.lead_thick {
            Verdict::LeadThick
        } else if ratio <= thresholds.lead_thin {
            Verdict::LeadThin
        } else {
            Verdict::Undecided
        }
    } else if ratio <= thresholds.gap_near {
        Verdict::GapNear
    } else if ratio >= thresholds.gap_far {
        Verdict::GapFar
    } else {
        Verdict::Undecided
    };
    Decision {
        verdict,
        ratio: Some(ratio),
    }
}

/// Scale factor applied to a one-bar ATR for a horizon of `remaining_secs`.
///
/// Comparing a one-minute ATR against a two-minute horizon understates the reachable
/// range; the square-root of time is the matching correction under a random walk.
/// Returns `1.0` when scaling is disabled or the inputs are degenerate.
#[must_use]
pub(crate) fn atr_scale(remaining_secs: f64, bar_secs: f64, enabled: bool) -> f64 {
    if !enabled || bar_secs <= 0.0 || remaining_secs <= 0.0 {
        return 1.0;
    }
    let bars = remaining_secs / bar_secs;
    if !bars.is_finite() || bars <= 0.0 {
        return 1.0;
    }
    bars.sqrt()
}

/// One completed bar of the reference feed, retaining the extremes each direction reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DirectionalBar {
    /// Bucket start in UNIX nanoseconds.
    pub(crate) start_ns: u64,
    /// First observation in the bucket.
    pub(crate) open: Decimal,
    /// Highest observation in the bucket.
    pub(crate) high: Decimal,
    /// Lowest observation in the bucket.
    pub(crate) low: Decimal,
    /// Most recent observation in the bucket.
    pub(crate) close: Decimal,
    /// Observations folded into the bucket.
    pub(crate) observations: u32,
}

impl DirectionalBar {
    /// Creates a bar seeded by its first observation.
    #[must_use]
    pub(crate) const fn seed(start_ns: u64, value: Decimal) -> Self {
        Self {
            start_ns,
            open: value,
            high: value,
            low: value,
            close: value,
            observations: 1,
        }
    }

    /// Folds one further observation into the bar.
    pub(crate) fn update(&mut self, value: Decimal) {
        if value > self.high {
            self.high = value;
        }
        if value < self.low {
            self.low = value;
        }
        self.close = value;
        self.observations += 1;
    }

    /// Distance the bar travelled above its open.
    #[must_use]
    pub(crate) fn upside(&self) -> Decimal {
        self.high - self.open
    }

    /// Distance the bar travelled below its open.
    #[must_use]
    pub(crate) fn downside(&self) -> Decimal {
        self.open - self.low
    }
}

/// Mean upside and downside excursion over completed bars.
///
/// A conventional ATR is direction-agnostic; the rules here need the two directions
/// separately, because a lead is threatened only by a decline and a deficit only by an
/// advance. Returns `None` until `min_bars` bars are available.
#[must_use]
pub(crate) fn directional_atr(
    bars: &VecDeque<DirectionalBar>,
    min_bars: usize,
) -> Option<(Decimal, Decimal)> {
    if bars.len() < min_bars.max(1) {
        return None;
    }
    let n = Decimal::from(bars.len());
    let up: Decimal = bars.iter().map(DirectionalBar::upside).sum();
    let down: Decimal = bars.iter().map(DirectionalBar::downside).sum();
    Some((up / n, down / n))
}

/// Accumulates reference observations into fixed-length bars.
///
/// Buckets on absolute time so bar boundaries align with the venue's own minute
/// boundaries rather than with whenever the strategy happened to start; two runs begun
/// seconds apart otherwise compute different ATRs from identical input.
///
/// A completed bar holding fewer than `min_observations` points is dropped: the feed
/// pauses for tens of seconds now and then, and a bar assembled from the few points
/// around such a pause has extremes that describe the gap, not the market.
#[derive(Debug)]
pub(crate) struct BarAccumulator {
    bucket_ns: u64,
    capacity: usize,
    min_observations: u32,
    completed: VecDeque<DirectionalBar>,
    current: Option<DirectionalBar>,
    last_ts_ns: Option<u64>,
    /// Observations rejected for arriving out of order.
    out_of_order: u64,
    /// Completed bars dropped for holding too few observations.
    sparse_dropped: u64,
}

impl BarAccumulator {
    /// Creates an accumulator for bars of `bar_secs` retaining `capacity` completed bars,
    /// keeping only bars with at least `min_observations` points.
    #[must_use]
    pub(crate) fn new(bar_secs: u64, capacity: usize, min_observations: u32) -> Self {
        Self {
            bucket_ns: bar_secs.max(1) * 1_000_000_000,
            capacity: capacity.max(1),
            min_observations: min_observations.max(1),
            completed: VecDeque::new(),
            current: None,
            last_ts_ns: None,
            out_of_order: 0,
            sparse_dropped: 0,
        }
    }

    /// Folds one observation in, returning whether it was accepted.
    ///
    /// The adapter suppresses replays, so an out-of-order point means the feed itself is
    /// inconsistent; admitting it would corrupt the extremes the ATR is computed from.
    pub(crate) fn push(&mut self, ts_event_ns: u64, value: Decimal) -> bool {
        if let Some(last) = self.last_ts_ns
            && ts_event_ns < last
        {
            self.out_of_order += 1;
            return false;
        }
        self.last_ts_ns = Some(ts_event_ns);
        let start_ns = ts_event_ns - (ts_event_ns % self.bucket_ns);
        match self.current.as_mut() {
            Some(bar) if bar.start_ns == start_ns => bar.update(value),
            Some(bar) => {
                let finished = *bar;
                if finished.observations < self.min_observations {
                    self.sparse_dropped += 1;
                } else {
                    self.completed.push_back(finished);
                    while self.completed.len() > self.capacity {
                        self.completed.pop_front();
                    }
                }
                self.current = Some(DirectionalBar::seed(start_ns, value));
            }
            None => self.current = Some(DirectionalBar::seed(start_ns, value)),
        }
        true
    }

    /// Returns the completed bars, oldest first.
    #[must_use]
    pub(crate) const fn completed(&self) -> &VecDeque<DirectionalBar> {
        &self.completed
    }

    /// Returns how many observations were rejected for arriving out of order.
    #[must_use]
    pub(crate) const fn out_of_order(&self) -> u64 {
        self.out_of_order
    }

    /// Returns how many completed bars were dropped for holding too few observations.
    #[must_use]
    pub(crate) const fn sparse_dropped(&self) -> u64 {
        self.sparse_dropped
    }

    /// Clears all state.
    pub(crate) fn clear(&mut self) {
        self.completed.clear();
        self.current = None;
        self.last_ts_ns = None;
        self.out_of_order = 0;
        self.sparse_dropped = 0;
    }
}

/// Break-even win rate for an entry at `price`.
///
/// A win returns `1 - price` and a loss costs `price`, so the expected value is zero
/// exactly at a win rate of `price`. Stated explicitly because the payoff ratio
/// degrades sharply as price approaches 1.
#[must_use]
pub(crate) fn break_even_win_rate(price: f64) -> f64 {
    price.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use rstest::rstest;
    use rust_decimal_macros::dec;

    use super::*;

    const ATR: Decimal = dec!(40);

    fn decide(reference: Decimal, current: Decimal) -> Decision {
        evaluate(reference, current, ATR, ATR, 1.0, &Thresholds::default())
    }

    #[rstest]
    // Leading by more than the downside band: the lead is held safe.
    #[case(dec!(100), dec!(142), Verdict::LeadThick, Some(Vote::Up))]
    // Leading by less than the band: a routine decline reverses it.
    #[case(dec!(100), dec!(130), Verdict::LeadThin, Some(Vote::Down))]
    // Trailing by less than the upside band: a routine advance recovers it.
    #[case(dec!(100), dec!(70), Verdict::GapNear, Some(Vote::Up))]
    // Trailing by more than the band: recovery is out of reach.
    #[case(dec!(100), dec!(50), Verdict::GapFar, Some(Vote::Down))]
    // Inside either no-trade band: no order.
    #[case(dec!(100), dec!(139), Verdict::Undecided, None)]
    #[case(dec!(100), dec!(63), Verdict::Undecided, None)]
    fn rule_produces_expected_vote(
        #[case] reference: Decimal,
        #[case] current: Decimal,
        #[case] expected: Verdict,
        #[case] vote: Option<Vote>,
    ) {
        let decision = decide(reference, current);
        assert_eq!(decision.verdict, expected);
        assert_eq!(decision.verdict.vote(), vote);
    }

    #[rstest]
    fn equal_levels_produce_no_vote() {
        let decision = decide(dec!(100), dec!(100));
        assert_eq!(decision.verdict, Verdict::Level);
        assert!(decision.ratio.is_none());
    }

    #[rstest]
    #[case(Decimal::ZERO)]
    #[case(dec!(-1))]
    fn non_positive_atr_produces_no_vote(#[case] atr: Decimal) {
        let decision = evaluate(dec!(100), dec!(150), atr, atr, 1.0, &Thresholds::default());
        assert_eq!(decision.verdict, Verdict::NoAtr);
        assert!(decision.ratio.is_none());
    }

    /// A lead must be judged against the downside ATR and a deficit against the upside
    /// one. Supplying asymmetric ATRs pins that wiring: were the two swapped, a lead of
    /// 42 measured against the upside ATR of 10 would read as a ratio of 4.2 and still
    /// return `LeadThick`, so a symmetric fixture could not detect the error.
    #[rstest]
    fn lead_uses_downside_atr_and_deficit_uses_upside() {
        let thresholds = Thresholds::default();
        // Lead of 42 against a downside ATR of 40 is 1.05 -> thick.
        let lead = evaluate(dec!(100), dec!(142), dec!(10), dec!(40), 1.0, &thresholds);
        assert_eq!(lead.verdict, Verdict::LeadThick);
        // Deficit of 30 against an upside ATR of 40 is 0.75 -> near.
        let deficit = evaluate(dec!(100), dec!(70), dec!(40), dec!(10), 1.0, &thresholds);
        assert_eq!(deficit.verdict, Verdict::GapNear);
    }

    /// Enumerates the ratio axis on both sides and asserts that the verdict each ratio
    /// receives is the one the thresholds define. Checking only that every rule is
    /// reachable would pass even if two rules' domains overlapped or left a hole.
    #[rstest]
    fn verdict_domains_are_exhaustive_and_disjoint() {
        let t = Thresholds::default();
        let mut seen_lead = [0_u32; 3];
        let mut seen_gap = [0_u32; 3];
        for step in 0..=400 {
            let ratio = f64::from(step) / 100.0;
            let distance = Decimal::try_from(ratio * 40.0).expect("finite distance");

            let lead = evaluate(dec!(100), dec!(100) + distance, ATR, ATR, 1.0, &t);
            let expected_lead = if ratio >= t.lead_thick {
                Verdict::LeadThick
            } else if ratio <= t.lead_thin {
                Verdict::LeadThin
            } else {
                Verdict::Undecided
            };
            if distance > Decimal::ZERO {
                assert_eq!(
                    lead.verdict, expected_lead,
                    "lead ratio {ratio} landed on {:?}",
                    lead.verdict
                );
                match lead.verdict {
                    Verdict::LeadThick => seen_lead[0] += 1,
                    Verdict::LeadThin => seen_lead[1] += 1,
                    Verdict::Undecided => seen_lead[2] += 1,
                    other => panic!("unexpected lead verdict {other:?}"),
                }
            }

            let gap = evaluate(dec!(100), dec!(100) - distance, ATR, ATR, 1.0, &t);
            let expected_gap = if ratio <= t.gap_near {
                Verdict::GapNear
            } else if ratio >= t.gap_far {
                Verdict::GapFar
            } else {
                Verdict::Undecided
            };
            if distance > Decimal::ZERO {
                assert_eq!(
                    gap.verdict, expected_gap,
                    "gap ratio {ratio} landed on {:?}",
                    gap.verdict
                );
                match gap.verdict {
                    Verdict::GapNear => seen_gap[0] += 1,
                    Verdict::GapFar => seen_gap[1] += 1,
                    Verdict::Undecided => seen_gap[2] += 1,
                    other => panic!("unexpected gap verdict {other:?}"),
                }
            }
        }
        // Every branch, including both no-trade bands, must be reachable.
        assert!(
            seen_lead.iter().all(|&n| n > 0),
            "lead branches {seen_lead:?}"
        );
        assert!(seen_gap.iter().all(|&n| n > 0), "gap branches {seen_gap:?}");
    }

    #[rstest]
    #[case(60.0, 60.0, true, 1.0)]
    #[case(240.0, 60.0, true, 2.0)]
    #[case(240.0, 60.0, false, 1.0)]
    #[case(0.0, 60.0, true, 1.0)]
    #[case(60.0, 0.0, true, 1.0)]
    fn atr_scale_follows_root_of_time(
        #[case] remaining: f64,
        #[case] bar: f64,
        #[case] enabled: bool,
        #[case] expected: f64,
    ) {
        assert!((atr_scale(remaining, bar, enabled) - expected).abs() < 1e-9);
    }

    /// Scaling must make a wider horizon harder to clear, not easier.
    #[rstest]
    fn scaling_dampens_ratio_over_longer_horizons() {
        let t = Thresholds::default();
        let unscaled = evaluate(dec!(100), dec!(142), ATR, ATR, 1.0, &t);
        let scaled = evaluate(
            dec!(100),
            dec!(142),
            ATR,
            ATR,
            atr_scale(240.0, 60.0, true),
            &t,
        );
        assert_eq!(unscaled.verdict, Verdict::LeadThick);
        assert!(scaled.ratio.expect("ratio") < unscaled.ratio.expect("ratio"));
        assert_ne!(scaled.verdict, Verdict::LeadThick);
    }

    #[rstest]
    fn bar_retains_both_extremes() {
        let mut bar = DirectionalBar::seed(0, dec!(100));
        bar.update(dec!(110));
        bar.update(dec!(95));
        bar.update(dec!(104));
        assert_eq!(bar.open, dec!(100));
        assert_eq!(bar.high, dec!(110));
        assert_eq!(bar.low, dec!(95));
        assert_eq!(bar.close, dec!(104));
        assert_eq!(bar.upside(), dec!(10));
        assert_eq!(bar.downside(), dec!(5));
    }

    #[rstest]
    fn directional_atr_separates_the_two_directions() {
        let mut bars = VecDeque::new();
        // Two bars that rise 20 and fall 4; a direction-agnostic ATR would blend them.
        for _ in 0..2 {
            let mut bar = DirectionalBar::seed(0, dec!(100));
            bar.update(dec!(120));
            bar.update(dec!(96));
            bars.push_back(bar);
        }
        let (up, down) = directional_atr(&bars, 2).expect("atr");
        assert_eq!(up, dec!(20));
        assert_eq!(down, dec!(4));
    }

    #[rstest]
    fn directional_atr_withholds_until_minimum_bars() {
        let mut bars = VecDeque::new();
        bars.push_back(DirectionalBar::seed(0, dec!(100)));
        assert!(directional_atr(&bars, 2).is_none());
        bars.push_back(DirectionalBar::seed(60_000_000_000, dec!(100)));
        assert!(directional_atr(&bars, 2).is_some());
    }

    /// The break-even win rate equals the entry price, which is why entries near 1 are
    /// refused: at 0.95 a strategy must be right 95% of the time merely to break even.
    #[rstest]
    #[case(0.50, 0.50)]
    #[case(0.95, 0.95)]
    #[case(1.20, 1.00)]
    #[case(-0.10, 0.00)]
    fn break_even_win_rate_equals_price(#[case] price: f64, #[case] expected: f64) {
        assert!((break_even_win_rate(price) - expected).abs() < 1e-9);
    }

    const SEC: u64 = 1_000_000_000;

    /// Bar boundaries must follow absolute time, not the first observation's offset.
    #[rstest]
    fn accumulator_buckets_on_absolute_time() {
        let mut acc = BarAccumulator::new(60, 10, 1);
        // Start mid-minute: 90s, 100s, 110s all belong to the 60..120 bucket.
        assert!(acc.push(90 * SEC, dec!(100)));
        assert!(acc.push(100 * SEC, dec!(110)));
        assert!(acc.push(110 * SEC, dec!(95)));
        assert!(acc.completed().is_empty(), "bucket not yet crossed");
        // 120s opens the next bucket and closes the previous one.
        assert!(acc.push(120 * SEC, dec!(101)));
        assert_eq!(acc.completed().len(), 1);
        let bar = acc.completed()[0];
        assert_eq!(bar.start_ns, 60 * SEC);
        assert_eq!(bar.open, dec!(100));
        assert_eq!(bar.high, dec!(110));
        assert_eq!(bar.low, dec!(95));
    }

    #[rstest]
    fn accumulator_rejects_out_of_order_observations() {
        let mut acc = BarAccumulator::new(60, 10, 1);
        assert!(acc.push(120 * SEC, dec!(100)));
        assert!(
            !acc.push(119 * SEC, dec!(999)),
            "older point must be refused"
        );
        assert_eq!(acc.out_of_order(), 1);
        // The refused value must not have touched the extremes.
        assert!(acc.push(180 * SEC, dec!(100)));
        assert_eq!(acc.completed()[0].high, dec!(100));
    }

    #[rstest]
    fn accumulator_bounds_retained_bars() {
        let mut acc = BarAccumulator::new(60, 3, 1);
        for i in 0..8 {
            acc.push(i * 60 * SEC, dec!(100));
        }
        assert_eq!(acc.completed().len(), 3, "capacity must bound growth");
    }

    #[rstest]
    fn accumulator_clear_resets_everything() {
        let mut acc = BarAccumulator::new(60, 5, 1);
        acc.push(60 * SEC, dec!(100));
        acc.push(180 * SEC, dec!(100));
        acc.push(120 * SEC, dec!(1));
        acc.clear();
        assert!(acc.completed().is_empty());
        assert_eq!(acc.out_of_order(), 0);
        assert!(
            acc.push(1 * SEC, dec!(100)),
            "clear must reset the ordering watermark"
        );
    }

    /// A bar spanning a feed gap must not reach the ATR; the extremes of two points
    /// around a pause describe the pause, not the market.
    #[rstest]
    fn accumulator_drops_sparse_bars() {
        let mut acc = BarAccumulator::new(60, 10, 3);
        // Bucket 60..120 receives three points: kept.
        assert!(acc.push(60 * SEC, dec!(100)));
        assert!(acc.push(70 * SEC, dec!(130)));
        assert!(acc.push(80 * SEC, dec!(90)));
        // Bucket 120..180 receives a single point across a gap: dropped on close.
        assert!(acc.push(150 * SEC, dec!(500)));
        assert!(acc.push(180 * SEC, dec!(100)));
        assert_eq!(
            acc.completed().len(),
            1,
            "only the well-populated bar survives"
        );
        assert_eq!(acc.completed()[0].observations, 3);
        assert_eq!(acc.completed()[0].high, dec!(130));
        assert_eq!(acc.sparse_dropped(), 1);
        // Threshold 1 keeps every bar, matching the previous behaviour.
        let mut lax = BarAccumulator::new(60, 10, 1);
        lax.push(150 * SEC, dec!(500));
        lax.push(180 * SEC, dec!(100));
        assert_eq!(lax.completed().len(), 1);
        assert_eq!(lax.sparse_dropped(), 0);
    }
}
