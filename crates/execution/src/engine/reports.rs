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

//! Runtime reconciliation cursor shared by synchronous and owner-local adapters.

use std::collections::VecDeque;

use nautilus_common::owner_delivery::{
    OwnerDeliveryError, OwnerNativeDispatch, OwnerReportDriver, OwnerReportInput, OwnerReportNext,
    OwnerReportRef, OwnerReportStepKind, OwnerReportSubmission, PreparedOwnerReportStep,
};

use super::{
    AHashSet, AccountId, ClientId, ClientOrderId, Debug, ExecutionEngine, ExecutionReport,
    FillReport, Instrument, InstrumentAny, InstrumentId, MessagingSwitchboard, Money, Order,
    OrderAccepted, OrderAny, OrderEventAny, OrderFilled, OrderInitialized, OrderStatus,
    OrderStatusReport, OrderType, Quantity, Rc, RefCell, StrategyId, TimeInForce, TradeId, UUID4,
    UnixNanos, Venue, VenueOrderId, generate_external_order_status_events,
    generate_reconciliation_order_events, generate_reconciliation_order_pre_fill_events,
    generate_reconciliation_order_snapshot_events, get_message_bus, msgbus, reconcile_fill,
};

#[derive(Clone, Copy, Debug)]
pub(super) enum NativeApplyStage {
    OrderLookup,
    PositionIdentity,
    Validation,
    OrderCache,
    PositionCache,
    InstrumentLookup,
    AccountLookup,
    PositionSnapshot,
    OrderSnapshot,
    Portfolio,
}

#[derive(Debug)]
pub(super) struct NativeApplyFailure {
    stage: NativeApplyStage,
    source: anyhow::Error,
}

#[derive(Clone, Copy, Debug)]
pub(super) enum ExpectedNativeNoop {
    ZeroFill,
    DuplicateFill,
    DuplicateFillVoid,
    ClosedNonFillRace,
}

#[derive(Clone, Copy, Debug)]
pub(super) enum NativeEventDisposition {
    Applied,
    Skipped(ExpectedNativeNoop),
}

#[derive(Debug)]
pub(super) struct NativeStage<T> {
    pub(super) value: T,
    failure: Option<NativeApplyFailure>,
}

pub(super) type NativeEventOutcome = NativeStage<NativeEventDisposition>;

impl<T> NativeStage<T> {
    pub(super) fn complete(value: T) -> Self {
        Self {
            value,
            failure: None,
        }
    }

    pub(super) fn record(&mut self, stage: NativeApplyStage, source: impl Into<anyhow::Error>) {
        if self.failure.is_none() {
            self.failure = Some(NativeApplyFailure {
                stage,
                source: source.into(),
            });
        }
    }

    fn into_result(self) -> anyhow::Result<T> {
        match self.failure {
            Some(failure) => Err(failure
                .source
                .context(format!("native stage {:?}", failure.stage))),
            None => Ok(self.value),
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum LegacyProgress {
    ContinueEvent,
    FinishCurrentSegment,
}

#[derive(Debug)]
struct ReportApplyFailure {
    source: anyhow::Error,
    legacy_progress: LegacyProgress,
}

impl From<anyhow::Error> for ReportApplyFailure {
    fn from(source: anyhow::Error) -> Self {
        Self {
            source,
            legacy_progress: LegacyProgress::FinishCurrentSegment,
        }
    }
}

impl ReportApplyFailure {
    fn continue_event(source: anyhow::Error) -> Self {
        Self {
            source,
            legacy_progress: LegacyProgress::ContinueEvent,
        }
    }
}

impl std::fmt::Display for ReportApplyFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.source, formatter)
    }
}

pub(super) struct PreparedExternal {
    pub(super) initialized: OrderInitialized,
    pub(super) client_order_id: ClientOrderId,
    pub(super) venue_order_id: VenueOrderId,
    pub(super) instrument_id: InstrumentId,
    pub(super) strategy_id: StrategyId,
    pub(super) ts_now: UnixNanos,
    pub(super) order_status: Option<OrderStatus>,
    pub(super) source_client_id: Option<ClientId>,
}

#[derive(Clone, Copy)]
struct MassReportLog {
    client_id: ClientId,
    account_id: AccountId,
    venue: Venue,
    orders: usize,
    fills: usize,
    positions: usize,
}

enum FilteredExternalKind {
    Order,
    Fill,
}

impl FilteredExternalKind {
    fn label(&self) -> &'static str {
        match self {
            Self::Order => "order",
            Self::Fill => "fill",
        }
    }
}

enum PreparedReportNative {
    Count(Option<MassReportLog>),
    Raw(ExecutionReport),
    RegisterExternal(PreparedExternal),
    Process(OrderEventAny),
    ProjectRetainedFill(OrderFilled),
    FilterExternal {
        kind: FilteredExternalKind,
        client_order_id: ClientOrderId,
        venue_order_id: VenueOrderId,
        instrument_id: InstrumentId,
    },
    Finalize(Option<MassReportLog>),
}

impl PreparedReportNative {
    fn kind(&self) -> OwnerReportStepKind {
        match self {
            Self::Count(_) | Self::Raw(_) | Self::FilterExternal { .. } => {
                OwnerReportStepKind::RawInputs
            }
            Self::RegisterExternal(_) => OwnerReportStepKind::ExternalInitialization,
            Self::Process(_) => OwnerReportStepKind::CanonicalEvent,
            Self::ProjectRetainedFill(_) => OwnerReportStepKind::RetainedFillProjection,
            Self::Finalize(_) => OwnerReportStepKind::Finalize,
        }
    }

    fn event(&self) -> Option<OrderEventAny> {
        match self {
            Self::RegisterExternal(external) => {
                Some(OrderEventAny::Initialized(external.initialized.clone()))
            }
            Self::Process(event) => Some(event.clone()),
            Self::ProjectRetainedFill(fill) => Some(OrderEventAny::Filled(fill.clone())),
            _ => None,
        }
    }
}

struct ReportNativeStep {
    action: PreparedReportNative,
    sources: Vec<OwnerReportRef>,
    checkpoint: Option<ReportStepCheckpoint>,
}

#[derive(Clone, Debug)]
struct ReportOrderBasis {
    order: OrderAny,
    snapshot: nautilus_model::events::OrderSnapshot,
    quantities: [Quantity; 3],
    times: [Option<UnixNanos>; 3],
    venue_ids: Vec<VenueOrderId>,
    trade_ids: Vec<TradeId>,
    history: Vec<OrderEventAny>,
    precision: Vec<Option<u8>>,
}

