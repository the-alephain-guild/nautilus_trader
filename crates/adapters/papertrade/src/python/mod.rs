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
    config::{
        MAX_EXECUTION_WINDOW_SECONDS, PAPERTRADE, PapertradeDataClientConfig,
        PapertradeExecutionClientConfig, PapertradeExecutionWindow, PapertradeJournalOpenMode,
        PapertradeRiskBlocker,
    },
    factories::{PapertradeDataClientFactory, PapertradeExecutionClientFactory},
};
use nautilus_common::factories::{ClientConfig, DataClientFactory, ExecutionClientFactory};
use nautilus_core::{UUID4, python::to_pyvalue_err, string::secret::SecretString};
use nautilus_model::identifiers::{AccountId, Venue};
use nautilus_system::get_global_pyo3_registry;
use pyo3::{
    prelude::*,
    types::{PyDict, PyInt},
};

fn optional_exact_int(
    value: Option<Bound<'_, PyAny>>,
    field: &str,
    maximum: u64,
) -> PyResult<Option<u64>> {
    value
        .map(|value| {
            if !value.is_exact_instance_of::<PyInt>() {
                return Err(to_pyvalue_err(format!(
                    "{field} must be an exact Python int"
                )));
            }
            let number = value.extract::<u64>().map_err(to_pyvalue_err)?;
            if number > maximum {
                return Err(to_pyvalue_err(format!(
                    "{field} exceeds the supported range"
                )));
            }
            Ok(number)
        })
        .transpose()
}

