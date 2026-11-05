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

//! Neutral HTTP requests with same-thread Python control scopes.
//!
//! These scopes do not establish financial authority. A strong operation handle and its
//! transport retain cleanup ownership independently of the caller's response future.
//! A factory is not provenance; an execution owner must separately bind its actual authority.
//! Unobserved operations occupy the bounded owner until consumed or explicitly reaped.
//!
//! Completion waits for the actual driver and blocking worker joins. Cancellation never undoes
//! a possible transmission, and cleanup failure does not erase an already received response.
//! Python must remain initialized until all scopes have actually finished, even when the original
//! Tokio runtime has shut down and another runtime performs the reap.

use std::{
    collections::BTreeMap,
    fmt,
    future::Future,
    num::NonZeroUsize,
    panic::{AssertUnwindSafe, catch_unwind},
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    task::{Context, Poll},
};

use bytes::Bytes;
use futures_util::{
    FutureExt,
    future::{BoxFuture, Shared, poll_fn},
    task::AtomicWaker,
};
use http::{Request, Response};
use http_body_util::Full;
use hyper::body::Incoming;
use parking_lot::Mutex;
// tokio-import-ok: Python transactions require actual OS threads, excluded under madsim
use tokio::runtime::Handle;
use tokio_util::sync::CancellationToken;

use super::transport::{Client, Settings, retryable};
use crate::python::control_handoff::PythonControlScope;

type TransportFuture = Pin<
    Box<dyn Future<Output = Result<Response<Incoming>, hyper_util::client::legacy::Error>> + Send>,
>;
type InitialPoll = (
    Poll<Result<Response<Incoming>, hyper_util::client::legacy::Error>>,
    TransportFuture,
);
type TaskCompletion = Shared<BoxFuture<'static, Result<(), HandoffFailure>>>;

/// A classified failure without Python exception or request contents.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HandoffFailure {
    Capacity,
    Canceled,
    ControlEnter,
    ControlExit,
    DriverLost,
    DriverPanicked,
    PollPanicked,
    ResponseConsumed,
    Transport,
    WakeExhausted,
    WorkerLost,
    WorkerPanicked,
}

/// A failure preserving whether transport might already have observed the request.
#[derive(Clone, Debug, thiserror::Error)]
#[error("controlled HTTP handoff failed: {failure:?}; may_have_sent={may_have_sent}")]
pub struct HandoffError {
    pub failure: HandoffFailure,
    pub may_have_sent: bool,
}

/// Terminal cleanup evidence for one neutral request.
#[derive(Clone, Debug)]
pub struct HandoffCompletion {
    pub may_have_sent: bool,
    pub failure: Option<HandoffFailure>,
}

/// A separate HTTP pool that disables library-internal canceled-request retries.
#[derive(Clone, Debug)]
pub struct ControlledHttpTransport {
    client: Client,
    runtime: Handle,
    operations: Arc<Mutex<BTreeMap<u64, Arc<OperationState>>>>,
    capacity: usize,
    next_id: Arc<Mutex<u64>>,
}

impl ControlledHttpTransport {
    /// Creates a bounded owner for actual Tokio and Python control scopes.
    ///
    /// # Errors
    ///
    /// Returns an error when the private transport cannot be built.
    pub fn new(runtime: Handle, capacity: NonZeroUsize) -> Result<Self, super::HttpClientError> {
        Ok(Self {
            client: Client::new_controlled(Settings::default())?,
            runtime,
            operations: Arc::default(),
            capacity: capacity.get(),
            next_id: Arc::new(Mutex::new(0)),
        })
    }

    /// Starts a neutral request and retains its cleanup independently of its waiter.
    ///
    /// # Errors
    ///
    /// Returns an error if pending or unobserved operations exhaust the bounded owner.
    pub fn start(
        &self,
        request: Request<Full<Bytes>>,
        scope: Arc<PythonControlScope>,
    ) -> Result<ControlledHttpOperation, HandoffError> {
        let mut operations = self.operations.lock();
        let mut next_id = self.next_id.lock();
        if operations.len() >= self.capacity || *next_id == u64::MAX {
            return Err(HandoffError {
                failure: HandoffFailure::Capacity,
                may_have_sent: false,
            });
        }
        *next_id += 1;
        let id = *next_id;
        let state = Arc::new(OperationState::default());
        operations.insert(id, state.clone());
        let driver = self.runtime.spawn(drive(
            self.client.clone(),
            self.runtime.clone(),
            request,
            scope,
            state.clone(),
        ));
        state.inner.lock().driver = Some(
            driver
                .map(|result| {
                    classify_join(
                        result,
                        HandoffFailure::DriverPanicked,
                        HandoffFailure::DriverLost,
                    )
                })
                .boxed()
                .shared(),
        );
        Ok(ControlledHttpOperation {
            state,
            owner: self.operations.clone(),
            id,
        })
    }

