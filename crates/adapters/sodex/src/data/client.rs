//! Live market data client.
//!
//! # Only bars, and only closed ones
//!
//! The venue's stream carries a candle channel; this client publishes from it and serves
//! historical bars over REST. Quotes, trades and order books are not implemented, so the
//! trait's defaults leave them logged and unhandled rather than silently pretending.
//!
//! Every push on the candle channel republishes the bar that is currently forming, so most
//! frames describe a bar whose high, low and close can still move. Only completed bars are
//! published. A strategy that acted on a forming bar would be deciding from values that were
//! not knowable at that timestamp, and its live results would not be comparable with any
//! backtest — the same look-ahead the historical path removes with `drop_forming_tail`.
//!
//! Completion cannot be read off the venue's `closed` flag alone. Observed on the live
//! testnet across two full bar periods: the flag was never set, and the bar simply rolled —
//! each push carried an open time one interval later than the last. A client that waited for
//! the flag would publish nothing at all while its connection looked healthy, which is the
//! worst failure available here, because it is indistinguishable from a market with no
//! trades. So the last push of each bar is held, and released when either the flag is set or
//! a push for a later bar proves the held one's window has elapsed. The flag stays
//! authoritative when present; the successor is the fallback evidence.
//!
//! The cost is one bar of latency: a bar is published when the next one starts rather than
//! the instant it closes. That is inherent to the venue's behaviour, not a choice — there is
//! no earlier moment at which completion is observable.
//!
//! # Pushes are matched to the subscription that asked for them
//!
//! A candle names its instrument and interval as text. Rebuilding a bar type from those two
//! strings would publish data for anything the venue happened to send, including a channel
//! nobody subscribed to. Instead each subscription records the exact bar type the engine
//! asked for, and a push with no matching entry is dropped.

use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use async_trait::async_trait;
use nautilus_common::{
    clients::DataClient,
    live::{get_runtime, runner::get_data_event_sender, task::TaskHandles},
    messages::{
        DataEvent,
        data::{
            BarsResponse, DataResponse, InstrumentResponse, InstrumentsResponse, RequestBars,
            RequestInstrument, RequestInstruments, SubscribeBars, SubscribeQuotes,
            SubscribeTrades, UnsubscribeBars, UnsubscribeQuotes, UnsubscribeTrades,
        },
    },
};
use nautilus_live::SocketControlFactory;
use nautilus_core::{
    UnixNanos,
    datetime::datetime_to_unix_nanos,
    time::{AtomicTime, get_atomic_clock_realtime},
};
use nautilus_model::{
    data::{BarType, Data},
    identifiers::{ClientId, InstrumentId, Venue},
    instruments::{Instrument, InstrumentAny},
};
use parking_lot::Mutex;
use tokio_util::sync::CancellationToken;

use super::{
    history::{BarRequest, fetch_bars},
    parse::{parse_completed_bar, parse_quote, parse_trade, spec_to_interval},
};
use crate::{
    common::Market,
    config::SodexDataClientConfig,
    http::SodexHttpClient,
    providers::{InstrumentCatalog, load_instruments, spawn_instrument_refresh},
    websocket::{Candle, SodexWebSocketClient, SodexWsEvent, Subscription},
};

/// Identifies a candle feed the way the venue's push frames do.
type FeedKey = (String, String);

/// The most recent push for one feed, held until its bar is provably complete.
type Pending = HashMap<FeedKey, Candle>;

/// What a subscribed symbol needs before its frames can become ticks.
///
/// The precisions come from the instrument definition rather than from the text of each
/// value. The venue writes one tick size several ways — `"77379"` and `"77378.5"` are both
/// this instrument's prices — and Nautilus rejects a quote whose two sides disagree about
/// precision, so the declared precision is the only consistent source.
#[derive(Debug, Clone, Copy)]
struct TickFeed {
    instrument_id: InstrumentId,
    price_precision: u8,
    size_precision: u8,
}

/// Everything the stream task needs to turn a push into engine data.
///
/// A push whose symbol is absent from the relevant map is dropped: rebuilding an identity
/// from the frame's own text would publish data for feeds nobody subscribed to.
#[derive(Debug, Default)]
struct Feeds {
    bars: HashMap<FeedKey, BarType>,
    quotes: HashMap<String, TickFeed>,
    trades: HashMap<String, TickFeed>,
}

