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

//! Managed reconciliation steps retained by the native delivery owner.

use std::{cell::RefCell, collections::VecDeque, fmt, rc::Rc};

use indexmap::{IndexMap, IndexSet};
use nautilus_common::{
    cache::Cache,
    live::dst,
    messages::ExecutionReport,
    owner_delivery::{
        OwnerDeliveryError, OwnerReportDisposition, OwnerReportDriver, OwnerReportInput,
        OwnerReportNext, OwnerReportRef, OwnerReportStepKind, OwnerReportSubmission,
        PreparedOwnerReportStep, dispatch_incoming_report,
    },
};
use nautilus_core::UUID4;
use nautilus_execution::engine::ExecutionEngine;
use nautilus_model::{
    events::{OrderEventAny, OrderFilled},
    identifiers::{
        AccountId, ClientId, ClientOrderId, InstrumentId, PositionId, TradeId, VenueOrderId,
    },
    orders::Order,
    reports::{ExecutionMassStatus, FillReport, OrderStatusReport, PositionStatusReport},
};

use super::{
    manager::{
        InflightCheckResult, InstrumentAccountKey, OpenOrderReportCheck, PositionReportCheck,
        SourcedOrderStatusReport, TargetedOrderReportResult,
    },
    recency::RecencyMap,
};

pub(super) type FillKey = (AccountId, InstrumentId, TradeId);

#[derive(Clone, Debug, Default)]
pub(super) struct ReconciliationHistory {
    pub(super) processed: RecencyMap<FillKey>,
    pub(super) recent: RecencyMap<FillKey>,
    pub(super) orders: RecencyMap<ClientOrderId>,
    pub(super) positions: RecencyMap<(InstrumentId, AccountId)>,
    pub(super) position_revisions: IndexMap<(InstrumentId, AccountId), u64>,
}

impl ReconciliationHistory {
    pub(super) fn mark_position(&mut self, instrument_id: InstrumentId, account_id: AccountId) {
        let key = (instrument_id, account_id);
        self.positions.mark(key);
        let revision = self.position_revisions.entry(key).or_default();
        *revision = revision.saturating_add(1);
    }
}

#[derive(Debug, Default)]
pub(super) struct ReconciliationBook(pub(super) Rc<RefCell<ReconciliationHistory>>);

impl Clone for ReconciliationBook {
    fn clone(&self) -> Self {
        // Cloning a manager keeps its original snapshot semantics. Only a
        // managed delivery step shares the book of the manager that built it.
        Self(Rc::new(RefCell::new(self.0.borrow().clone())))
    }
}

pub(super) fn fill_is_applied(cache: &Cache, fill: &OrderFilled, key: FillKey) -> bool {
    if fill.last_qty.is_zero() {
        return false;
    }

    cache
        .order(&fill.client_order_id)
        .or_else(|| {
            cache
                .client_order_id(&fill.venue_order_id)
                .and_then(|client_id| cache.order(client_id))
        })
        .is_some_and(|order| {
            order.account_id() == Some(key.0)
                && order.instrument_id() == key.1
                && order.trade_ids().contains(&&key.2)
        })
}

pub(super) struct ReconciliationStep {
    pub(super) event: OrderEventAny,
    pub(super) retained_projection: bool,
    pub(super) fill_key: Option<FillKey>,
    pub(super) sources: Option<Vec<OwnerReportRef>>,
}

#[derive(Default)]
struct ReconciliationProgress {
    unapplied_positions: IndexSet<PositionId>,
    native_failure: Option<anyhow::Error>,
}

struct ReconciliationDriver {
    engine: Rc<RefCell<ExecutionEngine>>,
    cache: Rc<RefCell<Cache>>,
    fill_book: Rc<RefCell<ReconciliationHistory>>,
    steps: VecDeque<ReconciliationStep>,
    next_index: usize,
    root: Option<UUID4>,
    continuous_input: Option<ContinuousReportInput>,
    orders: IndexMap<VenueOrderId, OrderStatusReport>,
    fills: IndexMap<VenueOrderId, Vec<FillReport>>,
    progress: Rc<RefCell<ReconciliationProgress>>,
}

impl fmt::Debug for ReconciliationDriver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReconciliationDriver")
            .field("report_id", &self.root)
            .field("next_index", &self.next_index)
            .field(
                "continuous",
                &self
                    .continuous_input
                    .as_ref()
                    .map(ContinuousReportInput::kind),
            )
            .finish_non_exhaustive()
    }
}

