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
#[allow(
    dead_code,
    reason = "Shared scenarios also include closed-position recovery"
)]
mod scenario;
#[path = "support/entropy.rs"]
mod support;
use nautilus_common::messages::execution::{SubmitOrder, TradingCommand};
use nautilus_core::{UUID4, UnixNanos};
use nautilus_hyperliquid::common::consts::HYPERLIQUID_CLIENT_ID;
use nautilus_model::{enums::OrderStatus, identifiers::ClientOrderId, orders::Order};
use rstest::rstest;
use rust_decimal::Decimal;
use scenario::{Retained, economics, history, local_order, venue_fill};
use serde_json::json;
use std::sync::atomic::Ordering;
use support::{Harness, SERIAL, run_to_running, wait_for};

#[rstest]
#[case::resting_close(false)]
#[case::filled_close(true)]
#[tokio::test]
async fn submitted_without_venue_id_recovers_original_cloid(#[case] filled: bool) {
    let _serial = SERIAL.lock().await;
    let harness = Harness::new(false).await;
    let mut first = harness.node().await;
    let cache = first.kernel().cache();
    let engine = first.kernel().exec_engine().clone();
    let handle = first.handle();
    let open = local_order(false);
    let close = local_order(true);
    let mut open_fill = None;
    let drive = async {
        let result = async {
            wait_for(|| handle.is_running()).await?;

            for order in [&open, &close] {
                if order.client_order_id() == close.client_order_id() {
                    harness
                        .source
                        .hold_order_updates
                        .store(true, Ordering::SeqCst);
                }
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

                if order.client_order_id() == open.client_order_id() {
                    wait_for(|| {
                        cache
                            .borrow()
                            .order(&open.client_order_id())
                            .is_some_and(|o| o.status() == OrderStatus::Accepted)
                    })
                    .await?;
                    let fill = venue_fill(
                        &open,
                        false,
                        nautilus_core::time::get_atomic_clock_realtime().get_time_ms(),
                    );
                    harness
                        .source
                        .messages
                        .send(json!({"channel":"user","data":{"fills":[fill.clone()]}}))
                        .unwrap();
                    open_fill = Some(fill);
                    wait_for(|| {
                        cache
                            .borrow()
                            .order(&open.client_order_id())
                            .is_some_and(|o| o.status() == OrderStatus::Filled)
                    })
                    .await?;
                } else {
                    wait_for(|| {
                        harness.source.actions.lock().len() == 2
                            && cache
                                .borrow()
                                .order(&close.client_order_id())
                                .is_some_and(|o| o.status() == OrderStatus::Submitted)
                    })
                    .await?;
                }
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
    let retained = Retained::capture(&first);
    assert_eq!(retained.positions.len(), 1);
    assert!(retained.positions[0].is_open());
    let order = retained
        .orders
        .iter()
        .find(|o| o.client_order_id() == close.client_order_id())
        .unwrap();
    assert_eq!(order.status(), OrderStatus::Submitted);
    assert!(order.venue_order_id().is_none());
    assert!(order.trade_ids().is_empty());
    let first_trade = retained.positions[0].events[0].trade_id;
    first.dispose();
    drop(first);
    let open_fill = open_fill.unwrap();
    let close_fill = venue_fill(
        &close,
        true,
        nautilus_core::time::get_atomic_clock_realtime().get_time_ms(),
    );
    *harness.source.fills.lock() = if filled {
        vec![close_fill.clone(), open_fill.clone()]
    } else {
        vec![open_fill.clone()]
    };
    *harness.source.history.lock() = if filled {
        vec![history(&close_fill), history(&open_fill)]
    } else {
        vec![history(&open_fill)]
    };

    if !filled {
        *harness.source.open_orders.lock() = vec![harness.source.order_rows.lock()[1].clone()];
        *harness.source.perp_positions.lock() = vec![
            json!({"type":"oneWay","position":{"coin":"io:SNDK", "cumFunding":{"allTime":"0","sinceOpen":"0","sinceChange":"0"},"entryPx":"100", "leverage":{"type":"cross","value":2}, "liquidationPx":null,"marginUsed":"100", "maxLeverage":10,"positionValue":"200","returnOnEquity":"0","szi":"2.00","unrealizedPnl":"0"}}),
        ];
    }
    let mut recovered = harness.node().await;
    retained.restore(&recovered);
    run_to_running(&mut recovered).await;
    let check = |node: &nautilus_live::node::LiveNode| {
        let cache = node.kernel().cache();
        let cache = cache.borrow();
        assert_eq!(
            cache.orders(None, None, None, None, None).len(),
            2,
            "CLOID created a shadow order"
        );
        let order = cache.order(&ClientOrderId::from("ENTROPY-CLOSE")).unwrap();
        assert_eq!(order.venue_order_id().unwrap().as_str(), "9002");
        assert_eq!(
            order.status(),
            if filled {
                OrderStatus::Filled
            } else {
                OrderStatus::Accepted
            }
        );
        let positions = cache.positions(None, None, None, None, None);
        assert_eq!(positions.len(), 1);
        let position = positions[0].as_ref();
        assert_eq!(position.events[0].trade_id, first_trade);
        assert_eq!(position.events.len(), if filled { 2 } else { 1 });
        if !filled {
            assert_eq!(position.quantity.as_decimal(), Decimal::from(2));
            assert_eq!(position.commissions()[0].as_decimal(), Decimal::new(2, 1));
            assert_eq!(position.realized_pnl.unwrap().as_decimal(), Decimal::ZERO);
        }
    };
    check(&recovered);
    if filled {
        economics(&recovered);
    }
    let healed = Retained::capture(&recovered);
    recovered.dispose();
    drop(recovered);
    let mut restarted = harness.node().await;
    healed.restore(&restarted);
    run_to_running(&mut restarted).await;
    check(&restarted);
    if filled {
        economics(&restarted);
    }
    assert_eq!(harness.source.actions.lock().len(), 2);
    restarted.dispose();
}