/// Live market data client for one SoDEX engine.
pub struct SodexDataClient {
    client_id: ClientId,
    venue: Venue,
    market: Market,
    config: SodexDataClientConfig,
    http: Arc<SodexHttpClient>,
    /// The published instrument set. Readers are synchronous trait methods, so they must not
    /// block on a network call; the refresh task publishes whole snapshots here instead.
    catalog: Arc<InstrumentCatalog>,
    ws: Option<Arc<SodexWebSocketClient>>,
    /// What each subscribed feed maps to, keyed as the venue's push frames are.
    feeds: Arc<Mutex<Feeds>>,
    is_connected: Arc<AtomicBool>,
    tasks: TaskHandles,
    cancellation: CancellationToken,
    socket_factory: SocketControlFactory,
    data_sender: tokio::sync::mpsc::UnboundedSender<DataEvent>,
    clock: &'static AtomicTime,
}

impl std::fmt::Debug for SodexDataClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SodexDataClient")
            .field("client_id", &self.client_id)
            .field("venue", &self.venue)
            .field("connected", &self.is_connected.load(Ordering::Relaxed))
            .field("bar_feeds", &self.feeds.lock().bars.len())
            .field("instruments", &self.catalog.len())
            .finish()
    }
}

impl SodexDataClient {
    /// Creates a client for one engine on one network.
    ///
    /// # Errors
    ///
    /// Returns an error if the HTTP client cannot be built.
    pub fn new(client_id: ClientId, config: SodexDataClientConfig) -> anyhow::Result<Self> {
        let http = SodexHttpClient::public_with_options(
            config.network,
            config.market,
            config.timeout_secs,
            None,
        )
        .map_err(|e| anyhow::anyhow!("failed to build HTTP client: {e}"))?;

        Ok(Self {
            client_id,
            venue: config.venue(),
            market: config.market,
            http: Arc::new(http),
            catalog: Arc::new(InstrumentCatalog::new()),
            ws: None,
            feeds: Arc::new(Mutex::new(Feeds::default())),
            is_connected: Arc::new(AtomicBool::new(false)),
            tasks: TaskHandles::default(),
            cancellation: CancellationToken::new(),
            socket_factory: SocketControlFactory::new(client_id, Some(config.venue())),
            data_sender: get_data_event_sender(),
            clock: get_atomic_clock_realtime(),
            config,
        })
    }

    fn ws_client(&self) -> anyhow::Result<Arc<SodexWebSocketClient>> {
        self.ws
            .as_ref()
            .map(Arc::clone)
            .ok_or_else(|| anyhow::anyhow!("SoDEX data client is not connected"))
    }

    /// The identity and precisions a symbol's ticks are built with.
    ///
    /// # Errors
    ///
    /// Returns an error when the instrument has not been loaded. Defaulting the precisions
    /// would produce ticks that claim a tick size the instrument does not have.
    fn tick_feed(&self, instrument_id: &InstrumentId) -> anyhow::Result<TickFeed> {
        let instrument = self
            .catalog
            .find(instrument_id)
            .ok_or_else(|| anyhow::anyhow!("{instrument_id} has not been loaded"))?;

        Ok(TickFeed {
            instrument_id: *instrument_id,
            price_precision: instrument.price_precision(),
            size_precision: instrument.size_precision(),
        })
    }

    fn spawn_subscribe(
        &self,
        ws: Arc<SodexWebSocketClient>,
        subscription: Subscription,
        subject: impl std::fmt::Display + Send + 'static,
    ) {
        get_runtime().spawn(async move {
            if let Err(e) = ws.subscribe(subscription).await {
                log::error!("sodex_subscribe_failed subject={subject} error={e}");
            }
        });
    }

    fn spawn_unsubscribe(
        &self,
        ws: Arc<SodexWebSocketClient>,
        subscription: Subscription,
        subject: impl std::fmt::Display + Send + 'static,
    ) {
        get_runtime().spawn(async move {
            if let Err(e) = ws.unsubscribe(&subscription).await {
                log::error!("sodex_unsubscribe_failed subject={subject} error={e}");
            }
        });
    }

    fn send(&self, event: DataEvent) {
        if let Err(e) = self.data_sender.send(event) {
            log::error!("sodex_data_event_undeliverable error={e}");
        }
    }
}

/// The venue symbol and interval a bar type maps to on the stream.
///
/// # Errors
///
/// Returns an error if this engine has no interval for the bar specification. Refusing here
/// keeps an unsupported request from becoming a subscription the venue silently never fills.
fn feed_key(bar_type: &BarType, market: Market) -> anyhow::Result<FeedKey> {
    let spec = bar_type.spec();
    let interval = spec_to_interval(&spec, market)
        .map_err(|e| anyhow::anyhow!("unsupported bar specification {spec}: {e}"))?;
    Ok((bar_type.instrument_id().symbol.to_string(), interval.to_string()))
}

