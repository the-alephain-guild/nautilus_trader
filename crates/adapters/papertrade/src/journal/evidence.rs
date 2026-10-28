// Copyright (C) 2015-2026 Nautech Systems Pty Ltd. All rights reserved.
// Licensed under the GNU Lesser General Public License Version 3.0.
// https://www.gnu.org/licenses/lgpl-3.0.en.html

//! Per-intent evidence read and sealed in the original persistent journal.

use alloy_primitives::{B256, keccak256};
use anyhow::{Result, ensure};
use nautilus_core::UUID4;
use nautilus_model::{enums::OrderStatus, identifiers::ClientId};
use redb::{Durability, ReadableDatabase, ReadableTable, TableDefinition, TableError};
use serde::{Deserialize, Serialize};

use super::{Journal, JournalEntry, JournalIdentitySnapshot, ORDERS, identity::JournalStorage};

const PROOFS: TableDefinition<&str, &str> = TableDefinition::new("order_no_fill_proofs");

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
        ensure!(
            !client_order_id.is_empty()
                && client_order_id.len() <= 512
                && client_order_id.is_ascii()
                && !client_order_id.bytes().any(|byte| byte.is_ascii_control()),
            "Invalid client order ID"
        );
        let tx = self.storage.db.begin_read()?;
        let journal = self.storage.identity_in_read(&tx)?;
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
                match proof.source {
                    NoFillSource::NeverSubmitted {} => ensure!(
                        entry.status == OrderStatus::Rejected && !entry.send_attempted,
                        "Unsent proof no longer names a sealed order"
                    ),
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
        self.storage.ensure_path_current()?;
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
}
