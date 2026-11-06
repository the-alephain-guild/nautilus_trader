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
use nautilus_model::events::OrderEventAny;

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

pub type OwnerAction = Box<dyn FnOnce() -> Result<(), OwnerDeliveryError>>;

/// Retains the original external payload, including an actual callback failure.
pub trait OwnerExternalAction {
    fn original_event(&self) -> &OrderEventAny;
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
                    binding: self.0,
                    requires_control,
                    action,
                    ingress: None,
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
                binding: self.0,
                requires_control,
                action,
                ingress: None,
            },
            false,
        )
    }

    #[must_use]
    pub fn matches_current(&self) -> bool {
        PHASE.with(|phase| {
            phase.borrow().as_ref().is_some_and(|phase| {
                phase.binding.cause.incarnation == self.0.cause.incarnation
                    && phase.binding.cause.command_id == self.0.cause.command_id
                    && Rc::ptr_eq(&phase.binding.context, &self.0.context)
            })
        })
    }
}

struct Work {
    binding: Binding,
    requires_control: bool,
    action: OwnerAction,
    ingress: Option<TradingCommandMessage>,
}

struct External {
    binding: Binding,
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

struct Phase {
    binding: Binding,
    external: VecDeque<External>,
    continuations: Vec<Work>,
    ingress: Option<TradingCommandMessage>,
}

struct RetainedCommand {
    origin: Option<Binding>,
    _context: Rc<dyn OwnerControlContext>,
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
    failure: Option<OwnerDeliveryError>,
    draining: bool,
    retired: bool,
}

thread_local! {
    static OWNER: RefCell<Option<OwnerDelivery>> = const { RefCell::new(None) };
    static PHASE: RefCell<Option<Phase>> = const { RefCell::new(None) };
    static SUBMISSION: RefCell<Option<Rc<dyn OwnerControlContext>>> = const { RefCell::new(None) };
    static DELIVERY: RefCell<Option<Binding>> = const { RefCell::new(None) };
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
                .map(|item| RetainedOwnerDelivery {
                    cause: item.binding.cause,
                    event_id: item.event_id,
                    event: item.action.original_event().clone(),
                    error: item.error,
                })
                .collect()
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
                    + state.retained_work.len(),
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
        self.check()?;
        let mut state = self.0.borrow_mut();
        if work.binding.cause.incarnation != state.incarnation {
            state.retained_work.push_back(work);
            state.failure.get_or_insert(OwnerDeliveryError::WrongOwner);
            return Err(OwnerDeliveryError::WrongOwner);
        }
        let blocked = state.failure.or_else(|| {
            (state.queue.len()
                + state.retained_external.len()
                + state.retained_commands.len()
                + state.retained_work.len()
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
            _context: context,
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
                .any(|item| item.binding.cause.incarnation != state.incarnation)
            {
                Some(OwnerDeliveryError::WrongOwner)
            } else if state.queue.len()
                + state.retained_external.len()
                + state.retained_commands.len()
                + state.retained_work.len()
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
        let parent_event_id = active_binding().and_then(|binding| binding.cause.parent_event_id);
        Ok(OwnerCause {
            command_id,
            parent_event_id,
            sequence: state.next_sequence,
            incarnation: state.incarnation,
        })
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
            state.failure = Some(error);
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
            if work.binding.cause.incarnation != self.0.borrow().incarnation {
                return Err(OwnerDeliveryError::WrongOwner);
            }
            let mut entered = if work.requires_control {
                match work.binding.context.enter() {
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
            while let Some(mut external) = phase.external.pop_front() {
                let mut binding = external.binding.clone();
                binding.cause.parent_event_id = Some(external.event_id);
                external.binding = binding.clone();
                DELIVERY.with(|delivery| *delivery.borrow_mut() = Some(binding));
                let delivered = catch_unwind(AssertUnwindSafe(|| external.action.deliver()))
                    .unwrap_or(Err(OwnerDeliveryError::ExternalCallback));
                DELIVERY.with(|delivery| delivery.borrow_mut().take());
                if let Err(error) = delivered {
                    // A failed specific callback must not erase itself or suppress its aggregate.
                    external.error = Some(error);
                    self.0.borrow_mut().retained_external.push_back(external);
                    delivery_error.get_or_insert(error);
                }
            }
            // Pending native work precedes newly emitted callback children, as in the original drain
            if let Some(error) = delivery_error {
                self.0
                    .borrow_mut()
                    .retained_work
                    .extend(phase.continuations);
                return Err(error);
            }
            self.queue_continuations(phase.continuations)?;
            native?;
        }
    }

    #[must_use]
    pub fn pending(&self) -> usize {
        let state = self.0.borrow();
        state.queue.len()
            + state.retained_external.len()
            + state.retained_commands.len()
            + state.retained_work.len()
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
    PHASE.with(|phase| phase.borrow().is_some())
}

#[must_use]
pub fn current_continuation() -> Option<OwnerContinuation> {
    PHASE.with(|phase| {
        phase
            .borrow()
            .as_ref()
            .map(|phase| OwnerContinuation(phase.binding.clone()))
    })
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
    owner.enqueue(
        Work {
            binding,
            requires_control: true,
            ingress: Some(ingress),
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
            },
            requires_control: true,
            ingress: Some(ingress),
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
        .with(|phase| phase.borrow().as_ref().map(|phase| phase.binding.clone()))
        .or_else(|| DELIVERY.with(|delivery| delivery.borrow().clone()))
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
            assert_eq!(last.binding.cause.sequence, (CAPACITY + 1) as u64);
            assert_eq!(last.binding.cause.incarnation, state.incarnation);
        }
        assert_eq!(owner.drain(), Err(OwnerDeliveryError::Capacity));
        assert_eq!(owner.retire(), Err(OwnerDeliveryError::PendingAtRetirement));
    }

    struct QueryChildDelivery {
        event: OrderEventAny,
        command: TradingCommand,
    }

    impl OwnerExternalAction for QueryChildDelivery {
        fn original_event(&self) -> &OrderEventAny {
            &self.event
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
                    },
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
                    },
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
                assert_eq!(work.binding.cause, cause);
                assert!(Rc::ptr_eq(&work.binding.context, &context));
                let ingress = work.ingress.as_ref().unwrap();
                assert_eq!(ingress.endpoint(), endpoint);
                assert_eq!(ingress.command(), &query);
            }
            PHASE.with(|phase| {
                let phase = phase.borrow();
                let phase = phase.as_ref().unwrap();
                assert_eq!(phase.binding.cause, existing_cause);
                assert!(Rc::ptr_eq(&phase.binding.context, &existing_context));
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
            assert_eq!(work.binding.cause.command_id, root_command_id);
            assert_eq!(work.binding.cause.parent_event_id, Some(parent_id));
            assert_eq!(work.binding.cause.sequence, 1);
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
            second.0.borrow().retained_work[0].binding.cause.command_id,
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