impl ReconciliationDriver {
    fn sources(&self, event: &OrderEventAny) -> Vec<OwnerReportRef> {
        let Some(root) = self.root else {
            return Vec::new();
        };
        let mut source = OwnerReportRef {
            root_report_id: root,
            source_report_id: root,
            root_index: 0,
            order_index: None,
            fill_index: None,
            position_index: None,
        };
        if let Some(venue_order_id) = event.venue_order_id() {
            if let Some((index, _, report)) = self.orders.get_full(&venue_order_id) {
                source.order_index = Some(index);
                source.source_report_id = report.report_id;
            }
            if let OrderEventAny::Filled(fill) = event
                && let Some((index, report)) = self.fills.get(&venue_order_id).and_then(|reports| {
                    reports.iter().enumerate().find(|(_, report)| {
                        report.trade_id == fill.trade_id
                            && report.account_id == fill.account_id
                            && report.instrument_id == fill.instrument_id
                    })
                })
            {
                source.fill_index = Some(index);
                source.source_report_id = report.report_id;
            }
        }
        vec![source]
    }
}

impl OwnerReportDriver for ReconciliationDriver {
    fn prepare_next(&mut self) -> Result<OwnerReportNext, OwnerDeliveryError> {
        let Some(step) = self.steps.pop_front() else {
            return Ok(OwnerReportNext::Complete);
        };
        self.next_index += 1;
        let sources = step
            .sources
            .clone()
            .unwrap_or_else(|| self.sources(&step.event));
        let event = step.event.clone();
        let committed_fill = step.fill_key.and_then(|key| {
            if let OrderEventAny::Filled(fill) = &event {
                Some((fill.clone(), key))
            } else {
                None
            }
        });
        let engine = self.engine.clone();
        let cache = self.cache.clone();
        let book = self.fill_book.clone();
        let progress = self.progress.clone();
        let native_progress = progress.clone();
        let continuous = self.continuous_input.is_some();
        let activity_book = book.clone();
        Ok(OwnerReportNext::Step(PreparedOwnerReportStep {
            kind: if step.retained_projection {
                OwnerReportStepKind::RetainedFillProjection
            } else {
                OwnerReportStepKind::CanonicalEvent
            },
            sources,
            event: Some(event),
            action: Box::new(move || {
                if continuous {
                    let mut history = activity_book.borrow_mut();
                    history.orders.mark(step.event.client_order_id());
                    if let OrderEventAny::Filled(fill) = &step.event {
                        history.mark_position(fill.instrument_id, fill.account_id);
                    }
                }
                let result = {
                    let mut engine = engine.borrow_mut();
                    match &step.event {
                        OrderEventAny::Filled(fill) if step.retained_projection => {
                            engine.apply_retained_reconciliation_fill(fill)
                        }
                        event => engine.apply_reconciliation_event(event),
                    }
                };
                result.map_err(|error| {
                    native_progress.borrow_mut().native_failure = Some(error);
                    OwnerDeliveryError::NativeAction
                })
            }),
            after_native: Box::new(move |_| {
                if let Some((fill, key)) = committed_fill {
                    if fill_is_applied(&cache.borrow(), &fill, key) {
                        let mut history = book.borrow_mut();
                        if continuous {
                            history.recent.mark(key);
                        } else {
                            history.processed.mark(key);
                        }
                    } else if let Some(position_id) = fill.position_id {
                        progress
                            .borrow_mut()
                            .unapplied_positions
                            .insert(position_id);
                    }
                }
                Ok(())
            }),
        }))
    }

    fn next_prepared_event_index(&self) -> Option<usize> {
        Some(self.next_index)
    }
}

pub(super) fn apply_startup_events(
    engine: &Rc<RefCell<ExecutionEngine>>,
    cache: &Rc<RefCell<Cache>>,
    book: &ReconciliationBook,
    mass: &ExecutionMassStatus,
    steps: VecDeque<ReconciliationStep>,
) -> anyhow::Result<IndexSet<PositionId>> {
    if steps.is_empty() {
        return Ok(IndexSet::new());
    }
    let progress = Rc::new(RefCell::new(ReconciliationProgress::default()));
    let input = OwnerReportInput {
        reports: vec![ExecutionReport::MassStatus(Box::new(mass.clone()))],
        prepared_events: steps.iter().map(|step| step.event.clone()).collect(),
    };
    let driver = ReconciliationDriver {
        engine: engine.clone(),
        cache: cache.clone(),
        fill_book: book.0.clone(),
        steps,
        next_index: 0,
        root: Some(mass.report_id),
        continuous_input: None,
        orders: mass.order_reports(),
        fills: mass.fill_reports(),
        progress,
    };
    dispatch_reconciliation(input, driver)
}