impl PartialEq for ReportOrderBasis {
    fn eq(&self, other: &Self) -> bool {
        self.snapshot == other.snapshot
            && self.quantities == other.quantities
            && self.times == other.times
            && self.venue_ids == other.venue_ids
            && self.trade_ids == other.trade_ids
            && self.history == other.history
            && self.precision == other.precision
    }
}

impl ReportOrderBasis {
    fn capture(order: OrderAny) -> Self {
        let history: Vec<_> = order.events().into_iter().cloned().collect();
        let quantities = [order.voided_qty(), order.leaves_qty(), order.overfill_qty()];
        let mut precision = vec![
            Some(order.quantity().precision),
            Some(order.filled_qty().precision),
            order.display_qty().map(|quantity| quantity.precision),
        ];
        precision.extend(quantities.iter().map(|quantity| Some(quantity.precision)));
        precision.extend(
            [
                order.price(),
                order.activation_price(),
                order.trigger_price(),
            ]
            .map(|price| price.map(|price| price.precision)),
        );
        for event in &history {
            match event {
                OrderEventAny::Initialized(event) => {
                    precision.push(Some(event.quantity.precision));
                    precision.push(event.display_qty.map(|quantity| quantity.precision));
                    precision.extend(
                        [event.price, event.activation_price, event.trigger_price]
                            .map(|price| price.map(|price| price.precision)),
                    );
                }
                OrderEventAny::Released(event) => {
                    precision.push(Some(event.released_price.precision));
                }
                OrderEventAny::Updated(event) => {
                    precision.push(Some(event.quantity.precision));
                    precision.extend(
                        [event.price, event.trigger_price, event.protection_price]
                            .map(|price| price.map(|price| price.precision)),
                    );
                }
                OrderEventAny::Filled(event) => precision.extend([
                    Some(event.last_qty.precision),
                    Some(event.last_px.precision),
                ]),
                OrderEventAny::FillVoided(event) => precision.extend([
                    Some(event.voided_qty.precision),
                    Some(event.last_px.precision),
                ]),
                _ => {}
            }
        }
        Self {
            snapshot: nautilus_model::events::OrderSnapshot::from(order.clone()),
            quantities,
            times: [order.ts_submitted(), order.ts_accepted(), order.ts_closed()],
            venue_ids: order.venue_order_ids().into_iter().copied().collect(),
            trade_ids: order.trade_ids().into_iter().copied().collect(),
            history,
            precision,
            order,
        }
    }
}

type ReportBasisSlot = Rc<RefCell<Result<Option<ReportOrderBasis>, String>>>;

#[derive(Clone)]
struct ReportStepCheckpoint {
    target: ClientOrderId,
    basis: ReportBasisSlot,
}

impl ReportStepCheckpoint {
    fn record(&self, engine: &ExecutionEngine) -> anyhow::Result<()> {
        let observed = engine
            .cache
            .try_borrow()
            .map_err(|error| {
                anyhow::anyhow!("Cannot observe report cache after native apply: {error}")
            })
            .map(|cache| {
                cache
                    .order_owned(&self.target)
                    .map(ReportOrderBasis::capture)
            });
        *self.basis.borrow_mut() = observed
            .as_ref()
            .map(Clone::clone)
            .map_err(ToString::to_string);
        observed.map(|_| ())
    }
}

#[derive(Clone, Copy)]
enum ReportEventPlanKind {
    OrderStatus,
    PreFill,
    Snapshot,
    ExternalStatus,
}

#[expect(
    clippy::large_enum_variant,
    reason = "the cursor transfers an owned step without an intermediate allocation"
)]
enum ReportNativeNext {
    Step(ReportNativeStep),
    Complete,
}

#[derive(Default)]
enum SegmentStage {
    #[default]
    Raw,
    Begin,
    AfterInitialize,
    Events,
    Fills(usize),
    Residual,
    Done,
}

struct ReportSegment {
    report: ExecutionReport,
    source: OwnerReportRef,
    stage: SegmentStage,
    events: VecDeque<OrderEventAny>,
    after_events: SegmentStage,
    client_order_id: Option<ClientOrderId>,
    event_plan: Option<ReportEventPlanKind>,
    checkpoint: ReportBasisSlot,
}

impl ReportSegment {
    fn new(report: ExecutionReport, source: OwnerReportRef) -> Self {
        Self {
            report,
            source,
            stage: SegmentStage::Raw,
            events: VecDeque::new(),
            after_events: SegmentStage::Done,
            client_order_id: None,
            event_plan: None,
            checkpoint: Rc::new(RefCell::new(Ok(None))),
        }
    }

    fn order_report(&self) -> Option<&OrderStatusReport> {
        match &self.report {
            ExecutionReport::Order(report) | ExecutionReport::OrderWithFills(report, _) => {
                Some(report)
            }
            _ => None,
        }
    }

    fn lookup_order(&self, engine: &ExecutionEngine) -> Option<OrderAny> {
        let (client_id, venue_id) = match &self.report {
            ExecutionReport::Order(report) | ExecutionReport::OrderWithFills(report, _) => {
                (report.client_order_id, report.venue_order_id)
            }
            ExecutionReport::Fill(report) => (report.client_order_id, report.venue_order_id),
            _ => return None,
        };
        let cache = engine.cache.borrow();
        self.client_order_id
            .or(client_id)
            .and_then(|id| cache.order(&id).map(|order| order.clone()))
            .or_else(|| {
                cache
                    .client_order_id(&venue_id)
                    .and_then(|id| cache.order(id).map(|order| order.clone()))
            })
    }

    fn instrument_id(&self) -> InstrumentId {
        match &self.report {
            ExecutionReport::Order(report) | ExecutionReport::OrderWithFills(report, _) => {
                report.instrument_id
            }
            ExecutionReport::Fill(report) => report.instrument_id,
            ExecutionReport::Position(report) => report.instrument_id,
            ExecutionReport::MassStatus(_) => {
                unreachable!("mass status is traversed by its cursor")
            }
        }
    }

    fn step_checkpoint(&self, action: &PreparedReportNative) -> Option<ReportStepCheckpoint> {
        let target = match action {
            PreparedReportNative::Process(event) => event.client_order_id(),
            PreparedReportNative::ProjectRetainedFill(fill) => fill.client_order_id,
            PreparedReportNative::RegisterExternal(external) => external.client_order_id,
            _ => return None,
        };
        Some(ReportStepCheckpoint {
            target,
            basis: self.checkpoint.clone(),
        })
    }

    fn set_event_plan(
        &mut self,
        kind: ReportEventPlanKind,
        events: Vec<OrderEventAny>,
        order: &OrderAny,
    ) {
        self.events = events.into();
        self.event_plan = Some(kind);
        *self.checkpoint.borrow_mut() = Ok(Some(ReportOrderBasis::capture(order.clone())));
    }

