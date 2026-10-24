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

use crate::{
    scenario::{Retained, local_order, venue_fill},
    support::{Harness, wait_for},
};
use nautilus_common::messages::execution::{SubmitOrder, TradingCommand};
use nautilus_core::{UUID4, UnixNanos};
use nautilus_hyperliquid::common::consts::HYPERLIQUID_CLIENT_ID;
use nautilus_live::node::LiveNode;
use nautilus_model::{
    enums::OrderStatus,
    identifiers::{ClientOrderId, StrategyId, TradeId},
    orders::{Order, OrderAny},
    types::Currency,
};
use rust_decimal::Decimal;
use serde_json::{Value, json};

pub(crate) fn check(
    node: &LiveNode,
    expected_ids: &[TradeId],
    buy_avg: Decimal,
    sell: Decimal,
    pnl: Decimal,
) -> Vec<TradeId> {
    let cache = node.kernel().cache();
    let cache = cache.borrow();
    assert_eq!(cache.orders(None, None, None, None, None).len(), 2);
    let positions = cache.positions(None, None, None, None, None);
    assert_eq!(positions.len(), 1);
    let position = positions[0].as_ref();
    assert!(position.is_closed());
    assert_eq!(position.events.len(), 3);
    assert_eq!(position.trade_ids.len(), 3);
    assert_eq!(position.strategy_id, StrategyId::from("ENTROPY-001"));
    assert_eq!(position.realized_pnl.unwrap().currency, Currency::USD());
    assert_eq!(position.realized_pnl.unwrap().as_decimal(), pnl);
    assert_eq!(position.commissions().len(), 1);
    assert_eq!(position.commissions()[0].currency, Currency::USDC());
    assert_eq!(position.commissions()[0].as_decimal(), Decimal::new(5, 1));
    let ids: Vec<_> = position.events.iter().map(|e| e.trade_id).collect();
    if !expected_ids.is_empty() {
        assert_eq!(ids, expected_ids);
    }
    for (id, oid, trades, fee, avg) in [
        (
            "ENTROPY-OPEN",
            "9001",
            &ids[..2],
            Decimal::new(2, 1),
            buy_avg,
        ),
        ("ENTROPY-CLOSE", "9002", &ids[2..], Decimal::new(3, 1), sell),
    ] {
        let order = cache.order(&ClientOrderId::from(id)).unwrap();
        assert_eq!(order.status(), OrderStatus::Filled);
        assert_eq!(order.venue_order_id().unwrap().as_str(), oid);
        assert_eq!(order.strategy_id(), StrategyId::from("ENTROPY-001"));
        assert_eq!(order.position_id(), Some(position.id));
        assert_eq!(order.trade_ids().len(), trades.len());
        assert_eq!(
            order
                .trade_ids()
                .into_iter()
                .copied()
                .collect::<std::collections::HashSet<_>>(),
            trades.iter().copied().collect()
        );
        assert_eq!(order.filled_qty().as_decimal(), Decimal::from(2));
        assert_eq!(order.commissions().len(), 1);
        assert_eq!(order.commissions()[&Currency::USDC()].as_decimal(), fee);
        assert_eq!(order.avg_px().unwrap(), avg);
    }
    ids
}

pub(crate) async fn round_trip(
    harness: &Harness,
    prices: [&str; 2],
    exit: &str,
) -> (Retained, Vec<OrderAny>, Vec<Value>, Vec<TradeId>) {
    let mut node = harness.node().await;
    let cache = node.kernel().cache();
    let engine = node.kernel().exec_engine().clone();
    let handle = node.handle();
    let orders = [local_order(false), local_order(true)];
    let mut fills = Vec::new();
    let drive = async {
        let result = async {
            wait_for(|| handle.is_running()).await?;
            let mut accepted = Vec::new();
            for (index, order) in orders.iter().enumerate() {
                engine
                    .borrow()
                    .execute(TradingCommand::SubmitOrder(SubmitOrder::from_order(
                        order,
                        order.trader_id(),
                        Some(*HYPERLIQUID_CLIENT_ID),
                        None,
                        UUID4::new(),
                        UnixNanos::default(),
                    )));
                wait_for(|| {
                    cache
                        .borrow()
                        .order(&order.client_order_id())
                        .is_some_and(|o| o.status() == OrderStatus::Accepted)
                })
                .await?;
                accepted.push(
                    cache
                        .borrow()
                        .order(&order.client_order_id())
                        .unwrap()
                        .clone(),
                );
                let time = nautilus_core::time::get_atomic_clock_realtime().get_time_ms();
                for (leg, open_price) in
                    prices
                        .iter()
                        .enumerate()
                        .take(if index == 0 { 2 } else { 1 })
                {
                    let mut fill = venue_fill(order, index == 1, time);
                    fill["px"] = json!(if index == 0 { *open_price } else { exit });
                    fill["sz"] = json!(if index == 0 { "1.00" } else { "2.00" });
                    fill["startPosition"] = json!(if index == 0 {
                        if leg == 0 { "0.00" } else { "1.00" }
                    } else {
                        "2.00"
                    });
                    fill["fee"] = json!(if index == 0 { "0.10" } else { "0.30" });
                    fill["tid"] = json!(if index == 0 { 101 + leg } else { 103 });
                    harness
                        .source
                        .messages
                        .send(json!({"channel":"user","data":{"fills":[fill.clone()]}}))
                        .unwrap();
                    fills.push(fill);
                    wait_for(|| {
                        cache
                            .borrow()
                            .order(&order.client_order_id())
                            .is_some_and(|o| {
                                o.filled_qty().as_decimal()
                                    == Decimal::from(if index == 0 { leg + 1 } else { 2 })
                            })
                    })
                    .await?;
                }
            }
            Ok::<_, anyhow::Error>(accepted)
        }
        .await;
        handle.stop();
        result
    };
    let (run, accepted) = tokio::join!(node.run(), drive);
    run.unwrap();
    let accepted = accepted.unwrap();
    let buy = (prices[0].parse::<Decimal>().unwrap() + prices[1].parse::<Decimal>().unwrap())
        / Decimal::from(2);
    let sell = exit.parse::<Decimal>().unwrap();
    let ids = check(&node, &[], buy, sell, (sell - buy) * Decimal::from(2));
    let retained = Retained::capture(&node);
    node.dispose();
    drop(node);
    (retained, accepted, fills, ids)
}
