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
pub(crate) mod support;

fn client(harness: &support::Harness) -> (PapertradeExecutionClient, &'static AtomicTime, u64) {
    let now = get_atomic_clock_realtime().get_time_ns().as_u64() / 1_000_000_000;
    client_at(harness, now)
}

pub(crate) fn client_at(
    harness: &support::Harness,
    now: u64,
) -> (PapertradeExecutionClient, &'static AtomicTime, u64) {
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
    // Each test owns its static clock; never mutate the global realtime clock.
    let clock = Box::leak(Box::new(AtomicTime::new(
        false,
        UnixNanos::from(now * 1_000_000_000),
    )));
    Arc::get_mut(&mut client.inner).unwrap().clock = clock;
    (client, clock, now)
}

pub(crate) fn order(harness: &support::Harness) -> (OrderAny, SubmitOrder) {
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

fn expiry_candidates(
    harness: &support::Harness,
    ids: &[&str],
) -> (
    PapertradeExecutionClient,
    AccountSnapshot,
    Vec<crate::journal::JournalEntry>,
    Vec<crate::journal::ExpiryCandidate>,
) {
    let snapshot: AccountSnapshot =
        serde_json::from_value(harness.source.wallet.borrow().clone()).unwrap();
    let position = &snapshot.open_positions[0];
    let now = snapshot.indexed_through_timestamp;
    for id in ids {
        let entry = support::prepared_open_with_id(harness, position, id, now - 7200, now - 3600);
        support::mark_attempted(harness, &entry);
    }
    let (client, _, _) = client(harness);
    let mut entries = client.inner.journal.entries().unwrap();
    let candidates = entries
        .iter_mut()
        .map(|entry| {
            entry.scanned_through = snapshot.indexed_through_block;
            client.inner.journal.update(entry).unwrap();
            crate::journal::ExpiryCandidate::new(
                entry,
                &snapshot,
                snapshot.indexed_through_block,
                now,
            )
            .unwrap()
        })
        .collect();
    (client, snapshot, entries, candidates)
}

#[rstest]
#[case("success")]
#[case("fill")]
#[case("service_terminal")]
#[tokio::test]
async fn expiry_checkpoint_rechecks_new_facts_after_candidate_creation(#[case] change: &str) {
    let _serial = support::SERIAL.lock().await;
    let harness = support::Harness::new().await;
    let (client, snapshot, mut entries, candidates) = expiry_candidates(&harness, &["EXPIRY-A"]);
    let entry = &mut entries[0];
    match change {
        "success" => client
            .inner
            .journal
            .observe_successes(&[entry.intent_id])
            .unwrap(),
        "fill" => {
            let trading: crate::wire::TradingState =
                serde_json::from_value(harness.source.trading.lock().clone()).unwrap();
            entry.status = OrderStatus::Filled;
            entry.fills = vec![
                crate::reports::open_fill(
                    &snapshot.open_positions[0],
                    &trading.instruments[0],
                    harness.config.account_id,
                    Some(entry.command.client_order_id),
                    UnixNanos::from(1),
                )
                .unwrap(),
            ];
            client.inner.journal.update(entry).unwrap();
        }
        "service_terminal" => {
            entry.status = OrderStatus::Rejected;
            client.inner.journal.update(entry).unwrap();
        }
        _ => unreachable!(),
    }
    let before = client.inner.journal.identity_snapshot().unwrap();
    assert!(
        client
            .inner
            .journal
            .checkpoint_with_expiry(&snapshot, &candidates)
            .is_err()
    );
    assert_eq!(client.inner.journal.identity_snapshot().unwrap(), before);
    assert!(client.inner.journal.snapshot().unwrap().is_none());
}

#[tokio::test]
async fn expiry_second_candidate_conflict_rolls_back_first_candidate_and_checkpoint() {
    let _serial = support::SERIAL.lock().await;
    let harness = support::Harness::new().await;
    let (client, snapshot, entries, candidates) =
        expiry_candidates(&harness, &["EXPIRY-A", "EXPIRY-B"]);
    client
        .inner
        .journal
        .observe_successes(&[entries[1].intent_id])
        .unwrap();
    let before = client.inner.journal.identity_snapshot().unwrap();
    assert!(
        client
            .inner
            .journal
            .checkpoint_with_expiry(&snapshot, &candidates)
            .is_err()
    );
    assert_eq!(client.inner.journal.identity_snapshot().unwrap(), before);
    assert!(client.inner.journal.snapshot().unwrap().is_none());
    assert!(
        client
            .inner
            .journal
            .entries()
            .unwrap()
            .iter()
            .all(|entry| entry.status == OrderStatus::Submitted)
    );
    let observed = client
        .inner
        .journal
        .order_evidence_snapshot(ClientId::from(PAPERTRADE), "EXPIRY-A")
        .unwrap();
    assert_eq!(serde_json::to_value(observed).unwrap()["kind"], "unproven");
}

#[tokio::test]
async fn expiry_proof_cannot_bind_a_different_wallet_snapshot_to_this_journal() {
    let _serial = support::SERIAL.lock().await;
    let harness = support::Harness::new().await;
    let (client, mut snapshot, entries, _) = expiry_candidates(&harness, &["EXPIRY-A"]);
    snapshot.address = alloy_primitives::Address::repeat_byte(9);
    let candidate = crate::journal::ExpiryCandidate::new(
        &entries[0],
        &snapshot,
        snapshot.indexed_through_block,
        snapshot.indexed_through_timestamp,
    )
    .unwrap();
    let before = client.inner.journal.identity_snapshot().unwrap();
    assert!(
        client
            .inner
            .journal
            .checkpoint_with_expiry(&snapshot, &[candidate])
            .is_err()
    );
    assert_eq!(client.inner.journal.identity_snapshot().unwrap(), before);
    assert!(client.inner.journal.snapshot().unwrap().is_none());
}

#[rstest]
#[case("account_revision")]
#[case("account_snapshot_keccak256")]
#[case("missing_witness")]
#[case("changed_witness")]
#[tokio::test]
async fn held_probe_rechecks_the_persisted_expiry_snapshot_binding(#[case] field: &str) {
    use redb::{Database, ReadableTable, TableDefinition};

    let _serial = support::SERIAL.lock().await;
    let harness = support::Harness::new().await;
    let (original_client, snapshot, _, candidates) = expiry_candidates(&harness, &["EXPIRY-A"]);
    original_client
        .inner
        .journal
        .checkpoint_with_expiry(&snapshot, &candidates)
        .unwrap();
    drop(original_client);
    {
        let db = Database::open(&harness.config.journal_path).unwrap();
        let tx = db.begin_write().unwrap();
        {
            let mut proofs = tx
                .open_table(TableDefinition::<&str, &str>::new("order_no_fill_proofs"))
                .unwrap();
            let mut proof: serde_json::Value =
                serde_json::from_str(proofs.get("EXPIRY-A").unwrap().unwrap().value()).unwrap();
            match field {
                "account_revision" => proof["source"]["coverage"][field] = serde_json::json!(0),
                "account_snapshot_keccak256" => {
                    proof["source"]["coverage"][field] =
                        serde_json::json!(alloy_primitives::B256::ZERO);
                }
                "missing_witness" | "changed_witness" => {
                    let mut witnesses = tx
                        .open_table(TableDefinition::<&str, &str>::new(
                            "expiry_account_witnesses",
                        ))
                        .unwrap();
                    if field == "missing_witness" {
                        witnesses.remove("EXPIRY-A").unwrap();
                    } else {
                        let mut changed = snapshot;
                        changed.account_revision += 1;
                        witnesses
                            .insert(
                                "EXPIRY-A",
                                serde_json::to_string(&changed).unwrap().as_str(),
                            )
                            .unwrap();
                    }
                }
                _ => unreachable!(),
            }
            proofs
                .insert("EXPIRY-A", serde_json::to_string(&proof).unwrap().as_str())
                .unwrap();
        }
        tx.commit().unwrap();
    }
    let (client, _, _) = client(&harness);
    assert!(
        client
            .journal_identity_probe()
            .unwrap()
            .order_evidence_json("EXPIRY-A")
            .is_err()
    );
}

#[tokio::test]
async fn expiry_witness_survives_new_checkpoints_and_original_journal_recovery() {
    let _serial = support::SERIAL.lock().await;
    let harness = support::Harness::new().await;
    let (original_client, mut snapshot, _, candidates) = expiry_candidates(&harness, &["EXPIRY-A"]);
    original_client
        .inner
        .journal
        .checkpoint_with_expiry(&snapshot, &candidates)
        .unwrap();
    let probe = original_client.journal_identity_probe().unwrap();
    let before: serde_json::Value =
        serde_json::from_str(&probe.order_evidence_json("EXPIRY-A").unwrap()).unwrap();
    assert_eq!(before["kind"], "proven_no_fill");
    snapshot.account_revision += 1;
    snapshot.indexed_through_block += 1;
    snapshot.indexed_through_timestamp += 1;
    original_client.inner.journal.checkpoint(&snapshot).unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&probe.order_evidence_json("EXPIRY-A").unwrap())
            .unwrap()["proof"],
        before["proof"]
    );
    drop(original_client);
    assert!(probe.order_evidence_json("EXPIRY-A").is_err());
    let (recovered, _, _) = client(&harness);
    let observed: serde_json::Value = serde_json::from_str(
        &recovered
            .journal_identity_probe()
            .unwrap()
            .order_evidence_json("EXPIRY-A")
            .unwrap(),
    )
    .unwrap();
    assert_eq!(observed["kind"], "proven_no_fill");
    assert_eq!(observed["proof"], before["proof"]);
    assert!(observed.get("snapshot").is_none());
    assert!(observed["proof"].get("snapshot").is_none());
}

