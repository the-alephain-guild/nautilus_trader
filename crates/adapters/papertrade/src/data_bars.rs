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

//! Historical requests and generation-bound delivery of newly closed daily bars.

use super::*;
use crate::reference_bars::{
    MAX_DAILY_BARS, REFERENCE_DAY_MS, ReferenceDailyWindow, fetch_daily_bars,
    validate_daily_bar_type,
};
use nautilus_core::UUID4;
use nautilus_model::data::{Bar, BarType};

#[derive(Debug, Clone, Copy)]
struct DailyCursor {
    id: UUID4,
    last_closed_ms: u64,
}

#[derive(Debug, Clone, Default)]
pub(super) struct DailyBarSubscriptions(Arc<RwLock<HashMap<BarType, DailyCursor>>>);
impl DailyBarSubscriptions {
    pub(super) fn clear(&self) {
        self.0.write().clear();
    }
    pub(super) fn unsubscribe(&self, bar_type: BarType) {
        self.0.write().remove(&bar_type);
    }
    fn subscribe(&self, bar_type: BarType, now_ms: u64) -> Result<()> {
        let mut cursors = self.0.write();
        ensure!(
            cursors.contains_key(&bar_type) || cursors.len() < 64,
            "Daily subscription bound exceeded"
        );
        cursors.entry(bar_type).or_insert_with(|| DailyCursor {
            id: UUID4::new(),
            last_closed_ms: now_ms / REFERENCE_DAY_MS * REFERENCE_DAY_MS,
        });
        Ok(())
    }
    fn deliver(
        &self,
        bar_type: BarType,
        cursor: DailyCursor,
        rows: Vec<Bar>,
        sender: &DataDelivery,
    ) -> Result<()> {
        let mut cursors = self.0.write();
        let Some(current) = cursors.get_mut(&bar_type) else {
            return Ok(());
        };
        if current.id != cursor.id || current.last_closed_ms != cursor.last_closed_ms {
            return Ok(());
        }
        for bar in rows {
            let close_ms = bar.ts_init.as_u64() / 1_000_000;
            sender.send(DataEvent::Data(Data::Bar(bar)))?;
            current.last_closed_ms = close_ms;
        }
        Ok(())
    }
    pub(super) async fn poll(
        &self,
        api: &PapertradeApi,
        markets: &RwLock<HashMap<InstrumentId, Market>>,
        sender: &DataDelivery,
        clock: &AtomicTime,
        cancel: &CancellationToken,
    ) -> bool {
        let selected: Vec<_> = self
            .0
            .read()
            .iter()
            .map(|(kind, cursor)| (*kind, *cursor))
            .collect();
        let mut healthy = true;
        for (bar_type, cursor) in selected {
            let now_ms = clock.get_time_ns().as_u64() / 1_000_000;
            let end_ms = now_ms / REFERENCE_DAY_MS * REFERENCE_DAY_MS;
            let result = async {
                let market = markets
                    .read()
                    .get(&bar_type.instrument_id())
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("Daily subscription market is unavailable"))?;
                validate_daily_bar_type(&market, bar_type)?;
                if end_ms == cursor.last_closed_ms {
                    return Ok(());
                }
                let window = ReferenceDailyWindow::new(cursor.last_closed_ms, end_ms, now_ms)?;
                let rows = tokio::select! {
                    ()=cancel.cancelled()=>return Ok(()),
                    result=fetch_daily_bars(api,&market,bar_type,window)=>result?,
                };
                let _market_guard = hold_current_market(markets, &market, bar_type)?;
                self.deliver(bar_type, cursor, rows, sender)
            }
            .await;
            if let Err(error) = result {
                healthy = false;
                if !cancel.is_cancelled() {
                    log::warn!(
                        "papertrade_reference_daily_unavailable bar_type={bar_type} error={error}"
                    );
                }
            }
        }
        healthy
    }
}

fn hold_current_market<'a>(
    markets: &'a RwLock<HashMap<InstrumentId, Market>>,
    original: &Market,
    bar_type: BarType,
) -> Result<parking_lot::RwLockReadGuard<'a, HashMap<InstrumentId, Market>>> {
    let guard = markets.read();
    let current = guard
        .get(&bar_type.instrument_id())
        .ok_or_else(|| anyhow::anyhow!("Daily response market was removed"))?;
    validate_daily_bar_type(current, bar_type)?;
    ensure!(
        current.id == original.id
            && current.symbol == original.symbol
            && current.price_scale.0 == original.price_scale.0,
        "Daily response market precision changed"
    );
    Ok(guard)
}

