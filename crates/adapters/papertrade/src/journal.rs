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

//! Transactional pre-send journal. The database lock excludes concurrent writers.

use std::path::Path;

use std::sync::{Arc, Weak};

use alloy_primitives::{Address, B256, U256};
use anyhow::{Result, ensure};
use nautilus_common::{clients::ExecutionJournalIdentityProbe, messages::execution::SubmitOrder};
use nautilus_core::UUID4;
use nautilus_model::{
    enums::OrderStatus,
    identifiers::{ClientId, ClientOrderId},
    reports::FillReport,
};
use redb::{Durability, ReadableDatabase, ReadableTable, TableDefinition};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    signing::SignedIntent,
    wire::{CloseRecord, Raw, VenuePosition},
};

const META: TableDefinition<&str, &str> = TableDefinition::new("metadata");
const ORDERS: TableDefinition<&str, &str> = TableDefinition::new("orders");
const POSITIONS: TableDefinition<&str, &str> = TableDefinition::new("observed_positions");
const CLOSES: TableDefinition<&str, &str> = TableDefinition::new("observed_closes");

#[derive(Debug)]
struct JournalProbe {
    journal: Weak<Journal>,
    process_id: u32,
    client_id: ClientId,
}

impl ExecutionJournalIdentityProbe for JournalProbe {
    fn snapshot(&self) -> Result<JournalIdentitySnapshot> {
        ensure!(
            std::process::id() == self.process_id,
            "Original execution journal belongs to another process"
        );
        self.journal
            .upgrade()
            .ok_or_else(|| anyhow::anyhow!("Original execution journal is unavailable"))?
            .identity_snapshot()
    }

