// Copyright (C) 2015-2026 Nautech Systems Pty Ltd. All rights reserved.
// Licensed under the GNU Lesser General Public License Version 3.0.
// https://www.gnu.org/licenses/lgpl-3.0.en.html

use std::{sync::atomic::Ordering, time::Duration};

use nautilus_common::messages::execution::QueryAccount;
use nautilus_core::{UUID4, UnixNanos};
use nautilus_model::identifiers::{AccountId, ClientId, TraderId};
use nautilus_papertrade::{journal::Journal, wire::VenuePosition};
use rstest::rstest;
use serde_json::{Value, json};

#[allow(
    dead_code,
    reason = "Shared harness also serves unrelated recovery tests"
)]
mod support;
use support::{Harness, SERIAL, mark_attempted, prepared_open};

fn query() -> QueryAccount {
    QueryAccount::new(
        TraderId::from("TESTER-001"),
        Some(ClientId::from("PAPERTRADE")),
        AccountId::from("PAPERTRADE-001"),
        UUID4::new(),
        UnixNanos::default(),
        None,
        None,
    )
}

#[tokio::test]
async fn silent_account_refresh_uses_a_new_snapshot_without_rewriting_source_time() {
    let _serial = SERIAL.lock().await;
    let harness = Harness::new().await;
    let mut client = harness.client();
    let probe = client.journal_identity_probe().unwrap();
    client.start().unwrap();
    client.connect().await.unwrap();
    let first: Value = serde_json::from_str(&probe.account_observation_json().unwrap()).unwrap();
    let mut refreshed = harness.source.wallet.borrow().clone();
    refreshed["indexedThroughBlock"] =
        json!(refreshed["indexedThroughBlock"].as_u64().unwrap() + 1);
    refreshed["indexedThroughTimestamp"] = json!(first["source_timestamp"].as_u64().unwrap() + 1);
    *harness.source.wallet_initial_override.lock() = Some(refreshed);
    tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            let later: Value =
                serde_json::from_str(&probe.account_observation_json().unwrap()).unwrap();
            if later["source_timestamp"] != first["source_timestamp"] {
                assert_eq!(
                    later["source_timestamp"].as_u64().unwrap(),
                    first["source_timestamp"].as_u64().unwrap() + 1
                );
                assert_eq!(later["account_revision"], first["account_revision"]);
                assert_ne!(later["source_epoch"], first["source_epoch"]);
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .unwrap();
    client.disconnect().await.unwrap();
}

#[tokio::test]
async fn explicit_queries_are_coalesced_before_network_work() {
    let _serial = SERIAL.lock().await;
    let harness = Harness::new().await;
    let mut client = harness.client();
    client.start().unwrap();
    client.connect().await.unwrap();
    harness.source.wallet_delay_ms.store(250, Ordering::SeqCst);
    for _ in 0..40 {
        client.query_account(query()).unwrap();
    }
    tokio::time::sleep(Duration::from_secs(2)).await;
    let requests = harness.source.wallet_requests.load(Ordering::SeqCst);
    client.disconnect().await.unwrap();
    assert!(
        (2..=3).contains(&requests),
        "Unbounded query requests: {requests}"
    );
}

#[tokio::test]
async fn disconnected_queries_are_explicitly_refused() {
    let _serial = SERIAL.lock().await;
    let harness = Harness::new().await;
    let client = harness.client();
    assert!(client.query_account(query()).is_err());
    assert_eq!(harness.source.wallet_requests.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_query_arriving_during_refresh_is_not_cleared_on_completion() {
    let _serial = SERIAL.lock().await;
    let harness = Harness::new().await;
    let mut client = harness.client();
    client.start().unwrap();
    client.connect().await.unwrap();
    harness.source.wallet_delay_ms.store(500, Ordering::SeqCst);
    client.query_account(query()).unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while harness.source.wallet_requests.load(Ordering::SeqCst) < 2 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    client.query_account(query()).unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        while harness.source.wallet_requests.load(Ordering::SeqCst) < 3 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    client.disconnect().await.unwrap();
    assert_eq!(harness.source.wallet_requests.load(Ordering::SeqCst), 3);
}

#[rstest]
#[case("wallet")]
#[case("history")]
#[tokio::test]
async fn stop_cancels_a_refresh_waiting_for_its_source(#[case] waiting: &str) {
    let _serial = SERIAL.lock().await;
    let harness = Harness::new().await;
    let mut client = harness.client();
    let probe = client.journal_identity_probe().unwrap();
    client.start().unwrap();
    client.connect().await.unwrap();
    let first: Value = serde_json::from_str(&probe.account_observation_json().unwrap()).unwrap();
    let initial_history = harness.source.history_calls.load(Ordering::SeqCst);
    if waiting == "wallet" {
        harness
            .source
            .wallet_delay_ms
            .store(30000, Ordering::SeqCst);
    } else {
        let mut fresh = harness.source.wallet.borrow().clone();
        fresh["indexedThroughBlock"] = json!(fresh["indexedThroughBlock"].as_u64().unwrap() + 1);
        fresh["walletRevision"] = fresh["indexedThroughBlock"].clone();
        *harness.source.wallet_initial_override.lock() = Some(fresh);
        harness
            .source
            .history_delay_ms
            .store(30000, Ordering::SeqCst);
    }
    client.query_account(query()).unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while if waiting == "wallet" {
            harness.source.wallet_requests.load(Ordering::SeqCst) < 2
        } else {
            harness.source.history_calls.load(Ordering::SeqCst) <= initial_history
        } {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    tokio::time::timeout(Duration::from_millis(500), client.disconnect())
        .await
        .unwrap()
        .unwrap();
    let later: Value = serde_json::from_str(&probe.account_observation_json().unwrap()).unwrap();
    assert_eq!(later["source_timestamp"], first["source_timestamp"]);
    assert_eq!(
        later["account_snapshot_keccak256"],
        first["account_snapshot_keccak256"]
    );
}

#[tokio::test]
async fn stale_stream_success_is_retained_without_publishing_the_old_account() {
    let _serial = SERIAL.lock().await;
    let harness = Harness::new().await;
    let original = harness.source.wallet.borrow().clone();
    let position: VenuePosition =
        serde_json::from_value(original["openPositions"][0].clone()).unwrap();
    let now = original["indexedThroughTimestamp"].as_u64().unwrap();
    let entry = prepared_open(&harness, &position, now, now + 3600);
    mark_attempted(&harness, &entry);
    let mut client = harness.client();
    let probe = client.journal_identity_probe().unwrap();
    client.start().unwrap();
    client.connect().await.unwrap();
    let mut fresh = original.clone();
    fresh["indexedThroughBlock"] = json!(fresh["indexedThroughBlock"].as_u64().unwrap() + 1);
    fresh["indexedThroughTimestamp"] = json!(now + 1);
    *harness.source.wallet_initial_override.lock() = Some(fresh.clone());
    client.query_account(query()).unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let observed: Value =
                serde_json::from_str(&probe.account_observation_json().unwrap()).unwrap();
            if observed["source_timestamp"] == fresh["indexedThroughTimestamp"] {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let mut stale = original;
    stale["accountRevision"] = json!(2);
    stale["intentOutcomes"] = json!([[entry.intent_id, "succeeded"]]);
    harness.source.wallet.send_replace(stale);
    tokio::time::sleep(Duration::from_millis(750)).await;
    let observed: Value = serde_json::from_str(&probe.account_observation_json().unwrap()).unwrap();
    assert_eq!(
        observed["source_timestamp"],
        fresh["indexedThroughTimestamp"]
    );
    client.disconnect().await.unwrap();
    drop(client);
    let journal = Journal::open(
        &harness.config.journal_path,
        fresh["address"].as_str().unwrap().parse().unwrap(),
        "PAPERTRADE-001",
    )
    .unwrap();
    assert!(
        journal
            .entries()
            .unwrap()
            .iter()
            .any(|row| row.intent_id == entry.intent_id && row.execution_succeeded)
    );
}

#[tokio::test]
async fn cached_polling_cannot_clear_a_fresh_source_conflict() {
    let _serial = SERIAL.lock().await;
    let harness = Harness::new().await;
    let mut client = harness.client();
    client.start().unwrap();
    client.connect().await.unwrap();
    let mut conflicting = harness.source.wallet.borrow().clone();
    conflicting["lifetime"]["depositedRaw"] = json!("1");
    *harness.source.wallet_initial_override.lock() = Some(conflicting);
    client.query_account(query()).unwrap();
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(
        !client.is_connected(),
        "Cached polling erased a known source conflict"
    );
    *harness.source.wallet_initial_override.lock() = None;
    client.query_account(query()).unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while !client.is_connected() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    client.disconnect().await.unwrap();
}

#[tokio::test]
async fn refresh_deadline_includes_history_and_releases_the_serial_writer() {
    let _serial = SERIAL.lock().await;
    let harness = Harness::new().await;
    let mut client = harness.client();
    let probe = client.journal_identity_probe().unwrap();
    client.start().unwrap();
    client.connect().await.unwrap();
    let before: Value = serde_json::from_str(&probe.account_observation_json().unwrap()).unwrap();
    let mut fresh = harness.source.wallet.borrow().clone();
    fresh["indexedThroughBlock"] = json!(fresh["indexedThroughBlock"].as_u64().unwrap() + 1);
    fresh["walletRevision"] = fresh["indexedThroughBlock"].clone();
    *harness.source.wallet_initial_override.lock() = Some(fresh.clone());
    harness
        .source
        .history_delay_ms
        .store(30000, Ordering::SeqCst);
    client.query_account(query()).unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while harness.source.wallet_requests.load(Ordering::SeqCst) < 2 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    tokio::time::timeout(Duration::from_secs(22), async {
        while client.is_connected() {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .unwrap();
    let failed: Value = serde_json::from_str(&probe.account_observation_json().unwrap()).unwrap();
    assert_eq!(
        failed["account_snapshot_keccak256"],
        before["account_snapshot_keccak256"]
    );
    assert_eq!(failed["source_timestamp"], before["source_timestamp"]);
    harness.source.history_delay_ms.store(0, Ordering::SeqCst);
    client.query_account(query()).unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        while !client.is_connected() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let recovered: Value =
        serde_json::from_str(&probe.account_observation_json().unwrap()).unwrap();
    assert_eq!(recovered["source_block"], fresh["indexedThroughBlock"]);
    client.disconnect().await.unwrap();
}
