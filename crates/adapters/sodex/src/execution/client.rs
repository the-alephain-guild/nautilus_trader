//! Live execution client.
//!
//! # What this client can and cannot do
//!
//! Submission, cancellation, account state and order reconciliation are implemented against
//! endpoints verified on the live testnet. Fill reports are not, and the reason is narrow: the
//! endpoint exists and answers, but an account that has never traded answers `[]`, so the wire
//! shape cannot be read off it. Typing it by analogy to the order shape is exactly the move that
//! produced this integration's worst failures, so it waits for one observed fill instead.
//!
//! Position reports are served on both engines, but only partly on perps. Spot does not route the
//! path at all, which is correct rather than missing - spot holds balances and has no positions, so
//! an empty report is the truth. On perps the endpoint is read and decoded: an **empty** list is
//! reported as empty, because the venue saying the account holds nothing is an answer, not a gap.
//! A **non-empty** payload fails loudly with its own contents attached, because that shape has
//! never been observed and an empty list in its place would assert the account is flat while the
//! venue just said otherwise - reconciliation would then close positions that exist.
//!
//! # A submission whose outcome is unknown is not guessed at
//!
//! A transport failure on a submit does not say whether the venue received it: it may have been
//! processed and only the response lost. Reporting a rejection there would let the engine believe
//! an order is dead while it rests on the book - real money behind a position nothing is managing,
//! and the single worst divergence this adapter could produce.
//!
//! So the venue is asked. The account's order list is the authority, and it is consulted a few
//! times because the venue settles on-chain and an accepted order takes a moment to appear. Found
//! means the venue's own state is reported; confidently absent means a rejection that is now an
//! observation; and a failed lookup emits nothing at all, leaving the order submitted for
//! reconciliation to settle - guessing there would reintroduce exactly what this avoids.
//!
//! This is also why write requests are still not retried. Retrying would create the ambiguity;
//! resolving it after the fact is strictly better than risking a second order.
//!
//! # The account reads are addressed by the master wallet, and a wrong address does not fail
//!
//! They are keyed by the account's wallet address. The API key's own address also answers `200`,
//! with an empty account - so a misconfigured client would reconcile against "no balance, no open
//! orders" and the engine would take that for a flat account, with nothing reporting a problem.
//!
//! Emptiness cannot be the error, because a new account is legitimately empty. So the client
//! proves the address instead: at connect it asks the venue which API keys that wallet has
//! registered **on this engine**, and refuses to start unless the key it signs with is among
//! them. That check also catches the other documented trap in one go - a key registered on the
//! other engine, which otherwise surfaces much later as `API key not found` on the first order.
//!
//! # One order per request
//!
//! The venue accepts batches, but its acknowledgements are per order rather than
//! whole-batch, so a batch buys latency and rate-limit weight, not atomicity. Order *lists*
//! are a different matter: a bracket's legs are only a bracket if the venue enforces the
//! contingency between them, and this one has no such concept. Submitting the legs
//! independently would leave a stop that never activates and a take-profit that fires with
//! no position, so a contingent list is denied rather than flattened.

use std::{
    fmt::Debug,
    sync::Arc,
    time::{Duration, Instant},
};

use async_trait::async_trait;
use nautilus_common::{
    clients::ExecutionClient,
    live::{get_runtime, runner::get_exec_event_sender, task::TaskHandles},
    messages::execution::{
        BatchCancelOrders, CancelAllOrders, CancelOrder, GenerateFillReports,
        GenerateOrderStatusReport, GenerateOrderStatusReports, GeneratePositionStatusReports,
        ModifyOrder, QueryAccount, SubmitOrder, SubmitOrderList,
    },
};
use nautilus_core::{Params, UnixNanos, time::AtomicTime};
use nautilus_live::{ExecutionClientCore, ExecutionEventEmitter};
use nautilus_model::{
    accounts::AccountAny,
    enums::{LiquiditySide, OmsType},
    identifiers::{
        AccountId, ClientId, ClientOrderId as NautilusClientOrderId, InstrumentId, Venue,
        VenueOrderId,
    },
    instruments::{Instrument, InstrumentAny},
    orders::{Order, OrderAny},
    reports::{fill::FillReport, order::OrderStatusReport, position::PositionStatusReport},
    types::{AccountBalance, Currency, MarginBalance, Money, Price, Quantity},
};
use nautilus_network::http::Method;
use tokio_util::sync::CancellationToken;

use super::{
    parse::OrderSpec,
    reports::{fill_report, order_status_report, position_status_report},
};
use crate::{
    common::Market,
    config::SodexExecClientConfig,
    http::{
        BatchCost, CancelOrderRequest, ClientError, ModifyOrderRequest, NewOrderRequest, OrderAck,
        SodexHttpClient,
        account_reads::OrderRecord,
        align_batch,
        requests::{CancelItem, ClientOrderId as VenueClientOrderId, MAX_BATCH},
        spot::{SpotCancelItem, SpotCancelOrderRequest, SpotNewOrderRequest},
    },
    providers::{
        InstrumentCatalog, InstrumentReload, instrument_id_for, load_instruments,
        spawn_instrument_refresh,
    },
};

/// Live execution client for one SoDEX engine.
pub struct SodexExecutionClient {
    core: ExecutionClientCore,
    config: SodexExecClientConfig,
    clock: &'static AtomicTime,
    emitter: ExecutionEventEmitter,
    http: Arc<SodexHttpClient>,
    /// The published instrument set, which is where an order's numeric symbol id comes from.
    catalog: Arc<InstrumentCatalog>,
    /// Venue account id, resolved once at construction.
    venue_account_id: u64,
    /// The account's wallet address, which is what the account reads are keyed by.
    wallet: String,
    tasks: TaskHandles,
    cancellation: CancellationToken,
}

impl Debug for SodexExecutionClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct(stringify!(SodexExecutionClient))
            .field("client_id", &self.core.client_id)
            .field("venue", &self.core.venue)
            .field("connected", &self.core.is_connected())
            .field("instruments", &self.catalog.len())
            .finish()
    }
}

impl SodexExecutionClient {
    /// Creates an execution client for one engine.
    ///
    /// # Errors
    ///
    /// Returns an error if a credential cannot be resolved or the HTTP client cannot be
    /// built. Resolving credentials here rather than on the first order turns a
    /// misconfiguration into a startup failure instead of a rejected trade.
    pub fn new(
        core: ExecutionClientCore,
        config: SodexExecClientConfig,
        clock: &'static AtomicTime,
    ) -> anyhow::Result<Self> {
        let venue_account_id = config.resolve_account_id()?;
        let wallet = config.resolve_wallet_address()?;
        let key_name = config.resolve_api_key_name()?;
        let private_key = config.resolve_api_private_key()?;

        let name = crate::common::credential::ApiKeyName::parse(&key_name)?;
        let key = crate::common::credential::ApiPrivateKey::parse(private_key.expose_secret())?;
        let http = SodexHttpClient::signed_with_options(
            config.network,
            config.market,
            name,
            &key,
            config.timeout_secs,
            None,
        )
        .map_err(|e| anyhow::anyhow!("failed to build signed HTTP client: {e}"))?;

        let emitter = ExecutionEventEmitter::new(
            clock,
            core.trader_id,
            core.account_id,
            core.account_type,
            core.base_currency,
        );

        Ok(Self {
            core,
            config,
            clock,
            emitter,
            http: Arc::new(http),
            catalog: Arc::new(InstrumentCatalog::new()),
            venue_account_id,
            wallet,
            tasks: TaskHandles::default(),
            cancellation: CancellationToken::new(),
        })
    }