/// Publishes completed bars, quotes and trades from the stream, and reports what the venue
/// refused.
async fn run_stream(
    mut events: tokio::sync::mpsc::UnboundedReceiver<SodexWsEvent>,
    feeds: Arc<Mutex<Feeds>>,
    sender: tokio::sync::mpsc::UnboundedSender<DataEvent>,
    clock: &'static AtomicTime,
) {
    let mut pending = Pending::new();

    while let Some(event) = events.recv().await {
        let published = match event {
            SodexWsEvent::Candle(candle) => publish_candle(&feeds, &mut pending, *candle, clock),
            SodexWsEvent::Trade(trade) => {
                let Some(feed) = feeds.lock().trades.get(&trade.symbol).copied() else {
                    log::debug!("sodex_trade_unsubscribed symbol={}", trade.symbol);
                    continue;
                };
                match parse_trade(
                    &trade,
                    feed.instrument_id,
                    feed.price_precision,
                    feed.size_precision,
                    clock.get_time_ns(),
                ) {
                    Ok(tick) => vec![Data::Trade(tick)],
                    Err(e) => {
                        log::error!("sodex_trade_unparsed symbol={} error={e}", trade.symbol);
                        continue;
                    }
                }
            }
            SodexWsEvent::Ticker(ticker) => {
                let Some(feed) = feeds.lock().quotes.get(&ticker.symbol).copied() else {
                    log::debug!("sodex_ticker_unsubscribed symbol={}", ticker.symbol);
                    continue;
                };
                match parse_quote(
                    &ticker,
                    feed.instrument_id,
                    feed.price_precision,
                    feed.size_precision,
                    clock.get_time_ns(),
                ) {
                    Ok(tick) => vec![Data::Quote(tick)],
                    Err(e) => {
                        log::error!("sodex_quote_unparsed symbol={} error={e}", ticker.symbol);
                        continue;
                    }
                }
            }
            SodexWsEvent::RequestRejected {
                op,
                subscription,
                reason,
            } => {
                // A refused subscribe leaves a feed permanently silent, which otherwise looks
                // exactly like a market with no trades.
                log::error!(
                    "sodex_stream_request_rejected op={op:?} subscription={subscription:?} {reason}"
                );
                continue;
            }
            SodexWsEvent::Reconnected => {
                // The held bars belong to the connection that just went away. A gap in the
                // feed means the held snapshot may no longer be that bar's final state, so
                // publishing it later would present a partial bar as a complete one.
                pending.clear();
                log::info!("sodex_stream_reconnected pending_bars_dropped");
                continue;
            }
        };

        for data in published {
            if sender.send(DataEvent::Data(data)).is_err() {
                log::debug!("sodex_data_consumer_gone");
                return;
            }
        }
    }
}

/// Turns a candle push into whatever bars it completed.
fn publish_candle(
    feeds: &Arc<Mutex<Feeds>>,
    pending: &mut Pending,
    candle: Candle,
    clock: &'static AtomicTime,
) -> Vec<Data> {
    let key = (candle.symbol.clone(), candle.interval.clone());
    let Some(bar_type) = feeds.lock().bars.get(&key).copied() else {
        log::debug!(
            "sodex_candle_unsubscribed symbol={} interval={}",
            candle.symbol,
            candle.interval
        );
        return Vec::new();
    };

    advance(pending, key, candle)
        .into_iter()
        .filter_map(
            |completed| match parse_completed_bar(&completed, bar_type, clock.get_time_ns()) {
                Ok(bar) => Some(Data::Bar(bar)),
                Err(e) => {
                    log::error!("sodex_bar_unparsed bar_type={bar_type} error={e}");
                    None
                }
            },
        )
        .collect()
}

/// Folds one push into the held state, returning whatever became complete.
///
/// Completion has two sources, and this is where they are reconciled: the venue's own flag,
/// and the arrival of a push for a later bar. A push for an *earlier* bar than the one held
/// is discarded — the venue republishes the forming bar constantly, so an out-of-order frame
/// would otherwise rewrite a bar that has already been published.
fn advance(pending: &mut Pending, key: FeedKey, candle: Candle) -> Vec<Candle> {
    if candle.is_final() {
        // The flag is authoritative. Anything still held for this feed is at or before this
        // bar, so it is superseded rather than published twice.
        let held = pending.remove(&key);
        let mut completed = Vec::new();
        if let Some(held) = held
            && held.open_time_ms < candle.open_time_ms
        {
            completed.push(held);
        }
        completed.push(candle);
        return completed;
    }

    match pending.insert(key.clone(), candle) {
        Some(held) => {
            let candle = &pending[&key];
            if held.open_time_ms < candle.open_time_ms {
                // The held bar's window has elapsed: its successor is already forming.
                vec![held]
            } else if held.open_time_ms > candle.open_time_ms {
                log::debug!(
                    "sodex_candle_out_of_order held={} received={}",
                    held.open_time_ms,
                    candle.open_time_ms
                );
                pending.insert(key, held);
                Vec::new()
            } else {
                // A later snapshot of the same forming bar, which has just replaced it.
                Vec::new()
            }
        }
        None => Vec::new(),
    }
}

