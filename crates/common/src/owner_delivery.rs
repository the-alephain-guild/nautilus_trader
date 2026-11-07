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

//! Owner-local control phases and classified external delivery.
//!
//! A context is a current control check, never permission to submit an order.

use std::{
    cell::RefCell,
    collections::VecDeque,
    fmt,
    panic::{AssertUnwindSafe, catch_unwind},
    rc::{Rc, Weak},
    thread::{self, ThreadId},
};

use nautilus_core::UUID4;
use nautilus_model::events::{OrderEventAny, OrderInitialized, PositionEvent};

use crate::{messages::execution::TradingCommand, runner::TradingCommandMessage};

const CAPACITY: usize = 1024;

/// An entered same-thread control context.
pub trait EnteredOwnerControl {
    /// Ends the context before external callbacks run.
    ///
    /// # Errors
    ///
    /// Returns an error if the context could not finish normally.
    fn finish(&mut self, failed: bool) -> Result<(), OwnerDeliveryError>;
}

/// Concrete adapters perform their actual durable control check on entry.
pub trait OwnerControlContext: fmt::Debug {
    /// Enters the original control binding, on the owner thread.
    ///
    /// # Errors
    ///
    /// Returns an error if that original binding is no longer current.
    fn enter(&self) -> Result<Box<dyn EnteredOwnerControl>, OwnerDeliveryError>;
}

/// Closed failures retain whether native work may already have happened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OwnerDeliveryError {
    Unbound,
    WrongOwner,
    Capacity,
    SequenceExhausted,
    UnsupportedEffect,
    ControlEnter,
    ControlExit,
    NativeAction,
    NativePanicked,
    ExternalCallback,
    ReentrantDrain,
    PendingAtRetirement,
}

impl fmt::Display for OwnerDeliveryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "native owner delivery {self:?}")
    }
}

impl std::error::Error for OwnerDeliveryError {}

/// Original causation survives external delivery and native continuation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OwnerCause {
    pub command_id: UUID4,
    pub parent_event_id: Option<UUID4>,
    pub sequence: u64,
    pub incarnation: UUID4,
}

/// An observed event is not a command or a control permission.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OwnerIncomingCause {
    pub event_id: UUID4,
    pub sequence: u64,
    pub incarnation: UUID4,
}

/// Original typed output retained without inventing an order for a position.
#[derive(Clone, Debug)]
pub enum OwnerExternalEvent {
    Order(OrderEventAny),
    Position(PositionEvent),
}

/// An original normal-ingress order, or an already-cached publication.
#[derive(Clone, Debug)]
pub enum OwnerIncomingOrder {
    Process(OrderEventAny),
    PublishInitialized(OrderInitialized),
}

impl OwnerIncomingOrder {
    fn event_id(&self) -> UUID4 {
        match self {
            Self::Process(event) => event.clone().into_boxed().id(),
            Self::PublishInitialized(event) => event.event_id,
        }
    }

    fn dispatch(self) {
        match self {
            Self::Process(event) => crate::msgbus::send_order_event(
                crate::msgbus::MessagingSwitchboard::exec_engine_process(),
                event,
            ),
            Self::PublishInitialized(event) => {
                let topic = crate::msgbus::switchboard::get_event_order_topic(event.strategy_id);
                crate::msgbus::publish_order_event(topic, &OrderEventAny::Initialized(event));
            }
        }
    }
}

/// Dispatch progress does not assert that the execution engine accepted an event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OwnerIncomingStage {
    NotDispatched,
    DispatchEntered,
    DispatchReturned,
}

#[derive(Clone, Debug)]
pub struct RetainedIncomingOrder {
    pub cause: Option<OwnerIncomingCause>,
    pub event: OwnerIncomingOrder,
    pub stage: OwnerIncomingStage,
    pub error: OwnerDeliveryError,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OwnerDeliveryCause {
    Control(OwnerCause),
    Incoming(OwnerIncomingCause),
}

#[derive(Clone, Debug)]
pub struct RetainedOwnerTypedDelivery {
    pub cause: OwnerDeliveryCause,
    pub event_id: UUID4,
    pub event: OwnerExternalEvent,
    pub error: Option<OwnerDeliveryError>,
}

pub type OwnerAction = Box<dyn FnOnce() -> Result<(), OwnerDeliveryError>>;

/// Retains the original external payload, including an actual callback failure.
pub trait OwnerExternalAction {
    fn original_event(&self) -> OwnerExternalEvent;
    /// # Errors
    ///
    /// Reports delivery failure without consuming the owned original payload.
    fn deliver(&mut self) -> Result<(), OwnerDeliveryError>;
}

#[derive(Clone)]
struct Binding {
    context: Rc<dyn OwnerControlContext>,
    cause: OwnerCause,
}

#[derive(Clone)]
enum PhaseBinding {
    Control(Binding),
    Incoming(OwnerIncomingCause),
}

impl PhaseBinding {
    fn control(&self) -> Option<&Binding> {
        match self {
            Self::Control(binding) => Some(binding),
            Self::Incoming(_) => None,
        }
    }

    fn incarnation(&self) -> UUID4 {
        match self {
            Self::Control(binding) => binding.cause.incarnation,
            Self::Incoming(cause) => cause.incarnation,
        }
    }
}

impl From<Binding> for PhaseBinding {
    fn from(binding: Binding) -> Self {
        Self::Control(binding)
    }
}

/// Original owner/context retained by a private native continuation.
#[derive(Clone)]
pub struct OwnerContinuation(Binding);

impl fmt::Debug for OwnerContinuation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("OwnerContinuation")
            .field(&self.0.cause)
            .finish()
    }
}

impl OwnerContinuation {
    #[must_use]
    pub fn with_parent_event(mut self, event_id: UUID4) -> Self {
        self.0.cause.parent_event_id = Some(event_id);
        self
    }

    /// # Errors
    ///
    /// Refuses another owner or an exhausted queue.
    pub fn schedule(
        self,
        requires_control: bool,
        action: OwnerAction,
    ) -> Result<(), OwnerDeliveryError> {
        if is_controlled_phase() {
            return PHASE.with(|phase| {
                let mut phase = phase.borrow_mut();
                let phase = phase.as_mut().ok_or(OwnerDeliveryError::Unbound)?;
                let overflow = phase.continuations.len() >= CAPACITY;
                phase.continuations.push(Work {
                    binding: self.0.into(),
                    requires_control,
                    action,
                    ingress: None,
                    incoming: None,
                });
                if overflow {
                    record_failure(OwnerDeliveryError::Capacity);
                    return Err(OwnerDeliveryError::Capacity);
                }
                Ok(())
            });
        }
        current_owner()?.enqueue(
            Work {
                binding: self.0.into(),
                requires_control,
                action,
                ingress: None,
                incoming: None,
            },
            false,
        )
    }

    #[must_use]
    pub fn matches_current(&self) -> bool {
        PHASE.with(|phase| {
            phase.borrow().as_ref().is_some_and(|phase| {
                phase.binding.control().is_some_and(|binding| {
                    binding.cause.incarnation == self.0.cause.incarnation
                        && binding.cause.command_id == self.0.cause.command_id
                        && Rc::ptr_eq(&binding.context, &self.0.context)
                })
            })
        })
    }
}

struct Work {
    binding: PhaseBinding,
    requires_control: bool,
    action: OwnerAction,
    ingress: Option<TradingCommandMessage>,
    incoming: Option<OwnerIncomingOrder>,
}

struct External {
    binding: PhaseBinding,
    event_id: UUID4,
    action: Box<dyn OwnerExternalAction>,
    error: Option<OwnerDeliveryError>,
}

/// Immutable identity of an external item retained by a failed owner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RetainedOwnerDelivery {
    pub cause: OwnerCause,
    pub event_id: UUID4,
    pub event: OrderEventAny,
    pub error: Option<OwnerDeliveryError>,
}

/// A rejected original command, with no invented sequence for an unassigned item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RetainedOwnerCommand {
    pub cause: Option<OwnerCause>,
    pub endpoint: crate::msgbus::MStr<crate::msgbus::Endpoint>,
    pub command: TradingCommand,
    pub error: OwnerDeliveryError,
}

/// Original control command work retained by an owner, including after admission failure.
///
/// This record excludes native continuations without an ingress command and grants no retry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RetainedOwnerCommandWork {
    pub cause: OwnerCause,
    pub endpoint: crate::msgbus::MStr<crate::msgbus::Endpoint>,
    pub command: TradingCommand,
}

struct Phase {
    binding: PhaseBinding,
    external: VecDeque<External>,
    continuations: Vec<Work>,
    ingress: Option<TradingCommandMessage>,
    incoming: Option<OwnerIncomingOrder>,
}

struct Delivery {
    binding: PhaseBinding,
    event_id: UUID4,
    preceding_children: usize,
    continuations: Vec<Work>,
}

struct RetainedCommand {
    origin: Option<Binding>,
    _context: Option<Rc<dyn OwnerControlContext>>,
    message: TradingCommandMessage,
    error: OwnerDeliveryError,
}

struct State {
    incarnation: UUID4,
    owner: ThreadId,
    pid: u32,
    next_sequence: u64,
    queue: VecDeque<Work>,
    retained_external: VecDeque<External>,
    retained_commands: VecDeque<RetainedCommand>,
    retained_work: VecDeque<Work>,
    retained_incoming_orders: Vec<RetainedIncomingOrder>,
    failure: Option<OwnerDeliveryError>,
    draining: bool,
    retired: bool,
}

thread_local! {
    static OWNER: RefCell<Option<OwnerDelivery>> = const { RefCell::new(None) };
    static PHASE: RefCell<Option<Phase>> = const { RefCell::new(None) };
    static SUBMISSION: RefCell<Option<Rc<dyn OwnerControlContext>>> = const { RefCell::new(None) };
    static DELIVERY: RefCell<Option<Delivery>> = const { RefCell::new(None) };
}

/// The actual node strongly owns pending work until completion or explicit fault.
#[derive(Clone)]
pub struct OwnerDelivery(Rc<RefCell<State>>);

/// A non-owning observer cannot keep a strategy/owner reference cycle alive.
#[derive(Clone)]
pub struct OwnerObserver {
    owner: Weak<RefCell<State>>,
    thread: ThreadId,
    pid: u32,
}