    fn refreshed_external_suffix(
        &self,
        order: &OrderAny,
        instrument: Option<&InstrumentAny>,
        now: UnixNanos,
    ) -> anyhow::Result<Vec<OrderEventAny>> {
        let report = self.order_report().expect("external status report");
        let mut working = order.clone();
        let mut refreshed = Vec::new();
        for goal in &self.events {
            let event = match goal {
                OrderEventAny::Accepted(_)
                    if matches!(
                        working.status(),
                        OrderStatus::Initialized | OrderStatus::Submitted
                    ) =>
                {
                    Some(OrderEventAny::Accepted(OrderAccepted::new(
                        working.trader_id(),
                        working.strategy_id(),
                        working.instrument_id(),
                        working.client_order_id(),
                        report.venue_order_id,
                        report.account_id,
                        UUID4::new(),
                        report.ts_accepted,
                        now,
                        true,
                    )))
                }
                OrderEventAny::Accepted(_) => None,
                OrderEventAny::Filled(_) if report.filled_qty > working.filled_qty() => instrument
                    .and_then(|instrument| {
                        crate::reconciliation::create_incremental_inferred_fill(
                            &working,
                            report,
                            &report.account_id,
                            instrument,
                            now,
                            None,
                        )
                    }),
                OrderEventAny::Filled(_) => None,
                OrderEventAny::Canceled(_) | OrderEventAny::Expired(_)
                    if working.filled_qty() >= report.quantity =>
                {
                    None
                }
                OrderEventAny::Canceled(_)
                | OrderEventAny::Expired(_)
                | OrderEventAny::FillVoided(_) => {
                    crate::reconciliation::reconcile_order_report(&working, report, instrument, now)
                }
                OrderEventAny::Rejected(_) if working.status() == OrderStatus::Rejected => None,
                OrderEventAny::Rejected(_) => Some(goal.clone()),
                _ => return Err(anyhow::anyhow!("Unexpected external report suffix goal")),
            };
            if let Some(event) = event {
                working.apply(event.clone()).map_err(|error| {
                    anyhow::anyhow!("Cannot project refreshed external suffix: {error}")
                })?;
                refreshed.push(event);
            }
        }
        Ok(refreshed)
    }

    fn refresh_pending_events(&mut self, engine: &ExecutionEngine) -> anyhow::Result<()> {
        if self.events.is_empty() {
            return Ok(());
        }
        let Some(kind) = self.event_plan else {
            return Ok(());
        };
        let checkpoint = self
            .checkpoint
            .borrow()
            .clone()
            .map_err(anyhow::Error::msg)?;
        let order = self
            .lookup_order(engine)
            .ok_or_else(|| anyhow::anyhow!("Report order disappeared before pending event"))?;
        let live = ReportOrderBasis::capture(order.clone());
        if checkpoint.as_ref() == Some(&live) {
            return Ok(());
        }
        let report = self.order_report().expect("event plan has order status");
        let instrument = engine
            .cache
            .borrow()
            .instrument(&report.instrument_id)
            .cloned();
        let now = engine.clock.borrow().timestamp_ns();
        let events = match kind {
            ReportEventPlanKind::OrderStatus => {
                generate_reconciliation_order_events(&order, report, instrument.as_ref(), now)
            }
            ReportEventPlanKind::PreFill => {
                generate_reconciliation_order_pre_fill_events(&order, report, now)
            }
            ReportEventPlanKind::Snapshot => generate_reconciliation_order_snapshot_events(
                &order,
                report,
                instrument.as_ref(),
                now,
            ),
            ReportEventPlanKind::ExternalStatus => {
                self.refreshed_external_suffix(&order, instrument.as_ref(), now)?
            }
        };
        self.set_event_plan(kind, events, &order);
        Ok(())
    }

    fn step(&self, action: PreparedReportNative) -> ReportNativeNext {
        let checkpoint = self.step_checkpoint(&action);
        ReportNativeNext::Step(ReportNativeStep {
            action,
            sources: vec![self.source.clone()],
            checkpoint,
        })
    }

    fn filtered_step(&self) -> ReportNativeNext {
        let (kind, client, venue_order_id, instrument_id) = match &self.report {
            ExecutionReport::Order(report) | ExecutionReport::OrderWithFills(report, _) => (
                FilteredExternalKind::Order,
                report.client_order_id,
                report.venue_order_id,
                report.instrument_id,
            ),
            ExecutionReport::Fill(report) => (
                FilteredExternalKind::Fill,
                report.client_order_id,
                report.venue_order_id,
                report.instrument_id,
            ),
            _ => unreachable!("only an external order/fill is filtered"),
        };
        self.step(PreparedReportNative::FilterExternal {
            kind,
            client_order_id: client.unwrap_or_else(|| ClientOrderId::from(venue_order_id.as_str())),
            venue_order_id,
            instrument_id,
        })
    }

    fn fill_step(
        &self,
        action: PreparedReportNative,
        index: usize,
        fill: &FillReport,
    ) -> ReportNativeNext {
        let mut source = self.source.clone();
        source.source_report_id = fill.report_id;
        source.fill_index = Some(index);
        let checkpoint = self.step_checkpoint(&action);
        ReportNativeNext::Step(ReportNativeStep {
            action,
            sources: vec![source],
            checkpoint,
        })
    }

