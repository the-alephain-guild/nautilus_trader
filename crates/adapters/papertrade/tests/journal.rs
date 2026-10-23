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

use alloy::signers::local::PrivateKeySigner;
use alloy_primitives::{Address, B256, U256, address};
use nautilus_common::messages::execution::SubmitOrder;
use nautilus_core::{UUID4, UnixNanos};
use nautilus_model::{
    enums::{OrderStatus, OrderType},
    identifiers::{ClientOrderId, InstrumentId, TraderId},
    orders::OrderTestBuilder,
    types::Quantity,
};
use nautilus_papertrade::{OpenPosition, PapertradeSigner, journal::Journal};
use rstest::rstest;

const WALLET: Address = address!("2222222222222222222222222222222222222222");
fn command(id: &str) -> SubmitOrder {
    let order = OrderTestBuilder::new(OrderType::Market)
        .instrument_id(InstrumentId::from("BTC-USD-PERP.PAPERTRADE"))
        .client_order_id(ClientOrderId::from(id))
        .quantity(Quantity::from(1000))
        .build();
    SubmitOrder::from_order(
        &order,
        TraderId::from("TESTER-001"),
        None,
        None,
        UUID4::new(),
        UnixNanos::from(1),
    )
}
fn sign(nonce: U256) -> anyhow::Result<nautilus_papertrade::SignedIntent> {
    PapertradeSigner::new(PrivateKeySigner::from_bytes(&B256::repeat_byte(17)).unwrap()).sign_open(
        &OpenPosition {
            user: WALLET,
            instrumentId: 0,
            isLong: true,
            size: 10_000_000_000_000_000_000,
            leverage: 100,
            useDebt: false,
            nonce,
            deadline: 1_700_003_600,
        },
        1_700_000_000,
    )
}
#[rstest]
fn restart_retains_request_and_duplicate_command_never_allocates_a_new_intent() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("journal.redb");
    let cmd = command("PT-1");
    let first = {
        let journal = Journal::open(&path, WALLET, "PAPERTRADE-001").unwrap();
        let (first, new) = journal.prepare(&cmd, None, 100, sign).unwrap();
        assert!(new);
        let (same, new) = journal
            .prepare(&cmd, None, 100, |_| panic!("duplicate was re-signed"))
            .unwrap();
        assert!(!new);
        assert_eq!(same.intent_id, first.intent_id);
        first
    };
    let journal = Journal::open(&path, WALLET, "PAPERTRADE-001").unwrap();
    let (same, new) = journal
        .prepare(&cmd, None, 200, |_| panic!("restart was re-signed"))
        .unwrap();
    assert!(!new);
    assert_eq!(same.body, first.body);
    let (second, new) = journal.prepare(&command("PT-2"), None, 201, sign).unwrap();
    assert!(new);
    assert_eq!(second.nonce.0, first.nonce.0 + U256::from(1));
}
#[rstest]
fn journal_rejects_another_account_and_concurrent_process_owner() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("journal.redb");
    let journal = Journal::open(&path, WALLET, "PAPERTRADE-001").unwrap();
    assert!(Journal::open(&path, WALLET, "PAPERTRADE-001").is_err());
    drop(journal);
    assert!(Journal::open(&path, Address::ZERO, "PAPERTRADE-001").is_err());
}
#[rstest]
fn late_acknowledgement_does_not_erase_a_terminal_state() {
    let dir = tempfile::tempdir().unwrap();
    let journal =
        Journal::open(&dir.path().join("journal.redb"), WALLET, "PAPERTRADE-001").unwrap();
    let (entry, _) = journal.prepare(&command("PT-1"), None, 100, sign).unwrap();
    let mut terminal = entry.clone();
    terminal.status = OrderStatus::Rejected;
    journal.update(&terminal).unwrap();
    let mut late = entry;
    late.status = OrderStatus::Accepted;
    journal.update(&late).unwrap();
    assert_eq!(journal.entries().unwrap()[0].status, OrderStatus::Rejected);
}

