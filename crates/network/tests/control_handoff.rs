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

//! Real Python control transactions and neutral HTTP handoffs.

#![cfg(all(feature = "python", not(all(feature = "simulation", madsim))))]

use std::{ffi::CString, num::NonZeroUsize, process::Command, sync::Arc, time::Duration};

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use nautilus_network::{
    http::handoff::{ControlledHttpTransport, HandoffFailure},
    python::control_handoff::PythonControlScope,
};
use pyo3::{prelude::*, types::PyModule};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires an explicit Custos checkout and its Python dependencies"]
async fn real_signed_control_scope_drives_fresh_and_reused_http_without_opening() {
    let fixture = ControlFixture::new();
    let scope = fixture.scope();
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        for _ in 0..2 {
            let mut header = Vec::new();
            while !header.ends_with(b"\r\n\r\n") {
                header.push(stream.read_u8().await.unwrap());
            }
            assert!(header.starts_with(b"GET /neutral-control HTTP/1.1\r\n"));
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 7\r\n\r\nneutral")
                .await
                .unwrap();
        }
    });
    let transport = ControlledHttpTransport::new(
        tokio::runtime::Handle::current(),
        NonZeroUsize::new(2).unwrap(),
    )
    .unwrap();
    for _ in 0..2 {
        let request = http::Request::get(format!("http://{address}/neutral-control"))
            .body(Full::new(Bytes::new()))
            .unwrap();
        let operation = transport.start(request, scope.clone()).unwrap();
        let response = tokio::time::timeout(Duration::from_secs(10), operation.wait())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(response.status(), http::StatusCode::OK);
        assert_eq!(
            response.into_body().collect().await.unwrap().to_bytes(),
            "neutral"
        );
    }
    transport.reap().await.unwrap();
    server.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires an explicit Custos checkout and its Python dependencies"]
async fn repeated_cancellation_waits_for_the_real_control_writer_to_close() {
    let fixture = ControlFixture::new();
    fixture.run(BLOCKING_SCOPE);
    fixture.run(TRACE_CONTEXT);
    let transport = controlled_transport();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let operation = transport
        .start(
            neutral_request(listener.local_addr().unwrap()),
            fixture.scope(),
        )
        .unwrap();
    fixture.wait_for("entered").await;
    fixture.run("assert database_is_busy()");
    let mut first_wait = Box::pin(operation.cancel_and_wait());
    assert!(
        tokio::time::timeout(Duration::from_millis(20), &mut first_wait)
            .await
            .is_err()
    );
    drop(first_wait);
    let mut second_wait = Box::pin(operation.cancel_and_wait());
    assert!(
        tokio::time::timeout(Duration::from_millis(20), &mut second_wait)
            .await
            .is_err()
    );
    assert!(!fixture.flag("exited"));
    fixture.run("release.set()");
    let completion = second_wait.await;
    assert_eq!(completion.failure, Some(HandoffFailure::Canceled));
    assert!(!completion.may_have_sent);
    fixture.assert_context_thread();
    fixture.run(
        "assert exited.is_set(); assert not database_is_busy(); assert len(set(threads)) == 1",
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(20), listener.accept())
            .await
            .is_err()
    );
    transport.reap().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires an explicit Custos checkout and its Python dependencies"]
