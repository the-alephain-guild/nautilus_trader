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

//! Reference-mid quotes for Papertrade instruments; no synthetic liquidity is published.

use crate::{
    api::PapertradeApi,
    config::{PAPERTRADE, PapertradeDataClientConfig},
    reports,
    wire::Market,
};
use anyhow::{Result, ensure};
use async_trait::async_trait;
use nautilus_common::{
    clients::DataClient,
    live::{get_runtime, runner::get_data_event_sender},
    messages::{
        DataEvent,
        data::{
            DataResponse, InstrumentResponse, InstrumentsResponse, RequestInstrument,
            RequestInstruments, SubscribeQuotes, UnsubscribeQuotes,
        },
    },
};
use nautilus_core::{
    UnixNanos,
    time::{AtomicTime, get_atomic_clock_realtime},
};
use nautilus_model::{
    data::{Data, QuoteTick},
    identifiers::{ClientId, InstrumentId, Venue},
    instruments::InstrumentAny,
    types::{Price, Quantity, fixed::FIXED_PRECISION},
};
use parking_lot::RwLock;
use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio_util::sync::CancellationToken;

#[derive(Debug)]
pub struct PapertradeDataClient {
    id: ClientId,
    config: PapertradeDataClientConfig,
    api: PapertradeApi,
    markets: Arc<RwLock<HashMap<InstrumentId, Market>>>,
    subscriptions: Arc<RwLock<HashSet<InstrumentId>>>,
    connected: Arc<AtomicBool>,
    cancellation: CancellationToken,
    sender: Option<tokio::sync::mpsc::UnboundedSender<DataEvent>>,
    clock: &'static AtomicTime,
}
impl PapertradeDataClient {
    pub fn new(id: ClientId, config: PapertradeDataClientConfig) -> Result<Self> {
        ensure!(
            config.poll_interval_ms >= 100,
            "Reference polling interval must be at least 100ms"
        );
        Ok(Self {
            id,
            api: PapertradeApi::new(config.origin.parse()?, config.rpc_url.parse()?, 1)?
                .with_reference(config.reference_info_url.parse()?),
            config,
            markets: Arc::new(RwLock::new(HashMap::new())),
            subscriptions: Arc::new(RwLock::new(HashSet::new())),
            connected: Arc::new(AtomicBool::new(false)),
            cancellation: CancellationToken::new(),
            sender: None,
            clock: get_atomic_clock_realtime(),
        })
    }
    fn send(&self, event: DataEvent) -> Result<()> {
        self.sender
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Data client not started"))?
            .send(event)
            .map_err(|_| anyhow::anyhow!("Data engine receiver closed"))
    }
}
async fn reference_quote(
    api: &PapertradeApi,
    market: &Market,
    clock: &'static AtomicTime,
) -> Result<QuoteTick> {
    let (mid, time) = if market.terminal {
        let bid = market
            .frozen_bid
            .ok_or_else(|| anyhow::anyhow!("Missing frozen bid"))?;
        let ask = market
            .frozen_ask
            .ok_or_else(|| anyhow::anyhow!("Missing frozen ask"))?;
        (
            (reports::market_price(market, bid)? + reports::market_price(market, ask)?)
                / rust_decimal::Decimal::from(2),
            clock.get_time_ns().as_u64() / 1_000_000,
        )
    } else {
        api.reference_mid(&market.symbol).await?
    };
    let precision = u8::try_from(market.price_scale.to_string().len() - 1)?;
    let price = Price::from_decimal_dp(mid, precision)?;
    let zero = Quantity::zero(FIXED_PRECISION);
    QuoteTick::new_checked(
        reports::instrument_id(market),
        price,
        price,
        zero,
        zero,
        UnixNanos::from(
            time.checked_mul(1_000_000)
                .ok_or_else(|| anyhow::anyhow!("Quote timestamp overflow"))?,
        ),
        clock.get_time_ns(),
    )
}
#[async_trait(?Send)]
impl DataClient for PapertradeDataClient {
    fn client_id(&self) -> ClientId {
        self.id
    }
    fn venue(&self) -> Option<Venue> {
        Some(Venue::from(PAPERTRADE))
    }
    fn start(&mut self) -> Result<()> {
        self.sender = Some(get_data_event_sender());
        self.cancellation = CancellationToken::new();
        Ok(())
    }
    fn stop(&mut self) -> Result<()> {
        self.cancellation.cancel();
        self.connected.store(false, Ordering::Release);
        Ok(())
    }
    fn reset(&mut self) -> Result<()> {
        self.stop()?;
        self.subscriptions.write().clear();
        Ok(())
    }
    fn dispose(&mut self) -> Result<()> {
        self.stop()
    }
    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Acquire)
    }
    fn is_disconnected(&self) -> bool {
        !self.is_connected()
    }
    async fn connect(&mut self) -> Result<()> {
        let state = self.api.trading().await?;
        for market in state.instruments {
            let instrument = reports::instrument(&market, self.clock.get_time_ns())?;
            self.markets
                .write()
                .insert(reports::instrument_id(&market), market.clone());
            self.send(DataEvent::Instrument(instrument))?;
            self.send(DataEvent::Data(Data::Quote(
                reference_quote(&self.api, &market, self.clock).await?,
            )))?;
        }
        self.connected.store(true, Ordering::Release);
        let api = self.api.clone();
        let markets = self.markets.clone();
        let subscriptions = self.subscriptions.clone();
        let connected = self.connected.clone();
        let cancel = self.cancellation.clone();
        let sender = self
            .sender
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Data client not started"))?;
        let clock = self.clock;
        let period = self.config.poll_interval_ms;
        get_runtime().spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(period));
            loop {
                tokio::select! {()=cancel.cancelled()=>break,_=interval.tick()=>{}}
                let state = match api.trading().await {
                    Ok(state) => state,
                    Err(e) => {
                        connected.store(false, Ordering::Release);
                        log::warn!("papertrade_reference_config_unavailable error={e}");
                        continue;
                    }
                };
                let selected = subscriptions.read().clone();
                for market in state.instruments {
                    let id = reports::instrument_id(&market);
                    markets.write().insert(id, market.clone());
                    if !selected.contains(&id) {
                        continue;
                    }
                    let quote = async {
                        let quote = reference_quote(&api, &market, clock).await?;
                        ensure!(!cancel.is_cancelled(), "Data client stopped");
                        sender
                            .send(DataEvent::Data(Data::Quote(quote)))
                            .map_err(|_| anyhow::anyhow!("Data engine receiver closed"))?;
                        Ok::<_, anyhow::Error>(())
                    }
                    .await;
                    match quote {
                        Ok(()) => connected.store(true, Ordering::Release),
                        Err(e) => {
                            connected.store(false, Ordering::Release);
                            log::warn!("papertrade_reference_quote_unavailable error={e}");
                        }
                    }
                }
            }
            connected.store(false, Ordering::Release);
        });
        Ok(())
    }
    async fn disconnect(&mut self) -> Result<()> {
        self.stop()
    }
    fn subscribe_quotes(&mut self, cmd: SubscribeQuotes) -> Result<()> {
        ensure!(
            self.markets.read().contains_key(&cmd.instrument_id),
            "Unknown Papertrade instrument"
        );
        self.subscriptions.write().insert(cmd.instrument_id);
        Ok(())
    }
    fn unsubscribe_quotes(&mut self, cmd: &UnsubscribeQuotes) -> Result<()> {
        self.subscriptions.write().remove(&cmd.instrument_id);
        Ok(())
    }
    fn request_instruments(&self, cmd: RequestInstruments) -> Result<()> {
        let instruments: Vec<InstrumentAny> = self
            .markets
            .read()
            .values()
            .map(|m| reports::instrument(m, self.clock.get_time_ns()))
            .collect::<Result<_>>()?;
        self.send(DataEvent::Response(DataResponse::Instruments(
            InstrumentsResponse::new(
                cmd.request_id,
                cmd.client_id.unwrap_or(self.id),
                Venue::from(PAPERTRADE),
                instruments,
                None,
                None,
                self.clock.get_time_ns(),
                cmd.params,
            ),
        )))
    }
    fn request_instrument(&self, cmd: RequestInstrument) -> Result<()> {
        let market = self
            .markets
            .read()
            .get(&cmd.instrument_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Unknown Papertrade instrument"))?;
        self.send(DataEvent::Response(DataResponse::Instrument(Box::new(
            InstrumentResponse::new(
                cmd.request_id,
                cmd.client_id.unwrap_or(self.id),
                cmd.instrument_id,
                reports::instrument(&market, self.clock.get_time_ns())?,
                None,
                None,
                self.clock.get_time_ns(),
                cmd.params,
            ),
        ))))
    }
}
