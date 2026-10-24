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

#[path = "support/scenario.rs"]
#[allow(dead_code, reason = "Shared scenarios include fully filled orders")]
mod scenario;
#[path = "support/entropy.rs"]
mod support;

use nautilus_common::messages::execution::{SubmitOrder, TradingCommand};
use nautilus_core::{UUID4, UnixNanos};
use nautilus_hyperliquid::common::consts::HYPERLIQUID_CLIENT_ID;
use nautilus_live::node::LiveNode;
use nautilus_model::{
    enums::OrderStatus,
    identifiers::{ClientOrderId, StrategyId, TradeId},
    orders::Order,
    types::{Currency, Quantity},
};
use rstest::rstest;
use rust_decimal::Decimal;
use scenario::{Retained, history, local_order, local_order_with_quantity, venue_fill};
use serde_json::{Value, json};
use support::{Harness, SERIAL, run_to_running, wait_for};

fn legacy_id(fill: &Value) -> TradeId {
    // Frozen pre-normalization fingerprint, independent of the production parser.
    let mut bytes = fill["hash"].as_str().unwrap().as_bytes().to_vec();
    bytes.extend(fill["oid"].as_u64().unwrap().to_le_bytes());
    bytes.extend(fill["px"].as_str().unwrap().as_bytes());
    bytes.extend(fill["sz"].as_str().unwrap().as_bytes());
    bytes.extend(fill["time"].as_u64().unwrap().to_le_bytes());
    bytes.extend(fill["startPosition"].as_str().unwrap().as_bytes());
    let hash = bytes.into_iter().fold(0xcbf29ce484222325u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x100000001b3)
    });
    TradeId::new(format!(
        "{hash:016x}-{:016x}",
        fill["oid"].as_u64().unwrap()
    ))
}

fn replace_ids(value: &mut Value, pairs: &[(TradeId, TradeId)]) {
    match value {
        Value::String(text) => {
            if let Some((_, legacy)) = pairs.iter().find(|(current, _)| current.as_str() == text) {
                *text = legacy.to_string();
            }
        }
        Value::Array(rows) => {
            for row in rows {
                replace_ids(row, pairs);
            }
        }
        Value::Object(fields) => {
            for row in fields.values_mut() {
                replace_ids(row, pairs);
            }
        }
        _ => {}
    }
}

fn recode(fill: &Value, scale: usize) -> Value {
    let mut result = fill.clone();
    for key in ["px", "sz", "startPosition", "fee", "closedPnl"] {
        let mut number = fill[key]
            .as_str()
            .unwrap()
            .parse::<Decimal>()
            .unwrap()
            .normalize();
        number.rescale((scale as u32).max(number.scale()));
        result[key] = json!(number.to_string());
    }
    result
}

fn check(node: &LiveNode, open_ids: &[TradeId], closed: bool) -> Vec<TradeId> {
    let cache = node.kernel().cache();
    let cache = cache.borrow();
    assert_eq!(
        cache.orders(None, None, None, None, None).len(),
        if closed { 2 } else { 1 }
    );
    let positions = cache.positions(None, None, None, None, None);
    assert_eq!(positions.len(), 1);
    let p = positions[0].as_ref();
    assert_eq!(p.strategy_id, StrategyId::from("ENTROPY-001"));
    assert_eq!(p.is_closed(), closed);
    assert_eq!(
        p.quantity.as_decimal(),
        if closed {
            Decimal::ZERO
        } else {
            Decimal::from(2)
        }
    );
    assert_eq!(p.events.len(), if closed { 3 } else { 2 });
    assert_eq!(p.trade_ids.len(), if closed { 3 } else { 2 });
    let ids: Vec<_> = p.events.iter().map(|f| f.trade_id).collect();
    assert_eq!(&ids[..2], open_ids);
    assert_eq!(p.commissions().len(), 1);
    assert_eq!(p.commissions()[0].currency, Currency::USDC());
    assert_eq!(
        p.commissions()[0].as_decimal(),
        if closed {
            Decimal::new(5, 1)
        } else {
            Decimal::new(2, 1)
        }
    );
    assert_eq!(p.realized_pnl.unwrap().currency, Currency::USD());
    assert_eq!(
        p.realized_pnl.unwrap().as_decimal(),
        if closed {
            Decimal::from(20)
        } else {
            Decimal::ZERO
        }
    );

    for (id, oid, trades, fee, status, qty) in [
        (
            "ENTROPY-OPEN",
            "9001",
            &ids[..2],
            Decimal::new(2, 1),
            OrderStatus::PartiallyFilled,
            4,
        ),
        (
            "ENTROPY-CLOSE",
            "9002",
            &ids[2..],
            Decimal::new(3, 1),
            OrderStatus::Filled,
            2,
        ),
    ]
    .into_iter()
    .take(if closed { 2 } else { 1 })
    {
        let order = cache.order(&ClientOrderId::from(id)).unwrap();
        assert_eq!(order.venue_order_id().unwrap().as_str(), oid);
        assert_eq!(order.strategy_id(), p.strategy_id);
        assert_eq!(order.position_id(), Some(p.id));
        assert_eq!(order.status(), status);
        assert_eq!(order.quantity().as_decimal(), Decimal::from(qty));
        assert_eq!(order.filled_qty().as_decimal(), Decimal::from(2));
        assert_eq!(
            order.avg_px().unwrap(),
            if id == "ENTROPY-OPEN" {
                Decimal::from(100)
            } else {
                Decimal::from(110)
            }
        );
        assert_eq!(order.commissions().len(), 1);
        assert_eq!(order.commissions()[&Currency::USDC()].as_decimal(), fee);
        assert_eq!(
            order
                .trade_ids()
                .into_iter()
                .copied()
                .collect::<std::collections::HashSet<_>>(),
            trades.iter().copied().collect()
        );
    }
    ids
}

