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

//! Async event loop runner for live and sandbox trading nodes.
//!
//! `AsyncRunner` owns seven tokio mpsc channel pairs plus a shutdown
//! signal channel. Construction creates the channels without side
//! effects. The sender halves are placed into thread-local storage
//! via [`AsyncRunner::bind_senders`] so that adapters and engine
//! components can resolve them through the `get_*_sender()` accessors
//! in `nautilus_common::runner` and `nautilus_common::live::runner`.
//!
//! Channel pairs:
//!
//! - **Time events**: timer callbacks dispatched by the clock.
//! - **System events**: system notifications handled by the live node.
//! - **System commands**: control requests handled by the live node.
//! - **Execution events**: fills, order updates, and account state from
//!   execution clients to the execution engine.
//! - **Trading commands**: deferred order actions routed to their direct endpoint.
//! - **Data events**: market data from adapters to the data engine.
//! - **Data commands**: subscribe/unsubscribe requests to data clients.
//!
//! Both `AsyncRunner::run` and `LiveNode::run` use a `biased;` select with
//! system and execution branches polled ahead of data branches. Within each
//! channel pair, events are polled before commands.
//!
//! The runner can drive the event loop in two ways:
//!
//! - **Standalone**: call [`AsyncRunner::run`], which binds senders and
//!   enters a `tokio::select!` loop internally.
//! - **Integrated**: call [`AsyncRunner::take_channels`] to extract the
//!   receivers and run the `select!` loop directly inside `LiveNode::run`,
//!   where it is interleaved with startup, reconciliation, and shutdown
//!   phases.
//!
//! # Invariants
//!
//! - `bind_senders` must be called before any code that reads from TLS.
//!   This includes adapter constructors, clock initialization, and
//!   execution client start methods. Every path from construction to
//!   the event loop must bind before the first TLS read.
//! - The event loop and all TLS consumers must execute on the same
//!   thread. Senders are cloneable and `Send`, but the `RefCell`-backed
//!   TLS slots are not accessible from other threads.
//! - Only one runner at a time should own the TLS slots on a given
//!   thread. `bind_senders` overwrites any existing TLS contents on the
//!   thread, so the last caller wins.

use std::{fmt::Debug, sync::Arc};

use nautilus_common::{
    live::runner::{
        replace_data_event_sender, replace_exec_event_sender, replace_system_command_sender,
        replace_system_event_sender,
    },
    messages::{
        DataEvent, ExecutionEvent, ExecutionReport, SystemCommand, SystemEvent, data::DataCommand,
        execution::TradingCommand,
    },
    msgbus::{self, MessagingSwitchboard},
    runner::{
        DataCommandSender, TimeEventMessage, TimeEventSender, TradingCommandMessage,
        TradingCommandSender, replace_data_cmd_sender, replace_exec_cmd_sender,
        replace_time_event_sender,
    },
};
use nautilus_model::events::OrderEventAny;

/// Asynchronous implementation of `DataCommandSender` for live environments.
#[derive(Debug)]
pub struct AsyncDataCommandSender {
    cmd_tx: tokio::sync::mpsc::UnboundedSender<DataCommand>,
}

impl AsyncDataCommandSender {
    #[must_use]
    pub const fn new(cmd_tx: tokio::sync::mpsc::UnboundedSender<DataCommand>) -> Self {
        Self { cmd_tx }
    }
}

impl DataCommandSender for AsyncDataCommandSender {
    fn execute(&self, command: DataCommand) {
        if let Err(e) = self.cmd_tx.send(command) {
            log::error!("Failed to send data command: {e}");
        }
    }
}

/// Asynchronous implementation of `TimeEventSender` for live environments.
#[derive(Debug, Clone)]
pub struct AsyncTimeEventSender {
    time_tx: tokio::sync::mpsc::UnboundedSender<TimeEventMessage>,
}

impl AsyncTimeEventSender {
    #[must_use]
    pub const fn new(time_tx: tokio::sync::mpsc::UnboundedSender<TimeEventMessage>) -> Self {
        Self { time_tx }
    }
}

impl TimeEventSender for AsyncTimeEventSender {
    fn send(&self, message: TimeEventMessage) {
        if let Err(e) = self.time_tx.send(message) {
            log::error!("Failed to send time event message: {e}");
        }
    }
}

/// Asynchronous implementation of `TradingCommandSender` for live environments.
#[derive(Debug)]
pub struct AsyncTradingCommandSender {
    cmd_tx: tokio::sync::mpsc::UnboundedSender<TradingCommandMessage>,
}

impl AsyncTradingCommandSender {
    #[must_use]
    pub const fn new(cmd_tx: tokio::sync::mpsc::UnboundedSender<TradingCommandMessage>) -> Self {
        Self { cmd_tx }
    }
}

impl TradingCommandSender for AsyncTradingCommandSender {
    fn execute(&self, message: TradingCommandMessage) {
        if let Err(e) = self.cmd_tx.send(message) {
            log::error!("Failed to send trading command: {e}");
        }
    }
}

pub trait Runner {
    fn run(&mut self);
}

/// Channel receivers for the async event loop.
///
/// These can be extracted from `AsyncRunner` via `take_channels()` to drive
/// the event loop directly on the same thread as the msgbus endpoints.
#[derive(Debug)]
pub struct AsyncRunnerChannels {
    pub time_evt_rx: tokio::sync::mpsc::UnboundedReceiver<TimeEventMessage>,
    pub system_evt_rx: tokio::sync::mpsc::UnboundedReceiver<SystemEvent>,
    pub system_cmd_rx: tokio::sync::mpsc::UnboundedReceiver<SystemCommand>,
    pub exec_evt_rx: tokio::sync::mpsc::UnboundedReceiver<ExecutionEvent>,
    pub exec_cmd_rx: tokio::sync::mpsc::UnboundedReceiver<TradingCommandMessage>,
    pub data_evt_rx: tokio::sync::mpsc::UnboundedReceiver<DataEvent>,
    pub data_cmd_rx: tokio::sync::mpsc::UnboundedReceiver<DataCommand>,
}

#[cfg(feature = "node")]
#[allow(
    clippy::large_enum_variant,
    reason = "runner events are consumed immediately; boxing would add routing allocations"
)]
pub(crate) enum PendingRunnerEvent {
    TimeEvent(TimeEventMessage),
    SystemEvent(SystemEvent),
    SystemCommand(SystemCommand),
    ExecEvent(ExecutionEvent),
    ExecCommand(TradingCommandMessage),
    DataEvent(DataEvent),
    DataCommand(DataCommand),
}

/// Drives channels on one concrete native owner thread.
///
/// This runner and its `run` future are not `Send`: construction, sender binding,
/// and execution remain on the same thread as the owner-local retained work.
/// Poll it directly or concurrently with other local futures, rather than using `tokio::spawn`.
/// The channel senders and [`AsyncRunnerHandle`] remain transferable to other threads.
pub struct AsyncRunner {
    owner_delivery: nautilus_common::owner_delivery::OwnerDelivery,
    startup_failure: Option<nautilus_common::owner_delivery::OwnerDeliveryError>,
    #[cfg(feature = "node")]
    pub(crate) capture_progress: std::sync::Arc<crate::node::account_capture::CaptureProgress>,
    channels: AsyncRunnerChannels,
    time_evt_tx: tokio::sync::mpsc::UnboundedSender<TimeEventMessage>,
    system_evt_tx: tokio::sync::mpsc::UnboundedSender<SystemEvent>,
    system_cmd_tx: tokio::sync::mpsc::UnboundedSender<SystemCommand>,
    signal_rx: tokio::sync::mpsc::UnboundedReceiver<()>,
    signal_tx: tokio::sync::mpsc::UnboundedSender<()>,
    exec_evt_tx: tokio::sync::mpsc::UnboundedSender<ExecutionEvent>,
    exec_cmd_tx: tokio::sync::mpsc::UnboundedSender<TradingCommandMessage>,
    data_evt_tx: tokio::sync::mpsc::UnboundedSender<DataEvent>,
    data_cmd_tx: tokio::sync::mpsc::UnboundedSender<DataCommand>,
}

/// Handle for stopping the `AsyncRunner` from another context.
#[derive(Clone, Debug)]
pub struct AsyncRunnerHandle {
    signal_tx: tokio::sync::mpsc::UnboundedSender<()>,
}

impl AsyncRunnerHandle {
    /// Signals the runner to stop.
    pub fn stop(&self) {
        if let Err(e) = self.signal_tx.send(()) {
            log::error!("Failed to send shutdown signal: {e}");
        }
    }
}

impl Default for AsyncRunner {
    fn default() -> Self {
        Self::new()
    }
}

impl Debug for AsyncRunner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct(stringify!(AsyncRunner)).finish()
    }
}

impl AsyncRunner {
    /// Creates a new [`AsyncRunner`] instance.
    ///
    /// Creates channels and the local owner, but does not bind thread-local senders.
    /// Construction and subsequent binding or execution stay on this same thread.
    /// Call [`bind_senders`](Self::bind_senders) before creating clients that
    /// read from TLS, and again before entering the event loop.
    #[must_use]
    pub fn new() -> Self {
        use tokio::sync::mpsc::unbounded_channel; // tokio-import-ok

        let (time_evt_tx, time_evt_rx) = unbounded_channel::<TimeEventMessage>();
        let (system_evt_tx, system_evt_rx) = unbounded_channel::<SystemEvent>();
        let (system_cmd_tx, system_cmd_rx) = unbounded_channel::<SystemCommand>();
        let (signal_tx, signal_rx) = unbounded_channel::<()>();
        let (exec_evt_tx, exec_evt_rx) = unbounded_channel::<ExecutionEvent>();
        let (exec_cmd_tx, exec_cmd_rx) = unbounded_channel::<TradingCommandMessage>();
        let (data_evt_tx, data_evt_rx) = unbounded_channel::<DataEvent>();
        let (data_cmd_tx, data_cmd_rx) = unbounded_channel::<DataCommand>();

        Self {
            owner_delivery: nautilus_common::owner_delivery::OwnerDelivery::new(),
            startup_failure: None,
            #[cfg(feature = "node")]
            capture_progress: Arc::default(),
            channels: AsyncRunnerChannels {
                time_evt_rx,
                system_evt_rx,
                system_cmd_rx,
                exec_evt_rx,
                exec_cmd_rx,
                data_evt_rx,
                data_cmd_rx,
            },
            time_evt_tx,
            system_evt_tx,
            system_cmd_tx,
            signal_rx,
            signal_tx,
            exec_evt_tx,
            exec_cmd_tx,
            data_evt_tx,
            data_cmd_tx,
        }
    }

    /// Binds this runner's channel senders to thread-local storage.
    ///
    /// Call before creating clients that read from TLS (e.g., in the builder),
    /// and again before entering the event loop to reclaim ownership if another
    /// runner was constructed on this thread in the interim.
    pub fn bind_senders(&self) {
        #[cfg(feature = "node")]
        self.capture_progress.bind();
        replace_time_event_sender(Arc::new(AsyncTimeEventSender::new(
            self.time_evt_tx.clone(),
        )));
        replace_system_event_sender(self.system_evt_tx.clone());
        replace_system_command_sender(self.system_cmd_tx.clone());
        replace_exec_event_sender(self.exec_evt_tx.clone());
        replace_exec_cmd_sender(Arc::new(AsyncTradingCommandSender::new(
            self.exec_cmd_tx.clone(),
        )));
        replace_data_event_sender(self.data_evt_tx.clone());
        replace_data_cmd_sender(Arc::new(AsyncDataCommandSender::new(
            self.data_cmd_tx.clone(),
        )));
    }