#[async_trait(?Send)]
impl DataClient for SodexDataClient {
    fn client_id(&self) -> ClientId {
        self.client_id
    }

    fn venue(&self) -> Option<Venue> {
        Some(self.venue)
    }

    fn start(&mut self) -> anyhow::Result<()> {
        log::info!(
            "sodex_data_client_start client_id={} venue={} network={:?}",
            self.client_id,
            self.venue,
            self.config.network
        );
        Ok(())
    }

    fn stop(&mut self) -> anyhow::Result<()> {
        log::info!("sodex_data_client_stop client_id={}", self.client_id);
        // Cancel first, then abort: the refresh loop selects on the token and will leave its
        // own await point, which an abort alone could cut mid-request.
        self.cancellation.cancel();
        self.tasks.abort_all();
        self.is_connected.store(false, Ordering::Relaxed);
        Ok(())
    }

    fn reset(&mut self) -> anyhow::Result<()> {
        self.stop()?;

        // Dropping the recorded feeds without closing the socket would leave the venue
        // pushing candles this client could no longer match to a bar type. The trait's reset
        // is synchronous, so the close is handed to the runtime rather than skipped.
        if let Some(ws) = self.ws.take() {
            get_runtime().spawn(async move { ws.close().await });
        }
        *self.feeds.lock() = Feeds::default();
        // A fresh token, so a later connect is not cancelled by this stop.
        self.cancellation = CancellationToken::new();
        Ok(())
    }

    fn dispose(&mut self) -> anyhow::Result<()> {
        self.stop()
    }

    fn is_connected(&self) -> bool {
        self.is_connected.load(Ordering::Relaxed)
    }

    fn is_disconnected(&self) -> bool {
        !self.is_connected()
    }

    async fn connect(&mut self) -> anyhow::Result<()> {
        if self.is_connected() {
            return Ok(());
        }

        // Instruments first: a bar cannot be published for an instrument the engine has never
        // seen, and the symbol ids loaded here are what order submission later needs.
        load_instruments(&self.http, self.market, self.venue, &self.catalog).await?;

        let ws = Arc::new(
            SodexWebSocketClient::new(self.config.network, self.config.market)
                .with_socket_control(self.socket_factory.control("sodex-data-streams")),
        );
        let events = ws.connect().await?;

        self.tasks.push(get_runtime().spawn(run_stream(
            events,
            Arc::clone(&self.feeds),
            self.data_sender.clone(),
            self.clock,
        )));
        if let Some(task) = spawn_instrument_refresh(
            self.config.update_instruments_interval_mins,
            Arc::clone(&self.http),
            self.market,
            self.venue,
            Arc::clone(&self.catalog),
            self.cancellation.clone(),
            self.client_id,
        ) {
            self.tasks.push(task);
        }
        self.ws = Some(ws);
        self.is_connected.store(true, Ordering::Relaxed);

        log::info!(
            "sodex_data_client_connected venue={} instruments={}",
            self.venue,
            self.catalog.len()
        );
        Ok(())
    }

    async fn disconnect(&mut self) -> anyhow::Result<()> {
        if let Some(ws) = self.ws.take() {
            ws.close().await;
        }
        self.cancellation.cancel();
        self.tasks.abort_all();
        *self.feeds.lock() = Feeds::default();
        self.is_connected.store(false, Ordering::Relaxed);
        Ok(())
    }

    fn subscribe_bars(&mut self, cmd: SubscribeBars) -> anyhow::Result<()> {
        let bar_type = cmd.bar_type;
        let (symbol, interval) = feed_key(&bar_type, self.market)?;
        // Resolved before anything is recorded, so a call made while disconnected leaves no
        // entry claiming a feed that was never requested.
        let ws = self.ws_client()?;

        // Recorded before the request goes out: the venue can push the first candle before it
        // acknowledges the subscribe, and a push with no entry here would be dropped.
        self.feeds
            .lock()
            .bars
            .insert((symbol.clone(), interval.clone()), bar_type);

        self.spawn_subscribe(ws, Subscription::candles(symbol, interval), bar_type);
        Ok(())
    }

    fn unsubscribe_bars(&mut self, cmd: &UnsubscribeBars) -> anyhow::Result<()> {
        let bar_type = cmd.bar_type;
        let (symbol, interval) = feed_key(&bar_type, self.market)?;
        let ws = self.ws_client()?;

        self.feeds
            .lock()
            .bars
            .remove(&(symbol.clone(), interval.clone()));

        self.spawn_unsubscribe(ws, Subscription::candles(symbol, interval), bar_type);
        Ok(())
    }

