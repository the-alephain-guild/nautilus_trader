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

//! Explicit cross-repository control cancellation without opening permission.

use std::{
    any::Any,
    cell::Cell,
    path::PathBuf,
    process::{Command, Output},
    time::Duration,
};
#[cfg(unix)]
use std::{
    io::{self, Read},
    os::{fd::OwnedFd, unix::net::UnixStream},
    process::{Child, ExitStatus, Stdio},
    thread,
    time::Instant,
};

use nautilus_common::{
    clients::ExecutionAccountCaptureError,
    enums::Environment,
    live::get_data_event_sender,
    logging::logger::LoggerConfig,
    messages::DataEvent,
    msgbus::{BusTap, Endpoint, MStr, Topic},
};
use nautilus_live::{
    config::{LiveExecutionEngineConfig, LiveNodeConfig},
    node::{NodeState, builder::LiveNodeBuilder},
    python::node::PyLiveNode,
};
use nautilus_model::{
    accounts::Account, enums::TriggerType, python::orders::order_any_to_pyobject,
};
use nautilus_trading::python::strategy::PyStrategy;
use pyo3::types::{PyCFunction, PyDict, PyModule, PyModuleMethods, PyTuple};

use super::{hosted_account_connect::ready_hosted_sandbox_client, *};

#[cfg(unix)]
struct SetupChild {
    child: Child,
    reaped: bool,
}

#[cfg(unix)]
impl SetupChild {
    fn wait(&mut self) -> io::Result<ExitStatus> {
        let status = self.child.wait()?;
        self.reaped = true;
        Ok(status)
    }

    fn terminate_and_wait(&mut self) -> (io::Result<()>, io::Result<ExitStatus>) {
        let kill = self.child.kill();
        let wait = self.wait();
        (kill, wait)
    }
}

#[cfg(unix)]
impl Drop for SetupChild {
    fn drop(&mut self) {
        if !self.reaped {
            let (kill, wait) = self.terminate_and_wait();
            eprintln!("setup subprocess emergency cleanup: kill={kill:?}, wait={wait:?}");
        }
    }
}

#[cfg(unix)]
fn read_setup_output(stream: &mut UnixStream, bytes: &mut Vec<u8>) -> Result<(), String> {
    const OUTPUT_LIMIT: usize = 64 * 1024;
    let mut buffer = [0_u8; 4096];
    loop {
        let remaining = OUTPUT_LIMIT - bytes.len();
        let size = (remaining + 1).min(buffer.len());
        match stream.read(&mut buffer[..size]) {
            Ok(0) => return Ok(()),
            Ok(count) => {
                if count > remaining {
                    bytes.extend_from_slice(&buffer[..remaining]);
                    return Err("setup subprocess output exceeded 64 KiB per stream".to_owned());
                }
                bytes.extend_from_slice(&buffer[..count]);
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(()),
            Err(error) => return Err(format!("setup subprocess output read failed: {error}")),
        }
    }
}

#[cfg(unix)]
fn bounded_setup_output(command: &mut Command) -> Output {
    const TIMEOUT: Duration = Duration::from_secs(10);
    let (mut stdout, child_stdout) = UnixStream::pair().unwrap();
    let (mut stderr, child_stderr) = UnixStream::pair().unwrap();
    stdout.set_nonblocking(true).unwrap();
    stderr.set_nonblocking(true).unwrap();
    let stdout_fd: OwnedFd = child_stdout.into();
    let stderr_fd: OwnedFd = child_stderr.into();
    command
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout_fd))
        .stderr(Stdio::from(stderr_fd));
    let started = Instant::now();
    let mut owned = SetupChild {
        child: command.spawn().unwrap(),
        reaped: false,
    };
    // Command retains its configured handles for reuse; release our writer copies
    command.stdout(Stdio::null()).stderr(Stdio::null());
    let mut stdout_bytes = Vec::new();
    let mut stderr_bytes = Vec::new();
    let outcome = loop {
        if let Err(error) = read_setup_output(&mut stdout, &mut stdout_bytes)
            .and_then(|()| read_setup_output(&mut stderr, &mut stderr_bytes))
        {
            break Err(error);
        }
        match owned.child.try_wait() {
            Ok(Some(_)) => {
                // The child has exited: collect available final bytes, never wait
                // for EOF from a descriptor potentially inherited by another process
                if let Err(error) = read_setup_output(&mut stdout, &mut stdout_bytes)
                    .and_then(|()| read_setup_output(&mut stderr, &mut stderr_bytes))
                {
                    break Err(error);
                }
                break owned
                    .wait()
                    .map_err(|error| format!("setup subprocess reap failed: {error}"));
            }
            Ok(None) => {}
            Err(error) => break Err(format!("setup subprocess status failed: {error}")),
        }
        if started.elapsed() >= TIMEOUT {
            break Err("setup subprocess exceeded its 10 second deadline".to_owned());
        }
        thread::sleep(Duration::from_millis(5));
    };
    match outcome {
        Ok(status) => {
            assert!(
                status.success(),
                "setup subprocess failed: command={command:?}; status={status}; stdout={}; stderr={}",
                String::from_utf8_lossy(&stdout_bytes),
                String::from_utf8_lossy(&stderr_bytes),
            );
            Output {
                status,
                stdout: stdout_bytes,
                stderr: stderr_bytes,
            }
        }
        Err(error) => {
            let (kill, wait) = owned.terminate_and_wait();
            // Both reads are nonblocking and remain capped even during failure cleanup
            let final_stdout = read_setup_output(&mut stdout, &mut stdout_bytes);
            let final_stderr = read_setup_output(&mut stderr, &mut stderr_bytes);
            panic!(
                "{error}; command={command:?}; kill={kill:?}; wait={wait:?}; final_stdout={final_stdout:?}; final_stderr={final_stderr:?}; stdout={}; stderr={}",
                String::from_utf8_lossy(&stdout_bytes),
                String::from_utf8_lossy(&stderr_bytes),
            );
        }
    }
}

#[cfg(not(unix))]
fn bounded_setup_output(_command: &mut Command) -> Output {
    panic!("explicit cross-repository setup currently requires Unix bounded subprocess I/O");
}

pub(super) struct CustosControlFixture {
    pub(super) module: pyo3::Py<PyModule>,
}