async fn dropping_the_response_owner_leaves_actual_cleanup_with_the_transport() {
    let fixture = ControlFixture::new();
    fixture.run(BLOCKING_SCOPE);
    let transport = controlled_transport();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let operation = transport
        .start(
            neutral_request(listener.local_addr().unwrap()),
            fixture.scope(),
        )
        .unwrap();
    fixture.wait_for("entered").await;
    drop(operation);
    let mut reap = Box::pin(transport.reap());
    assert!(
        tokio::time::timeout(Duration::from_millis(20), &mut reap)
            .await
            .is_err()
    );
    assert!(!fixture.flag("exited"));
    fixture.run("release.set()");
    let completed = reap.await.unwrap();
    assert_eq!(completed.len(), 1);
    assert_eq!(completed[0].failure, Some(HandoffFailure::Canceled));
    fixture.run(
        "assert exited.is_set(); assert not database_is_busy(); assert len(set(threads)) == 1",
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires an explicit Custos checkout and its Python dependencies"]
async fn a_new_signed_generation_refuses_the_old_control_scope_before_hyper_poll() {
    let fixture = ControlFixture::new();
    fixture.run(
        r#"
def retire(event):
    event["aggregate_version"] = 2
    event["payload"]["generation"] = 2
    event["payload"]["lifecycle_state"] = "stopped"

_, _, newer = _verified_command(mutate_event=retire)
asyncio.run(store.record_desired_command(
    command=newer.command,
    command_fingerprint=newer.command_fingerprint,
    verification_receipt=newer.verification_receipt,
))
"#,
    );
    let transport = controlled_transport();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let operation = transport
        .start(
            neutral_request(listener.local_addr().unwrap()),
            fixture.scope(),
        )
        .unwrap();
    let e = operation.wait().await.unwrap_err();
    assert_eq!(e.failure, HandoffFailure::ControlEnter);
    assert!(!e.may_have_sent);
    fixture.run("import sqlite3\nwith sqlite3.connect(database, timeout=0.01) as c:\n    c.execute('BEGIN IMMEDIATE')");
    assert!(
        tokio::time::timeout(Duration::from_millis(20), listener.accept())
            .await
            .is_err()
    );
    transport.reap().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires an explicit Custos checkout and its Python dependencies"]
async fn a_control_exit_failure_remains_visible_after_the_actual_writer_closes() {
    let fixture = ControlFixture::new();
    fixture.run(
        r#"
import contextlib
import sqlite3
import threading
exited = threading.Event()

@contextlib.contextmanager
def context_factory():
    with gate.control_action():
        yield
    assert store._outbox._opening_control.connection is None
    with sqlite3.connect(database, timeout=0.01) as connection:
        connection.execute("BEGIN IMMEDIATE")
    exited.set()
    raise RuntimeError("the control exit observer failed")
"#,
    );
    fixture.run(TRACE_CONTEXT);
    let transport = controlled_transport();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let operation = transport
        .start(
            neutral_request(listener.local_addr().unwrap()),
            fixture.scope(),
        )
        .unwrap();
    let e = operation.wait().await.unwrap_err();
    assert_eq!(e.failure, HandoffFailure::ControlExit);
    assert!(e.may_have_sent);
    assert!(fixture.flag("exited"));
    fixture.assert_context_thread();
    transport.reap().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires an explicit Custos checkout and its Python dependencies"]
async fn runtime_shutdown_keeps_cleanup_awaitable_from_another_runtime() {
    let fixture = ControlFixture::new();
    fixture.run(BLOCKING_SCOPE);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let transport =
        ControlledHttpTransport::new(runtime.handle().clone(), NonZeroUsize::new(2).unwrap())
            .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let operation = transport
        .start(
            neutral_request(listener.local_addr().unwrap()),
            fixture.scope(),
        )
        .unwrap();
    fixture.wait_for("entered").await;
    runtime.shutdown_background();
    let mut cleanup = Box::pin(operation.cancel_and_wait());
    assert!(
        tokio::time::timeout(Duration::from_millis(20), &mut cleanup)
            .await
            .is_err()
    );
    assert!(!fixture.flag("exited"));
    fixture.run("release.set()");
    let completion = cleanup.await;
    assert_eq!(completion.failure, Some(HandoffFailure::DriverLost));
    fixture.run(
        "assert exited.is_set(); assert not database_is_busy(); assert len(set(threads)) == 1",
    );
    transport.reap().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires an explicit Custos checkout and its Python dependencies"]
async fn the_actual_control_context_is_destroyed_on_its_worker_before_completion() {
    let fixture = ControlFixture::new();
    fixture.run(
        r#"
import threading
context_events = []

class RecordingScope:
    def __init__(self):
        self.scope = gate.control_action()
    def __enter__(self):
        result = self.scope.__enter__()
        context_events.append(("enter", threading.get_ident()))
        return result
    def __exit__(self, *arguments):
        result = self.scope.__exit__(*arguments)
        context_events.append(("exit", threading.get_ident()))
        return result
    def __del__(self):
        context_events.append(("drop", threading.get_ident()))

context_factory = RecordingScope
"#,
    );
    let transport = controlled_transport();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut header = Vec::new();
        while !header.ends_with(b"\r\n\r\n") {
            header.push(stream.read_u8().await.unwrap());
        }
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
            .await
            .unwrap();
    });
    let operation = transport
        .start(neutral_request(address), fixture.scope())
        .unwrap();
    let response = operation.wait().await.unwrap();
    response.into_body().collect().await.unwrap();
    server.await.unwrap();
    fixture.run("assert [event for event, _ in context_events] == ['enter', 'exit', 'drop']; assert len({thread for _, thread in context_events}) == 1, context_events");
    transport.reap().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires an explicit Custos checkout and its Python dependencies"]
async fn runtime_reap_waits_for_the_actual_request_future_to_be_destroyed() {
    let fixture = ControlFixture::new();
    fixture.run(
        r#"
import contextlib
import threading
writer_closed = threading.Event()
exit_release = threading.Event()

@contextlib.contextmanager
def context_factory():
    with gate.control_action():
        yield
    assert store._outbox._opening_control.connection is None
    writer_closed.set()
    if not exit_release.wait(5):
        raise TimeoutError("control exit continuation missing")
"#,
    );
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let transport =
        ControlledHttpTransport::new(runtime.handle().clone(), NonZeroUsize::new(2).unwrap())
            .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let body_drop = Arc::new(BodyDropBarrier::default());
    let _release_on_failure = ReleaseBodyOnDrop(body_drop.clone());
    let request = http::Request::post(format!(
        "https://{}/neutral-control",
        listener.local_addr().unwrap()
    ))
    .body(Full::new(Bytes::from_owner(ObservedBody(
        body_drop.clone(),
    ))))
    .unwrap();
    let operation = transport.start(request, fixture.scope()).unwrap();
    fixture.wait_for("writer_closed").await;
    runtime.shutdown_background();
    fixture.run("exit_release.set()");
    tokio::time::timeout(Duration::from_secs(5), async {
        while !body_drop.started.load(std::sync::atomic::Ordering::Acquire) {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
    let mut cleanup = Box::pin(operation.cancel_and_wait());
    let outcome = tokio::time::timeout(Duration::from_millis(20), &mut cleanup).await;
    let returned_before_request_drop = outcome.is_ok();
    body_drop.release();
    if outcome.is_err() {
        cleanup.await;
    }
    tokio::time::timeout(Duration::from_secs(5), async {
        while !body_drop
            .finished
            .load(std::sync::atomic::Ordering::Acquire)
        {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
    assert!(
        !returned_before_request_drop,
        "operation completed while its real request body destructor still held the worker"
    );
    transport.reap().await.unwrap();
}

#[derive(Default)]
struct BodyDropBarrier {
    started: std::sync::atomic::AtomicBool,
    finished: std::sync::atomic::AtomicBool,
    released: std::sync::Mutex<bool>,
    changed: std::sync::Condvar,
}

impl BodyDropBarrier {
    fn release(&self) {
        *self.released.lock().unwrap() = true;
        self.changed.notify_all();
    }
}

struct ReleaseBodyOnDrop(Arc<BodyDropBarrier>);

impl Drop for ReleaseBodyOnDrop {
    fn drop(&mut self) {
        self.0.release();
    }
}

struct ObservedBody(Arc<BodyDropBarrier>);

impl AsRef<[u8]> for ObservedBody {
    fn as_ref(&self) -> &[u8] {
        b"neutral"
    }
}

impl Drop for ObservedBody {
    fn drop(&mut self) {
        self.0
            .started
            .store(true, std::sync::atomic::Ordering::Release);
        let mut released = self.0.released.lock().unwrap();
        while !*released {
            released = self.0.changed.wait(released).unwrap();
        }
        self.0
            .finished
            .store(true, std::sync::atomic::Ordering::Release);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires an explicit Custos checkout and its Python dependencies"]
async fn a_real_request_cleanup_panic_cannot_be_hidden_by_a_buffered_success() {
    let fixture = ControlFixture::new();
    let transport = controlled_transport();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut header = Vec::new();
        while !header.ends_with(b"\r\n\r\n") {
            header.push(stream.read_u8().await.unwrap());
        }
        let mut body = [0; 7];
        stream.read_exact(&mut body).await.unwrap();
        assert_eq!(&body, b"neutral");
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
            .await
            .unwrap();
    });
    let request = http::Request::post(format!("http://{address}/neutral-control"))
        .body(Full::new(Bytes::from_owner(PanicBody)))
        .unwrap();
    let operation = transport.start(request, fixture.scope()).unwrap();
    let e = operation.wait().await.unwrap_err();
    assert_eq!(e.failure, HandoffFailure::DriverPanicked);
    assert!(e.may_have_sent);
    let (first, second) = tokio::join!(
        operation.take_received_response(),
        operation.take_received_response()
    );
    assert_eq!(
        usize::from(first.is_some()) + usize::from(second.is_some()),
        1
    );
    let received = first.or(second).unwrap();
    assert_eq!(received.status(), http::StatusCode::OK);
    received.into_body().collect().await.unwrap();
    assert!(operation.take_received_response().await.is_none());
    let completion = operation.cancel_and_wait().await;
    assert_eq!(completion.failure, Some(HandoffFailure::DriverPanicked));
    assert!(completion.may_have_sent);
    server.await.unwrap();
    transport.reap().await.unwrap();
}

struct PanicBody;

impl AsRef<[u8]> for PanicBody {
    fn as_ref(&self) -> &[u8] {
        b"neutral"
    }
}

impl Drop for PanicBody {
    fn drop(&mut self) {
        panic!("the actual request owner failed during cleanup");
    }
}

fn controlled_transport() -> ControlledHttpTransport {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    ControlledHttpTransport::new(
        tokio::runtime::Handle::current(),
        NonZeroUsize::new(2).unwrap(),
    )
    .unwrap()
}

fn neutral_request(address: std::net::SocketAddr) -> http::Request<Full<Bytes>> {
    http::Request::get(format!("http://{address}/neutral-control"))
        .body(Full::new(Bytes::new()))
        .unwrap()
}

const TRACE_CONTEXT: &str = r#"
import threading
original_context_factory = context_factory
context_events = []

class TracedContext:
    def __init__(self):
        self.scope = original_context_factory()
    def __enter__(self):
        context_events.append(("enter", threading.get_ident()))
        return self.scope.__enter__()
    def __exit__(self, *arguments):
        try:
            return self.scope.__exit__(*arguments)
        finally:
            context_events.append(("exit", threading.get_ident()))
    def __del__(self):
        context_events.append(("drop", threading.get_ident()))

context_factory = TracedContext
"#;

const BLOCKING_SCOPE: &str = r#"
import contextlib
import sqlite3
import threading

entered = threading.Event()
release = threading.Event()
exited = threading.Event()
threads = []

@contextlib.contextmanager
def context_factory():
    try:
        with gate.control_action():
            threads.append(threading.get_ident())
            entered.set()
            if not release.wait(5):
                raise TimeoutError("test control scope was not released")
            yield
    finally:
        threads.append(threading.get_ident())
        assert store._outbox._opening_control.connection is None
        exited.set()

def database_is_busy():
    connection = sqlite3.connect(database, timeout=0.01)
    try:
        connection.execute("BEGIN IMMEDIATE")
    except sqlite3.OperationalError as e:
        assert "locked" in str(e)
        return True
    finally:
        connection.close()
    return False
"#;

struct ControlFixture {
    _directory: tempfile::TempDir,
    module: Py<PyModule>,
}

impl ControlFixture {
    fn assert_context_thread(&self) {
        self.run("assert [event for event, _ in context_events] == ['enter', 'exit', 'drop']; assert len({thread for _, thread in context_events}) == 1, context_events");
    }

    fn new() -> Self {
        let source = std::env::var("CUSTOS_CONTROL_HANDOFF_SOURCE")
            .expect("CUSTOS_CONTROL_HANDOFF_SOURCE must name the actual Custos checkout");
        let interpreter =
            std::env::var("PYO3_PYTHON").expect("PYO3_PYTHON must name the test interpreter");
        let environment = Command::new(interpreter)
            .args([
                "-I",
                "-B",
                "-c",
                "import sysconfig; print(sysconfig.get_path('purelib'))",
            ])
            .output()
            .unwrap();
        assert!(environment.status.success());
        let site_packages = String::from_utf8(environment.stdout).unwrap();
        let directory = tempfile::tempdir().unwrap();
        Python::initialize();
        let module = Python::attach(|py| {
            let module = PyModule::new(py, "real_control_handoff_fixture").unwrap();
            module.setattr("source_root", source).unwrap();
            module
                .setattr("site_packages", site_packages.trim())
                .unwrap();
            module
                .setattr("database", directory.path().join("runner.sqlite3"))
                .unwrap();
            let code = CString::new(
                r#"
import asyncio
import site
import sys
from pathlib import Path

site.addsitedir(site_packages)
sys.path[:0] = [str(Path(source_root) / "src"), source_root]
from tests.test_runner_fact_store import _runner_fact_store, _verified_command
from custos.core.engine_protocol import EngineLifecycleAuthority
from custos.core.execution_reservation_store import PlatformOpeningGate, RunnerOpeningGateClosed

outbox, store = _runner_fact_store(Path(database))
_, _, verified = _verified_command()
asyncio.run(store.record_desired_command(
    command=verified.command,
    command_fingerprint=verified.command_fingerprint,
    verification_receipt=verified.verification_receipt,
))
gate = PlatformOpeningGate(store, EngineLifecycleAuthority.from_verified_command(verified))
try:
    gate.assert_released()
except RunnerOpeningGateClosed:
    pass
else:
    raise AssertionError("the real signed opening gate must remain closed")
context_factory = gate.control_action
"#,
            )
            .unwrap();
            py.run(&code, Some(&module.dict()), None).unwrap();
            module.unbind()
        });

        Self {
            _directory: directory,
            module,
        }
    }

    fn run(&self, code: &str) {
        Python::attach(|py| {
            py.run(
                &CString::new(code).unwrap(),
                Some(&self.module.bind(py).dict()),
                None,
            )
            .unwrap();
        });
    }

    fn scope(&self) -> Arc<PythonControlScope> {
        Python::attach(|py| {
            Arc::new(
                PythonControlScope::new(
                    self.module
                        .bind(py)
                        .getattr("context_factory")
                        .unwrap()
                        .unbind(),
                )
                .unwrap(),
            )
        })
    }

    fn flag(&self, name: &str) -> bool {
        Python::attach(|py| {
            self.module
                .bind(py)
                .getattr(name)
                .unwrap()
                .call_method0("is_set")
                .unwrap()
                .extract()
                .unwrap()
        })
    }

    async fn wait_for(&self, name: &str) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while !self.flag(name) {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .unwrap();
    }
}