    fn subscribe_quotes(&mut self, cmd: SubscribeQuotes) -> anyhow::Result<()> {
        let instrument_id = cmd.instrument_id;
        let feed = self.tick_feed(&instrument_id)?;
        let ws = self.ws_client()?;
        let symbol = instrument_id.symbol.to_string();

        self.feeds.lock().quotes.insert(symbol.clone(), feed);

        self.spawn_subscribe(ws, Subscription::ticker(symbol), instrument_id);
        Ok(())
    }

    fn unsubscribe_quotes(&mut self, cmd: &UnsubscribeQuotes) -> anyhow::Result<()> {
        let instrument_id = cmd.instrument_id;
        let ws = self.ws_client()?;
        let symbol = instrument_id.symbol.to_string();

        self.feeds.lock().quotes.remove(&symbol);

        self.spawn_unsubscribe(ws, Subscription::ticker(symbol), instrument_id);
        Ok(())
    }

    fn subscribe_trades(&mut self, cmd: SubscribeTrades) -> anyhow::Result<()> {
        let instrument_id = cmd.instrument_id;
        let feed = self.tick_feed(&instrument_id)?;
        let ws = self.ws_client()?;
        let symbol = instrument_id.symbol.to_string();

        self.feeds.lock().trades.insert(symbol.clone(), feed);

        self.spawn_subscribe(ws, Subscription::trades(symbol), instrument_id);
        Ok(())
    }

    fn unsubscribe_trades(&mut self, cmd: &UnsubscribeTrades) -> anyhow::Result<()> {
        let instrument_id = cmd.instrument_id;
        let ws = self.ws_client()?;
        let symbol = instrument_id.symbol.to_string();

        self.feeds.lock().trades.remove(&symbol);

        self.spawn_unsubscribe(ws, Subscription::trades(symbol), instrument_id);
        Ok(())
    }

    fn request_bars(&self, request: RequestBars) -> anyhow::Result<()> {
        let http = Arc::clone(&self.http);
        let sender = self.data_sender.clone();
        let market = self.market;
        let clock = self.clock;
        let client_id = request.client_id.unwrap_or(self.client_id);
        let bar_type = request.bar_type;
        let start_nanos = datetime_to_unix_nanos(request.start);
        let end_nanos = datetime_to_unix_nanos(request.end);
        let params = request.params;
        let request_id = request.request_id;

        let bar_request = BarRequest {
            instrument_id: bar_type.instrument_id(),
            spec: bar_type.spec(),
            start_ms: start_nanos.map(unix_nanos_to_millis),
            end_ms: end_nanos.map(unix_nanos_to_millis),
            limit: request.limit.map(|n| u32::try_from(n.get()).unwrap_or(u32::MAX)),
        };

        get_runtime().spawn(async move {
            match fetch_bars(&http, market, &bar_request).await {
                Ok(bars) => {
                    let response = DataResponse::Bars(BarsResponse::new(
                        request_id,
                        client_id,
                        bar_type,
                        bars,
                        start_nanos,
                        end_nanos,
                        clock.get_time_ns(),
                        params,
                    ));
                    if let Err(e) = sender.send(DataEvent::Response(response)) {
                        log::error!("sodex_bars_response_undeliverable error={e}");
                    }
                }
                Err(e) => log::error!("sodex_bars_request_failed bar_type={bar_type} error={e}"),
            }
        });
        Ok(())
    }

    fn request_instruments(&self, request: RequestInstruments) -> anyhow::Result<()> {
        let instruments: Vec<InstrumentAny> = self.catalog.all();
        let response = DataResponse::Instruments(InstrumentsResponse::new(
            request.request_id,
            request.client_id.unwrap_or(self.client_id),
            self.venue,
            instruments,
            datetime_to_unix_nanos(request.start),
            datetime_to_unix_nanos(request.end),
            self.clock.get_time_ns(),
            request.params,
        ));
        self.send(DataEvent::Response(response));
        Ok(())
    }

    fn request_instrument(&self, request: RequestInstrument) -> anyhow::Result<()> {
        let Some(instrument) = self.catalog.find(&request.instrument_id) else {
            log::warn!("sodex_instrument_unknown id={}", request.instrument_id);
            return Ok(());
        };

        let response = DataResponse::Instrument(Box::new(InstrumentResponse::new(
            request.request_id,
            request.client_id.unwrap_or(self.client_id),
            instrument.id(),
            instrument,
            datetime_to_unix_nanos(request.start),
            datetime_to_unix_nanos(request.end),
            self.clock.get_time_ns(),
            request.params,
        )));
        self.send(DataEvent::Response(response));
        Ok(())
    }
}

