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

//! Owned report submissions and resumptions at native owner safe points.

use std::{
    cell::RefCell,
    fmt,
    panic::{AssertUnwindSafe, catch_unwind},
    rc::{Rc, Weak},
    thread::{self, ThreadId},
};

use nautilus_core::UUID4;
use nautilus_model::events::OrderEventAny;

use super::{
    Binding, CAPACITY, DELIVERY, OWNER, OwnerAction, OwnerCause, OwnerDelivery, OwnerDeliveryError,
    OwnerObserver, PHASE, PhaseBinding, Work, active_binding, active_parent_event_id,
    active_report_ancestor, drain_current, has_delivery_phase, queue_callback_item,
};
use crate::messages::ExecutionReport;

/// The actual synchronous Portfolio processing branch of one native publication.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReportPortfolioBranch {
    Order,
    Position,
    AccountState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PortfolioSkipReason {
    NoAccountAssigned,
    AccountCalculationDisabled,
    EventNotApplicable,
    MirrorNoNewApply,
    AccountStateAlreadyApplied,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PortfolioPendingReason {
    MissingMarketPrice,
    MissingExchangeRate,
    MissingValuationInput,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PortfolioFailureStage {
    Lookup,
    BalanceCalculation,
    ReservationCalculation,
    AccountApply,
    AccountPersistence,
    AccountState,
}

#[derive(Debug)]
pub struct PortfolioNativeFailure {
    pub stage: PortfolioFailureStage,
    pub message: String,
}

#[derive(Debug)]
pub enum ReportPortfolioOutcome {
    Complete,
    Skipped(PortfolioSkipReason),
    Pending(PortfolioPendingReason),
    Failed(PortfolioNativeFailure),
    Unobserved,
}

#[derive(Debug)]
struct PortfolioReceiverIdentity {
    thread: ThreadId,
    pid: u32,
    target: Weak<dyn std::any::Any>,
}

/// A capability held by the concrete Portfolio's registered processing closures.
/// Registration belongs to trusted native composition, not financial authorization.
#[derive(Clone, Debug)]
pub struct ReportPortfolioReceiver(Rc<PortfolioReceiverIdentity>);

struct PortfolioReplyFrame {
    cause: OwnerReportCause,
    event_id: UUID4,
    branch: ReportPortfolioBranch,
    receiver: Option<Rc<PortfolioReceiverIdentity>>,
    receiver_target: Option<Rc<dyn std::any::Any>>,
    outcome: Option<ReportPortfolioOutcome>,
    invalid: bool,
    processing: bool,
}

impl fmt::Debug for PortfolioReplyFrame {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PortfolioReplyFrame")
            .field("cause", &self.cause)
            .field("event_id", &self.event_id)
            .field("branch", &self.branch)
            .field("receiver", &self.receiver)
            .field("has_target", &self.receiver_target.is_some())
            .field("invalid", &self.invalid)
            .finish_non_exhaustive()
    }
}

thread_local! {
    static PORTFOLIO_RECEIVER: RefCell<Weak<PortfolioReceiverIdentity>> = const { RefCell::new(Weak::new()) };
    static PORTFOLIO_REPLIES: RefCell<Vec<Rc<RefCell<PortfolioReplyFrame>>>> = const { RefCell::new(Vec::new()) };
}

impl ReportPortfolioReceiver {
    /// Registers the concrete native receiver; replacing it is a composition operation.
    #[must_use]
    pub fn register<T: 'static>(target: &Rc<T>) -> Self {
        let target: Rc<dyn std::any::Any> = target.clone();
        let receiver = Rc::new(PortfolioReceiverIdentity {
            thread: thread::current().id(),
            pid: std::process::id(),
            target: Rc::downgrade(&target),
        });
        PORTFOLIO_RECEIVER.with(|slot| *slot.borrow_mut() = Rc::downgrade(&receiver));
        Self(receiver)
    }

    /// Fixes the concrete publication frame at real handler entry.
    ///
    /// # Errors
    /// Refuses a foreign receiver or a stale, duplicate, or mismatched publication.
    pub fn begin_update(
        &self,
        event_id: UUID4,
        branch: ReportPortfolioBranch,
    ) -> Result<Option<ReportPortfolioUpdate>, OwnerDeliveryError> {
        let Some(cause) = current_report_cause() else {
            return Ok(None);
        };
        let frame = PORTFOLIO_REPLIES
            .with(|stack| stack.borrow().last().cloned())
            .ok_or(OwnerDeliveryError::UnsupportedEffect)?;
        {
            let mut frame = frame.borrow_mut();
            // A stale registration cannot mutate another concrete receiver's frame.
            if !frame
                .receiver
                .as_ref()
                .is_some_and(|receiver| Rc::ptr_eq(receiver, &self.0))
            {
                return Err(OwnerDeliveryError::UnsupportedEffect);
            }
            let matches = self.0.thread == thread::current().id()
                && self.0.pid == std::process::id()
                && frame.receiver_target.as_ref().is_some_and(|target| {
                    self.0
                        .target
                        .upgrade()
                        .as_ref()
                        .is_some_and(|actual| Rc::ptr_eq(target, actual))
                })
                && frame.cause == cause
                && frame.event_id == event_id
                && frame.branch == branch
                && frame.outcome.is_none()
                && !frame.invalid
                && !frame.processing;
            if !matches {
                frame.invalid = true;
                return Err(OwnerDeliveryError::UnsupportedEffect);
            }
            frame.processing = true;
        }
        Ok(Some(ReportPortfolioUpdate {
            frame,
            receiver: self.0.clone(),
            finished: false,
        }))
    }
}

/// An entered real receiver owns the original frame until its processing returns.
#[derive(Debug)]
pub struct ReportPortfolioUpdate {
    frame: Rc<RefCell<PortfolioReplyFrame>>,
    receiver: Rc<PortfolioReceiverIdentity>,
    finished: bool,
}

impl ReportPortfolioUpdate {
    /// Replies only to the original entered frame, never whichever frame is now current.
    ///
    /// # Errors
    /// Refuses an expired cause, non-LIFO return, duplicate, or changed receiver identity.
    pub fn finish(mut self, outcome: ReportPortfolioOutcome) -> Result<(), OwnerDeliveryError> {
        let top = PORTFOLIO_REPLIES.with(|stack| {
            stack
                .borrow()
                .last()
                .is_some_and(|top| Rc::ptr_eq(top, &self.frame))
        });
        let current = current_report_cause();
        let mut frame = self.frame.borrow_mut();
        let matches = top
            && current.as_ref() == Some(&frame.cause)
            && self.receiver.thread == thread::current().id()
            && self.receiver.pid == std::process::id()
            && frame
                .receiver
                .as_ref()
                .is_some_and(|receiver| Rc::ptr_eq(receiver, &self.receiver))
            && frame.processing
            && frame.outcome.is_none()
            && !frame.invalid;
        frame.processing = false;
        self.finished = true;
        if matches {
            frame.outcome = Some(outcome);
            Ok(())
        } else {
            frame.invalid = true;
            Err(OwnerDeliveryError::UnsupportedEffect)
        }
    }
}

impl Drop for ReportPortfolioUpdate {
    fn drop(&mut self) {
        if !self.finished {
            let mut frame = self.frame.borrow_mut();
            frame.invalid = true;
            frame.processing = false;
        }
    }
}

fn current_report_cause() -> Option<OwnerReportCause> {
    PHASE.with(|phase| {
        phase
            .borrow()
            .as_ref()
            .and_then(|phase| match &phase.binding {
                PhaseBinding::Report { cause, .. } => Some(cause.clone()),
                _ => None,
            })
    })
}

/// An owned, thread-local reply frame for exactly one actual send or publish call.
#[derive(Debug)]
pub struct ReportPortfolioReceipt {
    frame: Rc<RefCell<PortfolioReplyFrame>>,
}

/// Begins one concrete publication's Portfolio observation only inside a real report phase.
///
/// # Errors
/// Refuses an invalid actual owner; standalone calls do not obtain a report receipt.
pub fn begin_report_portfolio_update(
    event_id: UUID4,
    branch: ReportPortfolioBranch,
) -> Result<Option<ReportPortfolioReceipt>, OwnerDeliveryError> {
    let Some(cause) = current_report_cause() else {
        return Ok(None);
    };
    let owner = OWNER
        .with(|slot| slot.borrow().clone())
        .ok_or(OwnerDeliveryError::Unbound)?;
    owner.check()?;
    if owner.0.borrow().incarnation != cause.incarnation {
        return Err(OwnerDeliveryError::WrongOwner);
    }
    if PORTFOLIO_REPLIES.with(|stack| stack.borrow().len() >= CAPACITY) {
        PORTFOLIO_REPLIES.with(|stack| {
            if let Some(frame) = stack.borrow().last() {
                frame.borrow_mut().invalid = true;
            }
        });
        return Err(OwnerDeliveryError::Capacity);
    }
    let receiver = PORTFOLIO_RECEIVER.with(|receiver| receiver.borrow().upgrade());
    let receiver_target = receiver
        .as_ref()
        .and_then(|receiver| receiver.target.upgrade());
    let frame = Rc::new(RefCell::new(PortfolioReplyFrame {
        cause,
        event_id,
        branch,
        receiver,
        receiver_target,
        outcome: None,
        invalid: false,
        processing: false,
    }));
    PORTFOLIO_REPLIES.with(|stack| stack.borrow_mut().push(frame.clone()));
    Ok(Some(ReportPortfolioReceipt { frame }))
}

impl ReportPortfolioReceipt {
    #[must_use]
    pub fn finish(self) -> ReportPortfolioOutcome {
        let current = current_report_cause();
        let is_top = PORTFOLIO_REPLIES.with(|stack| {
            stack
                .borrow()
                .last()
                .is_some_and(|top| Rc::ptr_eq(top, &self.frame))
        });
        let mut frame = self.frame.borrow_mut();
        if !is_top || current.as_ref() != Some(&frame.cause) || frame.invalid {
            ReportPortfolioOutcome::Unobserved
        } else {
            frame
                .outcome
                .take()
                .unwrap_or(ReportPortfolioOutcome::Unobserved)
        }
    }
}

