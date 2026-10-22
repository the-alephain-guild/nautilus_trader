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

use std::{
    cell::RefCell,
    convert::Infallible,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response, Sse, sse::Event},
    routing::{get, post},
};
use nautilus_common::{
    cache::Cache, clients::ExecutionClient, factories::ExecutionClientFactory,
    live::runner::replace_exec_event_sender,
};
use nautilus_core::time::get_atomic_clock_realtime;
use nautilus_model::identifiers::{AccountId, TraderId};
use nautilus_papertrade::{
    config::{PAPERTRADE, PapertradeExecutionClientConfig},
    factories::PapertradeExecutionClientFactory,
};
use parking_lot::Mutex;
use serde_json::{Value, json};

static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

struct Source {
    wallet: tokio::sync::watch::Sender<Value>,
    unavailable: AtomicBool,
    trading: Mutex<Value>,
}
async fn wallet(State(source): State<Arc<Source>>) -> Response {
    if source.unavailable.load(Ordering::SeqCst) {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    let stream = futures_util::stream::unfold(
        (source.wallet.subscribe(), true),
        |(mut rx, first)| async move {
            if !first && rx.changed().await.is_err() {
                return None;
            }
            let value = rx.borrow().clone();
            Some((
                Ok::<_, Infallible>(Event::default().event("wallet").data(value.to_string())),
                (rx, false),
            ))
        },
    );
    Sse::new(stream).into_response()
}
async fn trading(State(source): State<Arc<Source>>) -> Json<Value> {
    Json(source.trading.lock().clone())
}
async fn history() -> Json<Value> {
    Json(json!({"items":[],"nextCursor":null}))
}
async fn rpc() -> Json<Value> {
    Json(json!({"jsonrpc":"2.0","id":1,"result":[]}))
}
struct Harness {
    source: Arc<Source>,
    config: PapertradeExecutionClientConfig,
    cache: Rc<RefCell<Cache>>,
    server: tokio::task::JoinHandle<()>,
    _events: tokio::sync::mpsc::UnboundedReceiver<nautilus_common::messages::ExecutionEvent>,
    _dir: tempfile::TempDir,
}
impl Harness {
    async fn new() -> Self {
        let mut snapshot: Value =
            serde_json::from_str(include_str!("../test_data/live_wallet.json")).unwrap();
        let now = get_atomic_clock_realtime().get_time_ns().as_u64() / 1_000_000_000;
        snapshot["indexedThroughTimestamp"] = json!(now);
        snapshot["sessionKeys"] = json!([[
            "0x19e7e376e7c213b7e7e7e46cc70a5dd086daff2a",
            (now + 86400).to_string()
        ]]);
        let (tx, _) = tokio::sync::watch::channel(snapshot.clone());
        let source = Arc::new(Source {
            wallet: tx,
            unavailable: AtomicBool::new(false),
            trading: Mutex::new(
                serde_json::from_str(include_str!("../test_data/live_trading.json")).unwrap(),
            ),
        });
        let app = Router::new()
            .route("/state/user/live", get(wallet))
            .route("/state/trading", get(trading))
            .route("/query/accounts/{wallet}/trades", get(history))
            .route("/rpc", post(rpc))
            .with_state(source.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}/", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let dir = tempfile::tempdir().unwrap();
        let mut config = PapertradeExecutionClientConfig::new(
            AccountId::from("PAPERTRADE-001"),
            snapshot["address"].as_str().unwrap().into(),
            dir.path().join("journal.redb"),
            "test-build".into(),
        );
        config.origin = origin.clone();
        config.rpc_url = format!("{origin}rpc");
        config.poll_interval_ms = 500;
        config.session_private_key = Some(format!("0x{}", "11".repeat(32)).into());
        let (tx, events) = tokio::sync::mpsc::unbounded_channel();
        replace_exec_event_sender(tx);
        Self {
            source,
            config,
            cache: Rc::new(RefCell::new(Cache::default())),
            server,
            _events: events,
            _dir: dir,
        }
    }
    fn client(&self) -> Box<dyn ExecutionClient> {
        PapertradeExecutionClientFactory
            .create(
                TraderId::from("TESTER-001"),
                PAPERTRADE,
                &self.config,
                self.cache.clone().into(),
            )
            .unwrap()
    }
}
impl Drop for Harness {
    fn drop(&mut self) {
        self.server.abort();
    }
}

#[tokio::test]
async fn failed_wallet_reconnect_never_restores_old_coverage_on_poll_ticks() {
    let _serial = SERIAL.lock().await;
    let harness = Harness::new().await;
    let mut client = harness.client();
    client.start().unwrap();
    client.connect().await.unwrap();
    harness.source.unavailable.store(true, Ordering::SeqCst);
    let address = harness.source.wallet.borrow()["address"].clone();
    harness.source.wallet.send(json!({"address":address,"baseAccountRevision":999,"accountRevision":1000,"state":{},"addedPositions":[],"removedPositionIds":[],"addedOperations":[],"removedOperationIds":[]})).unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        while client.is_connected() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    for _ in 0..100 {
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(
            !client.is_connected(),
            "A poll tick revived the invalidated account snapshot"
        );
        assert!(client.generate_mass_status(None).await.is_err());
    }
    client.disconnect().await.unwrap();
}

#[tokio::test]
async fn silent_wallet_expires_despite_successful_polling() {
    let _serial = SERIAL.lock().await;
    let harness = Harness::new().await;
    let mut client = harness.client();
    client.start().unwrap();
    client.connect().await.unwrap();
    harness.source.unavailable.store(true, Ordering::SeqCst);
    // The original response stays open but sends neither frames nor heartbeats.
    tokio::time::timeout(Duration::from_secs(38), async {
        while client.is_connected() {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("The idle deadline was reset by polling");
    assert!(client.generate_mass_status(None).await.is_err());
    client.disconnect().await.unwrap();
}
