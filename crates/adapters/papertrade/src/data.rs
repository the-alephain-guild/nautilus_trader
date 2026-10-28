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
            DataResponse, InstrumentResponse, InstrumentsResponse, RequestBars, RequestInstrument,
            RequestInstruments, SubscribeBars, SubscribeQuotes, UnsubscribeBars, UnsubscribeQuotes,
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
use parking_lot::{Mutex, RwLock};
use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio_util::sync::CancellationToken;

#[path = "data_bars.rs"]
mod bars;

#[derive(Debug)]
struct DataDelivery {
    sender: tokio::sync::mpsc::UnboundedSender<DataEvent>,
    live: Mutex<bool>,
    connected: Arc<AtomicBool>,
}
impl DataDelivery {
    fn send(&self, event: DataEvent) -> Result<()> {
        let live = self.live.lock();
        ensure!(*live, "Data client delivery has stopped");
        self.sender
            .send(event)
            .map_err(|_| anyhow::anyhow!("Data engine receiver closed"))
    }
    fn set_connected(&self, value: bool) {
        let live = self.live.lock();
        if *live {
            self.connected.store(value, Ordering::Release);
        }
    }
    fn close(&self) {
        let mut live = self.live.lock();
        if *live {
            *live = false;
            self.connected.store(false, Ordering::Release);
        }
    }
}

#[derive(Debug)]
pub struct PapertradeDataClient {
    id: ClientId,
    config: PapertradeDataClientConfig,
    api: PapertradeApi,
    markets: Arc<RwLock<HashMap<InstrumentId, Market>>>,
    subscriptions: Arc<RwLock<HashSet<InstrumentId>>>,
    connected: Arc<AtomicBool>,
    cancellation: CancellationToken,
    sender: Option<Arc<DataDelivery>>,
    bars: bars::DailyBarSubscriptions,
    task: Option<tokio::task::JoinHandle<()>>,
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
            bars: bars::DailyBarSubscriptions::default(),
            task: None,
            clock: get_atomic_clock_realtime(),
        })
    }
    fn send(&self, event: DataEvent) -> Result<()> {
        self.sender
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Data client not started"))?
            .send(event)
    }
}
impl Drop for PapertradeDataClient {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

async fn reference_quote(
    api: &PapertradeApi,
    market: &Market,
    clock: &'static AtomicTime,
) -> Result<QuoteTick> {
    ensure!(
        !market.terminal,
        "Terminal market has no live reference quote"
    );
    let (mid, time) = api.reference_mid(&market.symbol).await?;
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
        self.stop()?;
        self.markets = Arc::new(RwLock::new(HashMap::new()));
        self.subscriptions = Arc::new(RwLock::new(HashSet::new()));
        self.bars = bars::DailyBarSubscriptions::default();
        self.connected = Arc::new(AtomicBool::new(false));
        self.sender = Some(Arc::new(DataDelivery {
            sender: get_data_event_sender(),
            live: Mutex::new(true),
            connected: self.connected.clone(),
        }));
        self.cancellation = CancellationToken::new();
        Ok(())
    }
    fn stop(&mut self) -> Result<()> {
        if let Some(sender) = &self.sender {
            sender.close();
        }
        self.cancellation.cancel();
        if let Some(task) = self.task.take() {
            task.abort();
        }
        self.bars.clear();
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
        ensure!(self.task.is_none(), "Data client is already polling");
        ensure!(
            !self.cancellation.is_cancelled(),
            "Data client must be started before connecting"
        );
        let state = self.api.trading().await?;
        self.markets.write().clear();
        for market in state.instruments {
            let instrument = reports::instrument(&market, self.clock.get_time_ns())?;
            self.markets
                .write()
                .insert(reports::instrument_id(&market), market.clone());
            self.send(DataEvent::Instrument(instrument))?;
            if !market.terminal {
                self.send(DataEvent::Data(Data::Quote(
                    reference_quote(&self.api, &market, self.clock).await?,
                )))?;
            }
        }
        self.sender
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Data client not started"))?
            .set_connected(true);
        let api = self.api.clone();
        let markets = self.markets.clone();
        let subscriptions = self.subscriptions.clone();
        let bars = self.bars.clone();
        let cancel = self.cancellation.clone();
        let sender = self
            .sender
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Data client not started"))?;
        let clock = self.clock;
        let period = self.config.poll_interval_ms;
        self.task = Some(get_runtime().spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(period));
            loop {
                tokio::select! {()=cancel.cancelled()=>break,_=interval.tick()=>{}}
                let state = match api.trading().await {
                    Ok(state) => state,
                    Err(e) => {
                        sender.set_connected(false);
                        log::warn!("papertrade_reference_config_unavailable error={e}");
                        continue;
                    }
                };
                let selected = subscriptions.read().clone();
                let updated: HashMap<_, _> = state
                    .instruments
                    .iter()
                    .map(|market| (reports::instrument_id(market), market.clone()))
                    .collect();
                let mut healthy = selected
                    .iter()
                    .all(|identifier| updated.contains_key(identifier));
                *markets.write() = updated;
                for market in state.instruments {
                    let id = reports::instrument_id(&market);
                    if !selected.contains(&id) {
                        continue;
                    }
                    let quote = async {
                        let quote = reference_quote(&api, &market, clock).await?;
                        ensure!(!cancel.is_cancelled(), "Data client stopped");
                        sender.send(DataEvent::Data(Data::Quote(quote)))?;
                        Ok::<_, anyhow::Error>(())
                    }
                    .await;
                    match quote {
                        Ok(()) => {}
                        Err(e) => {
                            healthy = false;
                            log::warn!("papertrade_reference_quote_unavailable error={e}");
                        }
                    }
                }
                if !bars.poll(&api, &markets, &sender, clock, &cancel).await {
                    healthy = false;
                }
                sender.set_connected(healthy);
            }
            sender.set_connected(false);
        }));
        Ok(())
    }
    async fn disconnect(&mut self) -> Result<()> {
        self.stop()
    }
    fn request_bars(&self, cmd: RequestBars) -> Result<()> {
        self.request_reference_bars(cmd)
    }
    fn subscribe_bars(&mut self, cmd: SubscribeBars) -> Result<()> {
        self.subscribe_reference_bars(&cmd)
    }
    fn unsubscribe_bars(&mut self, cmd: &UnsubscribeBars) -> Result<()> {
        self.bars.unsubscribe(cmd.bar_type);
        Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::{Raw, TradingState};
    use alloy_primitives::U256;

    #[tokio::test]
    async fn terminal_market_cannot_emit_a_fresh_reference_quote() {
        let trading: TradingState =
            serde_json::from_str(include_str!("../test_data/live_trading.json")).unwrap();
        let mut market = trading.instruments[0].clone();
        market.terminal = true;
        market.frozen_bid = Some(Raw(U256::from(250000)));
        market.frozen_ask = Some(Raw(U256::from(250002)));
        let api = PapertradeApi::new(
            "http://127.0.0.1:1/".parse().unwrap(),
            "http://127.0.0.1:1/".parse().unwrap(),
            1,
        )
        .unwrap();
        assert!(
            reference_quote(&api, &market, get_atomic_clock_realtime())
                .await
                .is_err()
        );
    }
}

#[cfg(test)]
#[path = "data_bar_tests.rs"]
mod bar_tests;