impl Drop for ReportPortfolioReceipt {
    fn drop(&mut self) {
        PORTFOLIO_REPLIES.with(|stack| {
            let mut stack = stack.borrow_mut();
            if stack.last().is_some_and(|top| Rc::ptr_eq(top, &self.frame)) {
                stack.pop();
            } else {
                self.frame.borrow_mut().invalid = true;
                stack.retain(|frame| !Rc::ptr_eq(frame, &self.frame));
            }
        });
    }
}

#[derive(Clone, Debug)]
pub struct OwnerReportInput {
    pub reports: Vec<ExecutionReport>,
    pub prepared_events: Vec<OrderEventAny>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OwnerReportTicket {
    pub incarnation: UUID4,
    pub sequence: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OwnerReportStepKind {
    RawInputs,
    ExternalInitialization,
    CanonicalEvent,
    RetainedFillProjection,
    Finalize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OwnerReportRef {
    pub root_report_id: UUID4,
    pub source_report_id: UUID4,
    pub root_index: usize,
    pub order_index: Option<usize>,
    pub fill_index: Option<usize>,
    pub position_index: Option<(usize, usize)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OwnerReportCause {
    pub ticket: OwnerReportTicket,
    pub sources: Vec<OwnerReportRef>,
    pub canonical_event_id: Option<UUID4>,
    pub sequence: u64,
    pub incarnation: UUID4,
    pub parent_event_id: Option<UUID4>,
    pub parent_report_ticket: Option<OwnerReportTicket>,
    pub control_origin: Option<OwnerCause>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OwnerReportDispatchStage {
    Prepared,
    DispatchEntered,
    DispatchReturned,
    CompletionPending,
    Complete,
}

#[derive(Debug)]
pub enum OwnerNativeDispatch {
    Returned,
    Failed(OwnerDeliveryError),
}

pub type OwnerReportAfterNative =
    Box<dyn FnOnce(OwnerNativeDispatch) -> Result<(), OwnerDeliveryError>>;

pub struct PreparedOwnerReportStep {
    pub kind: OwnerReportStepKind,
    pub sources: Vec<OwnerReportRef>,
    pub event: Option<OrderEventAny>,
    pub action: OwnerAction,
    pub after_native: OwnerReportAfterNative,
}

impl fmt::Debug for PreparedOwnerReportStep {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PreparedOwnerReportStep")
            .field("kind", &self.kind)
            .field("sources", &self.sources)
            .field("event", &self.event)
            .finish_non_exhaustive()
    }
}

#[derive(Debug)]
#[expect(
    clippy::large_enum_variant,
    reason = "a prepared step moves directly into its task without a separate allocation"
)]
pub enum OwnerReportNext {
    Step(PreparedOwnerReportStep),
    Complete,
}

pub trait OwnerReportDriver: fmt::Debug {
    /// Prepares one native action without applying or publishing anything.
    ///
    /// # Errors
    /// Returns a preparation failure with the driver still owned by the node.
    fn prepare_next(&mut self) -> Result<OwnerReportNext, OwnerDeliveryError>;
}

#[derive(Debug)]
pub struct OwnerReportSubmission {
    input: Rc<OwnerReportInput>,
    driver: Box<dyn OwnerReportDriver>,
}

impl OwnerReportSubmission {
    #[must_use]
    pub fn new(input: OwnerReportInput, driver: Box<dyn OwnerReportDriver>) -> Self {
        Self {
            input: Rc::new(input),
            driver,
        }
    }

    #[must_use]
    pub fn input(&self) -> &OwnerReportInput {
        &self.input
    }
}

#[derive(Debug)]
pub enum OwnerReportDisposition {
    Standalone(OwnerReportSubmission),
    Queued(OwnerReportTicket),
    Complete(OwnerReportTicket),
}

#[derive(Clone, Debug)]
pub struct RetainedOwnerReport {
    pub ticket: Option<OwnerReportTicket>,
    pub input: OwnerReportInput,
    pub cause: Option<OwnerReportCause>,
    pub kind: Option<OwnerReportStepKind>,
    pub stage: OwnerReportDispatchStage,
    pub event: Option<OrderEventAny>,
    pub next_prepared_event_index: Option<usize>,
    pub error: OwnerDeliveryError,
}

pub(super) struct ReportTask {
    submission: OwnerReportSubmission,
    ticket: Option<OwnerReportTicket>,
    control: Option<Binding>,
    parent_event_id: Option<UUID4>,
    parent_report_ticket: Option<OwnerReportTicket>,
    cause: Option<OwnerReportCause>,
    kind: Option<OwnerReportStepKind>,
    event: Option<OrderEventAny>,
    stage: OwnerReportDispatchStage,
    error: Option<OwnerDeliveryError>,
    prepared: Option<PreparedOwnerReportStep>,
}

impl ReportTask {
    fn retained(&self, error: OwnerDeliveryError) -> RetainedOwnerReport {
        RetainedOwnerReport {
            ticket: self.ticket,
            input: (*self.submission.input).clone(),
            cause: self.cause.clone(),
            kind: self.kind,
            stage: self.stage,
            event: self.event.clone(),
            next_prepared_event_index: None,
            error: self.error.unwrap_or(error),
        }
    }
}

#[expect(
    clippy::large_enum_variant,
    reason = "normal native work stays inline in the owner queue"
)]
pub(super) enum QueueItem {
    Native(Work),
    ReportResume(OwnerReportTicket),
}

impl QueueItem {
    pub(super) fn work(&self) -> Option<&Work> {
        match self {
            Self::Native(work) => Some(work),
            Self::ReportResume(_) => None,
        }
    }

    pub(super) fn admission_cost(&self) -> usize {
        match self {
            Self::Native(work) => usize::from(!matches!(work.binding, PhaseBinding::Report { .. })),
            Self::ReportResume(_) => 0,
        }
    }

    pub(super) fn incarnation(&self) -> UUID4 {
        match self {
            Self::Native(work) => work.binding.incarnation(),
            Self::ReportResume(ticket) => ticket.incarnation,
        }
    }
}

impl From<Work> for QueueItem {
    fn from(work: Work) -> Self {
        Self::Native(work)
    }
}

impl OwnerObserver {
    /// Reads retained original reports and progress, without granting resume rights.
    ///
    /// # Errors
    /// Refuses a different thread or process.
    pub fn retained_reports(&self) -> Result<Vec<RetainedOwnerReport>, OwnerDeliveryError> {
        if self.thread != thread::current().id() || self.pid != std::process::id() {
            return Err(OwnerDeliveryError::WrongOwner);
        }
        Ok(self.owner.upgrade().map_or_else(Vec::new, |owner| {
            let state = owner.borrow();
            state
                .reports
                .iter()
                .map(|task| {
                    task.borrow().retained(
                        state
                            .failure
                            .unwrap_or(OwnerDeliveryError::PendingAtRetirement),
                    )
                })
                .collect()
        }))
    }
}

impl OwnerDelivery {
    pub(super) fn resume_report(
        &self,
        ticket: OwnerReportTicket,
    ) -> Result<Option<Work>, OwnerDeliveryError> {
        let task = self
            .0
            .borrow()
            .reports
            .iter()
            .find(|task| task.borrow().ticket == Some(ticket))
            .cloned()
            .ok_or(OwnerDeliveryError::WrongOwner)?;
        let prepared = catch_unwind(AssertUnwindSafe(|| {
            task.borrow_mut().submission.driver.prepare_next()
        }))
        .unwrap_or(Err(OwnerDeliveryError::NativePanicked));
        let next = match prepared {
            Ok(next) => next,
            Err(error) => {
                task.borrow_mut().error = Some(error);
                return Err(error);
            }
        };
        let OwnerReportNext::Step(step) = next else {
            task.borrow_mut().stage = OwnerReportDispatchStage::Complete;
            self.0
                .borrow_mut()
                .reports
                .retain(|item| !Rc::ptr_eq(item, &task));
            return Ok(None);
        };
        {
            let mut report = task.borrow_mut();
            report.kind = Some(step.kind);
            report.event.clone_from(&step.event);
            report.cause = None;
            report.stage = OwnerReportDispatchStage::Prepared;
            // Preparation has already produced owned effects: retain them before any refusal.
            report.prepared = Some(step);
        }
        let sequence = {
            let mut state = self.0.borrow_mut();
            let Some(sequence) = state.next_sequence.checked_add(1) else {
                task.borrow_mut().error = Some(OwnerDeliveryError::SequenceExhausted);
                return Err(OwnerDeliveryError::SequenceExhausted);
            };
            state.next_sequence = sequence;
            sequence
        };
        let mut report = task.borrow_mut();
        let step = report
            .prepared
            .as_ref()
            .expect("owned prepared report step");
        let cause = OwnerReportCause {
            ticket,
            sources: step.sources.clone(),
            canonical_event_id: step
                .event
                .as_ref()
                .map(|event| event.clone().into_boxed().id()),
            sequence,
            incarnation: ticket.incarnation,
            parent_event_id: report.parent_event_id,
            parent_report_ticket: report.parent_report_ticket,
            control_origin: report.control.as_ref().map(|binding| binding.cause),
        };
        report.cause = Some(cause.clone());
        let binding = PhaseBinding::Report {
            cause,
            control: report.control.clone(),
        };
        let requires_control = report.control.is_some();
        drop(report);
        Ok(Some(Work {
            binding,
            requires_control,
            ingress: None,
            incoming: None,
            action: Box::new(move || {
                let step = task
                    .borrow_mut()
                    .prepared
                    .take()
                    .expect("owned step until actual dispatch");
                task.borrow_mut().stage = OwnerReportDispatchStage::DispatchEntered;
                let (native, returned) = match catch_unwind(AssertUnwindSafe(step.action)) {
                    Ok(result) => (result, true),
                    Err(_) => (Err(OwnerDeliveryError::NativePanicked), false),
                };
                if returned {
                    task.borrow_mut().stage = OwnerReportDispatchStage::DispatchReturned;
                }
                let completion = catch_unwind(AssertUnwindSafe(|| {
                    (step.after_native)(match native {
                        Ok(()) => OwnerNativeDispatch::Returned,
                        Err(error) => OwnerNativeDispatch::Failed(error),
                    })
                }))
                .unwrap_or(Err(OwnerDeliveryError::NativePanicked));
                let result = native.and(completion);
                let mut report = task.borrow_mut();
                if let Err(error) = result {
                    report.error = Some(error);
                } else {
                    report.stage = OwnerReportDispatchStage::CompletionPending;
                }
                result
            }),
        }))
    }
}

