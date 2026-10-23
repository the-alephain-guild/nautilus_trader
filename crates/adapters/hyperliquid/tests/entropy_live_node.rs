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

#[path = "support/entropy.rs"]
mod support;
use anyhow::Context;
use nautilus_common::messages::execution::{SubmitOrder, TradingCommand};
use nautilus_core::{UUID4, UnixNanos};
use nautilus_hyperliquid::{common::consts::HYPERLIQUID_CLIENT_ID, http::models::Cloid};
use nautilus_live::node::LiveNode;
use nautilus_model::{
    enums::{OmsType, OrderSide, OrderStatus, TimeInForce},
    identifiers::{ClientOrderId, StrategyId, TradeId, TraderId},
    orders::{LimitOrder, Order, OrderAny},
    position::Position,
    types::{Currency, Price, Quantity},
};
use rstest::rstest;
use rust_decimal::Decimal;
use serde_json::{Value, json};
use support::{Harness, INSTRUMENT, SERIAL, run_to_running, wait_for};

#[tokio::test]
async fn entropy_live_node_starts_from_an_empty_account_cache() {
    let _serial = SERIAL.lock().await;
    let harness = Harness::new(false).await;
    let mut node = harness.node().await;
    assert!(
        node.kernel()
            .cache()
            .borrow()
            .accounts_all_owned()
            .is_empty()
    );
    run_to_running(&mut node).await;
    assert_eq!(node.kernel().cache().borrow().accounts_all_owned().len(), 1);
    assert!(harness.source.actions.lock().is_empty());
    node.dispose();
}

fn local_order(close: bool) -> OrderAny {
    OrderAny::Limit(LimitOrder::new(
        TraderId::from("TESTER-001"),
        StrategyId::from("ENTROPY-001"),
        INSTRUMENT.into(),
        ClientOrderId::from(if close {
            "ENTROPY-CLOSE"
        } else {
            "ENTROPY-OPEN"
        }),
        if close {
            OrderSide::Sell
        } else {
            OrderSide::Buy
        },
        Quantity::from("2.00"),
        Price::from(if close { "110.00" } else { "100.00" }),
        TimeInForce::Gtc,
        None,
        false,
        close,
        false,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        UUID4::new(),
        nautilus_core::time::get_atomic_clock_realtime().get_time_ns(),
    ))
}

fn venue_fill(order: &OrderAny, close: bool, time: u64) -> Value {
    json!({"coin":"io:SNDK", "px":if close {"110.00"} else {"100.00"}, "sz":"2.00",
        "side":if close {"A"} else {"B"}, "time":time,
        "startPosition":if close {"2.00"} else {"0.00"},
        "dir":if close {"Close Long"} else {"Open Long"}, "closedPnl":if close {"20.00"} else {"0.00"},
        "hash":format!("0x{:064x}",if close {2} else {1}), "oid":if close {9002} else {9001},
        "tid":if close {92} else {91}, "crossed":true, "fee":if close {"0.30"} else {"0.20"},
        "feeToken":"USDC", "cloid":Cloid::from_client_order_id(order.client_order_id()).to_hex()})
}

fn history(fill: &Value) -> Value {
    json!({"order":{"coin":fill["coin"], "side":fill["side"], "limitPx":fill["px"],
        "sz":"0.00", "origSz":"2.00", "oid":fill["oid"], "timestamp":fill["time"],
        "reduceOnly":fill["side"]=="A", "orderType":"Limit", "tif":"Gtc", "cloid":fill["cloid"]},
        "status":"filled", "statusTimestamp":fill["time"]})
}

struct Retained {
    orders: Vec<OrderAny>,
    positions: Vec<Position>,
}
impl Retained {
    fn capture(node: &LiveNode) -> Self {
        let cache = node.kernel().cache();
        let cache = cache.borrow();
        Self {
            orders: cache
                .orders(None, None, None, None, None)
                .into_iter()
                .map(|o| o.clone())
                .collect(),
            positions: cache
                .positions(None, None, None, None, None)
                .into_iter()
                .map(|p| p.cloned())
                .collect(),
        }
    }
    fn restore(&self, node: &LiveNode) {
        let cache = node.kernel().cache();
        let mut cache = cache.borrow_mut();
        for order in &self.orders {
            cache
                .add_order(order.clone(), None, Some(*HYPERLIQUID_CLIENT_ID), false)
                .unwrap();
        }
        for position in &self.positions {
            cache
                .add_position_without_order(position, OmsType::Netting)
                .unwrap();
        }
        cache.clear_index();
        cache.build_index();
        let claims: Vec<_> = self
            .orders
            .iter()
            .map(|o| (o.client_order_id(), *HYPERLIQUID_CLIENT_ID))
            .collect();
        cache.claim_order_clients(&claims).unwrap();
    }
}

