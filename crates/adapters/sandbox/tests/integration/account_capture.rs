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

//! Real node callers observe the actual sandbox account through the owner queue.

use std::time::Duration;

use nautilus_common::{enums::Environment, logging::logger::LoggerConfig};
use nautilus_live::node::{ExecutionAccountCaptureFailure, LiveNode, NodeRunMode};
use nautilus_model::{
    enums::AccountType,
    identifiers::{AccountId, ClientId, TraderId, Venue},
    types::{Currency, Money},
};
use nautilus_sandbox::{SandboxExecutionClientConfig, SandboxExecutionClientFactory};
use rstest::rstest;

#[rstest]
#[case::hosted(NodeRunMode::Hosted, Some("100 USD"), Some(("USD", "100")))]
#[case::owned(NodeRunMode::Owned, Some("100 USD"), Some(("USD", "100")))]
#[case::zero_row(NodeRunMode::Hosted, Some("0 USD"), Some(("USD", "0")))]
#[case::empty_current_account(NodeRunMode::Hosted, None, None)]
#[case::zero_non_settlement_only(NodeRunMode::Hosted, Some("0 BTC"), Some(("BTC", "0")))]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_account_capture_from_another_thread_while_the_actual_node_runs(
    #[case] mode: NodeRunMode,
    #[case] starting_balance: Option<&str>,
    #[case] expected_balance: Option<(&str, &str)>,
) {
    let owner_thread = std::thread::current().id();
    let client = ClientId::from("CAPTURE");
    let config = SandboxExecutionClientConfig {
        account_id: AccountId::from("SIM-001"),
        venue: Venue::from("SIM"),
        account_type: AccountType::Cash,
        base_currency: Some(Currency::USD()),
        starting_balances: starting_balance.into_iter().map(Money::from).collect(),
        ..Default::default()
    };
    let mut node = LiveNode::builder(TraderId::from("CAPTURE-001"), Environment::Sandbox)
        .unwrap()
        .with_logging(LoggerConfig {
            bypass_logging: true,
            ..Default::default()
        })
        .with_reconciliation(false)
        .with_delay_post_stop_secs(0)
        .with_delay_shutdown_secs(0)
        .add_simulated_exec_client(
            Some(client.to_string()),
            Box::new(SandboxExecutionClientFactory::new()),
            Box::new(config),
        )
        .unwrap()
        .build()
        .unwrap();
    let capture = node.account_capture_handle(client).unwrap();
    assert_eq!(
        capture.capture(Duration::from_secs(1)).await.unwrap_err(),
        ExecutionAccountCaptureFailure::NotRunning
    );
    let retired = capture.clone();
    let stop = node.handle();
    let inspect = tokio::spawn(async move {
        assert_ne!(std::thread::current().id(), owner_thread);
        let result = tokio::time::timeout(Duration::from_secs(5), async {
            while !stop.is_running() {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
            capture.capture(Duration::from_secs(1)).await
        })
        .await;
        stop.stop();
        result.unwrap().unwrap()
    });
    match mode {
        NodeRunMode::Owned => node.run().await.unwrap(),
        NodeRunMode::Hosted => node.run_with_mode(mode).await.unwrap(),
    }
    let observed = inspect.await.unwrap();
    assert_eq!(observed.environment, "SANDBOX");
    assert_eq!(observed.snapshot.source.client_id, client);
    assert_eq!(
        observed.snapshot.balances.len(),
        usize::from(expected_balance.is_some())
    );
    if let Some((asset_code, total)) = expected_balance {
        assert_eq!(observed.snapshot.balances[0].asset_code, asset_code);
        assert_eq!(observed.snapshot.balances[0].total, total);
    }
    assert!(observed.snapshot.no_borrowing_model);
    assert!(observed.snapshot.orders.is_empty());
    assert!(observed.snapshot.positions.is_empty());
    assert!(
        observed
            .frontier
            .iter()
            .all(|entry| entry.applied == entry.admitted)
    );
    assert_eq!(
        retired.capture(Duration::from_secs(1)).await.unwrap_err(),
        ExecutionAccountCaptureFailure::Retired
    );
    node.dispose();
}

#[rstest]
#[case::margin(false)]
#[case::spot_cash(true)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_account_capture_includes_actual_native_fill_fees_and_the_open_position(
    #[case] cash: bool,
) {
    use nautilus_common::{
        live::get_data_event_sender,
        messages::{
            DataEvent,
            execution::{SubmitOrder, TradingCommand},
        },
        msgbus::MessagingSwitchboard,
        runner::{TradingCommandMessage, get_trading_cmd_sender},
    };
    use nautilus_core::{UUID4, UnixNanos};
    use nautilus_model::{
        data::{Data, QuoteTick},
        enums::{OrderSide, OrderType},
        instruments::{
            Instrument, InstrumentAny,
            stubs::{crypto_perpetual_ethusdt, currency_pair_ethusdt},
        },
        orders::OrderTestBuilder,
        types::{Price, Quantity},
    };
    use rust_decimal::Decimal;

    let trader = TraderId::from("CAPTURE-FILL-001");
    let client = ClientId::from("CAPTURE-FILL");
    let instrument = if cash {
        let mut spot = currency_pair_ethusdt();
        spot.taker_fee = Decimal::from_str_exact("0.001").unwrap();
        InstrumentAny::CurrencyPair(spot)
    } else {
        let mut perpetual = crypto_perpetual_ethusdt();
        perpetual.taker_fee = Decimal::from_str_exact("0.001").unwrap();
        InstrumentAny::CryptoPerpetual(perpetual)
    };
    let config = SandboxExecutionClientConfig {
        account_id: AccountId::from("BINANCE-001"),
        venue: instrument.id().venue,
        account_type: if cash {
            AccountType::Cash
        } else {
            AccountType::Margin
        },
        base_currency: if cash { None } else { Some(Currency::USDT()) },
        starting_balances: if cash {
            vec![Money::from("1000 USDT"), Money::zero(Currency::ETH())]
        } else {
            vec![Money::from("1000 USDT")]
        },
        ..Default::default()
    };
    let mut node = LiveNode::builder(trader, Environment::Sandbox)
        .unwrap()
        .with_logging(LoggerConfig {
            bypass_logging: true,
            ..Default::default()
        })
        .with_reconciliation(false)
        .with_delay_post_stop_secs(0)
        .with_delay_shutdown_secs(0)
        .add_simulated_exec_client(
            Some(client.to_string()),
            Box::new(SandboxExecutionClientFactory::new()),
            Box::new(config),
        )
        .unwrap()
        .build()
        .unwrap();
    let order = OrderTestBuilder::new(OrderType::Market)
        .trader_id(trader)
        .instrument_id(instrument.id())
        .side(OrderSide::Buy)
        .quantity(Quantity::from_decimal_dp(Decimal::ONE, instrument.size_precision()).unwrap())
        .client_order_id("CAPTURE-REAL-FILL".into())
        .build();
    {
        let cache = node.kernel().cache();
        let mut cache = cache.borrow_mut();
        cache.add_instrument(instrument.clone()).unwrap();
        cache
            .add_order(order.clone(), None, Some(client), false)
            .unwrap();
    }
    let data = get_data_event_sender();
    let commands = get_trading_cmd_sender();
    let diagnostic_cache = node.kernel().cache();
    let quote_size =
        Quantity::from_decimal_dp(Decimal::from(100), instrument.size_precision()).unwrap();
    let capture = node.account_capture_handle(client).unwrap();
    let stop = node.handle();
    let inspect = async move {
        let mut last_snapshot = None;
        let result = tokio::time::timeout(Duration::from_secs(5), async {
            while !stop.is_running() {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
            data.send(DataEvent::Data(Data::Quote(QuoteTick::new(
                instrument.id(),
                Price::from("100.00"),
                Price::from("100.00"),
                quote_size,
                quote_size,
                UnixNanos::from(1),
                UnixNanos::from(1),
            ))))
            .unwrap();
            let before = capture.capture(Duration::from_secs(1)).await.unwrap();
            assert!(
                before
                    .snapshot
                    .orders
                    .iter()
                    .any(|item| item.client_order_id == "CAPTURE-REAL-FILL" && item.pending)
            );
            commands.execute(TradingCommandMessage::new(
                MessagingSwitchboard::exec_engine_execute(),
                TradingCommand::SubmitOrder(SubmitOrder::from_order(
                    &order,
                    trader,
                    Some(client),
                    None,
                    UUID4::new(),
                    UnixNanos::from(2),
                )),
            ));
            loop {
                let observed = capture.capture(Duration::from_secs(1)).await.unwrap();
                last_snapshot = Some(observed.snapshot.clone());
                let cash_fill_applied = cash
                    && observed.snapshot.orders.is_empty()
                    && observed
                        .snapshot
                        .balances
                        .iter()
                        .any(|row| row.asset_code == "USDT" && row.total != "1000");
                if cash_fill_applied || (!cash && !observed.snapshot.positions.is_empty()) {
                    break observed;
                }
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await;
        stop.stop();
        result.unwrap_or_else(|error| {
            panic!(
                "Native fill did not apply: {error}; {last_snapshot:?}; {:?}",
                diagnostic_cache
                    .borrow()
                    .order_owned(&"CAPTURE-REAL-FILL".into())
            )
        })
    };
    let (run, after) = tokio::join!(node.run_with_mode(NodeRunMode::Hosted), inspect);
    run.unwrap();
    if !cash {
        assert_eq!(after.snapshot.positions.len(), 1);
        assert_eq!(after.snapshot.positions[0].quantity, "1");
    }
    assert_eq!(
        after
            .snapshot
            .balances
            .iter()
            .find(|row| row.asset_code == "USDT")
            .unwrap()
            .total,
        if cash { "899.9" } else { "999.9" }
    );
    assert_eq!(after.snapshot.no_borrowing_model, cash);
    if cash {
        assert_eq!(
            after
                .snapshot
                .balances
                .iter()
                .find(|row| row.asset_code == "ETH")
                .unwrap()
                .total,
            "1"
        );
    }
    assert!(after.snapshot.orders.is_empty());
    assert!(
        after
            .frontier
            .iter()
            .find(|entry| entry.channel == "execution_events")
            .unwrap()
            .applied
            > 0
    );
    node.dispose();
}

#[cfg(feature = "python")]
#[rstest]
#[case::funded(Some("100 USD"), Some(("USD", "100")))]
#[case::empty_current_account(None, None)]
#[case::zero_non_settlement_only(Some("0 BTC"), Some(("BTC", "0")))]
fn test_python_caller_keeps_the_capture_transport_during_hosted_run_async(
    #[case] starting_balance: Option<&str>,
    #[case] expected_balance: Option<(&str, &str)>,
) {
    use nautilus_live::python::node::PyLiveNode;
    use pyo3::{
        Py, Python,
        types::{PyDict, PyDictMethods},
    };

    let client = ClientId::from("PY-CAPTURE");
    let config = SandboxExecutionClientConfig {
        account_id: AccountId::from("SIM-001"),
        venue: Venue::from("SIM"),
        account_type: AccountType::Cash,
        base_currency: Some(Currency::USD()),
        starting_balances: starting_balance.into_iter().map(Money::from).collect(),
        ..Default::default()
    };
    let node = LiveNode::builder(TraderId::from("PY-CAPTURE-001"), Environment::Sandbox)
        .unwrap()
        .with_logging(LoggerConfig {
            bypass_logging: true,
            ..Default::default()
        })
        .with_reconciliation(false)
        .with_delay_post_stop_secs(0)
        .with_delay_shutdown_secs(0)
        .add_simulated_exec_client(
            Some(client.to_string()),
            Box::new(SandboxExecutionClientFactory::new()),
            Box::new(config),
        )
        .unwrap()
        .build()
        .unwrap();
    Python::initialize();
    Python::attach(|py| {
        let object = Py::new(py, PyLiveNode::new(node)).unwrap();
        let locals = PyDict::new(py);
        locals.set_item("node", &object).unwrap();
        locals.set_item("client", client).unwrap();
        locals
            .set_item("expected_asset", expected_balance.map(|(asset, _)| asset))
            .unwrap();
        locals
            .set_item("expected_total", expected_balance.map(|(_, total)| total))
            .unwrap();
        py.run(
            pyo3::ffi::c_str!(
                r#"
import asyncio
import json
import uuid

async def actual_owner_capture():
    capture = node.account_capture_handle(client)
    source = json.loads(capture.source_json())
    stop = node.handle()
    run = asyncio.ensure_future(node.run_async())
    try:
        try:
            node.account_capture_handle(client)
        except RuntimeError:
            pass
        else:
            raise AssertionError("run_async did not take the node")
        while True:
            result = json.loads(await capture.capture_json(timeout_ms=1000))
            if result['status'] == 'captured':
                break
            assert result['reason'] == 'not_running', result
            await asyncio.sleep(0.001)
        assert result['capture']['snapshot']['source'] == source
        assert source['account_type'] == 'CASH'
        assert source['api_revision'] == 'execution-account-capture-v1'
        assert source['model_revision'] == 'sandbox-account-state-v1'
        assert uuid.UUID(source['incarnation']).version == 4
        assert uuid.UUID(result['capture']['node_instance_id']).version == 4
        assert result['capture']['environment'] == 'SANDBOX'
        assert result['capture']['capture_sequence'] > 0
        assert isinstance(result['capture']['snapshot']['observed_at'], int)
        assert result['capture']['snapshot']['observed_at'] > 0
        assert result['capture']['frontier']
        assert all(row['applied'] == row['admitted'] for row in result['capture']['frontier'])
        balances = result['capture']['snapshot']['balances']
        if expected_asset is None:
            assert balances == []
        else:
            assert len(balances) == 1
            assert balances[0]['asset_code'] == expected_asset
            assert balances[0]['total'] == expected_total
        assert result['capture']['snapshot']['no_borrowing_model'] is True
    finally:
        stop.stop()
        await asyncio.wait_for(run, timeout=5)
    retired = json.loads(await capture.capture_json())
    assert retired == {'status':'unavailable', 'reason':'retired'}

asyncio.run(asyncio.wait_for(actual_owner_capture(), timeout=10))
"#
            ),
            Some(&locals),
            None,
        )
        .unwrap();
        locals.clear();
    });
    nautilus_common::msgbus::get_message_bus()
        .borrow_mut()
        .dispose();
}

#[rstest]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_account_capture_queue_pressure_timeout_and_direct_stop_are_closed() {
    let client = ClientId::from("CAPTURE-QUEUE");
    let config = SandboxExecutionClientConfig {
        account_id: AccountId::from("SIM-001"),
        venue: Venue::from("SIM"),
        account_type: AccountType::Cash,
        base_currency: Some(Currency::USD()),
        starting_balances: vec![Money::from("100 USD")],
        ..Default::default()
    };
    let mut node = LiveNode::builder(TraderId::from("CAPTURE-QUEUE-001"), Environment::Sandbox)
        .unwrap()
        .with_logging(LoggerConfig {
            bypass_logging: true,
            ..Default::default()
        })
        .with_reconciliation(false)
        .with_delay_post_stop_secs(0)
        .with_delay_shutdown_secs(0)
        .add_simulated_exec_client(
            Some(client.to_string()),
            Box::new(SandboxExecutionClientFactory::new()),
            Box::new(config),
        )
        .unwrap()
        .build()
        .unwrap();
    let capture = node.account_capture_handle(client).unwrap();
    // start() intentionally does not drive the runner receivers, these requests
    // must time out rather than borrow the available native account from callers.
    node.start().await.unwrap();
    let mut callers = Vec::new();
    for _ in 0..33 {
        let capture = capture.clone();
        callers.push(tokio::spawn(async move {
            capture.capture(Duration::from_millis(20)).await
        }));
    }
    let mut full = 0;
    let mut timed_out = 0;
    for caller in callers {
        match caller.await.unwrap().unwrap_err() {
            ExecutionAccountCaptureFailure::QueueFull => full += 1,
            ExecutionAccountCaptureFailure::TimedOut => timed_out += 1,
            other => panic!("Unexpected capture result: {other:?}"),
        }
    }
    assert_eq!(full, 1);
    assert_eq!(timed_out, 32);
    node.stop().await.unwrap();
    assert_eq!(
        capture.capture(Duration::from_secs(1)).await.unwrap_err(),
        ExecutionAccountCaptureFailure::Retired
    );
    assert!(node.account_capture_handle(client).is_err());
    node.dispose();
}