/// Converts a Nautilus timestamp into the milliseconds the venue's REST API expects.
const fn unix_nanos_to_millis(ts: UnixNanos) -> u64 {
    ts.as_u64() / 1_000_000
}

#[cfg(test)]
mod tests {
    use nautilus_model::{
        data::BarSpecification,
        enums::{AggregationSource, BarAggregation, PriceType},
        identifiers::{InstrumentId, Symbol},
    };

    use super::*;
    use crate::{
        config::SODEX_PERPS,
        websocket::{Ticker, Trade},
    };

    fn bar_type(step: usize, aggregation: BarAggregation) -> BarType {
        BarType::new(
            InstrumentId::new(Symbol::from("vBTC_vUSDC"), Venue::from(SODEX_PERPS)),
            BarSpecification::new(step, aggregation, PriceType::Last),
            AggregationSource::External,
        )
    }

    fn candle(closed: bool) -> Candle {
        candle_at(1_767_972_900_000, closed)
    }

    fn candle_at(open_time_ms: u64, closed: bool) -> Candle {
        Candle {
            open_time_ms,
            update_time_ms: 1_767_972_960_000,
            symbol: "vBTC_vUSDC".to_string(),
            interval: "1m".to_string(),
            open: "91869".to_string(),
            high: "91982".to_string(),
            low: "91869".to_string(),
            close: "91976".to_string(),
            volume: "4.12298".to_string(),
            quote_volume: "379148.6798".to_string(),
            trade_count: 12,
            closed,
        }
    }

    fn feeds_with(entry: BarType) -> Arc<Mutex<Feeds>> {
        let mut feeds = Feeds::default();
        feeds
            .bars
            .insert(("vBTC_vUSDC".to_string(), "1m".to_string()), entry);
        Arc::new(Mutex::new(feeds))
    }

    fn tick_feeds() -> Arc<Mutex<Feeds>> {
        let feed = TickFeed {
            instrument_id: InstrumentId::new(
                Symbol::from("vBTC_vUSDC"),
                Venue::from(SODEX_PERPS),
            ),
            price_precision: 1,
            size_precision: 5,
        };
        let mut feeds = Feeds::default();
        feeds.quotes.insert("vBTC_vUSDC".to_string(), feed);
        feeds.trades.insert("vBTC_vUSDC".to_string(), feed);
        Arc::new(Mutex::new(feeds))
    }

    /// Drives the stream task over one batch of events and collects what it published.
    async fn publish(
        events: Vec<SodexWsEvent>,
        feeds: Arc<Mutex<Feeds>>,
    ) -> Vec<DataEvent> {
        let (events_tx, events_rx) = tokio::sync::mpsc::unbounded_channel();
        for event in events {
            events_tx.send(event).unwrap();
        }
        drop(events_tx);

        let (data_tx, mut data_rx) = tokio::sync::mpsc::unbounded_channel();
        run_stream(events_rx, feeds, data_tx, get_atomic_clock_realtime()).await;

        let mut published = Vec::new();
        while let Ok(event) = data_rx.try_recv() {
            published.push(event);
        }
        published
    }

    #[test]
    fn a_bar_type_maps_to_the_symbol_and_interval_the_venue_echoes() {
        let key = feed_key(&bar_type(1, BarAggregation::Minute), Market::Perps).unwrap();

        assert_eq!(key, ("vBTC_vUSDC".to_string(), "1m".to_string()));
    }

    #[test]
    fn a_bar_specification_this_engine_cannot_serve_is_refused_at_subscribe_time() {
        // Accepting it would register a feed key the venue never pushes, leaving the strategy
        // waiting on a subscription that was never going to arrive.
        assert!(feed_key(&bar_type(3, BarAggregation::Minute), Market::Perps).is_err());
    }

    #[tokio::test]
    async fn a_closed_bar_is_published() {
        let subscribed = bar_type(1, BarAggregation::Minute);

        let published = publish(
            vec![SodexWsEvent::Candle(Box::new(candle(true)))],
            feeds_with(subscribed),
        )
        .await;

        assert_eq!(published.len(), 1);
        let DataEvent::Data(Data::Bar(bar)) = &published[0] else {
            panic!("expected a bar");
        };
        assert_eq!(bar.bar_type, subscribed);
        assert_eq!(bar.close.to_string(), "91976");
    }

