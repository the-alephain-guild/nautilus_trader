// Copyright (C) 2015-2026 Nautech Systems Pty Ltd. All rights reserved.
// Licensed under the GNU Lesser General Public License Version 3.0.
// https://www.gnu.org/licenses/lgpl-3.0.en.html

//! Source-coherent venue observations, without a readiness or economic-proof claim.

use alloy_primitives::{B256, U256, keccak256};
use anyhow::{Result, ensure};
use nautilus_core::UUID4;
use nautilus_model::identifiers::{ClientId, InstrumentId};
use redb::{ReadTransaction, ReadableDatabase, ReadableTable, WriteTransaction};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{Journal, JournalIdentitySnapshot, META, ORDERS, evidence::ExpiryCandidate};
use crate::{
    reports,
    wire::{AccountSnapshot, TradingState},
};

const CONTEXT_KEY: &str = "account_observation_context_v1";

#[derive(Clone, Copy)]
pub(super) struct AccountContext<'a> {
    pub trading: &'a TradingState,
    pub complete: bool,
    pub source_epoch: UUID4,
}

pub(crate) struct AccountCheckpointStamp {
    pub journal_id: UUID4,
    pub checkpoint_revision: u64,
    pub account_snapshot_keccak256: B256,
    pub account_material_keccak256: B256,
    pub source_epoch: UUID4,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredContext {
    schema_version: u8,
    checkpoint_revision: u64,
    source_epoch: UUID4,
    wallet_snapshot_keccak256: B256,
    account_material_keccak256: B256,
    account_revision: u64,
    source_block: u64,
    source_timestamp: u64,
    wallet_revision: u64,
    queue_revision: crate::wire::Raw,
    trading: TradingState,
    reconciliation_complete: bool,
}

#[derive(Serialize)]
pub(super) struct AccountObservation {
    schema_version: u8,
    journal: JournalIdentitySnapshot,
    checkpoint_revision: u64,
    source_epoch: UUID4,
    account_snapshot_keccak256: B256,
    account_material_keccak256: B256,
    account_id: String,
    client_id: ClientId,
    account_revision: u64,
    source_block: u64,
    source_timestamp: u64,
    wallet_revision: u64,
    trading_covered_block: u64,
    reconciliation_complete: bool,
    balance_raw: String,
    locked_raw: String,
    queued_raw: String,
    consumable_debt_raw: String,
    queue_revision_raw: String,
    queue_node_count: u64,
    cashflow_counters: CashflowCounters,
    positions: Vec<ObservedPosition>,
    venue_operation_intent_ids: Vec<B256>,
    pending_client_order_ids: Vec<String>,
}

#[derive(Serialize)]
struct CashflowCounters {
    deposited_raw: String,
    withdrawn_raw: String,
    rewards_claimed_raw: String,
}

#[derive(Serialize)]
struct ObservedPosition {
    position_id: String,
    instrument_id: InstrumentId,
    opening_intent_id: Option<B256>,
    is_long: bool,
    use_debt: bool,
    margin_raw: String,
    leverage: u32,
    entry_price_raw: String,
    price_scale_raw: String,
    opened_at_ms: u64,
}

fn counter(lifetime: &Value, field: &str) -> Result<String> {
    let text = lifetime[field]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Missing canonical account counter {field}"))?;
    ensure!(
        !text.is_empty() && text.len() <= 78,
        "Account counter exceeds its bound"
    );
    let raw: U256 = text.parse()?;
    ensure!(raw.to_string() == text, "Account counter is not canonical");
    Ok(text.to_owned())
}

fn account_material(snapshot: &AccountSnapshot) -> Result<B256> {
    let mut value = serde_json::to_value(snapshot)?;
    let fields = value
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("Account snapshot is not an object"))?;
    fields.remove("indexedThroughBlock");
    fields.remove("indexedThroughTimestamp");
    fields.remove("accountRevision");
    // Outcomes are transient transport evidence retained independently by the journal.
    fields.remove("intentOutcomes");
    value.sort_all_objects();
    Ok(keccak256(serde_json::to_vec(&value)?))
}

impl Journal {
    #[cfg(test)]
    pub(crate) fn checkpoint_with_observation(
        &self,
        snapshot: &AccountSnapshot,
        candidates: &[ExpiryCandidate],
        trading: &TradingState,
        complete: bool,
    ) -> Result<AccountCheckpointStamp> {
        self.checkpoint_with_observation_epoch(
            snapshot,
            candidates,
            trading,
            complete,
            self.storage.journal_id,
        )
    }

