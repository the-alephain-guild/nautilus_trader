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

//! Same-thread Python context scopes for neutral transport handoffs.

use std::{fmt, marker::PhantomData, rc::Rc, thread::ThreadId};

use pyo3::{exceptions::PyRuntimeError, prelude::*};

/// A Python context factory, without financial authorization semantics.
pub struct PythonControlScope {
    factory: Py<PyAny>,
}

impl PythonControlScope {
    /// Creates a scope from a callable returning a synchronous context manager.
    ///
    /// # Errors
    ///
    /// Returns an error if the object is not callable.
    pub fn new(factory: Py<PyAny>) -> PyResult<Self> {
        Python::attach(|py| {
            if !factory.bind(py).is_callable() {
                return Err(PyRuntimeError::new_err(
                    "control scope factory must be callable",
                ));
            }
            Ok(Self { factory })
        })
    }

    pub(crate) fn enter(&self) -> Result<EnteredControlScope, ControlScopeError> {
        Python::attach(|py| {
            let result = (|| -> PyResult<EnteredControlScope> {
                let manager = self.factory.call0(py)?;
                manager.call_method0(py, "__enter__")?;
                Ok(EnteredControlScope {
                    manager: Some(manager),
                    thread: std::thread::current().id(),
                    local: PhantomData,
                })
            })();
            result.map_err(|e| {
                // Traceback frames can retain the context, so release them while attached
                drop(e);
                ControlScopeError::Enter
            })
        })
    }
}

impl fmt::Debug for PythonControlScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PythonControlScope").finish_non_exhaustive()
    }
}

pub(crate) struct EnteredControlScope {
    manager: Option<Py<PyAny>>,
    thread: ThreadId,
    local: PhantomData<Rc<()>>,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum ControlScopeError {
    Enter,
    Exit,
}

impl EnteredControlScope {
    #[expect(
        clippy::panic_in_result_fn,
        reason = "The !Send entered scope must never clean up on a different OS thread"
    )]
    pub(crate) fn finish(&mut self, failed: bool) -> Result<(), ControlScopeError> {
        assert_eq!(self.thread, std::thread::current().id());
        let Some(manager) = self.manager.take() else {
            return Ok(());
        };
        Python::attach(move |py| {
            let manager = manager.into_bound(py);
            let result = (|| -> PyResult<()> {
                if failed {
                    let e =
                        PyRuntimeError::new_err("native control handoff did not complete normally");
                    manager.call_method1("__exit__", (e.get_type(py), e.value(py), py.None()))?;
                } else {
                    manager.call_method1("__exit__", (py.None(), py.None(), py.None()))?;
                }
                Ok(())
            })();
            result.map_err(|e| {
                drop(e);
                ControlScopeError::Exit
            })
        })
    }
}

impl fmt::Debug for EnteredControlScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EnteredControlScope")
            .field("thread", &self.thread)
            .finish_non_exhaustive()
    }
}

impl Drop for EnteredControlScope {
    fn drop(&mut self) {
        if self.manager.is_some() && self.finish(true).is_err() {
            log::error!("Python control scope cleanup failed during unwinding");
        }
    }
}
