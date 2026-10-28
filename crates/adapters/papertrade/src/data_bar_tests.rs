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

//! Actual data-client requests and closed-day delivery over local HTTP fixtures.

use super::*;
use axum::{
    Json, Router,
    extract::State,
    routing::{get, post},
};
use nautilus_common::{
    live::runner::replace_data_event_sender,
    messages::data::{RequestBars, SubscribeBars, UnsubscribeBars},
};
use nautilus_core::UUID4;
use nautilus_model::{
    data::{Bar, BarType},
    instruments::Instrument,
};
use parking_lot::Mutex;
use serde_json::{Value, json};
use std::{
    str::FromStr,
    sync::atomic::{AtomicU64, AtomicUsize},
};

const DAY: u64 = 86_400_000;
struct Source {
    trading: Mutex<Value>,
    requests: Mutex<Vec<Value>>,
    now_ms: AtomicU64,
    fault: AtomicUsize,
    hold: AtomicBool,
    release: tokio::sync::Semaphore,
    completed: AtomicUsize,
    settled: AtomicUsize,
}
struct RequestSettled(Arc<Source>);
impl Drop for RequestSettled {
    fn drop(&mut self) {
        self.0.settled.fetch_add(1, Ordering::SeqCst);
    }
}
async fn trading(State(source): State<Arc<Source>>) -> Json<Value> {
    Json(source.trading.lock().clone())
}
async fn reference(State(source): State<Arc<Source>>, Json(body): Json<Value>) -> Json<Value> {
    if body["type"] == "l2Book" {
        return Json(
            json!({"coin":body["coin"],"time":source.now_ms.load(Ordering::SeqCst),"levels":[[{"px":"25000.0"}],[{"px":"25000.2"}]]}),
        );
    }
    source.requests.lock().push(body.clone());
    let _settled = RequestSettled(source.clone());
    if source.hold.load(Ordering::SeqCst) {
        let _permit = source.release.acquire().await.unwrap();
    }
    let start = body["req"]["startTime"].as_u64().unwrap();
    let end = body["req"]["endTime"].as_u64().unwrap();
    let mut rows: Vec<Value> = (start..=end)
        .step_by(DAY as usize)
        .map(|stamp| {
            json!({
                "t":stamp,"T":stamp+DAY-1,"s":body["req"]["coin"],"i":"1d",
                "o":"25000.0","h":"26000.0","l":"24000.0","c":"25500.0","v":"0.0","n":0
            })
        })
        .collect();
    if source.fault.load(Ordering::SeqCst) == 1 && rows.len() > 1 {
        rows.remove(1);
    }
    source.completed.fetch_add(1, Ordering::SeqCst);
    Json(json!(rows))
}
fn daily() -> BarType {
    BarType::from_str("BTC-USD-PERP.PAPERTRADE-1-DAY-LAST-EXTERNAL").unwrap()
}
fn subscribe() -> SubscribeBars {
    SubscribeBars::new(
        daily(),
        Some(ClientId::from(PAPERTRADE)),
        None,
        UUID4::new(),
        UnixNanos::from(1),
        None,
        None,
    )
}
fn request() -> RequestBars {
    RequestBars::new(
        daily(),
        Some("1970-04-11T12:30:00Z".parse().unwrap()),
        None,
        None,
        Some(ClientId::from(PAPERTRADE)),
        UUID4::new(),
        UnixNanos::from(1),
        None,
    )
}
struct Harness {
    client: PapertradeDataClient,
    source: Arc<Source>,
    receiver: tokio::sync::mpsc::UnboundedReceiver<DataEvent>,
    server: tokio::task::JoinHandle<()>,
}
impl Harness {
    async fn new(terminal: bool) -> Self {
        let mut document: Value =
            serde_json::from_str(include_str!("../test_data/live_trading.json")).unwrap();
        for row in document["instruments"].as_array_mut().unwrap() {
            if row[1] == "BTC" {
                row[6] = json!(terminal);
            }
        }
        let source = Arc::new(Source {
            trading: Mutex::new(document),
            requests: Mutex::new(Vec::new()),
            now_ms: AtomicU64::new(103 * DAY + 500),
            fault: AtomicUsize::new(0),
            hold: AtomicBool::new(false),
            release: tokio::sync::Semaphore::new(0),
            completed: AtomicUsize::new(0),
            settled: AtomicUsize::new(0),
        });
        let app = Router::new()
            .route("/state/trading", get(trading))
            .route("/reference", post(reference))
            .with_state(source.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}/", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        replace_data_event_sender(sender);
        let mut client = PapertradeDataClient::new(
            ClientId::from(PAPERTRADE),
            PapertradeDataClientConfig {
                origin: origin.clone(),
                rpc_url: origin.clone(),
                reference_info_url: format!("{origin}reference"),
                poll_interval_ms: 100,
            },
        )
        .unwrap();
        client.clock = Box::leak(Box::new(AtomicTime::new(
            false,
            UnixNanos::from((103 * DAY + 500) * 1_000_000),
        )));
        client.start().unwrap();
        client.connect().await.unwrap();
        Self {
            client,
            source,
            receiver,
            server,
        }
    }
    fn advance(&self, day: u64) {
        let now = day * DAY + 500;
        self.source.now_ms.store(now, Ordering::SeqCst);
        self.client.clock.set_time(UnixNanos::from(now * 1_000_000));
    }
    async fn bars(&mut self, count: usize) -> Vec<Bar> {
        tokio::time::timeout(Duration::from_secs(3), async {
            let mut rows = Vec::new();
            while rows.len() < count {
                if let DataEvent::Data(Data::Bar(bar)) = self.receiver.recv().await.unwrap() {
                    rows.push(bar);
                }
            }
            rows
        })
        .await
        .unwrap()
    }
    async fn history(&mut self, id: UUID4) -> Vec<Bar> {
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if let DataEvent::Response(DataResponse::Bars(response)) =
                    self.receiver.recv().await.unwrap()
                {
                    assert_eq!(response.correlation_id, id);
                    return response.data;
                }
            }
        })
        .await
        .unwrap()
    }
    fn discard_initial(&mut self) {
        while self.receiver.try_recv().is_ok() {}
    }
}
impl Drop for Harness {
    fn drop(&mut self) {
        self.client.stop().unwrap();
        self.server.abort();
    }
}
async fn until(mut predicate: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(3), async {
        while !predicate() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn historical_request_routes_complete_closed_window_back_to_original_request() {
    let mut h = Harness::new(false).await;
    h.discard_initial();
    let command = request();
    let id = command.request_id;
    h.client.request_bars(command).unwrap();
    let rows = h.history(id).await;
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].ts_event.as_u64(), 100 * DAY * 1_000_000);
    assert_eq!(rows[2].ts_init.as_u64(), 103 * DAY * 1_000_000);
    assert_eq!(
        h.source.requests.lock()[0]["req"]["endTime"],
        json!(103 * DAY - 1)
    );
}