/// Owns a report before returning and drives it only at an owner safe point.
///
/// # Errors
/// Retains the original submission on admission or processing failure.
pub fn dispatch_incoming_report(
    submission: OwnerReportSubmission,
) -> Result<OwnerReportDisposition, OwnerDeliveryError> {
    let Some(owner) = OWNER.with(|slot| slot.borrow().clone()) else {
        return Ok(OwnerReportDisposition::Standalone(submission));
    };
    let parent_report_ticket = active_report_ancestor();
    let task = Rc::new(RefCell::new(ReportTask {
        submission,
        ticket: None,
        control: active_binding(),
        parent_event_id: active_parent_event_id(),
        parent_report_ticket,
        cause: None,
        kind: None,
        event: None,
        stage: OwnerReportDispatchStage::Prepared,
        error: None,
        prepared: None,
    }));
    let checked = owner.check();
    let mut state = owner.0.borrow_mut();
    let error = checked
        .err()
        .or(state.failure)
        .or_else(|| (state.pending_count() >= CAPACITY).then_some(OwnerDeliveryError::Capacity));
    state.reports.push(task.clone());
    if let Some(error) = error {
        state.failure.get_or_insert(error);
        task.borrow_mut().error = Some(error);
        return Err(error);
    }
    let Some(sequence) = state.next_sequence.checked_add(1) else {
        let error = OwnerDeliveryError::SequenceExhausted;
        state.failure.get_or_insert(error);
        task.borrow_mut().error = Some(error);
        return Err(error);
    };
    state.next_sequence = sequence;
    let ticket = OwnerReportTicket {
        incarnation: state.incarnation,
        sequence,
    };
    task.borrow_mut().ticket = Some(ticket);
    drop(state);
    queue_callback_item(&owner, QueueItem::ReportResume(ticket))?;
    if !has_delivery_phase() && !DELIVERY.with(|delivery| delivery.borrow().is_some()) {
        drain_current()?;
    }
    if owner
        .0
        .borrow()
        .reports
        .iter()
        .any(|item| Rc::ptr_eq(item, &task))
    {
        Ok(OwnerReportDisposition::Queued(ticket))
    } else {
        Ok(OwnerReportDisposition::Complete(ticket))
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use nautilus_model::{
        events::OrderSubmitted,
        identifiers::{AccountId, TraderId},
    };

    use super::{super::*, *};
    use crate::{
        messages::execution::QueryAccount,
        msgbus::{self, Endpoint, MStr, TypedIntoHandler},
    };

    #[derive(Debug)]
    struct NeutralContext;
    struct EnteredNeutral;
    impl OwnerControlContext for NeutralContext {
        fn enter(&self) -> Result<Box<dyn EnteredOwnerControl>, OwnerDeliveryError> {
            Ok(Box::new(EnteredNeutral))
        }
    }
    impl EnteredOwnerControl for EnteredNeutral {
        fn finish(&mut self, _failed: bool) -> Result<(), OwnerDeliveryError> {
            Ok(())
        }
    }

    fn query() -> TradingCommand {
        TradingCommand::QueryAccount(QueryAccount::new(
            TraderId::from("REPORT-001"),
            None,
            AccountId::from("SIM-001"),
            UUID4::new(),
            1.into(),
            None,
            None,
        ))
    }

    struct CapacityDelivery {
        event: OrderEventAny,
        child: Option<TradingCommand>,
        endpoint: MStr<Endpoint>,
        admission: Rc<RefCell<Option<Result<(), OwnerDeliveryError>>>>,
    }
    impl OwnerExternalAction for CapacityDelivery {
        fn original_event(&self) -> OwnerExternalEvent {
            OwnerExternalEvent::Order(self.event.clone())
        }
        fn deliver(&mut self) -> Result<(), OwnerDeliveryError> {
            let Some(child) = self.child.clone() else {
                return Ok(());
            };
            let result = capture_control_command(TradingCommandMessage::new(self.endpoint, child))
                .and_then(|immediate| {
                    if immediate.is_none() {
                        Ok(())
                    } else {
                        Err(OwnerDeliveryError::UnsupportedEffect)
                    }
                });
            *self.admission.borrow_mut() = Some(result);
            result
        }
    }

    #[derive(Debug)]
    struct CapacityDriver {
        prepared: bool,
        event: OrderEventAny,
        child: Option<TradingCommand>,
        endpoint: MStr<Endpoint>,
        admission: Rc<RefCell<Option<Result<(), OwnerDeliveryError>>>>,
        completed: Rc<Cell<bool>>,
    }
    impl OwnerReportDriver for CapacityDriver {
        #[expect(
            clippy::panic_in_result_fn,
            reason = "the test checks admission inside the native action"
        )]
        fn prepare_next(&mut self) -> Result<OwnerReportNext, OwnerDeliveryError> {
            if self.prepared {
                self.completed.set(true);
                return Ok(OwnerReportNext::Complete);
            }
            self.prepared = true;
            let event = self.event.clone();
            let endpoint = self.endpoint;
            let child = self.child.clone();
            let admission = self.admission.clone();
            Ok(OwnerReportNext::Step(PreparedOwnerReportStep {
                kind: OwnerReportStepKind::CanonicalEvent,
                sources: vec![],
                event: Some(event.clone()),
                action: Box::new(move || {
                    for _ in 0..CAPACITY - 1 {
                        let immediate =
                            capture_control_command(TradingCommandMessage::new(endpoint, query()))?;
                        if immediate.is_some() {
                            return Err(OwnerDeliveryError::UnsupportedEffect);
                        }
                    }
                    assert_eq!(current_status().unwrap().0, CAPACITY);
                    let event_id = event.clone().into_boxed().id();
                    assert!(
                        capture_external(
                            event_id,
                            Box::new(CapacityDelivery {
                                event,
                                child,
                                endpoint,
                                admission
                            })
                        )?
                        .is_none()
                    );
                    Ok(())
                }),
                after_native: Box::new(|dispatch| match dispatch {
                    OwnerNativeDispatch::Returned => Ok(()),
                    OwnerNativeDispatch::Failed(error) => Err(error),
                }),
            }))
        }
    }

    #[rstest::rstest]
    #[case(false)]
    #[case(true)]
    fn test_public_report_capacity_resumes_without_readmitting_existing_task(
        #[case] new_child: bool,
    ) {
        let owner = OwnerDelivery::new();
        let _binding = owner.bind().unwrap();
        let observer = current_observer().unwrap();
        let context: Rc<dyn OwnerControlContext> = Rc::new(NeutralContext);
        let _submission = SubmissionBinding::new(context.clone()).unwrap();
        let endpoint: MStr<Endpoint> = "OwnerReport.capacity".into();
        let effects = Rc::new(Cell::new(0));
        msgbus::register_trading_command_endpoint(
            endpoint,
            TypedIntoHandler::from({
                let effects = effects.clone();
                move |_command: TradingCommand| {
                    effects.set(effects.get() + 1);
                }
            }),
        );
        let event = OrderEventAny::Submitted(OrderSubmitted {
            event_id: UUID4::new(),
            ..Default::default()
        });
        let extra = new_child.then(query);
        let admission = Rc::new(RefCell::new(None));
        let completed = Rc::new(Cell::new(false));
        let result = dispatch_incoming_report(OwnerReportSubmission::new(
            OwnerReportInput {
                reports: vec![],
                prepared_events: vec![event.clone()],
            },
            Box::new(CapacityDriver {
                prepared: false,
                event: event.clone(),
                child: extra.clone(),
                endpoint,
                admission: admission.clone(),
                completed: completed.clone(),
            }),
        ));
        if new_child {
            assert!(matches!(result, Err(OwnerDeliveryError::Capacity)));
            assert_eq!(*admission.borrow(), Some(Err(OwnerDeliveryError::Capacity)));
            assert_eq!(owner.failure(), Some(OwnerDeliveryError::Capacity));
            assert!(!completed.get());
            assert_eq!(effects.get(), 0);
            let reports = observer.retained_reports().unwrap();
            assert_eq!(reports.len(), 1);
            assert_eq!(reports[0].event.as_ref(), Some(&event));
            let cause = reports[0].cause.as_ref().unwrap();
            assert_eq!(cause.canonical_event_id, Some(event.into_boxed().id()));
            assert_eq!(cause.ticket, reports[0].ticket.unwrap());
            let retained = observer.retained_command_work().unwrap();
            assert_eq!(retained.len(), 1);
            assert_eq!(retained[0].command, extra.unwrap());
            assert_eq!(retained[0].endpoint, endpoint);
            assert_eq!(retained[0].cause.parent_event_id, cause.canonical_event_id);
            assert_eq!(retained[0].cause.incarnation, cause.incarnation);
            assert_eq!(owner.retire(), Err(OwnerDeliveryError::PendingAtRetirement));
        } else {
            assert!(matches!(result, Ok(OwnerReportDisposition::Complete(_))));
            assert!(admission.borrow().is_none());
            assert!(completed.get());
            assert_eq!(effects.get(), CAPACITY - 1);
            assert_eq!(owner.pending(), 0);
            assert_eq!(owner.failure(), None);
            assert!(observer.retained_reports().unwrap().is_empty());
            assert_eq!(owner.retire(), Ok(()));
        }
        msgbus::register_trading_command_endpoint(
            endpoint,
            TypedIntoHandler::from(|_command: TradingCommand| {}),
        );
    }
}

#[cfg(test)]
mod sequence_tests {
    use std::cell::Cell;

    use nautilus_model::{
        enums::{OrderSide, OrderStatus, OrderType, TimeInForce},
        events::OrderAccepted,
        identifiers::{AccountId, ClientOrderId, InstrumentId, StrategyId, TraderId, VenueOrderId},
        reports::OrderStatusReport,
        types::Quantity,
    };

    use super::{super::*, *};

    #[derive(Debug)]
    struct PreparedContext(Rc<Cell<usize>>);
    impl OwnerControlContext for PreparedContext {
        fn enter(&self) -> Result<Box<dyn EnteredOwnerControl>, OwnerDeliveryError> {
            self.0.set(self.0.get() + 1);
            Err(OwnerDeliveryError::ControlEnter)
        }
    }