    /// Stops the runner with an internal shutdown signal.
    pub fn stop(&self) {
        if let Err(e) = self.signal_tx.send(()) {
            log::error!("Failed to send shutdown signal: {e}");
        }
    }

    /// Returns a handle that can be used to stop the runner from another context.
    #[must_use]
    pub fn handle(&self) -> AsyncRunnerHandle {
        AsyncRunnerHandle {
            signal_tx: self.signal_tx.clone(),
        }
    }

    /// Consumes the runner and returns the channel receivers for direct event loop driving.
    ///
    /// This is used when the event loop needs to run on the same thread as the msgbus
    /// endpoints (which use thread-local storage).
    #[must_use]
    pub fn take_channels(self) -> AsyncRunnerChannels {
        self.channels
    }

    /// Flushes all pending data events and commands from the channels.
    ///
    /// Loops until both data channels are empty, processing each item
    /// into the cache immediately. Used in `start()` where channels are
    /// not extracted.
    pub fn flush_pending_data(&mut self) {
        let mut total = 0;

        loop {
            let mut progressed = false;

            // Events drain before commands here even though the runtime select
            // prefers the opposite for everything-else: `LiveNode::start()`
            // calls this after `connect_data_clients()` to push queued
            // `DataEvent::Instrument` items into the cache. A pending
            // subscription command (e.g. `SubscribeBars`) processed before the
            // matching instrument lands would be rejected by the data engine.
            while let Ok(evt) = self.channels.data_evt_rx.try_recv() {
                Self::handle_data_event(evt);
                progressed = true;
                total += 1;
            }

            while let Ok(cmd) = self.channels.data_cmd_rx.try_recv() {
                Self::handle_data_command(cmd);
                progressed = true;
                total += 1;
            }

            if !progressed {
                break;
            }
        }

        if total > 0 {
            log::debug!("Flushed {total} pending data events/commands");
        }
    }

    #[cfg(feature = "node")]
    pub(crate) fn drain_pending_system_events(&mut self) -> Vec<SystemEvent> {
        let mut events = Vec::new();

        while let Ok(event) = self.channels.system_evt_rx.try_recv() {
            events.push(event);
        }

        events
    }

    #[cfg(feature = "node")]
    pub(crate) fn drain_pending_system_commands(&mut self) -> Vec<SystemCommand> {
        let mut commands = Vec::new();

        while let Ok(command) = self.channels.system_cmd_rx.try_recv() {
            commands.push(command);
        }

        commands
    }

    /// Runs the async runner event loop.
    ///
    /// This method processes time, system, execution, and data events in an async loop.
    /// It will run until a signal is received or the event streams are closed.
    /// The future remains local to the construction thread when polled alongside other futures.
    pub async fn run(&mut self) {
        if let Some(error) = self.owner_delivery_failure() {
            log::error!("Cannot restart a failed native owner: {error}");
            return;
        }
        let owner_delivery = self.owner_delivery.clone();
        let _owner_binding = match owner_delivery.bind() {
            Ok(owner) => owner,
            Err(error) => {
                self.startup_failure.get_or_insert(error);
                log::error!("Cannot bind native owner: {error}");
                return;
            }
        };
        self.bind_senders();

        log::info!("AsyncRunner starting");

        loop {
            tokio::select! {
                biased;

                Some(()) = self.signal_rx.recv() => {
                    log::info!("AsyncRunner received signal, shutting down");
                    return;
                },
                Some(handler) = self.channels.time_evt_rx.recv() => {
                    let _ = Self::handle_time_event(handler);
                },
                Some(event) = self.channels.system_evt_rx.recv() => {
                    log::error!("System event {event} requires the LiveNode runner");
                },
                Some(command) = self.channels.system_cmd_rx.recv() => {
                    log::error!("System command {command} requires the LiveNode runner");
                },
                Some(evt) = self.channels.exec_evt_rx.recv() => {
                    Self::handle_exec_event(evt);
                },
                Some(cmd) = self.channels.exec_cmd_rx.recv() => {
                    Self::handle_trading_command(cmd);
                },
                Some(evt) = self.channels.data_evt_rx.recv() => {
                    Self::handle_data_event(evt);
                },
                Some(cmd) = self.channels.data_cmd_rx.recv() => {
                    Self::handle_data_command(cmd);
                },
                else => {
                    log::debug!("AsyncRunner all channels closed, exiting");
                    return;
                }
            };
            if let Err(error) = owner_delivery.drain() {
                log::error!("Native owner failed with retained work: {error}");
                return;
            }
        }
    }

    /// Returns the retained native owner failure, including after `run` returns.
    #[must_use]
    pub fn owner_delivery_failure(
        &self,
    ) -> Option<nautilus_common::owner_delivery::OwnerDeliveryError> {
        self.owner_delivery.failure().or(self.startup_failure)
    }

    /// Handles a time event by running its callback.
    #[inline]
    #[must_use]
    pub fn handle_time_event(message: TimeEventMessage) -> bool {
        #[cfg(feature = "node")]
        let _capture_dispatch =
            crate::node::account_capture::dispatch_guard(crate::node::account_capture::TIME_EVENTS);
        message.dispatch()
    }

    /// Handles a data command by sending to the `DataEngine`.
    #[inline]
    pub fn handle_data_command(cmd: DataCommand) {
        #[cfg(feature = "node")]
        let _capture_dispatch = crate::node::account_capture::dispatch_guard(
            crate::node::account_capture::DATA_COMMANDS,
        );
        msgbus::send_data_command(MessagingSwitchboard::data_engine_execute(), cmd);
    }

    /// Handles a data event by sending to the appropriate `DataEngine` endpoint.
    #[inline]
    pub fn handle_data_event(event: DataEvent) {
        #[cfg(feature = "node")]
        let _capture_dispatch =
            crate::node::account_capture::dispatch_guard(crate::node::account_capture::DATA_EVENTS);
        match event {
            DataEvent::Data(data) => {
                msgbus::send_data(MessagingSwitchboard::data_engine_process_data(), data);
            }
            DataEvent::Instrument(data) => {
                msgbus::send_any(MessagingSwitchboard::data_engine_process(), &data);
            }
            DataEvent::Response(resp) => {
                msgbus::send_data_response(MessagingSwitchboard::data_engine_response(), resp);
            }
            DataEvent::FundingRate(funding_rate) => {
                msgbus::send_any(MessagingSwitchboard::data_engine_process(), &funding_rate);
            }
            DataEvent::InstrumentStatus(status) => {
                msgbus::send_any(MessagingSwitchboard::data_engine_process(), &status);
            }
            DataEvent::OptionGreeks(greeks) => {
                msgbus::send_any(MessagingSwitchboard::data_engine_process(), &greeks);
            }
            #[cfg(feature = "defi")]
            DataEvent::DeFi(data) => {
                msgbus::send_defi_data(MessagingSwitchboard::data_engine_process_defi_data(), data);
            }
        }
        drain_owner_delivery();
    }

    /// Dispatches an internal execution command directly to the execution engine.
    #[inline]
    pub fn handle_exec_command(cmd: TradingCommand) {
        #[cfg(feature = "node")]
        let _capture_dispatch = crate::node::account_capture::dispatch_guard(
            crate::node::account_capture::EXEC_COMMANDS,
        );
        let message = TradingCommandMessage::new(MessagingSwitchboard::exec_engine_execute(), cmd);
        if let Some(message) = nautilus_common::owner_delivery::retain_undispatched_command(message)
        {
            msgbus::send_trading_command(message.endpoint(), message.command().clone());
        }
    }

    /// Dispatches a deferred trading command to its direct endpoint.
    #[inline]
    pub fn handle_trading_command(message: TradingCommandMessage) {
        #[cfg(feature = "node")]
        let _capture_dispatch = crate::node::account_capture::dispatch_guard(
            crate::node::account_capture::EXEC_COMMANDS,
        );
        let mut messages = vec![message];
        while let Some(message) = messages.pop() {
            if let Some(message) =
                nautilus_common::owner_delivery::retain_undispatched_command(message)
            {
                messages.extend(message.dispatch().into_iter().rev());
            }
        }
        drain_owner_delivery();
    }

    /// Handles an execution event by sending to the appropriate engine endpoint.
    #[inline]
    pub fn handle_exec_event(event: ExecutionEvent) {
        #[cfg(feature = "node")]
        let _capture_dispatch =
            crate::node::account_capture::dispatch_guard(crate::node::account_capture::EXEC_EVENTS);
        match event {
            ExecutionEvent::OrderInitializedPublication(initialized) => {
                Self::handle_incoming_orders(vec![
                    nautilus_common::owner_delivery::OwnerIncomingOrder::PublishInitialized(
                        initialized,
                    ),
                ]);
            }
            ExecutionEvent::Order(order_event) => {
                Self::handle_incoming_orders(vec![
                    nautilus_common::owner_delivery::OwnerIncomingOrder::Process(order_event),
                ]);
            }
            ExecutionEvent::OrderSubmittedBatch(batch) => {
                Self::handle_incoming_orders(
                    batch
                        .into_iter()
                        .map(|submitted| {
                            nautilus_common::owner_delivery::OwnerIncomingOrder::Process(
                                OrderEventAny::Submitted(submitted),
                            )
                        })
                        .collect(),
                );
            }
            ExecutionEvent::OrderAcceptedBatch(batch) => {
                Self::handle_incoming_orders(
                    batch
                        .into_iter()
                        .map(|accepted| {
                            nautilus_common::owner_delivery::OwnerIncomingOrder::Process(
                                OrderEventAny::Accepted(accepted),
                            )
                        })
                        .collect(),
                );
            }
            ExecutionEvent::OrderCanceledBatch(batch) => {
                Self::handle_incoming_orders(
                    batch
                        .into_iter()
                        .map(|canceled| {
                            nautilus_common::owner_delivery::OwnerIncomingOrder::Process(
                                OrderEventAny::Canceled(canceled),
                            )
                        })
                        .collect(),
                );
            }
            ExecutionEvent::Report(report) => {
                Self::handle_exec_report(report);
            }
            ExecutionEvent::Account(ref account) => {
                msgbus::send_account_state(
                    MessagingSwitchboard::portfolio_update_account(),
                    account,
                );
            }
        }
        drain_owner_delivery();
    }

    pub(crate) fn handle_incoming_orders(
        events: Vec<nautilus_common::owner_delivery::OwnerIncomingOrder>,
    ) {
        if let Err(error) = nautilus_common::owner_delivery::dispatch_incoming_orders(events) {
            nautilus_common::owner_delivery::record_failure(error);
        }
    }

    #[inline]
    pub fn handle_exec_report(report: ExecutionReport) {
        #[cfg(feature = "node")]
        let _capture_dispatch =
            crate::node::account_capture::dispatch_guard(crate::node::account_capture::EXEC_EVENTS);
        let endpoint = MessagingSwitchboard::exec_engine_reconcile_execution_report();
        msgbus::send_execution_report(endpoint, report);
    }
}

