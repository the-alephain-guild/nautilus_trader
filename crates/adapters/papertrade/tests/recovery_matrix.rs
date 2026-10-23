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

use std::{collections::HashSet, time::Duration};

use alloy_primitives::{B256, U256};
use nautilus_live::node::LiveNode;
use nautilus_model::{
    enums::OrderStatus,
    identifiers::{AccountId, ClientOrderId, PositionId},
    orders::Order,
    types::Currency,
};
use nautilus_papertrade::{
    journal::{Journal, JournalEntry},
    wire::VenuePosition,
};
use rust_decimal::Decimal;
use serde_json::{Value, json};
#[path = "support/node.rs"]
mod node;
#[allow(dead_code, reason = "Shared source harness includes other recovery fixtures")]
mod support;
use node::{RetainedCache, account_node, run_account_node};
use support::{Harness, SERIAL};

struct TradeCase {
    position: VenuePosition,
    entry: JournalEntry,
    history: Value,
    receipt: Value,
    pnl: Decimal,
    adjustment: Decimal,
}

fn trade(harness: &Harness, id: u64, long: bool, now: u64, block: u64) -> TradeCase {
    let opened = now - if long { 400 } else { 150 };
    let position: VenuePosition = serde_json::from_value(json!([
        id.to_string(),
        0,
        long,
        false,
        "830000",
        if long {
            "166000000000000000000"
        } else {
            "249000000000000000000"
        },
        10,
        "0",
        opened * 1000,
        null
    ]))
    .unwrap();
    let entry = support::prepared_open_with_id(
        harness,
        &position,
        &format!("PT-OPEN-{id}"),
        now - 1000,
        now - 100,
    );
    support::mark_attempted(harness, &entry);
    let mut history =
        support::closed_history(&position, (now - if long { 300 } else { 50 }) * 1000);
    history["exitPriceRaw"] = json!(if long { "840000" } else { "820000" });
    history["adjustedPnlRaw"] = json!(if long {
        "19000000000000000000"
    } else {
        "27500000000000000000"
    });
    history["userPaidFeeRaw"] = json!(if long {
        "500000000000000000"
    } else {
        "400000000000000000"
    });
    history["builderFeeRaw"] = json!(if long {
        "250000000000000000"
    } else {
        "100000000000000000"
    });
    let tx = B256::repeat_byte((id % 200 + 1) as u8);
    let hash = B256::repeat_byte((id % 200 + 2) as u8);
    let height = format!("0x{block:x}");
    let wallet: alloy_primitives::Address = harness.config.wallet_address.parse().unwrap();
    let result = json!({"address":nautilus_papertrade::chain::BATCH_EXECUTOR,
        "topics":[nautilus_papertrade::chain::RESULT_TOPIC,B256::ZERO,B256::ZERO,entry.intent_id],
        "transactionHash":tx,"blockHash":hash,"blockNumber":height,"removed":false,
        "data":format!("0x{:064x}{:064x}{:064x}{:064x}",1,64,32,position.position_id.0)});
    let open = json!({"address":nautilus_papertrade::signing::EXCHANGE,
        "topics":["0x88451ea1189f73b554f0bf39c09868560ed149b3d788fcc933ff66ba926827f3",format!("0x{:064x}",position.position_id.0),format!("0x{:064x}",U256::from_be_slice(wallet.as_slice()))],
        "transactionHash":tx,"blockHash":hash,"blockNumber":height,"blockTimestamp":format!("0x{opened:x}"),"removed":false,
        "data":format!("0x{:064x}{:064x}{:064x}{:064x}{:064x}{:064x}",0,u8::from(long),0,position.entry_price.0,position.margin.0,position.leverage)});
    TradeCase {
        position,
        entry,
        history,
        receipt: json!({"status":"0x1","transactionHash":tx,"blockHash":hash,"blockNumber":height,"logs":[result,open]}),
        pnl: if long {
            Decimal::new(1825, 2)
        } else {
            Decimal::new(2700, 2)
        },
        adjustment: if long {
            Decimal::new(175, 2)
        } else {
            Decimal::new(300, 2)
        },
    }
}

fn publish(
    harness: &Harness,
    block: u64,
    timestamp: u64,
    positions: &[VenuePosition],
    history: &[Value],
    cash: &str,
) {
    let mut snapshot = harness.source.wallet.borrow().clone();
    snapshot["accountRevision"] = json!(snapshot["accountRevision"].as_u64().unwrap() + 1);
    snapshot["indexedThroughBlock"] = json!(block);
    snapshot["walletRevision"] = json!(block);
    snapshot["indexedThroughTimestamp"] = json!(timestamp);
    snapshot["locked"] = json!(
        positions
            .iter()
            .fold(U256::ZERO, |sum, p| sum + p.margin.0)
            .to_string()
    );
    snapshot["openPositions"] = json!(positions);
    snapshot["balance"] = json!(cash);
    snapshot["funds"]["availableBalance"] = json!(cash);
    *harness.source.history.lock() = json!({"items":history,"nextCursor":null});
    harness.source.trading.lock()["indexedThroughBlock"] = json!(block);
    harness.source.wallet.send_replace(snapshot);
}

fn journal(harness: &Harness) -> Journal {
    Journal::open(
        &harness.config.journal_path,
        harness.config.wallet_address.parse().unwrap(),
        "PAPERTRADE-001",
    )
    .unwrap()
}

