// Copyright (C) 2015-2026 Nautech Systems Pty Ltd. All rights reserved.
// Licensed under the GNU Lesser General Public License Version 3.0.
// https://www.gnu.org/licenses/lgpl-3.0.en.html

//! Per-intent evidence read and sealed in the original persistent journal.

use alloy_primitives::{B256, keccak256};
use anyhow::{Result, ensure};
use nautilus_core::UUID4;
use nautilus_model::{
    enums::OrderStatus,
    identifiers::{ClientId, ClientOrderId},
};
use redb::{
    Durability, ReadTransaction, ReadableDatabase, ReadableTable, TableDefinition, TableError,
};
use serde::{Deserialize, Serialize};

use super::{Journal, JournalEntry, JournalIdentitySnapshot, ORDERS, identity::JournalStorage};
use crate::wire::AccountSnapshot;

const PROOFS: TableDefinition<&str, &str> = TableDefinition::new("order_no_fill_proofs");
const EXPIRY_WITNESSES: TableDefinition<&str, &str> =
    TableDefinition::new("expiry_account_witnesses");

#[derive(Debug, Serialize)]
pub struct JournalOrderEvidence {
    schema_version: u8,
    journal: JournalIdentitySnapshot,
    client_id: ClientId,
    account_id: String,
    client_order_id: String,
    #[serde(flatten)]
    observation: OrderObservation,
}