#[rstest]
#[case("instrumentId")]
#[case("isLong")]
#[case("entryPriceRaw")]
#[case("marginRaw")]
#[case("leverage")]
#[case("openedAtMs")]
fn conflicting_close_identity_is_rejected_without_poisoning_the_journal(
    #[case] field: &str,
    #[values(false, true)] history_first: bool,
) {
    use nautilus_papertrade::wire::{CloseRecord, VenuePosition};
    use serde_json::{Value, json};
    let source: Value =
        serde_json::from_str(include_str!("../test_data/live_wallet.json")).unwrap();
    let position: VenuePosition =
        serde_json::from_value(source["openPositions"][0].clone()).unwrap();
    let value = json!({"positionId":position.position_id,"instrumentId":position.instrument_id,
        "symbol":"BTC","priceScaleRaw":"10","isLong":position.is_long,
        "entryPriceRaw":position.entry_price,"marginRaw":position.margin,"leverage":position.leverage,
        "openedAtMs":position.opened_at_ms,"timestampMs":position.opened_at_ms+1000,
        "txHash":B256::repeat_byte(5),"kind":"PositionClosed","exitPriceRaw":position.entry_price,
        "adjustedPnlRaw":"0","userPaidFeeRaw":"0"});
    let correct: CloseRecord = serde_json::from_value(value.clone()).unwrap();
    let mut changed = value;
    changed[field] = match field {
        "isLong" => json!(!position.is_long),
        "instrumentId" => json!(position.instrument_id + 1),
        "entryPriceRaw" => json!((position.entry_price.0 + U256::from(1)).to_string()),
        "marginRaw" => json!((position.margin.0 + U256::from(1)).to_string()),
        "leverage" => json!(position.leverage + 1),
        "openedAtMs" => json!(position.opened_at_ms + 1),
        _ => unreachable!(),
    };
    let conflict: CloseRecord = serde_json::from_value(changed).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let journal =
        Journal::open(&dir.path().join("journal.redb"), WALLET, "PAPERTRADE-001").unwrap();
    if history_first {
        journal
            .remember(&[], std::slice::from_ref(&correct))
            .unwrap();
        let contradictory_open = nautilus_papertrade::reports::historical_position(&conflict);
        assert!(
            journal.remember(&[contradictory_open], &[]).is_err(),
            "Later open proof contradicted retained close economics"
        );
        assert!(journal.observed().unwrap().is_empty());
        journal.remember(&[position], &[]).unwrap();
        assert_eq!(journal.observed().unwrap().len(), 1);
        return;
    }
    journal
        .remember(std::slice::from_ref(&position), &[])
        .unwrap();
    assert!(
        journal.remember(&[], &[conflict]).is_err(),
        "Changed position economics were accepted as a close"
    );
    assert!(journal.closes().unwrap().is_empty());
    journal
        .remember(&[], std::slice::from_ref(&correct))
        .unwrap();
    assert_eq!(journal.closes().unwrap(), vec![correct]);
}

#[rstest]
fn send_attempt_and_scan_progress_survive_late_acknowledgements() {
    let dir = tempfile::tempdir().unwrap();
    let journal =
        Journal::open(&dir.path().join("journal.redb"), WALLET, "PAPERTRADE-001").unwrap();
    let (entry, _) = journal
        .prepare(&command("PT-ATTEMPT"), None, 100, sign)
        .unwrap();
    assert!(!entry.send_attempted);
    let mut attempted = journal
        .begin_submission(entry.command.client_order_id)
        .unwrap()
        .unwrap();
    assert!(
        journal
            .begin_submission(entry.command.client_order_id)
            .unwrap()
            .is_none()
    );
    attempted.scanned_through = 200;
    journal.update(&attempted).unwrap();
    let mut late = entry;
    late.status = OrderStatus::Accepted;
    journal.update(&late).unwrap();
    let saved = journal.entries().unwrap().pop().unwrap();
    assert!(saved.send_attempted);
    assert_eq!(saved.scanned_through, 200);
    let mut legacy = serde_json::to_value(saved).unwrap();
    legacy.as_object_mut().unwrap().remove("send_attempted");
    let legacy: nautilus_papertrade::journal::JournalEntry =
        serde_json::from_value(legacy).unwrap();
    assert!(
        legacy.send_attempted,
        "Old journals cannot prove that submission was never attempted"
    );
}