#[tokio::test]
async fn expiry_checkpoint_keeps_current_transport_marker_and_scan_watermark() {
    let _serial = support::SERIAL.lock().await;
    let harness = support::Harness::new().await;
    let (client, snapshot, mut entries, candidates) = expiry_candidates(&harness, &["EXPIRY-A"]);
    entries[0].scanned_through += 1;
    client.inner.journal.update(&entries[0]).unwrap();
    client
        .inner
        .journal
        .checkpoint_with_expiry(&snapshot, &candidates)
        .unwrap();
    let current = client.inner.journal.entries().unwrap().remove(0);
    assert!(current.send_attempted);
    assert_eq!(current.scanned_through, entries[0].scanned_through);
    assert_eq!(current.status, OrderStatus::Expired);
}

#[tokio::test]
async fn later_balance_report_failure_cannot_seal_an_expiry_candidate() {
    let _serial = support::SERIAL.lock().await;
    let harness = support::Harness::new().await;
    let (client, mut snapshot, entries, _) = expiry_candidates(&harness, &["EXPIRY-A"]);
    harness.source.trading.lock()["indexedThroughBlock"] =
        serde_json::json!(snapshot.indexed_through_block);
    *harness.source.block_timestamp.lock() = Some(snapshot.indexed_through_timestamp);
    snapshot.balance = crate::wire::Raw(alloy_primitives::U256::MAX);
    snapshot.funds.available_balance = snapshot.balance;
    snapshot.funds.reserved_balance = crate::wire::Raw::default();
    snapshot.validate(snapshot.address).unwrap();
    let error = crate::reconciliation::synchronize(&client.inner, snapshot)
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("Invalid decimal: overflow"),
        "{error:#}"
    );
    assert!(harness.source.block_calls.load(Ordering::SeqCst) > 0);
    assert!(client.inner.journal.snapshot().unwrap().is_none());
    let current = client
        .inner
        .journal
        .entries()
        .unwrap()
        .into_iter()
        .find(|entry| entry.command.client_order_id == entries[0].command.client_order_id)
        .unwrap();
    assert_eq!(current.status, OrderStatus::Submitted);
    let observed: serde_json::Value = serde_json::from_str(
        &client
            .journal_identity_probe()
            .unwrap()
            .order_evidence_json("EXPIRY-A")
            .unwrap(),
    )
    .unwrap();
    assert_eq!(observed["kind"], "unproven");
}

#[rstest]
#[case(OrderStatus::Rejected)]
#[case(OrderStatus::Canceled)]
#[case(OrderStatus::Expired)]
#[tokio::test]
async fn old_service_terminal_remains_unproven_after_successful_past_deadline_sync(
    #[case] status: OrderStatus,
) {
    let _serial = support::SERIAL.lock().await;
    let harness = support::Harness::new().await;
    let (mut client, snapshot, mut entries, _) = expiry_candidates(&harness, &["EXPIRY-A"]);
    client.start().unwrap();
    entries[0].status = status;
    client.inner.journal.update(&entries[0]).unwrap();
    *harness.source.block_timestamp.lock() = Some(snapshot.indexed_through_timestamp);
    crate::reconciliation::synchronize(&client.inner, snapshot)
        .await
        .unwrap();
    let observed: serde_json::Value = serde_json::from_str(
        &client
            .journal_identity_probe()
            .unwrap()
            .order_evidence_json("EXPIRY-A")
            .unwrap(),
    )
    .unwrap();
    assert_eq!(observed["kind"], "unproven");
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

pub(crate) fn cancellation(
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
                    .submission_time(&signed, false)
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
