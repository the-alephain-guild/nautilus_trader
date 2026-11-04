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

//! Bounded owner requests and checked ingress frontiers for native account captures.

use std::{
    cell::RefCell,
    sync::{
        Arc, Weak,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

use nautilus_common::{
    clients::{
        ExecutionAccountCaptureError, ExecutionAccountCaptureSource, ExecutionAccountSnapshot,
        ExecutionClient,
    },
    enums::Environment,
    live::dst,
};
use nautilus_core::UUID4;
use nautilus_model::identifiers::ClientId;
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, oneshot};

use super::{LiveNode, LiveNodeHandle, NodeState};

const CAPTURE_QUEUE_CAPACITY: usize = 32;
pub(crate) const TIME_EVENTS: usize = 0;
pub(crate) const SYSTEM_EVENTS: usize = 1;
pub(crate) const SYSTEM_COMMANDS: usize = 2;
pub(crate) const EXEC_EVENTS: usize = 3;
pub(crate) const EXEC_COMMANDS: usize = 4;
pub(crate) const DATA_EVENTS: usize = 5;
pub(crate) const DATA_COMMANDS: usize = 6;
pub(crate) const EXTERNAL_EVENTS: usize = 7;
pub(crate) const MAINTENANCE: usize = 8;
const CHANNELS: [&str; 9] = [
    "time_events",
    "system_events",
    "system_commands",
    "execution_events",
    "execution_commands",
    "data_events",
    "data_commands",
    "external_events",
    "maintenance",
];

thread_local! {
    static CURRENT_PROGRESS: RefCell<Weak<CaptureProgress>> = const { RefCell::new(Weak::new()) };
}

/// Applied and admitted work at one owner-thread snapshot boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionAccountCaptureFrontier {
    pub channel: String,
    pub applied: u64,
    pub admitted: u64,
}

/// Actual execution-owner state copied behind the live runtime's ingress frontier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionAccountCapture {
    pub node_instance_id: UUID4,
    pub environment: String,
    pub capture_sequence: u64,
    pub frontier: Vec<ExecutionAccountCaptureFrontier>,
    pub snapshot: ExecutionAccountSnapshot,
}

/// Closed failure reasons which never certify an empty account.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionAccountCaptureFailure {
    #[error("Account capture node is not running")]
    NotRunning,
    #[error("Account capture handle retired")]
    Retired,
    #[error("Account capture request queue is full")]
    QueueFull,
    #[error("Account capture request queue closed")]
    QueueClosed,
    #[error("Account capture request timed out")]
    TimedOut,
    #[error("Account capture request cancelled")]
    Cancelled,
    #[error("Account capture has pending ingress")]
    PendingIngress,
    #[error("Account capture counter overflowed")]
    CounterOverflow,
    #[error("Account capture runtime ownership differs")]
    ForeignRuntime,
    #[error("Account capture runtime configuration is unsupported")]
    UnsupportedRuntime,
    #[error("Account capture owner is borrowed")]
    Borrowed,
    #[error("Account capture current account is unavailable")]
    AccountUnavailable,
    #[error("Account capture owner configuration is unsupported")]
    UnsupportedConfiguration,
    #[error("Account capture owner state is inconsistent")]
    InconsistentState,
}

impl From<ExecutionAccountCaptureError> for ExecutionAccountCaptureFailure {
    fn from(value: ExecutionAccountCaptureError) -> Self {
        match value {
            ExecutionAccountCaptureError::Retired => Self::Retired,
            ExecutionAccountCaptureError::Borrowed => Self::Borrowed,
            ExecutionAccountCaptureError::AccountUnavailable => Self::AccountUnavailable,
            ExecutionAccountCaptureError::UnsupportedConfiguration => {
                Self::UnsupportedConfiguration
            }
            ExecutionAccountCaptureError::InconsistentState => Self::InconsistentState,
            ExecutionAccountCaptureError::PendingNativeState => Self::PendingIngress,
        }
    }
}

/// Thread-safe transport to a particular native client, not a native-state pointer.
#[derive(Debug, Clone)]
pub struct ExecutionAccountCaptureHandle {
    source: ExecutionAccountCaptureSource,
    requests: mpsc::Sender<CaptureRequest>,
    node: LiveNodeHandle,
}

impl ExecutionAccountCaptureHandle {
    /// Returns identity captured from the actual execution registry before the run.
    #[must_use]
    pub const fn source(&self) -> &ExecutionAccountCaptureSource {
        &self.source
    }

