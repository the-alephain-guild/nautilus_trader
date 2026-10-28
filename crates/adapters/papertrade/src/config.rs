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

//! Live client configuration; private keys remain outside the journal.

use alloy_primitives::B256;
use nautilus_core::{UUID4, string::secret::SecretString};
use nautilus_model::identifiers::AccountId;
use parking_lot::{RwLock, RwLockReadGuard};
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

pub const PAPERTRADE: &str = "PAPERTRADE";
pub const MAX_EXECUTION_WINDOW_SECONDS: u64 = u64::MAX / 1_000_000_000;

/// An additional, irreversible denial mechanism; an unfrozen value grants no authority.
#[derive(Debug, Clone, Default)]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(module = "nautilus_trader.adapters.papertrade", from_py_object, frozen)
)]
#[cfg_attr(
    feature = "python",
    pyo3_stub_gen::derive::gen_stub_pyclass(module = "nautilus_trader.adapters.papertrade")
)]
pub struct PapertradeRiskBlocker {
    state: Arc<PapertradeRiskState>,
}

#[derive(Debug, Default)]
struct PapertradeRiskState {
    frozen: AtomicBool,
    freshness_required: AtomicBool,
    account_observation_required: AtomicBool,
    observation: RwLock<PapertradeObservationBound>,
}

#[derive(Debug, Default)]
pub(crate) struct PapertradeObservationBound {
    until_ns: u64,
    account_snapshot: Option<B256>,
}

impl PapertradeRiskBlocker {
    pub fn freeze(&self) {
        self.state.frozen.store(true, Ordering::SeqCst);
    }

    pub fn is_frozen(&self) -> bool {
        self.state.frozen.load(Ordering::SeqCst)
    }

    /// Irreversibly requires a current observation for subsequent risk increases.
    pub fn require_fresh_observation(&self) {
        self.state.freshness_required.store(true, Ordering::SeqCst);
    }

    pub fn require_account_observation(&self) {
        self.require_fresh_observation();
        self.state
            .account_observation_required
            .store(true, Ordering::SeqCst);
    }

    pub(crate) fn requires_account_observation(&self) -> bool {
        self.state
            .account_observation_required
            .load(Ordering::SeqCst)
    }

    /// Zero temporarily denies increases; it does not reverse a permanent freeze.
    pub fn set_observation_valid_until_ns(&self, until_ns: u64) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.state.freshness_required.load(Ordering::SeqCst),
            "Observation freshness must be required before setting its deadline"
        );
        let mut observation = self.state.observation.write();
        anyhow::ensure!(
            !self.requires_account_observation() || until_ns == 0,
            "Account-bound admission requires the original account snapshot hash"
        );
        observation.until_ns = until_ns;
        Ok(())
    }

    pub fn set_account_observation_valid_until_ns(
        &self,
        account_snapshot: B256,
        until_ns: u64,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.requires_account_observation(),
            "Account observation binding must be required before setting its deadline"
        );
        *self.state.observation.write() = PapertradeObservationBound {
            until_ns,
            account_snapshot: Some(account_snapshot),
        };
        Ok(())
    }

    pub(crate) fn observation(&self) -> RwLockReadGuard<'_, PapertradeObservationBound> {
        self.state.observation.read()
    }

    pub(crate) fn allows_observation_at(
        &self,
        observation: &PapertradeObservationBound,
        now_ns: u64,
        account_snapshot: Option<B256>,
    ) -> bool {
        let fresh =
            !self.state.freshness_required.load(Ordering::SeqCst) || now_ns < observation.until_ns;
        let bound = !self.requires_account_observation()
            || (observation.account_snapshot.is_some()
                && observation.account_snapshot == account_snapshot);
        fresh && bound && !self.is_frozen()
    }

    #[cfg(test)]
    pub(crate) fn allows_risk_increase_at(&self, now_ns: u64) -> bool {
        self.allows_observation_at(&self.observation(), now_ns, None)
    }
}

/// Absolute exclusive bounds, independent of venue key validity and readiness.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum PapertradeExecutionWindow {
    Unbounded {},
    Bounded {
        risk_increase_until: u64,
        exit_until: u64,
    },
}

impl Default for PapertradeExecutionWindow {
    fn default() -> Self {
        Self::Unbounded {}
    }
}

