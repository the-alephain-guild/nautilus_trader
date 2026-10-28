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
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use alloy::signers::local::PrivateKeySigner;
use alloy_primitives::B256;
use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::{
        Sse,
        sse::{Event, KeepAlive},
    },
    routing::{get, post},
};
use futures_util::stream::Stream;
use nautilus_common::{
    cache::Cache, factories::ExecutionClientFactory, live::runner::replace_exec_event_sender,
    messages::execution::SubmitOrder,
};
use nautilus_core::{UUID4, UnixNanos, time::get_atomic_clock_realtime};
use nautilus_model::{
    enums::{OrderType, TimeInForce},
    identifiers::{AccountId, ClientOrderId, InstrumentId, TraderId},
    orders::OrderTestBuilder,
    types::Quantity,
};
use nautilus_papertrade::{
    config::{PAPERTRADE, PapertradeExecutionClientConfig},
    factories::PapertradeExecutionClientFactory,
};
use parking_lot::Mutex;
use serde_json::{Value, json};

struct ServerState {
    attempt_id: Mutex<Option<B256>>,
    rpc_calls: AtomicUsize,
    trading_calls: AtomicUsize,
    wallet: tokio::sync::watch::Sender<Value>,
    posts: Mutex<Vec<Value>>,
    count: AtomicUsize,
}
async fn wallet(
    State(state): State<Arc<ServerState>>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let stream = futures_util::stream::unfold(
        (state.wallet.subscribe(), true),
        |(mut rx, first)| async move {
            if !first && rx.changed().await.is_err() {
                return None;
            }
            let value = rx.borrow().clone();
            Some((
                Ok(Event::default().event("wallet").data(value.to_string())),
                (rx, false),
            ))
        },
    );
    Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(1)))
}
async fn submit(
    State(state): State<Arc<ServerState>>,
    Json(body): Json<Value>,
) -> (StatusCode, &'static str) {
    state.posts.lock().push(body);
    state.count.fetch_add(1, Ordering::SeqCst);
    (StatusCode::INTERNAL_SERVER_ERROR, "acknowledgement lost")
}
async fn trading(State(state): State<Arc<ServerState>>) -> Json<Value> {
    let mut value: Value =
        serde_json::from_str(include_str!("../test_data/live_trading.json")).unwrap();
    value["indexedThroughBlock"] = json!(
        value["indexedThroughBlock"].as_u64().unwrap()
            + state.trading_calls.fetch_add(1, Ordering::SeqCst) as u64
    );
    Json(value)
}
async fn history() -> Json<Value> {
    Json(json!({"items":[],"nextCursor":null}))
}
async fn rpc(State(state): State<Arc<ServerState>>, Json(request): Json<Value>) -> Json<Value> {
    let Some(id) = *state.attempt_id.lock() else {
        return Json(json!({"jsonrpc":"2.0","id":1,"result":[]}));
    };
    state.rpc_calls.fetch_add(1, Ordering::SeqCst);
    let hash = B256::repeat_byte(44);
    let block = B256::repeat_byte(55);
    let log = json!({"address":nautilus_papertrade::chain::BATCH_EXECUTOR,
        "topics":[nautilus_papertrade::chain::RESULT_TOPIC,B256::ZERO,B256::ZERO,id],
        "transactionHash":hash,"blockHash":block,"removed":false,"data":format!("0x{}","00".repeat(96))});
    let result = if request["method"] == "eth_getLogs" {
        json!([log])
    } else {
        json!({"status":"0x1","transactionHash":hash,"blockHash":block,"logs":[log]})
    };
    Json(json!({"jsonrpc":"2.0","id":1,"result":result}))
}
#[tokio::test]
async fn lost_acknowledgement_reconnect_and_restart_do_not_resubmit_or_duplicate_fills() {
    let dir = tempfile::tempdir().unwrap();
    let key = PrivateKeySigner::from_bytes(&B256::repeat_byte(17)).unwrap();
    let mut source: Value =
        serde_json::from_str(include_str!("../test_data/live_wallet.json")).unwrap();
    let now = get_atomic_clock_realtime().get_time_ns().as_u64() / 1_000_000_000;
    source["indexedThroughTimestamp"] = json!(now);
    source["sessionKeys"] = json!([[key.address(), (now + 86_400).to_string()]]);
    let (wallet_tx, _) = tokio::sync::watch::channel(source.clone());
    let state = Arc::new(ServerState {
        attempt_id: Mutex::new(None),
        rpc_calls: AtomicUsize::new(0),
        trading_calls: AtomicUsize::new(0),
        wallet: wallet_tx,
        posts: Mutex::new(Vec::new()),
        count: AtomicUsize::new(0),
    });
    let app = Router::new()
        .route("/state/trading", get(trading))
        .route("/state/user/live", get(wallet))
        .route("/query/accounts/{wallet}/trades", get(history))
        .route("/intents", post(submit))
        .route("/rpc", post(rpc))
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}/", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let cache = Rc::new(RefCell::new(Cache::default()));
    let order = OrderTestBuilder::new(OrderType::Market)
        .instrument_id(InstrumentId::from("BTC-USD-PERP.PAPERTRADE"))
        .client_order_id(ClientOrderId::from("PT-LOST-ACK"))
        .quantity(Quantity::from(1000))
        .quote_quantity(true)
        .time_in_force(TimeInForce::Gtc)
        .build();
    cache
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
    let mut config = PapertradeExecutionClientConfig::new(
        AccountId::from("PAPERTRADE-001"),
        source["address"].as_str().unwrap().to_owned(),
        dir.path().join("journal.redb"),
        "test-build".into(),
    );
    config.session_private_key = Some(format!("{:#x}", B256::repeat_byte(17)).into());
    config.origin = origin.clone();
    config.rpc_url = format!("{origin}rpc");
    config.poll_interval_ms = 500;
    let (events, mut receiver) = tokio::sync::mpsc::unbounded_channel();
    replace_exec_event_sender(events);
    let factory = PapertradeExecutionClientFactory;
    let mut client = factory
        .create(
            TraderId::from("TESTER-001"),
            PAPERTRADE,
            &config,
            cache.clone().into(),
        )
        .unwrap();
    // Exercise submission after the fixture's original wall-clock second.
    tokio::time::sleep(Duration::from_secs(2)).await;
    client.start().unwrap();
    client.connect().await.unwrap();
    assert_eq!(
        client
            .generate_mass_status(None)
            .await
            .unwrap()
            .unwrap()
            .position_reports()
            .len(),
        1
    );
    client.submit_order(command.clone()).unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while state.count.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    client.submit_order(command.clone()).unwrap();
    assert_eq!(state.count.load(Ordering::SeqCst), 1);
    let request = state.posts.lock()[0].clone();
    let journal_id = {
        // Compute the same identifier from the independently persisted request metadata
        let entries = std::fs::metadata(&config.journal_path).unwrap();
        assert!(entries.len() > 0);
        let mass = client.generate_mass_status(None).await.unwrap().unwrap();
        let _ = mass;
        let fields = &request["signed"];
        let signed_at = fields["deadline"]
            .as_str()
            .unwrap()
            .parse::<u64>()
            .unwrap()
            .checked_sub(3_600)
            .unwrap();
        let signer = nautilus_papertrade::PapertradeSigner::new(key);
        signer
            .sign_open(
                &nautilus_papertrade::OpenPosition {
                    user: fields["user"].as_str().unwrap().parse().unwrap(),
                    instrumentId: 0,
                    isLong: true,
                    size: fields["size"].as_str().unwrap().parse().unwrap(),
                    leverage: 10,
                    useDebt: false,
                    nonce: fields["nonce"].as_str().unwrap().parse().unwrap(),
                    deadline: fields["deadline"].as_str().unwrap().parse().unwrap(),
                },
                signed_at,
            )
            .unwrap()
            .id()
    };
    *state.attempt_id.lock() = Some(journal_id);
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if state.rpc_calls.load(Ordering::SeqCst) >= 2
                && client
                    .generate_mass_status(None)
                    .await
                    .unwrap()
                    .unwrap()
                    .order_reports()
                    .values()
                    .any(|r| r.client_order_id == Some(command.client_order_id))
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let before = client.generate_mass_status(None).await.unwrap().unwrap();
    let pending = before
        .order_reports()
        .into_values()
        .find(|r| r.client_order_id == Some(command.client_order_id))
        .unwrap();
    assert_ne!(
        pending.order_status,
        nautilus_model::enums::OrderStatus::Rejected
    );
    assert!(pending.filled_qty.is_zero());
    source["accountRevision"] = json!(2);
    source["openPositions"].as_array_mut().unwrap().push(json!([
        "1671875",
        0,
        true,
        false,
        "830000",
        request["signed"]["size"],
        10,
        "0",
        now * 1000,
        journal_id
    ]));
    source["locked"] = json!("1000100000000000000000000");
    state.wallet.send(source.clone()).unwrap();
    tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            if let Ok(Some(mass)) = client.generate_mass_status(None).await
                && mass.order_reports().values().any(|r| {
                    r.client_order_id == Some(command.client_order_id)
                        && r.filled_qty.as_decimal() > rust_decimal::Decimal::ZERO
                })
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    state
        .wallet
        .send(
            json!({"address":source["address"],"baseAccountRevision":999,
        "accountRevision":1000,"state":{},"addedPositions":[],"removedPositionIds":[],
        "addedOperations":[],"removedOperationIds":[]}),
        )
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while client.is_connected() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    state.wallet.send(source).unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while !client.is_connected() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(state.count.load(Ordering::SeqCst), 1);
    let mut normalized = false;
    while let Ok(event) = receiver.try_recv() {
        if let nautilus_common::messages::ExecutionEvent::Order(
            nautilus_model::events::OrderEventAny::Updated(update),
        ) = event
            && update.client_order_id == command.client_order_id
        {
            assert!(!update.is_quote_quantity);
            assert!(update.quantity.as_decimal() < rust_decimal::Decimal::ONE);
            normalized = true;
        }
    }
    assert!(
        normalized,
        "Actual base quantity must be published before its fill"
    );
    client.disconnect().await.unwrap();
    drop(client);
    let mut restarted = factory
        .create(
            TraderId::from("TESTER-001"),
            PAPERTRADE,
            &config,
            cache.into(),
        )
        .unwrap();
    restarted.start().unwrap();
    restarted.connect().await.unwrap();
    let mass = restarted.generate_mass_status(None).await.unwrap().unwrap();
    assert_eq!(
        mass.position_reports()
            .values()
            .map(Vec::len)
            .sum::<usize>(),
        3
    );
    let fills: Vec<_> = mass
        .fill_reports()
        .into_values()
        .flatten()
        .filter(|f| f.client_order_id == Some(command.client_order_id))
        .collect();
    assert_eq!(fills.len(), 1);
    assert_eq!(
        fills[0].venue_order_id.to_string(),
        format!("{journal_id:#x}")
    );
    restarted.submit_order(command).unwrap();
    assert_eq!(state.count.load(Ordering::SeqCst), 1);
    restarted.disconnect().await.unwrap();
    server.abort();
}