fn drain_owner_delivery() {
    if let Err(error) = nautilus_common::owner_delivery::drain_current() {
        nautilus_common::owner_delivery::record_failure(error);
    }
}

#[cfg(feature = "node")]
impl AsyncRunner {
    pub(crate) fn poll_pending(&mut self, mut process: impl FnMut(PendingRunnerEvent)) -> usize {
        self.bind_senders();

        let pending = (
            self.channels.time_evt_rx.len(),
            self.channels.system_evt_rx.len(),
            self.channels.system_cmd_rx.len(),
            self.channels.exec_evt_rx.len(),
            self.channels.exec_cmd_rx.len(),
            self.channels.data_evt_rx.len(),
            self.channels.data_cmd_rx.len(),
        );
        let mut processed = 0;
        processed += poll_channel(
            &mut self.channels.time_evt_rx,
            pending.0,
            PendingRunnerEvent::TimeEvent,
            &mut process,
        );
        processed += poll_channel(
            &mut self.channels.system_evt_rx,
            pending.1,
            PendingRunnerEvent::SystemEvent,
            &mut process,
        );
        processed += poll_channel(
            &mut self.channels.system_cmd_rx,
            pending.2,
            PendingRunnerEvent::SystemCommand,
            &mut process,
        );
        processed += poll_channel(
            &mut self.channels.exec_evt_rx,
            pending.3,
            PendingRunnerEvent::ExecEvent,
            &mut process,
        );
        processed += poll_channel(
            &mut self.channels.exec_cmd_rx,
            pending.4,
            PendingRunnerEvent::ExecCommand,
            &mut process,
        );
        processed += poll_channel(
            &mut self.channels.data_evt_rx,
            pending.5,
            PendingRunnerEvent::DataEvent,
            &mut process,
        );
        processed += poll_channel(
            &mut self.channels.data_cmd_rx,
            pending.6,
            PendingRunnerEvent::DataCommand,
            &mut process,
        );
        processed
    }

    pub(crate) async fn recv(&mut self) -> Option<PendingRunnerEvent> {
        tokio::select! {
            biased;

            Some(message) = self.channels.time_evt_rx.recv() => {
                Some(PendingRunnerEvent::TimeEvent(message))
            }
            Some(event) = self.channels.system_evt_rx.recv() => {
                Some(PendingRunnerEvent::SystemEvent(event))
            }
            Some(command) = self.channels.system_cmd_rx.recv() => {
                Some(PendingRunnerEvent::SystemCommand(command))
            }
            Some(event) = self.channels.exec_evt_rx.recv() => {
                Some(PendingRunnerEvent::ExecEvent(event))
            }
            Some(command) = self.channels.exec_cmd_rx.recv() => {
                Some(PendingRunnerEvent::ExecCommand(command))
            }
            Some(event) = self.channels.data_evt_rx.recv() => {
                Some(PendingRunnerEvent::DataEvent(event))
            }
            Some(command) = self.channels.data_cmd_rx.recv() => {
                Some(PendingRunnerEvent::DataCommand(command))
            }
            else => None,
        }
    }
}

