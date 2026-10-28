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

//! Bounded SSE decoding and atomic account snapshot/patch application.

use crate::wire::AccountSnapshot;
use alloy_primitives::Address;
use anyhow::{Result, ensure};
use serde_json::Value;

const MAX_FRAME: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct SseEvent {
    pub event: String,
    pub data: String,
}

#[derive(Debug, Default)]
pub struct SseDecoder {
    pending: Vec<u8>,
    event: String,
    data: String,
}
impl SseDecoder {
    pub fn push(&mut self, chunk: &[u8]) -> Result<Vec<SseEvent>> {
        ensure!(
            self.pending.len() + self.data.len() + chunk.len() <= MAX_FRAME,
            "SSE frame limit exceeded"
        );
        self.pending.extend_from_slice(chunk);
        let mut events = Vec::new();
        while let Some(end) = self.pending.iter().position(|b| *b == b'\n') {
            let bytes: Vec<u8> = self.pending.drain(..=end).collect();
            let line = std::str::from_utf8(&bytes)?.trim_end_matches(['\r', '\n']);
            if line.is_empty() {
                if !self.data.is_empty() {
                    events.push(SseEvent {
                        event: if self.event.is_empty() {
                            "message".into()
                        } else {
                            std::mem::take(&mut self.event)
                        },
                        data: std::mem::take(&mut self.data),
                    });
                }
                self.event.clear();
            } else if let Some(value) = line.strip_prefix("event:") {
                self.event = value.trim_start_matches(' ').to_owned();
            } else if let Some(value) = line.strip_prefix("data:") {
                if !self.data.is_empty() {
                    self.data.push('\n');
                }
                self.data.push_str(value.trim_start_matches(' '));
            }
        }
        Ok(events)
    }
}

#[derive(Debug)]
pub struct WalletState {
    wallet: Address,
    account: Option<AccountSnapshot>,
    current: bool,
    last_frame: Option<Value>,
}
impl WalletState {
    pub fn new(wallet: Address) -> Self {
        Self {
            wallet,
            account: None,
            current: false,
            last_frame: None,
        }
    }
    pub fn disconnect(&mut self) {
        self.current = false;
    }
    pub fn current(&self) -> Option<&AccountSnapshot> {
        self.account.as_ref().filter(|_| self.current)
    }
    pub fn apply(&mut self, frame: &Value) -> Result<bool> {
        let result = self.apply_checked(frame);
        if result.is_err() {
            self.current = false;
        }
        result
    }
    fn apply_checked(&mut self, frame: &Value) -> Result<bool> {
        ensure!(
            frame["address"] == serde_json::to_value(self.wallet)?,
            "Account frame wallet mismatch"
        );
        let is_patch = frame.get("baseAccountRevision").is_some();
        if self.current {
            ensure!(
                frame["accountRevision"].as_u64()
                    >= self.account.as_ref().map(|a| a.account_revision),
                "Account revision regressed within the stream"
            );
        }
        if self.current
            && frame["accountRevision"].as_u64().is_some_and(|v| {
                self.account
                    .as_ref()
                    .is_some_and(|a| v <= a.account_revision)
            })
        {
            if frame["accountRevision"].as_u64()
                == self.account.as_ref().map(|a| a.account_revision)
            {
                ensure!(
                    self.last_frame.as_ref() == Some(frame),
                    "Account revision payload conflict"
                );
            }
            return Ok(false);
        }
        let value = if is_patch {
            let account = self
                .current()
                .ok_or_else(|| anyhow::anyhow!("Patch without current snapshot"))?;
            ensure!(
                frame["address"] == serde_json::to_value(self.wallet)?,
                "Patch wallet mismatch"
            );
            ensure!(
                frame["baseAccountRevision"].as_u64() == Some(account.account_revision),
                "Account revision gap"
            );
            let mut value = serde_json::to_value(account)?;
            let state = frame["state"]
                .as_object()
                .ok_or_else(|| anyhow::anyhow!("Missing patch state"))?;
            for (key, item) in state {
                value[key] = item.clone();
            }
            value["accountRevision"] = frame["accountRevision"].clone();
            if let Some(lifetime) = frame.get("lifetime") {
                value["lifetime"] = lifetime.clone();
            }
            value["intentOutcomes"] = frame
                .get("intentOutcomes")
                .cloned()
                .unwrap_or_else(|| serde_json::json!([]));
            apply_array(
                &mut value,
                frame,
                "openPositions",
                "addedPositions",
                "removedPositionIds",
                false,
            )?;
            apply_array(
                &mut value,
                frame,
                "operations",
                "addedOperations",
                "removedOperationIds",
                true,
            )?;
            value
        } else {
            frame.clone()
        };
        let next: AccountSnapshot = serde_json::from_value(value)?;
        next.validate(self.wallet)?;
        if let Some(old) = &self.account {
            ensure!(
                next.wallet_revision >= old.wallet_revision
                    && next.queue_revision >= old.queue_revision
                    && next.indexed_through_block >= old.indexed_through_block
                    && next.indexed_through_timestamp >= old.indexed_through_timestamp,
                "Account coverage regressed"
            );
        }
        self.account = Some(next);
        self.current = true;
        self.last_frame = Some(frame.clone());
        Ok(true)
    }
}

fn apply_array(
    value: &mut Value,
    frame: &Value,
    key: &str,
    added: &str,
    removed: &str,
    intents: bool,
) -> Result<()> {
    let mut rows = value[key]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("Missing snapshot array"))?
        .clone();
    let removals = frame[removed]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("Missing patch removals"))?;
    let additions = frame[added]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("Missing patch additions"))?;
    let mut seen = std::collections::HashSet::new();
    for id in removals {
        ensure!(seen.insert(id.to_string()), "Duplicate removal");
        let index = rows
            .iter()
            .position(|r| r[0] == *id)
            .ok_or_else(|| anyhow::anyhow!("Removal of unknown entry"))?;
        rows.remove(index);
    }
    for row in additions {
        ensure!(
            row.is_array()
                && !rows.iter().any(|r| r[0] == row[0])
                && !removals.iter().any(|id| *id == row[0]),
            "Duplicate patch entry"
        );
        rows.push(row.clone());
    }
    if !intents {
        rows.sort_by(|a, b| {
            let a: crate::wire::Raw = serde_json::from_value(a[0].clone()).unwrap_or_default();
            let b: crate::wire::Raw = serde_json::from_value(b[0].clone()).unwrap_or_default();
            b.cmp(&a)
        });
    }
    value[key] = Value::Array(rows);
    Ok(())
}
