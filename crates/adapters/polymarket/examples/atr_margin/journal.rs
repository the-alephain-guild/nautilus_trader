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

//! Append-only journal of every evaluation and every settlement.
//!
//! Records **all** evaluations, not just the ones that placed an order, because the
//! trigger rate and the distribution of refusals are as much a part of the result as the
//! fills. Settlement is journalled separately: the outcome is only knowable once the
//! market expires, and a decision line written earlier cannot carry it.
//!
//! Each line is flushed on write. A buffered journal that loses its tail is worse than
//! none, since the loss is silent and the file still looks well-formed.

use std::{
    fs::{File, OpenOptions},
    io::{BufWriter, Write},
    path::{Path, PathBuf},
};

use rust_decimal::Decimal;
use serde::Serialize;

/// One evaluation at a decision point.
#[derive(Debug, Serialize)]
pub(crate) struct DecisionRecord<'a> {
    /// Always `"decision"`, so a consumer can split the two record kinds.
    pub kind: &'static str,
    /// Engine clock at evaluation, UNIX nanoseconds.
    pub ts_ns: u64,
    pub event_id: &'a str,
    pub expiration_ns: u64,
    /// 1-based index of this evaluation within the market.
    ///
    /// A refusal leaves the market open to re-evaluation, so several records can share an
    /// event; the index lets a consumer collapse them instead of counting them as
    /// independent observations.
    pub evaluation_index: u32,
    /// Seconds remaining until expiration at evaluation.
    pub remaining_secs: f64,
    /// Settlement baseline: the reference level at the market's activation instant.
    pub baseline: Decimal,
    /// Latest reference observation.
    pub current: Decimal,
    pub up_atr: Decimal,
    pub down_atr: Decimal,
    /// Multiplier applied to the ATR for the remaining horizon.
    pub atr_scale: f64,
    /// Completed bars behind the ATR.
    pub atr_bars: usize,
    /// Distance divided by the scaled directional ATR.
    pub ratio: Option<f64>,
    /// Which rule fired, or why none did.
    pub verdict: &'static str,
    /// Side the rules favoured, if any.
    pub vote: Option<&'static str>,
    /// `"submitted"` or `"declined"`.
    pub action: &'static str,
    /// Populated when `action` is `"declined"`.
    pub decline_reason: Option<&'static str>,
    pub leg_instrument_id: Option<String>,
    /// Price the order was placed at, or the price that failed the bounds check.
    pub entry_price: Option<f64>,
    pub bid: Option<f64>,
    pub ask: Option<f64>,
    /// Win rate at which this entry breaks even, which equals the entry price.
    pub break_even_win_rate: Option<f64>,
    pub trade_size: Option<f64>,
    /// Whether the order rested as post-only rather than crossing.
    pub post_only: bool,
}

/// The outcome of a market this strategy evaluated.
///
/// `settled_reference` is the reference feed's own value at expiration. For a sixty-second
/// TWAP read at expiration this is the quantity the market resolves against, so the
/// outcome is derived rather than guessed — and it is derived from the same feed the
/// decision used, which keeps the two sides of the comparison on one basis.
#[derive(Debug, Serialize)]
pub(crate) struct SettlementRecord<'a> {
    /// Always `"settlement"`.
    pub kind: &'static str,
    pub ts_ns: u64,
    pub event_id: &'a str,
    pub expiration_ns: u64,
    /// Total evaluations this market received.
    pub evaluations: u32,
    /// Settlement baseline, or `None` when it could never be established.
    pub baseline: Option<Decimal>,
    /// Why the market was not traded, when it was not.
    pub skip_reason: Option<&'static str>,
    /// Reference level at expiration, or `None` if no observation landed close enough.
    pub settled_reference: Option<Decimal>,
    /// Seconds between the nearest observation and expiration.
    pub settle_offset_secs: Option<f64>,
    /// `1` when the up outcome won, `0` when the down outcome won.
    pub up_won: Option<u8>,
    /// Whether this strategy had placed an order on the market.
    pub ordered: bool,
    /// The side ordered, when one was.
    pub ordered_vote: Option<&'static str>,
    /// Price paid, when an order was placed.
    pub entry_price: Option<f64>,
    /// Whether the ordered side won. `None` when nothing was ordered.
    pub won: Option<bool>,
    /// Realized profit per share: `(won ? 1 : 0) - entry_price`.
    pub pnl_per_share: Option<f64>,
}