#[rstest]
fn execution_success_survives_late_ack_and_rejects_terminal_failure() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("journal.redb");
    let journal = Journal::open(&path, WALLET, "PAPERTRADE-001").unwrap();
    let (entry, _) = journal
        .prepare(&command("PT-SUCCESS"), None, 100, sign)
        .unwrap();
    let mut success = journal
        .begin_submission(entry.command.client_order_id)
        .unwrap()
        .unwrap();
    success.execution_succeeded = true;
    success.scanned_through = 200;
    journal.update(&success).unwrap();
    let mut late = entry;
    late.status = OrderStatus::Accepted;
    journal.update(&late).unwrap();
    for status in [
        OrderStatus::Expired,
        OrderStatus::Rejected,
        OrderStatus::Canceled,
    ] {
        late.status = status;
        assert!(journal.update(&late).is_err());
    }
    drop(journal);
    let reopened = Journal::open(&path, WALLET, "PAPERTRADE-001").unwrap();
    let saved = reopened.entries().unwrap().pop().unwrap();
    assert!(saved.execution_succeeded);
    assert_eq!(saved.status, OrderStatus::Accepted);
    assert_eq!(saved.scanned_through, 200);
}

#[rstest]
fn legacy_unresolved_journal_rescans_results_without_changing_request_identity() {
    use redb::{Database, TableDefinition};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("journal.redb");
    let journal = Journal::open(&path, WALLET, "PAPERTRADE-001").unwrap();
    let (entry, _) = journal
        .prepare(&command("PT-LEGACY"), None, 100, sign)
        .unwrap();
    let mut attempted = journal
        .begin_submission(entry.command.client_order_id)
        .unwrap()
        .unwrap();
    attempted.status = OrderStatus::Accepted;
    attempted.scanned_through = 200;
    journal.update(&attempted).unwrap();
    drop(journal);
    // This is the on-disk format produced before execution-success evidence was recorded.
    let mut legacy = serde_json::to_value(&attempted).unwrap();
    legacy
        .as_object_mut()
        .unwrap()
        .remove("execution_succeeded");
    {
        let db = Database::open(&path).unwrap();
        let tx = db.begin_write().unwrap();
        {
            let mut orders = tx
                .open_table(TableDefinition::<&str, &str>::new("orders"))
                .unwrap();
            orders
                .insert(
                    entry.command.client_order_id.as_str(),
                    serde_json::to_string(&legacy).unwrap().as_str(),
                )
                .unwrap();
        }
        tx.commit().unwrap();
    }
    let reopened = Journal::open(&path, WALLET, "PAPERTRADE-001").unwrap();
    let restored = reopened.entries().unwrap().pop().unwrap();
    assert_eq!(
        restored.scanned_through, 99,
        "Legacy cursor skipped success evidence that was never retained"
    );
    assert_eq!(restored.intent_id, entry.intent_id);
    assert_eq!(restored.body, entry.body);
    assert_eq!(restored.nonce, entry.nonce);
    assert!(restored.send_attempted);
    assert!(
        reopened
            .begin_submission(entry.command.client_order_id)
            .unwrap()
            .is_none()
    );
}