    #[derive(Debug)]
    struct PreparedDriver {
        step: Option<PreparedOwnerReportStep>,
    }
    impl OwnerReportDriver for PreparedDriver {
        fn prepare_next(&mut self) -> Result<OwnerReportNext, OwnerDeliveryError> {
            Ok(self
                .step
                .take()
                .map_or(OwnerReportNext::Complete, OwnerReportNext::Step))
        }
    }

    #[rstest::rstest]
    fn test_public_report_sequence_refusal_retains_the_owned_prepared_step() {
        let owner = OwnerDelivery::new();
        let binding = owner.bind().unwrap();
        let observer = current_observer().unwrap();
        owner.0.borrow_mut().next_sequence = u64::MAX - 1;
        let source = OrderStatusReport::new(
            AccountId::from("SIM-001"),
            InstrumentId::from("EUR/USD.SIM"),
            Some(ClientOrderId::from("REPORT-SEQUENCE")),
            VenueOrderId::from("V-REPORT-SEQUENCE"),
            Some(OrderSide::Buy),
            OrderType::Limit,
            TimeInForce::Gtc,
            OrderStatus::Accepted,
            Quantity::from(1),
            Quantity::from(0),
            1.into(),
            1.into(),
            1.into(),
            None,
        );
        let source_ref = OwnerReportRef {
            root_report_id: source.report_id,
            source_report_id: source.report_id,
            root_index: 0,
            order_index: None,
            fill_index: None,
            position_index: None,
        };
        let event = OrderEventAny::Accepted(OrderAccepted::new(
            TraderId::from("REPORT-001"),
            StrategyId::from("Report-001"),
            source.instrument_id,
            source.client_order_id.unwrap(),
            source.venue_order_id,
            source.account_id,
            UUID4::new(),
            source.ts_accepted,
            source.ts_init,
            true,
        ));
        let target = Rc::new(Cell::new(0));
        let completion = Rc::new(Cell::new(0));
        let weak_target = Rc::downgrade(&target);
        let weak_completion = Rc::downgrade(&completion);
        let action_target = target.clone();
        let completion_target = completion.clone();
        let context_calls = Rc::new(Cell::new(0));
        let context: Rc<dyn OwnerControlContext> = Rc::new(PreparedContext(context_calls.clone()));
        let weak_context = Rc::downgrade(&context);
        let action_context = context.clone();
        let step = PreparedOwnerReportStep {
            kind: OwnerReportStepKind::CanonicalEvent,
            sources: vec![source_ref.clone()],
            event: Some(event.clone()),
            action: Box::new(move || {
                let _context = action_context;
                action_target.set(action_target.get() + 1);
                Ok(())
            }),
            after_native: Box::new(move |_native| {
                completion_target.set(completion_target.get() + 1);
                Ok(())
            }),
        };
        let result = dispatch_incoming_report(OwnerReportSubmission::new(
            OwnerReportInput {
                reports: vec![ExecutionReport::Order(Box::new(source.clone()))],
                prepared_events: vec![],
            },
            Box::new(PreparedDriver { step: Some(step) }),
        ));
        assert!(matches!(result, Err(OwnerDeliveryError::SequenceExhausted)));
        assert_eq!(owner.failure(), Some(OwnerDeliveryError::SequenceExhausted));
        assert_eq!(target.get(), 0);
        assert_eq!(completion.get(), 0);
        assert_eq!(Rc::strong_count(&target), 2);
        assert_eq!(Rc::strong_count(&completion), 2);
        assert_eq!(Rc::strong_count(&context), 2);
        assert_eq!(context_calls.get(), 0);
        drop(context);
        drop(target);
        drop(completion);
        assert!(weak_context.upgrade().is_some());
        assert_eq!(weak_target.upgrade().unwrap().get(), 0);
        assert_eq!(weak_completion.upgrade().unwrap().get(), 0);
        let retained = observer.retained_reports().unwrap();
        assert_eq!(retained.len(), 1);
        assert_eq!(retained[0].ticket.unwrap().sequence, u64::MAX);
        assert!(retained[0].cause.is_none());
        assert_eq!(retained[0].event.as_ref(), Some(&event));
        assert_eq!(retained[0].kind, Some(OwnerReportStepKind::CanonicalEvent));
        assert_eq!(retained[0].stage, OwnerReportDispatchStage::Prepared);
        assert_eq!(retained[0].error, OwnerDeliveryError::SequenceExhausted);
        assert!(retained[0].input.prepared_events.is_empty());
        let ExecutionReport::Order(original) = &retained[0].input.reports[0] else {
            panic!("original report");
        };
        assert_eq!(**original, source);
        {
            let state = owner.0.borrow();
            let report = state.reports[0].borrow();
            assert!(report.prepared.is_some());
            let prepared = report.prepared.as_ref().unwrap();
            assert_eq!(prepared.sources, vec![source_ref]);
            assert!(report.control.is_none(), "no control context is invented");
        }
        assert_eq!(owner.pending(), 1);
        assert_eq!(owner.retire(), Err(OwnerDeliveryError::PendingAtRetirement));
        drop(binding);
        drop(owner);
        assert!(weak_context.upgrade().is_none());
        assert_eq!(context_calls.get(), 0);
        assert!(weak_target.upgrade().is_none());
        assert!(weak_completion.upgrade().is_none());
        assert!(observer.retained_reports().unwrap().is_empty());
    }
}

#[cfg(test)]
mod child_tests {
    use nautilus_model::{
        events::OrderSubmitted,
        identifiers::{AccountId, TraderId},
    };

    use super::{super::*, *};
    use crate::{
        messages::execution::QueryAccount,
        msgbus::{self, Endpoint, MStr, TypedIntoHandler},
    };

    #[derive(Debug)]
    struct NeutralContext;
    struct EnteredNeutral;
    impl OwnerControlContext for NeutralContext {
        fn enter(&self) -> Result<Box<dyn EnteredOwnerControl>, OwnerDeliveryError> {
            Ok(Box::new(EnteredNeutral))
        }
    }
    impl EnteredOwnerControl for EnteredNeutral {
        fn finish(&mut self, _failed: bool) -> Result<(), OwnerDeliveryError> {
            Ok(())
        }
    }

    #[derive(Debug)]
    struct Steps(VecDeque<PreparedOwnerReportStep>);
    impl OwnerReportDriver for Steps {
        fn prepare_next(&mut self) -> Result<OwnerReportNext, OwnerDeliveryError> {
            Ok(self
                .0
                .pop_front()
                .map_or(OwnerReportNext::Complete, OwnerReportNext::Step))
        }
    }

    fn event(label: &str) -> OrderEventAny {
        OrderEventAny::Submitted(OrderSubmitted {
            event_id: UUID4::new(),
            client_order_id: label.into(),
            ..Default::default()
        })
    }

    fn step(event: OrderEventAny, action: OwnerAction) -> PreparedOwnerReportStep {
        PreparedOwnerReportStep {
            kind: OwnerReportStepKind::CanonicalEvent,
            sources: vec![],
            event: Some(event),
            action,
            after_native: Box::new(|native| match native {
                OwnerNativeDispatch::Returned => Ok(()),
                OwnerNativeDispatch::Failed(error) => Err(error),
            }),
        }
    }

