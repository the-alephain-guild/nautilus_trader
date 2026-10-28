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

//! Final intent admission and relayer acknowledgement handling.

use std::sync::atomic::Ordering;

use alloy_primitives::U256;
use anyhow::{Result, ensure};
use nautilus_model::enums::OrderStatus;

use super::{Inner, WalletActivity};
use crate::{
    config::PapertradeExecutionWindow,
    http::{IntentState, SubmissionOutcome},
    journal::JournalEntry,
    signing::{PapertradeSigner, SignedIntent},
    wire::AccountSnapshot,
};

impl Inner {
    pub(crate) fn available(&self) -> bool {
        let fresh = self
            .stream_activity
            .read()
            .as_ref()
            .is_some_and(WalletActivity::is_fresh);
        fresh
            && self.live.load(Ordering::Acquire)
            && !self.account_source_fault.load(Ordering::Acquire)
            && self.stream_healthy.load(Ordering::Acquire)
    }
    pub(super) fn authorized_signer(
        &self,
        account: &AccountSnapshot,
        deadline: u64,
        now: u64,
    ) -> Result<&PapertradeSigner> {
        ensure!(
            deadline > now && deadline - now <= 3_600,
            "Intent deadline is outside the next hour"
        );
        let signer = self
            .signer
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Papertrade observer is read-only"))?;
        ensure!(
            account
                .session_keys
                .iter()
                .any(|(key, expiry)| *key == signer.address() && expiry.0 > U256::from(deadline)),
            "Session key is not registered beyond the signed deadline"
        );
        Ok(signer)
    }

    pub(super) fn check_execution_window(
        &self,
        action: &str,
        cancelling: bool,
        now: u64,
        deadline: u64,
    ) -> Result<()> {
        let increases_risk = match (action, cancelling) {
            ("openPosition", false) | ("closePositions", true) => true,
            ("closePositions", false) | ("openPosition", true) => false,
            _ => anyhow::bail!("Unknown execution action cannot select an authorization window"),
        };
        if increases_risk {
            ensure!(
                !self
                    .config
                    .risk_blocker
                    .as_ref()
                    .is_some_and(|blocker| blocker.is_frozen()),
                "Papertrade risk increases frozen"
            );
        }
        if let PapertradeExecutionWindow::Bounded {
            risk_increase_until,
            exit_until,
        } = self.config.execution_window
        {
            let until = if increases_risk {
                risk_increase_until
            } else {
                exit_until
            };
            ensure!(
                now < until && deadline < until,
                "Signed intent must remain strictly inside its execution window"
            );
        }
        Ok(())
    }