impl CustosControlFixture {
    // All external discovery is inside the explicitly ignored test body
    pub(super) fn new(same_outbox_database: bool) -> Self {
        let source = PathBuf::from(
            std::env::var("CUSTOS_SOURCE_ROOT")
                .expect("CUSTOS_SOURCE_ROOT must explicitly name the Custos checkout"),
        );
        assert!(source.is_absolute(), "CUSTOS_SOURCE_ROOT must be absolute");
        let source = source.canonicalize().unwrap();
        for relative in [
            "pyproject.toml",
            "src/custos/__init__.py",
            "tests/test_runner_fact_store.py",
            "tests/test_runner_policy_runtime.py",
            "docs/authority/runner-deployment-command-golden-v1.json",
        ] {
            assert!(
                source.join(relative).is_file(),
                "missing Custos input: {relative}"
            );
        }
        let interpreter = PathBuf::from(
            std::env::var("PYO3_PYTHON")
                .expect("PYO3_PYTHON must explicitly name the compatible Custos interpreter"),
        );
        assert!(interpreter.is_absolute(), "PYO3_PYTHON must be absolute");
        assert!(
            interpreter.is_file(),
            "PYO3_PYTHON must name an existing interpreter"
        );
        // Preserve the venv entry: resolving its symlink would invoke the base interpreter
        let report = bounded_setup_output(Command::new(&interpreter)
            .args([
                "-I",
                "-B",
                "-c",
                "import json,sys,sysconfig; print(json.dumps({'version':list(sys.version_info[:2]),'implementation':sys.implementation.name,'executable':sys.executable,'prefix':sys.prefix,'purelib':sysconfig.get_path('purelib')}))",
            ]));
        assert!(
            report.status.success(),
            "explicit Custos interpreter query failed: {}",
            String::from_utf8_lossy(&report.stderr)
        );
        let report = String::from_utf8(report.stdout).unwrap();
        let head = bounded_setup_output(Command::new("git").args([
            "-C",
            source.to_str().unwrap(),
            "rev-parse",
            "HEAD",
        ]));
        assert!(
            head.status.success(),
            "CUSTOS_SOURCE_ROOT must be an actual checkout"
        );
        let dirty = bounded_setup_output(Command::new("git").args([
            "-C",
            source.to_str().unwrap(),
            "status",
            "--short",
        ]));
        assert!(dirty.status.success());
        let native_source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../..")
            .canonicalize()
            .unwrap();
        let native_head = bounded_setup_output(Command::new("git").args([
            "-C",
            native_source.to_str().unwrap(),
            "rev-parse",
            "HEAD",
        ]));
        assert!(native_head.status.success());
        let native_dirty = bounded_setup_output(Command::new("git").args([
            "-C",
            native_source.to_str().unwrap(),
            "status",
            "--short",
        ]));
        assert!(native_dirty.status.success());
        Python::initialize();
        let module = Python::attach(|py| {
            let module = PyModule::new(py, "cross_repository_control_cancel").unwrap();
            module
                .setattr("source_root", source.to_str().unwrap())
                .unwrap();
            module.setattr("interpreter_report", report).unwrap();
            module
                .setattr("source_head", String::from_utf8(head.stdout).unwrap())
                .unwrap();
            module
                .setattr("source_dirty", String::from_utf8(dirty.stdout).unwrap())
                .unwrap();
            module
                .setattr("native_source_root", native_source.to_str().unwrap())
                .unwrap();
            module
                .setattr(
                    "native_source_head",
                    String::from_utf8(native_head.stdout).unwrap(),
                )
                .unwrap();
            module
                .setattr(
                    "native_source_dirty",
                    String::from_utf8(native_dirty.stdout).unwrap(),
                )
                .unwrap();
            module
                .setattr("native_source_crate_version", env!("CARGO_PKG_VERSION"))
                .unwrap();
            module
                .setattr("same_outbox_database", same_outbox_database)
                .unwrap();
            py.run(c_str!(r#"
import asyncio
import hashlib
import importlib.metadata
import json
import os
import site
import sqlite3
import sys
import tempfile
import threading
from contextlib import contextmanager
from datetime import UTC, datetime, timedelta
from pathlib import Path

sys.dont_write_bytecode = True
expected_python = json.loads(interpreter_report)
assert expected_python["implementation"] == "cpython", expected_python
assert expected_python["version"] == [3, 12], expected_python
assert list(sys.version_info[:2]) == expected_python["version"], (sys.version, expected_python)
assert sys.implementation.name == expected_python["implementation"]
source = Path(source_root).resolve(strict=True)
site.addsitedir(expected_python["purelib"])
source_paths = [
    source / "src",
    source,
    source / "packages/custos-strategy-toolkit/src",
    source / "packages/custos-strategy-toolkit-nautilus/src",
]
assert all(path.is_dir() for path in source_paths), source_paths
sys.path[:0] = [str(path) for path in source_paths]

from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from tests.test_runner_fact_store import _runner_fact_store, _verified_command, COMMAND_GOLDEN_PATH
from tests.test_runner_policy_runtime import _verified_policy
from custos.core.engine_protocol import EngineLifecycleAuthority
from custos.contracts.deployment import canonical_deployment_spec_digest
from custos.core.execution_reservation_store import RunnerOpeningGateClosed
from custos.core.runner_safety_policy import DurableRunnerSafetyPolicyResolver
from custos.cli._daemon import _build_runner_safety_boundary_factory
from custos.engines.nautilus.runner_safety import RunnerSafetyOrderGate, install_order_gate
import nautilus_trader.model as installed_native_model
import nautilus_trader._libnautilus as installed_native_extension

def source_audit():
    result = {}
    for name, value in tuple(sys.modules.items()):
        if name == "custos" or name.startswith("custos."):
            expected_root = source / "src/custos"
        elif name == "custos_toolkit" or name.startswith("custos_toolkit."):
            expected_root = source / "packages/custos-strategy-toolkit/src/custos_toolkit"
        elif name == "custos_toolkit_nautilus" or name.startswith("custos_toolkit_nautilus."):
            expected_root = source / "packages/custos-strategy-toolkit-nautilus/src/custos_toolkit_nautilus"
        elif name == "tests" or name.startswith("tests."):
            expected_root = source / "tests"
        else:
            continue
        origins = []
        origin = getattr(value, "__file__", None)
        if origin is not None:
            origins.append(Path(origin).resolve(strict=True))
        origins.extend(Path(path).resolve(strict=True) for path in getattr(value, "__path__", ()))
        assert origins, (name, "module has no auditable source origin")
        assert all(path.is_relative_to(expected_root) for path in origins), (name, origins, expected_root)
        result[name] = [str(path) for path in origins]
    return result

initial_origins = source_audit()
runtime_origins = {
    "custos_source_root": str(source),
    "custos_source_head": source_head.strip(),
    "custos_source_dirty": source_dirty,
    "rust_native_source_root": native_source_root,
    "rust_native_source_head": native_source_head.strip(),
    "rust_native_source_dirty": native_source_dirty,
    "rust_native_source_crate_version": native_source_crate_version,
    "requested_interpreter": expected_python,
    "embedded_version": sys.version,
    "embedded_executable": sys.executable,
    "embedded_prefix": sys.prefix,
    "installed_native_distribution": importlib.metadata.version("nautilus-trader"),
    "installed_native_model": str(Path(installed_native_model.__file__).resolve(strict=True)),
    "installed_native_extension": str(Path(installed_native_extension.__file__).resolve(strict=True)),
    "command_golden": str(COMMAND_GOLDEN_PATH.resolve(strict=True)),
    "command_golden_sha256": hashlib.sha256(COMMAND_GOLDEN_PATH.read_bytes()).hexdigest(),
    "initial_module_origins": initial_origins,
}

directory = tempfile.TemporaryDirectory()
outbox_database = (Path(directory.name) / "runner.sqlite3").resolve()
observer_database = outbox_database if same_outbox_database else (Path(directory.name) / "observer.sqlite3").resolve()
refusals = []
callback_errors = []
control_errors = []
canceled_callbacks = []
aggregate_callbacks = []
phase_observations = []
native_delivery_status = []
desired_commits = []
retirement_errors = []
retirement_thread = None
retirement_attempted = threading.Event()
writer_barrier_observations = []
wrong_thread_errors = []
writer_probes = []
nested_submit_results = []
refusal_counts_inside_callback = []
callback_rows = []
started = False
control_returned = False
hook_installed = False
after_control_writer_released = False

async def prepare_signed_control():
    global outbox, store, verified, signed_policy, boundary, gate
    outbox, store = _runner_fact_store(outbox_database)
    _, _, verified = _verified_command()
    await store.record_desired_command(
        command=verified.command,
        command_fingerprint=verified.command_fingerprint,
        verification_receipt=verified.verification_receipt,
    )
    now = datetime.now(UTC)
    instant = lambda value: value.isoformat(timespec="microseconds").replace("+00:00", "Z")
    signed_policy = _verified_policy(
        Ed25519PrivateKey.generate(),
        tenant_id=verified.command.tenant_id,
        runner_id=verified.command.runner_id,
        trading_mode=verified.command.trading_mode,
        settlement_currency="USDC",
        effective_at=instant(now - timedelta(seconds=1)),
        expires_at=instant(now + timedelta(hours=1)),
    )
    await store.record_verified_runner_safety_policy(signed_policy)
    resolver = DurableRunnerSafetyPolicyResolver(store)
    limits = await resolver.resolve(verified.command.trading_mode)
    assert limits.policy_id == signed_policy.policy.policy_id
    assert limits.source == "verified_crucible_runner_policy", limits
    spec = verified.command.to_runtime_spec().model_dump(mode="python")
    boundary = await _build_runner_safety_boundary_factory(
        state_store=store, safety_policy_resolver=resolver
    )(spec)
    authority = EngineLifecycleAuthority.from_verified_command(verified)
    assert boundary.platform_opening_gate.authority == authority
    assert store.load_opening_gate_state_sync(authority) == "awaiting_release"
    try:
        boundary.platform_opening_gate.assert_released()
    except RunnerOpeningGateClosed as error:
        assert error.reason_code == "awaiting_opening_release"
    else:
        raise AssertionError("the actual signed opening gate must remain closed")
    gate = RunnerSafetyOrderGate(
        boundary=boundary, client_order_id_len_limit=None, on_refusal=refusals.append
    )

asyncio.run(prepare_signed_control())
connection = sqlite3.connect(observer_database)
try:
    connection.execute("CREATE TABLE callback_observations (event_id TEXT, client_order_id TEXT)")
    connection.commit()
finally:
    connection.close()

def database_identity(path):
    connection = sqlite3.connect(path, timeout=0)
    try:
        main = next(row[2] for row in connection.execute("PRAGMA database_list") if row[1] == "main")
    finally:
        connection.close()
    resolved = Path(main).resolve(strict=True)
    info = resolved.stat()
    return {"path": str(resolved), "device": info.st_dev, "inode": info.st_ino}

outbox_identity = database_identity(outbox_database)
observer_identity = database_identity(observer_database)
assert os.path.samefile(outbox_database, observer_database) == same_outbox_database
assert ((outbox_identity["device"], outbox_identity["inode"]) == (observer_identity["device"], observer_identity["inode"])) == same_outbox_database

def observe_control_writer():
    connection = sqlite3.connect(outbox_database, timeout=0)
    try:
        connection.execute("BEGIN IMMEDIATE")
    except sqlite3.OperationalError as error:
        writer_probes.append((True, error.sqlite_errorcode, str(error)))
    else:
        connection.rollback()
        writer_probes.append((False, None, ""))
    finally:
        connection.close()

def record_cancellation(event):
    event_id = str(event.event_id)
    client_order_id = str(event.client_order_id)
    observe_control_writer()
    connection = sqlite3.connect(observer_database, timeout=0)
    try:
        connection.execute("BEGIN IMMEDIATE")
        connection.execute("INSERT INTO callback_observations VALUES (?, ?)", (event_id, client_order_id))
        connection.commit()
        callback_rows.append((event_id, client_order_id))
    finally:
        connection.close()

def record_phase(label, event):
    first = strategy.cache.order(strategy.orders[0].client_order_id)
    second = strategy.cache.order(strategy.orders[1].client_order_id)
    phase_observations.append((label, str(event.event_id), str(event.client_order_id), str(first.status), str(second.status)))
    if with_oco:
        assert first.status == canceled_status, phase_observations
        expected_second = emulated_status if event.client_order_id == strategy.orders[0].client_order_id else canceled_status
        assert second.status == expected_second, phase_observations

def start_new_desired(lifecycle, replace_spec=False, observe_begin=False):
    global retirement_thread
    def mutate(event):
        payload = event["payload"]
        payload["generation"] = verified.command.generation + 1
        payload["lifecycle_state"] = lifecycle
        event["aggregate_version"] = payload["generation"]
        event["event_id"] = "80000000-0000-4000-8000-000000000008"
        if replace_spec:
            payload["deployment_spec_id"] = "90000000-0000-4000-8000-000000000009"
            payload["deployment_spec"]["deployment_spec_id"] = payload["deployment_spec_id"]
            payload["deployment_spec_digest"] = canonical_deployment_spec_digest(payload["deployment_spec"])
    _, _, current = _verified_command(mutate_event=mutate)
    if observe_begin:
        original_connect = outbox._connect
        owner_thread = threading.get_ident()
        def observed_connect():
            connection = original_connect()
            if threading.get_ident() != owner_thread:
                def trace(statement):
                    if statement.upper() == "BEGIN IMMEDIATE":
                        retirement_attempted.set()
                connection.set_trace_callback(trace)
            return connection
        outbox._connect = observed_connect
    attempted = threading.Event()
    def write():
        attempted.set()
        try:
            asyncio.run(store.record_desired_command(command=current.command, command_fingerprint=current.command_fingerprint, verification_receipt=current.verification_receipt))
            desired_commits.append((threading.get_ident(), current.command.generation, current.command.lifecycle_state, str(current.command.deployment_spec_id), current.command.deployment_spec_digest))
        except BaseException as error:
            retirement_errors.append((type(error).__name__, str(error)))
    retirement_thread = threading.Thread(target=write)
    retirement_thread.start()
    assert attempted.wait(timeout=3), "retirement writer never started"

def finish_new_desired():
    assert retirement_thread is not None
    retirement_thread.join(timeout=3)
    if retirement_thread.is_alive():
        retirement_thread.join(timeout=12)
        raise AssertionError("retirement exceeded its observation deadline")
    assert retirement_errors == [], retirement_errors
    assert len(desired_commits) == 1, desired_commits

def commit_new_desired(lifecycle, replace_spec=False):
    start_new_desired(lifecycle, replace_spec)
    finish_new_desired()

def begin_retirement_while_native_writer_held():
    start_new_desired("paused", observe_begin=True)
    assert retirement_attempted.wait(timeout=3), "real retirement SQL never reached BEGIN IMMEDIATE"
    probe = sqlite3.connect(outbox_database, timeout=0)
    try:
        try:
            probe.execute("BEGIN IMMEDIATE")
        except sqlite3.OperationalError as error:
            assert error.sqlite_errorcode in (sqlite3.SQLITE_BUSY, sqlite3.SQLITE_LOCKED)
            writer_barrier_observations.append((True, error.sqlite_errorcode, len(desired_commits)))
        else:
            probe.rollback()
            raise AssertionError("native phase did not hold the real shared writer")
    finally:
        probe.close()
    assert desired_commits == [], desired_commits

def attempt_control_from_foreign_python_thread():
    def attempt():
        try:
            strategy.cancel_order(strategy.orders[1].client_order_id, client_id=client_id)
        except BaseException as error:
            wrong_thread_errors.append((type(error).__name__, str(error)))
    thread = threading.Thread(target=attempt)
    thread.start()
    thread.join(timeout=3)
    if thread.is_alive():
        thread.join(timeout=12)
        raise AssertionError("foreign native access did not finish within its deadline")
    assert len(wrong_thread_errors) == 1, wrong_thread_errors

@contextmanager
def fail_exit_after_canceled():
    with boundary.dispatch_control():
        yield
    if strategy.cache.order(strategy.orders[0].client_order_id).status == canceled_status:
        raise RuntimeError("injected exit failure after real canceled phase")

def assert_control_released_and_observations():
    global after_control_writer_released
    connection = sqlite3.connect(outbox_database, timeout=0)
    try:
        connection.execute("BEGIN IMMEDIATE")
        connection.rollback()
        after_control_writer_released = True
    finally:
        connection.close()
    if scenario == "callback_failure":
        assert len(callback_errors) == 1 and "injected callback failure" in callback_errors[0]["message"], callback_errors
    else:
        assert callback_errors == [], callback_errors
    assert control_errors == [], control_errors
    expected_count = 0 if scenario.startswith("retire_queued_") or scenario in {"exit_failure", "capacity_native_continuation"} else (1 if scenario in {"retire_after_first", "retire_writer_first", "callback_failure"} or scenario.startswith("raw_submit_") else (2 if with_oco else 1))
    assert len(canceled_callbacks) == expected_count, canceled_callbacks
    assert callback_rows == canceled_callbacks
    connection = sqlite3.connect(observer_database, timeout=0)
    try:
        rows = list(connection.execute("SELECT event_id, client_order_id FROM callback_observations ORDER BY rowid"))
    finally:
        connection.close()
    assert rows == canceled_callbacks, (rows, canceled_callbacks)
    assert len(writer_probes) == expected_count, writer_probes
    for blocked, code, message in writer_probes:
        if blocked:
            assert code in (sqlite3.SQLITE_BUSY, sqlite3.SQLITE_LOCKED), writer_probes
        else:
            assert (code, message) == (None, ""), writer_probes
    assert nested_submit_results == ([False] if expected_count else []), nested_submit_results
    assert len(refusals) == (1 if expected_count else 0), refusals
    if expected_count:
        expected_reason = "custos_runtime_control_unavailable" if scenario == "retire_writer_first" else "awaiting_opening_release"
        assert refusals[0].reason_code == expected_reason, refusals
    expected_phases = []
    for event_id, client_order_id in canceled_callbacks:
        expected_phases.extend([("specific", event_id, client_order_id), ("aggregate", event_id, client_order_id)])
    assert [value[:3] for value in phase_observations] == expected_phases, phase_observations
    if scenario.startswith("retire_"):
        assert len(desired_commits) == 1 and retirement_errors == [], (desired_commits, retirement_errors)
        assert retirement_thread is not None and not retirement_thread.is_alive()
    if scenario == "retire_writer_first":
        assert writer_barrier_observations == [(True, sqlite3.SQLITE_BUSY, 0)], writer_barrier_observations
    if scenario == "wrong_thread":
        assert len(wrong_thread_errors) == 1, wrong_thread_errors
    source_audit()

def diagnostics():
    return json.dumps({
        "runtime_origins": runtime_origins,
        "outbox_identity": outbox_identity, "observer_identity": observer_identity,
        "same_outbox_database": same_outbox_database,
        "callback_errors": callback_errors, "control_errors": control_errors,
        "canceled_callbacks": canceled_callbacks, "aggregate_callbacks": aggregate_callbacks,
        "phase_observations": phase_observations,
        "native_delivery_status": native_delivery_status,
        "desired_commits": desired_commits, "retirement_errors": retirement_errors,
        "writer_barrier_observations": writer_barrier_observations, "wrong_thread_errors": wrong_thread_errors,
        "callback_rows": callback_rows, "writer_probes": writer_probes,
        "nested_submit_results": nested_submit_results,
        "refusal_counts_inside_callback": refusal_counts_inside_callback,
        "refusals": [value.reason_code for value in refusals],
        "started": started, "control_returned": control_returned,
        "after_control_writer_released": after_control_writer_released,
    }, sort_keys=True)

def cleanup():
    if retirement_thread is not None and retirement_thread.is_alive():
        retirement_thread.join(timeout=12)
        assert not retirement_thread.is_alive(), "owned retirement writer was not reaped"
    directory.cleanup()
"#), Some(&module.dict()), None).unwrap();
            module.unbind()
        });
        Self { module }
    }

    pub(super) fn diagnostics(&self) -> String {
        Python::attach(|py| {
            self.module
                .bind(py)
                .call_method0("diagnostics")
                .unwrap()
                .extract()
                .unwrap()
        })
    }
}

impl Drop for CustosControlFixture {
    fn drop(&mut self) {
        Python::attach(|py| {
            if let Err(e) = self.module.bind(py).call_method0("cleanup") {
                eprintln!("cross-repository fixture cleanup failed: {e}");
            }
        });
    }
}

#[derive(Default)]
struct ControlNativeTrace {
    orders: Vec<OrderEventAny>,
    commands: Vec<TradingCommand>,
    venue_commands: Vec<TradingCommand>,
    raw_execution_events: Vec<OrderEventAny>,
    quotes: Vec<QuoteTick>,
}

struct ControlNativeTap {
    trace: Rc<RefCell<ControlNativeTrace>>,
    busy_submit: Option<TradingCommand>,
    injected: Cell<bool>,
    retirement_observer: Option<pyo3::Py<pyo3::PyAny>>,
    capacity_pressure: bool,
    pressure_injected: Cell<bool>,
}

#[pyo3::pyclass(unsendable)]
struct RunningRawOrderInitializer {
    cache: Rc<RefCell<Cache>>,
    order: OrderAny,
    client_id: ClientId,
}

#[pyo3::pymethods]
impl RunningRawOrderInitializer {
    fn __call__(&self) -> pyo3::PyResult<()> {
        self.cache
            .borrow_mut()
            .add_order(self.order.clone(), None, Some(self.client_id), false)
            .map_err(|error| pyo3::exceptions::PyRuntimeError::new_err(error.to_string()))
    }
}

impl BusTap for ControlNativeTap {
    fn on_publish(&self, topic: MStr<Topic>, message: &dyn Any) {
        if let Some(event) = message.downcast_ref::<OrderEventAny>()
            && topic.as_str() == format!("events.order.{}", event.strategy_id())
        {
            self.trace.borrow_mut().orders.push(event.clone());
            if self.capacity_pressure
                && matches!(event, OrderEventAny::Canceled(value) if value.client_order_id == ClientOrderId::from("CONTROL-CANCEL"))
                && !self.pressure_injected.replace(true)
            {
                // Re-publish the real original value without re-applying it to canonical cache.
                // This drives the real Python trait outlets and busy registered emulator handler.
                for _ in 0..512 {
                    msgbus::publish_order_event(topic, event);
                }
            }
            if matches!(event, OrderEventAny::Canceled(value) if value.client_order_id == ClientOrderId::from("CONTROL-CANCEL"))
                && let Some(observer) = &self.retirement_observer
            {
                Python::attach(|py| {
                    observer
                        .call_method0(py, "begin_retirement_while_native_writer_held")
                        .unwrap()
                });
            }
            if matches!(event, OrderEventAny::Canceled(value) if value.client_order_id == ClientOrderId::from("CONTROL-CANCEL"))
                && !self.injected.replace(true)
                && let Some(command) = &self.busy_submit
            {
                // This is a real internal publication, while the registered emulator is borrowed.
                msgbus::send_trading_command(
                    MessagingSwitchboard::order_emulator_execute(),
                    command.clone(),
                );
            }
        }
    }

    fn on_send(&self, endpoint: MStr<Endpoint>, message: &dyn Any) {
        if let Some(command) = message.downcast_ref::<TradingCommand>() {
            if endpoint == MessagingSwitchboard::order_emulator_execute() {
                self.trace.borrow_mut().commands.push(command.clone());
            } else if endpoint == MessagingSwitchboard::exec_engine_queue_execute() {
                self.trace.borrow_mut().venue_commands.push(command.clone());
            }
        }
        if endpoint == MessagingSwitchboard::exec_engine_process()
            && let Some(event) = message.downcast_ref::<OrderEventAny>()
        {
            self.trace
                .borrow_mut()
                .raw_execution_events
                .push(event.clone());
        }
        if endpoint == MessagingSwitchboard::data_engine_process_data()
            && let Some(Data::Quote(quote)) = message.downcast_ref::<Data>()
        {
            self.trace.borrow_mut().quotes.push(*quote);
        }
    }
}

#[rstest]
#[case::separate_observer_database(false)]
#[case::same_physical_outbox_database(true)]
#[ignore = "requires explicit CUSTOS_SOURCE_ROOT and compatible Custos PYO3_PYTHON"]
fn test_cross_repo_signed_control_cancel_keeps_native_and_sqlite_observations_consistent(
    #[case] same_outbox_database: bool,
) {
    run_control_cancel(same_outbox_database, false);
}

pub(super) fn run_control_cancel(same_outbox_database: bool, with_oco: bool) {
    run_control_scenario(same_outbox_database, with_oco, "normal");
}

pub(super) fn run_control_scenario(same_outbox_database: bool, with_oco: bool, scenario: &str) {
    let expected_failure = match scenario {
        "normal" | "stop_after_queue" | "wrong_thread" => None,
        "retire_queued_paused"
        | "retire_queued_stopped"
        | "retire_queued_spec"
        | "retire_after_first" => Some("ControlEnter"),
        "retire_writer_first" => Some("ControlEnter"),
        "callback_failure" => Some("ExternalCallback"),
        "exit_failure" => Some("ControlExit"),
        "raw_submit_busy" | "raw_submit_free" => Some("UnsupportedEffect"),
        "capacity_native_continuation" => Some("Capacity"),
        _ => panic!("unknown control scenario"),
    };
    let fixture = CustosControlFixture::new(same_outbox_database);
    let trader_id = TraderId::from("CONTROL-001");
    let account_id = AccountId::from("SIM-001");
    let client_id = ClientId::from("CONTROL");
    let config = LiveNodeConfig {
        trader_id,
        environment: Environment::Sandbox,
        timeout_connection: Duration::from_secs(3),
        timeout_disconnection: Duration::from_millis(500),
        delay_post_stop: Duration::ZERO,
        timeout_shutdown: Duration::from_millis(500),
        exec_engine: LiveExecutionEngineConfig {
            reconciliation: false,
            ..Default::default()
        },
        ..Default::default()
    };
    let node = LiveNodeBuilder::from_config(config)
        .unwrap()
        .with_logging(LoggerConfig {
            bypass_logging: true,
            ..Default::default()
        })
        .build()
        .unwrap();
    let cache = node.kernel().cache();
    let emulator = node.kernel().order_emulator.emulator();
    let emulator_initial_owners = Rc::strong_count(&emulator);
    let clock = node.kernel().clock();
    let now = clock.borrow().timestamp_ns();
    let mut instrument = binary_option();
    instrument.id = InstrumentId::from("CONTROL.SIM");
    instrument.activation_ns = UnixNanos::from(1);
    instrument.expiration_ns = UnixNanos::from(now.as_u64() + 86_400_000_000_000);
    let instrument = InstrumentAny::BinaryOption(instrument);
    let quote = |bid: &str, ask: &str, offset: u64| {
        QuoteTick::new(
            instrument.id(),
            Price::from(bid),
            Price::from(ask),
            Quantity::from("10000.00"),
            Quantity::from("10000.00"),
            UnixNanos::from(now.as_u64() + offset),
            UnixNanos::from(now.as_u64() + offset),
        )
    };
    cache
        .borrow_mut()
        .add_instrument(instrument.clone())
        .unwrap();
    cache
        .borrow_mut()
        .add_quote(quote("0.400", "0.410", 0))
        .unwrap();
    let neutral_quote = quote("0.410", "0.420", 1_000_000);
    let sender = get_data_event_sender();
    Python::attach(|py| {
        let module = fixture.module.bind(py);
        module.setattr("with_oco", with_oco).unwrap();
        module.setattr("scenario", scenario).unwrap();
        module
            .setattr(
                "canceled_status",
                OrderStatus::Canceled.into_py_any(py).unwrap(),
            )
            .unwrap();
        module
            .setattr(
                "emulated_status",
                OrderStatus::Emulated.into_py_any(py).unwrap(),
            )
            .unwrap();
        module
            .setattr("Strategy", py.get_type::<PyStrategy>())
            .unwrap();
        module
            .setattr("client_id", client_id.into_py_any(py).unwrap())
            .unwrap();
        module
            .setattr("instrument_id", instrument.id().into_py_any(py).unwrap())
            .unwrap();
        let send = PyCFunction::new_closure(
            py,
            None,
            None,
            move |_args: &pyo3::Bound<'_, PyTuple>, _kwargs| -> pyo3::PyResult<()> {
                sender
                    .send(DataEvent::Data(Data::Quote(neutral_quote)))
                    .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))
            },
        )
        .unwrap();
        module.setattr("send_control_quote", send).unwrap();
        py.run(c_str!(r#"
class ControlledCancellationStrategy(Strategy):
    def on_start(self):
        global started
        self.submit_order(self.orders[0], client_id=client_id)
        self.submit_order(self.orders[1], client_id=client_id)
        self.subscribe_quotes(instrument_id)
        started = True
    def on_stop(self): pass
    def on_dispose(self): pass
    def on_quote(self, quote):
        global hook_installed, control_returned
        if hook_installed:
            return
        try:
            if with_oco:
                assert callable(getattr(self, "_bind_control_delivery", None)), "native owner delivery API is absent"
            install_order_gate(self, gate)
            if scenario == "exit_failure":
                self._bind_control_delivery(fail_exit_after_canceled)
            native_delivery_status.append(("after_bind", self._control_delivery_status()))
            hook_installed = True
            if scenario == "wrong_thread":
                attempt_control_from_foreign_python_thread()
            if scenario.startswith("raw_submit_"):
                prepare_raw_initialized_order()
                assert str(self.cache.order(raw_client_order_id).status) == "INITIALIZED"
            self.cancel_order(self.orders[0].client_order_id, client_id=client_id)
            native_delivery_status.append(("after_request", self._control_delivery_status()))
            if scenario.startswith("retire_queued_"):
                lifecycle = "stopped" if scenario == "retire_queued_stopped" else "paused"
                commit_new_desired(lifecycle, scenario == "retire_queued_spec")
            if scenario == "stop_after_queue":
                request_native_stop()
        except BaseException as error:
            control_errors.append({"type": type(error).__name__, "message": str(error)})
            raise
        finally:
            control_returned = True
    def on_order_canceled(self, event):
        canceled_callbacks.append((str(event.event_id), str(event.client_order_id)))
        try:
            if scenario == "retire_writer_first" and event.client_order_id == self.orders[0].client_order_id:
                finish_new_desired()
            record_phase("specific", event)
            if event.client_order_id == self.orders[0].client_order_id:
                nested_submit_results.append(self.submit_order(self.orders[2], client_id=client_id))
                refusal_counts_inside_callback.append(len(refusals))
            record_cancellation(event)
            if event.client_order_id == self.orders[0].client_order_id:
                if scenario == "raw_submit_free":
                    send_raw_emulator_submit()
                if scenario == "retire_after_first":
                    commit_new_desired("paused")
                if scenario == "callback_failure":
                    raise RuntimeError("injected callback failure after real callback SQLite commit")
        except BaseException as error:
            callback_errors.append({
                "type": type(error).__name__, "message": str(error),
                "sqlite_errorcode": getattr(error, "sqlite_errorcode", None),
                "event_id": str(event.event_id), "client_order_id": str(event.client_order_id),
                "database": str(observer_database),
            })
            raise
    def on_order_event(self, event):
        aggregate_callbacks.append((type(event).__name__, str(event.event_id), str(event.client_order_id)))
        if type(event).__name__ == "OrderCanceled":
            try:
                record_phase("aggregate", event)
            except BaseException as error:
                callback_errors.append({"type": type(error).__name__, "message": str(error), "event_id": str(event.event_id), "client_order_id": str(event.client_order_id)})
                raise

strategy = ControlledCancellationStrategy()
runtime_origins["rust_strategy_type"] = {"module": Strategy.__module__, "name": Strategy.__qualname__}
runtime_origins["native_fixture_role"] = "independent inherited emulated history; not platform opening or trusted P1 attachment"
def mark_waiting(): pass
"#), Some(&module.dict()), None).unwrap();
    });
    let strategy = Python::attach(|py| {
        fixture
            .module
            .bind(py)
            .getattr("strategy")
            .unwrap()
            .unbind()
    });
    let strategy_id = node
        .kernel()
        .trader()
        .borrow_mut()
        .add_python_strategy_instance(&strategy)
        .unwrap();
    let mut orders = Vec::new();
    for (id, trigger, price) in [
        ("CONTROL-CANCEL", "0.600", "0.620"),
        ("CONTROL-RETAIN", "0.650", "0.670"),
    ] {
        let mut builder = OrderTestBuilder::new(OrderType::StopLimit);
        builder
            .trader_id(trader_id)
            .strategy_id(strategy_id)
            .instrument_id(instrument.id())
            .client_order_id(id.into())
            .side(OrderSide::Buy)
            .quantity(Quantity::from("100.00"))
            .price(Price::from(price))
            .trigger_price(Price::from(trigger))
            .trigger_type(TriggerType::BidAsk)
            .emulation_trigger(TriggerType::BidAsk);
        if with_oco {
            builder
                .contingency_type(nautilus_model::enums::ContingencyType::Oco)
                .linked_order_ids(vec![ClientOrderId::from(if id == "CONTROL-CANCEL" {
                    "CONTROL-RETAIN"
                } else {
                    "CONTROL-CANCEL"
                })]);
        }
        orders.push(builder.build());
    }
    orders.push(
        OrderTestBuilder::new(OrderType::Market)
            .trader_id(trader_id)
            .strategy_id(strategy_id)
            .instrument_id(instrument.id())
            .client_order_id("CONTROL-MUST-REFUSE".into())
            .side(OrderSide::Buy)
            .quantity(Quantity::from("100.00"))
            .build(),
    );
    let raw_order = OrderTestBuilder::new(OrderType::StopLimit)
        .trader_id(trader_id)
        .strategy_id(strategy_id)
        .instrument_id(instrument.id())
        .client_order_id("CONTROL-RAW-MUST-REFUSE".into())
        .side(OrderSide::Buy)
        .quantity(Quantity::from("100.00"))
        .price(Price::from("0.720"))
        .trigger_price(Price::from("0.700"))
        .trigger_type(TriggerType::BidAsk)
        .emulation_trigger(TriggerType::BidAsk)
        .build();
    let raw_submit = TradingCommand::SubmitOrder(SubmitOrder::from_order(
        &raw_order,
        trader_id,
        Some(client_id),
        None,
        UUID4::new(),
        now,
    ));
    let raw_scenario = scenario.starts_with("raw_submit_");
    Python::attach(|py| {
        fixture
            .module
            .bind(py)
            .setattr(
                "prepare_raw_initialized_order",
                pyo3::Py::new(
                    py,
                    RunningRawOrderInitializer {
                        cache: cache.clone(),
                        order: raw_order.clone(),
                        client_id,
                    },
                )
                .unwrap(),
            )
            .unwrap();
        let command = raw_submit.clone();
        let send_raw = PyCFunction::new_closure(
            py,
            None,
            None,
            move |_args: &pyo3::Bound<'_, PyTuple>, _kwargs| -> pyo3::PyResult<()> {
                msgbus::send_trading_command(
                    MessagingSwitchboard::order_emulator_execute(),
                    command.clone(),
                );
                Ok(())
            },
        )
        .unwrap();
        fixture
            .module
            .bind(py)
            .setattr("send_raw_emulator_submit", send_raw)
            .unwrap();
        fixture
            .module
            .bind(py)
            .setattr(
                "raw_client_order_id",
                raw_order.client_order_id().into_py_any(py).unwrap(),
            )
            .unwrap();
        let values: Vec<_> = orders
            .iter()
            .map(|order| order_any_to_pyobject(py, order.clone()).unwrap())
            .collect();
        strategy.bind(py).setattr("orders", values).unwrap();
    });
    let client_config = SandboxExecutionClientConfig {
        account_id,
        venue: Venue::from("SIM"),
        account_type: AccountType::Cash,
        oms_type: OmsType::Netting,
        base_currency: Some(Currency::USDC()),
        starting_balances: vec![Money::from("100000 USDC")],
        ..Default::default()
    };
    let core = ExecutionClientCore::new(
        trader_id,
        client_id,
        client_config.venue,
        client_config.oms_type,
        account_id,
        client_config.account_type,
        client_config.base_currency,
        cache.clone(),
    );
    let inner = SandboxExecutionClient::new(core, client_config, clock, cache.clone());
    let capture = inner.account_capture_probe().unwrap();
    let stopped = Rc::new(Cell::new(0));
    let waiting = Python::attach(|py| {
        fixture
            .module
            .bind(py)
            .getattr("mark_waiting")
            .unwrap()
            .unbind()
    });
    node.kernel()
        .exec_engine()
        .borrow_mut()
        .register_client(ready_hosted_sandbox_client(inner, waiting, stopped.clone()))
        .unwrap();
    let trace = Rc::new(RefCell::new(ControlNativeTrace::default()));
    msgbus::set_bus_tap(Rc::new(ControlNativeTap {
        trace: trace.clone(),
        busy_submit: (scenario == "raw_submit_busy").then_some(raw_submit),
        injected: Cell::new(false),
        retirement_observer: (scenario == "retire_writer_first")
            .then(|| Python::attach(|py| fixture.module.clone_ref(py).into_any())),
        capacity_pressure: scenario == "capacity_native_continuation",
        pressure_injected: Cell::new(false),
    }));
    let (py_node, lifecycle) = Python::attach(|py| {
        let stop_handle = node.handle();
        let request_stop = PyCFunction::new_closure(
            py,
            None,
            None,
            move |_args: &pyo3::Bound<'_, PyTuple>, _kwargs| -> pyo3::PyResult<()> {
                stop_handle.stop();
                Ok(())
            },
        )
        .unwrap();
        fixture
            .module
            .bind(py)
            .setattr("request_native_stop", request_stop)
            .unwrap();
        let py_node = pyo3::Py::new(py, PyLiveNode::new(node)).unwrap();
        let globals = PyDict::new(py);
        globals.set_item("node", &py_node).unwrap();
        globals
            .set_item("observer", fixture.module.bind(py))
            .unwrap();
        let result = py.run(
            c_str!(
                r#"
import asyncio
async def exercise():
    capture_handle = node.account_capture_handle(observer.client_id)
    task = asyncio.create_task(node.run_async())
    async def until(predicate):
        while not predicate():
            if task.done():
                await task
                raise AssertionError("native node stopped before the control callback")
            await asyncio.sleep(0)
    try:
        await asyncio.wait_for(until(lambda: observer.started), timeout=3)
        observer.send_control_quote()
        await asyncio.wait_for(until(lambda: observer.control_errors or len(observer.canceled_callbacks) == (2 if observer.with_oco else 1)), timeout=3)
        if observer.scenario.startswith("raw_submit_"):
            observer.send_control_quote()
            await asyncio.wait_for(until(lambda: str(observer.strategy.cache.order(observer.raw_client_order_id).status) != "INITIALIZED" or observer.strategy._control_delivery_status()[1][1] is not None), timeout=3)
    finally:
        node.handle().stop()
        try:
            await asyncio.wait_for(task, timeout=3)
        finally:
            observer.final_capture = observer.json.loads(await capture_handle.capture_json(timeout_ms=100))
asyncio.run(exercise())
"#
            ),
            Some(&globals),
            None,
        );
        (py_node, result)
    });
    msgbus::clear_bus_tap();
    let diagnostic = fixture.diagnostics();
    eprintln!("cross-repository control cancellation evidence: {diagnostic}");
    if raw_scenario {
        let actual = cache
            .borrow()
            .order_owned(&raw_order.client_order_id())
            .unwrap();
        eprintln!(
            "raw registered endpoint actual history: {:?}; commands: {:?}",
            actual.events(),
            trace.borrow().commands
        );
        assert_eq!(
            actual.status(),
            OrderStatus::Initialized,
            "a current control context must not authorize raw SubmitOrder at the registered emulator endpoint"
        );
        assert_eq!(
            actual.events(),
            vec![&OrderEventAny::Initialized(raw_order.init_event().clone())]
        );
        assert!(
            trace
                .borrow()
                .orders
                .iter()
                .all(|event| event.client_order_id() != raw_order.client_order_id())
        );
        assert!(trace.borrow().venue_commands.is_empty());
    }
    if lifecycle.is_err() && expected_failure.is_none() {
        eprintln!(
            "native prefix commands: {:?}; publications: {:?}",
            trace.borrow().commands,
            trace.borrow().orders
        );
        for order in &orders {
            eprintln!(
                "native prefix cache {}: {:?}",
                order.client_order_id(),
                cache
                    .borrow()
                    .order_owned(&order.client_order_id())
                    .map(|order| (order.status(), order.events().len()))
            );
        }
        Python::attach(|py| {
            py_node.call_method0(py, "dispose").unwrap();
        });
    }
    if let Some(expected) = expected_failure {
        let failure = lifecycle.expect_err("fault or retired context must fail the actual owner");
        assert!(failure.to_string().contains(expected), "{failure}");
    } else {
        lifecycle.unwrap();
    }
    if expected_failure.is_some() && !scenario.starts_with("retire_queued_") {
        assert!(
            Rc::strong_count(&emulator) > emulator_initial_owners,
            "a failed phase must keep the actual emulator continuation target strongly owned"
        );
    }
    assert!(stopped.get() >= 1);
    assert_eq!(
        capture.capture().unwrap_err(),
        ExecutionAccountCaptureError::Retired
    );
    Python::attach(|py| {
        assert_eq!(
            py_node
                .call_method0(py, "handle")
                .unwrap()
                .getattr(py, "state")
                .unwrap()
                .extract::<NodeState>(py)
                .unwrap(),
            NodeState::Stopped
        );
    });
    let trace = trace.borrow();
    assert_eq!(trace.quotes, [neutral_quote]);
    assert!(trace.venue_commands.is_empty());
    assert!(trace.raw_execution_events.is_empty());
    let canceled_before_dispatch = scenario.starts_with("retire_queued_");
    assert_eq!(
        trace.commands.len(),
        if canceled_before_dispatch {
            2
        } else if raw_scenario {
            4
        } else {
            3
        }
    );
    assert!(
        matches!(&trace.commands[0], TradingCommand::SubmitOrder(value)
        if value.client_order_id == orders[0].client_order_id())
    );
    assert!(
        matches!(&trace.commands[1], TradingCommand::SubmitOrder(value)
        if value.client_order_id == orders[1].client_order_id())
    );
    if !canceled_before_dispatch {
        assert!(
            matches!(&trace.commands[2], TradingCommand::CancelOrder(value)
        if value.client_order_id == orders[0].client_order_id())
        );
    }
    for (index, status) in [
        if canceled_before_dispatch {
            OrderStatus::Emulated
        } else {
            OrderStatus::Canceled
        },
        if with_oco && expected_failure.is_none() {
            OrderStatus::Canceled
        } else {
            OrderStatus::Emulated
        },
    ]
    .into_iter()
    .enumerate()
    {
        let original = &orders[index];
        let actual = cache
            .borrow()
            .order_owned(&original.client_order_id())
            .unwrap();
        assert_eq!(actual.status(), status);
        assert_eq!(actual.quantity(), original.quantity());
        assert_eq!(actual.filled_qty(), Quantity::zero(2));
        let history = actual.events();
        assert_eq!(
            history.first().unwrap(),
            &&OrderEventAny::Initialized(original.init_event().clone())
        );
        assert_eq!(
            history.len(),
            if status == OrderStatus::Canceled {
                3
            } else {
                2
            }
        );
        assert!(matches!(history[1], OrderEventAny::Emulated(_)));
        if status == OrderStatus::Canceled {
            assert!(matches!(history[2], OrderEventAny::Canceled(_)));
        }
        let published: Vec<_> = trace
            .orders
            .iter()
            .filter(|event| event.client_order_id() == original.client_order_id())
            .collect();
        if scenario == "capacity_native_continuation" && index == 0 {
            assert_eq!(published.len(), 515);
            assert_eq!(published[0..2], history[0..2]);
            assert!(published[2..].iter().all(|event| **event == *history[2]));
        } else {
            assert_eq!(published, history);
        }
    }
    assert!(
        cache
            .borrow()
            .order_owned(&orders[2].client_order_id())
            .is_none()
    );
    assert!(
        cache
            .borrow()
            .positions(
                None,
                Some(&instrument.id()),
                Some(&strategy_id),
                Some(&account_id),
                None
            )
            .is_empty()
    );
    assert_eq!(
        cache
            .borrow()
            .account_owned(&account_id)
            .unwrap()
            .balance_total(None)
            .unwrap(),
        Money::from("100000 USDC")
    );
    let canceled: Vec<_> = trace
        .orders
        .iter()
        .filter_map(|event| match event {
            OrderEventAny::Canceled(value) => Some((
                value.event_id.to_string(),
                value.client_order_id.to_string(),
            )),
            _ => None,
        })
        .collect();
    let expected_delivered = if canceled_before_dispatch
        || scenario == "exit_failure"
        || scenario == "capacity_native_continuation"
    {
        0
    } else if expected_failure.is_some() {
        1
    } else if with_oco {
        2
    } else {
        1
    };
    assert_eq!(
        canceled.len(),
        if scenario == "capacity_native_continuation" {
            513
        } else if canceled_before_dispatch {
            0
        } else if expected_failure.is_some() {
            1
        } else if with_oco {
            2
        } else {
            1
        }
    );
    let observation = Python::attach(|py| {
        let module = fixture.module.bind(py);
        let observed: Vec<(String, String)> = module
            .getattr("canceled_callbacks")
            .unwrap()
            .extract()
            .unwrap();
        assert_eq!(observed, canceled[..expected_delivered]);
        let status: (bool, Option<(usize, Option<String>)>) = module
            .getattr("strategy")
            .unwrap()
            .call_method0("_control_delivery_status")
            .unwrap()
            .extract()
            .unwrap();
        let retained: Vec<(String, Option<String>, String, Option<String>)> = module
            .getattr("strategy")
            .unwrap()
            .call_method0("_control_delivery_failures")
            .unwrap()
            .extract()
            .unwrap();
        if scenario == "callback_failure" {
            let command_id = match &trace.commands[2] {
                TradingCommand::CancelOrder(command) => command.command_id.to_string(),
                _ => unreachable!(),
            };
            assert_eq!(
                retained,
                vec![(
                    command_id,
                    Some(canceled[0].0.clone()),
                    canceled[0].0.clone(),
                    Some("native owner delivery ExternalCallback".to_owned())
                )],
                "retain the failed specific item itself and its original cause after attempting aggregate"
            );
        }
        if scenario == "exit_failure" {
            assert_eq!(
                retained.len(),
                2,
                "the phase's unplayed specific and aggregate remain owned"
            );
            assert!(
                retained
                    .iter()
                    .all(|item| item.2 == canceled[0].0 && item.3.is_none())
            );
        }
        if let Some(expected) = expected_failure {
            let (pending, failure) = status.1.as_ref().unwrap();
            assert!(
                *pending > 0,
                "failed owner must retain native or external work"
            );
            assert!(failure.as_ref().unwrap().contains(expected));
        }
        if scenario == "capacity_native_continuation" {
            let pending = status.1.as_ref().unwrap().0;
            assert_eq!(
                pending, 1027,
                "1026 original external items and the one already accepted real emulator continuation must remain owned"
            );
            assert_eq!(retained.len(), 1026);
            assert!(retained.iter().all(|item| item.2 == canceled[0].0));
        }
        let final_capture = module.getattr("final_capture").unwrap();
        assert_eq!(
            final_capture
                .get_item("status")
                .unwrap()
                .extract::<String>()
                .unwrap(),
            "unavailable"
        );
        assert_eq!(
            final_capture
                .get_item("reason")
                .unwrap()
                .extract::<String>()
                .unwrap(),
            "retired"
        );
        let result = module
            .call_method0("assert_control_released_and_observations")
            .map(|_| ());
        py_node.call_method0(py, "dispose").unwrap();
        result
    });
    let final_diagnostic = fixture.diagnostics();
    eprintln!("cross-repository control cancellation final evidence: {final_diagnostic}");
    if let Err(e) = observation {
        panic!(
            "actual native cancellation completed but control/callback observations failed: {e}; evidence={final_diagnostic}"
        );
    }
}