/// Periodic snapshot of the run's counters.
///
/// Written on a timer rather than only at shutdown: a run ended by a signal never
/// reaches its stop handler, and counters that exist only in memory are lost exactly
/// when the run needs explaining. Each snapshot also proves the journal is still live,
/// which an empty file cannot.
#[derive(Debug, Serialize)]
pub(crate) struct HeartbeatRecord {
    /// Always `"heartbeat"`.
    pub kind: &'static str,
    pub ts_ns: u64,
    /// Reference observations accepted so far.
    pub reference_observations: u64,
    /// Observations rejected for arriving out of order.
    pub out_of_order: u64,
    /// Completed bars behind the ATR.
    pub atr_bars: usize,
    /// Markets currently tracked.
    pub windows_tracked: usize,
    /// Markets whose baseline could never be established.
    pub windows_without_baseline: usize,
    pub submitted: u64,
    pub declines_no_atr: u64,
    pub declines_no_baseline: u64,
    pub declines_no_leg: u64,
    pub declines_no_quote: u64,
    pub declines_price_bounds: u64,
    pub declines_undecided: u64,
    pub declines_rule_filtered: u64,
    pub declines_unpaired: u64,
}

/// Append-only JSONL writer.
#[derive(Debug)]
pub(crate) struct DecisionJournal {
    writer: Option<BufWriter<File>>,
    path: PathBuf,
    written: u64,
    failures: u64,
}

impl DecisionJournal {
    /// Opens `path` for appending, creating it when absent.
    ///
    /// A journal that cannot be opened is reported once and then disabled: the run is
    /// still worth continuing, but it must not look as though records were kept.
    #[must_use]
    pub(crate) fn open(path: impl AsRef<Path>) -> Self {
        let path = path.as_ref().to_path_buf();
        match OpenOptions::new().create(true).append(true).open(&path) {
            Ok(file) => {
                log::info!("Journalling decisions to {}", path.display());
                Self {
                    writer: Some(BufWriter::new(file)),
                    path,
                    written: 0,
                    failures: 0,
                }
            }
            Err(e) => {
                log::error!(
                    "Cannot open journal {}: {e}; decisions will not be recorded",
                    path.display()
                );
                Self {
                    writer: None,
                    path,
                    written: 0,
                    failures: 1,
                }
            }
        }
    }

    /// Serializes one record and flushes it.
    pub(crate) fn write<T: Serialize>(&mut self, record: &T) {
        let Some(writer) = self.writer.as_mut() else {
            return;
        };
        let line = match serde_json::to_string(record) {
            Ok(line) => line,
            Err(e) => {
                self.failures += 1;
                log::error!("Cannot serialize journal record: {e}");
                return;
            }
        };
        // Flushed per line: an unflushed tail is lost without any sign that it existed.
        if let Err(e) = writeln!(writer, "{line}").and_then(|()| writer.flush()) {
            self.failures += 1;
            log::error!("Cannot write to journal {}: {e}", self.path.display());
            return;
        }
        self.written += 1;
    }

    /// Returns how many records were written.
    #[must_use]
    pub(crate) const fn written(&self) -> u64 {
        self.written
    }

    /// Returns how many records could not be written.
    #[must_use]
    pub(crate) const fn failures(&self) -> u64 {
        self.failures
    }

    /// Returns the journal path.
    #[must_use]
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;
    use rust_decimal_macros::dec;

    use super::*;