    fn prepare(&mut self, engine: &ExecutionEngine) -> anyhow::Result<ReportNativeNext> {
        loop {
            let stage = std::mem::replace(&mut self.stage, SegmentStage::Done);
            match stage {
                SegmentStage::Raw => {
                    self.stage = SegmentStage::Begin;
                    return Ok(self.step(PreparedReportNative::Raw(self.report.clone())));
                }
                SegmentStage::Begin => {
                    if matches!(self.report, ExecutionReport::Position(_)) {
                        return Ok(ReportNativeNext::Complete);
                    }
                    if let ExecutionReport::Fill(fill) = &self.report
                        && fill.last_qty.is_zero()
                    {
                        log::warn!("Skipping zero-quantity fill report: {fill}");
                        return Ok(ReportNativeNext::Complete);
                    }
                    let order = self.lookup_order(engine);
                    let instrument = engine
                        .cache
                        .borrow()
                        .instrument(&self.instrument_id())
                        .cloned();
                    if instrument.is_none() {
                        if let Some(order) = order
                            && let Some(report) = self.order_report()
                            && !matches!(&self.report, ExecutionReport::OrderWithFills(_, fills) if !fills.is_empty())
                        {
                            let now = engine.clock.borrow().timestamp_ns();
                            self.events = if matches!(self.report, ExecutionReport::Order(_)) {
                                generate_reconciliation_order_events(&order, report, None, now)
                            } else {
                                generate_reconciliation_order_snapshot_events(
                                    &order, report, None, now,
                                )
                            }
                            .into();
                            self.event_plan =
                                Some(if matches!(self.report, ExecutionReport::Order(_)) {
                                    ReportEventPlanKind::OrderStatus
                                } else {
                                    ReportEventPlanKind::Snapshot
                                });
                            *self.checkpoint.borrow_mut() =
                                Ok(Some(ReportOrderBasis::capture(order)));
                            self.stage = SegmentStage::Events;
                            continue;
                        }
                        return Ok(ReportNativeNext::Complete);
                    }
                    if let Some(order) = order {
                        self.client_order_id = Some(order.client_order_id());
                        let now = engine.clock.borrow().timestamp_ns();
                        match &self.report {
                            ExecutionReport::Order(report) => {
                                self.events = generate_reconciliation_order_events(
                                    &order,
                                    report,
                                    instrument.as_ref(),
                                    now,
                                )
                                .into();
                                self.event_plan = Some(ReportEventPlanKind::OrderStatus);
                                *self.checkpoint.borrow_mut() =
                                    Ok(Some(ReportOrderBasis::capture(order.clone())));
                                self.stage = SegmentStage::Events;
                            }
                            ExecutionReport::OrderWithFills(report, _) => {
                                self.events = generate_reconciliation_order_pre_fill_events(
                                    &order, report, now,
                                )
                                .into();
                                self.event_plan = Some(ReportEventPlanKind::PreFill);
                                *self.checkpoint.borrow_mut() =
                                    Ok(Some(ReportOrderBasis::capture(order.clone())));
                                self.after_events = SegmentStage::Fills(0);
                                self.stage = SegmentStage::Events;
                            }
                            ExecutionReport::Fill(_) => self.stage = SegmentStage::Fills(0),
                            _ => unreachable!("position handled above"),
                        }
                        continue;
                    }
                    let strategy = engine.resolve_external_strategy(&self.instrument_id());
                    if engine.should_filter_unclaimed_external_order(strategy) {
                        return Ok(self.filtered_step());
                    }
                    let external = match &self.report {
                        ExecutionReport::Order(report)
                        | ExecutionReport::OrderWithFills(report, _) => {
                            engine.prepare_external_status(report, strategy)
                        }
                        ExecutionReport::Fill(report) => {
                            Some(engine.prepare_external_fill(report, strategy))
                        }
                        _ => unreachable!("position handled above"),
                    };
                    let Some(external) = external else {
                        return Ok(ReportNativeNext::Complete);
                    };
                    self.client_order_id = Some(external.client_order_id);
                    self.stage = SegmentStage::AfterInitialize;
                    return Ok(self.step(PreparedReportNative::RegisterExternal(external)));
                }
                SegmentStage::AfterInitialize => {
                    let Some(order) = self.lookup_order(engine) else {
                        return Ok(ReportNativeNext::Complete);
                    };
                    let now = engine.clock.borrow().timestamp_ns();
                    match &self.report {
                        ExecutionReport::Order(report) => {
                            let instrument = engine
                                .cache
                                .borrow()
                                .instrument(&report.instrument_id)
                                .cloned();
                            if let Some(instrument) = instrument {
                                let initial = self
                                    .checkpoint
                                    .borrow()
                                    .clone()
                                    .map_err(anyhow::Error::msg)?;
                                let initialized =
                                    initial.as_ref().map_or(&order, |basis| &basis.order);
                                self.events = generate_external_order_status_events(
                                    initialized,
                                    report,
                                    &report.account_id,
                                    &instrument,
                                    now,
                                )
                                .into();
                                self.event_plan = Some(ReportEventPlanKind::ExternalStatus);
                                self.stage = SegmentStage::Events;
                            }
                        }
                        ExecutionReport::OrderWithFills(report, _) => {
                            self.stage = SegmentStage::Fills(0);
                            return Ok(self.step(PreparedReportNative::Process(
                                OrderEventAny::Accepted(OrderAccepted::new(
                                    order.trader_id(),
                                    order.strategy_id(),
                                    order.instrument_id(),
                                    order.client_order_id(),
                                    report.venue_order_id,
                                    report.account_id,
                                    UUID4::new(),
                                    report.ts_accepted,
                                    now,
                                    true,
                                )),
                            )));
                        }
                        ExecutionReport::Fill(report) => {
                            self.stage = SegmentStage::Fills(0);
                            return Ok(self.step(PreparedReportNative::Process(
                                OrderEventAny::Accepted(OrderAccepted::new(
                                    order.trader_id(),
                                    order.strategy_id(),
                                    order.instrument_id(),
                                    order.client_order_id(),
                                    report.venue_order_id,
                                    report.account_id,
                                    UUID4::new(),
                                    report.ts_event,
                                    now,
                                    true,
                                )),
                            )));
                        }
                        _ => unreachable!("only order and fill can initialize"),
                    }
                }
                SegmentStage::Events => {
                    self.refresh_pending_events(engine)?;
                    if let Some(event) = self.events.pop_front() {
                        self.stage = SegmentStage::Events;
                        return Ok(self.step(PreparedReportNative::Process(event)));
                    }
                    self.stage = std::mem::replace(&mut self.after_events, SegmentStage::Done);
                }
                SegmentStage::Fills(index) => {
                    let fill = match &self.report {
                        ExecutionReport::Fill(fill) if index == 0 => Some(&**fill),
                        ExecutionReport::OrderWithFills(_, fills) => fills.get(index),
                        _ => None,
                    };
                    let Some(fill) = fill else {
                        self.stage = if matches!(self.report, ExecutionReport::OrderWithFills(_, _))
                        {
                            SegmentStage::Residual
                        } else {
                            SegmentStage::Done
                        };
                        continue;
                    };
                    let Some(order) = self.lookup_order(engine) else {
                        return Ok(ReportNativeNext::Complete);
                    };
                    let instrument = engine
                        .cache
                        .borrow()
                        .instrument(&fill.instrument_id)
                        .cloned();
                    let Some(instrument) = instrument else {
                        return Ok(ReportNativeNext::Complete);
                    };
                    self.stage = SegmentStage::Fills(index + 1);
                    if let Some(event) = reconcile_fill(
                        &order,
                        fill,
                        &instrument,
                        engine.clock.borrow().timestamp_ns(),
                        engine.config.allow_overfills,
                    ) {
                        let action = if matches!(self.report, ExecutionReport::OrderWithFills(_, _))
                            && let OrderEventAny::Filled(filled) = &event
                            && engine.retained_fill_evidence(filled)?
                        {
                            PreparedReportNative::ProjectRetainedFill(filled.clone())
                        } else {
                            PreparedReportNative::Process(event)
                        };
                        return Ok(self.fill_step(action, index, fill));
                    }
                }
                SegmentStage::Residual => {
                    let Some(order) = self.lookup_order(engine) else {
                        return Ok(ReportNativeNext::Complete);
                    };
                    let report = self
                        .order_report()
                        .expect("residual belongs to bundled report");
                    let instrument = engine
                        .cache
                        .borrow()
                        .instrument(&report.instrument_id)
                        .cloned();
                    self.events = generate_reconciliation_order_snapshot_events(
                        &order,
                        report,
                        instrument.as_ref(),
                        engine.clock.borrow().timestamp_ns(),
                    )
                    .into();
                    self.event_plan = Some(ReportEventPlanKind::Snapshot);
                    *self.checkpoint.borrow_mut() = Ok(Some(ReportOrderBasis::capture(order)));
                    self.stage = SegmentStage::Events;
                }
                SegmentStage::Done => return Ok(ReportNativeNext::Complete),
            }
        }
    }
}

