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
