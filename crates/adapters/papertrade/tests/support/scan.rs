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

use std::collections::BTreeMap;

use alloy_primitives::B256;
use anyhow::{Result, ensure};

#[derive(Clone, Debug)]
pub(crate) enum ScanEvent {
    Logs {
        from: u64,
        to: u64,
        intents: Vec<B256>,
        bound: u64,
        transactions: Option<Vec<B256>>,
    },
    Receipt {
        hash: B256,
        available: bool,
    },
}

/// Audits cursor claims against the RPC transcript, retaining the original fixture
/// prefixes across restarts. A saved cursor never becomes evidence of coverage.
pub(crate) struct ScanCoverage {
    initial: BTreeMap<B256, u64>,
    previous: BTreeMap<B256, u64>,
}

impl ScanCoverage {
    pub(crate) fn new(prefixes: impl IntoIterator<Item = (B256, u64)>) -> Self {
        let initial: BTreeMap<_, _> = prefixes.into_iter().collect();
        Self {
            previous: initial.clone(),
            initial,
        }
    }

    pub(crate) fn verify(
        &mut self,
        events: &[ScanEvent],
        checkpoints: &[(B256, u64, bool)],
        bound: u64,
    ) -> Result<()> {
        let mut intervals: BTreeMap<B256, Vec<(u64, u64)>> =
            self.initial.keys().map(|id| (*id, Vec::new())).collect();
        for (index, event) in events.iter().enumerate() {
            let ScanEvent::Logs {
                from,
                to,
                intents,
                bound,
                transactions,
            } = event
            else {
                continue;
            };
            ensure!(
                from <= to && to - from < 1000,
                "Invalid log page {from}..{to}"
            );
            ensure!(to <= bound, "Log page exceeds source bound: {to} > {bound}");
            let Some(transactions) = transactions else {
                continue;
            };
            // Receipt failures do not certify the log page. A successful retry
            // after the next log request must not retroactively certify it either.
            let receipts: Vec<_> = events[index + 1..]
                .iter()
                .take_while(|e| matches!(e, ScanEvent::Receipt { .. }))
                .collect();
            if !transactions.iter().all(|tx| {
                receipts.iter().any(
                    |e| matches!(e, ScanEvent::Receipt { hash, available: true } if hash == tx),
                )
            }) {
                continue;
            }
            for id in intents {
                let ranges = intervals
                    .get_mut(id)
                    .ok_or_else(|| anyhow::anyhow!("Unknown scan intent {id}"))?;
                ranges.push((*from, *to));
            }
        }
        let mut next = BTreeMap::new();
        for (id, cursor, expired) in checkpoints {
            let initial = self
                .initial
                .get(id)
                .ok_or_else(|| anyhow::anyhow!("Unknown checkpoint intent {id}"))?;
            let mut through = *initial;
            let ranges = intervals.get_mut(id).unwrap();
            ranges.sort_unstable();
            for (from, to) in ranges {
                if *from > through + 1 {
                    break;
                }
                through = through.max(*to);
            }
            ensure!(*cursor >= self.previous[id], "Cursor regressed for {id}");
            ensure!(
                *cursor <= bound,
                "Cursor exceeds source bound for {id}: {cursor} > {bound}"
            );
            ensure!(
                *cursor <= through,
                "Unscanned gap for {id}: cursor {cursor}, continuous coverage {through}"
            );
            ensure!(
                !expired || *cursor == bound,
                "Expired before source bound for {id}"
            );
            ensure!(
                next.insert(*id, *cursor).is_none(),
                "Duplicate checkpoint for {id}"
            );
        }
        ensure!(
            next.len() == self.initial.len(),
            "Missing order checkpoints"
        );
        self.previous = next;
        Ok(())
    }
}
