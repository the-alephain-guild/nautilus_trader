// -------------------------------------------------------------------------------------------------
//  Copyright (C) 2015-2026 Nautech Systems Pty Ltd. All rights reserved.
//  https://nautilus.io
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

//! Same-owner Python control scopes and classified external order callbacks.

use std::{
    fmt,
    marker::PhantomData,
    rc::Rc,
    thread::{self, ThreadId},
};

use nautilus_common::owner_delivery::{
    self, EnteredOwnerControl, OwnerControlContext, OwnerDeliveryError, OwnerExternalAction,
    OwnerExternalEvent,
};
use nautilus_model::{
    events::{OrderEventAny, PositionEvent},
    python::events::order::order_event_to_pyobject,
};
use pyo3::{IntoPyObjectExt, exceptions::PyRuntimeError, prelude::*};

pub(super) struct PythonOwnerControl {
    factory: Py<PyAny>,
    owner: ThreadId,
}

impl PythonOwnerControl {
    pub(super) fn new(factory: Py<PyAny>) -> PyResult<Self> {
        Python::attach(|py| {
            if !factory.bind(py).is_callable() {
                return Err(PyRuntimeError::new_err(
                    "native control factory must be callable",
                ));
            }
            Ok(Self {
                factory,
                owner: thread::current().id(),
            })
        })
    }
}

impl fmt::Debug for PythonOwnerControl {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PythonOwnerControl")
            .finish_non_exhaustive()
    }
}

impl OwnerControlContext for PythonOwnerControl {
    fn enter(&self) -> Result<Box<dyn EnteredOwnerControl>, OwnerDeliveryError> {
        if self.owner != thread::current().id() {
            return Err(OwnerDeliveryError::WrongOwner);
        }
        Python::attach(|py| {
            let manager = self.factory.call0(py).map_err(|error| {
                drop(error);
                OwnerDeliveryError::ControlEnter
            })?;
            manager.call_method0(py, "__enter__").map_err(|error| {
                drop(error);
                OwnerDeliveryError::ControlEnter
            })?;
            Ok(Box::new(EnteredPythonOwnerControl {
                manager: Some(manager),
                owner: self.owner,
                local: PhantomData,
            }) as Box<dyn EnteredOwnerControl>)
        })
    }
}

struct EnteredPythonOwnerControl {
    manager: Option<Py<PyAny>>,
    owner: ThreadId,
    local: PhantomData<Rc<()>>,
}

impl EnteredOwnerControl for EnteredPythonOwnerControl {
    fn finish(&mut self, failed: bool) -> Result<(), OwnerDeliveryError> {
        if self.owner != thread::current().id() {
            return Err(OwnerDeliveryError::WrongOwner);
        }
        let Some(manager) = self.manager.take() else {
            return Ok(());
        };
        Python::attach(move |py| {
            let manager = manager.into_bound(py);
            let result = if failed {
                let error = PyRuntimeError::new_err("native owner phase failed");
                manager.call_method1("__exit__", (error.get_type(py), error.value(py), py.None()))
            } else {
                manager.call_method1("__exit__", (py.None(), py.None(), py.None()))
            };
            result.map(|_| ()).map_err(|error| {
                drop(error);
                OwnerDeliveryError::ControlExit
            })
        })
    }
}

impl Drop for EnteredPythonOwnerControl {
    fn drop(&mut self) {
        if self.manager.is_some() && self.finish(true).is_err() {
            log::error!("Native owner control cleanup failed");
        }
    }
}

pub(super) fn dispatch_order_callback(
    target: Py<PyAny>,
    method: &'static str,
    event: OrderEventAny,
) -> PyResult<()> {
    dispatch_callback(target, method, OwnerExternalEvent::Order(event))
}

pub(super) fn dispatch_position_callback(
    target: Py<PyAny>,
    method: &'static str,
    event: PositionEvent,
) -> PyResult<()> {
    dispatch_callback(target, method, OwnerExternalEvent::Position(event))
}

fn dispatch_callback(
    target: Py<PyAny>,
    method: &'static str,
    event: OwnerExternalEvent,
) -> PyResult<()> {
    let event_id = match &event {
        OwnerExternalEvent::Order(event) => event.clone().into_boxed().id(),
        OwnerExternalEvent::Position(event) => match event {
            PositionEvent::PositionOpened(event) => event.event_id,
            PositionEvent::PositionChanged(event) => event.event_id,
            PositionEvent::PositionClosed(event) => event.event_id,
            PositionEvent::PositionAdjusted(event) => event.event_id,
        },
    };
    let callback = Box::new(PythonOwnerCallback {
        target: Some(target),
        method,
        event,
        error: None,
        owner: thread::current().id(),
    });
    let immediate = owner_delivery::capture_external(event_id, callback)
        .map_err(|error| PyRuntimeError::new_err(error.to_string()))?;
    if let Some(mut callback) = immediate {
        callback
            .deliver()
            .map_err(|error| PyRuntimeError::new_err(error.to_string()))?;
    }
    Ok(())
}

struct PythonOwnerCallback {
    target: Option<Py<PyAny>>,
    method: &'static str,
    event: OwnerExternalEvent,
    error: Option<PyErr>,
    owner: ThreadId,
}

impl OwnerExternalAction for PythonOwnerCallback {
    fn original_event(&self) -> OwnerExternalEvent {
        self.event.clone()
    }

    fn deliver(&mut self) -> Result<(), OwnerDeliveryError> {
        if self.owner != thread::current().id() {
            return Err(OwnerDeliveryError::WrongOwner);
        }
        Python::attach(|py| {
            let event = match self.event.clone() {
                OwnerExternalEvent::Order(event) => order_event_to_pyobject(py, event),
                OwnerExternalEvent::Position(event) => match event {
                    PositionEvent::PositionOpened(event) => event.into_py_any(py),
                    PositionEvent::PositionChanged(event) => event.into_py_any(py),
                    PositionEvent::PositionClosed(event) => event.into_py_any(py),
                    PositionEvent::PositionAdjusted(event) => event.into_py_any(py),
                },
            };
            let result = event.and_then(|event| {
                self.target
                    .as_ref()
                    .expect("owned callback target")
                    .bind(py)
                    .call_method1(self.method, (event,))
                    .map(|_| ())
            });
            result.map_err(|error| {
                // Keep the real Python exception and its original event in the failed owner.
                self.error = Some(error);
                OwnerDeliveryError::ExternalCallback
            })
        })
    }
}

impl Drop for PythonOwnerCallback {
    fn drop(&mut self) {
        assert_eq!(self.owner, thread::current().id());
        Python::attach(|py| {
            drop(self.error.take());
            drop(self.target.take().map(|target| target.into_bound(py)));
        });
    }
}
