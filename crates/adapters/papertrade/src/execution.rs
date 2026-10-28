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

//! Native execution client with durable intent identity and evidence-based reconciliation.

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use alloy::signers::local::PrivateKeySigner;
use alloy_primitives::{Address, U256};
use anyhow::{Result, ensure};
use async_trait::async_trait;
use nautilus_common::{
    clients::{ExecutionClient, ExecutionJournalIdentityProbe},
    live::{get_runtime, runner::get_exec_event_sender},
    messages::execution::{
        BatchCancelOrders, CancelAllOrders, CancelOrder, GenerateFillReports,
        GenerateOrderStatusReport, GenerateOrderStatusReports, GeneratePositionStatusReports,
        ModifyOrder, QueryAccount, QueryOrder, SubmitOrder, SubmitOrderList,
    },
};
use nautilus_core::{
    Params, UnixNanos,
    time::{AtomicTime, get_atomic_clock_realtime},
};
use nautilus_live::{ExecutionClientCore, ExecutionEventEmitter};
use nautilus_model::{
    accounts::AccountAny,
    enums::{OmsType, OrderSide, OrderStatus, OrderType, TimeInForce},
    identifiers::{AccountId, ClientId, InstrumentId, Venue, VenueOrderId},
    orders::{Order, OrderAny},
    reports::{ExecutionMassStatus, FillReport, OrderStatusReport, PositionStatusReport},
    types::{AccountBalance, MarginBalance},
};
use parking_lot::RwLock;
use tokio_util::sync::CancellationToken;

use crate::{
    api::{PapertradeApi, WalletActivity},
    config::{PAPERTRADE, PapertradeExecutionClientConfig, PapertradeJournalOpenMode},
    http::PapertradeRelayerClient,
    journal::Journal,
    reports,
    signing::{CancelIntent, Close, OpenPosition, PapertradeSigner, margin_to_raw},
    wire::{AccountSnapshot, Raw, TradingState},
};

#[derive(Debug, Clone, Default)]
pub(crate) struct ReportBundle {
    pub orders: Vec<OrderStatusReport>,
    pub fills: Vec<FillReport>,
    pub positions: Vec<PositionStatusReport>,
    pub complete: bool,
}
#[derive(Debug)]
pub(crate) struct Inner {
    pub sent_orders: parking_lot::Mutex<std::collections::HashMap<VenueOrderId, String>>,
    pub normalized:
        parking_lot::Mutex<std::collections::HashSet<nautilus_model::identifiers::ClientOrderId>>,
    pub config: PapertradeExecutionClientConfig,
    pub wallet: Address,
    pub signer: Option<PapertradeSigner>,
    pub api: PapertradeApi,
    pub relayer: PapertradeRelayerClient,
    pub journal: Arc<Journal>,
    pub snapshot: RwLock<Option<AccountSnapshot>>,
    pub trading: RwLock<Option<TradingState>>,
    pub bundle: RwLock<Option<ReportBundle>>,
    pub live: AtomicBool,
    pub stream_healthy: AtomicBool,
    stream_activity: RwLock<Option<WalletActivity>>,
    pub sync: tokio::sync::Mutex<()>,
    pub emitter: ExecutionEventEmitter,
    pub clock: &'static AtomicTime,
}
mod admission;

