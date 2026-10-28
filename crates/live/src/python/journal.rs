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

//! Python handles for an execution client's existing persistent journal.

use std::sync::Arc;

use nautilus_common::clients::ExecutionJournalIdentityProbe;
use nautilus_core::python::to_pyruntime_err;
use pyo3::prelude::*;

/// Reads identity evidence from the original execution journal independently of node borrows.
#[pyo3::pyclass(
    module = "nautilus_trader.live",
    name = "ExecutionJournalIdentityProbe",
    frozen
)]
#[pyo3_stub_gen::derive::gen_stub_pyclass(module = "nautilus_trader.live")]
#[derive(Debug)]
pub struct PyExecutionJournalIdentityProbe {
    pub(super) inner: Arc<dyn ExecutionJournalIdentityProbe>,
}

#[pymethods]
#[pyo3_stub_gen::derive::gen_stub_pymethods]
impl PyExecutionJournalIdentityProbe {
    /// Returns the current committed identity snapshot as JSON.
    ///
    /// A successful read does not establish connection health, trading authorization or flatness.
    ///
    /// # Errors
    ///
    /// Returns an error if the original journal is unavailable or its identity cannot be verified.
    #[pyo3(name = "snapshot_json")]
    fn py_snapshot_json(&self) -> PyResult<String> {
        let snapshot = self.inner.snapshot().map_err(to_pyruntime_err)?;
        serde_json::to_string(&snapshot).map_err(to_pyruntime_err)
    }

    /// Returns per-order evidence from the original adapter-owned journal.
    ///
    /// # Errors
    ///
    /// Returns an error for unsupported adapters or unavailable original storage.
    #[pyo3(name = "order_evidence_json")]
    fn py_order_evidence_json(&self, client_order_id: &str) -> PyResult<String> {
        self.inner
            .order_evidence_json(client_order_id)
            .map_err(to_pyruntime_err)
    }

    /// Reads the original adapter checkpoint without refreshing its source time.
    ///
    /// # Errors
    ///
    /// Returns an error when account observations are unsupported or unavailable.
    #[pyo3(name = "account_observation_json")]
    fn py_account_observation_json(&self) -> PyResult<String> {
        self.inner
            .account_observation_json()
            .map_err(to_pyruntime_err)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc, Weak,
        atomic::{AtomicU64, Ordering},
    };

    use async_trait::async_trait;
    use nautilus_common::{
        clients::{ExecutionClient, ExecutionJournalIdentity, ExecutionJournalIdentityProbe},
        enums::Environment,
        msgbus::get_message_bus,
    };
    use nautilus_core::{Params, UUID4, UnixNanos};
    use nautilus_model::{
        accounts::AccountAny,
        enums::OmsType,
        identifiers::{AccountId, ClientId, TraderId, Venue},
        types::{AccountBalance, MarginBalance},
    };
    use pyo3::{
        exceptions::{PyRuntimeError, PyTypeError},
        prelude::*,
        types::PyDict,
    };
    use rstest::rstest;

    use crate::{node::LiveNode, python::node::PyLiveNode};

    #[derive(Debug)]
    struct TestProbe {
        revision: Weak<AtomicU64>,
        journal_id: UUID4,
    }

    impl ExecutionJournalIdentityProbe for TestProbe {
        fn snapshot(&self) -> anyhow::Result<ExecutionJournalIdentity> {
            let revision = self
                .revision
                .upgrade()
                .ok_or_else(|| anyhow::anyhow!("Fixture journal released"))?;
            Ok(ExecutionJournalIdentity {
                schema_version: 1,
                journal_id: self.journal_id,
                revision: revision.load(Ordering::SeqCst),
                owner: "fixture-owner".to_owned(),
                canonical_path: "/fixture/journal.redb".into(),
            })
        }
    }

    struct TestClient {
        id: ClientId,
        probe: Option<Arc<TestProbe>>,
        _revision: Arc<AtomicU64>,
        connected: bool,
    }

    #[async_trait(?Send)]
    impl ExecutionClient for TestClient {
        fn is_connected(&self) -> bool {
            self.connected
        }
        fn client_id(&self) -> ClientId {
            self.id
        }
        fn account_id(&self) -> AccountId {
            AccountId::from("JOURNAL-001")
        }
        fn venue(&self) -> Venue {
            Venue::from(self.id.as_str())
        }
        fn oms_type(&self) -> OmsType {
            OmsType::Hedging
        }
        fn get_account(&self) -> Option<AccountAny> {
            None
        }
        fn journal_identity_probe(&self) -> Option<Arc<dyn ExecutionJournalIdentityProbe>> {
            self.probe
                .as_ref()
                .map(|probe| probe.clone() as Arc<dyn ExecutionJournalIdentityProbe>)
        }
        fn generate_account_state(
            &self,
            _: Vec<AccountBalance>,
            _: Vec<MarginBalance>,
            _: bool,
            _: UnixNanos,
            _: Option<Params>,
        ) -> anyhow::Result<()> {
            Ok(())
        }
        fn start(&mut self) -> anyhow::Result<()> {
            Ok(())
        }
        fn stop(&mut self) -> anyhow::Result<()> {
            self.connected = false;
            Ok(())
        }
        async fn connect(&mut self) -> anyhow::Result<()> {
            self.connected = true;
            Ok(())
        }
        async fn disconnect(&mut self) -> anyhow::Result<()> {
            self.connected = false;
            Ok(())
        }
    }