    /// Cancels retained work and waits for actual scope cleanup before clearing it.
    ///
    /// # Errors
    ///
    /// Returns an error if cleanup, polling, or transport failed rather than being canceled.
    pub async fn reap(&self) -> Result<Vec<HandoffCompletion>, HandoffError> {
        let retained: Vec<_> = self
            .operations
            .lock()
            .iter()
            .map(|(&id, s)| (id, s.clone()))
            .collect();
        for (_, state) in &retained {
            state.cancel.cancel();
        }
        let mut results = Vec::new();
        let mut failure = None;
        for (id, state) in retained {
            let completion = state.finished().await;
            if let Some(kind) = completion.failure
                && kind != HandoffFailure::Canceled
            {
                failure = Some(HandoffError {
                    failure: kind,
                    may_have_sent: completion.may_have_sent,
                });
            }
            self.operations.lock().remove(&id);
            results.push(completion);
        }
        match failure {
            Some(e) => Err(e),
            None => Ok(results),
        }
    }
}

/// A strong request handle whose cancellation remains awaitable after waiter cancellation.
#[derive(Debug)]
pub struct ControlledHttpOperation {
    state: Arc<OperationState>,
    owner: Arc<Mutex<BTreeMap<u64, Arc<OperationState>>>>,
    id: u64,
}

impl ControlledHttpOperation {
    /// Waits for one response, after the synchronous control scope has exited.
    ///
    /// # Errors
    ///
    /// Returns the classified failure or an error when the response was already consumed.
    pub async fn wait(&self) -> Result<Response<Incoming>, HandoffError> {
        let completion = self.state.finished().await;
        self.owner.lock().remove(&self.id);
        if let Some(failure) = completion.failure {
            return Err(HandoffError {
                failure,
                may_have_sent: completion.may_have_sent,
            });
        }
        self.state.inner.lock().response.take().ok_or(HandoffError {
            failure: HandoffFailure::ResponseConsumed,
            may_have_sent: completion.may_have_sent,
        })
    }

    /// Requests cancellation and waits until the real worker and context have finished.
    pub async fn cancel_and_wait(&self) -> HandoffCompletion {
        self.state.cancel.cancel();
        let completion = self.state.finished().await;
        self.owner.lock().remove(&self.id);
        completion
    }

    /// Takes a received response even when subsequent task cleanup failed.
    ///
    /// This does not clear the cleanup failure or establish execution authority.
    /// Concurrent consumers receive the response at most once; later consumers receive `None`.
    pub async fn take_received_response(&self) -> Option<Response<Incoming>> {
        self.state.finished().await;
        self.owner.lock().remove(&self.id);
        self.state.inner.lock().response.take()
    }
}

impl Drop for ControlledHttpOperation {
    fn drop(&mut self) {
        self.state.cancel.cancel();
    }
}

#[derive(Debug, Default)]
struct OperationState {
    inner: Mutex<OperationInner>,
    cancel: CancellationToken,
}

#[derive(Default)]
struct OperationInner {
    response: Option<Response<Incoming>>,
    completion: Option<HandoffCompletion>,
    driver: Option<TaskCompletion>,
    worker: Option<TaskCompletion>,
    worker_active: bool,
    abandoned: bool,
    may_have_sent: bool,
}

impl OperationState {
    fn finish(&self, result: Result<Response<Incoming>, HandoffFailure>) {
        let mut inner = self.inner.lock();
        if inner.completion.is_some() {
            return;
        }
        let failure = match result {
            Ok(response) => {
                inner.response = Some(response);
                None
            }
            Err(e) => Some(e),
        };
        inner.completion = Some(HandoffCompletion {
            may_have_sent: inner.may_have_sent,
            failure,
        });
    }

