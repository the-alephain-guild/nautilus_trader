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

//! Python transport handles for owner-thread account captures.

use std::time::Duration;

use nautilus_core::python::to_pyruntime_err;
use pyo3::prelude::*;

use crate::node::ExecutionAccountCaptureHandle;

/// Read-only requests to the actual owner while a hosted node is running.
#[pyo3::pyclass(
    module = "nautilus_trader.live",
    name = "ExecutionAccountCaptureHandle",
    frozen
)]
#[pyo3_stub_gen::derive::gen_stub_pyclass(module = "nautilus_trader.live")]
#[derive(Debug)]
pub struct PyExecutionAccountCaptureHandle {
    pub(super) inner: ExecutionAccountCaptureHandle,
}

#[pymethods]
#[pyo3_stub_gen::derive::gen_stub_pymethods]
impl PyExecutionAccountCaptureHandle {
    /// Returns the actual client identity captured from the execution registry.
    ///
    /// # Errors
    ///
    /// Returns an error when the identity cannot be serialized.
    #[pyo3(name = "source_json")]
    fn py_source_json(&self) -> PyResult<String> {
        serde_json::to_string(self.inner.source()).map_err(to_pyruntime_err)
    }

    /// Requests an atomic owner capture, or a closed unavailable result.
    ///
    /// This method sends values through a bounded queue, it never borrows native
    /// state from the caller or a Tokio worker thread.
    ///
    /// # Errors
    ///
    /// Returns an error when Python cannot create an awaitable or JSON cannot be encoded.
    #[gen_stub(override_return_type(type_repr = "collections.abc.Awaitable[str]", imports = ("collections.abc",)))]
    #[pyo3(name = "capture_json", signature = (timeout_ms=1000))]
    fn py_capture_json<'py>(
        &self,
        py: Python<'py>,
        timeout_ms: u64,
    ) -> PyResult<Bound<'py, PyAny>> {
        let handle = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let document = match handle.capture(Duration::from_millis(timeout_ms)).await {
                Ok(capture) => serde_json::json!({"status":"captured", "capture":capture}),
                Err(reason) => serde_json::json!({"status":"unavailable", "reason":reason}),
            };
            serde_json::to_string(&document).map_err(to_pyruntime_err)
        })
    }
}
