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

//! Loopback admission faults after preparation; no production venue is contacted.

use nautilus_core::UUID4;
use nautilus_model::{
    enums::AccountType,
    identifiers::{ClientOrderId, TraderId},
    orders::OrderTestBuilder,
    types::{Currency, Quantity},
};
use rstest::rstest;

use super::*;

#[path = "../tests/support/mod.rs"]
#[allow(
    dead_code,
    reason = "The shared harness also serves recovery and coverage tests"
)]
mod support;

fn client(harness: &support::Harness) -> (PapertradeExecutionClient, &'static AtomicTime, u64) {
    let core = ExecutionClientCore::new(
        TraderId::from("TESTER-001"),
        ClientId::from(PAPERTRADE),
        Venue::from(PAPERTRADE),
        OmsType::Hedging,
        harness.config.account_id,
        AccountType::Margin,
        Some(Currency::USDC()),
        harness.cache.clone(),
    );
    let mut client = PapertradeExecutionClient::new(core, harness.config.clone()).unwrap();
    let now = client.now();
    // Each test owns its static clock; never mutate the global realtime clock.
    let clock = Box::leak(Box::new(AtomicTime::new(
        false,
        UnixNanos::from(now * 1_000_000_000),
    )));
    Arc::get_mut(&mut client.inner).unwrap().clock = clock;
    (client, clock, now)
}

fn order(harness: &support::Harness) -> (OrderAny, SubmitOrder) {
    let order = OrderTestBuilder::new(OrderType::Market)
        .instrument_id(InstrumentId::from("BTC-USD-PERP.PAPERTRADE"))
        .client_order_id(ClientOrderId::from("ADMISSION-001"))
        .quantity(Quantity::from(1000))
        .quote_quantity(true)
        .time_in_force(TimeInForce::Gtc)
        .build();
    harness
        .cache
        .borrow_mut()
        .add_order(order.clone(), None, None, false)
        .unwrap();
    let command = SubmitOrder::from_order(
        &order,
        TraderId::from("TESTER-001"),
        None,
        None,
        UUID4::new(),
        UnixNanos::from(1),
    );
    (order, command)
}