    pub(super) fn submission_time(&self, signed: &SignedIntent, cancelling: bool) -> Result<u64> {
        let increases_risk = matches!(
            (signed.action(), cancelling),
            ("openPosition", false) | ("closePositions", true)
        );
        let source_bound = increases_risk
            && self
                .config
                .risk_blocker
                .as_ref()
                .is_some_and(|blocker| blocker.requires_account_observation());
        let source_guard =
            if source_bound {
                Some(self.sync.try_lock().map_err(|_| {
                    anyhow::anyhow!("Account source is updating before risk increase")
                })?)
            } else {
                None
            };
        let risk_instrument = if increases_risk {
            Some(
                self.journal
                    .entries()?
                    .into_iter()
                    .find(|entry| entry.intent_id == signed.id())
                    .ok_or_else(|| anyhow::anyhow!("Risk increase has no original journal entry"))?
                    .command
                    .instrument_id,
            )
        } else {
            None
        };
        let trading = increases_risk.then(|| self.trading.read());
        if let (Some(identifier), Some(trading)) = (risk_instrument, trading.as_deref()) {
            let trading = trading.as_ref().ok_or_else(|| {
                anyhow::anyhow!("No current trading configuration before risk increase")
            })?;
            let market = trading
                .instruments
                .iter()
                .find(|market| crate::reports::instrument_id(market) == identifier)
                .ok_or_else(|| anyhow::anyhow!("Risk increase market is no longer available"))?;
            ensure!(
                !trading.trading_paused && market.active && !market.paused && !market.terminal,
                "Risk increase market is paused, inactive or terminal"
            );
        }
        let account = self.snapshot.read();
        let account = account
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("No current account before submission"))?;
        let account_hash = if source_bound {
            Some(crate::journal::snapshot_keccak256(account)?)
        } else {
            None
        };
        let observation = self
            .config
            .risk_blocker
            .as_ref()
            .map(|blocker| blocker.observation());
        // Hold the complete budget read guard until the final check has finished.
        let available = self.available();
        let now_ns = self.clock.get_time_ns().as_u64();
        let now = now_ns / 1_000_000_000;
        ensure!(signed.deadline() > now, "Intent expired before submission");
        ensure!(
            available
                && self.live.load(Ordering::Acquire)
                && !self.account_source_fault.load(Ordering::Acquire)
                && self.stream_healthy.load(Ordering::Acquire),
            "Account stream unavailable before submission"
        );
        self.authorized_signer(account, signed.deadline(), now)?;
        self.check_execution_window(signed.action(), cancelling, now, signed.deadline())?;
        if increases_risk
            && let (Some(blocker), Some(observation)) =
                (self.config.risk_blocker.as_ref(), observation.as_deref())
        {
            ensure!(
                !blocker.requires_account_observation()
                    || (source_guard.is_some() && self.account_apply_valid.load(Ordering::Acquire)),
                "Account source application is incomplete before risk increase"
            );
            ensure!(
                blocker.allows_observation_at(observation, now_ns, account_hash),
                "Papertrade risk increase lacks a fresh observation with its bound account or is frozen"
            );
        }
        Ok(now)
    }

    pub(super) fn current_entry(&self, expected: &JournalEntry) -> Result<JournalEntry> {
        let current = self
            .journal
            .entries()?
            .into_iter()
            .find(|entry| entry.command.client_order_id == expected.command.client_order_id)
            .ok_or_else(|| anyhow::anyhow!("Submission target is absent from journal"))?;
        ensure!(
            current.intent_id == expected.intent_id
                && current.command == expected.command
                && current.body == expected.body
                && current.nonce == expected.nonce
                && current.target == expected.target
                && current.deadline == expected.deadline
                && current.action == expected.action
                && current.submitted_block == expected.submitted_block,
            "Submission target immutable identity differs"
        );
        Ok(current)
    }

    pub(super) async fn submit_prepared(
        &self,
        client_order_id: nautilus_model::identifiers::ClientOrderId,
    ) -> Result<()> {
        let original = self
            .journal
            .entries()?
            .into_iter()
            .find(|entry| entry.command.client_order_id == client_order_id)
            .ok_or_else(|| anyhow::anyhow!("Unknown submission"))?;
        if original.send_attempted
            || !matches!(
                original.status,
                OrderStatus::Submitted | OrderStatus::Accepted
            )
        {
            return Ok(());
        }
        self.submission_time(&original.signed()?, false)?;
        let Some(entry) = self.journal.begin_submission(client_order_id)? else {
            return Ok(());
        };
        let signed = entry.signed()?;
        // The durable marker may block on storage; recheck after that commit too.
        let now = self.submission_time(&signed, false)?;
        match self.relayer.submit(&signed, now).await? {
            SubmissionOutcome::Unknown {
                intent_id,
                http_status,
            } => {
                log::warn!(
                    "papertrade_submit_outcome_unknown intent_id={intent_id:#x} http_status={http_status:?}"
                );
            }
            SubmissionOutcome::Acknowledged(receipt) => {
                // Do not hold this lock across transport; reconciliation must remain live.
                let _serial = self.sync.lock().await;
                let mut current = self.current_entry(&entry)?;
                if !matches!(
                    current.status,
                    OrderStatus::Submitted | OrderStatus::Accepted
                ) {
                    return Ok(());
                }
                if matches!(
                    receipt.state,
                    IntentState::Queued
                        | IntentState::Prepared
                        | IntentState::Submitted
                        | IntentState::Confirmed
                ) {
                    current.status = OrderStatus::Accepted;
                    self.journal.update(&current)?;
                } else {
                    log::warn!(
                        "papertrade_submit_terminal_label_unconfirmed intent_id={:#x} state={:?}",
                        receipt.intent_id,
                        receipt.state
                    );
                }
            }
        }
        Ok(())
    }

    pub(super) async fn cancel_prepared(
        &self,
        cancel: &SignedIntent,
        entry: JournalEntry,
        prepared_at: u64,
    ) -> Result<()> {
        ensure!(
            cancel.id() == entry.intent_id && cancel.action() == entry.action,
            "Cancel target differs from prepared intent"
        );
        {
            let _serial = self.sync.lock().await;
            let current = self.current_entry(&entry)?;
            ensure!(
                matches!(
                    current.status,
                    OrderStatus::Submitted | OrderStatus::Accepted
                ),
                "Cancel target is already terminal"
            );
        }
        let now = self.submission_time(cancel, true)?;
        ensure!(
            now >= prepared_at,
            "Clock moved backwards after cancel preparation"
        );
        match self.relayer.submit(cancel, now).await? {
            SubmissionOutcome::Unknown {
                intent_id,
                http_status,
            } => {
                log::warn!(
                    "papertrade_cancel_outcome_unknown intent_id={intent_id:#x} http_status={http_status:?}"
                );
            }
            SubmissionOutcome::Acknowledged(receipt) => {
                log::info!(
                    "papertrade_cancel_acknowledged intent_id={:#x} state={:?} terminal_confirmed=false",
                    receipt.intent_id,
                    receipt.state
                );
            }
        }
        // An ACK cannot prove cancellation. In particular, never replace later fills,
        // expiry or scan progress with the target captured before awaiting the reply.
        let _serial = self.sync.lock().await;
        self.current_entry(&entry)?;
        Ok(())
    }
}