#[pymethods]
#[pyo3_stub_gen::derive::gen_stub_pymethods]
impl PapertradeRiskBlocker {
    #[new]
    fn py_new() -> Self {
        Self::default()
    }
    #[pyo3(name = "freeze")]
    fn py_freeze(&self) {
        self.freeze();
    }
    #[pyo3(name = "is_frozen")]
    fn py_is_frozen(&self) -> bool {
        self.is_frozen()
    }
    #[pyo3(name = "require_fresh_observation")]
    fn py_require_fresh_observation(&self) {
        self.require_fresh_observation();
    }
    #[pyo3(name = "set_observation_valid_until_ns")]
    fn py_set_observation_valid_until_ns(
        &self,
        #[gen_stub(override_type(type_repr = "int"))] until_ns: Bound<'_, PyAny>,
    ) -> PyResult<()> {
        let until = optional_exact_int(Some(until_ns), "Observation deadline", u64::MAX)?
            .expect("A required observation deadline was supplied");
        self.set_observation_valid_until_ns(until)
            .map_err(to_pyvalue_err)
    }
    #[pyo3(name = "require_account_observation")]
    fn py_require_account_observation(&self) {
        self.require_account_observation();
    }
    #[pyo3(name = "set_account_observation_valid_until_ns")]
    fn py_set_account_observation_valid_until_ns(
        &self,
        account_snapshot_keccak256: &str,
        #[gen_stub(override_type(type_repr = "int"))] until_ns: Bound<'_, PyAny>,
    ) -> PyResult<()> {
        if account_snapshot_keccak256.len() != 66
            || !account_snapshot_keccak256.starts_with("0x")
            || !account_snapshot_keccak256[2..]
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(to_pyvalue_err(
                "Account snapshot hash must be canonical lowercase Keccak256",
            ));
        }
        let hash = account_snapshot_keccak256.parse().map_err(to_pyvalue_err)?;
        let until = optional_exact_int(Some(until_ns), "Observation deadline", u64::MAX)?
            .expect("A required observation deadline was supplied");
        self.set_account_observation_valid_until_ns(hash, until)
            .map_err(to_pyvalue_err)
    }
}

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
        origin=None,rpc_url=None,poll_interval_ms=3000,max_history_pages=1000,read_only=false,
        journal_mode="unregistered",expected_journal_id=None,minimum_journal_revision=None,
        risk_increase_until=None,exit_until=None,risk_blocker=None))]
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
        journal_mode: &str,
        expected_journal_id: Option<String>,
        #[gen_stub(override_type(type_repr = "int | None"))] minimum_journal_revision: Option<
            Bound<'_, PyAny>,
        >,
        #[gen_stub(override_type(type_repr = "int | None"))] risk_increase_until: Option<
            Bound<'_, PyAny>,
        >,
        #[gen_stub(override_type(type_repr = "int | None"))] exit_until: Option<Bound<'_, PyAny>>,
        risk_blocker: Option<PapertradeRiskBlocker>,
    ) -> PyResult<Self> {
        let minimum_revision = optional_exact_int(
            minimum_journal_revision,
            "Minimum journal revision",
            i64::MAX as u64,
        )?;
        let risk_until = optional_exact_int(
            risk_increase_until,
            "Risk increase deadline",
            MAX_EXECUTION_WINDOW_SECONDS,
        )?;
        let exit_until =
            optional_exact_int(exit_until, "Exit deadline", MAX_EXECUTION_WINDOW_SECONDS)?;
        let execution_window = match (risk_until, exit_until) {
            (None, None) => PapertradeExecutionWindow::Unbounded {},
            (Some(risk_increase_until), Some(exit_until)) => PapertradeExecutionWindow::Bounded {
                risk_increase_until,
                exit_until,
            },
            _ => return Err(to_pyvalue_err("Execution window requires both deadlines")),
        };
        let journal_open = match (journal_mode, expected_journal_id, minimum_revision) {
            ("unregistered", None, None) => PapertradeJournalOpenMode::Unregistered {},
            ("initialize_new", None, None) => PapertradeJournalOpenMode::InitializeNew {},
            ("recover_existing", Some(id), Some(minimum_revision)) => {
                let journal_id = id.parse::<UUID4>().map_err(to_pyvalue_err)?;
                if journal_id.to_string() != id {
                    return Err(to_pyvalue_err("Expected journal UUID is not canonical"));
                }
                PapertradeJournalOpenMode::RecoverExisting {
                    journal_id,
                    minimum_revision,
                }
            }
            _ => {
                return Err(to_pyvalue_err(
                    "Journal mode requires complete and consistent recovery material",
                ));
            }
        };
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
        config.journal_open = journal_open;
        config.execution_window = execution_window;
        config.risk_blocker = risk_blocker;
        config.validate_runtime_controls().map_err(to_pyvalue_err)?;
        Ok(config)
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
    /// Hashes parameters through the actual Native submission conversion.
    #[staticmethod]
    #[pyo3(name = "submission_params_keccak256", signature = (params=None))]
    fn py_submission_params_keccak256(
        py: Python<'_>,
        params: Option<Py<PyDict>>,
    ) -> PyResult<String> {
        let params = match params {
            Some(params) => nautilus_core::from_pydict(py, &params)?,
            None => None,
        };
        crate::journal::submission_params_keccak256(params.as_ref())
            .map(|digest| format!("{digest:#x}"))
            .map_err(to_pyvalue_err)
    }
    /// Returns the compiled signing domain without keys, files or network access.
    #[staticmethod]
    #[pyo3(name = "deployment_identity_json")]
    fn py_deployment_identity_json() -> String {
        serde_json::json!({"schema_version": 1, "connector": "papertrade", "chain_id": crate::signing::CHAIN_ID,
            "exchange": format!("{:#x}", crate::signing::EXCHANGE)}).to_string()
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
    m.add_class::<PapertradeRiskBlocker>()?;
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

#[cfg(test)]
mod tests {
    use pyo3::{exceptions::PyValueError, prelude::*, types::PyDict};
    use rstest::rstest;

    use super::PapertradeExecutionClientConfig;
    use crate::config::PapertradeJournalOpenMode;

    #[rstest]
    #[case("'0x'+'12'*32", "0", true)]
    #[case("'0x'+'12'*32", "2**64-1", true)]
    #[case("'0x'+'AB'*32", "1", false)]
    #[case("'0x12'", "1", false)]
    #[case("'0x'+'g1'*32", "1", false)]
    #[case("'0X'+'12'*32", "1", false)]
    #[case("None", "1", false)]
    #[case("b'0x' + b'12'*32", "1", false)]
    #[case("'0x'+'12'*32", "True", false)]
    #[case("'0x'+'12'*32", "1.0", false)]
    #[case("'0x'+'12'*32", "-1", false)]
    #[case("'0x'+'12'*32", "2**64", false)]
    #[case("'0x'+'12'*32", "'1'", false)]
    #[case("'0x'+'12'*32", "type('I',(int,),{})(1)", false)]
    fn account_bound_python_setter_requires_canonical_hash_and_exact_time(
        #[case] hash: &str,
        #[case] deadline: &str,
        #[case] allowed: bool,
    ) {
        Python::initialize();
        Python::attach(|py| {
            let blocker = Py::new(py, super::PapertradeRiskBlocker::default()).unwrap();
            let hash = py
                .eval(&std::ffi::CString::new(hash).unwrap(), None, None)
                .unwrap();
            let deadline = py
                .eval(&std::ffi::CString::new(deadline).unwrap(), None, None)
                .unwrap();
            assert!(
                blocker
                    .call_method1(
                        py,
                        "set_account_observation_valid_until_ns",
                        (&hash, &deadline)
                    )
                    .is_err()
            );
            blocker
                .call_method0(py, "require_account_observation")
                .unwrap();
            assert_eq!(
                blocker
                    .call_method1(
                        py,
                        "set_account_observation_valid_until_ns",
                        (hash, deadline)
                    )
                    .is_ok(),
                allowed
            );
        });
    }

    #[rstest]
    #[case("True", false)]
    #[case("1.0", false)]
    #[case("'1'", false)]
    #[case("None", false)]
    #[case("-1", false)]
    #[case("2**64", false)]
    #[case("0", true)]
    #[case("2**64-1", true)]
    fn freshness_python_deadline_requires_an_exact_unsigned_integer(
        #[case] expression: &str,
        #[case] allowed: bool,
    ) {
        Python::initialize();
        Python::attach(|py| {
            let blocker = Py::new(py, super::PapertradeRiskBlocker::default()).unwrap();
            let value = py
                .eval(&std::ffi::CString::new(expression).unwrap(), None, None)
                .unwrap();
            assert!(
                blocker
                    .call_method1(py, "set_observation_valid_until_ns", (&value,))
                    .is_err()
            );
            blocker
                .call_method0(py, "require_fresh_observation")
                .unwrap();
            assert_eq!(
                blocker
                    .call_method1(py, "set_observation_valid_until_ns", (value,))
                    .is_ok(),
                allowed
            );
        });
    }

    #[rstest]
    #[case("None", "{}", true)]
    #[case("{'b':2, 'a':{'z':0,'c':[]}}", "{'a':{'c':[],'z':0},'b':2}", true)]
    #[case("{'items':(1,'中文',1.25)}", "{'items':[1,'中文',1.25]}", true)]
    #[case("{'items':[1,2]}", "{'items':[2,1]}", false)]
    #[case("{'a':{'v':1}}", "{'a':{'v':2}}", false)]
    #[case("{'v':1}", "{'v':'1'}", false)]
    #[case("{'v':{}}", "{}", false)]
    #[case("{'v':1.0}", "{'v':1}", false)]
    fn native_submission_params_digest_preserves_native_conversion_and_value_identity(
        #[case] left: &str,
        #[case] right: &str,
        #[case] equal: bool,
    ) {
        Python::initialize();
        Python::attach(|py| {
            let factory = Py::new(py, super::PapertradeExecutionClientFactory).unwrap();
            let digest = |expression: &str| -> String {
                let expression = std::ffi::CString::new(expression).unwrap();
                let params = py.eval(&expression, None, None).unwrap();
                factory
                    .call_method1(py, "submission_params_keccak256", (params,))
                    .unwrap()
                    .extract(py)
                    .unwrap()
            };
            let first = digest(left);
            assert_eq!(first.len(), 66);
            assert_eq!(first == digest(right), equal);
        });
    }

    #[rstest]
    #[case("True")]
    #[case("[]")]
    #[case("{'v':object()}")]
    #[case("{'v':float('nan')}")]
    #[case("{'v':float('inf')}")]
    fn native_submission_params_digest_has_no_conversion_failure_fallback(
        #[case] expression: &str,
    ) {
        Python::initialize();
        Python::attach(|py| {
            let expression = std::ffi::CString::new(expression).unwrap();
            let params = py.eval(&expression, None, None).unwrap();
            let factory = Py::new(py, super::PapertradeExecutionClientFactory).unwrap();
            assert!(
                factory
                    .call_method1(py, "submission_params_keccak256", (params,))
                    .is_err()
            );
            let default: String = factory
                .call_method0(py, "submission_params_keccak256")
                .unwrap()
                .extract(py)
                .unwrap();
            assert_eq!(
                default,
                format!("{:#x}", alloy_primitives::keccak256(b"null"))
            );
        });
    }

    #[test]
    fn native_window_python_deployment_matches_the_original_journal_domain() {
        Python::initialize();
        let value = Python::attach(|py| {
            let factory = Py::new(py, super::PapertradeExecutionClientFactory).unwrap();
            let raw: String = factory
                .call_method0(py, "deployment_identity_json")
                .unwrap()
                .extract(py)
                .unwrap();
            serde_json::from_str::<serde_json::Value>(&raw).unwrap()
        });
        assert_eq!(value.as_object().unwrap().len(), 4);
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["connector"], "papertrade");
        assert_eq!(value["chain_id"], crate::signing::CHAIN_ID);
        assert_eq!(
            value["exchange"],
            format!("{:#x}", crate::signing::EXCHANGE)
        );
        let root = tempfile::tempdir().unwrap();
        let wallet = "0x2222222222222222222222222222222222222222"
            .parse()
            .unwrap();
        let journal = crate::journal::Journal::initialize_new(
            &root.path().join("identity.redb"),
            wallet,
            "PAPERTRADE-001",
        )
        .unwrap();
        assert_eq!(
            journal.identity_snapshot().unwrap().owner,
            format!(
                "{}:{}:{wallet:#x}:PAPERTRADE-001",
                value["chain_id"].as_u64().unwrap(),
                value["exchange"].as_str().unwrap()
            )
        );
    }

    #[rstest]
    #[case("True", "1700010000")]
    #[case("1700000000.0", "1700010000")]
    #[case("'1700000000'", "1700010000")]
    #[case("-1", "1700010000")]
    #[case("0", "1700010000")]
    #[case("None", "1700010000")]
    #[case("1700000000", "None")]
    #[case("1700000000", "True")]
    #[case("1700010001", "1700010000")]
    #[case("1700000000", "18446744074")]
    #[case("2**64", "2**64")]
    #[case("type('InheritedInt', (int,), {})(1700000000)", "1700010000")]
    fn native_window_python_deadlines_require_an_exact_bounded_integer_pair(
        #[case] risk: &str,
        #[case] exit: &str,
    ) {
        Python::initialize();
        Python::attach(|py| {
            let fields = PyDict::new(py);
            let risk = std::ffi::CString::new(risk).unwrap();
            let exit = std::ffi::CString::new(exit).unwrap();
            fields
                .set_item(
                    "risk_increase_until",
                    py.eval(risk.as_c_str(), None, None).unwrap(),
                )
                .unwrap();
            fields
                .set_item("exit_until", py.eval(exit.as_c_str(), None, None).unwrap())
                .unwrap();
            fields
                .set_item(
                    "risk_blocker",
                    Py::new(py, super::PapertradeRiskBlocker::default()).unwrap(),
                )
                .unwrap();
            assert!(
                config(py, &fields)
                    .unwrap_err()
                    .is_instance_of::<PyValueError>(py)
            );
        });
    }

    #[test]
    fn native_window_python_bounded_config_cannot_omit_its_runtime_blocker() {
        Python::initialize();
        Python::attach(|py| {
            let fields = PyDict::new(py);
            fields
                .set_item("risk_increase_until", 1700000000_u64)
                .unwrap();
            fields.set_item("exit_until", 1700010000_u64).unwrap();
            assert!(
                config(py, &fields)
                    .unwrap_err()
                    .is_instance_of::<PyValueError>(py)
            );
        });
    }

    #[tokio::test]
    async fn native_window_python_extractor_and_factory_keep_the_original_blocker() {
        use crate::execution::tests::{
            order,
            support::{Harness, SERIAL},
        };
        use nautilus_common::{factories::ExecutionClientFactory, messages::ExecutionEvent};
        use nautilus_model::{events::OrderEventAny, identifiers::TraderId};
        use std::{sync::atomic::Ordering, time::Duration};

        let _serial = SERIAL.lock().await;
        let mut harness = Harness::new().await;
        let now = nautilus_core::time::get_atomic_clock_realtime()
            .get_time_ns()
            .as_u64()
            / 1_000_000_000;
        Python::initialize();
        let (configuration, blocker) = Python::attach(|py| {
            let module = PyModule::new(py, "papertrade").unwrap();
            super::papertrade(py, &module).unwrap();
            let blocker = module
                .getattr("PapertradeRiskBlocker")
                .unwrap()
                .call0()
                .unwrap();
            assert!(
                !blocker
                    .call_method0("is_frozen")
                    .unwrap()
                    .extract::<bool>()
                    .unwrap()
            );
            assert!(!blocker.hasattr("unfreeze").unwrap());
            let fields = PyDict::new(py);
            fields
                .set_item("session_private_key", format!("0x{}", "11".repeat(32)))
                .unwrap();
            fields.set_item("origin", &harness.config.origin).unwrap();
            fields.set_item("rpc_url", &harness.config.rpc_url).unwrap();
            fields.set_item("poll_interval_ms", 500).unwrap();
            fields.set_item("risk_increase_until", now + 7200).unwrap();
            fields.set_item("exit_until", now + 10800).unwrap();
            fields.set_item("risk_blocker", &blocker).unwrap();
            let object = py
                .get_type::<PapertradeExecutionClientConfig>()
                .call(
                    (
                        harness.config.account_id,
                        harness.config.wallet_address.as_str(),
                        harness.config.journal_path.to_str().unwrap(),
                        "test-build",
                    ),
                    Some(&fields),
                )
                .unwrap();
            (
                super::extract_execution_config(py, object.unbind()).unwrap(),
                blocker.unbind(),
            )
        });
        let mut client = super::PapertradeExecutionClientFactory
            .create(
                TraderId::from("TESTER-001"),
                "PAPERTRADE",
                configuration.as_ref(),
                harness.cache.clone().into(),
            )
            .unwrap();
        client.start().unwrap();
        client.connect().await.unwrap();
        let (_order, command) = order(&harness);
        let identity = command.client_order_id;
        Python::attach(|py| {
            blocker.call_method0(py, "freeze").unwrap();
            blocker.call_method0(py, "freeze").unwrap();
            assert!(
                blocker
                    .call_method0(py, "is_frozen")
                    .unwrap()
                    .extract::<bool>(py)
                    .unwrap()
            );
        });
        client.submit_order(command).unwrap();
        let result = tokio::time::timeout(Duration::from_secs(3), async {
            while let Some(event) = harness._events.recv().await {
                match event {
                    ExecutionEvent::Order(OrderEventAny::Denied(event))
                        if event.client_order_id == identity =>
                    {
                        return Ok(event.reason.to_string());
                    }
                    ExecutionEvent::Order(OrderEventAny::Submitted(event))
                        if event.client_order_id == identity =>
                    {
                        return Err("factory clone lost the blocker");
                    }
                    _ => {}
                }
            }
            Err("order result absent")
        })
        .await
        .unwrap();
        client.disconnect().await.unwrap();
        assert!(result.unwrap().contains("frozen"));
        assert_eq!(harness.source.intent_requests.load(Ordering::SeqCst), 0);
    }

    fn config<'py>(py: Python<'py>, fields: &Bound<'py, PyDict>) -> PyResult<Bound<'py, PyAny>> {
        py.get_type::<PapertradeExecutionClientConfig>().call(
            (
                super::AccountId::from("PAPERTRADE-001"),
                "0x2222222222222222222222222222222222222222",
                "fixture.redb",
                "test-build",
            ),
            Some(fields),
        )
    }

    #[rstest]
    #[case(false)]
    #[case(true)]
    fn journal_python_configuration_preserves_legacy_default(#[case] read_only: bool) {
        Python::initialize();
        Python::attach(|py| {
            let fields = PyDict::new(py);
            fields.set_item("read_only", read_only).unwrap();
            let object = config(py, &fields).unwrap();
            let value = object
                .extract::<PyRef<'_, PapertradeExecutionClientConfig>>()
                .unwrap();
            assert!(matches!(
                value.journal_open,
                PapertradeJournalOpenMode::Unregistered {}
            ));
            assert_eq!(value.read_only, read_only);
        });
    }

    #[rstest]
    #[case("initialize_new")]
    #[case("recover_existing")]
    fn journal_python_configuration_preserves_explicit_open_mode(#[case] mode: &str) {
        Python::initialize();
        Python::attach(|py| {
            let fields = PyDict::new(py);
            fields.set_item("journal_mode", mode).unwrap();
            let expected = nautilus_core::UUID4::new();
            if mode == "recover_existing" {
                fields
                    .set_item("expected_journal_id", expected.to_string())
                    .unwrap();
                fields.set_item("minimum_journal_revision", 7).unwrap();
            }
            let object = config(py, &fields).unwrap();
            let value = object
                .extract::<PyRef<'_, PapertradeExecutionClientConfig>>()
                .unwrap();
            match &value.journal_open {
                PapertradeJournalOpenMode::InitializeNew {} => assert_eq!(mode, "initialize_new"),
                PapertradeJournalOpenMode::RecoverExisting {
                    journal_id,
                    minimum_revision,
                } => {
                    assert_eq!(mode, "recover_existing");
                    assert_eq!(*journal_id, expected);
                    assert_eq!(*minimum_revision, 7);
                }
                PapertradeJournalOpenMode::Unregistered {} => {
                    panic!("Explicit journal mode was discarded")
                }
            }
        });
    }

    #[rstest]
    #[case("unknown", false, false)]
    #[case("recover_existing", false, false)]
    #[case("recover_existing", true, false)]
    #[case("recover_existing", false, true)]
    #[case("initialize_new", true, true)]
    #[case("unregistered", true, true)]
    fn journal_python_configuration_rejects_partial_or_conflicting_material(
        #[case] mode: &str,
        #[case] with_id: bool,
        #[case] with_revision: bool,
    ) {
        Python::initialize();
        Python::attach(|py| {
            let fields = PyDict::new(py);
            fields.set_item("journal_mode", mode).unwrap();
            if with_id {
                fields
                    .set_item(
                        "expected_journal_id",
                        nautilus_core::UUID4::new().to_string(),
                    )
                    .unwrap();
            }
            if with_revision {
                fields.set_item("minimum_journal_revision", 1).unwrap();
            }
            let error = config(py, &fields).unwrap_err();
            assert!(error.is_instance_of::<PyValueError>(py), "{error}");
        });
    }

    #[rstest]
    #[case("True")]
    #[case("False")]
    #[case("1.0")]
    #[case("-1")]
    #[case("9223372036854775808")]
    #[case("type('Index', (), {'__index__': lambda self: 1})()")]
    #[case("type('Integer', (int,), {})(1)")]
    fn journal_python_configuration_rejects_non_exact_or_unbounded_revision(
        #[case] expression: &str,
    ) {
        Python::initialize();
        Python::attach(|py| {
            let fields = PyDict::new(py);
            fields.set_item("journal_mode", "recover_existing").unwrap();
            fields
                .set_item(
                    "expected_journal_id",
                    nautilus_core::UUID4::new().to_string(),
                )
                .unwrap();
            let expression = std::ffi::CString::new(expression).unwrap();
            fields
                .set_item(
                    "minimum_journal_revision",
                    py.eval(expression.as_c_str(), None, None).unwrap(),
                )
                .unwrap();
            let error = config(py, &fields).unwrap_err();
            assert!(error.is_instance_of::<PyValueError>(py), "{error}");
        });
    }

    #[rstest]
    #[case("not-a-uuid")]
    #[case("11111111-ABCD-4111-8111-111111111111")]
    #[case("11111111-1111-1111-8111-111111111111")]
    #[case(" 11111111-1111-4111-8111-111111111111")]
    fn journal_python_configuration_rejects_invalid_or_noncanonical_uuid(#[case] id: &str) {
        Python::initialize();
        Python::attach(|py| {
            let fields = PyDict::new(py);
            fields.set_item("journal_mode", "recover_existing").unwrap();
            fields.set_item("expected_journal_id", id).unwrap();
            fields.set_item("minimum_journal_revision", 1).unwrap();
            let error = config(py, &fields).unwrap_err();
            assert!(error.is_instance_of::<PyValueError>(py), "{error}");
        });
    }

    #[rstest]
    #[case(0_u64)]
    #[case(i64::MAX as u64)]
    fn journal_python_configuration_preserves_revision_endpoints(#[case] minimum: u64) {
        Python::initialize();
        Python::attach(|py| {
            let fields = PyDict::new(py);
            fields.set_item("journal_mode", "recover_existing").unwrap();
            fields
                .set_item(
                    "expected_journal_id",
                    nautilus_core::UUID4::new().to_string(),
                )
                .unwrap();
            fields
                .set_item("minimum_journal_revision", minimum)
                .unwrap();
            let object = config(py, &fields).unwrap();
            let value = object
                .extract::<PyRef<'_, PapertradeExecutionClientConfig>>()
                .unwrap();
            assert!(
                matches!(&value.journal_open, PapertradeJournalOpenMode::RecoverExisting { minimum_revision, .. } if *minimum_revision == minimum)
            );
        });
    }
}
