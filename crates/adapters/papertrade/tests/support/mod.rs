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
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
};

use axum::{
    Json, Router,
    extract::State,
    http::{StatusCode, Uri},
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

#[allow(
    dead_code,
    reason = "Only scan coverage tests audit the shared RPC transcript"
)]
pub(crate) mod scan;
use scan::ScanEvent;

pub(crate) static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub(crate) struct Source {
    pub(crate) intent_requests: AtomicUsize,
    pub(crate) intent_ack: Mutex<Option<Value>>,
    pub(crate) intent_paused: AtomicBool,
    pub(crate) intent_entered: tokio::sync::Notify,
    pub(crate) intent_release: tokio::sync::Notify,
    pub(crate) wallet: tokio::sync::watch::Sender<Value>,
    pub(crate) wallet_requests: AtomicUsize,
    pub(crate) wallet_initial_override: Mutex<Option<Value>>,
    pub(crate) wallet_delay_ms: AtomicU64,
    pub(crate) unavailable: AtomicBool,
    pub(crate) trading: Mutex<Value>,
    pub(crate) trading_requests: AtomicUsize,
    pub(crate) trading_delay_ms: AtomicU64,
    pub(crate) history: Mutex<Value>,
    pub(crate) receipt: Mutex<Option<Value>>,
    pub(crate) receipts: Mutex<Vec<Value>>,
    pub(crate) receipt_unavailable: Mutex<std::collections::HashSet<String>>,
    pub(crate) receipt_calls: Mutex<Vec<String>>,
    pub(crate) block_timestamp: Mutex<Option<u64>>,
    pub(crate) block_paused: AtomicBool,
    pub(crate) block_calls: AtomicUsize,
    pub(crate) log_ranges: Mutex<Vec<(u64, u64)>>,
    pub(crate) scan_events: Mutex<Vec<ScanEvent>>,
    pub(crate) log_fail_from: Mutex<Option<u64>>,
    pub(crate) history_delay_ms: AtomicU64,
    pub(crate) history_calls: AtomicUsize,
}
async fn wallet(State(source): State<Arc<Source>>) -> Response {
    source.wallet_requests.fetch_add(1, Ordering::SeqCst);
    let delay = source.wallet_delay_ms.load(Ordering::SeqCst);
    if delay > 0 {
        tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
    }
    if source.unavailable.load(Ordering::SeqCst) {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    let stream = futures_util::stream::unfold(
        (
            source.wallet.subscribe(),
            true,
            source.wallet_initial_override.lock().clone(),
        ),
        |(mut rx, first, initial)| async move {
            if !first && rx.changed().await.is_err() {
                return None;
            }
            let value = if first {
                initial.unwrap_or_else(|| rx.borrow().clone())
            } else {
                rx.borrow().clone()
            };
            Some((
                Ok::<_, Infallible>(Event::default().event("wallet").data(value.to_string())),
                (rx, false, None),
            ))
        },
    );
    Sse::new(stream).into_response()
}
async fn trading(State(source): State<Arc<Source>>) -> Json<Value> {
    source.trading_requests.fetch_add(1, Ordering::SeqCst);
    tokio::time::sleep(std::time::Duration::from_millis(
        source.trading_delay_ms.load(Ordering::SeqCst),
    ))
    .await;
    Json(source.trading.lock().clone())
}
async fn history(State(source): State<Arc<Source>>, uri: Uri) -> Json<Value> {
    source.history_calls.fetch_add(1, Ordering::SeqCst);
    let delay = source.history_delay_ms.load(Ordering::SeqCst);
    if delay > 0 {
        tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
        if !uri.query().unwrap_or_default().contains("cursor=") {
            return Json(json!({"items":[],"nextCursor":"slow-page"}));
        }
    }
    Json(source.history.lock().clone())
}
async fn rpc(State(source): State<Arc<Source>>, Json(request): Json<Value>) -> Json<Value> {
    let result = if request["method"] == "eth_getBlockByNumber" {
        source.block_calls.fetch_add(1, Ordering::SeqCst);
        while source.block_paused.load(Ordering::SeqCst) {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        source.block_timestamp.lock().map(|stamp| json!({"number":request["params"][0],"timestamp":format!("0x{stamp:x}"),"hash":format!("0x{}","11".repeat(32))})).unwrap_or(Value::Null)
    } else if request["method"] == "eth_getTransactionReceipt" {
        let hash = request["params"][0].as_str().unwrap().to_owned();
        source.receipt_calls.lock().push(hash.clone());
        let receipt = if source.receipt_unavailable.lock().contains(&hash) {
            Value::Null
        } else {
            source
                .receipts
                .lock()
                .iter()
                .chain(source.receipt.lock().iter())
                .find(|receipt| receipt["transactionHash"] == hash)
                .cloned()
                .unwrap_or(Value::Null)
        };
        source.scan_events.lock().push(ScanEvent::Receipt {
            hash: hash.parse().unwrap(),
            available: receipt.is_object() && receipt["status"] == "0x1",
        });
        receipt
    } else {
        let from = u64::from_str_radix(
            request["params"][0]["fromBlock"]
                .as_str()
                .unwrap()
                .trim_start_matches("0x"),
            16,
        )
        .unwrap();
        let to = u64::from_str_radix(
            request["params"][0]["toBlock"]
                .as_str()
                .unwrap()
                .trim_start_matches("0x"),
            16,
        )
        .unwrap();
        let receipts: Vec<_> = source
            .receipts
            .lock()
            .iter()
            .chain(source.receipt.lock().iter())
            .cloned()
            .collect();
        source.log_ranges.lock().push((from, to));
        let intents = request["params"][0]["topics"][3]
            .as_array()
            .unwrap()
            .iter()
            .map(|id| serde_json::from_value(id.clone()).unwrap())
            .collect();
        let bound = source.trading.lock()["indexedThroughBlock"]
            .as_u64()
            .unwrap()
            .min(
                source.wallet.borrow()["indexedThroughBlock"]
                    .as_u64()
                    .unwrap(),
            );
        if *source.log_fail_from.lock() == Some(from) {
            source.scan_events.lock().push(ScanEvent::Logs {
                from,
                to,
                intents,
                bound,
                transactions: None,
            });
            return Json(
                json!({"jsonrpc":"2.0","id":1,"error":{"code":-32000,"message":"Injected log failure"}}),
            );
        }
        let logs: Vec<Value> = receipts
            .iter()
            .filter_map(|r| r["logs"].as_array())
            .flatten()
            .filter(|log| {
                let number = log["blockNumber"]
                    .as_str()
                    .and_then(|b| u64::from_str_radix(b.trim_start_matches("0x"), 16).ok());
                number.is_some_and(|n| n >= from && n <= to)
                    && request["params"][0]["topics"][3]
                        .as_array()
                        .unwrap()
                        .contains(&log["topics"][3])
            })
            .cloned()
            .collect();
        source.scan_events.lock().push(ScanEvent::Logs {
            from,
            to,
            intents,
            bound,
            transactions: Some(
                logs.iter()
                    .map(|log| serde_json::from_value(log["transactionHash"].clone()).unwrap())
                    .collect(),
            ),
        });
        json!(logs)
    };
    Json(json!({"jsonrpc":"2.0","id":1,"result":result}))
}
async fn intent_request(State(source): State<Arc<Source>>) -> (StatusCode, Json<Value>) {
    source.intent_requests.fetch_add(1, Ordering::SeqCst);
    let ack = source.intent_ack.lock().clone();
    if source.intent_paused.load(Ordering::SeqCst) {
        source.intent_entered.notify_one();
        source.intent_release.notified().await;
    }
    match ack {
        Some(body) => (StatusCode::OK, Json(body)),
        None => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error":"acknowledgement lost"})),
        ),
    }
}

pub(crate) struct Harness {
    pub(crate) source: Arc<Source>,
    pub(crate) config: PapertradeExecutionClientConfig,
    pub(crate) cache: Rc<RefCell<Cache>>,
    server: tokio::task::JoinHandle<()>,
    pub(crate) _events:
        tokio::sync::mpsc::UnboundedReceiver<nautilus_common::messages::ExecutionEvent>,
    _dir: tempfile::TempDir,
}
impl Harness {
    pub(crate) async fn new() -> Self {
        let mut snapshot: Value =
            serde_json::from_str(include_str!("../../test_data/live_wallet.json")).unwrap();
        let now = get_atomic_clock_realtime().get_time_ns().as_u64() / 1_000_000_000;
        snapshot["indexedThroughTimestamp"] = json!(now);
        snapshot["sessionKeys"] = json!([[
            "0x19e7e376e7c213b7e7e7e46cc70a5dd086daff2a",
            (now + 86400).to_string()
        ]]);
        let (tx, _) = tokio::sync::watch::channel(snapshot.clone());
        let source = Arc::new(Source {
            intent_requests: AtomicUsize::new(0),
            intent_ack: Mutex::new(None),
            intent_paused: AtomicBool::new(false),
            intent_entered: tokio::sync::Notify::new(),
            intent_release: tokio::sync::Notify::new(),
            wallet: tx,
            wallet_requests: AtomicUsize::new(0),
            wallet_initial_override: Mutex::new(None),
            wallet_delay_ms: AtomicU64::new(0),
            unavailable: AtomicBool::new(false),
            history: Mutex::new(json!({"items":[],"nextCursor":null})),
            receipt: Mutex::new(None),
            receipts: Mutex::new(Vec::new()),
            receipt_unavailable: Mutex::new(std::collections::HashSet::new()),
            receipt_calls: Mutex::new(Vec::new()),
            block_timestamp: Mutex::new(None),
            block_paused: AtomicBool::new(false),
            block_calls: AtomicUsize::new(0),
            log_ranges: Mutex::new(Vec::new()),
            scan_events: Mutex::new(Vec::new()),
            log_fail_from: Mutex::new(None),
            history_delay_ms: AtomicU64::new(0),
            history_calls: AtomicUsize::new(0),
            trading_requests: AtomicUsize::new(0),
            trading_delay_ms: AtomicU64::new(0),
            trading: Mutex::new(
                serde_json::from_str(include_str!("../../test_data/live_trading.json")).unwrap(),
            ),
        });
        let app = Router::new()
            .route("/state/user/live", get(wallet))
            .route("/state/trading", get(trading))
            .route("/query/accounts/{wallet}/trades", get(history))
            .route("/rpc", post(rpc))
            .route("/intents", post(intent_request))
            .route("/intents/{id}/cancel", post(intent_request))
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
    #[allow(dead_code, reason = "Shared harness also serves full LiveNode tests")]
    pub(crate) fn client(&self) -> Box<dyn ExecutionClient> {
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

pub(crate) fn closed_history(
    position: &nautilus_papertrade::wire::VenuePosition,
    timestamp_ms: u64,
) -> Value {
    json!({"positionId":position.position_id,"instrumentId":position.instrument_id,"symbol":"BTC",
        "priceScaleRaw":"10","isLong":position.is_long,"entryPriceRaw":position.entry_price,
        "marginRaw":position.margin,"leverage":position.leverage,"openedAtMs":position.opened_at_ms,
        "timestampMs":timestamp_ms,"txHash":format!("0x{}","31".repeat(32)),"kind":"PositionClosed",
        "exitPriceRaw":position.entry_price,"adjustedPnlRaw":"0","userPaidFeeRaw":"0"})
}
pub(crate) fn prepared_open(
    harness: &Harness,
    position: &nautilus_papertrade::wire::VenuePosition,
    now: u64,
    deadline: u64,
) -> nautilus_papertrade::journal::JournalEntry {
    prepared_open_with_id(harness, position, "PT-OPEN-RECOVERY", now, deadline)
}

pub(crate) fn prepared_open_with_id(
    harness: &Harness,
    position: &nautilus_papertrade::wire::VenuePosition,
    client_id: &str,
    now: u64,
    deadline: u64,
) -> nautilus_papertrade::journal::JournalEntry {
    use alloy::signers::local::PrivateKeySigner;
    use alloy_primitives::B256;
    use nautilus_common::messages::execution::SubmitOrder;
    use nautilus_core::{UUID4, UnixNanos};
    use nautilus_model::{
        enums::{OrderSide, OrderType},
        identifiers::{ClientOrderId, InstrumentId},
        orders::OrderTestBuilder,
        types::Quantity,
    };
    use nautilus_papertrade::{
        journal::Journal,
        signing::{OpenPosition, PapertradeSigner},
    };
    let wallet = harness.source.wallet.borrow();
    let order = OrderTestBuilder::new(OrderType::Market)
        .instrument_id(InstrumentId::from("BTC-USD-PERP.PAPERTRADE"))
        .client_order_id(ClientOrderId::from(client_id))
        .side(if position.is_long {
            OrderSide::Buy
        } else {
            OrderSide::Sell
        })
        .quantity(
            Quantity::from_decimal_dp(
                position.margin.decimal().unwrap() * rust_decimal::Decimal::from(position.leverage),
                2,
            )
            .unwrap(),
        )
        .quote_quantity(true)
        .build();
    harness
        .cache
        .borrow_mut()
        .add_order(order.clone(), None, None, false)
        .unwrap();
    let command = SubmitOrder::from_order(
        &order,
        TraderId::from("TESTER-001"),
        None,
        None,
        UUID4::new(),
        UnixNanos::from(1),
    );
    let signer =
        PapertradeSigner::new(PrivateKeySigner::from_bytes(&B256::repeat_byte(17)).unwrap());
    let journal = Journal::open(
        &harness.config.journal_path,
        wallet["address"].as_str().unwrap().parse().unwrap(),
        "PAPERTRADE-001",
    )
    .unwrap();
    journal
        .prepare(
            &command,
            None,
            wallet["indexedThroughBlock"].as_u64().unwrap(),
            |nonce| {
                signer.sign_open(
                    &OpenPosition {
                        user: wallet["address"].as_str().unwrap().parse().unwrap(),
                        instrumentId: position.instrument_id,
                        isLong: position.is_long,
                        size: u128::try_from(position.margin.0).unwrap(),
                        leverage: position.leverage,
                        useDebt: false,
                        nonce,
                        deadline,
                    },
                    now,
                )
            },
        )
        .unwrap()
        .0
}

pub(crate) fn mark_attempted(
    harness: &Harness,
    entry: &nautilus_papertrade::journal::JournalEntry,
) {
    let wallet = harness.source.wallet.borrow();
    let journal = nautilus_papertrade::journal::Journal::open(
        &harness.config.journal_path,
        wallet["address"].as_str().unwrap().parse().unwrap(),
        "PAPERTRADE-001",
    )
    .unwrap();
    journal
        .begin_submission(entry.command.client_order_id)
        .unwrap()
        .unwrap();
}

pub(crate) fn write_legacy_expired(
    path: &std::path::Path,
    entry: &nautilus_papertrade::journal::JournalEntry,
) {
    use redb::{Database, TableDefinition};
    let mut old = entry.clone();
    old.status = nautilus_model::enums::OrderStatus::Expired;
    old.send_attempted = true;
    old.scanned_through = old.submitted_block + 100;
    let mut value = serde_json::to_value(old).unwrap();
    value.as_object_mut().unwrap().remove("execution_succeeded");
    let db = Database::open(path).unwrap();
    let tx = db.begin_write().unwrap();
    {
        let mut table = tx
            .open_table(TableDefinition::<&str, &str>::new("orders"))
            .unwrap();
        table
            .insert(
                entry.command.client_order_id.as_str(),
                value.to_string().as_str(),
            )
            .unwrap();
    }
    tx.commit().unwrap();
}
