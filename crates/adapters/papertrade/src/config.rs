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

use nautilus_core::string::secret::SecretString;
use nautilus_model::identifiers::AccountId;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const PAPERTRADE: &str = "PAPERTRADE";

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
    pub journal_path: PathBuf,
    pub build_id: String,
    pub leverage: u32,
    pub origin: String,
    pub rpc_url: String,
    pub poll_interval_ms: u64,
    pub max_history_pages: usize,
}
impl PapertradeExecutionClientConfig {
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
            journal_path,
            build_id,
            leverage: 10,
            origin: public.origin,
            rpc_url: public.rpc_url,
            poll_interval_ms: 3_000,
            max_history_pages: 1_000,
        }
    }
}