    pub(crate) fn checkpoint_with_observation_epoch(
        &self,
        snapshot: &AccountSnapshot,
        candidates: &[ExpiryCandidate],
        trading: &TradingState,
        complete: bool,
        source_epoch: UUID4,
    ) -> Result<AccountCheckpointStamp> {
        self.checkpoint_with_context(
            snapshot,
            candidates,
            Some(AccountContext {
                trading,
                complete,
                source_epoch,
            }),
        )?
        .ok_or_else(|| anyhow::anyhow!("Account checkpoint has no observation stamp"))
    }

    pub(super) fn write_account_context(
        &self,
        tx: &WriteTransaction,
        wallet_json: &str,
        context: AccountContext<'_>,
        revision: u64,
    ) -> Result<AccountCheckpointStamp> {
        context.trading.validate()?;
        let snapshot: AccountSnapshot = serde_json::from_str(wallet_json)?;
        let mut meta = tx.open_table(META)?;
        if let Some(previous) = meta.get(CONTEXT_KEY)? {
            let previous: StoredContext = serde_json::from_str(previous.value())?;
            let wallet = meta
                .get("wallet_snapshot")?
                .ok_or_else(|| anyhow::anyhow!("Prior account context has no snapshot"))?;
            let old: AccountSnapshot = serde_json::from_str(wallet.value())?;
            self.storage.require_wallet(old.address)?;
            old.validate(old.address)?;
            previous.validate_snapshot(wallet.value(), &old)?;
            ensure!(
                previous.checkpoint_revision < revision
                    && snapshot.indexed_through_block >= old.indexed_through_block
                    && snapshot.indexed_through_timestamp >= old.indexed_through_timestamp
                    && snapshot.wallet_revision >= old.wallet_revision
                    && snapshot.queue_revision >= old.queue_revision
                    && context.trading.indexed_through_block
                        >= previous.trading.indexed_through_block,
                "Account observation source coverage regressed"
            );
            if context.source_epoch == previous.source_epoch {
                ensure!(
                    snapshot.account_revision >= old.account_revision,
                    "Account revision regressed within its source epoch"
                );
                ensure!(
                    snapshot.account_revision != old.account_revision
                        || previous.wallet_snapshot_keccak256 == keccak256(wallet_json.as_bytes()),
                    "Account revision has conflicting source contents"
                );
            } else if snapshot.indexed_through_block == old.indexed_through_block
                && snapshot.indexed_through_timestamp == old.indexed_through_timestamp
                && snapshot.wallet_revision == old.wallet_revision
                && snapshot.queue_revision == old.queue_revision
            {
                ensure!(
                    previous.account_material_keccak256 == account_material(&snapshot)?,
                    "New source epoch conflicts at unchanged global coverage"
                );
            }
        }
        let stored = StoredContext {
            schema_version: 1,
            checkpoint_revision: revision,
            source_epoch: context.source_epoch,
            wallet_snapshot_keccak256: keccak256(wallet_json.as_bytes()),
            account_material_keccak256: account_material(&snapshot)?,
            account_revision: snapshot.account_revision,
            source_block: snapshot.indexed_through_block,
            source_timestamp: snapshot.indexed_through_timestamp,
            wallet_revision: snapshot.wallet_revision,
            queue_revision: snapshot.queue_revision,
            trading: context.trading.clone(),
            reconciliation_complete: context.complete,
        };
        meta.insert(CONTEXT_KEY, serde_json::to_string(&stored)?.as_str())?;
        Ok(AccountCheckpointStamp {
            journal_id: self.storage.journal_id,
            checkpoint_revision: revision,
            account_snapshot_keccak256: stored.wallet_snapshot_keccak256,
            account_material_keccak256: stored.account_material_keccak256,
            source_epoch: stored.source_epoch,
        })
    }