    async fn finished(&self) -> HandoffCompletion {
        let driver = self.inner.lock().driver.clone();
        let driver_failure = match driver {
            Some(driver) => driver.await.err(),
            None => Some(HandoffFailure::DriverLost),
        };
        // The completed driver cannot create another worker after this snapshot
        let worker = self.inner.lock().worker.clone();
        let worker_failure = match worker {
            Some(worker) => worker.await.err(),
            None => None,
        };
        self.inner.lock().worker_active = false;
        if let Some(failure) = worker_failure.or(driver_failure) {
            let mut inner = self.inner.lock();
            inner.completion = Some(HandoffCompletion {
                may_have_sent: inner.may_have_sent,
                failure: Some(failure),
            });
        } else if self.inner.lock().completion.is_none() {
            self.finish(Err(worker_failure
                .or(driver_failure)
                .unwrap_or(HandoffFailure::DriverLost)));
        }
        self.inner
            .lock()
            .completion
            .clone()
            .expect("joined operation has a terminal result")
    }

    fn abandon(&self) {
        let mut inner = self.inner.lock();
        inner.abandoned = true;
        self.cancel.cancel();
    }
}

impl fmt::Debug for OperationInner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OperationInner")
            .field("completion", &self.completion)
            .field("worker_active", &self.worker_active)
            .field("abandoned", &self.abandoned)
            .field("may_have_sent", &self.may_have_sent)
            .finish_non_exhaustive()
    }
}

fn classify_join(
    result: Result<(), tokio::task::JoinError>,
    panicked: HandoffFailure,
    lost: HandoffFailure,
) -> Result<(), HandoffFailure> {
    result.map_err(|e| if e.is_panic() { panicked } else { lost })
}

struct DriverGuard(Arc<OperationState>);

impl Drop for DriverGuard {
    fn drop(&mut self) {
        self.0.abandon();
    }
}

async fn drive(
    client: Client,
    runtime: Handle,
    request: Request<Full<Bytes>>,
    scope: Arc<PythonControlScope>,
    state: Arc<OperationState>,
) {
    let _cleanup = DriverGuard(state.clone());
    for attempt in 0..=2 {
        if state.cancel.is_cancelled() {
            state.finish(Err(HandoffFailure::Canceled));
            return;
        }
        let future: TransportFuture = Box::pin(client.request_once(request.clone()));
        state.inner.lock().worker_active = true;
        let worker_state = state.clone();
        let worker_scope = scope.clone();
        let worker_runtime = runtime.clone();
        let wake = Arc::new(PollWake::default());
        let worker_wake = wake.clone();
        let (result_sender, result_receiver) = tokio::sync::oneshot::channel();
        let worker = runtime.spawn_blocking(move || {
            let result = catch_unwind(AssertUnwindSafe(|| {
                first_poll(
                    future,
                    &worker_scope,
                    &worker_runtime,
                    &worker_state,
                    worker_wake,
                )
            }));
            let result = result.unwrap_or(Err(HandoffFailure::WorkerPanicked));
            if let Err(returned) = result_sender.send(result) {
                // A retired driver leaves future destruction with this worker
                drop(returned);
            }
        });
        let mut worker = worker
            .map(|result| {
                classify_join(
                    result,
                    HandoffFailure::WorkerPanicked,
                    HandoffFailure::WorkerLost,
                )
            })
            .boxed()
            .shared();
        state.inner.lock().worker = Some(worker.clone());
        // Cancellation cannot abort a blocking worker holding a Python transaction
        let joined = poll_fn(|cx| {
            wake.driver.register(cx.waker());
            Pin::new(&mut worker).poll(cx)
        })
        .await;
        state.inner.lock().worker_active = false;
        if let Err(e) = joined {
            state.finish(Err(e));
            return;
        }
        let result = match result_receiver.await {
            Ok(result) => result,
            Err(_) => Err(HandoffFailure::WorkerLost),
        };
        let result = match result {
            Ok((Poll::Ready(result), _)) => result,
            Ok((Poll::Pending, mut future)) => {
                tokio::select! {
                    biased;
                    () = state.cancel.cancelled() => {
                        drop(future);
                        state.finish(Err(HandoffFailure::Canceled));
                        return;
                    }
                    result = poll_fn(|cx| {
                        wake.driver.register(cx.waker());
                        future.as_mut().poll(cx)
                    }) => result,
                }
            }
            Err(e) => {
                state.finish(Err(e));
                return;
            }
        };
        if wake.overflow.load(Ordering::Acquire) {
            state.finish(Err(HandoffFailure::WakeExhausted));
            return;
        }
        match result {
            Ok(response) => {
                state.finish(Ok(response));
                return;
            }
            Err(e) if attempt < 2 && retryable(&e) => {}
            Err(_) => {
                state.finish(Err(HandoffFailure::Transport));
                return;
            }
        }
    }
}