impl fmt::Debug for OwnerObserver {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OwnerObserver")
            .field("thread", &self.thread)
            .field("pid", &self.pid)
            .finish_non_exhaustive()
    }
}

impl OwnerObserver {
    /// # Errors
    ///
    /// Refuses access from a different process or thread.
    pub fn retained_commands(&self) -> Result<Vec<RetainedOwnerCommand>, OwnerDeliveryError> {
        if self.thread != thread::current().id() || self.pid != std::process::id() {
            return Err(OwnerDeliveryError::WrongOwner);
        }
        Ok(self.owner.upgrade().map_or_else(Vec::new, |owner| {
            owner
                .borrow()
                .retained_commands
                .iter()
                .map(|item| RetainedOwnerCommand {
                    cause: item.origin.as_ref().map(|binding| binding.cause),
                    endpoint: item.message.endpoint(),
                    command: item.message.command().clone(),
                    error: item.error,
                })
                .collect()
        }))
    }

    /// Reads original control commands held in retained work, without changing their owner.
    ///
    /// Excludes queued work and native continuations without an ingress command. The owner
    /// status reports its failure separately; this does not assign an error to an unexecuted item.
    /// A record may follow admission rejection or phase failure and never proves authorization.
    ///
    /// # Errors
    ///
    /// Refuses access from a different process or thread.
    pub fn retained_command_work(
        &self,
    ) -> Result<Vec<RetainedOwnerCommandWork>, OwnerDeliveryError> {
        if self.thread != thread::current().id() || self.pid != std::process::id() {
            return Err(OwnerDeliveryError::WrongOwner);
        }
        Ok(self.owner.upgrade().map_or_else(Vec::new, |owner| {
            owner
                .borrow()
                .retained_work
                .iter()
                .filter_map(|work| {
                    let binding = work.binding.control()?;
                    let ingress = work.ingress.as_ref()?;
                    Some(RetainedOwnerCommandWork {
                        cause: binding.cause,
                        endpoint: ingress.endpoint(),
                        command: ingress.command().clone(),
                    })
                })
                .collect()
        }))
    }
    /// # Errors
    ///
    /// Refuses access from a different process or thread.
    pub fn retained_deliveries(&self) -> Result<Vec<RetainedOwnerDelivery>, OwnerDeliveryError> {
        if self.thread != thread::current().id() || self.pid != std::process::id() {
            return Err(OwnerDeliveryError::WrongOwner);
        }
        Ok(self.owner.upgrade().map_or_else(Vec::new, |owner| {
            owner
                .borrow()
                .retained_external
                .iter()
                .filter_map(|item| {
                    let binding = item.binding.control()?;
                    let OwnerExternalEvent::Order(event) = item.action.original_event() else {
                        return None;
                    };
                    Some(RetainedOwnerDelivery {
                        cause: binding.cause,
                        event_id: item.event_id,
                        event,
                        error: item.error,
                    })
                })
                .collect()
        }))
    }

    /// Reads every typed output and its original control or incoming cause.
    ///
    /// # Errors
    ///
    /// Refuses another process or owner thread.
    pub fn retained_typed_deliveries(
        &self,
    ) -> Result<Vec<RetainedOwnerTypedDelivery>, OwnerDeliveryError> {
        if self.thread != thread::current().id() || self.pid != std::process::id() {
            return Err(OwnerDeliveryError::WrongOwner);
        }
        Ok(self.owner.upgrade().map_or_else(Vec::new, |owner| {
            owner
                .borrow()
                .retained_external
                .iter()
                .map(|item| RetainedOwnerTypedDelivery {
                    cause: match &item.binding {
                        PhaseBinding::Control(binding) => {
                            OwnerDeliveryCause::Control(binding.cause)
                        }
                        PhaseBinding::Incoming(cause) => OwnerDeliveryCause::Incoming(*cause),
                    },
                    event_id: item.event_id,
                    event: item.action.original_event(),
                    error: item.error,
                })
                .collect()
        }))
    }

    /// Reads dispatched failures and original incoming items that have not been dispatched.
    ///
    /// # Errors
    ///
    /// Refuses another process or owner thread.
    pub fn retained_incoming_orders(
        &self,
    ) -> Result<Vec<RetainedIncomingOrder>, OwnerDeliveryError> {
        if self.thread != thread::current().id() || self.pid != std::process::id() {
            return Err(OwnerDeliveryError::WrongOwner);
        }
        Ok(self.owner.upgrade().map_or_else(Vec::new, |owner| {
            let state = owner.borrow();
            let mut result = state.retained_incoming_orders.clone();
            for work in state.queue.iter().chain(&state.retained_work) {
                if let PhaseBinding::Incoming(cause) = &work.binding
                    && let Some(event) = &work.incoming
                {
                    result.push(RetainedIncomingOrder {
                        cause: Some(*cause),
                        event: event.clone(),
                        stage: OwnerIncomingStage::NotDispatched,
                        error: state
                            .failure
                            .unwrap_or(OwnerDeliveryError::PendingAtRetirement),
                    });
                }
            }
            result
        }))
    }
    /// # Errors
    ///
    /// Refuses an observation from another process or owner thread.
    pub fn status(
        &self,
    ) -> Result<Option<(usize, Option<OwnerDeliveryError>)>, OwnerDeliveryError> {
        if self.thread != thread::current().id() || self.pid != std::process::id() {
            return Err(OwnerDeliveryError::WrongOwner);
        }
        Ok(self.owner.upgrade().map(|owner| {
            let state = owner.borrow();
            (
                state.queue.len()
                    + state.retained_external.len()
                    + state.retained_commands.len()
                    + state.retained_work.len()
                    + state.retained_incoming_orders.len(),
                state.failure,
            )
        }))
    }

    #[must_use]
    pub fn is_current(&self) -> bool {
        if self.thread != thread::current().id() || self.pid != std::process::id() {
            return false;
        }
        OWNER.with(|owner| {
            owner.borrow().as_ref().is_some_and(|owner| {
                self.owner
                    .upgrade()
                    .is_some_and(|observed| Rc::ptr_eq(&observed, &owner.0))
            })
        })
    }
}

impl fmt::Debug for OwnerDelivery {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = self.0.borrow();
        formatter
            .debug_struct("OwnerDelivery")
            .field("incarnation", &state.incarnation)
            .field("pending", &state.queue.len())
            .field("failure", &state.failure)
            .finish()
    }
}

impl Default for OwnerDelivery {
    fn default() -> Self {
        Self::new()
    }
}

impl OwnerDelivery {
    #[must_use]
    pub fn new() -> Self {
        Self(Rc::new(RefCell::new(State {
            incarnation: UUID4::new(),
            owner: thread::current().id(),
            pid: std::process::id(),
            next_sequence: 0,
            queue: VecDeque::new(),
            retained_external: VecDeque::new(),
            retained_commands: VecDeque::new(),
            retained_work: VecDeque::new(),
            retained_incoming_orders: Vec::new(),
            failure: None,
            draining: false,
            retired: false,
        })))
    }

    fn check(&self) -> Result<(), OwnerDeliveryError> {
        let state = self.0.borrow();
        if state.owner != thread::current().id() || state.pid != std::process::id() || state.retired
        {
            return Err(OwnerDeliveryError::WrongOwner);
        }
        Ok(())
    }

    /// Binds this concrete owner to the current dispatch thread.
    ///
    /// # Errors
    ///
    /// Refuses a retired owner or another active owner on the thread.
    pub fn bind(&self) -> Result<OwnerBinding, OwnerDeliveryError> {
        self.check()?;
        OWNER.with(|slot| {
            let previous = slot.borrow().clone();
            if previous
                .as_ref()
                .is_some_and(|owner| !Rc::ptr_eq(&owner.0, &self.0))
            {
                return Err(OwnerDeliveryError::WrongOwner);
            }
            *slot.borrow_mut() = Some(self.clone());
            Ok(OwnerBinding { previous })
        })
    }

    fn enqueue(&self, work: Work, front: bool) -> Result<(), OwnerDeliveryError> {
        if let Err(error) = self.check() {
            let mut state = self.0.borrow_mut();
            state.failure.get_or_insert(error);
            state.retained_work.push_back(work);
            return Err(error);
        }
        let mut state = self.0.borrow_mut();
        if work.binding.incarnation() != state.incarnation {
            state.retained_work.push_back(work);
            state.failure.get_or_insert(OwnerDeliveryError::WrongOwner);
            return Err(OwnerDeliveryError::WrongOwner);
        }
        let blocked = state.failure.or_else(|| {
            (state.queue.len()
                + state.retained_external.len()
                + state.retained_commands.len()
                + state.retained_work.len()
                + state.retained_incoming_orders.len()
                >= CAPACITY)
                .then_some(OwnerDeliveryError::Capacity)
        });
        if let Some(error) = blocked {
            state.failure.get_or_insert(error);
            state.retained_work.push_back(work);
            return Err(error);
        }
        if front {
            state.queue.push_front(work);
        } else {
            state.queue.push_back(work);
        }
        Ok(())
    }

    fn reject_command(
        &self,
        origin: Option<Binding>,
        context: Rc<dyn OwnerControlContext>,
        message: TradingCommandMessage,
        error: OwnerDeliveryError,
    ) -> OwnerDeliveryError {
        let mut state = self.0.borrow_mut();
        state.failure.get_or_insert(error);
        state.retained_commands.push_back(RetainedCommand {
            origin,
            _context: Some(context),
            message,
            error,
        });
        error
    }

    fn queue_continuations(&self, work: Vec<Work>) -> Result<(), OwnerDeliveryError> {
        let mut state = self.0.borrow_mut();
        let error = state.failure.or_else(|| {
            if work
                .iter()
                .any(|item| item.binding.incarnation() != state.incarnation)
            {
                Some(OwnerDeliveryError::WrongOwner)
            } else if state.queue.len()
                + state.retained_external.len()
                + state.retained_commands.len()
                + state.retained_work.len()
                + state.retained_incoming_orders.len()
                + work.len()
                > CAPACITY
            {
                Some(OwnerDeliveryError::Capacity)
            } else {
                None
            }
        });
        if let Some(error) = error {
            state.failure.get_or_insert(error);
            state.retained_work.extend(work);
            return Err(error);
        }
        for item in work.into_iter().rev() {
            state.queue.push_front(item);
        }
        Ok(())
    }

