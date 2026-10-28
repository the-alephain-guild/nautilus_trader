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

use std::{
    fs::OpenOptions,
    io,
    path::{Path, PathBuf},
};

use alloy_primitives::Address;
use anyhow::{Result, ensure};
pub use nautilus_common::clients::ExecutionJournalIdentity as JournalIdentitySnapshot;
use nautilus_core::UUID4;
use redb::{
    Database, ReadableDatabase, ReadableTable, StorageBackend, WriteTransaction,
    backends::FileBackend,
};
use same_file::Handle;

use super::META;
use crate::signing::{CHAIN_ID, EXCHANGE};

pub(super) const UUID_KEY: &str = "journal_uuid";
pub(super) const REVISION_KEY: &str = "journal_revision";
pub(super) const MAX_REVISION: u64 = u64::MAX / 2;

/// Reuse redb's locked backend while forbidding its empty-file creation branch.
#[derive(Debug)]
struct ExistingBackend(FileBackend);

impl StorageBackend for ExistingBackend {
    fn len(&self) -> io::Result<u64> {
        let length = self.0.len()?;
        if length == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Existing journal is empty",
            ));
        }
        Ok(length)
    }
    fn read(&self, offset: u64, out: &mut [u8]) -> io::Result<()> {
        self.0.read(offset, out)
    }
    fn set_len(&self, len: u64) -> io::Result<()> {
        self.0.set_len(len)
    }
    fn sync_data(&self) -> io::Result<()> {
        self.0.sync_data()
    }
    fn write(&self, offset: u64, data: &[u8]) -> io::Result<()> {
        self.0.write(offset, data)
    }
    fn close(&self) -> io::Result<()> {
        self.0.close()
    }
}

#[derive(Debug)]
pub(super) struct JournalStorage {
    pub(super) db: Database,
    pub(super) journal_id: UUID4,
    pub(super) owner: String,
    path: PathBuf,
    canonical_path: PathBuf,
    file: Handle,
}

impl JournalStorage {
    fn open_file(path: &Path, create_new: bool) -> Result<(Database, PathBuf, Handle)> {
        if !create_new {
            let metadata = std::fs::symlink_metadata(path)?;
            ensure!(
                metadata.file_type().is_file(),
                "Journal must be an existing regular file"
            );
            ensure!(metadata.len() > 0, "Existing journal is empty");
        }
        let mut options = OpenOptions::new();
        options.read(true).write(true).create_new(create_new);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(path)?;
        let identity = Handle::from_file(file.try_clone()?)?;
        Self::check_path(path, &identity)?;
        let canonical = path.canonicalize()?;
        let db = if create_new {
            Database::builder().create_file(file)?
        } else {
            Database::builder().create_with_backend(ExistingBackend(FileBackend::new(file)?))?
        };
        Self::check_path(path, &identity)?;
        Self::check_path(&canonical, &identity)?;
        Ok((db, canonical, identity))
    }

    fn domain(wallet: Address, account: &str) -> String {
        format!("{CHAIN_ID}:{EXCHANGE:#x}:{wallet:#x}:{account}")
    }

    pub(super) fn create_new(path: &Path, wallet: Address, account: &str) -> Result<Self> {
        let path = std::path::absolute(path)?;
        let (db, canonical_path, file) = Self::open_file(&path, true)?;
        let storage = Self {
            db,
            path,
            canonical_path,
            file,
            journal_id: UUID4::new(),
            owner: Self::domain(wallet, account),
        };
        storage.secure_permissions()?;
        Ok(storage)
    }

    pub(super) fn open_existing(
        path: &Path,
        wallet: Address,
        account: &str,
        expected: Option<(UUID4, u64)>,
        allow_legacy: bool,
    ) -> Result<(Self, bool)> {
        let path = std::path::absolute(path)?;
        let (db, canonical_path, file) = Self::open_file(&path, false)?;
        let owner = Self::domain(wallet, account);
        let snapshot = Self::read_identity(&db, &owner)?;
        let legacy = snapshot.is_none();
        ensure!(
            !legacy || allow_legacy,
            "Legacy journal requires explicit identity migration"
        );
        let journal_id = match snapshot {
            Some(snapshot) => {
                if let Some((expected_id, minimum_revision)) = expected {
                    ensure!(
                        minimum_revision <= MAX_REVISION,
                        "Invalid minimum journal revision"
                    );
                    ensure!(
                        snapshot.journal_id == expected_id,
                        "Journal UUID differs from registration"
                    );
                    ensure!(
                        snapshot.revision >= minimum_revision,
                        "Journal revision regressed"
                    );
                }
                snapshot.journal_id
            }
            None => {
                ensure!(expected.is_none(), "Registered journal identity is missing");
                UUID4::new()
            }
        };
        let storage = Self {
            db,
            journal_id,
            owner,
            path,
            canonical_path,
            file,
        };
        storage.ensure_path_current()?;
        storage.secure_permissions()?;
        Ok((storage, legacy))
    }

