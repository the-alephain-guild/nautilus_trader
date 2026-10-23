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
#[allow(
    dead_code,
    reason = "Shared source harness includes other recovery fixtures"
)]
mod support;
use node::{RetainedCache, account_node, run_account_node};
use support::scan::ScanCoverage;
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

fn check_scan_coverage(harness: &Harness, coverage: &mut ScanCoverage, bound: u64) {
    let checkpoints: Vec<_> = journal(harness)
        .entries()
        .unwrap()
        .iter()
        .map(|entry| {
            (
                entry.intent_id,
                entry.scanned_through,
                entry.status == OrderStatus::Expired,
            )
        })
        .collect();
    coverage
        .verify(&harness.source.scan_events.lock(), &checkpoints, bound)
        .unwrap();
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

fn fill_identity(
    node: &LiveNode,
) -> Vec<(ClientOrderId, Vec<nautilus_model::identifiers::TradeId>)> {
    let cache = node.kernel().cache();
    let cache = cache.borrow();
    let mut identity: Vec<_> = cache
        .orders(None, None, None, None, None)
        .into_iter()
        .map(|order| {
            (
                order.client_order_id(),
                order.trade_ids().into_iter().copied().collect(),
            )
        })
        .collect();
    identity.sort_by_key(|row| row.0);
    identity
}

struct KillOnDrop(std::process::Child);
impl Drop for KillOnDrop {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

#[tokio::test]
async fn live_node_recovers_mixed_cursors_after_a_partial_commit_and_growing_coverage() {
    use std::{
        io::{BufRead, BufReader, Write},
        process::{Command, Stdio},
    };
    const CHILD: &str = "PAPERTRADE_TEST_PARTIAL_CURSOR";
    const READY: &str = "FIRST_CURSOR_COMMITTED";
    if let Ok(value) = std::env::var(CHILD) {
        let value: Value = serde_json::from_str(&value).unwrap();
        let journal = Journal::open(
            std::path::Path::new(value["path"].as_str().unwrap()),
            value["wallet"].as_str().unwrap().parse().unwrap(),
            "PAPERTRADE-001",
        )
        .unwrap();
        let mut entries = journal.entries().unwrap();
        let from = entries.iter().map(|e| e.scanned_through + 1).min().unwrap();
        let to = from + 999;
        let ids: Vec<_> = entries.iter().map(|e| e.intent_id).collect();
        let api = nautilus_papertrade::api::PapertradeApi::new(
            value["origin"].as_str().unwrap().parse().unwrap(),
            format!("{}rpc", value["origin"].as_str().unwrap())
                .parse()
                .unwrap(),
            1,
        )
        .unwrap();
        assert!(
            api.intent_receipts(&ids, from, to)
                .await
                .unwrap()
                .is_empty()
        );
        entries[0].scanned_through = entries[0].scanned_through.max(to);
        journal.update(&entries[0]).unwrap();
        println!("{READY}");
        std::io::stdout().flush().unwrap();
        let mut line = String::new();
        std::io::stdin().read_line(&mut line).unwrap();
        panic!("The parent must terminate the writer before the next order commit");
    }
    let _serial = SERIAL.lock().await;
    let harness = Harness::new().await;
    let initial = harness.source.wallet.borrow().clone();
    let start = initial["indexedThroughBlock"].as_u64().unwrap();
    let now = initial["indexedThroughTimestamp"].as_u64().unwrap();
    let a = trade(&harness, 9001, true, now, start + 2200);
    let b = trade(&harness, 9002, false, now, start + 5200);
    let unused = support::prepared_open_with_id(
        &harness,
        &a.position,
        "PT-OPEN-9003",
        now - 1000,
        now - 100,
    );
    support::mark_attempted(&harness, &unused);
    let mut coverage = ScanCoverage::new([
        (a.entry.intent_id, start + 999),
        (b.entry.intent_id, start + 1499),
        (unused.intent_id, start + 3999),
    ]);
    {
        let journal = journal(&harness);
        for (entry, progress) in journal
            .entries()
            .unwrap()
            .into_iter()
            .zip([999, 1499, 3999])
        {
            let mut entry = entry;
            entry.scanned_through = start + progress;
            journal.update(&entry).unwrap();
        }
    }
    *harness.source.receipts.lock() = vec![a.receipt.clone(), b.receipt.clone()];
    publish(
        &harness,
        start + 5000,
        now - 200,
        &[],
        std::slice::from_ref(&a.history),
        "1018250000000000000000",
    );
    let mut child = KillOnDrop(Command::new(std::env::current_exe().unwrap())
        .args(["--exact", std::thread::current().name().unwrap(), "--nocapture"])
        .env(CHILD, json!({"path":harness.config.journal_path,"wallet":harness.config.wallet_address,"origin":harness.config.origin}).to_string())
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::inherit()).spawn().unwrap());
    let stdout = child.0.stdout.take().unwrap();
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            if line.unwrap() == READY {
                let _ = sender.send(());
                return;
            }
        }
    });
    let reached = tokio::time::timeout(Duration::from_secs(8), receiver).await;
    child.0.kill().unwrap();
    let status = child.0.wait().unwrap();
    reader.join().unwrap();
    reached.unwrap().unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(status.signal(), Some(9));
    }
    assert!(!status.success());
    check_scan_coverage(&harness, &mut coverage, start + 5000);
    {
        let journal = journal(&harness);
        let entries = journal.entries().unwrap();
        for ((saved, original), progress) in entries
            .iter()
            .zip([&a.entry, &b.entry, &unused])
            .zip([1999, 1499, 3999])
        {
            assert_eq!(saved.scanned_through, start + progress);
            assert_eq!(saved.intent_id, original.intent_id);
            assert_eq!(saved.body, original.body);
            assert_eq!(saved.nonce, original.nonce);
            assert!(saved.fills.is_empty());
            assert!(
                journal
                    .begin_submission(saved.command.client_order_id)
                    .unwrap()
                    .is_none()
            );
        }
    }
    assert_eq!(
        *harness.source.log_ranges.lock(),
        vec![(start + 1000, start + 1999)]
    );
    harness.source.log_ranges.lock().clear();
    let mut partial = account_node(&harness);
    let cache = partial.kernel().cache();
    run_until(&mut partial, || {
        cache
            .borrow()
            .position(&PositionId::from("PT-9001"))
            .is_some_and(|p| p.is_closed())
    })
    .await;
    assert_eq!(
        harness.source.log_ranges.lock()[0],
        (start + 1500, start + 2499)
    );
    {
        let cache = cache.borrow();
        assert_eq!(cache.positions(None, None, None, None, None).len(), 1);
        assert_eq!(
            cache
                .position(&PositionId::from("PT-9001"))
                .unwrap()
                .realized_pnl
                .unwrap()
                .as_decimal(),
            a.pnl
        );
        for id in [
            b.entry.command.client_order_id,
            unused.command.client_order_id,
        ] {
            let order = cache.order(&id).unwrap();
            assert!(matches!(
                order.status(),
                OrderStatus::Submitted | OrderStatus::Accepted
            ));
            assert!(order.trade_ids().is_empty());
        }
    }
    let retained = RetainedCache::capture(&partial);
    partial.dispose();
    drop(partial);
    check_scan_coverage(&harness, &mut coverage, start + 5000);
    let slow_cursor = {
        let journal = journal(&harness);
        let entries = journal.entries().unwrap();
        assert!(
            entries[2].scanned_through >= start + 3999,
            "The advanced cursor regressed"
        );
        assert_eq!(entries[2].status, OrderStatus::Submitted);
        entries[1].scanned_through
    };
    harness.source.log_ranges.lock().clear();
    let mut recovered = account_node(&harness);
    retained.restore(&recovered);
    let cache = recovered.kernel().cache();
    let advanced = std::cell::Cell::new(false);
    run_until(&mut recovered, || {
        if !advanced.get() && !harness.source.log_ranges.lock().is_empty() {
            publish(
                &harness,
                start + 5500,
                now,
                &[],
                &[a.history.clone(), b.history.clone()],
                "1045250000000000000000",
            );
            *harness.source.block_timestamp.lock() = Some(now);
            advanced.set(true);
        }
        let cache = cache.borrow();
        advanced.get()
            && cache
                .position(&PositionId::from("PT-9002"))
                .is_some_and(|p| p.is_closed())
            && cache
                .order(&unused.command.client_order_id)
                .is_some_and(|o| o.status() == OrderStatus::Expired)
    })
    .await;
    assert_eq!(harness.source.log_ranges.lock()[0].0, slow_cursor + 1);
    assert!(
        harness
            .source
            .log_ranges
            .lock()
            .iter()
            .all(|(from, to)| to - from < 1000)
    );
    assert!(
        harness
            .source
            .log_ranges
            .lock()
            .iter()
            .any(|(_, to)| *to == start + 5500)
    );
    assert_economics(&recovered, &[&a, &b], 1);
    let identity = fill_identity(&recovered);
    recovered.dispose();
    drop(recovered);
    check_scan_coverage(&harness, &mut coverage, start + 5500);
    {
        let entries = journal(&harness).entries().unwrap();
        assert_eq!(entries[2].status, OrderStatus::Expired);
        assert_eq!(entries[2].scanned_through, start + 5500);
        assert!(entries[2].fills.is_empty());
    }
    let mut restarted = account_node(&harness);
    run_account_node(&mut restarted).await;
    assert_economics(&restarted, &[&a, &b], 1);
    assert_eq!(fill_identity(&restarted), identity);
    restarted.dispose();
    drop(restarted);
    check_scan_coverage(&harness, &mut coverage, start + 5500);
}