    fn cause(&self, command_id: UUID4) -> Result<OwnerCause, OwnerDeliveryError> {
        self.check()?;
        let mut state = self.0.borrow_mut();
        state.next_sequence = state
            .next_sequence
            .checked_add(1)
            .ok_or(OwnerDeliveryError::SequenceExhausted)?;
        let parent_event_id = active_parent_event_id();
        Ok(OwnerCause {
            command_id,
            parent_event_id,
            sequence: state.next_sequence,
            incarnation: state.incarnation,
        })
    }

    fn enqueue_incoming(&self, event: OwnerIncomingOrder) -> Result<(), OwnerDeliveryError> {
        if let Err(error) = self.check() {
            let mut state = self.0.borrow_mut();
            state.failure.get_or_insert(error);
            state.retained_incoming_orders.push(RetainedIncomingOrder {
                cause: None,
                event,
                stage: OwnerIncomingStage::NotDispatched,
                error,
            });
            return Err(error);
        }
        let cause = {
            let mut state = self.0.borrow_mut();
            let Some(sequence) = state.next_sequence.checked_add(1) else {
                state
                    .failure
                    .get_or_insert(OwnerDeliveryError::SequenceExhausted);
                state.retained_incoming_orders.push(RetainedIncomingOrder {
                    cause: None,
                    event,
                    stage: OwnerIncomingStage::NotDispatched,
                    error: OwnerDeliveryError::SequenceExhausted,
                });
                return Err(OwnerDeliveryError::SequenceExhausted);
            };
            state.next_sequence = sequence;
            OwnerIncomingCause {
                event_id: event.event_id(),
                sequence,
                incarnation: state.incarnation,
            }
        };
        let original = event.clone();
        self.enqueue(
            Work {
                binding: PhaseBinding::Incoming(cause),
                requires_control: false,
                action: Box::new(move || {
                    event.dispatch();
                    Ok(())
                }),
                ingress: None,
                incoming: Some(original),
            },
            false,
        )
    }

    fn retain_incoming_phase(
        &self,
        phase: &mut Phase,
        stage: OwnerIncomingStage,
        error: OwnerDeliveryError,
    ) {
        if let PhaseBinding::Incoming(cause) = &phase.binding
            && let Some(event) = phase.incoming.take()
        {
            self.0
                .borrow_mut()
                .retained_incoming_orders
                .push(RetainedIncomingOrder {
                    cause: Some(*cause),
                    event,
                    stage,
                    error,
                });
        }
    }

    /// Finishes one phase before calling external code or taking another phase.
    ///
    /// # Errors
    ///
    /// Retains remaining work on any control, callback, or native failure.
    pub fn drain(&self) -> Result<(), OwnerDeliveryError> {
        self.check()?;
        {
            let mut state = self.0.borrow_mut();
            if let Some(error) = state.failure {
                return Err(error);
            }
            if state.draining {
                return Err(OwnerDeliveryError::ReentrantDrain);
            }
            state.draining = true;
        }
        let result = self.drain_inner();
        let mut state = self.0.borrow_mut();
        state.draining = false;
        if let Err(error) = result {
            state.failure.get_or_insert(error);
        }
        result
    }

    fn drain_inner(&self) -> Result<(), OwnerDeliveryError> {
        loop {
            if let Some(error) = self.0.borrow().failure {
                return Err(error);
            }
            let Some(mut work) = self.0.borrow_mut().queue.pop_front() else {
                return Ok(());
            };
            if work.binding.incarnation() != self.0.borrow().incarnation {
                self.0.borrow_mut().queue.push_front(work);
                return Err(OwnerDeliveryError::WrongOwner);
            }
            let mut entered = if work.requires_control {
                let Some(binding) = work.binding.control() else {
                    self.0.borrow_mut().queue.push_front(work);
                    return Err(OwnerDeliveryError::UnsupportedEffect);
                };
                match binding.context.enter() {
                    Ok(entered) => Some(entered),
                    Err(error) => {
                        self.0.borrow_mut().queue.push_front(work);
                        return Err(error);
                    }
                }
            } else {
                None
            };
            let installed = PHASE.with(|phase| {
                let mut phase = phase.borrow_mut();
                if phase.is_some() {
                    return false;
                }
                *phase = Some(Phase {
                    binding: work.binding.clone(),
                    external: VecDeque::new(),
                    continuations: Vec::new(),
                    ingress: work.ingress.take(),
                    incoming: work.incoming.take(),
                });
                true
            });
            if !installed {
                self.0.borrow_mut().queue.push_front(work);
                let exit = entered.as_mut().map_or(Ok(()), |scope| scope.finish(true));
                drop(entered);
                exit?;
                return Err(OwnerDeliveryError::ReentrantDrain);
            }
            let native = catch_unwind(AssertUnwindSafe(work.action))
                .unwrap_or(Err(OwnerDeliveryError::NativePanicked));
            let incoming_stage = if native.is_ok() {
                OwnerIncomingStage::DispatchReturned
            } else {
                OwnerIncomingStage::DispatchEntered
            };
            let mut phase =
                PHASE.with(|phase| phase.borrow_mut().take().expect("active owner phase"));
            let exit = entered.as_mut().map_or(Ok(()), |scope| {
                scope.finish(native.is_err() || self.failure().is_some())
            });
            drop(entered);
            let phase_failure = exit.err().or_else(|| {
                self.failure()
                    .filter(|error| *error == OwnerDeliveryError::Capacity)
            });
            if let Some(error) = phase_failure {
                self.retain_incoming_phase(&mut phase, incoming_stage, error);
                self.0
                    .borrow_mut()
                    .retained_external
                    .append(&mut phase.external);
                self.0
                    .borrow_mut()
                    .retained_work
                    .extend(phase.continuations);
                return Err(error);
            }
            let mut delivery_error = None;
            let mut callback_children = Vec::new();
            while let Some(mut external) = phase.external.pop_front() {
                let mut binding = external.binding.clone();
                if let PhaseBinding::Control(binding) = &mut binding {
                    binding.cause.parent_event_id = Some(external.event_id);
                }
                external.binding = binding.clone();
                DELIVERY.with(|delivery| {
                    *delivery.borrow_mut() = Some(Delivery {
                        binding,
                        event_id: external.event_id,
                        preceding_children: phase
                            .continuations
                            .len()
                            .saturating_add(callback_children.len()),
                        continuations: Vec::new(),
                    });
                });
                let delivered = catch_unwind(AssertUnwindSafe(|| external.action.deliver()))
                    .unwrap_or(Err(OwnerDeliveryError::ExternalCallback));
                let delivery = DELIVERY.with(|delivery| {
                    delivery
                        .borrow_mut()
                        .take()
                        .expect("active external delivery")
                });
                callback_children.extend(delivery.continuations);
                if let Err(error) = delivered {
                    // A failed specific callback must not erase itself or suppress its aggregate.
                    external.error = Some(error);
                    self.0.borrow_mut().retained_external.push_back(external);
                    delivery_error.get_or_insert(error);
                }
            }
            // Pending native work precedes newly emitted callback children, as in the original drain
            if let Some(error) = delivery_error {
                self.retain_incoming_phase(&mut phase, incoming_stage, error);
                self.0
                    .borrow_mut()
                    .retained_work
                    .extend(phase.continuations);
                self.0.borrow_mut().retained_work.extend(callback_children);
                return Err(error);
            }
            if let Err(error) = native {
                self.retain_incoming_phase(&mut phase, incoming_stage, error);
                self.0
                    .borrow_mut()
                    .retained_work
                    .extend(phase.continuations);
                self.0.borrow_mut().retained_work.extend(callback_children);
                return Err(error);
            }
            phase.continuations.extend(callback_children);
            if let Err(error) = self.queue_continuations(std::mem::take(&mut phase.continuations)) {
                self.retain_incoming_phase(&mut phase, incoming_stage, error);
                return Err(error);
            }
        }
    }

    #[must_use]
    pub fn pending(&self) -> usize {
        let state = self.0.borrow();
        state.queue.len()
            + state.retained_external.len()
            + state.retained_commands.len()
            + state.retained_work.len()
            + state.retained_incoming_orders.len()
    }

    #[must_use]
    pub fn failure(&self) -> Option<OwnerDeliveryError> {
        self.0.borrow().failure
    }

    /// Marks a completely reaped owner as retired.
    ///
    /// # Errors
    ///
    /// Refuses to erase pending work or a failed delivery.
    pub fn retire(&self) -> Result<(), OwnerDeliveryError> {
        self.check()?;
        let mut state = self.0.borrow_mut();
        if state.draining
            || !state.queue.is_empty()
            || !state.retained_external.is_empty()
            || !state.retained_commands.is_empty()
            || !state.retained_work.is_empty()
            || !state.retained_incoming_orders.is_empty()
        {
            return Err(OwnerDeliveryError::PendingAtRetirement);
        }
        state.retired = true;
        state.failure.map_or(Ok(()), Err)
    }
}

#[derive(Debug)]
pub struct OwnerBinding {
    previous: Option<OwnerDelivery>,
}

impl Drop for OwnerBinding {
    fn drop(&mut self) {
        OWNER.with(|owner| *owner.borrow_mut() = self.previous.take());
    }
}

/// Keeps the actual current control factory while a typed command is built.
#[derive(Debug)]
pub struct SubmissionBinding {
    previous: Option<Rc<dyn OwnerControlContext>>,
}

impl SubmissionBinding {
    /// # Errors
    ///
    /// Requires an actual bound node owner.
    pub fn new(context: Rc<dyn OwnerControlContext>) -> Result<Self, OwnerDeliveryError> {
        current_owner()?.check()?;
        let previous = SUBMISSION.with(|slot| slot.borrow_mut().replace(context));
        Ok(Self { previous })
    }
}

impl Drop for SubmissionBinding {
    fn drop(&mut self) {
        SUBMISSION.with(|slot| *slot.borrow_mut() = self.previous.take());
    }
}

fn current_owner() -> Result<OwnerDelivery, OwnerDeliveryError> {
    OWNER.with(|owner| owner.borrow().clone().ok_or(OwnerDeliveryError::Unbound))
}

#[must_use]
pub fn is_controlled_phase() -> bool {
    PHASE.with(|phase| {
        phase
            .borrow()
            .as_ref()
            .is_some_and(|phase| phase.binding.control().is_some())
    })
}