    fn secure_permissions(&self) -> Result<()> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            self.file
                .as_file()
                .set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }
        Ok(())
    }

    fn read_identity(db: &Database, owner: &str) -> Result<Option<JournalIdentitySnapshot>> {
        let tx = db.begin_read()?;
        let meta = tx.open_table(META)?;
        let actual_owner = meta
            .get("owner")?
            .ok_or_else(|| anyhow::anyhow!("Journal owner is missing"))?;
        ensure!(
            actual_owner.value() == owner,
            "Journal belongs to a different account"
        );
        let id = meta.get(UUID_KEY)?.map(|value| value.value().to_owned());
        let revision = meta
            .get(REVISION_KEY)?
            .map(|value| value.value().to_owned());
        match (id, revision) {
            (None, None) => Ok(None),
            (Some(id), Some(revision)) => {
                let journal_id = id
                    .parse::<UUID4>()
                    .map_err(|_| anyhow::anyhow!("Invalid persisted journal UUID"))?;
                ensure!(
                    journal_id.to_string() == id,
                    "Journal UUID is not canonical"
                );
                let parsed_revision = parse_revision(&revision)?;
                ensure!(parsed_revision > 0, "Journal initialization is incomplete");
                Ok(Some(JournalIdentitySnapshot {
                    schema_version: 1,
                    journal_id,
                    revision: parsed_revision,
                    owner: owner.to_owned(),
                }))
            }
            _ => anyhow::bail!("Journal identity metadata is incomplete"),
        }
    }

    fn check_path(path: &Path, expected: &Handle) -> Result<()> {
        ensure!(
            std::fs::symlink_metadata(path)?.file_type().is_file(),
            "Journal path is not a regular file"
        );
        let current = Handle::from_path(path)?;
        ensure!(
            expected == &current,
            "Journal path no longer names the opened file"
        );
        ensure!(
            std::fs::symlink_metadata(path)?.file_type().is_file(),
            "Journal path is not a regular file"
        );
        Ok(())
    }

    pub(super) fn ensure_path_current(&self) -> Result<()> {
        Self::check_path(&self.path, &self.file)?;
        Self::check_path(&self.canonical_path, &self.file)
    }

    pub(super) fn identity_snapshot(&self) -> Result<JournalIdentitySnapshot> {
        self.ensure_path_current()?;
        let snapshot = Self::read_identity(&self.db, &self.owner)?
            .ok_or_else(|| anyhow::anyhow!("Journal identity metadata is missing"))?;
        ensure!(
            snapshot.journal_id == self.journal_id,
            "Journal UUID changed while open"
        );
        self.ensure_path_current()?;
        Ok(snapshot)
    }

    pub(super) fn commit(&self, mut tx: WriteTransaction) -> Result<()> {
        self.ensure_path_current()?;
        tx.set_durability(redb::Durability::Immediate)?;
        {
            let mut meta = tx.open_table(META)?;
            ensure!(
                meta.get("owner")?
                    .is_some_and(|value| value.value() == self.owner),
                "Journal owner changed"
            );
            ensure!(
                meta.get(UUID_KEY)?
                    .is_some_and(|value| value.value() == self.journal_id.to_string()),
                "Journal UUID changed"
            );
            let previous = meta
                .get(REVISION_KEY)?
                .ok_or_else(|| anyhow::anyhow!("Journal revision is missing"))?
                .value()
                .to_owned();
            let next = parse_revision(&previous)?
                .checked_add(1)
                .filter(|value| *value <= MAX_REVISION)
                .ok_or_else(|| anyhow::anyhow!("Journal revision exhausted"))?;
            meta.insert(REVISION_KEY, next.to_string().as_str())?;
        }
        self.ensure_path_current()?;
        tx.commit()?;
        Ok(())
    }
}

fn parse_revision(value: &str) -> Result<u64> {
    let revision = value
        .parse::<u64>()
        .map_err(|_| anyhow::anyhow!("Invalid journal revision"))?;
    ensure!(
        revision <= MAX_REVISION && revision.to_string() == value,
        "Invalid journal revision"
    );
    Ok(revision)
}

#[cfg(test)]
mod tests {
    use alloy::signers::local::PrivateKeySigner;
    use alloy_primitives::{Address, B256};
    use nautilus_common::messages::execution::SubmitOrder;
    use nautilus_core::{UUID4, UnixNanos};
    use nautilus_model::{
        enums::{OrderStatus, OrderType},
        identifiers::{ClientOrderId, InstrumentId, TraderId},
        orders::OrderTestBuilder,
        types::Quantity,
    };
    use rstest::rstest;

    use crate::{OpenPosition, PapertradeSigner, journal::Journal};

    #[rstest]
    fn observed_success_keeps_its_committed_revision_when_reporting_a_conflict() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("journal.redb");
        let wallet = Address::repeat_byte(7);
        let journal = Journal::initialize_new(&path, wallet, "PAPERTRADE-001").unwrap();
        let order = OrderTestBuilder::new(OrderType::Market)
            .instrument_id(InstrumentId::from("BTC-USD-PERP.PAPERTRADE"))
            .client_order_id(ClientOrderId::from("PT-SUCCESS-CONFLICT"))
            .quantity(Quantity::from(1000))
            .build();
        let command = SubmitOrder::from_order(
            &order,
            TraderId::from("TESTER-001"),
            None,
            None,
            UUID4::new(),
            UnixNanos::from(1),
        );
        let signer =
            PapertradeSigner::new(PrivateKeySigner::from_bytes(&B256::repeat_byte(17)).unwrap());
        let (mut entry, _) = journal
            .prepare(&command, None, 100, |nonce| {
                signer.sign_open(
                    &OpenPosition {
                        user: wallet,
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
            })
            .unwrap();
        entry.status = OrderStatus::Rejected;
        journal.update(&entry).unwrap();
        let before = journal.identity_snapshot().unwrap();
        assert!(journal.observe_successes(&[entry.intent_id]).is_err());
        let after = journal.identity_snapshot().unwrap();
        assert_eq!(after.revision, before.revision + 1);
        assert!(journal.entries().unwrap()[0].execution_succeeded);
        drop(journal);
        assert_eq!(
            Journal::recover_existing(
                &path,
                wallet,
                "PAPERTRADE-001",
                after.journal_id,
                after.revision
            )
            .unwrap()
            .identity_snapshot()
            .unwrap(),
            after
        );
    }
}