#[derive(Debug)]
pub struct PapertradeExecutionClient {
    core: ExecutionClientCore,
    inner: Arc<Inner>,
    cancellation: CancellationToken,
    task: Option<tokio::task::JoinHandle<()>>,
    outbound: parking_lot::Mutex<Vec<tokio::task::JoinHandle<()>>>,
}
impl PapertradeExecutionClient {
    pub fn new(
        core: ExecutionClientCore,
        mut config: PapertradeExecutionClientConfig,
    ) -> Result<Self> {
        config.validate_runtime_controls()?;
        ensure!(
            config.leverage > 0 && config.leverage <= 1_000 && config.poll_interval_ms >= 500,
            "Invalid execution configuration"
        );
        let wallet: Address = config.wallet_address.parse()?;
        let signer = if config.read_only {
            config.session_private_key = None;
            None
        } else {
            let key = config
                .session_private_key
                .clone()
                .or_else(|| {
                    std::env::var("PAPERTRADE_SESSION_PRIVATE_KEY")
                        .ok()
                        .map(Into::into)
                })
                .ok_or_else(|| {
                    anyhow::anyhow!("Set PAPERTRADE_SESSION_PRIVATE_KEY or session_private_key")
                })?;
            let signer: PrivateKeySigner = key
                .expose_secret()
                .parse()
                .map_err(|_| anyhow::anyhow!("Invalid session private key"))?;
            Some(PapertradeSigner::new(signer))
        };
        let clock = get_atomic_clock_realtime();
        let emitter = ExecutionEventEmitter::new(
            clock,
            core.trader_id,
            core.account_id,
            core.account_type,
            core.base_currency,
        );
        let journal = match &config.journal_open {
            PapertradeJournalOpenMode::Unregistered {} => {
                Journal::open(&config.journal_path, wallet, config.account_id.as_str())?
            }
            PapertradeJournalOpenMode::InitializeNew {} => {
                Journal::initialize_new(&config.journal_path, wallet, config.account_id.as_str())?
            }
            PapertradeJournalOpenMode::RecoverExisting {
                journal_id,
                minimum_revision,
            } => Journal::recover_existing(
                &config.journal_path,
                wallet,
                config.account_id.as_str(),
                *journal_id,
                *minimum_revision,
            )?,
        };
        if matches!(
            config.journal_open,
            PapertradeJournalOpenMode::RecoverExisting { .. }
        ) {
            journal.seal_registered_unsent()?;
        } else {
            journal.recover_unsent()?;
        }
        let checkpoint = journal.snapshot()?;
        if let Some(snapshot) = &checkpoint {
            snapshot.validate(wallet)?;
        }
        let inner = Inner {
            sent_orders: parking_lot::Mutex::new(std::collections::HashMap::new()),
            normalized: parking_lot::Mutex::new(std::collections::HashSet::new()),
            api: PapertradeApi::new(
                config.origin.parse()?,
                config.rpc_url.parse()?,
                config.max_history_pages,
            )?,
            relayer: PapertradeRelayerClient::with_origin(
                config.origin.parse()?,
                config.build_id.clone(),
            )?,
            config,
            wallet,
            signer,
            journal: Arc::new(journal),
            snapshot: RwLock::new(checkpoint),
            trading: RwLock::new(None),
            bundle: RwLock::new(None),
            live: AtomicBool::new(false),
            stream_healthy: AtomicBool::new(false),
            stream_activity: RwLock::new(None),
            sync: tokio::sync::Mutex::new(()),
            emitter,
            clock,
        };
        Ok(Self {
            core,
            inner: Arc::new(inner),
            cancellation: CancellationToken::new(),
            task: None,
            outbound: parking_lot::Mutex::new(Vec::new()),
        })
    }
    fn track(&self, task: tokio::task::JoinHandle<()>) {
        let mut tasks = self.outbound.lock();
        tasks.retain(|task| !task.is_finished());
        tasks.push(task);
    }
    fn now(&self) -> u64 {
        self.inner.clock.get_time_ns().as_u64() / 1_000_000_000
    }
    fn bundle(&self) -> Result<ReportBundle> {
        ensure!(
            self.inner.available(),
            "Papertrade source coverage unavailable"
        );
        self.inner
            .bundle
            .read()
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Papertrade not reconciled"))
    }
    fn deny(&self, order: &OrderAny, error: &anyhow::Error) {
        self.inner
            .emitter
            .emit_order_denied(order, &error.to_string());
    }
    fn prepare_order(
        &self,
        cmd: &SubmitOrder,
        order: &OrderAny,
    ) -> Result<(crate::journal::JournalEntry, bool)> {
        ensure!(self.inner.available(), "Account stream unavailable");
        let account = self
            .inner
            .snapshot
            .read()
            .clone()
            .ok_or_else(|| anyhow::anyhow!("No account snapshot"))?;
        let trading = self
            .inner
            .trading
            .read()
            .clone()
            .ok_or_else(|| anyhow::anyhow!("No trading configuration"))?;
        let market = trading
            .instruments
            .iter()
            .find(|m| reports::instrument_id(m) == cmd.instrument_id)
            .ok_or_else(|| anyhow::anyhow!("Unknown Papertrade instrument"))?;
        ensure!(
            order.order_type() == OrderType::Market
                && order.time_in_force() == TimeInForce::Gtc
                && order.contingency_type().is_none()
                && !order.is_post_only(),
            "Papertrade supports independent MARKET GTC intents only"
        );
        let now = self.now();
        let deadline = now
            .checked_add(3_600)
            .ok_or_else(|| anyhow::anyhow!("Intent deadline overflow"))?;
        self.inner.check_execution_window(
            if order.is_reduce_only() {
                "closePositions"
            } else {
                "openPosition"
            },
            false,
            now,
            deadline,
        )?;
        let signer = self.inner.authorized_signer(&account, deadline, now)?;
        if order.is_reduce_only() {
            ensure!(!order.is_quote_quantity(), "Close requires base quantity");
            let position_id = cmd
                .params
                .as_ref()
                .and_then(|p| p.get("papertrade_position_id"))
                .and_then(|v| v.as_str())
                .map(str::to_owned)
                .or_else(|| {
                    cmd.position_id
                        .map(|id| id.to_string().trim_start_matches("PT-").to_owned())
                })
                .ok_or_else(|| {
                    anyhow::anyhow!("Close requires papertrade_position_id or venue position_id")
                })?;
            let raw = Raw(U256::from_str_radix(&position_id, 10)?);
            let position = account
                .open_positions
                .iter()
                .find(|p| p.position_id == raw)
                .ok_or_else(|| anyhow::anyhow!("Close target is absent from current snapshot"))?;
            ensure!(
                position.instrument_id == market.id
                    && position.is_long == (order.order_side() == OrderSide::Sell)
                    && order.quantity() == reports::quantity(position, market)?,
                "Close must match the complete venue position"
            );
            ensure!(
                trading.action_enabled("closePositions"),
                "Close intake paused"
            );
            ensure!(
                !self
                    .inner
                    .journal
                    .entries()?
                    .iter()
                    .any(|e| e.target == Some(raw)
                        && matches!(e.status, OrderStatus::Submitted | OrderStatus::Accepted)
                        && e.command.client_order_id != cmd.client_order_id),
                "Another close is unresolved for this position"
            );
            self.inner
                .journal
                .prepare(cmd, Some(raw), trading.indexed_through_block, |nonce| {
                    signer.sign_close(
                        &Close {
                            user: self.inner.wallet,
                            positionIds: vec![raw.0],
                            nonce,
                            deadline,
                        },
                        now,
                    )
                })
        } else {
            ensure!(
                self.bundle()?.complete,
                "An unresolved prior intent must be reconciled before opening exposure"
            );
            ensure!(
                order.is_quote_quantity(),
                "Open requires quote_quantity=True (USD notional)"
            );
            ensure!(
                !trading.trading_paused
                    && market.active
                    && !market.paused
                    && trading.action_enabled("openPosition"),
                "Open intake paused"
            );
            ensure!(
                self.inner.config.leverage <= market.max_leverage
                    && (trading.maximum_leverage == 0
                        || self.inner.config.leverage <= trading.maximum_leverage)
                    && self.inner.config.leverage >= trading.minimum_leverage,
                "Leverage outside current venue limits"
            );
            let notional = order.quantity().as_decimal();
            let margin = notional / rust_decimal::Decimal::from(self.inner.config.leverage);
            let size = margin_to_raw(margin)?;
            ensure!(
                rust_decimal::Decimal::from(self.inner.config.leverage) * margin == notional,
                "Notional cannot be represented as exact margin at this leverage"
            );
            ensure!(
                U256::from(size) >= trading.minimum_trade_size_raw.0
                    && notional >= trading.minimum_open_notional_raw.decimal()?
                    && notional <= market.max_position_notional.decimal()?,
                "Open violates margin or notional limits"
            );
            let oi = if order.order_side() == OrderSide::Buy {
                market.long_oi
            } else {
                market.short_oi
            };
            let cap = if order.order_side() == OrderSide::Buy {
                market.long_cap
            } else {
                market.short_cap
            };
            // OI counters are venue-native units, not dollar notional. The contract
            // validates the requested increment at execution; do not add USD to them.
            ensure!(oi.0 < cap.0, "No current OI headroom");
            let pending = self
                .inner
                .journal
                .entries()?
                .into_iter()
                .filter(|e| {
                    e.action == "openPosition"
                        && matches!(e.status, OrderStatus::Submitted | OrderStatus::Accepted)
                        && !account
                            .operations
                            .iter()
                            .any(|o| o.intent_id == e.intent_id)
                        && !account
                            .open_positions
                            .iter()
                            .any(|p| p.intent_id == Some(e.intent_id))
                })
                .try_fold(rust_decimal::Decimal::ZERO, |sum, e| {
                    let amount = e.body["signed"]["size"]
                        .as_str()
                        .ok_or_else(|| anyhow::anyhow!("Corrupt reserved margin"))?;
                    Ok::<_, anyhow::Error>(sum + crate::wire::scaled_decimal(amount, 18)?)
                })?;
            ensure!(
                margin + pending <= account.funds.available_balance.decimal()?,
                "Insufficient available cash after local reservations"
            );
            self.inner
                .journal
                .prepare(cmd, None, trading.indexed_through_block, |nonce| {
                    signer.sign_open(
                        &OpenPosition {
                            user: self.inner.wallet,
                            instrumentId: market.id,
                            isLong: order.order_side() == OrderSide::Buy,
                            size,
                            leverage: self.inner.config.leverage,
                            useDebt: false,
                            nonce,
                            deadline,
                        },
                        now,
                    )
                })
        }
    }
}

