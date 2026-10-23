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

struct RetainedCache {
    orders: Vec<nautilus_model::orders::OrderAny>,
    positions: Vec<nautilus_model::position::Position>,
}

impl RetainedCache {
    fn capture(node: &nautilus_live::node::LiveNode) -> Self {
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
    fn restore(&self, node: &nautilus_live::node::LiveNode) {
        let cache = node.kernel().cache();
        let mut cache = cache.borrow_mut();
        for order in &self.orders {
            cache.add_order(order.clone(), None, None, false).unwrap();
        }
        for position in &self.positions {
            cache
                .add_position_without_order(position, nautilus_model::enums::OmsType::Hedging)
                .unwrap();
        }
        cache.clear_index();
        cache.build_index();
    }
}

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
#[case::same_round(false, false, false, false, false, false)]
#[case::conflicting_proof(true, false, false, false, false, false)]
#[case::legacy_expired(false, true, false, false, false, false)]
#[case::later_round(false, false, true, false, false, false)]
#[case::restart_between_rounds(false, false, true, true, false, false)]
#[case::unattributed_open(false, false, true, false, true, false)]
#[case::unattributed_open_restart(false, false, true, true, true, false)]
#[case::retained_expired(false, true, true, false, false, true)]
#[case::retained_expired_restart(false, true, true, true, false, true)]
#[case::retained_open_expired(false, true, true, false, true, true)]
#[case::retained_open_expired_restart(false, true, true, true, true, true)]
#[tokio::test]
async fn live_node_recovers_closed_during_downtime_with_history_before_open_proof(
    #[case] conflict: bool,
    #[case] legacy_expired: bool,
    #[case] delayed: bool,
    #[case] restart: bool,
    #[case] unattributed_open: bool,
    #[case] retained_cache: bool,
) {
    use alloy_primitives::{B256, U256};
    use nautilus_model::{enums::OrderStatus, identifiers::PositionId, orders::Order};
    use nautilus_papertrade::wire::VenuePosition;
    use rust_decimal::Decimal;
    let _serial = SERIAL.lock().await;
    let mut harness = Harness::new().await;
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
    if unattributed_open {
        let mut observed = source.clone();
        observed["openPositions"] = json!([position]);
        observed["locked"] = json!(position.margin);
        observed["balance"] = json!("834000000000000000000");
        observed["funds"]["availableBalance"] = observed["balance"].clone();
        harness.source.wallet.send_replace(observed);
        source["accountRevision"] = json!(2);
        source["walletRevision"] = source["indexedThroughBlock"].clone();
    } else {
        harness.source.wallet.send_replace(source.clone());
    }
    harness.source.trading.lock()["indexedThroughBlock"] = source["indexedThroughBlock"].clone();
    let initial_snapshot = harness.source.wallet.borrow().clone();
    if retained_cache {
        harness.config.max_history_pages = 1;
        let mut submission = initial_snapshot.clone();
        submission["indexedThroughBlock"] =
            json!(source["indexedThroughBlock"].as_u64().unwrap() - 2500);
        harness.source.wallet.send_replace(submission);
    }
    let entry = support::prepared_open(&harness, &position, now - 60, now + 3600 - 60);
    if retained_cache {
        harness.source.wallet.send_replace(initial_snapshot);
    }
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
    let history = json!({"items":[record],"nextCursor":null});
    if !unattributed_open {
        *harness.source.history.lock() = history.clone();
    }
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
    let retained = if retained_cache {
        let path = harness.config.journal_path.clone();
        let seed_path = path.with_file_name("seed.redb");
        std::fs::copy(&path, &seed_path).unwrap();
        harness.config.journal_path = seed_path;
        let mut seed = account_node(&harness);
        harness.config.journal_path = path;
        let cache = seed.kernel().cache();
        let handle = seed.handle();
        let stop = async {
            let ready = tokio::time::timeout(Duration::from_secs(8), async {
                while cache
                    .borrow()
                    .position(&PositionId::from("PT-9001"))
                    .is_none()
                    || !cache
                        .borrow()
                        .order(&entry.command.client_order_id)
                        .is_some_and(|o| o.status() == OrderStatus::Filled)
                {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await;
            handle.stop();
            ready
        };
        let (run, ready) = tokio::join!(seed.run(), stop);
        run.unwrap();
        ready.unwrap();
        let retained = RetainedCache::capture(&seed);
        assert_eq!(retained.positions.len(), 1);
        assert_eq!(retained.orders.len(), if unattributed_open { 1 } else { 2 });
        assert_eq!(retained.positions[0].is_open(), unattributed_open);
        seed.dispose();
        drop(seed);
        harness
            .source
            .history_calls
            .store(0, std::sync::atomic::Ordering::SeqCst);
        Some(retained)
    } else {
        None
    };
    if delayed {
        harness.source.trading.lock()["indexedThroughBlock"] =
            json!(source["indexedThroughBlock"].as_u64().unwrap() - 1);
    }
    let mut node = account_node(&harness);
    if legacy_expired {
        use nautilus_core::UUID4;
        use nautilus_model::{
            events::{OrderEventAny, OrderExpired},
            identifiers::VenueOrderId,
            orders::OrderAny,
        };
        let mut order = if let Some(retained) = &retained {
            let original = retained
                .orders
                .iter()
                .find(|o| o.client_order_id() == entry.command.client_order_id)
                .unwrap();
            OrderAny::from_events(
                original
                    .events()
                    .into_iter()
                    .filter(|e| !matches!(e, OrderEventAny::Filled(_)))
                    .cloned()
                    .collect(),
            )
            .unwrap()
        } else {
            OrderAny::try_from(entry.command.order_init.clone()).unwrap()
        };
        order
            .apply(OrderEventAny::Expired(OrderExpired::new(
                order.trader_id(),
                order.strategy_id(),
                order.instrument_id(),
                order.client_order_id(),
                UUID4::new(),
                entry.command.ts_init,
                entry.command.ts_init,
                true,
                Some(VenueOrderId::from(format!("{:#x}", entry.intent_id))),
                Some(harness.config.account_id),
            )))
            .unwrap();
        node.kernel()
            .cache()
            .borrow_mut()
            .add_order(order, None, None, false)
            .unwrap();
    }
    if let Some(retained) = &retained {
        RetainedCache {
            orders: retained
                .orders
                .iter()
                .filter(|o| o.client_order_id() != entry.command.client_order_id)
                .cloned()
                .collect(),
            positions: retained.positions.clone(),
        }
        .restore(&node);
    }
    if delayed {
        use std::sync::atomic::Ordering;
        let cache = node.kernel().cache();
        let handle = node.handle();
        let unchanged = || {
            let cache = cache.borrow();
            if let Some(retained) = &retained {
                let original = &retained.positions[0];
                cache.positions(None, None, None, None, None).len() == 1
                    && cache.position(&original.id).is_some_and(|p| {
                        p.realized_pnl == original.realized_pnl
                            && p.quantity == original.quantity
                            && p.avg_px_open == original.avg_px_open
                            && p.trade_ids == original.trade_ids
                            && p.events.len() == original.events.len()
                            && p.commissions() == original.commissions()
                    })
                    && cache
                        .order(&entry.command.client_order_id)
                        .is_some_and(|o| {
                            o.status() == OrderStatus::Expired && o.trade_ids().is_empty()
                        })
            } else {
                cache.positions(None, None, None, None, None).is_empty()
                    && cache.orders(None, None, None, None, None).iter().all(|o| {
                        o.client_order_id() == entry.command.client_order_id
                            && o.trade_ids().is_empty()
                    })
            }
        };
        let advance = async {
            let mut observed = tokio::time::timeout(Duration::from_secs(4), async {
                while harness.source.history_calls.load(Ordering::SeqCst) < 2 {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
                unchanged()
            })
            .await;
            if unattributed_open && matches!(observed, Ok(true)) {
                let calls = harness.source.history_calls.load(Ordering::SeqCst);
                *harness.source.history.lock() = history.clone();
                harness.source.wallet.send_replace(source.clone());
                observed = tokio::time::timeout(Duration::from_secs(4), async {
                    while harness.source.history_calls.load(Ordering::SeqCst) < calls + 2 {
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    unchanged()
                })
                .await;
            }
            if !restart && matches!(observed, Ok(true)) {
                harness.source.trading.lock()["indexedThroughBlock"] =
                    source["indexedThroughBlock"].clone();
                let recovered = tokio::time::timeout(Duration::from_secs(4), async {
                    loop {
                        if cache
                            .borrow()
                            .order(&entry.command.client_order_id)
                            .is_some_and(|o| o.status() == OrderStatus::Filled)
                        {
                            break;
                        }
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                })
                .await;
                handle.stop();
                recovered.unwrap();
            } else {
                handle.stop();
            }
            observed
        };
        let (run, observed) = tokio::join!(node.run(), advance);
        run.unwrap();
        assert!(
            observed.unwrap(),
            "History was attributed externally before local open proof coverage"
        );
        if restart {
            let before_restart = RetainedCache::capture(&node);
            node.dispose();
            drop(node);
            harness.source.trading.lock()["indexedThroughBlock"] =
                source["indexedThroughBlock"].clone();
            node = account_node(&harness);
            before_restart.restore(&node);
        }
    }
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
    if !delayed || restart {
        run_account_node(&mut node).await;
    }
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
        assert_eq!(cache.orders(None, None, None, None, None).len(), 2);
        let order = cache.order(&entry.command.client_order_id).unwrap();
        assert_eq!(order.status(), OrderStatus::Filled);
        assert_eq!(
            order.venue_order_id().unwrap().as_str(),
            format!("{:#x}", entry.intent_id)
        );
        assert_eq!(order.trade_ids().len(), 1);
    }
    let before = {
        let cache = node.kernel().cache();
        let cache = cache.borrow();
        let mut orders: Vec<_> = cache
            .orders(None, None, None, None, None)
            .into_iter()
            .map(|o| {
                (
                    o.client_order_id(),
                    o.venue_order_id(),
                    o.trade_ids().into_iter().copied().collect::<Vec<_>>(),
                    o.commissions().clone(),
                )
            })
            .collect();
        orders.sort_by_key(|o| o.0);
        orders
    };
    node.dispose();
    drop(node);
    let mut restarted = account_node(&harness);
    run_account_node(&mut restarted).await;
    {
        let cache = restarted.kernel().cache();
        let cache = cache.borrow();
        let mut orders: Vec<_> = cache
            .orders(None, None, None, None, None)
            .into_iter()
            .map(|o| {
                (
                    o.client_order_id(),
                    o.venue_order_id(),
                    o.trade_ids().into_iter().copied().collect::<Vec<_>>(),
                    o.commissions().clone(),
                )
            })
            .collect();
        orders.sort_by_key(|o| o.0);
        assert_eq!(orders, before);
        assert_eq!(cache.positions(None, None, None, None, None).len(), 1);
        let position = cache.position(&PositionId::from("PT-9001")).unwrap();
        assert_eq!(position.trade_ids().len(), 2);
        assert_eq!(position.events.len(), 2);
        assert_eq!(
            position.realized_pnl.unwrap().as_decimal(),
            Decimal::new(1825, 2)
        );
        assert_eq!(position.commissions()[0].as_decimal(), Decimal::new(175, 2));
    }
    restarted.dispose();
}