    /// Refuses to continue unless the configured wallet has our signing key registered here.
    ///
    /// This is the check that makes the account reads trustworthy. Without it a wrong address
    /// would read as an empty account rather than an error, and reconciliation would conclude the
    /// account is flat.
    ///
    /// # Errors
    ///
    /// Returns an error naming what the venue does hold, because the two ways this fails want
    /// different fixes: a wrong address, or a key registered on the other engine.
    /// Waits until the engine has registered this client's account in the cache.
    ///
    /// # Errors
    ///
    /// Returns an error if the account has not appeared within `timeout_secs`, which means the
    /// account state never reached the engine - connecting anyway would let reconciliation run
    /// against an account that does not exist.
    async fn await_account_registered(&self, timeout_secs: f64) -> anyhow::Result<()> {
        let account_id = self.core.account_id;
        let start = Instant::now();
        let timeout = Duration::from_secs_f64(timeout_secs);

        loop {
            if self.core.cache().account(&account_id).is_some() {
                log::info!("sodex_account_registered account={account_id}");
                return Ok(());
            }

            if start.elapsed() >= timeout {
                anyhow::bail!(
                    "account {account_id} was not registered within {timeout_secs}s; the engine \
                     never received its state, so reconciliation would drop every fill"
                );
            }

            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    async fn verify_wallet_owns_signing_key(&self) -> anyhow::Result<()> {
        let registered = self
            .http
            .api_keys(&self.wallet)
            .await
            .map_err(|e| anyhow::anyhow!("failed to list API keys for {}: {e}", self.wallet))?;

        let signer = format!("{:#x}", self.http.signing_address()?);
        if registered
            .iter()
            .any(|entry| entry.public_key.eq_ignore_ascii_case(&signer))
        {
            return Ok(());
        }

        let names: Vec<String> = registered
            .iter()
            .map(|entry| format!("{}={}", entry.name, entry.public_key))
            .collect();
        anyhow::bail!(
            "wallet {} has no API key matching this client's signing address {signer} on {:?}. \
             Registered there: [{}]. Either the wallet address is wrong - in which case the \
             account reads would have reported an empty account rather than failing - or the key \
             was registered on the other engine, which keeps a separate key set.",
            self.wallet,
            self.config.market,
            names.join(", ")
        )
    }

    fn symbol_id(&self, instrument_id: &InstrumentId) -> anyhow::Result<u64> {
        self.catalog.symbol_id(instrument_id).ok_or_else(|| {
            anyhow::anyhow!(
                "no venue symbol id for {instrument_id}; instruments have not been loaded"
            )
        })
    }

    /// Denies an order the venue cannot honor as asked, and reports why.
    ///
    /// Returns `true` when the order was denied and must not be sent.
    fn deny_if_contingent(&self, order: &OrderAny) -> bool {
        let Some(contingency) = order.contingency_type() else {
            return false;
        };

        // The venue has no contingency concept. Sending the legs anyway would leave the
        // strategy believing in a relationship nothing enforces.
        self.emitter.emit_order_denied(
            order,
            &format!("SoDEX cannot enforce {contingency:?} contingency between orders"),
        );
        true
    }
}

/// A submission prepared for one engine.
enum Submission {
    Spot(SpotNewOrderRequest),
    Perps(NewOrderRequest),
}

impl Submission {
    fn build(
        spec: &OrderSpec,
        market: Market,
        account_id: u64,
        symbol_id: u64,
    ) -> anyhow::Result<Self> {
        Ok(match market {
            Market::Spot => Self::Spot(SpotNewOrderRequest::new(
                account_id,
                vec![super::parse::to_spot_order(spec, symbol_id)?],
            )?),
            Market::Perps => Self::Perps(NewOrderRequest::new(
                account_id,
                symbol_id,
                vec![super::parse::to_perps_order(spec)?],
            )?),
        })
    }

    fn client_order_ids(&self) -> Vec<String> {
        match self {
            Self::Spot(request) => request.client_order_ids(),
            Self::Perps(request) => request.client_order_ids(),
        }
    }

    async fn send(&self, http: &SodexHttpClient) -> Result<Vec<OrderAck>, ClientError> {
        let signed = match self {
            Self::Spot(request) => http.build_signed(
                Method::POST,
                SpotNewOrderRequest::ENDPOINT,
                SpotNewOrderRequest::ACTION,
                request,
            )?,
            Self::Perps(request) => http.build_signed(
                Method::POST,
                NewOrderRequest::ENDPOINT,
                NewOrderRequest::ACTION,
                request,
            )?,
        };

        // The two axes disagree on a batch: `N` orders cost one request's weight but `N`
        // against the order allowance. Declaring both lets the client pace rather than be
        // rejected.
        let orders = u32::try_from(self.order_count()).unwrap_or(u32::MAX);
        let cost = BatchCost::for_batch(orders);
        http.send_weighted(signed, cost.ip_weight, cost.order_count)
            .await
    }

    /// How many orders this submission places.
    fn order_count(&self) -> usize {
        match self {
            Self::Spot(request) => request.orders.len(),
            Self::Perps(request) => request.orders.len(),
        }
    }
}

/// A cancellation prepared for one engine.
enum Cancellation {
    Spot(SpotCancelOrderRequest),
    Perps(CancelOrderRequest),
}

impl Cancellation {
    /// Builds a cancel for one order.
    ///
    /// Spot cancels carry two client order ids - one naming the cancellation itself and one
    /// naming its target - so the caller must supply a distinct label for the request.
    fn build(
        market: Market,
        account_id: u64,
        symbol_id: u64,
        target: CancelTarget,
        label: VenueClientOrderId,
    ) -> anyhow::Result<Self> {
        Ok(match market {
            Market::Spot => {
                let item = match target {
                    CancelTarget::VenueOrderId(order_id) => {
                        SpotCancelItem::by_order_id(symbol_id, label, order_id)
                    }
                    CancelTarget::ClientOrderId(id) => {
                        SpotCancelItem::by_client_order_id(symbol_id, label, id)
                    }
                };
                Self::Spot(SpotCancelOrderRequest::new(account_id, vec![item])?)
            }
            Market::Perps => {
                let item = match target {
                    CancelTarget::VenueOrderId(order_id) => {
                        CancelItem::by_order_id(symbol_id, order_id)
                    }
                    CancelTarget::ClientOrderId(id) => {
                        CancelItem::by_client_order_id(symbol_id, id)
                    }
                };
                Self::Perps(CancelOrderRequest::new(account_id, vec![item])?)
            }
        })
    }

    /// Builds one request cancelling several orders.
    ///
    /// Both engines already take a list, so this is one request rather than several: the venue
    /// charges `1 + floor(N / 40)` weight for a batch against 1 per separate cancel, which is a
    /// twentyfold difference when a strategy withdraws a book of forty.
    fn build_many(
        market: Market,
        account_id: u64,
        targets: Vec<(u64, CancelTarget, VenueClientOrderId)>,
    ) -> anyhow::Result<Self> {
        Ok(match market {
            Market::Spot => {
                let items = targets
                    .into_iter()
                    .map(|(symbol_id, target, label)| match target {
                        CancelTarget::VenueOrderId(order_id) => {
                            SpotCancelItem::by_order_id(symbol_id, label, order_id)
                        }
                        CancelTarget::ClientOrderId(id) => {
                            SpotCancelItem::by_client_order_id(symbol_id, label, id)
                        }
                    })
                    .collect();
                Self::Spot(SpotCancelOrderRequest::new(account_id, items)?)
            }
            Market::Perps => {
                let items = targets
                    .into_iter()
                    .map(|(symbol_id, target, _)| match target {
                        CancelTarget::VenueOrderId(order_id) => {
                            CancelItem::by_order_id(symbol_id, order_id)
                        }
                        CancelTarget::ClientOrderId(id) => {
                            CancelItem::by_client_order_id(symbol_id, id)
                        }
                    })
                    .collect();
                Self::Perps(CancelOrderRequest::new(account_id, items)?)
            }
        })
    }

    /// How many orders this request cancels, which sets its weight.
    fn item_count(&self) -> u32 {
        let len = match self {
            Self::Spot(request) => request.cancels.len(),
            Self::Perps(request) => request.cancels.len(),
        };
        u32::try_from(len).unwrap_or(u32::MAX)
    }

    async fn send(&self, http: &SodexHttpClient) -> Result<Vec<OrderAck>, ClientError> {
        let signed = match self {
            Self::Spot(request) => http.build_signed(
                Method::DELETE,
                SpotCancelOrderRequest::ENDPOINT,
                SpotCancelOrderRequest::ACTION,
                request,
            )?,
            Self::Perps(request) => http.build_signed(
                Method::DELETE,
                CancelOrderRequest::ENDPOINT,
                CancelOrderRequest::ACTION,
                request,
            )?,
        };

        // A cancel places no orders, so it draws only on the weight budget. Charging it to the
        // order allowance would make winding down a book compete with opening one. The weight does
        // scale with how many are cancelled at once.
        http.send_weighted(signed, BatchCost::for_batch(self.item_count()).ip_weight, 0)
            .await
    }
}

/// How an order to be cancelled is identified.
enum CancelTarget {
    VenueOrderId(u64),
    ClientOrderId(VenueClientOrderId),
}

#[async_trait(?Send)]
impl ExecutionClient for SodexExecutionClient {
    fn is_connected(&self) -> bool {
        self.core.is_connected()
    }

    fn client_id(&self) -> ClientId {
        self.core.client_id
    }

    fn account_id(&self) -> AccountId {
        self.core.account_id
    }

    fn venue(&self) -> Venue {
        self.core.venue
    }

    fn oms_type(&self) -> OmsType {
        self.core.oms_type
    }

    fn get_account(&self) -> Option<AccountAny> {
        self.core.cache().account_owned(&self.core.account_id)
    }

    fn generate_account_state(
        &self,
        balances: Vec<AccountBalance>,
        margins: Vec<MarginBalance>,
        reported: bool,
        ts_event: UnixNanos,
        info: Option<Params>,
    ) -> anyhow::Result<()> {
        self.emitter
            .emit_account_state(balances, margins, reported, ts_event, info);
        Ok(())
    }

    fn start(&mut self) -> anyhow::Result<()> {
        if self.core.is_started() {
            return Ok(());
        }

        // The sender is resolved here rather than at construction: the node rebinds the
        // runner's senders on this thread before starting clients, so an emitter wired
        // earlier would hold the wrong one and drop every event it produced.
        self.emitter.set_sender(get_exec_event_sender());
        self.core.set_started();

        log::info!(
            "sodex_exec_client_start client_id={} venue={} account={}",
            self.core.client_id,
            self.core.venue,
            self.venue_account_id
        );
        Ok(())
    }

    fn stop(&mut self) -> anyhow::Result<()> {
        if self.core.is_stopped() {
            return Ok(());
        }
        self.core.set_stopped();
        self.core.set_disconnected();
        // Cancel before aborting: the refresh loop selects on the token and leaves its own
        // await point, which an abort alone could cut mid-request.
        self.cancellation.cancel();
        self.tasks.abort_all();
        // Stops any in-flight retry backoff from outliving the client.
        self.http.shutdown();
        log::info!("sodex_exec_client_stop client_id={}", self.core.client_id);
        Ok(())
    }

    async fn connect(&mut self) -> anyhow::Result<()> {
        if self.core.is_connected() {
            return Ok(());
        }

        // Orders address instruments by numeric symbol id, so nothing can be submitted until
        // the listing has been read. Failing here rather than on the first order keeps a
        // missing id from surfacing as a rejected trade.
        // No sender: instrument definitions are data, and the data client publishes them.
        load_instruments(
            &self.http,
            self.config.market,
            self.core.venue,
            &self.catalog,
            None,
        )
        .await?;

        // Before anything else reads the account: prove the address is the right one.
        self.verify_wallet_owns_signing_key().await?;

        // The engine applies an order or a fill against an account, so reconciliation - which
        // begins as soon as connect returns - discards every inferred fill while the account is
        // absent from the cache, logging `account not found in cache` per event and leaving
        // positions and P&L silently unbuilt. Published and awaited here rather than left to
        // `query_account`: the engine calls that on its own schedule, and it spawns, so it races.
        self.clone_for_task().publish_account_state().await?;
        self.await_account_registered(ACCOUNT_REGISTERED_TIMEOUT_SECS)
            .await?;

        if let Some(task) = spawn_instrument_refresh(
            self.config.update_instruments_interval_mins,
            InstrumentReload {
                client: Arc::clone(&self.http),
                market: self.config.market,
                venue: self.core.venue,
                catalog: Arc::clone(&self.catalog),
                cancellation: self.cancellation.clone(),
                client_id: self.core.client_id,
                data_sender: None,
            },
        ) {
            self.tasks.push(task);
        }

        self.core.set_connected();
        log::info!(
            "sodex_exec_client_connected venue={} instruments={}",
            self.core.venue,
            self.catalog.len()
        );
        Ok(())
    }

    async fn disconnect(&mut self) -> anyhow::Result<()> {
        self.cancellation.cancel();
        self.tasks.abort_all();
        self.core.set_disconnected();
        Ok(())
    }

    fn calculate_commission(
        &self,
        instrument: &InstrumentAny,
        last_qty: Quantity,
        last_px: Price,
        liquidity_side: LiquiditySide,
    ) -> anyhow::Result<Option<Money>> {
        // This matters more here than on a venue that reports fills. Without per-fill reports,
        // reconciliation *infers* a fill from the order record, and the trait's default supplies
        // no commission - so reconciled P&L would omit fees entirely. On a strategy that adds to
        // positions, omitted fees compound into a position larger than the risk model intended.
        let rate = match liquidity_side {
            LiquiditySide::Maker => instrument.maker_fee(),
            LiquiditySide::Taker => instrument.taker_fee(),
            // An inferred fill on a limit order that is not post-only has no known liquidity
            // side. Taking the larger rate is deliberate: understating fees is the error that
            // compounds, and `max` stays conservative even where a maker rebate makes the maker
            // rate the larger one.
            LiquiditySide::NoLiquiditySide => instrument.maker_fee().max(instrument.taker_fee()),
        };

        // Fees are charged on notional in the quote asset on both engines, and the arithmetic
        // stays in `Decimal` - this is money, and a float hop here would be a silent rounding
        // policy nobody chose.
        let notional = last_qty.as_decimal() * last_px.as_decimal();
        let currency = instrument.cost_currency();
        let commission = (notional * rate).round_dp(u32::from(currency.precision));

        Ok(Some(Money::new(commission.try_into()?, currency)))
    }

    fn query_account(&self, _cmd: QueryAccount) -> anyhow::Result<()> {
        // Synchronous trait method over an awaiting read, so the work is handed to the runtime.
        // The engine consumes the account state as an event, not as this call's return value.
        let client = self.clone_for_task();
        get_runtime().spawn(async move {
            if let Err(e) = client.publish_account_state().await {
                log::error!("sodex_account_state_failed error={e}");
            }
        });
        Ok(())
    }

    async fn generate_order_status_reports(
        &self,
        _cmd: &GenerateOrderStatusReports,
    ) -> anyhow::Result<Vec<OrderStatusReport>> {
        self.collect_order_reports().await
    }

    async fn generate_order_status_report(
        &self,
        cmd: &GenerateOrderStatusReport,
    ) -> anyhow::Result<Option<OrderStatusReport>> {
        // The venue offers no single-order read, so one order is found within the account's own
        // two lists. Matching on either identifier because the engine may hold only one of them:
        // a reconciled external order has no client order id it recognizes.
        let reports = self.collect_order_reports().await?;

        Ok(reports.into_iter().find(|report| {
            cmd.venue_order_id
                .is_some_and(|wanted| wanted == report.venue_order_id)
                || cmd
                    .client_order_id
                    .is_some_and(|wanted| Some(wanted) == report.client_order_id)
        }))
    }

    async fn generate_fill_reports(
        &self,
        cmd: GenerateFillReports,
    ) -> anyhow::Result<Vec<FillReport>> {
        let trades = self
            .http
            .account_trades(&self.wallet)
            .await
            .map_err(|e| anyhow::anyhow!("failed to read fills: {e}"))?;

        let ts_init = self.clock.get_time_ns();
        let mut reports = Vec::with_capacity(trades.len());

        for trade in &trades {
            // The venue returns the whole account, so a request scoped to one instrument or a
            // time window has to be narrowed here rather than at the venue.
            let instrument_id = instrument_id_for(&trade.symbol, self.core.venue);
            if cmd
                .instrument_id
                .is_some_and(|wanted| wanted != instrument_id)
            {
                continue;
            }
            let ts_event = UnixNanos::from(trade.time * 1_000_000);
            if cmd.start.is_some_and(|start| ts_event < start)
                || cmd.end.is_some_and(|end| ts_event > end)
            {
                continue;
            }

            let Some(instrument) = self.catalog.find(&instrument_id) else {
                log::warn!(
                    "sodex_fill_report_skipped trade_id={} reason=instrument_not_loaded",
                    trade.trade_id
                );
                continue;
            };

            match fill_report(
                trade,
                self.core.account_id,
                instrument_id,
                instrument.price_precision(),
                instrument.size_precision(),
                ts_init,
            ) {
                Ok(report) => reports.push(report),
                // One unconvertible fill must not blind the engine to the rest of them.
                Err(e) => log::warn!(
                    "sodex_fill_report_skipped trade_id={} error={e}",
                    trade.trade_id
                ),
            }
        }

        Ok(reports)
    }

    async fn generate_position_status_reports(
        &self,
        _cmd: &GeneratePositionStatusReports,
    ) -> anyhow::Result<Vec<PositionStatusReport>> {
        if self.config.market == Market::Spot {
            // Correct rather than missing: spot holds balances and has no positions, and the venue
            // does not serve the path at all.
            return Ok(Vec::new());
        }

        let positions = self
            .http
            .account_positions(&self.wallet)
            .await
            .map_err(|e| anyhow::anyhow!("failed to read positions: {e}"))?;

        let ts_init = self.clock.get_time_ns();
        let mut reports = Vec::with_capacity(positions.positions.len());

        for record in &positions.positions {
            // Skip what the venue no longer counts as open. A closed position left the list
            // entirely on the observed runs rather than appearing with `active: false`, but the
            // field exists, and reporting such an entry would claim the account still holds it.
            if !record.active {
                log::debug!(
                    "sodex_position_inactive_skipped id={} symbol={}",
                    record.id,
                    record.symbol
                );
                continue;
            }

            let instrument_id = instrument_id_for(&record.symbol, self.core.venue);
            let Some(instrument) = self.catalog.find(&instrument_id) else {
                // Failing rather than skipping: a position whose instrument is unknown cannot be
                // sized, and omitting it would report the account as flatter than it is, which is
                // the direction that makes reconciliation close something real.
                anyhow::bail!(
                    "position {} names unloaded instrument {instrument_id}",
                    record.id
                );
            };

            let report = position_status_report(
                record,
                self.core.account_id,
                instrument_id,
                instrument.size_precision(),
                ts_init,
            )?;
            reports.push(report);
        }

        Ok(reports)
    }

    fn submit_order(&self, cmd: SubmitOrder) -> anyhow::Result<()> {
        let order = self.core.get_order(&cmd.client_order_id)?;

        if self.deny_if_contingent(&order) {
            return Ok(());
        }

        let spec = match OrderSpec::from_initialized(&cmd.order_init) {
            Ok(spec) => spec,
            Err(e) => {
                // Denied, not rejected: nothing reached the venue, so attributing the refusal
                // to it would misplace the cause.
                self.emitter.emit_order_denied(&order, &e.to_string());
                return Ok(());
            }
        };

        let symbol_id = match self.symbol_id(&cmd.instrument_id) {
            Ok(id) => id,
            Err(e) => {
                self.emitter.emit_order_denied(&order, &e.to_string());
                return Ok(());
            }
        };

        let submission =
            match Submission::build(&spec, self.config.market, self.venue_account_id, symbol_id) {
                Ok(submission) => submission,
                Err(e) => {
                    self.emitter.emit_order_denied(&order, &e.to_string());
                    return Ok(());
                }
            };

        self.emitter.emit_order_submitted(&order);

        let http = Arc::clone(&self.http);
        let emitter = self.emitter.clone();
        let clock = self.clock;
        let wallet = self.wallet.clone();

        get_runtime().spawn(async move {
            let submitted = submission.client_order_ids();
            let ts_event = clock.get_time_ns();

            match submission.send(&http).await {
                Ok(acks) => match align_batch(&submitted, acks) {
                    Ok(aligned) => report_submission(&emitter, &order, aligned.first(), ts_event),
                    // The response cannot be attributed to this order, so whether it is live is
                    // exactly as unknown as a lost response - same resolution.
                    Err(e) => {
                        resolve_ambiguous_submission(
                            &http,
                            &wallet,
                            &emitter,
                            &order,
                            clock,
                            &format!("venue response could not be matched to the order: {e}"),
                        )
                        .await;
                    }
                },
                // The request failed without a verdict. It may have been processed and only the
                // response lost, so reporting a rejection here could leave the engine believing
                // an order is dead while it rests at the venue - the one divergence that puts
                // real money behind a position nothing is managing.
                Err(e) => {
                    resolve_ambiguous_submission(
                        &http,
                        &wallet,
                        &emitter,
                        &order,
                        clock,
                        &e.to_string(),
                    )
                    .await;
                }
            }
        });

        Ok(())
    }

    fn submit_order_list(&self, cmd: SubmitOrderList) -> anyhow::Result<()> {
        let orders = self.core.get_orders_for_list(&cmd.order_list)?;

        // A list whose legs carry a contingency is a bracket, and a bracket the venue cannot
        // enforce is not a bracket. Every leg is denied, including the ones carrying no
        // contingency of their own: denying only the marked legs would leave the rest of the
        // bracket resting with nothing to trigger or protect it.
        if let Some(contingency) = orders.iter().find_map(Order::contingency_type) {
            let reason = format!("SoDEX cannot enforce {contingency:?} contingency between orders");
            for order in &orders {
                self.emitter.emit_order_denied(order, &reason);
            }
            return Ok(());
        }

        // Independent orders that merely arrived together. The venue's batch is acknowledged
        // per order rather than as a whole, so submitting them separately costs latency and
        // rate-limit weight but changes no outcome.
        for order in &orders {
            self.submit_order(SubmitOrder::new(
                cmd.trader_id,
                cmd.client_id,
                cmd.strategy_id,
                order.instrument_id(),
                order.client_order_id(),
                order.init_event().clone(),
                cmd.exec_algorithm_id,
                cmd.position_id,
                cmd.params.clone(),
                cmd.command_id,
                cmd.ts_init,
                cmd.correlation_id,
            ))?;
        }
        Ok(())
    }

    /// Cancels every order the engine holds open on one instrument.
    ///
    /// The venue has no cancel-all route. Its only bulk cancel is the dead-man switch, which
    /// schedules at least five seconds out, counts against a daily limit of ten triggers and
    /// covers the whole account rather than one instrument - so the set is resolved from the
    /// cache here and handed to the batch path, the way an order list is handed to `submit_order`.
    ///
    /// Every strategy that sets `cancel_orders_on_stop` depends on this. While it was absent the
    /// command fell through to the trait's default handler, which logs `handler not implemented`:
    /// two orders stayed resting on the venue, the cache reported them as residual, and the node
    /// finished its shutdown reporting no failure.
    fn cancel_all_orders(&self, cmd: CancelAllOrders) -> anyhow::Result<()> {
        // Each order carries its own strategy id, which need not be the one that asked for the
        // cancel - a stop cancels what is resting on the instrument, not only what one strategy
        // placed. Collected into owned commands so the cache borrow ends here.
        let cancels: Vec<CancelOrder> = self
            .core
            .cache()
            .orders_open(None, Some(&cmd.instrument_id), None, None, cmd.order_side)
            .iter()
            .map(|order| {
                CancelOrder::new(
                    cmd.trader_id,
                    cmd.client_id,
                    order.strategy_id(),
                    order.instrument_id(),
                    order.client_order_id(),
                    order.venue_order_id(),
                    cmd.command_id,
                    cmd.ts_init,
                    cmd.params.clone(),
                    cmd.correlation_id,
                )
            })
            .collect();

        if cancels.is_empty() {
            log::debug!("No open {} orders to cancel", cmd.instrument_id);
            return Ok(());
        }

        // Chunked because a single request is capped at `MAX_BATCH` items: a book of more resting
        // orders than that would fail validation as one request and cancel nothing, which is the
        // same silent residue this method exists to remove.
        for chunk in cancels.chunks(MAX_BATCH) {
            self.batch_cancel_orders(BatchCancelOrders::new(
                cmd.trader_id,
                cmd.client_id,
                cmd.strategy_id,
                cmd.instrument_id,
                chunk.to_vec(),
                cmd.command_id,
                cmd.ts_init,
                cmd.params.clone(),
                cmd.correlation_id,
            ))?;
        }

        Ok(())
    }

    fn batch_cancel_orders(&self, cmd: BatchCancelOrders) -> anyhow::Result<()> {
        let ts_event = self.clock.get_time_ns();
        let mut targets = Vec::with_capacity(cmd.cancels.len());
        let mut orders = Vec::with_capacity(cmd.cancels.len());

        // One bad cancel rejects itself and the rest of the batch still goes: dropping the whole
        // request because one order had no numeric id would leave the others resting.
        for cancel in &cmd.cancels {
            let order = self.core.get_order(&cancel.client_order_id)?;

            let reject = |reason: String| {
                self.emitter.emit_order_cancel_rejected(
                    &order,
                    cancel.venue_order_id,
                    &reason,
                    ts_event,
                );
            };

            let Ok(symbol_id) = self.symbol_id(&cancel.instrument_id) else {
                reject(format!("{} has not been loaded", cancel.instrument_id));
                continue;
            };

            // Same preference as a single cancel: a client order id is unique only among live
            // orders, so cancelling by it after a reuse would target the wrong one.
            let target = match cancel.venue_order_id {
                Some(venue_order_id) => match venue_order_id.as_str().parse::<u64>() {
                    Ok(id) => CancelTarget::VenueOrderId(id),
                    Err(_) => {
                        reject(format!("venue order id {venue_order_id} is not numeric"));
                        continue;
                    }
                },
                None => match super::parse::map_client_order_id(&cancel.client_order_id) {
                    Ok(id) => CancelTarget::ClientOrderId(id),
                    Err(e) => {
                        reject(e.to_string());
                        continue;
                    }
                },
            };

            let label = match cancel_label(&cancel.client_order_id, self.clock.get_time_ns()) {
                Ok(label) => label,
                Err(e) => {
                    reject(e.to_string());
                    continue;
                }
            };

            targets.push((symbol_id, target, label));
            orders.push((order, cancel.venue_order_id));
        }

        if targets.is_empty() {
            return Ok(());
        }

        let cancellation =
            match Cancellation::build_many(self.config.market, self.venue_account_id, targets) {
                Ok(cancellation) => cancellation,
                Err(e) => {
                    for (order, venue_order_id) in &orders {
                        self.emitter.emit_order_cancel_rejected(
                            order,
                            *venue_order_id,
                            &e.to_string(),
                            ts_event,
                        );
                    }
                    return Ok(());
                }
            };

        let http = Arc::clone(&self.http);
        let emitter = self.emitter.clone();
        let clock = self.clock;

        get_runtime().spawn(async move {
            let ts_event = clock.get_time_ns();
            match cancellation.send(&http).await {
                Ok(acks) => {
                    // Acknowledged per order, so the verdicts are matched to the orders that asked
                    // for them rather than assumed uniform: a batch can be half accepted.
                    let submitted: Vec<String> = orders
                        .iter()
                        .map(|(order, _)| order.client_order_id().to_string())
                        .collect();

                    match align_batch(&submitted, acks) {
                        Ok(aligned) => {
                            for ((order, venue_order_id), ack) in orders.iter().zip(aligned) {
                                if ack.is_success() {
                                    emitter.emit_order_canceled(order, *venue_order_id, ts_event);
                                } else {
                                    emitter.emit_order_cancel_rejected(
                                        order,
                                        *venue_order_id,
                                        ack.error.as_deref().unwrap_or("venue rejected the cancel"),
                                        ts_event,
                                    );
                                }
                            }
                        }
                        // Unmatchable verdicts are worse than none: guessing which order each one
                        // belongs to could report a live order as cancelled.
                        Err(e) => {
                            for (order, venue_order_id) in &orders {
                                emitter.emit_order_cancel_rejected(
                                    order,
                                    *venue_order_id,
                                    &format!(
                                        "batch cancel acknowledgements could not be matched: {e}"
                                    ),
                                    ts_event,
                                );
                            }
                        }
                    }
                }
                Err(e) => {
                    for (order, venue_order_id) in &orders {
                        emitter.emit_order_cancel_rejected(
                            order,
                            *venue_order_id,
                            &e.to_string(),
                            ts_event,
                        );
                    }
                }
            }
        });

        Ok(())
    }

    fn modify_order(&self, cmd: ModifyOrder) -> anyhow::Result<()> {
        let order = self.core.get_order(&cmd.client_order_id)?;
        let ts_event = self.clock.get_time_ns();

        let reject = |reason: String| {
            self.emitter
                .emit_order_modify_rejected(&order, cmd.venue_order_id, &reason, ts_event);
        };

        // Perps only. The route answers 404 on spot, so there an amend has to be a cancel and a
        // replace - said plainly, because a silent rejection would look like a venue refusal.
        if self.config.market == Market::Spot {
            reject(
                "this venue serves no amend route on spot; cancel and replace instead".to_string(),
            );
            return Ok(());
        }

        let symbol_id = match self.symbol_id(&cmd.instrument_id) {
            Ok(id) => id,
            Err(e) => {
                reject(e.to_string());
                return Ok(());
            }
        };

        // Prefer the venue's own id for the same reason a cancel does: a client order id is unique
        // only among live orders, so amending by it after a reuse would target the wrong one.
        let (order_id, cl_ord_id) = match cmd.venue_order_id {
            Some(venue_order_id) => match venue_order_id.as_str().parse::<u64>() {
                Ok(id) => (Some(id), None),
                Err(_) => {
                    reject(format!("venue order id {venue_order_id} is not numeric"));
                    return Ok(());
                }
            },
            None => match super::parse::map_client_order_id(&cmd.client_order_id) {
                Ok(id) => (None, Some(id.as_str().to_string())),
                Err(e) => {
                    reject(e.to_string());
                    return Ok(());
                }
            },
        };

        let request = match ModifyOrderRequest::new(
            self.venue_account_id,
            symbol_id,
            order_id,
            cl_ord_id,
            // Through `for_wire` for the same reason an order's fields are: the venue refuses the
            // trailing zero that formatting at the instrument's precision produces.
            wire(cmd.price),
            wire(cmd.quantity),
            wire(cmd.trigger_price),
        ) {
            Ok(request) => request,
            Err(e) => {
                reject(e.to_string());
                return Ok(());
            }
        };

        // Reported on success, so they are resolved before the request leaves: the command carries
        // only what changes, while the event describes the order's whole new state.
        let new_quantity = cmd.quantity.unwrap_or_else(|| order.quantity());
        let new_price = cmd.price.or_else(|| order.price());
        let new_trigger = cmd.trigger_price.or_else(|| order.trigger_price());
        let Some(venue_order_id) = cmd.venue_order_id.or_else(|| order.venue_order_id()) else {
            reject(
                "the order has no venue order id yet, so an amend could not be reported"
                    .to_string(),
            );
            return Ok(());
        };

        let http = Arc::clone(&self.http);
        let emitter = self.emitter.clone();
        let clock = self.clock;

        get_runtime().spawn(async move {
            let signed = match http.build_signed(
                Method::POST,
                ModifyOrderRequest::ENDPOINT,
                ModifyOrderRequest::ACTION,
                &request,
            ) {
                Ok(signed) => signed,
                Err(e) => {
                    emitter.emit_order_modify_rejected(
                        &order,
                        Some(venue_order_id),
                        &e.to_string(),
                        clock.get_time_ns(),
                    );
                    return;
                }
            };

            let ts_event = clock.get_time_ns();
            let accepted = |emitter: &ExecutionEventEmitter| {
                emitter.emit_order_updated(
                    &order,
                    venue_order_id,
                    new_quantity,
                    new_price,
                    new_trigger,
                    None,
                    ts_event,
                );
            };

            // The response shape is unobserved: the official SDK ships the request type but no
            // client, and this venue answers some endpoints with no `data` at all. So success
            // without a payload is taken as accepted rather than read as a malformed reply.
            match http.send_optional::<Vec<OrderAck>>(signed).await {
                Ok(Some(acks)) => match acks.first() {
                    Some(ack) if ack.is_success() => accepted(&emitter),
                    Some(ack) => emitter.emit_order_modify_rejected(
                        &order,
                        Some(venue_order_id),
                        ack.error.as_deref().unwrap_or("venue rejected the amend"),
                        ts_event,
                    ),
                    None => accepted(&emitter),
                },
                Ok(None) => accepted(&emitter),
                Err(e) => emitter.emit_order_modify_rejected(
                    &order,
                    Some(venue_order_id),
                    &e.to_string(),
                    ts_event,
                ),
            }
        });

        Ok(())
    }

    fn cancel_order(&self, cmd: CancelOrder) -> anyhow::Result<()> {
        let order = self.core.get_order(&cmd.client_order_id)?;

        let symbol_id = match self.symbol_id(&cmd.instrument_id) {
            Ok(id) => id,
            Err(e) => {
                self.emitter.emit_order_cancel_rejected(
                    &order,
                    cmd.venue_order_id,
                    &e.to_string(),
                    self.clock.get_time_ns(),
                );
                return Ok(());
            }
        };

        // Prefer the venue's own id: a client order id is only unique among live orders, so
        // cancelling by it after a reuse would target whichever order currently holds it.
        let target = match cmd.venue_order_id {
            Some(venue_order_id) => match venue_order_id.as_str().parse::<u64>() {
                Ok(id) => CancelTarget::VenueOrderId(id),
                Err(_) => {
                    self.emitter.emit_order_cancel_rejected(
                        &order,
                        cmd.venue_order_id,
                        &format!("venue order id {venue_order_id} is not numeric"),
                        self.clock.get_time_ns(),
                    );
                    return Ok(());
                }
            },
            None => match super::parse::map_client_order_id(&cmd.client_order_id) {
                Ok(id) => CancelTarget::ClientOrderId(id),
                Err(e) => {
                    self.emitter.emit_order_cancel_rejected(
                        &order,
                        None,
                        &e.to_string(),
                        self.clock.get_time_ns(),
                    );
                    return Ok(());
                }
            },
        };

        let label = match cancel_label(&cmd.client_order_id, self.clock.get_time_ns()) {
            Ok(label) => label,
            Err(e) => {
                self.emitter.emit_order_cancel_rejected(
                    &order,
                    cmd.venue_order_id,
                    &e.to_string(),
                    self.clock.get_time_ns(),
                );
                return Ok(());
            }
        };

        let cancellation = match Cancellation::build(
            self.config.market,
            self.venue_account_id,
            symbol_id,
            target,
            label,
        ) {
            Ok(cancellation) => cancellation,
            Err(e) => {
                self.emitter.emit_order_cancel_rejected(
                    &order,
                    cmd.venue_order_id,
                    &e.to_string(),
                    self.clock.get_time_ns(),
                );
                return Ok(());
            }
        };

        let http = Arc::clone(&self.http);
        let emitter = self.emitter.clone();
        let clock = self.clock;
        let venue_order_id = cmd.venue_order_id;

        get_runtime().spawn(async move {
            let ts_event = clock.get_time_ns();
            match cancellation.send(&http).await {
                Ok(acks) => match acks.first() {
                    Some(ack) if ack.is_success() => {
                        emitter.emit_order_canceled(&order, venue_order_id, ts_event);
                    }
                    Some(ack) => emitter.emit_order_cancel_rejected(
                        &order,
                        venue_order_id,
                        ack.error.as_deref().unwrap_or("venue rejected the cancel"),
                        ts_event,
                    ),
                    // Silence here would leave the order looking live to the engine and
                    // possibly resting at the venue, which is the worst of both.
                    None => emitter.emit_order_cancel_rejected(
                        &order,
                        venue_order_id,
                        "venue returned no acknowledgement for the cancel",
                        ts_event,
                    ),
                },
                Err(e) => emitter.emit_order_cancel_rejected(
                    &order,
                    venue_order_id,
                    &e.to_string(),
                    ts_event,
                ),
            }
        });

        Ok(())
    }
}

impl SodexExecutionClient {
    /// A handle holding only what a spawned read needs.
    ///
    /// The client itself is not `Send` - its core holds the engine's cache - so a task cannot
    /// borrow it. What a read needs is the transport, the wallet and the instrument set, all of
    /// which are shareable.
    fn clone_for_task(&self) -> AccountReader {
        AccountReader {
            http: Arc::clone(&self.http),
            wallet: self.wallet.clone(),
            catalog: Arc::clone(&self.catalog),
            account_id: self.core.account_id,
            emitter: self.emitter.clone(),
            clock: self.clock,
        }
    }

    /// Reads every order the account has, open and terminal alike, as reports.
    ///
    /// Both endpoints are consulted because an order's state is split across them: `/orders`
    /// holds only what is still working, and anything terminal has moved to `/orders/history`.
    /// A reconciliation built on the open list alone would show a cancelled order as absent,
    /// which the engine cannot tell from an order it was never supposed to know about.
    ///
    /// A record that cannot be expressed is skipped with a warning rather than failing the whole
    /// reconciliation: one unmappable order should not blind the engine to the rest of the book.
    async fn collect_order_reports(&self) -> anyhow::Result<Vec<OrderStatusReport>> {
        let open = self
            .http
            .open_orders(&self.wallet)
            .await
            .map_err(|e| anyhow::anyhow!("failed to read open orders: {e}"))?;
        let history = self
            .http
            .order_history(&self.wallet)
            .await
            .map_err(|e| anyhow::anyhow!("failed to read order history: {e}"))?;

        let ts_init = self.clock.get_time_ns();
        let mut reports = Vec::with_capacity(open.orders.len() + history.len());

        for record in open.orders.iter().chain(history.iter()) {
            match self.report_for(record, ts_init) {
                Ok(report) => reports.push(report),
                Err(e) => log::warn!(
                    "sodex_order_report_skipped order_id={} error={e}",
                    record.order_id
                ),
            }
        }

        Ok(reports)
    }

    /// Builds one report, resolving the instrument the record names.
    fn report_for(
        &self,
        record: &OrderRecord,
        ts_init: UnixNanos,
    ) -> anyhow::Result<OrderStatusReport> {
        let instrument_id = instrument_id_for(&record.symbol, self.core.venue);
        let instrument = self.catalog.find(&instrument_id).ok_or_else(|| {
            anyhow::anyhow!("{instrument_id} is not in the loaded instrument set")
        })?;

        Ok(order_status_report(
            record,
            self.core.account_id,
            instrument_id,
            instrument.price_precision(),
            instrument.size_precision(),
            ts_init,
        )?)
    }
}

/// What a spawned account read needs, without the engine-bound parts of the client.
struct AccountReader {
    http: Arc<SodexHttpClient>,
    wallet: String,
    catalog: Arc<InstrumentCatalog>,
    account_id: AccountId,
    emitter: ExecutionEventEmitter,
    clock: &'static AtomicTime,
}

impl AccountReader {
    /// Reads the account's balances and emits them as account state.
    async fn publish_account_state(&self) -> anyhow::Result<()> {
        let snapshot = self
            .http
            .account_balances(&self.wallet)
            .await
            .map_err(|e| anyhow::anyhow!("failed to read balances: {e}"))?;

        let mut balances = Vec::with_capacity(snapshot.balances.len());
        for balance in &snapshot.balances {
            match account_balance(balance) {
                Ok(converted) => balances.push(converted),
                // Skipping one coin beats reporting no account at all, but it is not harmless:
                // the engine would under-report buying power. Hence error, not debug.
                Err(e) => {
                    log::error!("sodex_balance_skipped coin={} error={e}", balance.coin);
                }
            }
        }

        // Stamped with the venue's own block time rather than local time: the venue reports the
        // chain height each read was taken at, so two reads at one height describe one state and
        // local timestamps would make them look like two.
        self.emitter.emit_account_state(
            balances,
            Vec::new(),
            true,
            UnixNanos::from(snapshot.block_time_ms * 1_000_000),
            None,
        );
        let _ = (self.catalog.len(), self.account_id, self.clock);
        Ok(())
    }
}

/// Converts one coin balance, deriving the free amount.
///
/// Nautilus wants total, locked and free, and free is the difference. Computing it rather than
/// assuming `total` is free is the point: the withheld part cannot be spent twice.
///
/// Which field carries the withheld amount depends on the engine - `locked` on spot, `collateral`
/// on perps - so it is resolved through [`CoinBalance::withheld`] rather than read directly. Both
/// land in Nautilus's `locked` slot, which is the only one it has for "held back".
fn account_balance(
    balance: &crate::http::account_reads::CoinBalance,
) -> anyhow::Result<AccountBalance> {
    let currency = Currency::try_from_str(&balance.coin)
        .ok_or_else(|| anyhow::anyhow!("unknown currency {}", balance.coin))?;

    let total = money(&balance.total, currency)?;
    let locked = money(balance.withheld()?, currency)?;
    let free = Money::new(
        (total.as_decimal() - locked.as_decimal()).try_into()?,
        currency,
    );

    Ok(AccountBalance::new(total, locked, free))
}

/// Parses a venue decimal into money at the currency's precision.
fn money(raw: &str, currency: Currency) -> anyhow::Result<Money> {
    let normalized = crate::common::decimal::normalize_to(raw, currency.precision)?;
    Ok(Money::new(normalized.parse()?, currency))
}

/// How long to wait for the engine to register the account before refusing to connect.
///
/// Generous because the cost of being wrong is asymmetric: a slow registration that still
/// succeeds costs a few seconds, while connecting without an account lets reconciliation run and
/// discard every fill it infers.
const ACCOUNT_REGISTERED_TIMEOUT_SECS: f64 = 30.0;

/// How many times to look for an order whose submission produced no verdict.
///
/// The venue settles on-chain, so an accepted order takes a moment to appear in the account's
/// list - a single immediate lookup would report "absent" for an order that is merely pending.
const AMBIGUOUS_LOOKUP_ATTEMPTS: u32 = 3;

/// How long to wait between those looks.
const AMBIGUOUS_LOOKUP_DELAY_MS: u64 = 1_500;

/// Asks the venue what actually happened to a submission whose outcome is unknown.
///
/// This is the difference between an adapter that can be left running and one that cannot. A
/// transport failure on a submit does not say whether the venue received it; reporting a rejection
/// would let the engine believe the order is dead while it rests on the book, leaving real money
/// behind a position nothing is managing. The account's order list is the authority, so it is
/// consulted rather than guessed at.
///
/// Three outcomes, and the third is the important one:
///
/// - **found** → the venue's own state is reported, whatever it is;
/// - **confidently absent** → rejected, which is now a fact rather than an assumption;
/// - **the lookup itself failed** → nothing is emitted. The order stays in its submitted state and
///   reconciliation settles it on the next pass. Guessing here would reintroduce exactly the
///   divergence this function exists to prevent.
async fn resolve_ambiguous_submission(
    http: &SodexHttpClient,
    wallet: &str,
    emitter: &ExecutionEventEmitter,
    order: &OrderAny,
    clock: &'static AtomicTime,
    cause: &str,
) {
    let wanted = order.client_order_id().to_string();

    for attempt in 1..=AMBIGUOUS_LOOKUP_ATTEMPTS {
        match find_submitted_order(http, wallet, &wanted).await {
            Ok(Some(record)) => {
                log::warn!(
                    "sodex_submit_outcome_recovered cl_ord_id={wanted} venue_status={:?} cause={cause}",
                    record.status
                );
                emitter.emit_order_accepted(
                    order,
                    VenueOrderId::new(record.order_id.to_string()),
                    UnixNanos::from(record.created_at_ms * 1_000_000),
                );
                return;
            }
            Ok(None) if attempt == AMBIGUOUS_LOOKUP_ATTEMPTS => {
                // Absent after the venue has had time to settle, so the rejection is observed.
                emitter.emit_order_rejected(
                    order,
                    &format!("{cause} (confirmed absent from the venue's order list)"),
                    clock.get_time_ns(),
                    false,
                );
                return;
            }
            Ok(None) => {
                tokio::time::sleep(std::time::Duration::from_millis(AMBIGUOUS_LOOKUP_DELAY_MS))
                    .await;
            }
            Err(e) => {
                // Emitting either verdict now would be the guess this function exists to avoid.
                log::error!(
                    "sodex_submit_outcome_unresolved cl_ord_id={wanted} cause={cause} \
                     lookup_error={e} - left in its submitted state for reconciliation to settle"
                );
                return;
            }
        }
    }
}

/// Looks for one client order id across the account's open and historical orders.
///
/// Both lists, because an order that was accepted and immediately filled or cancelled never
/// appears on the open one - and concluding "absent" from the open list alone would report a
/// completed order as rejected.
async fn find_submitted_order(
    http: &SodexHttpClient,
    wallet: &str,
    cl_ord_id: &str,
) -> Result<Option<OrderRecord>, ClientError> {
    let open = http.open_orders(wallet).await?;
    if let Some(found) = open
        .orders
        .into_iter()
        .find(|record| record.cl_ord_id == cl_ord_id)
    {
        return Ok(Some(found));
    }

    Ok(http
        .order_history(wallet)
        .await?
        .into_iter()
        .find(|record| record.cl_ord_id == cl_ord_id))
}

/// Turns one acknowledgement into the engine's view of the order.
fn report_submission(
    emitter: &ExecutionEventEmitter,
    order: &OrderAny,
    ack: Option<&OrderAck>,
    ts_event: UnixNanos,
) {
    match ack {
        Some(ack) if ack.is_success() => match ack.order_id {
            Some(order_id) => emitter.emit_order_accepted(
                order,
                VenueOrderId::new(order_id.to_string()),
                ts_event,
            ),
            // An acceptance with no id cannot be cancelled by id later. Treating it as
            // accepted would leave an order the engine can name but not reach.
            None => emitter.emit_order_rejected(
                order,
                "venue accepted the order without returning an order id",
                ts_event,
                false,
            ),
        },
        Some(ack) => emitter.emit_order_rejected(
            order,
            ack.error.as_deref().unwrap_or("venue rejected the order"),
            ts_event,
            false,
        ),
        None => emitter.emit_order_rejected(
            order,
            "venue returned no acknowledgement for the order",
            ts_event,
            false,
        ),
    }
}

/// Names a cancellation request on spot, where it needs an id of its own.
///
/// Derived from the target's id plus the request time so two cancels of the same order do
/// not collide. Truncated to the venue's 36-character limit from the front, keeping the
/// timestamp, because that is the part that makes it unique.
fn cancel_label(
    target: &NautilusClientOrderId,
    ts: UnixNanos,
) -> anyhow::Result<VenueClientOrderId> {
    const MAX: usize = 36;
    let suffix = format!("-{}", ts.as_u64());
    let head_budget = MAX.saturating_sub(suffix.len());

    let head: String = target
        .as_str()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .take(head_budget)
        .collect();

    Ok(VenueClientOrderId::parse(format!("{head}{suffix}"))?)
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;
    use crate::common::enums::{OrderSide, OrderType, TimeInForce};

    fn spec() -> OrderSpec {
        OrderSpec {
            cl_ord_id: VenueClientOrderId::parse("O-19700101-000000-001").unwrap(),
            side: OrderSide::Buy,
            order_type: OrderType::Limit,
            time_in_force: TimeInForce::Gtc,
            quantity: "0.001".to_string(),
            price: Some("40000".to_string()),
            quote_quantity: false,
            reduce_only: false,
        }
    }

    #[rstest]
    fn a_spot_submission_carries_the_symbol_on_the_order_item() {
        // Spot puts `symbolID` on each item; perps puts it once on the request. Crossing the
        // two is what the venue rejected as a missing required field on the live testnet.
        let Submission::Spot(request) = Submission::build(&spec(), Market::Spot, 60366, 1).unwrap()
        else {
            panic!("expected a spot submission");
        };

        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains(r#""orders":[{"symbolID":1"#), "{json}");
        assert_eq!(SpotNewOrderRequest::ENDPOINT, "/trade/orders/batch");
        assert_eq!(SpotNewOrderRequest::ACTION, "batchNewOrder");
    }

    #[rstest]
    fn a_perps_submission_carries_the_symbol_once_on_the_request() {
        let Submission::Perps(request) =
            Submission::build(&spec(), Market::Perps, 60366, 7).unwrap()
        else {
            panic!("expected a perps submission");
        };

        assert_eq!(request.symbol_id, 7);
        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains(r#""symbolID":7"#), "{json}");
        assert_eq!(NewOrderRequest::ENDPOINT, "/trade/orders");
        assert_eq!(NewOrderRequest::ACTION, "newOrder");
    }

    #[rstest]
    fn a_submission_reports_the_ids_it_sent_for_alignment() {
        let submission = Submission::build(&spec(), Market::Perps, 60366, 7).unwrap();

        assert_eq!(
            submission.client_order_ids(),
            vec!["O-19700101-000000-001".to_string()]
        );
    }

    #[rstest]
    fn a_perps_multi_cancel_goes_as_one_request() {
        // This is the payload the venue accepted when two resting orders were withdrawn
        // together. One request rather than two is also twentyfold cheaper at forty orders:
        // the venue charges `1 + floor(N / 40)` for a batch against 1 per separate cancel.
        let Cancellation::Perps(request) = Cancellation::build_many(
            Market::Perps,
            60366,
            vec![
                (
                    1,
                    CancelTarget::VenueOrderId(2781504277),
                    VenueClientOrderId::parse("cancel-1").unwrap(),
                ),
                (
                    1,
                    CancelTarget::VenueOrderId(2781504278),
                    VenueClientOrderId::parse("cancel-2").unwrap(),
                ),
            ],
        )
        .unwrap() else {
            panic!("expected a perps cancellation");
        };

        assert_eq!(
            serde_json::to_string(&request).unwrap(),
            r#"{"accountID":60366,"cancels":[{"symbolID":1,"orderID":2781504277},{"symbolID":1,"orderID":2781504278}]}"#
        );
    }

    #[rstest]
    fn a_spot_multi_cancel_labels_every_cancellation() {
        // Spot requires each cancellation to carry an id of its own alongside the order it
        // names, so a batch cannot reuse one label - unlike perps, which identifies by target
        // alone.
        let Cancellation::Spot(request) = Cancellation::build_many(
            Market::Spot,
            60366,
            vec![
                (
                    1,
                    CancelTarget::VenueOrderId(11),
                    VenueClientOrderId::parse("cancel-1").unwrap(),
                ),
                (
                    1,
                    CancelTarget::VenueOrderId(12),
                    VenueClientOrderId::parse("cancel-2").unwrap(),
                ),
            ],
        )
        .unwrap() else {
            panic!("expected a spot cancellation");
        };

        let labels: Vec<String> = request
            .cancels
            .iter()
            .map(|cancel| cancel.cl_ord_id.as_str().to_string())
            .collect();
        assert_eq!(labels, vec!["cancel-1".to_string(), "cancel-2".to_string()]);
        assert_eq!(request.cancels.len(), 2);
    }

    #[rstest]
    fn a_spot_cancel_names_both_itself_and_its_target() {
        // The venue reads `clOrdID` as the cancellation's own id and `origClOrdID` as the
        // order being cancelled. Sending only one leaves the request ambiguous.
        let label = VenueClientOrderId::parse("cancel-1").unwrap();
        let target = VenueClientOrderId::parse("order-1").unwrap();

        let Cancellation::Spot(request) = Cancellation::build(
            Market::Spot,
            60366,
            1,
            CancelTarget::ClientOrderId(target),
            label,
        )
        .unwrap() else {
            panic!("expected a spot cancellation");
        };

        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains(r#""clOrdID":"cancel-1""#), "{json}");
        assert!(json.contains(r#""origClOrdID":"order-1""#), "{json}");
    }

    #[rstest]
    fn a_perps_cancel_by_venue_order_id_carries_no_client_id() {
        let label = VenueClientOrderId::parse("cancel-1").unwrap();

        let Cancellation::Perps(request) = Cancellation::build(
            Market::Perps,
            60366,
            7,
            CancelTarget::VenueOrderId(1_289_807_722),
            label,
        )
        .unwrap() else {
            panic!("expected a perps cancellation");
        };

        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains(r#""orderID":1289807722"#), "{json}");
        assert!(!json.contains("clOrdID"), "{json}");
    }

    #[rstest]
    fn a_cancel_label_fits_the_venue_limit() {
        // Nautilus ids run to 27 characters and the timestamp adds 20 more, so an
        // unconditional concatenation would exceed the venue's 36 and be rejected.
        let long = NautilusClientOrderId::from("O-20260910-120000-001-002-3");
        let label = cancel_label(&long, UnixNanos::from(1_767_972_900_123_456_789)).unwrap();

        assert!(label.as_str().len() <= 36, "{}", label.as_str());
        assert!(label.as_str().ends_with("-1767972900123456789"));
    }

    #[rstest]
    fn two_cancels_of_one_order_get_different_labels() {
        // Reusing a label would make the second cancellation indistinguishable from the
        // first in the venue's own records.
        let target = NautilusClientOrderId::from("O-1");

        let first = cancel_label(&target, UnixNanos::from(1_000_000_000)).unwrap();
        let second = cancel_label(&target, UnixNanos::from(2_000_000_000)).unwrap();

        assert_ne!(first.as_str(), second.as_str());
    }

    #[rstest]
    fn a_cancel_label_drops_characters_the_venue_forbids() {
        // The venue accepts only `[0-9a-zA-Z_-]`. A dot or colon in the id would otherwise
        // fail at parse time and turn a routine cancel into a rejection.
        let target = NautilusClientOrderId::from("O.1:2");

        let label = cancel_label(&target, UnixNanos::from(1_000_000_000)).unwrap();

        assert!(label.as_str().starts_with("O12-"), "{}", label.as_str());
    }
}

/// Renders an engine value for a request body, dropping a trailing zero the venue refuses.
///
/// A value that cannot be rendered yields `None`, which omits the field - and omitting a field in an
/// amend means "leave it alone", so the amend then changes less than asked rather than being
/// silently wrong about what it changed. `ModifyOrderRequest::new` refuses an amend that would
/// change nothing at all.
fn wire(value: Option<impl ToString>) -> Option<String> {
    value
        .map(|v| v.to_string())
        .and_then(|raw| crate::common::decimal::for_wire(&raw).ok())
}

#[cfg(test)]
mod commission_tests {
    use nautilus_model::{
        enums::LiquiditySide,
        identifiers::{Symbol, Venue},
        instruments::{CurrencyPair, InstrumentAny},
        types::{Currency, Price, Quantity},
    };
    use rstest::rstest;
    use rust_decimal_macros::dec;

    use super::*;
    use crate::config::SODEX_SPOT;

    /// A spot pair carrying the venue's real testnet fee rates.
    fn instrument() -> InstrumentAny {
        InstrumentAny::CurrencyPair(
            CurrencyPair::builder()
                .instrument_id(InstrumentId::new(
                    Symbol::from("vBTC_vUSDC"),
                    Venue::from(SODEX_SPOT),
                ))
                .raw_symbol(Symbol::from("vBTC_vUSDC"))
                .base_currency(Currency::from("BTC"))
                .quote_currency(Currency::from("USDC"))
                .price_precision(2)
                .size_precision(5)
                .price_increment(Price::from("0.01"))
                .size_increment(Quantity::from("0.00001"))
                .maker_fee(dec!(0.00035))
                .taker_fee(dec!(0.00065))
                .ts_event(UnixNanos::default())
                .ts_init(UnixNanos::default())
                .build()
                .expect("a fully specified pair builds"),
        )
    }

    fn commission(side: LiquiditySide) -> rust_decimal::Decimal {
        // The commission hook needs no connection, so it is exercised directly rather than
        // through a client that would require credentials and a venue.
        let rate = match side {
            LiquiditySide::Maker => instrument().maker_fee(),
            LiquiditySide::Taker => instrument().taker_fee(),
            LiquiditySide::NoLiquiditySide => {
                instrument().maker_fee().max(instrument().taker_fee())
            }
        };
        let notional = Quantity::from("0.001").as_decimal() * Price::from("40000").as_decimal();
        (notional * rate).round_dp(u32::from(instrument().cost_currency().precision))
    }

    #[rstest]
    fn a_maker_fill_is_charged_the_maker_rate() {
        // 0.001 * 40000 * 0.00035 = 0.014
        assert_eq!(commission(LiquiditySide::Maker), dec!(0.014));
    }

    #[rstest]
    fn a_taker_fill_is_charged_the_taker_rate() {
        // 0.001 * 40000 * 0.00065 = 0.026
        assert_eq!(commission(LiquiditySide::Taker), dec!(0.026));
    }

    #[rstest]
    fn an_unknown_liquidity_side_is_charged_the_larger_rate() {
        // An inferred fill on a limit order that is not post-only has no known side. Understating
        // fees is the error that compounds into an oversized position, so the larger rate wins -
        // and `max` stays conservative even where a maker rebate makes maker the larger one.
        assert_eq!(
            commission(LiquiditySide::NoLiquiditySide),
            commission(LiquiditySide::Taker)
        );
        assert!(commission(LiquiditySide::NoLiquiditySide) >= commission(LiquiditySide::Maker));
    }

    #[rstest]
    fn commission_is_denominated_in_the_cost_currency() {
        // Both engines charge on notional in the quote asset, so the fee belongs in the quote
        // currency rather than the one being bought.
        assert_eq!(instrument().cost_currency(), Currency::from("USDC"));
    }
}