fn first_poll(
    mut future: TransportFuture,
    scope: &PythonControlScope,
    runtime: &Handle,
    state: &OperationState,
    wake: Arc<PollWake>,
) -> Result<InitialPoll, HandoffFailure> {
    let mut entered = scope.enter().map_err(|_| HandoffFailure::ControlEnter)?;
    if state.cancel.is_cancelled() {
        entered
            .finish(true)
            .map_err(|_| HandoffFailure::ControlExit)?;
        return Err(HandoffFailure::Canceled);
    }
    let waker = std::task::Waker::from(wake);
    let result = catch_unwind(AssertUnwindSafe(|| {
        let _runtime = runtime.enter();
        state.inner.lock().may_have_sent = true;
        future.as_mut().poll(&mut Context::from_waker(&waker))
    }));
    entered
        .finish(result.is_err())
        .map_err(|_| HandoffFailure::ControlExit)?;
    let poll = result.map_err(|_| HandoffFailure::PollPanicked)?;
    Ok((poll, future))
}

#[derive(Default)]
struct PollWake {
    driver: AtomicWaker,
    sequence: AtomicU64,
    overflow: AtomicBool,
}

impl PollWake {
    fn notify(&self) {
        if self
            .sequence
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_add(1))
            .is_err()
        {
            self.overflow.store(true, Ordering::Release);
        }
        self.driver.wake();
    }
}

impl std::task::Wake for PollWake {
    fn wake(self: Arc<Self>) {
        self.notify();
    }
    fn wake_by_ref(self: &Arc<Self>) {
        self.notify();
    }
}

#[cfg(test)]
mod tests {
    use std::{ffi::CString, sync::atomic::AtomicUsize};