    struct ChildDelivery {
        event: OrderEventAny,
        trace: Rc<RefCell<Vec<&'static str>>>,
        context: Rc<dyn OwnerControlContext>,
        commands: [TradingCommand; 2],
        endpoint: MStr<Endpoint>,
        nested: Option<OwnerReportSubmission>,
        causes: Rc<RefCell<Vec<OwnerReportCause>>>,
    }
    impl OwnerExternalAction for ChildDelivery {
        fn original_event(&self) -> OwnerExternalEvent {
            OwnerExternalEvent::Order(self.event.clone())
        }
        #[expect(
            clippy::panic_in_result_fn,
            reason = "the test checks its live callback context"
        )]
        fn deliver(&mut self) -> Result<(), OwnerDeliveryError> {
            self.trace.borrow_mut().push("A.external");
            DELIVERY.with(|delivery| {
                let delivery = delivery.borrow();
                let PhaseBinding::Report { cause, .. } = &delivery.as_ref().unwrap().binding else {
                    panic!("actual report delivery");
                };
                self.causes.borrow_mut().push(cause.clone());
            });
            let _factory = SubmissionBinding::new(self.context.clone())?;
            assert!(
                capture_control_command(TradingCommandMessage::new(
                    self.endpoint,
                    self.commands[0].clone()
                ))?
                .is_none()
            );
            assert!(matches!(
                dispatch_incoming_report(self.nested.take().unwrap())?,
                OwnerReportDisposition::Queued(_)
            ));
            assert!(
                capture_control_command(TradingCommandMessage::new(
                    self.endpoint,
                    self.commands[1].clone()
                ))?
                .is_none()
            );
            Ok(())
        }
    }

    #[rstest::rstest]
    fn test_public_report_callback_report_and_command_children_keep_fifo_before_resume() {
        let owner = OwnerDelivery::new();
        let _bound = owner.bind().unwrap();
        let trace = Rc::new(RefCell::new(Vec::new()));
        let causes = Rc::new(RefCell::new(Vec::new()));
        let child_causes = Rc::new(RefCell::new(Vec::new()));
        let parent_a = event("REPORT-PARENT-A");
        let parent_b = event("REPORT-PARENT-B");
        let nested_event = event("REPORT-NESTED");
        let endpoint: MStr<Endpoint> = "OwnerReport.childFIFO".into();
        let make_query = || {
            TradingCommand::QueryAccount(QueryAccount::new(
                TraderId::from("REPORT-001"),
                None,
                AccountId::from("SIM-001"),
                UUID4::new(),
                1.into(),
                None,
                None,
            ))
        };
        let commands = [make_query(), make_query()];
        msgbus::register_trading_command_endpoint(
            endpoint,
            TypedIntoHandler::from({
                let trace = trace.clone();
                let child_causes = child_causes.clone();
                let first = commands[0].clone();
                move |command: TradingCommand| {
                    trace.borrow_mut().push(if command == first {
                        "child.1"
                    } else {
                        "child.2"
                    });
                    child_causes
                        .borrow_mut()
                        .push(current_continuation().unwrap().0.cause);
                }
            }),
        );
        let nested = OwnerReportSubmission::new(
            OwnerReportInput {
                reports: vec![],
                prepared_events: vec![nested_event.clone()],
            },
            Box::new(Steps(VecDeque::from([step(
                nested_event.clone(),
                Box::new({
                    let trace = trace.clone();
                    let causes = causes.clone();
                    move || {
                        trace.borrow_mut().push("nested.native");
                        PHASE.with(|phase| {
                            let phase = phase.borrow();
                            let PhaseBinding::Report { cause, control } =
                                &phase.as_ref().unwrap().binding
                            else {
                                panic!("actual nested report phase");
                            };
                            assert!(
                                control.is_none(),
                                "SUBMISSION does not grant control to a new report"
                            );
                            causes.borrow_mut().push(cause.clone());
                        });
                        Ok(())
                    }
                }),
            )]))),
        );
        let delivery = ChildDelivery {
            event: parent_a.clone(),
            trace: trace.clone(),
            context: Rc::new(NeutralContext),
            commands: commands.clone(),
            endpoint,
            nested: Some(nested),
            causes: causes.clone(),
        };
        let a_step = step(
            parent_a.clone(),
            Box::new({
                let trace = trace.clone();
                let parent_a = parent_a.clone();
                move || {
                    trace.borrow_mut().push("A.native");
                    assert!(
                        capture_external(parent_a.clone().into_boxed().id(), Box::new(delivery))?
                            .is_none()
                    );
                    Ok(())
                }
            }),
        );
        let b_step = step(
            parent_b.clone(),
            Box::new({
                let trace = trace.clone();
                move || {
                    trace.borrow_mut().push("B.native");
                    Ok(())
                }
            }),
        );
        let result = dispatch_incoming_report(OwnerReportSubmission::new(
            OwnerReportInput {
                reports: vec![],
                prepared_events: vec![parent_a.clone(), parent_b],
            },
            Box::new(Steps(VecDeque::from([a_step, b_step]))),
        ))
        .unwrap();
        let OwnerReportDisposition::Complete(parent_ticket) = result else {
            panic!("parent must complete after its children");
        };
        assert_eq!(
            &*trace.borrow(),
            &[
                "A.native",
                "A.external",
                "child.1",
                "nested.native",
                "child.2",
                "B.native"
            ]
        );
        let causes = causes.borrow();
        assert_eq!(causes.len(), 2);
        let root = &causes[0];
        let nested = &causes[1];
        assert_eq!(root.ticket, parent_ticket);
        assert_eq!(root.canonical_event_id, Some(parent_a.into_boxed().id()));
        assert!(root.parent_event_id.is_none());
        assert!(root.parent_report_ticket.is_none());
        assert_eq!(nested.parent_event_id, root.canonical_event_id);
        assert_eq!(nested.parent_report_ticket, Some(root.ticket));
        assert_eq!(
            nested.canonical_event_id,
            Some(nested_event.into_boxed().id())
        );
        assert_eq!(nested.incarnation, root.incarnation);
        assert_ne!(nested.ticket, root.ticket);
        assert!(nested.sequence > root.sequence);
        assert!(nested.control_origin.is_none());
        let children = child_causes.borrow();
        assert_eq!(children.len(), 2);
        for (cause, command) in children.iter().zip(&commands) {
            let TradingCommand::QueryAccount(command) = command else {
                unreachable!()
            };
            assert_eq!(cause.command_id, command.command_id);
            assert_eq!(cause.parent_event_id, root.canonical_event_id);
            assert_eq!(cause.incarnation, root.incarnation);
        }
        assert!(children[0].sequence < children[1].sequence);
        assert_eq!(owner.pending(), 0);
        assert_eq!(owner.failure(), None);
        assert_eq!(owner.retire(), Ok(()));
        msgbus::register_trading_command_endpoint(
            endpoint,
            TypedIntoHandler::from(|_command: TradingCommand| {}),
        );
    }
    struct ControlReportDelivery {
        event: OrderEventAny,
        action: Option<OwnerAction>,
    }

    impl OwnerExternalAction for ControlReportDelivery {
        fn original_event(&self) -> OwnerExternalEvent {
            OwnerExternalEvent::Order(self.event.clone())
        }

        fn deliver(&mut self) -> Result<(), OwnerDeliveryError> {
            self.action.take().unwrap()()
        }
    }

    #[rstest::rstest]
    #[case::native(false)]
    #[case::callback(true)]
    fn test_public_report_control_native_children_keep_fifo_before_ancestor_resume(
        #[case] callback_children: bool,
    ) {
        let owner = OwnerDelivery::new();
        let _bound = owner.bind().unwrap();
        let trace = Rc::new(RefCell::new(Vec::new()));
        let causes = Rc::new(RefCell::new(Vec::new()));
        let context: Rc<dyn OwnerControlContext> = Rc::new(NeutralContext);
        let a = event("CONTROL-REPORT-A");
        let b = event("CONTROL-REPORT-B");
        let nested_event = event("CONTROL-REPORT-NESTED");
        let control_event = event("CONTROL-NATIVE-OUTPUT");
        let endpoint: MStr<Endpoint> = "OwnerReport.controlNativeFIFO".into();
        let make_query = || {
            TradingCommand::QueryAccount(QueryAccount::new(
                TraderId::from("REPORT-001"),
                None,
                AccountId::from("SIM-001"),
                UUID4::new(),
                1.into(),
                None,
                None,
            ))
        };
        let commands = [make_query(), make_query(), make_query(), make_query()];
        let nested = Rc::new(RefCell::new(Some(OwnerReportSubmission::new(
            OwnerReportInput {
                reports: vec![],
                prepared_events: vec![nested_event.clone()],
            },
            Box::new(Steps(VecDeque::from([step(
                nested_event,
                Box::new({
                    let trace = trace.clone();
                    let causes = causes.clone();
                    let context = context.clone();
                    move || {
                        trace.borrow_mut().push("nested.native");
                        PHASE.with(|phase| {
                            let phase = phase.borrow();
                            let PhaseBinding::Report { cause, control } =
                                &phase.as_ref().unwrap().binding
                            else {
                                panic!("actual nested report binding");
                            };
                            assert!(Rc::ptr_eq(&control.as_ref().unwrap().context, &context));
                            causes.borrow_mut().push(cause.clone());
                        });
                        Ok(())
                    }
                }),
            )]))),
        ))));
        msgbus::register_trading_command_endpoint(
            endpoint,
            TypedIntoHandler::from({
                let trace = trace.clone();
                let commands = commands.clone();
                let context = context.clone();
                move |command: TradingCommand| {
                    let original = TradingCommandMessage::new(endpoint, command);
                    let Some(original) = capture_registered_control_command(original).unwrap()
                    else {
                        return;
                    };
                    let command = original.command();
                    let current = current_continuation().unwrap();
                    assert!(Rc::ptr_eq(&current.0.context, &context));
                    if command == &commands[0] {
                        trace.borrow_mut().push("control.native");
                        if !callback_children {
                            msgbus::send_trading_command(endpoint, commands[1].clone());
                            let original = nested.borrow_mut().take().unwrap();
                            assert!(matches!(
                                dispatch_incoming_report(original).unwrap(),
                                OwnerReportDisposition::Queued(_)
                            ));
                            msgbus::send_trading_command(endpoint, commands[2].clone());
                        }
                        current
                            .schedule(
                                true,
                                Box::new({
                                    let trace = trace.clone();
                                    move || {
                                        trace.borrow_mut().push("control.continuation");
                                        Ok(())
                                    }
                                }),
                            )
                            .unwrap();
                        let callback_command = commands[3].clone();
                        let callback_commands = commands.clone();
                        let callback_nested = nested.clone();
                        assert!(
                            capture_external(
                                control_event.clone().into_boxed().id(),
                                Box::new(ControlReportDelivery {
                                    event: control_event.clone(),
                                    action: Some(Box::new({
                                        let trace = trace.clone();
                                        move || {
                                            trace.borrow_mut().push("control.external");
                                            if callback_children {
                                                msgbus::send_trading_command(
                                                    endpoint,
                                                    callback_commands[1].clone(),
                                                );
                                                let report =
                                                    callback_nested.borrow_mut().take().unwrap();
                                                assert!(matches!(
                                                    dispatch_incoming_report(report).unwrap(),
                                                    OwnerReportDisposition::Queued(_)
                                                ));
                                                msgbus::send_trading_command(
                                                    endpoint,
                                                    callback_commands[2].clone(),
                                                );
                                            }
                                            msgbus::send_trading_command(
                                                endpoint,
                                                callback_command,
                                            );
                                            Ok(())
                                        }
                                    })),
                                })
                            )
                            .unwrap()
                            .is_none()
                        );
                    } else {
                        let index = commands
                            .iter()
                            .position(|expected| expected == command)
                            .unwrap();
                        trace
                            .borrow_mut()
                            .push(["unused", "child.1", "child.2", "callback.child"][index]);
                    }
                }
            }),
        );
        let delivery = ControlReportDelivery {
            event: a.clone(),
            action: Some(Box::new({
                let trace = trace.clone();
                let context = context.clone();
                let command = commands[0].clone();
                move || {
                    trace.borrow_mut().push("A.external");
                    let _factory = SubmissionBinding::new(context)?;
                    assert!(
                        capture_control_command(TradingCommandMessage::new(endpoint, command))?
                            .is_none()
                    );
                    Ok(())
                }
            })),
        };
        let result = dispatch_incoming_report(OwnerReportSubmission::new(
            OwnerReportInput {
                reports: vec![],
                prepared_events: vec![a.clone(), b.clone()],
            },
            Box::new(Steps(VecDeque::from([
                step(
                    a.clone(),
                    Box::new({
                        let trace = trace.clone();
                        move || {
                            trace.borrow_mut().push("A.native");
                            assert!(
                                capture_external(a.into_boxed().id(), Box::new(delivery))?
                                    .is_none()
                            );
                            Ok(())
                        }
                    }),
                ),
                step(
                    b,
                    Box::new({
                        let trace = trace.clone();
                        move || {
                            trace.borrow_mut().push("B.native");
                            Ok(())
                        }
                    }),
                ),
            ]))),
        ))
        .unwrap();
        let OwnerReportDisposition::Complete(root_ticket) = result else {
            panic!("actual parent completion");
        };
        let expected: &[&str] = if callback_children {
            &[
                "A.native",
                "A.external",
                "control.native",
                "control.external",
                "control.continuation",
                "child.1",
                "nested.native",
                "child.2",
                "callback.child",
                "B.native",
            ]
        } else {
            &[
                "A.native",
                "A.external",
                "control.native",
                "control.external",
                "child.1",
                "nested.native",
                "child.2",
                "control.continuation",
                "callback.child",
                "B.native",
            ]
        };
        assert_eq!(&*trace.borrow(), expected);
        let causes = causes.borrow();
        assert_eq!(causes.len(), 1);
        assert_eq!(causes[0].parent_report_ticket, Some(root_ticket));
        assert!(causes[0].control_origin.is_some());
        assert_eq!(owner.pending(), 0);
        assert_eq!(owner.failure(), None);
        assert_eq!(owner.retire(), Ok(()));
        msgbus::register_trading_command_endpoint(
            endpoint,
            TypedIntoHandler::from(|_command: TradingCommand| {}),
        );
    }
    #[rstest::rstest]
    fn test_public_report_registered_child_uses_nearest_report_and_preserves_plain_control_order() {
        let owner = OwnerDelivery::new();
        let _bound = owner.bind().unwrap();
        let trace = Rc::new(RefCell::new(Vec::new()));
        let causes = Rc::new(RefCell::new(Vec::new()));
        let context: Rc<dyn OwnerControlContext> = Rc::new(NeutralContext);
        let root_event = event("PLAIN-CONTROL-REPORT");
        let leaf_event = event("NEAREST-REPORT-LEAF");
        let plain_event = event("PLAIN-CONTROL-EXTERNAL");
        let endpoint: MStr<Endpoint> = "OwnerReport.nearestControlFIFO".into();
        let make_query = || {
            TradingCommand::QueryAccount(QueryAccount::new(
                TraderId::from("REPORT-001"),
                None,
                AccountId::from("SIM-001"),
                UUID4::new(),
                1.into(),
                None,
                None,
            ))
        };
        let commands = [make_query(), make_query(), make_query(), make_query()];
        let leaf = Rc::new(RefCell::new(Some(OwnerReportSubmission::new(
            OwnerReportInput {
                reports: vec![],
                prepared_events: vec![leaf_event.clone()],
            },
            Box::new(Steps(VecDeque::from([step(
                leaf_event,
                Box::new({
                    let trace = trace.clone();
                    let causes = causes.clone();
                    let context = context.clone();
                    move || {
                        trace.borrow_mut().push("leaf.native");
                        PHASE.with(|phase| {
                            let phase = phase.borrow();
                            let PhaseBinding::Report { cause, control } =
                                &phase.as_ref().unwrap().binding
                            else {
                                panic!("leaf report");
                            };
                            assert!(Rc::ptr_eq(&control.as_ref().unwrap().context, &context));
                            causes.borrow_mut().push(cause.clone());
                        });
                        Ok(())
                    }
                }),
            )]))),
        ))));
        let root = Rc::new(RefCell::new(Some(OwnerReportSubmission::new(
            OwnerReportInput {
                reports: vec![],
                prepared_events: vec![root_event.clone()],
            },
            Box::new(Steps(VecDeque::from([step(
                root_event.clone(),
                Box::new({
                    let trace = trace.clone();
                    let causes = causes.clone();
                    let context = context.clone();
                    let command = commands[3].clone();
                    let root_event = root_event.clone();
                    move || {
                        trace.borrow_mut().push("report.native");
                        PHASE.with(|phase| {
                            let phase = phase.borrow();
                            let PhaseBinding::Report { cause, control } =
                                &phase.as_ref().unwrap().binding
                            else {
                                panic!("root report");
                            };
                            assert!(
                                control.as_ref().unwrap().report_ancestor.is_none(),
                                "older Control binding has no report ancestor"
                            );
                            assert!(Rc::ptr_eq(&control.as_ref().unwrap().context, &context));
                            causes.borrow_mut().push(cause.clone());
                        });
                        assert!(
                            capture_external(
                                root_event.clone().into_boxed().id(),
                                Box::new(ControlReportDelivery {
                                    event: root_event,
                                    action: Some(Box::new(move || {
                                        trace.borrow_mut().push("report.external");
                                        msgbus::send_trading_command(endpoint, command);
                                        Ok(())
                                    })),
                                })
                            )?
                            .is_none()
                        );
                        Ok(())
                    }
                }),
            )]))),
        ))));
        msgbus::register_trading_command_endpoint(
            endpoint,
            TypedIntoHandler::from({
                let trace = trace.clone();
                let commands = commands.clone();
                let context = context.clone();
                move |command: TradingCommand| {
                    let Some(original) = capture_registered_control_command(
                        TradingCommandMessage::new(endpoint, command),
                    )
                    .unwrap() else {
                        return;
                    };
                    let command = original.command();
                    let current = current_continuation().unwrap();
                    assert!(Rc::ptr_eq(&current.0.context, &context));
                    if command == &commands[0] {
                        assert!(current.0.report_ancestor.is_none());
                        trace.borrow_mut().push("ordinary.root");
                        assert!(matches!(
                            dispatch_incoming_report(root.borrow_mut().take().unwrap()).unwrap(),
                            OwnerReportDisposition::Queued(_)
                        ));
                        current
                            .schedule(
                                true,
                                Box::new({
                                    let trace = trace.clone();
                                    move || {
                                        trace.borrow_mut().push("ordinary.native.continuation");
                                        Ok(())
                                    }
                                }),
                            )
                            .unwrap();
                        let callback_command = commands[2].clone();
                        assert!(
                            capture_external(
                                plain_event.clone().into_boxed().id(),
                                Box::new(ControlReportDelivery {
                                    event: plain_event.clone(),
                                    action: Some(Box::new({
                                        let trace = trace.clone();
                                        move || {
                                            trace.borrow_mut().push("ordinary.external");
                                            msgbus::send_trading_command(
                                                endpoint,
                                                callback_command,
                                            );
                                            Ok(())
                                        }
                                    })),
                                })
                            )
                            .unwrap()
                            .is_none()
                        );
                    } else if command == &commands[1] {
                        trace.borrow_mut().push("already.queued");
                    } else if command == &commands[2] {
                        trace.borrow_mut().push("ordinary.callback.child");
                        assert!(current.0.report_ancestor.is_none());
                    } else {
                        assert_eq!(command, &commands[3]);
                        trace.borrow_mut().push("registered.child");
                        assert!(current.0.report_ancestor.is_some());
                        let report = leaf.borrow_mut().take().unwrap();
                        assert!(matches!(
                            dispatch_incoming_report(report).unwrap(),
                            OwnerReportDisposition::Queued(_)
                        ));
                    }
                }
            }),
        );
        {
            let _factory = SubmissionBinding::new(context).unwrap();
            for command in &commands[..2] {
                assert!(
                    capture_control_command(TradingCommandMessage::new(endpoint, command.clone()))
                        .unwrap()
                        .is_none()
                );
            }
        }
        owner.drain().unwrap();
        assert_eq!(
            &*trace.borrow(),
            &[
                "ordinary.root",
                "ordinary.external",
                "ordinary.native.continuation",
                "already.queued",
                "report.native",
                "report.external",
                "registered.child",
                "leaf.native",
                "ordinary.callback.child",
            ]
        );
        let causes = causes.borrow();
        assert_eq!(causes.len(), 2);
        let root = &causes[0];
        let leaf = &causes[1];
        assert!(root.parent_report_ticket.is_none());
        assert!(root.control_origin.is_some());
        assert_eq!(leaf.parent_report_ticket, Some(root.ticket));
        assert_eq!(leaf.parent_event_id, Some(root_event.into_boxed().id()));
        let TradingCommand::QueryAccount(command) = &commands[3] else {
            unreachable!()
        };
        assert_eq!(
            leaf.control_origin.as_ref().unwrap().command_id,
            command.command_id
        );
        assert_eq!(leaf.incarnation, root.incarnation);
        assert!(leaf.sequence > root.sequence);
        assert_eq!(owner.pending(), 0);
        assert_eq!(owner.failure(), None);
        assert_eq!(owner.retire(), Ok(()));
        msgbus::register_trading_command_endpoint(
            endpoint,
            TypedIntoHandler::from(|_command: TradingCommand| {}),
        );
    }
    #[rstest::rstest]
    #[case::resume_only(false)]
    #[case::new_child_refused(true)]
    fn test_public_report_control_native_capacity_keeps_original_children_and_report(
        #[case] extra_child: bool,
    ) {
        let owner = OwnerDelivery::new();
        let _bound = owner.bind().unwrap();
        let observer = current_observer().unwrap();
        let context: Rc<dyn OwnerControlContext> = Rc::new(NeutralContext);
        let endpoint: MStr<Endpoint> = "OwnerReport.nativeCapacity".into();
        let a = event("CONTROL-CAPACITY-A");
        let b = event("CONTROL-CAPACITY-B");
        let nested_event = event("CONTROL-CAPACITY-NESTED");
        let make_query = || {
            TradingCommand::QueryAccount(QueryAccount::new(
                TraderId::from("REPORT-001"),
                None,
                AccountId::from("SIM-001"),
                UUID4::new(),
                1.into(),
                None,
                None,
            ))
        };
        let root_command = make_query();
        let commands: Vec<_> = (0..CAPACITY - 2 + usize::from(extra_child))
            .map(|_| make_query())
            .collect();
        let effects = Rc::new(std::cell::Cell::new(0));
        let nested_effect = Rc::new(std::cell::Cell::new(0));
        let weak_nested = Rc::downgrade(&nested_effect);
        let b_effect = Rc::new(std::cell::Cell::new(0));
        let admission = Rc::new(RefCell::new(None));
        let nested = Rc::new(RefCell::new(Some(OwnerReportSubmission::new(
            OwnerReportInput {
                reports: vec![],
                prepared_events: vec![nested_event.clone()],
            },
            Box::new(Steps(VecDeque::from([step(
                nested_event.clone(),
                Box::new({
                    let effect = nested_effect.clone();
                    move || {
                        effect.set(effect.get() + 1);
                        Ok(())
                    }
                }),
            )]))),
        ))));
        msgbus::register_trading_command_endpoint(
            endpoint,
            TypedIntoHandler::from({
                let root_command = root_command.clone();
                let commands = commands.clone();
                let admission = admission.clone();
                let effects = effects.clone();
                move |command: TradingCommand| {
                    let Some(message) = capture_registered_control_command(
                        TradingCommandMessage::new(endpoint, command),
                    )
                    .unwrap() else {
                        return;
                    };
                    if message.command() == &root_command {
                        for command in &commands[..CAPACITY - 2] {
                            assert!(
                                capture_control_command(TradingCommandMessage::new(
                                    endpoint,
                                    command.clone()
                                ))
                                .unwrap()
                                .is_none()
                            );
                        }
                        let report = nested.borrow_mut().take().unwrap();
                        assert!(matches!(
                            dispatch_incoming_report(report).unwrap(),
                            OwnerReportDisposition::Queued(_)
                        ));
                        if extra_child {
                            *admission.borrow_mut() = Some(
                                capture_control_command(TradingCommandMessage::new(
                                    endpoint,
                                    commands.last().unwrap().clone(),
                                ))
                                .map(|original| {
                                    assert!(original.is_none());
                                }),
                            );
                        }
                    } else {
                        assert_eq!(Some(message.command()), commands.get(effects.get()));
                        effects.set(effects.get() + 1);
                    }
                }
            }),
        );
        let delivery = ControlReportDelivery {
            event: a.clone(),
            action: Some(Box::new({
                let context = context.clone();
                let command = root_command;
                move || {
                    let _factory = SubmissionBinding::new(context)?;
                    assert!(
                        capture_control_command(TradingCommandMessage::new(endpoint, command))?
                            .is_none()
                    );
                    Ok(())
                }
            })),
        };
        let result = dispatch_incoming_report(OwnerReportSubmission::new(
            OwnerReportInput {
                reports: vec![],
                prepared_events: vec![a.clone(), b.clone()],
            },
            Box::new(Steps(VecDeque::from([
                step(
                    a.clone(),
                    Box::new({
                        let a = a.clone();
                        move || {
                            assert!(
                                capture_external(a.into_boxed().id(), Box::new(delivery))?
                                    .is_none()
                            );
                            Ok(())
                        }
                    }),
                ),
                step(
                    b,
                    Box::new({
                        let b_effect = b_effect.clone();
                        move || {
                            b_effect.set(b_effect.get() + 1);
                            Ok(())
                        }
                    }),
                ),
            ]))),
        ));
        if extra_child {
            assert!(matches!(result, Err(OwnerDeliveryError::Capacity)));
            assert_eq!(*admission.borrow(), Some(Err(OwnerDeliveryError::Capacity)));
            assert_eq!(owner.failure(), Some(OwnerDeliveryError::Capacity));
            assert_eq!(effects.get(), 0);
            assert_eq!(b_effect.get(), 0);
            assert_eq!(nested_effect.get(), 0);
            let originals = observer.retained_command_work().unwrap();
            assert_eq!(originals.len(), CAPACITY - 1);
            for (held, expected) in originals.iter().zip(&commands) {
                assert_eq!(&held.command, expected);
                assert_eq!(held.endpoint, endpoint);
                assert_eq!(
                    held.cause.parent_event_id,
                    Some(a.clone().into_boxed().id())
                );
            }
            assert!(
                originals
                    .windows(2)
                    .all(|pair| pair[0].cause.sequence < pair[1].cause.sequence)
            );
            let reports = observer.retained_reports().unwrap();
            assert_eq!(reports.len(), 2);
            assert_eq!(reports[1].input.prepared_events, vec![nested_event]);
            assert!(reports[1].ticket.is_some());
            assert!(reports[1].cause.is_none());
            assert_eq!(reports[1].error, OwnerDeliveryError::Capacity);
            let state = owner.0.borrow();
            let nested_task = state.reports[1].borrow();
            assert!(Rc::ptr_eq(
                &nested_task.control.as_ref().unwrap().context,
                &context
            ));
            assert_eq!(nested_task.parent_report_ticket, reports[0].ticket);
            assert_eq!(weak_nested.upgrade().unwrap().get(), 0);
            drop(nested_task);
            drop(state);
            assert!(owner.pending() > 0);
            assert_eq!(owner.retire(), Err(OwnerDeliveryError::PendingAtRetirement));
        } else {
            assert!(matches!(result, Ok(OwnerReportDisposition::Complete(_))));
            assert!(admission.borrow().is_none());
            assert_eq!(effects.get(), CAPACITY - 2);
            assert_eq!(nested_effect.get(), 1);
            assert_eq!(b_effect.get(), 1);
            assert_eq!(owner.failure(), None);
            assert_eq!(owner.pending(), 0);
            assert_eq!(owner.retire(), Ok(()));
        }
        msgbus::register_trading_command_endpoint(
            endpoint,
            TypedIntoHandler::from(|_command: TradingCommand| {}),
        );
    }
}