    #[tokio::test]
    async fn a_bar_is_published_once_its_successor_starts() {
        // The venue was observed never to set the closed flag: the bar simply rolls. Waiting
        // for the flag would publish nothing at all, so a later open time is what proves the
        // held bar's window has elapsed.
        let subscribed = bar_type(1, BarAggregation::Minute);

        let published = publish(
            vec![
                SodexWsEvent::Candle(Box::new(candle_at(1_767_972_900_000, false))),
                SodexWsEvent::Candle(Box::new(candle_at(1_767_972_900_000, false))),
                SodexWsEvent::Candle(Box::new(candle_at(1_767_972_960_000, false))),
            ],
            feeds_with(subscribed),
        )
        .await;

        assert_eq!(published.len(), 1);
        let DataEvent::Data(Data::Bar(bar)) = &published[0] else {
            panic!("expected a bar");
        };
        // The first bar, not the one that is still forming.
        assert_eq!(bar.ts_event.as_u64(), 1_767_972_900_000 * 1_000_000);
    }

    #[tokio::test]
    async fn the_bar_still_forming_is_never_published() {
        // Two bars start, so only the first is complete; the second is still open when the
        // stream ends and must not be released by the ending itself.
        let published = publish(
            vec![
                SodexWsEvent::Candle(Box::new(candle_at(1_767_972_900_000, false))),
                SodexWsEvent::Candle(Box::new(candle_at(1_767_972_960_000, false))),
                SodexWsEvent::Candle(Box::new(candle_at(1_767_972_960_000, false))),
            ],
            feeds_with(bar_type(1, BarAggregation::Minute)),
        )
        .await;

        assert_eq!(published.len(), 1);
    }

    #[tokio::test]
    async fn a_bar_is_published_once_even_when_the_flag_arrives_after_the_roll() {
        // Belt and braces: if the venue starts setting the flag, a bar already released by
        // its successor must not be published a second time.
        let published = publish(
            vec![
                SodexWsEvent::Candle(Box::new(candle_at(1_767_972_900_000, false))),
                SodexWsEvent::Candle(Box::new(candle_at(1_767_972_960_000, false))),
                SodexWsEvent::Candle(Box::new(candle_at(1_767_972_960_000, true))),
            ],
            feeds_with(bar_type(1, BarAggregation::Minute)),
        )
        .await;

        assert_eq!(published.len(), 2);
        let opens: Vec<u64> = published
            .iter()
            .map(|event| {
                let DataEvent::Data(Data::Bar(bar)) = event else {
                    panic!("expected a bar");
                };
                bar.ts_event.as_u64() / 1_000_000
            })
            .collect();
        assert_eq!(opens, vec![1_767_972_900_000, 1_767_972_960_000]);
    }

    #[tokio::test]
    async fn an_out_of_order_push_does_not_republish_a_finished_bar() {
        // The venue republishes constantly; a late frame for an earlier bar must not rewrite
        // one the engine has already consumed.
        let published = publish(
            vec![
                SodexWsEvent::Candle(Box::new(candle_at(1_767_972_960_000, false))),
                SodexWsEvent::Candle(Box::new(candle_at(1_767_972_900_000, false))),
                SodexWsEvent::Candle(Box::new(candle_at(1_767_972_960_000, false))),
            ],
            feeds_with(bar_type(1, BarAggregation::Minute)),
        )
        .await;

        assert!(published.is_empty());
    }

    #[tokio::test]
    async fn a_reconnect_drops_the_held_bar_rather_than_completing_it_later() {
        // The gap means the held snapshot may not be that bar's final state. Publishing it
        // after the reconnect would present a partial bar as a complete one.
        let published = publish(
            vec![
                SodexWsEvent::Candle(Box::new(candle_at(1_767_972_900_000, false))),
                SodexWsEvent::Reconnected,
                SodexWsEvent::Candle(Box::new(candle_at(1_767_972_960_000, false))),
            ],
            feeds_with(bar_type(1, BarAggregation::Minute)),
        )
        .await;

        assert!(published.is_empty());
    }

    #[tokio::test]
    async fn a_forming_bar_is_not_published() {
        // The venue republishes the forming bar on every block. Publishing it would hand the
        // strategy an open, high, low and close that were not yet final at that timestamp,
        // and nothing yet proves this one is over.
        let published = publish(
            vec![SodexWsEvent::Candle(Box::new(candle(false)))],
            feeds_with(bar_type(1, BarAggregation::Minute)),
        )
        .await;

        assert!(published.is_empty());
    }

    #[tokio::test]
    async fn a_push_for_an_unsubscribed_feed_is_dropped() {
        // Rebuilding a bar type from the candle's own text would publish data for anything
        // the venue sent, including a feed nobody asked for.
        let published = publish(
            vec![SodexWsEvent::Candle(Box::new(candle(true)))],
            Arc::new(Mutex::new(Feeds::default())),
        )
        .await;

        assert!(published.is_empty());
    }

