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

use axum::{
    Json, Router,
    extract::{
        State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    response::Response,
    routing::{get, post},
};
use futures_util::StreamExt;
use nautilus_common::enums::Environment;
use nautilus_hyperliquid::{
    HyperliquidExecutionClientConfig, HyperliquidExecutionClientFactory, HyperliquidHttpClient,
    common::enums::HyperliquidEnvironment,
};
use nautilus_live::node::LiveNode;
use nautilus_model::{
    data::QuoteTick,
    identifiers::TraderId,
    types::{Price, Quantity},
};
use parking_lot::Mutex;
use serde_json::{Value, json};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

pub(crate) const INSTRUMENT: &str = "io:SNDK-USD-PERP.HYPERLIQUID";
pub(crate) static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub(crate) struct Source {
    pub(crate) metas: Value,
    pub(crate) dexes: Value,
    pub(crate) fills: Mutex<Vec<Value>>,
    pub(crate) history: Mutex<Vec<Value>>,
    pub(crate) actions: Mutex<Vec<Value>>,
    pub(crate) queries: Mutex<Vec<Value>>,
    pub(crate) messages: tokio::sync::broadcast::Sender<Value>,
    pub(crate) subscriptions: AtomicUsize,
    pub(crate) hold_order_updates: AtomicBool,
    pub(crate) order_rows: Mutex<Vec<Value>>,
    pub(crate) open_orders: Mutex<Vec<Value>>,
    pub(crate) perp_positions: Mutex<Vec<Value>>,
}

async fn info(State(source): State<Arc<Source>>, Json(query): Json<Value>) -> Json<Value> {
    source.queries.lock().push(query.clone());
    Json(match query["type"].as_str().unwrap() {
        "allPerpMetas" => source.metas.clone(),
        "meta" => source.metas[0].clone(),
        "perpDexs" => source.dexes.clone(),
        "spotMeta" => json!({"universe":[], "tokens":[]}),
        "outcomeMeta" => json!({"outcomes":[]}),
        "spotClearinghouseState" => json!({"balances":[]}),
        "clearinghouseState" => json!({
            "marginSummary":{"accountValue":"1000", "totalMarginUsed":"0", "totalNtlPos":"0", "totalRawUsd":"1000"},
            "crossMarginSummary":{"accountValue":"1000", "totalMarginUsed":"0", "totalNtlPos":"0", "totalRawUsd":"1000"},
            "crossMaintenanceMarginUsed":"0", "withdrawable":"1000", "assetPositions": if query["dex"]=="io" { source.perp_positions.lock().clone() } else { Vec::new() }
        }),
        "openOrders" | "frontendOpenOrders" => json!(if query["dex"] == "io" {
            source.open_orders.lock().clone()
        } else {
            Vec::new()
        }),
        "userFills" => json!(source.fills.lock().clone()),
        "historicalOrders" => json!(source.history.lock().clone()),
        "orderStatus" => json!({"status":"unknownOid"}),
        "userFees" => json!({"userCrossRate":"0.00045", "userAddRate":"0.00015"}),
        other => panic!("Unexpected info request {other}"),
    })
}

async fn websocket(State(source): State<Arc<Source>>, ws: WebSocketUpgrade) -> Response {
    ws.on_upgrade(move |socket| serve_socket(socket, source))
}

async fn serve_socket(mut socket: WebSocket, source: Arc<Source>) {
    let mut messages = source.messages.subscribe();
    loop {
        tokio::select! {
            message = socket.next() => {
                let Some(Ok(message)) = message else { break };
                let mut order_update = None;
                let response = match message {
                    Message::Text(text) => {
                        let value: Value = serde_json::from_str(&text).unwrap();
                        match value["method"].as_str().unwrap() {
                            "ping" => Some(json!({"channel":"pong"})),
                            "subscribe" => { source.subscriptions.fetch_add(1, Ordering::SeqCst); None },
                            "unsubscribe" => None,
                            "post" => {
                                let signed = value["request"]["payload"].clone();
                                let mut actions = source.actions.lock();
                                actions.push(signed);
                                let oid = 9000 + actions.len();
                                let order = &actions.last().unwrap()["action"]["orders"][0];
                                let now = nautilus_core::time::get_atomic_clock_realtime().get_time_ms();
                                order_update = Some(json!({"channel":"orderUpdates", "data":[{
                                    "order":{"coin":"io:SNDK", "side":if order["b"]==true {"B"} else {"A"},
                                    "limitPx":order["p"], "sz":order["s"], "origSz":order["s"], "oid":oid,
                                    "timestamp":now, "cloid":order["c"], "reduceOnly":order["r"], "tif":order["t"]["limit"]["tif"]}, "status":"open", "statusTimestamp":now
                                }]}));
                                source.order_rows.lock().push(order_update.as_ref().unwrap()["data"][0]["order"].clone());
                                if source.hold_order_updates.load(Ordering::SeqCst) { order_update = None; }
                                Some(json!({"channel":"post", "data":{"id":value["id"], "response":{"type":"action", "payload":{
                                    "status":"ok", "response":{"type":"order", "data":{"statuses":[{"resting":{"oid":oid}}]}}
                                }}}}))
                            },
                            other => panic!("Unexpected WS method {other}"),
                        }
                    },
                    Message::Ping(bytes) => { if socket.send(Message::Pong(bytes)).await.is_err() { break; } None },
                    Message::Close(_) => break,
                    _ => None,
                };
                if let Some(response) = response
                    && socket.send(Message::Text(response.to_string().into())).await.is_err() { break; }
                if let Some(update) = order_update
                    && socket.send(Message::Text(update.to_string().into())).await.is_err() { break; }
            },
            message = messages.recv() => {
                let Ok(message) = message else { break };
                if socket.send(Message::Text(message.to_string().into())).await.is_err() { break; }
            }
        }
    }
}

pub(crate) struct Harness {
    pub(crate) source: Arc<Source>,
    pub(crate) config: HyperliquidExecutionClientConfig,
    pub(crate) origin: String,
    server: tokio::task::JoinHandle<()>,
}
impl Drop for Harness {
    fn drop(&mut self) {
        self.server.abort();
    }
}
impl Harness {
    pub(crate) async fn new(io_first: bool) -> Self {
        let meta = |coin| json!({"universe":[{"name":coin, "szDecimals":2, "maxLeverage":10}]});
        let (metas, dexes) = if io_first {
            (
                json!([meta("BTC"), meta("io:SNDK"), meta("xyz:SNDK")]),
                json!([null,{"name":"io"},{"name":"xyz"}]),
            )
        } else {
            (
                json!([meta("BTC"), meta("xyz:SNDK"), meta("io:SNDK")]),
                json!([null,{"name":"xyz"},{"name":"io"}]),
            )
        };
        let source = Arc::new(Source {
            metas,
            dexes,
            fills: Mutex::new(Vec::new()),
            history: Mutex::new(Vec::new()),
            actions: Mutex::new(Vec::new()),
            queries: Mutex::new(Vec::new()),
            messages: tokio::sync::broadcast::channel(32).0,
            subscriptions: AtomicUsize::new(0),
            hold_order_updates: AtomicBool::new(false),
            order_rows: Mutex::new(Vec::new()),
            open_orders: Mutex::new(Vec::new()),
            perp_positions: Mutex::new(Vec::new()),
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let router = Router::new()
            .route("/info", post(info))
            .route("/ws", get(websocket))
            .with_state(source.clone());
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let config = HyperliquidExecutionClientConfig {
            private_key: Some(format!("0x{}", "17".repeat(32)).into()),
            account_address: Some(format!("0x{}", "11".repeat(20))),
            base_url_http: Some(format!("{origin}/info")),
            base_url_exchange: Some(format!("{origin}/exchange")),
            base_url_ws: Some(format!("{}/ws", origin.replace("http:", "ws:"))),
            http_timeout_secs: 2,
            max_retries: 0,
            ws_post_timeout_secs: 2,
            ..Default::default()
        };
        Self {
            source,
            config,
            origin,
            server,
        }
    }

    pub(crate) async fn node(&self) -> LiveNode {
        let node = LiveNode::builder(TraderId::from("TESTER-001"), Environment::Live)
            .unwrap()
            .with_reconciliation(true)
            .with_timeout_connection(3)
            .with_timeout_reconciliation(3)
            .with_timeout_portfolio(3)
            .with_timeout_disconnection_secs(2)
            .with_delay_post_stop_secs(0)
            .with_delay_shutdown_secs(0)
            .add_exec_client(
                None,
                Box::new(HyperliquidExecutionClientFactory),
                Box::new(self.config.clone()),
            )
            .unwrap()
            .build()
            .unwrap();
        let mut http =
            HyperliquidHttpClient::new(HyperliquidEnvironment::Mainnet, 2, None).unwrap();
        http.set_base_info_url(format!("{}/info", self.origin));
        let instruments = http.request_instruments().await.unwrap();
        let cache = node.kernel().cache();
        for instrument in instruments {
            cache.borrow_mut().add_instrument(instrument).unwrap();
        }
        let now = nautilus_core::time::get_atomic_clock_realtime().get_time_ns();
        cache
            .borrow_mut()
            .add_quote(QuoteTick::new(
                INSTRUMENT.into(),
                Price::from("100"),
                Price::from("100"),
                Quantity::from("1"),
                Quantity::from("1"),
                now,
                now,
            ))
            .unwrap();
        // Mirrors the Entropy execution example's external_order_instrument_ids.
        cache
            .borrow_mut()
            .register_external_order_claims(
                nautilus_model::identifiers::StrategyId::from("ENTROPY-001"),
                &[INSTRUMENT.into()],
            )
            .unwrap();
        node
    }
}

pub(crate) async fn wait_for(ready: impl Fn() -> bool) -> anyhow::Result<()> {
    tokio::time::timeout(Duration::from_secs(8), async {
        while !ready() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .map_err(Into::into)
}

pub(crate) async fn run_to_running(node: &mut LiveNode) {
    let handle = node.handle();
    let observe = async {
        let result = wait_for(|| handle.is_running()).await;
        handle.stop();
        result
    };
    let (run, result) = tokio::join!(node.run(), observe);
    run.unwrap();
    result.unwrap();
}
