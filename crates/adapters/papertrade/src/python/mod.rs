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
        PAPERTRADE, PapertradeDataClientConfig, PapertradeExecutionClientConfig,
        PapertradeJournalOpenMode,
    },
    factories::{PapertradeDataClientFactory, PapertradeExecutionClientFactory},
};
use nautilus_common::factories::{ClientConfig, DataClientFactory, ExecutionClientFactory};
use nautilus_core::{UUID4, python::to_pyvalue_err, string::secret::SecretString};
use nautilus_model::identifiers::{AccountId, Venue};
use nautilus_system::get_global_pyo3_registry;
use pyo3::{prelude::*, types::PyInt};

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
        journal_mode="unregistered",expected_journal_id=None,minimum_journal_revision=None))]
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
    ) -> PyResult<Self> {
        let minimum_revision = minimum_journal_revision
            .map(|value| {
                if !value.is_exact_instance_of::<PyInt>() {
                    return Err(to_pyvalue_err(
                        "Minimum journal revision must be an exact Python int",
                    ));
                }
                let revision = value.extract::<u64>().map_err(to_pyvalue_err)?;
                if revision > i64::MAX as u64 {
                    return Err(to_pyvalue_err(
                        "Minimum journal revision exceeds the persistent range",
                    ));
                }
                Ok(revision)
            })
            .transpose()?;
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

#[cfg(test)]
mod tests {
    use pyo3::{exceptions::PyValueError, prelude::*, types::PyDict};
    use rstest::rstest;

    use super::PapertradeExecutionClientConfig;
    use crate::config::PapertradeJournalOpenMode;

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