    fn ticker() -> Ticker {
        // The venue's own frame, with the mixed precision that makes this worth pinning: a
        // bid of "77378.5" beside an ask of "77379".
        Ticker {
            event_time_ms: 1_789_051_618_837,
            symbol: "vBTC_vUSDC".to_string(),
            last_price: "77341".to_string(),
            last_quantity: "0.00111".to_string(),
            weighted_average_price: "79419.4852395990663188".to_string(),
            ask_price: "77379".to_string(),
            ask_quantity: "0.00102".to_string(),
            bid_price: "77378.5".to_string(),
            bid_quantity: "0.00124".to_string(),
            price_change: "-1488".to_string(),
            price_change_percent: -1.887_630_186_860_17,
            open: "78829".to_string(),
            high: "89000".to_string(),
            low: "76760".to_string(),
            volume: "0.21849".to_string(),
            quote_volume: "17352.36333".to_string(),
            window_open_ms: 1_788_965_220_000,
            window_close_ms: 1_789_051_616_322,
        }
    }

    fn trade(side: crate::common::enums::OrderSide) -> Trade {
        Trade {
            event_time_ms: 1_789_051_802_631,
            trade_time_ms: 1_789_051_802_606,
            trade_id: 3_110_013,
            symbol: "vBTC_vUSDC".to_string(),
            side,
            price: "77304".to_string(),
            quantity: "0.00001".to_string(),
            buyer_account_id: 2140,
            seller_account_id: 1001,
        }
    }

    #[tokio::test]
    async fn a_ticker_becomes_a_quote_with_both_sides_at_one_precision() {
        // The venue writes the same tick size two ways. Parsing each side's precision from
        // its own text would make this pair look inconsistent and the engine would reject it.
        let published = publish(
            vec![SodexWsEvent::Ticker(Box::new(ticker()))],
            tick_feeds(),
        )
        .await;

        assert_eq!(published.len(), 1);
        let DataEvent::Data(Data::Quote(quote)) = &published[0] else {
            panic!("expected a quote");
        };
        assert_eq!(quote.bid_price.to_string(), "77378.5");
        assert_eq!(quote.ask_price.to_string(), "77379.0");
        assert_eq!(quote.bid_price.precision, quote.ask_price.precision);
    }

    #[tokio::test]
    async fn a_trade_carries_the_aggressing_side_and_its_own_timestamp() {
        // `S` names the taker: every observed BUY printed at the ask. And the tick is stamped
        // with the trade's time, not the frame's, which moves with the network.
        let published = publish(
            vec![SodexWsEvent::Trade(Box::new(trade(
                crate::common::enums::OrderSide::Buy,
            )))],
            tick_feeds(),
        )
        .await;

        assert_eq!(published.len(), 1);
        let DataEvent::Data(Data::Trade(tick)) = &published[0] else {
            panic!("expected a trade");
        };
        assert_eq!(tick.aggressor_side, nautilus_model::enums::AggressorSide::Buy);
        assert_eq!(tick.trade_id.to_string(), "3110013");
        assert_eq!(tick.ts_event.as_u64(), 1_789_051_802_606 * 1_000_000);
    }

    #[tokio::test]
    async fn a_sell_trade_is_the_seller_aggressing() {
        let published = publish(
            vec![SodexWsEvent::Trade(Box::new(trade(
                crate::common::enums::OrderSide::Sell,
            )))],
            tick_feeds(),
        )
        .await;

        let DataEvent::Data(Data::Trade(tick)) = &published[0] else {
            panic!("expected a trade");
        };
        assert_eq!(
            tick.aggressor_side,
            nautilus_model::enums::AggressorSide::Sell
        );
    }

    #[tokio::test]
    async fn ticks_for_unsubscribed_symbols_are_dropped() {
        // Each channel keeps its own registry, so subscribing to trades must not start
        // publishing quotes for the same symbol.
        let mut feeds = Feeds::default();
        feeds.trades.insert(
            "vBTC_vUSDC".to_string(),
            TickFeed {
                instrument_id: InstrumentId::new(
                    Symbol::from("vBTC_vUSDC"),
                    Venue::from(SODEX_PERPS),
                ),
                price_precision: 1,
                size_precision: 5,
            },
        );

        let published = publish(
            vec![SodexWsEvent::Ticker(Box::new(ticker()))],
            Arc::new(Mutex::new(feeds)),
        )
        .await;

        assert!(published.is_empty());
    }

    #[tokio::test]
    async fn a_rejection_publishes_no_data_and_does_not_stop_the_stream() {
        let published = publish(
            vec![
                SodexWsEvent::RequestRejected {
                    op: crate::websocket::Op::Subscribe,
                    subscription: None,
                    reason: "unknown symbol".to_string(),
                },
                SodexWsEvent::Candle(Box::new(candle(true))),
            ],
            feeds_with(bar_type(1, BarAggregation::Minute)),
        )
        .await;

        assert_eq!(published.len(), 1);
    }
}