struct ReportCursor {
    original: Rc<ExecutionReport>,
    segments: VecDeque<ReportSegment>,
    counted: bool,
    count: bool,
    finalized: bool,
    mass_log: Option<MassReportLog>,
}

impl ReportCursor {
    fn new(report: ExecutionReport, count: bool) -> Self {
        let mass_log = match &report {
            ExecutionReport::MassStatus(mass) => Some(MassReportLog {
                client_id: mass.client_id,
                account_id: mass.account_id,
                venue: mass.venue,
                orders: mass.order_reports().len(),
                fills: mass.fill_reports().values().map(Vec::len).sum(),
                positions: mass.position_reports().values().map(Vec::len).sum(),
            }),
            _ => None,
        };
        let mut segments = VecDeque::new();
        let source = |root_report_id, source_report_id, order_index, fill_index, position_index| {
            OwnerReportRef {
                root_report_id,
                source_report_id,
                root_index: 0,
                order_index,
                fill_index,
                position_index,
            }
        };
        let id = match &report {
            ExecutionReport::Order(report) | ExecutionReport::OrderWithFills(report, _) => {
                report.report_id
            }
            ExecutionReport::Fill(report) => report.report_id,
            ExecutionReport::Position(report) => report.report_id,
            ExecutionReport::MassStatus(report) => report.report_id,
        };
        match &report {
            ExecutionReport::MassStatus(mass) => {
                let orders = mass.order_reports();
                let fills = mass.fill_reports();
                let mut paired = AHashSet::new();
                for (index, order) in orders.values().enumerate() {
                    let segment = if let Some(fills) = fills
                        .get(&order.venue_order_id)
                        .filter(|fills| !fills.is_empty())
                    {
                        paired.insert(order.venue_order_id);
                        ExecutionReport::OrderWithFills(Box::new(order.clone()), fills.clone())
                    } else {
                        ExecutionReport::Order(Box::new(order.clone()))
                    };
                    segments.push_back(ReportSegment::new(
                        segment,
                        source(id, order.report_id, Some(index), None, None),
                    ));
                }
                for reports in fills.values() {
                    for (index, fill) in reports.iter().enumerate() {
                        if !paired.contains(&fill.venue_order_id) {
                            segments.push_back(ReportSegment::new(
                                ExecutionReport::Fill(Box::new(fill.clone())),
                                source(id, fill.report_id, None, Some(index), None),
                            ));
                        }
                    }
                }
                for (outer, reports) in mass.position_reports().values().enumerate() {
                    for (inner, position) in reports.iter().enumerate() {
                        segments.push_back(ReportSegment::new(
                            ExecutionReport::Position(Box::new(position.clone())),
                            source(id, position.report_id, None, None, Some((outer, inner))),
                        ));
                    }
                }
            }
            _ => segments.push_back(ReportSegment::new(
                report.clone(),
                source(id, id, None, None, None),
            )),
        }
        Self {
            original: Rc::new(report),
            segments,
            counted: false,
            count,
            finalized: false,
            mass_log,
        }
    }
}

impl ExecutionEngine {
    fn prepare_report_step(&self, cursor: &mut ReportCursor) -> anyhow::Result<ReportNativeNext> {
        if !cursor.counted {
            cursor.counted = true;
            if cursor.count {
                return Ok(ReportNativeNext::Step(ReportNativeStep {
                    action: PreparedReportNative::Count(cursor.mass_log),
                    sources: vec![],
                    checkpoint: None,
                }));
            }
        }
        while let Some(segment) = cursor.segments.front_mut() {
            match segment.prepare(self)? {
                ReportNativeNext::Complete => {
                    cursor.segments.pop_front();
                }
                step => return Ok(step),
            }
        }
        if !cursor.finalized {
            cursor.finalized = true;
            return Ok(ReportNativeNext::Step(ReportNativeStep {
                action: PreparedReportNative::Finalize(cursor.mass_log),
                sources: vec![],
                checkpoint: None,
            }));
        }
        Ok(ReportNativeNext::Complete)
    }

    pub(super) fn materialize_external_order_outcome(
        &self,
        external: PreparedExternal,
    ) -> NativeStage<Option<OrderAny>> {
        let PreparedExternal {
            initialized,
            client_order_id,
            venue_order_id,
            instrument_id,
            strategy_id,
            ts_now,
            order_status,
            source_client_id,
        } = external;
        let mut outcome = NativeStage::complete(None);
        let initialized = OrderEventAny::Initialized(initialized);
        let order = match OrderAny::from_events(vec![initialized.clone()]) {
            Ok(order) => order,
            Err(e) => {
                log::error!("Failed to create external order from report: {e}");
                outcome.record(NativeApplyStage::OrderCache, e);
                return outcome;
            }
        };

        {
            let mut cache = self.cache.borrow_mut();
            if let Err(e) = cache.add_venue_order_id(&client_order_id, &venue_order_id, false) {
                log::warn!("Failed to claim venue order ID for external order: {e}");
                outcome.record(NativeApplyStage::OrderCache, e);
                return outcome;
            }

            if let Err(e) = cache.add_order(order.clone(), None, source_client_id, false) {
                log::error!("Failed to add external order to cache: {e}");
                outcome.record(NativeApplyStage::OrderCache, e);
                return outcome;
            }
        }

        self.publish_order_event_outcome(&initialized, &mut outcome);

        match order_status {
            Some(status) => log::info!(
                "Created external order {client_order_id} ({venue_order_id}) for {instrument_id} [{status}]",
            ),
            None => log::info!(
                "Created external order {client_order_id} ({venue_order_id}) for {instrument_id}",
            ),
        }

        self.register_external_order(
            client_order_id,
            venue_order_id,
            instrument_id,
            strategy_id,
            ts_now,
        );

        outcome.value = Some(order);
        outcome
    }

