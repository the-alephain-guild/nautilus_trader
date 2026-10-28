// Copyright (C) 2015-2026 Nautech Systems Pty Ltd. All rights reserved.
// Licensed under the GNU Lesser General Public License Version 3.0.
// https://www.gnu.org/licenses/lgpl-3.0.en.html

use nautilus_model::enums::OrderStatus;
use nautilus_papertrade::{
    config::PapertradeJournalOpenMode, journal::Journal, wire::VenuePosition,
};
use serde_json::{Value, json};

#[allow(
    dead_code,
    reason = "Shared harness also contains other recovery fixtures"
)]
mod support;
use support::{Harness, SERIAL, mark_attempted, prepared_open};

#[tokio::test]
async fn held_account_observation_requires_a_committed_source_context() {
    let _serial = SERIAL.lock().await;
    let harness = Harness::new().await;
    let client = harness.client();
    let probe = client.journal_identity_probe().unwrap();
    assert!(probe.account_observation_json().is_err());
}

#[tokio::test]
async fn held_account_observation_keeps_original_raw_values_and_held_owner() {
    let _serial = SERIAL.lock().await;
    let mut harness = Harness::new().await;
    let source = harness.source.wallet.borrow().clone();
    let mut client = harness.client();
    let probe = client.journal_identity_probe().unwrap();
    client.start().unwrap();
    client.connect().await.unwrap();
    let raw = probe.account_observation_json().unwrap();
    let observed: Value = serde_json::from_str(&raw).unwrap();
    let published = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            if let Some(nautilus_common::messages::ExecutionEvent::Account(state)) =
                harness._events.recv().await
                && state
                    .info
                    .as_ref()
                    .and_then(|info| info.get("papertrade_checkpoint_revision"))
                    == Some(&observed["checkpoint_revision"])
            {
                break state;
            }
        }
    })
    .await
    .unwrap();
    let info = published.info.unwrap();
    assert_eq!(
        info.get("papertrade_journal_id"),
        Some(&observed["journal"]["journal_id"])
    );
    assert_eq!(
        info.get("papertrade_source_epoch"),
        Some(&observed["source_epoch"])
    );
    assert_eq!(
        info.get("papertrade_account_snapshot_keccak256"),
        Some(&observed["account_snapshot_keccak256"])
    );
    assert_eq!(
        info.get("papertrade_account_material_keccak256"),
        Some(&observed["account_material_keccak256"])
    );
    assert_eq!(observed["schema_version"], 1);
    assert_eq!(observed["client_id"], "PAPERTRADE");
    assert_eq!(observed["account_id"], "PAPERTRADE-001");
    assert_eq!(observed["account_revision"], source["accountRevision"]);
    assert_eq!(observed["balance_raw"], source["balance"]);
    assert_eq!(
        observed["source_timestamp"],
        source["indexedThroughTimestamp"]
    );
    assert_eq!(observed["queued_raw"], source["queued"]);
    assert_eq!(observed["consumable_debt_raw"], source["consumableDebt"]);
    assert_eq!(observed["positions"].as_array().unwrap().len(), 2);
    let position = &source["openPositions"][0];
    let position_id = format!("PT-{}", position[0].as_str().unwrap());
    let first = observed["positions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["position_id"] == position_id)
        .unwrap();
    assert_eq!(first["position_id"], position_id);
    assert_eq!(first["margin_raw"], position[5]);
    assert_eq!(first["entry_price_raw"], position[4]);
    assert_eq!(first["leverage"], position[6]);
    assert_eq!(first["price_scale_raw"], "10");
    assert_eq!(
        observed["cashflow_counters"]["deposited_raw"],
        source["lifetime"]["depositedRaw"]
    );
    assert_eq!(
        observed["cashflow_counters"]["withdrawn_raw"],
        source["lifetime"]["withdrawnRaw"]
    );
    assert_eq!(
        observed["cashflow_counters"]["rewards_claimed_raw"],
        source["lifetime"]["paperRewardsClaimedRaw"]
    );
    for forbidden in [
        "\"session_keys\"",
        "\"sessionKeys\"",
        "\"body\"",
        "\"signed\"",
        "\"signature\"",
    ] {
        assert!(
            !raw.contains(forbidden),
            "Unexpected private field {forbidden}"
        );
    }
    client.disconnect().await.unwrap();
    drop(client);
    assert!(probe.account_observation_json().is_err());
}