#[cfg(test)]
mod portfolio_reply_tests {
    use nautilus_model::events::OrderSubmitted;

    use super::{super::*, *};

    impl ReportPortfolioReceiver {
        fn finish_update(
            &self,
            event_id: UUID4,
            branch: ReportPortfolioBranch,
            outcome: ReportPortfolioOutcome,
        ) -> Result<(), OwnerDeliveryError> {
            let entered = self
                .begin_update(event_id, branch)?
                .ok_or(OwnerDeliveryError::UnsupportedEffect)?;
            entered.finish(outcome)
        }
    }

    #[derive(Debug)]
    struct OneStep(Option<PreparedOwnerReportStep>);
    impl OwnerReportDriver for OneStep {
        fn prepare_next(&mut self) -> Result<OwnerReportNext, OwnerDeliveryError> {
            Ok(self
                .0
                .take()
                .map_or(OwnerReportNext::Complete, OwnerReportNext::Step))
        }
    }

    #[rstest::rstest]
    #[case("complete")]
    #[case("wrong-id")]
    #[case("wrong-branch")]
    #[case("duplicate")]
    #[case("missing")]
    #[case("replacement")]
    #[case("outer-before-inner")]
    #[case("nested-wrong")]
    #[case("nested-drop")]
    #[case("nested-panic")]
    #[case("nested-same-id")]
    fn test_report_portfolio_reply_is_receiver_frame_and_lifo_bound(#[case] mode: &'static str) {
        let target = Rc::new(());
        let receiver = ReportPortfolioReceiver::register(&target);
        let owner = OwnerDelivery::new();
        let _binding = owner.bind().unwrap();
        let event = OrderEventAny::Submitted(OrderSubmitted {
            event_id: UUID4::new(),
            ..Default::default()
        });
        let event_id = event.clone().into_boxed().id();
        let action = Box::new(move || {
            let outer =
                begin_report_portfolio_update(event_id, ReportPortfolioBranch::Order)?.unwrap();
            match mode {
                "wrong-id" => assert_eq!(
                    receiver.finish_update(
                        UUID4::new(),
                        ReportPortfolioBranch::Order,
                        ReportPortfolioOutcome::Complete
                    ),
                    Err(OwnerDeliveryError::UnsupportedEffect)
                ),
                "wrong-branch" => assert_eq!(
                    receiver.finish_update(
                        event_id,
                        ReportPortfolioBranch::Position,
                        ReportPortfolioOutcome::Complete
                    ),
                    Err(OwnerDeliveryError::UnsupportedEffect)
                ),
                "duplicate" => {
                    receiver.finish_update(
                        event_id,
                        ReportPortfolioBranch::Order,
                        ReportPortfolioOutcome::Complete,
                    )?;
                    assert_eq!(
                        receiver.finish_update(
                            event_id,
                            ReportPortfolioBranch::Order,
                            ReportPortfolioOutcome::Complete
                        ),
                        Err(OwnerDeliveryError::UnsupportedEffect)
                    );
                }
                "missing" => {}
                "replacement" => {
                    let other_target = Rc::new(());
                    let other = ReportPortfolioReceiver::register(&other_target);
                    assert_eq!(
                        other.finish_update(
                            event_id,
                            ReportPortfolioBranch::Order,
                            ReportPortfolioOutcome::Complete
                        ),
                        Err(OwnerDeliveryError::UnsupportedEffect)
                    );
                }
                "outer-before-inner" => {
                    receiver.finish_update(
                        event_id,
                        ReportPortfolioBranch::Order,
                        ReportPortfolioOutcome::Complete,
                    )?;
                    let inner =
                        begin_report_portfolio_update(event_id, ReportPortfolioBranch::Order)?
                            .unwrap();
                    assert!(matches!(outer.finish(), ReportPortfolioOutcome::Unobserved));
                    receiver.finish_update(
                        event_id,
                        ReportPortfolioBranch::Order,
                        ReportPortfolioOutcome::Complete,
                    )?;
                    assert!(matches!(inner.finish(), ReportPortfolioOutcome::Complete));
                    return Ok(());
                }
                "nested-wrong" => {
                    let inner =
                        begin_report_portfolio_update(event_id, ReportPortfolioBranch::Position)?
                            .unwrap();
                    assert_eq!(
                        receiver.finish_update(
                            event_id,
                            ReportPortfolioBranch::Order,
                            ReportPortfolioOutcome::Complete
                        ),
                        Err(OwnerDeliveryError::UnsupportedEffect)
                    );
                    assert!(matches!(inner.finish(), ReportPortfolioOutcome::Unobserved));
                    receiver.finish_update(
                        event_id,
                        ReportPortfolioBranch::Order,
                        ReportPortfolioOutcome::Complete,
                    )?;
                }
                "nested-drop" => {
                    let inner =
                        begin_report_portfolio_update(event_id, ReportPortfolioBranch::Order)?
                            .unwrap();
                    drop(inner);
                    receiver.finish_update(
                        event_id,
                        ReportPortfolioBranch::Order,
                        ReportPortfolioOutcome::Complete,
                    )?;
                }
                "nested-panic" => {
                    let result = catch_unwind(AssertUnwindSafe(|| {
                        let _inner =
                            begin_report_portfolio_update(event_id, ReportPortfolioBranch::Order)
                                .unwrap()
                                .unwrap();
                        panic!("owned receipt is dropped during unwind");
                    }));
                    assert!(result.is_err());
                    receiver.finish_update(
                        event_id,
                        ReportPortfolioBranch::Order,
                        ReportPortfolioOutcome::Complete,
                    )?;
                }
                "nested-same-id" => {
                    let inner =
                        begin_report_portfolio_update(event_id, ReportPortfolioBranch::Order)?
                            .unwrap();
                    receiver.finish_update(
                        event_id,
                        ReportPortfolioBranch::Order,
                        ReportPortfolioOutcome::Complete,
                    )?;
                    assert!(matches!(inner.finish(), ReportPortfolioOutcome::Complete));
                    receiver.finish_update(
                        event_id,
                        ReportPortfolioBranch::Order,
                        ReportPortfolioOutcome::Complete,
                    )?;
                }
                "complete" => receiver.finish_update(
                    event_id,
                    ReportPortfolioBranch::Order,
                    ReportPortfolioOutcome::Complete,
                )?,
                _ => unreachable!("fixed case list"),
            }
            let result = outer.finish();
            if matches!(
                mode,
                "complete" | "nested-wrong" | "nested-drop" | "nested-panic" | "nested-same-id"
            ) {
                assert!(matches!(result, ReportPortfolioOutcome::Complete));
            } else {
                assert!(matches!(result, ReportPortfolioOutcome::Unobserved));
            }
            assert!(PORTFOLIO_REPLIES.with(|stack| stack.borrow().is_empty()));
            Ok(())
        });
        dispatch_incoming_report(OwnerReportSubmission::new(
            OwnerReportInput {
                reports: vec![],
                prepared_events: vec![event.clone()],
            },
            Box::new(OneStep(Some(PreparedOwnerReportStep {
                kind: OwnerReportStepKind::CanonicalEvent,
                sources: vec![],
                event: Some(event),
                action,
                after_native: Box::new(|dispatch| match dispatch {
                    OwnerNativeDispatch::Returned => Ok(()),
                    OwnerNativeDispatch::Failed(error) => Err(error),
                }),
            }))),
        ))
        .unwrap();
        assert_eq!(owner.failure(), None);
        assert_eq!(owner.retire(), Ok(()));
        assert!(
            begin_report_portfolio_update(event_id, ReportPortfolioBranch::Order)
                .unwrap()
                .is_none()
        );
    }
    #[rstest::rstest]
    #[case(false)]
    #[case(true)]
    fn test_report_portfolio_receipt_holds_actual_target_only_for_its_frame(#[case] expired: bool) {
        let target = Rc::new(());
        let weak = Rc::downgrade(&target);
        let receiver = ReportPortfolioReceiver::register(&target);
        let owner = OwnerDelivery::new();
        let _binding = owner.bind().unwrap();
        let event = OrderEventAny::Submitted(OrderSubmitted {
            event_id: UUID4::new(),
            ..Default::default()
        });
        let event_id = event.clone().into_boxed().id();
        let action = Box::new(move || {
            let mut target = Some(target);
            if expired {
                drop(target.take());
            }
            let receipt =
                begin_report_portfolio_update(event_id, ReportPortfolioBranch::Order)?.unwrap();
            drop(target);
            assert_eq!(weak.upgrade().is_none(), expired);
            let result = receiver.finish_update(
                event_id,
                ReportPortfolioBranch::Order,
                ReportPortfolioOutcome::Complete,
            );
            if expired {
                assert_eq!(result, Err(OwnerDeliveryError::UnsupportedEffect));
                assert!(matches!(
                    receipt.finish(),
                    ReportPortfolioOutcome::Unobserved
                ));
            } else {
                result?;
                assert!(matches!(receipt.finish(), ReportPortfolioOutcome::Complete));
            }
            assert!(
                weak.upgrade().is_none(),
                "registration cannot keep an expired receiver alive"
            );
            Ok(())
        });
        dispatch_incoming_report(OwnerReportSubmission::new(
            OwnerReportInput {
                reports: vec![],
                prepared_events: vec![event.clone()],
            },
            Box::new(OneStep(Some(PreparedOwnerReportStep {
                kind: OwnerReportStepKind::CanonicalEvent,
                sources: vec![],
                event: Some(event),
                action,
                after_native: Box::new(|dispatch| match dispatch {
                    OwnerNativeDispatch::Returned => Ok(()),
                    OwnerNativeDispatch::Failed(error) => Err(error),
                }),
            }))),
        ))
        .unwrap();
        assert_eq!(owner.retire(), Ok(()));
    }
    #[rstest::rstest]
    fn test_report_portfolio_reply_frames_refuse_capacity_without_leaking_scopes() {
        let target = Rc::new(());
        let _receiver = ReportPortfolioReceiver::register(&target);
        let owner = OwnerDelivery::new();
        let _binding = owner.bind().unwrap();
        let event = OrderEventAny::Submitted(OrderSubmitted {
            event_id: UUID4::new(),
            ..Default::default()
        });
        let event_id = event.clone().into_boxed().id();
        let result = dispatch_incoming_report(OwnerReportSubmission::new(
            OwnerReportInput {
                reports: vec![],
                prepared_events: vec![event.clone()],
            },
            Box::new(OneStep(Some(PreparedOwnerReportStep {
                kind: OwnerReportStepKind::CanonicalEvent,
                sources: vec![],
                event: Some(event),
                action: Box::new(move || {
                    let mut frames = Vec::new();
                    for _ in 0..CAPACITY {
                        frames.push(
                            begin_report_portfolio_update(event_id, ReportPortfolioBranch::Order)?
                                .unwrap(),
                        );
                    }
                    assert!(matches!(
                        begin_report_portfolio_update(event_id, ReportPortfolioBranch::Order),
                        Err(OwnerDeliveryError::Capacity)
                    ));
                    drop(frames);
                    assert!(PORTFOLIO_REPLIES.with(|stack| stack.borrow().is_empty()));
                    Err(OwnerDeliveryError::Capacity)
                }),
                after_native: Box::new(|dispatch| match dispatch {
                    OwnerNativeDispatch::Returned => Ok(()),
                    OwnerNativeDispatch::Failed(error) => Err(error),
                }),
            }))),
        ));
        assert!(matches!(result, Err(OwnerDeliveryError::Capacity)));
        assert_eq!(owner.failure(), Some(OwnerDeliveryError::Capacity));
        assert_eq!(owner.retire(), Err(OwnerDeliveryError::PendingAtRetirement));
    }
    #[rstest::rstest]
    #[case("nested-return")]
    #[case("outer-before-inner")]
    #[case("outer-drop")]
    #[case("nested-panic")]
    #[case("expired-token")]
    fn test_report_portfolio_entered_token_never_replies_to_another_frame(
        #[case] mode: &'static str,
    ) {
        let target = Rc::new(());
        let receiver = ReportPortfolioReceiver::register(&target);
        let owner = OwnerDelivery::new();
        let _binding = owner.bind().unwrap();
        let event = OrderEventAny::Submitted(OrderSubmitted {
            event_id: UUID4::new(),
            ..Default::default()
        });
        let event_id = event.clone().into_boxed().id();
        dispatch_incoming_report(OwnerReportSubmission::new(
            OwnerReportInput {
                reports: vec![],
                prepared_events: vec![event.clone()],
            },
            Box::new(OneStep(Some(PreparedOwnerReportStep {
                kind: OwnerReportStepKind::CanonicalEvent,
                sources: vec![],
                event: Some(event),
                action: Box::new(move || {
                    let outer =
                        begin_report_portfolio_update(event_id, ReportPortfolioBranch::Order)?
                            .unwrap();
                    let entered = receiver
                        .begin_update(event_id, ReportPortfolioBranch::Order)?
                        .unwrap();
                    if mode == "nested-panic" {
                        let result = catch_unwind(AssertUnwindSafe(|| {
                            let _inner = begin_report_portfolio_update(
                                event_id,
                                ReportPortfolioBranch::Order,
                            )
                            .unwrap()
                            .unwrap();
                            let _entered_inner = receiver
                                .begin_update(event_id, ReportPortfolioBranch::Order)
                                .unwrap()
                                .unwrap();
                            panic!("entered nested receiver unwinds");
                        }));
                        assert!(result.is_err());
                        entered.finish(ReportPortfolioOutcome::Complete)?;
                        assert!(matches!(outer.finish(), ReportPortfolioOutcome::Complete));
                        return Ok(());
                    }
                    if mode == "expired-token" {
                        assert!(matches!(outer.finish(), ReportPortfolioOutcome::Unobserved));
                        let next =
                            begin_report_portfolio_update(event_id, ReportPortfolioBranch::Order)?
                                .unwrap();
                        let next_entered = receiver
                            .begin_update(event_id, ReportPortfolioBranch::Order)?
                            .unwrap();
                        assert_eq!(
                            entered.finish(ReportPortfolioOutcome::Complete),
                            Err(OwnerDeliveryError::UnsupportedEffect)
                        );
                        next_entered.finish(ReportPortfolioOutcome::Complete)?;
                        assert!(matches!(next.finish(), ReportPortfolioOutcome::Complete));
                        return Ok(());
                    }
                    let inner =
                        begin_report_portfolio_update(event_id, ReportPortfolioBranch::Order)?
                            .unwrap();
                    let inner_entered = receiver
                        .begin_update(event_id, ReportPortfolioBranch::Order)?
                        .unwrap();
                    if mode == "outer-before-inner" {
                        assert_eq!(
                            entered.finish(ReportPortfolioOutcome::Complete),
                            Err(OwnerDeliveryError::UnsupportedEffect)
                        );
                        inner_entered.finish(ReportPortfolioOutcome::Complete)?;
                        assert!(matches!(inner.finish(), ReportPortfolioOutcome::Complete));
                        assert!(matches!(outer.finish(), ReportPortfolioOutcome::Unobserved));
                    } else if mode == "outer-drop" {
                        drop(entered);
                        inner_entered.finish(ReportPortfolioOutcome::Complete)?;
                        assert!(matches!(inner.finish(), ReportPortfolioOutcome::Complete));
                        assert!(matches!(outer.finish(), ReportPortfolioOutcome::Unobserved));
                    } else {
                        inner_entered.finish(ReportPortfolioOutcome::Complete)?;
                        assert!(matches!(inner.finish(), ReportPortfolioOutcome::Complete));
                        entered.finish(ReportPortfolioOutcome::Complete)?;
                        assert!(matches!(outer.finish(), ReportPortfolioOutcome::Complete));
                    }
                    assert!(PORTFOLIO_REPLIES.with(|stack| stack.borrow().is_empty()));
                    Ok(())
                }),
                after_native: Box::new(|dispatch| match dispatch {
                    OwnerNativeDispatch::Returned => Ok(()),
                    OwnerNativeDispatch::Failed(error) => Err(error),
                }),
            }))),
        ))
        .unwrap();
        assert_eq!(owner.failure(), None);
        assert_eq!(owner.retire(), Ok(()));
    }
}