impl JournalOrderEvidence {
    pub(super) fn is_unresolved(&self) -> bool {
        matches!(self.observation, OrderObservation::Unproven { .. })
    }
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum OrderObservation {
    Missing,
    Unproven {
        binding: OrderBinding,
    },
    Filled {
        binding: OrderBinding,
    },
    ProvenNoFill {
        binding: OrderBinding,
        proof: NoFillProof,
    },
}

#[derive(Debug, Serialize)]
struct OrderBinding {
    intent_id: B256,
    command_keccak256: B256,
    submission_params_keccak256: B256,
    action: String,
    target: Option<String>,
    instrument_id: String,
    side: String,
    quantity: String,
    quote_quantity: bool,
    reduce_only: bool,
    deadline: u64,
}

impl OrderBinding {
    fn new(entry: &JournalEntry) -> Result<Self> {
        let command = serde_json::to_vec(&(
            &entry.command,
            entry.intent_id,
            &entry.action,
            entry.target,
            entry.deadline,
            entry.nonce,
        ))?;
        Ok(Self {
            intent_id: entry.intent_id,
            command_keccak256: keccak256(command),
            submission_params_keccak256: super::submission_params_keccak256(
                entry.command.params.as_ref(),
            )?,
            action: entry.action.clone(),
            target: entry.target.map(|target| target.to_string()),
            instrument_id: entry.command.instrument_id.to_string(),
            side: entry.command.order_init.order_side.to_string(),
            quantity: entry.command.order_init.quantity.to_string(),
            quote_quantity: entry.command.order_init.quote_quantity,
            reduce_only: entry.command.order_init.reduce_only,
            deadline: entry.deadline,
        })
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct NoFillProof {
    schema_version: u8,
    journal_id: UUID4,
    command_keccak256: B256,
    sealed_at_revision: u64,
    source: NoFillSource,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum NoFillSource {
    NeverSubmitted {},
    ExpiredWithCoverage { coverage: ExpiryCoverage },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExpiryCoverage {
    deadline: u64,
    coverage_block: u64,
    scanned_through: u64,
    block_timestamp: u64,
    account_block: u64,
    account_timestamp: u64,
    account_revision: u64,
    account_snapshot_keccak256: B256,
}

impl ExpiryCoverage {
    fn validate(&self, entry: &JournalEntry) -> Result<()> {
        ensure!(
            self.deadline == entry.deadline
                && self.coverage_block >= entry.submitted_block
                && self.scanned_through >= self.coverage_block
                && entry.scanned_through >= self.scanned_through
                && self.account_block >= self.coverage_block
                && self.block_timestamp > entry.deadline
                && self.account_timestamp >= self.block_timestamp
                && self.account_timestamp > entry.deadline
                && self.account_revision > 0,
            "Expiry proof has incomplete execution coverage"
        );
        Ok(())
    }

    fn validate_snapshot(
        &self,
        entry: &JournalEntry,
        snapshot_json: &str,
        storage: &JournalStorage,
    ) -> Result<()> {
        self.validate(entry)?;
        ensure!(
            keccak256(snapshot_json.as_bytes()) == self.account_snapshot_keccak256,
            "Expiry proof account snapshot digest differs"
        );
        let snapshot: AccountSnapshot = serde_json::from_str(snapshot_json)?;
        storage.require_wallet(snapshot.address)?;
        snapshot.validate(snapshot.address)?;
        ensure!(
            snapshot.account_revision == self.account_revision
                && snapshot.indexed_through_block == self.account_block
                && snapshot.indexed_through_timestamp == self.account_timestamp
                && !snapshot
                    .operations
                    .iter()
                    .any(|operation| operation.intent_id == entry.intent_id)
                && !snapshot
                    .intent_outcomes
                    .iter()
                    .any(|(intent, outcome)| *intent == entry.intent_id && outcome == "succeeded")
                && !snapshot
                    .open_positions
                    .iter()
                    .any(|position| position.intent_id == Some(entry.intent_id)),
            "Expiry proof conflicts with its account snapshot witness"
        );
        Ok(())
    }
}

#[derive(Debug)]
pub(crate) struct ExpiryCandidate {
    client_order_id: ClientOrderId,
    command_keccak256: B256,
    coverage: ExpiryCoverage,
}

impl ExpiryCandidate {
    pub(crate) fn new(
        entry: &JournalEntry,
        snapshot: &AccountSnapshot,
        coverage_block: u64,
        block_timestamp: u64,
    ) -> Result<Self> {
        let coverage = ExpiryCoverage {
            deadline: entry.deadline,
            coverage_block,
            scanned_through: entry.scanned_through,
            block_timestamp,
            account_block: snapshot.indexed_through_block,
            account_timestamp: snapshot.indexed_through_timestamp,
            account_revision: snapshot.account_revision,
            account_snapshot_keccak256: keccak256(serde_json::to_vec(snapshot)?),
        };
        coverage.validate(entry)?;
        ensure!(
            entry.fills.is_empty()
                && !entry.execution_succeeded
                && matches!(entry.status, OrderStatus::Submitted | OrderStatus::Accepted),
            "Expiry candidate is not an unresolved active intent"
        );
        Ok(Self {
            client_order_id: entry.command.client_order_id,
            command_keccak256: OrderBinding::new(entry)?.command_keccak256,
            coverage,
        })
    }
}

impl Journal {
    /// Reads one order, identity and any no-fill proof in the same transaction.
    ///
    /// # Errors
    ///
    /// Returns an error for unavailable original storage, invalid IDs or conflicting proof.
    pub fn order_evidence_snapshot(
        &self,
        client_id: ClientId,
        client_order_id: &str,
    ) -> Result<JournalOrderEvidence> {
        self.storage.ensure_path_current()?;
        let tx = self.storage.db.begin_read()?;
        let observation = self.order_evidence_in_read(&tx, client_id, client_order_id)?;
        self.storage.ensure_path_current()?;
        Ok(observation)
    }

    pub(super) fn order_evidence_in_read(
        &self,
        tx: &ReadTransaction,
        client_id: ClientId,
        client_order_id: &str,
    ) -> Result<JournalOrderEvidence> {
        ensure!(
            !client_order_id.is_empty()
                && client_order_id.len() <= 512
                && client_order_id.is_ascii()
                && !client_order_id.bytes().any(|byte| byte.is_ascii_control()),
            "Invalid client order ID"
        );
        let journal = self.storage.identity_in_read(tx)?;
        let table = tx.open_table(ORDERS)?;
        let entry: Option<JournalEntry> = table
            .get(client_order_id)?
            .map(|row| serde_json::from_str(row.value()))
            .transpose()?;
        let proof: Option<NoFillProof> = match tx.open_table(PROOFS) {
            Ok(proofs) => proofs
                .get(client_order_id)?
                .map(|row| serde_json::from_str(row.value()))
                .transpose()?,
            Err(TableError::TableDoesNotExist(_)) => None,
            Err(error) => return Err(error.into()),
        };
        let observation = if let Some(entry) = entry {
            ensure!(
                entry.command.client_order_id.as_str() == client_order_id,
                "Journal order key differs from its command"
            );
            ensure!(
                entry
                    .command
                    .client_id
                    .is_none_or(|route| route == client_id),
                "Journal order belongs to another execution client"
            );
            let binding = OrderBinding::new(&entry)?;
            if let Some(proof) = proof {
                ensure!(
                    proof.schema_version == 1
                        && proof.journal_id == journal.journal_id
                        && proof.command_keccak256 == binding.command_keccak256
                        && proof.sealed_at_revision > 0
                        && proof.sealed_at_revision <= journal.revision
                        && entry.fills.is_empty()
                        && !entry.execution_succeeded,
                    "No-fill proof conflicts with journal evidence"
                );
                match &proof.source {
                    NoFillSource::NeverSubmitted {} => ensure!(
                        entry.status == OrderStatus::Rejected && !entry.send_attempted,
                        "Unsent proof no longer names a sealed order"
                    ),
                    NoFillSource::ExpiredWithCoverage { coverage } => {
                        ensure!(
                            entry.status == OrderStatus::Expired,
                            "Expiry proof no longer names an expired intent"
                        );
                        let witnesses = tx.open_table(EXPIRY_WITNESSES)?;
                        let witness = witnesses.get(client_order_id)?.ok_or_else(|| {
                            anyhow::anyhow!("Expiry proof account witness is missing")
                        })?;
                        coverage.validate_snapshot(&entry, witness.value(), &self.storage)?;
                    }
                }
                OrderObservation::ProvenNoFill { binding, proof }
            } else if !entry.fills.is_empty() {
                OrderObservation::Filled { binding }
            } else {
                OrderObservation::Unproven { binding }
            }
        } else {
            ensure!(proof.is_none(), "Order proof has no corresponding command");
            OrderObservation::Missing
        };
        Ok(JournalOrderEvidence {
            schema_version: 1,
            journal,
            client_id,
            account_id: self.storage.account.clone(),
            client_order_id: client_order_id.to_owned(),
            observation,
        })
    }

    pub(crate) fn seal_registered_unsent(&self) -> Result<()> {
        self.storage.ensure_path_current()?;
        let mut tx = self.storage.db.begin_write()?;
        tx.set_durability(Durability::Immediate)?;
        let mut sealed = Vec::new();
        {
            let orders = tx.open_table(ORDERS)?;
            for row in orders.iter()? {
                let (key, value) = row?;
                let mut entry: JournalEntry = serde_json::from_str(value.value())?;
                ensure!(
                    key.value() == entry.command.client_order_id.as_str(),
                    "Journal order key differs from its command"
                );
                if entry.send_attempted
                    || !matches!(entry.status, OrderStatus::Submitted | OrderStatus::Accepted)
                {
                    continue;
                }
                ensure!(
                    entry.fills.is_empty() && !entry.execution_succeeded,
                    "Unsent intent has observed execution"
                );
                entry.status = OrderStatus::Rejected;
                sealed.push((key.value().to_owned(), entry));
            }
        }
        if sealed.is_empty() {
            return Ok(());
        }
        let revision = JournalStorage::next_revision(&tx)?;
        {
            let mut orders = tx.open_table(ORDERS)?;
            let mut proofs = tx.open_table(PROOFS)?;
            for (key, entry) in sealed {
                let proof = NoFillProof {
                    schema_version: 1,
                    journal_id: self.storage.journal_id,
                    command_keccak256: OrderBinding::new(&entry)?.command_keccak256,
                    sealed_at_revision: revision,
                    source: NoFillSource::NeverSubmitted {},
                };
                ensure!(
                    proofs.get(key.as_str())?.is_none(),
                    "Active order contradicts an existing proof"
                );
                orders.insert(key.as_str(), serde_json::to_string(&entry)?.as_str())?;
                proofs.insert(key.as_str(), serde_json::to_string(&proof)?.as_str())?;
            }
        }
        self.storage.commit(tx)
    }

    #[cfg(test)]
    pub(crate) fn checkpoint_with_expiry(
        &self,
        snapshot: &AccountSnapshot,
        candidates: &[ExpiryCandidate],
    ) -> Result<()> {
        self.checkpoint_with_context(snapshot, candidates, None)
            .map(|_| ())
    }

    pub(super) fn checkpoint_with_context(
        &self,
        snapshot: &AccountSnapshot,
        candidates: &[ExpiryCandidate],
        context: Option<super::account_observation::AccountContext<'_>>,
    ) -> Result<Option<super::account_observation::AccountCheckpointStamp>> {
        if candidates.is_empty() && context.is_none() {
            return self.checkpoint(snapshot).map(|()| None);
        }
        self.storage.ensure_path_current()?;
        self.storage.require_wallet(snapshot.address)?;
        snapshot.validate(snapshot.address)?;
        let snapshot_json = serde_json::to_string(snapshot)?;
        let snapshot_digest = keccak256(snapshot_json.as_bytes());
        let mut tx = self.storage.db.begin_write()?;
        tx.set_durability(Durability::Immediate)?;
        let revision = JournalStorage::next_revision(&tx)?;
        {
            let mut orders = tx.open_table(ORDERS)?;
            let mut proofs = tx.open_table(PROOFS)?;
            let mut witnesses = tx.open_table(EXPIRY_WITNESSES)?;
            for candidate in candidates {
                let key = candidate.client_order_id.as_str();
                let mut current: JournalEntry = serde_json::from_str(
                    orders
                        .get(key)?
                        .ok_or_else(|| anyhow::anyhow!("Expiry candidate disappeared"))?
                        .value(),
                )?;
                ensure!(
                    current.command.client_order_id == candidate.client_order_id
                        && matches!(
                            current.status,
                            OrderStatus::Submitted | OrderStatus::Accepted
                        )
                        && current.fills.is_empty()
                        && !current.execution_succeeded
                        && OrderBinding::new(&current)?.command_keccak256
                            == candidate.command_keccak256
                        && candidate.coverage.account_snapshot_keccak256 == snapshot_digest
                        && candidate.coverage.account_block == snapshot.indexed_through_block
                        && candidate.coverage.account_timestamp
                            == snapshot.indexed_through_timestamp
                        && candidate.coverage.account_revision == snapshot.account_revision
                        && proofs.get(key)?.is_none()
                        && witnesses.get(key)?.is_none(),
                    "Expiry candidate conflicts with current journal evidence"
                );
                candidate
                    .coverage
                    .validate_snapshot(&current, &snapshot_json, &self.storage)?;
                current.status = OrderStatus::Expired;
                let proof = NoFillProof {
                    schema_version: 1,
                    journal_id: self.storage.journal_id,
                    command_keccak256: candidate.command_keccak256,
                    sealed_at_revision: revision,
                    source: NoFillSource::ExpiredWithCoverage {
                        coverage: candidate.coverage.clone(),
                    },
                };
                orders.insert(key, serde_json::to_string(&current)?.as_str())?;
                proofs.insert(key, serde_json::to_string(&proof)?.as_str())?;
                witnesses.insert(key, snapshot_json.as_str())?;
            }
        }
        let stamp = context
            .map(|context| self.write_account_context(&tx, &snapshot_json, context, revision))
            .transpose()?;
        {
            let mut meta = tx.open_table(super::META)?;
            meta.insert("wallet_snapshot", snapshot_json.as_str())?;
        }
        self.storage.commit(tx)?;
        Ok(stamp)
    }
}
