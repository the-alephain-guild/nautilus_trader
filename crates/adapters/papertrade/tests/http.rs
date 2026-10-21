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

//! Loopback HTTP tests; these never contact Papertrade or submit real intents.

use std::sync::{Arc, Mutex};

use alloy::signers::local::PrivateKeySigner;
use alloy_primitives::{B256, U256, address};
use axum::{
    Router,
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode, Uri},
    routing::post,
};
use nautilus_papertrade::{
    CancelIntent, IntentState, OpenPosition, PapertradeRelayerClient, PapertradeSigner,
    SubmissionOutcome,
};
use rstest::rstest;
use serde_json::{Value, json};
use tokio::{net::TcpListener, task::JoinHandle};
use url::Url;

const NOW: u64 = 1_700_000_000;

#[derive(Debug)]
struct Captured {
    path: String,
    headers: HeaderMap,
    body: Value,
}

struct Mock {
    status: StatusCode,
    response: String,
    requests: Mutex<Vec<Captured>>,
}

async fn handle(
    State(state): State<Arc<Mock>>,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> (StatusCode, String) {
    state.requests.lock().unwrap().push(Captured {
        path: uri.path().to_owned(),
        headers,
        body: serde_json::from_slice(&body).unwrap(),
    });
    (state.status, state.response.clone())
}

async fn server(status: StatusCode, response: String) -> (Url, Arc<Mock>, JoinHandle<()>) {
    let state = Arc::new(Mock {
        status,
        response,
        requests: Mutex::new(Vec::new()),
    });
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    let app = Router::new()
        .fallback(post(handle))
        .with_state(state.clone());
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (url, state, task)
}

fn intent() -> nautilus_papertrade::SignedIntent {
    PapertradeSigner::new(PrivateKeySigner::from_bytes(&B256::repeat_byte(17)).unwrap())
        .sign_open(
            &OpenPosition {
                user: address!("2222222222222222222222222222222222222222"),
                instrumentId: 0,
                isLong: true,
                size: 1_000_000_000_000_000_000,
                leverage: 10,
                useDebt: false,
                nonce: U256::from(1),
                deadline: NOW + 3_600,
            },
            NOW,
        )
        .unwrap()
}

#[rstest]
#[case("queued", IntentState::Queued)]
#[case("prepared", IntentState::Prepared)]
#[case("submitted", IntentState::Submitted)]
#[case("confirmed", IntentState::Confirmed)]
#[case("failed", IntentState::Failed)]
#[case("reverted", IntentState::Reverted)]
#[case("expired", IntentState::Expired)]
#[case("cancelled", IntentState::Cancelled)]
#[tokio::test]
async fn binds_acknowledgement_and_preserves_lifecycle(
    #[case] label: &str,
    #[case] expected: IntentState,
) {
    let intent = intent();
    let response = json!({
        "intentId": intent.id(),
        "action": intent.action(),
        "state": label,
        "queueRevision": "42",
    })
    .to_string();
    let (url, mock, task) = server(StatusCode::OK, response).await;
    let client = PapertradeRelayerClient::with_origin(url, "test-build".to_owned()).unwrap();
    let outcome = client.submit(&intent, NOW).await.unwrap();
    let SubmissionOutcome::Acknowledged(receipt) = outcome else {
        panic!("{outcome:?}")
    };
    assert_eq!(receipt.intent_id, intent.id());
    assert_eq!(receipt.state, expected);
    assert_eq!(receipt.queue_revision.as_deref(), Some("42"));
    let requests = mock.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].path, "/intents");
    assert_eq!(requests[0].headers["X-Papertrade-Build"], "test-build");
    assert_eq!(requests[0].body, *intent.body());
    task.abort();
}

#[rstest]
#[case(StatusCode::TOO_MANY_REQUESTS, "rate limited")]
#[case(StatusCode::INTERNAL_SERVER_ERROR, "unavailable")]
#[case(StatusCode::CONFLICT, r#"{"error":"intent_conflict"}"#)]
#[case(StatusCode::OK, r#"{"ok":false}"#)]
#[case(StatusCode::OK, r#"{"error":"rejected"}"#)]
#[case(StatusCode::OK, r#"{"action":"openPosition","state":"queued"}"#)]
#[case(StatusCode::OK, "invalid JSON")]
#[tokio::test]
async fn uncertain_responses_preserve_identity_without_retry(
    #[case] status: StatusCode,
    #[case] body: &str,
) {
    let intent = intent();
    let (url, mock, task) = server(status, body.to_owned()).await;
    let client = PapertradeRelayerClient::with_origin(url, "test-build".to_owned()).unwrap();
    let outcome = client.submit(&intent, NOW).await.unwrap();
    let SubmissionOutcome::Unknown {
        intent_id,
        http_status,
    } = outcome
    else {
        panic!("{outcome:?}")
    };
    assert_eq!(intent_id, intent.id());
    assert_eq!(http_status, Some(status.as_u16()));
    assert_eq!(mock.requests.lock().unwrap().len(), 1);
    task.abort();
}