    /// Requests one bounded observation from the owner's runtime loop.
    ///
    /// # Errors
    ///
    /// Returns a closed failure for a stopped owner, queue pressure, timeout,
    /// pending ingress, conflicting borrow or unavailable actual account.
    pub async fn capture(
        &self,
        timeout: Duration,
    ) -> Result<ExecutionAccountCapture, ExecutionAccountCaptureFailure> {
        if self.node.should_stop()
            || matches!(
                self.node.state(),
                NodeState::ShuttingDown | NodeState::Stopped
            )
        {
            return Err(ExecutionAccountCaptureFailure::Retired);
        }
        if !self.node.is_running() {
            return Err(ExecutionAccountCaptureFailure::NotRunning);
        }
        if timeout.is_zero() || timeout > Duration::from_secs(60) {
            return Err(ExecutionAccountCaptureFailure::TimedOut);
        }
        let (reply, response) = oneshot::channel();
        let deadline = dst::time::Instant::now()
            .checked_add(timeout)
            .ok_or(ExecutionAccountCaptureFailure::TimedOut)?;
        self.requests
            .try_send(CaptureRequest {
                source: self.source.clone(),
                deadline,
                reply,
            })
            .map_err(|e| match e {
                mpsc::error::TrySendError::Full(_) => ExecutionAccountCaptureFailure::QueueFull,
                mpsc::error::TrySendError::Closed(_) => ExecutionAccountCaptureFailure::QueueClosed,
            })?;
        let result = dst::time::timeout(timeout, response)
            .await
            .map_err(|_| ExecutionAccountCaptureFailure::TimedOut)?
            .map_err(|_| ExecutionAccountCaptureFailure::QueueClosed)??;
        if self.node.should_stop() || !self.node.is_running() {
            return Err(ExecutionAccountCaptureFailure::Retired);
        }
        if result.snapshot.source != self.source {
            return Err(ExecutionAccountCaptureFailure::Retired);
        }
        Ok(result)
    }
}

#[derive(Debug)]
pub(crate) struct CaptureRequest {
    source: ExecutionAccountCaptureSource,
    deadline: dst::time::Instant,
    reply: oneshot::Sender<Result<ExecutionAccountCapture, ExecutionAccountCaptureFailure>>,
}

#[derive(Debug, Default)]
pub(crate) struct CaptureProgress {
    enabled: AtomicBool,
    applied: [AtomicU64; 9],
    active: AtomicU64,
    overflowed: AtomicBool,
}

impl CaptureProgress {
    pub(crate) fn bind(self: &Arc<Self>) {
        CURRENT_PROGRESS.with(|current| {
            *current.borrow_mut() = if self.enabled.load(Ordering::SeqCst) {
                Arc::downgrade(self)
            } else {
                Weak::new()
            };
        });
    }

    fn activate(self: &Arc<Self>) {
        self.enabled.store(true, Ordering::SeqCst);
        self.bind();
    }

    fn frontier(
        &self,
        depths: [usize; 9],
    ) -> Result<Vec<ExecutionAccountCaptureFrontier>, ExecutionAccountCaptureFailure> {
        if self.overflowed.load(Ordering::SeqCst) {
            return Err(ExecutionAccountCaptureFailure::CounterOverflow);
        }
        if self.active.load(Ordering::SeqCst) != 0 {
            return Err(ExecutionAccountCaptureFailure::PendingIngress);
        }
        let applied: [u64; 9] =
            std::array::from_fn(|index| self.applied[index].load(Ordering::SeqCst));
        CHANNELS
            .iter()
            .enumerate()
            .map(|(index, channel)| {
                let admitted = applied[index]
                    .checked_add(
                        u64::try_from(depths[index])
                            .map_err(|_| ExecutionAccountCaptureFailure::CounterOverflow)?,
                    )
                    .ok_or(ExecutionAccountCaptureFailure::CounterOverflow)?;
                Ok(ExecutionAccountCaptureFrontier {
                    channel: (*channel).to_owned(),
                    applied: applied[index],
                    admitted,
                })
            })
            .collect()
    }

    fn copy_at_frontier<T>(
        &self,
        depths: &mut impl FnMut() -> [usize; 9],
        copy: impl FnOnce() -> Result<T, ExecutionAccountCaptureFailure>,
    ) -> Result<(T, Vec<ExecutionAccountCaptureFrontier>), ExecutionAccountCaptureFailure> {
        let before = self.frontier(depths())?;
        if before.iter().any(|entry| entry.admitted != entry.applied) {
            return Err(ExecutionAccountCaptureFailure::PendingIngress);
        }
        let value = copy()?;
        // No receiver or owner mutation can progress while this synchronous copy
        // is running, any message admitted before the cut remains visible here.
        let after = self.frontier(depths())?;
        if after != before {
            return Err(ExecutionAccountCaptureFailure::PendingIngress);
        }
        Ok((value, after))
    }
}

#[derive(Debug)]
pub(crate) struct CaptureDispatchGuard {
    progress: Option<Arc<CaptureProgress>>,
    channel: usize,
}

