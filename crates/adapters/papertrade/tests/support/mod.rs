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

pub(crate) static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub(crate) struct Source {
    pub(crate) wallet: tokio::sync::watch::Sender<Value>,
    pub(crate) unavailable: AtomicBool,
    pub(crate) trading: Mutex<Value>,
    pub(crate) history: Mutex<Value>,
    pub(crate) receipt: Mutex<Option<Value>>,
    pub(crate) block_timestamp: Mutex<Option<u64>>,
    pub(crate) history_delay_ms: AtomicU64,
    pub(crate) history_calls: AtomicUsize,
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
        source.block_timestamp.lock().map(|stamp| json!({"number":request["params"][0],"timestamp":format!("0x{stamp:x}"),"hash":format!("0x{}","11".repeat(32))})).unwrap_or(Value::Null)
    } else if request["method"] == "eth_getTransactionReceipt" {
        source.receipt.lock().clone().unwrap_or(Value::Null)
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
        let receipt = source.receipt.lock();
        let logs: Vec<Value> = receipt
            .as_ref()
            .and_then(|r| r["logs"].as_array())
            .into_iter()
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
        json!(logs)
    };
    Json(json!({"jsonrpc":"2.0","id":1,"result":result}))
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
            wallet: tx,
            unavailable: AtomicBool::new(false),
            history: Mutex::new(json!({"items":[],"nextCursor":null})),
            receipt: Mutex::new(None),
            block_timestamp: Mutex::new(None),
            history_delay_ms: AtomicU64::new(0),
            history_calls: AtomicUsize::new(0),
            trading: Mutex::new(
                serde_json::from_str(include_str!("../../test_data/live_trading.json")).unwrap(),
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
    use alloy::signers::local::PrivateKeySigner;
    use alloy_primitives::B256;
    use nautilus_common::messages::execution::SubmitOrder;
    use nautilus_core::{UUID4, UnixNanos};
    use nautilus_model::{
        enums::OrderType,
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
        .client_order_id(ClientOrderId::from("PT-OPEN-RECOVERY"))
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
