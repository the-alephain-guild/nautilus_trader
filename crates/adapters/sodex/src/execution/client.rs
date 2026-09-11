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
//! Position reports are likewise unimplemented, and only for perps: spot does not serve the path
//! at all, which is correct rather than missing — spot holds balances and has no positions.
//!
//! # The account reads are addressed by the master wallet, and a wrong address does not fail
//!
//! They are keyed by the account's wallet address. The API key's own address also answers `200`,
//! with an empty account — so a misconfigured client would reconcile against "no balance, no open
//! orders" and the engine would take that for a flat account, with nothing reporting a problem.
//!
//! Emptiness cannot be the error, because a new account is legitimately empty. So the client
//! proves the address instead: at connect it asks the venue which API keys that wallet has
//! registered **on this engine**, and refuses to start unless the key it signs with is among
//! them. That check also catches the other documented trap in one go — a key registered on the
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

use std::sync::Arc;

use async_trait::async_trait;
use nautilus_common::{
    clients::ExecutionClient,
    live::{get_runtime, runner::get_exec_event_sender, task::TaskHandles},
    messages::execution::{
        CancelOrder, GenerateOrderStatusReport, GenerateOrderStatusReports,
        GeneratePositionStatusReports, QueryAccount, SubmitOrder, SubmitOrderList,
    },
};
use nautilus_core::{Params, UnixNanos, time::AtomicTime};
use nautilus_live::{ExecutionClientCore, ExecutionEventEmitter};
use nautilus_model::{
    accounts::AccountAny,
    enums::OmsType,
    identifiers::{AccountId, ClientId, InstrumentId, Venue, VenueOrderId},
    instruments::Instrument,
    orders::{Order, OrderAny},
    reports::{order::OrderStatusReport, position::PositionStatusReport},
    types::{AccountBalance, Currency, MarginBalance, Money},
};
use nautilus_network::http::Method;
use tokio_util::sync::CancellationToken;