    fn node(revision: &Arc<AtomicU64>, supported: bool) -> PyLiveNode {
        node_with_clients(revision, supported, &["JOURNAL"])
    }

    fn node_with_clients(
        revision: &Arc<AtomicU64>,
        supported: bool,
        clients: &[&str],
    ) -> PyLiveNode {
        let node = LiveNode::builder(TraderId::from("TESTER-001"), Environment::Sandbox)
            .unwrap()
            .with_reconciliation(false)
            .with_delay_post_stop_secs(0)
            .with_delay_shutdown_secs(0)
            .with_timeout_connection(0)
            .with_timeout_disconnection_secs(0)
            .build()
            .unwrap();
        for id in clients {
            node.kernel()
                .exec_engine
                .borrow_mut()
                .register_client(Box::new(TestClient {
                    id: ClientId::from(*id),
                    probe: supported.then(|| {
                        Arc::new(TestProbe {
                            revision: Arc::downgrade(revision),
                            journal_id: UUID4::new(),
                        })
                    }),
                    _revision: revision.clone(),
                    connected: false,
                }))
                .unwrap();
        }
        PyLiveNode::new(node)
    }

    #[rstest]
    #[case(false)]
    #[case(true)]
    fn journal_python_sole_client_capture_checks_actual_registry(#[case] additional: bool) {
        Python::initialize();
        let clients = if additional {
            vec!["JOURNAL", "OTHER"]
        } else {
            vec!["JOURNAL"]
        };
        let node = node_with_clients(&Arc::new(AtomicU64::new(1)), true, &clients);
        Python::attach(|py| {
            let object = Py::new(py, node).unwrap();
            let kwargs = PyDict::new(py);
            kwargs
                .set_item("require_sole_execution_client", true)
                .unwrap();
            let result = object.call_method(
                py,
                "journal_identity_probe",
                (ClientId::from("JOURNAL"),),
                Some(&kwargs),
            );
            if additional {
                let error = result.unwrap_err();
                assert!(error.is_instance_of::<PyRuntimeError>(py), "{error}");
            } else {
                result.unwrap();
            }
            // Existing platform callers retain their previous capture behavior.
            object
                .call_method1(py, "journal_identity_probe", (ClientId::from("JOURNAL"),))
                .unwrap();
            let missing = object
                .call_method(
                    py,
                    "journal_identity_probe",
                    (ClientId::from("UNKNOWN"),),
                    Some(&kwargs),
                )
                .unwrap_err();
            assert!(missing.is_instance_of::<PyRuntimeError>(py), "{missing}");
        });
        get_message_bus().borrow_mut().dispose();
    }

    #[rstest]
    fn journal_python_handle_survives_node_move_without_retaining_the_native_owner() {
        Python::initialize();
        let revision = Arc::new(AtomicU64::new(1));
        let node = node(&revision, true);
        let probe = Python::attach(|py| {
            let object = Py::new(py, node).unwrap();
            let probe = object
                .call_method1(py, "journal_identity_probe", (ClientId::from("JOURNAL"),))
                .unwrap();
            let constructor_error = probe.bind(py).get_type().call0().unwrap_err();
            assert!(constructor_error.is_instance_of::<PyTypeError>(py));
            let unsupported = probe
                .call_method1(py, "order_evidence_json", ("UNKNOWN-ORDER",))
                .unwrap_err();
            assert!(unsupported.is_instance_of::<PyRuntimeError>(py));
            let unsupported_account = probe
                .call_method0(py, "account_observation_json")
                .unwrap_err();
            assert!(unsupported_account.is_instance_of::<PyRuntimeError>(py));
            let first: String = probe
                .call_method0(py, "snapshot_json")
                .unwrap()
                .extract(py)
                .unwrap();
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(&first).unwrap()["revision"],
                1
            );
            revision.store(2, Ordering::SeqCst);
            let locals = PyDict::new(py);
            locals.set_item("node", &object).unwrap();
            locals.set_item("probe", &probe).unwrap();
            locals
                .set_item("client_id", ClientId::from("JOURNAL"))
                .unwrap();
            py.run(pyo3::ffi::c_str!(
                "import asyncio, json\nasync def run_node():\n    handle = node.handle()\n    run = node.run_async()\n    assert json.loads(probe.snapshot_json())['revision'] == 2\n    try:\n        node.journal_identity_probe(client_id)\n    except RuntimeError:\n        pass\n    else:\n        raise AssertionError('node was not moved into run_async')\n    asyncio.get_running_loop().call_soon(handle.stop)\n    await asyncio.wait_for(run, timeout=5)\nasyncio.run(run_node())"
            ), Some(&locals), None).unwrap();
            locals.clear();
            drop(object);
            probe
        });
        drop(revision);
        Python::attach(|py| {
            let error = probe.call_method0(py, "snapshot_json").unwrap_err();
            assert!(error.is_instance_of::<PyRuntimeError>(py));
        });
        get_message_bus().borrow_mut().dispose();
    }

    #[rstest]
    #[case("UNKNOWN")]
    #[case("JOURNAL")]
    fn journal_python_capture_refuses_unknown_or_unsupported_clients(#[case] client: &str) {
        Python::initialize();
        let node = node(&Arc::new(AtomicU64::new(1)), false);
        Python::attach(|py| {
            let object = Py::new(py, node).unwrap();
            let error = object
                .call_method1(py, "journal_identity_probe", (ClientId::from(client),))
                .unwrap_err();
            assert!(error.is_instance_of::<PyRuntimeError>(py), "{error}");
        });
        get_message_bus().borrow_mut().dispose();
    }
}