fn check_route(
    client_id: Option<ClientId>,
    venue: Option<Venue>,
    expected: ClientId,
) -> Result<()> {
    ensure!(
        client_id.is_none_or(|value| value == expected)
            && venue.is_none_or(|value| value == Venue::from(PAPERTRADE)),
        "Daily command selects another data route"
    );
    Ok(())
}

impl PapertradeDataClient {
    fn running_delivery(&self) -> Result<Arc<DataDelivery>> {
        ensure!(
            !self.cancellation.is_cancelled()
                && self.task.as_ref().is_some_and(|task| !task.is_finished()),
            "Data client is not running"
        );
        self.sender
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Data client not started"))
    }
    pub(super) fn subscribe_reference_bars(&self, cmd: &SubscribeBars) -> Result<()> {
        self.running_delivery()?;
        check_route(cmd.client_id, cmd.venue, self.id)?;
        let markets = self.markets.read();
        let market = markets
            .get(&cmd.bar_type.instrument_id())
            .ok_or_else(|| anyhow::anyhow!("Unknown daily reference market"))?;
        validate_daily_bar_type(market, cmd.bar_type)?;
        self.bars
            .subscribe(cmd.bar_type, self.clock.get_time_ns().as_u64() / 1_000_000)
    }
    pub(super) fn request_reference_bars(&self, cmd: RequestBars) -> Result<()> {
        let sender = self.running_delivery()?;
        check_route(cmd.client_id, None, self.id)?;
        let market = self
            .markets
            .read()
            .get(&cmd.bar_type.instrument_id())
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Unknown daily reference market"))?;
        validate_daily_bar_type(&market, cmd.bar_type)?;
        let now_ms = self.clock.get_time_ns().as_u64() / 1_000_000;
        let requested_end = cmd
            .end
            .map(|value| u64::try_from(value.as_millisecond()))
            .transpose()?
            .unwrap_or(now_ms);
        ensure!(
            requested_end <= now_ms,
            "Daily history end is in the future"
        );
        let end_ms = requested_end / REFERENCE_DAY_MS * REFERENCE_DAY_MS;
        let limit = cmd
            .limit
            .map(|value| u64::try_from(value.get()))
            .transpose()?;
        ensure!(
            limit.is_none_or(|value| value <= MAX_DAILY_BARS),
            "Daily history limit exceeds retention"
        );
        let requested_start = cmd
            .start
            .map(|value| u64::try_from(value.as_millisecond()))
            .transpose()?;
        let mut start_ms = requested_start.map_or_else(
            || end_ms.saturating_sub(limit.unwrap_or(1000) * REFERENCE_DAY_MS),
            |value| value / REFERENCE_DAY_MS * REFERENCE_DAY_MS,
        );
        if let Some(limit) = limit {
            start_ms = start_ms.max(end_ms.saturating_sub(limit * REFERENCE_DAY_MS));
        }
        let window = ReferenceDailyWindow::new(start_ms, end_ms, now_ms)?;
        let api = self.api.clone();
        let cancel = self.cancellation.clone();
        let markets = self.markets.clone();
        let clock = self.clock;
        let client_id = cmd.client_id.unwrap_or(self.id);
        get_runtime().spawn(async move {
            let response=tokio::select! {()=cancel.cancelled()=>return,result=fetch_daily_bars(&api,&market,cmd.bar_type,window)=>result};
            let result=(|| {
                let rows=response?;
                let _market_guard = hold_current_market(&markets,&market,cmd.bar_type)?;
                sender.send(DataEvent::Response(DataResponse::Bars(nautilus_common::messages::data::BarsResponse::new(
                    cmd.request_id,client_id,cmd.bar_type,rows,
                    Some(UnixNanos::from(window.start_ms()*1_000_000)),Some(UnixNanos::from(window.end_ms()*1_000_000)),clock.get_time_ns(),cmd.params,
                ))))
            })();
            if let Err(error)=result && !cancel.is_cancelled() {log::warn!("papertrade_reference_history_unavailable request_id={} error={error}",cmd.request_id);}
        });
        Ok(())
    }
}