impl PapertradeExecutionWindow {
    pub fn validate(&self) -> anyhow::Result<()> {
        if let Self::Bounded {
            risk_increase_until,
            exit_until,
        } = self
        {
            anyhow::ensure!(
                *risk_increase_until > 0
                    && *risk_increase_until <= *exit_until
                    && *exit_until <= MAX_EXECUTION_WINDOW_SECONDS,
                "Invalid execution window bounds"
            );
        }
        Ok(())
    }
}

fn reject_blocker_serialization<S: serde::Serializer>(
    _: &Option<PapertradeRiskBlocker>,
    _: S,
) -> Result<S::Ok, S::Error> {
    Err(serde::ser::Error::custom(
        "Runtime risk blocker cannot be serialized",
    ))
}

fn reject_blocker_deserialization<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<PapertradeRiskBlocker>, D::Error> {
    let _ = serde::de::IgnoredAny::deserialize(deserializer)?;
    Err(serde::de::Error::custom(
        "Runtime risk blocker must be injected by the kernel",
    ))
}

/// Selects first creation or recovery bound to a previously registered journal.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum PapertradeJournalOpenMode {
    /// Preserves the unregistered native client's existing open-or-create behavior.
    Unregistered {},
    InitializeNew {},
    RecoverExisting {
        journal_id: UUID4,
        minimum_revision: u64,
    },
}

impl Default for PapertradeJournalOpenMode {
    fn default() -> Self {
        Self::Unregistered {}
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(module = "nautilus_trader.adapters.papertrade", from_py_object)
)]
#[cfg_attr(
    feature = "python",
    pyo3_stub_gen::derive::gen_stub_pyclass(module = "nautilus_trader.adapters.papertrade")
)]
pub struct PapertradeDataClientConfig {
    pub origin: String,
    pub rpc_url: String,
    pub reference_info_url: String,
    pub poll_interval_ms: u64,
}
impl Default for PapertradeDataClientConfig {
    fn default() -> Self {
        Self {
            origin: "https://exchange.papertrade.xyz/".into(),
            rpc_url: "https://rpc.hyperliquid.xyz/evm".into(),
            reference_info_url: "https://api.hyperliquid.xyz/info".into(),
            poll_interval_ms: 1_000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(module = "nautilus_trader.adapters.papertrade", from_py_object)
)]
#[cfg_attr(
    feature = "python",
    pyo3_stub_gen::derive::gen_stub_pyclass(module = "nautilus_trader.adapters.papertrade")
)]
pub struct PapertradeExecutionClientConfig {
    pub account_id: AccountId,
    pub wallet_address: String,
    pub session_private_key: Option<SecretString>,
    #[serde(default)]
    pub read_only: bool,
    pub journal_path: PathBuf,
    #[serde(default)]
    pub journal_open: PapertradeJournalOpenMode,
    #[serde(default)]
    pub execution_window: PapertradeExecutionWindow,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "reject_blocker_serialization",
        deserialize_with = "reject_blocker_deserialization"
    )]
    pub risk_blocker: Option<PapertradeRiskBlocker>,
    pub build_id: String,
    pub leverage: u32,
    pub origin: String,
    pub rpc_url: String,
    pub poll_interval_ms: u64,
    pub max_history_pages: usize,
}
impl PapertradeExecutionClientConfig {
    pub fn validate_runtime_controls(&self) -> anyhow::Result<()> {
        self.execution_window.validate()?;
        anyhow::ensure!(
            !matches!(
                self.execution_window,
                PapertradeExecutionWindow::Bounded { .. }
            ) || self.risk_blocker.is_some(),
            "Bounded execution requires an injected risk blocker"
        );
        Ok(())
    }
    pub fn new(
        account_id: AccountId,
        wallet_address: String,
        journal_path: PathBuf,
        build_id: String,
    ) -> Self {
        let public = PapertradeDataClientConfig::default();
        Self {
            account_id,
            wallet_address,
            session_private_key: None,
            read_only: false,
            journal_path,
            journal_open: PapertradeJournalOpenMode::default(),
            execution_window: PapertradeExecutionWindow::default(),
            risk_blocker: None,
            build_id,
            leverage: 10,
            origin: public.origin,
            rpc_url: public.rpc_url,
            poll_interval_ms: 3_000,
            max_history_pages: 1_000,
        }
    }
}
