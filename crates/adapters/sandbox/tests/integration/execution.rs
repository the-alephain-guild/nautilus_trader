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

//! Tests for sandbox execution client.

#[cfg(feature = "python")]
#[path = "execution/hosted_control_cancel.rs"]
mod hosted_control_cancel;
#[cfg(feature = "python")]
#[path = "execution/hosted_control_delivery.rs"]
mod hosted_control_delivery;
#[cfg(feature = "python")]
#[path = "execution/hosted_emulator_callbacks.rs"]
mod hosted_emulator_callbacks;

use std::{cell::RefCell, rc::Rc};

use nautilus_common::{
    cache::Cache,
    clients::ExecutionClient,
    clock::{Clock, TestClock},
    live::set_exec_event_sender,
    messages::{
        ExecutionEvent,
        execution::{
            BatchModifyOrders, CancelAllOrders, ModifyOrder, SubmitOrder, SubmitOrderList,
            TradingCommand,
        },
    },
    msgbus::{
        self, MessageBus, MessagingSwitchboard, TypedHandler,
        stubs::get_typed_into_message_saving_handler, typed_handler::TypedIntoHandler,
    },
};
use nautilus_core::{DurationNanos, UUID4, UnixNanos};
use nautilus_data::engine::DataEngine;
#[cfg(feature = "python")]
use nautilus_execution::python::fee::PythonFeeModel;
use nautilus_execution::{
    client::core::ExecutionClientCore,
    engine::ExecutionEngine,
    models::{
        fee::{FeeModelAny, ProbabilityPriceFeeModel},
        fill::{DefaultFillModel, FillModel, FillModelAny, FillModelHandle},
    },
};
use nautilus_model::{
    accounts::{AccountAny, MarginAccount},
    data::{Bar, BarType, Data, InstrumentClose, InstrumentStatus, QuoteTick, TradeTick},
    enums::{
        AccountType, AggressorSide, BookType, InstrumentCloseType, MarketStatusAction, OmsType,
        OrderSide, OrderStatus, OrderType, PositionSide, TimeInForce,
    },
    events::{
        AccountState, OrderEventAny, OrderFilled, PositionClosed, PositionEvent,
        account::stubs::margin_account_state,
    },
    identifiers::{
        AccountId, ClientId, ClientOrderId, InstrumentId, OrderListId, PositionId, StrategyId,
        TradeId, TraderId, Venue,
    },
    instruments::{
        CryptoPerpetual, Instrument, InstrumentAny,
        stubs::{binary_option, crypto_perpetual_ethusdt},
    },
    orders::{Order, OrderAny, OrderList, OrderTestBuilder, stubs::TestOrderEventStubs},
    position::Position,
    types::{Currency, Money, Price, Quantity},
};
use nautilus_sandbox::{SandboxExecutionClient, SandboxExecutionClientConfig};
#[cfg(feature = "python")]
use pyo3::{IntoPyObjectExt, Python, ffi::c_str, types::PyAnyMethods};
use rstest::{fixture, rstest};
use rust_decimal::Decimal;
use ustr::Ustr;

#[fixture]
fn trader_id() -> TraderId {
    TraderId::from("SANDBOX-001")
}

#[fixture]
fn account_id() -> AccountId {
    AccountId::from("SANDBOX-001")
}

#[fixture]
fn venue() -> Venue {
    Venue::new("SIM")
}

#[fixture]
fn client_id() -> ClientId {
    ClientId::new("SANDBOX")
}

#[fixture]
fn instrument(crypto_perpetual_ethusdt: CryptoPerpetual) -> InstrumentAny {
    InstrumentAny::CryptoPerpetual(crypto_perpetual_ethusdt)
}

fn create_config(
    _trader_id: TraderId,
    account_id: AccountId,
    venue: Venue,
) -> SandboxExecutionClientConfig {
    let usd = Currency::USD();
    SandboxExecutionClientConfig {
        account_id,
        venue,
        starting_balances: vec![Money::new(100_000.0, usd)],
        base_currency: Some(usd),
        oms_type: OmsType::Netting,
        account_type: AccountType::Margin,
        default_leverage: Decimal::ONE,
        leverages: ahash::AHashMap::new(),
        book_type: BookType::L1_MBP,
        fee_model: None,
        fill_model: None,
        frozen_account: false,
        bar_execution: false,
        trade_execution: false,
        reject_stop_orders: true,
        support_gtd_orders: true,
        support_contingent_orders: true,
        use_position_ids: true,
        use_random_ids: false,
        use_reduce_only: true,
        queue_position: false,
        liquidity_consumption: false,
        bar_adaptive_high_low_ordering: false,
        use_market_order_acks: false,
        oto_full_trigger: false,
        price_protection_points: 0,
    }
}

#[fixture]
fn config(
    trader_id: TraderId,
    account_id: AccountId,
    venue: Venue,
) -> SandboxExecutionClientConfig {
    create_config(trader_id, account_id, venue)
}

/// Test context bundling execution client with shared cache for tests that need both
struct TestContext {
    client: SandboxExecutionClient,
    cache: Rc<RefCell<Cache>>,
}

fn create_test_context(trader_id: TraderId, account_id: AccountId, venue: Venue) -> TestContext {
    create_test_context_with(trader_id, account_id, venue, |_| {})
}

fn create_test_context_with_trade_execution(
    trader_id: TraderId,
    account_id: AccountId,
    venue: Venue,
) -> TestContext {
    create_test_context_with(trader_id, account_id, venue, |config| {
        config.trade_execution = true;
    })
}

fn create_test_context_with(
    trader_id: TraderId,
    account_id: AccountId,
    venue: Venue,
    customize: impl FnOnce(&mut SandboxExecutionClientConfig),
) -> TestContext {
    let cache = Rc::new(RefCell::new(Cache::default()));
    let clock: Rc<RefCell<dyn Clock>> = Rc::new(RefCell::new(TestClock::new()));
    let mut config = create_config(trader_id, account_id, venue);
    customize(&mut config);

    let core = ExecutionClientCore::new(
        trader_id,
        ClientId::new("SANDBOX"),
        config.venue,
        config.oms_type,
        config.account_id,
        config.account_type,
        config.base_currency,
        cache.clone(),
    );

    let client = SandboxExecutionClient::new(core, config, clock, cache.clone());
    TestContext { client, cache }
}

#[fixture]
fn test_context(trader_id: TraderId, account_id: AccountId, venue: Venue) -> TestContext {
    create_test_context(trader_id, account_id, venue)
}

#[fixture]
fn execution_client(test_context: TestContext) -> SandboxExecutionClient {
    test_context.client
}

#[rstest]
fn test_account_capture_never_substitutes_starting_balances(test_context: TestContext) {
    let probe = test_context
        .client
        .account_capture_probe()
        .expect("Sandbox exposes its actual account owner");

    let result = probe.capture();

    assert!(
        result.is_err(),
        "An uninitialized account is not its configured balance"
    );
}

#[rstest]
fn test_account_capture_source_binds_the_actual_base_denomination(
    trader_id: TraderId,
    account_id: AccountId,
    venue: Venue,
) {
    let original = create_test_context_with(trader_id, account_id, venue, |config| {
        config.base_currency = Some(Currency::USD());
    });
    let changed = create_test_context_with(trader_id, account_id, venue, |config| {
        let mut denomination = Currency::USD();
        denomination.precision += 1;
        config.base_currency = Some(denomination);
    });
    let original = original.client.account_capture_probe().unwrap();
    let changed = changed.client.account_capture_probe().unwrap();
    assert_ne!(
        original.source().configuration_sha256,
        changed.source().configuration_sha256,
        "Code-only currency serialization must not erase the actual native configuration"
    );
}

#[rstest]
#[tokio::test]
async fn test_account_capture_reads_the_actual_account_and_retires(
    trader_id: TraderId,
    account_id: AccountId,
    venue: Venue,
) {
    *msgbus::get_message_bus().borrow_mut() = MessageBus::default();
    let mut context = create_test_context_with(trader_id, account_id, venue, |config| {
        config.account_type = AccountType::Cash;
        config.starting_balances = vec![Money::from("100 USD"), Money::from("2 BTC")];
    });
    setup_account_state_handler(context.cache.clone());
    let probe = context.client.account_capture_probe().unwrap();
    context.client.connect().await.unwrap();
    let initial = probe.capture().unwrap();
    assert!(initial.no_borrowing_model);
    assert_eq!(initial.balances.len(), 2);
    assert_eq!(initial.balances[0].asset_code, "BTC");
    assert_eq!(initial.balances[0].total, "2");
    assert_eq!(initial.source.configuration_sha256.len(), 64);

    {
        let mut cache = context.cache.borrow_mut();
        let mut account = cache.account_owned(&account_id).unwrap();
        let mut state = account.events().last().unwrap().clone();
        state.event_id = UUID4::new();
        *state
            .balances
            .iter_mut()
            .find(|balance| balance.currency == Currency::USD())
            .unwrap() = nautilus_model::types::AccountBalance::new(
            Money::from("91 USD"),
            Money::from("1 USD"),
            Money::from("90 USD"),
        );
        state
            .balances
            .push(nautilus_model::types::AccountBalance::new(
                Money::from("3 ETH"),
                Money::zero(Currency::ETH()),
                Money::from("3 ETH"),
            ));
        account.apply(state).unwrap();
        cache.update_account(&account).unwrap();
    }
    let updated = probe.capture().unwrap();
    assert_eq!(
        updated
            .balances
            .iter()
            .find(|balance| balance.asset_code == "USD")
            .unwrap()
            .total,
        "91"
    );
    assert_eq!(
        updated
            .balances
            .iter()
            .find(|row| row.asset_code == "ETH")
            .unwrap()
            .total,
        "3"
    );
    assert_eq!(updated.source.incarnation, initial.source.incarnation);
    {
        let _held = context.cache.borrow_mut();
        assert_eq!(
            probe.capture().unwrap_err(),
            nautilus_common::clients::ExecutionAccountCaptureError::Borrowed
        );
    }
    *context.cache.borrow_mut() = Cache::default();
    assert_eq!(
        probe.capture().unwrap_err(),
        nautilus_common::clients::ExecutionAccountCaptureError::AccountUnavailable
    );
    context.client.disconnect().await.unwrap();
    assert_eq!(
        probe.capture().unwrap_err(),
        nautilus_common::clients::ExecutionAccountCaptureError::Retired
    );
}

#[rstest]
#[case::precision(0)]
#[case::currency_type(1)]
#[case::iso4217(2)]
#[case::mixed_balance_denominations(3)]
#[case::current_account_base_denomination(4)]
#[tokio::test]
async fn test_account_capture_rejects_same_code_denomination_changes(
    trader_id: TraderId,
    account_id: AccountId,
    venue: Venue,
    #[case] change: u8,
) {
    *msgbus::get_message_bus().borrow_mut() = MessageBus::default();
    let mut context = create_test_context_with(trader_id, account_id, venue, |config| {
        config.account_type = AccountType::Cash;
        config.starting_balances = vec![Money::from("100 USD")];
    });
    setup_account_state_handler(context.cache.clone());
    let probe = context.client.account_capture_probe().unwrap();
    context.client.connect().await.unwrap();
    assert!(probe.capture().is_ok());
    {
        let mut cache = context.cache.borrow_mut();
        let mut account = cache.account_owned(&account_id).unwrap();
        let mut state = account.events().last().unwrap().clone();
        state.event_id = UUID4::new();
        let mut changed = Currency::USD();
        match change {
            0 | 3 | 4 => changed.precision += 1,
            1 => changed.currency_type = nautilus_model::enums::CurrencyType::Crypto,
            2 => changed.iso4217 += 1,
            _ => unreachable!(),
        }
        let total_currency = if change >= 3 {
            Currency::USD()
        } else {
            changed
        };
        state.balances = vec![nautilus_model::types::AccountBalance::new(
            Money::new(100.0, total_currency),
            Money::zero(if change == 4 {
                Currency::USD()
            } else {
                changed
            }),
            Money::new(100.0, total_currency),
        )];
        account.apply(state).unwrap();
        if change == 4 {
            let AccountAny::Cash(cash) = &mut account else {
                unreachable!()
            };
            cash.base.base_currency = Some(changed);
        }
        cache.update_account(&account).unwrap();
    }
    assert_eq!(
        probe.capture().unwrap_err(),
        nautilus_common::clients::ExecutionAccountCaptureError::InconsistentState
    );
    context.client.disconnect().await.unwrap();
}

#[rstest]
#[tokio::test]
async fn test_account_capture_keeps_negative_native_balances_without_claiming_no_borrowing(
    trader_id: TraderId,
    account_id: AccountId,
    venue: Venue,
) {
    *msgbus::get_message_bus().borrow_mut() = MessageBus::default();
    let mut context = create_test_context_with(trader_id, account_id, venue, |config| {
        config.account_type = AccountType::Margin;
        config.starting_balances = vec![Money::from("100 USD")];
    });
    setup_account_state_handler(context.cache.clone());
    let probe = context.client.account_capture_probe().unwrap();
    context.client.connect().await.unwrap();
    {
        let mut cache = context.cache.borrow_mut();
        let mut account = cache.account_owned(&account_id).unwrap();
        let mut state = account.events().last().unwrap().clone();
        state.event_id = UUID4::new();
        state.balances = vec![nautilus_model::types::AccountBalance::new(
            Money::from("-1.25 USD"),
            Money::zero(Currency::USD()),
            Money::from("-1.25 USD"),
        )];
        account.apply(state).unwrap();
        cache.update_account(&account).unwrap();
    }
    let captured = probe.capture().unwrap();
    assert_eq!(captured.balances[0].total, "-1.25");
    assert_eq!(captured.balances[0].free, "-1.25");
    assert!(!captured.no_borrowing_model);
    assert_eq!(captured.source.account_type, "MARGIN");
    context.client.disconnect().await.unwrap();
}

fn create_quote_tick_with_price_precision(
    instrument_id: InstrumentId,
    bid: f64,
    ask: f64,
    price_precision: u8,
) -> QuoteTick {
    QuoteTick::new(
        instrument_id,
        Price::new(bid, price_precision),
        Price::new(ask, price_precision),
        Quantity::new(100.0, 3),
        Quantity::new(100.0, 3),
        UnixNanos::default(),
        UnixNanos::default(),
    )
}

fn create_quote_tick(instrument_id: InstrumentId, bid: f64, ask: f64) -> QuoteTick {
    // Use price precision 2 to match crypto_perpetual_ethusdt fixture.
    create_quote_tick_with_price_precision(instrument_id, bid, ask, 2)
}

fn create_mismatched_quote_tick(instrument_id: InstrumentId, bid: f64, ask: f64) -> QuoteTick {
    // Uses price precision 3 (instrument fixture uses 2), should be rejected by sandbox guard.
    create_quote_tick_with_price_precision(instrument_id, bid, ask, 3)
}

fn create_trade_tick_with_precision(
    instrument_id: InstrumentId,
    price: f64,
    size: f64,
    price_precision: u8,
    size_precision: u8,
) -> TradeTick {
    TradeTick::new(
        instrument_id,
        Price::new(price, price_precision),
        Quantity::new(size, size_precision),
        AggressorSide::Buy,
        TradeId::new("1"),
        UnixNanos::default(),
        UnixNanos::default(),
    )
}

fn create_mismatched_trade_tick(instrument_id: InstrumentId) -> TradeTick {
    // Uses price precision 3 (instrument fixture uses 2), should be rejected by sandbox guard.
    create_trade_tick_with_precision(instrument_id, 1000.0, 1.0, 3, 3)
}

fn make_binary_option_instrument(
    condition_id: &str,
    token_id: &str,
    outcome: &str,
    expiration_ns: u64,
) -> InstrumentAny {
    let mut binary = binary_option();
    let raw_symbol = format!("{condition_id}-{token_id}");
    binary.raw_symbol = raw_symbol.as_str().into();
    binary.id = InstrumentId::from(format!("{raw_symbol}.POLYMARKET").as_str());
    binary.activation_ns = UnixNanos::from(1);
    binary.expiration_ns = UnixNanos::from(expiration_ns);
    binary.outcome = Some(Ustr::from(outcome));
    InstrumentAny::BinaryOption(binary)
}

fn create_binary_option_quote(instrument_id: InstrumentId) -> QuoteTick {
    QuoteTick::new(
        instrument_id,
        Price::new(0.40, 3),
        Price::new(0.41, 3),
        Quantity::new(100.0, 2),
        Quantity::new(100.0, 2),
        UnixNanos::default(),
        UnixNanos::default(),
    )
}

fn submit_open_position_and_seed_cache(
    client: &SandboxExecutionClient,
    cache: &Rc<RefCell<Cache>>,
    trader_id: TraderId,
    instrument: &InstrumentAny,
    client_order_id: &str,
    position_id: &str,
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<ExecutionEvent>,
) -> Position {
    submit_market_open_order(client, cache, trader_id, instrument, client_order_id, 10);

    let mut filled = None;

    for event in std::iter::from_fn(|| rx.try_recv().ok()) {
        let ExecutionEvent::Order(OrderEventAny::Filled(fill)) = event else {
            continue;
        };

        if fill.client_order_id.as_str() == client_order_id {
            filled = Some(fill);
            break;
        }
    }

    let fill_event =
        OrderEventAny::Filled(filled.expect("expected opening fill from sandbox market order"));
    cache.borrow_mut().update_order(&fill_event).unwrap();

    let OrderEventAny::Filled(mut filled) = fill_event else {
        unreachable!("constructed filled order event");
    };
    filled.position_id = Some(PositionId::new(position_id));
    Position::new(instrument, filled)
}

fn position_closed_event(position: &Position, account_id: AccountId) -> PositionEvent {
    PositionEvent::PositionClosed(PositionClosed {
        trader_id: position.trader_id,
        strategy_id: position.strategy_id,
        instrument_id: position.instrument_id,
        position_id: position.id,
        account_id,
        opening_order_id: position.opening_order_id,
        closing_order_id: position.closing_order_id,
        entry: position.entry,
        side: PositionSide::Flat,
        signed_qty: 0.0,
        quantity: Quantity::zero(position.size_precision),
        peak_quantity: position.peak_qty,
        last_qty: Quantity::zero(position.size_precision),
        last_px: Price::zero(position.price_precision),
        currency: position.quote_currency,
        avg_px_open: position.avg_px_open,
        avg_px_close: position.avg_px_close,
        realized_return: position.realized_return,
        realized_pnl: position.realized_pnl,
        unrealized_pnl: Money::zero(position.quote_currency),
        duration: DurationNanos::new(1),
        event_id: UUID4::new(),
        ts_opened: position.ts_opened,
        ts_closed: position.ts_closed.or(Some(position.ts_last)),
        ts_event: position.ts_last,
        ts_init: position.ts_last,
    })
}

fn settle_position_from_expiration_fill(
    cache: &Rc<RefCell<Cache>>,
    position: &Position,
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<ExecutionEvent>,
) -> Position {
    let mut expiration_fill = None;

    for event in std::iter::from_fn(|| rx.try_recv().ok()) {
        let ExecutionEvent::Order(OrderEventAny::Filled(fill)) = event else {
            continue;
        };

        if fill.client_order_id.as_str().starts_with("EXPIRATION-") {
            expiration_fill = Some(fill);
            break;
        }
    }

    let expiration_fill = expiration_fill.expect("expected expiration fill after InstrumentClose");

    let mut closed = position.clone();
    closed.apply(&expiration_fill);
    cache.borrow_mut().update_position(&closed).unwrap();
    closed
}

fn apply_order_events_from_channel(
    cache: &Rc<RefCell<Cache>>,
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<ExecutionEvent>,
) -> Vec<OrderEventAny> {
    let mut order_events = Vec::new();

    for event in std::iter::from_fn(|| rx.try_recv().ok()) {
        let ExecutionEvent::Order(order_event) = event else {
            continue;
        };

        let _ = cache.borrow_mut().update_order(&order_event);
        order_events.push(order_event);
    }

    order_events
}

fn create_submit_order_list(
    trader_id: TraderId,
    client_id: ClientId,
    instrument_id: InstrumentId,
    orders: &[OrderAny],
) -> SubmitOrderList {
    let strategy_id = orders
        .first()
        .expect("expected non-empty order list")
        .strategy_id();
    let order_list = OrderList::new(
        OrderListId::from("OL-SANDBOX-001"),
        instrument_id,
        strategy_id,
        orders.iter().map(OrderAny::client_order_id).collect(),
        UnixNanos::default(),
    );

    SubmitOrderList {
        trader_id,
        client_id: Some(client_id),
        strategy_id,
        instrument_id,
        order_list,
        order_inits: orders
            .iter()
            .map(|order| order.init_event().clone())
            .collect(),
        exec_algorithm_id: None,
        position_id: None,
        params: None,
        command_id: UUID4::new(),
        ts_init: UnixNanos::default(),
        correlation_id: None,
        causation_id: None,
    }
}

fn seed_binary_option_position_from_fill(
    cache: &Rc<RefCell<Cache>>,
    instrument: &InstrumentAny,
    fill: OrderFilled,
    position_id: &str,
) {
    let mut fill = fill;
    fill.position_id = Some(PositionId::new(position_id));
    let position = Position::new(instrument, fill);
    cache
        .borrow_mut()
        .add_position(&position, OmsType::Netting)
        .unwrap();
}

fn submit_market_open_order(
    client: &SandboxExecutionClient,
    cache: &Rc<RefCell<Cache>>,
    trader_id: TraderId,
    instrument: &InstrumentAny,
    client_order_id: &str,
    ts_init: u64,
) {
    let order = OrderTestBuilder::new(OrderType::Market)
        .instrument_id(instrument.id())
        .side(OrderSide::Buy)
        .quantity(Quantity::from("1.00"))
        .client_order_id(client_order_id.into())
        .ts_init(UnixNanos::from(ts_init))
        .submit(true)
        .build();
    cache
        .borrow_mut()
        .add_order(order.clone(), None, None, false)
        .unwrap();
    client
        .submit_order(SubmitOrder::from_order(
            &order,
            trader_id,
            Some(client.client_id()),
            None,
            UUID4::new(),
            UnixNanos::from(ts_init),
        ))
        .unwrap();
}

struct BinaryOptionLifecycleHarness {
    client: SandboxExecutionClient,
    cache: Rc<RefCell<Cache>>,
    test_clock: Rc<RefCell<TestClock>>,
    instrument: InstrumentAny,
    rx: tokio::sync::mpsc::UnboundedReceiver<ExecutionEvent>,
}

fn setup_binary_option_lifecycle_harness(
    trader_id: TraderId,
    account_id: AccountId,
    condition_id: &str,
    token_id: &str,
    outcome: &str,
    expiration_ns: u64,
) -> BinaryOptionLifecycleHarness {
    let instrument = make_binary_option_instrument(condition_id, token_id, outcome, expiration_ns);
    let venue = instrument.id().venue;
    let cache = Rc::new(RefCell::new(Cache::default()));
    let test_clock = Rc::new(RefCell::new(TestClock::new()));
    let clock: Rc<RefCell<dyn Clock>> = test_clock.clone();
    let config = create_config(trader_id, account_id, venue);
    let core = ExecutionClientCore::new(
        trader_id,
        ClientId::new("SANDBOX"),
        config.venue,
        config.oms_type,
        config.account_id,
        config.account_type,
        config.base_currency,
        cache.clone(),
    );
    let mut client = SandboxExecutionClient::new(core, config, clock, cache.clone());
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<ExecutionEvent>();

    set_exec_event_sender(tx);
    cache
        .borrow_mut()
        .add_instrument(instrument.clone())
        .unwrap();
    client.start().unwrap();
    client
        .process_quote_tick(&create_binary_option_quote(instrument.id()))
        .unwrap();

    BinaryOptionLifecycleHarness {
        client,
        cache,
        test_clock,
        instrument,
        rx,
    }
}

fn publish_expired_close(
    test_clock: &Rc<RefCell<TestClock>>,
    instrument: &InstrumentAny,
    close_price: Price,
    ts_ns: u64,
) {
    let _ = test_clock
        .borrow_mut()
        .advance_time(UnixNanos::from(ts_ns), true);

    let close = InstrumentClose::new(
        instrument.id(),
        close_price,
        InstrumentCloseType::ContractExpired,
        UnixNanos::from(ts_ns),
        UnixNanos::from(ts_ns),
    );
    msgbus::publish_any(
        nautilus_common::msgbus::switchboard::get_instrument_close_topic(instrument.id()),
        &close,
    );
}

struct PendingResolutionHarness {
    context: TestContext,
    instrument: InstrumentAny,
    clock: Rc<RefCell<dyn Clock>>,
    rx: tokio::sync::mpsc::UnboundedReceiver<ExecutionEvent>,
}

fn setup_pending_resolution_harness(
    trader_id: TraderId,
    account_id: AccountId,
    client_order_suffix: &str,
) -> PendingResolutionHarness {
    let mut binary = binary_option();
    binary.activation_ns = UnixNanos::from(1);
    binary.expiration_ns = UnixNanos::from(100);
    let instrument = InstrumentAny::BinaryOption(binary);
    let venue = instrument.id().venue;
    let cache = Rc::new(RefCell::new(Cache::default()));
    let test_clock = Rc::new(RefCell::new(TestClock::new()));
    let clock: Rc<RefCell<dyn Clock>> = test_clock.clone();

    let mut config = create_config(trader_id, account_id, venue);
    config.base_currency = Some(Currency::USDC());
    config.starting_balances = vec![Money::new(100_000.0, Currency::USDC())];
    let core = ExecutionClientCore::new(
        trader_id,
        ClientId::new("SANDBOX"),
        config.venue,
        config.oms_type,
        config.account_id,
        config.account_type,
        config.base_currency,
        cache.clone(),
    );
    let mut client = SandboxExecutionClient::new(core, config, clock.clone(), cache.clone());

    cache
        .borrow_mut()
        .add_instrument(instrument.clone())
        .unwrap();
    let _ = test_clock
        .borrow_mut()
        .advance_time(UnixNanos::from(50), true);

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<ExecutionEvent>();
    nautilus_common::live::runner::replace_exec_event_sender(tx);
    client.start().unwrap();

    let quote = QuoteTick::new(
        instrument.id(),
        Price::new(0.40, 3),
        Price::new(0.41, 3),
        Quantity::new(100.0, 2),
        Quantity::new(100.0, 2),
        UnixNanos::default(),
        UnixNanos::default(),
    );
    client.process_quote_tick(&quote).unwrap();

    let position = submit_open_position_and_seed_cache(
        &client,
        &cache,
        trader_id,
        &instrument,
        &format!("OPEN-{client_order_suffix}"),
        &format!("P-{client_order_suffix}"),
        &mut rx,
    );
    cache
        .borrow_mut()
        .add_position(&position, OmsType::Netting)
        .unwrap();

    let resting_order = OrderTestBuilder::new(OrderType::Limit)
        .instrument_id(instrument.id())
        .side(OrderSide::Buy)
        .price(Price::from("0.050"))
        .quantity(Quantity::from("1.00"))
        .client_order_id(format!("REST-{client_order_suffix}").into())
        .ts_init(UnixNanos::from(20))
        .submit(true)
        .build();
    cache
        .borrow_mut()
        .add_order(resting_order.clone(), None, None, false)
        .unwrap();
    client
        .submit_order(SubmitOrder::from_order(
            &resting_order,
            trader_id,
            Some(client.client_id()),
            None,
            UUID4::new(),
            UnixNanos::from(20),
        ))
        .unwrap();

    let _ = test_clock
        .borrow_mut()
        .advance_time(UnixNanos::from(200), true);

    let probe_order = OrderTestBuilder::new(OrderType::Limit)
        .instrument_id(instrument.id())
        .side(OrderSide::Buy)
        .price(Price::from("0.050"))
        .quantity(Quantity::from("1.00"))
        .client_order_id(format!("PROBE-{client_order_suffix}").into())
        .ts_init(UnixNanos::from(200))
        .submit(true)
        .build();
    cache
        .borrow_mut()
        .add_order(probe_order.clone(), None, None, false)
        .unwrap();
    client
        .submit_order(SubmitOrder::from_order(
            &probe_order,
            trader_id,
            Some(client.client_id()),
            None,
            UUID4::new(),
            UnixNanos::from(200),
        ))
        .unwrap();

    PendingResolutionHarness {
        context: TestContext { client, cache },
        instrument,
        clock,
        rx,
    }
}

fn assert_pending_resolution_transition(
    harness: &mut PendingResolutionHarness,
    resting_order_id: &str,
    probe_order_id: &str,
) {
    let mut seen_resting_canceled = false;
    let mut seen_probe_rejected = false;

    for event in std::iter::from_fn(|| harness.rx.try_recv().ok()) {
        if let ExecutionEvent::Order(order_event) = event {
            match order_event {
                OrderEventAny::Canceled(c) if c.client_order_id.as_str() == resting_order_id => {
                    seen_resting_canceled = true;
                }
                OrderEventAny::Rejected(r) if r.client_order_id.as_str() == probe_order_id => {
                    seen_probe_rejected = r.reason.contains("pending resolution");
                }
                _ => {}
            }
        }
    }

    assert!(
        seen_resting_canceled,
        "expected resting order cancellation at pending_resolution boundary"
    );
    assert!(
        seen_probe_rejected,
        "expected probe order rejection with pending resolution reason"
    );
}

fn updated_instrument_with_price_precision_3(instrument: InstrumentAny) -> InstrumentAny {
    match instrument {
        InstrumentAny::CryptoPerpetual(mut crypto_perp) => {
            crypto_perp.price_precision = 3;
            crypto_perp.price_increment = Price::from("0.001");
            InstrumentAny::CryptoPerpetual(crypto_perp)
        }
        _ => panic!("Test fixture expected CryptoPerpetual instrument"),
    }
}

fn setup_order_event_handler() {
    let (handler, _saving_handler) = get_typed_into_message_saving_handler::<OrderEventAny>(Some(
        Ustr::from("ExecEngine.process"),
    ));
    msgbus::register_order_event_endpoint(MessagingSwitchboard::exec_engine_process(), handler);
}

fn setup_account_state_handler(cache: Rc<RefCell<Cache>>) {
    let handler = TypedHandler::from(move |state: &AccountState| {
        cache.borrow_mut().update_account_state(state).unwrap();
    });
    msgbus::register_account_state_endpoint(
        MessagingSwitchboard::portfolio_update_account(),
        handler,
    );
}

#[rstest]
fn test_config_default() {
    let config = SandboxExecutionClientConfig::default();

    assert_eq!(config.account_id, AccountId::from("SANDBOX-001"));
    assert_eq!(config.venue, Venue::new("SANDBOX"));
    assert!(config.starting_balances.is_empty());
    assert!(config.base_currency.is_none());
    assert_eq!(config.oms_type, OmsType::Netting);
    assert_eq!(config.account_type, AccountType::Margin);
    assert_eq!(config.default_leverage, Decimal::ONE);
    assert_eq!(config.book_type, BookType::L1_MBP);
    assert!(config.fee_model.is_none());
    assert!(config.fill_model.is_none());
    assert!(!config.frozen_account);
    assert!(config.bar_execution);
    assert!(config.trade_execution);
    assert!(config.reject_stop_orders);
    assert!(config.support_gtd_orders);
    assert!(config.support_contingent_orders);
    assert!(config.use_position_ids);
    assert!(!config.use_random_ids);
    assert!(config.use_reduce_only);
    assert!(!config.queue_position);
    assert!(!config.liquidity_consumption);
    assert!(!config.bar_adaptive_high_low_ordering);
    assert!(!config.use_market_order_acks);
    assert!(!config.oto_full_trigger);
    assert_eq!(config.price_protection_points, 0);
}

#[rstest]
#[case::sports_p50("0.03", "0.500", "0.00750")]
#[case::sports_p30("0.03", "0.300", "0.00630")]
#[case::crypto_p97("0.072", "0.970", "0.00210")]
fn test_probability_price_fee_model_config_drives_sandbox_commission(
    #[case] taker_fee: &str,
    #[case] price: &str,
    #[case] expected: &str,
    trader_id: TraderId,
    account_id: AccountId,
) {
    assert_fee_model_config_drives_sandbox_commission(
        FeeModelAny::ProbabilityPrice(ProbabilityPriceFeeModel),
        taker_fee,
        price,
        expected,
        trader_id,
        account_id,
    );
}

#[cfg(feature = "python")]
#[rstest]
fn test_python_fee_model_config_drives_sandbox_commission(
    trader_id: TraderId,
    account_id: AccountId,
) {
    Python::initialize();

    let expected = "1.234567";
    let fee_model = Python::attach(|py| {
        let model = py
            .eval(
                c_str!(
                    "type('CustomFeeModel', (), {\
                        'get_commission': \
                            lambda self, order, fill_quantity, fill_px, instrument: self.commission\
                    })()"
                ),
                None,
                None,
            )
            .unwrap();
        model
            .setattr(
                "commission",
                Money::from(format!("{expected} USDC").as_str())
                    .into_py_any(py)
                    .unwrap(),
            )
            .unwrap();

        FeeModelAny::Python(PythonFeeModel::new(model.unbind()))
    });

    assert_fee_model_config_drives_sandbox_commission(
        fee_model, "0.03", "0.500", expected, trader_id, account_id,
    );
}

fn assert_fee_model_config_drives_sandbox_commission(
    fee_model: FeeModelAny,
    taker_fee: &str,
    price: &str,
    expected: &str,
    trader_id: TraderId,
    account_id: AccountId,
) {
    setup_order_event_handler();

    let mut binary = binary_option();
    binary.taker_fee = Decimal::from_str_exact(taker_fee).unwrap();
    let instrument = InstrumentAny::BinaryOption(binary);
    let venue = instrument.id().venue;
    let cache = Rc::new(RefCell::new(Cache::default()));
    let clock: Rc<RefCell<dyn Clock>> = Rc::new(RefCell::new(TestClock::new()));

    let mut config = create_config(trader_id, account_id, venue);
    config.base_currency = Some(Currency::USDC());
    config.starting_balances = vec![Money::new(100_000.0, Currency::USDC())];
    config.fee_model = Some(fee_model);

    let core = ExecutionClientCore::new(
        trader_id,
        ClientId::new("SANDBOX"),
        config.venue,
        config.oms_type,
        config.account_id,
        config.account_type,
        config.base_currency,
        cache.clone(),
    );
    let mut client = SandboxExecutionClient::new(core, config, clock, cache.clone());

    cache
        .borrow_mut()
        .add_instrument(instrument.clone())
        .unwrap();

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<ExecutionEvent>();
    set_exec_event_sender(tx);
    client.start().unwrap();

    let quote = QuoteTick::new(
        instrument.id(),
        Price::from(price),
        Price::from(price),
        Quantity::new(100.0, 2),
        Quantity::new(100.0, 2),
        UnixNanos::default(),
        UnixNanos::default(),
    );
    client.process_quote_tick(&quote).unwrap();

    submit_market_open_order(&client, &cache, trader_id, &instrument, "OPEN-FEE", 10);

    let mut fill_commission = None;

    for event in std::iter::from_fn(|| rx.try_recv().ok()) {
        let ExecutionEvent::Order(OrderEventAny::Filled(fill)) = event else {
            continue;
        };

        if fill.client_order_id.as_str() == "OPEN-FEE" {
            fill_commission = fill.commission;
        }
    }

    assert_eq!(
        fill_commission,
        Some(Money::from(format!("{expected} USDC").as_str()))
    );
}

#[rstest]
fn test_config_builder(account_id: AccountId, venue: Venue) {
    let usd = Currency::USD();
    let starting_balances = vec![Money::new(50_000.0, usd)];

    let config = SandboxExecutionClientConfig::builder()
        .account_id(account_id)
        .venue(venue)
        .starting_balances(starting_balances)
        .build();

    assert_eq!(config.account_id, account_id);
    assert_eq!(config.venue, venue);
    assert_eq!(config.starting_balances.len(), 1);
    assert_eq!(config.starting_balances[0].as_f64(), 50_000.0);
}

#[rstest]
fn test_config_builder_with_overrides(account_id: AccountId, venue: Venue) {
    let usd = Currency::USD();
    let starting_balances = vec![Money::new(50_000.0, usd)];

    let config = SandboxExecutionClientConfig::builder()
        .account_id(account_id)
        .venue(venue)
        .starting_balances(starting_balances)
        .base_currency(usd)
        .oms_type(OmsType::Hedging)
        .account_type(AccountType::Cash)
        .default_leverage(Decimal::new(10, 0))
        .book_type(BookType::L2_MBP)
        .frozen_account(true)
        .bar_execution(false)
        .trade_execution(true)
        .build();

    assert_eq!(config.base_currency, Some(usd));
    assert_eq!(config.oms_type, OmsType::Hedging);
    assert_eq!(config.account_type, AccountType::Cash);
    assert_eq!(config.default_leverage, Decimal::new(10, 0));
    assert_eq!(config.book_type, BookType::L2_MBP);
    assert!(config.frozen_account);
    assert!(!config.bar_execution);
    assert!(config.trade_execution);
}

#[rstest]
fn test_config_to_matching_engine_config(config: SandboxExecutionClientConfig) {
    let engine_config = config.to_matching_engine_config();

    assert!(!engine_config.bar_execution);
    assert!(!engine_config.trade_execution);
    assert!(engine_config.reject_stop_orders);
    assert!(engine_config.support_gtd_orders);
    assert!(engine_config.support_contingent_orders);
    assert!(engine_config.use_position_ids);
    assert!(!engine_config.use_random_ids);
    assert!(engine_config.use_reduce_only);
    assert!(!engine_config.queue_position);
    assert!(!engine_config.liquidity_consumption);
    assert!(!engine_config.bar_adaptive_high_low_ordering);
    assert!(!engine_config.use_market_order_acks);
    assert!(!engine_config.oto_full_trigger);
    assert_eq!(engine_config.price_protection_points, None);
}

#[rstest]
#[case::queue_on(true, false)]
#[case::queue_off(false, true)]
fn test_queue_position_gates_trade_driven_limit_fill(
    #[case] queue_position: bool,
    #[case] expect_fill: bool,
    trader_id: TraderId,
    account_id: AccountId,
    instrument: InstrumentAny,
) {
    setup_order_event_handler();

    let venue = instrument.id().venue;
    let mut test_context = create_test_context_with(trader_id, account_id, venue, |config| {
        config.trade_execution = true;
        config.queue_position = queue_position;
    });
    let cache = test_context.cache.clone();
    cache
        .borrow_mut()
        .add_instrument(instrument.clone())
        .unwrap();

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<ExecutionEvent>();
    set_exec_event_sender(tx);
    test_context.client.start().unwrap();

    let quote = create_quote_tick(instrument.id(), 1000.0, 1001.0);
    test_context.client.process_quote_tick(&quote).unwrap();

    let order = OrderTestBuilder::new(OrderType::Limit)
        .trader_id(trader_id)
        .instrument_id(instrument.id())
        .client_order_id(ClientOrderId::from("QUEUE-LIMIT-001"))
        .side(OrderSide::Buy)
        .price(Price::from("1000.00"))
        .quantity(Quantity::from("1.000"))
        .submit(true)
        .build();
    cache
        .borrow_mut()
        .add_order(order.clone(), None, Some(ClientId::new("SANDBOX")), false)
        .unwrap();
    test_context
        .client
        .submit_order(SubmitOrder::from_order(
            &order,
            trader_id,
            Some(test_context.client.client_id()),
            None,
            UUID4::new(),
            UnixNanos::default(),
        ))
        .unwrap();

    let accepted_events = apply_order_events_from_channel(&cache, &mut rx);
    assert!(
        accepted_events
            .iter()
            .any(|event| matches!(event, OrderEventAny::Accepted(_))),
        "expected limit order acceptance before the queue-reducing trade",
    );
    assert!(
        accepted_events
            .iter()
            .all(|event| !matches!(event, OrderEventAny::Filled(_))),
        "limit order must rest behind displayed bid size",
    );

    let trade = TradeTick::new(
        instrument.id(),
        Price::from("1000.00"),
        Quantity::from("10.000"),
        AggressorSide::Sell,
        TradeId::new("T-QUEUE-1"),
        UnixNanos::from(1),
        UnixNanos::from(1),
    );
    test_context.client.process_trade_tick(&trade).unwrap();

    let trade_events = apply_order_events_from_channel(&cache, &mut rx);
    let filled = trade_events
        .iter()
        .any(|event| matches!(event, OrderEventAny::Filled(_)));

    assert_eq!(
        filled, expect_fill,
        "queue_position={queue_position}: a 10-unit sell into 100 displayed bid units \
         must fill the resting 1-unit buy only when queue tracking is off",
    );
}

#[rstest]
#[case::consumption_on(true, 1)]
#[case::consumption_off(false, 2)]
fn test_liquidity_consumption_shares_trade_size_across_limits(
    #[case] liquidity_consumption: bool,
    #[case] expected_fills: usize,
    trader_id: TraderId,
    account_id: AccountId,
    instrument: InstrumentAny,
) {
    setup_order_event_handler();

    let venue = instrument.id().venue;
    let mut test_context = create_test_context_with(trader_id, account_id, venue, |config| {
        config.trade_execution = true;
        config.liquidity_consumption = liquidity_consumption;
    });
    let cache = test_context.cache.clone();
    cache
        .borrow_mut()
        .add_instrument(instrument.clone())
        .unwrap();

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<ExecutionEvent>();
    set_exec_event_sender(tx);
    test_context.client.start().unwrap();

    let quote = create_quote_tick(instrument.id(), 1000.0, 1001.0);
    test_context.client.process_quote_tick(&quote).unwrap();

    for (idx, client_order_id) in ["CONS-LIMIT-001", "CONS-LIMIT-002"].iter().enumerate() {
        let order = OrderTestBuilder::new(OrderType::Limit)
            .trader_id(trader_id)
            .instrument_id(instrument.id())
            .client_order_id(ClientOrderId::from(*client_order_id))
            .side(OrderSide::Buy)
            .price(Price::from("1000.00"))
            .quantity(Quantity::from("1.000"))
            .submit(true)
            .build();
        cache
            .borrow_mut()
            .add_order(order.clone(), None, Some(ClientId::new("SANDBOX")), false)
            .unwrap();
        test_context
            .client
            .submit_order(SubmitOrder::from_order(
                &order,
                trader_id,
                Some(test_context.client.client_id()),
                None,
                UUID4::new(),
                UnixNanos::from(idx as u64),
            ))
            .unwrap();
    }

    let accepted_events = apply_order_events_from_channel(&cache, &mut rx);
    assert_eq!(
        accepted_events
            .iter()
            .filter(|event| matches!(event, OrderEventAny::Accepted(_)))
            .count(),
        2,
        "expected both limit orders to rest before the shared trade",
    );

    let trade = TradeTick::new(
        instrument.id(),
        Price::from("1000.00"),
        Quantity::from("1.000"),
        AggressorSide::Sell,
        TradeId::new("T-CONS-1"),
        UnixNanos::from(10),
        UnixNanos::from(10),
    );
    test_context.client.process_trade_tick(&trade).unwrap();

    let fill_count = apply_order_events_from_channel(&cache, &mut rx)
        .iter()
        .filter(|event| matches!(event, OrderEventAny::Filled(_)))
        .count();

    assert_eq!(
        fill_count, expected_fills,
        "liquidity_consumption={liquidity_consumption}: a 1-unit sell must fill \
         one resting 1-unit buy when consumption is on, and both when it is off",
    );
}

#[rstest]
fn test_fill_model_can_block_limit_touch_fills(
    trader_id: TraderId,
    account_id: AccountId,
    instrument: InstrumentAny,
) {
    setup_order_event_handler();

    let venue = instrument.id().venue;
    let mut test_context = create_test_context_with(trader_id, account_id, venue, |config| {
        config.trade_execution = true;
        config.fill_model = Some(FillModelAny::Default(
            DefaultFillModel::new(0.0, 0.0, Some(1)).unwrap(),
        ));
    });
    let cache = test_context.cache.clone();
    cache
        .borrow_mut()
        .add_instrument(instrument.clone())
        .unwrap();

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<ExecutionEvent>();
    set_exec_event_sender(tx);
    test_context.client.start().unwrap();

    let quote = create_quote_tick(instrument.id(), 1000.0, 1001.0);
    test_context.client.process_quote_tick(&quote).unwrap();

    let order = OrderTestBuilder::new(OrderType::Limit)
        .trader_id(trader_id)
        .instrument_id(instrument.id())
        .client_order_id(ClientOrderId::from("FILL-MODEL-LIMIT-001"))
        .side(OrderSide::Buy)
        .price(Price::from("1000.00"))
        .quantity(Quantity::from("1.000"))
        .submit(true)
        .build();
    cache
        .borrow_mut()
        .add_order(order.clone(), None, Some(ClientId::new("SANDBOX")), false)
        .unwrap();
    test_context
        .client
        .submit_order(SubmitOrder::from_order(
            &order,
            trader_id,
            Some(test_context.client.client_id()),
            None,
            UUID4::new(),
            UnixNanos::default(),
        ))
        .unwrap();
    let _ = apply_order_events_from_channel(&cache, &mut rx);

    let trade = TradeTick::new(
        instrument.id(),
        Price::from("1000.00"),
        Quantity::from("1.000"),
        AggressorSide::Sell,
        TradeId::new("T-FILL-MODEL-1"),
        UnixNanos::from(1),
        UnixNanos::from(1),
    );
    test_context.client.process_trade_tick(&trade).unwrap();

    let trade_events = apply_order_events_from_channel(&cache, &mut rx);
    assert!(
        trade_events
            .iter()
            .all(|event| !matches!(event, OrderEventAny::Filled(_))),
        "prob_fill_on_limit=0 must not fill a limit order on touch",
    );
}

fn first_two_shared_limit_fills(seed: u64) -> [bool; 2] {
    let mut handle = FillModelHandle::from(FillModelAny::Default(
        DefaultFillModel::new(0.5, 0.0, Some(seed)).unwrap(),
    ));

    [
        handle.is_limit_filled().unwrap(),
        handle.is_limit_filled().unwrap(),
    ]
}

fn first_fills_from_independent_handles(seed: u64) -> [bool; 2] {
    let model = DefaultFillModel::new(0.5, 0.0, Some(seed)).unwrap();
    let mut first = FillModelHandle::from(FillModelAny::Default(model.clone()));
    let mut second = FillModelHandle::from(FillModelAny::Default(model));

    [
        first.is_limit_filled().unwrap(),
        second.is_limit_filled().unwrap(),
    ]
}

fn process_limit_touch_fill(
    test_context: &TestContext,
    trader_id: TraderId,
    instrument: &InstrumentAny,
    client_order_id: &str,
    trade_id: &str,
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<ExecutionEvent>,
) -> bool {
    let cache = test_context.cache.clone();
    cache
        .borrow_mut()
        .add_instrument(instrument.clone())
        .unwrap();

    let quote = create_quote_tick(instrument.id(), 1000.0, 1001.0);
    test_context.client.process_quote_tick(&quote).unwrap();

    let order = OrderTestBuilder::new(OrderType::Limit)
        .trader_id(trader_id)
        .instrument_id(instrument.id())
        .client_order_id(ClientOrderId::from(client_order_id))
        .side(OrderSide::Buy)
        .price(Price::from("1000.00"))
        .quantity(Quantity::from("1.000"))
        .submit(true)
        .build();
    cache
        .borrow_mut()
        .add_order(order.clone(), None, Some(ClientId::new("SANDBOX")), false)
        .unwrap();
    test_context
        .client
        .submit_order(SubmitOrder::from_order(
            &order,
            trader_id,
            Some(test_context.client.client_id()),
            None,
            UUID4::new(),
            UnixNanos::default(),
        ))
        .unwrap();
    let _ = apply_order_events_from_channel(&cache, rx);

    let trade = TradeTick::new(
        instrument.id(),
        Price::from("1000.00"),
        Quantity::from("1.000"),
        AggressorSide::Sell,
        TradeId::new(trade_id),
        UnixNanos::from(1),
        UnixNanos::from(1),
    );
    test_context.client.process_trade_tick(&trade).unwrap();

    apply_order_events_from_channel(&cache, rx)
        .iter()
        .any(|event| matches!(event, OrderEventAny::Filled(_)))
}

#[rstest]
fn test_seeded_fill_model_is_shared_across_instruments(
    trader_id: TraderId,
    account_id: AccountId,
    instrument: InstrumentAny,
) {
    setup_order_event_handler();

    // Seed 42's first two draws match; pick a seed where shared vs cloned From() diverge.
    let seed = (0u64..64)
        .find(|&seed| {
            first_two_shared_limit_fills(seed) != first_fills_from_independent_handles(seed)
        })
        .expect("a seed in 0..64 must discriminate shared vs cloned From() draws");
    let expected_shared = first_two_shared_limit_fills(seed);
    let expected_independent = first_fills_from_independent_handles(seed);

    assert_ne!(expected_shared, expected_independent);

    let venue = instrument.id().venue;
    let other = match instrument.clone() {
        InstrumentAny::CryptoPerpetual(mut perp) => {
            perp.id = InstrumentId::from("BTCUSDT-PERP.BINANCE");
            InstrumentAny::CryptoPerpetual(perp)
        }
        other => panic!("expected crypto perpetual fixture, was {other:?}"),
    };

    let mut test_context = create_test_context_with(trader_id, account_id, venue, |config| {
        config.trade_execution = true;
        config.fill_model = Some(FillModelAny::Default(
            DefaultFillModel::new(0.5, 0.0, Some(seed)).unwrap(),
        ));
    });

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<ExecutionEvent>();
    set_exec_event_sender(tx);
    test_context.client.start().unwrap();

    let first_filled = process_limit_touch_fill(
        &test_context,
        trader_id,
        &instrument,
        "SHARED-FILL-ETH-001",
        "T-SHARED-ETH-1",
        &mut rx,
    );
    let second_filled = process_limit_touch_fill(
        &test_context,
        trader_id,
        &other,
        "SHARED-FILL-BTC-001",
        "T-SHARED-BTC-1",
        &mut rx,
    );
    let observed = [first_filled, second_filled];

    assert_eq!(test_context.client.matching_engine_count(), 2);
    assert_eq!(observed, expected_shared);
    assert_ne!(observed, expected_independent);
}

#[rstest]
fn test_client_initial_state(execution_client: SandboxExecutionClient, venue: Venue) {
    assert!(!execution_client.is_connected());
    assert_eq!(execution_client.venue(), venue);
    assert_eq!(execution_client.oms_type(), OmsType::Netting);
    assert_eq!(execution_client.matching_engine_count(), 0);
}

#[rstest]
fn test_client_start(mut execution_client: SandboxExecutionClient) {
    setup_order_event_handler();

    let result = execution_client.start();

    assert!(result.is_ok());
    assert!(!execution_client.is_connected());
}

#[rstest]
fn test_client_start_idempotent(mut execution_client: SandboxExecutionClient) {
    setup_order_event_handler();

    execution_client.start().unwrap();
    let result = execution_client.start();

    assert!(result.is_ok());
}

#[rstest]
#[tokio::test]
async fn test_client_connect(mut execution_client: SandboxExecutionClient) {
    setup_order_event_handler();

    let result = execution_client.connect().await;

    assert!(result.is_ok());
    assert!(execution_client.is_connected());
}

#[rstest]
#[tokio::test]
async fn test_client_connect_syncs_cached_margin_account_config(
    trader_id: TraderId,
    account_id: AccountId,
    venue: Venue,
    instrument: InstrumentAny,
) {
    *msgbus::get_message_bus().borrow_mut() = MessageBus::default();

    let leverage = Decimal::from(5);
    let instrument_id = instrument.id();
    let context = create_test_context_with(trader_id, account_id, venue, |config| {
        config.default_leverage = leverage;
        config.leverages.insert(instrument_id, leverage);
    });
    setup_account_state_handler(context.cache.clone());

    let mut execution_client = context.client;
    execution_client.connect().await.unwrap();

    let cache = context.cache.borrow();
    let account = cache
        .account(&account_id)
        .expect("expected cached account after initial AccountState");

    let AccountAny::Margin(margin) = &*account else {
        panic!("expected margin account");
    };

    assert!(margin.base.calculate_account_state);
    assert_eq!(margin.default_leverage, leverage);
    assert_eq!(margin.get_leverage(&instrument_id), leverage);
}

#[rstest]
#[tokio::test]
async fn test_client_connect_respects_frozen_account_config(
    trader_id: TraderId,
    account_id: AccountId,
    venue: Venue,
) {
    *msgbus::get_message_bus().borrow_mut() = MessageBus::default();

    let context = create_test_context_with(trader_id, account_id, venue, |config| {
        config.frozen_account = true;
    });
    setup_account_state_handler(context.cache.clone());

    let mut execution_client = context.client;
    execution_client.connect().await.unwrap();

    let cache = context.cache.borrow();
    let account = cache
        .account(&account_id)
        .expect("expected cached account after initial AccountState");

    let AccountAny::Margin(margin) = &*account else {
        panic!("expected margin account");
    };

    assert!(!margin.base.calculate_account_state);
}

#[rstest]
#[tokio::test]
async fn test_client_connect_idempotent(mut execution_client: SandboxExecutionClient) {
    setup_order_event_handler();

    execution_client.connect().await.unwrap();
    let result = execution_client.connect().await;

    assert!(result.is_ok());
    assert!(execution_client.is_connected());
}

#[rstest]
#[tokio::test]
async fn test_client_disconnect(mut execution_client: SandboxExecutionClient) {
    setup_order_event_handler();

    execution_client.connect().await.unwrap();
    let result = execution_client.disconnect().await;

    assert!(result.is_ok());
    assert!(!execution_client.is_connected());
}

#[rstest]
#[tokio::test]
async fn test_client_disconnect_when_not_connected(mut execution_client: SandboxExecutionClient) {
    setup_order_event_handler();

    let result = execution_client.disconnect().await;

    assert!(result.is_ok());
    assert!(!execution_client.is_connected());
}

#[rstest]
#[tokio::test]
async fn test_client_stop(mut execution_client: SandboxExecutionClient) {
    setup_order_event_handler();

    execution_client.start().unwrap();
    execution_client.connect().await.unwrap();
    let result = execution_client.stop();

    assert!(result.is_ok());
    assert!(!execution_client.is_connected());
}

#[rstest]
fn test_client_stop_when_not_started(mut execution_client: SandboxExecutionClient) {
    setup_order_event_handler();

    let result = execution_client.stop();

    assert!(result.is_ok());
}

#[cfg(feature = "python")]
#[rstest]
fn test_expiration_initialized_publication_waits_for_writer_exit(
    trader_id: TraderId,
    account_id: AccountId,
) {
    use pyo3::types::{PyModule, PyModuleMethods};

    setup_order_event_handler();
    let mut harness = setup_pending_resolution_harness(trader_id, account_id, "PUBLICATION-WRITER");
    Python::initialize();
    let module = Python::attach(|py| {
        let module = PyModule::new(py, "initialized_publication_writer").unwrap();
        py.run(
            c_str!(
                r#"
import sqlite3
import tempfile
from pathlib import Path

directory = tempfile.TemporaryDirectory()
database = Path(directory.name) / "scope.sqlite3"
with sqlite3.connect(database) as connection:
    connection.execute("CREATE TABLE observations (value INTEGER NOT NULL)")
writer = sqlite3.connect(database, timeout=0)
writer.execute("BEGIN IMMEDIATE")
observed = []

def observe():
    observed.append(writer is not None)
    connection = sqlite3.connect(database, timeout=0)
    try:
        connection.execute("BEGIN IMMEDIATE")
        connection.execute("INSERT INTO observations VALUES (1)")
        connection.commit()
    finally:
        connection.close()

def close_writer():
    global writer
    writer.rollback()
    writer.close()
    writer = None

def cleanup():
    directory.cleanup()
"#
            ),
            Some(&module.dict()),
            None,
        )
        .unwrap();
        module.unbind()
    });
    let callback = Python::attach(|py| module.bind(py).getattr("observe").unwrap().unbind());
    let errors = Rc::new(RefCell::new(Vec::new()));
    let callback_errors = errors.clone();
    let handler = TypedHandler::from(move |event: &OrderEventAny| {
        if matches!(event, OrderEventAny::Initialized(_))
            && event.client_order_id().as_str().starts_with("EXPIRATION-")
        {
            Python::attach(|py| {
                if let Err(error) = callback.call0(py) {
                    callback_errors.borrow_mut().push(error.to_string());
                }
            });
        }
    });
    msgbus::subscribe_order_events("events.order.*".into(), handler.clone(), None);
    let close = InstrumentClose::new(
        harness.instrument.id(),
        Price::from("1.000"),
        InstrumentCloseType::ContractExpired,
        UnixNanos::from(300),
        UnixNanos::from(300),
    );
    msgbus::publish_any(
        nautilus_common::msgbus::switchboard::get_instrument_close_topic(harness.instrument.id()),
        &close,
    );
    let before_exit = Python::attach(|py| {
        let module = module.bind(py);
        let observed = module
            .getattr("observed")
            .unwrap()
            .extract::<Vec<bool>>()
            .unwrap();
        module.call_method0("close_writer").unwrap();
        observed
    });
    if !before_exit.is_empty() {
        Python::attach(|py| {
            module.bind(py).call_method0("cleanup").unwrap();
        });
        msgbus::unsubscribe_order_events("events.order.*".into(), &handler);
        assert!(
            before_exit.is_empty(),
            "Initialized reached Python before the real writer exited: {before_exit:?}; errors={:?}",
            errors.borrow(),
        );
    }
    while let Ok(event) = harness.rx.try_recv() {
        nautilus_live::runner::AsyncRunner::handle_exec_event(event);
    }
    let after_exit = Python::attach(|py| {
        let module = module.bind(py);
        let observed = module
            .getattr("observed")
            .unwrap()
            .extract::<Vec<bool>>()
            .unwrap();
        module.call_method0("cleanup").unwrap();
        observed
    });
    msgbus::unsubscribe_order_events("events.order.*".into(), &handler);
    assert_eq!(after_exit, vec![false]);
    assert!(errors.borrow().is_empty(), "{:?}", errors.borrow());
}

#[rstest]
#[tokio::test]
async fn test_initialized_publication_channel_closed_is_not_a_success(
    trader_id: TraderId,
    account_id: AccountId,
) {
    use nautilus_common::clients::ExecutionAccountCaptureError;
    use nautilus_execution::matching_engine::{
        InitializedPublicationFault, InitializedPublicationSource,
    };

    setup_order_event_handler();
    let mut harness = setup_pending_resolution_harness(trader_id, account_id, "PUBLICATION-CLOSED");
    setup_account_state_handler(harness.context.cache.clone());
    harness.context.client.connect().await.unwrap();
    let probe = harness.context.client.account_capture_probe().unwrap();
    harness.rx.close();
    assert!(harness.rx.is_closed());
    let initialized = Rc::new(RefCell::new(Vec::new()));
    let observed = initialized.clone();
    let handler = TypedHandler::from(move |event: &OrderEventAny| {
        if let OrderEventAny::Initialized(event) = event
            && event.client_order_id.as_str().starts_with("EXPIRATION-")
        {
            observed.borrow_mut().push(event.clone());
        }
    });
    msgbus::subscribe_order_events("events.order.*".into(), handler.clone(), None);
    let close = InstrumentClose::new(
        harness.instrument.id(),
        Price::from("1.000"),
        InstrumentCloseType::ContractExpired,
        UnixNanos::from(300),
        UnixNanos::from(300),
    );
    msgbus::publish_any(
        nautilus_common::msgbus::switchboard::get_instrument_close_topic(harness.instrument.id()),
        &close,
    );
    let quote = QuoteTick::new(
        harness.instrument.id(),
        Price::new(0.40, 3),
        Price::new(0.41, 3),
        Quantity::new(100.0, 2),
        Quantity::new(100.0, 2),
        UnixNanos::from(301),
        UnixNanos::from(301),
    );
    let result = harness.context.client.process_quote_tick(&quote);
    msgbus::unsubscribe_order_events("events.order.*".into(), &handler);
    assert!(
        result.is_err(),
        "closed execution receiver must leave an observable owner failure, got {result:?}; synchronous Initialized publications={}",
        initialized.borrow().len(),
    );
    assert!(initialized.borrow().is_empty());
    let error = result.unwrap_err();
    let returned = error.downcast_ref::<InitializedPublicationFault>().unwrap();
    let retained = harness
        .context
        .client
        .initialized_publication_failure()
        .unwrap();
    assert_eq!(returned.failure.event(), retained.failure.event());
    let event = retained.failure.event().clone();
    assert!(matches!(
        retained.source,
        InitializedPublicationSource::Expiration { close: Some(saved), .. } if saved == close
    ));
    let cache = harness.context.cache.clone();
    let automatic = cache.borrow().order_owned(&event.client_order_id).unwrap();
    assert_eq!(automatic.init_event(), &event);
    assert_eq!(automatic.status(), OrderStatus::Initialized);
    assert_eq!(automatic.events().len(), 1);
    assert_eq!(
        probe.capture().unwrap_err(),
        ExecutionAccountCaptureError::PendingNativeState,
    );
    let before_orders = cache
        .borrow()
        .orders_total_count(None, None, None, None, None);
    let positions_before: Vec<_> = cache
        .borrow()
        .positions_open(None, None, None, None, None)
        .iter()
        .map(|position| (position.id, position.quantity))
        .collect();
    msgbus::publish_any(
        nautilus_common::msgbus::switchboard::get_instrument_close_topic(harness.instrument.id()),
        &close,
    );
    let after_fault = OrderTestBuilder::new(OrderType::Market)
        .instrument_id(harness.instrument.id())
        .client_order_id("AFTER-TERMINAL-PUBLICATION".into())
        .side(OrderSide::Buy)
        .quantity(Quantity::from("1.00"))
        .build();
    let result = harness.context.client.submit_order(SubmitOrder::from_order(
        &after_fault,
        trader_id,
        Some(harness.context.client.client_id()),
        None,
        UUID4::new(),
        UnixNanos::from(301),
    ));
    assert!(result.unwrap_err().is::<InitializedPublicationFault>());
    harness.context.client.reset();
    assert_eq!(
        harness
            .context
            .client
            .initialized_publication_failure()
            .unwrap()
            .failure
            .event(),
        &event,
    );
    assert!(
        harness
            .context
            .client
            .process_quote_tick(&quote)
            .unwrap_err()
            .is::<InitializedPublicationFault>()
    );
    assert_eq!(
        cache
            .borrow()
            .orders_total_count(None, None, None, None, None),
        before_orders
    );
    let positions_after: Vec<_> = cache
        .borrow()
        .positions_open(None, None, None, None, None)
        .iter()
        .map(|position| (position.id, position.quantity))
        .collect();
    assert_eq!(positions_after, positions_before);
    // An independently observed closing fill may later reach this owner. Cleanup must not
    // discard the failed initialization merely because the original position is now closed.
    let mut externally_closed = cache
        .borrow()
        .position_owned(&positions_before[0].0)
        .unwrap();
    let external = OrderTestBuilder::new(OrderType::Market)
        .trader_id(externally_closed.trader_id)
        .strategy_id(externally_closed.strategy_id)
        .instrument_id(externally_closed.instrument_id)
        .client_order_id("OBSERVED-EXTERNAL-CLOSE".into())
        .side(OrderSide::Sell)
        .quantity(externally_closed.quantity)
        .submit(true)
        .build();
    let OrderEventAny::Filled(observed_close) = TestOrderEventStubs::filled(
        &external,
        &harness.instrument,
        Some(TradeId::from("OBSERVED-EXTERNAL-CLOSE")),
        Some(externally_closed.id),
        Some(Price::from("1.000")),
        None,
        None,
        Some(Money::from("0 USDC")),
        Some(UnixNanos::from(302)),
        Some(account_id),
    ) else {
        unreachable!()
    };
    externally_closed.apply(&observed_close);
    assert!(externally_closed.is_closed());
    cache
        .borrow_mut()
        .update_position(&externally_closed)
        .unwrap();
    msgbus::publish_position_event(
        "events.position.TEST".into(),
        &position_closed_event(&externally_closed, account_id),
    );
    assert_eq!(harness.context.client.matching_engine_count(), 1);
    assert_eq!(
        harness
            .context
            .client
            .initialized_publication_failure()
            .unwrap()
            .failure
            .event(),
        &event
    );
    assert!(
        harness
            .context
            .client
            .process_quote_tick(&quote)
            .unwrap_err()
            .is::<InitializedPublicationFault>()
    );
    harness.context.client.stop().unwrap();
    assert_eq!(
        harness
            .context
            .client
            .initialized_publication_failure()
            .unwrap()
            .failure
            .event(),
        &event
    );
}

#[cfg(feature = "python")]
#[rstest]
#[case::existing_engine(true)]
#[case::later_engine(false)]
#[tokio::test]
async fn test_real_python_strategy_observes_publication_after_writer_and_queues_child(
    #[case] existing_engine: bool,
) {
    use std::sync::Arc;

    use nautilus_common::{
        enums::Environment,
        live::runner::replace_exec_event_sender,
        logging::logger::LoggerConfig,
        runner::{
            SyncTradingCommandSender, TradingCommandMessage, drain_trading_cmd_queue,
            replace_exec_cmd_sender, trading_cmd_queue_is_empty,
        },
    };
    use nautilus_live::{node::LiveNode, runner::AsyncRunner};
    use nautilus_model::python::orders::order_any_to_pyobject;
    use nautilus_trading::python::strategy::PyStrategy;
    use pyo3::types::{PyModule, PyModuleMethods};

    Python::initialize();
    let trader_id = TraderId::from("PUBLICATION-001");
    let account_id = AccountId::from("SIM-001");
    let client_id = ClientId::from("PUBLICATION");
    let test_clock = Rc::new(RefCell::new(TestClock::new()));
    test_clock.borrow_mut().set_time(UnixNanos::from(50));
    let clock: Rc<RefCell<dyn Clock>> = test_clock.clone();
    let factory_clock = clock.clone();
    let mut node = LiveNode::builder(trader_id, Environment::Sandbox)
        .unwrap()
        .with_clock_factory(move || factory_clock.clone())
        .with_logging(LoggerConfig {
            bypass_logging: true,
            ..Default::default()
        })
        .with_reconciliation(false)
        .build()
        .unwrap();
    let cache = node.kernel().cache();
    let mut parent = binary_option();
    parent.id = InstrumentId::from("PARENT.SIM");
    parent.activation_ns = UnixNanos::from(1);
    parent.expiration_ns = UnixNanos::from(100);
    parent.maker_fee = Decimal::ZERO;
    parent.taker_fee = Decimal::ZERO;
    let mut child_instrument = parent.clone();
    child_instrument.id = InstrumentId::from("CHILD.SIM");
    child_instrument.expiration_ns = UnixNanos::from(10_000);
    let parent = InstrumentAny::BinaryOption(parent);
    let child_instrument = InstrumentAny::BinaryOption(child_instrument);
    let quote = QuoteTick::new(
        parent.id(),
        Price::from("0.400"),
        Price::from("0.410"),
        Quantity::from("100.00"),
        Quantity::from("100.00"),
        UnixNanos::from(50),
        UnixNanos::from(50),
    );
    let child_quote = QuoteTick::new(
        child_instrument.id(),
        quote.bid_price,
        quote.ask_price,
        quote.bid_size,
        quote.ask_size,
        quote.ts_event,
        quote.ts_init,
    );
    {
        let mut cache = cache.borrow_mut();
        cache.add_instrument(parent.clone()).unwrap();
        cache.add_instrument(child_instrument.clone()).unwrap();
        cache.add_quote(quote).unwrap();
        cache.add_quote(child_quote).unwrap();
    }
    let (sender, mut events) = tokio::sync::mpsc::unbounded_channel();
    replace_exec_event_sender(sender);
    replace_exec_cmd_sender(Arc::new(SyncTradingCommandSender));
    let config = SandboxExecutionClientConfig {
        account_id,
        venue: Venue::from("SIM"),
        base_currency: Some(Currency::USDC()),
        starting_balances: vec![Money::from("100000 USDC")],
        ..Default::default()
    };
    let core = ExecutionClientCore::new(
        trader_id,
        client_id,
        config.venue,
        config.oms_type,
        account_id,
        config.account_type,
        config.base_currency,
        cache.clone(),
    );
    let mut client = SandboxExecutionClient::new(core, config, clock.clone(), cache.clone());
    if existing_engine {
        client.process_quote_tick(&quote).unwrap();
    }
    client.start().unwrap();
    client.connect().await.unwrap();
    if !existing_engine {
        client.process_quote_tick(&quote).unwrap();
    }
    node.kernel()
        .exec_engine()
        .borrow_mut()
        .register_client(Box::new(client))
        .unwrap();
    let module = Python::attach(|py| {
        let module = PyModule::new(py, "actual_strategy_publication").unwrap();
        module
            .setattr("Strategy", py.get_type::<PyStrategy>())
            .unwrap();
        py.run(
            c_str!(
                r#"
import sqlite3
import tempfile
from pathlib import Path
directory = tempfile.TemporaryDirectory()
database = Path(directory.name) / "control.sqlite3"
with sqlite3.connect(database) as connection:
    connection.execute("CREATE TABLE observations (value INTEGER NOT NULL)")
writer = None
observed = []
errors = []
seen = []
class PublicationStrategy(Strategy):
    def on_start(self): pass
    def on_stop(self): pass
    def on_dispose(self): pass
    def on_order_initialized(self, event):
        if not str(event.client_order_id).startswith("EXPIRATION-"): return
        try:
            cached = self.cache.order(event.client_order_id)
            assert cached is not None
            observed.append((str(cached.status), writer is not None))
            connection = sqlite3.connect(database, timeout=0)
            try:
                connection.execute("BEGIN IMMEDIATE")
                connection.execute("INSERT INTO observations VALUES (1)")
                connection.commit()
            finally:
                connection.close()
            self.submit_order(self.child, client_id=client_id)
        except BaseException as error:
            errors.append(repr(error))
    def on_order_event(self, event):
        seen.append((str(event.client_order_id), type(event).__name__))
strategy = PublicationStrategy()
def open_writer():
    global writer
    writer = sqlite3.connect(database, timeout=0)
    writer.execute("BEGIN IMMEDIATE")
def close_writer():
    global writer
    writer.rollback()
    writer.close()
    writer = None
def cleanup(): directory.cleanup()
"#
            ),
            Some(&module.dict()),
            None,
        )
        .unwrap();
        module.unbind()
    });
    let strategy = Python::attach(|py| module.bind(py).getattr("strategy").unwrap().unbind());
    let strategy_id = node
        .kernel()
        .trader()
        .borrow_mut()
        .add_python_strategy_instance(&strategy)
        .unwrap();
    let child = OrderTestBuilder::new(OrderType::Limit)
        .trader_id(trader_id)
        .strategy_id(strategy_id)
        .instrument_id(child_instrument.id())
        .client_order_id("PUBLICATION-CHILD".into())
        .side(OrderSide::Buy)
        .price(Price::from("0.050"))
        .quantity(Quantity::from("1.00"))
        .build();
    Python::attach(|py| {
        strategy
            .bind(py)
            .setattr("child", order_any_to_pyobject(py, child.clone()).unwrap())
            .unwrap();
        module
            .bind(py)
            .setattr("client_id", client_id.into_py_any(py).unwrap())
            .unwrap();
        strategy.call_method0(py, "start").unwrap();
    });
    let opening = OrderTestBuilder::new(OrderType::Market)
        .trader_id(trader_id)
        .strategy_id(strategy_id)
        .instrument_id(parent.id())
        .client_order_id("PUBLICATION-OPEN".into())
        .side(OrderSide::Buy)
        .quantity(Quantity::from("1.00"))
        .build();
    cache
        .borrow_mut()
        .add_order(opening.clone(), None, Some(client_id), false)
        .unwrap();
    let command = SubmitOrder::from_order(
        &opening,
        trader_id,
        Some(client_id),
        None,
        UUID4::new(),
        UnixNanos::from(50),
    );
    assert!(
        TradingCommandMessage::new(
            MessagingSwitchboard::exec_engine_execute(),
            TradingCommand::SubmitOrder(command)
        )
        .dispatch()
        .is_empty()
    );
    assert_eq!(
        cache
            .borrow()
            .order_owned(&opening.client_order_id())
            .unwrap()
            .status(),
        OrderStatus::Initialized
    );
    while let Ok(event) = events.try_recv() {
        AsyncRunner::handle_exec_event(event);
    }
    assert_eq!(
        cache
            .borrow()
            .order_owned(&opening.client_order_id())
            .unwrap()
            .status(),
        OrderStatus::Filled
    );
    let parent_position_id = {
        let cache = cache.borrow();
        let positions = cache.positions_open(None, Some(&parent.id()), None, None, None);
        assert_eq!(positions.len(), 1);
        positions[0].id
    };
    Python::attach(|py| {
        module.bind(py).call_method0("open_writer").unwrap();
    });
    test_clock.borrow_mut().set_time(UnixNanos::from(200));
    let close = InstrumentClose::new(
        parent.id(),
        Price::from("1.000"),
        InstrumentCloseType::ContractExpired,
        UnixNanos::from(200),
        UnixNanos::from(200),
    );
    msgbus::publish_any(
        nautilus_common::msgbus::switchboard::get_instrument_close_topic(parent.id()),
        &close,
    );
    let before = Python::attach(|py| {
        let module = module.bind(py);
        let observed = module
            .getattr("observed")
            .unwrap()
            .extract::<Vec<(String, bool)>>()
            .unwrap();
        module.call_method0("close_writer").unwrap();
        observed
    });
    assert!(before.is_empty());
    while let Ok(event) = events.try_recv() {
        AsyncRunner::handle_exec_event(event);
    }
    assert!(!trading_cmd_queue_is_empty());
    let (observed, errors, seen) = Python::attach(|py| {
        let module = module.bind(py);
        (
            module
                .getattr("observed")
                .unwrap()
                .extract::<Vec<(String, bool)>>()
                .unwrap(),
            module
                .getattr("errors")
                .unwrap()
                .extract::<Vec<String>>()
                .unwrap(),
            module
                .getattr("seen")
                .unwrap()
                .extract::<Vec<(String, String)>>()
                .unwrap(),
        )
    });
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(observed.len(), 1);
    assert!(observed[0].0.ends_with("INITIALIZED"), "{observed:?}");
    assert!(!observed[0].1);
    let automatic_events: Vec<_> = seen
        .iter()
        .filter(|(id, _)| id.starts_with("EXPIRATION-"))
        .map(|(_, kind)| kind.as_str())
        .collect();
    assert_eq!(
        automatic_events,
        ["OrderInitialized", "OrderAccepted", "OrderFilled"]
    );
    assert!(
        !seen
            .iter()
            .any(|(id, kind)| id == "PUBLICATION-CHILD" && kind == "OrderSubmitted")
    );
    drain_trading_cmd_queue();
    while let Ok(event) = events.try_recv() {
        AsyncRunner::handle_exec_event(event);
    }
    assert!(trading_cmd_queue_is_empty());
    assert_eq!(
        cache
            .borrow()
            .order_owned(&child.client_order_id())
            .unwrap()
            .status(),
        OrderStatus::Accepted
    );
    let closed_position = cache.borrow().position_owned(&parent_position_id).unwrap();
    assert!(closed_position.is_closed());
    assert_eq!(closed_position.quantity, Quantity::zero(2));
    let automatic_ids: std::collections::HashSet<_> = seen
        .iter()
        .filter(|(id, _)| id.starts_with("EXPIRATION-"))
        .map(|(id, _)| id.as_str())
        .collect();
    assert_eq!(automatic_ids.len(), 1);
    // Expiry cleanup removes instrument indexes; inspect retained entities by their actual IDs.
    let automatic_id = ClientOrderId::from(*automatic_ids.iter().next().unwrap());
    let automatic = cache.borrow().order_owned(&automatic_id).unwrap();
    assert_eq!(automatic.status(), OrderStatus::Filled);
    assert_eq!(automatic.events().len(), 3);
    assert_eq!(
        cache
            .borrow()
            .account_owned(&account_id)
            .unwrap()
            .balances()
            .get(&Currency::USDC())
            .unwrap()
            .total,
        Money::from("100000.59 USDC")
    );
    Python::attach(|py| {
        strategy.call_method0(py, "stop").unwrap();
        module.bind(py).call_method0("cleanup").unwrap();
    });
    node.dispose();
}

#[cfg(feature = "python")]
mod hosted_account_connect {
    use std::{any::Any, cell::Cell, sync::Mutex, time::Duration};

    use nautilus_common::{
        clients::ExecutionAccountCaptureProbe,
        enums::Environment,
        logging::logger::LoggerConfig,
        messages::execution::{
            BatchCancelOrders, CancelOrder, GenerateFillReports, GenerateOrderStatusReport,
            GenerateOrderStatusReports, GeneratePositionStatusReports, QueryAccount, QueryOrder,
        },
        msgbus::{BusTap, Endpoint, MStr, Topic},
    };
    use nautilus_live::{
        config::{LiveExecutionEngineConfig, LiveNodeConfig},
        node::{NodeState, builder::LiveNodeBuilder},
        python::node::PyLiveNode,
    };
    use nautilus_model::{
        accounts::Account,
        enums::LiquiditySide,
        events::PortfolioSnapshot,
        identifiers::VenueOrderId,
        reports::{ExecutionMassStatus, FillReport, OrderStatusReport, PositionStatusReport},
        types::{AccountBalance, MarginBalance},
    };
    use pyo3::types::{PyCFunction, PyDict, PyModule, PyModuleMethods, PyTuple};
    use tokio::sync::oneshot;

    use super::*;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum ConnectPhase {
        Before,
        Waiting,
        Returned,
        Dropped,
    }

    struct ConnectWaitGuard(Rc<Cell<ConnectPhase>>);

    impl Drop for ConnectWaitGuard {
        fn drop(&mut self) {
            if self.0.get() == ConnectPhase::Waiting {
                self.0.set(ConnectPhase::Dropped);
            }
        }
    }

    // Only the return from connect is delayed; account state and queries use the real client
    struct HeldSandboxConnect {
        inner: SandboxExecutionClient,
        finish: Option<oneshot::Receiver<()>>,
        phase: Rc<Cell<ConnectPhase>>,
        waiting_callback: pyo3::Py<pyo3::PyAny>,
        queries: Rc<RefCell<Vec<(QueryAccount, ConnectPhase)>>>,
        stopped: Rc<Cell<usize>>,
        completed_query_callback: Option<pyo3::Py<pyo3::PyAny>>,
    }

    pub(super) fn ready_hosted_sandbox_client(
        inner: SandboxExecutionClient,
        waiting_callback: pyo3::Py<pyo3::PyAny>,
        stopped: Rc<Cell<usize>>,
    ) -> Box<dyn ExecutionClient> {
        let (finish_tx, finish_rx) = oneshot::channel();
        finish_tx.send(()).unwrap();
        Box::new(HeldSandboxConnect {
            inner,
            finish: Some(finish_rx),
            phase: Rc::new(Cell::new(ConnectPhase::Before)),
            waiting_callback,
            queries: Rc::new(RefCell::new(Vec::new())),
            stopped,
            completed_query_callback: None,
        })
    }

    #[async_trait::async_trait(?Send)]
    impl ExecutionClient for HeldSandboxConnect {
        fn is_connected(&self) -> bool {
            self.inner.is_connected()
        }
        fn client_id(&self) -> ClientId {
            self.inner.client_id()
        }
        fn account_id(&self) -> AccountId {
            self.inner.account_id()
        }
        fn venue(&self) -> Venue {
            self.inner.venue()
        }
        fn oms_type(&self) -> OmsType {
            self.inner.oms_type()
        }
        fn get_account(&self) -> Option<AccountAny> {
            self.inner.get_account()
        }
        fn account_capture_probe(&self) -> Option<Rc<dyn ExecutionAccountCaptureProbe>> {
            self.inner.account_capture_probe()
        }
        fn handles_order_venue(&self, venue: Venue) -> bool {
            self.inner.handles_order_venue(venue)
        }
        fn provides_bulk_position_coverage(&self, instrument_id: InstrumentId) -> bool {
            self.inner.provides_bulk_position_coverage(instrument_id)
        }
        fn position_reconciliation_tolerance(&self) -> Decimal {
            self.inner.position_reconciliation_tolerance()
        }
        fn generate_account_state(
            &self,
            balances: Vec<AccountBalance>,
            margins: Vec<MarginBalance>,
            reported: bool,
            ts_event: UnixNanos,
            info: Option<nautilus_core::Params>,
        ) -> anyhow::Result<()> {
            self.inner
                .generate_account_state(balances, margins, reported, ts_event, info)
        }
        fn start(&mut self) -> anyhow::Result<()> {
            self.inner.start()
        }
        fn stop(&mut self) -> anyhow::Result<()> {
            self.inner.stop()?;
            self.stopped.set(self.stopped.get() + 1);
            Ok(())
        }
        fn reset(&mut self) -> anyhow::Result<()> {
            ExecutionClient::reset(&mut self.inner)
        }
        fn dispose(&mut self) -> anyhow::Result<()> {
            self.inner.dispose()
        }
        async fn connect(&mut self) -> anyhow::Result<()> {
            self.inner.connect().await?;
            self.phase.set(ConnectPhase::Waiting);
            let _guard = ConnectWaitGuard(self.phase.clone());
            Python::attach(|py| self.waiting_callback.call0(py))
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            self.finish.take().unwrap().await?;
            self.phase.set(ConnectPhase::Returned);
            Ok(())
        }
        async fn disconnect(&mut self) -> anyhow::Result<()> {
            self.inner.disconnect().await
        }
        fn submit_order(&self, cmd: SubmitOrder) -> anyhow::Result<()> {
            self.inner.submit_order(cmd)
        }
        fn submit_order_list(&self, cmd: SubmitOrderList) -> anyhow::Result<()> {
            self.inner.submit_order_list(cmd)
        }
        fn modify_order(&self, cmd: ModifyOrder) -> anyhow::Result<()> {
            self.inner.modify_order(cmd)
        }
        fn batch_modify_orders(&self, cmd: BatchModifyOrders) -> anyhow::Result<()> {
            self.inner.batch_modify_orders(cmd)
        }
        fn cancel_order(&self, cmd: CancelOrder) -> anyhow::Result<()> {
            self.inner.cancel_order(cmd)
        }
        fn batch_cancel_orders(&self, cmd: BatchCancelOrders) -> anyhow::Result<()> {
            self.inner.batch_cancel_orders(cmd)
        }
        fn cancel_all_orders(&self, cmd: CancelAllOrders) -> anyhow::Result<()> {
            self.inner.cancel_all_orders(cmd)
        }
        fn query_account(&self, cmd: QueryAccount) -> anyhow::Result<()> {
            self.queries
                .borrow_mut()
                .push((cmd.clone(), self.phase.get()));
            let before = self
                .completed_query_callback
                .as_ref()
                .map(|_| self.inner.get_account().unwrap());
            self.inner.query_account(cmd.clone())?;
            if let Some(callback) = &self.completed_query_callback {
                let before = before.unwrap();
                let after = self.inner.get_account().unwrap();
                let previous = before.last_event().unwrap();
                let observed = after.last_event().unwrap();
                Python::attach(|py| {
                    callback.call1(
                        py,
                        (
                            cmd.command_id.to_string(),
                            previous.event_id.to_string(),
                            observed.event_id.to_string(),
                            before.events().len(),
                            after.events().len(),
                            observed.ts_event.as_u64(),
                            after.balance_total(None).unwrap().to_string(),
                        ),
                    )
                })
                .map_err(|error| anyhow::anyhow!("{error}"))?;
            }
            Ok(())
        }
        fn query_order(&self, cmd: QueryOrder) -> anyhow::Result<()> {
            self.inner.query_order(cmd)
        }
        async fn generate_order_status_report(
            &self,
            cmd: &GenerateOrderStatusReport,
        ) -> anyhow::Result<Option<OrderStatusReport>> {
            self.inner.generate_order_status_report(cmd).await
        }
        async fn generate_order_status_reports(
            &self,
            cmd: &GenerateOrderStatusReports,
        ) -> anyhow::Result<Vec<OrderStatusReport>> {
            self.inner.generate_order_status_reports(cmd).await
        }
        async fn generate_fill_reports(
            &self,
            cmd: GenerateFillReports,
        ) -> anyhow::Result<Vec<FillReport>> {
            self.inner.generate_fill_reports(cmd).await
        }
        async fn generate_position_status_reports(
            &self,
            cmd: &GeneratePositionStatusReports,
        ) -> anyhow::Result<Vec<PositionStatusReport>> {
            self.inner.generate_position_status_reports(cmd).await
        }
        async fn generate_mass_status(
            &self,
            lookback: Option<u64>,
        ) -> anyhow::Result<Option<ExecutionMassStatus>> {
            self.inner.generate_mass_status(lookback).await
        }
        fn register_external_order(
            &self,
            client_order_id: ClientOrderId,
            venue_order_id: VenueOrderId,
            instrument_id: InstrumentId,
            strategy_id: StrategyId,
            ts_init: UnixNanos,
        ) {
            self.inner.register_external_order(
                client_order_id,
                venue_order_id,
                instrument_id,
                strategy_id,
                ts_init,
            );
        }
        fn on_instrument(&mut self, instrument: InstrumentAny) {
            self.inner.on_instrument(instrument);
        }
        fn calculate_commission(
            &self,
            instrument: &InstrumentAny,
            qty: Quantity,
            price: Price,
            side: LiquiditySide,
        ) -> anyhow::Result<Option<Money>> {
            self.inner
                .calculate_commission(instrument, qty, price, side)
        }
    }

    struct HostedQueryTap {
        engine: Rc<RefCell<ExecutionEngine>>,
        queued: Rc<RefCell<Vec<QueryAccount>>>,
        dispatched: Rc<RefCell<Vec<QueryAccount>>>,
        callback: pyo3::Py<pyo3::PyAny>,
    }

    impl BusTap for HostedQueryTap {
        fn on_publish(&self, _topic: MStr<Topic>, _message: &dyn Any) {}
        fn on_send(&self, endpoint: MStr<Endpoint>, message: &dyn Any) {
            let Some(TradingCommand::QueryAccount(query)) =
                message.downcast_ref::<TradingCommand>()
            else {
                return;
            };
            if endpoint == MessagingSwitchboard::exec_engine_queue_execute() {
                self.queued.borrow_mut().push(query.clone());
            } else if endpoint == MessagingSwitchboard::exec_engine_execute() {
                assert!(self.engine.try_borrow_mut().is_ok());
                self.dispatched.borrow_mut().push(query.clone());
                Python::attach(|py| {
                    self.callback
                        .call1(py, (query.command_id.to_string(),))
                        .unwrap();
                });
            }
        }
    }

    #[rstest]
    #[case::normal("normal")]
    #[case::timeout("timeout")]
    #[case::cancel("cancel")]
    fn test_run_async_buffers_callback_queries_until_connect_finishes(#[case] outcome: &str) {
        Python::initialize();
        let trader_id = TraderId::from("HOSTED-001");
        let account_id = AccountId::from("SIM-001");
        let client_id = ClientId::from("HOSTED");
        let timeout = if outcome == "timeout" {
            Duration::from_secs(1)
        } else {
            Duration::from_secs(3)
        };
        let node_config = LiveNodeConfig {
            trader_id,
            environment: Environment::Sandbox,
            timeout_connection: timeout,
            timeout_disconnection: Duration::from_millis(500),
            delay_post_stop: Duration::ZERO,
            timeout_shutdown: Duration::from_millis(500),
            exec_engine: LiveExecutionEngineConfig {
                reconciliation: false,
                ..Default::default()
            },
            ..Default::default()
        };
        let node = LiveNodeBuilder::from_config(node_config)
            .unwrap()
            .with_logging(LoggerConfig {
                bypass_logging: true,
                ..Default::default()
            })
            .build()
            .unwrap();
        let cache = node.kernel().cache();
        let engine = node.kernel().exec_engine().clone();
        let handle = node.handle();
        let (finish_tx, finish_rx) = oneshot::channel();
        let finish_tx = Mutex::new(Some(finish_tx));
        let query_ids = [UUID4::new(), UUID4::new(), UUID4::new()];
        let query_template: Vec<_> = query_ids
            .iter()
            .enumerate()
            .map(|(index, id)| {
                QueryAccount::new(
                    trader_id,
                    Some(client_id),
                    account_id,
                    *id,
                    UnixNanos::from(100 + index as u64),
                    None,
                    None,
                )
            })
            .collect();
        let queue_template = query_template.clone();
        let observer = Python::attach(|py| {
            let module = PyModule::new(py, "hosted_account_observer").unwrap();
            let enqueue = PyCFunction::new_closure(
                py,
                None,
                None,
                move |args: &pyo3::Bound<'_, PyTuple>, _kwargs| -> pyo3::PyResult<()> {
                    let cause: UUID4 = args
                        .get_item(0)?
                        .extract::<String>()?
                        .parse()
                        .map_err(pyo3::exceptions::PyValueError::new_err)?;
                    let late = args.get_item(1)?.extract::<bool>()?;
                    for template in if late {
                        &queue_template[2..]
                    } else {
                        &queue_template[..2]
                    } {
                        let mut query = template.clone();
                        query.causation_id = Some(cause);
                        msgbus::send_trading_command(
                            MessagingSwitchboard::exec_engine_queue_execute(),
                            TradingCommand::QueryAccount(query),
                        );
                    }
                    Ok(())
                },
            )
            .unwrap();
            module.setattr("enqueue", enqueue).unwrap();
            py.run(
                c_str!(
                    r#"
import asyncio
import sqlite3
import tempfile
from pathlib import Path

directory = tempfile.TemporaryDirectory()
database = Path(directory.name) / "hosted.sqlite3"
connection = sqlite3.connect(database)
connection.execute("CREATE TABLE observations (kind TEXT, identity TEXT)")
connection.commit()
connection.close()
pending = asyncio.Event()
queries_done = asyncio.Event()
snapshots = []
dispatches = []

def on_snapshot(snapshot_id, account_event_id, balance, borrowed):
    with sqlite3.connect(database, timeout=0) as connection:
        connection.execute("BEGIN IMMEDIATE")
        connection.execute("INSERT INTO observations VALUES ('snapshot', ?)", (snapshot_id,))
    snapshots.append((snapshot_id, account_event_id, balance, borrowed))
    if len(snapshots) == 1:
        assert borrowed
        enqueue(snapshot_id, False)

def mark_waiting():
    pending.set()

def on_dispatch(command_id):
    with sqlite3.connect(database, timeout=0) as connection:
        connection.execute("BEGIN IMMEDIATE")
        connection.execute("INSERT INTO observations VALUES ('dispatch', ?)", (command_id,))
    dispatches.append(command_id)
    if len(dispatches) == 2:
        queries_done.set()

def cleanup():
    directory.cleanup()
"#
                ),
                Some(&module.dict()),
                None,
            )
            .unwrap();
            module.unbind()
        });
        let phase = Rc::new(Cell::new(ConnectPhase::Before));
        let queries = Rc::new(RefCell::new(Vec::new()));
        let stopped = Rc::new(Cell::new(0));
        let client_config = SandboxExecutionClientConfig {
            account_id,
            venue: Venue::from("SIM"),
            account_type: AccountType::Cash,
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
        let inner =
            SandboxExecutionClient::new(core, client_config, node.kernel().clock(), cache.clone());
        let probe = inner.account_capture_probe().unwrap();
        let client = HeldSandboxConnect {
            inner,
            finish: Some(finish_rx),
            phase: phase.clone(),
            waiting_callback: Python::attach(|py| {
                observer.bind(py).getattr("mark_waiting").unwrap().unbind()
            }),
            queries: queries.clone(),
            stopped: stopped.clone(),
            completed_query_callback: None,
        };
        engine
            .borrow_mut()
            .register_client(Box::new(client))
            .unwrap();
        let queued = Rc::new(RefCell::new(Vec::new()));
        let dispatched = Rc::new(RefCell::new(Vec::new()));
        msgbus::set_bus_tap(Rc::new(HostedQueryTap {
            engine: engine.clone(),
            queued: queued.clone(),
            dispatched: dispatched.clone(),
            callback: Python::attach(|py| {
                observer.bind(py).getattr("on_dispatch").unwrap().unbind()
            }),
        }));
        let snapshot_records = Rc::new(RefCell::new(Vec::new()));
        let records = snapshot_records.clone();
        let callback_cache = cache.clone();
        let callback_engine = engine.clone();
        let callback =
            Python::attach(|py| observer.bind(py).getattr("on_snapshot").unwrap().unbind());
        let snapshot_handler = TypedHandler::from(move |snapshot: &PortfolioSnapshot| {
            let account = callback_cache
                .borrow()
                .account_owned(&snapshot.account_id)
                .unwrap();
            let account_event = account.last_event().unwrap();
            assert_eq!(
                account.balance_total(None),
                Some(Money::from("100000 USDC"))
            );
            records.borrow_mut().push((
                snapshot.clone(),
                account_event.clone(),
                account.events().len(),
            ));
            Python::attach(|py| {
                callback
                    .call1(
                        py,
                        (
                            snapshot.event_id.to_string(),
                            account_event.event_id.to_string(),
                            account.balance_total(None).unwrap().to_string(),
                            callback_engine.try_borrow_mut().is_err(),
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
        let py_node = Python::attach(|py| {
            let finish = PyCFunction::new_closure(py, None, None, move |_args, _kwargs| -> bool {
                finish_tx
                    .lock()
                    .unwrap()
                    .take()
                    .is_some_and(|sender| sender.send(()).is_ok())
            })
            .unwrap();
            let py_node = pyo3::Py::new(py, PyLiveNode::new(node)).unwrap();
            let globals = PyDict::new(py);
            globals.set_item("node", &py_node).unwrap();
            globals.set_item("observer", observer.bind(py)).unwrap();
            globals.set_item("finish_connect", finish).unwrap();
            globals.set_item("outcome", outcome).unwrap();
            py.run(
                c_str!(
                    r#"
import asyncio

async def exercise():
    handle = node.handle()
    task = asyncio.create_task(node.run_async())
    await asyncio.wait_for(observer.pending.wait(), timeout=2)
    assert not task.done()
    assert not handle.is_running
    assert observer.dispatches == []
    assert len(observer.snapshots) == 1
    if outcome == "normal":
        assert finish_connect()
        await asyncio.wait_for(observer.queries_done.wait(), timeout=2)
        assert handle.is_running
        handle.stop()
        await asyncio.wait_for(task, timeout=3)
    elif outcome == "cancel":
        task.cancel()
        checkpoint = asyncio.get_running_loop().create_future()
        asyncio.get_running_loop().call_soon(checkpoint.set_result, None)
        await checkpoint
        assert handle.is_stopping
        assert not task.done()
        assert finish_connect()
        try:
            await asyncio.wait_for(task, timeout=3)
        except asyncio.CancelledError:
            pass
        else:
            raise AssertionError("hosted cancellation was not re-raised")
    else:
        try:
            await asyncio.wait_for(task, timeout=3)
        except RuntimeError as error:
            assert "exec-connect" in str(error), str(error)
        else:
            raise AssertionError("held native connect did not time out")
    assert not handle.is_running
    assert handle.is_stopping == (outcome != "timeout")
    assert len(observer.dispatches) == 2
    assert len(observer.snapshots) == 2
    assert not finish_connect()
    observer.enqueue(observer.snapshots[0][0], True)

asyncio.run(exercise())
"#
                ),
                Some(&globals),
                None,
            )
            .unwrap();
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
            msgbus::unsubscribe_portfolio_snapshot("events.portfolio.*".into(), &snapshot_handler);
            msgbus::clear_bus_tap();
            py_node
        });
        assert_eq!(handle.state(), NodeState::Stopped);
        assert!(stopped.get() >= 1);
        assert_eq!(
            probe.capture().unwrap_err(),
            nautilus_common::clients::ExecutionAccountCaptureError::Retired
        );
        assert_eq!(snapshot_records.borrow().len(), 2);
        assert_eq!(snapshot_records.borrow()[0].2, 1);
        assert_eq!(snapshot_records.borrow()[1].2, 3);
        let account = cache.borrow().account_owned(&account_id).unwrap();
        assert_eq!(account.events().len(), 3);
        let event_ids: std::collections::HashSet<_> = account
            .events()
            .iter()
            .map(|event| event.event_id)
            .collect();
        assert_eq!(event_ids.len(), 3);
        assert_eq!(engine.borrow().command_count(), 2);
        assert_eq!(queued.borrow().len(), 3);
        assert_eq!(dispatched.borrow().len(), 2);
        assert_eq!(queries.borrow().len(), 2);
        assert_eq!(dispatched.borrow().as_slice(), &queued.borrow()[..2]);
        let expected_phase = if outcome == "timeout" {
            ConnectPhase::Dropped
        } else {
            ConnectPhase::Returned
        };
        assert_eq!(phase.get(), expected_phase);
        for (index, (query, query_phase)) in queries.borrow().iter().enumerate() {
            assert_eq!(*query, queued.borrow()[index]);
            assert_eq!(*query_phase, expected_phase);
            assert_eq!(query.command_id, query_ids[index]);
            assert_eq!(
                query.causation_id,
                Some(snapshot_records.borrow()[0].0.event_id)
            );
            assert_eq!(query.ts_init, query_template[index].ts_init);
        }
        eprintln!(
            "Hosted {outcome}: query phases={:?}; direct IDs={:?}; capture retired after stop",
            queries
                .borrow()
                .iter()
                .map(|(_, phase)| *phase)
                .collect::<Vec<_>>(),
            dispatched
                .borrow()
                .iter()
                .map(|query| query.command_id)
                .collect::<Vec<_>>()
        );
        Python::attach(|py| {
            py_node.call_method0(py, "dispose").unwrap();
            observer.bind(py).call_method0("cleanup").unwrap();
        });
    }
    #[pyo3::pyclass(unsendable)]
    struct OrdinaryCallbackBorrowProbe {
        engine: Rc<RefCell<ExecutionEngine>>,
    }

    #[pyo3::pymethods]
    impl OrdinaryCallbackBorrowProbe {
        fn __call__(&self) -> bool {
            let borrowed = self.engine.try_borrow_mut();
            let available = borrowed.is_ok();
            // The observation must not itself hold the engine through Python or SQLite work.
            drop(borrowed);
            available
        }
    }

    #[pyo3::pyclass(unsendable)]
    struct FlipCallbackStateProbe {
        engine: Rc<RefCell<ExecutionEngine>>,
        cache: Rc<RefCell<Cache>>,
        account_id: AccountId,
    }

    #[pyo3::pymethods]
    impl FlipCallbackStateProbe {
        fn __call__(&self, position_id: PositionId) -> (bool, String, String) {
            let borrow = self.engine.try_borrow_mut();
            let available = borrow.is_ok();
            drop(borrow);
            let cache = self.cache.borrow();
            let account = cache.account_owned(&self.account_id).unwrap();
            let position = cache.position_owned(&position_id).unwrap();
            (
                available,
                serde_json::to_string(&account).unwrap(),
                serde_json::to_string(&position).unwrap(),
            )
        }
    }

    #[rstest]
    #[case::long_to_short(true)]
    #[case::short_to_long(false)]
    fn test_hosted_netting_flip_delivers_completed_fill(#[case] initially_long: bool) {
        run_hosted_netting_flip(initially_long, false, false);
    }

    #[rstest]
    fn test_hosted_netting_flip_retains_closed_callback_fault() {
        run_hosted_netting_flip(true, true, false);
    }

    #[rstest]
    #[ignore = "requires explicit CUSTOS_SOURCE_ROOT and compatible Custos PYO3_PYTHON"]
    fn test_cross_repo_netting_flip_retains_signed_child_after_closed_fault() {
        run_hosted_netting_flip(true, true, true);
    }

    fn run_hosted_netting_flip(initially_long: bool, fail_closed: bool, signed_child: bool) {
        use nautilus_common::owner_delivery::{
            self, OwnerDeliveryCause, OwnerDeliveryError, OwnerExternalEvent, OwnerIncomingOrder,
            OwnerIncomingStage, OwnerObserver,
        };
        use nautilus_model::python::orders::order_any_to_pyobject;
        use nautilus_trading::python::strategy::PyStrategy;
        use pyo3::types::PyTypeMethods;

        type FlipRow = (String, String, String, String, String, String, bool);
        type FlipDeliveryStatus = (bool, Option<(usize, Option<String>)>);

        fn typed_value(event: &OwnerExternalEvent) -> serde_json::Value {
            match event {
                OwnerExternalEvent::Order(event) => serde_json::to_value(event).unwrap(),
                OwnerExternalEvent::Position(PositionEvent::PositionOpened(event)) => {
                    serde_json::to_value(event).unwrap()
                }
                OwnerExternalEvent::Position(PositionEvent::PositionClosed(event)) => {
                    serde_json::to_value(event).unwrap()
                }
                other => panic!("unexpected typed output {other:?}"),
            }
        }

        struct FlipTap {
            incoming: Rc<RefCell<Vec<OrderEventAny>>>,
            outputs: Rc<RefCell<Vec<OwnerExternalEvent>>>,
            owner: Rc<RefCell<Option<OwnerObserver>>>,
            native_cancels: Rc<RefCell<Vec<TradingCommand>>>,
        }
        impl BusTap for FlipTap {
            fn on_send(&self, endpoint: MStr<Endpoint>, message: &dyn Any) {
                if endpoint == MessagingSwitchboard::order_emulator_execute()
                    && let Some(command @ TradingCommand::CancelOrder(_)) =
                        message.downcast_ref::<TradingCommand>()
                {
                    self.native_cancels.borrow_mut().push(command.clone());
                }
                if endpoint == MessagingSwitchboard::exec_engine_process()
                    && let Some(event) = message.downcast_ref::<OrderEventAny>()
                {
                    self.incoming.borrow_mut().push(event.clone());
                    self.owner
                        .borrow_mut()
                        .get_or_insert_with(|| owner_delivery::current_observer().unwrap());
                }
            }
            fn on_publish(&self, topic: MStr<Topic>, message: &dyn Any) {
                if let Some(event) = message.downcast_ref::<OrderEventAny>()
                    && topic.as_str() == format!("events.order.{}", event.strategy_id())
                    && matches!(event, OrderEventAny::Filled(_))
                {
                    self.outputs
                        .borrow_mut()
                        .push(OwnerExternalEvent::Order(event.clone()));
                }
                if let Some(event) = message.downcast_ref::<PositionEvent>()
                    && topic.as_str().starts_with("events.position.")
                {
                    self.outputs
                        .borrow_mut()
                        .push(OwnerExternalEvent::Position(event.clone()));
                }
            }
        }

        Python::initialize();
        let signed_fixture =
            signed_child.then(|| super::hosted_control_cancel::CustosControlFixture::new(true));
        let trader_id = TraderId::from("FLIP-001");
        let account_id = AccountId::from("SIM-001");
        let client_id = ClientId::from("FLIP");
        let node = LiveNodeBuilder::from_config(LiveNodeConfig {
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
        })
        .unwrap()
        .with_logging(LoggerConfig {
            bypass_logging: true,
            ..Default::default()
        })
        .build()
        .unwrap();
        let cache = node.kernel().cache();
        let engine = node.kernel().exec_engine().clone();
        let clock = node.kernel().clock();
        let mut perpetual = crypto_perpetual_ethusdt();
        perpetual.id = InstrumentId::from("FLIP.SIM");
        perpetual.maker_fee = Decimal::new(1, 3);
        perpetual.taker_fee = Decimal::new(1, 3);
        let instrument = InstrumentAny::CryptoPerpetual(perpetual);
        cache
            .borrow_mut()
            .add_instrument(instrument.clone())
            .unwrap();
        let observer = Python::attach(|py| {
            let module = PyModule::new(py, "hosted_netting_flip_observer").unwrap();
            module
                .setattr("Strategy", py.get_type::<PyStrategy>())
                .unwrap();
            module
                .setattr("client_id", client_id.into_py_any(py).unwrap())
                .unwrap();
            module
                .setattr("account_id", account_id.into_py_any(py).unwrap())
                .unwrap();
            module.setattr("fail_closed", fail_closed).unwrap();
            if let Some(fixture) = &signed_fixture {
                module
                    .setattr("signed_fixture", fixture.module.bind(py))
                    .unwrap();
            } else {
                module.setattr("signed_fixture", py.None()).unwrap();
            }
            module
                .setattr(
                    "control_id",
                    ClientOrderId::from("FLIP-CONTROL").into_py_any(py).unwrap(),
                )
                .unwrap();
            module
                .setattr(
                    "native_state",
                    pyo3::Py::new(
                        py,
                        FlipCallbackStateProbe {
                            engine: engine.clone(),
                            cache: cache.clone(),
                            account_id,
                        },
                    )
                    .unwrap(),
                )
                .unwrap();
            py.run(c_str!(r#"
import asyncio
import sqlite3
import tempfile
from pathlib import Path
import os

directory = tempfile.TemporaryDirectory()
database = signed_fixture.outbox_database if signed_fixture is not None else Path(directory.name) / "flip.sqlite3"
with sqlite3.connect(database) as connection:
    connection.execute("CREATE TABLE deliveries(kind TEXT, event_id TEXT, outlet TEXT, position_id TEXT, quantity TEXT, side TEXT, borrow_free INTEGER)")
records = []
payloads = []
states = []
errors = []
done = False
node_error = None
stop_status = None
control_at_start = None
control_requests = []
original_authority = signed_fixture.boundary.platform_opening_gate.authority if signed_fixture is not None else None
database_identity = (str(database.resolve()), os.stat(database).st_dev, os.stat(database).st_ino)

def observe(outlet, event):
    position = strategy.cache.position(event.position_id)
    native_free, account_json, position_json = native_state(event.position_id)
    row = (type(event).__name__, str(event.event_id), outlet, str(event.position_id),
           str(position.quantity), str(position.side), native_free)
    with sqlite3.connect(database, timeout=0) as connection:
        connection.execute("BEGIN IMMEDIATE")
        connection.execute("INSERT INTO deliveries VALUES (?, ?, ?, ?, ?, ?, ?)", row)
    records.append(row)
    payloads.append((outlet, event))
    states.append((str(event.event_id), account_json, position_json))

class FlipStrategy(Strategy):
    def on_start(self):
        global control_at_start
        if signed_fixture is not None:
            # Standalone local history predates the control-only gate; no financial grant is added.
            self.submit_order(control_order, client_id=client_id)
            control_at_start = str(self.cache.order(control_id).status)
        self.submit_order(self.orders[0], client_id=client_id)
    def on_stop(self):
        global stop_status
        stop_status = self._control_delivery_status()
    def on_dispose(self): pass
    def on_order_filled(self, event): observe("specific", event)
    def on_order_event(self, event):
        if type(event).__name__ == "OrderFilled": observe("aggregate", event)
    def on_position_opened(self, event):
        observe("specific", event)
        if event.opening_order_id == self.orders[0].client_order_id:
            self.submit_order(self.orders[1], client_id=client_id)
    def on_position_closed(self, event):
        observe("specific", event)
        if fail_closed:
            if signed_fixture is not None:
                signed_fixture.install_order_gate(self, signed_fixture.gate)
                before = self.clock.timestamp_ns()
                self.cancel_order(control_id, client_id=client_id, params={"closed_parent": str(event.event_id)})
                after = self.clock.timestamp_ns()
                control_requests.append((str(event.event_id), before, after, str(self.cache.order(control_id).status)))
            enqueue_query(str(event.event_id), event.ts_init)
            send_late()
            try:
                raise ValueError("closed callback:" + str(event.event_id))
            except ValueError as error:
                errors.append(repr(error))
                raise
    def on_position_event(self, event):
        global done
        observe("aggregate", event)
        if type(event).__name__ == "PositionOpened" and event.opening_order_id == self.orders[1].client_order_id:
            done = True

strategy = FlipStrategy()
def mark_waiting(): pass
def persisted():
    with sqlite3.connect(database, timeout=0) as connection:
        return [(a,b,c,d,e,f,bool(g)) for a,b,c,d,e,f,g in connection.execute("SELECT * FROM deliveries ORDER BY rowid")]
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
        if signed_child {
            use nautilus_model::enums::TriggerType;
            let control_order = OrderTestBuilder::new(OrderType::StopLimit)
                .trader_id(trader_id)
                .strategy_id(strategy_id)
                .instrument_id(instrument.id())
                .client_order_id(ClientOrderId::from("FLIP-CONTROL"))
                .side(OrderSide::Buy)
                .quantity(Quantity::from("0.100"))
                .price(Price::from("101.00"))
                .trigger_price(Price::from("101.00"))
                .trigger_type(TriggerType::BidAsk)
                .emulation_trigger(TriggerType::BidAsk)
                .build();
            Python::attach(|py| {
                observer
                    .bind(py)
                    .setattr(
                        "control_order",
                        order_any_to_pyobject(py, control_order).unwrap(),
                    )
                    .unwrap();
            });
        }
        let orders: Vec<_> = [
            ("FLIP-OPEN", initially_long, "1.000"),
            ("FLIP-CROSS", !initially_long, "1.500"),
        ]
        .into_iter()
        .map(|(id, buy, quantity)| {
            OrderTestBuilder::new(OrderType::Limit)
                .trader_id(trader_id)
                .strategy_id(strategy_id)
                .instrument_id(instrument.id())
                .client_order_id(id.into())
                .side(if buy { OrderSide::Buy } else { OrderSide::Sell })
                .quantity(Quantity::from(quantity))
                .price(Price::from(if buy { "100.02" } else { "100.00" }))
                .build()
        })
        .collect();
        Python::attach(|py| {
            strategy
                .bind(py)
                .setattr(
                    "orders",
                    orders
                        .iter()
                        .map(|order| order_any_to_pyobject(py, order.clone()).unwrap())
                        .collect::<Vec<_>>(),
                )
                .unwrap();
        });
        let config = SandboxExecutionClientConfig {
            account_id,
            venue: Venue::from("SIM"),
            oms_type: OmsType::Netting,
            account_type: AccountType::Margin,
            base_currency: Some(Currency::USDT()),
            starting_balances: vec![Money::from("1000000 USDT")],
            ..Default::default()
        };
        let late_order = OrderTestBuilder::new(OrderType::Limit)
            .trader_id(trader_id)
            .strategy_id(strategy_id)
            .instrument_id(instrument.id())
            .client_order_id("FLIP-LATE".into())
            .side(OrderSide::Buy)
            .quantity(Quantity::from("1.000"))
            .price(Price::from("99.00"))
            .build();
        let late_input = TestOrderEventStubs::submitted(&late_order, account_id);
        if fail_closed {
            cache
                .borrow_mut()
                .add_order(late_order.clone(), None, Some(client_id), true)
                .unwrap();
        }
        let late_sender = nautilus_common::live::get_exec_event_sender();
        let late = Mutex::new(Some(late_input.clone()));
        Python::attach(|py| {
            let send_late = PyCFunction::new_closure(
                py,
                None,
                None,
                move |_args: &pyo3::Bound<'_, PyTuple>, _| -> pyo3::PyResult<()> {
                    late_sender
                        .send(ExecutionEvent::Order(late.lock().unwrap().take().unwrap()))
                        .map_err(|error| {
                            pyo3::exceptions::PyRuntimeError::new_err(error.to_string())
                        })
                },
            )
            .unwrap();
            observer.bind(py).setattr("send_late", send_late).unwrap();
            let enqueue_query = PyCFunction::new_closure(
                py,
                None,
                None,
                move |args: &pyo3::Bound<'_, PyTuple>, _| -> pyo3::PyResult<()> {
                    let cause: UUID4 = args
                        .get_item(0)?
                        .extract::<String>()?
                        .parse()
                        .map_err(pyo3::exceptions::PyValueError::new_err)?;
                    let ts = args.get_item(1)?.extract::<u64>()?;
                    let mut query = QueryAccount::new(
                        trader_id,
                        Some(client_id),
                        account_id,
                        UUID4::new(),
                        UnixNanos::from(ts),
                        None,
                        None,
                    );
                    query.causation_id = Some(cause);
                    msgbus::send_trading_command(
                        MessagingSwitchboard::exec_engine_queue_execute(),
                        TradingCommand::QueryAccount(query),
                    );
                    Ok(())
                },
            )
            .unwrap();
            observer
                .bind(py)
                .setattr("enqueue_query", enqueue_query)
                .unwrap();
        });
        let core = ExecutionClientCore::new(
            trader_id,
            client_id,
            config.venue,
            config.oms_type,
            account_id,
            config.account_type,
            config.base_currency,
            cache.clone(),
        );
        let inner = SandboxExecutionClient::new(core, config, clock.clone(), cache.clone());
        let capture = inner.account_capture_probe().unwrap();
        let stopped = Rc::new(Cell::new(0));
        engine
            .borrow_mut()
            .register_client(ready_hosted_sandbox_client(
                inner,
                Python::attach(|py| observer.bind(py).getattr("mark_waiting").unwrap().unbind()),
                stopped.clone(),
            ))
            .unwrap();
        let incoming = Rc::new(RefCell::new(Vec::new()));
        let outputs = Rc::new(RefCell::new(Vec::new()));
        let observed_owner = Rc::new(RefCell::new(None));
        let native_cancels = Rc::new(RefCell::new(Vec::new()));
        msgbus::set_bus_tap(Rc::new(FlipTap {
            incoming: incoming.clone(),
            outputs: outputs.clone(),
            owner: observed_owner.clone(),
            native_cancels: native_cancels.clone(),
        }));
        let quote_sender = nautilus_common::live::get_data_event_sender();
        let quote = QuoteTick::new(
            instrument.id(),
            Price::from("100.00"),
            Price::from("100.02"),
            Quantity::from("100.000"),
            Quantity::from("100.000"),
            clock.borrow().timestamp_ns(),
            clock.borrow().timestamp_ns(),
        );
        quote_sender
            .send(nautilus_common::messages::DataEvent::Data(Data::Quote(
                quote,
            )))
            .unwrap();
        let (py_node, result) = Python::attach(|py| {
            let node = pyo3::Py::new(py, PyLiveNode::new(node)).unwrap();
            let globals = PyDict::new(py);
            globals.set_item("node", &node).unwrap();
            globals.set_item("observer", observer.bind(py)).unwrap();
            let result = py.run(
                c_str!(
                    r#"
import asyncio
async def exercise():
    task = asyncio.create_task(node.run_async())
    async def wait_done():
        while not observer.done:
            if task.done():
                await task
                raise AssertionError("node exited before flip notifications")
            await asyncio.sleep(0)
    try:
        if observer.fail_closed:
            try:
                await asyncio.wait_for(task, timeout=3)
            except TimeoutError:
                raise
            except Exception as error:
                observer.node_error = repr(error)
            else:
                raise AssertionError("normal node swallowed Closed callback failure")
        else:
            await asyncio.wait_for(wait_done(), timeout=3)
    finally:
        node.handle().stop()
        if not task.done():
            await asyncio.wait_for(task, timeout=3)
        elif not observer.fail_closed:
            await task
asyncio.run(exercise())
"#
                ),
                Some(&globals),
                None,
            );
            (node, result)
        });
        msgbus::clear_bus_tap();
        let account = cache.borrow().account_owned(&account_id).unwrap();
        let position_id = PositionId::new(format!("{}-{strategy_id}", instrument.id()));
        let position = cache.borrow().position_owned(&position_id);
        let snapshots = cache.borrow().position_snapshots(Some(&position_id), None);
        let position_indexes = (
            cache.borrow().is_position_open(&position_id),
            cache.borrow().is_position_closed(&position_id),
        );
        let final_orders: Vec<_> = orders
            .iter()
            .map(|order| {
                cache
                    .borrow()
                    .order_owned(&order.client_order_id())
                    .unwrap()
            })
            .collect();
        let owner = observed_owner.borrow().clone().unwrap();
        let owner_status = owner.status().unwrap().unwrap();
        let retained_inputs = owner.retained_incoming_orders().unwrap();
        let retained_outputs = owner.retained_typed_deliveries().unwrap();
        let retained_commands = owner.retained_commands().unwrap();
        let late_after_stop = cache.borrow().order_owned(&late_order.client_order_id());
        let command_count = engine.borrow().command_count();
        let control_after_stop = cache
            .borrow()
            .order_owned(&ClientOrderId::from("FLIP-CONTROL"));
        let command_work = owner.retained_command_work().unwrap();
        assert_eq!(owner.retained_command_work().unwrap(), command_work);
        if let Some(fixture) = &signed_fixture {
            Python::attach(|py| {
                let scope = PyDict::new(py);
                scope.set_item("observer", observer.bind(py)).unwrap();
                scope.set_item("fixture", fixture.module.bind(py)).unwrap();
                py.run(c_str!(r#"
import os
assert observer.control_at_start == "EMULATED"
assert fixture.boundary.platform_opening_gate.authority == observer.original_authority == fixture.EngineLifecycleAuthority.from_verified_command(fixture.verified)
assert fixture.outbox_identity == fixture.observer_identity
assert os.path.samefile(observer.database, fixture.outbox_database)
assert observer.database_identity[1:] == (fixture.outbox_identity["device"], fixture.outbox_identity["inode"])
assert fixture.source_audit()
assert not fixture.refusals
assert not fixture.canceled_callbacks
"#), Some(&scope), None).unwrap();
            });
        }
        let control_requests: Vec<(String, u64, u64, String)> = Python::attach(|py| {
            observer
                .bind(py)
                .getattr("control_requests")
                .unwrap()
                .extract()
                .unwrap()
        });
        let (rows, persisted, state, callback_payloads, states, errors, node_error, stop_status) =
            Python::attach(|py| {
                let module = observer.bind(py);
                let rows: Vec<FlipRow> = module.getattr("records").unwrap().extract().unwrap();
                let persisted: Vec<FlipRow> =
                    module.call_method0("persisted").unwrap().extract().unwrap();
                let state = py_node
                    .call_method0(py, "handle")
                    .unwrap()
                    .getattr(py, "state")
                    .unwrap()
                    .extract::<NodeState>(py)
                    .unwrap();
                let payloads: Vec<(String, pyo3::Py<pyo3::PyAny>)> =
                    module.getattr("payloads").unwrap().extract().unwrap();
                let callback_payloads: Vec<_> = payloads
                    .into_iter()
                    .map(|(outlet, event)| {
                        let kind: String = event.bind(py).get_type().name().unwrap().to_string();
                        let payload = match kind.as_str() {
                            "OrderFilled" => OwnerExternalEvent::Order(OrderEventAny::Filled(
                                event.bind(py).extract::<OrderFilled>().unwrap(),
                            )),
                            "PositionClosed" => {
                                OwnerExternalEvent::Position(PositionEvent::PositionClosed(
                                    event.bind(py).extract::<PositionClosed>().unwrap(),
                                ))
                            }
                            "PositionOpened" => {
                                OwnerExternalEvent::Position(PositionEvent::PositionOpened(
                                    event
                                        .bind(py)
                                        .extract::<nautilus_model::events::PositionOpened>()
                                        .unwrap(),
                                ))
                            }
                            other => panic!("unexpected callback payload {other}"),
                        };
                        (outlet, payload)
                    })
                    .collect();
                let states: Vec<(String, String, String)> =
                    module.getattr("states").unwrap().extract().unwrap();
                let errors: Vec<String> = module.getattr("errors").unwrap().extract().unwrap();
                let node_error: Option<String> =
                    module.getattr("node_error").unwrap().extract().unwrap();
                let stop_status: FlipDeliveryStatus =
                    module.getattr("stop_status").unwrap().extract().unwrap();
                py_node.call_method0(py, "dispose").unwrap();
                module.call_method0("cleanup").unwrap();
                (
                    rows,
                    persisted,
                    state,
                    callback_payloads,
                    states,
                    errors,
                    node_error,
                    stop_status,
                )
            });
        assert!(
            result.is_ok(),
            "flip runtime {result:?}, rows={rows:?}, incoming={:?}, outputs={:?}",
            incoming.borrow(),
            outputs.borrow()
        );
        assert_eq!(state, NodeState::Stopped);
        assert!(stopped.get() > 0);
        assert_eq!(
            capture.capture().unwrap_err(),
            nautilus_common::clients::ExecutionAccountCaptureError::Retired
        );
        assert_eq!(rows, persisted);
        assert!(!rows.is_empty());
        assert!(
            rows.iter().all(|row| row.6),
            "all specific and aggregate callbacks must be outside the native borrow: {rows:?}"
        );
        let position = position.unwrap();
        assert_eq!(position.quantity, Quantity::from("0.500"));
        assert_eq!(
            position.side,
            if initially_long {
                PositionSide::Short
            } else {
                PositionSide::Long
            }
        );
        assert_eq!(snapshots.len(), 1);
        assert!(snapshots[0].is_closed());
        assert_eq!(position_indexes, (true, false));
        let output_kinds: Vec<_> = outputs
            .borrow()
            .iter()
            .map(|event| match event {
                OwnerExternalEvent::Order(OrderEventAny::Filled(_)) => "Filled",
                OwnerExternalEvent::Position(PositionEvent::PositionOpened(_)) => "Opened",
                OwnerExternalEvent::Position(PositionEvent::PositionClosed(_)) => "Closed",
                other => panic!("unexpected output {other:?}"),
            })
            .collect();
        assert_eq!(
            output_kinds,
            ["Filled", "Opened", "Filled", "Closed", "Opened"]
        );
        let mut expected_rows = Vec::new();
        for output in outputs.borrow().iter() {
            let (kind, id, quantity, side) = match output {
                OwnerExternalEvent::Order(OrderEventAny::Filled(fill)) => (
                    "OrderFilled",
                    fill.event_id,
                    if fill.client_order_id == orders[0].client_order_id() {
                        "1.000"
                    } else {
                        "0.500"
                    },
                    if fill.client_order_id == orders[0].client_order_id() {
                        initially_long
                    } else {
                        !initially_long
                    },
                ),
                OwnerExternalEvent::Position(PositionEvent::PositionOpened(event)) => (
                    "PositionOpened",
                    event.event_id,
                    if event.opening_order_id == orders[0].client_order_id() {
                        "1.000"
                    } else {
                        "0.500"
                    },
                    if event.opening_order_id == orders[0].client_order_id() {
                        initially_long
                    } else {
                        !initially_long
                    },
                ),
                OwnerExternalEvent::Position(PositionEvent::PositionClosed(event)) => {
                    ("PositionClosed", event.event_id, "0.500", !initially_long)
                }
                other => panic!("unexpected flip output {other:?}"),
            };
            for outlet in ["specific", "aggregate"] {
                expected_rows.push((
                    kind.to_owned(),
                    id.to_string(),
                    outlet.to_owned(),
                    position_id.to_string(),
                    quantity.to_owned(),
                    if side { "LONG" } else { "SHORT" }.to_owned(),
                    true,
                ));
            }
        }
        assert_eq!(rows, expected_rows);
        assert_eq!(outputs.borrow().len(), 5);
        assert_eq!(callback_payloads.len(), 10);
        for ((outlet, actual), (expected_outlet, expected)) in callback_payloads.iter().zip(
            outputs
                .borrow()
                .iter()
                .flat_map(|event| [("specific", event), ("aggregate", event)]),
        ) {
            assert_eq!(outlet, expected_outlet);
            assert_eq!(
                typed_value(actual),
                typed_value(expected),
                "every Python callback retains the complete published typed original"
            );
        }
        assert_eq!(states.len(), 10);
        for (_, account_json, position_json) in &states[4..] {
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(account_json).unwrap(),
                serde_json::to_value(&account).unwrap()
            );
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(position_json).unwrap(),
                serde_json::to_value(&position).unwrap()
            );
        }
        let original_fills: Vec<_> = incoming
            .borrow()
            .iter()
            .filter_map(|event| {
                if let OrderEventAny::Filled(fill) = event {
                    Some(fill.clone())
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(original_fills.len(), 2);
        for (index, original) in original_fills.iter().enumerate() {
            let canonical: Vec<_> = final_orders[index]
                .events()
                .into_iter()
                .filter_map(|event| {
                    if let OrderEventAny::Filled(fill) = event {
                        Some(fill.clone())
                    } else {
                        None
                    }
                })
                .collect();
            assert_eq!(canonical.len(), 1);
            let mut normalized = original.clone();
            normalized.position_id = Some(position_id);
            assert_eq!(canonical[0], normalized);
            let OwnerExternalEvent::Order(OrderEventAny::Filled(published)) =
                &outputs.borrow()[index * 2]
            else {
                panic!("real published fill required")
            };
            assert_eq!(published, &canonical[0]);
            assert_eq!(canonical[0].liquidity_side, LiquiditySide::Taker);
            let expected_fee = Money::from_decimal(
                canonical[0].last_qty.as_decimal()
                    * canonical[0].last_px.as_decimal()
                    * instrument.taker_fee(),
                Currency::USDT(),
            )
            .unwrap();
            assert_eq!(canonical[0].commission, Some(expected_fee));
            assert_eq!(
                final_orders[index].commissions().get(&Currency::USDT()),
                Some(&expected_fee)
            );
        }
        let closing = snapshots[0].events.last().unwrap();
        let opening = position.events.first().unwrap();
        assert_eq!(closing.event_id, original_fills[1].event_id);
        assert_ne!(opening.event_id, closing.event_id);
        assert_eq!(opening.causation_id, Some(closing.event_id));
        assert_eq!(opening.trade_id, closing.trade_id);
        assert_eq!(closing.last_qty, Quantity::from("1.000"));
        assert_eq!(opening.last_qty, Quantity::from("0.500"));
        assert_eq!(
            opening.last_qty + closing.last_qty,
            original_fills[1].last_qty
        );
        assert_eq!(
            closing.commission.unwrap() + opening.commission.unwrap(),
            original_fills[1].commission.unwrap()
        );
        let fees = original_fills
            .iter()
            .fold(Money::zero(Currency::USDT()), |sum, fill| {
                sum + fill.commission.unwrap()
            });
        // Closing one unit across the actual 100.00/100.02 spread loses exactly 0.02.
        let gross = if initially_long {
            original_fills[1].last_px.as_decimal() - original_fills[0].last_px.as_decimal()
        } else {
            original_fills[0].last_px.as_decimal() - original_fills[1].last_px.as_decimal()
        } * closing.last_qty.as_decimal();
        assert_eq!(gross, Decimal::new(-2, 2));
        let expected_pnl = Money::from_decimal(gross, Currency::USDT()).unwrap() - fees;
        assert_eq!(
            account.balance_total(None).unwrap(),
            Money::from("1000000 USDT") + expected_pnl
        );
        assert_eq!(
            snapshots[0].realized_pnl.unwrap() + position.realized_pnl.unwrap(),
            expected_pnl
        );
        assert_eq!(
            snapshots[0].closing_order_id,
            Some(orders[1].client_order_id())
        );
        assert_eq!(position.opening_order_id, orders[1].client_order_id());
        let outputs_ref = outputs.borrow();
        let OwnerExternalEvent::Position(PositionEvent::PositionClosed(closed)) = &outputs_ref[3]
        else {
            panic!("closed output required")
        };
        let OwnerExternalEvent::Position(PositionEvent::PositionOpened(opened)) = &outputs_ref[4]
        else {
            panic!("opened output required")
        };
        // The cache archive assigns its clone a distinct UUID-suffixed ID. This expected-value
        // normalization never changes the archive or the live cache observed by callbacks.
        let archive_prefix = format!("{position_id}-");
        snapshots[0]
            .id
            .as_str()
            .strip_prefix(&archive_prefix)
            .unwrap()
            .parse::<UUID4>()
            .unwrap();
        let mut closed_cycle = snapshots[0].clone();
        closed_cycle.id = position_id;
        assert_eq!(
            closed,
            &PositionClosed::create(&closed_cycle, closing, closed.event_id, closed.ts_init)
        );
        assert_eq!(
            opened,
            &nautilus_model::events::PositionOpened::create(
                &position,
                opening,
                opened.event_id,
                opened.ts_init
            )
        );
        assert_ne!(closed.event_id, opened.event_id);
        assert_ne!(closed.event_id, original_fills[1].event_id);
        assert_eq!(closed.quantity, Quantity::from("0.000"));
        assert_eq!(opened.quantity, Quantity::from("0.500"));
        if fail_closed {
            assert_eq!(
                errors,
                [format!("ValueError('closed callback:{}')", closed.event_id)]
            );
            assert!(node_error.unwrap().contains("ExternalCallback"));
            assert_eq!(owner_status.1, Some(OwnerDeliveryError::ExternalCallback));
            assert!(owner_status.0 > 0);
            assert_eq!(stop_status.0, signed_child);
            let stopped_owner = stop_status
                .1
                .expect("actual owner status remains available");
            assert!(stopped_owner.0 > 0 && stopped_owner.1.unwrap().contains("ExternalCallback"));
            assert_eq!(retained_outputs.len(), 1);
            assert_eq!(
                typed_value(&retained_outputs[0].event),
                typed_value(&outputs_ref[3])
            );
            assert_eq!(
                retained_outputs[0].error,
                Some(OwnerDeliveryError::ExternalCallback)
            );
            let OwnerDeliveryCause::Incoming(cause) = retained_outputs[0].cause else {
                panic!("real original Incoming cause required")
            };
            assert_eq!(cause.event_id, original_fills[1].event_id);
            assert_ne!(
                cause.incarnation.to_string(),
                "00000000-0000-0000-0000-000000000000"
            );
            assert_eq!(retained_inputs.len(), 2);
            for (item, expected, stage) in [
                (
                    &retained_inputs[0],
                    OrderEventAny::Filled(original_fills[1].clone()),
                    OwnerIncomingStage::DispatchReturned,
                ),
                (
                    &retained_inputs[1],
                    late_input,
                    OwnerIncomingStage::NotDispatched,
                ),
            ] {
                let OwnerIncomingOrder::Process(original) = &item.event else {
                    panic!("original incoming required")
                };
                assert_eq!(original, &expected);
                assert_eq!(item.stage, stage);
                let original_cause = item.cause.unwrap();
                assert_eq!(original_cause.event_id, original.clone().into_boxed().id());
                assert_eq!(original_cause.incarnation, cause.incarnation);
            }
            assert!(
                retained_inputs[1].cause.unwrap().sequence
                    > retained_inputs[0].cause.unwrap().sequence
            );
            assert_eq!(retained_commands.len(), 1);
            let TradingCommand::QueryAccount(query) = &retained_commands[0].command else {
                panic!("original queued neutral query required")
            };
            assert_eq!(query.causation_id, Some(closed.event_id));
            assert_eq!(query.account_id, account_id);
            assert_eq!(
                retained_commands[0].error,
                OwnerDeliveryError::ExternalCallback
            );
            let late = late_after_stop.unwrap();
            assert_eq!(late.status(), OrderStatus::Initialized);
            assert_eq!(
                late.events().into_iter().cloned().collect::<Vec<_>>(),
                [OrderEventAny::Initialized(late_order.init_event().clone())]
            );
            assert_eq!(
                command_count, 2,
                "neither the late incoming nor the queued query may produce a new effect"
            );
        } else {
            assert!(errors.is_empty());
            assert!(node_error.is_none());
            assert_eq!(owner_status, (0, None));
            assert_eq!(stop_status, (false, Some((0, None))));
            assert!(
                retained_inputs.is_empty()
                    && retained_outputs.is_empty()
                    && retained_commands.is_empty()
            );
            assert!(late_after_stop.is_none());
            assert_eq!(command_count, 2);
        }
        assert!(
            native_cancels.borrow().is_empty(),
            "no retained Cancel child may reach the native endpoint"
        );
        if signed_child {
            assert_eq!(command_work.len(), 1);
            let work = &command_work[0];
            let TradingCommand::CancelOrder(cancel) = &work.command else {
                panic!("real captured signed Cancel child required")
            };
            assert_eq!(control_requests.len(), 1);
            let (parent, before, after, state) = &control_requests[0];
            assert_eq!(parent, &closed.event_id.to_string());
            assert_eq!(state, "EMULATED");
            assert!(*before <= cancel.ts_init.as_u64() && cancel.ts_init.as_u64() <= *after);
            assert_eq!(cancel.trader_id, trader_id);
            assert_eq!(cancel.strategy_id, strategy_id);
            assert_eq!(cancel.client_id, Some(client_id));
            assert_eq!(cancel.instrument_id, instrument.id());
            assert_eq!(cancel.client_order_id, ClientOrderId::from("FLIP-CONTROL"));
            assert_eq!(cancel.venue_order_id, None);
            assert_eq!(cancel.correlation_id, None);
            assert_eq!(
                cancel.causation_id, None,
                "native constructor fields are not rewritten to invent a wire parent"
            );
            assert_eq!(
                serde_json::to_value(&cancel.params).unwrap(),
                serde_json::json!({"closed_parent": closed.event_id.to_string()})
            );
            assert_ne!(
                cancel.command_id.to_string(),
                "00000000-0000-0000-0000-000000000000"
            );
            assert_eq!(work.cause.command_id, cancel.command_id);
            assert_eq!(work.cause.parent_event_id, Some(closed.event_id));
            let original_cause = retained_inputs[0].cause.unwrap();
            assert_eq!(work.cause.incarnation, original_cause.incarnation);
            assert!(work.cause.sequence > original_cause.sequence);
            assert!(retained_inputs[1].cause.unwrap().sequence > work.cause.sequence);
            assert_eq!(
                work.endpoint,
                MessagingSwitchboard::order_emulator_execute()
            );
            let control = control_after_stop.unwrap();
            assert_eq!(control.status(), OrderStatus::Emulated);
            assert_eq!(control.quantity(), Quantity::from("0.100"));
            assert_eq!(control.filled_qty(), Quantity::from("0.000"));
            assert!(control.commissions().is_empty());
            assert_eq!(
                control
                    .events()
                    .iter()
                    .map(|event| match event {
                        OrderEventAny::Initialized(_) => "Initialized",
                        OrderEventAny::Emulated(_) => "Emulated",
                        other => panic!("retained cancel changed control order {other:?}"),
                    })
                    .collect::<Vec<_>>(),
                ["Initialized", "Emulated"]
            );
            // The first complete observed construction is retained for re-read. Its UUID was
            // generated inside cancel_order, not independently observed before capture.
            assert_eq!(owner.retained_command_work().unwrap(), command_work);
            assert_eq!(owner.status().unwrap().unwrap(), owner_status);
            eprintln!(
                "signed flip retained original={command_work:?}; request={control_requests:?}; {}",
                signed_fixture.as_ref().unwrap().diagnostics()
            );
        } else {
            assert!(control_after_stop.is_none());
            assert!(control_requests.is_empty());
            assert!(command_work.is_empty());
        }
        for cached in &final_orders {
            assert_eq!(cached.status(), OrderStatus::Filled);
            assert_eq!(
                cached
                    .events()
                    .iter()
                    .filter(|event| matches!(event, OrderEventAny::Filled(_)))
                    .count(),
                1
            );
        }
    }

    #[rstest]
    #[case::base_quantity(false)]
    #[case::quote_quantity(true)]
    fn test_hosted_ordinary_callbacks_preserve_accounting_and_query_fifo(
        #[case] quote_quantity: bool,
    ) {
        use nautilus_model::python::orders::order_any_to_pyobject;
        use nautilus_trading::python::strategy::PyStrategy;

        fn order_identity(event: &OrderEventAny) -> (String, String, String) {
            let (kind, event_id) = match event {
                OrderEventAny::Initialized(event) => ("Initialized", event.event_id),
                OrderEventAny::Submitted(event) => ("Submitted", event.event_id),
                OrderEventAny::Updated(event) => ("Updated", event.event_id),
                OrderEventAny::Accepted(event) => ("Accepted", event.event_id),
                OrderEventAny::Filled(event) => ("Filled", event.event_id),
                _ => panic!("unexpected ordinary order event"),
            };
            (
                format!("Order{kind}"),
                event_id.to_string(),
                event.client_order_id().to_string(),
            )
        }

        struct OrdinaryTap {
            queries: HostedQueryTap,
            cache: Rc<RefCell<Cache>>,
            ingress: Rc<RefCell<Vec<(OrderEventAny, OrderStatus)>>>,
            published: Rc<RefCell<Vec<OrderEventAny>>>,
        }
        impl BusTap for OrdinaryTap {
            fn on_publish(&self, topic: MStr<Topic>, message: &dyn Any) {
                if let Some(event) = message.downcast_ref::<OrderEventAny>()
                    && topic.as_str() == format!("events.order.{}", event.strategy_id())
                {
                    self.published.borrow_mut().push(event.clone());
                }
            }
            fn on_send(&self, endpoint: MStr<Endpoint>, message: &dyn Any) {
                self.queries.on_send(endpoint, message);
                if endpoint == MessagingSwitchboard::exec_engine_process()
                    && let Some(event) = message.downcast_ref::<OrderEventAny>()
                {
                    let status = self
                        .cache
                        .borrow()
                        .order_owned(&event.client_order_id())
                        .unwrap()
                        .status();
                    self.ingress.borrow_mut().push((event.clone(), status));
                }
            }
        }
        Python::initialize();
        let trader_id = TraderId::from("CALLBACK-001");
        let account_id = AccountId::from("SIM-001");
        let client_id = ClientId::from("CALLBACK");
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
        let engine = node.kernel().exec_engine().clone();
        let clock = node.kernel().clock();
        let now = clock.borrow().timestamp_ns();
        let mut instrument = binary_option();
        instrument.id = InstrumentId::from("ORDINARY.SIM");
        instrument.activation_ns = UnixNanos::from(1);
        instrument.expiration_ns = UnixNanos::from(now.as_u64() + 86_400_000_000_000);
        instrument.maker_fee = Decimal::new(1, 3);
        instrument.taker_fee = Decimal::new(1, 3);
        let instrument = InstrumentAny::BinaryOption(instrument);
        let quote = QuoteTick::new(
            instrument.id(),
            Price::from("0.400"),
            Price::from("0.410"),
            Quantity::from("10000.00"),
            Quantity::from("10000.00"),
            now,
            now,
        );
        cache
            .borrow_mut()
            .add_instrument(instrument.clone())
            .unwrap();
        cache.borrow_mut().add_quote(quote).unwrap();
        let queries = Rc::new(RefCell::new(Vec::new()));
        let observer = Python::attach(|py| {
            let module = PyModule::new(py, "hosted_ordinary_observer").unwrap();
            module
                .setattr("Strategy", py.get_type::<PyStrategy>())
                .unwrap();
            module
                .setattr("client_id", client_id.into_py_any(py).unwrap())
                .unwrap();
            module
                .setattr(
                    "native_borrow_available",
                    pyo3::Py::new(
                        py,
                        OrdinaryCallbackBorrowProbe {
                            engine: engine.clone(),
                        },
                    )
                    .unwrap(),
                )
                .unwrap();
            let enqueue = PyCFunction::new_closure(
                py,
                None,
                None,
                move |args: &pyo3::Bound<'_, PyTuple>, _kwargs| -> pyo3::PyResult<()> {
                    let cause: UUID4 = args
                        .get_item(0)?
                        .extract::<String>()?
                        .parse()
                        .map_err(pyo3::exceptions::PyValueError::new_err)?;
                    let ts = args.get_item(1)?.extract::<u64>()?;
                    let mut query = QueryAccount::new(
                        trader_id,
                        Some(client_id),
                        account_id,
                        UUID4::new(),
                        UnixNanos::from(ts),
                        None,
                        None,
                    );
                    query.causation_id = Some(cause);
                    msgbus::send_trading_command(
                        MessagingSwitchboard::exec_engine_queue_execute(),
                        TradingCommand::QueryAccount(query),
                    );
                    Ok(())
                },
            )
            .unwrap();
            module.setattr("enqueue", enqueue).unwrap();
            py.run(c_str!(r#"
import asyncio
import sqlite3
import tempfile
from pathlib import Path

directory = tempfile.TemporaryDirectory()
database = Path(directory.name) / "ordinary.sqlite3"
with sqlite3.connect(database) as connection:
    connection.execute("CREATE TABLE observations (kind TEXT, event_id TEXT, identity TEXT, state TEXT)")
    connection.execute("CREATE TABLE callback_delivery (kind TEXT, event_id TEXT, identity TEXT, outlet TEXT, state TEXT, native_borrow_free INTEGER)")
seen = []
aggregate = []
callback_delivery = []
accounts = []
portfolio_snapshots = []
query_effects = []
errors = []
closed = False

def record(kind, event_id, identity, state):
    with sqlite3.connect(database, timeout=0) as connection:
        connection.execute("BEGIN IMMEDIATE")
        connection.execute("INSERT INTO observations VALUES (?, ?, ?, ?)", (kind, event_id, identity, state))
    seen.append((kind, event_id, identity, state))

def record_callback(outlet, event, identity, state):
    native_free = native_borrow_available()
    row = (type(event).__name__, str(event.event_id), identity, outlet, state, native_free)
    with sqlite3.connect(database, timeout=0) as connection:
        connection.execute("BEGIN IMMEDIATE")
        connection.execute("INSERT INTO callback_delivery VALUES (?, ?, ?, ?, ?, ?)", row)
    callback_delivery.append(row)

def persisted_callback_delivery():
    with sqlite3.connect(database, timeout=0) as connection:
        rows = connection.execute("SELECT kind, event_id, identity, outlet, state, native_borrow_free FROM callback_delivery ORDER BY rowid").fetchall()
    return [(kind, event_id, identity, outlet, state, bool(native_free))
            for kind, event_id, identity, outlet, state, native_free in rows]

def on_account(event_id, cached_event_id, balance, count, query_count):
    record("AccountState", event_id, cached_event_id, balance)
    accounts.append((event_id, cached_event_id, balance, count, query_count))

def on_portfolio_snapshot(snapshot_id, account_event_id, count):
    record("PortfolioSnapshot", snapshot_id, account_event_id, str(count))
    portfolio_snapshots.append((snapshot_id, account_event_id, count))

def on_query_completed(command_id, previous_id, observed_id, previous_count, observed_count, ts_event, balance):
    record("QueryCompleted", observed_id, command_id, balance)
    query_effects.append((command_id, previous_id, observed_id, previous_count, observed_count, ts_event, balance))

class OrdinaryStrategy(Strategy):
    def on_start(self):
        self.submit_order(self.orders[0], client_id=client_id)
    def on_stop(self): pass
    def on_dispose(self): pass
    def observe_order(self, event):
        try:
            cached = self.cache.order(event.client_order_id)
            record(type(event).__name__, str(event.event_id), str(event.client_order_id), str(cached.status))
            record_callback("specific", event, str(event.client_order_id), str(cached.status))
            if type(event).__name__ == "OrderSubmitted":
                enqueue(str(event.event_id), event.ts_init)
        except BaseException as error:
            errors.append(repr(error))
            raise
    def on_order_event(self, event):
        aggregate.append((type(event).__name__, str(event.event_id)))
        if type(event).__name__ in {"OrderSubmitted", "OrderUpdated", "OrderAccepted", "OrderFilled"}:
            try:
                cached = self.cache.order(event.client_order_id)
                record_callback("aggregate", event, str(event.client_order_id), str(cached.status))
            except BaseException as error:
                errors.append(repr(error))
                raise
    def on_order_submitted(self, event): self.observe_order(event)
    def on_order_accepted(self, event): self.observe_order(event)
    def on_order_filled(self, event): self.observe_order(event)
    def on_order_updated(self, event): self.observe_order(event)
    def observe_position(self, event, next_order=None):
        global closed
        try:
            cached = self.cache.position(event.position_id)
            record(type(event).__name__, str(event.event_id), str(event.position_id), str(cached.quantity))
            record_callback("specific", event, str(event.position_id), str(cached.quantity))
            enqueue(str(event.event_id), event.ts_init)
            if next_order is not None:
                self.submit_order(self.orders[next_order], client_id=client_id)
            else:
                closed = True
        except BaseException as error:
            errors.append(repr(error))
            raise
    def on_position_opened(self, event): self.observe_position(event, 1)
    def on_position_changed(self, event): self.observe_position(event, 2)
    def on_position_closed(self, event): self.observe_position(event)
    def on_position_event(self, event):
        try:
            cached = self.cache.position(event.position_id)
            record_callback("aggregate", event, str(event.position_id), str(cached.quantity))
        except BaseException as error:
            errors.append(repr(error))
            raise

strategy = OrdinaryStrategy()
def mark_waiting(): pass
def on_dispatch(command_id):
    record("QueryDispatch", command_id, command_id, "before actual client")
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
            ("ORDINARY-OPEN", OrderSide::Buy, "100.00", "0.410"),
            ("ORDINARY-ADD", OrderSide::Buy, "100.00", "0.410"),
            ("ORDINARY-CLOSE", OrderSide::Sell, "200.00", "0.400"),
        ]
        .into_iter()
        .map(|(id, side, quantity, price)| {
            OrderTestBuilder::new(OrderType::Limit)
                .trader_id(trader_id)
                .strategy_id(strategy_id)
                .instrument_id(instrument.id())
                .client_order_id(id.into())
                .side(side)
                .quantity(if quote_quantity {
                    let mut notional =
                        Quantity::from(quantity).as_decimal() * Price::from(price).as_decimal();
                    notional.rescale(2);
                    Quantity::from_decimal(notional).unwrap()
                } else {
                    Quantity::from(quantity)
                })
                .quote_quantity(quote_quantity)
                .price(Price::from(price))
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
        let (finish_tx, finish_rx) = oneshot::channel();
        finish_tx.send(()).unwrap();
        let client = HeldSandboxConnect {
            inner,
            finish: Some(finish_rx),
            phase: Rc::new(Cell::new(ConnectPhase::Before)),
            waiting_callback: Python::attach(|py| {
                observer.bind(py).getattr("mark_waiting").unwrap().unbind()
            }),
            queries: queries.clone(),
            stopped: stopped.clone(),
            completed_query_callback: Some(Python::attach(|py| {
                observer
                    .bind(py)
                    .getattr("on_query_completed")
                    .unwrap()
                    .unbind()
            })),
        };
        engine
            .borrow_mut()
            .register_client(Box::new(client))
            .unwrap();
        let queued = Rc::new(RefCell::new(Vec::new()));
        let dispatched = Rc::new(RefCell::new(Vec::new()));
        let ingress = Rc::new(RefCell::new(Vec::new()));
        let published = Rc::new(RefCell::new(Vec::new()));
        msgbus::set_bus_tap(Rc::new(OrdinaryTap {
            queries: HostedQueryTap {
                engine,
                queued: queued.clone(),
                dispatched: dispatched.clone(),
                callback: Python::attach(|py| {
                    observer.bind(py).getattr("on_dispatch").unwrap().unbind()
                }),
            },
            cache: cache.clone(),
            ingress: ingress.clone(),
            published: published.clone(),
        }));
        let account_records = Rc::new(RefCell::new(Vec::new()));
        let records = account_records.clone();
        let callback_cache = cache.clone();
        let query_records = queries.clone();
        let callback =
            Python::attach(|py| observer.bind(py).getattr("on_account").unwrap().unbind());
        let account_handler = TypedHandler::from(move |event: &AccountState| {
            let account = callback_cache
                .borrow()
                .account_owned(&event.account_id)
                .unwrap();
            let last = account.last_event().unwrap();
            let balance = account.balance_total(None).unwrap();
            let count = account.events().len();
            let query_count = query_records.borrow().len();
            records
                .borrow_mut()
                .push((event.clone(), last.event_id, count, query_count));
            Python::attach(|py| {
                callback
                    .call1(
                        py,
                        (
                            event.event_id.to_string(),
                            last.event_id.to_string(),
                            balance.to_string(),
                            count,
                            query_count,
                        ),
                    )
                    .unwrap()
            });
        });
        msgbus::subscribe_account_state("events.account.*".into(), account_handler.clone(), None);
        let snapshot_cache = cache.clone();
        let snapshot_queries = queries.clone();
        let snapshot_callback = Python::attach(|py| {
            observer
                .bind(py)
                .getattr("on_portfolio_snapshot")
                .unwrap()
                .unbind()
        });
        let snapshot_handler = TypedHandler::from(move |snapshot: &PortfolioSnapshot| {
            let account = snapshot_cache
                .borrow()
                .account_owned(&snapshot.account_id)
                .unwrap();
            Python::attach(|py| {
                snapshot_callback
                    .call1(
                        py,
                        (
                            snapshot.event_id.to_string(),
                            account.last_event().unwrap().event_id.to_string(),
                            snapshot_queries.borrow().len(),
                        ),
                    )
                    .unwrap()
            });
        });
        msgbus::subscribe_portfolio_snapshot(
            "events.portfolio.*".into(),
            snapshot_handler.clone(),
            None,
        );
        let (py_node, exercise_result) = Python::attach(|py| {
            let py_node = pyo3::Py::new(py, PyLiveNode::new(node)).unwrap();
            let globals = PyDict::new(py);
            globals.set_item("node", &py_node).unwrap();
            globals.set_item("observer", observer.bind(py)).unwrap();
            let exercise_result = py.run(
                c_str!(
                    r#"
import asyncio
async def exercise():
    task = asyncio.create_task(node.run_async())
    async def wait_for_effects():
        while not (observer.closed and len(observer.query_effects) == 6):
            if task.done():
                await task
                raise AssertionError("hosted node stopped before callbacks completed")
            if observer.errors:
                raise AssertionError(observer.errors)
            await asyncio.sleep(0)
    try:
        await asyncio.wait_for(wait_for_effects(), timeout=3)
    except BaseException:
        print("ordinary callbacks:", observer.seen, "accounts:", observer.accounts, "snapshots:", observer.portfolio_snapshots, "query effects:", observer.query_effects, "errors:", observer.errors, flush=True)
        raise
    finally:
        node.handle().stop()
        await asyncio.wait_for(task, timeout=3)
asyncio.run(exercise())
"#
                ),
                Some(&globals),
                None,
            );
            if exercise_result.is_err() {
                for order in &orders {
                    eprintln!(
                        "ordinary cached order {} events={:?}",
                        order.client_order_id(),
                        cache
                            .borrow()
                            .order_owned(&order.client_order_id())
                            .unwrap()
                            .events()
                    );
                }
                eprintln!("ordinary actual queries={:?}", queries.borrow());
                eprintln!("ordinary actual ingress={:?}", ingress.borrow());
                eprintln!("ordinary actual published={:?}", published.borrow());
                eprintln!(
                    "ordinary Python aggregate={:?}",
                    observer.bind(py).getattr("aggregate").unwrap()
                );
            }
            (py_node, exercise_result)
        });
        assert!(stopped.get() >= 1);
        assert_eq!(
            capture.capture().unwrap_err(),
            nautilus_common::clients::ExecutionAccountCaptureError::Retired
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
        msgbus::unsubscribe_portfolio_snapshot("events.portfolio.*".into(), &snapshot_handler);
        msgbus::unsubscribe_account_state("events.account.*".into(), &account_handler);
        msgbus::clear_bus_tap();
        for order in &orders {
            let kind = |event: &OrderEventAny| match event {
                OrderEventAny::Initialized(_) => "Initialized",
                OrderEventAny::Submitted(_) => "Submitted",
                OrderEventAny::Updated(_) => "Updated",
                OrderEventAny::Accepted(_) => "Accepted",
                OrderEventAny::Filled(_) => "Filled",
                _ => "Unexpected",
            };
            let raw: Vec<_> = ingress
                .borrow()
                .iter()
                .filter(|(event, _)| event.client_order_id() == order.client_order_id())
                .map(|(event, _)| kind(event))
                .collect();
            assert_eq!(
                raw,
                if quote_quantity {
                    vec!["Submitted", "Updated", "Accepted", "Filled"]
                } else {
                    vec!["Submitted", "Accepted", "Filled"]
                },
                "owner ingress must retain the actual event FIFO"
            );
            let outward: Vec<_> = published
                .borrow()
                .iter()
                .filter(|event| event.client_order_id() == order.client_order_id())
                .map(kind)
                .collect();
            assert_eq!(
                outward,
                if quote_quantity {
                    vec!["Initialized", "Submitted", "Updated", "Accepted", "Filled"]
                } else {
                    vec!["Initialized", "Submitted", "Accepted", "Filled"]
                },
                "normal owner apply must publish Submitted/Accepted before Filled; ingress and canonical cache are logged above"
            );
            let canonical: Vec<_> = cache
                .borrow()
                .order_owned(&order.client_order_id())
                .unwrap()
                .events()
                .iter()
                .map(|event| kind(event))
                .collect();
            assert_eq!(
                canonical,
                if quote_quantity {
                    vec!["Initialized", "Submitted", "Updated", "Accepted", "Filled"]
                } else {
                    vec!["Initialized", "Submitted", "Accepted", "Filled"]
                },
                "matching metadata must not replace the owner event prefix"
            );
            let raw_ids: Vec<_> = ingress
                .borrow()
                .iter()
                .filter(|(event, _)| event.client_order_id() == order.client_order_id())
                .map(|(event, _)| order_identity(event))
                .collect();
            let initial = OrderEventAny::Initialized(order.init_event().clone());
            let original_ids: Vec<_> = std::iter::once(order_identity(&initial))
                .chain(raw_ids)
                .collect();
            let published_ids: Vec<_> = published
                .borrow()
                .iter()
                .filter(|event| event.client_order_id() == order.client_order_id())
                .map(order_identity)
                .collect();
            let canonical_ids: Vec<_> = cache
                .borrow()
                .order_owned(&order.client_order_id())
                .unwrap()
                .events()
                .iter()
                .map(|event| order_identity(event))
                .collect();
            assert_eq!(
                published_ids, original_ids,
                "owner publication must preserve each original kind, UUID and CUID"
            );
            assert_eq!(
                canonical_ids, original_ids,
                "canonical history must preserve each original kind, UUID and CUID"
            );
        }
        exercise_result.unwrap();
        let seen: Vec<(String, String, String, String)> = Python::attach(|py| {
            assert!(
                observer
                    .bind(py)
                    .getattr("errors")
                    .unwrap()
                    .extract::<Vec<String>>()
                    .unwrap()
                    .is_empty()
            );
            observer
                .bind(py)
                .getattr("seen")
                .unwrap()
                .extract()
                .unwrap()
        });
        let aggregate: Vec<(String, String)> = Python::attach(|py| {
            observer
                .bind(py)
                .getattr("aggregate")
                .unwrap()
                .extract()
                .unwrap()
        });
        let expected_aggregate: Vec<_> = published
            .borrow()
            .iter()
            .filter(|event| !matches!(event, OrderEventAny::Initialized(_)))
            .map(|event| {
                let (kind, id, _) = order_identity(event);
                (kind, id)
            })
            .collect();
        assert_eq!(
            aggregate
                .into_iter()
                .filter(|(kind, _)| kind != "OrderInitialized")
                .collect::<Vec<_>>(),
            expected_aggregate,
            "Python aggregate callbacks must preserve published non-Initial type/UUID order"
        );
        for order in &orders {
            let expected_specific: Vec<_> = published
                .borrow()
                .iter()
                .filter(|event| {
                    event.client_order_id() == order.client_order_id()
                        && !matches!(event, OrderEventAny::Initialized(_))
                })
                .map(order_identity)
                .collect();
            let specific: Vec<_> = seen
                .iter()
                .filter(|(kind, _, id, _)| {
                    id == order.client_order_id().as_str() && kind.starts_with("Order")
                })
                .map(|(kind, event_id, id, _)| (kind.clone(), event_id.clone(), id.clone()))
                .collect();
            assert_eq!(
                specific, expected_specific,
                "Python specific callbacks must preserve each original kind, UUID and CUID"
            );
            let sequence: Vec<_> = seen
                .iter()
                .filter(|(kind, _, id, _)| {
                    id == order.client_order_id().as_str() && kind.starts_with("Order")
                })
                .map(|(kind, _, _, state)| (kind.as_str(), state.as_str()))
                .collect();
            assert_eq!(
                sequence,
                if quote_quantity {
                    vec![
                        ("OrderSubmitted", "SUBMITTED"),
                        ("OrderUpdated", "SUBMITTED"),
                        ("OrderAccepted", "ACCEPTED"),
                        ("OrderFilled", "FILLED"),
                    ]
                } else {
                    vec![
                        ("OrderSubmitted", "SUBMITTED"),
                        ("OrderAccepted", "ACCEPTED"),
                        ("OrderFilled", "FILLED"),
                    ]
                }
            );
            let cached = cache
                .borrow()
                .order_owned(&order.client_order_id())
                .unwrap();
            assert_eq!(cached.status(), OrderStatus::Filled);
            let expected_quantity = if order.order_side() == OrderSide::Sell {
                Quantity::from("200.00")
            } else {
                Quantity::from("100.00")
            };
            assert_eq!(cached.quantity(), expected_quantity);
            assert_eq!(cached.filled_qty(), expected_quantity);
            assert!(!cached.is_quote_quantity());
            assert_eq!(cached.events().len(), if quote_quantity { 5 } else { 4 });
            let history = cached.events();
            let accepted = history
                .iter()
                .find_map(|event| match event {
                    OrderEventAny::Accepted(event) => Some(event.venue_order_id),
                    _ => None,
                })
                .unwrap();
            let fill = history
                .iter()
                .find_map(|event| match event {
                    OrderEventAny::Filled(event) => Some(event),
                    _ => None,
                })
                .unwrap();
            assert_eq!(cached.venue_order_id(), Some(accepted));
            assert_eq!(fill.venue_order_id, accepted);
            assert_eq!(fill.last_qty, expected_quantity);
            assert_eq!(
                fill.commission,
                Some(if order.order_side() == OrderSide::Sell {
                    Money::from("0.08 USDC")
                } else {
                    Money::from("0.041 USDC")
                })
            );
        }
        let position_callbacks: Vec<_> = seen
            .iter()
            .filter(|(kind, _, _, _)| kind.starts_with("Position"))
            .map(|(kind, _, _, quantity)| (kind.as_str(), quantity.as_str()))
            .collect();
        assert_eq!(
            position_callbacks,
            [
                ("PositionOpened", "100.00"),
                ("PositionChanged", "200.00"),
                ("PositionClosed", "0.00")
            ]
        );
        let position_ids: std::collections::HashSet<_> = seen
            .iter()
            .filter(|(kind, _, _, _)| kind.starts_with("Position"))
            .map(|(_, _, id, _)| id)
            .collect();
        assert_eq!(position_ids.len(), 1);
        let position = cache
            .borrow()
            .position_owned(&PositionId::from(
                position_ids.into_iter().next().unwrap().as_str(),
            ))
            .unwrap();
        assert!(position.is_closed());
        assert_eq!(position.quantity, Quantity::zero(2));
        assert_eq!(position.realized_pnl, Some(Money::from("-2.162 USDC")));
        let account = cache.borrow().account_owned(&account_id).unwrap();
        assert_eq!(
            account.balance_total(None),
            Some(Money::from("99997.838 USDC"))
        );
        let fees: Decimal = orders
            .iter()
            .map(|order| {
                cache
                    .borrow()
                    .order_owned(&order.client_order_id())
                    .unwrap()
                    .events()
                    .into_iter()
                    .filter_map(|event| match event {
                        OrderEventAny::Filled(fill) => Some(fill.commission.unwrap().as_decimal()),
                        _ => None,
                    })
                    .sum::<Decimal>()
            })
            .sum();
        assert_eq!(fees, Decimal::new(162, 3));
        assert_eq!(queued.borrow().len(), 6);
        assert_eq!(*queued.borrow(), *dispatched.borrow());
        assert_eq!(queries.borrow().len(), 6);
        let causes: Vec<_> = seen
            .iter()
            .filter(|(kind, _, _, _)| kind == "OrderSubmitted" || kind.starts_with("Position"))
            .map(|(_, id, _, _)| id.parse::<UUID4>().unwrap())
            .collect();
        for (index, (query, phase)) in queries.borrow().iter().enumerate() {
            assert_eq!(*query, queued.borrow()[index]);
            assert_eq!(*phase, ConnectPhase::Returned);
            assert_eq!(query.causation_id, Some(causes[index]));
            assert!(query.ts_init >= now);
        }
        let events = account.events();
        let event_ids: std::collections::HashSet<_> =
            events.iter().map(|event| event.event_id).collect();
        assert_eq!(
            event_ids.len(),
            events.len(),
            "a mirror must not apply an original event twice"
        );
        for (event, last_id, _, _) in account_records.borrow().iter() {
            assert_eq!(
                event.event_id, *last_id,
                "account topic must mirror the current actual account event"
            );
            assert!(event_ids.contains(&event.event_id));
        }
        let effects: Vec<(String, String, String, usize, usize, u64, String)> =
            Python::attach(|py| {
                observer
                    .bind(py)
                    .getattr("query_effects")
                    .unwrap()
                    .extract()
                    .unwrap()
            });
        assert_eq!(effects.len(), 6);
        let mut query_event_ids = std::collections::HashSet::new();
        for (index, (command, previous, observed, before_count, after_count, ts_event, _)) in
            effects.iter().enumerate()
        {
            assert_eq!(*command, queries.borrow()[index].0.command_id.to_string());
            assert_ne!(
                observed, previous,
                "a legal new query must create a fresh account observation"
            );
            assert_eq!(*after_count, before_count + 1);
            let event_id = observed.parse::<UUID4>().unwrap();
            assert!(event_ids.contains(&event_id));
            assert!(query_event_ids.insert(event_id));
            assert!(*ts_event >= queries.borrow()[index].0.ts_init.as_u64());
        }
        assert_eq!(query_event_ids.len(), 6);
        Python::attach(|py| {
            let rows: usize = observer
                .bind(py)
                .getattr("sqlite3")
                .unwrap()
                .call_method1("connect", (observer.bind(py).getattr("database").unwrap(),))
                .unwrap()
                .call_method1("execute", ("SELECT COUNT(*) FROM observations",))
                .unwrap()
                .call_method0("fetchone")
                .unwrap()
                .get_item(0)
                .unwrap()
                .extract()
                .unwrap();
            assert_eq!(rows, seen.len());
            let delivered: Vec<(String, String, String, String, String, bool)> = observer
                .bind(py)
                .getattr("callback_delivery")
                .unwrap()
                .extract()
                .unwrap();
            let persisted: Vec<(String, String, String, String, String, bool)> = observer
                .bind(py)
                .call_method0("persisted_callback_delivery")
                .unwrap()
                .extract()
                .unwrap();
            assert_eq!(
                persisted, delivered,
                "each original callback must commit its own SQLite observation"
            );
            let identities: Vec<_> = delivered
                .iter()
                .map(|(kind, id, identity, outlet, state, _)| {
                    (
                        kind.clone(),
                        id.clone(),
                        identity.clone(),
                        outlet.clone(),
                        state.clone(),
                    )
                })
                .collect();
            let expected: Vec<_> = seen
                .iter()
                .filter(|(kind, _, _, _)| kind.starts_with("Order") || kind.starts_with("Position"))
                .flat_map(|(kind, id, identity, state)| {
                    ["specific", "aggregate"].map(|outlet| {
                        (
                            kind.clone(),
                            id.clone(),
                            identity.clone(),
                            outlet.to_owned(),
                            state.clone(),
                        )
                    })
                })
                .collect();
            assert!(!expected.is_empty());
            assert_eq!(
                identities, expected,
                "specific and aggregate must observe the original UUID, identity and canonical stage in pairs"
            );
            let held_borrows: Vec<_> = delivered
                .iter()
                .filter(|(_, _, _, _, _, available)| !available)
                .collect();
            py_node.call_method0(py, "dispose").unwrap();
            observer.bind(py).call_method0("cleanup").unwrap();
            assert!(
                held_borrows.is_empty(),
                "ordinary external callbacks must run after the actual execution-engine borrow ends: {held_borrows:?}"
            );
        });
    }

    #[rstest]
    #[case::post_only_rejected(false)]
    #[case::quote_driven_gtd_expired(true)]
    fn test_hosted_terminal_callbacks_preserve_stage_and_native_exit(#[case] expired: bool) {
        exercise_hosted_terminal_callbacks(expired, false);
    }

    #[rstest]
    #[case::rejected_specific(false)]
    #[case::expired_specific(true)]
    fn test_hosted_incoming_callback_failure_retains_actual_owner_and_originals(
        #[case] expired: bool,
    ) {
        exercise_hosted_terminal_callbacks(expired, true);
    }

    #[rstest]
    #[case::submitted_running("submitted", "running")]
    #[case::accepted_running("accepted", "running")]
    #[case::canceled_running("canceled", "running")]
    #[case::submitted_pending("submitted", "pending")]
    #[case::accepted_pending("accepted", "pending")]
    #[case::canceled_pending("canceled", "pending")]
    #[case::submitted_shutdown("submitted", "shutdown")]
    #[case::accepted_shutdown("accepted", "shutdown")]
    #[case::canceled_shutdown("canceled", "shutdown")]
    #[case::submitted_failure("submitted", "failure")]
    #[case::accepted_failure("accepted", "failure")]
    #[case::canceled_failure("canceled", "failure")]
    fn test_hosted_typed_batches_preserve_leg_stages_and_lifecycle(
        #[case] kind: &str,
        #[case] lifecycle: &str,
    ) {
        exercise_hosted_typed_batches(kind, lifecycle, None);
    }

    #[rstest]
    #[case::current_signed_factory("positive")]
    #[case::queued_new_desired_generation("generation")]
    #[case::queued_old_generation_paused("paused")]
    #[case::queued_old_generation_stopped("stopped")]
    #[case::queued_old_spec("spec")]
    #[ignore = "requires explicit CUSTOS_SOURCE_ROOT and compatible Custos PYO3_PYTHON"]
    fn test_cross_repo_incoming_child_uses_original_signed_control_factory(#[case] scenario: &str) {
        exercise_hosted_typed_batches("accepted", "running", Some(scenario));
    }

    fn exercise_hosted_typed_batches(kind: &str, lifecycle: &str, signed_scenario: Option<&str>) {
        use nautilus_common::{
            live::get_exec_event_sender,
            owner_delivery::{self, OwnerDeliveryError, OwnerIncomingOrder, OwnerIncomingStage},
        };
        use nautilus_model::events::{
            OrderAccepted, OrderAcceptedBatch, OrderCanceled, OrderCanceledBatch, OrderSubmitted,
            OrderSubmittedBatch,
        };
        use nautilus_trading::python::strategy::PyStrategy;

        type BatchRow = (String, String, String, String, String, String, String, bool);
        type QueryCompletion = (String, String, String, usize, usize, u64, String);

        struct BatchTap {
            queries: HostedQueryTap,
            raw: Rc<RefCell<Vec<OrderEventAny>>>,
            published: Rc<RefCell<Vec<OrderEventAny>>>,
            owner: Rc<RefCell<Option<owner_delivery::OwnerObserver>>>,
        }
        impl BusTap for BatchTap {
            fn on_publish(&self, topic: MStr<Topic>, message: &dyn Any) {
                if let Some(event) = message.downcast_ref::<OrderEventAny>()
                    && topic.as_str() == format!("events.order.{}", event.strategy_id())
                {
                    self.published.borrow_mut().push(event.clone());
                }
            }
            fn on_send(&self, endpoint: MStr<Endpoint>, message: &dyn Any) {
                self.queries.on_send(endpoint, message);
                if endpoint == MessagingSwitchboard::exec_engine_process()
                    && let Some(event) = message.downcast_ref::<OrderEventAny>()
                {
                    self.raw.borrow_mut().push(event.clone());
                    self.owner
                        .borrow_mut()
                        .get_or_insert_with(|| owner_delivery::current_observer().unwrap());
                }
            }
        }

        Python::initialize();
        let signed_fixture =
            signed_scenario.map(|_| super::hosted_control_cancel::CustosControlFixture::new(true));
        let trader_id = TraderId::from("BATCH-001");
        let account_id = AccountId::from("SIM-001");
        let client_id = ClientId::from("BATCH");
        let node = LiveNodeBuilder::from_config(LiveNodeConfig {
            trader_id,
            environment: Environment::Sandbox,
            timeout_connection: Duration::from_secs(3),
            timeout_disconnection: Duration::from_millis(500),
            delay_post_stop: Duration::from_millis(100),
            timeout_shutdown: Duration::from_millis(500),
            exec_engine: LiveExecutionEngineConfig {
                reconciliation: false,
                ..Default::default()
            },
            ..Default::default()
        })
        .unwrap()
        .with_logging(LoggerConfig {
            bypass_logging: true,
            ..Default::default()
        })
        .build()
        .unwrap();
        let cache = node.kernel().cache();
        let engine = node.kernel().exec_engine().clone();
        let clock = node.kernel().clock();
        let now = clock.borrow().timestamp_ns();
        let mut instrument = binary_option();
        instrument.id = InstrumentId::from("BATCH.SIM");
        instrument.activation_ns = 1.into();
        instrument.expiration_ns = (now.as_u64() + 86_400_000_000_000).into();
        let instrument = InstrumentAny::BinaryOption(instrument);
        cache
            .borrow_mut()
            .add_instrument(instrument.clone())
            .unwrap();
        cache
            .borrow_mut()
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
        let order_ids = ["P-A", "P-B", "R-A", "R-B", "L-C", "L-D"].map(ClientOrderId::from);
        let (finish, receive) = oneshot::channel();
        let release = Mutex::new(Some(finish));
        let observer = Python::attach(|py| {
            let module = PyModule::new(py, "hosted_batch_observer").unwrap();
            module
                .setattr("Strategy", py.get_type::<PyStrategy>())
                .unwrap();
            module.setattr("kind", kind).unwrap();
            module.setattr("lifecycle", lifecycle).unwrap();
            module.setattr("signed_scenario", signed_scenario).unwrap();
            if let Some(fixture) = &signed_fixture {
                module
                    .setattr("signed_fixture", fixture.module.bind(py))
                    .unwrap();
            } else {
                module.setattr("signed_fixture", py.None()).unwrap();
            }
            module
                .setattr("client_id", client_id.into_py_any(py).unwrap())
                .unwrap();
            module
                .setattr(
                    "control_id",
                    ClientOrderId::from("SIGNED-CHILD").into_py_any(py).unwrap(),
                )
                .unwrap();
            module
                .setattr("ids", order_ids.map(|id| id.to_string()).to_vec())
                .unwrap();
            module
                .setattr(
                    "native_borrow_available",
                    pyo3::Py::new(
                        py,
                        OrdinaryCallbackBorrowProbe {
                            engine: engine.clone(),
                        },
                    )
                    .unwrap(),
                )
                .unwrap();
            let release = PyCFunction::new_closure(
                py,
                None,
                None,
                move |_: &pyo3::Bound<'_, PyTuple>, _| -> pyo3::PyResult<()> {
                    release
                        .lock()
                        .unwrap()
                        .take()
                        .unwrap()
                        .send(())
                        .map_err(|()| {
                            pyo3::exceptions::PyRuntimeError::new_err("connect receiver closed")
                        })
                },
            )
            .unwrap();
            module.setattr("release_connect", release).unwrap();
            let enqueue = PyCFunction::new_closure(
                py,
                None,
                None,
                move |args: &pyo3::Bound<'_, PyTuple>, _| -> pyo3::PyResult<()> {
                    let cause = args
                        .get_item(0)?
                        .extract::<String>()?
                        .parse::<UUID4>()
                        .map_err(pyo3::exceptions::PyValueError::new_err)?;
                    let mut query = QueryAccount::new(
                        trader_id,
                        Some(client_id),
                        account_id,
                        UUID4::new(),
                        args.get_item(1)?.extract::<u64>()?.into(),
                        None,
                        None,
                    );
                    query.causation_id = Some(cause);
                    msgbus::send_trading_command(
                        MessagingSwitchboard::exec_engine_queue_execute(),
                        TradingCommand::QueryAccount(query),
                    );
                    Ok(())
                },
            )
            .unwrap();
            module.setattr("enqueue", enqueue).unwrap();
            py.run(c_str!(r#"
import asyncio
import sqlite3
import tempfile
from pathlib import Path

directory = tempfile.TemporaryDirectory()
database = signed_fixture.outbox_database if signed_fixture is not None else Path(directory.name) / "batch.sqlite3"
with sqlite3.connect(database) as connection:
    connection.execute("CREATE TABLE observations(kind TEXT, event_id TEXT, cuid TEXT, outlet TEXT, own TEXT, first TEXT, second TEXT, borrow_free INTEGER)")
records = []
errors = []
query_effects = []
dispatches = []
waiting = False
running = False
stopping = False
node_error = None
stop_status = None
child_rows = []
control_requests = []
control_at_start = None

class BatchStrategy(Strategy):
    def on_start(self):
        global running, control_at_start
        if signed_fixture is not None:
            # This is an inherited standalone emulator history, before binding the closed gate.
            self.submit_order(control_order, client_id=client_id)
            control_at_start = str(self.cache.order(control_id).status)
            signed_fixture.install_order_gate(self, signed_fixture.gate)
        running = True
    def on_stop(self):
        global stopping, stop_status
        stopping = True
        stop_status = self._control_delivery_status()
    def on_dispose(self): pass
    def observe(self, event, outlet):
        cuid = str(event.client_order_id)
        if cuid not in ids: return
        try:
            row = (type(event).__name__, str(event.event_id), cuid, outlet,
                str(self.cache.order(event.client_order_id).status),
                str(self.cache.order(order_ids[2]).status),
                str(self.cache.order(order_ids[3]).status), native_borrow_available())
            with sqlite3.connect(database, timeout=0) as connection:
                connection.execute("BEGIN IMMEDIATE")
                connection.execute("INSERT INTO observations VALUES (?, ?, ?, ?, ?, ?, ?, ?)", row)
            records.append(row)
            if outlet == "specific":
                enqueue(str(event.event_id), event.ts_init)
                if signed_fixture is not None and cuid == ids[2]:
                    self.cancel_order(control_id, client_id=client_id)
                    control_requests.append((str(event.event_id), str(self.cache.order(control_id).status)))
                    if signed_scenario != "positive":
                        desired_lifecycle = "running" if signed_scenario == "generation" else "stopped" if signed_scenario == "stopped" else "paused"
                        signed_fixture.commit_new_desired(desired_lifecycle, signed_scenario == "spec")
                if lifecycle == "failure" and cuid == ids[2]:
                    raise ValueError("batch callback failure:" + str(event.event_id))
        except BaseException as error:
            errors.append(repr(error))
            raise
    def on_order_submitted(self, event): self.observe(event, "specific")
    def on_order_accepted(self, event): self.observe(event, "specific")
    def child(self, event, outlet):
        row = (str(event.event_id), str(event.client_order_id), outlet,
            str(self.cache.order(control_id).status),
            str(self.cache.order(order_ids[2]).status),
            str(self.cache.order(order_ids[3]).status), native_borrow_available())
        assert row[3:] == ("CANCELED", "ACCEPTED", "SUBMITTED", True), row
        signed_fixture.record_cancellation(event)
        child_rows.append(row)
    def on_order_canceled(self, event):
        if signed_fixture is not None and event.client_order_id == control_id:
            self.child(event, "specific")
        else:
            self.observe(event, "specific")
    def on_order_event(self, event):
        if signed_fixture is not None and event.client_order_id == control_id:
            if type(event).__name__ == "OrderCanceled": self.child(event, "aggregate")
        elif type(event).__name__ != "OrderInitialized":
            self.observe(event, "aggregate")

strategy = BatchStrategy()
def mark_waiting():
    global waiting
    waiting = True
def on_dispatch(command_id): dispatches.append(command_id)
def on_query_completed(*row): query_effects.append(row)
def persisted():
    with sqlite3.connect(database, timeout=0) as connection:
        return [(*row[:-1], bool(row[-1])) for row in connection.execute("SELECT * FROM observations ORDER BY rowid")]
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
        let mut inputs = Vec::new();
        let mut prefixes = Vec::new();
        for (index, order_id) in order_ids.iter().enumerate() {
            let venue_id = VenueOrderId::from(format!("V-{index}"));
            let submitted = OrderSubmitted::new(
                trader_id,
                strategy_id,
                instrument.id(),
                *order_id,
                account_id,
                UUID4::new(),
                now,
                now,
            );
            let accepted = OrderAccepted::new(
                trader_id,
                strategy_id,
                instrument.id(),
                *order_id,
                venue_id,
                account_id,
                UUID4::new(),
                now,
                now,
                false,
            );
            let canceled = OrderCanceled::new(
                trader_id,
                strategy_id,
                instrument.id(),
                *order_id,
                UUID4::new(),
                now,
                now,
                false,
                Some(venue_id),
                Some(account_id),
                None,
            );
            let mut order = OrderTestBuilder::new(OrderType::Limit)
                .trader_id(trader_id)
                .strategy_id(strategy_id)
                .instrument_id(instrument.id())
                .client_order_id(*order_id)
                .side(OrderSide::Buy)
                .quantity(Quantity::from("100.00"))
                .price(Price::from("0.390"))
                .build();
            if kind != "submitted" {
                order.apply(OrderEventAny::Submitted(submitted)).unwrap();
            }
            if kind == "canceled" {
                order.apply(OrderEventAny::Accepted(accepted)).unwrap();
            }
            inputs.push(match kind {
                "submitted" => OrderEventAny::Submitted(submitted),
                "accepted" => OrderEventAny::Accepted(accepted),
                "canceled" => OrderEventAny::Canceled(canceled),
                _ => unreachable!(),
            });
            prefixes.push(order.events().into_iter().cloned().collect::<Vec<_>>());
            cache
                .borrow_mut()
                .add_order(order, None, Some(client_id), false)
                .unwrap();
        }
        if signed_fixture.is_some() {
            use nautilus_model::{enums::TriggerType, python::orders::order_any_to_pyobject};
            let order = OrderTestBuilder::new(OrderType::StopLimit)
                .trader_id(trader_id)
                .strategy_id(strategy_id)
                .instrument_id(instrument.id())
                .client_order_id(ClientOrderId::from("SIGNED-CHILD"))
                .side(OrderSide::Buy)
                .quantity(Quantity::from("100.00"))
                .price(Price::from("0.620"))
                .trigger_price(Price::from("0.600"))
                .trigger_type(TriggerType::BidAsk)
                .emulation_trigger(TriggerType::BidAsk)
                .build();
            Python::attach(|py| {
                observer
                    .bind(py)
                    .setattr("control_order", order_any_to_pyobject(py, order).unwrap())
                    .unwrap();
            });
        }
        let batches: Vec<_> = inputs
            .as_chunks::<2>()
            .0
            .iter()
            .map(|events| {
                Some(match kind {
                    "submitted" => ExecutionEvent::OrderSubmittedBatch(OrderSubmittedBatch::new(
                        events
                            .iter()
                            .map(|event| {
                                let OrderEventAny::Submitted(event) = event else {
                                    unreachable!()
                                };
                                *event
                            })
                            .collect(),
                    )),
                    "accepted" => ExecutionEvent::OrderAcceptedBatch(OrderAcceptedBatch::new(
                        events
                            .iter()
                            .map(|event| {
                                let OrderEventAny::Accepted(event) = event else {
                                    unreachable!()
                                };
                                *event
                            })
                            .collect(),
                    )),
                    "canceled" => ExecutionEvent::OrderCanceledBatch(OrderCanceledBatch::new(
                        events
                            .iter()
                            .map(|event| {
                                let OrderEventAny::Canceled(event) = event else {
                                    unreachable!()
                                };
                                *event
                            })
                            .collect(),
                    )),
                    _ => unreachable!(),
                })
            })
            .collect();
        let batches = Mutex::new(batches);
        let sender = get_exec_event_sender();
        Python::attach(|py| {
            let send = PyCFunction::new_closure(
                py,
                None,
                None,
                move |args: &pyo3::Bound<'_, PyTuple>, _| -> pyo3::PyResult<()> {
                    let index = args.get_item(0)?.extract::<usize>()?;
                    sender
                        .send(batches.lock().unwrap()[index].take().unwrap())
                        .map_err(|error| {
                            pyo3::exceptions::PyRuntimeError::new_err(error.to_string())
                        })
                },
            )
            .unwrap();
            observer.bind(py).setattr("send_batch", send).unwrap();
            observer
                .bind(py)
                .setattr(
                    "order_ids",
                    order_ids
                        .into_iter()
                        .map(|id| id.into_py_any(py).unwrap())
                        .collect::<Vec<_>>(),
                )
                .unwrap();
            observer
                .bind(py)
                .setattr(
                    "predecessor",
                    match kind {
                        "submitted" => "INITIALIZED",
                        "accepted" => "SUBMITTED",
                        "canceled" => "ACCEPTED",
                        _ => unreachable!(),
                    },
                )
                .unwrap();
            observer
                .bind(py)
                .setattr(
                    "target",
                    match kind {
                        "submitted" => "SUBMITTED",
                        "accepted" => "ACCEPTED",
                        "canceled" => "CANCELED",
                        _ => unreachable!(),
                    },
                )
                .unwrap();
        });
        let config = SandboxExecutionClientConfig {
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
            config.venue,
            config.oms_type,
            account_id,
            config.account_type,
            config.base_currency,
            cache.clone(),
        );
        let inner = SandboxExecutionClient::new(core, config, clock, cache.clone());
        let capture = inner.account_capture_probe().unwrap();
        let queries = Rc::new(RefCell::new(Vec::new()));
        let stopped = Rc::new(Cell::new(0));
        engine
            .borrow_mut()
            .register_client(Box::new(HeldSandboxConnect {
                inner,
                finish: Some(receive),
                phase: Rc::new(Cell::new(ConnectPhase::Before)),
                waiting_callback: Python::attach(|py| {
                    observer.bind(py).getattr("mark_waiting").unwrap().unbind()
                }),
                queries: queries.clone(),
                stopped: stopped.clone(),
                completed_query_callback: Some(Python::attach(|py| {
                    observer
                        .bind(py)
                        .getattr("on_query_completed")
                        .unwrap()
                        .unbind()
                })),
            }))
            .unwrap();
        let queued = Rc::new(RefCell::new(Vec::new()));
        let dispatched = Rc::new(RefCell::new(Vec::new()));
        let raw = Rc::new(RefCell::new(Vec::new()));
        let published = Rc::new(RefCell::new(Vec::new()));
        let observed_owner = Rc::new(RefCell::new(None));
        msgbus::set_bus_tap(Rc::new(BatchTap {
            queries: HostedQueryTap {
                engine,
                queued: queued.clone(),
                dispatched: dispatched.clone(),
                callback: Python::attach(|py| {
                    observer.bind(py).getattr("on_dispatch").unwrap().unbind()
                }),
            },
            raw: raw.clone(),
            published: published.clone(),
            owner: observed_owner.clone(),
        }));
        let (py_node, result) = Python::attach(|py| {
            let node = pyo3::Py::new(py, PyLiveNode::new(node)).unwrap();
            let globals = PyDict::new(py);
            globals.set_item("node", &node).unwrap();
            globals.set_item("observer", observer.bind(py)).unwrap();
            let result = py.run(c_str!(r#"
import asyncio
async def exercise():
    task = asyncio.create_task(node.run_async())
    async def wait_for(predicate):
        while not predicate():
            if task.done():
                await task
                raise AssertionError("node stopped before batch checkpoint")
            await asyncio.sleep(0)
    try:
        await asyncio.wait_for(wait_for(lambda: observer.waiting), timeout=3)
        if observer.lifecycle == "pending":
            observer.send_batch(0)
            # Real connect remains held; no canonical or external processing is possible yet.
            await asyncio.sleep(0.02)
            assert not observer.running and not observer.records
            for order_id in observer.order_ids[:2]:
                assert str(observer.strategy.cache.order(order_id).status) == observer.predecessor
        observer.release_connect()
        await asyncio.wait_for(wait_for(lambda: observer.running), timeout=3)
        if observer.lifecycle == "pending":
            for order_id in observer.order_ids[:2]:
                assert str(observer.strategy.cache.order(order_id).status) == observer.target
            assert not observer.records
        observer.send_batch(1)
        if observer.lifecycle == "failure" or (observer.signed_scenario is not None and observer.signed_scenario != "positive"):
            await asyncio.wait_for(wait_for(lambda: observer.stopping), timeout=3)
            if observer.lifecycle == "failure": observer.send_batch(2)
            try:
                await asyncio.wait_for(task, timeout=3)
            except TimeoutError:
                raise
            except Exception as error:
                observer.node_error = repr(error)
            else:
                raise AssertionError("normal node swallowed batch callback failure")
        else:
            await asyncio.wait_for(wait_for(lambda: len(observer.records) == 4 and len(observer.query_effects) == 2), timeout=3)
            if observer.signed_scenario == "positive": assert len(observer.child_rows) == 2, observer.child_rows
            node.handle().stop()
            if observer.lifecycle == "shutdown":
                await asyncio.wait_for(wait_for(lambda: observer.stopping), timeout=3)
                observer.send_batch(2)
            await asyncio.wait_for(task, timeout=3)
    finally:
        node.handle().stop()
        if not task.done(): await asyncio.wait_for(task, timeout=3)
asyncio.run(exercise())
"#), Some(&globals), None);
            (node, result)
        });
        msgbus::clear_bus_tap();
        let final_orders: Vec<_> = order_ids
            .iter()
            .map(|id| cache.borrow().order_owned(id).unwrap())
            .collect();
        let final_account = cache.borrow().account_owned(&account_id).unwrap();
        let final_control = cache
            .borrow()
            .order_owned(&ClientOrderId::from("SIGNED-CHILD"));
        let final_positions = (
            cache
                .borrow()
                .positions_open(None, None, None, None, None)
                .len(),
            cache
                .borrow()
                .positions_closed(None, None, None, None, None)
                .len(),
        );
        let owner = observed_owner.borrow().clone().unwrap();
        let owner_status = owner.status().unwrap().unwrap();
        let retained = owner.retained_incoming_orders().unwrap();
        let signed_observations = Python::attach(|py| {
            let module = observer.bind(py);
            let child_rows: Vec<(String, String, String, String, String, String, bool)> =
                module.getattr("child_rows").unwrap().extract().unwrap();
            let requests: Vec<(String, String)> = module
                .getattr("control_requests")
                .unwrap()
                .extract()
                .unwrap();
            let started: Option<String> = module
                .getattr("control_at_start")
                .unwrap()
                .extract()
                .unwrap();
            (child_rows, requests, started)
        });
        let (rows, persisted, errors, query_effects, node_error, state) = Python::attach(|py| {
            let module = observer.bind(py);
            let rows: Vec<BatchRow> = module.getattr("records").unwrap().extract().unwrap();
            let persisted: Vec<BatchRow> =
                module.call_method0("persisted").unwrap().extract().unwrap();
            let errors: Vec<String> = module.getattr("errors").unwrap().extract().unwrap();
            let query_effects: Vec<QueryCompletion> =
                module.getattr("query_effects").unwrap().extract().unwrap();
            let node_error: Option<String> =
                module.getattr("node_error").unwrap().extract().unwrap();
            let state: NodeState = py_node
                .call_method0(py, "handle")
                .unwrap()
                .getattr(py, "state")
                .unwrap()
                .extract(py)
                .unwrap();
            py_node.call_method0(py, "dispose").unwrap();
            module.call_method0("cleanup").unwrap();
            (rows, persisted, errors, query_effects, node_error, state)
        });
        assert!(
            result.is_ok(),
            "batch {kind}/{lifecycle}: {result:?}; raw={:?}; rows={rows:?}; errors={errors:?}; retained={retained:?}",
            raw.borrow()
        );
        assert_eq!(state, NodeState::Stopped);
        assert!(stopped.get() > 0);
        assert_eq!(
            capture.capture().unwrap_err(),
            nautilus_common::clients::ExecutionAccountCaptureError::Retired
        );
        let predecessor = match kind {
            "submitted" => OrderStatus::Initialized,
            "accepted" => OrderStatus::Submitted,
            "canceled" => OrderStatus::Accepted,
            _ => unreachable!(),
        };
        let target = match kind {
            "submitted" => OrderStatus::Submitted,
            "accepted" => OrderStatus::Accepted,
            "canceled" => OrderStatus::Canceled,
            _ => unreachable!(),
        };
        let callback_failure = lifecycle == "failure";
        let signed_retired = signed_scenario.is_some_and(|scenario| scenario != "positive");
        let failure = callback_failure || signed_retired;
        let applied: Vec<_> = (0..6)
            .filter(|index| match index {
                0 | 1 => lifecycle == "pending",
                2 => true,
                3 => !failure,
                4 | 5 => lifecycle == "shutdown",
                _ => unreachable!(),
            })
            .collect();
        let expected_raw: Vec<_> = applied.iter().map(|index| inputs[*index].clone()).collect();
        let is_batch_order = |event: &&OrderEventAny| order_ids.contains(&event.client_order_id());
        assert_eq!(
            raw.borrow()
                .iter()
                .filter(is_batch_order)
                .cloned()
                .collect::<Vec<_>>(),
            expected_raw
        );
        assert_eq!(
            published
                .borrow()
                .iter()
                .filter(is_batch_order)
                .cloned()
                .collect::<Vec<_>>(),
            expected_raw
        );
        assert_eq!(rows, persisted);
        let mut expected_rows = Vec::new();
        for index in 2..if failure { 3 } else { 4 } {
            let event = inputs[index].clone().into_boxed();
            for outlet in ["specific", "aggregate"] {
                expected_rows.push((
                    event.type_name().to_owned(),
                    event.id().to_string(),
                    order_ids[index].to_string(),
                    outlet.to_owned(),
                    target.to_string(),
                    target.to_string(),
                    if index == 2 { predecessor } else { target }.to_string(),
                    true,
                ));
            }
        }
        assert_eq!(rows, expected_rows);
        for (index, order) in final_orders.iter().enumerate() {
            let is_applied = applied.contains(&index);
            assert_eq!(
                order.status(),
                if is_applied { target } else { predecessor }
            );
            let mut history = prefixes[index].clone();
            if is_applied {
                history.push(inputs[index].clone());
            }
            assert_eq!(
                order.events().into_iter().cloned().collect::<Vec<_>>(),
                history
            );
            assert_eq!(order.quantity(), Quantity::from("100.00"));
            assert_eq!(order.filled_qty(), Quantity::from("0.00"));
            assert!(order.commissions().is_empty());
        }
        assert_eq!(
            final_account.balance_total(None),
            Some(Money::from("100000 USDC"))
        );
        assert_eq!(final_positions, (0, 0));
        if callback_failure {
            assert_eq!(
                errors,
                vec![format!(
                    "ValueError('batch callback failure:{}')",
                    inputs[2].clone().into_boxed().id()
                )]
            );
            assert!(node_error.unwrap().contains("ExternalCallback"));
            assert_eq!(owner_status.1, Some(OwnerDeliveryError::ExternalCallback));
            assert!(owner_status.0 >= 5);
            assert_eq!(retained.len(), 4);
            let mut previous_cause = None;
            for (retained, index) in retained.iter().zip([2, 3, 4, 5]) {
                let OwnerIncomingOrder::Process(original) = &retained.event else {
                    panic!("original typed batch leg required")
                };
                assert_eq!(original, &inputs[index]);
                let cause = retained.cause.expect("original admitted incoming identity");
                assert_eq!(cause.event_id, original.clone().into_boxed().id());
                assert_ne!(
                    cause.incarnation.to_string(),
                    "00000000-0000-0000-0000-000000000000"
                );
                if let Some(previous) = previous_cause {
                    let previous: owner_delivery::OwnerIncomingCause = previous;
                    assert_eq!(cause.incarnation, previous.incarnation);
                    assert!(cause.sequence > previous.sequence);
                }
                previous_cause = Some(cause);
                assert_eq!(
                    retained.stage,
                    if index == 2 {
                        OwnerIncomingStage::DispatchReturned
                    } else {
                        OwnerIncomingStage::NotDispatched
                    }
                );
            }
            assert_eq!(queued.borrow().len(), 1);
            assert!(dispatched.borrow().is_empty());
            assert!(queries.borrow().is_empty());
            assert!(query_effects.is_empty());
        } else if signed_retired {
            assert!(errors.is_empty());
            assert!(node_error.unwrap().contains("ControlEnter"));
            assert_eq!(owner_status.1, Some(OwnerDeliveryError::ControlEnter));
            assert!(owner_status.0 > 0);
            assert_eq!(retained.len(), 1);
            let OwnerIncomingOrder::Process(original) = &retained[0].event else {
                panic!("original unprocessed batch leg required")
            };
            assert_eq!(original, &inputs[3]);
            assert_eq!(retained[0].stage, OwnerIncomingStage::NotDispatched);
            assert_eq!(retained[0].error, OwnerDeliveryError::ControlEnter);
            assert_eq!(queued.borrow().len(), 1);
            assert!(dispatched.borrow().is_empty());
            assert!(queries.borrow().is_empty());
            assert!(query_effects.is_empty());
        } else {
            assert!(errors.is_empty());
            assert!(node_error.is_none());
            assert_eq!(owner_status, (0, None));
            assert!(retained.is_empty());
            assert_eq!(*queued.borrow(), *dispatched.borrow());
            assert_eq!(queries.borrow().len(), 2);
            assert_eq!(query_effects.len(), 2);
            for (index, ((query, phase), effect)) in
                queries.borrow().iter().zip(query_effects).enumerate()
            {
                assert_eq!(query, &queued.borrow()[index]);
                assert_eq!(*phase, ConnectPhase::Returned);
                assert_eq!(
                    query.causation_id,
                    Some(inputs[index + 2].clone().into_boxed().id())
                );
                assert_eq!(effect.0, query.command_id.to_string());
                assert_ne!(effect.1, effect.2);
                assert_eq!(effect.4, effect.3 + 1);
                assert_eq!(effect.6, Money::from("100000 USDC").to_string());
                assert!(
                    final_account
                        .events()
                        .iter()
                        .any(|event| event.event_id.to_string() == effect.2)
                );
            }
        }
        if let Some(fixture) = &signed_fixture {
            let (child_rows, requests, started) = signed_observations;
            eprintln!(
                "incoming signed {signed_scenario:?}: {}; child_rows={child_rows:?}; requests={requests:?}; owner={owner_status:?}; retained={retained:?}",
                fixture.diagnostics()
            );
            assert_eq!(started.as_deref(), Some("EMULATED"));
            assert_eq!(
                requests,
                vec![(
                    inputs[2].clone().into_boxed().id().to_string(),
                    "EMULATED".to_owned()
                )]
            );
            let control = final_control.unwrap();
            assert_eq!(
                control.status(),
                if signed_retired {
                    OrderStatus::Emulated
                } else {
                    OrderStatus::Canceled
                }
            );
            assert_eq!(control.filled_qty(), Quantity::from("0.00"));
            assert!(control.commissions().is_empty());
            // The emulator's direct native source differs from the batch's Async ingress.
            // Pair its typed publication with the actual canonical history and user callbacks.
            let cancellations: Vec<_> = published.borrow().iter().filter(|event| matches!(event, OrderEventAny::Canceled(event) if event.client_order_id == ClientOrderId::from("SIGNED-CHILD"))).cloned().collect();
            if signed_retired {
                assert!(child_rows.is_empty());
                assert!(cancellations.is_empty());
            } else {
                assert_eq!(cancellations.len(), 1);
                let cancel = cancellations[0].clone().into_boxed();
                assert_eq!(
                    child_rows,
                    ["specific", "aggregate"]
                        .map(|outlet| (
                            cancel.id().to_string(),
                            "SIGNED-CHILD".to_owned(),
                            outlet.to_owned(),
                            "CANCELED".to_owned(),
                            "ACCEPTED".to_owned(),
                            "SUBMITTED".to_owned(),
                            true,
                        ))
                        .to_vec()
                );
                let batch_trace: Vec<_> = published
                    .borrow()
                    .iter()
                    .filter(|event| {
                        event.client_order_id() == order_ids[2]
                            || event.client_order_id() == order_ids[3]
                            || matches!(event, OrderEventAny::Canceled(_))
                    })
                    .cloned()
                    .collect();
                assert_eq!(
                    batch_trace,
                    vec![
                        inputs[2].clone(),
                        cancellations[0].clone(),
                        inputs[3].clone()
                    ]
                );
                assert_eq!(
                    control
                        .events()
                        .into_iter()
                        .filter(|event| matches!(event, OrderEventAny::Canceled(_)))
                        .cloned()
                        .collect::<Vec<_>>(),
                    cancellations
                );
            }
            Python::attach(|py| {
                let scope = PyDict::new(py);
                scope.set_item("fixture", fixture.module.bind(py)).unwrap();
                scope.set_item("retired", signed_retired).unwrap();
                py.run(c_str!(r#"
assert fixture.outbox_identity == fixture.observer_identity
assert fixture.retirement_errors == []
assert not fixture.refusals
assert fixture.source_audit()
if retired:
    assert len(fixture.desired_commits) == 1
    assert fixture.desired_commits[0][1] == fixture.verified.command.generation + 1
    assert fixture.boundary.platform_opening_gate.authority.generation == fixture.verified.command.generation
else:
    assert fixture.desired_commits == []
    assert len(fixture.callback_rows) == 2
    assert fixture.callback_rows[0] == fixture.callback_rows[1]
    with fixture.sqlite3.connect(fixture.observer_database, timeout=0) as connection:
        assert connection.execute("SELECT event_id, client_order_id FROM callback_observations ORDER BY rowid").fetchall() == fixture.callback_rows
    assert fixture.store.load_opening_gate_state_sync(fixture.boundary.platform_opening_gate.authority) == "awaiting_release"
    try:
        fixture.boundary.platform_opening_gate.assert_released()
    except fixture.RunnerOpeningGateClosed as error:
        assert error.reason_code == "awaiting_opening_release"
    else:
        raise AssertionError("engineering cancel must not release the actual opening gate")
"#), Some(&scope), None).unwrap();
            });
        }
    }

    fn exercise_hosted_terminal_callbacks(expired: bool, callback_failure: bool) {
        use nautilus_common::{
            live::get_data_event_sender,
            messages::DataEvent,
            owner_delivery::{
                self, OwnerDeliveryCause, OwnerDeliveryError, OwnerExternalEvent,
                OwnerIncomingOrder, OwnerIncomingStage, OwnerObserver,
            },
        };
        use nautilus_trading::python::strategy::PyStrategy;

        type DeliveryRow = (String, String, String, String, String, bool);
        type QueryCompletion = (String, String, String, usize, usize, u64, String);
        type DeliveryStatus = (bool, Option<(usize, Option<String>)>);

        fn identity(event: &OrderEventAny) -> (String, String, String) {
            let event = event.clone().into_boxed();
            (
                event.type_name().to_owned(),
                event.id().to_string(),
                event.client_order_id().to_string(),
            )
        }

        struct TerminalTap {
            queries: HostedQueryTap,
            cache: Rc<RefCell<Cache>>,
            raw: Rc<RefCell<Vec<OrderEventAny>>>,
            outward: Rc<RefCell<Vec<(OrderEventAny, OrderStatus)>>>,
            quotes: Rc<RefCell<Vec<QuoteTick>>>,
            owner: Rc<RefCell<Option<OwnerObserver>>>,
        }
        impl BusTap for TerminalTap {
            fn on_publish(&self, topic: MStr<Topic>, message: &dyn Any) {
                if let Some(event) = message.downcast_ref::<OrderEventAny>()
                    && topic.as_str() == format!("events.order.{}", event.strategy_id())
                    && !matches!(event, OrderEventAny::Initialized(_))
                {
                    let cached = self
                        .cache
                        .borrow()
                        .order_owned(&event.client_order_id())
                        .unwrap();
                    self.outward
                        .borrow_mut()
                        .push((event.clone(), cached.status()));
                }
            }
            fn on_send(&self, endpoint: MStr<Endpoint>, message: &dyn Any) {
                if let Some(observer) = owner_delivery::current_observer() {
                    self.owner.borrow_mut().get_or_insert(observer);
                }
                self.queries.on_send(endpoint, message);
                if endpoint == MessagingSwitchboard::exec_engine_process()
                    && let Some(event) = message.downcast_ref::<OrderEventAny>()
                {
                    self.raw.borrow_mut().push(event.clone());
                }
                if endpoint == MessagingSwitchboard::data_engine_process_data()
                    && let Some(Data::Quote(quote)) = message.downcast_ref::<Data>()
                {
                    self.quotes.borrow_mut().push(*quote);
                }
            }
        }

        Python::initialize();
        let trader_id = TraderId::from("TERMINAL-001");
        let account_id = AccountId::from("SIM-001");
        let client_id = ClientId::from("TERMINAL");
        let order_id = ClientOrderId::from("TERMINAL-ORDER");
        let node = LiveNodeBuilder::from_config(LiveNodeConfig {
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
        })
        .unwrap()
        .with_logging(LoggerConfig {
            bypass_logging: true,
            ..Default::default()
        })
        .build()
        .unwrap();
        let cache = node.kernel().cache();
        let engine = node.kernel().exec_engine().clone();
        let clock = node.kernel().clock();
        let now = clock.borrow().timestamp_ns();
        let mut instrument = binary_option();
        instrument.id = InstrumentId::from("TERMINAL.SIM");
        instrument.activation_ns = 1.into();
        instrument.expiration_ns = (now.as_u64() + 86_400_000_000_000).into();
        let instrument = InstrumentAny::BinaryOption(instrument);
        let quote = QuoteTick::new(
            instrument.id(),
            Price::from("0.400"),
            Price::from("0.410"),
            Quantity::from("10000.00"),
            Quantity::from("10000.00"),
            now,
            now,
        );
        cache
            .borrow_mut()
            .add_instrument(instrument.clone())
            .unwrap();
        cache.borrow_mut().add_quote(quote).unwrap();
        let data_sender = get_data_event_sender();
        let observer = Python::attach(|py| {
            let module = PyModule::new(py, "hosted_terminal_observer").unwrap();
            module
                .setattr("Strategy", py.get_type::<PyStrategy>())
                .unwrap();
            module
                .setattr("client_id", client_id.into_py_any(py).unwrap())
                .unwrap();
            module
                .setattr("order_id", order_id.into_py_any(py).unwrap())
                .unwrap();
            module
                .setattr("instrument_id", instrument.id().into_py_any(py).unwrap())
                .unwrap();
            module
                .setattr("side", OrderSide::Buy.into_py_any(py).unwrap())
                .unwrap();
            module
                .setattr(
                    "quantity",
                    Quantity::from("100.00").into_py_any(py).unwrap(),
                )
                .unwrap();
            module
                .setattr(
                    "price",
                    Price::from(if expired { "0.390" } else { "0.420" })
                        .into_py_any(py)
                        .unwrap(),
                )
                .unwrap();
            module
                .setattr(
                    "tif",
                    (if expired {
                        TimeInForce::Gtd
                    } else {
                        TimeInForce::Gtc
                    })
                    .into_py_any(py)
                    .unwrap(),
                )
                .unwrap();
            module.setattr("expired_case", expired).unwrap();
            module
                .setattr("callback_failure", callback_failure)
                .unwrap();
            module
                .setattr(
                    "native_borrow_available",
                    pyo3::Py::new(
                        py,
                        OrdinaryCallbackBorrowProbe {
                            engine: engine.clone(),
                        },
                    )
                    .unwrap(),
                )
                .unwrap();
            let send_quote = PyCFunction::new_closure(
                py,
                None,
                None,
                move |args: &pyo3::Bound<'_, PyTuple>, _| -> pyo3::PyResult<()> {
                    let timestamp = args.get_item(0)?.extract::<u64>()?;
                    let mut current = quote;
                    current.ts_event = timestamp.into();
                    current.ts_init = timestamp.into();
                    data_sender
                        .send(DataEvent::Data(Data::Quote(current)))
                        .map_err(|error| {
                            pyo3::exceptions::PyRuntimeError::new_err(error.to_string())
                        })
                },
            )
            .unwrap();
            module.setattr("send_quote", send_quote).unwrap();
            let enqueue = PyCFunction::new_closure(
                py,
                None,
                None,
                move |args: &pyo3::Bound<'_, PyTuple>, _| -> pyo3::PyResult<()> {
                    let cause = args
                        .get_item(0)?
                        .extract::<String>()?
                        .parse::<UUID4>()
                        .map_err(pyo3::exceptions::PyValueError::new_err)?;
                    let mut query = QueryAccount::new(
                        trader_id,
                        Some(client_id),
                        account_id,
                        UUID4::new(),
                        args.get_item(1)?.extract::<u64>()?.into(),
                        None,
                        None,
                    );
                    query.causation_id = Some(cause);
                    msgbus::send_trading_command(
                        MessagingSwitchboard::exec_engine_queue_execute(),
                        TradingCommand::QueryAccount(query),
                    );
                    Ok(())
                },
            )
            .unwrap();
            module.setattr("enqueue", enqueue).unwrap();
            py.run(c_str!(r#"
import asyncio
import sqlite3
import tempfile
from pathlib import Path

directory = tempfile.TemporaryDirectory()
database = Path(directory.name) / "terminal.sqlite3"
with sqlite3.connect(database) as connection:
    connection.execute("CREATE TABLE observations(kind TEXT, event_id TEXT, identity TEXT, outlet TEXT, state TEXT, native_borrow_free INTEGER)")
delivered = []
query_effects = []
dispatches = []
errors = []
accepted = False
done = False
expiry = None
quote_timestamp = None
initial_status = None
stop_status = None
node_error = None

class TerminalStrategy(Strategy):
    def on_start(self):
        global expiry, initial_status
        expiry = self.clock.timestamp_ns() + 1_000_000_000 if expired_case else None
        self.order = self.order_factory.limit(
            instrument_id, side, quantity, price, time_in_force=tif,
            expire_time=expiry, post_only=not expired_case, client_order_id=order_id,
        )
        initial_status = str(self.order.status)
        self.submit_order(self.order, client_id=client_id)
    def on_stop(self):
        global stop_status
        stop_status = self._control_delivery_status()
    def on_dispose(self): pass
    def observe(self, event, outlet):
        global accepted, done
        try:
            cached = self.cache.order(event.client_order_id)
            row = (type(event).__name__, str(event.event_id), str(event.client_order_id), outlet, str(cached.status), native_borrow_available())
            with sqlite3.connect(database, timeout=0) as connection:
                connection.execute("BEGIN IMMEDIATE")
                connection.execute("INSERT INTO observations VALUES (?, ?, ?, ?, ?, ?)", row)
            delivered.append(row)
            if outlet == "specific":
                if type(event).__name__ in {"OrderSubmitted", "OrderRejected", "OrderExpired"}:
                    enqueue(str(event.event_id), event.ts_init)
                if type(event).__name__ == "OrderAccepted": accepted = True
                if type(event).__name__ in {"OrderRejected", "OrderExpired"}:
                    done = True
                    if callback_failure:
                        raise ValueError("terminal callback failure:" + str(event.event_id))
        except BaseException as error:
            errors.append(repr(error))
            raise
    def on_order_submitted(self, event): self.observe(event, "specific")
    def on_order_accepted(self, event): self.observe(event, "specific")
    def on_order_rejected(self, event): self.observe(event, "specific")
    def on_order_expired(self, event): self.observe(event, "specific")
    def on_order_event(self, event):
        if type(event).__name__ != "OrderInitialized": self.observe(event, "aggregate")

strategy = TerminalStrategy()
def mark_waiting(): pass
def on_dispatch(command_id): dispatches.append(command_id)
def on_query_completed(command_id, previous_id, observed_id, before_count, after_count, ts_event, balance):
    query_effects.append((command_id, previous_id, observed_id, before_count, after_count, ts_event, balance))
def persisted():
    with sqlite3.connect(database, timeout=0) as connection:
        rows = connection.execute("SELECT * FROM observations ORDER BY rowid").fetchall()
    return [(*row[:-1], bool(row[-1])) for row in rows]
def cleanup(): directory.cleanup()
"#), Some(&module.dict()), None).unwrap();
            module.unbind()
        });
        let strategy = Python::attach(|py| observer.bind(py).getattr("strategy").unwrap().unbind());
        node.kernel()
            .trader()
            .borrow_mut()
            .add_python_strategy_instance(&strategy)
            .unwrap();
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
        let queries = Rc::new(RefCell::new(Vec::new()));
        let stopped = Rc::new(Cell::new(0));
        let (finish, receive) = oneshot::channel();
        finish.send(()).unwrap();
        engine
            .borrow_mut()
            .register_client(Box::new(HeldSandboxConnect {
                inner,
                finish: Some(receive),
                phase: Rc::new(Cell::new(ConnectPhase::Before)),
                waiting_callback: Python::attach(|py| {
                    observer.bind(py).getattr("mark_waiting").unwrap().unbind()
                }),
                queries: queries.clone(),
                stopped: stopped.clone(),
                completed_query_callback: Some(Python::attach(|py| {
                    observer
                        .bind(py)
                        .getattr("on_query_completed")
                        .unwrap()
                        .unbind()
                })),
            }))
            .unwrap();
        let queued = Rc::new(RefCell::new(Vec::new()));
        let dispatched = Rc::new(RefCell::new(Vec::new()));
        let raw = Rc::new(RefCell::new(Vec::new()));
        let outward = Rc::new(RefCell::new(Vec::new()));
        let quotes = Rc::new(RefCell::new(Vec::new()));
        let observed_owner = Rc::new(RefCell::new(None));
        msgbus::set_bus_tap(Rc::new(TerminalTap {
            queries: HostedQueryTap {
                engine,
                queued: queued.clone(),
                dispatched: dispatched.clone(),
                callback: Python::attach(|py| {
                    observer.bind(py).getattr("on_dispatch").unwrap().unbind()
                }),
            },
            cache: cache.clone(),
            raw: raw.clone(),
            outward: outward.clone(),
            quotes: quotes.clone(),
            owner: observed_owner.clone(),
        }));
        let (py_node, result) = Python::attach(|py| {
            let node = pyo3::Py::new(py, PyLiveNode::new(node)).unwrap();
            let globals = PyDict::new(py);
            globals.set_item("node", &node).unwrap();
            globals.set_item("observer", observer.bind(py)).unwrap();
            let result = py.run(c_str!(r#"
import asyncio
async def exercise():
    task = asyncio.create_task(node.run_async())
    async def wait_for(predicate):
        while not predicate():
            if task.done():
                await task
                raise AssertionError("native node stopped before terminal observations")
            if observer.errors: raise AssertionError(observer.errors)
            await asyncio.sleep(0)
    try:
        if observer.expired_case:
            await asyncio.wait_for(wait_for(lambda: observer.accepted), timeout=3)
            async def reach_expiry():
                while observer.strategy.clock.timestamp_ns() <= observer.expiry:
                    await asyncio.sleep(0.001)
            await asyncio.wait_for(reach_expiry(), timeout=3)
            observer.quote_timestamp = observer.strategy.clock.timestamp_ns()
            observer.send_quote(observer.quote_timestamp)
        if observer.callback_failure:
            try:
                await asyncio.wait_for(task, timeout=3)
            except TimeoutError:
                raise
            except Exception as error:
                observer.node_error = repr(error)
            else:
                raise AssertionError("normal node swallowed an actual callback failure")
        else:
            await asyncio.wait_for(wait_for(lambda: observer.done and len(observer.query_effects) == 2), timeout=3)
    except BaseException:
        print("terminal diagnostics", observer.delivered, observer.errors, observer.query_effects, flush=True)
        raise
    finally:
        node.handle().stop()
        if not task.done():
            await asyncio.wait_for(task, timeout=3)
        elif not observer.callback_failure:
            await task
asyncio.run(exercise())
"#), Some(&globals), None);
            (node, result)
        });
        msgbus::clear_bus_tap();
        let order_after_stop = cache.borrow().order_owned(&order_id);
        let account_after_stop = cache.borrow().account_owned(&account_id);
        let open_after_stop = cache
            .borrow()
            .positions_open(None, None, None, None, None)
            .len();
        let closed_after_stop = cache
            .borrow()
            .positions_closed(None, None, None, None, None)
            .len();
        let owner = observed_owner
            .borrow()
            .clone()
            .expect("actual hosted owner observed at ingress");
        let owner_status = owner.status().unwrap().unwrap();
        let retained_outputs = owner.retained_typed_deliveries().unwrap();
        let retained_inputs = owner.retained_incoming_orders().unwrap();
        let retained_commands = owner.retained_commands().unwrap();
        let observations = Python::attach(|py| {
            let module = observer.bind(py);
            let delivered: Vec<DeliveryRow> =
                module.getattr("delivered").unwrap().extract().unwrap();
            let persisted: Vec<DeliveryRow> =
                module.call_method0("persisted").unwrap().extract().unwrap();
            let errors: Vec<String> = module.getattr("errors").unwrap().extract().unwrap();
            let effects: Vec<QueryCompletion> =
                module.getattr("query_effects").unwrap().extract().unwrap();
            let expiry: Option<u64> = module.getattr("expiry").unwrap().extract().unwrap();
            let quote_timestamp: Option<u64> = module
                .getattr("quote_timestamp")
                .unwrap()
                .extract()
                .unwrap();
            let initial_status: String =
                module.getattr("initial_status").unwrap().extract().unwrap();
            let state: NodeState = py_node
                .call_method0(py, "handle")
                .unwrap()
                .getattr(py, "state")
                .unwrap()
                .extract(py)
                .unwrap();
            let stop_status: DeliveryStatus =
                module.getattr("stop_status").unwrap().extract().unwrap();
            let node_error: Option<String> =
                module.getattr("node_error").unwrap().extract().unwrap();
            let old_failures: Vec<(String, Option<String>, String, Option<String>)> = strategy
                .call_method0(py, "_control_delivery_failures")
                .unwrap()
                .extract(py)
                .unwrap();
            py_node.call_method0(py, "dispose").unwrap();
            module.call_method0("cleanup").unwrap();
            (
                delivered,
                persisted,
                errors,
                effects,
                expiry,
                quote_timestamp,
                initial_status,
                state,
                stop_status,
                node_error,
                old_failures,
            )
        });
        assert!(
            result.is_ok(),
            "actual hosted terminal exercise: {result:?}"
        );
        assert!(stopped.get() >= 1);
        assert_eq!(
            capture.capture().unwrap_err(),
            nautilus_common::clients::ExecutionAccountCaptureError::Retired
        );
        let (
            delivered,
            persisted,
            errors,
            effects,
            expiry,
            quote_timestamp,
            initial_status,
            state,
            stop_status,
            node_error,
            old_failures,
        ) = observations;
        if callback_failure {
            eprintln!(
                "incoming terminal failure: node_error={node_error:?}, stop_status={stop_status:?}, owner_status={owner_status:?}, inputs={retained_inputs:?}, outputs={retained_outputs:?}, commands={retained_commands:?}"
            );
        }
        assert_eq!(state, NodeState::Stopped);
        if callback_failure {
            assert_eq!(errors.len(), 1);
            assert!(errors[0].contains("ValueError('terminal callback failure:"));
            assert!(node_error.as_ref().unwrap().contains("ExternalCallback"));
            assert!(!stop_status.0);
            let (pending_at_stop, failure_at_stop) = stop_status.1.unwrap();
            assert!(pending_at_stop >= 2);
            assert_eq!(
                failure_at_stop.as_deref(),
                Some("native owner delivery ExternalCallback")
            );
            assert_eq!(owner_status.1, Some(OwnerDeliveryError::ExternalCallback));
            assert!(owner_status.0 >= 3);
        } else {
            assert!(errors.is_empty());
            assert!(node_error.is_none());
            assert_eq!(owner_status, (0, None));
        }
        assert!(
            old_failures.is_empty(),
            "command-only empty failures are not the node's overall status"
        );
        assert_eq!(initial_status, "INITIALIZED");
        assert_eq!(persisted, delivered);
        let cached = order_after_stop.unwrap();
        let canonical: Vec<_> = cached
            .events()
            .iter()
            .filter(|event| !matches!(event, OrderEventAny::Initialized(_)))
            .map(|event| identity(event))
            .collect();
        let ingress: Vec<_> = raw.borrow().iter().map(identity).collect();
        let published: Vec<_> = outward
            .borrow()
            .iter()
            .map(|(event, _)| identity(event))
            .collect();
        assert_eq!(ingress, canonical);
        assert_eq!(published, canonical);
        let expected_kinds = if expired {
            vec!["OrderSubmitted", "OrderAccepted", "OrderExpired"]
        } else {
            vec!["OrderSubmitted", "OrderRejected"]
        };
        assert_eq!(
            canonical
                .iter()
                .map(|(kind, _, _)| kind.as_str())
                .collect::<Vec<_>>(),
            expected_kinds
        );
        let expected_states = if expired {
            vec![
                OrderStatus::Submitted,
                OrderStatus::Accepted,
                OrderStatus::Expired,
            ]
        } else {
            vec![OrderStatus::Submitted, OrderStatus::Rejected]
        };
        assert_eq!(
            outward
                .borrow()
                .iter()
                .map(|(_, state)| *state)
                .collect::<Vec<_>>(),
            expected_states
        );
        let expected: Vec<_> = canonical
            .iter()
            .zip(expected_states)
            .flat_map(|((kind, event_id, cuid), state)| {
                ["specific", "aggregate"].map(|outlet| {
                    (
                        kind.clone(),
                        event_id.clone(),
                        cuid.clone(),
                        outlet.to_owned(),
                        state.to_string(),
                        true,
                    )
                })
            })
            .collect();
        assert_eq!(delivered, expected);
        assert_eq!(
            cached.status(),
            if expired {
                OrderStatus::Expired
            } else {
                OrderStatus::Rejected
            }
        );
        assert_eq!(cached.quantity(), Quantity::from("100.00"));
        assert_eq!(cached.filled_qty(), Quantity::from("0.00"));
        assert!(cached.commissions().is_empty());
        assert_eq!(cached.account_id(), Some(account_id));
        assert_eq!(
            account_after_stop.as_ref().unwrap().balance_total(None),
            Some(Money::from("100000 USDC"))
        );
        assert_eq!(open_after_stop, 0);
        assert_eq!(closed_after_stop, 0);
        assert_eq!(queued.borrow().len(), 2);
        let executed_queries = if callback_failure {
            usize::from(expired)
        } else {
            2
        };
        assert_eq!(
            &queued.borrow()[..executed_queries],
            dispatched.borrow().as_slice()
        );
        assert_eq!(queries.borrow().len(), executed_queries);
        assert_eq!(effects.len(), executed_queries);
        let account = account_after_stop.unwrap();
        let account_events: std::collections::HashSet<_> = account
            .events()
            .iter()
            .map(|event| event.event_id.to_string())
            .collect();
        let mut fresh_query_events = std::collections::HashSet::new();
        let causes: Vec<_> = canonical
            .iter()
            .filter(|(kind, _, _)| kind != "OrderAccepted")
            .map(|(_, event_id, _)| event_id.clone())
            .collect();
        for (index, ((query, phase), effect)) in queries.borrow().iter().zip(&effects).enumerate() {
            assert_eq!(*query, queued.borrow()[index]);
            assert_eq!(*phase, ConnectPhase::Returned);
            assert_eq!(query.causation_id.unwrap().to_string(), causes[index]);
            assert_eq!(effect.0, query.command_id.to_string());
            assert_ne!(effect.1, effect.2);
            assert!(fresh_query_events.insert(effect.2.clone()));
            assert!(account_events.contains(&effect.2));
            assert_eq!(effect.4, effect.3 + 1);
            assert!(effect.5 >= query.ts_init.as_u64());
            assert_eq!(effect.6, Money::from("100000 USDC").to_string());
        }
        if callback_failure {
            let terminal = raw.borrow().last().unwrap().clone();
            let terminal_id = terminal.clone().into_boxed().id();
            assert_eq!(
                errors,
                vec![format!(
                    "ValueError('terminal callback failure:{terminal_id}')"
                )]
            );
            assert_eq!(retained_outputs.len(), 1);
            let retained = &retained_outputs[0];
            let OwnerDeliveryCause::Incoming(cause) = retained.cause else {
                panic!("normal ingress cannot acquire a synthetic control cause");
            };
            assert_eq!(cause.event_id, terminal_id);
            assert_eq!(cause.sequence, canonical.len() as u64);
            assert_eq!(retained.event_id, terminal_id);
            assert_eq!(retained.error, Some(OwnerDeliveryError::ExternalCallback));
            let OwnerExternalEvent::Order(original) = &retained.event else {
                panic!("the failed terminal callback must retain its original order event");
            };
            assert_eq!(*original, terminal);
            assert_eq!(retained_inputs.len(), 1);
            assert_eq!(retained_inputs[0].cause, Some(cause));
            assert_eq!(
                retained_inputs[0].stage,
                OwnerIncomingStage::DispatchReturned
            );
            assert_eq!(
                retained_inputs[0].error,
                OwnerDeliveryError::ExternalCallback
            );
            let OwnerIncomingOrder::Process(original) = &retained_inputs[0].event else {
                panic!("an applied terminal event cannot become a publication");
            };
            assert_eq!(*original, terminal);
            assert_eq!(retained_commands.len(), 2 - executed_queries);
            for (retained, query) in retained_commands
                .iter()
                .zip(&queued.borrow()[executed_queries..])
            {
                assert_eq!(
                    retained.endpoint,
                    MessagingSwitchboard::exec_engine_execute()
                );
                assert_eq!(
                    retained.command,
                    TradingCommand::QueryAccount(query.clone())
                );
                assert_eq!(
                    retained.cause, None,
                    "a plain Async query has not entered a control factory"
                );
                assert_eq!(retained.error, OwnerDeliveryError::ExternalCallback);
            }
            assert_eq!(owner_status.0, 2 + retained_commands.len());
        } else {
            assert!(retained_outputs.is_empty());
            assert!(retained_inputs.is_empty());
            assert!(retained_commands.is_empty());
        }
        if expired {
            assert!(quote_timestamp.unwrap() > expiry.unwrap());
            assert_eq!(quotes.borrow().len(), 1);
            assert_eq!(
                quotes.borrow()[0].ts_init.as_u64(),
                quote_timestamp.unwrap()
            );
            assert_eq!(cached.expire_time().unwrap().as_u64(), expiry.unwrap());
            let accepted = raw
                .borrow()
                .iter()
                .find_map(|event| {
                    if let OrderEventAny::Accepted(event) = event {
                        Some(event.venue_order_id)
                    } else {
                        None
                    }
                })
                .unwrap();
            assert_eq!(cached.venue_order_id(), Some(accepted));
        } else {
            assert!(quotes.borrow().is_empty());
            assert!(expiry.is_none());
            assert!(cached.venue_order_id().is_none());
        }
    }
}

#[rstest]
#[case::normal(None)]
#[case::first_publication_closed(Some(0))]
#[case::second_publication_closed(Some(1))]
fn test_option_settlement_publications_preserve_cached_plan_and_stop_on_channel_failure(
    #[case] fail_at: Option<usize>,
) {
    use std::cell::Cell;

    use nautilus_execution::matching_engine::{
        InitializedPublicationFailure, InitializedPublicationSource, OrderMatchingEngine,
    };
    use nautilus_model::instruments::stubs::{equity_aapl, option_contract_appl};

    let cache = Rc::new(RefCell::new(Cache::default()));
    let mut option = option_contract_appl();
    option.activation_ns = UnixNanos::from(1);
    option.expiration_ns = UnixNanos::from(100);
    let option = InstrumentAny::OptionContract(option);
    let mut underlying = equity_aapl();
    underlying.id = InstrumentId::from("AAPL.OPRA");
    let underlying = InstrumentAny::Equity(underlying);
    cache.borrow_mut().add_instrument(option.clone()).unwrap();
    cache
        .borrow_mut()
        .add_instrument(underlying.clone())
        .unwrap();
    cache
        .borrow_mut()
        .add_trade(TradeTick::new(
            underlying.id(),
            Price::from("160.00"),
            Quantity::from(1),
            AggressorSide::NoAggressor,
            TradeId::from("OPTION-UNDERLYING"),
            UnixNanos::from(1),
            UnixNanos::from(1),
        ))
        .unwrap();
    let opening = OrderTestBuilder::new(OrderType::Market)
        .instrument_id(option.id())
        .side(OrderSide::Buy)
        .quantity(Quantity::from(1))
        .submit(true)
        .build();
    let position_id = PositionId::from("PUBLICATION-OPTION-POSITION");
    let account_id = AccountId::from("OPRA-001");
    cache
        .borrow_mut()
        .add_order(opening.clone(), Some(position_id), None, false)
        .unwrap();
    let fill = TestOrderEventStubs::filled(
        &opening,
        &option,
        Some(TradeId::from("OPTION-OPEN")),
        Some(position_id),
        Some(Price::from("5.00")),
        Some(Quantity::from(1)),
        None,
        Some(Money::from("0 USD")),
        Some(UnixNanos::from(1)),
        Some(account_id),
    );
    cache.borrow_mut().update_order(&fill).unwrap();
    let OrderEventAny::Filled(fill) = fill else {
        unreachable!()
    };
    let position = Position::new(&option, fill);
    cache
        .borrow_mut()
        .add_position(&position, OmsType::Netting)
        .unwrap();
    let clock = Rc::new(RefCell::new(TestClock::new()));
    clock.borrow_mut().set_time(UnixNanos::from(100));
    let mut engine = OrderMatchingEngine::new(
        option.clone(),
        1,
        FillModelHandle::default(),
        Default::default(),
        BookType::L1_MBP,
        OmsType::Netting,
        AccountType::Cash,
        clock,
        cache.clone(),
        Default::default(),
    );
    let outcomes = Rc::new(RefCell::new(Vec::new()));
    let emitted = outcomes.clone();
    let handler_cache = cache.clone();
    engine.set_event_handler(Rc::new(move |event| {
        handler_cache.borrow_mut().update_order(&event).unwrap();
        emitted.borrow_mut().push(event);
    }));
    let (sender, receiver) = tokio::sync::mpsc::unbounded_channel::<ExecutionEvent>();
    let receiver = Rc::new(RefCell::new(receiver));
    let close_receiver = receiver.clone();
    let count = Cell::new(0);
    engine.set_initialized_publication_handler(Rc::new(move |event| {
        if fail_at == Some(count.get()) {
            close_receiver.borrow_mut().close();
        }
        count.set(count.get() + 1);
        sender
            .send(ExecutionEvent::OrderInitializedPublication(event))
            .map_err(|error| {
                let ExecutionEvent::OrderInitializedPublication(event) = error.0 else {
                    unreachable!()
                };
                InitializedPublicationFailure::ChannelClosed(Box::new(event))
            })
    }));
    let close = InstrumentClose::new(
        option.id(),
        Price::from("7.50"),
        InstrumentCloseType::ContractExpired,
        UnixNanos::from(100),
        UnixNanos::from(100),
    );
    engine.process_instrument_close(close);
    let publications: Vec<_> = std::iter::from_fn(|| receiver.borrow_mut().try_recv().ok())
        .map(|event| {
            let ExecutionEvent::OrderInitializedPublication(event) = event else {
                unreachable!()
            };
            event
        })
        .collect();
    match fail_at {
        Some(failed_index) => {
            assert!(!engine.is_expiration_processed());
            assert!(outcomes.borrow().is_empty());
            assert_eq!(publications.len(), failed_index);
            let fault = engine.initialized_publication_failure().unwrap().clone();
            let InitializedPublicationSource::OptionSettlement {
                leg_position_id,
                timestamp,
                close: saved_close,
                cached_plan_initializations,
            } = &fault.source
            else {
                panic!("expected real option settlement evidence")
            };
            assert_eq!(*timestamp, UnixNanos::from(100));
            assert_eq!(*saved_close, Some(close));
            assert_eq!(
                *leg_position_id,
                if failed_index == 0 {
                    Some(position_id)
                } else {
                    None
                }
            );
            assert_eq!(cached_plan_initializations.len(), 2);
            assert_eq!(
                fault.failure.event(),
                &cached_plan_initializations[failed_index]
            );
            assert_eq!(&publications, &cached_plan_initializations[..failed_index]);
            for event in cached_plan_initializations.iter() {
                let order = cache.borrow().order_owned(&event.client_order_id).unwrap();
                assert_eq!(order.init_event(), event);
                assert_eq!(order.status(), OrderStatus::Initialized);
                assert_eq!(order.events().len(), 1);
            }
            assert!(
                engine
                    .account_capture_has_pending_work(&account_id)
                    .unwrap()
            );
            let before = cache
                .borrow()
                .orders_total_count(None, None, None, None, None);
            engine.process_status(MarketStatusAction::Trading);
            engine.reset();
            engine.process_instrument_close(close);
            engine.iterate(UnixNanos::from(101), AggressorSide::NoAggressor);
            engine.fill_market_order(fault.failure.event().client_order_id);
            assert!(outcomes.borrow().is_empty());
            assert_eq!(
                cache
                    .borrow()
                    .orders_total_count(None, None, None, None, None),
                before
            );
            assert_eq!(
                engine
                    .initialized_publication_failure()
                    .unwrap()
                    .failure
                    .event(),
                fault.failure.event()
            );
            assert_eq!(
                cache
                    .borrow()
                    .position_owned(&position_id)
                    .unwrap()
                    .quantity,
                position.quantity
            );
        }
        None => {
            assert!(engine.is_expiration_processed());
            assert!(engine.initialized_publication_failure().is_none());
            assert_eq!(publications.len(), 2);
            let outcomes = outcomes.borrow();
            assert_eq!(outcomes.len(), 4);
            assert!(matches!(outcomes[0], OrderEventAny::Accepted(_)));
            assert!(matches!(outcomes[1], OrderEventAny::Accepted(_)));
            let OrderEventAny::Filled(close_fill) = &outcomes[2] else {
                panic!("expected option close fill")
            };
            let OrderEventAny::Filled(delivery_fill) = &outcomes[3] else {
                panic!("expected underlying delivery")
            };
            assert_eq!(close_fill.client_order_id, publications[0].client_order_id);
            assert_eq!(
                delivery_fill.client_order_id,
                publications[1].client_order_id
            );
            assert_eq!(close_fill.position_id, Some(position_id));
            assert_eq!(delivery_fill.position_id, None);
            assert_eq!(close_fill.last_px, Price::from("7.50"));
            assert_eq!(delivery_fill.last_px, Price::from("149.00"));
            assert_eq!(close_fill.last_qty, Quantity::from(1));
            assert_eq!(delivery_fill.last_qty, Quantity::from(1));
            assert_eq!(close_fill.commission, Some(Money::from("0 USD")));
            assert_eq!(delivery_fill.commission, Some(Money::from("0 USD")));
            for event in publications {
                assert_eq!(
                    cache
                        .borrow()
                        .order_owned(&event.client_order_id)
                        .unwrap()
                        .status(),
                    OrderStatus::Filled
                );
            }
        }
    }
}

#[rstest]
#[case::normal_cancel(false, "cancel")]
#[case::normal_reduce(false, "reduce")]
#[case::quote(true, "quote")]
#[case::trade(true, "trade")]
#[case::bar(true, "bar")]
#[case::submit(true, "submit")]
#[case::modify(true, "modify")]
#[case::cancel(true, "cancel")]
#[case::cancel_all(true, "cancel_all")]
#[case::batch_modify(true, "batch_modify")]
#[case::batch_cancel(true, "batch_cancel")]
#[case::iterate(true, "iterate")]
#[case::fill_market(true, "fill_market")]
#[case::fill_limit(true, "fill_limit")]
#[case::trigger_stop(true, "trigger_stop")]
#[case::close(true, "close")]
#[case::expiration(true, "expiration")]
#[case::liquidation(true, "liquidation")]
#[case::update_instrument(true, "update_instrument")]
#[case::book_delta(true, "book_delta")]
#[case::book_deltas(true, "book_deltas")]
#[case::book_depth(true, "book_depth")]
#[case::reset_and_trading(true, "reset_and_trading")]
#[case::replace_handler(true, "replace_handler")]
fn test_terminal_publication_fault_prevents_effectful_matching_entries(
    #[case] faulted: bool,
    #[case] action: &str,
) {
    use nautilus_common::messages::execution::{BatchCancelOrders, CancelOrder};
    use nautilus_execution::matching_engine::{
        InitializedPublicationFailure, InitializedPublicationFault, OrderMatchingEngine,
        config::OrderMatchingEngineConfig,
    };
    use nautilus_model::{
        data::{OrderBookDelta, OrderBookDeltas, OrderBookDepth10, order::BookOrder},
        enums::{BookAction, MarketStatus},
    };

    let instrument = InstrumentAny::CryptoPerpetual(crypto_perpetual_ethusdt());
    let cache = Rc::new(RefCell::new(Cache::default()));
    cache
        .borrow_mut()
        .add_instrument(instrument.clone())
        .unwrap();
    let account_id = AccountId::from("BINANCE-001");
    let position_id = PositionId::from("PUBLICATION-GUARD-POSITION");
    let opening = OrderTestBuilder::new(OrderType::Market)
        .instrument_id(instrument.id())
        .client_order_id("GUARD-OPEN".into())
        .side(OrderSide::Buy)
        .quantity(Quantity::from("1.000"))
        .submit(true)
        .build();
    cache
        .borrow_mut()
        .add_order(opening.clone(), Some(position_id), None, false)
        .unwrap();
    let fill = TestOrderEventStubs::filled(
        &opening,
        &instrument,
        Some(TradeId::from("GUARD-OPEN")),
        Some(position_id),
        Some(Price::from("100.00")),
        None,
        None,
        Some(Money::from("0 USDT")),
        None,
        Some(account_id),
    );
    cache.borrow_mut().update_order(&fill).unwrap();
    let OrderEventAny::Filled(fill) = fill else {
        unreachable!()
    };
    let position = Position::new(&instrument, fill);
    cache
        .borrow_mut()
        .add_position(&position, OmsType::Netting)
        .unwrap();
    let mut engine = OrderMatchingEngine::new(
        instrument.clone(),
        1,
        FillModelHandle::default(),
        Default::default(),
        BookType::L1_MBP,
        OmsType::Netting,
        AccountType::Margin,
        Rc::new(RefCell::new(TestClock::new())),
        cache.clone(),
        OrderMatchingEngineConfig {
            bar_execution: true,
            trade_execution: true,
            ..Default::default()
        },
    );
    let outcomes = Rc::new(RefCell::new(Vec::new()));
    let emitted = outcomes.clone();
    let handler_cache = cache.clone();
    engine.set_event_handler(Rc::new(move |event| {
        handler_cache.borrow_mut().update_order(&event).unwrap();
        emitted.borrow_mut().push(event);
    }));
    let quote = QuoteTick::new(
        instrument.id(),
        Price::from("99.00"),
        Price::from("100.00"),
        Quantity::from("100.000"),
        Quantity::from("100.000"),
        UnixNanos::from(1),
        UnixNanos::from(1),
    );
    engine.process_quote_tick(&quote);
    let mut resting = if action == "trigger_stop" {
        OrderTestBuilder::new(OrderType::StopLimit)
            .instrument_id(instrument.id())
            .client_order_id("GUARD-RESTING".into())
            .side(OrderSide::Buy)
            .quantity(Quantity::from("1.000"))
            .trigger_price(Price::from("150.00"))
            .price(Price::from("149.00"))
            .submit(true)
            .build()
    } else {
        OrderTestBuilder::new(OrderType::Limit)
            .instrument_id(instrument.id())
            .client_order_id("GUARD-RESTING".into())
            .side(OrderSide::Buy)
            .quantity(Quantity::from("1.000"))
            .price(Price::from("99.00"))
            .submit(true)
            .build()
    };
    cache
        .borrow_mut()
        .add_order(resting.clone(), None, None, false)
        .unwrap();
    engine.process_order(&mut resting, account_id);
    assert_eq!(
        cache
            .borrow()
            .order_owned(&resting.client_order_id())
            .unwrap()
            .status(),
        OrderStatus::Accepted
    );
    assert_eq!(engine.get_open_orders().len(), 1);
    assert!(
        !engine
            .account_capture_has_pending_work(&account_id)
            .unwrap()
    );
    let mut candidate = OrderTestBuilder::new(OrderType::Market)
        .instrument_id(instrument.id())
        .client_order_id("GUARD-CANDIDATE".into())
        .side(OrderSide::Sell)
        .quantity(Quantity::from("1.000"))
        .reduce_only(true)
        .submit(true)
        .build();
    cache
        .borrow_mut()
        .add_order(candidate.clone(), Some(position_id), None, false)
        .unwrap();
    let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel::<ExecutionEvent>();
    if faulted {
        receiver.close();
    }
    engine.set_initialized_publication_handler(Rc::new(move |event| {
        sender
            .send(ExecutionEvent::OrderInitializedPublication(event))
            .map_err(|error| {
                let ExecutionEvent::OrderInitializedPublication(event) = error.0 else {
                    unreachable!()
                };
                InitializedPublicationFailure::ChannelClosed(Box::new(event))
            })
    }));
    let before_events = outcomes.borrow().len();
    if faulted {
        engine.liquidate_open_positions(
            UnixNanos::from(2),
            false,
            instrument.settlement_currency(),
        );
        assert!(engine.initialized_publication_failure().is_some());
        assert_eq!(engine.market_status, MarketStatus::Open);
        assert_eq!(outcomes.borrow().len(), before_events);
        assert!(
            engine
                .account_capture_has_pending_work(&account_id)
                .unwrap()
        );
    }
    let fault_event = engine
        .initialized_publication_failure()
        .map(|fault| fault.failure.event().event_id);
    let before_orders = cache
        .borrow()
        .orders_total_count(None, None, None, None, None);
    let before_bid = engine.best_bid_price();
    let before_ask = engine.best_ask_price();
    let modify = ModifyOrder::new(
        resting.trader_id(),
        None,
        resting.strategy_id(),
        instrument.id(),
        resting.client_order_id(),
        resting.venue_order_id(),
        Some(Quantity::from("0.500")),
        Some(Price::from("98.00")),
        None,
        UUID4::new(),
        UnixNanos::from(3),
        None,
        None,
    );
    let cancel = CancelOrder::new(
        resting.trader_id(),
        None,
        resting.strategy_id(),
        instrument.id(),
        resting.client_order_id(),
        resting.venue_order_id(),
        UUID4::new(),
        UnixNanos::from(3),
        None,
        None,
    );
    match action {
        "cancel" => engine.process_cancel(&cancel, account_id),
        "reduce" | "submit" => engine.process_order(&mut candidate, account_id),
        "modify" => engine.process_modify(&modify, account_id),
        "cancel_all" => engine.process_cancel_all(
            &CancelAllOrders::new(
                resting.trader_id(),
                None,
                resting.strategy_id(),
                instrument.id(),
                None,
                UUID4::new(),
                UnixNanos::from(3),
                None,
                None,
            ),
            account_id,
        ),
        "batch_cancel" => engine.process_batch_cancel(
            &BatchCancelOrders::new(
                resting.trader_id(),
                None,
                resting.strategy_id(),
                instrument.id(),
                vec![cancel],
                UUID4::new(),
                UnixNanos::from(3),
                None,
                None,
            ),
            account_id,
        ),
        "batch_modify" => engine.process_batch_modify(
            &BatchModifyOrders::new(
                resting.trader_id(),
                None,
                resting.strategy_id(),
                instrument.id(),
                vec![modify],
                UUID4::new(),
                UnixNanos::from(3),
                None,
                None,
            ),
            account_id,
        ),
        "iterate" => engine.iterate(UnixNanos::from(3), AggressorSide::Sell),
        "fill_market" => engine.fill_market_order(candidate.client_order_id()),
        "fill_limit" => engine.fill_limit_order(resting.client_order_id()),
        "trigger_stop" => engine.trigger_stop_order(resting.client_order_id()),
        "close" => engine.process_instrument_close(InstrumentClose::new(
            instrument.id(),
            Price::from("100.00"),
            InstrumentCloseType::ContractExpired,
            UnixNanos::from(3),
            UnixNanos::from(3),
        )),
        "expiration" => engine.process_instrument_expiration(UnixNanos::from(u64::MAX)),
        "liquidation" => engine.liquidate_open_positions(
            UnixNanos::from(3),
            true,
            instrument.settlement_currency(),
        ),
        "quote" => engine.process_quote_tick(&QuoteTick::new(
            instrument.id(),
            Price::from("98.00"),
            Price::from("99.00"),
            quote.bid_size,
            quote.ask_size,
            UnixNanos::from(3),
            UnixNanos::from(3),
        )),
        "trade" => engine.process_trade_tick(&TradeTick::new(
            instrument.id(),
            Price::from("99.00"),
            Quantity::from("1.000"),
            AggressorSide::Sell,
            TradeId::from("GUARD-TRIGGER"),
            UnixNanos::from(3),
            UnixNanos::from(3),
        )),
        "bar" => engine.process_bar(&Bar::new(
            format!("{}-1-MINUTE-LAST-EXTERNAL", instrument.id())
                .parse()
                .unwrap(),
            Price::from("100.00"),
            Price::from("101.00"),
            Price::from("98.00"),
            Price::from("99.00"),
            Quantity::from("1.000"),
            UnixNanos::from(3),
            UnixNanos::from(3),
        )),
        "update_instrument" => assert!(
            engine
                .update_instrument(instrument)
                .unwrap_err()
                .is::<InitializedPublicationFault>()
        ),
        "book_delta" | "book_deltas" => {
            let delta = OrderBookDelta::new(
                instrument.id(),
                BookAction::Add,
                BookOrder::new(
                    OrderSide::Buy,
                    Price::from("98.00"),
                    Quantity::from("1.000"),
                    1,
                ),
                0,
                1,
                UnixNanos::from(3),
                UnixNanos::from(3),
            );
            let result = if action == "book_delta" {
                engine.process_order_book_delta(&delta)
            } else {
                engine
                    .process_order_book_deltas(&OrderBookDeltas::new(instrument.id(), vec![delta]))
            };
            assert!(result.unwrap_err().is::<InitializedPublicationFault>());
        }
        "book_depth" => {
            let depth = OrderBookDepth10::new(
                instrument.id(),
                [BookOrder::new(
                    OrderSide::Buy,
                    Price::from("98.00"),
                    Quantity::from("1.000"),
                    1,
                ); 10],
                [BookOrder::new(
                    OrderSide::Sell,
                    Price::from("99.00"),
                    Quantity::from("1.000"),
                    2,
                ); 10],
                [1; 10],
                [1; 10],
                0,
                1,
                UnixNanos::from(3),
                UnixNanos::from(3),
            );
            assert!(
                engine
                    .process_order_book_depth10(&depth)
                    .unwrap_err()
                    .is::<InitializedPublicationFault>()
            );
        }
        "reset_and_trading" => {
            engine.reset();
            engine.process_status(MarketStatusAction::Trading);
            engine.process_order(&mut candidate, account_id);
        }
        "replace_handler" => {
            engine.set_initialized_publication_handler(Rc::new(|_| Ok(())));
            engine.process_order(&mut candidate, account_id);
        }
        _ => panic!("unknown matching entry test"),
    }
    if faulted {
        assert_eq!(
            outcomes.borrow().len(),
            before_events,
            "unexpected effect for {action}"
        );
        assert_eq!(engine.get_open_orders().len(), 1);
        assert_eq!(engine.best_bid_price(), before_bid);
        assert_eq!(engine.best_ask_price(), before_ask);
        assert_eq!(
            cache
                .borrow()
                .orders_total_count(None, None, None, None, None),
            before_orders
        );
        let retained = engine.initialized_publication_failure().unwrap();
        assert_eq!(Some(retained.failure.event().event_id), fault_event);
        let retained_order = cache
            .borrow()
            .order_owned(&retained.failure.event().client_order_id)
            .unwrap();
        assert_eq!(retained_order.status(), OrderStatus::Initialized);
        assert_eq!(retained_order.events().len(), 1);
        assert_eq!(
            cache
                .borrow()
                .position_owned(&position_id)
                .unwrap()
                .quantity,
            position.quantity
        );
        assert_eq!(
            cache
                .borrow()
                .order_owned(&resting.client_order_id())
                .unwrap()
                .status(),
            OrderStatus::Accepted
        );
    } else {
        assert!(engine.initialized_publication_failure().is_none());
        assert!(outcomes.borrow().len() > before_events);
        match action {
            "cancel" => assert_eq!(cache.borrow().order_owned(&resting.client_order_id()).unwrap().status(), OrderStatus::Canceled),
            "reduce" => assert!(outcomes.borrow().iter().any(|event| matches!(event, OrderEventAny::Filled(fill) if fill.client_order_id == candidate.client_order_id() && fill.last_qty == Quantity::from("1.000")))),
            _ => unreachable!(),
        }
    }
}

#[rstest]
#[case::delta("delta")]
#[case::deltas("deltas")]
#[case::depth("depth")]
fn test_matching_result_reports_a_publication_fault_created_by_the_same_book_call(
    #[case] entry: &str,
) {
    use nautilus_execution::matching_engine::{
        InitializedPublicationFailure, InitializedPublicationFault, OrderMatchingEngine,
    };
    use nautilus_model::{
        data::{OrderBookDelta, OrderBookDeltas, OrderBookDepth10, order::BookOrder},
        enums::{BookAction, RecordFlag},
        instruments::stubs::futures_contract_es,
    };

    let mut future = futures_contract_es(None, None);
    future.activation_ns = UnixNanos::from(1);
    future.expiration_ns = UnixNanos::from(100);
    let instrument = InstrumentAny::FuturesContract(future);
    let cache = Rc::new(RefCell::new(Cache::default()));
    cache
        .borrow_mut()
        .add_instrument(instrument.clone())
        .unwrap();
    let account_id = AccountId::from("XCME-001");
    let position_id = PositionId::from("BOOK-PUBLICATION-POSITION");
    let opening = OrderTestBuilder::new(OrderType::Market)
        .instrument_id(instrument.id())
        .client_order_id("BOOK-PUBLICATION-OPEN".into())
        .side(OrderSide::Buy)
        .quantity(Quantity::from(1))
        .submit(true)
        .build();
    cache
        .borrow_mut()
        .add_order(opening.clone(), Some(position_id), None, false)
        .unwrap();
    let fill = TestOrderEventStubs::filled(
        &opening,
        &instrument,
        Some(TradeId::from("BOOK-PUBLICATION-OPEN")),
        Some(position_id),
        Some(Price::from("5000.00")),
        None,
        None,
        Some(Money::from("0 USD")),
        None,
        Some(account_id),
    );
    cache.borrow_mut().update_order(&fill).unwrap();
    let OrderEventAny::Filled(fill) = fill else {
        unreachable!()
    };
    cache
        .borrow_mut()
        .add_position(&Position::new(&instrument, fill), OmsType::Netting)
        .unwrap();
    let clock = Rc::new(RefCell::new(TestClock::new()));
    clock.borrow_mut().set_time(UnixNanos::from(50));
    let mut engine = OrderMatchingEngine::new(
        instrument.clone(),
        1,
        FillModelHandle::default(),
        Default::default(),
        BookType::L1_MBP,
        OmsType::Netting,
        AccountType::Margin,
        clock.clone(),
        cache.clone(),
        Default::default(),
    );
    engine.set_event_handler(Rc::new(|_| {
        panic!("a failed initialization must not produce a verdict or fill")
    }));
    engine.process_quote_tick(&QuoteTick::new(
        instrument.id(),
        Price::from("5000.00"),
        Price::from("5000.25"),
        Quantity::from(100),
        Quantity::from(100),
        UnixNanos::from(50),
        UnixNanos::from(50),
    ));
    let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel::<ExecutionEvent>();
    receiver.close();
    engine.set_initialized_publication_handler(Rc::new(move |event| {
        sender
            .send(ExecutionEvent::OrderInitializedPublication(event))
            .map_err(|error| {
                let ExecutionEvent::OrderInitializedPublication(event) = error.0 else {
                    unreachable!()
                };
                InitializedPublicationFailure::ChannelClosed(Box::new(event))
            })
    }));
    clock.borrow_mut().set_time(UnixNanos::from(100));
    let delta = OrderBookDelta::new(
        instrument.id(),
        BookAction::Add,
        BookOrder::new(OrderSide::Buy, Price::from("4999.75"), Quantity::from(1), 1),
        RecordFlag::F_LAST as u8,
        1,
        UnixNanos::from(100),
        UnixNanos::from(100),
    );
    let result = match entry {
        "delta" => engine.process_order_book_delta(&delta),
        "deltas" => {
            engine.process_order_book_deltas(&OrderBookDeltas::new(instrument.id(), vec![delta]))
        }
        "depth" => {
            let mut bids = [BookOrder::default(); 10];
            let mut asks = [BookOrder::default(); 10];
            bids[0] = BookOrder::new(
                OrderSide::Buy,
                Price::from("5000.00"),
                Quantity::from(100),
                1,
            );
            asks[0] = BookOrder::new(
                OrderSide::Sell,
                Price::from("5000.25"),
                Quantity::from(100),
                2,
            );
            engine.process_order_book_depth10(&OrderBookDepth10::new(
                instrument.id(),
                bids,
                asks,
                [1; 10],
                [1; 10],
                0,
                1,
                UnixNanos::from(100),
                UnixNanos::from(100),
            ))
        }
        _ => unreachable!(),
    };
    let fault = engine
        .initialized_publication_failure()
        .expect("the real expired position must hit the closed channel");
    assert!(
        result.is_err(),
        "{entry} returned success after creating terminal fault: {fault}"
    );
    assert!(result.unwrap_err().is::<InitializedPublicationFault>());
    let order = cache
        .borrow()
        .order_owned(&fault.failure.event().client_order_id)
        .unwrap();
    assert_eq!(order.init_event(), fault.failure.event());
    assert_eq!(order.status(), OrderStatus::Initialized);
}

#[rstest]
fn test_paper_binary_option_pending_resolution_then_close_settlement(
    trader_id: TraderId,
    account_id: AccountId,
) {
    let mut harness = setup_pending_resolution_harness(trader_id, account_id, "BO-PAPER");
    assert_pending_resolution_transition(&mut harness, "REST-BO-PAPER", "PROBE-BO-PAPER");

    let close = InstrumentClose::new(
        harness.instrument.id(),
        Price::from("1.000"),
        InstrumentCloseType::ContractExpired,
        UnixNanos::from(300),
        UnixNanos::from(300),
    );
    msgbus::publish_any(
        nautilus_common::msgbus::switchboard::get_instrument_close_topic(harness.instrument.id()),
        &close,
    );

    let mut seen_expiration_fill = false;

    for event in std::iter::from_fn(|| harness.rx.try_recv().ok()) {
        if let ExecutionEvent::Order(OrderEventAny::Filled(fill)) = event
            && fill.client_order_id.as_str().starts_with("EXPIRATION-")
            && fill.last_px == Price::from("1.000")
        {
            seen_expiration_fill = true;
        }
    }
    assert!(
        seen_expiration_fill,
        "expected EXPIRATION fill after publishing InstrumentClose to sandbox paper lane"
    );
}

#[rstest]
fn test_paper_binary_option_pending_resolution_then_close_settlement_via_data_engine(
    trader_id: TraderId,
    account_id: AccountId,
) {
    let mut harness = setup_pending_resolution_harness(trader_id, account_id, "BO-DE");
    let cache = harness.context.cache.clone();
    let data_engine = Rc::new(RefCell::new(DataEngine::new(
        harness.clock.clone(),
        cache,
        None,
    )));
    DataEngine::register_msgbus_handlers(&data_engine);
    assert_pending_resolution_transition(&mut harness, "REST-BO-DE", "PROBE-BO-DE");

    let close = InstrumentClose::new(
        harness.instrument.id(),
        Price::from("1.000"),
        InstrumentCloseType::ContractExpired,
        UnixNanos::from(300),
        UnixNanos::from(300),
    );
    msgbus::send_data(
        MessagingSwitchboard::data_engine_process_data(),
        Data::InstrumentClose(close),
    );

    let mut seen_expiration_fill = false;

    for event in std::iter::from_fn(|| harness.rx.try_recv().ok()) {
        if let ExecutionEvent::Order(OrderEventAny::Filled(fill)) = event
            && fill.client_order_id.as_str().starts_with("EXPIRATION-")
            && fill.last_px == Price::from("1.000")
        {
            seen_expiration_fill = true;
        }
    }
    assert!(
        seen_expiration_fill,
        "expected EXPIRATION fill after sending InstrumentClose through DataEngine endpoint"
    );
}

#[rstest]
fn test_instrument_status_lazy_creates_but_close_requires_existing_engine(
    trader_id: TraderId,
    account_id: AccountId,
) {
    setup_order_event_handler();

    let instrument = make_binary_option_instrument("0xCOND", "0xYES", "Yes", 100);
    let mut test_context = create_test_context(trader_id, account_id, instrument.id().venue);
    test_context
        .cache
        .borrow_mut()
        .add_instrument(instrument.clone())
        .unwrap();
    test_context.client.start().unwrap();

    let status = InstrumentStatus::new(
        instrument.id(),
        MarketStatusAction::Trading,
        UnixNanos::from(1),
        UnixNanos::from(1),
        None,
        None,
        Some(true),
        Some(true),
        None,
    );
    msgbus::publish_any(
        nautilus_common::msgbus::switchboard::get_instrument_status_topic(instrument.id()),
        &status,
    );
    assert_eq!(test_context.client.matching_engine_count(), 1);

    let second_instrument = make_binary_option_instrument("0xCOND2", "0xYES2", "Yes", 100);
    test_context
        .cache
        .borrow_mut()
        .add_instrument(second_instrument.clone())
        .unwrap();

    let close = InstrumentClose::new(
        second_instrument.id(),
        Price::from("1.000"),
        InstrumentCloseType::ContractExpired,
        UnixNanos::from(2),
        UnixNanos::from(2),
    );
    msgbus::publish_any(
        nautilus_common::msgbus::switchboard::get_instrument_close_topic(second_instrument.id()),
        &close,
    );

    assert_eq!(
        test_context.client.matching_engine_count(),
        1,
        "InstrumentClose should not lazy-create a matching engine from cache",
    );
}

#[rstest]
fn test_instrument_close_finalizes_expired_engine_without_open_state(
    trader_id: TraderId,
    account_id: AccountId,
) {
    *msgbus::get_message_bus().borrow_mut() = MessageBus::default();
    setup_order_event_handler();

    let mut harness = setup_binary_option_lifecycle_harness(
        trader_id,
        account_id,
        "0xFINALIZE",
        "0xYES",
        "Yes",
        100,
    );
    assert_eq!(harness.client.matching_engine_count(), 1);

    publish_expired_close(
        &harness.test_clock,
        &harness.instrument,
        Price::from("1.000"),
        200,
    );

    assert_eq!(harness.client.matching_engine_count(), 0);
    assert!(
        harness
            .cache
            .borrow()
            .instrument(&harness.instrument.id())
            .is_none()
    );

    harness.client.stop().unwrap();
}

#[rstest]
fn test_periodic_sweep_retires_expired_quote_only_engine(
    trader_id: TraderId,
    account_id: AccountId,
) {
    *msgbus::get_message_bus().borrow_mut() = MessageBus::default();
    setup_order_event_handler();

    // Quote-only lifecycle: the matching engine is lazily created from a quote, with no order,
    // position, or InstrumentClose to drive the event-driven cleanup paths.
    let mut harness = setup_binary_option_lifecycle_harness(
        trader_id, account_id, "0xSWEEP", "0xYES", "Yes", 100,
    );
    assert_eq!(harness.client.matching_engine_count(), 1);
    assert!(
        harness
            .cache
            .borrow()
            .instrument(&harness.instrument.id())
            .is_some()
    );

    // Expiry alone, before the sweep interval elapses, does not release the engine.
    let pre_sweep = harness
        .test_clock
        .borrow_mut()
        .advance_time(UnixNanos::from(200), true);
    assert!(
        pre_sweep.is_empty(),
        "sweep timer must not fire before its interval",
    );
    assert_eq!(harness.client.matching_engine_count(), 1);
    assert!(
        harness
            .cache
            .borrow()
            .instrument(&harness.instrument.id())
            .is_some()
    );

    // Advance past one sweep interval and fire the timer (60s matches
    // EXPIRED_ENGINE_SWEEP_INTERVAL in `execution.rs`).
    let events = harness
        .test_clock
        .borrow_mut()
        .advance_time(UnixNanos::from(60_000_000_001), true);
    let handlers = harness.test_clock.borrow().match_handlers(events);
    for handler in handlers {
        handler.run();
    }

    assert_eq!(
        harness.client.matching_engine_count(),
        0,
        "periodic sweep should retire the expired quote-only matching engine",
    );
    assert!(
        harness
            .cache
            .borrow()
            .instrument(&harness.instrument.id())
            .is_none(),
        "periodic sweep should purge the expired instrument from the cache",
    );

    harness.client.stop().unwrap();
}

#[rstest]
fn test_periodic_sweep_retains_expired_engine_with_open_position(
    trader_id: TraderId,
    account_id: AccountId,
) {
    *msgbus::get_message_bus().borrow_mut() = MessageBus::default();
    setup_order_event_handler();

    let mut harness = setup_binary_option_lifecycle_harness(
        trader_id,
        account_id,
        "0xSWEEPOPEN",
        "0xYES",
        "Yes",
        100,
    );

    let position = submit_open_position_and_seed_cache(
        &harness.client,
        &harness.cache,
        trader_id,
        &harness.instrument,
        "OPEN-SWEEP",
        "P-OPEN-SWEEP",
        &mut harness.rx,
    );
    let venue = harness.instrument.id().venue;
    harness
        .cache
        .borrow_mut()
        .add_position(&position, OmsType::Netting)
        .unwrap();

    // Isolate any event produced by the sweep from the opening fill emitted during setup.
    while harness.rx.try_recv().is_ok() {}

    // Fire the sweep past expiry: settlement safety must retain the engine while a position is open.
    let events = harness
        .test_clock
        .borrow_mut()
        .advance_time(UnixNanos::from(60_000_000_001), true);
    let handlers = harness.test_clock.borrow().match_handlers(events);
    for handler in handlers {
        handler.run();
    }

    // The sweep performs no settlement: no fill or position-close event, and the position stays open.
    let sweep_events: Vec<ExecutionEvent> =
        std::iter::from_fn(|| harness.rx.try_recv().ok()).collect();
    assert!(
        sweep_events.is_empty(),
        "periodic sweep must not emit execution events for an open position, was {sweep_events:?}",
    );
    assert!(
        harness.cache.borrow().has_positions_open(
            Some(&venue),
            Some(&harness.instrument.id()),
            None,
            None,
            None,
        ),
        "periodic sweep must leave the open position open",
    );
    assert_eq!(
        harness.client.matching_engine_count(),
        1,
        "periodic sweep must not retire an expired engine with an open position",
    );
    assert!(
        harness
            .cache
            .borrow()
            .instrument(&harness.instrument.id())
            .is_some(),
        "periodic sweep must not purge an instrument with an open position",
    );

    harness.client.stop().unwrap();
}

#[rstest]
fn test_periodic_sweep_retains_expired_engine_with_open_order(
    trader_id: TraderId,
    account_id: AccountId,
) {
    *msgbus::get_message_bus().borrow_mut() = MessageBus::default();
    setup_order_event_handler();

    let mut harness = setup_binary_option_lifecycle_harness(
        trader_id,
        account_id,
        "0xSWEEPORDER",
        "0xYES",
        "Yes",
        100,
    );

    // A limit order well below the bid rests unfilled, so the instrument expires carrying a
    // non-terminal order and no position.
    let order = OrderTestBuilder::new(OrderType::Limit)
        .instrument_id(harness.instrument.id())
        .side(OrderSide::Buy)
        .price(Price::new(0.10, 3))
        .quantity(Quantity::from("1.00"))
        .client_order_id("RESTING-SWEEP".into())
        .ts_init(UnixNanos::from(10))
        .submit(true)
        .build();
    harness
        .cache
        .borrow_mut()
        .add_order(order.clone(), None, None, false)
        .unwrap();
    harness
        .client
        .submit_order(SubmitOrder::from_order(
            &order,
            trader_id,
            Some(harness.client.client_id()),
            None,
            UUID4::new(),
            UnixNanos::from(10),
        ))
        .unwrap();

    // Apply the acceptance so the order registers in the cache open-order index.
    let accepted = std::iter::from_fn(|| harness.rx.try_recv().ok())
        .find_map(|event| match event {
            ExecutionEvent::Order(order_event @ OrderEventAny::Accepted(_)) => Some(order_event),
            _ => None,
        })
        .expect("expected acceptance for the resting limit order");
    harness.cache.borrow_mut().update_order(&accepted).unwrap();

    let venue = harness.instrument.id().venue;
    let instrument_id = harness.instrument.id();
    assert!(
        harness.cache.borrow().has_orders_open(
            Some(&venue),
            Some(&instrument_id),
            None,
            None,
            None,
        ),
        "resting limit order should be open before the sweep",
    );
    assert!(
        !harness.cache.borrow().has_positions_open(
            Some(&venue),
            Some(&instrument_id),
            None,
            None,
            None,
        ),
        "resting limit order should not have opened a position",
    );

    let events = harness
        .test_clock
        .borrow_mut()
        .advance_time(UnixNanos::from(60_000_000_001), true);
    let handlers = harness.test_clock.borrow().match_handlers(events);
    for handler in handlers {
        handler.run();
    }

    // `purge_instrument_skip_order_guard` requires callers to have already terminalized order
    // state, which this sweep has not done, so engine and instrument must both be retained.
    assert_eq!(
        harness.client.matching_engine_count(),
        1,
        "periodic sweep must not retire an expired engine with an open order",
    );
    assert!(
        harness.cache.borrow().instrument(&instrument_id).is_some(),
        "periodic sweep must not purge an instrument with an open order",
    );
    assert!(
        harness.cache.borrow().has_orders_open(
            Some(&venue),
            Some(&instrument_id),
            None,
            None,
            None,
        ),
        "periodic sweep must leave the resting order open and reachable",
    );

    harness.client.stop().unwrap();
}

#[rstest]
fn test_instrument_close_keeps_engine_until_position_closed(
    trader_id: TraderId,
    account_id: AccountId,
) {
    *msgbus::get_message_bus().borrow_mut() = MessageBus::default();
    setup_order_event_handler();

    let mut harness = setup_binary_option_lifecycle_harness(
        trader_id, account_id, "0xSETTLE", "0xYES", "Yes", 100,
    );
    let venue = harness.instrument.id().venue;
    assert_eq!(harness.client.matching_engine_count(), 1);

    let position = submit_open_position_and_seed_cache(
        &harness.client,
        &harness.cache,
        trader_id,
        &harness.instrument,
        "OPEN-POSITION",
        "P-OPEN-POSITION",
        &mut harness.rx,
    );
    harness
        .cache
        .borrow_mut()
        .add_position(&position, OmsType::Netting)
        .unwrap();

    publish_expired_close(
        &harness.test_clock,
        &harness.instrument,
        Price::from("1.000"),
        200,
    );

    assert_eq!(harness.client.matching_engine_count(), 1);
    assert!(
        harness
            .cache
            .borrow()
            .instrument(&harness.instrument.id())
            .is_some()
    );

    let closed = settle_position_from_expiration_fill(&harness.cache, &position, &mut harness.rx);
    assert!(!harness.cache.borrow().has_orders_open(
        Some(&venue),
        Some(&harness.instrument.id()),
        None,
        None,
        None,
    ));
    assert!(!harness.cache.borrow().has_positions_open(
        Some(&venue),
        Some(&harness.instrument.id()),
        None,
        None,
        None,
    ));
    msgbus::publish_position_event(
        "events.position.TEST".into(),
        &position_closed_event(&closed, account_id),
    );

    assert_eq!(harness.client.matching_engine_count(), 0);

    harness.client.stop().unwrap();
}

#[rstest]
fn test_position_closed_finalize_ignores_other_account(trader_id: TraderId, account_id: AccountId) {
    *msgbus::get_message_bus().borrow_mut() = MessageBus::default();
    setup_order_event_handler();

    let mut harness = setup_binary_option_lifecycle_harness(
        trader_id,
        account_id,
        "0xACCOUNT",
        "0xYES",
        "Yes",
        100,
    );

    let position = submit_open_position_and_seed_cache(
        &harness.client,
        &harness.cache,
        trader_id,
        &harness.instrument,
        "OPEN-ACCOUNT",
        "P-OPEN-ACCOUNT",
        &mut harness.rx,
    );
    harness
        .cache
        .borrow_mut()
        .add_position(&position, OmsType::Netting)
        .unwrap();

    publish_expired_close(
        &harness.test_clock,
        &harness.instrument,
        Price::from("1.000"),
        200,
    );

    let closed = settle_position_from_expiration_fill(&harness.cache, &position, &mut harness.rx);
    msgbus::publish_position_event(
        "events.position.TEST".into(),
        &position_closed_event(&closed, AccountId::from("OTHER-001")),
    );

    assert_eq!(harness.client.matching_engine_count(), 1);
    assert!(
        harness
            .cache
            .borrow()
            .instrument(&harness.instrument.id())
            .is_some()
    );

    harness.client.stop().unwrap();
}

#[rstest]
fn test_position_closed_does_not_purge_non_expired_instrument(
    trader_id: TraderId,
    account_id: AccountId,
) {
    *msgbus::get_message_bus().borrow_mut() = MessageBus::default();
    setup_order_event_handler();

    let mut harness = setup_binary_option_lifecycle_harness(
        trader_id, account_id, "0xACTIVE", "0xYES", "Yes", 1_000,
    );

    let position = submit_open_position_and_seed_cache(
        &harness.client,
        &harness.cache,
        trader_id,
        &harness.instrument,
        "OPEN-ACTIVE",
        "P-ACTIVE",
        &mut harness.rx,
    );
    harness
        .cache
        .borrow_mut()
        .add_position(&position, OmsType::Netting)
        .unwrap();

    let mut closed = position;
    closed.side = PositionSide::Flat;
    closed.ts_closed = Some(closed.ts_last);
    harness.cache.borrow_mut().update_position(&closed).unwrap();

    msgbus::publish_position_event(
        "events.position.TEST".into(),
        &position_closed_event(&closed, account_id),
    );

    assert_eq!(
        harness.client.matching_engine_count(),
        1,
        "non-expired position close should not release sandbox matching engines",
    );
    assert!(
        harness
            .cache
            .borrow()
            .instrument(&harness.instrument.id())
            .is_some()
    );

    harness.client.stop().unwrap();
}

#[rstest]
fn test_instrument_close_removes_resting_order_only_engine_before_cancel_event_applies(
    trader_id: TraderId,
    account_id: AccountId,
) {
    *msgbus::get_message_bus().borrow_mut() = MessageBus::default();
    setup_order_event_handler();

    let mut harness = setup_binary_option_lifecycle_harness(
        trader_id,
        account_id,
        "0xRESTING",
        "0xYES",
        "Yes",
        100,
    );
    let venue = harness.instrument.id().venue;
    assert_eq!(harness.client.matching_engine_count(), 1);

    let resting_order = OrderTestBuilder::new(OrderType::Limit)
        .instrument_id(harness.instrument.id())
        .side(OrderSide::Buy)
        .price(Price::from("0.050"))
        .quantity(Quantity::from("1.00"))
        .client_order_id("REST-CLOSE-ONLY".into())
        .ts_init(UnixNanos::from(20))
        .submit(true)
        .build();
    harness
        .cache
        .borrow_mut()
        .add_order(resting_order.clone(), None, None, false)
        .unwrap();
    harness
        .client
        .submit_order(SubmitOrder::from_order(
            &resting_order,
            trader_id,
            Some(harness.client.client_id()),
            None,
            UUID4::new(),
            UnixNanos::from(20),
        ))
        .unwrap();

    let order_events = apply_order_events_from_channel(&harness.cache, &mut harness.rx);
    assert!(
        order_events
            .iter()
            .any(|event| matches!(event, OrderEventAny::Accepted(accepted)
                if accepted.client_order_id.as_str() == "REST-CLOSE-ONLY")),
        "expected resting order acceptance before expiration",
    );
    assert!(harness.cache.borrow().has_orders_open(
        Some(&venue),
        Some(&harness.instrument.id()),
        None,
        None,
        None,
    ));

    publish_expired_close(
        &harness.test_clock,
        &harness.instrument,
        Price::from("1.000"),
        200,
    );

    assert_eq!(
        harness.client.matching_engine_count(),
        0,
        "order-only expired instruments should release their matching engine immediately",
    );
    assert!(
        harness
            .cache
            .borrow()
            .instrument(&harness.instrument.id())
            .is_none()
    );

    let order_events = apply_order_events_from_channel(&harness.cache, &mut harness.rx);
    assert!(
        order_events
            .iter()
            .any(|event| matches!(event, OrderEventAny::Canceled(canceled)
                if canceled.client_order_id.as_str() == "REST-CLOSE-ONLY")),
        "expected expiration to cancel the resting order",
    );
    assert!(!harness.cache.borrow().has_orders_open(
        Some(&venue),
        Some(&harness.instrument.id()),
        None,
        None,
        None,
    ));
    assert_eq!(
        harness.client.matching_engine_count(),
        0,
        "cancellation replay should not recreate engine retention after close",
    );
    assert!(
        harness
            .cache
            .borrow()
            .instrument(&harness.instrument.id())
            .is_none()
    );

    harness.client.stop().unwrap();
}

#[rstest]
fn test_submit_order_list_keeps_processing_all_expired_legs_before_cleanup(
    trader_id: TraderId,
    account_id: AccountId,
) {
    *msgbus::get_message_bus().borrow_mut() = MessageBus::default();
    let mut harness = setup_binary_option_lifecycle_harness(
        trader_id,
        account_id,
        "0xORDER-LIST",
        "0xYES",
        "Yes",
        100,
    );
    let client_id = harness.client.client_id();

    let _ = harness
        .test_clock
        .borrow_mut()
        .advance_time(UnixNanos::from(200), true);

    let first = OrderTestBuilder::new(OrderType::Limit)
        .trader_id(trader_id)
        .instrument_id(harness.instrument.id())
        .client_order_id(ClientOrderId::from("O-EXPIRED-LIST-001"))
        .side(OrderSide::Buy)
        .price(Price::from("0.400"))
        .quantity(Quantity::from("1"))
        .build();
    let second = OrderTestBuilder::new(OrderType::Limit)
        .trader_id(trader_id)
        .instrument_id(harness.instrument.id())
        .client_order_id(ClientOrderId::from("O-EXPIRED-LIST-002"))
        .side(OrderSide::Buy)
        .price(Price::from("0.450"))
        .quantity(Quantity::from("1"))
        .build();
    let orders = vec![first, second];

    for order in &orders {
        harness
            .cache
            .borrow_mut()
            .add_order(order.clone(), None, Some(client_id), false)
            .unwrap();
    }

    harness
        .client
        .submit_order_list(create_submit_order_list(
            trader_id,
            client_id,
            harness.instrument.id(),
            &orders,
        ))
        .unwrap();

    let order_events = apply_order_events_from_channel(&harness.cache, &mut harness.rx);

    for order in &orders {
        assert!(
            order_events.iter().any(|event| {
                event.client_order_id() == order.client_order_id()
                    && matches!(event, OrderEventAny::Rejected(_))
            }),
            "expired order-list leg should emit a terminal rejection, not stay SUBMITTED",
        );
    }

    assert_eq!(harness.client.matching_engine_count(), 0);
    assert!(
        harness
            .cache
            .borrow()
            .instrument(&harness.instrument.id())
            .is_none()
    );

    harness.client.stop().unwrap();
}

#[rstest]
fn test_instrument_close_sync_cleanup_handles_synchronous_position_closed_reentry(
    trader_id: TraderId,
) {
    std::thread::spawn(move || {
        *msgbus::get_message_bus().borrow_mut() = MessageBus::default();

        let venue = Venue::new("BINANCE");
        let account_id = AccountId::from("BINANCE-001");
        let client_id = ClientId::new("SANDBOX");
        let cache = Rc::new(RefCell::new(Cache::default()));
        let test_clock = Rc::new(RefCell::new(TestClock::new()));
        let clock: Rc<RefCell<dyn Clock>> = test_clock.clone();

        let mut binary = binary_option();
        binary.id = InstrumentId::from("YES.BINANCE");
        binary.raw_symbol = "YES".into();
        binary.activation_ns = UnixNanos::from(1);
        binary.expiration_ns = UnixNanos::from(100);
        let instrument = InstrumentAny::BinaryOption(binary);

        cache
            .borrow_mut()
            .add_instrument(instrument.clone())
            .unwrap();
        cache
            .borrow_mut()
            .add_quote(create_binary_option_quote(instrument.id()))
            .unwrap();

        let cache_for_handler = cache.clone();
        let order_events = Rc::new(RefCell::new(Vec::<OrderEventAny>::new()));
        let order_events_for_handler = order_events.clone();
        let opening_fill = Rc::new(RefCell::new(None::<OrderFilled>));
        let opening_fill_for_handler = opening_fill.clone();
        let instrument_for_position = instrument.clone();
        let instrument_for_handler = instrument.clone();
        let order_handler = TypedIntoHandler::from(move |event: OrderEventAny| {
            order_events_for_handler.borrow_mut().push(event.clone());
            let _ = cache_for_handler.borrow_mut().update_order(&event);

            let OrderEventAny::Filled(mut fill) = event else {
                return;
            };

            if fill.client_order_id.as_str().starts_with("EXPIRATION-") {
                let position = cache_for_handler
                    .borrow()
                    .positions_open(
                        Some(&venue),
                        Some(&instrument_for_handler.id()),
                        None,
                        Some(&account_id),
                        None,
                    )
                    .into_iter()
                    .next()
                    .expect("expected open position before expiration fill")
                    .clone();
                fill.position_id = Some(position.id);

                let mut closed = position;
                closed.apply(&fill);
                cache_for_handler
                    .borrow_mut()
                    .update_position(&closed)
                    .unwrap();

                let position_closed =
                    PositionClosed::create(&closed, &fill, UUID4::new(), fill.ts_event);
                msgbus::publish_position_event(
                    "events.position.TEST".into(),
                    &PositionEvent::PositionClosed(position_closed),
                );
            } else {
                *opening_fill_for_handler.borrow_mut() = Some(fill);
            }
        });
        msgbus::register_order_event_endpoint(
            MessagingSwitchboard::exec_engine_process(),
            order_handler,
        );

        let usd = Currency::USD();
        let config = SandboxExecutionClientConfig {
            account_id,
            venue,
            starting_balances: vec![Money::new(100_000.0, usd)],
            base_currency: Some(usd),
            oms_type: OmsType::Netting,
            account_type: AccountType::Margin,
            default_leverage: Decimal::ONE,
            leverages: ahash::AHashMap::new(),
            book_type: BookType::L1_MBP,
            fee_model: None,
            fill_model: None,
            frozen_account: false,
            bar_execution: false,
            trade_execution: false,
            reject_stop_orders: true,
            support_gtd_orders: true,
            support_contingent_orders: true,
            use_position_ids: true,
            use_random_ids: false,
            use_reduce_only: true,
            queue_position: false,
            liquidity_consumption: false,
            bar_adaptive_high_low_ordering: false,
            use_market_order_acks: false,
            oto_full_trigger: false,
            price_protection_points: 0,
        };
        let core = ExecutionClientCore::new(
            trader_id,
            client_id,
            venue,
            config.oms_type,
            config.account_id,
            config.account_type,
            config.base_currency,
            cache.clone(),
        );
        let mut client = SandboxExecutionClient::new(core, config, clock, cache.clone());
        client.start().unwrap();

        let order = OrderTestBuilder::new(OrderType::Market)
            .trader_id(trader_id)
            .instrument_id(instrument.id())
            .client_order_id(ClientOrderId::from("O-SYNC-REENTRY-001"))
            .side(OrderSide::Buy)
            .quantity(Quantity::from("1.00"))
            .build();
        cache
            .borrow_mut()
            .add_order(order.clone(), None, Some(client_id), false)
            .unwrap();

        let ts = test_clock.borrow().timestamp_ns();
        client
            .submit_order(SubmitOrder::from_order(
                &order,
                trader_id,
                Some(client_id),
                None,
                UUID4::new(),
                ts,
            ))
            .unwrap();

        assert!(
            order_events
                .borrow()
                .iter()
                .any(|event| matches!(event, OrderEventAny::Filled(_))),
            "expected opening fill event, found {:?}",
            order_events.borrow(),
        );

        let mut opening_fill = opening_fill
            .borrow_mut()
            .take()
            .expect("expected opening fill before expiration");
        opening_fill.position_id = Some(PositionId::new("P-SYNC-REENTRY"));
        let position = Position::new(&instrument_for_position, opening_fill);
        cache
            .borrow_mut()
            .add_position(&position, OmsType::Netting)
            .unwrap();

        assert!(cache.borrow().has_positions_open(
            Some(&venue),
            Some(&instrument.id()),
            None,
            Some(&account_id),
            None,
        ));

        publish_expired_close(&test_clock, &instrument, Price::from("1.000"), 200);

        assert_eq!(client.matching_engine_count(), 0);
        assert!(cache.borrow().instrument(&instrument.id()).is_none());
        client.stop().unwrap();
    })
    .join()
    .unwrap();
}

#[rstest]
fn test_local_expiry_removes_resting_order_only_engine_before_cancel_event_applies(
    trader_id: TraderId,
    account_id: AccountId,
) {
    *msgbus::get_message_bus().borrow_mut() = MessageBus::default();
    setup_order_event_handler();

    let mut harness = setup_binary_option_lifecycle_harness(
        trader_id,
        account_id,
        "0xLOCAL-REST",
        "0xYES",
        "Yes",
        100,
    );
    let venue = harness.instrument.id().venue;
    assert_eq!(harness.client.matching_engine_count(), 1);

    let resting_order = OrderTestBuilder::new(OrderType::Limit)
        .instrument_id(harness.instrument.id())
        .side(OrderSide::Buy)
        .price(Price::from("0.050"))
        .quantity(Quantity::from("1.00"))
        .client_order_id("REST-LOCAL-ONLY".into())
        .ts_init(UnixNanos::from(20))
        .submit(true)
        .build();
    harness
        .cache
        .borrow_mut()
        .add_order(resting_order.clone(), None, None, false)
        .unwrap();
    harness
        .client
        .submit_order(SubmitOrder::from_order(
            &resting_order,
            trader_id,
            Some(harness.client.client_id()),
            None,
            UUID4::new(),
            UnixNanos::from(20),
        ))
        .unwrap();

    let order_events = apply_order_events_from_channel(&harness.cache, &mut harness.rx);
    assert!(
        order_events
            .iter()
            .any(|event| matches!(event, OrderEventAny::Accepted(accepted)
                if accepted.client_order_id.as_str() == "REST-LOCAL-ONLY")),
        "expected resting order acceptance before local expiry",
    );
    assert!(harness.cache.borrow().has_orders_open(
        Some(&venue),
        Some(&harness.instrument.id()),
        None,
        None,
        None,
    ));

    let _ = harness
        .test_clock
        .borrow_mut()
        .advance_time(UnixNanos::from(200), true);

    let probe_order = OrderTestBuilder::new(OrderType::Limit)
        .instrument_id(harness.instrument.id())
        .side(OrderSide::Buy)
        .price(Price::from("0.050"))
        .quantity(Quantity::from("1.00"))
        .client_order_id("PROBE-LOCAL-ONLY".into())
        .ts_init(UnixNanos::from(200))
        .submit(true)
        .build();
    harness
        .cache
        .borrow_mut()
        .add_order(probe_order.clone(), None, None, false)
        .unwrap();
    harness
        .client
        .submit_order(SubmitOrder::from_order(
            &probe_order,
            trader_id,
            Some(harness.client.client_id()),
            None,
            UUID4::new(),
            UnixNanos::from(200),
        ))
        .unwrap();

    assert_eq!(
        harness.client.matching_engine_count(),
        0,
        "local expiry should release the engine before cancel/reject events apply",
    );
    assert!(
        harness
            .cache
            .borrow()
            .instrument(&harness.instrument.id())
            .is_none()
    );

    let order_events = apply_order_events_from_channel(&harness.cache, &mut harness.rx);
    assert!(
        order_events
            .iter()
            .any(|event| matches!(event, OrderEventAny::Canceled(canceled)
                if canceled.client_order_id.as_str() == "REST-LOCAL-ONLY")),
        "expected local expiry to cancel the resting order",
    );
    assert!(
        order_events
            .iter()
            .any(|event| matches!(event, OrderEventAny::Rejected(rejected)
                if rejected.client_order_id.as_str() == "PROBE-LOCAL-ONLY"
                    && rejected.reason.contains("pending resolution"))),
        "expected local expiry to reject new orders while pending resolution",
    );
    assert!(!harness.cache.borrow().has_orders_open(
        Some(&venue),
        Some(&harness.instrument.id()),
        None,
        None,
        None,
    ));
    assert_eq!(harness.client.matching_engine_count(), 0);

    harness.client.stop().unwrap();
}

#[rstest]
fn test_paper_binary_option_multiple_instruments_close_settlement_via_data_engine(
    trader_id: TraderId,
    account_id: AccountId,
) {
    let instruments = vec![
        (
            make_binary_option_instrument("0xCOND-BTC", "0xBTC-YES", "Yes", 100),
            Price::from("1.000"),
            "OPEN-BTC-YES",
            "P-BTC-YES",
        ),
        (
            make_binary_option_instrument("0xCOND-BTC", "0xBTC-NO", "No", 100),
            Price::from("0.000"),
            "OPEN-BTC-NO",
            "P-BTC-NO",
        ),
        (
            make_binary_option_instrument("0xCOND-ETH", "0xETH-YES", "Yes", 100),
            Price::from("0.000"),
            "OPEN-ETH-YES",
            "P-ETH-YES",
        ),
        (
            make_binary_option_instrument("0xCOND-ETH", "0xETH-NO", "No", 100),
            Price::from("1.000"),
            "OPEN-ETH-NO",
            "P-ETH-NO",
        ),
    ];
    let venue = instruments[0].0.id().venue;
    let cache = Rc::new(RefCell::new(Cache::default()));
    let test_clock = Rc::new(RefCell::new(TestClock::new()));
    let clock: Rc<RefCell<dyn Clock>> = test_clock.clone();

    let mut config = create_config(trader_id, account_id, venue);
    config.base_currency = Some(Currency::USDC());
    config.starting_balances = vec![Money::new(100_000.0, Currency::USDC())];
    let core = ExecutionClientCore::new(
        trader_id,
        ClientId::new("SANDBOX"),
        config.venue,
        config.oms_type,
        config.account_id,
        config.account_type,
        config.base_currency,
        cache.clone(),
    );
    let mut client = SandboxExecutionClient::new(core, config, clock.clone(), cache.clone());

    let data_engine = Rc::new(RefCell::new(DataEngine::new(clock, cache.clone(), None)));
    DataEngine::register_msgbus_handlers(&data_engine);

    for (instrument, _, _, _) in &instruments {
        cache
            .borrow_mut()
            .add_instrument(instrument.clone())
            .unwrap();
    }
    let _ = test_clock
        .borrow_mut()
        .advance_time(UnixNanos::from(50), true);

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<ExecutionEvent>();
    nautilus_common::live::runner::replace_exec_event_sender(tx);
    client.start().unwrap();

    for (instrument, _, _, _) in &instruments {
        let quote = QuoteTick::new(
            instrument.id(),
            Price::new(0.40, 3),
            Price::new(0.41, 3),
            Quantity::new(100.0, 2),
            Quantity::new(100.0, 2),
            UnixNanos::default(),
            UnixNanos::default(),
        );
        client.process_quote_tick(&quote).unwrap();
    }

    for (idx, (instrument, _, client_order_id, _)) in instruments.iter().enumerate() {
        submit_market_open_order(
            &client,
            &cache,
            trader_id,
            instrument,
            client_order_id,
            10 + idx as u64,
        );
    }

    let mut seeded_positions = ahash::AHashSet::new();

    for event in std::iter::from_fn(|| rx.try_recv().ok()) {
        let ExecutionEvent::Order(OrderEventAny::Filled(fill)) = event else {
            continue;
        };

        if let Some((instrument, _, client_order_id, position_id)) =
            instruments
                .iter()
                .find(|(_, _, expected_client_order_id, _)| {
                    fill.client_order_id.as_str() == *expected_client_order_id
                })
        {
            seed_binary_option_position_from_fill(&cache, instrument, fill, position_id);
            seeded_positions.insert(*client_order_id);
        }
    }

    assert_eq!(
        seeded_positions.len(),
        instruments.len(),
        "expected one opened position per instrument before settlement"
    );

    let _ = test_clock
        .borrow_mut()
        .advance_time(UnixNanos::from(200), true);

    for (idx, (instrument, close_price, _, _)) in instruments.iter().enumerate() {
        let close = InstrumentClose::new(
            instrument.id(),
            *close_price,
            InstrumentCloseType::ContractExpired,
            UnixNanos::from(300 + idx as u64),
            UnixNanos::from(300 + idx as u64),
        );
        msgbus::send_data(
            MessagingSwitchboard::data_engine_process_data(),
            Data::InstrumentClose(close),
        );
    }

    let mut expiration_fills = ahash::AHashMap::new();

    for event in std::iter::from_fn(|| rx.try_recv().ok()) {
        let ExecutionEvent::Order(OrderEventAny::Filled(fill)) = event else {
            continue;
        };

        if fill.client_order_id.as_str().starts_with("EXPIRATION-") {
            expiration_fills.insert(fill.instrument_id, fill.last_px);
        }
    }

    assert_eq!(
        expiration_fills.len(),
        instruments.len(),
        "expected one settlement fill per open instrument"
    );

    for (instrument, close_price, _, _) in &instruments {
        assert_eq!(
            expiration_fills.get(&instrument.id()),
            Some(close_price),
            "expected settlement price to match InstrumentClose for {}",
            instrument.id()
        );
    }
}

#[rstest]
fn test_process_quote_tick_creates_matching_engine(
    test_context: TestContext,
    instrument: InstrumentAny,
) {
    setup_order_event_handler();

    test_context
        .cache
        .borrow_mut()
        .add_instrument(instrument.clone())
        .unwrap();

    let quote = create_quote_tick(instrument.id(), 1000.0, 1001.0);
    let result = test_context.client.process_quote_tick(&quote);

    assert!(result.is_ok());
    assert_eq!(test_context.client.matching_engine_count(), 1);
}

#[rstest]
fn test_process_quote_tick_reuses_matching_engine(
    test_context: TestContext,
    instrument: InstrumentAny,
) {
    setup_order_event_handler();

    test_context
        .cache
        .borrow_mut()
        .add_instrument(instrument.clone())
        .unwrap();

    let quote1 = create_quote_tick(instrument.id(), 1000.0, 1001.0);
    let quote2 = create_quote_tick(instrument.id(), 1002.0, 1003.0);

    test_context.client.process_quote_tick(&quote1).unwrap();
    test_context.client.process_quote_tick(&quote2).unwrap();

    assert_eq!(test_context.client.matching_engine_count(), 1);
}

#[rstest]
fn test_process_quote_tick_drops_precision_mismatch(
    test_context: TestContext,
    instrument: InstrumentAny,
) {
    setup_order_event_handler();

    test_context
        .cache
        .borrow_mut()
        .add_instrument(instrument.clone())
        .unwrap();

    let quote = create_mismatched_quote_tick(instrument.id(), 1000.0, 1001.0);
    let result = test_context.client.process_quote_tick(&quote);

    assert!(result.is_ok());
    assert_eq!(test_context.client.matching_engine_count(), 0);
}

#[rstest]
fn test_on_instrument_updates_engine_precision(
    mut test_context: TestContext,
    instrument: InstrumentAny,
) {
    setup_order_event_handler();

    test_context
        .cache
        .borrow_mut()
        .add_instrument(instrument.clone())
        .unwrap();

    let quote_before = create_quote_tick(instrument.id(), 1000.0, 1001.0);
    test_context
        .client
        .process_quote_tick(&quote_before)
        .unwrap();
    assert_eq!(test_context.client.matching_engine_count(), 1);

    let updated_instrument = updated_instrument_with_price_precision_3(instrument);
    test_context
        .cache
        .borrow_mut()
        .add_instrument(updated_instrument.clone())
        .unwrap();
    test_context
        .client
        .on_instrument(updated_instrument.clone());

    let stale_quote = create_quote_tick(updated_instrument.id(), 1000.0, 1001.0);
    let stale_result = test_context.client.process_quote_tick(&stale_quote);
    assert!(stale_result.is_ok());

    let updated_quote =
        create_quote_tick_with_price_precision(updated_instrument.id(), 1000.0, 1001.0, 3);
    let updated_result = test_context.client.process_quote_tick(&updated_quote);
    assert!(updated_result.is_ok());
    assert_eq!(test_context.client.matching_engine_count(), 1);
}

#[rstest]
fn test_process_quote_tick_instrument_not_found(execution_client: SandboxExecutionClient) {
    setup_order_event_handler();

    let quote = create_quote_tick(InstrumentId::from("UNKNOWN.SIM"), 1000.0, 1001.0);
    let result = execution_client.process_quote_tick(&quote);

    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("not found"));
}

#[rstest]
fn test_process_trade_tick_disabled(test_context: TestContext, instrument: InstrumentAny) {
    setup_order_event_handler();

    test_context
        .cache
        .borrow_mut()
        .add_instrument(instrument.clone())
        .unwrap();

    // Config has trade_execution = false, so this should be a no-op
    let trade = TradeTick::new(
        instrument.id(),
        Price::from("1000.0"),
        Quantity::from("1.0"),
        AggressorSide::Buy,
        TradeId::new("1"),
        UnixNanos::default(),
        UnixNanos::default(),
    );

    let result = test_context.client.process_trade_tick(&trade);

    assert!(result.is_ok());
    // No matching engine created because trade_execution is disabled
    assert_eq!(test_context.client.matching_engine_count(), 0);
}

#[rstest]
fn test_process_trade_tick_drops_precision_mismatch(
    trader_id: TraderId,
    account_id: AccountId,
    instrument: InstrumentAny,
) {
    setup_order_event_handler();

    let venue = instrument.id().venue;
    let mut test_context = create_test_context_with_trade_execution(trader_id, account_id, venue);
    test_context.client.start().unwrap();
    test_context
        .cache
        .borrow_mut()
        .add_instrument(instrument.clone())
        .unwrap();

    let trade = create_mismatched_trade_tick(instrument.id());
    let result = test_context.client.process_trade_tick(&trade);

    assert!(result.is_ok());
    assert_eq!(test_context.client.matching_engine_count(), 0);
}

#[rstest]
fn test_message_handler_drops_precision_mismatched_trade(
    trader_id: TraderId,
    account_id: AccountId,
    instrument: InstrumentAny,
) {
    setup_order_event_handler();

    let venue = instrument.id().venue;
    let mut test_context = create_test_context_with_trade_execution(trader_id, account_id, venue);
    test_context
        .cache
        .borrow_mut()
        .add_instrument(instrument.clone())
        .unwrap();
    test_context.client.start().unwrap();

    let trade = create_mismatched_trade_tick(instrument.id());
    msgbus::publish_trade(
        format!("data.trades.{}.{}", instrument.id().venue, instrument.id()).into(),
        &trade,
    );

    assert_eq!(test_context.client.matching_engine_count(), 0);
    test_context.client.stop().unwrap();
}

#[rstest]
fn test_process_bar_disabled(test_context: TestContext, instrument: InstrumentAny) {
    use nautilus_model::data::{Bar, BarType};

    setup_order_event_handler();

    test_context
        .cache
        .borrow_mut()
        .add_instrument(instrument.clone())
        .unwrap();

    // Config has bar_execution = false, so this should be a no-op
    let bar_type = BarType::from(format!("{}-1-MINUTE-LAST-INTERNAL", instrument.id()));
    let bar = Bar::new(
        bar_type,
        Price::from("1000.0"),
        Price::from("1001.0"),
        Price::from("999.0"),
        Price::from("1000.5"),
        Quantity::from("100.0"),
        UnixNanos::default(),
        UnixNanos::default(),
    );

    let result = test_context.client.process_bar(&bar);

    assert!(result.is_ok());
    // No matching engine created because bar_execution is disabled
    assert_eq!(test_context.client.matching_engine_count(), 0);
}

#[rstest]
fn test_process_bar_drops_precision_mismatch(
    trader_id: TraderId,
    account_id: AccountId,
    venue: Venue,
    instrument: InstrumentAny,
) {
    setup_order_event_handler();

    let cache = Rc::new(RefCell::new(Cache::default()));
    let clock: Rc<RefCell<dyn Clock>> = Rc::new(RefCell::new(TestClock::new()));
    let mut config = create_config(trader_id, account_id, venue);
    config.bar_execution = true;

    let core = ExecutionClientCore::new(
        trader_id,
        ClientId::new("SANDBOX"),
        config.venue,
        config.oms_type,
        config.account_id,
        config.account_type,
        config.base_currency,
        cache.clone(),
    );
    let client = SandboxExecutionClient::new(core, config, clock, cache.clone());

    cache
        .borrow_mut()
        .add_instrument(instrument.clone())
        .unwrap();

    let bar_type = BarType::from(format!("{}-1-MINUTE-LAST-EXTERNAL", instrument.id()));
    let bar = Bar::new(
        bar_type,
        Price::new(1000.0, 3),
        Price::new(1001.0, 3),
        Price::new(999.0, 3),
        Price::new(1000.5, 3),
        Quantity::new(100.0, 3),
        UnixNanos::default(),
        UnixNanos::default(),
    );

    let result = client.process_bar(&bar);

    assert!(result.is_ok());
    assert_eq!(client.matching_engine_count(), 0);
}

#[rstest]
fn test_message_handler_drops_precision_mismatched_bar(
    trader_id: TraderId,
    account_id: AccountId,
    instrument: InstrumentAny,
) {
    setup_order_event_handler();

    let cache = Rc::new(RefCell::new(Cache::default()));
    let clock: Rc<RefCell<dyn Clock>> = Rc::new(RefCell::new(TestClock::new()));
    let mut config = create_config(trader_id, account_id, instrument.id().venue);
    config.bar_execution = true;

    let core = ExecutionClientCore::new(
        trader_id,
        ClientId::new("SANDBOX"),
        config.venue,
        config.oms_type,
        config.account_id,
        config.account_type,
        config.base_currency,
        cache.clone(),
    );
    let mut client = SandboxExecutionClient::new(core, config, clock, cache.clone());

    cache
        .borrow_mut()
        .add_instrument(instrument.clone())
        .unwrap();
    client.start().unwrap();

    let bar_type = BarType::from(format!("{}-1-MINUTE-LAST-EXTERNAL", instrument.id()));
    let bar = Bar::new(
        bar_type,
        Price::new(1000.0, 3),
        Price::new(1001.0, 3),
        Price::new(999.0, 3),
        Price::new(1000.5, 3),
        Quantity::new(100.0, 3),
        UnixNanos::default(),
        UnixNanos::default(),
    );
    msgbus::publish_bar(format!("data.bars.{bar_type}").into(), &bar);

    assert_eq!(client.matching_engine_count(), 0);
    client.stop().unwrap();
}

#[rstest]
fn test_reset_with_no_engines(execution_client: SandboxExecutionClient) {
    setup_order_event_handler();

    assert_eq!(execution_client.matching_engine_count(), 0);

    // Reset should work even with no engines
    execution_client.reset();

    assert_eq!(execution_client.matching_engine_count(), 0);
}

#[rstest]
fn test_client_id(execution_client: SandboxExecutionClient, client_id: ClientId) {
    assert_eq!(execution_client.client_id(), client_id);
}

#[rstest]
fn test_account_id(execution_client: SandboxExecutionClient, account_id: AccountId) {
    assert_eq!(execution_client.account_id(), account_id);
}

#[rstest]
#[tokio::test]
async fn test_generate_mass_status_returns_empty_report(
    execution_client: SandboxExecutionClient,
    client_id: ClientId,
    account_id: AccountId,
    venue: Venue,
) {
    let mass_status = execution_client
        .generate_mass_status(None)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(mass_status.client_id, client_id);
    assert_eq!(mass_status.account_id, account_id);
    assert_eq!(mass_status.venue, venue);
    assert!(mass_status.order_reports().is_empty());
    assert!(mass_status.fill_reports().is_empty());
    assert!(mass_status.position_reports().is_empty());
}

#[rstest]
fn test_config_accessor(execution_client: SandboxExecutionClient, venue: Venue) {
    let config = execution_client.config();

    assert_eq!(config.venue, venue);
    assert_eq!(config.oms_type, OmsType::Netting);
    assert_eq!(config.account_type, AccountType::Margin);
}

#[rstest]
fn test_get_account_when_none(execution_client: SandboxExecutionClient) {
    // No account in cache yet
    assert!(execution_client.get_account().is_none());
}

#[rstest]
fn test_initialized_ioc_market_order_cancels_remainder_through_live_runner(
    trader_id: TraderId,
    account_id: AccountId,
    instrument: InstrumentAny,
) {
    *msgbus::get_message_bus().borrow_mut() = MessageBus::default();

    let instrument_id = instrument.id();
    let mut context = create_test_context(trader_id, account_id, instrument_id.venue);
    context
        .cache
        .borrow_mut()
        .add_instrument(instrument)
        .unwrap();

    let quote = QuoteTick::new(
        instrument_id,
        Price::from("1000.00"),
        Price::from("1010.00"),
        Quantity::from("0.500"),
        Quantity::from("0.500"),
        UnixNanos::default(),
        UnixNanos::default(),
    );
    context.cache.borrow_mut().add_quote(quote).unwrap();

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<ExecutionEvent>();
    set_exec_event_sender(tx);
    context.client.start().unwrap();
    context.client.process_quote_tick(&quote).unwrap();

    let order = OrderTestBuilder::new(OrderType::Market)
        .instrument_id(instrument_id)
        .side(OrderSide::Buy)
        .quantity(Quantity::from("1.500"))
        .time_in_force(TimeInForce::Ioc)
        .client_order_id(ClientOrderId::from("O-IOC-INITIALIZED"))
        .build();
    assert_eq!(order.status(), OrderStatus::Initialized);
    context
        .cache
        .borrow_mut()
        .add_order(order.clone(), None, Some(context.client.client_id()), false)
        .unwrap();

    context
        .client
        .submit_order(SubmitOrder::from_order(
            &order,
            trader_id,
            Some(context.client.client_id()),
            None,
            UUID4::new(),
            UnixNanos::default(),
        ))
        .unwrap();

    let events: Vec<OrderEventAny> = std::iter::from_fn(|| rx.try_recv().ok())
        .filter_map(|event| match event {
            ExecutionEvent::Order(order_event) => Some(order_event),
            _ => None,
        })
        .collect();
    assert_eq!(events.len(), 3);
    let OrderEventAny::Submitted(submitted) = &events[0] else {
        panic!("Expected OrderSubmitted, was {:?}", events[0]);
    };
    assert_eq!(submitted.client_order_id, order.client_order_id());
    let OrderEventAny::Filled(fill) = &events[1] else {
        panic!("Expected OrderFilled, was {:?}", events[1]);
    };
    assert_eq!(fill.client_order_id, order.client_order_id());
    assert_eq!(fill.last_px, Price::from("1010.00"));
    assert_eq!(fill.last_qty, Quantity::from("0.500"));
    let OrderEventAny::Canceled(canceled) = &events[2] else {
        panic!("Expected OrderCanceled, was {:?}", events[2]);
    };
    assert_eq!(canceled.client_order_id, order.client_order_id());

    for event in &events {
        context.cache.borrow_mut().update_order(event).unwrap();
    }

    let cached_order = context
        .cache
        .borrow()
        .order(&order.client_order_id())
        .unwrap()
        .clone();
    assert_eq!(cached_order.status(), OrderStatus::Canceled);
    assert_eq!(cached_order.filled_qty(), Quantity::from("0.500"));
    assert_eq!(cached_order.leaves_qty(), Quantity::from("1.000"));

    context.client.stop().unwrap();
}

#[rstest]
#[case(None, None)]
#[case(Some("SANDBOX-A"), Some(OrderSide::Buy))]
#[case(Some("SANDBOX-B"), Some(OrderSide::Sell))]
fn test_cancel_all_orders_routes_by_client_account_and_side(
    trader_id: TraderId,
    instrument: InstrumentAny,
    #[case] selected_client: Option<&str>,
    #[case] selected_side: Option<OrderSide>,
) {
    struct SeededOrder {
        client_order_id: ClientOrderId,
        client_id: ClientId,
        account_id: AccountId,
        strategy_id: StrategyId,
        instrument_id: InstrumentId,
        side: OrderSide,
        status: OrderStatus,
    }

    *msgbus::get_message_bus().borrow_mut() = MessageBus::default();

    let InstrumentAny::CryptoPerpetual(mut other) = instrument.clone() else {
        panic!("Expected crypto perpetual fixture");
    };
    other.id = InstrumentId::from("BTCUSDT-PERP.BINANCE");
    other.raw_symbol = "BTCUSDT".into();
    let other = InstrumentAny::CryptoPerpetual(other);
    let instrument_id = instrument.id();
    let other_instrument_id = other.id();
    let venue = instrument_id.venue;
    let client_a_id = ClientId::new("SANDBOX-A");
    let client_b_id = ClientId::new("SANDBOX-B");
    let account_a_id = AccountId::from("BINANCE-001");
    let account_b_id = AccountId::from("BINANCE-002");
    let cache = Rc::new(RefCell::new(Cache::default()));
    let clock: Rc<RefCell<dyn Clock>> = Rc::new(RefCell::new(TestClock::new()));

    {
        let mut cache = cache.borrow_mut();
        cache.add_instrument(instrument).unwrap();
        cache.add_instrument(other).unwrap();
        cache
            .add_quote(create_quote_tick(instrument_id, 1000.0, 1001.0))
            .unwrap();
        cache
            .add_quote(create_quote_tick(other_instrument_id, 2000.0, 2001.0))
            .unwrap();
    }

    let create_client = |client_id: ClientId, account_id: AccountId| {
        let config = create_config(trader_id, account_id, venue);
        let core = ExecutionClientCore::new(
            trader_id,
            client_id,
            venue,
            config.oms_type,
            config.account_id,
            config.account_type,
            config.base_currency,
            cache.clone(),
        );
        SandboxExecutionClient::new(core, config, clock.clone(), cache.clone())
    };
    let mut client_a = create_client(client_a_id, account_a_id);
    let mut client_b = create_client(client_b_id, account_b_id);
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<ExecutionEvent>();
    set_exec_event_sender(tx);
    client_a.start().unwrap();
    client_b.start().unwrap();

    let mut orders = Vec::new();
    let resting_orders = [
        (
            &client_a,
            client_a_id,
            account_a_id,
            StrategyId::from("STRATEGY-A-001"),
            instrument_id,
            OrderSide::Buy,
            ClientOrderId::from("O-A-BUY-ACCEPTED"),
        ),
        (
            &client_a,
            client_a_id,
            account_a_id,
            StrategyId::from("STRATEGY-A-002"),
            instrument_id,
            OrderSide::Sell,
            ClientOrderId::from("O-A-SELL-ACCEPTED"),
        ),
        (
            &client_a,
            client_a_id,
            account_a_id,
            StrategyId::from("STRATEGY-A-001"),
            other_instrument_id,
            OrderSide::Buy,
            ClientOrderId::from("O-A-OTHER-BUY-ACCEPTED"),
        ),
        (
            &client_b,
            client_b_id,
            account_b_id,
            StrategyId::from("STRATEGY-B-001"),
            instrument_id,
            OrderSide::Buy,
            ClientOrderId::from("O-B-BUY-ACCEPTED"),
        ),
        (
            &client_b,
            client_b_id,
            account_b_id,
            StrategyId::from("STRATEGY-B-002"),
            instrument_id,
            OrderSide::Sell,
            ClientOrderId::from("O-B-SELL-ACCEPTED"),
        ),
    ];

    for (client, client_id, account_id, strategy_id, order_instrument_id, side, order_id) in
        resting_orders
    {
        let price = match side {
            OrderSide::Buy => Price::from("900.00"),
            OrderSide::Sell => Price::from("1100.00"),
        };
        let order = OrderTestBuilder::new(OrderType::Limit)
            .trader_id(trader_id)
            .strategy_id(strategy_id)
            .instrument_id(order_instrument_id)
            .client_order_id(order_id)
            .side(side)
            .price(price)
            .quantity(Quantity::from("1.000"))
            .build();
        cache
            .borrow_mut()
            .add_order(order.clone(), None, Some(client_id), false)
            .unwrap();
        client
            .submit_order(SubmitOrder::from_order(
                &order,
                trader_id,
                Some(client_id),
                None,
                UUID4::new(),
                UnixNanos::default(),
            ))
            .unwrap();
        let events = apply_order_events_from_channel(&cache, &mut rx);
        assert_eq!(events.len(), 2);
        assert!(matches!(events[0], OrderEventAny::Submitted(_)));
        assert!(matches!(events[1], OrderEventAny::Accepted(_)));
        orders.push(SeededOrder {
            client_order_id: order_id,
            client_id,
            account_id,
            strategy_id,
            instrument_id: order_instrument_id,
            side,
            status: OrderStatus::Accepted,
        });
    }

    for (client_id, account_id, strategy_id, side, order_id) in [
        (
            client_a_id,
            account_a_id,
            StrategyId::from("STRATEGY-A-002"),
            OrderSide::Buy,
            ClientOrderId::from("O-A-BUY-SUBMITTED"),
        ),
        (
            client_b_id,
            account_b_id,
            StrategyId::from("STRATEGY-B-001"),
            OrderSide::Sell,
            ClientOrderId::from("O-B-SELL-SUBMITTED"),
        ),
    ] {
        let order = OrderTestBuilder::new(OrderType::Limit)
            .trader_id(trader_id)
            .strategy_id(strategy_id)
            .instrument_id(instrument_id)
            .client_order_id(order_id)
            .side(side)
            .price(Price::from("950.00"))
            .quantity(Quantity::from("2.000"))
            .build();
        let submitted = TestOrderEventStubs::submitted(&order, account_id);
        let mut cache = cache.borrow_mut();
        cache
            .add_order(order, None, Some(client_id), false)
            .unwrap();
        cache.update_order(&submitted).unwrap();
        orders.push(SeededOrder {
            client_order_id: order_id,
            client_id,
            account_id,
            strategy_id,
            instrument_id,
            side,
            status: OrderStatus::Submitted,
        });
    }

    let mut engine = ExecutionEngine::new(clock, cache.clone(), None);
    engine.register_client(Box::new(client_a)).unwrap();
    engine.register_default_client(Box::new(client_b));
    let command_client = selected_client.map(ClientId::new);
    let routed_client = command_client.unwrap_or(client_a_id);
    let routed_account = if routed_client == client_a_id {
        account_a_id
    } else {
        account_b_id
    };
    let command = CancelAllOrders::new(
        trader_id,
        command_client,
        StrategyId::from("CANCEL-CALLER-001"),
        instrument_id,
        selected_side,
        UUID4::new(),
        UnixNanos::default(),
        None,
        None,
    );
    engine.execute(TradingCommand::CancelAllOrders(command));

    let events = apply_order_events_from_channel(&cache, &mut rx);
    let canceled: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            OrderEventAny::Canceled(canceled) => Some(canceled),
            _ => None,
        })
        .collect();
    let expected_ids: ahash::AHashSet<_> = orders
        .iter()
        .filter(|order| {
            order.client_id == routed_client
                && order.account_id == routed_account
                && order.instrument_id == instrument_id
                && selected_side.is_none_or(|side| order.side == side)
        })
        .map(|order| order.client_order_id)
        .collect();
    let actual_ids: ahash::AHashSet<_> =
        canceled.iter().map(|event| event.client_order_id).collect();

    assert_eq!(actual_ids, expected_ids);

    for event in canceled {
        let order = orders
            .iter()
            .find(|order| order.client_order_id == event.client_order_id)
            .unwrap();
        assert_eq!(event.strategy_id, order.strategy_id);
        assert_eq!(event.account_id, Some(routed_account));
    }
    let cache = cache.borrow();
    for order in &orders {
        let cached = cache.order(&order.client_order_id).unwrap();
        let expected_status = if expected_ids.contains(&order.client_order_id) {
            OrderStatus::Canceled
        } else {
            order.status
        };
        assert_eq!(cached.status(), expected_status);
        assert_eq!(cached.strategy_id(), order.strategy_id);
        assert_eq!(cached.account_id(), Some(order.account_id));
        assert_eq!(
            cache.client_id(&order.client_order_id),
            Some(&order.client_id)
        );
    }
    drop(cache);
    engine.stop();
}

// Regression test for https://github.com/nautechsystems/nautilus_trader/issues/3732
//
// The exec_engine_execute handler holds an immutable borrow on the ExecutionEngine.
// Without the fix, the sandbox client and matching engine synchronously dispatch order
// events back through msgbus to exec_engine_process, which tries borrow_mut() on the
// same RefCell and panics with "RefCell already borrowed".
//
// The fix routes sandbox events through the async runner channel so they are processed
// in the next iteration, after the borrow is released.
#[rstest]
fn test_submit_order_through_exec_engine_no_reentrant_panic(
    trader_id: TraderId,
    instrument: InstrumentAny,
) {
    let venue = Venue::new("BINANCE");
    let account_id = AccountId::from("BINANCE-001");
    let client_id = ClientId::new("SANDBOX");

    let cache = Rc::new(RefCell::new(Cache::default()));
    let clock: Rc<RefCell<dyn Clock>> = Rc::new(RefCell::new(TestClock::new()));

    cache
        .borrow_mut()
        .add_instrument(instrument.clone())
        .unwrap();

    let instrument_id = instrument.id();
    let quote = create_quote_tick(instrument_id, 1000.0, 1001.0);
    cache.borrow_mut().add_quote(quote).unwrap();

    // Wire up exec engine with registered msgbus handlers
    let engine = Rc::new(RefCell::new(ExecutionEngine::new(
        clock.clone(),
        cache.clone(),
        None,
    )));
    ExecutionEngine::register_msgbus_handlers(&engine);

    // Initialize the exec event sender (simulates the async runner)
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<ExecutionEvent>();
    set_exec_event_sender(tx);

    // Create and register the sandbox client (venue must match the instrument)
    let usd = Currency::USD();
    let config = SandboxExecutionClientConfig {
        account_id,
        venue,
        starting_balances: vec![Money::new(100_000.0, usd)],
        base_currency: Some(usd),
        oms_type: OmsType::Netting,
        account_type: AccountType::Margin,
        default_leverage: Decimal::ONE,
        leverages: ahash::AHashMap::new(),
        book_type: BookType::L1_MBP,
        fee_model: None,
        fill_model: None,
        frozen_account: false,
        bar_execution: false,
        trade_execution: false,
        reject_stop_orders: true,
        support_gtd_orders: true,
        support_contingent_orders: true,
        use_position_ids: true,
        use_random_ids: false,
        use_reduce_only: true,
        queue_position: false,
        liquidity_consumption: false,
        bar_adaptive_high_low_ordering: false,
        use_market_order_acks: false,
        oto_full_trigger: false,
        price_protection_points: 0,
    };
    let core = ExecutionClientCore::new(
        trader_id,
        client_id,
        venue,
        config.oms_type,
        config.account_id,
        config.account_type,
        config.base_currency,
        cache.clone(),
    );
    let mut sandbox_client =
        SandboxExecutionClient::new(core, config, clock.clone(), cache.clone());
    sandbox_client.start().unwrap();
    engine
        .borrow_mut()
        .register_client(Box::new(sandbox_client))
        .unwrap();

    // Build and cache the order
    let order = OrderTestBuilder::new(OrderType::Market)
        .instrument_id(instrument_id)
        .side(OrderSide::Buy)
        .quantity(Quantity::from("0.001"))
        .build();
    cache
        .borrow_mut()
        .add_order(order.clone(), None, Some(client_id), false)
        .unwrap();

    // Submit through the exec engine endpoint (this panicked before the fix)
    let ts = clock.borrow().timestamp_ns();
    let submit =
        SubmitOrder::from_order(&order, trader_id, Some(client_id), None, UUID4::new(), ts);
    let endpoint = MessagingSwitchboard::exec_engine_execute();
    msgbus::send_trading_command(endpoint, TradingCommand::SubmitOrder(submit));

    // Verify events arrived through the channel instead of re-entering the engine
    let mut events = Vec::new();
    while let Ok(event) = rx.try_recv() {
        events.push(event);
    }
    assert!(
        !events.is_empty(),
        "Expected order events through the exec event channel"
    );
}

#[rstest]
fn test_batch_reduce_only_modifies_share_pending_position_quantity(
    trader_id: TraderId,
    instrument: InstrumentAny,
    mut margin_account_state: AccountState,
    #[values(false, true)] first_reduce_only: bool,
) {
    let venue = instrument.id().venue;
    let account_id = AccountId::from("BINANCE-001");
    let client_id = ClientId::from("SANDBOX");
    let TestContext { mut client, cache } = create_test_context(trader_id, account_id, venue);
    margin_account_state.account_id = account_id;
    cache
        .borrow_mut()
        .add_account(AccountAny::Margin(MarginAccount::new(
            margin_account_state,
            true,
        )))
        .unwrap();
    cache
        .borrow_mut()
        .add_instrument(instrument.clone())
        .unwrap();
    cache
        .borrow_mut()
        .add_quote(create_quote_tick(instrument.id(), 1000.0, 1001.0))
        .unwrap();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    nautilus_common::live::runner::replace_exec_event_sender(tx);
    client.start().unwrap();
    let mut engine =
        ExecutionEngine::new(Rc::new(RefCell::new(TestClock::new())), cache.clone(), None);
    engine.register_client(Box::new(client)).unwrap();
    let opening = OrderTestBuilder::new(OrderType::Market)
        .trader_id(trader_id)
        .instrument_id(instrument.id())
        .client_order_id(ClientOrderId::from("OPEN-SHORT"))
        .side(OrderSide::Sell)
        .quantity(Quantity::from("0.500"))
        .build();
    let closing = [("CLOSE-FIRST", "0.400"), ("CLOSE-SECOND", "0.300")].map(|(id, qty)| {
        OrderTestBuilder::new(OrderType::Limit)
            .trader_id(trader_id)
            .instrument_id(instrument.id())
            .client_order_id(ClientOrderId::from(id))
            .side(OrderSide::Buy)
            .quantity(Quantity::from(qty))
            .price(Price::from("999.00"))
            .reduce_only(id != "CLOSE-FIRST" || first_reduce_only)
            .build()
    });

    for order in [&opening, &closing[0], &closing[1]] {
        cache
            .borrow_mut()
            .add_order(order.clone(), None, Some(client_id), false)
            .unwrap();
        engine.execute(TradingCommand::SubmitOrder(SubmitOrder::from_order(
            order,
            trader_id,
            Some(client_id),
            None,
            UUID4::new(),
            UnixNanos::default(),
        )));

        while let Ok(event) = rx.try_recv() {
            if let ExecutionEvent::Order(event) = event {
                engine.process(&event);
            }
        }
    }
    {
        let cache = cache.borrow();
        let position = cache
            .position_for_order(&opening.client_order_id())
            .unwrap();
        assert_eq!(
            (position.side, position.quantity),
            (PositionSide::Short, Quantity::from("0.500"))
        );

        for order in &closing {
            assert_eq!(
                cache.order(&order.client_order_id()).unwrap().status(),
                OrderStatus::Accepted
            );
        }
    }
    let modifies = closing
        .iter()
        .map(|order| {
            ModifyOrder::new(
                trader_id,
                Some(client_id),
                order.strategy_id(),
                instrument.id(),
                order.client_order_id(),
                None,
                None,
                Some(Price::from("1002.00")),
                None,
                UUID4::new(),
                UnixNanos::from(1),
                None,
                None,
            )
        })
        .collect();
    engine.execute(TradingCommand::ModifyOrders(BatchModifyOrders::new(
        trader_id,
        Some(client_id),
        opening.strategy_id(),
        instrument.id(),
        modifies,
        UUID4::new(),
        UnixNanos::from(1),
        None,
        None,
    )));
    let mut fills = Vec::new();

    while let Ok(event) = rx.try_recv() {
        if let ExecutionEvent::Order(event) = event {
            if let OrderEventAny::Filled(fill) = &event {
                fills.push((fill.client_order_id, fill.last_qty));
            }
            engine.process(&event);
        }
    }
    let cache = cache.borrow();
    let position = cache
        .position_for_order(&opening.client_order_id())
        .unwrap();
    assert_eq!(
        fills,
        vec![
            (closing[0].client_order_id(), Quantity::from("0.400")),
            (closing[1].client_order_id(), Quantity::from("0.100")),
        ]
    );
    assert_eq!(
        (position.side, position.quantity),
        (PositionSide::Flat, Quantity::from("0.000"))
    );

    for order in &closing {
        assert_eq!(
            cache.order(&order.client_order_id()).unwrap().status(),
            OrderStatus::Filled
        );
    }
    drop(position);
    drop(cache);
    engine.stop();
}

#[rstest]
#[tokio::test]
async fn test_account_capture_keeps_resting_orders_from_every_actual_matching_engine(
    trader_id: TraderId,
    account_id: AccountId,
) {
    use nautilus_model::instruments::stubs::currency_pair_btcusdt;
    *msgbus::get_message_bus().borrow_mut() = MessageBus::default();
    let instruments = [
        InstrumentAny::CryptoPerpetual(crypto_perpetual_ethusdt()),
        InstrumentAny::CurrencyPair(currency_pair_btcusdt()),
    ];
    let mut context =
        create_test_context_with(trader_id, account_id, instruments[0].id().venue, |config| {
            config.base_currency = Some(Currency::USDT());
            config.starting_balances = vec![Money::from("100000 USDT")];
        });
    setup_account_state_handler(context.cache.clone());
    let (sender, mut events) = tokio::sync::mpsc::unbounded_channel();
    nautilus_common::live::runner::replace_exec_event_sender(sender);
    context.client.start().unwrap();
    context.client.connect().await.unwrap();
    for (index, instrument) in instruments.iter().enumerate() {
        context
            .cache
            .borrow_mut()
            .add_instrument(instrument.clone())
            .unwrap();
        let price =
            Price::from_decimal_dp(Decimal::from(100), instrument.price_precision()).unwrap();
        let size =
            Quantity::from_decimal_dp(Decimal::from(100), instrument.size_precision()).unwrap();
        context
            .client
            .process_quote_tick(&QuoteTick::new(
                instrument.id(),
                price,
                price,
                size,
                size,
                UnixNanos::from(1),
                UnixNanos::from(1),
            ))
            .unwrap();
        let order = OrderTestBuilder::new(OrderType::Limit)
            .trader_id(trader_id)
            .instrument_id(instrument.id())
            .client_order_id(ClientOrderId::from(format!("CAPTURE-RESTING-{index}")))
            .side(OrderSide::Buy)
            .quantity(Quantity::from_decimal_dp(Decimal::ONE, instrument.size_precision()).unwrap())
            .price(Price::from_decimal_dp(Decimal::from(50), instrument.price_precision()).unwrap())
            .build();
        context
            .cache
            .borrow_mut()
            .add_order(order.clone(), None, Some(context.client.client_id()), false)
            .unwrap();
        context
            .client
            .submit_order(SubmitOrder::from_order(
                &order,
                trader_id,
                Some(context.client.client_id()),
                None,
                UUID4::new(),
                UnixNanos::from(2),
            ))
            .unwrap();
        apply_order_events_from_channel(&context.cache, &mut events);
    }
    assert_eq!(context.client.matching_engine_count(), 2);
    let captured = context
        .client
        .account_capture_probe()
        .unwrap()
        .capture()
        .unwrap();
    assert_eq!(captured.orders.len(), 2);
    assert!(captured.orders.iter().all(|order| order.in_matching_engine
        && !order.pending
        && order.price.as_deref() == Some("50")));
    assert_ne!(
        captured.orders[0].instrument_id,
        captured.orders[1].instrument_id
    );
    // A sole execution client can also receive default-routed commands. Until
    // account assignment, a different instrument venue cannot prove exclusion.
    let intent_id = ClientOrderId::from("CAPTURE-UNASSIGNED");
    let intent = OrderTestBuilder::new(OrderType::Market)
        .trader_id(trader_id)
        .instrument_id(InstrumentId::from("ETHUSDT.UNROUTED"))
        .client_order_id(intent_id)
        .side(OrderSide::Buy)
        .quantity(Quantity::from("1"))
        .build();
    context
        .cache
        .borrow_mut()
        .add_order(intent, None, Some(context.client.client_id()), false)
        .unwrap();
    let with_intent = context
        .client
        .account_capture_probe()
        .unwrap()
        .capture()
        .unwrap();
    let pending = with_intent
        .orders
        .iter()
        .find(|order| order.client_order_id == intent_id.as_str())
        .expect("An unassigned route is potential account work, not an empty order set");
    assert!(pending.pending);
    assert!(!pending.in_matching_engine);
    context.client.stop().unwrap();
}

#[cfg(feature = "python")]
#[rstest]
#[case::normal_lifecycle(false)]
#[case::invalid_connect_inside_writer(true)]
#[tokio::test]
async fn test_kernel_connect_snapshot_callbacks_keep_account_updates_and_queue_queries(
    #[case] connect_inside_writer: bool,
) {
    use std::{
        any::Any,
        cell::Cell,
        collections::VecDeque,
        sync::{Arc, Mutex, mpsc},
    };

    use nautilus_common::{
        clients::ExecutionAccountCaptureError,
        enums::Environment,
        live::runner::replace_exec_event_sender,
        logging::logger::LoggerConfig,
        messages::execution::QueryAccount,
        msgbus::{
            BusMessage, BusPayloadType, BusTap, MessageBusConfig, MessageBusExternalEgress,
            mstr::{Endpoint, MStr, Topic},
        },
        runner::{
            SyncTradingCommandSender, drain_trading_cmd_queue, replace_exec_cmd_sender,
            trading_cmd_queue_is_empty,
        },
    };
    use nautilus_live::{node::LiveNode, runner::AsyncRunner};
    use nautilus_model::{accounts::Account, events::PortfolioSnapshot};
    use pyo3::types::{PyCFunction, PyModule, PyModuleMethods, PyTuple};

    struct LifecycleEgress {
        sender: mpsc::Sender<BusMessage>,
        closed: Cell<bool>,
    }

    impl MessageBusExternalEgress for LifecycleEgress {
        fn is_closed(&self) -> bool {
            self.closed.get()
        }

        fn publish(&self, message: BusMessage) {
            self.sender.send(message).unwrap();
        }

        fn close(&mut self) {
            self.closed.set(true);
        }
    }

    struct LifecycleTap {
        engine: Rc<RefCell<ExecutionEngine>>,
        commands: Rc<RefCell<Vec<(String, QueryAccount)>>>,
        accounts: Rc<RefCell<Vec<AccountState>>>,
        query_callback: pyo3::Py<pyo3::PyAny>,
    }

    impl BusTap for LifecycleTap {
        fn on_publish(&self, _topic: MStr<Topic>, _message: &dyn Any) {}

        fn on_send(&self, endpoint: MStr<Endpoint>, message: &dyn Any) {
            if let Some(event) = message.downcast_ref::<AccountState>() {
                self.accounts.borrow_mut().push(event.clone());
            }
            if let Some(TradingCommand::QueryAccount(command)) =
                message.downcast_ref::<TradingCommand>()
            {
                self.commands
                    .borrow_mut()
                    .push((endpoint.to_string(), command.clone()));
                if endpoint == MessagingSwitchboard::exec_engine_execute() {
                    assert!(self.engine.try_borrow_mut().is_ok());
                    Python::attach(|py| {
                        self.query_callback
                            .call1(py, (command.command_id.to_string(),))
                            .unwrap();
                    });
                }
            }
        }
    }

    Python::initialize();
    let trader_id = TraderId::from("LIFECYCLE-001");
    let account_id = AccountId::from("SIM-001");
    let client_id = ClientId::from("LIFECYCLE");
    let test_clock = Rc::new(RefCell::new(TestClock::new()));
    test_clock.borrow_mut().set_time(UnixNanos::from(100));
    let clock: Rc<RefCell<dyn Clock>> = test_clock.clone();
    let factory_clock = clock.clone();
    let mut node = LiveNode::builder(trader_id, Environment::Sandbox)
        .unwrap()
        .with_clock_factory(move || factory_clock.clone())
        .with_logging(LoggerConfig {
            bypass_logging: true,
            ..Default::default()
        })
        .with_reconciliation(false)
        .build()
        .unwrap();
    let cache = node.kernel().cache();
    let engine = node.kernel().exec_engine().clone();
    let (event_sender, mut events) = tokio::sync::mpsc::unbounded_channel();
    replace_exec_event_sender(event_sender);
    replace_exec_cmd_sender(Arc::new(SyncTradingCommandSender));
    let config = SandboxExecutionClientConfig {
        account_id,
        venue: Venue::from("SIM"),
        account_type: AccountType::Cash,
        base_currency: Some(Currency::USDC()),
        starting_balances: vec![Money::from("100000 USDC")],
        ..Default::default()
    };
    let core = ExecutionClientCore::new(
        trader_id,
        client_id,
        config.venue,
        config.oms_type,
        account_id,
        config.account_type,
        config.base_currency,
        cache.clone(),
    );
    let client = SandboxExecutionClient::new(core, config, clock, cache.clone());
    let probe = client.account_capture_probe().unwrap();
    assert_eq!(
        probe.capture().unwrap_err(),
        ExecutionAccountCaptureError::AccountUnavailable
    );
    engine
        .borrow_mut()
        .register_client(Box::new(client))
        .unwrap();
    engine.borrow_mut().start();

    let query_ids = [UUID4::new(), UUID4::new()];
    let queries = Mutex::new(VecDeque::from([
        QueryAccount::new(
            trader_id,
            Some(client_id),
            account_id,
            query_ids[0],
            UnixNanos::from(100),
            None,
            None,
        ),
        QueryAccount::new(
            trader_id,
            Some(client_id),
            account_id,
            query_ids[1],
            UnixNanos::from(300),
            None,
            None,
        ),
    ]));
    let module = Python::attach(|py| {
        let module = PyModule::new(py, "lifecycle_portfolio_observer").unwrap();
        let enqueue = PyCFunction::new_closure(
            py,
            None,
            None,
            move |args: &pyo3::Bound<'_, PyTuple>, _kwargs| -> pyo3::PyResult<()> {
                let cause = args.get_item(0)?.extract::<String>()?;
                let mut query =
                    queries.lock().unwrap().pop_front().ok_or_else(|| {
                        pyo3::exceptions::PyRuntimeError::new_err("no query remains")
                    })?;
                query.causation_id = Some(
                    cause
                        .parse()
                        .map_err(pyo3::exceptions::PyValueError::new_err)?,
                );
                msgbus::send_trading_command(
                    MessagingSwitchboard::exec_engine_queue_execute(),
                    TradingCommand::QueryAccount(query),
                );
                Ok(())
            },
        )
        .unwrap();
        module.setattr("enqueue_query", enqueue).unwrap();
        py.run(
            c_str!(
                r#"
import sqlite3
import tempfile
import threading
from pathlib import Path

directory = tempfile.TemporaryDirectory()
database = Path(directory.name) / "lifecycle.sqlite3"
with sqlite3.connect(database) as connection:
    connection.execute("CREATE TABLE observations (kind TEXT, identity TEXT)")
writer = None
observed = []
failures = []
worker_trace = []
executed_queries = []

def begin_outer_writer():
    global writer
    writer = sqlite3.connect(database, timeout=0)
    writer.execute("BEGIN IMMEDIATE")

def close_outer_writer():
    global writer
    if writer is not None:
        writer.rollback()
        writer.close()
        writer = None

def observe(snapshot_id, account_event_id, balance, timestamp, engine_borrowed):
    connection = sqlite3.connect(database, timeout=0)
    try:
        try:
            connection.execute("BEGIN IMMEDIATE")
        except sqlite3.OperationalError as error:
            failures.append((error.sqlite_errorcode, str(error)))
            return
        connection.execute("INSERT INTO observations VALUES (?, ?)", ("snapshot", snapshot_id))
        observed.append((snapshot_id, account_event_id, balance, timestamp, engine_borrowed))
        if timestamp == 100:
            checked = threading.Barrier(2)
            released = threading.Barrier(2)

            def competing_writer():
                other = sqlite3.connect(database, timeout=0)
                try:
                    try:
                        other.execute("BEGIN IMMEDIATE")
                        worker_trace.append("unexpected_writer")
                    except sqlite3.OperationalError as error:
                        worker_trace.append(("locked", error.sqlite_errorcode))
                finally:
                    other.close()
                checked.wait(timeout=5)
                released.wait(timeout=5)
                with sqlite3.connect(database, timeout=0) as other:
                    other.execute("BEGIN IMMEDIATE")
                    other.execute("INSERT INTO observations VALUES ('worker', 'after_callback_writer')")
                worker_trace.append("released")

            worker = threading.Thread(target=competing_writer)
            worker.start()
            checked.wait(timeout=5)
            enqueue_query(snapshot_id)
            connection.commit()
            connection.close()
            released.wait(timeout=5)
            worker.join(timeout=5)
            assert not worker.is_alive()
        else:
            connection.commit()
    finally:
        connection.close()

def on_query_execute(command_id):
    with sqlite3.connect(database, timeout=0) as connection:
        connection.execute("BEGIN IMMEDIATE")
        connection.execute("INSERT INTO observations VALUES (?, ?)", ("query", command_id))
    executed_queries.append(command_id)

def cleanup():
    close_outer_writer()
    directory.cleanup()
"#
            ),
            Some(&module.dict()),
            None,
        )
        .unwrap();
        if connect_inside_writer {
            module.call_method0("begin_outer_writer").unwrap();
        }
        module.unbind()
    });
    let commands = Rc::new(RefCell::new(Vec::new()));
    let accounts = Rc::new(RefCell::new(Vec::new()));
    msgbus::set_bus_tap(Rc::new(LifecycleTap {
        engine: engine.clone(),
        commands: commands.clone(),
        accounts: accounts.clone(),
        query_callback: Python::attach(|py| {
            module
                .bind(py)
                .getattr("on_query_execute")
                .unwrap()
                .unbind()
        }),
    }));
    let (egress_sender, egress_receiver) = mpsc::channel();
    msgbus::get_message_bus()
        .borrow_mut()
        .set_external_egress_config(
            Box::new(LifecycleEgress {
                sender: egress_sender,
                closed: Cell::new(false),
            }),
            &MessageBusConfig {
                external_streams: Some(vec!["engineering.lifecycle".to_owned()]),
                ..Default::default()
            },
        )
        .unwrap();
    let snapshots = Rc::new(RefCell::new(Vec::new()));
    let snapshot_records = snapshots.clone();
    let callback_errors = Rc::new(RefCell::new(Vec::new()));
    let errors = callback_errors.clone();
    let callback_cache = cache.clone();
    let callback_engine = engine.clone();
    let callback = Python::attach(|py| module.bind(py).getattr("observe").unwrap().unbind());
    let handler = TypedHandler::from(move |snapshot: &PortfolioSnapshot| {
        let account = callback_cache
            .borrow()
            .account_owned(&snapshot.account_id)
            .unwrap();
        let state = account.last_event().unwrap();
        let balance = account.balance_total(Some(Currency::USDC())).unwrap();
        assert_eq!(snapshot.total_equity, vec![balance]);
        snapshot_records.borrow_mut().push((
            snapshot.clone(),
            state.clone(),
            account.events().len(),
            callback_engine.try_borrow_mut().is_err(),
        ));
        Python::attach(|py| {
            if let Err(e) = callback.call1(
                py,
                (
                    snapshot.event_id.to_string(),
                    state.event_id.to_string(),
                    balance.to_string(),
                    snapshot.ts_event.as_u64(),
                    callback_engine.try_borrow_mut().is_err(),
                ),
            ) {
                errors.borrow_mut().push(e.to_string());
            }
        });
    });
    msgbus::subscribe_portfolio_snapshot("events.portfolio.*".into(), handler.clone(), None);
    assert!(cache.borrow().account(&account_id).is_none());

    node.kernel_mut().connect_exec_clients().await;

    assert!(engine.borrow().client_connection_status()[0].1);
    assert_eq!(engine.borrow().command_count(), 0);
    assert!(
        callback_errors.borrow().is_empty(),
        "{:?}",
        callback_errors.borrow()
    );
    assert_eq!(snapshots.borrow().len(), 1);
    assert_eq!(snapshots.borrow()[0].2, 1);
    assert!(snapshots.borrow()[0].3);
    let initial = cache.borrow().account_owned(&account_id).unwrap();
    let initial_state = initial.last_event().unwrap();
    assert_eq!(initial.events().len(), 1);
    assert_eq!(initial_state.ts_event, UnixNanos::from(100));
    assert_eq!(
        initial.balance_total(None),
        Some(Money::from("100000 USDC"))
    );
    assert_eq!(snapshots.borrow()[0].1.event_id, initial_state.event_id);
    assert_eq!(accounts.borrow().len(), 1);
    assert_eq!(accounts.borrow()[0].event_id, initial_state.event_id);
    assert_eq!(node.kernel().portfolio().snapshots(&account_id).len(), 1);
    let captured = probe.capture().unwrap();
    assert_eq!(captured.balances.len(), 1);
    assert!(captured.positions.is_empty());
    assert!(captured.orders.is_empty());
    let source_snapshot = snapshots.borrow()[0].0.clone();
    let first_egress: Vec<_> = egress_receiver.try_iter().collect();
    assert_eq!(first_egress.len(), 1);
    assert_eq!(
        first_egress[0].payload_type,
        BusPayloadType::PortfolioSnapshot
    );
    let decoded: PortfolioSnapshot = serde_json::from_slice(&first_egress[0].payload).unwrap();
    assert_eq!(decoded.event_id, source_snapshot.event_id);
    assert_eq!(decoded.total_equity, source_snapshot.total_equity);
    assert_eq!(decoded.ts_event, UnixNanos::from(100));

    Python::attach(|py| {
        let module = module.bind(py);
        let failures = module
            .getattr("failures")
            .unwrap()
            .extract::<Vec<(i32, String)>>()
            .unwrap();
        if connect_inside_writer {
            assert_eq!(failures, vec![(5, "database is locked".to_owned())]);
            eprintln!("Invalid lifecycle assembly callback: {failures:?}");
            assert!(trading_cmd_queue_is_empty());
        } else {
            assert!(failures.is_empty());
            assert!(!trading_cmd_queue_is_empty());
            assert_eq!(commands.borrow().len(), 1);
            assert_eq!(commands.borrow()[0].1.command_id, query_ids[0]);
            assert_eq!(
                commands.borrow()[0].1.causation_id,
                Some(source_snapshot.event_id)
            );
            assert_eq!(
                module
                    .getattr("executed_queries")
                    .unwrap()
                    .extract::<Vec<String>>()
                    .unwrap(),
                Vec::<String>::new()
            );
            let trace = module.getattr("worker_trace").unwrap();
            assert_eq!(trace.len().unwrap(), 2);
            assert_eq!(
                trace
                    .get_item(0)
                    .unwrap()
                    .extract::<(String, i32)>()
                    .unwrap(),
                ("locked".to_owned(), 5)
            );
            assert_eq!(
                trace.get_item(1).unwrap().extract::<String>().unwrap(),
                "released"
            );
        }
        module.call_method0("close_outer_writer").unwrap();
    });

    // A repeated delivery of the current account event does not reapply it or register twice
    AsyncRunner::handle_exec_event(ExecutionEvent::Account(initial_state.clone()));
    assert_eq!(
        cache
            .borrow()
            .account_owned(&account_id)
            .unwrap()
            .events()
            .len(),
        1
    );
    assert_eq!(snapshots.borrow().len(), 1);
    assert!(egress_receiver.try_recv().is_err());

    if !connect_inside_writer {
        test_clock.borrow_mut().set_time(UnixNanos::from(200));
        drain_trading_cmd_queue();
        let first_query = cache.borrow().account_owned(&account_id).unwrap();
        assert_eq!(first_query.events().len(), 2);
        assert_ne!(
            first_query.last_event().unwrap().event_id,
            initial_state.event_id
        );
        assert_eq!(
            first_query.last_event().unwrap().ts_event,
            UnixNanos::from(200)
        );
        test_clock.borrow_mut().set_time(UnixNanos::from(300));
        Python::attach(|py| {
            module
                .bind(py)
                .call_method1("enqueue_query", (source_snapshot.event_id.to_string(),))
                .unwrap();
        });
        drain_trading_cmd_queue();
        let second_query = cache.borrow().account_owned(&account_id).unwrap();
        assert_eq!(second_query.events().len(), 3);
        assert_ne!(
            second_query.last_event().unwrap().event_id,
            first_query.last_event().unwrap().event_id
        );
        assert_eq!(
            second_query.last_event().unwrap().ts_event,
            UnixNanos::from(300)
        );
        assert_eq!(engine.borrow().command_count(), 2);
        assert_eq!(accounts.borrow().len(), 4);
        assert_eq!(accounts.borrow()[0].event_id, accounts.borrow()[1].event_id);
        assert_ne!(accounts.borrow()[1].event_id, accounts.borrow()[2].event_id);
        assert_ne!(accounts.borrow()[2].event_id, accounts.borrow()[3].event_id);
        assert_eq!(commands.borrow().len(), 4);
        assert_eq!(commands.borrow()[1].1.command_id, query_ids[0]);
        assert_eq!(commands.borrow()[2].1.command_id, query_ids[1]);
        assert_eq!(commands.borrow()[3].1.command_id, query_ids[1]);
        assert_eq!(snapshots.borrow().len(), 1);
        assert_eq!(node.kernel().portfolio().snapshots(&account_id).len(), 1);
        assert!(egress_receiver.try_recv().is_err());
        assert!(trading_cmd_queue_is_empty());
    }
    test_clock.borrow_mut().set_time(UnixNanos::from(400));
    node.kernel_mut().finalize_stop().await.unwrap();
    assert_eq!(
        probe.capture().unwrap_err(),
        ExecutionAccountCaptureError::Retired
    );
    assert_eq!(snapshots.borrow().len(), 2);
    assert_ne!(snapshots.borrow()[1].0.event_id, source_snapshot.event_id);
    assert_eq!(snapshots.borrow()[1].0.ts_event, UnixNanos::from(400));
    assert!(!snapshots.borrow()[1].3);
    let shutdown_egress: Vec<_> = egress_receiver.try_iter().collect();
    assert_eq!(shutdown_egress.len(), 1);
    let decoded: PortfolioSnapshot = serde_json::from_slice(&shutdown_egress[0].payload).unwrap();
    assert_eq!(decoded.event_id, snapshots.borrow()[1].0.event_id);
    node.kernel_mut().finalize_stop().await.unwrap();
    assert_eq!(snapshots.borrow().len(), 2);
    assert!(
        callback_errors.borrow().is_empty(),
        "{:?}",
        callback_errors.borrow()
    );
    assert!(events.try_recv().is_err());
    msgbus::unsubscribe_portfolio_snapshot("events.portfolio.*".into(), &handler);
    msgbus::clear_bus_tap();
    Python::attach(|py| {
        module.bind(py).call_method0("cleanup").unwrap();
    });
    node.dispose();
}