impl Drop for CaptureDispatchGuard {
    fn drop(&mut self) {
        if let Some(progress) = &self.progress
            && (progress
                .active
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |value| {
                    value.checked_sub(1)
                })
                .is_err()
                || progress.applied[self.channel]
                    .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |value| {
                        value.checked_add(1)
                    })
                    .is_err())
        {
            progress.overflowed.store(true, Ordering::SeqCst);
        }
    }
}

pub(crate) fn dispatch_guard(channel: usize) -> CaptureDispatchGuard {
    let progress = CURRENT_PROGRESS.with(|current| current.borrow().upgrade());
    if let Some(progress) = &progress
        && progress
            .active
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |value| {
                value.checked_add(1)
            })
            .is_err()
    {
        progress.overflowed.store(true, Ordering::SeqCst);
    }
    CaptureDispatchGuard { progress, channel }
}

#[derive(Debug)]
pub(crate) struct CaptureService {
    pub(crate) requests: mpsc::Sender<CaptureRequest>,
    pub(crate) receiver: Option<mpsc::Receiver<CaptureRequest>>,
    sequence: u64,
}

impl CaptureService {
    pub(crate) fn new() -> Self {
        let (requests, receiver) = mpsc::channel(CAPTURE_QUEUE_CAPACITY);
        Self {
            requests,
            receiver: Some(receiver),
            sequence: 0,
        }
    }
}

impl LiveNode {
    /// Captures a read-only transport handle from the actual sole execution client.
    ///
    /// The handle must be obtained before a hosted `run_async` takes the node,
    /// only request and response values cross threads, native state stays owned.
    ///
    /// # Errors
    ///
    /// Returns an error for an unknown client, multiple clients, a running node
    /// or an execution client without a concrete owner probe.
    pub fn account_capture_handle(
        &self,
        client_id: ClientId,
    ) -> anyhow::Result<ExecutionAccountCaptureHandle> {
        if self.runner.is_none()
            || self.handle.state() != NodeState::Idle
            || self.handle.should_stop()
        {
            anyhow::bail!("Account capture handle must be obtained before the node run");
        }
        let engine = self
            .kernel
            .exec_engine
            .try_borrow()
            .map_err(|_| anyhow::anyhow!("Execution engine is busy"))?;
        if engine.client_ids() != [client_id] {
            anyhow::bail!("Account capture requires the actual sole execution client");
        }
        let probe = engine
            .get_client(&client_id)
            .and_then(ExecutionClient::account_capture_probe)
            .ok_or_else(|| anyhow::anyhow!("Execution client has no account capture owner"))?;
        self.capture_progress.activate();
        Ok(ExecutionAccountCaptureHandle {
            source: probe.source().clone(),
            requests: self.capture_service.requests.clone(),
            node: self.handle.clone(),
        })
    }

    pub(super) fn process_account_capture(
        &mut self,
        request: CaptureRequest,
        mut depths: impl FnMut() -> [usize; 9],
        maintenance_pending: bool,
    ) {
        if request.reply.is_closed() {
            return;
        }
        let result = self.capture_account_at_frontier(&request, &mut depths, maintenance_pending);
        if !request.reply.is_closed() {
            let _ = request.reply.send(result);
        }
    }