#[async_trait(?Send)]
impl ExecutionClient for PapertradeExecutionClient {
    fn journal_identity_probe(&self) -> Option<Arc<dyn ExecutionJournalIdentityProbe>> {
        Some(self.inner.journal.identity_probe(self.core.client_id))
    }

    fn is_connected(&self) -> bool {
        self.core.is_connected() && self.inner.available()
    }
    fn client_id(&self) -> ClientId {
        self.core.client_id
    }
    fn account_id(&self) -> AccountId {
        self.core.account_id
    }
    fn venue(&self) -> Venue {
        Venue::from(PAPERTRADE)
    }
    fn oms_type(&self) -> OmsType {
        OmsType::Hedging
    }
    fn get_account(&self) -> Option<AccountAny> {
        self.core.cache().account_owned(&self.core.account_id)
    }
    fn provides_bulk_position_coverage(&self, _id: InstrumentId) -> bool {
        self.inner.available()
            && self
                .inner
                .bundle
                .read()
                .as_ref()
                .is_some_and(|b| b.complete)
    }
    fn generate_account_state(
        &self,
        balances: Vec<AccountBalance>,
        margins: Vec<MarginBalance>,
        reported: bool,
        ts: UnixNanos,
        info: Option<Params>,
    ) -> Result<()> {
        self.inner
            .emitter
            .try_emit_account_state(balances, margins, reported, ts, info)
    }
    fn start(&mut self) -> Result<()> {
        let mut emitter = self.inner.emitter.clone();
        emitter.set_sender(get_exec_event_sender());
        self.core.set_started();
        self.cancellation = CancellationToken::new();
        Ok(())
    }
    fn stop(&mut self) -> Result<()> {
        self.cancellation.cancel();
        self.inner.stream_healthy.store(false, Ordering::Release);
        self.inner.live.store(false, Ordering::Release);
        if let Some(task) = &self.task {
            task.abort();
        }
        for task in self.outbound.lock().iter() {
            task.abort();
        }
        self.core.set_disconnected();
        self.core.set_stopped();
        Ok(())
    }
    async fn connect(&mut self) -> Result<()> {
        if let Some(task) = self.task.take() {
            task.abort();
            let _ = task.await;
        }
        self.inner.stream_healthy.store(false, Ordering::Release);
        self.inner.live.store(false, Ordering::Release);
        let mut source_epoch = nautilus_core::UUID4::new();
        let mut stream = self.inner.api.wallet_stream(self.inner.wallet).await?;
        let snapshot = stream.next().await?;
        *self.inner.stream_activity.write() = Some(stream.activity.clone());
        crate::reconciliation::synchronize_with_epoch(&self.inner, snapshot.clone(), source_epoch)
            .await?;
        self.inner.stream_healthy.store(true, Ordering::Release);
        self.core.set_connected();
        let inner = self.inner.clone();
        let cancel = self.cancellation.clone();
        self.task = Some(get_runtime().spawn(async move {
            let mut latest = snapshot;
            let mut interval = tokio::time::interval(Duration::from_millis(inner.config.poll_interval_ms));
            'connected: loop {
                // Preserve the pending read and its idle deadline across poll ticks.
                let result = {
                    let incoming = stream.next();
                    tokio::pin!(incoming);
                    loop {
                        tokio::select! {
                            () = cancel.cancelled() => break 'connected,
                            result = &mut incoming => break result,
                            _ = interval.tick() => {
                                if let Err(e) = crate::reconciliation::synchronize_with_epoch(&inner, latest.clone(), source_epoch).await {
                                    inner.live.store(false, Ordering::Release);
                                    log::error!("papertrade_reconciliation_unavailable error={e}");
                                }
                            }
                        }
                    }
                };
                match result {
                    Ok(snapshot) => latest = snapshot,
                    Err(e) => {
                        inner.stream_healthy.store(false, Ordering::Release);
                        inner.live.store(false, Ordering::Release);
                        log::warn!("papertrade_wallet_reconnecting error={e}");
                        loop {
                            tokio::select! {
                                () = cancel.cancelled() => break 'connected,
                                () = tokio::time::sleep(Duration::from_secs(1)) => {}
                            }
                            let replacement = tokio::select! {
                                () = cancel.cancelled() => break 'connected,
                                result = async {
                                    let source_epoch = nautilus_core::UUID4::new();
                                    let mut stream = inner.api.wallet_stream(inner.wallet).await?;
                                    let snapshot = stream.next().await?;
                                    Ok::<_, anyhow::Error>((stream, snapshot, source_epoch))
                                } => result,
                            };
                            match replacement {
                                Ok((replacement, snapshot, replacement_epoch)) => {
                                    *inner.stream_activity.write() = Some(replacement.activity.clone());
                                    stream = replacement;
                                    latest = snapshot;
                                    source_epoch = replacement_epoch;
                                    break;
                                }
                                Err(e) => log::warn!("papertrade_wallet_connect_failed error={e}"),
                            }
                        }
                    }
                }
                inner.stream_healthy.store(true, Ordering::Release);
                match crate::reconciliation::synchronize_with_epoch(&inner, latest.clone(), source_epoch).await {
                    Ok(()) => {},
                    Err(e) => {
                        inner.live.store(false, Ordering::Release);
                        log::error!("papertrade_reconciliation_unavailable error={e}");
                    }
                }
            }
            inner.stream_healthy.store(false, Ordering::Release);
            inner.live.store(false, Ordering::Release);
        }));
        Ok(())
    }
    async fn disconnect(&mut self) -> Result<()> {
        self.stop()?;
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
        let tasks = std::mem::take(&mut *self.outbound.lock());
        for task in tasks {
            let _ = task.await;
        }
        Ok(())
    }
    fn submit_order(&self, cmd: SubmitOrder) -> Result<()> {
        if let Some(old) = self
            .inner
            .journal
            .entries()?
            .iter()
            .find(|e| e.command.client_order_id == cmd.client_order_id)
        {
            ensure!(
                old.command.order_init == cmd.order_init
                    && old.command.position_id == cmd.position_id
                    && old.command.params == cmd.params,
                "Client order ID payload conflict"
            );
            return Ok(());
        }
        let order = self.core.get_order(&cmd.client_order_id)?;
        let prepared = match self.prepare_order(&cmd, &order) {
            Ok(prepared) => prepared,
            Err(e) => {
                self.deny(&order, &e);
                return Ok(());
            }
        };
        let (entry, new) = prepared;
        if !new {
            return Ok(());
        }
        self.inner.emitter.emit_order_submitted(&order);
        let inner = self.inner.clone();
        let task = get_runtime().spawn(async move {
            let result = inner.submit_prepared(entry.command.client_order_id).await;
            if let Err(e) = result {
                log::error!(
                    "papertrade_submit_processing_failed client_order_id={} error={e}",
                    entry.command.client_order_id
                );
            }
        });
        self.track(task);
        Ok(())
    }
    fn cancel_order(&self, cmd: CancelOrder) -> Result<()> {
        self.core.get_order(&cmd.client_order_id)?;
        let entry = self
            .inner
            .journal
            .entries()?
            .into_iter()
            .find(|e| e.command.client_order_id == cmd.client_order_id)
            .ok_or_else(|| anyhow::anyhow!("Cancel target is not in journal"))?;
        let account = self
            .inner
            .snapshot
            .read()
            .clone()
            .ok_or_else(|| anyhow::anyhow!("No current account"))?;
        ensure!(
            matches!(entry.status, OrderStatus::Submitted | OrderStatus::Accepted),
            "Order is already terminal"
        );
        let now = self.now();
        let deadline = now
            .checked_add(3_600)
            .ok_or_else(|| anyhow::anyhow!("Cancel deadline overflow"))?;
        self.inner
            .check_execution_window(&entry.action, true, now, deadline)?;
        let signer = self.inner.authorized_signer(&account, deadline, now)?;
        let cancel = signer.sign_cancel(
            &CancelIntent {
                user: self.inner.wallet,
                intentId: entry.intent_id,
                deadline: U256::from(deadline),
            },
            &entry.signed()?,
            now,
        )?;
        let inner = self.inner.clone();
        let task = get_runtime().spawn(async move {
            if let Err(error) = inner.cancel_prepared(&cancel, entry, now).await {
                log::error!("papertrade_cancel_submission_failed error={error}");
            }
        });
        self.track(task);
        Ok(())
    }
    fn submit_order_list(&self, cmd: SubmitOrderList) -> Result<()> {
        for order in self.core.get_orders_for_list(&cmd.order_list)? {
            self.inner
                .emitter
                .emit_order_denied(&order, "Papertrade has no contingent order lists");
        }
        Ok(())
    }
    fn modify_order(&self, cmd: ModifyOrder) -> Result<()> {
        let order = self.core.get_order(&cmd.client_order_id)?;
        self.inner.emitter.emit_order_modify_rejected(
            &order,
            cmd.venue_order_id,
            "Papertrade has no modify operation",
            self.inner.clock.get_time_ns(),
        );
        Ok(())
    }
    fn cancel_all_orders(&self, cmd: CancelAllOrders) -> Result<()> {
        let entries = self.inner.journal.entries()?;
        for entry in entries.into_iter().filter(|e| {
            e.command.instrument_id == cmd.instrument_id
                && e.command.strategy_id == cmd.strategy_id
                && cmd
                    .order_side
                    .is_none_or(|side| side == e.command.order_init.order_side)
                && matches!(e.status, OrderStatus::Submitted | OrderStatus::Accepted)
        }) {
            self.cancel_order(CancelOrder::new(
                cmd.trader_id,
                cmd.client_id,
                cmd.strategy_id,
                cmd.instrument_id,
                entry.command.client_order_id,
                Some(VenueOrderId::from(format!("{:#x}", entry.intent_id))),
                nautilus_core::UUID4::new(),
                cmd.ts_init,
                cmd.params.clone(),
                cmd.correlation_id,
            ))?;
        }
        Ok(())
    }
    fn batch_cancel_orders(&self, cmd: BatchCancelOrders) -> Result<()> {
        for cancel in cmd.cancels {
            self.cancel_order(cancel)?;
        }
        Ok(())
    }
    fn query_account(&self, _cmd: QueryAccount) -> Result<()> {
        let source_epoch = nautilus_core::UUID4::new();
        let inner = self.inner.clone();
        let task = get_runtime().spawn(async move {
            let result = async {
                let snapshot = inner.api.snapshot(inner.wallet).await?;
                crate::reconciliation::synchronize_with_epoch(&inner, snapshot, source_epoch).await
            }
            .await;
            if let Err(e) = result {
                inner.live.store(false, Ordering::Release);
                log::error!("papertrade_account_query_failed error={e}");
            }
        });
        self.track(task);
        Ok(())
    }
    fn query_order(&self, cmd: QueryOrder) -> Result<()> {
        self.query_account(QueryAccount::new(
            cmd.trader_id,
            cmd.client_id,
            self.core.account_id,
            cmd.command_id,
            cmd.ts_init,
            cmd.params,
            cmd.correlation_id,
        ))
    }
    async fn generate_order_status_report(
        &self,
        cmd: &GenerateOrderStatusReport,
    ) -> Result<Option<OrderStatusReport>> {
        Ok(self.bundle()?.orders.into_iter().find(|r| {
            cmd.venue_order_id.is_some_and(|id| id == r.venue_order_id)
                || cmd
                    .client_order_id
                    .is_some_and(|id| Some(id) == r.client_order_id)
        }))
    }
    async fn generate_order_status_reports(
        &self,
        cmd: &GenerateOrderStatusReports,
    ) -> Result<Vec<OrderStatusReport>> {
        let bundle = self.bundle()?;
        ensure!(
            bundle.complete,
            "Open order coverage includes unresolved venue intents"
        );
        Ok(bundle
            .orders
            .into_iter()
            .filter(|r| {
                cmd.instrument_id.is_none_or(|id| id == r.instrument_id)
                    && cmd.start.is_none_or(|t| r.ts_last >= t)
                    && cmd.end.is_none_or(|t| r.ts_last <= t)
                    && (!cmd.open_only
                        || !matches!(
                            r.order_status,
                            OrderStatus::Filled
                                | OrderStatus::Canceled
                                | OrderStatus::Rejected
                                | OrderStatus::Expired
                        ))
            })
            .collect())
    }
    async fn generate_fill_reports(&self, cmd: GenerateFillReports) -> Result<Vec<FillReport>> {
        Ok(self
            .bundle()?
            .fills
            .into_iter()
            .filter(|r| {
                cmd.instrument_id.is_none_or(|id| id == r.instrument_id)
                    && cmd.start.is_none_or(|t| r.ts_event >= t)
                    && cmd.end.is_none_or(|t| r.ts_event <= t)
            })
            .collect())
    }
    async fn generate_position_status_reports(
        &self,
        cmd: &GeneratePositionStatusReports,
    ) -> Result<Vec<PositionStatusReport>> {
        Ok(self
            .bundle()?
            .positions
            .into_iter()
            .filter(|r| cmd.instrument_id.is_none_or(|id| id == r.instrument_id))
            .collect())
    }
    async fn generate_mass_status(
        &self,
        _lookback: Option<u64>,
    ) -> Result<Option<ExecutionMassStatus>> {
        let bundle = self.bundle()?;
        let mut report = ExecutionMassStatus::new(
            self.client_id(),
            self.account_id(),
            self.venue(),
            self.inner.clock.get_time_ns(),
            None,
        );
        report.add_order_reports(bundle.orders);
        report.add_fill_reports(bundle.fills);
        report.add_position_reports(bundle.positions);
        report.set_report_window(None, bundle.complete);
        Ok(Some(report))
    }
}

#[cfg(test)]
#[path = "execution_tests.rs"]
pub(crate) mod tests;

#[cfg(test)]
#[path = "execution_window_tests.rs"]
mod window_tests;