async fn wait_for(ready: impl Fn() -> bool) -> Result<(), tokio::time::error::Elapsed> {
    tokio::time::timeout(Duration::from_secs(8), async {
        while !ready() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
}

#[rstest::rstest]
#[case::long_proof_first(true)]
#[case::short_proof_first(false)]
#[tokio::test]
async fn live_node_attributes_two_unbound_positions_independently_across_restart(
    #[case] long_first: bool,
) {
    use std::sync::atomic::Ordering;
    let _serial = SERIAL.lock().await;
    let harness = Harness::new().await;
    let initial = harness.source.wallet.borrow().clone();
    let start = initial["indexedThroughBlock"].as_u64().unwrap();
    let now = initial["indexedThroughTimestamp"].as_u64().unwrap();
    let mut a = trade(
        &harness,
        9001,
        true,
        now,
        start + if long_first { 10 } else { 20 },
    );
    let mut b = trade(
        &harness,
        9002,
        false,
        now,
        start + if long_first { 20 } else { 10 },
    );
    for case in [&mut a, &mut b] {
        let opened = now
            - if case.position.is_long == long_first {
                400
            } else {
                150
            };
        case.position.opened_at_ms = opened * 1000;
        case.history["openedAtMs"] = json!(opened * 1000);
        case.history["timestampMs"] = json!((now - 50) * 1000);
        case.receipt["logs"][1]["blockTimestamp"] = json!(format!("0x{opened:x}"));
    }
    let (first, second) = if long_first { (&a, &b) } else { (&b, &a) };
    let first_id = PositionId::from(format!("PT-{}", first.position.position_id));
    let second_id = PositionId::from(format!("PT-{}", second.position.position_id));
    *harness.source.receipts.lock() = vec![a.receipt.clone(), b.receipt.clone()];
    publish(
        &harness,
        start + 30,
        now,
        &[a.position.clone(), b.position.clone()],
        &[],
        "585000000000000000000",
    );
    harness.source.trading.lock()["indexedThroughBlock"] = json!(start - 1);
    let mut partial = account_node(&harness);
    let cache = partial.kernel().cache();
    let handle = partial.handle();
    let empty_economics = || {
        let cache = cache.borrow();
        cache.positions(None, None, None, None, None).is_empty()
            && cache.orders(None, None, None, None, None).len() == 2
            && [&a, &b].iter().all(|case| {
                cache
                    .order(&case.entry.command.client_order_id)
                    .is_some_and(|o| o.trade_ids().is_empty())
            })
    };
    let advance = async {
        let outcome = async {
            wait_for(|| harness.source.history_calls.load(Ordering::SeqCst) >= 2).await?;
            tokio::time::sleep(Duration::from_millis(100)).await;
            let before_history = empty_economics();
            let calls = harness.source.history_calls.load(Ordering::SeqCst);
            publish(
                &harness,
                start + 30,
                now,
                &[],
                &[a.history.clone(), b.history.clone()],
                "1045250000000000000000",
            );
            harness.source.trading.lock()["indexedThroughBlock"] = json!(start - 1);
            wait_for(|| harness.source.history_calls.load(Ordering::SeqCst) >= calls + 2).await?;
            tokio::time::sleep(Duration::from_millis(100)).await;
            let before_proof = empty_economics();
            *harness.source.block_timestamp.lock() = Some(now - 400);
            harness.source.trading.lock()["indexedThroughBlock"] = json!(start + 10);
            wait_for(|| {
                cache
                    .borrow()
                    .position(&first_id)
                    .is_some_and(|p| p.is_closed())
            })
            .await?;
            Ok::<_, tokio::time::error::Elapsed>((before_history, before_proof))
        }
        .await;
        handle.stop();
        outcome
    };
    let (run, stages) = tokio::join!(partial.run(), advance);
    run.unwrap();
    assert_eq!(
        stages.unwrap(),
        (true, true),
        "Unbound positions were published before attribution was proven"
    );
    {
        let cache = cache.borrow();
        assert_eq!(cache.positions(None, None, None, None, None).len(), 1);
        assert!(cache.position(&second_id).is_none());
        let position = cache.position(&first_id).unwrap();
        assert_eq!(position.realized_pnl.unwrap().as_decimal(), first.pnl);
        assert_eq!(position.commissions()[0].as_decimal(), first.adjustment);
        assert_eq!(position.events.len(), 2);
        assert_eq!(
            cache
                .order(&first.entry.command.client_order_id)
                .unwrap()
                .position_id(),
            Some(first_id)
        );
        assert!(
            cache
                .order(&second.entry.command.client_order_id)
                .unwrap()
                .trade_ids()
                .is_empty()
        );
    }
    let retained = RetainedCache::capture(&partial);
    let first_trades = retained.positions[0].trade_ids.clone();
    partial.dispose();
    drop(partial);
    {
        let journal = journal(&harness);
        let observed = journal.observed().unwrap();
        assert_eq!(observed.len(), 2);
        assert_eq!(
            observed
                .iter()
                .find(|p| p.position_id == first.position.position_id)
                .unwrap()
                .intent_id,
            Some(first.entry.intent_id)
        );
        assert!(
            observed
                .iter()
                .find(|p| p.position_id == second.position.position_id)
                .unwrap()
                .intent_id
                .is_none()
        );
        let entries = journal.entries().unwrap();
        assert_eq!(
            entries
                .iter()
                .find(|e| e.intent_id == first.entry.intent_id)
                .unwrap()
                .status,
            OrderStatus::Filled
        );
        assert!(
            entries
                .iter()
                .find(|e| e.intent_id == second.entry.intent_id)
                .unwrap()
                .fills
                .is_empty()
        );
    }
    let calls = harness.source.history_calls.load(Ordering::SeqCst);
    let mut recovered = account_node(&harness);
    retained.restore(&recovered);
    let cache = recovered.kernel().cache();
    let handle = recovered.handle();
    let finish = async {
        let outcome = async {
            wait_for(|| harness.source.history_calls.load(Ordering::SeqCst) >= calls + 2).await?;
            tokio::time::sleep(Duration::from_millis(100)).await;
            let retained_unchanged = {
                let cache = cache.borrow();
                let position = cache.position(&first_id).unwrap();
                position.realized_pnl.unwrap().as_decimal() == first.pnl
                    && position.commissions()[0].as_decimal() == first.adjustment
                    && position.trade_ids == first_trades
                    && position.events.len() == 2
                    && cache.position(&second_id).is_none()
                    && cache
                        .order(&second.entry.command.client_order_id)
                        .unwrap()
                        .trade_ids()
                        .is_empty()
            };
            *harness.source.block_timestamp.lock() = Some(now);
            harness.source.trading.lock()["indexedThroughBlock"] = json!(start + 30);
            wait_for(|| {
                cache
                    .borrow()
                    .position(&second_id)
                    .is_some_and(|p| p.is_closed())
            })
            .await?;
            Ok::<_, tokio::time::error::Elapsed>(retained_unchanged)
        }
        .await;
        handle.stop();
        outcome
    };
    let (run, preserved) = tokio::join!(recovered.run(), finish);
    run.unwrap();
    assert!(
        preserved.unwrap(),
        "Restart changed the already-attributed position or prematurely attributed the remaining one"
    );
    assert_economics(&recovered, &[&a, &b], 0);
    let identity = fill_identity(&recovered);
    recovered.dispose();
    drop(recovered);
    harness.source.receipts.lock().clear();
    let mut cold = account_node(&harness);
    run_account_node(&mut cold).await;
    assert_economics(&cold, &[&a, &b], 0);
    assert_eq!(fill_identity(&cold), identity);
    cold.dispose();
}