async fn run_until(node: &mut LiveNode, ready: impl Fn() -> bool) {
    let handle = node.handle();
    let wait = async {
        let reached = tokio::time::timeout(Duration::from_secs(10), async {
            while !ready() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await;
        handle.stop();
        reached
    };
    let (run, reached) = tokio::join!(node.run(), wait);
    run.unwrap();
    reached.unwrap();
}

fn assert_economics(node: &LiveNode, cases: &[&TradeCase], expired: usize) {
    let cache = node.kernel().cache();
    let cache = cache.borrow();
    assert_eq!(
        cache.positions(None, None, None, None, None).len(),
        cases.len()
    );
    assert_eq!(
        cache.orders(None, None, None, None, None).len(),
        2 * cases.len() + expired
    );
    let mut ids = HashSet::new();
    for case in cases {
        let id = PositionId::from(format!("PT-{}", case.position.position_id));
        let position = cache.position(&id).unwrap();
        assert!(position.is_closed());
        assert_eq!(position.events.len(), 2);
        assert_eq!(position.trade_ids.len(), 2);
        assert_eq!(position.realized_pnl.unwrap().as_decimal(), case.pnl);
        assert_eq!(position.commissions()[0].as_decimal(), case.adjustment);
        for trade in &position.trade_ids {
            assert!(ids.insert(*trade));
        }
        let order = cache.order(&case.entry.command.client_order_id).unwrap();
        assert_eq!(order.status(), OrderStatus::Filled);
        assert_eq!(
            order.venue_order_id().unwrap().as_str(),
            format!("{:#x}", case.entry.intent_id)
        );
        assert_eq!(order.position_id(), Some(id));
        assert_eq!(order.trade_ids(), vec![&position.events[0].trade_id]);
        assert_eq!(
            order.filled_qty().as_decimal(),
            if case.position.is_long {
                Decimal::new(2, 2)
            } else {
                Decimal::new(3, 2)
            }
        );
        let close = cache
            .order(&ClientOrderId::from(format!(
                "PT-CLOSE-{}",
                case.position.position_id
            )))
            .unwrap();
        assert_eq!(close.status(), OrderStatus::Filled);
        assert_eq!(close.trade_ids(), vec![&position.events[1].trade_id]);
        assert_eq!(
            close
                .commissions()
                .get(&Currency::USDC())
                .unwrap()
                .as_decimal(),
            case.adjustment
        );
    }
    let account = cache.account(&AccountId::from("PAPERTRADE-001")).unwrap();
    assert_eq!(
        account
            .balance(Some(Currency::USDC()))
            .unwrap()
            .total
            .as_decimal(),
        Decimal::from(1000) + cases.iter().map(|c| c.pnl).sum::<Decimal>()
    );
}

#[tokio::test]
async fn live_node_retries_a_visible_log_when_its_receipt_is_unavailable() {
    let _serial = SERIAL.lock().await;
    let harness = Harness::new().await;
    let initial = harness.source.wallet.borrow().clone();
    let start = initial["indexedThroughBlock"].as_u64().unwrap();
    let now = initial["indexedThroughTimestamp"].as_u64().unwrap();
    let case = trade(&harness, 9001, true, now, start + 10);
    let hash = case.receipt["transactionHash"].as_str().unwrap().to_owned();
    *harness.source.receipts.lock() = vec![case.receipt.clone()];
    harness
        .source
        .receipt_unavailable
        .lock()
        .insert(hash.clone());
    publish(
        &harness,
        start + 30,
        now,
        &[],
        std::slice::from_ref(&case.history),
        "1018250000000000000000",
    );
    let mut failed = account_node(&harness);
    let handle = failed.handle();
    let stop = async {
        let called = tokio::time::timeout(Duration::from_secs(4), async {
            while !harness.source.receipt_calls.lock().contains(&hash) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        })
        .await;
        let running = handle.is_running();
        handle.stop();
        (called, running)
    };
    let (_run, (called, running)) = tokio::join!(failed.run(), stop);
    called.unwrap();
    assert!(!running);
    let before = RetainedCache::capture(&failed);
    assert!(before.orders.is_empty());
    assert!(before.positions.is_empty());
    failed.dispose();
    drop(failed);
    {
        let journal = journal(&harness);
        let saved = journal.entries().unwrap().pop().unwrap();
        assert_eq!(saved.scanned_through, start - 1);
        assert_eq!(saved.status, OrderStatus::Submitted);
        assert!(saved.fills.is_empty());
        assert!(!saved.execution_succeeded);
        assert!(journal.observed().unwrap().is_empty());
        assert!(
            journal
                .begin_submission(saved.command.client_order_id)
                .unwrap()
                .is_none()
        );
    }
    assert_eq!(harness.source.log_ranges.lock()[0], (start, start + 30));
    harness.source.log_ranges.lock().clear();
    harness.source.receipt_unavailable.lock().clear();
    let mut recovered = account_node(&harness);
    let cache = recovered.kernel().cache();
    run_until(&mut recovered, || {
        cache
            .borrow()
            .position(&PositionId::from("PT-9001"))
            .is_some_and(|p| p.is_closed())
    })
    .await;
    assert_eq!(harness.source.log_ranges.lock()[0], (start, start + 30));
    assert_economics(&recovered, &[&case], 0);
    let retained = RetainedCache::capture(&recovered);
    recovered.dispose();
    drop(recovered);
    let mut restarted = account_node(&harness);
    retained.restore(&restarted);
    run_account_node(&mut restarted).await;
    assert_economics(&restarted, &[&case], 0);
    assert_eq!(
        RetainedCache::capture(&restarted).positions[0].trade_ids,
        retained.positions[0].trade_ids
    );
    restarted.dispose();
}