    fn order_evidence_json(&self, client_order_id: &str) -> Result<String> {
        ensure!(
            std::process::id() == self.process_id,
            "Original execution journal belongs to another process"
        );
        let journal = self
            .journal
            .upgrade()
            .ok_or_else(|| anyhow::anyhow!("Original execution journal is unavailable"))?;
        serde_json::to_string(&journal.order_evidence_snapshot(self.client_id, client_order_id)?)
            .map_err(Into::into)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JournalEntry {
    pub command: SubmitOrder,
    pub intent_id: B256,
    pub action: String,
    pub deadline: u64,
    pub body: Value,
    pub target: Option<Raw>,
    pub status: OrderStatus,
    pub fills: Vec<FillReport>,
    pub nonce: Raw,
    pub submitted_block: u64,
    pub scanned_through: u64,
    // Older journals cannot prove that a transport attempt never began.
    #[serde(default = "legacy_send_attempted")]
    pub send_attempted: bool,
    #[serde(default)]
    pub execution_succeeded: bool,
}
fn legacy_send_attempted() -> bool {
    true
}

impl JournalEntry {
    pub(crate) fn signed(&self) -> Result<SignedIntent> {
        SignedIntent::restore(
            self.intent_id,
            &self.action,
            self.deadline,
            self.body.clone(),
        )
    }
}

mod evidence;
mod identity;
pub use evidence::JournalOrderEvidence;
pub use identity::JournalIdentitySnapshot;
use identity::{JournalStorage, REVISION_KEY, UUID_KEY};

#[derive(Debug)]
pub struct Journal {
    storage: JournalStorage,
}

impl Journal {
    pub(crate) fn identity_probe(
        self: &Arc<Self>,
        client_id: ClientId,
    ) -> Arc<dyn ExecutionJournalIdentityProbe> {
        Arc::new(JournalProbe {
            journal: Arc::downgrade(self),
            process_id: self.storage.process_id,
            client_id,
        })
    }

    /// Explicit first creation. Existing files are never reused or overwritten.
    pub fn initialize_new(path: &Path, wallet: Address, account: &str) -> Result<Self> {
        let storage = JournalStorage::create_new(path, wallet, account)?;
        let journal = Self { storage };
        let mut tx = journal.storage.db.begin_write()?;
        tx.set_durability(Durability::Immediate)?;
        {
            let mut meta = tx.open_table(META)?;
            meta.insert("owner", journal.storage.owner.as_str())?;
            meta.insert(UUID_KEY, journal.storage.journal_id.to_string().as_str())?;
            meta.insert(REVISION_KEY, "0")?;
            let seed: U256 =
                U256::from_str_radix(&UUID4::new().to_string().replace('-', ""), 16)? << 64;
            meta.insert("next_nonce", seed.to_string().as_str())?;
            tx.open_table(ORDERS)?;
            tx.open_table(POSITIONS)?;
            tx.open_table(CLOSES)?;
        }
        journal.storage.commit(tx)?;
        Ok(journal)
    }

    /// Recover only the registered identity, before applying any legacy entry repairs.
    pub fn recover_existing(
        path: &Path,
        wallet: Address,
        account: &str,
        expected_journal_id: UUID4,
        minimum_revision: u64,
    ) -> Result<Self> {
        let (storage, _) = JournalStorage::open_existing(
            path,
            wallet,
            account,
            Some((expected_journal_id, minimum_revision)),
            false,
        )?;
        let journal = Self { storage };
        journal.validate_tables()?;
        journal.repair_legacy_entries()?;
        Ok(journal)
    }

    /// Explicitly identify an existing legacy owner without rewriting its business rows.
    pub fn migrate_legacy(path: &Path, wallet: Address, account: &str) -> Result<Self> {
        let (storage, missing) = JournalStorage::open_existing(path, wallet, account, None, true)?;
        let journal = Self { storage };
        journal.validate_tables()?;
        if missing {
            let mut tx = journal.storage.db.begin_write()?;
            tx.set_durability(Durability::Immediate)?;
            {
                let mut meta = tx.open_table(META)?;
                meta.insert(UUID_KEY, journal.storage.journal_id.to_string().as_str())?;
                meta.insert(REVISION_KEY, "0")?;
            }
            journal.storage.commit(tx)?;
        }
        Ok(journal)
    }

    /// Compatibility entry for callers without a registration; strict recovery uses recover_existing.
    pub fn open(path: &Path, wallet: Address, account: &str) -> Result<Self> {
        if !path.try_exists()? {
            return Self::initialize_new(path, wallet, account);
        }
        let (storage, _) = JournalStorage::open_existing(path, wallet, account, None, false)?;
        let journal = Self { storage };
        journal.validate_tables()?;
        journal.repair_legacy_entries()?;
        Ok(journal)
    }

    pub fn identity_snapshot(&self) -> Result<JournalIdentitySnapshot> {
        self.storage.identity_snapshot()
    }

    fn validate_tables(&self) -> Result<()> {
        self.storage.ensure_path_current()?;
        let tx = self.storage.db.begin_read()?;
        let meta = tx.open_table(META)?;
        let nonce = meta
            .get("next_nonce")?
            .ok_or_else(|| anyhow::anyhow!("Journal nonce is missing"))?;
        U256::from_str_radix(nonce.value(), 10)?;
        tx.open_table(ORDERS)?;
        tx.open_table(POSITIONS)?;
        tx.open_table(CLOSES)?;
        self.storage.ensure_path_current()?;
        Ok(())
    }

    fn repair_legacy_entries(&self) -> Result<()> {
        self.storage.ensure_path_current()?;
        let mut tx = self.storage.db.begin_write()?;
        tx.set_durability(Durability::Immediate)?;
        let changed = {
            let mut orders = tx.open_table(ORDERS)?;
            let mut legacy = Vec::new();
            for row in orders.iter()? {
                let (key, value) = row?;
                let raw: Value = serde_json::from_str(value.value())?;
                if raw.get("execution_succeeded").is_some() {
                    continue;
                }
                let mut entry: JournalEntry = serde_json::from_value(raw)?;
                if entry.fills.is_empty()
                    && matches!(
                        entry.status,
                        OrderStatus::Submitted | OrderStatus::Accepted | OrderStatus::Expired
                    )
                {
                    // Earlier readers could expire executed intents after discarding success.
                    // Recheck that terminal decision without ever resubmitting its payload.
                    if entry.status == OrderStatus::Expired {
                        entry.status = OrderStatus::Submitted;
                        entry.send_attempted = true;
                    }
                    entry.scanned_through = entry.submitted_block.saturating_sub(1);
                    legacy.push((key.value().to_owned(), serde_json::to_string(&entry)?));
                }
            }
            let changed = !legacy.is_empty();
            for (key, value) in legacy {
                orders.insert(key.as_str(), value.as_str())?;
            }
            changed
        };
        if changed {
            self.storage.commit(tx)?;
        }
        Ok(())
    }
    pub fn entries(&self) -> Result<Vec<JournalEntry>> {
        self.storage.ensure_path_current()?;
        let tx = self.storage.db.begin_read()?;
        let table = tx.open_table(ORDERS)?;
        table
            .iter()?
            .map(|row| {
                let (_, value) = row?;
                Ok(serde_json::from_str(value.value())?)
            })
            .collect()
    }
    pub(crate) fn recover_unsent(&self) -> Result<()> {
        for mut entry in self.entries()? {
            if !entry.send_attempted
                && matches!(entry.status, OrderStatus::Submitted | OrderStatus::Accepted)
            {
                ensure!(entry.fills.is_empty(), "Unsent intent has a fill");
                entry.status = OrderStatus::Rejected;
                self.update(&entry)?;
            }
        }
        Ok(())
    }
    /// Commits the transport-attempt marker before any request can leave this process.
    pub fn begin_submission(&self, client_order_id: ClientOrderId) -> Result<Option<JournalEntry>> {
        self.storage.ensure_path_current()?;
        let mut tx = self.storage.db.begin_write()?;
        tx.set_durability(Durability::Immediate)?;
        let entry = {
            let mut table = tx.open_table(ORDERS)?;
            let mut entry: JournalEntry = serde_json::from_str(
                table
                    .get(client_order_id.as_str())?
                    .ok_or_else(|| anyhow::anyhow!("Unknown submission"))?
                    .value(),
            )?;
            if entry.send_attempted
                || !matches!(entry.status, OrderStatus::Submitted | OrderStatus::Accepted)
            {
                return Ok(None);
            }
            entry.send_attempted = true;
            table.insert(
                client_order_id.as_str(),
                serde_json::to_string(&entry)?.as_str(),
            )?;
            entry
        };
        self.storage.commit(tx)?;
        Ok(Some(entry))
    }
    pub(crate) fn snapshot(&self) -> Result<Option<crate::wire::AccountSnapshot>> {
        self.storage.ensure_path_current()?;
        let tx = self.storage.db.begin_read()?;
        let table = tx.open_table(META)?;
        table
            .get("wallet_snapshot")?
            .map(|value| serde_json::from_str(value.value()).map_err(Into::into))
            .transpose()
    }
    pub fn checkpoint(&self, snapshot: &crate::wire::AccountSnapshot) -> Result<()> {
        self.storage.ensure_path_current()?;
        let json = serde_json::to_string(snapshot)?;
        let mut tx = self.storage.db.begin_write()?;
        tx.set_durability(Durability::Immediate)?;
        {
            let mut table = tx.open_table(META)?;
            table.insert("wallet_snapshot", json.as_str())?;
        }
        self.storage.commit(tx)?;
        Ok(())
    }
    /// Retains stream success before fallible reconciliation or scan advancement.
    pub(crate) fn observe_successes(&self, successes: &[B256]) -> Result<()> {
        self.storage.ensure_path_current()?;
        let mut tx = self.storage.db.begin_write()?;
        tx.set_durability(Durability::Immediate)?;
        let mut conflict = false;
        {
            let mut table = tx.open_table(ORDERS)?;
            let mut changes = Vec::new();
            for row in table.iter()? {
                let (key, value) = row?;
                let mut entry: JournalEntry = serde_json::from_str(value.value())?;
                if !entry.execution_succeeded && successes.contains(&entry.intent_id) {
                    entry.execution_succeeded = true;
                    changes.push((key.value().to_owned(), serde_json::to_string(&entry)?));
                }
                conflict |= entry.execution_succeeded
                    && matches!(
                        entry.status,
                        OrderStatus::Rejected | OrderStatus::Canceled | OrderStatus::Expired
                    );
            }
            for (key, value) in changes {
                table.insert(key.as_str(), value.as_str())?;
            }
        }
        self.storage.commit(tx)?;
        // Contradictory order state blocks reconciliation without discarding other successes.
        ensure!(
            !conflict,
            "Terminal failure contradicts observed execution success"
        );
        Ok(())
    }
    pub fn remember(&self, positions: &[VenuePosition], closes: &[CloseRecord]) -> Result<()> {
        self.storage.ensure_path_current()?;
        let mut tx = self.storage.db.begin_write()?;
        tx.set_durability(Durability::Immediate)?;
        {
            let closes = tx.open_table(CLOSES)?;
            let mut table = tx.open_table(POSITIONS)?;
            for position in positions {
                let mut position = position.clone();
                let key = position.position_id.to_string();
                if let Some(close) = closes.get(key.as_str())? {
                    let close: CloseRecord = serde_json::from_str(close.value())?;
                    close.validate_position(&position)?;
                }
                if let Some(old) = table.get(key.as_str())? {
                    let old: VenuePosition = serde_json::from_str(old.value())?;
                    ensure!(
                        old.entry_price == position.entry_price
                            && old.margin == position.margin
                            && old.leverage == position.leverage
                            && old.is_long == position.is_long
                            && old.instrument_id == position.instrument_id
                            && old.use_debt == position.use_debt
                            && old.opened_at_ms == position.opened_at_ms
                            && (old.intent_id.is_none()
                                || position.intent_id.is_none()
                                || old.intent_id == position.intent_id),
                        "Position identity changed"
                    );
                    position.intent_id = position.intent_id.or(old.intent_id);
                }
                let json = serde_json::to_string(&position)?;
                table.insert(key.as_str(), json.as_str())?;
            }
        }
        {
            let positions = tx.open_table(POSITIONS)?;
            let mut table = tx.open_table(CLOSES)?;
            for record in closes {
                record.validate()?;
                let key = record.position_id.to_string();
                if let Some(position) = positions.get(key.as_str())? {
                    let position: VenuePosition = serde_json::from_str(position.value())?;
                    record.validate_position(&position)?;
                }
                let json = serde_json::to_string(record)?;
                if let Some(old) = table.get(key.as_str())? {
                    let old: CloseRecord = serde_json::from_str(old.value())?;
                    ensure!(old == *record, "Close history changed");
                }
                table.insert(key.as_str(), json.as_str())?;
            }
        }
        self.storage.commit(tx)?;
        Ok(())
    }
    pub fn observed(&self) -> Result<Vec<VenuePosition>> {
        self.storage.ensure_path_current()?;
        let tx = self.storage.db.begin_read()?;
        let table = tx.open_table(POSITIONS)?;
        table
            .iter()?
            .map(|row| {
                let (_, v) = row?;
                Ok(serde_json::from_str(v.value())?)
            })
            .collect()
    }
    pub fn closes(&self) -> Result<Vec<CloseRecord>> {
        self.storage.ensure_path_current()?;
        let tx = self.storage.db.begin_read()?;
        let table = tx.open_table(CLOSES)?;
        table
            .iter()?
            .map(|row| {
                let (_, v) = row?;
                Ok(serde_json::from_str(v.value())?)
            })
            .collect()
    }
    pub fn prepare<F>(
        &self,
        command: &SubmitOrder,
        target: Option<Raw>,
        submitted_block: u64,
        build: F,
    ) -> Result<(JournalEntry, bool)>
    where
        F: FnOnce(U256) -> Result<SignedIntent>,
    {
        self.storage.ensure_path_current()?;
        let key = command.client_order_id.to_string();
        let mut tx = self.storage.db.begin_write()?;
        tx.set_durability(Durability::Immediate)?;
        let entry = {
            let mut table = tx.open_table(ORDERS)?;
            if let Some(old) = table.get(key.as_str())? {
                let entry: JournalEntry = serde_json::from_str(old.value())?;
                ensure!(
                    entry.command.order_init == command.order_init
                        && entry.target == target
                        && entry.command.params == command.params,
                    "Client order ID payload conflict"
                );
                return Ok((entry, false));
            }
            let mut meta = tx.open_table(META)?;
            let nonce = U256::from_str_radix(
                meta.get("next_nonce")?
                    .ok_or_else(|| anyhow::anyhow!("Missing nonce"))?
                    .value(),
                10,
            )?;
            let next = nonce
                .checked_add(U256::from(1))
                .ok_or_else(|| anyhow::anyhow!("Nonce exhausted"))?;
            let intent = build(nonce)?;
            let entry = JournalEntry {
                command: command.clone(),
                intent_id: intent.id(),
                action: intent.action().to_owned(),
                deadline: intent.deadline(),
                body: intent.body().clone(),
                target,
                status: OrderStatus::Submitted,
                fills: Vec::new(),
                nonce: Raw(nonce),
                submitted_block,
                scanned_through: submitted_block.saturating_sub(1),
                send_attempted: false,
                execution_succeeded: false,
            };
            let json = serde_json::to_string(&entry)?;
            table.insert(key.as_str(), json.as_str())?;
            meta.insert("next_nonce", next.to_string().as_str())?;
            entry
        };
        self.storage.commit(tx)?;
        Ok((entry, true))
    }
    pub fn update(&self, entry: &JournalEntry) -> Result<()> {
        self.storage.ensure_path_current()?;
        let key = entry.command.client_order_id.to_string();
        let mut tx = self.storage.db.begin_write()?;
        tx.set_durability(Durability::Immediate)?;
        {
            let mut table = tx.open_table(ORDERS)?;
            let previous: JournalEntry = serde_json::from_str(
                table
                    .get(key.as_str())?
                    .ok_or_else(|| anyhow::anyhow!("Unknown journal order"))?
                    .value(),
            )?;
            ensure!(
                previous.intent_id == entry.intent_id
                    && previous.body == entry.body
                    && previous.nonce == entry.nonce
                    && previous.command == entry.command
                    && previous.action == entry.action
                    && previous.target == entry.target
                    && previous.deadline == entry.deadline
                    && previous.submitted_block == entry.submitted_block,
                "Immutable journal payload changed"
            );
            ensure!(
                !(previous.execution_succeeded || entry.execution_succeeded)
                    || (!matches!(
                        entry.status,
                        OrderStatus::Rejected | OrderStatus::Canceled | OrderStatus::Expired
                    ) && !matches!(
                        previous.status,
                        OrderStatus::Rejected | OrderStatus::Canceled | OrderStatus::Expired
                    )),
                "Terminal failure contradicts observed execution success"
            );
            if matches!(
                previous.status,
                OrderStatus::Filled
                    | OrderStatus::Rejected
                    | OrderStatus::Canceled
                    | OrderStatus::Expired
            ) && matches!(entry.status, OrderStatus::Submitted | OrderStatus::Accepted)
            {
                return Ok(());
            }
            ensure!(
                previous
                    .fills
                    .iter()
                    .all(
                        |old| entry.fills.iter().any(|new| old.trade_id == new.trade_id
                            && old.last_qty == new.last_qty
                            && old.last_px == new.last_px
                            && old.commission == new.commission
                            && old.venue_position_id == new.venue_position_id)
                    ),
                "Confirmed fill cannot be removed or rewritten"
            );
            let mut merged = entry.clone();
            merged.send_attempted |= previous.send_attempted;
            merged.execution_succeeded |= previous.execution_succeeded;
            merged.scanned_through = merged.scanned_through.max(previous.scanned_through);
            table.insert(key.as_str(), serde_json::to_string(&merged)?.as_str())?;
        }
        self.storage.commit(tx)?;
        Ok(())
    }
}