#[rstest]
#[case("id")]
#[case("action")]
#[case("state")]
#[case("revision")]
#[tokio::test]
async fn malformed_or_unbound_success_reply_is_unknown(#[case] changed: &str) {
    let intent = intent();
    let mut response = json!({
        "intentId": intent.id(), "action": intent.action(),
        "state": "queued", "queueRevision": "0",
    });
    match changed {
        "id" => response["intentId"] = json!(B256::ZERO),
        "action" => response["action"] = json!("closePositions"),
        "state" => response["state"] = json!("newUnknownState"),
        "revision" => response["queueRevision"] = json!("01"),
        _ => unreachable!(),
    }
    let (url, mock, task) = server(StatusCode::OK, response.to_string()).await;
    let client = PapertradeRelayerClient::with_origin(url, "test-build".to_owned()).unwrap();
    assert!(matches!(
        client.submit(&intent, NOW).await.unwrap(),
        SubmissionOutcome::Unknown { .. }
    ));
    assert_eq!(mock.requests.lock().unwrap().len(), 1);
    task.abort();
}

#[tokio::test]
async fn expired_intent_never_reaches_transport() {
    let intent = intent();
    let (url, mock, task) = server(StatusCode::OK, "{}".to_owned()).await;
    let client = PapertradeRelayerClient::with_origin(url, "test-build".to_owned()).unwrap();
    assert!(client.submit(&intent, intent.deadline()).await.is_err());
    assert!(mock.requests.lock().unwrap().is_empty());
    task.abort();
}

#[tokio::test]
async fn transport_failure_is_unknown() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    drop(listener);
    let intent = intent();
    let client = PapertradeRelayerClient::with_origin(url, "test-build".to_owned()).unwrap();
    assert!(matches!(
        client.submit(&intent, NOW).await.unwrap(),
        SubmissionOutcome::Unknown { intent_id, http_status: None } if intent_id == intent.id()
    ));
}

#[tokio::test]
async fn cancellation_posts_the_target_id_with_flat_signature_fields() {
    let target = intent();
    let signer =
        PapertradeSigner::new(PrivateKeySigner::from_bytes(&B256::repeat_byte(17)).unwrap());
    let cancel = signer
        .sign_cancel(
            &CancelIntent {
                user: address!("2222222222222222222222222222222222222222"),
                intentId: target.id(),
                deadline: U256::from(NOW + 100),
            },
            &target,
            NOW,
        )
        .unwrap();
    let response = json!({
        "intentId": target.id(), "action": target.action(), "state": "cancelled",
    })
    .to_string();
    let (url, mock, task) = server(StatusCode::OK, response).await;
    let client = PapertradeRelayerClient::with_origin(url, "test-build".to_owned()).unwrap();
    let outcome = client.submit(&cancel, NOW).await.unwrap();
    assert!(matches!(outcome, SubmissionOutcome::Acknowledged(receipt)
        if receipt.state == IntentState::Cancelled));
    let requests = mock.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].path,
        format!("/intents/{:#x}/cancel", target.id())
    );
    assert_eq!(requests[0].body, *cancel.body());
    assert!(requests[0].body.get("signed").is_none());
    task.abort();
}

#[tokio::test]
async fn redirects_never_forward_signature_to_another_origin() {
    let (target_url, mock, target_task) = server(StatusCode::OK, "{}".to_owned()).await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let source_url = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    let app = Router::new().fallback(post(move || {
        let location = target_url.to_string();
        async move { (StatusCode::TEMPORARY_REDIRECT, [("Location", location)], "") }
    }));
    let source_task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = PapertradeRelayerClient::with_origin(source_url, "test-build".to_owned()).unwrap();
    assert!(matches!(
        client.submit(&intent(), NOW).await.unwrap(),
        SubmissionOutcome::Unknown { .. }
    ));
    assert!(mock.requests.lock().unwrap().is_empty());
    source_task.abort();
    target_task.abort();
}

#[rstest]
#[case("http://example.com/")]
#[case("https://user:pass@example.com/")]
#[case("https://example.com/path/")]
#[case("https://example.com/?token=value")]
#[case("https://example.com/#fragment")]
fn refuses_unsafe_origins(#[case] origin: &str) {
    assert!(
        PapertradeRelayerClient::with_origin(Url::parse(origin).unwrap(), "test-build".to_owned())
            .is_err()
    );
}
