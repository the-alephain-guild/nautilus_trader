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

//! Python configuration and factory bindings for the native LiveNode registry.

use crate::{
    config::{PAPERTRADE, PapertradeDataClientConfig, PapertradeExecutionClientConfig},
    factories::{PapertradeDataClientFactory, PapertradeExecutionClientFactory},
};
use nautilus_common::factories::{ClientConfig, DataClientFactory, ExecutionClientFactory};
use nautilus_core::{python::to_pyvalue_err, string::secret::SecretString};
use nautilus_model::identifiers::{AccountId, Venue};
use nautilus_system::get_global_pyo3_registry;
use pyo3::prelude::*;

#[pymethods]
#[pyo3_stub_gen::derive::gen_stub_pymethods]
impl PapertradeDataClientConfig {
    #[new]
    #[pyo3(signature=(origin=None,rpc_url=None,poll_interval_ms=1000,reference_info_url=None))]
    fn py_new(
        origin: Option<String>,
        rpc_url: Option<String>,
        poll_interval_ms: u64,
        reference_info_url: Option<String>,
    ) -> Self {
        let default = Self::default();
        Self {
            origin: origin.unwrap_or(default.origin),
            rpc_url: rpc_url.unwrap_or(default.rpc_url),
            reference_info_url: reference_info_url.unwrap_or(default.reference_info_url),
            poll_interval_ms,
        }
    }
}
#[pymethods]
#[pyo3_stub_gen::derive::gen_stub_pymethods]
impl PapertradeExecutionClientConfig {
    #[new]
    #[pyo3(signature=(account_id,wallet_address,journal_path,build_id,session_private_key=None,leverage=10,
        origin=None,rpc_url=None,poll_interval_ms=3000,max_history_pages=1000,read_only=false))]
    #[expect(clippy::too_many_arguments)]
    fn py_new(
        account_id: AccountId,
        wallet_address: String,
        journal_path: String,
        build_id: String,
        session_private_key: Option<String>,
        leverage: u32,
        origin: Option<String>,
        rpc_url: Option<String>,
        poll_interval_ms: u64,
        max_history_pages: usize,
        read_only: bool,
    ) -> Self {
        let mut config = Self::new(account_id, wallet_address, journal_path.into(), build_id);
        config.session_private_key = session_private_key.map(SecretString::from);
        config.leverage = leverage;
        if let Some(origin) = origin {
            config.origin = origin;
        }
        if let Some(rpc) = rpc_url {
            config.rpc_url = rpc;
        }
        config.poll_interval_ms = poll_interval_ms;
        config.max_history_pages = max_history_pages;
        config.read_only = read_only;
        config
    }
}

#[pymethods]
#[pyo3_stub_gen::derive::gen_stub_pymethods]
impl PapertradeDataClientFactory {
    #[new]
    fn py_new() -> Self {
        Self
    }
    #[pyo3(name = "name")]
    fn py_name(&self) -> &'static str {
        PAPERTRADE
    }
}

#[pymethods]
#[pyo3_stub_gen::derive::gen_stub_pymethods]
impl PapertradeExecutionClientFactory {
    #[new]
    fn py_new() -> Self {
        Self
    }
    #[pyo3(name = "name")]
    fn py_name(&self) -> &'static str {
        PAPERTRADE
    }
}

#[expect(clippy::needless_pass_by_value)]
fn extract_data_factory(py: Python<'_>, object: Py<PyAny>) -> PyResult<Box<dyn DataClientFactory>> {
    object
        .extract::<PapertradeDataClientFactory>(py)
        .map(|value| Box::new(value) as Box<dyn DataClientFactory>)
        .map_err(to_pyvalue_err)
}

#[expect(clippy::needless_pass_by_value)]
fn extract_execution_factory(
    py: Python<'_>,
    object: Py<PyAny>,
) -> PyResult<Box<dyn ExecutionClientFactory>> {
    object
        .extract::<PapertradeExecutionClientFactory>(py)
        .map(|value| Box::new(value) as Box<dyn ExecutionClientFactory>)
        .map_err(to_pyvalue_err)
}

#[expect(clippy::needless_pass_by_value)]
fn extract_data_config(py: Python<'_>, object: Py<PyAny>) -> PyResult<Box<dyn ClientConfig>> {
    object
        .extract::<PapertradeDataClientConfig>(py)
        .map(|value| Box::new(value) as Box<dyn ClientConfig>)
        .map_err(to_pyvalue_err)
}

#[expect(clippy::needless_pass_by_value)]
fn extract_execution_config(py: Python<'_>, object: Py<PyAny>) -> PyResult<Box<dyn ClientConfig>> {
    object
        .extract::<PapertradeExecutionClientConfig>(py)
        .map(|value| Box::new(value) as Box<dyn ClientConfig>)
        .map_err(to_pyvalue_err)
}

#[pymodule]
pub fn papertrade(_: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("PAPERTRADE", PAPERTRADE)?;
    m.add("PAPERTRADE_VENUE", Venue::from(PAPERTRADE))?;
    m.add_class::<PapertradeDataClientConfig>()?;
    m.add_class::<PapertradeExecutionClientConfig>()?;
    m.add_class::<PapertradeDataClientFactory>()?;
    m.add_class::<PapertradeExecutionClientFactory>()?;
    let registry = get_global_pyo3_registry();
    registry
        .register_factory_extractor(PAPERTRADE.to_owned(), extract_data_factory)
        .map_err(to_pyvalue_err)?;
    registry
        .register_exec_factory_extractor(PAPERTRADE.to_owned(), extract_execution_factory)
        .map_err(to_pyvalue_err)?;
    registry
        .register_config_extractor("PapertradeDataClientConfig".to_owned(), extract_data_config)
        .map_err(to_pyvalue_err)?;
    registry
        .register_config_extractor(
            "PapertradeExecutionClientConfig".to_owned(),
            extract_execution_config,
        )
        .map_err(to_pyvalue_err)?;
    Ok(())
}