    fn capture_account_at_frontier(
        &mut self,
        request: &CaptureRequest,
        depths: &mut impl FnMut() -> [usize; 9],
        maintenance_pending: bool,
    ) -> Result<ExecutionAccountCapture, ExecutionAccountCaptureFailure> {
        use ExecutionAccountCaptureFailure::{
            Borrowed, CounterOverflow, ForeignRuntime, Retired, TimedOut, UnsupportedRuntime,
        };
        if self.handle.should_stop() || !self.handle.is_running() {
            return Err(Retired);
        }
        if dst::time::Instant::now() >= request.deadline {
            return Err(TimedOut);
        }
        if self.environment() != Environment::Sandbox
            || self.config.exec_engine.reconciliation
            || maintenance_pending
            || self.external_msgbus.is_some()
            || !self.stream_processors.is_empty()
        {
            return Err(UnsupportedRuntime);
        }
        let current = CURRENT_PROGRESS.with(|current| current.borrow().upgrade());
        if current
            .as_ref()
            .is_none_or(|current| !Arc::ptr_eq(current, &self.capture_progress))
        {
            return Err(ForeignRuntime);
        }
        let engine = self.kernel.exec_engine.try_borrow().map_err(|_| Borrowed)?;
        if engine.client_ids() != [request.source.client_id] {
            return Err(Retired);
        }
        let probe = engine
            .get_client(&request.source.client_id)
            .and_then(ExecutionClient::account_capture_probe)
            .ok_or(Retired)?;
        if probe.source() != &request.source {
            return Err(Retired);
        }
        let (snapshot, frontier) = self
            .capture_progress
            .copy_at_frontier(depths, || probe.capture().map_err(Into::into))?;
        drop(engine);
        if self.handle.should_stop() || !self.handle.is_running() {
            return Err(Retired);
        }
        if dst::time::Instant::now() >= request.deadline {
            return Err(TimedOut);
        }
        self.capture_service.sequence = self
            .capture_service
            .sequence
            .checked_add(1)
            .ok_or(CounterOverflow)?;
        Ok(ExecutionAccountCapture {
            node_instance_id: self.instance_id(),
            environment: self.environment().to_string(),
            capture_sequence: self.capture_service.sequence,
            frontier,
            snapshot,
        })
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[rstest]
    fn test_account_capture_frontier_includes_secondary_work_and_rejects_overflow() {
        let progress = Arc::new(CaptureProgress::default());
        progress.activate();
        {
            let _outer = dispatch_guard(DATA_EVENTS);
            let _secondary = dispatch_guard(EXEC_COMMANDS);
            assert_eq!(
                progress.frontier([0; 9]).unwrap_err(),
                ExecutionAccountCaptureFailure::PendingIngress
            );
        }
        let mut depths = [0; 9];
        depths[EXEC_EVENTS] = 1;
        let frontier = progress.frontier(depths).unwrap();
        assert_eq!(frontier[DATA_EVENTS].applied, 1);
        assert_eq!(frontier[EXEC_COMMANDS].applied, 1);
        assert_eq!(frontier[EXEC_EVENTS].admitted, 1);
        progress.applied[DATA_EVENTS].store(u64::MAX, Ordering::SeqCst);
        depths[DATA_EVENTS] = 1;
        assert_eq!(
            progress.frontier(depths).unwrap_err(),
            ExecutionAccountCaptureFailure::CounterOverflow
        );
        drop(dispatch_guard(DATA_EVENTS));
        assert_eq!(
            progress.frontier([0; 9]).unwrap_err(),
            ExecutionAccountCaptureFailure::CounterOverflow
        );
    }

    #[rstest]
    fn test_account_capture_never_copies_pending_ingress() {
        let progress = CaptureProgress::default();
        let (sender, receiver) = mpsc::unbounded_channel();
        std::thread::spawn(move || sender.send(1).unwrap())
            .join()
            .unwrap();
        let copied = std::cell::Cell::new(false);
        let result = progress.copy_at_frontier(
            &mut || {
                let mut depths = [0; 9];
                depths[EXEC_COMMANDS] = receiver.len();
                depths
            },
            || {
                copied.set(true);
                Ok(7)
            },
        );
        assert_eq!(
            result.unwrap_err(),
            ExecutionAccountCaptureFailure::PendingIngress
        );
        assert!(!copied.get());
    }

    #[rstest]
    fn test_account_capture_discards_work_admitted_during_the_copy() {
        let progress = CaptureProgress::default();
        let (sender, receiver) = mpsc::unbounded_channel();
        let (start, go) = std::sync::mpsc::sync_channel(0);
        let (completed, done) = std::sync::mpsc::sync_channel(0);
        let caller = std::thread::spawn(move || {
            go.recv_timeout(Duration::from_secs(5)).unwrap();
            sender.send(1).unwrap();
            completed.send(()).unwrap();
        });
        let result = progress.copy_at_frontier(
            &mut || {
                let mut depths = [0; 9];
                depths[EXEC_EVENTS] = receiver.len();
                depths
            },
            || {
                start.send(()).unwrap();
                done.recv_timeout(Duration::from_secs(5)).unwrap();
                Ok(7)
            },
        );
        caller.join().unwrap();
        assert_eq!(
            result.unwrap_err(),
            ExecutionAccountCaptureFailure::PendingIngress
        );
    }

    #[rstest]
    fn test_account_capture_frontier_is_a_past_cut_not_future_stability() {
        let progress = CaptureProgress::default();
        let (sender, receiver) = mpsc::unbounded_channel();
        let (release, go) = std::sync::mpsc::sync_channel(0);
        let caller = std::thread::spawn(move || {
            go.recv_timeout(Duration::from_secs(5)).unwrap();
            sender.send(1).unwrap();
        });
        let mut depths = || {
            let mut values = [0; 9];
            values[DATA_EVENTS] = receiver.len();
            values
        };
        let (value, frontier) = progress.copy_at_frontier(&mut depths, || Ok(7)).unwrap();
        release.send(()).unwrap();
        caller.join().unwrap();
        assert_eq!(value, 7);
        assert_eq!(frontier[DATA_EVENTS].admitted, 0);
        assert_eq!(
            progress
                .copy_at_frontier(&mut depths, || Ok(8))
                .unwrap_err(),
            ExecutionAccountCaptureFailure::PendingIngress
        );
    }
}