    use http_body_util::BodyExt;
    use pyo3::{prelude::*, types::PyModule};
    use rstest::rstest;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn real_hyper_future_survives_early_wakes_and_pending_owner_transfer() {
        let fixture = SqliteScope::new();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (release_response, response_allowed) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut header = Vec::new();
            while !header.ends_with(b"\r\n\r\n") {
                header.push(stream.read_u8().await.unwrap());
            }
            response_allowed.await.unwrap();
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
                .await
                .unwrap();
        });
        let wake = Arc::new(PollWake::default());
        let observed = Arc::new(AtomicUsize::new(0));
        let future: TransportFuture = Box::pin(ObservedHyper {
            inner: actual_hyper_future(address),
            observed: observed.clone(),
            panic_after_poll: false,
        });
        let runtime = Handle::current();
        let scope = fixture.scope.clone();
        let worker_wake = wake.clone();
        let (poll, mut future) = runtime
            .clone()
            .spawn_blocking(move || {
                first_poll(
                    future,
                    &scope,
                    &runtime,
                    &OperationState::default(),
                    worker_wake,
                )
            })
            .await
            .unwrap()
            .unwrap();
        assert!(poll.is_pending());
        assert_eq!(observed.load(Ordering::Acquire), 1);
        let before = wake.sequence.load(Ordering::Acquire);
        assert!(before > 0);
        std::task::Wake::wake_by_ref(&wake);
        assert_eq!(wake.sequence.load(Ordering::Acquire), before + 1);
        fixture.assert_rows_and_unlocked(1);
        release_response.send(()).unwrap();
        let response = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            poll_fn(|cx| {
                wake.driver.register(cx.waker());
                future.as_mut().poll(cx)
            }),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(
            response.into_body().collect().await.unwrap().to_bytes(),
            "ok"
        );
        assert!(observed.load(Ordering::Acquire) > 1);
        server.await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_panic_after_polling_actual_hyper_rolls_back_on_the_same_worker() {
        let fixture = SqliteScope::new();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let future: TransportFuture = Box::pin(ObservedHyper {
            inner: actual_hyper_future(listener.local_addr().unwrap()),
            observed: Arc::default(),
            panic_after_poll: true,
        });
        let runtime = Handle::current();
        let scope = fixture.scope.clone();
        let state = Arc::new(OperationState::default());
        let worker_state = state.clone();
        let result = runtime
            .clone()
            .spawn_blocking(move || {
                first_poll(future, &scope, &runtime, &worker_state, Arc::default())
            })
            .await
            .unwrap();
        assert!(matches!(result, Err(HandoffFailure::PollPanicked)));
        assert!(state.inner.lock().may_have_sent);
        fixture.assert_rows_and_unlocked(0);
    }

    #[rstest]
    #[case::existing_default(false)]
    #[case::controlled(true)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn canceled_reused_connections_retry_only_in_the_existing_default_client(
        #[case] controlled: bool,
    ) {
        let fixture = SqliteScope::new();
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        let client = if controlled {
            Client::new_controlled(Settings::default()).unwrap()
        } else {
            Client::new(None, false, Settings::default()).unwrap()
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (old_closed, closure_observed) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut first, _) = listener.accept().await.unwrap();
            assert!(
                read_request_header(&mut first)
                    .await
                    .starts_with(b"GET /seed ")
            );
            first
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
                .await
                .unwrap();
            let mut byte = [0];
            assert_eq!(first.read(&mut byte).await.unwrap(), 0);
            old_closed.send(()).unwrap();
            if controlled {
                assert!(
                    tokio::time::timeout(std::time::Duration::from_millis(100), listener.accept())
                        .await
                        .is_err()
                );
            } else {
                let (mut replacement, _) =
                    tokio::time::timeout(std::time::Duration::from_secs(5), listener.accept())
                        .await
                        .unwrap()
                        .unwrap();
                assert!(
                    read_request_header(&mut replacement)
                        .await
                        .starts_with(b"GET /after-cancel ")
                );
                replacement
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
                    .await
                    .unwrap();
            }
        });
        let mut old_runtime = RuntimeFreeze::new();
        let seed_client = client.clone();
        let mut seed_request = Request::get(format!("http://{address}/seed"))
            .body(Full::new(Bytes::new()))
            .unwrap();
        let seed_capture =
            hyper_util::client::legacy::connect::capture_connection(&mut seed_request);
        let seed_runtime = old_runtime.runtime.take().unwrap();
        let seed_runtime = tokio::task::spawn_blocking(move || {
            seed_runtime.block_on(async move {
                let response = seed_client.request_once(seed_request).await.unwrap();
                response.into_body().collect().await.unwrap();
                tokio::task::yield_now().await;
            });
            seed_runtime
        })
        .await
        .unwrap();
        old_runtime.runtime = Some(seed_runtime);
        let original_socket = captured_socket(&seed_capture);
        let mut next_request = Request::get(format!("http://{address}/after-cancel"))
            .body(Full::new(Bytes::new()))
            .unwrap();
        let next_capture =
            hyper_util::client::legacy::connect::capture_connection(&mut next_request);
        let future: TransportFuture = Box::pin(client.request_once(next_request));
        let scope = fixture.scope.clone();
        let runtime = Handle::current();
        let wake = Arc::new(PollWake::default());
        let worker_wake = wake.clone();
        let (poll, mut future) = runtime
            .clone()
            .spawn_blocking(move || {
                first_poll(
                    future,
                    &scope,
                    &runtime,
                    &OperationState::default(),
                    worker_wake,
                )
            })
            .await
            .unwrap()
            .unwrap();
        assert!(poll.is_pending());
        assert_eq!(captured_socket(&next_capture), original_socket);
        fixture.assert_rows_and_unlocked(1);
        old_runtime.shutdown();
        tokio::time::timeout(std::time::Duration::from_secs(5), closure_observed)
            .await
            .unwrap()
            .unwrap();
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            poll_fn(|cx| {
                wake.driver.register(cx.waker());
                future.as_mut().poll(cx)
            }),
        )
        .await
        .unwrap();
        if controlled {
            let e = result.unwrap_err();
            assert!(!e.is_connect());
            assert!(
                std::error::Error::source(&e)
                    .and_then(|source| source.downcast_ref::<hyper::Error>())
                    .is_some_and(hyper::Error::is_canceled),
                "{e:?}"
            );
        } else {
            let response = result.unwrap();
            assert_eq!(response.status(), http::StatusCode::OK);
            response.into_body().collect().await.unwrap();
        }
        fixture.assert_rows_and_unlocked(1);
        server.await.unwrap();
    }

    fn captured_socket(
        capture: &hyper_util::client::legacy::connect::CaptureConnection,
    ) -> std::net::SocketAddr {
        let mut extra = http::Extensions::new();
        capture
            .connection_metadata()
            .as_ref()
            .expect("the first poll must have selected the existing idle connection")
            .get_extras(&mut extra);
        extra
            .get::<hyper_util::client::legacy::connect::HttpInfo>()
            .unwrap()
            .local_addr()
    }

    async fn read_request_header(stream: &mut tokio::net::TcpStream) -> Vec<u8> {
        let mut bytes = Vec::new();
        while !bytes.ends_with(b"\r\n\r\n") {
            bytes.push(stream.read_u8().await.unwrap());
        }
        bytes
    }

    struct RuntimeFreeze {
        runtime: Option<tokio::runtime::Runtime>,
    }

    impl RuntimeFreeze {
        fn new() -> Self {
            Self {
                runtime: Some(
                    tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .unwrap(),
                ),
            }
        }

        fn shutdown(&mut self) {
            if let Some(runtime) = self.runtime.take() {
                runtime.shutdown_background();
            }
        }
    }

    impl Drop for RuntimeFreeze {
        fn drop(&mut self) {
            self.shutdown();
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn actual_hyper_poll_progresses_under_another_gil_owner_then_waits_for_exit() {
        let fixture = SqliteScope::new();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (allow_response, response_allowed) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            read_request_header(&mut stream).await;
            response_allowed.await.unwrap();
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
                .await
                .unwrap();
        });
        let poll_resume = Arc::new(ThreadGate::default());
        let gil_release = Arc::new(ThreadGate::default());
        let _poll_cleanup = OpenOnDrop(poll_resume.clone());
        let _gil_cleanup = OpenOnDrop(gil_release.clone());
        let polled = Arc::new(AtomicBool::new(false));
        let (before_poll, poll_waiting) = std::sync::mpsc::channel();
        let future: TransportFuture = Box::pin(GilObservedHyper {
            inner: actual_hyper_future(address),
            before_poll: Some(before_poll),
            resume: poll_resume.clone(),
            polled: polled.clone(),
        });
        let scope = fixture.scope.clone();
        let runtime = Handle::current();
        let wake = Arc::new(PollWake::default());
        let worker_wake = wake.clone();
        let worker = runtime.clone().spawn_blocking(move || {
            first_poll(
                future,
                &scope,
                &runtime,
                &OperationState::default(),
                worker_wake,
            )
        });
        poll_waiting
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        let (gil_held, holder_ready) = std::sync::mpsc::channel();
        let holder_release = gil_release.clone();
        let holder = std::thread::spawn(move || {
            Python::attach(|_py| {
                gil_held.send(()).unwrap();
                holder_release.wait();
            });
        });
        holder_ready
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        poll_resume.open();
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while !polled.load(Ordering::Acquire) {
                tokio::time::sleep(std::time::Duration::from_millis(1)).await;
            }
        })
        .await
        .unwrap();
        assert!(
            !worker.is_finished(),
            "Python exit must still wait for the actual GIL owner"
        );
        gil_release.open();
        holder.join().unwrap();
        let (poll, mut future) = worker.await.unwrap().unwrap();
        assert!(poll.is_pending());
        fixture.assert_rows_and_unlocked(1);
        allow_response.send(()).unwrap();
        let response = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            poll_fn(|cx| {
                wake.driver.register(cx.waker());
                future.as_mut().poll(cx)
            }),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(
            response.into_body().collect().await.unwrap().to_bytes(),
            "ok"
        );
        server.await.unwrap();
    }

    #[derive(Default)]
    struct ThreadGate {
        open: std::sync::Mutex<bool>,
        changed: std::sync::Condvar,
    }

    impl ThreadGate {
        fn open(&self) {
            *self.open.lock().unwrap() = true;
            self.changed.notify_all();
        }

        fn wait(&self) {
            let mut open = self.open.lock().unwrap();
            while !*open {
                open = self.changed.wait(open).unwrap();
            }
        }
    }

    struct OpenOnDrop(Arc<ThreadGate>);

    impl Drop for OpenOnDrop {
        fn drop(&mut self) {
            self.0.open();
        }
    }

    struct GilObservedHyper {
        inner: TransportFuture,
        before_poll: Option<std::sync::mpsc::Sender<()>>,
        resume: Arc<ThreadGate>,
        polled: Arc<AtomicBool>,
    }

    impl Future for GilObservedHyper {
        type Output = Result<Response<Incoming>, hyper_util::client::legacy::Error>;

        fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
            if let Some(waiting) = self.before_poll.take() {
                waiting.send(()).unwrap();
                self.resume.wait();
            }
            let result = self.inner.as_mut().poll(cx);
            self.polled.store(true, Ordering::Release);
            result
        }
    }

    fn actual_hyper_future(address: std::net::SocketAddr) -> TransportFuture {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        let client = Client::new_controlled(Settings::default()).unwrap();
        let request = Request::get(format!("http://{address}/neutral-wake"))
            .body(Full::new(Bytes::new()))
            .unwrap();
        Box::pin(client.request_once(request))
    }

    struct ObservedHyper {
        inner: TransportFuture,
        observed: Arc<AtomicUsize>,
        panic_after_poll: bool,
    }

    impl Future for ObservedHyper {
        type Output = Result<Response<Incoming>, hyper_util::client::legacy::Error>;

        fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
            let count = self.observed.fetch_add(1, Ordering::AcqRel);
            let result = self.inner.as_mut().poll(cx);
            assert!(!self.panic_after_poll, "panic after an actual Hyper poll");
            if count == 0 {
                cx.waker().wake_by_ref();
            }
            result
        }
    }

    struct SqliteScope {
        _directory: tempfile::TempDir,
        module: Py<PyModule>,
        scope: Arc<PythonControlScope>,
    }

    impl SqliteScope {
        fn new() -> Self {
            let directory = tempfile::tempdir().unwrap();
            Python::initialize();
            let (module, scope) = Python::attach(|py| {
                let module = PyModule::new(py, "neutral_sqlite_scope").unwrap();
                module
                    .setattr("database", directory.path().join("scope.sqlite3"))
                    .unwrap();
                let code = CString::new(
                    r#"
import contextlib
import sqlite3
import threading
threads = []
with sqlite3.connect(database) as c:
    c.execute("CREATE TABLE attempts (value INTEGER NOT NULL)")

@contextlib.contextmanager
def transaction():
    connection = sqlite3.connect(database, timeout=0.1)
    try:
        connection.execute("BEGIN IMMEDIATE")
        connection.execute("INSERT INTO attempts VALUES (1)")
        threads.append(threading.get_ident())
        try:
            yield
        except BaseException:
            connection.rollback()
            raise
        else:
            connection.commit()
    finally:
        threads.append(threading.get_ident())
        connection.close()
"#,
                )
                .unwrap();
                py.run(&code, Some(&module.dict()), None).unwrap();
                let scope = Arc::new(
                    PythonControlScope::new(module.getattr("transaction").unwrap().unbind())
                        .unwrap(),
                );
                (module.unbind(), scope)
            });
            Self {
                _directory: directory,
                module,
                scope,
            }
        }

        fn assert_rows_and_unlocked(&self, expected: usize) {
            Python::attach(|py| {
                let module = self.module.bind(py);
                module.setattr("expected_rows", expected).unwrap();
                py.run(
                    &CString::new(
                        r#"
assert len(set(threads)) == 1
with sqlite3.connect(database, timeout=0.01) as c:
    c.execute("BEGIN IMMEDIATE")
    assert c.execute("SELECT COUNT(*) FROM attempts").fetchone()[0] == expected_rows
"#,
                    )
                    .unwrap(),
                    Some(&module.dict()),
                    None,
                )
                .unwrap();
            });
        }
    }
}
