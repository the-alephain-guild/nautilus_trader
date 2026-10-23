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

use std::time::Duration;

use nautilus_core::time::get_atomic_clock_realtime;
use nautilus_model::identifiers::TraderId;
use nautilus_papertrade::factories::PapertradeExecutionClientFactory;
use serde_json::json;
mod support;
use support::{Harness, SERIAL, closed_history};

fn account_node(harness: &Harness) -> nautilus_live::node::LiveNode {
    use nautilus_common::enums::Environment;
    use nautilus_papertrade::{reports, wire::TradingState};
    let node =
        nautilus_live::node::LiveNode::builder(TraderId::from("TESTER-001"), Environment::Live)
            .unwrap()
            .with_reconciliation(true)
            .with_timeout_connection(5)
            .with_timeout_reconciliation(5)
            .with_timeout_portfolio(3)
            .with_timeout_disconnection_secs(2)
            .with_delay_post_stop_secs(0)
            .with_delay_shutdown_secs(0)
            .add_exec_client(
                None,
                Box::new(PapertradeExecutionClientFactory),
                Box::new(harness.config.clone()),
            )
            .unwrap()
            .build()
            .unwrap();
    let markets: TradingState =
        serde_json::from_value(harness.source.trading.lock().clone()).unwrap();
    for market in &markets.instruments {
        node.kernel()
            .cache()
            .borrow_mut()
            .add_instrument(
                reports::instrument(market, get_atomic_clock_realtime().get_time_ns()).unwrap(),
            )
            .unwrap();
    }
    let now = get_atomic_clock_realtime().get_time_ns();
    node.kernel()
        .cache()
        .borrow_mut()
        .add_quote(nautilus_model::data::QuoteTick::new(
            nautilus_model::identifiers::InstrumentId::from("BTC-USD-PERP.PAPERTRADE"),
            nautilus_model::types::Price::from("83000.0"),
            nautilus_model::types::Price::from("83000.0"),
            nautilus_model::types::Quantity::from(0),
            nautilus_model::types::Quantity::from(0),
            now,
            now,
        ))
        .unwrap();
    node
}