fn economics(node: &LiveNode) -> Vec<TradeId> {
    let cache = node.kernel().cache();
    let cache = cache.borrow();
    assert_eq!(cache.orders(None, None, None, None, None).len(), 2);
    let positions = cache.positions(None, None, None, None, None);
    assert_eq!(positions.len(), 1);
    let position = positions[0].as_ref();
    assert_eq!(position.instrument_id, INSTRUMENT.into());
    assert_eq!(position.strategy_id, StrategyId::from("ENTROPY-001"));
    assert!(position.is_closed());
    assert_eq!(position.events.len(), 2);
    assert_eq!(position.trade_ids.len(), 2);
    assert_ne!(position.events[0].trade_id, position.events[1].trade_id);
    // Native USD-quoted perps retain USD PnL and USDC commissions separately.
    assert_eq!(position.realized_pnl.unwrap().currency, Currency::USD());
    assert_eq!(
        position.realized_pnl.unwrap().as_decimal(),
        Decimal::from(20)
    );
    assert_eq!(position.commissions().len(), 1);
    assert_eq!(position.commissions()[0].currency, Currency::USDC());
    assert_eq!(position.commissions()[0].as_decimal(), Decimal::new(5, 1));
    for (index, id, oid, fee) in [
        (0, "ENTROPY-OPEN", "9001", Decimal::new(2, 1)),
        (1, "ENTROPY-CLOSE", "9002", Decimal::new(3, 1)),
    ] {
        let order = cache.order(&ClientOrderId::from(id)).unwrap();
        assert_eq!(order.status(), OrderStatus::Filled);
        assert_eq!(order.venue_order_id().unwrap().as_str(), oid);
        assert_eq!(order.position_id(), Some(position.id));
        assert_eq!(order.instrument_id(), INSTRUMENT.into());
        assert_eq!(order.strategy_id(), StrategyId::from("ENTROPY-001"));
        assert_eq!(order.trade_ids(), vec![&position.events[index].trade_id]);
        assert_eq!(order.filled_qty().as_decimal(), Decimal::from(2));
        assert_eq!(order.commissions().len(), 1);
        assert_eq!(order.commissions()[&Currency::USDC()].as_decimal(), fee);
    }
    position.events.iter().map(|e| e.trade_id).collect()
}

async fn live_round_trip(harness: &Harness) -> (Retained, Vec<OrderAny>, Vec<Value>, Vec<TradeId>) {
    let mut node = harness.node().await;
    let cache = node.kernel().cache();
    let engine = node.kernel().exec_engine().clone();
    let handle = node.handle();
    let orders = [local_order(false), local_order(true)];
    let mut fills = Vec::new();
    let drive = async {
        let result=async {
            wait_for(||handle.is_running()).await?;
            let mut accepted=Vec::new();
            for (index,order) in orders.iter().enumerate() {
                engine.borrow().execute(TradingCommand::SubmitOrder(SubmitOrder::from_order(order,order.trader_id(),Some(*HYPERLIQUID_CLIENT_ID),None,UUID4::new(),UnixNanos::default())));
                wait_for(||cache.borrow().order(&order.client_order_id()).is_some_and(|o|o.status()==OrderStatus::Accepted)).await.with_context(||format!("Acceptance timed out: status={:?}, actions={}", cache.borrow().order(&order.client_order_id()).map(|o|o.status()), harness.source.actions.lock().len()))?;
                accepted.push(cache.borrow().order(&order.client_order_id()).unwrap().clone());
                let time = nautilus_core::time::get_atomic_clock_realtime().get_time_ms();
                fills.push(venue_fill(order,index==1,time));
                harness.source.messages.send(json!({"channel":"user","data":{"fills":[fills[index].clone(),fills[index].clone()]}})).unwrap();
                wait_for(||cache.borrow().order(&order.client_order_id()).is_some_and(|o|o.status()==OrderStatus::Filled)).await?;
            }
            Ok::<_,anyhow::Error>(accepted)
        }.await;
        handle.stop();
        result
    };
    let (run, accepted) = tokio::join!(node.run(), drive);
    run.unwrap();
    let accepted = accepted.unwrap();
    let ids = economics(&node);
    let retained = Retained::capture(&node);
    node.dispose();
    drop(node);
    (retained, accepted, fills, ids)
}