#[tokio::test]
async fn live_days_begin_after_subscription_without_repeating_historical_tail() {
    let mut h = Harness::new(false).await;
    h.discard_initial();
    h.client.subscribe_bars(subscribe()).unwrap();
    let command = request();
    let id = command.request_id;
    h.client.request_bars(command).unwrap();
    let history = h.history(id).await;
    h.advance(104);
    let live = h.bars(1).await;
    assert!(live[0].ts_event > history.last().unwrap().ts_event);
    assert_eq!(live[0].ts_init.as_u64(), 104 * DAY * 1_000_000);
    h.client.subscribe_bars(subscribe()).unwrap();
    h.advance(105);
    let next = h.bars(1).await;
    assert_eq!(next[0].ts_event.as_u64(), 104 * DAY * 1_000_000);
}

#[tokio::test]
async fn missing_day_does_not_advance_delivery_and_recovery_is_ordered_once() {
    let mut h = Harness::new(false).await;
    h.discard_initial();
    h.client.subscribe_bars(subscribe()).unwrap();
    h.source.fault.store(1, Ordering::SeqCst);
    h.advance(106);
    until(|| !h.source.requests.lock().is_empty()).await;
    assert!(
        tokio::time::timeout(Duration::from_millis(150), h.receiver.recv())
            .await
            .is_err()
    );
    h.source.fault.store(0, Ordering::SeqCst);
    let rows = h.bars(3).await;
    assert_eq!(
        rows.iter()
            .map(|bar| bar.ts_init.as_u64())
            .collect::<Vec<_>>(),
        vec![
            104 * DAY * 1_000_000,
            105 * DAY * 1_000_000,
            106 * DAY * 1_000_000
        ]
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(150), h.receiver.recv())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn unsubscribe_and_stop_prevent_later_daily_delivery() {
    let mut h = Harness::new(false).await;
    h.discard_initial();
    h.client.subscribe_bars(subscribe()).unwrap();
    h.client
        .unsubscribe_bars(&UnsubscribeBars::new(
            daily(),
            Some(ClientId::from(PAPERTRADE)),
            None,
            UUID4::new(),
            UnixNanos::from(1),
            None,
            None,
        ))
        .unwrap();
    h.advance(104);
    assert!(
        tokio::time::timeout(Duration::from_millis(150), h.receiver.recv())
            .await
            .is_err()
    );
    h.client.stop().unwrap();
    assert!(h.client.request_bars(request()).is_err());
}

#[tokio::test]
async fn stopped_generation_cannot_publish_an_inflight_historical_response() {
    let mut h = Harness::new(false).await;
    h.discard_initial();
    h.source.hold.store(true, Ordering::SeqCst);
    h.client.request_bars(request()).unwrap();
    until(|| !h.source.requests.lock().is_empty()).await;
    h.client.stop().unwrap();
    h.source.release.add_permits(1);
    until(|| h.source.settled.load(Ordering::SeqCst) > 0).await;
    assert!(
        tokio::time::timeout(Duration::from_millis(150), h.receiver.recv())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn terminal_cold_start_keeps_the_instrument_without_a_fresh_quote_or_daily_admission() {
    let mut h = Harness::new(true).await;
    let mut instrument = false;
    while let Ok(event) = h.receiver.try_recv() {
        match event {
            DataEvent::Instrument(value) if value.id() == daily().instrument_id() => {
                instrument = true;
            }
            DataEvent::Data(Data::Quote(quote)) => {
                assert_ne!(quote.instrument_id, daily().instrument_id());
            }
            _ => {}
        }
    }
    assert!(instrument);
    assert!(h.client.subscribe_bars(subscribe()).is_err());
    assert!(h.client.request_bars(request()).is_err());
    assert!(h.source.requests.lock().is_empty());
}

#[tokio::test]
async fn terminal_update_disables_daily_health_before_the_next_day_boundary() {
    let mut h = Harness::new(false).await;
    h.discard_initial();
    h.client.subscribe_bars(subscribe()).unwrap();
    for row in h.source.trading.lock()["instruments"]
        .as_array_mut()
        .unwrap()
    {
        if row[1] == "BTC" {
            row[6] = json!(true);
        }
    }
    until(|| !h.client.is_connected()).await;
    h.advance(104);
    assert!(
        tokio::time::timeout(Duration::from_millis(150), h.receiver.recv())
            .await
            .is_err()
    );
    assert!(h.source.requests.lock().is_empty());
}

#[tokio::test]
async fn dropping_the_original_data_owner_prevents_inflight_delivery() {
    let mut h = Harness::new(false).await;
    h.discard_initial();
    h.source.hold.store(true, Ordering::SeqCst);
    h.client.request_bars(request()).unwrap();
    until(|| !h.source.requests.lock().is_empty()).await;
    let replacement =
        PapertradeDataClient::new(ClientId::from(PAPERTRADE), h.client.config.clone()).unwrap();
    let old = std::mem::replace(&mut h.client, replacement);
    drop(old);
    h.source.release.add_permits(1);
    until(|| h.source.settled.load(Ordering::SeqCst) > 0).await;
    assert!(
        tokio::time::timeout(Duration::from_millis(500), h.receiver.recv())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn removed_market_is_not_kept_as_current_daily_metadata() {
    let mut h = Harness::new(false).await;
    h.discard_initial();
    h.client.subscribe_bars(subscribe()).unwrap();
    {
        let mut document = h.source.trading.lock();
        document["instruments"]
            .as_array_mut()
            .unwrap()
            .retain(|row| row[1] != "BTC");
        let parsed: crate::wire::TradingState = serde_json::from_value(document.clone()).unwrap();
        parsed.validate().unwrap();
    }
    until(|| !h.client.is_connected()).await;
    assert!(h.client.request_bars(request()).is_err());
    assert!(h.source.requests.lock().is_empty());
}

#[tokio::test]
async fn invalid_daily_bounds_and_other_bar_types_fail_before_reference_http() {
    let h = Harness::new(false).await;
    let mut wrong = request();
    wrong.bar_type = BarType::from_str("BTC-USD-PERP.PAPERTRADE-1-HOUR-LAST-EXTERNAL").unwrap();
    assert!(h.client.request_bars(wrong).is_err());
    let mut oversized = request();
    oversized.limit = std::num::NonZeroUsize::new(5001);
    assert!(h.client.request_bars(oversized).is_err());
    let mut future = request();
    future.end = Some("1970-04-15T00:00:00Z".parse().unwrap());
    assert!(h.client.request_bars(future).is_err());
    let mut negative = request();
    negative.start = Some("1969-12-31T00:00:00Z".parse().unwrap());
    assert!(h.client.request_bars(negative).is_err());
    assert!(h.source.requests.lock().is_empty());
}

#[tokio::test]
async fn a_restarted_client_does_not_receive_the_previous_generation_response() {
    let mut h = Harness::new(false).await;
    h.discard_initial();
    h.source.hold.store(true, Ordering::SeqCst);
    h.client.request_bars(request()).unwrap();
    until(|| !h.source.requests.lock().is_empty()).await;
    h.client.stop().unwrap();
    h.client.start().unwrap();
    h.client.connect().await.unwrap();
    h.discard_initial();
    h.source.release.add_permits(1);
    until(|| h.source.settled.load(Ordering::SeqCst) > 0).await;
    println!(
        "restart_old_request_completed={} settled={}",
        h.source.completed.load(Ordering::SeqCst),
        h.source.settled.load(Ordering::SeqCst)
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(150), h.receiver.recv())
            .await
            .is_err()
    );
    h.source.hold.store(false, Ordering::SeqCst);
    let next = request();
    let id = next.request_id;
    h.client.request_bars(next).unwrap();
    assert_eq!(h.history(id).await.len(), 3);
}

#[tokio::test]
async fn resubscription_during_an_inflight_window_does_not_replay_the_old_cursor() {
    let mut h = Harness::new(false).await;
    h.discard_initial();
    h.client.subscribe_bars(subscribe()).unwrap();
    h.source.hold.store(true, Ordering::SeqCst);
    h.advance(104);
    until(|| !h.source.requests.lock().is_empty()).await;
    h.client
        .unsubscribe_bars(&UnsubscribeBars::new(
            daily(),
            Some(ClientId::from(PAPERTRADE)),
            None,
            UUID4::new(),
            UnixNanos::from(1),
            None,
            None,
        ))
        .unwrap();
    h.client.subscribe_bars(subscribe()).unwrap();
    h.source.hold.store(false, Ordering::SeqCst);
    h.source.release.add_permits(1);
    until(|| h.source.settled.load(Ordering::SeqCst) > 0).await;
    assert!(
        tokio::time::timeout(Duration::from_millis(150), h.receiver.recv())
            .await
            .is_err()
    );
    h.advance(105);
    assert_eq!(h.bars(1).await[0].ts_event.as_u64(), 104 * DAY * 1_000_000);
}

#[tokio::test]
async fn previous_generation_market_state_cannot_change_new_daily_admission() {
    let mut h = Harness::new(false).await;
    let previous = h.client.markets.clone();
    h.client.stop().unwrap();
    h.client.start().unwrap();
    h.client.connect().await.unwrap();
    previous
        .write()
        .get_mut(&daily().instrument_id())
        .unwrap()
        .terminal = true;
    assert!(h.client.subscribe_bars(subscribe()).is_ok());
}

#[tokio::test]
async fn a_removed_quote_only_subscription_is_not_reported_as_connected() {
    let mut h = Harness::new(false).await;
    h.discard_initial();
    h.client
        .subscribe_quotes(SubscribeQuotes::new(
            daily().instrument_id(),
            Some(ClientId::from(PAPERTRADE)),
            None,
            UUID4::new(),
            UnixNanos::from(1),
            None,
            None,
        ))
        .unwrap();
    {
        let mut document = h.source.trading.lock();
        document["instruments"]
            .as_array_mut()
            .unwrap()
            .retain(|row| row[1] != "BTC");
        let parsed: crate::wire::TradingState = serde_json::from_value(document.clone()).unwrap();
        parsed.validate().unwrap();
    }
    until(|| !h.client.is_connected()).await;
}