/// An output phase may have an incoming identity without any control context.
#[must_use]
pub fn has_delivery_phase() -> bool {
    PHASE.with(|phase| phase.borrow().is_some())
}

#[must_use]
pub fn current_continuation() -> Option<OwnerContinuation> {
    PHASE.with(|phase| {
        phase
            .borrow()
            .as_ref()
            .and_then(|phase| phase.binding.control().cloned().map(OwnerContinuation))
    })
}

/// Dispatches each original order under one canonical output phase.
///
/// All remaining members belong to this owner before the first is dispatched.
/// A failure retains them, while admitted callback children precede the next member.
///
/// # Errors
///
/// Reports an owner, admission, dispatch or delivery failure with original items retained.
pub fn dispatch_incoming_orders(events: Vec<OwnerIncomingOrder>) -> Result<(), OwnerDeliveryError> {
    if is_controlled_phase() {
        // This native work already belongs to an actual scope; only that scope's exit can release it.
        for event in events {
            event.dispatch();
        }
        return Ok(());
    }
    let Some(owner) = OWNER.with(|slot| slot.borrow().clone()) else {
        for event in events {
            event.dispatch();
        }
        return Ok(());
    };
    let mut rejected = None;
    for event in events {
        if let Err(error) = owner.enqueue_incoming(event) {
            rejected.get_or_insert(error);
        }
    }
    if let Some(error) = rejected {
        record_failure(error);
        return Err(error);
    }
    drain_current()
}

/// Keeps an original command instead of executing it after this owner has failed.
///
/// No context or sequence is invented for a command received outside a control phase.
#[must_use]
pub fn retain_undispatched_command(
    message: TradingCommandMessage,
) -> Option<TradingCommandMessage> {
    let Some(owner) = OWNER.with(|slot| slot.borrow().clone()) else {
        return Some(message);
    };
    let Some(error) = owner.check().err().or_else(|| owner.failure()) else {
        return Some(message);
    };
    let origin = active_binding();
    let context = origin.as_ref().map(|binding| binding.context.clone());
    let mut state = owner.0.borrow_mut();
    state.failure.get_or_insert(error);
    state.retained_commands.push_back(RetainedCommand {
        origin,
        _context: context,
        message,
        error,
    });
    None
}

/// Captures only classified external Python callbacks during a real phase.
///
/// # Errors
///
/// Refuses capacity exhaustion without dispatching the callback under a writer.
pub fn capture_external(
    event_id: UUID4,
    action: Box<dyn OwnerExternalAction>,
) -> Result<Option<Box<dyn OwnerExternalAction>>, OwnerDeliveryError> {
    PHASE.with(|phase| {
        let mut phase = phase.borrow_mut();
        let Some(phase) = phase.as_mut() else {
            return Ok(Some(action));
        };
        let overflow = phase.external.len() >= CAPACITY;
        phase.external.push_back(External {
            binding: phase.binding.clone(),
            event_id,
            action,
            error: overflow.then_some(OwnerDeliveryError::Capacity),
        });
        if overflow {
            // A void native trait may discard PyResult. Retain the already-produced phase output,
            // latch the actual owner, and stop all following work at the owner safe point.
            record_failure(OwnerDeliveryError::Capacity);
            return Err(OwnerDeliveryError::Capacity);
        }
        Ok(None)
    })
}

/// Defers a real typed control command; no submit effect is admitted by this bridge.
///
/// # Errors
///
/// Refuses unbound owners, unsupported effects or exhausted queues.
pub fn capture_control_command(
    message: TradingCommandMessage,
) -> Result<Option<TradingCommandMessage>, OwnerDeliveryError> {
    let origin = active_binding();
    let context = origin
        .as_ref()
        .map(|binding| binding.context.clone())
        .or_else(|| SUBMISSION.with(|submission| submission.borrow().clone()));
    let Some(context) = context else {
        return Ok(Some(message));
    };
    let command_id = match message.command() {
        TradingCommand::CancelOrder(command) => command.command_id,
        TradingCommand::QueryAccount(command) => command.command_id,
        TradingCommand::QueryOrder(command) => command.command_id,
        _ => {
            let owner = current_owner()?;
            return Err(owner.reject_command(
                origin,
                context,
                message,
                OwnerDeliveryError::UnsupportedEffect,
            ));
        }
    };
    let owner = current_owner()?;
    let cause = match owner.cause(command_id) {
        Ok(cause) => cause,
        Err(error) => return Err(owner.reject_command(origin, context, message, error)),
    };
    let binding = Binding { context, cause };
    let ingress = TradingCommandMessage::new(message.endpoint(), message.command().clone());
    queue_callback_child(
        &owner,
        Work {
            binding: binding.into(),
            requires_control: true,
            ingress: Some(ingress),
            incoming: None,
            action: Box::new(move || {
                let children = message.dispatch();
                if !children.is_empty() {
                    return Err(OwnerDeliveryError::UnsupportedEffect);
                }
                Ok(())
            }),
        },
    )?;
    Ok(None)
}

/// Classifies the actual registered endpoint, before either borrowing or queuing its owner.
///
/// # Errors
///
/// Retains a raw unsupported command instead of treating current control as opening permission.
pub fn capture_registered_control_command(
    message: TradingCommandMessage,
) -> Result<Option<TradingCommandMessage>, OwnerDeliveryError> {
    let binding = active_binding();
    let Some(binding) = binding else {
        return Ok(Some(message));
    };
    if !is_control_command(message.command()) {
        return Err(current_owner()?.reject_command(
            Some(binding.clone()),
            binding.context,
            message,
            OwnerDeliveryError::UnsupportedEffect,
        ));
    }
    let original = PHASE.with(|phase| {
        let mut phase = phase.borrow_mut();
        let Some(phase) = phase.as_mut() else {
            return false;
        };
        if phase.ingress.as_ref().is_some_and(|original| {
            original.endpoint() == message.endpoint() && original.command() == message.command()
        }) {
            phase.ingress.take();
            true
        } else {
            false
        }
    });
    if original {
        return Ok(Some(message));
    }
    let owner = current_owner()?;
    let command_id = match message.command() {
        TradingCommand::CancelOrder(command) => command.command_id,
        TradingCommand::QueryAccount(command) => command.command_id,
        TradingCommand::QueryOrder(command) => command.command_id,
        _ => unreachable!("closed command classification above"),
    };
    let mut cause = match owner.cause(command_id) {
        Ok(cause) => cause,
        Err(error) => {
            return Err(owner.reject_command(
                Some(binding.clone()),
                binding.context,
                message,
                error,
            ));
        }
    };
    cause.parent_event_id = binding.cause.parent_event_id;
    let ingress = TradingCommandMessage::new(message.endpoint(), message.command().clone());
    owner.enqueue(
        Work {
            binding: Binding {
                context: binding.context,
                cause,
            }
            .into(),
            requires_control: true,
            ingress: Some(ingress),
            incoming: None,
            action: Box::new(move || {
                let children = message.dispatch();
                if !children.is_empty() {
                    return Err(OwnerDeliveryError::UnsupportedEffect);
                }
                Ok(())
            }),
        },
        false,
    )?;
    Ok(None)
}

#[must_use]
pub const fn is_control_command(command: &TradingCommand) -> bool {
    matches!(
        command,
        TradingCommand::CancelOrder(_)
            | TradingCommand::QueryAccount(_)
            | TradingCommand::QueryOrder(_)
    )
}

fn active_binding() -> Option<Binding> {
    PHASE
        .with(|phase| {
            phase
                .borrow()
                .as_ref()
                .and_then(|phase| phase.binding.control().cloned())
        })
        .or_else(|| {
            DELIVERY.with(|delivery| {
                delivery
                    .borrow()
                    .as_ref()
                    .and_then(|delivery| delivery.binding.control().cloned())
            })
        })
}

fn active_parent_event_id() -> Option<UUID4> {
    DELIVERY
        .with(|delivery| delivery.borrow().as_ref().map(|delivery| delivery.event_id))
        .or_else(|| {
            PHASE.with(|phase| {
                phase
                    .borrow()
                    .as_ref()
                    .and_then(|phase| match &phase.binding {
                        PhaseBinding::Control(binding) => binding.cause.parent_event_id,
                        PhaseBinding::Incoming(cause) => Some(cause.event_id),
                    })
            })
        })
}

fn queue_callback_child(owner: &OwnerDelivery, work: Work) -> Result<(), OwnerDeliveryError> {
    let mut original = Some(work);
    let handled = DELIVERY.with(|delivery| {
        let mut delivery = delivery.borrow_mut();
        let delivery = delivery
            .as_mut()
            .filter(|delivery| matches!(delivery.binding, PhaseBinding::Incoming(_)))?;
        let work = original.take().expect("owned command before admission");
        let invalid_owner = owner.check().err();
        let mut state = owner.0.borrow_mut();
        let error = state.failure.or(invalid_owner).or_else(|| {
            if work.binding.incarnation() == state.incarnation {
                let pending = state.queue.len()
                    + state.retained_external.len()
                    + state.retained_commands.len()
                    + state.retained_work.len()
                    + state.retained_incoming_orders.len();
                (pending
                    .saturating_add(delivery.preceding_children)
                    .saturating_add(delivery.continuations.len())
                    >= CAPACITY)
                    .then_some(OwnerDeliveryError::Capacity)
            } else {
                Some(OwnerDeliveryError::WrongOwner)
            }
        });
        // These are concrete factory-bound command children, not permission inherited from ingress.
        // An already-requested rejected child remains owned alongside the accepted prefix.
        delivery.continuations.push(work);
        Some(match error {
            Some(error) => {
                state.failure.get_or_insert(error);
                Err(error)
            }
            None => Ok(()),
        })
    });
    match handled {
        Some(result) => result,
        None => owner.enqueue(original.expect("unclaimed original command"), false),
    }
}

/// Runs only at an owner safe point after engine/actor borrows have ended.
///
/// # Errors
///
/// Returns the retained owner failure instead of silently discarding work.
pub fn drain_current() -> Result<(), OwnerDeliveryError> {
    OWNER
        .with(|owner| owner.borrow().clone())
        .map_or(Ok(()), |owner| {
            if owner.0.borrow().draining {
                return Ok(());
            }
            owner.drain()
        })
}

