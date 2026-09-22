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

//! Configuration for the ATR-normalized margin strategy on binary outcome markets.

use nautilus_model::{identifiers::StrategyId, types::Quantity};

use nautilus_trading::strategy::StrategyConfig;

/// Configuration for [`AtrMarginBinary`](super::AtrMarginBinary).
///
/// Thresholds default to the values the strategy was specified with. The gap between
/// `k_lead_thick` (1.05) and `k_lead_thin` (0.90), and between `k_gap_near` (0.80) and
/// `k_gap_far` (1.04), is an intentional no-trade band: ratios inside it express
/// "too close to call" and produce no order.
#[derive(Debug, Clone, bon::Builder)]
/// Legs are not configured here: each five-minute market is a distinct instrument pair,
/// so the strategy pairs them at runtime from the venue's event identifier and outcome
/// label. A down vote buys the down token rather than selling the up token, because under
/// the conditional token framework a naked sale would require splitting collateral first.
pub(crate) struct AtrMarginBinaryConfig {
    /// Base strategy configuration.
    #[builder(default = StrategyConfig {
        strategy_id: Some(StrategyId::from("ATR-MARGIN-BINARY-001")),
        order_id_tag: Some("001".to_string()),
        ..Default::default()
    })]
    pub(crate) base: StrategyConfig,

    /// Settlement reference symbol on the venue's price stream, e.g. `btc/usd`.
    ///
    /// This must be the same feed the market resolves against. Substituting a spot
    /// exchange feed introduces a basis that does not cancel between the baseline and
    /// the current observation when the two are read at different times.
    pub(crate) reference_symbol: String,

    /// TWAP lookback window of the reference feed in seconds. The venue accepts 30 or 60.
    #[builder(default = 60)]
    pub(crate) reference_window_seconds: u64,

    /// Order quantity per signal.
    pub(crate) trade_size: Quantity,

    /// Seconds before expiration at which the decision is evaluated.
    #[builder(default = 75)]
    pub(crate) decide_before_expiry_secs: u64,

    /// Tolerance around the decision point, in seconds.
    ///
    /// The reference feed publishes irregularly, so an exact match on
    /// `decide_before_expiry_secs` would usually miss.
    #[builder(default = 20)]
    pub(crate) decide_tolerance_secs: u64,

    /// Number of completed bars in the directional ATR lookback.
    #[builder(default = 10)]
    pub(crate) atr_bars: usize,

    /// Bar length for the directional ATR, in seconds.
    #[builder(default = 60)]
    pub(crate) atr_bar_secs: u64,

    /// Minimum completed bars required before the strategy will trade.
    #[builder(default = 5)]
    pub(crate) atr_min_bars: usize,

    /// Lead ratio at or above which a lead is treated as safe: buy up.
    #[builder(default = 1.05)]
    pub(crate) k_lead_thick: f64,

    /// Lead ratio at or below which a lead is treated as fragile: buy down.
    #[builder(default = 0.90)]
    pub(crate) k_lead_thin: f64,

    /// Gap ratio at or below which a deficit is treated as recoverable: buy up.
    #[builder(default = 0.80)]
    pub(crate) k_gap_near: f64,

    /// Gap ratio at or above which a deficit is treated as unrecoverable: buy down.
    #[builder(default = 1.04)]
    pub(crate) k_gap_far: f64,

    /// Scales the ATR by the square root of the remaining minutes.
    ///
    /// A fixed one-minute ATR compared against a multi-minute remaining horizon
    /// understates the reachable range; Brownian scaling is the matching correction.
    #[builder(default = true)]
    pub(crate) scale_atr_by_remaining: bool,

    /// Submits entries as post-only limit orders resting at the near touch.
    #[builder(default = true)]
    pub(crate) post_only: bool,

    /// Rejects an entry whose price exceeds this level.
    ///
    /// At a price of `p` a win returns `1 - p` while a loss costs `p`, so the payoff
    /// ratio degrades as `p` approaches 1 and the break-even win rate equals `p`.
    /// This bound is the control on that asymmetry.
    #[builder(default = 0.95)]
    pub(crate) max_entry_price: f64,

    /// Rejects an entry whose price falls below this level.
    #[builder(default = 0.02)]
    pub(crate) min_entry_price: f64,

    /// Restricts trading to the thick-lead rule only.
    ///
    /// Of the four rules, only the thick-lead one showed a positive point estimate in
    /// the exploratory study; the other three were negative or indistinguishable.
    #[builder(default = false)]
    pub(crate) thick_lead_only: bool,

    /// Order time-in-force expiry in seconds. `None` leaves the order resting.
    pub(crate) order_expire_secs: Option<u64>,
}