fn dispatch_reconciliation(
    input: OwnerReportInput,
    driver: ReconciliationDriver,
) -> anyhow::Result<IndexSet<PositionId>> {
    let progress = driver.progress.clone();
    match dispatch_incoming_report(OwnerReportSubmission::new(input, Box::new(driver))) {
        Ok(OwnerReportDisposition::Complete(_)) => Ok(std::mem::take(
            &mut progress.borrow_mut().unapplied_positions,
        )),
        Ok(OwnerReportDisposition::Queued(_)) => {
            nautilus_common::owner_delivery::record_failure(OwnerDeliveryError::ReentrantDrain);
            anyhow::bail!("Reconciliation did not complete at its owner safe point");
        }
        Ok(OwnerReportDisposition::Standalone(_)) => {
            anyhow::bail!("Reconciliation requires the live node owner");
        }
        Err(error) => Err(progress
            .borrow_mut()
            .native_failure
            .take()
            .unwrap_or_else(|| error.into())),
    }
}

pub(crate) enum ContinuousReportInput {
    Open {
        check: OpenOrderReportCheck,
        reports: Vec<SourcedOrderStatusReport>,
        queried: IndexSet<ClientId>,
        failed: IndexSet<ClientId>,
    },
    Targeted(Vec<TargetedOrderReportResult>),
    Position {
        check: PositionReportCheck,
        reports: Vec<PositionStatusReport>,
        queried: IndexSet<ClientId>,
        failed: IndexSet<ClientId>,
        fills: IndexMap<InstrumentAccountKey, Vec<FillReport>>,
        successful: IndexSet<InstrumentAccountKey>,
    },
    Inflight {
        checked_at: dst::time::Instant,
        result: InflightCheckResult,
    },
}

impl ContinuousReportInput {
    fn kind(&self) -> &'static str {
        match self {
            Self::Open { .. } => "open orders",
            Self::Targeted(_) => "targeted orders",
            Self::Position { .. } => "positions",
            Self::Inflight { .. } => "inflight",
        }
    }

    fn reports(&self) -> Vec<ExecutionReport> {
        match self {
            Self::Open {
                check,
                reports,
                queried,
                failed,
            } => {
                log::trace!(
                    "Open reconciliation: orders={}, queried={}, failed={}",
                    check.filtered_orders.len(),
                    queried.len(),
                    failed.len(),
                );
                reports
                    .iter()
                    .map(|source| ExecutionReport::Order(Box::new(source.report.clone())))
                    .collect()
            }
            Self::Targeted(results) => {
                let mut reports = Vec::new();
                for result in results {
                    if let Some(report) = &result.report {
                        reports.push(ExecutionReport::OrderWithFills(
                            Box::new(report.clone()),
                            result.fills.clone(),
                        ));
                    } else {
                        reports.extend(
                            result
                                .fills
                                .iter()
                                .cloned()
                                .map(|fill| ExecutionReport::Fill(Box::new(fill))),
                        );
                    }
                }
                reports
            }
            Self::Position {
                check,
                reports,
                queried,
                failed,
                fills,
                successful,
            } => {
                log::trace!(
                    "Position reconciliation: covered={}, queried={}, failed={}, fills_covered={}",
                    check.client_coverage.len(),
                    queried.len(),
                    failed.len(),
                    successful.len(),
                );
                reports
                    .iter()
                    .cloned()
                    .map(|report| ExecutionReport::Position(Box::new(report)))
                    .chain(
                        fills
                            .values()
                            .flatten()
                            .cloned()
                            .map(|fill| ExecutionReport::Fill(Box::new(fill))),
                    )
                    .collect()
            }
            Self::Inflight { checked_at, result } => {
                log::trace!(
                    "Inflight reconciliation at {checked_at:?}: events={}, queries={}",
                    result.events.len(),
                    result.queries.len(),
                );
                Vec::new()
            }
        }
    }
}