#[rstest]
#[case::current_orders(false, false)]
#[case::current_positions_only(false, true)]
#[case::legacy_orders(true, false)]
#[case::legacy_positions_only(true, true)]
#[tokio::test]
async fn differently_encoded_replays_preserve_partial_fills_and_economics(
    #[case] legacy: bool,
    #[case] drop_orders: bool,
) {
    let _serial = SERIAL.lock().await;
    let h = Harness::new(false).await;
    let open = local_order_with_quantity(false, Quantity::from("4.00"));
    let mut first = h.node().await;
    let cache = first.kernel().cache();
    let engine = first.kernel().exec_engine().clone();
    let handle = first.handle();
    let mut fills = Vec::new();
    let drive = async {
        let result = async {
            wait_for(|| handle.is_running()).await?;
            engine
                .borrow()
                .execute(TradingCommand::SubmitOrder(SubmitOrder::from_order(
                    &open,
                    open.trader_id(),
                    Some(*HYPERLIQUID_CLIENT_ID),
                    None,
                    UUID4::new(),
                    UnixNanos::default(),
                )));
            wait_for(|| {
                cache
                    .borrow()
                    .order(&open.client_order_id())
                    .is_some_and(|o| o.status() == OrderStatus::Accepted)
            })
            .await?;
            let time = nautilus_core::time::get_atomic_clock_realtime().get_time_ms();
            for leg in 0..2 {
                let mut fill = venue_fill(&open, false, time);
                fill["sz"] = json!("1.00");
                fill["fee"] = json!("0.10");
                fill["startPosition"] = json!(if leg == 0 { "0.00" } else { "1.00" });
                fill["tid"] = json!(101 + leg);
                h.source
                    .messages
                    .send(json!({"channel":"user","data":{"fills":[fill.clone()]}}))
                    .unwrap();
                fills.push(fill);
                wait_for(|| {
                    cache
                        .borrow()
                        .order(&open.client_order_id())
                        .is_some_and(|o| o.filled_qty().as_decimal() == Decimal::from(leg + 1))
                })
                .await?;
            }
            Ok::<_, anyhow::Error>(())
        }
        .await;
        handle.stop();
        result
    };
    let (run, result) = tokio::join!(first.run(), drive);
    run.unwrap();
    result.unwrap();
    let mut retained = Retained::capture(&first);
    let mut ids: Vec<_> = retained.positions[0]
        .events
        .iter()
        .map(|f| f.trade_id)
        .collect();
    assert_ne!(
        ids[0], ids[1],
        "Equal-price, equal-size same-ms partials must remain distinct"
    );

    if legacy {
        let pairs: Vec<_> = ids
            .iter()
            .copied()
            .zip(fills.iter().map(legacy_id))
            .collect();
        let mut encoded = serde_json::to_value(&retained.orders).unwrap();
        replace_ids(&mut encoded, &pairs);
        retained.orders = serde_json::from_value(encoded).unwrap();
        let mut encoded = serde_json::to_value(&retained.positions).unwrap();
        replace_ids(&mut encoded, &pairs);
        retained.positions = serde_json::from_value(encoded).unwrap();
        ids = pairs.iter().map(|(_, id)| *id).collect();
    }
    first.dispose();
    drop(first);

    if drop_orders {
        retained.orders.clear();
    }
    *h.source.fills.lock() = fills.iter().rev().map(|f| recode(f, 0)).collect();
    let mut row = h.source.order_rows.lock()[0].clone();
    row["sz"] = json!("2");
    *h.source.open_orders.lock() = vec![row];
    *h.source.perp_positions.lock() = vec![
        json!({"type":"oneWay","position":{"coin":"io:SNDK","cumFunding":{"allTime":"0","sinceOpen":"0","sinceChange":"0"},"entryPx":"100","leverage":{"type":"cross","value":2},"liquidationPx":null,"marginUsed":"100","maxLeverage":10,"positionValue":"200","returnOnEquity":"0","szi":"2","unrealizedPnl":"0"}}),
    ];
    let mut recovered = h.node().await;
    retained.restore(&recovered);
    let cache = recovered.kernel().cache();
    let engine = recovered.kernel().exec_engine().clone();
    let handle = recovered.handle();
    let close = local_order(true);
    let drive = async {
        let result = async {
            wait_for(|| handle.is_running()).await?;
            {
                let state = cache.borrow();
                let order = state.order(&open.client_order_id()).unwrap();
                assert_eq!(order.filled_qty().as_decimal(), Decimal::from(2));
                assert_eq!(
                    order
                        .trade_ids()
                        .into_iter()
                        .copied()
                        .collect::<std::collections::HashSet<_>>(),
                    ids.iter().copied().collect()
                );
                assert_eq!(
                    state.positions(None, None, None, None, None)[0]
                        .quantity
                        .as_decimal(),
                    Decimal::from(2)
                );
            }
            engine
                .borrow()
                .execute(TradingCommand::SubmitOrder(SubmitOrder::from_order(
                    &close,
                    close.trader_id(),
                    Some(*HYPERLIQUID_CLIENT_ID),
                    None,
                    UUID4::new(),
                    UnixNanos::default(),
                )));
            wait_for(|| {
                cache
                    .borrow()
                    .order(&close.client_order_id())
                    .is_some_and(|o| o.status() == OrderStatus::Accepted)
            })
            .await?;
            let mut close_fill = venue_fill(
                &close,
                true,
                nautilus_core::time::get_atomic_clock_realtime().get_time_ms(),
            );
            close_fill["tid"] = json!(103);
            // A fresh fill in the same frame proves all replay rows were consumed before assertions.
            let mut frame: Vec<_> = fills.iter().map(|f| recode(f, 3)).collect();
            frame.push(close_fill.clone());
            h.source
                .messages
                .send(json!({"channel":"user","data":{"fills":frame}}))
                .unwrap();
            wait_for(|| {
                cache
                    .borrow()
                    .order(&close.client_order_id())
                    .is_some_and(|o| o.status() == OrderStatus::Filled)
            })
            .await?;
            Ok::<_, anyhow::Error>(close_fill)
        }
        .await;
        handle.stop();
        result
    };
    // The observer uses cache handles, so avoid borrowing the running node.
    let (run, result) = tokio::join!(recovered.run(), drive);
    run.unwrap();
    let close_fill = result.unwrap();
    let all_ids = check(&recovered, &ids, true);
    let healed = Retained::capture(&recovered);
    recovered.dispose();
    drop(recovered);
    fills.push(close_fill.clone());
    *h.source.fills.lock() = fills.iter().rev().map(|f| recode(f, 1)).collect();
    *h.source.history.lock() = vec![history(&close_fill)];
    h.source.perp_positions.lock().clear();
    let mut restarted = h.node().await;
    healed.restore(&restarted);
    run_to_running(&mut restarted).await;
    assert_eq!(check(&restarted, &ids, true), all_ids);
    assert_eq!(
        h.source.actions.lock().len(),
        2,
        "Recovery must not resubmit an order"
    );
    restarted.dispose();
}

#[rstest]
fn legacy_fingerprint_fixture_is_pinned() {
    assert_eq!(legacy_id(&json!({"hash":"0xabc123","oid":12345,"px":"50000.0","sz":"0.1","time":1704470400000u64,"startPosition":"0.0"})).as_str(),"a846ae6f557868e9-0000000000003039");
}