#[tokio::test]
async fn held_account_observation_does_not_refresh_source_time_by_reading() {
    let _serial = SERIAL.lock().await;
    let harness = Harness::new().await;
    let mut client = harness.client();
    let probe = client.journal_identity_probe().unwrap();
    client.start().unwrap();
    client.connect().await.unwrap();
    let first: Value = serde_json::from_str(&probe.account_observation_json().unwrap()).unwrap();
    let second: Value = serde_json::from_str(&probe.account_observation_json().unwrap()).unwrap();
    assert_eq!(first["source_timestamp"], second["source_timestamp"]);
    assert_eq!(first["account_revision"], second["account_revision"]);
    assert_eq!(first["cashflow_counters"], second["cashflow_counters"]);
    assert_eq!(first["pending_client_order_ids"], json!([]));
    client.disconnect().await.unwrap();
}

#[tokio::test]
async fn held_account_observation_keeps_attempted_and_legacy_terminal_intents_pending() {
    let _serial = SERIAL.lock().await;
    for legacy_terminal in [false, true] {
        let harness = Harness::new().await;
        let snapshot = harness.source.wallet.borrow().clone();
        let position: VenuePosition =
            serde_json::from_value(snapshot["openPositions"][0].clone()).unwrap();
        let now = snapshot["indexedThroughTimestamp"].as_u64().unwrap();
        let mut entry = prepared_open(&harness, &position, now, now + 3600);
        if legacy_terminal {
            entry.status = OrderStatus::Rejected;
            let journal = Journal::open(
                &harness.config.journal_path,
                snapshot["address"].as_str().unwrap().parse().unwrap(),
                harness.config.account_id.as_str(),
            )
            .unwrap();
            journal.update(&entry).unwrap();
        } else {
            mark_attempted(&harness, &entry);
        }
        let mut client = harness.client();
        let probe = client.journal_identity_probe().unwrap();
        client.start().unwrap();
        client.connect().await.unwrap();
        let observed: Value =
            serde_json::from_str(&probe.account_observation_json().unwrap()).unwrap();
        assert_eq!(
            observed["pending_client_order_ids"],
            json!([entry.command.client_order_id.as_str()])
        );
        client.disconnect().await.unwrap();
    }
}

#[tokio::test]
async fn held_account_observation_excludes_only_proved_no_fill_from_pending() {
    let _serial = SERIAL.lock().await;
    let mut harness = Harness::new().await;
    let snapshot = harness.source.wallet.borrow().clone();
    let position: VenuePosition =
        serde_json::from_value(snapshot["openPositions"][0].clone()).unwrap();
    let now = snapshot["indexedThroughTimestamp"].as_u64().unwrap();
    let entry = prepared_open(&harness, &position, now, now + 3600);
    let journal = Journal::open(
        &harness.config.journal_path,
        snapshot["address"].as_str().unwrap().parse().unwrap(),
        harness.config.account_id.as_str(),
    )
    .unwrap();
    let identity = journal.identity_snapshot().unwrap();
    drop(journal);
    harness.config.journal_open = PapertradeJournalOpenMode::RecoverExisting {
        journal_id: identity.journal_id,
        minimum_revision: identity.revision,
    };
    let mut client = harness.client();
    let probe = client.journal_identity_probe().unwrap();
    client.start().unwrap();
    client.connect().await.unwrap();
    let observed: Value = serde_json::from_str(&probe.account_observation_json().unwrap()).unwrap();
    assert_eq!(observed["pending_client_order_ids"], json!([]));
    let order: Value = serde_json::from_str(
        &probe
            .order_evidence_json(entry.command.client_order_id.as_str())
            .unwrap(),
    )
    .unwrap();
    assert_eq!(order["kind"], "proven_no_fill");
    client.disconnect().await.unwrap();
}
