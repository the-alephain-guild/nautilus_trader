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
    history: Mutex<Value>,
    receipt: Mutex<Option<Value>>,
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
async fn history(State(source): State<Arc<Source>>) -> Json<Value> {
    Json(source.history.lock().clone())
}
async fn rpc(State(source): State<Arc<Source>>, Json(request): Json<Value>) -> Json<Value> {
    let result = if request["method"] == "eth_getTransactionReceipt" {
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
            history: Mutex::new(json!({"items":[],"nextCursor":null})),
            receipt: Mutex::new(None),
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

#[tokio::test]
async fn restart_rejects_regressed_coverage_and_keeps_the_durable_checkpoint() {
    let _serial = SERIAL.lock().await;
    let harness = Harness::new().await;
    let current = harness.source.wallet.borrow().clone();
    let mut client = harness.client();
    client.start().unwrap();
    client.connect().await.unwrap();
    client.disconnect().await.unwrap();
    drop(client);
    let mut stale = current.clone();
    stale["indexedThroughBlock"] = json!(current["indexedThroughBlock"].as_u64().unwrap() - 1);
    stale["indexedThroughTimestamp"] =
        json!(current["indexedThroughTimestamp"].as_u64().unwrap() - 1);
    harness.source.wallet.send_replace(stale);
    let mut restarted = harness.client();
    restarted.start().unwrap();
    assert!(
        restarted.connect().await.is_err(),
        "Restart accepted coverage older than its checkpoint"
    );
    assert!(!restarted.is_connected());
    restarted.disconnect().await.unwrap();
    drop(restarted);
    harness.source.wallet.send_replace(current);
    let mut recovered = harness.client();
    recovered.start().unwrap();
    recovered.connect().await.unwrap();
    assert!(recovered.is_connected());
    recovered.disconnect().await.unwrap();
}

#[tokio::test]
async fn failed_position_reconciliation_does_not_advance_restart_checkpoint() {
    let _serial = SERIAL.lock().await;
    let harness = Harness::new().await;
    let current = harness.source.wallet.borrow().clone();
    let mut client = harness.client();
    client.start().unwrap();
    client.connect().await.unwrap();
    client.disconnect().await.unwrap();
    drop(client);
    let mut missing = current.clone();
    missing["indexedThroughBlock"] = json!(current["indexedThroughBlock"].as_u64().unwrap() + 10);
    missing["indexedThroughTimestamp"] =
        json!(current["indexedThroughTimestamp"].as_u64().unwrap() + 10);
    missing["walletRevision"] = missing["indexedThroughBlock"].clone();
    missing["openPositions"] = json!([]);
    missing["locked"] = json!("0");
    harness.source.wallet.send_replace(missing);
    let mut failed = harness.client();
    failed.start().unwrap();
    assert!(failed.connect().await.is_err());
    failed.disconnect().await.unwrap();
    drop(failed);
    harness.source.wallet.send_replace(current);
    let mut recovered = harness.client();
    recovered.start().unwrap();
    recovered
        .connect()
        .await
        .expect("A failed reconciliation poisoned the restart checkpoint");
    recovered.disconnect().await.unwrap();
}

#[tokio::test]
async fn delayed_close_receipt_never_publishes_an_external_identity_for_a_local_fill() {
    use alloy::signers::local::PrivateKeySigner;
    use alloy_primitives::{B256, U256};
    use nautilus_common::messages::execution::SubmitOrder;
    use nautilus_core::{UUID4, UnixNanos};
    use nautilus_model::{
        enums::{OrderSide, OrderStatus, OrderType},
        identifiers::{ClientOrderId, InstrumentId},
        orders::OrderTestBuilder,
    };
    use nautilus_papertrade::{
        journal::Journal,
        reports,
        signing::{Close, PapertradeSigner},
        wire::{TradingState, VenuePosition},
    };
    let _serial = SERIAL.lock().await;
    let harness = Harness::new().await;
    let mut source = harness.source.wallet.borrow().clone();
    let position: VenuePosition =
        serde_json::from_value(source["openPositions"][0].clone()).unwrap();
    let markets: TradingState =
        serde_json::from_value(harness.source.trading.lock().clone()).unwrap();
    let mut client = harness.client();
    client.start().unwrap();
    client.connect().await.unwrap();
    client.disconnect().await.unwrap();
    drop(client);
    let order = OrderTestBuilder::new(OrderType::Market)
        .instrument_id(InstrumentId::from("BTC-USD-PERP.PAPERTRADE"))
        .client_order_id(ClientOrderId::from("PT-CLOSE-RECOVERY"))
        .side(OrderSide::Sell)
        .quantity(reports::quantity(&position, &markets.instruments[0]).unwrap())
        .reduce_only(true)
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
    let now = source["indexedThroughTimestamp"].as_u64().unwrap();
    let signer =
        PapertradeSigner::new(PrivateKeySigner::from_bytes(&B256::repeat_byte(17)).unwrap());
    let journal = Journal::open(
        &harness.config.journal_path,
        source["address"].as_str().unwrap().parse().unwrap(),
        "PAPERTRADE-001",
    )
    .unwrap();
    let (entry, _) = journal
        .prepare(
            &command,
            Some(position.position_id),
            source["indexedThroughBlock"].as_u64().unwrap(),
            |nonce| {
                signer.sign_close(
                    &Close {
                        user: source["address"].as_str().unwrap().parse().unwrap(),
                        positionIds: vec![position.position_id.0],
                        nonce,
                        deadline: now + 3600,
                    },
                    now,
                )
            },
        )
        .unwrap();
    drop(journal);
    let tx = B256::repeat_byte(31);
    let block = B256::repeat_byte(32);
    let exit = position.entry_price.0 + U256::from(100);
    *harness.source.history.lock() = json!({"items":[{
        "positionId":position.position_id,"instrumentId":position.instrument_id,"symbol":"BTC","priceScaleRaw":"10","isLong":position.is_long,"entryPriceRaw":position.entry_price,"marginRaw":position.margin,"leverage":position.leverage,"openedAtMs":position.opened_at_ms,"timestampMs":now*1000,"txHash":tx,"kind":"PositionClosed","exitPriceRaw":exit.to_string(),"adjustedPnlRaw":"0","userPaidFeeRaw":"0"
    }],"nextCursor":null});
    source["openPositions"].as_array_mut().unwrap().remove(0);
    source["locked"] = json!(source["openPositions"][0][5].as_str().unwrap());
    source["walletRevision"] = source["indexedThroughBlock"].clone();
    source["accountRevision"] = json!(2);
    harness.source.wallet.send_replace(source.clone());
    let mut recovered = harness.client();
    recovered.start().unwrap();
    assert!(
        recovered.connect().await.is_err(),
        "Missing attribution evidence was published as an external close"
    );
    let user: alloy_primitives::Address = source["address"].as_str().unwrap().parse().unwrap();
    let log = json!({"address":nautilus_papertrade::chain::BATCH_EXECUTOR,"topics":[nautilus_papertrade::chain::RESULT_TOPIC,B256::ZERO,B256::ZERO,entry.intent_id],"transactionHash":tx,"blockHash":block,"blockNumber":format!("0x{:x}",source["indexedThroughBlock"].as_u64().unwrap()),"removed":false,"data":format!("0x{:064x}",U256::from(1))});
    let close = json!({"address":nautilus_papertrade::signing::EXCHANGE,"topics":["0x87902ed2451849f465fc4d36a01d7f5c67b53af12f63ee85d2814a2d582f89bf",format!("0x{:064x}",position.position_id.0),format!("0x{:064x}",U256::from_be_slice(user.as_slice()))],"transactionHash":tx,"blockHash":block,"removed":false,"data":format!("0x{exit:064x}{}","0".repeat(256))});
    *harness.source.receipt.lock() = Some(
        json!({"status":"0x1","transactionHash":tx,"blockHash":block,"blockNumber":log["blockNumber"],"logs":[log,close]}),
    );
    recovered.connect().await.unwrap();
    let mass = recovered.generate_mass_status(None).await.unwrap().unwrap();
    let closes: Vec<_> = mass
        .fill_reports()
        .into_values()
        .flatten()
        .filter(|f| {
            f.venue_position_id == Some(reports::position_id(&position))
                && f.order_side == OrderSide::Sell
        })
        .collect();
    assert_eq!(closes.len(), 1);
    assert_eq!(closes[0].client_order_id, Some(command.client_order_id));
    assert_eq!(
        closes[0].venue_order_id.to_string(),
        format!("{:#x}", entry.intent_id)
    );
    assert!(
        mass.order_reports()
            .values()
            .any(|r| r.client_order_id == Some(command.client_order_id)
                && r.order_status == OrderStatus::Filled)
    );
    recovered.disconnect().await.unwrap();
}

fn prepared_open(
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

#[rstest::rstest]
#[case(false)]
#[case(true)]
#[tokio::test]
async fn newer_chain_evidence_cannot_complete_an_older_empty_account_snapshot(
    #[case] account_covers_execution: bool,
) {
    use alloy_primitives::b256;
    use nautilus_papertrade::wire::VenuePosition;
    let _serial = SERIAL.lock().await;
    let harness = Harness::new().await;
    let mut source = harness.source.wallet.borrow().clone();
    let mut position: VenuePosition =
        serde_json::from_value(source["openPositions"][0].clone()).unwrap();
    source["openPositions"] = json!([]);
    source["locked"] = json!("0");
    let covered = source["indexedThroughBlock"].as_u64().unwrap();
    harness.source.wallet.send_replace(source.clone());
    let now = source["indexedThroughTimestamp"].as_u64().unwrap();
    let entry = prepared_open(&harness, &position, now, now + 3600);
    let mut receipt: Value =
        serde_json::from_str(include_str!("../test_data/live_open_receipt.json")).unwrap();
    let old_id = b256!("adfae189b04060dcbef2af7322b00447844d8119b819e1820308e38e86339614");
    let execution_block = covered + u64::from(!account_covers_execution);
    receipt["blockNumber"] = json!(format!("0x{execution_block:x}"));
    for log in receipt["logs"].as_array_mut().unwrap() {
        log["blockNumber"] = json!(format!("0x{execution_block:x}"));
        if log["topics"][3] == json!(old_id) {
            log["topics"][3] = json!(entry.intent_id);
        }
    }
    *harness.source.receipt.lock() = Some(receipt);
    harness.source.trading.lock()["indexedThroughBlock"] = json!(covered + 2);
    let mut client = harness.client();
    client.start().unwrap();
    let connected = client.connect().await;
    if account_covers_execution {
        assert!(
            connected.is_err(),
            "Covered execution disappeared from the account without terminal history"
        );
    } else {
        connected.unwrap();
        let mass = client.generate_mass_status(None).await.unwrap().unwrap();
        assert!(
            !mass.reports_complete(),
            "Future chain fill was paired with authoritative empty positions"
        );
        assert!(mass.fill_reports().is_empty());
        assert!(mass.position_reports().is_empty());
    }
    client.disconnect().await.unwrap();
    drop(client);
    position.intent_id = Some(entry.intent_id);
    source["openPositions"] = json!([position]);
    source["locked"] = json!(position.margin);
    source["indexedThroughBlock"] = json!(covered + 2);
    source["walletRevision"] = json!(covered + 1);
    source["accountRevision"] = json!(2);
    harness.source.wallet.send_replace(source);
    let mut restarted = harness.client();
    restarted.start().unwrap();
    restarted.connect().await.unwrap();
    let mass = restarted.generate_mass_status(None).await.unwrap().unwrap();
    assert!(mass.reports_complete());
    assert_eq!(
        mass.position_reports()
            .values()
            .map(Vec::len)
            .sum::<usize>(),
        1
    );
    assert_eq!(mass.fill_reports().values().map(Vec::len).sum::<usize>(), 1);
    assert_eq!(
        mass.fill_reports()
            .values()
            .flatten()
            .next()
            .unwrap()
            .client_order_id,
        Some(entry.command.client_order_id)
    );
    restarted.disconnect().await.unwrap();
}