#[rstest]
#[case("before_migration")]
#[case("after_migration")]
#[case("before_source")]
#[case("after_source")]
#[case("after_success")]
fn journal_commit_boundaries_survive_forced_process_termination(#[case] boundary: &str) {
    use std::{
        io::{BufRead, BufReader, Write},
        process::{Child, Command, Stdio},
        time::Duration,
    };

    use redb::{Database, ReadableDatabase, TableDefinition};
    const CHILD_PATH: &str = "PAPERTRADE_TEST_CRASH_JOURNAL";
    const READY: &str = "JOURNAL_BOUNDARY_READY";
    struct KillOnDrop(Child);
    impl Drop for KillOnDrop {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    if let Some(path) = std::env::var_os(CHILD_PATH) {
        let journal = (boundary != "before_migration")
            .then(|| Journal::open(std::path::Path::new(&path), WALLET, "PAPERTRADE-001").unwrap());
        if matches!(boundary, "after_source" | "after_success") {
            let journal = journal.as_ref().unwrap();
            let mut entry = journal.entries().unwrap().pop().unwrap();
            let wallet: serde_json::Value =
                serde_json::from_str(include_str!("../test_data/live_wallet.json")).unwrap();
            let mut position: nautilus_papertrade::wire::VenuePosition =
                serde_json::from_value(wallet["openPositions"][0].clone()).unwrap();
            position.intent_id = Some(entry.intent_id);
            journal.remember(&[position], &[]).unwrap();
            if boundary == "after_success" {
                entry.execution_succeeded = true;
                entry.scanned_through = 200;
                journal.update(&entry).unwrap();
            }
        }
        println!("{READY}");
        std::io::stdout().flush().unwrap();
        let mut line = String::new();
        std::io::stdin().read_line(&mut line).unwrap();
        panic!("Child must be killed while the journal remains open");
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("journal.redb");
    let original = {
        let journal = Journal::open(&path, WALLET, "PAPERTRADE-001").unwrap();
        let (entry, _) = journal
            .prepare(&command("PT-CRASH"), None, 100, sign)
            .unwrap();
        journal
            .begin_submission(entry.command.client_order_id)
            .unwrap()
            .unwrap()
    };
    if boundary.ends_with("migration") {
        let mut legacy = serde_json::to_value(&original).unwrap();
        legacy
            .as_object_mut()
            .unwrap()
            .remove("execution_succeeded");
        legacy["status"] = serde_json::to_value(OrderStatus::Expired).unwrap();
        legacy["scanned_through"] = serde_json::json!(200);
        let db = Database::open(&path).unwrap();
        let tx = db.begin_write().unwrap();
        {
            let mut table = tx
                .open_table(TableDefinition::<&str, &str>::new("orders"))
                .unwrap();
            table
                .insert(
                    original.command.client_order_id.as_str(),
                    legacy.to_string().as_str(),
                )
                .unwrap();
        }
        tx.commit().unwrap();
    }
    let mut child = KillOnDrop(
        Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                std::thread::current().name().unwrap(),
                "--nocapture",
            ])
            .env(CHILD_PATH, &path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap(),
    );
    let stdout = child.0.stdout.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            if line.unwrap() == READY {
                tx.send(()).unwrap();
                return;
            }
        }
    });
    let ready = rx.recv_timeout(Duration::from_secs(10));
    child.0.kill().unwrap();
    let status = child.0.wait().unwrap();
    reader.join().unwrap();
    ready.expect("Child did not reach the requested commit boundary");
    assert!(!status.success());
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(status.signal(), Some(9));
    }
    let raw: serde_json::Value = {
        let db = Database::open(&path).unwrap();
        let tx = db.begin_read().unwrap();
        let table = tx
            .open_table(TableDefinition::<&str, &str>::new("orders"))
            .unwrap();
        serde_json::from_str(
            table
                .get(original.command.client_order_id.as_str())
                .unwrap()
                .unwrap()
                .value(),
        )
        .unwrap()
    };
    if boundary == "before_migration" {
        assert!(raw.get("execution_succeeded").is_none());
        assert_eq!(
            raw["status"],
            serde_json::to_value(OrderStatus::Expired).unwrap()
        );
        assert_eq!(raw["scanned_through"], 200);
    } else {
        assert_eq!(raw["execution_succeeded"], boundary == "after_success");
        assert_eq!(
            raw["scanned_through"],
            if boundary == "after_success" { 200 } else { 99 }
        );
        assert_eq!(
            raw["status"],
            serde_json::to_value(OrderStatus::Submitted).unwrap()
        );
    }
    let journal = Journal::open(&path, WALLET, "PAPERTRADE-001").unwrap();
    let saved = journal.entries().unwrap().pop().unwrap();
    assert_eq!(saved.intent_id, original.intent_id);
    assert_eq!(saved.body, original.body);
    assert_eq!(saved.nonce, original.nonce);
    assert_eq!(
        journal.observed().unwrap().len(),
        usize::from(matches!(boundary, "after_source" | "after_success"))
    );
    assert!(
        journal
            .begin_submission(saved.command.client_order_id)
            .unwrap()
            .is_none()
    );
}