/// Retains a delivery failure for the actual node to report after native cleanup.
pub fn record_failure(error: OwnerDeliveryError) {
    OWNER.with(|owner| {
        if let Some(owner) = owner.borrow().as_ref() {
            owner.0.borrow_mut().failure.get_or_insert(error);
        }
    });
}

/// Reports the current concrete owner without creating or authorizing one.
#[must_use]
pub fn current_status() -> Option<(usize, Option<OwnerDeliveryError>)> {
    OWNER.with(|owner| {
        owner
            .borrow()
            .as_ref()
            .map(|owner| (owner.pending(), owner.failure()))
    })
}

#[must_use]
pub fn current_observer() -> Option<OwnerObserver> {
    OWNER.with(|owner| {
        owner.borrow().as_ref().map(|owner| OwnerObserver {
            owner: Rc::downgrade(&owner.0),
            thread: thread::current().id(),
            pid: std::process::id(),
        })
    })
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use nautilus_model::identifiers::{AccountId, TraderId};

    use super::*;
    use crate::messages::execution::QueryAccount;

    #[derive(Debug)]
    struct NeutralQueryContext;

    struct EnteredNeutralQuery;

    impl OwnerControlContext for NeutralQueryContext {
        fn enter(&self) -> Result<Box<dyn EnteredOwnerControl>, OwnerDeliveryError> {
            Ok(Box::new(EnteredNeutralQuery))
        }
    }

    impl EnteredOwnerControl for EnteredNeutralQuery {
        fn finish(&mut self, _failed: bool) -> Result<(), OwnerDeliveryError> {
            Ok(())
        }
    }

    #[rstest::rstest]
    fn test_queue_capacity_retains_the_original_rejected_query() {
        let owner = OwnerDelivery::new();
        let _binding = owner.bind().unwrap();
        let _submission = SubmissionBinding::new(Rc::new(NeutralQueryContext)).unwrap();
        let effects = Rc::new(Cell::new(0));
        crate::msgbus::register_trading_command_endpoint(
            "OwnerDelivery.queryCapacity".into(),
            crate::msgbus::TypedIntoHandler::from({
                let effects = effects.clone();
                move |_command: TradingCommand| effects.set(effects.get() + 1)
            }),
        );
        let mut last_query = None;
        for index in 0..=CAPACITY {
            let query = TradingCommand::QueryAccount(QueryAccount::new(
                TraderId::from("QUERY-001"),
                None,
                AccountId::from("SIM-001"),
                UUID4::new(),
                1.into(),
                None,
                None,
            ));
            last_query = Some(query.clone());
            let result = capture_control_command(TradingCommandMessage::new(
                "OwnerDelivery.queryCapacity".into(),
                query,
            ));
            if index < CAPACITY {
                assert!(result.unwrap().is_none());
            } else {
                assert!(matches!(result, Err(OwnerDeliveryError::Capacity)));
            }
        }
        assert_eq!(
            owner.pending(),
            CAPACITY + 1,
            "the capacity error must retain the rejected original command"
        );
        {
            let state = owner.0.borrow();
            assert_eq!(state.queue.len(), CAPACITY);
            assert_eq!(state.retained_work.len(), 1);
            let last = state.retained_work.back().unwrap();
            assert_eq!(
                last.ingress.as_ref().unwrap().command(),
                last_query.as_ref().unwrap()
            );
            assert_eq!(
                last.binding.control().unwrap().cause.sequence,
                (CAPACITY + 1) as u64
            );
            assert_eq!(
                last.binding.control().unwrap().cause.incarnation,
                state.incarnation
            );
        }
        let records = current_observer().unwrap().retained_command_work().unwrap();
        assert_eq!(
            records.len(),
            1,
            "queued commands are not retained-work records"
        );
        assert_eq!(records[0].command, last_query.unwrap());
        assert_eq!(records[0].endpoint, "OwnerDelivery.queryCapacity".into());
        assert_eq!(records[0].cause.sequence, (CAPACITY + 1) as u64);
        assert_eq!(records[0].cause.incarnation, owner.0.borrow().incarnation);
        assert_eq!(owner.drain(), Err(OwnerDeliveryError::Capacity));
        assert_eq!(
            effects.get(),
            0,
            "diagnosing a rejected work item does not admit any effect"
        );
        assert_eq!(owner.retire(), Err(OwnerDeliveryError::PendingAtRetirement));
    }

    struct QueryChildDelivery {
        event: OrderEventAny,
        command: TradingCommand,
    }

    impl OwnerExternalAction for QueryChildDelivery {
        fn original_event(&self) -> OwnerExternalEvent {
            OwnerExternalEvent::Order(self.event.clone())
        }
        fn deliver(&mut self) -> Result<(), OwnerDeliveryError> {
            let immediate = capture_control_command(TradingCommandMessage::new(
                "OwnerDelivery.groupCapacity".into(),
                self.command.clone(),
            ))?;
            if immediate.is_some() {
                return Err(OwnerDeliveryError::UnsupportedEffect);
            }
            Ok(())
        }
    }

    fn neutral_query(command_id: UUID4) -> TradingCommand {
        TradingCommand::QueryAccount(QueryAccount::new(
            TraderId::from("QUERY-001"),
            None,
            AccountId::from("SIM-001"),
            command_id,
            1.into(),
            None,
            None,
        ))
    }

    struct FailingCommandWorkDelivery {
        event: OrderEventAny,
        endpoint: crate::msgbus::MStr<crate::msgbus::Endpoint>,
        command: TradingCommand,
        context: Rc<dyn OwnerControlContext>,
    }

    impl OwnerExternalAction for FailingCommandWorkDelivery {
        fn original_event(&self) -> OwnerExternalEvent {
            OwnerExternalEvent::Order(self.event.clone())
        }

        fn deliver(&mut self) -> Result<(), OwnerDeliveryError> {
            let _submission = SubmissionBinding::new(self.context.clone())?;
            let immediate = capture_control_command(TradingCommandMessage::new(
                self.endpoint,
                self.command.clone(),
            ))?;
            if immediate.is_some() {
                return Err(OwnerDeliveryError::UnsupportedEffect);
            }
            Err(OwnerDeliveryError::ExternalCallback)
        }
    }

    #[rstest::rstest]
    fn test_retained_command_work_preserves_original_and_context_without_effects() {
        use crate::msgbus::{self, Endpoint, MStr, TypedIntoHandler};

        let owner = OwnerDelivery::new();
        let binding = owner.bind().unwrap();
        let observer = current_observer().unwrap();
        let context: Rc<dyn OwnerControlContext> = Rc::new(NeutralQueryContext);
        let weak_context = Rc::downgrade(&context);
        let root_endpoint = msgbus::MessagingSwitchboard::exec_engine_process();
        let child_endpoint: MStr<Endpoint> = "OwnerDelivery.retainedWorkChild".into();
        let command = neutral_query(UUID4::new());
        let event = nautilus_model::events::OrderCanceled {
            event_id: UUID4::new(),
            ..Default::default()
        };
        let parent_event_id = event.event_id;
        let event = OrderEventAny::Canceled(event);
        let root_effects = Rc::new(Cell::new(0));
        let later_effects = Rc::new(Cell::new(0));
        msgbus::register_trading_command_endpoint(
            child_endpoint,
            TypedIntoHandler::from({
                let effects = later_effects.clone();
                move |_command: TradingCommand| effects.set(effects.get() + 1)
            }),
        );
        msgbus::register_order_event_endpoint(
            root_endpoint,
            TypedIntoHandler::from({
                let effects = root_effects.clone();
                let command = command.clone();
                let context = context.clone();
                move |event: OrderEventAny| {
                    effects.set(effects.get() + 1);
                    assert!(
                        capture_external(
                            parent_event_id,
                            Box::new(FailingCommandWorkDelivery {
                                event,
                                endpoint: child_endpoint,
                                command: command.clone(),
                                context: context.clone(),
                            })
                        )
                        .unwrap()
                        .is_none()
                    );
                }
            }),
        );
        assert_eq!(
            dispatch_incoming_orders(vec![OwnerIncomingOrder::Process(event)]),
            Err(OwnerDeliveryError::ExternalCallback)
        );
        assert_eq!(root_effects.get(), 1);
        assert_eq!(later_effects.get(), 0);
        let before = (owner.pending(), owner.failure(), Rc::strong_count(&context));
        let records = observer.retained_command_work().unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].command, command);
        assert_eq!(records[0].endpoint, child_endpoint);
        let TradingCommand::QueryAccount(query) = &command else {
            unreachable!()
        };
        assert_eq!(records[0].cause.command_id, query.command_id);
        assert_eq!(records[0].cause.parent_event_id, Some(parent_event_id));
        assert_eq!(records[0].cause.sequence, 2);
        assert_eq!(records[0].cause.incarnation, owner.0.borrow().incarnation);
        {
            let state = owner.0.borrow();
            assert_eq!(state.retained_work.len(), 1);
            let original = state
                .retained_work
                .iter()
                .find(|work| work.ingress.is_some())
                .unwrap();
            assert_eq!(original.ingress.as_ref().unwrap().command(), &command);
            assert_eq!(
                original.ingress.as_ref().unwrap().endpoint(),
                child_endpoint
            );
            assert!(Rc::ptr_eq(
                &original.binding.control().unwrap().context,
                &context
            ));
        }
        assert!(
            observer.retained_commands().unwrap().is_empty(),
            "the rejected-command getter keeps its old meaning"
        );
        drop(binding);
        assert_eq!(
            observer.retained_command_work().unwrap(),
            records,
            "observation does not require an ambient binding"
        );
        assert_eq!(
            (owner.pending(), owner.failure(), Rc::strong_count(&context)),
            before
        );
        assert_eq!(owner.retire(), Err(OwnerDeliveryError::PendingAtRetirement));
        assert_eq!(later_effects.get(), 0);
        drop(context);
        assert!(weak_context.upgrade().is_some());
        *msgbus::get_message_bus().borrow_mut() = msgbus::MessageBus::default();
        assert!(
            weak_context.upgrade().is_some(),
            "the owner keeps its retained contexts alive"
        );
        drop(owner);
        assert!(observer.retained_command_work().unwrap().is_empty());
        assert!(weak_context.upgrade().is_none());
    }

    #[rstest::rstest]
    fn test_retained_command_work_checks_owner_thread_pid_and_weak_lifetime() {
        let owner = OwnerDelivery::new();
        let binding = owner.bind().unwrap();
        let observer = current_observer().unwrap();
        let mut wrong_thread = observer.clone();
        wrong_thread.thread = std::thread::spawn(|| std::thread::current().id())
            .join()
            .unwrap();
        assert_eq!(
            wrong_thread.retained_command_work(),
            Err(OwnerDeliveryError::WrongOwner)
        );
        let mut wrong_pid = observer.clone();
        wrong_pid.pid = observer.pid.wrapping_add(1);
        assert_eq!(
            wrong_pid.retained_command_work(),
            Err(OwnerDeliveryError::WrongOwner)
        );
        drop(binding);
        assert!(observer.retained_command_work().unwrap().is_empty());
        drop(owner);
        assert!(observer.retained_command_work().unwrap().is_empty());
    }

    type ChildAdmission = (
        UUID4,
        Result<bool, OwnerDeliveryError>,
        Option<OwnerDeliveryError>,
    );

    struct IncomingChildDelivery {
        event: OrderEventAny,
        endpoint: crate::msgbus::MStr<crate::msgbus::Endpoint>,
        commands: Rc<[TradingCommand]>,
        range: std::ops::Range<usize>,
        context: Rc<dyn OwnerControlContext>,
        admissions: Rc<RefCell<Vec<ChildAdmission>>>,
        outlets: Rc<RefCell<Vec<&'static str>>>,
        outlet: &'static str,
    }

    impl OwnerExternalAction for IncomingChildDelivery {
        fn original_event(&self) -> OwnerExternalEvent {
            OwnerExternalEvent::Order(self.event.clone())
        }

        fn deliver(&mut self) -> Result<(), OwnerDeliveryError> {
            self.outlets.borrow_mut().push(self.outlet);
            let _submission = SubmissionBinding::new(self.context.clone())?;
            for index in self.range.clone() {
                let command = self.commands[index].clone();
                let TradingCommand::QueryAccount(query) = &command else {
                    return Err(OwnerDeliveryError::UnsupportedEffect);
                };
                let command_id = query.command_id;
                let accepted =
                    capture_control_command(TradingCommandMessage::new(self.endpoint, command))
                        .map(|immediate| immediate.is_none());
                let failure = current_status().unwrap().1;
                self.admissions
                    .borrow_mut()
                    .push((command_id, accepted, failure));
            }
            // A caller that handles a refusal still cannot clear the concrete owner fault.
            Ok(())
        }
    }

    #[rstest::rstest]
    fn test_incoming_callback_children_enforce_shared_capacity_before_next_leg() {
        use crate::msgbus::{self, Endpoint, MStr, TypedIntoHandler};

        let owner = OwnerDelivery::new();
        let _binding = owner.bind().unwrap();
        let observer = current_observer().unwrap();
        let native = Rc::new(RefCell::new(Vec::new()));
        let native_children = Rc::new(Cell::new(0));
        let admissions = Rc::new(RefCell::new(Vec::new()));
        let outlets = Rc::new(RefCell::new(Vec::new()));
        let context: Rc<dyn OwnerControlContext> = Rc::new(NeutralQueryContext);
        let commands: Rc<[TradingCommand]> = (0..=CAPACITY)
            .map(|_| neutral_query(UUID4::new()))
            .collect::<Vec<_>>()
            .into();
        let child_endpoint: MStr<Endpoint> = "OwnerDelivery.incomingCapacityChild".into();
        let child_calls = native_children.clone();
        msgbus::register_trading_command_endpoint(
            child_endpoint,
            TypedIntoHandler::from(move |_command: TradingCommand| {
                child_calls.set(child_calls.get() + 1);
            }),
        );
        let first = nautilus_model::events::OrderSubmitted {
            event_id: UUID4::new(),
            client_order_id: "INCOMING-CAPACITY-A".into(),
            ..Default::default()
        };
        let second = nautilus_model::events::OrderSubmitted {
            event_id: UUID4::new(),
            client_order_id: "INCOMING-CAPACITY-B".into(),
            ..first
        };
        let event = OrderEventAny::Submitted(first);
        let expected_first = event.clone();
        let expected_second = OrderEventAny::Submitted(second);
        let expected_event_id = first.event_id;
        let native_for_handler = native.clone();
        let admissions_for_handler = admissions.clone();
        let outlets_for_handler = outlets.clone();
        let commands_for_handler = commands.clone();
        let context_for_handler = context.clone();
        msgbus::register_order_event_endpoint(
            crate::msgbus::MessagingSwitchboard::exec_engine_process(),
            TypedIntoHandler::from(move |received: OrderEventAny| {
                native_for_handler.borrow_mut().push(received.clone());
                if received == expected_first {
                    for (outlet, range) in [
                        ("specific", 0..CAPACITY / 2),
                        ("aggregate", CAPACITY / 2..CAPACITY + 1),
                    ] {
                        assert!(
                            capture_external(
                                expected_event_id,
                                Box::new(IncomingChildDelivery {
                                    event: received.clone(),
                                    endpoint: child_endpoint,
                                    commands: commands_for_handler.clone(),
                                    range,
                                    context: context_for_handler.clone(),
                                    admissions: admissions_for_handler.clone(),
                                    outlets: outlets_for_handler.clone(),
                                    outlet,
                                })
                            )
                            .unwrap()
                            .is_none()
                        );
                    }
                }
            }),
        );
        assert_eq!(
            dispatch_incoming_orders(vec![
                OwnerIncomingOrder::Process(event.clone()),
                OwnerIncomingOrder::Process(expected_second.clone()),
            ]),
            Err(OwnerDeliveryError::Capacity)
        );
        assert_eq!(&*outlets.borrow(), &["specific", "aggregate"]);
        assert_eq!(&*native.borrow(), &[event]);
        assert_eq!(native_children.get(), 0);
        let admissions = admissions.borrow();
        assert_eq!(admissions.len(), CAPACITY + 1);
        let first_refused = admissions.iter().position(|(_, result, _)| result.is_err());
        assert_eq!(
            first_refused,
            Some(CAPACITY - 1),
            "the queued second leg and children from both outlets share the admission budget"
        );
        for (index, (_, result, failure)) in admissions.iter().enumerate() {
            if index < CAPACITY - 1 {
                assert_eq!(*result, Ok(true));
                assert_eq!(*failure, None);
            } else {
                assert_eq!(*result, Err(OwnerDeliveryError::Capacity));
                assert_eq!(*failure, Some(OwnerDeliveryError::Capacity));
            }
        }
        let state = owner.0.borrow();
        assert_eq!(state.queue.len(), 1);
        assert_eq!(state.retained_work.len(), CAPACITY + 1);
        assert_eq!(
            state.queue.front().unwrap().incoming.as_ref().map(|input| {
                match input {
                    OwnerIncomingOrder::Process(event) => event.clone(),
                    OwnerIncomingOrder::PublishInitialized(_) => panic!("not a publication"),
                }
            }),
            Some(expected_second)
        );
        for (index, work) in state.retained_work.iter().enumerate() {
            let binding = work.binding.control().unwrap();
            let TradingCommand::QueryAccount(original) = &commands[index] else {
                unreachable!("prepared neutral query");
            };
            assert_eq!(binding.cause.command_id, original.command_id);
            assert_eq!(binding.cause.parent_event_id, Some(expected_event_id));
            assert_eq!(binding.cause.sequence, index as u64 + 3);
            assert_eq!(binding.cause.incarnation, state.incarnation);
            assert!(Rc::ptr_eq(&binding.context, &context));
            let ingress = work.ingress.as_ref().unwrap();
            assert_eq!(ingress.endpoint(), child_endpoint);
            assert_eq!(ingress.command(), &commands[index]);
        }
        drop(state);
        assert_eq!(owner.failure(), Some(OwnerDeliveryError::Capacity));
        assert_eq!(observer.retained_incoming_orders().unwrap().len(), 2);
        assert_eq!(owner.retire(), Err(OwnerDeliveryError::PendingAtRetirement));
        assert_eq!(native_children.get(), 0);
    }

    struct PositionDelivery {
        event: PositionEvent,
        target: Rc<RefCell<Vec<&'static str>>>,
        method: &'static str,
        fail: bool,
    }

    #[rstest::rstest]
    fn test_incoming_neutral_query_child_precedes_next_leg_with_original_context() {
        use crate::msgbus::{self, Endpoint, MStr, TypedIntoHandler};

        let owner = OwnerDelivery::new();
        let _binding = owner.bind().unwrap();
        let timeline = Rc::new(RefCell::new(Vec::new()));
        let admissions = Rc::new(RefCell::new(Vec::new()));
        let context: Rc<dyn OwnerControlContext> = Rc::new(RecordedQueryContext {
            lifecycle: timeline.clone(),
            fail_exit: false,
        });
        let first = nautilus_model::events::OrderSubmitted {
            event_id: UUID4::new(),
            client_order_id: "INCOMING-CHILD-A".into(),
            ..Default::default()
        };
        let second = nautilus_model::events::OrderSubmitted {
            event_id: UUID4::new(),
            client_order_id: "INCOMING-CHILD-B".into(),
            ..first
        };
        let command_id = UUID4::new();
        let mut command = neutral_query(command_id);
        if let TradingCommand::QueryAccount(query) = &mut command {
            query.causation_id = Some(first.event_id);
        }
        let commands: Rc<[TradingCommand]> = vec![command.clone()].into();
        let child_endpoint: MStr<Endpoint> = "OwnerDelivery.incomingOrderedChild".into();
        let original_context = context.clone();
        let expected_command = command;
        let child_timeline = timeline.clone();
        msgbus::register_trading_command_endpoint(
            child_endpoint,
            TypedIntoHandler::from(move |received: TradingCommand| {
                assert_eq!(received, expected_command);
                let continuation = current_continuation().unwrap();
                assert!(continuation.matches_current());
                assert_eq!(continuation.0.cause.command_id, command_id);
                assert_eq!(continuation.0.cause.parent_event_id, Some(first.event_id));
                assert_eq!(continuation.0.cause.sequence, 3);
                assert!(Rc::ptr_eq(&continuation.0.context, &original_context));
                child_timeline.borrow_mut().push("child");
            }),
        );
        let handler_timeline = timeline.clone();
        let handler_admissions = admissions.clone();
        let handler_context = context.clone();
        msgbus::register_order_event_endpoint(
            crate::msgbus::MessagingSwitchboard::exec_engine_process(),
            TypedIntoHandler::from(move |event: OrderEventAny| {
                assert!(has_delivery_phase());
                assert!(!is_controlled_phase());
                assert!(current_continuation().is_none());
                let is_first = event.client_order_id() == first.client_order_id;
                handler_timeline
                    .borrow_mut()
                    .push(if is_first { "native-A" } else { "native-B" });
                for (outlet, range) in if is_first {
                    [("specific-A", 0..1), ("aggregate-A", 1..1)]
                } else {
                    [("specific-B", 1..1), ("aggregate-B", 1..1)]
                } {
                    assert!(
                        capture_external(
                            event.clone().into_boxed().id(),
                            Box::new(IncomingChildDelivery {
                                event: event.clone(),
                                endpoint: child_endpoint,
                                commands: commands.clone(),
                                range,
                                context: handler_context.clone(),
                                admissions: handler_admissions.clone(),
                                outlets: handler_timeline.clone(),
                                outlet,
                            })
                        )
                        .unwrap()
                        .is_none()
                    );
                }
            }),
        );
        assert_eq!(
            dispatch_incoming_orders(vec![
                OwnerIncomingOrder::Process(OrderEventAny::Submitted(first)),
                OwnerIncomingOrder::Process(OrderEventAny::Submitted(second)),
            ]),
            Ok(())
        );
        assert_eq!(
            &*timeline.borrow(),
            &[
                "native-A",
                "specific-A",
                "aggregate-A",
                "enter",
                "child",
                "finish-ok",
                "drop",
                "native-B",
                "specific-B",
                "aggregate-B",
            ]
        );
        assert_eq!(&*admissions.borrow(), &[(command_id, Ok(true), None)]);
        assert_eq!(owner.failure(), None);
        assert_eq!(owner.pending(), 0);
        assert_eq!(owner.retire(), Ok(()));
    }

    impl OwnerExternalAction for PositionDelivery {
        fn original_event(&self) -> OwnerExternalEvent {
            OwnerExternalEvent::Position(self.event.clone())
        }

        fn deliver(&mut self) -> Result<(), OwnerDeliveryError> {
            self.target.borrow_mut().push(self.method);
            if self.fail {
                Err(OwnerDeliveryError::ExternalCallback)
            } else {
                Ok(())
            }
        }
    }

    #[rstest::rstest]
    fn test_control_position_failure_retains_typed_event_cause_and_target() {
        use nautilus_model::{
            enums::{OrderSide, PositionSide},
            events::PositionOpened,
            identifiers::{ClientOrderId, InstrumentId, PositionId, StrategyId},
            types::{Currency, Money, Price, Quantity},
        };

        use crate::msgbus::{self, Endpoint, MStr, TypedIntoHandler};

        let owner = OwnerDelivery::new();
        let _binding = owner.bind().unwrap();
        let observer = current_observer().unwrap();
        let target = Rc::new(RefCell::new(Vec::new()));
        let event_id = UUID4::new();
        let event = PositionEvent::PositionOpened(PositionOpened {
            trader_id: TraderId::from("QUERY-001"),
            strategy_id: StrategyId::from("POSITION-001"),
            instrument_id: InstrumentId::from("EURUSD.SIM"),
            position_id: PositionId::from("POSITION-OBSERVATION-001"),
            account_id: AccountId::from("SIM-001"),
            opening_order_id: ClientOrderId::from("POSITION-OPEN-001"),
            entry: OrderSide::Buy,
            side: PositionSide::Long,
            signed_qty: 100.0,
            quantity: Quantity::from("100"),
            last_qty: Quantity::from("100"),
            last_px: Price::from("1.0500"),
            currency: Currency::USD(),
            avg_px_open: 1.0500,
            realized_pnl: Some(Money::from("-2.00 USD")),
            event_id,
            ts_event: 1_000_000_000.into(),
            ts_init: 1_000_000_001.into(),
        });
        let original = serde_json::to_value(&event).unwrap();
        let endpoint: MStr<Endpoint> = "OwnerDelivery.positionFailure".into();
        let target_for_handler = target.clone();
        msgbus::register_trading_command_endpoint(
            endpoint,
            TypedIntoHandler::from(move |_command: TradingCommand| {
                for (method, fail) in [("specific", true), ("aggregate", false)] {
                    assert!(
                        capture_external(
                            event_id,
                            Box::new(PositionDelivery {
                                event: event.clone(),
                                target: target_for_handler.clone(),
                                method,
                                fail,
                            }),
                        )
                        .unwrap()
                        .is_none()
                    );
                }
            }),
        );
        let original_target_refs = Rc::strong_count(&target);
        let command_id = UUID4::new();
        let _submission = SubmissionBinding::new(Rc::new(NeutralQueryContext)).unwrap();
        assert!(
            capture_control_command(TradingCommandMessage::new(
                endpoint,
                neutral_query(command_id),
            ))
            .unwrap()
            .is_none()
        );
        assert_eq!(owner.drain(), Err(OwnerDeliveryError::ExternalCallback));
        assert_eq!(&*target.borrow(), &["specific", "aggregate"]);
        let retained = observer.retained_typed_deliveries().unwrap();
        assert_eq!(retained.len(), 1);
        let OwnerDeliveryCause::Control(cause) = retained[0].cause else {
            panic!("a control output cannot be relabelled as incoming");
        };
        assert_eq!(cause.command_id, command_id);
        assert_eq!(cause.parent_event_id, Some(event_id));
        assert_eq!(cause.sequence, 1);
        assert_eq!(cause.incarnation, owner.0.borrow().incarnation);
        let OwnerExternalEvent::Position(position) = &retained[0].event else {
            panic!("a position cannot be relabelled as an order");
        };
        assert_eq!(serde_json::to_value(position).unwrap(), original);
        assert_eq!(retained[0].event_id, event_id);
        assert_eq!(
            retained[0].error,
            Some(OwnerDeliveryError::ExternalCallback)
        );
        assert!(observer.retained_deliveries().unwrap().is_empty());
        assert!(observer.retained_incoming_orders().unwrap().is_empty());
        assert_eq!(owner.pending(), 1);
        assert_eq!(owner.failure(), Some(OwnerDeliveryError::ExternalCallback));
        assert_eq!(owner.retire(), Err(OwnerDeliveryError::PendingAtRetirement));
        assert_eq!(Rc::strong_count(&target), original_target_refs + 1);
    }

    #[rstest::rstest]
    fn test_faulted_commands_preserve_original_without_inventing_control() {
        let endpoint = "OwnerDelivery.faultedCommand".into();
        let command = neutral_query(UUID4::new());
        let make_message = || TradingCommandMessage::new(endpoint, command.clone());
        let unbound = retain_undispatched_command(make_message()).unwrap();
        assert_eq!(unbound.endpoint(), endpoint);
        assert_eq!(unbound.command(), &command);
        let owner = OwnerDelivery::new();
        let _binding = owner.bind().unwrap();
        let observer = current_observer().unwrap();
        let healthy = retain_undispatched_command(make_message()).unwrap();
        assert_eq!(healthy.endpoint(), endpoint);
        assert_eq!(healthy.command(), &command);
        assert!(observer.retained_commands().unwrap().is_empty());
        record_failure(OwnerDeliveryError::ExternalCallback);
        assert!(retain_undispatched_command(make_message()).is_none());
        let retained = observer.retained_commands().unwrap();
        assert_eq!(retained.len(), 1);
        assert_eq!(retained[0].endpoint, endpoint);
        assert_eq!(retained[0].command, command);
        assert_eq!(retained[0].cause, None);
        assert_eq!(retained[0].error, OwnerDeliveryError::ExternalCallback);
        assert_eq!(owner.pending(), 1);
        assert_eq!(owner.retire(), Err(OwnerDeliveryError::PendingAtRetirement));
        owner.0.borrow_mut().retired = true;
        assert!(retain_undispatched_command(make_message()).is_none());
        let retained = observer.retained_commands().unwrap();
        assert_eq!(retained.len(), 2);
        assert_eq!(retained[1].endpoint, endpoint);
        assert_eq!(retained[1].command, command);
        assert_eq!(retained[1].cause, None);
        assert_eq!(retained[1].error, OwnerDeliveryError::WrongOwner);
        assert_eq!(owner.failure(), Some(OwnerDeliveryError::ExternalCallback));
        assert_eq!(owner.pending(), 2);
        assert_eq!(owner.retire(), Err(OwnerDeliveryError::WrongOwner));
    }

    #[derive(Debug)]
    struct RecordedQueryContext {
        lifecycle: Rc<RefCell<Vec<&'static str>>>,
        fail_exit: bool,
    }

    struct EnteredRecordedQuery {
        lifecycle: Rc<RefCell<Vec<&'static str>>>,
        fail_exit: bool,
    }

    impl OwnerControlContext for RecordedQueryContext {
        fn enter(&self) -> Result<Box<dyn EnteredOwnerControl>, OwnerDeliveryError> {
            self.lifecycle.borrow_mut().push("enter");
            Ok(Box::new(EnteredRecordedQuery {
                lifecycle: self.lifecycle.clone(),
                fail_exit: self.fail_exit,
            }))
        }
    }

    impl EnteredOwnerControl for EnteredRecordedQuery {
        fn finish(&mut self, failed: bool) -> Result<(), OwnerDeliveryError> {
            self.lifecycle
                .borrow_mut()
                .push(if failed { "finish-failed" } else { "finish-ok" });
            if self.fail_exit {
                return Err(OwnerDeliveryError::ControlExit);
            }
            Ok(())
        }
    }

    impl Drop for EnteredRecordedQuery {
        fn drop(&mut self) {
            self.lifecycle.borrow_mut().push("drop");
        }
    }

    struct PhaseCleanup;

    impl Drop for PhaseCleanup {
        fn drop(&mut self) {
            PHASE.with(|phase| phase.borrow_mut().take());
        }
    }

    #[rstest::rstest]
    #[case(false, Some(OwnerDeliveryError::ReentrantDrain))]
    #[case(true, Some(OwnerDeliveryError::ControlExit))]
    #[case(false, None)]
    fn test_phase_collision_retains_original_work_and_finishes_failed_scope(
        #[case] fail_exit: bool,
        #[case] expected_error: Option<OwnerDeliveryError>,
    ) {
        let owner = OwnerDelivery::new();
        let _binding = owner.bind().unwrap();
        let _phase_cleanup = PhaseCleanup;
        let lifecycle = Rc::new(RefCell::new(Vec::new()));
        let context: Rc<dyn OwnerControlContext> = Rc::new(RecordedQueryContext {
            lifecycle: lifecycle.clone(),
            fail_exit,
        });
        let cause = owner.cause(UUID4::new()).unwrap();
        let query = neutral_query(cause.command_id);
        let endpoint = "OwnerDelivery.phaseCollision".into();
        let target = Rc::new(Cell::new(0));
        let target_weak = Rc::downgrade(&target);
        let calls = Rc::new(Cell::new(0));
        let action_calls = calls.clone();
        owner
            .enqueue(
                Work {
                    binding: Binding {
                        context: context.clone(),
                        cause,
                    }
                    .into(),
                    incoming: None,
                    requires_control: true,
                    ingress: Some(TradingCommandMessage::new(endpoint, query.clone())),
                    action: Box::new(move || {
                        action_calls.set(action_calls.get() + 1);
                        target.set(target.get() + 1);
                        Ok(())
                    }),
                },
                false,
            )
            .unwrap();

        let existing_cause = owner.cause(UUID4::new()).unwrap();
        let existing_query = neutral_query(existing_cause.command_id);
        let existing_context: Rc<dyn OwnerControlContext> = Rc::new(NeutralQueryContext);
        if expected_error.is_some() {
            PHASE.with(|phase| {
                assert!(phase.borrow().is_none());
                *phase.borrow_mut() = Some(Phase {
                    binding: Binding {
                        context: existing_context.clone(),
                        cause: existing_cause,
                    }
                    .into(),
                    incoming: None,
                    external: VecDeque::new(),
                    continuations: Vec::new(),
                    ingress: Some(TradingCommandMessage::new(endpoint, existing_query.clone())),
                });
            });
        }

        assert_eq!(owner.drain(), expected_error.map_or(Ok(()), Err));
        if let Some(error) = expected_error {
            assert_eq!(&*lifecycle.borrow(), &["enter", "finish-failed", "drop"]);
            assert_eq!(target_weak.upgrade().unwrap().get(), 0);
            assert_eq!(calls.get(), 0);
            assert_eq!(owner.pending(), 1);
            assert_eq!(owner.failure(), Some(error));
            {
                let state = owner.0.borrow();
                assert!(!state.draining);
                let work = state.queue.front().unwrap();
                assert_eq!(work.binding.control().unwrap().cause, cause);
                assert!(Rc::ptr_eq(
                    &work.binding.control().unwrap().context,
                    &context
                ));
                let ingress = work.ingress.as_ref().unwrap();
                assert_eq!(ingress.endpoint(), endpoint);
                assert_eq!(ingress.command(), &query);
            }
            PHASE.with(|phase| {
                let phase = phase.borrow();
                let phase = phase.as_ref().unwrap();
                assert_eq!(phase.binding.control().unwrap().cause, existing_cause);
                assert!(Rc::ptr_eq(
                    &phase.binding.control().unwrap().context,
                    &existing_context
                ));
                assert_eq!(phase.ingress.as_ref().unwrap().command(), &existing_query);
                assert!(phase.external.is_empty());
                assert!(phase.continuations.is_empty());
            });
            assert_eq!(owner.drain(), Err(error));
            assert_eq!(owner.retire(), Err(OwnerDeliveryError::PendingAtRetirement));
            assert_eq!(&*lifecycle.borrow(), &["enter", "finish-failed", "drop"]);
        } else {
            assert_eq!(&*lifecycle.borrow(), &["enter", "finish-ok", "drop"]);
            assert!(target_weak.upgrade().is_none());
            assert_eq!(calls.get(), 1);
            assert_eq!(owner.pending(), 0);
            assert_eq!(owner.failure(), None);
            PHASE.with(|phase| assert!(phase.borrow().is_none()));
            assert_eq!(owner.retire(), Ok(()));
        }
    }

    #[rstest::rstest]
    fn test_normal_continuation_group_capacity_preserves_order_cause_and_strong_targets() {
        use crate::msgbus::{self, Endpoint, MStr, TypedIntoHandler};

        let owner = OwnerDelivery::new();
        let _binding = owner.bind().unwrap();
        let calls = Rc::new(Cell::new(0));
        let target = Rc::new(Cell::new(0));
        let parent_ids = [UUID4::new(), UUID4::new()];
        let root_command_id = UUID4::new();
        let endpoint: MStr<Endpoint> = "OwnerDelivery.groupCapacity".into();
        let handler = TypedIntoHandler::from({
            let calls = calls.clone();
            let target = target.clone();
            move |_command: TradingCommand| {
                calls.set(calls.get() + 1);
                if calls.get() == 1 {
                    let continuation = current_continuation().unwrap();
                    for event_id in parent_ids {
                        let target = target.clone();
                        continuation
                            .clone()
                            .with_parent_event(event_id)
                            .schedule(
                                true,
                                Box::new(move || {
                                    target.set(target.get() + 1);
                                    Ok(())
                                }),
                            )
                            .unwrap();
                    }
                    let event =
                        OrderEventAny::Canceled(nautilus_model::events::OrderCanceled::default());
                    assert!(
                        capture_external(
                            event.clone().into_boxed().id(),
                            Box::new(QueryChildDelivery {
                                event,
                                command: neutral_query(UUID4::new())
                            })
                        )
                        .unwrap()
                        .is_none()
                    );
                }
            }
        });
        msgbus::register_trading_command_endpoint(endpoint, handler);
        let initial_target_refs = Rc::strong_count(&target);
        {
            let _submission = SubmissionBinding::new(Rc::new(NeutralQueryContext)).unwrap();
            for index in 0..CAPACITY {
                let command_id = if index == 0 {
                    root_command_id
                } else {
                    UUID4::new()
                };
                assert!(
                    capture_control_command(TradingCommandMessage::new(
                        endpoint,
                        neutral_query(command_id)
                    ))
                    .unwrap()
                    .is_none()
                );
            }
        }
        assert_eq!(owner.drain(), Err(OwnerDeliveryError::Capacity));
        assert_eq!(calls.get(), 1);
        assert_eq!(target.get(), 0);
        assert_eq!(Rc::strong_count(&target), initial_target_refs + 2);
        let state = owner.0.borrow();
        assert_eq!(state.queue.len(), CAPACITY);
        assert_eq!(state.retained_work.len(), 2);
        for (work, parent_id) in state.retained_work.iter().zip(parent_ids) {
            assert_eq!(
                work.binding.control().unwrap().cause.command_id,
                root_command_id
            );
            assert_eq!(
                work.binding.control().unwrap().cause.parent_event_id,
                Some(parent_id)
            );
            assert_eq!(work.binding.control().unwrap().cause.sequence, 1);
        }
    }

    #[rstest::rstest]
    fn test_foreign_incarnation_retains_only_rejected_work_and_blocks_retirement() {
        use crate::msgbus::{self, Endpoint, MStr, TypedIntoHandler};

        let first = OwnerDelivery::new();
        let first_binding = first.bind().unwrap();
        let held = Rc::new(RefCell::new(None));
        let endpoint: MStr<Endpoint> = "OwnerDelivery.originalContinuation".into();
        msgbus::register_trading_command_endpoint(
            endpoint,
            TypedIntoHandler::from({
                let held = held.clone();
                move |_command: TradingCommand| *held.borrow_mut() = current_continuation()
            }),
        );
        let command_id = UUID4::new();
        {
            let _submission = SubmissionBinding::new(Rc::new(NeutralQueryContext)).unwrap();
            capture_control_command(TradingCommandMessage::new(
                endpoint,
                neutral_query(command_id),
            ))
            .unwrap();
        }
        first.drain().unwrap();
        drop(first_binding);
        let second = OwnerDelivery::new();
        let _second_binding = second.bind().unwrap();
        let effect = Rc::new(Cell::new(0));
        let result = held.borrow_mut().take().unwrap().schedule(
            true,
            Box::new({
                let effect = effect.clone();
                move || {
                    effect.set(effect.get() + 1);
                    Ok(())
                }
            }),
        );
        assert_eq!(result, Err(OwnerDeliveryError::WrongOwner));
        assert_eq!(
            second.pending(),
            1,
            "retained work alone must keep capture pending"
        );
        assert_eq!(effect.get(), 0);
        assert_eq!(Rc::strong_count(&effect), 2);
        assert!(second.0.borrow().queue.is_empty());
        assert_eq!(
            second.0.borrow().retained_work[0]
                .binding
                .control()
                .unwrap()
                .cause
                .command_id,
            command_id
        );
        assert_eq!(
            second.retire(),
            Err(OwnerDeliveryError::PendingAtRetirement)
        );
        assert!(!second.0.borrow().retired);
    }

    #[rstest::rstest]
    fn test_sequence_exhaustion_keeps_the_original_unassigned_command() {
        let owner = OwnerDelivery::new();
        let _binding = owner.bind().unwrap();
        let _submission = SubmissionBinding::new(Rc::new(NeutralQueryContext)).unwrap();
        owner.0.borrow_mut().next_sequence = u64::MAX;
        let original = neutral_query(UUID4::new());
        let result = capture_control_command(TradingCommandMessage::new(
            "OwnerDelivery.exhausted".into(),
            original.clone(),
        ));
        assert!(matches!(result, Err(OwnerDeliveryError::SequenceExhausted)));
        assert_eq!(owner.failure(), Some(OwnerDeliveryError::SequenceExhausted));
        let records = current_observer().unwrap().retained_commands().unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].command, original);
        assert_eq!(
            records[0].cause, None,
            "an unallocated sequence cannot be relabelled as the last admitted work"
        );
        assert_eq!(records[0].error, OwnerDeliveryError::SequenceExhausted);
        assert_eq!(owner.0.borrow().next_sequence, u64::MAX);
        assert_eq!(
            owner.pending(),
            1,
            "an exhausted sequence must not erase the original unassigned command"
        );
        assert_eq!(owner.retire(), Err(OwnerDeliveryError::PendingAtRetirement));
    }
}