#[cfg(feature = "node")]
fn poll_channel<T>(
    receiver: &mut tokio::sync::mpsc::UnboundedReceiver<T>,
    pending: usize,
    event: impl Fn(T) -> PendingRunnerEvent,
    process: &mut impl FnMut(PendingRunnerEvent),
) -> usize {
    let mut processed = 0;

    for _ in 0..pending {
        let Ok(message) = receiver.try_recv() else {
            break;
        };

        process(event(message));
        processed += 1;
    }

    processed
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, rc::Rc, sync::Arc, time::Duration};

    use nautilus_common::{
        cache::Cache,
        clock::TestClock,
        live::{
            dst,
            runner::{
                get_data_event_sender, get_exec_event_sender, get_system_command_sender,
                get_system_event_sender, try_get_system_command_sender,
                try_get_system_event_sender,
            },
        },
        messages::{
            ExecutionEvent, ExecutionReport,
            data::{SubscribeCommand, SubscribeCustomData},
            execution::{CancelAllOrders, QueryAccount, TradingCommand},
            system::{ReconnectSocket, SocketState, SocketStateChange},
        },
        msgbus::{TypedIntoHandler, stubs::get_typed_into_message_saving_handler},
        runner::{
            TimeEventMessage, get_data_cmd_sender, get_time_event_sender, get_trading_cmd_sender,
            replace_exec_cmd_sender, try_get_time_event_sender, try_get_trading_cmd_sender,
        },
        timer::{TimeEvent, TimeEventCallback},
    };
    use nautilus_core::{UUID4, UnixNanos};
    use nautilus_execution::engine::ExecutionEngine;
    use nautilus_model::{
        data::{Data, DataType, quote::QuoteTick},
        enums::{
            AccountType, LiquiditySide, OrderSide, OrderStatus, OrderType, PositionSide,
            TimeInForce,
        },
        events::{
            OrderAcceptedBatch, OrderCanceledBatch, OrderEvent, OrderEventAny, OrderSubmittedBatch,
            account::state::AccountState,
            order::spec::{OrderAcceptedSpec, OrderCanceledSpec, OrderSubmittedSpec},
        },
        identifiers::{
            AccountId, ClientId, ClientOrderId, InstrumentId, PositionId, StrategyId, TradeId,
            TraderId, Venue, VenueOrderId,
        },
        reports::{FillReport, OrderStatusReport, PositionStatusReport},
        types::{Money, Price, Quantity},
    };
    use rstest::rstest;
    use ustr::Ustr;

    use super::*;

    async fn finish_runner_after_stop<T>(
        run: impl std::future::Future<Output = T>,
        stopped: &mut tokio::sync::oneshot::Receiver<dst::time::Instant>,
        budget: Duration,
    ) -> Result<T, &'static str> {
        tokio::pin!(run);
        tokio::select! {
            biased;
            stopped_at = stopped => {
                let stopped_at = stopped_at.map_err(|_| "producer stop marker was dropped")?;
                let remaining = budget
                    .checked_sub(stopped_at.elapsed())
                    .filter(|remaining| !remaining.is_zero())
                    .ok_or("original stop deadline elapsed before the runner was polled")?;
                dst::time::timeout(remaining, &mut run)
                    .await
                    .map_err(|_| "original stop deadline elapsed while the runner was pending")
            }
            result = &mut run => Ok(result),
        }
    }

    // Test fixture for creating test quotes
    fn test_quote() -> QuoteTick {
        QuoteTick {
            instrument_id: InstrumentId::from("EUR/USD.SIM"),
            bid_price: Price::from("1.10000"),
            ask_price: Price::from("1.10001"),
            bid_size: Quantity::from(1_000_000),
            ask_size: Quantity::from(1_000_000),
            ts_event: UnixNanos::default(),
            ts_init: UnixNanos::default(),
        }
    }

    fn test_system_event() -> SystemEvent {
        SystemEvent::SocketState(SocketStateChange::new(
            ClientId::from("BINANCE"),
            Some(Venue::from("BINANCE")),
            Ustr::from("binance-futures-market-streams"),
            SocketState::Connected,
        ))
    }

    fn test_system_command() -> SystemCommand {
        SystemCommand::ReconnectSocket(ReconnectSocket::new(
            TraderId::from("TRADER-001"),
            ClientId::from("POLYMARKET"),
            Ustr::from("polymarket-market-streams"),
            UnixNanos::from(3),
        ))
    }

    // Test fixture to create AsyncRunner with manual channels.
    // Sender halves are dummies (not connected to the test receivers) since
    // these tests exercise the event loop, not TLS binding.
    fn create_test_runner(
        time_evt_rx: tokio::sync::mpsc::UnboundedReceiver<TimeEventMessage>,
        data_evt_rx: tokio::sync::mpsc::UnboundedReceiver<DataEvent>,
        data_cmd_rx: tokio::sync::mpsc::UnboundedReceiver<DataCommand>,
        exec_evt_rx: tokio::sync::mpsc::UnboundedReceiver<ExecutionEvent>,
        exec_cmd_rx: tokio::sync::mpsc::UnboundedReceiver<TradingCommandMessage>,
        signal_rx: tokio::sync::mpsc::UnboundedReceiver<()>,
        signal_tx: tokio::sync::mpsc::UnboundedSender<()>,
    ) -> AsyncRunner {
        let (time_evt_tx, _) = tokio::sync::mpsc::unbounded_channel();
        let (system_evt_tx, system_evt_rx) = tokio::sync::mpsc::unbounded_channel();
        let (system_cmd_tx, system_cmd_rx) = tokio::sync::mpsc::unbounded_channel();
        let (data_evt_tx, _) = tokio::sync::mpsc::unbounded_channel();
        let (data_cmd_tx, _) = tokio::sync::mpsc::unbounded_channel();
        let (exec_evt_tx, _) = tokio::sync::mpsc::unbounded_channel();
        let (exec_cmd_tx, _) = tokio::sync::mpsc::unbounded_channel();

        AsyncRunner {
            owner_delivery: nautilus_common::owner_delivery::OwnerDelivery::new(),
            startup_failure: None,
            #[cfg(feature = "node")]
            capture_progress: Arc::default(),
            channels: AsyncRunnerChannels {
                time_evt_rx,
                system_evt_rx,
                system_cmd_rx,
                exec_evt_rx,
                exec_cmd_rx,
                data_evt_rx,
                data_cmd_rx,
            },
            time_evt_tx,
            system_evt_tx,
            system_cmd_tx,
            exec_evt_tx,
            exec_cmd_tx,
            data_evt_tx,
            data_cmd_tx,
            signal_rx,
            signal_tx,
        }
    }

    #[cfg(feature = "node")]
    #[rstest]
    fn test_poll_pending_processes_entry_snapshot_across_channels() {
        let (time_evt_tx, time_evt_rx) = tokio::sync::mpsc::unbounded_channel();
        let (data_evt_tx, data_evt_rx) = tokio::sync::mpsc::unbounded_channel();
        let (data_cmd_tx, data_cmd_rx) = tokio::sync::mpsc::unbounded_channel();
        let (exec_evt_tx, exec_evt_rx) = tokio::sync::mpsc::unbounded_channel();
        let (exec_cmd_tx, exec_cmd_rx) = tokio::sync::mpsc::unbounded_channel();
        let (signal_tx, signal_rx) = tokio::sync::mpsc::unbounded_channel();

        let time_event = TimeEvent::new(
            Ustr::from("test"),
            UUID4::new(),
            UnixNanos::from(1),
            UnixNanos::from(2),
        );
        time_evt_tx
            .send(TimeEventMessage::new(
                time_event,
                TimeEventCallback::from(|_: TimeEvent| {}),
            ))
            .unwrap();
        exec_evt_tx
            .send(ExecutionEvent::Order(OrderEventAny::Submitted(
                OrderSubmittedSpec::builder()
                    .client_order_id(ClientOrderId::from("O-POLL-001"))
                    .build(),
            )))
            .unwrap();
        exec_cmd_tx
            .send(TradingCommandMessage::new(
                MessagingSwitchboard::exec_engine_execute(),
                TradingCommand::CancelAllOrders(CancelAllOrders::new(
                    TraderId::from("TRADER-001"),
                    None,
                    StrategyId::from("S-POLL-001"),
                    InstrumentId::from("EUR/USD.SIM"),
                    Some(OrderSide::Buy),
                    UUID4::new(),
                    UnixNanos::from(3),
                    None,
                    None,
                )),
            ))
            .unwrap();
        data_evt_tx
            .send(DataEvent::Data(Data::Quote(test_quote())))
            .unwrap();
        data_cmd_tx
            .send(DataCommand::Subscribe(SubscribeCommand::Data(
                SubscribeCustomData {
                    client_id: Some(ClientId::from("POLL")),
                    venue: None,
                    data_type: DataType::new("QuoteTick", None, None),
                    command_id: UUID4::new(),
                    ts_init: UnixNanos::from(4),
                    correlation_id: None,
                    params: None,
                },
            )))
            .unwrap();

        let mut runner = create_test_runner(
            time_evt_rx,
            data_evt_rx,
            data_cmd_rx,
            exec_evt_rx,
            exec_cmd_rx,
            signal_rx,
            signal_tx,
        );
        runner.bind_senders();
        get_system_command_sender()
            .send(test_system_command())
            .unwrap();
        get_system_event_sender().send(test_system_event()).unwrap();
        get_system_event_sender().send(test_system_event()).unwrap();
        let mut processed_by_channel = [0; 7];
        let mut processed_order = Vec::new();

        let first = runner.poll_pending(|event| match event {
            PendingRunnerEvent::TimeEvent(_) => {
                processed_by_channel[0] += 1;
                processed_order.push("time");
            }
            PendingRunnerEvent::SystemEvent(_) => {
                processed_by_channel[1] += 1;
                processed_order.push("system_event");
            }
            PendingRunnerEvent::SystemCommand(_) => {
                processed_by_channel[2] += 1;
                processed_order.push("system_command");
            }
            PendingRunnerEvent::ExecEvent(_) => {
                processed_by_channel[3] += 1;
                processed_order.push("exec_event");
            }
            PendingRunnerEvent::ExecCommand(_) => {
                processed_by_channel[4] += 1;
                processed_order.push("exec_command");
            }
            PendingRunnerEvent::DataEvent(_) => {
                processed_by_channel[5] += 1;
                processed_order.push("data_event");
                data_evt_tx
                    .send(DataEvent::Data(Data::Quote(test_quote())))
                    .unwrap();
            }
            PendingRunnerEvent::DataCommand(_) => {
                processed_by_channel[6] += 1;
                processed_order.push("data_command");
            }
        });
        let second = runner.poll_pending(|event| match event {
            PendingRunnerEvent::DataEvent(_) => {
                processed_by_channel[5] += 1;
                processed_order.push("data_event");
            }
            _ => panic!("Unexpected runner event"),
        });

        assert_eq!(first, 8);
        assert_eq!(second, 1);
        assert_eq!(processed_by_channel, [1, 2, 1, 1, 1, 2, 1]);
        assert_eq!(
            processed_order,
            [
                "time",
                "system_event",
                "system_event",
                "system_command",
                "exec_event",
                "exec_command",
                "data_event",
                "data_command",
                "data_event",
            ]
        );
    }

    #[cfg(feature = "node")]
    #[tokio::test]
    async fn test_recv_processes_system_event_before_command() {
        let (_time_evt_tx, time_evt_rx) = tokio::sync::mpsc::unbounded_channel();
        let (_data_evt_tx, data_evt_rx) = tokio::sync::mpsc::unbounded_channel();
        let (_data_cmd_tx, data_cmd_rx) = tokio::sync::mpsc::unbounded_channel();
        let (_exec_evt_tx, exec_evt_rx) = tokio::sync::mpsc::unbounded_channel();
        let (_exec_cmd_tx, exec_cmd_rx) = tokio::sync::mpsc::unbounded_channel();
        let (signal_tx, signal_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut runner = create_test_runner(
            time_evt_rx,
            data_evt_rx,
            data_cmd_rx,
            exec_evt_rx,
            exec_cmd_rx,
            signal_rx,
            signal_tx,
        );

        runner.system_cmd_tx.send(test_system_command()).unwrap();
        runner.system_evt_tx.send(test_system_event()).unwrap();

        assert!(matches!(
            runner.recv().await,
            Some(PendingRunnerEvent::SystemEvent(_))
        ));
        assert!(matches!(
            runner.recv().await,
            Some(PendingRunnerEvent::SystemCommand(_))
        ));
    }

    #[rstest]
    fn test_async_data_command_sender_creation() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let sender = AsyncDataCommandSender::new(tx);
        assert!(format!("{sender:?}").contains("AsyncDataCommandSender"));
    }

    #[rstest]
    fn test_async_time_event_sender_creation() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let sender = AsyncTimeEventSender::new(tx);
        assert!(format!("{sender:?}").contains("AsyncTimeEventSender"));
    }

    #[tokio::test]
    async fn test_async_data_command_sender_execute() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let sender = AsyncDataCommandSender::new(tx);

        let command = DataCommand::Subscribe(SubscribeCommand::Data(SubscribeCustomData {
            client_id: Some(ClientId::from("TEST")),
            venue: None,
            data_type: DataType::new("QuoteTick", None, None),
            command_id: UUID4::new(),
            ts_init: UnixNanos::default(),
            correlation_id: None,
            params: None,
        }));

        sender.execute(command.clone());

        let received = rx.recv().await.unwrap();
        match (received, command) {
            (
                DataCommand::Subscribe(SubscribeCommand::Data(r)),
                DataCommand::Subscribe(SubscribeCommand::Data(c)),
            ) => {
                assert_eq!(r.client_id, c.client_id);
                assert_eq!(r.data_type, c.data_type);
            }
            _ => panic!("Command mismatch"),
        }
    }

    #[tokio::test]
    async fn test_async_time_event_sender_send() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let sender = AsyncTimeEventSender::new(tx);

        let event = TimeEvent::new(
            Ustr::from("test"),
            UUID4::new(),
            UnixNanos::from(1),
            UnixNanos::from(2),
        );
        let callback = TimeEventCallback::from(|_: TimeEvent| {});
        let message = TimeEventMessage::new(event, callback);

        sender.send(message);

        assert!(rx.recv().await.is_some());
    }

    #[tokio::test]
    async fn test_runner_shutdown_signal() {
        // Create runner with manual channels to avoid global state
        let (_data_tx, data_evt_rx) = tokio::sync::mpsc::unbounded_channel::<DataEvent>();
        let (_cmd_tx, data_cmd_rx) = tokio::sync::mpsc::unbounded_channel::<DataCommand>();
        let (_time_tx, time_evt_rx) = tokio::sync::mpsc::unbounded_channel::<TimeEventMessage>();
        let (_exec_evt_tx, exec_evt_rx) = tokio::sync::mpsc::unbounded_channel::<ExecutionEvent>();
        let (_exec_cmd_tx, exec_cmd_rx) =
            tokio::sync::mpsc::unbounded_channel::<TradingCommandMessage>();
        let (signal_tx, signal_rx) = tokio::sync::mpsc::unbounded_channel::<()>();

        let mut runner = create_test_runner(
            time_evt_rx,
            data_evt_rx,
            data_cmd_rx,
            exec_evt_rx,
            exec_cmd_rx,
            signal_rx,
            signal_tx.clone(),
        );

        // Start runner
        let (stopped_tx, mut stopped_rx) = tokio::sync::oneshot::channel();
        let producer = async move {
            // Send shutdown signal
            let stopped_at = dst::time::Instant::now();
            signal_tx.send(()).unwrap();
            stopped_tx.send(stopped_at).unwrap();

            // Runner should stop quickly
        };
        let run = async {
            runner.run().await;
            runner.owner_delivery_failure()
        };
        let observed = finish_runner_after_stop(run, &mut stopped_rx, Duration::from_millis(100));
        let (result, ()) = tokio::join!(observed, producer);

        assert!(result.is_ok(), "Runner should stop on signal");
        assert_eq!(
            result.expect("local runner exceeded its original stop deadline"),
            None,
            "local runner must not hide an owner-delivery failure"
        );
    }

    #[tokio::test]
    async fn test_runner_closes_on_channel_drop() {
        let (data_tx, data_evt_rx) = tokio::sync::mpsc::unbounded_channel::<DataEvent>();
        let (_cmd_tx, data_cmd_rx) = tokio::sync::mpsc::unbounded_channel::<DataCommand>();
        let (_time_tx, time_evt_rx) = tokio::sync::mpsc::unbounded_channel::<TimeEventMessage>();
        let (_exec_evt_tx, exec_evt_rx) = tokio::sync::mpsc::unbounded_channel::<ExecutionEvent>();
        let (_exec_cmd_tx, exec_cmd_rx) =
            tokio::sync::mpsc::unbounded_channel::<TradingCommandMessage>();
        let (signal_tx, signal_rx) = tokio::sync::mpsc::unbounded_channel::<()>();

        let mut runner = create_test_runner(
            time_evt_rx,
            data_evt_rx,
            data_cmd_rx,
            exec_evt_rx,
            exec_cmd_rx,
            signal_rx,
            signal_tx.clone(),
        );

        // Start runner
        let (stopped_tx, mut stopped_rx) = tokio::sync::oneshot::channel();
        let producer = async move {
            drop(data_tx);

            // Yield to let runner enter event loop before stop signal
            tokio::task::yield_now().await;
            let stopped_at = dst::time::Instant::now();
            signal_tx.send(()).ok();
            stopped_tx.send(stopped_at).unwrap();

            // Runner should stop when channels close or on signal
        };
        let run = async {
            runner.run().await;
            runner.owner_delivery_failure()
        };
        let observed = finish_runner_after_stop(run, &mut stopped_rx, Duration::from_millis(200));
        let (result, ()) = tokio::join!(observed, producer);

        assert!(
            result.is_ok(),
            "Runner should stop when channels close or on signal"
        );
        assert_eq!(
            result.expect("local runner exceeded its original stop deadline"),
            None,
            "local runner must not hide an owner-delivery failure"
        );
    }

    #[tokio::test]
    async fn test_concurrent_event_sending() {
        let (data_evt_tx, data_evt_rx) = tokio::sync::mpsc::unbounded_channel::<DataEvent>();
        let (_data_cmd_tx, data_cmd_rx) = tokio::sync::mpsc::unbounded_channel::<DataCommand>();
        let (_time_evt_tx, time_evt_rx) =
            tokio::sync::mpsc::unbounded_channel::<TimeEventMessage>();
        let (_exec_evt_tx, exec_evt_rx) = tokio::sync::mpsc::unbounded_channel::<ExecutionEvent>();
        let (_exec_cmd_tx, exec_cmd_rx) =
            tokio::sync::mpsc::unbounded_channel::<TradingCommandMessage>();
        let (signal_tx, signal_rx) = tokio::sync::mpsc::unbounded_channel::<()>();

        // Setup runner
        let mut runner = create_test_runner(
            time_evt_rx,
            data_evt_rx,
            data_cmd_rx,
            exec_evt_rx,
            exec_cmd_rx,
            signal_rx,
            signal_tx.clone(),
        );

        // Spawn multiple concurrent senders
        let mut handles = vec![];

        for _ in 0..5 {
            let tx_clone = data_evt_tx.clone();

            let handle = tokio::spawn(async move {
                for _ in 0..20 {
                    let quote = test_quote();
                    tx_clone.send(DataEvent::Data(Data::Quote(quote))).unwrap();
                    tokio::task::yield_now().await;
                }
            });
            handles.push(handle);
        }

        // Start runner in background
        let (stopped_tx, mut stopped_rx) = tokio::sync::oneshot::channel();
        let producer = async move {
            // Wait for all senders
            for handle in handles {
                handle.await.unwrap();
            }

            // Yield to let runner enter event loop before stop signal
            tokio::task::yield_now().await;
            let stopped_at = dst::time::Instant::now();
            signal_tx.send(()).unwrap();
            stopped_tx.send(stopped_at).unwrap();
        };
        let run = async {
            runner.run().await;
            runner.owner_delivery_failure()
        };
        let observed = finish_runner_after_stop(run, &mut stopped_rx, Duration::from_millis(200));
        let (result, ()) = tokio::join!(observed, producer);

        assert_eq!(
            result.expect("local runner exceeded its original stop deadline"),
            None,
            "local runner must not hide an owner-delivery failure"
        );
    }

    #[rstest]
    #[case(10)]
    #[case(100)]
    #[case(1000)]
    fn test_channel_send_performance(#[case] count: usize) {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<DataEvent>();
        let quote = test_quote();

        // Send events
        for _ in 0..count {
            tx.send(DataEvent::Data(Data::Quote(quote))).unwrap();
        }

        // Verify all received
        let mut received = 0;
        while rx.try_recv().is_ok() {
            received += 1;
        }

        assert_eq!(received, count);
    }

    #[rstest]
    fn test_async_trading_command_sender_creation() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let sender = AsyncTradingCommandSender::new(tx);
        assert!(format!("{sender:?}").contains("AsyncTradingCommandSender"));
    }

    #[rstest]
    fn test_async_trading_command_sender_preserves_target_endpoints() {
        std::thread::spawn(|| {
            msgbus::get_message_bus().borrow_mut().dispose();
            let (risk_handler, risk_saving_handler) =
                get_typed_into_message_saving_handler::<TradingCommand>(Some(Ustr::from(
                    "RiskEngine.execute",
                )));
            msgbus::register_trading_command_endpoint(
                MessagingSwitchboard::risk_engine_execute(),
                risk_handler,
            );
            let (exec_handler, exec_saving_handler) =
                get_typed_into_message_saving_handler::<TradingCommand>(Some(Ustr::from(
                    "ExecEngine.execute",
                )));
            msgbus::register_trading_command_endpoint(
                MessagingSwitchboard::exec_engine_execute(),
                exec_handler,
            );

            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<TradingCommandMessage>();
            let sender = AsyncTradingCommandSender::new(tx);
            sender.execute(TradingCommandMessage::new(
                MessagingSwitchboard::risk_engine_execute(),
                TradingCommand::CancelAllOrders(CancelAllOrders::new(
                    TraderId::from("TRADER-001"),
                    None,
                    StrategyId::from("RISK-001"),
                    InstrumentId::from("EUR/USD.SIM"),
                    Some(OrderSide::Buy),
                    UUID4::new(),
                    UnixNanos::default(),
                    None,
                    None,
                )),
            ));
            sender.execute(TradingCommandMessage::new(
                MessagingSwitchboard::exec_engine_execute(),
                TradingCommand::CancelAllOrders(CancelAllOrders::new(
                    TraderId::from("TRADER-001"),
                    None,
                    StrategyId::from("EXEC-001"),
                    InstrumentId::from("EUR/USD.SIM"),
                    Some(OrderSide::Sell),
                    UUID4::new(),
                    UnixNanos::default(),
                    None,
                    None,
                )),
            ));

            AsyncRunner::handle_trading_command(rx.try_recv().unwrap());
            AsyncRunner::handle_trading_command(rx.try_recv().unwrap());

            let risk_commands = risk_saving_handler.get_messages();
            let exec_commands = exec_saving_handler.get_messages();
            assert!(rx.try_recv().is_err());
            assert_eq!(risk_commands.len(), 1);
            assert_eq!(
                risk_commands[0].strategy_id(),
                Some(StrategyId::from("RISK-001"))
            );
            assert_eq!(exec_commands.len(), 1);
            assert_eq!(
                exec_commands[0].strategy_id(),
                Some(StrategyId::from("EXEC-001"))
            );
        })
        .join()
        .unwrap();
    }

    #[rstest]
    fn test_async_runner_preserves_deferred_follow_up_order() {
        std::thread::spawn(|| {
            msgbus::get_message_bus().borrow_mut().dispose();
            let clock = Rc::new(RefCell::new(TestClock::new()));
            let cache = Rc::new(RefCell::new(Cache::default()));
            let exec_engine = Rc::new(RefCell::new(ExecutionEngine::new(clock, cache, None)));
            ExecutionEngine::register_msgbus_handlers(&exec_engine);
            msgbus::register_trading_command_endpoint(
                MessagingSwitchboard::risk_engine_execute(),
                TypedIntoHandler::from(|command: TradingCommand| {
                    msgbus::send_trading_command(
                        MessagingSwitchboard::exec_engine_queue_execute(),
                        command,
                    );
                }),
            );
            let (exec_handler, exec_saving_handler) =
                get_typed_into_message_saving_handler::<TradingCommand>(Some(Ustr::from(
                    "ExecEngine.execute",
                )));
            msgbus::register_trading_command_endpoint(
                MessagingSwitchboard::exec_engine_execute(),
                exec_handler,
            );

            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<TradingCommandMessage>();
            let sender = Arc::new(AsyncTradingCommandSender::new(tx));
            replace_exec_cmd_sender(sender.clone());
            sender.execute(TradingCommandMessage::new(
                MessagingSwitchboard::risk_engine_execute(),
                TradingCommand::CancelAllOrders(CancelAllOrders::new(
                    TraderId::from("TRADER-001"),
                    None,
                    StrategyId::from("FIRST-001"),
                    InstrumentId::from("EUR/USD.SIM"),
                    Some(OrderSide::Buy),
                    UUID4::new(),
                    UnixNanos::default(),
                    None,
                    None,
                )),
            ));
            sender.execute(TradingCommandMessage::new(
                MessagingSwitchboard::exec_engine_execute(),
                TradingCommand::CancelAllOrders(CancelAllOrders::new(
                    TraderId::from("TRADER-001"),
                    None,
                    StrategyId::from("SECOND-001"),
                    InstrumentId::from("EUR/USD.SIM"),
                    Some(OrderSide::Sell),
                    UUID4::new(),
                    UnixNanos::default(),
                    None,
                    None,
                )),
            ));

            AsyncRunner::handle_trading_command(rx.try_recv().unwrap());
            AsyncRunner::handle_trading_command(rx.try_recv().unwrap());

            let commands = exec_saving_handler.get_messages();
            let strategy_ids = commands
                .iter()
                .map(TradingCommand::strategy_id)
                .collect::<Vec<_>>();
            assert!(rx.try_recv().is_err());
            assert_eq!(commands.len(), 2);
            assert_eq!(
                strategy_ids,
                vec![
                    Some(StrategyId::from("FIRST-001")),
                    Some(StrategyId::from("SECOND-001"))
                ]
            );
        })
        .join()
        .unwrap();
    }

    #[rstest]
    fn test_async_runner_dispatches_deferred_exec_command_once() {
        std::thread::spawn(|| {
            msgbus::get_message_bus().borrow_mut().dispose();
            let clock = Rc::new(RefCell::new(TestClock::new()));
            let cache = Rc::new(RefCell::new(Cache::default()));
            let exec_engine = Rc::new(RefCell::new(ExecutionEngine::new(clock, cache, None)));
            ExecutionEngine::register_msgbus_handlers(&exec_engine);

            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<TradingCommandMessage>();
            replace_exec_cmd_sender(Arc::new(AsyncTradingCommandSender::new(tx)));
            let command = TradingCommand::CancelAllOrders(CancelAllOrders::new(
                TraderId::from("TRADER-001"),
                None,
                StrategyId::from("EXEC-001"),
                InstrumentId::from("EUR/USD.SIM"),
                Some(OrderSide::Buy),
                UUID4::new(),
                UnixNanos::default(),
                None,
                None,
            ));

            msgbus::send_trading_command(
                MessagingSwitchboard::exec_engine_queue_execute(),
                command,
            );
            assert_eq!(exec_engine.borrow().command_count(), 0);

            AsyncRunner::handle_trading_command(rx.try_recv().unwrap());

            assert!(rx.try_recv().is_err());
            assert_eq!(exec_engine.borrow().command_count(), 1);
        })
        .join()
        .unwrap();
    }

    #[tokio::test]
    async fn test_runner_processes_trading_commands() {
        let (_data_evt_tx, data_evt_rx) = tokio::sync::mpsc::unbounded_channel::<DataEvent>();
        let (_data_cmd_tx, data_cmd_rx) = tokio::sync::mpsc::unbounded_channel::<DataCommand>();
        let (_time_evt_tx, time_evt_rx) =
            tokio::sync::mpsc::unbounded_channel::<TimeEventMessage>();
        let (_exec_evt_tx, exec_evt_rx) = tokio::sync::mpsc::unbounded_channel::<ExecutionEvent>();
        let (exec_cmd_tx, exec_cmd_rx) =
            tokio::sync::mpsc::unbounded_channel::<TradingCommandMessage>();
        let (signal_tx, signal_rx) = tokio::sync::mpsc::unbounded_channel::<()>();

        let mut runner = create_test_runner(
            time_evt_rx,
            data_evt_rx,
            data_cmd_rx,
            exec_evt_rx,
            exec_cmd_rx,
            signal_rx,
            signal_tx.clone(),
        );

        let (stopped_tx, mut stopped_rx) = tokio::sync::oneshot::channel();
        let producer = async move {
            let command = TradingCommand::CancelAllOrders(CancelAllOrders::new(
                TraderId::from("TRADER-001"),
                None,
                StrategyId::from("S-001"),
                InstrumentId::from("EUR/USD.SIM"),
                Some(OrderSide::Buy),
                UUID4::new(),
                UnixNanos::default(),
                None,
                None, // correlation_id
            ));
            exec_cmd_tx
                .send(TradingCommandMessage::new(
                    MessagingSwitchboard::exec_engine_execute(),
                    command,
                ))
                .unwrap();

            tokio::task::yield_now().await;
            let stopped_at = dst::time::Instant::now();
            signal_tx.send(()).unwrap();
            stopped_tx.send(stopped_at).unwrap();
        };
        let run = async {
            runner.run().await;
            runner.owner_delivery_failure()
        };
        let observed = finish_runner_after_stop(run, &mut stopped_rx, Duration::from_millis(100));
        let (result, ()) = tokio::join!(observed, producer);

        assert!(result.is_ok(), "Runner should process command and stop");
        assert_eq!(
            result.expect("local runner exceeded its original stop deadline"),
            None,
            "local runner must not hide an owner-delivery failure"
        );
    }

    #[tokio::test]
    async fn test_runner_processes_multiple_trading_commands() {
        let (_data_evt_tx, data_evt_rx) = tokio::sync::mpsc::unbounded_channel::<DataEvent>();
        let (_data_cmd_tx, data_cmd_rx) = tokio::sync::mpsc::unbounded_channel::<DataCommand>();
        let (_time_evt_tx, time_evt_rx) =
            tokio::sync::mpsc::unbounded_channel::<TimeEventMessage>();
        let (_exec_evt_tx, exec_evt_rx) = tokio::sync::mpsc::unbounded_channel::<ExecutionEvent>();
        let (exec_cmd_tx, exec_cmd_rx) =
            tokio::sync::mpsc::unbounded_channel::<TradingCommandMessage>();
        let (signal_tx, signal_rx) = tokio::sync::mpsc::unbounded_channel::<()>();

        let mut runner = create_test_runner(
            time_evt_rx,
            data_evt_rx,
            data_cmd_rx,
            exec_evt_rx,
            exec_cmd_rx,
            signal_rx,
            signal_tx.clone(),
        );

        let (stopped_tx, mut stopped_rx) = tokio::sync::oneshot::channel();
        let producer = async move {
            for i in 0..10 {
                let strategy_id = format!("S-{i:03}");
                let command = TradingCommand::CancelAllOrders(CancelAllOrders::new(
                    TraderId::from("TRADER-001"),
                    None,
                    StrategyId::from(strategy_id.as_str()),
                    InstrumentId::from("EUR/USD.SIM"),
                    Some(OrderSide::Buy),
                    UUID4::new(),
                    UnixNanos::default(),
                    None,
                    None, // correlation_id
                ));
                exec_cmd_tx
                    .send(TradingCommandMessage::new(
                        MessagingSwitchboard::exec_engine_execute(),
                        command,
                    ))
                    .unwrap();
            }

            tokio::task::yield_now().await;
            let stopped_at = dst::time::Instant::now();
            signal_tx.send(()).unwrap();
            stopped_tx.send(stopped_at).unwrap();
        };
        let run = async {
            runner.run().await;
            runner.owner_delivery_failure()
        };
        let observed = finish_runner_after_stop(run, &mut stopped_rx, Duration::from_millis(100));
        let (result, ()) = tokio::join!(observed, producer);

        assert!(
            result.is_ok(),
            "Runner should process all commands and stop"
        );
        assert_eq!(
            result.expect("local runner exceeded its original stop deadline"),
            None,
            "local runner must not hide an owner-delivery failure"
        );
    }

    #[tokio::test]
    async fn test_execution_event_order_channel() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<ExecutionEvent>();

        let event = OrderSubmittedSpec::builder()
            .client_order_id(ClientOrderId::from("O-001"))
            .build();

        tx.send(ExecutionEvent::Order(OrderEventAny::Submitted(event)))
            .unwrap();

        let received = rx.recv().await.unwrap();
        match received {
            ExecutionEvent::Order(OrderEventAny::Submitted(e)) => {
                assert_eq!(e.client_order_id(), ClientOrderId::from("O-001"));
            }
            _ => panic!("Expected OrderSubmitted event"),
        }
    }

    #[tokio::test]
    async fn test_execution_report_order_status_channel() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<ExecutionEvent>();

        let report = OrderStatusReport::new(
            AccountId::from("SIM-001"),
            InstrumentId::from("EUR/USD.SIM"),
            Some(ClientOrderId::from("O-001")),
            VenueOrderId::from("V-001"),
            OrderSide::Buy.into(),
            OrderType::Market,
            TimeInForce::Gtc,
            OrderStatus::Accepted,
            Quantity::from(100_000),
            Quantity::from(100_000),
            UnixNanos::from(1),
            UnixNanos::from(2),
            UnixNanos::from(3),
            None,
        );

        tx.send(ExecutionEvent::Report(ExecutionReport::Order(Box::new(
            report,
        ))))
        .unwrap();

        let received = rx.recv().await.unwrap();
        match received {
            ExecutionEvent::Report(ExecutionReport::Order(r)) => {
                assert_eq!(r.venue_order_id.as_str(), "V-001");
                assert_eq!(r.order_status, OrderStatus::Accepted);
            }
            _ => panic!("Expected OrderStatusReport"),
        }
    }

    #[tokio::test]
    async fn test_execution_report_fill() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<ExecutionEvent>();

        let report = FillReport::new(
            AccountId::from("SIM-001"),
            InstrumentId::from("EUR/USD.SIM"),
            VenueOrderId::from("V-001"),
            TradeId::from("T-001"),
            OrderSide::Buy,
            Quantity::from(100_000),
            Price::from("1.10000"),
            Money::from("10 USD"),
            LiquiditySide::Taker,
            Some(ClientOrderId::from("O-001")),
            None,
            UnixNanos::from(1),
            UnixNanos::from(2),
            None,
        );

        tx.send(ExecutionEvent::Report(ExecutionReport::Fill(Box::new(
            report,
        ))))
        .unwrap();

        let received = rx.recv().await.unwrap();
        match received {
            ExecutionEvent::Report(ExecutionReport::Fill(r)) => {
                assert_eq!(r.venue_order_id.as_str(), "V-001");
                assert_eq!(r.trade_id.to_string(), "T-001");
            }
            _ => panic!("Expected FillReport"),
        }
    }

    #[tokio::test]
    async fn test_execution_report_position() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<ExecutionEvent>();

        let report = PositionStatusReport::new(
            AccountId::from("SIM-001"),
            InstrumentId::from("EUR/USD.SIM"),
            PositionSide::Long,
            Quantity::from(100_000),
            UnixNanos::from(1),
            UnixNanos::from(2),
            None,
            Some(PositionId::from("P-001")),
            None,
        );

        tx.send(ExecutionEvent::Report(ExecutionReport::Position(Box::new(
            report,
        ))))
        .unwrap();

        let received = rx.recv().await.unwrap();
        match received {
            ExecutionEvent::Report(ExecutionReport::Position(r)) => {
                assert_eq!(r.venue_position_id.unwrap().as_str(), "P-001");
            }
            _ => panic!("Expected PositionStatusReport"),
        }
    }

    #[tokio::test]
    async fn test_execution_event_account() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<ExecutionEvent>();

        let account_state = AccountState::new(
            AccountId::from("SIM-001"),
            AccountType::Cash,
            vec![],
            vec![],
            true,
            UUID4::new(),
            UnixNanos::from(1),
            UnixNanos::from(2),
            None,
        );

        tx.send(ExecutionEvent::Account(account_state)).unwrap();

        let received = rx.recv().await.unwrap();
        match received {
            ExecutionEvent::Account(r) => {
                assert_eq!(r.account_id.as_str(), "SIM-001");
            }
            _ => panic!("Expected AccountState"),
        }
    }

    #[tokio::test]
    async fn test_runner_stop_method() {
        let (_data_tx, data_evt_rx) = tokio::sync::mpsc::unbounded_channel::<DataEvent>();
        let (_cmd_tx, data_cmd_rx) = tokio::sync::mpsc::unbounded_channel::<DataCommand>();
        let (_time_tx, time_evt_rx) = tokio::sync::mpsc::unbounded_channel::<TimeEventMessage>();
        let (_exec_evt_tx, exec_evt_rx) = tokio::sync::mpsc::unbounded_channel::<ExecutionEvent>();
        let (_exec_cmd_tx, exec_cmd_rx) =
            tokio::sync::mpsc::unbounded_channel::<TradingCommandMessage>();
        let (signal_tx, signal_rx) = tokio::sync::mpsc::unbounded_channel::<()>();

        let mut runner = create_test_runner(
            time_evt_rx,
            data_evt_rx,
            data_cmd_rx,
            exec_evt_rx,
            exec_cmd_rx,
            signal_rx,
            signal_tx.clone(),
        );

        let (stopped_tx, mut stopped_rx) = tokio::sync::oneshot::channel();
        let producer = async move {
            // Use stop via signal_tx directly
            let stopped_at = dst::time::Instant::now();
            signal_tx.send(()).unwrap();
            stopped_tx.send(stopped_at).unwrap();
        };
        let run = async {
            runner.run().await;
            runner.owner_delivery_failure()
        };
        let observed = finish_runner_after_stop(run, &mut stopped_rx, Duration::from_millis(100));
        let (result, ()) = tokio::join!(observed, producer);

        assert!(result.is_ok(), "Runner should stop when stop() is called");
        assert_eq!(
            result.expect("local runner exceeded its original stop deadline"),
            None,
            "local runner must not hide an owner-delivery failure"
        );
    }

    #[tokio::test]
    async fn test_all_event_types_integration() {
        let (data_evt_tx, data_evt_rx) = tokio::sync::mpsc::unbounded_channel::<DataEvent>();
        let (data_cmd_tx, data_cmd_rx) = tokio::sync::mpsc::unbounded_channel::<DataCommand>();
        let (time_evt_tx, time_evt_rx) = tokio::sync::mpsc::unbounded_channel::<TimeEventMessage>();
        let (exec_evt_tx, exec_evt_rx) = tokio::sync::mpsc::unbounded_channel::<ExecutionEvent>();
        let (_exec_cmd_tx, exec_cmd_rx) =
            tokio::sync::mpsc::unbounded_channel::<TradingCommandMessage>();
        let (signal_tx, signal_rx) = tokio::sync::mpsc::unbounded_channel::<()>();

        let mut runner = create_test_runner(
            time_evt_rx,
            data_evt_rx,
            data_cmd_rx,
            exec_evt_rx,
            exec_cmd_rx,
            signal_rx,
            signal_tx.clone(),
        );

        let (stopped_tx, mut stopped_rx) = tokio::sync::oneshot::channel();
        let producer = async move {
            // Send data event
            let quote = test_quote();
            data_evt_tx
                .send(DataEvent::Data(Data::Quote(quote)))
                .unwrap();

            // Send data command
            let command = DataCommand::Subscribe(SubscribeCommand::Data(SubscribeCustomData {
                client_id: Some(ClientId::from("TEST")),
                venue: None,
                data_type: DataType::new("QuoteTick", None, None),
                command_id: UUID4::new(),
                ts_init: UnixNanos::default(),
                correlation_id: None,
                params: None,
            }));
            data_cmd_tx.send(command).unwrap();

            // Send time event
            let event = TimeEvent::new(
                Ustr::from("test"),
                UUID4::new(),
                UnixNanos::from(1),
                UnixNanos::from(2),
            );
            let callback = TimeEventCallback::from(|_: TimeEvent| {});
            let message = TimeEventMessage::new(event, callback);
            time_evt_tx.send(message).unwrap();

            // Send execution order event
            let order_event = OrderSubmittedSpec::builder()
                .client_order_id(ClientOrderId::from("O-001"))
                .build();
            exec_evt_tx
                .send(ExecutionEvent::Order(OrderEventAny::Submitted(order_event)))
                .unwrap();

            // Send execution report (OrderStatus)
            let order_status = OrderStatusReport::new(
                AccountId::from("SIM-001"),
                InstrumentId::from("EUR/USD.SIM"),
                Some(ClientOrderId::from("O-001")),
                VenueOrderId::from("V-001"),
                OrderSide::Buy.into(),
                OrderType::Market,
                TimeInForce::Gtc,
                OrderStatus::Accepted,
                Quantity::from(100_000),
                Quantity::from(100_000),
                UnixNanos::from(1),
                UnixNanos::from(2),
                UnixNanos::from(3),
                None,
            );
            exec_evt_tx
                .send(ExecutionEvent::Report(ExecutionReport::Order(Box::new(
                    order_status,
                ))))
                .unwrap();

            // Send execution report (Fill)
            let fill = FillReport::new(
                AccountId::from("SIM-001"),
                InstrumentId::from("EUR/USD.SIM"),
                VenueOrderId::from("V-001"),
                TradeId::from("T-001"),
                OrderSide::Buy,
                Quantity::from(100_000),
                Price::from("1.10000"),
                Money::from("10 USD"),
                LiquiditySide::Taker,
                Some(ClientOrderId::from("O-001")),
                None,
                UnixNanos::from(1),
                UnixNanos::from(2),
                None,
            );
            exec_evt_tx
                .send(ExecutionEvent::Report(ExecutionReport::Fill(Box::new(
                    fill,
                ))))
                .unwrap();

            // Send execution report (Position)
            let position = PositionStatusReport::new(
                AccountId::from("SIM-001"),
                InstrumentId::from("EUR/USD.SIM"),
                PositionSide::Long,
                Quantity::from(100_000),
                UnixNanos::from(1),
                UnixNanos::from(2),
                None,
                Some(PositionId::from("P-001")),
                None,
            );
            exec_evt_tx
                .send(ExecutionEvent::Report(ExecutionReport::Position(Box::new(
                    position,
                ))))
                .unwrap();

            // Send account event
            let account_state = AccountState::new(
                AccountId::from("SIM-001"),
                AccountType::Cash,
                vec![],
                vec![],
                true,
                UUID4::new(),
                UnixNanos::from(1),
                UnixNanos::from(2),
                None,
            );
            exec_evt_tx
                .send(ExecutionEvent::Account(account_state))
                .unwrap();

            // Yield to let runner enter event loop before stop signal
            tokio::task::yield_now().await;
            let stopped_at = dst::time::Instant::now();
            signal_tx.send(()).unwrap();
            stopped_tx.send(stopped_at).unwrap();
        };
        let run = async {
            runner.run().await;
            runner.owner_delivery_failure()
        };
        let observed = finish_runner_after_stop(run, &mut stopped_rx, Duration::from_millis(200));
        let (result, ()) = tokio::join!(observed, producer);

        assert!(
            result.is_ok(),
            "Runner should process all event types and stop cleanly"
        );
        assert_eq!(
            result.expect("local runner exceeded its original stop deadline"),
            None,
            "local runner must not hide an owner-delivery failure"
        );
    }

    #[tokio::test]
    async fn test_runner_handle_stops_runner() {
        let (_data_tx, data_evt_rx) = tokio::sync::mpsc::unbounded_channel::<DataEvent>();
        let (_cmd_tx, data_cmd_rx) = tokio::sync::mpsc::unbounded_channel::<DataCommand>();
        let (_time_tx, time_evt_rx) = tokio::sync::mpsc::unbounded_channel::<TimeEventMessage>();
        let (_exec_evt_tx, exec_evt_rx) = tokio::sync::mpsc::unbounded_channel::<ExecutionEvent>();
        let (_exec_cmd_tx, exec_cmd_rx) =
            tokio::sync::mpsc::unbounded_channel::<TradingCommandMessage>();
        let (signal_tx, signal_rx) = tokio::sync::mpsc::unbounded_channel::<()>();

        let mut runner = create_test_runner(
            time_evt_rx,
            data_evt_rx,
            data_cmd_rx,
            exec_evt_rx,
            exec_cmd_rx,
            signal_rx,
            signal_tx.clone(),
        );

        // Get handle before moving runner
        let handle = runner.handle();

        let (stopped_tx, mut stopped_rx) = tokio::sync::oneshot::channel();
        let producer = async move {
            // Use handle to stop
            let stopped_at = dst::time::Instant::now();
            handle.stop();
            stopped_tx.send(stopped_at).unwrap();
        };
        let run = async {
            runner.run().await;
            runner.owner_delivery_failure()
        };
        let observed = finish_runner_after_stop(run, &mut stopped_rx, Duration::from_millis(100));
        let (result, ()) = tokio::join!(observed, producer);

        assert!(result.is_ok(), "Runner should stop via handle");
        assert_eq!(
            result.expect("local runner exceeded its original stop deadline"),
            None,
            "local runner must not hide an owner-delivery failure"
        );
    }

    #[tokio::test]
    async fn test_rejected_runner_preserves_first_owner_senders_and_original_queues() {
        msgbus::get_message_bus().borrow_mut().dispose();
        let mut first = AsyncRunner::new();
        let first_owner = first.owner_delivery.bind().unwrap();
        first.bind_senders();
        let data = get_data_cmd_sender();
        let trading = get_trading_cmd_sender();
        let time = get_time_event_sender();
        let data_events = get_data_event_sender();
        let exec_events = get_exec_event_sender();
        let system_events = get_system_event_sender();
        let system_commands = get_system_command_sender();
        let effects = Rc::new(std::cell::Cell::new(0));
        let endpoint = "LocalRunner.rejectedOwnerQuery".into();
        msgbus::register_trading_command_endpoint(
            endpoint,
            TypedIntoHandler::from({
                let effects = effects.clone();
                move |_command: TradingCommand| effects.set(effects.get() + 1)
            }),
        );
        let make_query = || {
            TradingCommand::QueryAccount(QueryAccount::new(
                TraderId::from("TRADER-001"),
                None,
                AccountId::from("SIM-001"),
                UUID4::new(),
                UnixNanos::from(1),
                None,
                None,
            ))
        };
        let first_query = make_query();
        first
            .exec_cmd_tx
            .send(TradingCommandMessage::new(endpoint, first_query.clone()))
            .unwrap();
        let mut second = AsyncRunner::new();
        let second_query = make_query();
        second
            .exec_cmd_tx
            .send(TradingCommandMessage::new(endpoint, second_query.clone()))
            .unwrap();
        for _ in 0..2 {
            tokio::time::timeout(Duration::from_millis(100), second.run())
                .await
                .expect("another owner must be rejected without waiting for channel input");
        }
        let initial_failure = second.owner_delivery_failure();
        drop(first_owner);
        tokio::time::timeout(Duration::from_millis(100), second.run())
            .await
            .expect("a rejected runner must stay failed after the first owner leaves");
        let same_senders = [
            Arc::ptr_eq(&data, &get_data_cmd_sender()),
            Arc::ptr_eq(&trading, &get_trading_cmd_sender()),
            Arc::ptr_eq(&time, &get_time_event_sender()),
            data_events.same_channel(&get_data_event_sender()),
            exec_events.same_channel(&get_exec_event_sender()),
            system_events.same_channel(&get_system_event_sender()),
            system_commands.same_channel(&get_system_command_sender()),
        ];
        let first_pending = first.channels.exec_cmd_rx.try_recv().unwrap();
        let second_pending = second.channels.exec_cmd_rx.try_recv().unwrap();
        assert_eq!(first_pending.endpoint(), endpoint);
        assert_eq!(first_pending.command(), &first_query);
        assert_eq!(second_pending.endpoint(), endpoint);
        assert_eq!(second_pending.command(), &second_query);
        assert!(first.channels.exec_cmd_rx.try_recv().is_err());
        assert!(second.channels.exec_cmd_rx.try_recv().is_err());
        let observation = (
            same_senders,
            first.owner_delivery_failure(),
            first.owner_delivery.failure(),
            initial_failure,
            second.owner_delivery_failure(),
            effects.get(),
        );
        eprintln!("rejected owner observations: {observation:?}");
        assert_eq!(
            observation,
            (
                [true; 7],
                None,
                None,
                Some(nautilus_common::owner_delivery::OwnerDeliveryError::WrongOwner),
                Some(nautilus_common::owner_delivery::OwnerDeliveryError::WrongOwner),
                0,
            )
        );
        msgbus::get_message_bus().borrow_mut().dispose();
    }

    #[tokio::test]
    async fn test_local_runner_consumes_cross_thread_senders_and_stop_handle() {
        msgbus::get_message_bus().borrow_mut().dispose();
        let owner_thread = std::thread::current().id();
        let mut runner = AsyncRunner::new();
        let data_sender = AsyncDataCommandSender::new(runner.data_cmd_tx.clone());
        let trading_sender = AsyncTradingCommandSender::new(runner.exec_cmd_tx.clone());
        let time_sender = AsyncTimeEventSender::new(runner.time_evt_tx.clone());
        let stop = runner.handle();
        let (ack_tx, ack_rx) = std::sync::mpsc::channel();
        let data_observed = Rc::new(RefCell::new(Vec::new()));
        let trading_observed = Rc::new(RefCell::new(Vec::new()));
        let time_observed = Arc::new(std::sync::Mutex::new(Vec::new()));
        let endpoint = "LocalRunner.crossThreadQuery".into();
        msgbus::register_data_command_endpoint(
            MessagingSwitchboard::data_engine_execute(),
            TypedIntoHandler::from({
                let observed = data_observed.clone();
                let ack = ack_tx.clone();
                move |command: DataCommand| {
                    observed.borrow_mut().push(command);
                    ack.send(("data", std::thread::current().id())).unwrap();
                }
            }),
        );
        msgbus::register_trading_command_endpoint(
            endpoint,
            TypedIntoHandler::from({
                let observed = trading_observed.clone();
                let ack = ack_tx.clone();
                move |command: TradingCommand| {
                    observed.borrow_mut().push(command);
                    ack.send(("trading", std::thread::current().id())).unwrap();
                }
            }),
        );
        let data_command = DataCommand::Subscribe(SubscribeCommand::Data(SubscribeCustomData {
            client_id: Some(ClientId::from("TEST")),
            venue: None,
            data_type: DataType::new("cross_thread_test", None, None),
            command_id: UUID4::new(),
            ts_init: UnixNanos::from(1),
            correlation_id: None,
            params: None,
        }));
        let trading_command = TradingCommand::QueryAccount(QueryAccount::new(
            TraderId::from("TRADER-001"),
            None,
            AccountId::from("SIM-001"),
            UUID4::new(),
            UnixNanos::from(2),
            None,
            None,
        ));
        let time_event = TimeEvent::new(
            Ustr::from("cross_thread_test"),
            UUID4::new(),
            UnixNanos::from(3),
            UnixNanos::from(4),
        );
        let worker = std::thread::spawn({
            let data_command = data_command.clone();
            let trading_command = trading_command.clone();
            let time_event = time_event.clone();
            let time_observed = time_observed.clone();
            move || {
                data_sender.execute(data_command);
                trading_sender.execute(TradingCommandMessage::new(endpoint, trading_command));
                time_sender.send(TimeEventMessage::new(
                    time_event,
                    TimeEventCallback::from(move |event: TimeEvent| {
                        time_observed.lock().unwrap().push(event);
                        ack_tx.send(("time", std::thread::current().id())).unwrap();
                    }),
                ));
                let mut labels = Vec::new();
                for _ in 0..3 {
                    let (label, actual_thread) =
                        ack_rx.recv_timeout(Duration::from_secs(2)).unwrap();
                    assert_eq!(actual_thread, owner_thread);
                    labels.push(label);
                }
                labels.sort_unstable();
                assert_eq!(labels, ["data", "time", "trading"]);
                stop.stop();
                std::thread::current().id()
            }
        });
        let result = dst::time::timeout(Duration::from_secs(5), async {
            runner.run().await;
            (
                runner.owner_delivery_failure(),
                runner.owner_delivery.pending(),
            )
        })
        .await;
        let sender_thread = worker.join().expect("cross-thread sender panicked");
        assert_ne!(sender_thread, owner_thread);
        assert_eq!(
            result.expect("local runner did not stop after all three acknowledgements"),
            (None, 0)
        );
        assert_eq!(&*data_observed.borrow(), &[data_command]);
        assert_eq!(&*trading_observed.borrow(), &[trading_command]);
        assert_eq!(&*time_observed.lock().unwrap(), &[time_event]);
        msgbus::get_message_bus().borrow_mut().dispose();
    }

    #[tokio::test]
    async fn test_runner_handle_is_cloneable() {
        let (signal_tx, _signal_rx) = tokio::sync::mpsc::unbounded_channel::<()>();
        let handle = AsyncRunnerHandle { signal_tx };

        let handle2 = handle.clone();

        // Both handles should be able to send stop signals
        assert!(handle.signal_tx.send(()).is_ok());
        assert!(handle2.signal_tx.send(()).is_ok());
    }

    #[tokio::test]
    async fn test_runner_processes_events_before_stop() {
        let (data_evt_tx, data_evt_rx) = tokio::sync::mpsc::unbounded_channel::<DataEvent>();
        let (_cmd_tx, data_cmd_rx) = tokio::sync::mpsc::unbounded_channel::<DataCommand>();
        let (_time_tx, time_evt_rx) = tokio::sync::mpsc::unbounded_channel::<TimeEventMessage>();
        let (_exec_evt_tx, exec_evt_rx) = tokio::sync::mpsc::unbounded_channel::<ExecutionEvent>();
        let (_exec_cmd_tx, exec_cmd_rx) =
            tokio::sync::mpsc::unbounded_channel::<TradingCommandMessage>();
        let (signal_tx, signal_rx) = tokio::sync::mpsc::unbounded_channel::<()>();

        let mut runner = create_test_runner(
            time_evt_rx,
            data_evt_rx,
            data_cmd_rx,
            exec_evt_rx,
            exec_cmd_rx,
            signal_rx,
            signal_tx.clone(),
        );

        let handle = runner.handle();

        // Send events before starting runner
        for _ in 0..10 {
            let quote = test_quote();
            data_evt_tx
                .send(DataEvent::Data(Data::Quote(quote)))
                .unwrap();
        }

        let (stopped_tx, mut stopped_rx) = tokio::sync::oneshot::channel();
        let producer = async move {
            // Yield to let runner enter event loop before stop signal
            tokio::task::yield_now().await;
            let stopped_at = dst::time::Instant::now();
            handle.stop();
            stopped_tx.send(stopped_at).unwrap();
        };
        let run = async {
            runner.run().await;
            runner.owner_delivery_failure()
        };
        let observed = finish_runner_after_stop(run, &mut stopped_rx, Duration::from_millis(200));
        let (result, ()) = tokio::join!(observed, producer);

        assert!(result.is_ok(), "Runner should process events and stop");
        assert_eq!(
            result.expect("local runner exceeded its original stop deadline"),
            None,
            "local runner must not hide an owner-delivery failure"
        );
    }

    #[rstest]
    fn test_new_does_not_bind_tls() {
        std::thread::spawn(|| {
            let _runner = AsyncRunner::new();
            assert!(try_get_time_event_sender().is_none());
            assert!(try_get_system_command_sender().is_none());
            assert!(try_get_system_event_sender().is_none());
            assert!(try_get_trading_cmd_sender().is_none());
        })
        .join()
        .unwrap();
    }

    #[rstest]
    fn test_bind_senders_routes_to_runner_channels() {
        std::thread::spawn(|| {
            let mut runner = AsyncRunner::new();
            runner.bind_senders();

            get_data_cmd_sender().execute(DataCommand::Subscribe(SubscribeCommand::Data(
                SubscribeCustomData {
                    client_id: Some(ClientId::from("TEST")),
                    venue: None,
                    data_type: DataType::new("test", None, None),
                    command_id: UUID4::new(),
                    ts_init: UnixNanos::default(),
                    correlation_id: None,
                    params: None,
                },
            )));
            assert!(runner.channels.data_cmd_rx.try_recv().is_ok());

            get_trading_cmd_sender().execute(TradingCommandMessage::new(
                MessagingSwitchboard::exec_engine_execute(),
                TradingCommand::CancelAllOrders(CancelAllOrders::new(
                    TraderId::from("TRADER-001"),
                    None,
                    StrategyId::from("S-001"),
                    InstrumentId::from("EUR/USD.SIM"),
                    Some(OrderSide::Buy),
                    UUID4::new(),
                    UnixNanos::default(),
                    None,
                    None, // correlation_id
                )),
            ));
            assert!(runner.channels.exec_cmd_rx.try_recv().is_ok());

            let event = TimeEvent::new(
                Ustr::from("test"),
                UUID4::new(),
                UnixNanos::from(1),
                UnixNanos::from(2),
            );
            let callback = TimeEventCallback::from(|_: TimeEvent| {});
            get_time_event_sender().send(TimeEventMessage::new(event, callback));
            assert!(runner.channels.time_evt_rx.try_recv().is_ok());

            get_system_event_sender().send(test_system_event()).unwrap();
            assert_eq!(
                runner.channels.system_evt_rx.try_recv().unwrap(),
                test_system_event()
            );

            get_system_command_sender()
                .send(test_system_command())
                .unwrap();
            assert_eq!(
                runner.channels.system_cmd_rx.try_recv().unwrap(),
                test_system_command()
            );

            get_data_event_sender()
                .send(DataEvent::Data(Data::Quote(test_quote())))
                .unwrap();
            assert!(runner.channels.data_evt_rx.try_recv().is_ok());

            let account = AccountState::new(
                AccountId::from("SIM-001"),
                AccountType::Cash,
                vec![],
                vec![],
                true,
                UUID4::new(),
                UnixNanos::from(1),
                UnixNanos::from(2),
                None,
            );
            get_exec_event_sender()
                .send(ExecutionEvent::Account(account))
                .unwrap();
            assert!(runner.channels.exec_evt_rx.try_recv().is_ok());
        })
        .join()
        .unwrap();
    }

    #[cfg(feature = "node")]
    #[rstest]
    fn test_drain_pending_system_events_keeps_data_events_separate() {
        std::thread::spawn(|| {
            let mut runner = AsyncRunner::new();
            runner.bind_senders();
            let system_event = test_system_event();

            get_system_event_sender().send(system_event).unwrap();
            get_data_event_sender()
                .send(DataEvent::Data(Data::Quote(test_quote())))
                .unwrap();

            let system_events = runner.drain_pending_system_events();

            assert_eq!(system_events, vec![system_event]);
            assert!(runner.channels.system_evt_rx.try_recv().is_err());
            assert!(runner.channels.data_evt_rx.try_recv().is_ok());
        })
        .join()
        .unwrap();
    }

    #[cfg(feature = "node")]
    #[rstest]
    fn test_drain_pending_system_commands_keeps_events_separate() {
        std::thread::spawn(|| {
            let mut runner = AsyncRunner::new();
            runner.bind_senders();
            let system_command = test_system_command();

            get_system_command_sender().send(system_command).unwrap();
            get_system_event_sender().send(test_system_event()).unwrap();

            let system_commands = runner.drain_pending_system_commands();

            assert_eq!(system_commands, vec![system_command]);
            assert!(runner.channels.system_cmd_rx.try_recv().is_err());
            assert!(runner.channels.system_evt_rx.try_recv().is_ok());
        })
        .join()
        .unwrap();
    }

    #[rstest]
    fn test_bind_senders_reclaims_tls_from_previous_runner() {
        std::thread::spawn(|| {
            let mut runner1 = AsyncRunner::new();
            runner1.bind_senders();

            let mut runner2 = AsyncRunner::new();
            runner2.bind_senders();

            get_data_cmd_sender().execute(DataCommand::Subscribe(SubscribeCommand::Data(
                SubscribeCustomData {
                    client_id: Some(ClientId::from("TEST")),
                    venue: None,
                    data_type: DataType::new("test", None, None),
                    command_id: UUID4::new(),
                    ts_init: UnixNanos::default(),
                    correlation_id: None,
                    params: None,
                },
            )));

            assert!(runner2.channels.data_cmd_rx.try_recv().is_ok());
            assert!(runner1.channels.data_cmd_rx.try_recv().is_err());
        })
        .join()
        .unwrap();
    }

    #[tokio::test]
    async fn test_execution_event_order_submitted_batch_channel() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<ExecutionEvent>();

        let events = vec![
            OrderSubmittedSpec::builder()
                .client_order_id(ClientOrderId::from("O-001"))
                .build(),
            OrderSubmittedSpec::builder()
                .client_order_id(ClientOrderId::from("O-002"))
                .build(),
        ];

        let batch = OrderSubmittedBatch::new(events);
        tx.send(ExecutionEvent::OrderSubmittedBatch(batch)).unwrap();

        let received = rx.recv().await.unwrap();
        match received {
            ExecutionEvent::OrderSubmittedBatch(b) => {
                assert_eq!(b.len(), 2);
                assert_eq!(b.events[0].client_order_id, ClientOrderId::from("O-001"));
                assert_eq!(b.events[1].client_order_id, ClientOrderId::from("O-002"));
            }
            _ => panic!("Expected OrderSubmittedBatch event"),
        }
    }

    #[tokio::test]
    async fn test_execution_event_order_accepted_batch_channel() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<ExecutionEvent>();

        let events = vec![
            OrderAcceptedSpec::builder()
                .client_order_id(ClientOrderId::from("O-001"))
                .build(),
            OrderAcceptedSpec::builder()
                .client_order_id(ClientOrderId::from("O-002"))
                .build(),
        ];

        let batch = OrderAcceptedBatch::new(events);
        tx.send(ExecutionEvent::OrderAcceptedBatch(batch)).unwrap();

        let received = rx.recv().await.unwrap();
        match received {
            ExecutionEvent::OrderAcceptedBatch(b) => {
                assert_eq!(b.len(), 2);
                assert_eq!(b.events[0].client_order_id, ClientOrderId::from("O-001"));
                assert_eq!(b.events[1].client_order_id, ClientOrderId::from("O-002"));
            }
            _ => panic!("Expected OrderAcceptedBatch event"),
        }
    }

    #[tokio::test]
    async fn test_execution_event_order_canceled_batch_channel() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<ExecutionEvent>();

        let events = vec![
            OrderCanceledSpec::builder()
                .client_order_id(ClientOrderId::from("O-001"))
                .build(),
            OrderCanceledSpec::builder()
                .client_order_id(ClientOrderId::from("O-002"))
                .build(),
        ];

        let batch = OrderCanceledBatch::new(events);
        tx.send(ExecutionEvent::OrderCanceledBatch(batch)).unwrap();

        let received = rx.recv().await.unwrap();
        match received {
            ExecutionEvent::OrderCanceledBatch(b) => {
                assert_eq!(b.len(), 2);
                assert_eq!(b.events[0].client_order_id, ClientOrderId::from("O-001"));
                assert_eq!(b.events[1].client_order_id, ClientOrderId::from("O-002"));
            }
            _ => panic!("Expected OrderCanceledBatch event"),
        }
    }
}
