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

use alloy_primitives::Address;
use nautilus_core::UUID4;
use nautilus_papertrade::journal::Journal;
use rstest::rstest;

const ACCOUNT: &str = "PAPERTRADE-IDENTITY";

#[rstest]
fn initializing_an_existing_journal_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("journal.redb");
    drop(Journal::initialize_new(&path, Address::repeat_byte(7), ACCOUNT).unwrap());
    assert!(Journal::initialize_new(&path, Address::repeat_byte(7), ACCOUNT).is_err());
}

#[rstest]
#[case::missing(false)]
#[case::empty(true)]
fn recovery_never_initializes_missing_or_empty_storage(#[case] empty_file: bool) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("journal.redb");
    if empty_file {
        std::fs::write(&path, []).unwrap();
    }
    assert!(
        Journal::recover_existing(&path, Address::repeat_byte(7), ACCOUNT, UUID4::new(), 0)
            .is_err()
    );
    if empty_file {
        assert_eq!(std::fs::metadata(path).unwrap().len(), 0);
    } else {
        assert!(!path.exists());
    }
}

#[rstest]
fn recovery_rejects_an_unrelated_persistent_identity() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("journal.redb");
    drop(Journal::initialize_new(&path, Address::repeat_byte(7), ACCOUNT).unwrap());
    assert!(
        Journal::recover_existing(&path, Address::repeat_byte(7), ACCOUNT, UUID4::new(), 0)
            .is_err()
    );
}

#[rstest]
fn persistent_identity_survives_reopen_and_rejects_a_higher_registration_floor() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("journal.redb");
    let journal = Journal::initialize_new(&path, Address::repeat_byte(7), ACCOUNT).unwrap();
    let identity = journal.identity_snapshot().unwrap();
    assert_eq!(identity.revision, 1);
    assert_eq!(journal.identity_snapshot().unwrap(), identity);
    drop(journal);
    assert!(
        Journal::recover_existing(
            &path,
            Address::repeat_byte(7),
            ACCOUNT,
            identity.journal_id,
            2
        )
        .is_err()
    );
    let recovered = Journal::recover_existing(
        &path,
        Address::repeat_byte(7),
        ACCOUNT,
        identity.journal_id,
        1,
    )
    .unwrap();
    assert_eq!(recovered.identity_snapshot().unwrap(), identity);
}

#[rstest]
#[case::wallet(Address::repeat_byte(8), ACCOUNT)]
#[case::routing_account(Address::repeat_byte(7), "PAPERTRADE-OTHER")]
fn recovery_never_rebinds_the_existing_owner(#[case] wallet: Address, #[case] account: &str) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("journal.redb");
    let journal = Journal::initialize_new(&path, Address::repeat_byte(7), ACCOUNT).unwrap();
    let identity = journal.identity_snapshot().unwrap();
    drop(journal);
    assert!(Journal::recover_existing(&path, wallet, account, identity.journal_id, 1).is_err());
    assert!(Journal::migrate_legacy(&path, wallet, account).is_err());
    assert_eq!(
        Journal::recover_existing(
            &path,
            Address::repeat_byte(7),
            ACCOUNT,
            identity.journal_id,
            1
        )
        .unwrap()
        .identity_snapshot()
        .unwrap(),
        identity
    );
}

#[rstest]
#[case::uuid("journal_uuid")]
#[case::revision("journal_revision")]
fn partially_missing_identity_requires_repair_not_legacy_migration(#[case] missing: &str) {
    use redb::{Database, TableDefinition};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("journal.redb");
    let journal = Journal::initialize_new(&path, Address::repeat_byte(7), ACCOUNT).unwrap();
    let identity = journal.identity_snapshot().unwrap();
    drop(journal);
    {
        let db = Database::open(&path).unwrap();
        let tx = db.begin_write().unwrap();
        tx.open_table(TableDefinition::<&str, &str>::new("metadata"))
            .unwrap()
            .remove(missing)
            .unwrap();
        tx.commit().unwrap();
    }
    assert!(
        Journal::recover_existing(
            &path,
            Address::repeat_byte(7),
            ACCOUNT,
            identity.journal_id,
            1
        )
        .is_err()
    );
    assert!(Journal::migrate_legacy(&path, Address::repeat_byte(7), ACCOUNT).is_err());
}

#[rstest]
fn concurrent_initialization_has_exactly_one_winner() {
    use std::sync::{Arc, Barrier};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("journal.redb");
    let barrier = Arc::new(Barrier::new(2));
    let mut handles = Vec::new();
    for _ in 0..2 {
        let path = path.clone();
        let barrier = barrier.clone();
        handles.push(std::thread::spawn(move || {
            barrier.wait();
            Journal::initialize_new(&path, Address::repeat_byte(7), ACCOUNT)
        }));
    }
    let results: Vec<_> = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect();
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
}