use super::{parse::OrderSpec, reports::order_status_report};
use crate::{
    common::Market,
    config::SodexExecClientConfig,
    http::{
        BatchCost, account_reads::OrderRecord, CancelOrderRequest, ClientError, NewOrderRequest, OrderAck, SodexHttpClient,
        align_batch,
        requests::{CancelItem, ClientOrderId as VenueClientOrderId},
        spot::{SpotCancelItem, SpotCancelOrderRequest, SpotNewOrderRequest},
    },
    providers::{InstrumentCatalog, instrument_id_for, load_instruments, spawn_instrument_refresh},
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

impl std::fmt::Debug for SodexExecutionClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SodexExecutionClient")
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
             Registered there: [{}]. Either the wallet address is wrong — in which case the \
             account reads would have reported an empty account rather than failing — or the key \
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

    /// Denies an order the venue cannot honour as asked, and reports why.
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
    /// Spot cancels carry two client order ids — one naming the cancellation itself and one
    /// naming its target — so the caller must supply a distinct label for the request.
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
        // order allowance would make winding down a book compete with opening one.
        http.send_weighted(signed, BatchCost::for_batch(0).ip_weight, 0)
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
        self.emitter
            .set_sender(get_exec_event_sender());
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
        load_instruments(
            &self.http,
            self.config.market,
            self.core.venue,
            &self.catalog,
        )
        .await?;

        // Before anything else reads the account: prove the address is the right one.
        self.verify_wallet_owns_signing_key().await?;

        if let Some(task) = spawn_instrument_refresh(
            self.config.update_instruments_interval_mins,
            Arc::clone(&self.http),
            self.config.market,
            self.core.venue,
            Arc::clone(&self.catalog),
            self.cancellation.clone(),
            self.core.client_id,
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
        // a reconciled external order has no client order id it recognises.
        let reports = self.collect_order_reports().await?;

        Ok(reports.into_iter().find(|report| {
            cmd.venue_order_id
                .is_some_and(|wanted| wanted == report.venue_order_id)
                || cmd
                    .client_order_id
                    .is_some_and(|wanted| Some(wanted) == report.client_order_id)
        }))
    }

    async fn generate_position_status_reports(
        &self,
        _cmd: &GeneratePositionStatusReports,
    ) -> anyhow::Result<Vec<PositionStatusReport>> {
        match self.config.market {
            // Correct rather than missing: spot holds balances and has no positions, and the
            // venue does not serve the path at all.
            Market::Spot => Ok(Vec::new()),
            // The endpoint exists and answers, but its payload shape has never been observed —
            // reading it needs an open perps position, and typing it from the spot order shape by
            // analogy is what produced this integration's worst failures. Reporting an empty list
            // would claim the account is flat, so this says plainly that it does not know.
            Market::Perps => anyhow::bail!(
                "SoDEX perps position reports are not implemented: the payload shape of \
                 /accounts/{{wallet}}/positions has not been observed, and reporting an empty \
                 list would assert the account is flat when it may not be"
            ),
        }
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

        let submission = match Submission::build(
            &spec,
            self.config.market,
            self.venue_account_id,
            symbol_id,
        ) {
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

        get_runtime().spawn(async move {
            let submitted = submission.client_order_ids();
            let ts_event = clock.get_time_ns();

            match submission.send(&http).await {
                Ok(acks) => match align_batch(&submitted, acks) {
                    Ok(aligned) => report_submission(&emitter, &order, aligned.first(), ts_event),
                    // The response cannot be attributed to this order. Reporting an outcome
                    // anyway would be a guess about whether it is live.
                    Err(e) => emitter.emit_order_rejected(
                        &order,
                        &format!("venue response could not be matched to the order: {e}"),
                        ts_event,
                        false,
                    ),
                },
                Err(e) => {
                    emitter.emit_order_rejected(&order, &e.to_string(), ts_event, false);
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
            let reason =
                format!("SoDEX cannot enforce {contingency:?} contingency between orders");
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
    /// The client itself is not `Send` — its core holds the engine's cache — so a task cannot
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
/// The venue reports `total` and `locked`; Nautilus wants total, locked and free, and free is the
/// difference. Computing it rather than assuming `total` is free is the point: the locked part is
/// reserved against open orders and cannot be spent twice.
fn account_balance(
    balance: &crate::http::account_reads::CoinBalance,
) -> anyhow::Result<AccountBalance> {
    let currency = Currency::try_from_str(&balance.coin)
        .ok_or_else(|| anyhow::anyhow!("unknown currency {}", balance.coin))?;

    let total = money(&balance.total, currency)?;
    let locked = money(&balance.locked, currency)?;
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
    target: &nautilus_model::identifiers::ClientOrderId,
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
    use nautilus_model::identifiers::ClientOrderId;

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

    #[test]
    fn a_spot_submission_carries_the_symbol_on_the_order_item() {
        // Spot puts `symbolID` on each item; perps puts it once on the request. Crossing the
        // two is what the venue rejected as a missing required field on the live testnet.
        let Submission::Spot(request) =
            Submission::build(&spec(), Market::Spot, 60366, 1).unwrap()
        else {
            panic!("expected a spot submission");
        };

        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains(r#""orders":[{"symbolID":1"#), "{json}");
        assert_eq!(SpotNewOrderRequest::ENDPOINT, "/trade/orders/batch");
        assert_eq!(SpotNewOrderRequest::ACTION, "batchNewOrder");
    }

    #[test]
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

    #[test]
    fn a_submission_reports_the_ids_it_sent_for_alignment() {
        let submission = Submission::build(&spec(), Market::Perps, 60366, 7).unwrap();

        assert_eq!(
            submission.client_order_ids(),
            vec!["O-19700101-000000-001".to_string()]
        );
    }

    #[test]
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

    #[test]
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

    #[test]
    fn a_cancel_label_fits_the_venue_limit() {
        // Nautilus ids run to 27 characters and the timestamp adds 20 more, so an
        // unconditional concatenation would exceed the venue's 36 and be rejected.
        let long = ClientOrderId::from("O-20260910-120000-001-002-3");
        let label = cancel_label(&long, UnixNanos::from(1_767_972_900_123_456_789)).unwrap();

        assert!(label.as_str().len() <= 36, "{}", label.as_str());
        assert!(label.as_str().ends_with("-1767972900123456789"));
    }

    #[test]
    fn two_cancels_of_one_order_get_different_labels() {
        // Reusing a label would make the second cancellation indistinguishable from the
        // first in the venue's own records.
        let target = ClientOrderId::from("O-1");

        let first = cancel_label(&target, UnixNanos::from(1_000_000_000)).unwrap();
        let second = cancel_label(&target, UnixNanos::from(2_000_000_000)).unwrap();

        assert_ne!(first.as_str(), second.as_str());
    }

    #[test]
    fn a_cancel_label_drops_characters_the_venue_forbids() {
        // The venue accepts only `[0-9a-zA-Z_-]`. A dot or colon in the id would otherwise
        // fail at parse time and turn a routine cancel into a rejection.
        let target = ClientOrderId::from("O.1:2");

        let label = cancel_label(&target, UnixNanos::from(1_000_000_000)).unwrap();

        assert!(label.as_str().starts_with("O12-"), "{}", label.as_str());
    }
}