async fn run_account_node(node: &mut nautilus_live::node::LiveNode) {
    let handle = node.handle();
    let observe = async {
        let started = tokio::time::timeout(Duration::from_secs(10), async {
            while !handle.is_running() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await;
        handle.stop();
        started
    };
    let (run, started) = tokio::join!(node.run(), observe);
    run.unwrap();
    started.expect("LiveNode did not finish startup reconciliation");
}

#[rstest::rstest]
#[case(false, false)]
#[case(true, false)]
#[case(false, true)]
#[case(true, true)]
#[tokio::test]
async fn live_node_retained_position_recovers_missing_or_lagging_orders_without_economic_replay(
    #[case] lagging: bool,
    #[case] position_open: bool,
) {
    use nautilus_model::{
        enums::{OmsType, OrderStatus},
        events::OrderEventAny,
        identifiers::PositionId,
        orders::{Order, OrderAny},
    };
    use nautilus_papertrade::wire::VenuePosition;
    use rust_decimal::Decimal;
    let _serial = SERIAL.lock().await;
    let harness = Harness::new().await;
    let mut source = harness.source.wallet.borrow().clone();
    let time = source["indexedThroughTimestamp"].as_u64().unwrap() * 1000;
    let position: VenuePosition = serde_json::from_value(json!([
        "9001",
        0,
        true,
        false,
        "830000",
        "166000000000000000000",
        10,
        "0",
        time - 20_000,
        null
    ]))
    .unwrap();
    let mut record = closed_history(&position, time - 10_000);
    record["exitPriceRaw"] = json!("840000");
    record["adjustedPnlRaw"] = json!("19000000000000000000");
    record["userPaidFeeRaw"] = json!("500000000000000000");
    record["builderFeeRaw"] = json!("250000000000000000");
    source["openPositions"] = if position_open {
        json!([position])
    } else {
        json!([])
    };
    source["locked"] = if position_open {
        json!(position.margin)
    } else {
        json!("0")
    };
    source["balance"] = if position_open {
        json!("834000000000000000000")
    } else {
        json!("1018250000000000000000")
    };
    source["funds"]["availableBalance"] = source["balance"].clone();
    harness.source.wallet.send_replace(source);
    *harness.source.history.lock() = if position_open {
        json!({"items":[],"nextCursor":null})
    } else {
        json!({"items":[record],"nextCursor":null})
    };
    let mut original = account_node(&harness);
    run_account_node(&mut original).await;
    let cache = original.kernel().cache();
    let retained = cache
        .borrow()
        .position(&PositionId::from("PT-9001"))
        .unwrap()
        .cloned();
    let orders: Vec<_> = cache
        .borrow()
        .orders(None, None, None, None, None)
        .into_iter()
        .map(|o| o.clone())
        .collect();
    assert_eq!(
        retained.realized_pnl.unwrap().as_decimal(),
        if position_open {
            Decimal::ZERO
        } else {
            Decimal::new(1825, 2)
        }
    );
    assert_eq!(orders.len(), if position_open { 1 } else { 2 });
    original.dispose();
    drop(original);
    let mut restarted = account_node(&harness);
    let restored = restarted.kernel().cache();
    if lagging {
        for order in &orders {
            let events = order
                .events()
                .into_iter()
                .filter(|event| !matches!(event, OrderEventAny::Filled(_)))
                .cloned()
                .collect();
            let pending = OrderAny::from_events(events).unwrap();
            assert!(pending.filled_qty().is_zero());
            assert_eq!(pending.status(), OrderStatus::Accepted);
            restored
                .borrow_mut()
                .add_order(pending, None, None, false)
                .unwrap();
        }
        restored
            .borrow_mut()
            .add_position(&retained, OmsType::Hedging)
            .unwrap();
    } else {
        restored
            .borrow_mut()
            .add_position_without_order(&retained, OmsType::Hedging)
            .unwrap();
    }
    restored.borrow_mut().clear_index();
    restored.borrow_mut().build_index();
    run_account_node(&mut restarted).await;
    {
        let cache = restored.borrow();
        assert_eq!(
            cache.positions(None, None, None, None, None).len(),
            1,
            "Recovery created extra economic positions"
        );
        let recovered = cache.position(&retained.id).unwrap();
        assert_eq!(recovered.is_open(), position_open);
        assert_eq!(recovered.quantity, retained.quantity);
        assert_eq!(recovered.avg_px_open, retained.avg_px_open);
        assert_eq!(recovered.realized_pnl, retained.realized_pnl);
        assert_eq!(recovered.commissions(), retained.commissions());
        assert_eq!(recovered.trade_ids, retained.trade_ids);
        assert_eq!(recovered.events.len(), retained.events.len());
        for order in &orders {
            let recovered_order = cache.order(&order.client_order_id()).unwrap();
            assert_eq!(recovered_order.status(), OrderStatus::Filled);
            assert_eq!(
                recovered_order.trade_ids(),
                order.trade_ids(),
                "Recovery substituted an inferred trade for a retained venue fill"
            );
            assert_eq!(recovered_order.commissions(), order.commissions());
        }
    }
    restarted.dispose();
}

#[rstest::rstest]
#[case(false, false)]
#[case(true, false)]
#[case(false, true)]
#[tokio::test]
async fn live_node_recovers_closed_during_downtime_with_history_before_open_proof(
    #[case] conflict: bool,
    #[case] legacy_expired: bool,
) {
    use alloy_primitives::{B256, U256};
    use nautilus_model::{enums::OrderStatus, identifiers::PositionId, orders::Order};
    use nautilus_papertrade::wire::VenuePosition;
    use rust_decimal::Decimal;
    let _serial = SERIAL.lock().await;
    let harness = Harness::new().await;
    let mut source = harness.source.wallet.borrow().clone();
    let now = source["indexedThroughTimestamp"].as_u64().unwrap();
    let position: VenuePosition = serde_json::from_value(json!([
        "9001",
        0,
        true,
        false,
        "830000",
        "166000000000000000000",
        10,
        "0",
        (now - 20) * 1000,
        null
    ]))
    .unwrap();
    source["openPositions"] = json!([]);
    source["locked"] = json!("0");
    source["balance"] = json!("1018250000000000000000");
    source["funds"]["availableBalance"] = source["balance"].clone();
    harness.source.wallet.send_replace(source.clone());
    harness.source.trading.lock()["indexedThroughBlock"] = source["indexedThroughBlock"].clone();
    let entry = support::prepared_open(&harness, &position, now - 60, now + 3600 - 60);
    support::mark_attempted(&harness, &entry);
    if legacy_expired {
        support::write_legacy_expired(&harness.config.journal_path, &entry);
    }
    let mut record = closed_history(&position, (now - 10) * 1000);
    record["exitPriceRaw"] = json!("840000");
    record["adjustedPnlRaw"] = json!("19000000000000000000");
    record["userPaidFeeRaw"] = json!("500000000000000000");
    record["builderFeeRaw"] = json!("250000000000000000");
    if conflict {
        record["marginRaw"] = json!("332000000000000000000");
    }
    *harness.source.history.lock() = json!({"items":[record],"nextCursor":null});
    let tx = B256::repeat_byte(42);
    let hash = B256::repeat_byte(43);
    let height = format!("0x{:x}", source["indexedThroughBlock"].as_u64().unwrap());
    let wallet: alloy_primitives::Address = source["address"].as_str().unwrap().parse().unwrap();
    let result = json!({"address":nautilus_papertrade::chain::BATCH_EXECUTOR,
        "topics":[nautilus_papertrade::chain::RESULT_TOPIC,B256::ZERO,B256::ZERO,entry.intent_id],
        "transactionHash":tx,"blockHash":hash,"blockNumber":height,"removed":false,
        "data":format!("0x{:064x}{:064x}{:064x}{:064x}",1,64,32,position.position_id.0)});
    let open = json!({"address":nautilus_papertrade::signing::EXCHANGE,
        "topics":["0x88451ea1189f73b554f0bf39c09868560ed149b3d788fcc933ff66ba926827f3",format!("0x{:064x}",position.position_id.0),format!("0x{:064x}",U256::from_be_slice(wallet.as_slice()))],
        "transactionHash":tx,"blockHash":hash,"blockNumber":height,"blockTimestamp":format!("0x{:x}",now-20),"removed":false,
        "data":format!("0x{:064x}{:064x}{:064x}{:064x}{:064x}{:064x}",0,1,0,position.entry_price.0,position.margin.0,position.leverage)});
    *harness.source.receipt.lock() = Some(
        json!({"status":"0x1","transactionHash":tx,"blockHash":hash,"blockNumber":height,"logs":[result,open]}),
    );
    let mut node = account_node(&harness);
    if conflict {
        let handle = node.handle();
        let stop = async {
            tokio::time::sleep(Duration::from_secs(1)).await;
            let running = handle.is_running();
            handle.stop();
            running
        };
        let (run, running) = tokio::join!(node.run(), stop);
        run.unwrap();
        assert!(
            !running,
            "Contradictory evidence allowed LiveNode to enter Running"
        );
        assert!(
            node.kernel()
                .cache()
                .borrow()
                .positions(None, None, None, None, None)
                .is_empty()
        );
        assert!(
            node.kernel()
                .cache()
                .borrow()
                .orders(None, None, None, None, None)
                .is_empty()
        );
        node.dispose();
        drop(node);
        harness.source.history.lock()["items"][0]["marginRaw"] = json!(position.margin);
        node = account_node(&harness);
    }
    run_account_node(&mut node).await;
    {
        let cache = node.kernel().cache();
        let cache = cache.borrow();
        assert_eq!(cache.positions(None, None, None, None, None).len(), 1);
        let recovered = cache.position(&PositionId::from("PT-9001")).unwrap();
        assert!(recovered.is_closed());
        assert_eq!(
            recovered.realized_pnl.unwrap().as_decimal(),
            Decimal::new(1825, 2)
        );
        assert_eq!(
            recovered.commissions()[0].as_decimal(),
            Decimal::new(175, 2)
        );
        assert_eq!(recovered.trade_ids().len(), 2);
        let order = cache.order(&entry.command.client_order_id).unwrap();
        assert_eq!(order.status(), OrderStatus::Filled);
        assert_eq!(order.trade_ids().len(), 1);
    }
    node.dispose();
}