fn continuous_sources(reports: &[ExecutionReport], event: &OrderEventAny) -> Vec<OwnerReportRef> {
    let matches_order = |report: &OrderStatusReport| {
        event.instrument_id() == report.instrument_id
            && event
                .account_id()
                .is_none_or(|account| account == report.account_id)
            && (event.venue_order_id() == Some(report.venue_order_id)
                || report.client_order_id == Some(event.client_order_id()))
    };
    let matches_fill = |report: &FillReport| {
        matches!(event, OrderEventAny::Filled(fill)
            if fill.trade_id == report.trade_id && fill.account_id == report.account_id
                && fill.instrument_id == report.instrument_id && fill.venue_order_id == report.venue_order_id)
    };
    let same_position_scope = |report: &PositionStatusReport| {
        report.instrument_id == event.instrument_id()
            && event.account_id() == Some(report.account_id)
    };
    let matches_position = |report: &PositionStatusReport| {
        if !same_position_scope(report) {
            return false;
        }
        if let Some(position_id) = report.venue_position_id {
            return matches!(event, OrderEventAny::Filled(fill) if fill.position_id == Some(position_id));
        }
        reports
            .iter()
            .filter(
                |raw| matches!(raw, ExecutionReport::Position(other) if same_position_scope(other)),
            )
            .count()
            == 1
    };
    reports
        .iter()
        .enumerate()
        .filter_map(|(root_index, report)| {
            let (root, source, fill_index) = match report {
                ExecutionReport::Order(report) if matches_order(report) => {
                    (report.report_id, report.report_id, None)
                }
                ExecutionReport::Fill(report) if matches_fill(report) => {
                    (report.report_id, report.report_id, None)
                }
                ExecutionReport::OrderWithFills(order, fills) => {
                    if let Some((index, fill)) = fills
                        .iter()
                        .enumerate()
                        .find(|(_, fill)| matches_fill(fill))
                    {
                        (order.report_id, fill.report_id, Some(index))
                    } else if matches_order(order) {
                        (order.report_id, order.report_id, None)
                    } else {
                        return None;
                    }
                }
                ExecutionReport::Position(report) if matches_position(report) => {
                    (report.report_id, report.report_id, None)
                }
                _ => return None,
            };
            Some(OwnerReportRef {
                root_report_id: root,
                source_report_id: source,
                root_index,
                order_index: None,
                fill_index,
                position_index: None,
            })
        })
        .collect()
}

pub(super) fn apply_continuous_events(
    engine: &Rc<RefCell<ExecutionEngine>>,
    cache: &Rc<RefCell<Cache>>,
    book: &ReconciliationBook,
    original: ContinuousReportInput,
    events: Vec<OrderEventAny>,
) -> anyhow::Result<()> {
    if events.is_empty() {
        return Ok(());
    }
    let reports = original.reports();
    let steps = events
        .iter()
        .map(|event| ReconciliationStep {
            event: event.clone(),
            retained_projection: false,
            fill_key: match event {
                OrderEventAny::Filled(fill) => {
                    Some((fill.account_id, fill.instrument_id, fill.trade_id))
                }
                _ => None,
            },
            sources: Some(continuous_sources(&reports, event)),
        })
        .collect();
    let driver = ReconciliationDriver {
        engine: engine.clone(),
        cache: cache.clone(),
        fill_book: book.0.clone(),
        steps,
        next_index: 0,
        root: None,
        continuous_input: Some(original),
        orders: IndexMap::new(),
        fills: IndexMap::new(),
        progress: Rc::new(RefCell::new(ReconciliationProgress::default())),
    };
    dispatch_reconciliation(
        OwnerReportInput {
            reports,
            prepared_events: events,
        },
        driver,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[rstest]
    fn managed_fill_handle_shares_its_book_without_sharing_manager_clones() {
        let original = ReconciliationBook::default();
        let clone = original.clone();
        let driver = original.0.clone();
        let key = (
            AccountId::from("RECONCILIATION-001"),
            InstrumentId::from("ETHUSDT.SIM"),
            TradeId::from("RECONCILIATION-FILL"),
        );
        driver.borrow_mut().processed.mark(key);
        assert!(original.0.borrow().processed.contains_key(&key));
        assert!(!clone.0.borrow().processed.contains_key(&key));
        assert!(!original.0.borrow().recent.contains_key(&key));
    }
}
