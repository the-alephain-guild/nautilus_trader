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

//! Public hosted emulator callbacks, real asynchronous market data and owner accounting.

use std::{
    any::Any,
    cell::Cell,
    collections::HashSet,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use nautilus_common::{
    clients::ExecutionAccountCaptureError,
    enums::Environment,
    live::get_data_event_sender,
    logging::logger::LoggerConfig,
    messages::DataEvent,
    msgbus::{BusTap, Endpoint, MStr, Topic},
};
use nautilus_execution::order_emulator::emulator::OrderEmulator;
use nautilus_live::{
    config::{LiveExecutionEngineConfig, LiveNodeConfig},
    node::{NodeState, builder::LiveNodeBuilder},
    python::node::PyLiveNode,
};
use nautilus_model::{
    accounts::Account,
    enums::{LiquiditySide, TriggerType},
    events::PortfolioSnapshot,
    python::orders::order_any_to_pyobject,
};
use nautilus_trading::python::strategy::PyStrategy;
use pyo3::types::{PyCFunction, PyDict, PyModule, PyModuleMethods, PyTuple};

use super::{hosted_account_connect::ready_hosted_sandbox_client, *};

type EventIdentity = (String, String, String);

#[derive(Clone, Debug)]
struct PublishedOrder {
    event: OrderEventAny,
    cached_status: OrderStatus,
    during_start: bool,
}

#[derive(Default)]
struct EmulatorTrace {
    published: Vec<PublishedOrder>,
    ingress: Vec<OrderEventAny>,
    commands: Vec<(TradingCommand, bool)>,
    venue_queued: Vec<TradingCommand>,
    venue_dispatched: Vec<TradingCommand>,
    market_data: Vec<QuoteTick>,
}

struct EmulatorTap {
    cache: Rc<RefCell<Cache>>,
    emulator: Rc<RefCell<OrderEmulator>>,
    trace: Rc<RefCell<EmulatorTrace>>,
    during_start: Arc<AtomicBool>,
}

impl BusTap for EmulatorTap {
    fn on_publish(&self, topic: MStr<Topic>, message: &dyn Any) {
        if let Some(event) = message.downcast_ref::<OrderEventAny>()
            && topic.as_str() == format!("events.order.{}", event.strategy_id())
        {
            let cached_status = self
                .cache
                .borrow()
                .order_owned(&event.client_order_id())
                .unwrap()
                .status();
            self.trace.borrow_mut().published.push(PublishedOrder {
                event: event.clone(),
                cached_status,
                during_start: self.during_start.load(Ordering::SeqCst),
            });
        }
    }

    fn on_send(&self, endpoint: MStr<Endpoint>, message: &dyn Any) {
        if let Some(command) = message.downcast_ref::<TradingCommand>() {
            if endpoint == MessagingSwitchboard::order_emulator_execute() {
                let busy = self.emulator.try_borrow_mut().is_err();
                self.trace
                    .borrow_mut()
                    .commands
                    .push((command.clone(), busy));
            } else if endpoint == MessagingSwitchboard::exec_engine_queue_execute() {
                self.trace.borrow_mut().venue_queued.push(command.clone());
            } else if endpoint == MessagingSwitchboard::exec_engine_execute() {
                self.trace
                    .borrow_mut()
                    .venue_dispatched
                    .push(command.clone());
            }
        }
        if endpoint == MessagingSwitchboard::exec_engine_process()
            && let Some(event) = message.downcast_ref::<OrderEventAny>()
        {
            self.trace.borrow_mut().ingress.push(event.clone());
        }
        if endpoint == MessagingSwitchboard::data_engine_process_data()
            && let Some(Data::Quote(quote)) = message.downcast_ref::<Data>()
        {
            self.trace.borrow_mut().market_data.push(*quote);
        }
    }
}

fn identity(event: &OrderEventAny) -> EventIdentity {
    let event = event.clone().into_boxed();
    (
        event.type_name().to_owned(),
        event.id().to_string(),
        event.client_order_id().to_string(),
    )
}

#[rstest]
fn test_hosted_emulator_release_defers_callback_children_and_preserves_owner_accounting() {
    Python::initialize();
    let trader_id = TraderId::from("EMULATOR-001");
    let account_id = AccountId::from("SIM-001");
    let client_id = ClientId::from("EMULATOR");
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
        portfolio: Some(Default::default()),
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
    let engine = node.kernel().exec_engine().clone();
    let emulator = node.kernel().order_emulator.emulator();
    let clock = node.kernel().clock();
    let now = clock.borrow().timestamp_ns();
    let mut instrument = binary_option();
    instrument.id = InstrumentId::from("EMULATED.SIM");
    instrument.activation_ns = UnixNanos::from(1);
    instrument.expiration_ns = UnixNanos::from(now.as_u64() + 86_400_000_000_000);
    instrument.maker_fee = Decimal::new(1, 3);
    instrument.taker_fee = Decimal::new(1, 3);
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
    let first_quote = quote("0.450", "0.460", 1_000_000);
    let second_quote = quote("0.340", "0.350", 2_000_000);
    cache
        .borrow_mut()
        .add_instrument(instrument.clone())
        .unwrap();
    cache
        .borrow_mut()
        .add_quote(quote("0.400", "0.410", 0))
        .unwrap();
    let data_sender = get_data_event_sender();
    let during_start = Arc::new(AtomicBool::new(false));
    let observer = Python::attach(|py| {
        let module = PyModule::new(py, "hosted_emulator_observer").unwrap();
        module
            .setattr("Strategy", py.get_type::<PyStrategy>())
            .unwrap();
        module
            .setattr("client_id", client_id.into_py_any(py).unwrap())
            .unwrap();
        let stage = during_start.clone();
        let mark_start = PyCFunction::new_closure(
            py,
            None,
            None,
            move |args: &pyo3::Bound<'_, PyTuple>, _kwargs| -> pyo3::PyResult<()> {
                stage.store(args.get_item(0)?.extract::<bool>()?, Ordering::SeqCst);
                Ok(())
            },
        )
        .unwrap();
        module.setattr("mark_start", mark_start).unwrap();
        let publish_quote = PyCFunction::new_closure(
            py,
            None,
            None,
            move |args: &pyo3::Bound<'_, PyTuple>, _kwargs| -> pyo3::PyResult<()> {
                let quote = match args.get_item(0)?.extract::<u8>()? {
                    1 => first_quote,
                    2 => second_quote,
                    _ => {
                        return Err(pyo3::exceptions::PyValueError::new_err(
                            "invalid quote stage",
                        ));
                    }
                };
                data_sender
                    .send(DataEvent::Data(Data::Quote(quote)))
                    .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))
            },
        )
        .unwrap();
        module.setattr("publish_quote", publish_quote).unwrap();
        py.run(c_str!(r#"
import asyncio
import sqlite3
import tempfile
from pathlib import Path

directory = tempfile.TemporaryDirectory()
database = Path(directory.name) / "emulator.sqlite3"
with sqlite3.connect(database) as connection:
    connection.execute("CREATE TABLE observations (kind TEXT, event_id TEXT, identity TEXT, state TEXT)")
seen = []
specific = []
aggregate = []
pending_after_submit = []
mirror_callbacks = []
snapshot_callbacks = []
errors = []
started = False
parent_released = False
parent_filled = False
child_emulated = False
cancelled = False
closed = False

def record(kind, event_id, identity, state):
    with sqlite3.connect(database, timeout=0) as connection:
        connection.execute("BEGIN IMMEDIATE")
        connection.execute("INSERT INTO observations VALUES (?, ?, ?, ?)", (kind, event_id, identity, state))
    seen.append((kind, event_id, identity, state))

def observe_account(event_id, cached_id, balance, count):
    record("AccountMirror", event_id, cached_id, balance)
    mirror_callbacks.append((event_id, cached_id, balance, count))

def observe_snapshot(snapshot_id, account_event_id, account_id, balance, equity):
    record("PortfolioSnapshot", snapshot_id, account_event_id, balance + " | " + equity)
    snapshot_callbacks.append((snapshot_id, account_event_id, account_id, balance, equity))

class EmulatorStrategy(Strategy):
    def on_start(self):
        global started
        mark_start(True)
        try:
            self.submit_order(self.orders[0], client_id=client_id)
        finally:
            mark_start(False)
        started = True
    def on_stop(self):
        record("StrategyStopped", "", "", "actual lifecycle")
    def on_dispose(self): pass
    def observe(self, event):
        item = (type(event).__name__, str(event.event_id), str(event.client_order_id))
        specific.append(item)
        record(*item, "typed strategy callback")
    def on_order_event(self, event):
        aggregate.append((type(event).__name__, str(event.event_id), str(event.client_order_id)))
    def on_order_initialized(self, event): self.observe(event)
    def on_order_emulated(self, event):
        global child_emulated
        self.observe(event)
        if str(event.client_order_id) == "EMU-CHILD":
            child_emulated = True
    def on_order_released(self, event):
        global parent_released
        try:
            self.observe(event)
            if str(event.client_order_id) == "EMU-PARENT":
                assert not parent_released
                parent_released = True
                self.submit_order(self.orders[1], client_id=client_id)
                self.submit_order(self.orders[2], client_id=client_id)
                pending_after_submit.append((str(self.cache.order(self.orders[1].client_order_id).status), str(self.cache.order(self.orders[2].client_order_id).status)))
                self.cancel_order(self.orders[2].client_order_id, client_id=client_id)
        except BaseException as e:
            errors.append(repr(e))
            raise
    def on_order_submitted(self, event): self.observe(event)
    def on_order_accepted(self, event): self.observe(event)
    def on_order_filled(self, event):
        global parent_filled
        self.observe(event)
        if str(event.client_order_id) == "EMU-PARENT":
            parent_filled = True
    def on_order_canceled(self, event):
        global cancelled
        self.observe(event)
        if str(event.client_order_id) == "EMU-CANCEL":
            cancelled = True
    def on_position_opened(self, event):
        record("PositionOpened", str(event.event_id), str(event.position_id), str(self.cache.position(event.position_id).quantity))
    def on_position_closed(self, event):
        global closed
        record("PositionClosed", str(event.event_id), str(event.position_id), str(self.cache.position(event.position_id).quantity))
        closed = True

strategy = EmulatorStrategy()
def mark_waiting(): pass
def cleanup(): directory.cleanup()
"#), Some(&module.dict()), None).unwrap();
        module.unbind()
    });
    let strategy = Python::attach(|py| observer.bind(py).getattr("strategy").unwrap().unbind());
    let strategy_id = node
        .kernel()
        .trader()
        .borrow_mut()
        .add_python_strategy_instance(&strategy)
        .unwrap();
    let orders: Vec<_> = [
        ("EMU-PARENT", OrderSide::Buy, "100.00", "0.470", "0.450"),
        ("EMU-CHILD", OrderSide::Sell, "100.00", "0.330", "0.350"),
        ("EMU-CANCEL", OrderSide::Buy, "50.00", "0.620", "0.600"),
    ]
    .into_iter()
    .map(|(id, side, quantity, price, trigger)| {
        OrderTestBuilder::new(OrderType::StopLimit)
            .trader_id(trader_id)
            .strategy_id(strategy_id)
            .instrument_id(instrument.id())
            .client_order_id(id.into())
            .side(side)
            .quantity(Quantity::from(quantity))
            .price(Price::from(price))
            .trigger_price(Price::from(trigger))
            .trigger_type(TriggerType::BidAsk)
            .emulation_trigger(TriggerType::BidAsk)
            .build()
    })
    .collect();
    Python::attach(|py| {
        let py_orders: Vec<_> = orders
            .iter()
            .map(|order| order_any_to_pyobject(py, order.clone()).unwrap())
            .collect();
        strategy.bind(py).setattr("orders", py_orders).unwrap();
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
    let waiting = Python::attach(|py| observer.bind(py).getattr("mark_waiting").unwrap().unbind());
    engine
        .borrow_mut()
        .register_client(ready_hosted_sandbox_client(inner, waiting, stopped.clone()))
        .unwrap();
    let trace = Rc::new(RefCell::new(EmulatorTrace::default()));
    msgbus::set_bus_tap(Rc::new(EmulatorTap {
        cache: cache.clone(),
        emulator: emulator.clone(),
        trace: trace.clone(),
        during_start,
    }));
    let mirror_records = Rc::new(RefCell::new(Vec::new()));
    let records = mirror_records.clone();
    let mirror_cache = cache.clone();
    let callback = Python::attach(|py| {
        observer
            .bind(py)
            .getattr("observe_account")
            .unwrap()
            .unbind()
    });
    let account_handler = TypedHandler::from(move |event: &AccountState| {
        let account = mirror_cache
            .borrow()
            .account_owned(&event.account_id)
            .unwrap();
        let last = account.last_event().unwrap();
        records.borrow_mut().push((
            event.clone(),
            last.clone(),
            account.balance_total(None).unwrap(),
            account.events().len(),
        ));
        Python::attach(|py| {
            callback
                .call1(
                    py,
                    (
                        event.event_id.to_string(),
                        last.event_id.to_string(),
                        account.balance_total(None).unwrap().to_string(),
                        account.events().len(),
                    ),
                )
                .unwrap();
        });
    });
    msgbus::subscribe_account_state("events.account.*".into(), account_handler.clone(), None);
    let snapshot_records = Rc::new(RefCell::new(Vec::new()));
    let records = snapshot_records.clone();
    let snapshot_cache = cache.clone();
    let callback = Python::attach(|py| {
        observer
            .bind(py)
            .getattr("observe_snapshot")
            .unwrap()
            .unbind()
    });
    let snapshot_handler = TypedHandler::from(move |event: &PortfolioSnapshot| {
        let account = snapshot_cache
            .borrow()
            .account_owned(&event.account_id)
            .unwrap();
        let account_event = account.last_event().unwrap();
        records
            .borrow_mut()
            .push((event.clone(), account_event.clone()));
        Python::attach(|py| {
            callback
                .call1(
                    py,
                    (
                        event.event_id.to_string(),
                        account_event.event_id.to_string(),
                        event.account_id.to_string(),
                        account.balance_total(None).unwrap().to_string(),
                        event.base_currency_equity.unwrap().to_string(),
                    ),
                )
                .unwrap();
        });
    });
    msgbus::subscribe_portfolio_snapshot(
        "events.portfolio.*".into(),
        snapshot_handler.clone(),
        None,
    );
    let (py_node, outcome) = Python::attach(|py| {
        let py_node = pyo3::Py::new(py, PyLiveNode::new(node)).unwrap();
        let globals = PyDict::new(py);
        globals.set_item("node", &py_node).unwrap();
        globals.set_item("observer", observer.bind(py)).unwrap();
        let outcome = py.run(c_str!(r#"
import asyncio

async def exercise():
    task = asyncio.create_task(node.run_async())
    async def until(predicate):
        while not predicate():
            if task.done():
                await task
                raise AssertionError("hosted node stopped before the emulator observations")
            if observer.errors:
                raise AssertionError(observer.errors)
            await asyncio.sleep(0)
    try:
        await asyncio.wait_for(until(lambda: observer.started), timeout=3)
        observer.publish_quote(1)
        await asyncio.wait_for(until(lambda: observer.parent_filled and observer.child_emulated and observer.cancelled), timeout=3)
        observer.publish_quote(2)
        await asyncio.wait_for(until(lambda: observer.closed), timeout=3)
    except BaseException:
        print("emulator callbacks:", observer.seen, "pending snapshots:", observer.pending_after_submit, "errors:", observer.errors, flush=True)
        raise
    finally:
        node.handle().stop()
        await asyncio.wait_for(task, timeout=3)
asyncio.run(exercise())
"#), Some(&globals), None);
        if outcome.is_err() {
            eprintln!("emulator trace: {:?}", trace.borrow().published);
            eprintln!("emulator commands: {:?}", trace.borrow().commands);
            for order in &orders {
                eprintln!(
                    "cached emulator order {}: {:?}",
                    order.client_order_id(),
                    cache.borrow().order_owned(&order.client_order_id())
                );
            }
        }
        (py_node, outcome)
    });
    msgbus::unsubscribe_account_state("events.account.*".into(), &account_handler);
    msgbus::unsubscribe_portfolio_snapshot("events.portfolio.*".into(), &snapshot_handler);
    msgbus::clear_bus_tap();
    if outcome.is_err() {
        Python::attach(|py| {
            py_node.call_method0(py, "dispose").unwrap();
            observer.bind(py).call_method0("cleanup").unwrap();
        });
    }
    outcome.unwrap();
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
    assert_eq!(trace.market_data, [first_quote, second_quote]);
    assert_eq!(trace.commands.len(), 4);
    assert!(!trace.commands[0].1);
    assert!(trace.commands[1..].iter().all(|(_, busy)| *busy));
    let command_ids: Vec<_> = trace
        .commands
        .iter()
        .map(|(command, _)| match command {
            TradingCommand::SubmitOrder(command) => {
                ("submit", command.client_order_id, command.command_id)
            }
            TradingCommand::CancelOrder(command) => {
                ("cancel", command.client_order_id, command.command_id)
            }
            _ => panic!("unexpected emulator command"),
        })
        .collect();
    assert_eq!(
        command_ids
            .iter()
            .map(|(kind, id, _)| (*kind, id.to_string()))
            .collect::<Vec<_>>(),
        vec![
            ("submit", "EMU-PARENT".to_owned()),
            ("submit", "EMU-CHILD".to_owned()),
            ("submit", "EMU-CANCEL".to_owned()),
            ("cancel", "EMU-CANCEL".to_owned())
        ]
    );
    assert_eq!(
        command_ids
            .iter()
            .map(|(_, _, id)| *id)
            .collect::<HashSet<_>>()
            .len(),
        4
    );
    assert_eq!(trace.venue_queued, trace.venue_dispatched);
    assert_eq!(
        trace.venue_queued,
        [trace.commands[0].0.clone(), trace.commands[1].0.clone()],
        "both original submit commands must survive emulation and the async owner queue"
    );
    let released = trace
        .published
        .iter()
        .position(|record| {
            matches!(record.event, OrderEventAny::Released(_))
                && record.event.client_order_id() == orders[0].client_order_id()
        })
        .unwrap();
    let submitted = trace
        .published
        .iter()
        .position(|record| {
            matches!(record.event, OrderEventAny::Submitted(_))
                && record.event.client_order_id() == orders[0].client_order_id()
        })
        .unwrap();
    let local_fifo: Vec<_> = trace.published[released + 1..submitted]
        .iter()
        .map(|record| {
            (
                record.event.clone().into_boxed().type_name(),
                record.event.client_order_id().to_string(),
            )
        })
        .collect();
    assert_eq!(
        local_fifo,
        [
            ("OrderInitialized", "EMU-CHILD".to_owned()),
            ("OrderInitialized", "EMU-CANCEL".to_owned()),
            ("OrderEmulated", "EMU-CHILD".to_owned()),
            ("OrderEmulated", "EMU-CANCEL".to_owned()),
            ("OrderCanceled", "EMU-CANCEL".to_owned()),
        ]
    );
    for (order, expected_status) in orders.iter().zip([
        OrderStatus::Filled,
        OrderStatus::Filled,
        OrderStatus::Canceled,
    ]) {
        let cached = cache
            .borrow()
            .order_owned(&order.client_order_id())
            .unwrap();
        assert_eq!(cached.status(), expected_status);
        assert_eq!(cached.quantity(), order.quantity());
        let outward: Vec<_> = trace
            .published
            .iter()
            .filter(|record| record.event.client_order_id() == order.client_order_id())
            .map(|record| identity(&record.event))
            .collect();
        let canonical: Vec<_> = cached
            .events()
            .iter()
            .map(|event| identity(event))
            .collect();
        assert_eq!(
            outward, canonical,
            "original and transformed event identities must survive owner application"
        );
        assert_eq!(
            canonical.first().unwrap().1,
            order.init_event().event_id.to_string()
        );
        assert_eq!(
            canonical
                .iter()
                .map(|(_, id, _)| id)
                .collect::<HashSet<_>>()
                .len(),
            canonical.len()
        );
        let expected = if expected_status == OrderStatus::Filled {
            vec![
                "OrderInitialized",
                "OrderEmulated",
                "OrderInitialized",
                "OrderReleased",
                "OrderSubmitted",
                "OrderAccepted",
                "OrderFilled",
            ]
        } else {
            vec!["OrderInitialized", "OrderEmulated", "OrderCanceled"]
        };
        assert_eq!(
            canonical
                .iter()
                .map(|(kind, _, _)| kind.as_str())
                .collect::<Vec<_>>(),
            expected
        );
        if expected_status == OrderStatus::Filled {
            let raw: Vec<_> = trace
                .ingress
                .iter()
                .filter(|event| event.client_order_id() == order.client_order_id())
                .map(identity)
                .collect();
            let from_sandbox: Vec<_> = trace
                .published
                .iter()
                .filter(|record| {
                    record.event.client_order_id() == order.client_order_id()
                        && matches!(
                            record.event,
                            OrderEventAny::Submitted(_)
                                | OrderEventAny::Accepted(_)
                                | OrderEventAny::Filled(_)
                        )
                })
                .map(|record| identity(&record.event))
                .collect();
            let canonical_sandbox: Vec<_> = cached
                .events()
                .iter()
                .filter(|event| {
                    matches!(
                        event,
                        OrderEventAny::Submitted(_)
                            | OrderEventAny::Accepted(_)
                            | OrderEventAny::Filled(_)
                    )
                })
                .map(|event| identity(event))
                .collect();
            assert_eq!(
                raw.iter()
                    .map(|(kind, _, _)| kind.as_str())
                    .collect::<Vec<_>>(),
                ["OrderSubmitted", "OrderAccepted", "OrderFilled"]
            );
            assert_eq!(raw, from_sandbox);
            assert_eq!(raw, canonical_sandbox);
            assert_eq!(cached.order_type(), OrderType::Limit);
            assert_eq!(cached.filled_qty(), Quantity::from("100.00"));
            assert!(cached.venue_order_id().is_some());
        } else {
            assert_eq!(cached.filled_qty(), Quantity::zero(2));
            assert!(cached.venue_order_id().is_none());
        }
    }
    let running_outward: Vec<_> = trace
        .published
        .iter()
        .filter(|record| !record.during_start)
        .map(|record| identity(&record.event))
        .collect();
    let (specific, aggregate): (Vec<EventIdentity>, Vec<EventIdentity>) = Python::attach(|py| {
        let module = observer.bind(py);
        assert!(
            module
                .getattr("errors")
                .unwrap()
                .extract::<Vec<String>>()
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            module
                .getattr("pending_after_submit")
                .unwrap()
                .extract::<Vec<(String, String)>>()
                .unwrap(),
            vec![("INITIALIZED".to_owned(), "INITIALIZED".to_owned())]
        );
        (
            module.getattr("specific").unwrap().extract().unwrap(),
            module.getattr("aggregate").unwrap().extract().unwrap(),
        )
    });
    assert_eq!(specific, running_outward);
    for order in &orders {
        let order_id = order.client_order_id().to_string();
        let published: Vec<_> = running_outward
            .iter()
            .filter(|(_, _, id)| *id == order_id)
            .collect();
        let observed: Vec<_> = aggregate
            .iter()
            .filter(|(_, _, id)| *id == order_id)
            .collect();
        assert_eq!(observed, published);
    }
    assert_eq!(aggregate.len(), running_outward.len());
    // The parent aggregate hook follows its specific hook and nested child publications
    let parent_released = aggregate
        .iter()
        .position(|(kind, _, id)| kind == "OrderReleased" && id == "EMU-PARENT")
        .unwrap();
    for child in ["EMU-CHILD", "EMU-CANCEL"] {
        let initialized = aggregate
            .iter()
            .position(|(kind, _, id)| kind == "OrderInitialized" && id == child)
            .unwrap();
        assert!(initialized < parent_released);
    }
    let start_records: Vec<_> = trace
        .published
        .iter()
        .filter(|record| record.during_start)
        .collect();
    assert_eq!(start_records.len(), 2);
    assert_eq!(start_records[0].cached_status, OrderStatus::Initialized);
    assert_eq!(start_records[1].cached_status, OrderStatus::Emulated);
    assert!(
        trace
            .published
            .iter()
            .filter(|record| matches!(record.event, OrderEventAny::Released(_)))
            .all(|record| record.cached_status == OrderStatus::Released)
    );
    assert!(emulator.borrow().get_submit_order_commands().is_empty());
    assert!(
        emulator
            .borrow()
            .get_matching_core(&instrument.id())
            .unwrap()
            .get_orders()
            .is_empty()
    );
    let fills: Vec<_> = orders
        .iter()
        .flat_map(|order| {
            cache
                .borrow()
                .order_owned(&order.client_order_id())
                .unwrap()
                .events()
                .into_iter()
                .filter_map(|event| match event {
                    OrderEventAny::Filled(fill) => Some(fill.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>()
        })
        .collect();
    assert_eq!(fills.len(), 2);
    assert_eq!(fills[0].last_px, first_quote.ask_price);
    assert_eq!(fills[1].last_px, second_quote.bid_price);
    assert!(
        fills
            .iter()
            .all(|fill| fill.liquidity_side == LiquiditySide::Taker)
    );
    assert_eq!(
        fills
            .iter()
            .map(|fill| fill.commission.unwrap().as_decimal())
            .sum::<Decimal>(),
        Decimal::new(8, 2)
    );
    let positions: Vec<_> = cache
        .borrow()
        .positions(
            None,
            Some(&instrument.id()),
            Some(&strategy_id),
            Some(&account_id),
            None,
        )
        .into_iter()
        .map(|position| position.cloned())
        .collect();
    assert_eq!(positions.len(), 1);
    assert!(positions[0].is_closed());
    assert_eq!(positions[0].quantity, Quantity::zero(2));
    assert_eq!(
        positions[0].realized_pnl.unwrap(),
        Money::from("-12.08 USDC")
    );
    let account = cache.borrow().account_owned(&account_id).unwrap();
    assert_eq!(
        account.balance_total(None).unwrap(),
        Money::from("99987.92 USDC")
    );
    let account_events = account.events();
    assert_eq!(
        account_events
            .iter()
            .map(|event| event.event_id)
            .collect::<HashSet<_>>()
            .len(),
        account_events.len()
    );
    let mirrors = mirror_records.borrow();
    assert!(
        !mirrors.is_empty(),
        "the real account mirror must be observed"
    );
    for (event, observed, balance, count) in mirrors.iter() {
        assert_eq!(event.account_id, account_id);
        assert_eq!(event.event_id, observed.event_id);
        assert_eq!(event.balances, observed.balances);
        let original = account_events
            .iter()
            .find(|original| original.event_id == event.event_id)
            .unwrap();
        assert_eq!(original.balances, observed.balances);
        let usdc = observed
            .balances
            .iter()
            .find(|entry| entry.total.currency == Currency::USDC())
            .unwrap();
        assert_eq!(*balance, usdc.total);
        assert!((1..=account_events.len()).contains(count));
    }
    assert_eq!(mirrors.first().unwrap().2, Money::from("100000 USDC"));
    assert_eq!(mirrors.last().unwrap().2, Money::from("99987.92 USDC"));
    let snapshots = snapshot_records.borrow();
    assert!(
        snapshots.len() >= 2,
        "the enabled equity curve must emit registration and real shutdown snapshots"
    );
    let mut snapshot_ids = HashSet::new();
    for (snapshot, observed) in snapshots.iter() {
        assert_eq!(snapshot.account_id, account_id);
        assert_eq!(snapshot.account_type, AccountType::Cash);
        assert_eq!(snapshot.base_currency, Some(Currency::USDC()));
        assert!(snapshot_ids.insert(snapshot.event_id));
        let original = account_events
            .iter()
            .find(|event| event.event_id == observed.event_id)
            .unwrap();
        assert_eq!(observed.account_id, account_id);
        assert_eq!(observed.balances, original.balances);
        assert_eq!(snapshot.balances.len(), observed.balances.len());
        for balance in &snapshot.balances {
            assert!(observed.balances.contains(balance));
        }
    }
    assert_eq!(
        snapshots.first().unwrap().0.base_currency_equity,
        Some(Money::from("100000 USDC"))
    );
    assert_eq!(
        snapshots.last().unwrap().0.base_currency_equity,
        Some(Money::from("99987.92 USDC"))
    );
    Python::attach(|py| {
        let module = observer.bind(py);
        let python_mirrors: Vec<(String, String, String, usize)> = module
            .getattr("mirror_callbacks")
            .unwrap()
            .extract()
            .unwrap();
        let expected_mirrors: Vec<_> = mirrors
            .iter()
            .map(|(event, observed, balance, count)| {
                (
                    event.event_id.to_string(),
                    observed.event_id.to_string(),
                    balance.to_string(),
                    *count,
                )
            })
            .collect();
        assert_eq!(python_mirrors, expected_mirrors);
        let python_snapshots: Vec<(String, String, String, String, String)> = module
            .getattr("snapshot_callbacks")
            .unwrap()
            .extract()
            .unwrap();
        let expected_snapshots: Vec<_> = snapshots
            .iter()
            .map(|(snapshot, observed)| {
                (
                    snapshot.event_id.to_string(),
                    observed.event_id.to_string(),
                    snapshot.account_id.to_string(),
                    observed
                        .balances
                        .iter()
                        .find(|balance| balance.total.currency == Currency::USDC())
                        .unwrap()
                        .total
                        .to_string(),
                    snapshot.base_currency_equity.unwrap().to_string(),
                )
            })
            .collect();
        assert_eq!(python_snapshots, expected_snapshots);
        let rows: usize = module
            .getattr("sqlite3")
            .unwrap()
            .call_method1("connect", (module.getattr("database").unwrap(),))
            .unwrap()
            .call_method1("execute", ("SELECT COUNT(*) FROM observations",))
            .unwrap()
            .call_method0("fetchone")
            .unwrap()
            .get_item(0)
            .unwrap()
            .extract()
            .unwrap();
        assert_eq!(rows, module.getattr("seen").unwrap().len().unwrap());
        py_node.call_method0(py, "dispose").unwrap();
        module.call_method0("cleanup").unwrap();
    });
}

#[rstest]
fn test_hosted_twap_timer_children_preserve_canonical_fills_and_callback_writer() {
    use nautilus_model::identifiers::ExecAlgorithmId;
    use nautilus_trading::algorithm::twap::{TwapAlgorithm, TwapAlgorithmConfig};

    Python::initialize();
    let trader_id = TraderId::from("TIMED-001");
    let account_id = AccountId::from("SIM-001");
    let client_id = ClientId::from("TIMED");
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
        portfolio: Some(Default::default()),
        ..Default::default()
    };
    let mut node = LiveNodeBuilder::from_config(config)
        .unwrap()
        .with_logging(LoggerConfig {
            bypass_logging: true,
            ..Default::default()
        })
        .build()
        .unwrap();
    let algorithm_id = ExecAlgorithmId::new("TWAP");
    node.add_exec_algorithm(TwapAlgorithm::new(TwapAlgorithmConfig {
        exec_algorithm_id: Some(algorithm_id),
        ..Default::default()
    }))
    .unwrap();
    let cache = node.kernel().cache();
    let engine = node.kernel().exec_engine().clone();
    let clock = node.kernel().clock();
    let now = clock.borrow().timestamp_ns();
    let mut instrument = binary_option();
    instrument.id = InstrumentId::from("TIMED.SIM");
    instrument.activation_ns = UnixNanos::from(1);
    instrument.expiration_ns = UnixNanos::from(now.as_u64() + 86_400_000_000_000);
    instrument.maker_fee = Decimal::ZERO;
    instrument.taker_fee = Decimal::ZERO;
    let instrument = InstrumentAny::BinaryOption(instrument);
    {
        let mut cache = cache.borrow_mut();
        cache.add_instrument(instrument.clone()).unwrap();
        cache
            .add_quote(QuoteTick::new(
                instrument.id(),
                Price::from("0.400"),
                Price::from("0.410"),
                Quantity::from("10000.00"),
                Quantity::from("10000.00"),
                now,
                now,
            ))
            .unwrap();
    }
    let observer = Python::attach(|py| {
        let module = PyModule::new(py, "hosted_twap_observer").unwrap();
        module
            .setattr("Strategy", py.get_type::<PyStrategy>())
            .unwrap();
        module
            .setattr("client_id", client_id.into_py_any(py).unwrap())
            .unwrap();
        py.run(
            c_str!(
                r#"
import sqlite3
import tempfile
from pathlib import Path

directory = tempfile.TemporaryDirectory()
database = Path(directory.name) / "twap.sqlite3"
with sqlite3.connect(database) as connection:
    connection.execute("CREATE TABLE fills (event_id TEXT PRIMARY KEY, order_id TEXT UNIQUE)")
filled = []
errors = []

class TimedStrategy(Strategy):
    def on_start(self):
        self.submit_order(self.order, client_id=client_id)
    def on_stop(self): pass
    def on_dispose(self): pass
    def on_order_denied(self, event):
        errors.append(("denied", str(event.client_order_id), str(event.reason)))
    def on_order_rejected(self, event):
        errors.append(("rejected", str(event.client_order_id), str(event.reason)))
    def on_order_filled(self, event):
        try:
            assert str(self.cache.order(event.client_order_id).status) == "FILLED"
            row = (str(event.event_id), str(event.client_order_id))
            with sqlite3.connect(database, timeout=0) as connection:
                connection.execute("BEGIN IMMEDIATE")
                connection.execute("INSERT INTO fills VALUES (?, ?)", row)
            filled.append(row)
        except BaseException as error:
            errors.append(repr(error))
            raise

strategy = TimedStrategy()
def mark_waiting(): pass
def records():
    with sqlite3.connect(database) as connection:
        return connection.execute("SELECT event_id,order_id FROM fills ORDER BY rowid").fetchall()
def cleanup(): directory.cleanup()
"#
            ),
            Some(&module.dict()),
            None,
        )
        .unwrap();
        module.unbind()
    });
    let strategy = Python::attach(|py| observer.bind(py).getattr("strategy").unwrap().unbind());
    let strategy_id = node
        .kernel()
        .trader()
        .borrow_mut()
        .add_python_strategy_instance(&strategy)
        .unwrap();
    let primary_id = ClientOrderId::from("TIMED-PARENT");
    let order = OrderTestBuilder::new(OrderType::Market)
        .trader_id(trader_id)
        .strategy_id(strategy_id)
        .instrument_id(instrument.id())
        .client_order_id(primary_id)
        .side(OrderSide::Buy)
        .quantity(Quantity::from("120.00"))
        .exec_algorithm_id(algorithm_id)
        .exec_spawn_id(primary_id)
        .exec_algorithm_params(
            [
                (Ustr::from("horizon_secs"), Ustr::from("0.375")),
                (Ustr::from("interval_secs"), Ustr::from("0.125")),
            ]
            .into_iter()
            .collect(),
        )
        .build();
    Python::attach(|py| {
        strategy
            .bind(py)
            .setattr("order", order_any_to_pyobject(py, order).unwrap())
            .unwrap();
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
    let waiting = Python::attach(|py| observer.bind(py).getattr("mark_waiting").unwrap().unbind());
    engine
        .borrow_mut()
        .register_client(ready_hosted_sandbox_client(inner, waiting, stopped.clone()))
        .unwrap();
    let (py_node, records) = Python::attach(|py| {
        let node = pyo3::Py::new(py, PyLiveNode::new(node)).unwrap();
        let globals = PyDict::new(py);
        globals.set_item("node", &node).unwrap();
        globals.set_item("observer", observer.bind(py)).unwrap();
        let outcome = py.run(
            c_str!(
                r#"
import asyncio

async def exercise():
    task = asyncio.create_task(node.run_async())
    async def wait_for_slices():
        while len(observer.filled) < 3:
            if observer.errors: raise AssertionError(observer.errors)
            if task.done():
                await task
                raise AssertionError("node stopped before three native TWAP fills")
            await asyncio.sleep(0)
    try:
        await asyncio.wait_for(wait_for_slices(), timeout=3)
    finally:
        node.handle().stop()
        await asyncio.wait_for(task, timeout=3)
asyncio.run(exercise())
"#
            ),
            Some(&globals),
            None,
        );
        let rows: Vec<(String, String)> = observer
            .bind(py)
            .call_method0("records")
            .unwrap()
            .extract()
            .unwrap();
        if let Err(e) = outcome {
            e.print(py);
            eprintln!(
                "canonical TWAP primary at exit: {:?}",
                cache.borrow().order_owned(&primary_id)
            );
            node.call_method0(py, "dispose").unwrap();
            observer.bind(py).call_method0("cleanup").unwrap();
            panic!("hosted TWAP failed after {} fills", rows.len());
        }
        (node, rows)
    });
    assert_eq!(records.len(), 3);
    let cache = cache.borrow();
    let mut times = Vec::new();
    for (event_id, order_id) in records {
        let order = cache
            .order_owned(&ClientOrderId::from(order_id.as_str()))
            .unwrap();
        assert_eq!(order.status(), OrderStatus::Filled);
        assert_eq!(order.filled_qty(), Quantity::from("40.00"));
        assert_eq!(
            order
                .exec_spawn_id()
                .unwrap_or_else(|| order.client_order_id()),
            primary_id
        );
        let mut fills = order.events().into_iter().filter_map(|event| {
            if let OrderEventAny::Filled(fill) = event {
                Some(fill)
            } else {
                None
            }
        });
        let fill = fills.next().unwrap();
        assert!(fills.next().is_none());
        assert_eq!(event_id, fill.event_id.to_string());
        times.push(fill.ts_event);
    }
    assert!(times.windows(2).all(|pair| pair[0] < pair[1]));
    assert_eq!(
        cache
            .account_owned(&account_id)
            .unwrap()
            .balance_total(None)
            .unwrap(),
        Money::from("99950.8 USDC")
    );
    assert!(stopped.get() >= 1);
    assert_eq!(
        capture.capture().unwrap_err(),
        ExecutionAccountCaptureError::Retired
    );
    drop(cache);
    Python::attach(|py| {
        py_node.call_method0(py, "dispose").unwrap();
        observer.bind(py).call_method0("cleanup").unwrap();
    });
}