    pub(super) fn account_observation(&self, client_id: ClientId) -> Result<AccountObservation> {
        self.storage.ensure_path_current()?;
        let tx = self.storage.db.begin_read()?;
        let journal = self.storage.identity_in_read(&tx)?;
        let (snapshot, context) = self.read_account_context(&tx, &journal)?;
        ensure!(
            snapshot.open_positions.len() <= 64,
            "Too many positions for account observation"
        );
        let mut positions = Vec::new();
        for position in &snapshot.open_positions {
            let market = context
                .trading
                .instruments
                .iter()
                .find(|market| market.id == position.instrument_id)
                .ok_or_else(|| anyhow::anyhow!("Account position has no checkpoint market"))?;
            let scale = market.price_scale.to_string();
            ensure!(
                scale.starts_with('1')
                    && scale.len() <= 19
                    && scale[1..].bytes().all(|byte| byte == b'0'),
                "Invalid checkpoint price scale"
            );
            positions.push(ObservedPosition {
                position_id: reports::position_id(position).to_string(),
                instrument_id: reports::instrument_id(market),
                opening_intent_id: position.intent_id,
                is_long: position.is_long,
                use_debt: position.use_debt,
                margin_raw: position.margin.to_string(),
                leverage: position.leverage,
                entry_price_raw: position.entry_price.to_string(),
                price_scale_raw: scale,
                opened_at_ms: position.opened_at_ms,
            });
        }
        positions.sort_by(|left, right| left.position_id.cmp(&right.position_id));
        let mut pending_client_order_ids = Vec::new();
        for row in tx.open_table(ORDERS)?.iter()? {
            let (key, _) = row?;
            if self
                .order_evidence_in_read(&tx, client_id, key.value())?
                .is_unresolved()
            {
                ensure!(
                    pending_client_order_ids.len() < 256,
                    "Too many unresolved journal intents"
                );
                pending_client_order_ids.push(key.value().to_owned());
            }
        }
        let observed = AccountObservation {
            schema_version: 1,
            journal,
            checkpoint_revision: context.checkpoint_revision,
            source_epoch: context.source_epoch,
            account_snapshot_keccak256: context.wallet_snapshot_keccak256,
            account_material_keccak256: context.account_material_keccak256,
            account_id: self.storage.account.clone(),
            client_id,
            account_revision: snapshot.account_revision,
            source_block: snapshot.indexed_through_block,
            source_timestamp: snapshot.indexed_through_timestamp,
            wallet_revision: snapshot.wallet_revision,
            trading_covered_block: context.trading.indexed_through_block,
            reconciliation_complete: context.reconciliation_complete,
            balance_raw: snapshot.balance.to_string(),
            locked_raw: snapshot.locked.to_string(),
            queued_raw: snapshot.queued.to_string(),
            consumable_debt_raw: snapshot.consumable_debt.to_string(),
            queue_revision_raw: snapshot.queue_revision.to_string(),
            queue_node_count: snapshot.queue_node_count,
            cashflow_counters: CashflowCounters {
                deposited_raw: counter(&snapshot.lifetime, "depositedRaw")?,
                withdrawn_raw: counter(&snapshot.lifetime, "withdrawnRaw")?,
                rewards_claimed_raw: counter(&snapshot.lifetime, "paperRewardsClaimedRaw")?,
            },
            positions,
            venue_operation_intent_ids: snapshot
                .operations
                .iter()
                .map(|operation| operation.intent_id)
                .collect(),
            pending_client_order_ids,
        };
        self.storage.ensure_path_current()?;
        Ok(observed)
    }

    fn read_account_context(
        &self,
        tx: &ReadTransaction,
        journal: &JournalIdentitySnapshot,
    ) -> Result<(AccountSnapshot, StoredContext)> {
        let meta = tx.open_table(META)?;
        let wallet = meta
            .get("wallet_snapshot")?
            .ok_or_else(|| anyhow::anyhow!("Original account checkpoint is missing"))?;
        let context = meta
            .get(CONTEXT_KEY)?
            .ok_or_else(|| anyhow::anyhow!("Original account checkpoint has no market context"))?;
        let context: StoredContext = serde_json::from_str(context.value())?;
        ensure!(
            context.schema_version == 1
                && context.checkpoint_revision > 0
                && context.checkpoint_revision <= journal.revision
                && context.wallet_snapshot_keccak256 == keccak256(wallet.value().as_bytes()),
            "Account checkpoint context differs from the original snapshot"
        );
        let snapshot: AccountSnapshot = serde_json::from_str(wallet.value())?;
        context.validate_snapshot(wallet.value(), &snapshot)?;
        self.storage.require_wallet(snapshot.address)?;
        snapshot.validate(snapshot.address)?;
        context.trading.validate()?;
        Ok((snapshot, context))
    }
}