    fn sample_decision<'a>(event_id: &'a str) -> DecisionRecord<'a> {
        DecisionRecord {
            kind: "decision",
            ts_ns: 1_700_000_000_000_000_000,
            event_id,
            expiration_ns: 1_700_000_075_000_000_000,
            evaluation_index: 1,
            remaining_secs: 75.0,
            baseline: dec!(85000.10),
            current: dec!(85042.55),
            up_atr: dec!(41.2),
            down_atr: dec!(38.7),
            atr_scale: 1.118,
            atr_bars: 10,
            ratio: Some(1.09),
            verdict: "lead_thick",
            vote: Some("up"),
            action: "submitted",
            decline_reason: None,
            leg_instrument_id: Some("token.POLYMARKET".to_string()),
            entry_price: Some(0.93),
            bid: Some(0.93),
            ask: Some(0.94),
            break_even_win_rate: Some(0.93),
            trade_size: Some(5.0),
            post_only: true,
        }
    }

    #[rstest]
    fn writes_one_line_per_record() {
        let dir = std::env::temp_dir().join(format!("atr-journal-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("decisions.jsonl");
        let _ = std::fs::remove_file(&path);

        let mut journal = DecisionJournal::open(&path);
        journal.write(&sample_decision("1056902"));
        journal.write(&sample_decision("1056909"));
        assert_eq!(journal.written(), 2);
        assert_eq!(journal.failures(), 0);

        // Readable before the journal is dropped, which is what per-line flushing buys.
        let body = std::fs::read_to_string(&path).expect("read back");
        let lines: Vec<&str> = body.lines().collect();
        assert_eq!(lines.len(), 2);
        let parsed: serde_json::Value = serde_json::from_str(lines[0]).expect("valid json");
        assert_eq!(parsed["kind"], "decision");
        assert_eq!(parsed["event_id"], "1056902");
        assert_eq!(parsed["verdict"], "lead_thick");
        // Decimals must not be serialized as floats: a reference level rounded in the
        // journal cannot be reconciled against the feed afterwards.
        assert_eq!(parsed["baseline"], "85000.10");
        std::fs::remove_file(&path).ok();
    }

    #[rstest]
    fn appends_rather_than_truncates() {
        let dir = std::env::temp_dir().join(format!("atr-journal-append-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("decisions.jsonl");
        let _ = std::fs::remove_file(&path);

        DecisionJournal::open(&path).write(&sample_decision("first"));
        // A restart must not discard what an earlier run recorded.
        DecisionJournal::open(&path).write(&sample_decision("second"));

        let body = std::fs::read_to_string(&path).expect("read back");
        assert_eq!(body.lines().count(), 2);
        assert!(body.contains("\"first\""));
        assert!(body.contains("\"second\""));
        std::fs::remove_file(&path).ok();
    }

    #[rstest]
    fn unopenable_journal_disables_itself_without_panicking() {
        // A directory path can never be opened as a file.
        let dir = std::env::temp_dir().join(format!("atr-journal-dir-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let mut journal = DecisionJournal::open(&dir);
        journal.write(&sample_decision("x"));
        assert_eq!(journal.written(), 0, "must not claim to have written");
        assert!(journal.failures() > 0, "failure must be visible");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[rstest]
    fn settlement_record_carries_derived_outcome() {
        let record = SettlementRecord {
            kind: "settlement",
            ts_ns: 1_700_000_075_000_000_000,
            event_id: "1056902",
            expiration_ns: 1_700_000_075_000_000_000,
            evaluations: 3,
            baseline: Some(dec!(85000.10)),
            skip_reason: None,
            settled_reference: Some(dec!(85042.55)),
            settle_offset_secs: Some(0.4),
            up_won: Some(1),
            ordered: true,
            ordered_vote: Some("up"),
            entry_price: Some(0.93),
            won: Some(true),
            pnl_per_share: Some(0.07),
        };
        let parsed: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&record).expect("serialize"))
                .expect("valid json");
        assert_eq!(parsed["kind"], "settlement");
        assert_eq!(parsed["up_won"], 1);
        assert_eq!(parsed["won"], true);
        // 1 - 0.93; the journal is what the failure-rate statistic is computed from.
        assert!((parsed["pnl_per_share"].as_f64().expect("pnl") - 0.07).abs() < 1e-9);
    }
}