#[rstest]
#[case::io_first(true, 110000)]
#[case::io_after_another_deployer(false, 120000)]
#[tokio::test]
async fn entropy_live_node_routes_orders_and_replays_rest_without_duplicate_economics(
    #[case] io_first: bool,
    #[case] expected_asset: u64,
) {
    let _serial = SERIAL.lock().await;
    let harness = Harness::new(io_first).await;
    let (retained, _, fills, ids) = live_round_trip(&harness).await;
    {
        let actions = harness.source.actions.lock();
        assert_eq!(actions.len(), 2);
        for (i, signed) in actions.iter().enumerate() {
            assert!(signed["signature"].is_object());
            assert!(signed["nonce"].is_u64());
            assert_eq!(signed["action"]["orders"].as_array().unwrap().len(), 1);
            let request = &signed["action"]["orders"][0];
            assert_eq!(request["a"], expected_asset);
            assert_eq!(request["b"], i == 0);
            assert_eq!(request["r"], i == 1);
            assert_eq!(request["c"], fills[i]["cloid"]);
            assert_eq!(
                request["s"].as_str().unwrap().parse::<Decimal>().unwrap(),
                Decimal::from(2)
            );
        }
    }
    *harness.source.fills.lock() = vec![fills[1].clone(), fills[0].clone(), fills[0].clone()];
    *harness.source.history.lock() = vec![history(&fills[1]), history(&fills[0])];
    let mut restarted = harness.node().await;
    retained.restore(&restarted);
    run_to_running(&mut restarted).await;
    assert_eq!(economics(&restarted), ids);
    assert_eq!(
        harness.source.actions.lock().len(),
        2,
        "Restart resubmitted orders"
    );
    for kind in ["frontendOpenOrders", "clearinghouseState"] {
        assert!(
            harness
                .source
                .queries
                .lock()
                .iter()
                .any(|q| q["type"] == kind && q["dex"] == "io")
        );
    }
    restarted.dispose();
}

#[rstest]
#[case::missing_orders(true)]
#[case::orders_lagging_behind_retained_position(false)]
#[tokio::test]
async fn entropy_live_node_recovers_retained_positions_with_missing_or_lagging_orders(
    #[case] missing: bool,
) {
    let _serial = SERIAL.lock().await;
    let harness = Harness::new(false).await;
    let (mut retained, accepted, fills, ids) = live_round_trip(&harness).await;
    retained.orders = if missing { Vec::new() } else { accepted };
    *harness.source.fills.lock() = vec![fills[1].clone(), fills[0].clone(), fills[1].clone()];
    *harness.source.history.lock() = vec![history(&fills[1]), history(&fills[0])];
    let mut recovered = harness.node().await;
    retained.restore(&recovered);
    {
        let cache = recovered.kernel().cache();
        let cache = cache.borrow();
        assert_eq!(cache.positions(None, None, None, None, None).len(), 1);
        assert_eq!(
            cache.orders(None, None, None, None, None).len(),
            if missing { 0 } else { 2 }
        );
        for order in &retained.orders {
            assert_eq!(order.status(), OrderStatus::Accepted);
            assert!(order.trade_ids().is_empty());
        }
    }
    run_to_running(&mut recovered).await;
    assert_eq!(economics(&recovered), ids);
    let healed = Retained::capture(&recovered);
    recovered.dispose();
    drop(recovered);
    let mut restarted = harness.node().await;
    healed.restore(&restarted);
    run_to_running(&mut restarted).await;
    assert_eq!(economics(&restarted), ids);
    assert_eq!(
        harness.source.actions.lock().len(),
        2,
        "Recovery resubmitted an order"
    );
    restarted.dispose();
}