    fn apply_report_step(&mut self, step: PreparedReportNative) -> Result<(), ReportApplyFailure> {
        match step {
            PreparedReportNative::Count(mass) => {
                self.report_count += 1;
                if let Some(mass) = mass {
                    log::info!(
                        "Reconciling mass status for client={}, account={}, venue={}",
                        mass.client_id,
                        mass.account_id,
                        mass.venue
                    );
                }
            }
            PreparedReportNative::Raw(report) => match report {
                ExecutionReport::Order(report) => msgbus::publish_any(
                    MessagingSwitchboard::reconciliation_raw_order_status_report_topic(),
                    &*report,
                ),
                ExecutionReport::Fill(report) => msgbus::publish_any(
                    MessagingSwitchboard::reconciliation_raw_fill_report_topic(),
                    &*report,
                ),
                ExecutionReport::OrderWithFills(report, fills) => {
                    msgbus::publish_any(
                        MessagingSwitchboard::reconciliation_raw_order_status_report_topic(),
                        &*report,
                    );
                    for fill in &fills {
                        msgbus::publish_any(
                            MessagingSwitchboard::reconciliation_raw_fill_report_topic(),
                            fill,
                        );
                    }
                }
                ExecutionReport::Position(report) => {
                    msgbus::publish_any(
                        MessagingSwitchboard::reconciliation_raw_position_status_report_topic(),
                        &*report,
                    );
                    let cache = self.cache.borrow();
                    let precision = cache
                        .instrument(&report.instrument_id)
                        .map(InstrumentAny::size_precision);
                    if report.venue_position_id.is_some() {
                        self.reconcile_position_report_hedging(&report, &cache);
                    } else {
                        self.reconcile_position_report_netting(&report, &cache, precision);
                    }
                }
                ExecutionReport::MassStatus(_) => unreachable!("mass status has child segments"),
            },
            PreparedReportNative::RegisterExternal(external) => {
                let applied = self.materialize_external_order_outcome(external);
                let legacy_progress = if applied.value.is_some() {
                    LegacyProgress::ContinueEvent
                } else {
                    LegacyProgress::FinishCurrentSegment
                };
                applied
                    .into_result()
                    .map_err(|source| ReportApplyFailure {
                        source,
                        legacy_progress,
                    })?
                    .ok_or_else(|| anyhow::anyhow!("External order materialization failed"))?;
            }
            PreparedReportNative::Process(event) => {
                if let NativeEventDisposition::Skipped(reason) = self
                    .handle_event_outcome(&event, true)
                    .into_result()
                    .map_err(ReportApplyFailure::continue_event)?
                {
                    log::debug!("Expected native report no-op: {reason:?}");
                }
            }
            PreparedReportNative::ProjectRetainedFill(fill) => {
                if !self.retained_fill_evidence(&fill)? {
                    return Err(anyhow::anyhow!(
                        "Retained fill evidence disappeared before projection"
                    )
                    .into());
                }
                self.handle_event_outcome(&OrderEventAny::Filled(fill), false)
                    .into_result()?;
            }
            PreparedReportNative::FilterExternal {
                kind,
                client_order_id,
                venue_order_id,
                instrument_id,
            } => {
                self.filtered_unclaimed_external_order_count += 1;
                if self.filtered_unclaimed_external_order_count == 1 {
                    log::info!(
                        "Filtering unclaimed external orders; first filtered {} {} ({}) for {}",
                        kind.label(),
                        client_order_id,
                        venue_order_id,
                        instrument_id
                    );
                } else {
                    log::debug!(
                        "Filtered unclaimed external {} {} ({}) for {}",
                        kind.label(),
                        client_order_id,
                        venue_order_id,
                        instrument_id
                    );
                }
            }
            PreparedReportNative::Finalize(mass) => {
                if let Some(mass) = mass {
                    log::info!(
                        "Mass status reconciliation complete: {} orders, {} fills, {} positions",
                        mass.orders,
                        mass.fills,
                        mass.positions
                    );
                }
            }
        }
        Ok(())
    }

    fn apply_report_step_legacy(
        &mut self,
        step: PreparedReportNative,
    ) -> Result<(), ReportApplyFailure> {
        match step {
            PreparedReportNative::RegisterExternal(external) => self
                .materialize_external_order(
                    external.initialized,
                    external.client_order_id,
                    external.venue_order_id,
                    external.instrument_id,
                    external.strategy_id,
                    external.ts_now,
                    external.order_status,
                    external.source_client_id,
                )
                .map(|_| ())
                .ok_or_else(|| anyhow::anyhow!("External order materialization failed").into()),
            step => self.apply_report_step(step),
        }
    }

    pub(super) fn drain_report_cursor(&mut self, report: ExecutionReport, count: bool) {
        let mut cursor = ReportCursor::new(report, count);
        loop {
            match self.prepare_report_step(&mut cursor) {
                Ok(ReportNativeNext::Step(step)) => {
                    let result = self.apply_report_step_legacy(step.action);
                    let observed = step
                        .checkpoint
                        .as_ref()
                        .map_or(Ok(()), |checkpoint| checkpoint.record(self));
                    if let Err(error) = result {
                        log::error!("Cannot reconcile report: {error}");
                        match error.legacy_progress {
                            LegacyProgress::ContinueEvent => {}
                            LegacyProgress::FinishCurrentSegment => {
                                cursor.segments.pop_front();
                            }
                        }
                    } else if let Err(error) = observed {
                        log::error!("Cannot record report checkpoint: {error}");
                        cursor.segments.pop_front();
                    }
                }
                Ok(ReportNativeNext::Complete) => break,
                Err(error) => {
                    log::error!("Cannot prepare report step: {error}");
                    if cursor.segments.pop_front().is_none() {
                        break;
                    }
                }
            }
        }
    }
    fn prepare_external_status(
        &self,
        report: &OrderStatusReport,
        strategy_id: StrategyId,
    ) -> Option<PreparedExternal> {
        let client_order_id = report
            .client_order_id
            .unwrap_or_else(|| ClientOrderId::from(report.venue_order_id.as_str()));

        let trader_id = get_message_bus().borrow().trader_id;
        let ts_now = self.clock.borrow().timestamp_ns();
        let Some(order_side) = report.order_side else {
            log::error!(
                "Skipping external order {} ({}) for {}: order side is not specified",
                client_order_id,
                report.venue_order_id,
                report.instrument_id,
            );
            return None;
        };

        let initialized = match OrderInitialized::new_checked(
            trader_id,
            strategy_id,
            report.instrument_id,
            client_order_id,
            order_side,
            report.order_type,
            report.quantity,
            report.time_in_force,
            report.post_only,
            report.reduce_only,
            false, // quote_quantity
            true,  // reconciliation
            UUID4::new(),
            ts_now,
            ts_now,
            report.price,
            report.activation_price,
            report.trigger_price,
            report.trigger_type,
            report.limit_offset,
            report.trailing_offset,
            report.trailing_offset_type,
            report.expire_time,
            report.display_qty,
            None, // emulation_trigger
            None, // trigger_instrument_id
            report.contingency_type,
            report.order_list_id,
            report.linked_order_ids.clone(),
            report.parent_order_id,
            None, // exec_algorithm_id
            None, // exec_algorithm_params
            None, // exec_spawn_id
            None, // tags
        ) {
            Ok(initialized) => initialized,
            Err(e) => {
                log::error!("Failed to create external order from report: {e}");
                return None;
            }
        };

        Some(PreparedExternal {
            initialized,
            client_order_id,
            venue_order_id: report.venue_order_id,
            instrument_id: report.instrument_id,
            strategy_id,
            ts_now,
            order_status: Some(report.order_status),
            source_client_id: self
                .source_client_id_for_account(report.account_id, &report.instrument_id),
        })
    }