impl StoredContext {
    fn validate_snapshot(&self, raw: &str, snapshot: &AccountSnapshot) -> Result<()> {
        ensure!(
            self.schema_version == 1
                && self.checkpoint_revision > 0
                && self.wallet_snapshot_keccak256 == keccak256(raw.as_bytes())
                && self.account_material_keccak256 == account_material(snapshot)?
                && self.account_revision == snapshot.account_revision
                && self.source_block == snapshot.indexed_through_block
                && self.source_timestamp == snapshot.indexed_through_timestamp
                && self.wallet_revision == snapshot.wallet_revision
                && self.queue_revision == snapshot.queue_revision,
            "Account context identity or source watermarks differ"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;
    use serde_json::json;

    fn source() -> (AccountSnapshot, TradingState) {
        let mut wallet: AccountSnapshot =
            serde_json::from_str(include_str!("../../test_data/live_wallet.json")).unwrap();
        wallet.indexed_through_timestamp += 10;
        wallet.account_revision += 10;
        wallet.queue_revision = crate::wire::Raw(U256::from(10));
        let trading =
            serde_json::from_str(include_str!("../../test_data/live_trading.json")).unwrap();
        (wallet, trading)
    }

    #[rstest]
    #[case("same_revision_content")]
    #[case("account_revision")]
    #[case("source_time")]
    #[case("source_block")]
    #[case("wallet_revision")]
    #[case("queue_revision")]
    #[case("trading_coverage")]
    fn account_context_refuses_regression_without_replacing_the_checkpoint(#[case] damage: &str) {
        let (snapshot, trading) = source();
        let dir = tempfile::tempdir().unwrap();
        let journal = Journal::initialize_new(
            &dir.path().join("journal.redb"),
            snapshot.address,
            "PAPERTRADE-001",
        )
        .unwrap();
        journal
            .checkpoint_with_observation(&snapshot, &[], &trading, true)
            .unwrap();
        let first = serde_json::to_value(
            journal
                .account_observation(ClientId::from("PAPERTRADE"))
                .unwrap(),
        )
        .unwrap();
        let mut changed = snapshot.clone();
        let mut changed_trading = trading;
        if damage != "same_revision_content" {
            changed.account_revision += 1;
        }
        match damage {
            "same_revision_content" => changed.lifetime["depositedRaw"] = json!("1"),
            "account_revision" => changed.account_revision = snapshot.account_revision - 1,
            "source_time" => changed.indexed_through_timestamp -= 1,
            "source_block" => {
                changed.indexed_through_block -= 1;
                changed.wallet_revision -= 1;
            }
            "wallet_revision" => changed.wallet_revision -= 1,
            "queue_revision" => changed.queue_revision = crate::wire::Raw(U256::from(9)),
            "trading_coverage" => changed_trading.indexed_through_block -= 1,
            _ => unreachable!(),
        }
        changed.validate(changed.address).unwrap();
        assert!(
            journal
                .checkpoint_with_observation(&changed, &[], &changed_trading, true)
                .is_err()
        );
        assert_eq!(
            serde_json::to_value(
                journal
                    .account_observation(ClientId::from("PAPERTRADE"))
                    .unwrap()
            )
            .unwrap(),
            first
        );
    }

    #[rstest]
    fn ordinary_checkpoint_cannot_reuse_an_older_market_association() {
        let (snapshot, trading) = source();
        let dir = tempfile::tempdir().unwrap();
        let journal = Journal::initialize_new(
            &dir.path().join("journal.redb"),
            snapshot.address,
            "PAPERTRADE-001",
        )
        .unwrap();
        journal
            .checkpoint_with_observation(&snapshot, &[], &trading, true)
            .unwrap();
        let mut changed = snapshot;
        changed.account_revision += 1;
        journal.checkpoint(&changed).unwrap();
        assert!(
            journal
                .account_observation(ClientId::from("PAPERTRADE"))
                .is_err()
        );
    }

    #[rstest]
    fn unchanged_account_revision_can_advance_global_coverage_and_reconciliation() {
        let (snapshot, mut trading) = source();
        let dir = tempfile::tempdir().unwrap();
        let journal = Journal::initialize_new(
            &dir.path().join("journal.redb"),
            snapshot.address,
            "PAPERTRADE-001",
        )
        .unwrap();
        journal
            .checkpoint_with_observation(&snapshot, &[], &trading, false)
            .unwrap();
        let first = serde_json::to_value(
            journal
                .account_observation(ClientId::from("PAPERTRADE"))
                .unwrap(),
        )
        .unwrap();
        let mut refreshed = snapshot;
        refreshed.indexed_through_block += 1;
        refreshed.indexed_through_timestamp += 1;
        trading.indexed_through_block += 1;
        journal
            .checkpoint_with_observation_epoch(&refreshed, &[], &trading, true, UUID4::new())
            .unwrap();
        let later = serde_json::to_value(
            journal
                .account_observation(ClientId::from("PAPERTRADE"))
                .unwrap(),
        )
        .unwrap();
        assert_eq!(first["account_revision"], later["account_revision"]);
        assert_ne!(
            first["account_snapshot_keccak256"],
            later["account_snapshot_keccak256"]
        );
        assert!(
            first["account_material_keccak256"]
                .as_str()
                .unwrap()
                .starts_with("0x")
        );
        assert_eq!(
            first["account_material_keccak256"],
            later["account_material_keccak256"]
        );
        assert_eq!(first["reconciliation_complete"], false);
        assert_eq!(later["reconciliation_complete"], true);
    }

    #[rstest]
    fn invalid_context_rolls_back_wallet_and_journal_revision_together() {
        let (snapshot, trading) = source();
        let dir = tempfile::tempdir().unwrap();
        let journal = Journal::initialize_new(
            &dir.path().join("journal.redb"),
            snapshot.address,
            "PAPERTRADE-001",
        )
        .unwrap();
        journal
            .checkpoint_with_observation(&snapshot, &[], &trading, true)
            .unwrap();
        let first = serde_json::to_value(
            journal
                .account_observation(ClientId::from("PAPERTRADE"))
                .unwrap(),
        )
        .unwrap();
        let mut changed = snapshot;
        changed.account_revision += 1;
        let mut invalid = trading;
        invalid.instruments.clear();
        assert!(
            journal
                .checkpoint_with_observation(&changed, &[], &invalid, true)
                .is_err()
        );
        assert_eq!(
            serde_json::to_value(
                journal
                    .account_observation(ClientId::from("PAPERTRADE"))
                    .unwrap()
            )
            .unwrap(),
            first
        );
    }

    #[rstest]
    fn a_fresh_source_epoch_can_reset_connection_local_account_revision() {
        let (snapshot, trading) = source();
        let dir = tempfile::tempdir().unwrap();
        let journal = Journal::initialize_new(
            &dir.path().join("journal.redb"),
            snapshot.address,
            "PAPERTRADE-001",
        )
        .unwrap();
        journal
            .checkpoint_with_observation(&snapshot, &[], &trading, true)
            .unwrap();
        let mut refreshed = snapshot;
        refreshed.account_revision = 1;
        journal
            .checkpoint_with_observation_epoch(
                &refreshed,
                &[],
                &trading,
                true,
                nautilus_core::UUID4::new(),
            )
            .unwrap();
        assert_eq!(
            journal
                .account_observation(ClientId::from("PAPERTRADE"))
                .unwrap()
                .account_revision,
            1
        );
    }

    #[rstest]
    fn new_epoch_without_global_progress_cannot_change_account_material() {
        let (snapshot, trading) = source();
        let dir = tempfile::tempdir().unwrap();
        let journal = Journal::initialize_new(
            &dir.path().join("journal.redb"),
            snapshot.address,
            "PAPERTRADE-001",
        )
        .unwrap();
        journal
            .checkpoint_with_observation(&snapshot, &[], &trading, true)
            .unwrap();
        let mut changed = snapshot;
        changed.account_revision = 1;
        changed.lifetime["depositedRaw"] = json!("1");
        assert!(
            journal
                .checkpoint_with_observation_epoch(
                    &changed,
                    &[],
                    &trading,
                    true,
                    nautilus_core::UUID4::new()
                )
                .is_err()
        );
    }

    #[rstest]
    fn new_epoch_can_omit_transient_outcomes_without_changing_cash_material() {
        let (mut snapshot, trading) = source();
        snapshot.intent_outcomes = vec![(B256::repeat_byte(0x11), "succeeded".to_owned())];
        let dir = tempfile::tempdir().unwrap();
        let journal = Journal::initialize_new(
            &dir.path().join("journal.redb"),
            snapshot.address,
            "PAPERTRADE-001",
        )
        .unwrap();
        journal
            .checkpoint_with_observation(&snapshot, &[], &trading, true)
            .unwrap();
        let first = journal
            .account_observation(ClientId::from("PAPERTRADE"))
            .unwrap();
        snapshot.intent_outcomes.clear();
        snapshot.account_revision = 1;
        journal
            .checkpoint_with_observation_epoch(&snapshot, &[], &trading, true, UUID4::new())
            .unwrap();
        let later = journal
            .account_observation(ClientId::from("PAPERTRADE"))
            .unwrap();
        assert_eq!(
            first.account_material_keccak256,
            later.account_material_keccak256
        );
        assert_ne!(
            first.account_snapshot_keccak256,
            later.account_snapshot_keccak256
        );
    }
}