async fn fault(
    harness: &support::Harness,
    client: &PapertradeExecutionClient,
    clock: &AtomicTime,
    deadline: u64,
    damage: &str,
) {
    match damage {
        "expired" => clock.set_time(UnixNanos::from(deadline * 1_000_000_000)),
        "revoked" | "expiry_equal" => {
            let mut snapshot = harness.source.wallet.borrow().clone();
            let revision = snapshot["accountRevision"].as_u64().unwrap() + 1;
            snapshot["accountRevision"] = serde_json::json!(revision);
            snapshot["indexedThroughBlock"] =
                serde_json::json!(snapshot["indexedThroughBlock"].as_u64().unwrap() + 1);
            if damage == "revoked" {
                snapshot["sessionKeys"] = serde_json::json!([]);
            } else {
                snapshot["sessionKeys"][0][1] = serde_json::json!(deadline.to_string());
            }
            harness.source.wallet.send_replace(snapshot);
            tokio::time::timeout(Duration::from_secs(3), async {
                loop {
                    if client
                        .inner
                        .snapshot
                        .read()
                        .as_ref()
                        .is_some_and(|snapshot| snapshot.account_revision == revision)
                    {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            })
            .await
            .unwrap();
            // Let the same synchronization pass publish its availability flags too.
            let _complete = client.inner.sync.lock().await;
        }
        "unavailable" => client.inner.stream_healthy.store(false, Ordering::Release),
        "valid" => {}
        _ => unreachable!(),
    }
}

#[rstest]
#[case("expired")]
#[case("revoked")]
#[case("expiry_equal")]
#[case("unavailable")]
#[case("valid")]
#[tokio::test]
async fn submission_admission_rechecks_prepared_orders_before_transport(#[case] damage: &str) {
    let _serial = support::SERIAL.lock().await;
    let harness = support::Harness::new().await;
    let (mut client, clock, _now) = client(&harness);
    let (order, command) = order(&harness);
    client.start().unwrap();
    client.connect().await.unwrap();
    assert!(client.inner.available());
    let (entry, new) = client.prepare_order(&command, &order).unwrap();
    assert!(new);
    fault(&harness, &client, clock, entry.deadline, damage).await;
    let result = client.inner.submit_prepared(command.client_order_id).await;
    let persisted = client.inner.journal.entries().unwrap().remove(0);
    assert_eq!(persisted.intent_id, entry.intent_id);
    assert_eq!(persisted.body, entry.body);
    assert_eq!(persisted.nonce, entry.nonce);
    let allowed = damage == "valid";
    assert_eq!(result.is_ok(), allowed, "{result:?}");
    assert_eq!(
        harness.source.intent_requests.load(Ordering::SeqCst),
        usize::from(allowed)
    );
    assert_eq!(persisted.send_attempted, allowed);
    if allowed {
        client
            .inner
            .submit_prepared(command.client_order_id)
            .await
            .unwrap();
        assert_eq!(harness.source.intent_requests.load(Ordering::SeqCst), 1);
    }
    client.disconnect().await.unwrap();
}

#[rstest]
#[case("expired")]
#[case("revoked")]
#[case("expiry_equal")]
#[case("unavailable")]
#[case("valid")]
#[tokio::test]
async fn submission_admission_rechecks_prepared_cancels_with_current_time(#[case] damage: &str) {
    let _serial = support::SERIAL.lock().await;
    let harness = support::Harness::new().await;
    let (mut client, clock, now) = client(&harness);
    let (order, command) = order(&harness);
    client.start().unwrap();
    client.connect().await.unwrap();
    assert!(client.inner.available());
    let (entry, _new) = client.prepare_order(&command, &order).unwrap();
    let cancel = client
        .inner
        .signer
        .as_ref()
        .unwrap()
        .sign_cancel(
            &CancelIntent {
                user: client.inner.wallet,
                intentId: entry.intent_id,
                deadline: U256::from(now + 3_600),
            },
            &entry.signed().unwrap(),
            now,
        )
        .unwrap();
    fault(&harness, &client, clock, cancel.deadline(), damage).await;
    let result = client
        .inner
        .cancel_prepared(&cancel, entry.clone(), now)
        .await;
    let persisted = client.inner.journal.entries().unwrap().remove(0);
    assert_eq!(persisted.intent_id, entry.intent_id);
    assert_eq!(persisted.body, entry.body);
    assert_eq!(persisted.status, entry.status);
    let allowed = damage == "valid";
    assert_eq!(result.is_ok(), allowed, "{result:?}");
    assert_eq!(
        harness.source.intent_requests.load(Ordering::SeqCst),
        usize::from(allowed)
    );
    client.disconnect().await.unwrap();
}

#[rstest]
#[case("queued")]
#[case("confirmed")]
#[case("failed")]
#[case("reverted")]
#[case("expired")]
#[case("cancelled")]
#[tokio::test]
async fn submission_admission_relayer_labels_do_not_establish_a_terminal_order(
    #[case] state: &str,
) {
    let _serial = support::SERIAL.lock().await;
    let harness = support::Harness::new().await;
    let (mut client, _clock, _now) = client(&harness);
    let (order, command) = order(&harness);
    client.start().unwrap();
    client.connect().await.unwrap();
    let (entry, _) = client.prepare_order(&command, &order).unwrap();
    *harness.source.intent_ack.lock() = Some(serde_json::json!({
        "intentId": entry.intent_id, "action": entry.action, "state": state,
    }));
    client
        .inner
        .submit_prepared(command.client_order_id)
        .await
        .unwrap();
    let persisted = client.inner.journal.entries().unwrap().remove(0);
    assert!(matches!(
        persisted.status,
        OrderStatus::Submitted | OrderStatus::Accepted
    ));
    assert!(persisted.send_attempted);
    assert_eq!(harness.source.intent_requests.load(Ordering::SeqCst), 1);
    client.disconnect().await.unwrap();
}

fn cancellation(
    client: &PapertradeExecutionClient,
    entry: &crate::journal::JournalEntry,
    now: u64,
) -> crate::signing::SignedIntent {
    client
        .inner
        .signer
        .as_ref()
        .unwrap()
        .sign_cancel(
            &CancelIntent {
                user: client.inner.wallet,
                intentId: entry.intent_id,
                deadline: U256::from(now + 3_600),
            },
            &entry.signed().unwrap(),
            now,
        )
        .unwrap()
}

#[tokio::test]
async fn submission_admission_cancel_ack_does_not_publish_terminal_success() {
    let _serial = support::SERIAL.lock().await;
    let mut harness = support::Harness::new().await;
    let (mut client, _clock, now) = client(&harness);
    let (order, command) = order(&harness);
    client.start().unwrap();
    client.connect().await.unwrap();
    let (entry, _) = client.prepare_order(&command, &order).unwrap();
    let cancel = cancellation(&client, &entry, now);
    *harness.source.intent_ack.lock() = Some(serde_json::json!({
        "intentId": entry.intent_id, "action": entry.action, "state": "cancelled",
    }));
    client
        .inner
        .cancel_prepared(&cancel, entry.clone(), now)
        .await
        .unwrap();
    assert_eq!(
        client.inner.journal.entries().unwrap()[0].status,
        entry.status
    );
    while let Ok(event) = harness._events.try_recv() {
        assert!(!matches!(
            event,
            nautilus_common::messages::ExecutionEvent::Order(
                nautilus_model::events::OrderEventAny::Canceled(_)
            )
        ));
    }
    assert_eq!(harness.source.intent_requests.load(Ordering::SeqCst), 1);
    client.disconnect().await.unwrap();
}

#[tokio::test]
async fn submission_admission_cancel_refuses_a_target_already_terminal_before_send() {
    let _serial = support::SERIAL.lock().await;
    let harness = support::Harness::new().await;
    let (mut client, _clock, now) = client(&harness);
    let (order, command) = order(&harness);
    client.start().unwrap();
    client.connect().await.unwrap();
    let (entry, _) = client.prepare_order(&command, &order).unwrap();
    let cancel = cancellation(&client, &entry, now);
    let mut terminal = entry.clone();
    terminal.status = OrderStatus::Expired;
    client.inner.journal.update(&terminal).unwrap();
    assert!(
        client
            .inner
            .cancel_prepared(&cancel, entry, now)
            .await
            .is_err()
    );
    assert_eq!(harness.source.intent_requests.load(Ordering::SeqCst), 0);
    assert_eq!(
        client.inner.journal.entries().unwrap()[0].status,
        OrderStatus::Expired
    );
    client.disconnect().await.unwrap();
}

#[rstest]
#[case(false, "failed")]
#[case(true, "cancelled")]
#[tokio::test]
async fn submission_admission_a_late_ack_preserves_newer_terminal_evidence(
    #[case] cancel_order: bool,
    #[case] state: &str,
) {
    let _serial = support::SERIAL.lock().await;
    let harness = support::Harness::new().await;
    let (mut client, _clock, now) = client(&harness);
    let (order, command) = order(&harness);
    client.start().unwrap();
    client.connect().await.unwrap();
    let (entry, _) = client.prepare_order(&command, &order).unwrap();
    let cancel = cancellation(&client, &entry, now);
    *harness.source.intent_ack.lock() = Some(serde_json::json!({
        "intentId": entry.intent_id, "action": entry.action, "state": state,
    }));
    harness.source.intent_paused.store(true, Ordering::SeqCst);
    let send = async {
        if cancel_order {
            client
                .inner
                .cancel_prepared(&cancel, entry.clone(), now)
                .await
        } else {
            client.inner.submit_prepared(command.client_order_id).await
        }
    };
    let advance = async {
        tokio::time::timeout(
            Duration::from_secs(3),
            harness.source.intent_entered.notified(),
        )
        .await
        .unwrap();
        let _serial = client.inner.sync.lock().await;
        let mut terminal = client.inner.journal.entries().unwrap().remove(0);
        terminal.status = OrderStatus::Expired;
        terminal.scanned_through += 1;
        client.inner.journal.update(&terminal).unwrap();
        harness.source.intent_release.notify_one();
    };
    let (result, ()) = tokio::join!(send, advance);
    result.unwrap();
    let persisted = client.inner.journal.entries().unwrap().remove(0);
    assert_eq!(persisted.status, OrderStatus::Expired);
    assert!(persisted.scanned_through > entry.scanned_through);
    assert_eq!(harness.source.intent_requests.load(Ordering::SeqCst), 1);
    client.disconnect().await.unwrap();
}

#[tokio::test]
async fn submission_admission_cancel_reports_storage_loss_after_receiving_an_ack() {
    let _serial = support::SERIAL.lock().await;
    let harness = support::Harness::new().await;
    let (mut client, _clock, now) = client(&harness);
    let (order, command) = order(&harness);
    client.start().unwrap();
    client.connect().await.unwrap();
    let (entry, _) = client.prepare_order(&command, &order).unwrap();
    let cancel = cancellation(&client, &entry, now);
    *harness.source.intent_ack.lock() = Some(serde_json::json!({
        "intentId": entry.intent_id, "action": entry.action, "state": "cancelled",
    }));
    harness.source.intent_paused.store(true, Ordering::SeqCst);
    let send = client.inner.cancel_prepared(&cancel, entry, now);
    let remove_storage = async {
        tokio::time::timeout(
            Duration::from_secs(3),
            harness.source.intent_entered.notified(),
        )
        .await
        .unwrap();
        let _serial = client.inner.sync.lock().await;
        std::fs::remove_file(&harness.config.journal_path).unwrap();
        harness.source.intent_release.notify_one();
    };
    let (result, ()) = tokio::join!(send, remove_storage);
    assert!(
        result.is_err(),
        "an ACK cannot hide unavailable journal storage"
    );
    assert_eq!(harness.source.intent_requests.load(Ordering::SeqCst), 1);
    client.disconnect().await.unwrap();
}

#[tokio::test]
async fn submission_admission_cancel_reads_time_after_waiting_for_reconciliation() {
    let _serial = support::SERIAL.lock().await;
    let harness = support::Harness::new().await;
    let (mut client, clock, now) = client(&harness);
    let (order, command) = order(&harness);
    client.start().unwrap();
    client.connect().await.unwrap();
    let (entry, _) = client.prepare_order(&command, &order).unwrap();
    let cancel = cancellation(&client, &entry, now);
    let lock = client.inner.sync.lock().await;
    let send = client.inner.cancel_prepared(&cancel, entry, now);
    let advance = async {
        clock.set_time(UnixNanos::from(cancel.deadline() * 1_000_000_000));
        drop(lock);
    };
    let (result, ()) = tokio::join!(send, advance);
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("expired before submission")
    );
    assert_eq!(harness.source.intent_requests.load(Ordering::SeqCst), 0);
    client.disconnect().await.unwrap();
}

#[rstest]
#[case(true, true)]
#[case(true, false)]
#[case(false, true)]
#[case(false, false)]
#[tokio::test]
async fn submission_admission_lock_waits_invalidate_earlier_time_and_availability(
    #[case] account_lock: bool,
    #[case] expire: bool,
) {
    let _serial = support::SERIAL.lock().await;
    let harness = support::Harness::new().await;
    let (mut client, clock, _now) = client(&harness);
    let (order, command) = order(&harness);
    client.start().unwrap();
    client.connect().await.unwrap();
    let (entry, _) = client.prepare_order(&command, &order).unwrap();
    let signed = entry.signed().unwrap();
    let held = (
        account_lock.then(|| client.inner.snapshot.write()),
        (!account_lock).then(|| client.inner.stream_activity.write()),
    );
    let inner = client.inner.clone();
    let (entered, entering) = std::sync::mpsc::channel();
    let (completed, completion) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        entered.send(()).unwrap();
        completed
            .send(
                inner
                    .submission_time(&signed)
                    .map_err(|error| error.to_string()),
            )
            .unwrap();
    });
    entering.recv_timeout(Duration::from_secs(3)).unwrap();
    let blocked = completion.recv_timeout(Duration::from_millis(50));
    if expire {
        clock.set_time(UnixNanos::from(entry.deadline * 1_000_000_000));
    } else {
        client.inner.stream_healthy.store(false, Ordering::Release);
    }
    drop(held);
    let result = completion.recv_timeout(Duration::from_secs(3));
    worker.join().unwrap();
    client.disconnect().await.unwrap();
    assert!(matches!(
        blocked,
        Err(std::sync::mpsc::RecvTimeoutError::Timeout)
    ));
    assert!(
        result.unwrap().is_err(),
        "a lock wait must invalidate an earlier check"
    );
}