#[rstest]
#[case::removed(false)]
#[case::copied_same_uuid(true)]
fn active_handle_rejects_a_removed_or_replaced_path(#[case] replace: bool) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("journal.redb");
    let journal = Journal::initialize_new(&path, Address::repeat_byte(7), ACCOUNT).unwrap();
    let identity = journal.identity_snapshot().unwrap();
    let retained = dir.path().join("retained.redb");
    std::fs::rename(&path, &retained).unwrap();
    if replace {
        std::fs::copy(&retained, &path).unwrap();
    }
    assert!(journal.identity_snapshot().is_err());
    assert!(journal.remember(&[], &[]).is_err());
    assert!(journal.entries().is_err());
    drop(journal);
    assert_eq!(
        Journal::recover_existing(
            &retained,
            Address::repeat_byte(7),
            ACCOUNT,
            identity.journal_id,
            1
        )
        .unwrap()
        .identity_snapshot()
        .unwrap(),
        identity
    );
}

#[cfg(unix)]
#[rstest]
fn changing_a_parent_directory_alias_invalidates_the_active_handle() {
    let dir = tempfile::tempdir().unwrap();
    let original = dir.path().join("original");
    let replacement = dir.path().join("replacement");
    std::fs::create_dir(&original).unwrap();
    std::fs::create_dir(&replacement).unwrap();
    let alias = dir.path().join("current");
    std::os::unix::fs::symlink(&original, &alias).unwrap();
    let path = alias.join("journal.redb");
    let journal = Journal::initialize_new(&path, Address::repeat_byte(7), ACCOUNT).unwrap();
    let identity = journal.identity_snapshot().unwrap();
    std::fs::copy(
        original.join("journal.redb"),
        replacement.join("journal.redb"),
    )
    .unwrap();
    std::fs::remove_file(&alias).unwrap();
    std::os::unix::fs::symlink(&replacement, &alias).unwrap();
    assert!(journal.identity_snapshot().is_err());
    assert!(journal.remember(&[], &[]).is_err());
    assert!(journal.entries().is_err());
    drop(journal);
    assert_eq!(
        Journal::recover_existing(
            &original.join("journal.redb"),
            Address::repeat_byte(7),
            ACCOUNT,
            identity.journal_id,
            identity.revision,
        )
        .unwrap()
        .identity_snapshot()
        .unwrap(),
        identity,
    );
}

#[cfg(unix)]
#[rstest]
fn a_symbolic_alias_is_not_a_registered_journal_path() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("journal.redb");
    let journal = Journal::initialize_new(&path, Address::repeat_byte(7), ACCOUNT).unwrap();
    let identity = journal.identity_snapshot().unwrap();
    drop(journal);
    let alias = dir.path().join("alias.redb");
    std::os::unix::fs::symlink(&path, &alias).unwrap();
    assert!(
        Journal::recover_existing(
            &alias,
            Address::repeat_byte(7),
            ACCOUNT,
            identity.journal_id,
            1
        )
        .is_err()
    );
}

#[rstest]
#[case("owner")]
#[case("next_nonce")]
fn legacy_migration_does_not_reconstruct_missing_owner_or_nonce(#[case] missing: &str) {
    use redb::{Database, ReadableDatabase, TableDefinition};
    const META: TableDefinition<&str, &str> = TableDefinition::new("metadata");
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("journal.redb");
    drop(Journal::initialize_new(&path, Address::repeat_byte(7), ACCOUNT).unwrap());
    {
        let db = Database::open(&path).unwrap();
        let tx = db.begin_write().unwrap();
        {
            let mut meta = tx.open_table(META).unwrap();
            for key in ["journal_uuid", "journal_revision", missing] {
                meta.remove(key).unwrap();
            }
        }
        tx.commit().unwrap();
    }
    assert!(Journal::migrate_legacy(&path, Address::repeat_byte(7), ACCOUNT).is_err());
    let db = Database::open(&path).unwrap();
    let read = db.begin_read().unwrap();
    let meta = read.open_table(META).unwrap();
    for key in ["journal_uuid", "journal_revision", missing] {
        assert!(meta.get(key).unwrap().is_none());
    }
}

#[rstest]
#[case("journal_uuid", "not-a-uuid")]
#[case("journal_revision", "0")]
#[case("journal_revision", "+1")]
#[case("journal_revision", "01")]
#[case("journal_revision", "9223372036854775808")]
fn malformed_identity_is_not_silently_migrated(#[case] key: &str, #[case] value: &str) {
    use redb::{Database, TableDefinition};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("journal.redb");
    let journal = Journal::initialize_new(&path, Address::repeat_byte(7), ACCOUNT).unwrap();
    let identity = journal.identity_snapshot().unwrap();
    drop(journal);
    {
        let db = Database::open(&path).unwrap();
        let tx = db.begin_write().unwrap();
        tx.open_table(TableDefinition::<&str, &str>::new("metadata"))
            .unwrap()
            .insert(key, value)
            .unwrap();
        tx.commit().unwrap();
    }
    assert!(
        Journal::recover_existing(
            &path,
            Address::repeat_byte(7),
            ACCOUNT,
            identity.journal_id,
            1
        )
        .is_err()
    );
    assert!(Journal::migrate_legacy(&path, Address::repeat_byte(7), ACCOUNT).is_err());
}