    fn prepare_external_fill(
        &self,
        report: &FillReport,
        strategy_id: StrategyId,
    ) -> PreparedExternal {
        let client_order_id = report
            .client_order_id
            .unwrap_or_else(|| ClientOrderId::from(report.venue_order_id.as_str()));

        let trader_id = get_message_bus().borrow().trader_id;
        let ts_now = self.clock.borrow().timestamp_ns();

        let initialized = OrderInitialized::new(
            trader_id,
            strategy_id,
            report.instrument_id,
            client_order_id,
            report.order_side,
            OrderType::Market,
            report.last_qty,
            TimeInForce::Ioc,
            false, // post_only
            true,  // reduce_only: venue-initiated closes always reduce
            false, // quote_quantity
            true,  // reconciliation
            UUID4::new(),
            ts_now,
            ts_now,
            None, // price
            None, // activation_price
            None, // trigger_price
            None, // trigger_type
            None, // limit_offset
            None, // trailing_offset
            None,
            None, // expire_time
            None, // display_qty
            None, // emulation_trigger
            None, // trigger_instrument_id
            None,
            None, // order_list_id
            None, // linked_order_ids
            None, // parent_order_id
            None, // exec_algorithm_id
            None, // exec_algorithm_params
            None, // exec_spawn_id
            None, // tags
        );

        PreparedExternal {
            initialized,
            client_order_id,
            venue_order_id: report.venue_order_id,
            instrument_id: report.instrument_id,
            strategy_id,
            ts_now,
            order_status: None,
            source_client_id: self
                .source_client_id_for_account(report.account_id, &report.instrument_id),
        }
    }

    fn retained_fill_evidence(&self, fill: &OrderFilled) -> anyhow::Result<bool> {
        {
            let cache = self.cache.borrow();
            let position_id = fill
                .position_id
                .or_else(|| cache.position_id(&fill.client_order_id).copied());
            let Some(position) = position_id.and_then(|id| cache.position(&id)) else {
                return Ok(false);
            };
            if !position.trade_ids.contains(&fill.trade_id) {
                return Ok(false);
            }
            let retained = position
                .events
                .iter()
                .find(|event| event.trade_id == fill.trade_id)
                .ok_or_else(|| anyhow::anyhow!("Retained trade has no economic evidence"))?;
            anyhow::ensure!(
                retained.account_id == fill.account_id
                    && retained.instrument_id == fill.instrument_id
                    && retained.client_order_id == fill.client_order_id
                    && retained.venue_order_id == fill.venue_order_id
                    && retained.order_side == fill.order_side
                    && retained.last_qty == fill.last_qty
                    && retained.last_px == fill.last_px
                    && retained
                        .commission
                        .unwrap_or(Money::zero(retained.currency))
                        == fill.commission.unwrap_or(Money::zero(fill.currency)),
                "Retained trade identity or economics disagree with fill report"
            );
        }
        Ok(true)
    }
}

struct RuntimeReportDriver {
    engine: Rc<RefCell<ExecutionEngine>>,
    cursor: ReportCursor,
}

impl Debug for RuntimeReportDriver {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RuntimeReportDriver")
            .field("report", &self.cursor.original)
            .finish_non_exhaustive()
    }
}

impl OwnerReportDriver for RuntimeReportDriver {
    fn prepare_next(&mut self) -> Result<OwnerReportNext, OwnerDeliveryError> {
        let next = self
            .engine
            .borrow()
            .prepare_report_step(&mut self.cursor)
            .map_err(|error| {
                log::error!("Cannot prepare runtime report: {error}");
                OwnerDeliveryError::NativeAction
            })?;
        let ReportNativeNext::Step(step) = next else {
            return Ok(OwnerReportNext::Complete);
        };
        let kind = step.action.kind();
        let event = step.action.event();
        let engine = self.engine.clone();
        let checkpoint_engine = self.engine.clone();
        let checkpoint = step.checkpoint;
        Ok(OwnerReportNext::Step(PreparedOwnerReportStep {
            kind,
            sources: step.sources,
            event,
            action: Box::new(move || {
                engine
                    .borrow_mut()
                    .apply_report_step(step.action)
                    .map_err(|error| {
                        log::error!("Cannot apply runtime report: {error}");
                        OwnerDeliveryError::NativeAction
                    })
            }),
            after_native: Box::new(move |dispatch| {
                let observed = checkpoint.as_ref().map_or(Ok(()), |checkpoint| {
                    checkpoint.record(&checkpoint_engine.borrow())
                });
                match dispatch {
                    OwnerNativeDispatch::Failed(error) => Err(error),
                    OwnerNativeDispatch::Returned => observed.map_err(|error| {
                        log::error!("Cannot record runtime report checkpoint: {error}");
                        OwnerDeliveryError::NativeAction
                    }),
                }
            }),
        }))
    }
}

