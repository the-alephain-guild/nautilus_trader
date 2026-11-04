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

//! Owner-thread observations from a concrete execution client.

use std::fmt::Debug;

use nautilus_core::{UUID4, UnixNanos};
use nautilus_model::identifiers::{AccountId, ClientId, Venue};
use serde::{Deserialize, Serialize};

/// Identity of the execution client which actually owns an account capture.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionAccountCaptureSource {
    pub incarnation: UUID4,
    pub client_id: ClientId,
    pub account_id: AccountId,
    pub venue: Venue,
    pub account_type: String,
    pub api_revision: String,
    pub model_revision: String,
    pub runtime_version: String,
    pub configuration_json: String,
    pub configuration_sha256: String,
}

/// Exact native currency identity and current account balances.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionAccountCaptureBalance {
    pub asset_code: String,
    pub asset_precision: u8,
    pub asset_type: String,
    pub asset_iso4217: u16,
    pub total: String,
    pub locked: String,
    pub free: String,
}

/// A current account position, without a float-derived opening price.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionAccountCapturePosition {
    pub position_id: String,
    pub instrument_id: String,
    pub side: String,
    pub quantity: String,
    pub settlement_currency: String,
}

/// A nonterminal order, including intents not yet present in a matching engine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionAccountCaptureOrder {
    pub client_order_id: String,
    pub venue_order_id: Option<String>,
    pub instrument_id: String,
    pub order_type: String,
    pub side: String,
    pub status: String,
    pub quantity: String,
    pub filled_quantity: String,
    pub price: Option<String>,
    pub trigger_price: Option<String>,
    pub in_matching_engine: bool,
    pub pending: bool,
}

/// A synchronous copy of the actual owner state.
///
/// A runtime queue frontier is deliberately absent, the live node must fence its
/// ingress separately before treating this observation as a coherent capture.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionAccountSnapshot {
    pub source: ExecutionAccountCaptureSource,
    pub observed_at: UnixNanos,
    pub balances: Vec<ExecutionAccountCaptureBalance>,
    pub positions: Vec<ExecutionAccountCapturePosition>,
    pub orders: Vec<ExecutionAccountCaptureOrder>,
    pub no_borrowing_model: bool,
}

/// Closed reasons why an execution owner cannot provide an account observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionAccountCaptureError {
    #[error("Execution account capture owner retired")]
    Retired,
    #[error("Execution account capture owner is borrowed")]
    Borrowed,
    #[error("Execution account capture account is unavailable")]
    AccountUnavailable,
    #[error("Execution account capture configuration is unsupported")]
    UnsupportedConfiguration,
    #[error("Execution account capture owner state is inconsistent")]
    InconsistentState,
    #[error("Execution account capture has pending native state")]
    PendingNativeState,
}

/// Read-only access to a concrete execution owner's state on its own thread.
///
/// This trait is intentionally neither `Send` nor `Sync`, callers on another
/// thread must use the owning runtime's request queue, never move the probe.
pub trait ExecutionAccountCaptureProbe: Debug {
    fn source(&self) -> &ExecutionAccountCaptureSource;

    /// Copies the current native account without mutating or initializing it.
    ///
    /// # Errors
    ///
    /// Returns an error for a retired owner, a conflicting borrow, missing state
    /// or an unsupported account model.
    fn capture(&self) -> Result<ExecutionAccountSnapshot, ExecutionAccountCaptureError>;
}