pub(super) fn runtime_report_submission(
    engine: Rc<RefCell<ExecutionEngine>>,
    report: ExecutionReport,
) -> OwnerReportSubmission {
    let input = OwnerReportInput {
        reports: vec![report.clone()],
        prepared_events: vec![],
    };
    OwnerReportSubmission::new(
        input,
        Box::new(RuntimeReportDriver {
            engine,
            cursor: ReportCursor::new(report, true),
        }),
    )
}
#[cfg(test)]
mod checkpoint_tests {
    use nautilus_common::{cache::Cache, clock::TestClock};
    use nautilus_model::{
        enums::OrderSide,
        identifiers::PositionId,
        instruments::stubs::audusd_sim,
        orders::{builder::OrderTestBuilder, stubs::TestOrderEventStubs},
        types::Price,
    };
    use rstest::rstest;
    use rust_decimal::Decimal;

    use super::*;

    fn checkpoint_order() -> OrderAny {
        OrderTestBuilder::new(OrderType::Limit)
            .instrument_id(audusd_sim().id)
            .quantity(Quantity::from(100))
            .price(Price::from("1.00000"))
            .build()
    }

    #[rstest]
    #[case("quantity")]
    #[case("leaves")]
    #[case("quote")]
    fn test_report_basis_detects_no_history_setters(#[case] field: &str) {
        let original = checkpoint_order();
        let before = ReportOrderBasis::capture(original.clone());
        let mut changed = original;
        match field {
            "quantity" => changed.set_quantity(Quantity::from(120)),
            "leaves" => changed.set_leaves_qty(Quantity::from(75)),
            "quote" => changed.set_is_quote_quantity(true),
            _ => unreachable!(),
        }
        let after = ReportOrderBasis::capture(changed);
        assert_eq!(before.history, after.history);
        assert_eq!(
            before.order.client_order_id(),
            after.order.client_order_id()
        );
        assert_ne!(before, after);
    }

    #[rstest]
    #[case("quantity")]
    #[case("price")]
    fn test_report_basis_detects_numeric_equal_history_precision(#[case] field: &str) {
        let instrument = InstrumentAny::CurrencyPair(audusd_sim());
        let mut original = checkpoint_order();
        original
            .apply(TestOrderEventStubs::accepted(
                &original,
                AccountId::from("SIM-001"),
                VenueOrderId::from("CHECKPOINT-VENUE"),
            ))
            .unwrap();
        let fill = TestOrderEventStubs::filled(
            &original,
            &instrument,
            Some(TradeId::from("CHECKPOINT-TRADE")),
            Some(PositionId::from("CHECKPOINT-POSITION")),
            Some(Price::from("1.00000")),
            Some(Quantity::from(50)),
            None,
            Some(Money::from("0 USD")),
            None,
            Some(AccountId::from("SIM-001")),
        );
        original.apply(fill).unwrap();
        let mut history: Vec<_> = original.events().into_iter().cloned().collect();
        let Some(OrderEventAny::Filled(fill)) = history.last_mut() else {
            panic!("actual fill is the latest historical event");
        };
        match field {
            "quantity" => fill.last_qty = Quantity::from("50.0"),
            "price" => fill.last_px = Price::from("1.000000"),
            _ => unreachable!(),
        }
        let changed = OrderAny::from_events(history).unwrap();
        let before = ReportOrderBasis::capture(original);
        let after = ReportOrderBasis::capture(changed);
        assert_eq!(before.snapshot, after.snapshot);
        assert_eq!(before.history, after.history);
        assert_ne!(before.precision, after.precision);
        assert_ne!(before, after);
    }

    #[rstest]
    fn test_report_checkpoint_keeps_pending_uuids_until_real_cache_change() {
        let clock = Rc::new(RefCell::new(TestClock::new()));
        let cache = Rc::new(RefCell::new(Cache::default()));
        let engine = ExecutionEngine::new(clock, cache.clone(), None);
        let instrument = InstrumentAny::CurrencyPair(audusd_sim());
        cache
            .borrow_mut()
            .add_instrument(instrument.clone())
            .unwrap();
        let mut order = checkpoint_order();
        order
            .apply(TestOrderEventStubs::submitted(
                &order,
                AccountId::from("SIM-001"),
            ))
            .unwrap();
        cache
            .borrow_mut()
            .add_order(order.clone(), None, None, true)
            .unwrap();
        let report = OrderStatusReport::new(
            AccountId::from("SIM-001"),
            instrument.id(),
            Some(order.client_order_id()),
            VenueOrderId::from("CHECKPOINT-VENUE"),
            Some(OrderSide::Buy),
            OrderType::Limit,
            TimeInForce::Gtc,
            OrderStatus::Filled,
            Quantity::from(100),
            Quantity::from(100),
            1.into(),
            2.into(),
            3.into(),
            Some(UUID4::new()),
        )
        .with_price(Price::from("1.00000"))
        .with_avg_px(Decimal::ONE);
        let events =
            generate_reconciliation_order_events(&order, &report, Some(&instrument), 3.into());
        assert_eq!(events.len(), 2);
        let source = OwnerReportRef {
            root_report_id: report.report_id,
            source_report_id: report.report_id,
            root_index: 0,
            order_index: None,
            fill_index: None,
            position_index: None,
        };
        let mut segment = ReportSegment::new(ExecutionReport::Order(Box::new(report)), source);
        segment.set_event_plan(ReportEventPlanKind::OrderStatus, events, &order);
        let accepted = segment.events.pop_front().unwrap();
        assert!(matches!(accepted, OrderEventAny::Accepted(_)));
        cache.borrow_mut().update_order(&accepted).unwrap();
        segment
            .step_checkpoint(&PreparedReportNative::Process(accepted.clone()))
            .unwrap()
            .record(&engine)
            .unwrap();
        let pending = segment.events.clone();
        segment.refresh_pending_events(&engine).unwrap();
        assert_eq!(segment.events, pending);
        assert_eq!(
            segment
                .events
                .iter()
                .map(|event| event.clone().into_boxed().id())
                .collect::<Vec<_>>(),
            pending
                .iter()
                .map(|event| event.clone().into_boxed().id())
                .collect::<Vec<_>>()
        );
        let mut changed = cache
            .borrow()
            .order_owned(&order.client_order_id())
            .unwrap();
        changed.set_quantity(Quantity::from(120));
        cache.borrow_mut().replace_order(&changed).unwrap();
        segment.refresh_pending_events(&engine).unwrap();
        assert!(!segment.events.is_empty());
        assert!(
            segment
                .events
                .iter()
                .all(|event| event.clone().into_boxed().id() != accepted.clone().into_boxed().id())
        );
        assert!(
            segment
                .events
                .iter()
                .all(|event| !matches!(event, OrderEventAny::Accepted(_)))
        );
        assert_ne!(
            segment
                .events
                .iter()
                .map(|event| event.clone().into_boxed().id())
                .collect::<Vec<_>>(),
            pending
                .iter()
                .map(|event| event.clone().into_boxed().id())
                .collect::<Vec<_>>()
        );
    }
}
