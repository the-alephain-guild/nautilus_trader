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

use alloy_primitives::address;
use axum::{
    Json, Router,
    extract::State,
    http::{StatusCode, Uri},
    routing::get,
};
use nautilus_papertrade::api::PapertradeApi;
use parking_lot::Mutex;
use rstest::rstest;
use serde_json::{Value, json};
use std::sync::Arc;
struct Source {
    queries: Mutex<Vec<String>>,
    page: Value,
    status: StatusCode,
}
async fn page(State(state): State<Arc<Source>>, uri: Uri) -> (StatusCode, Json<Value>) {
    state
        .queries
        .lock()
        .push(uri.query().unwrap_or_default().to_owned());
    (state.status, Json(state.page.clone()))
}
async fn api(
    page_body: Value,
    status: StatusCode,
    budget: usize,
) -> (PapertradeApi, Arc<Source>, tokio::task::JoinHandle<()>) {
    let state = Arc::new(Source {
        queries: Mutex::new(Vec::new()),
        page: page_body,
        status,
    });
    let app = Router::new().fallback(get(page)).with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (
        PapertradeApi::new(url.parse().unwrap(), url.parse().unwrap(), budget).unwrap(),
        state,
        server,
    )
}
#[tokio::test]
async fn observed_history_is_complete_and_uses_canonical_parameter_order() {
    let history: Value =
        serde_json::from_str(include_str!("../test_data/live_history.json")).unwrap();
    let expected = history["items"].as_array().unwrap().len();
    let (api, state, server) = api(history, StatusCode::OK, 2).await;
    let history = api
        .history(address!("a23dadf056158c5ff1081654dbf8e167530fbf71"))
        .await
        .unwrap();
    assert_eq!(history.len(), expected);
    assert_eq!(state.queries.lock()[0], "filter=all&limit=75");
    server.abort();
}
#[rstest]
#[case(StatusCode::INTERNAL_SERVER_ERROR,json!({"items":[],"nextCursor":null}),2)]
#[case(StatusCode::OK,json!({"items":[],"nextCursor":"loop"}),3)]
#[case(StatusCode::OK,json!({"items":[],"nextCursor":"next"}),1)]
#[case(StatusCode::OK,json!({"nextCursor":null}),2)]
#[tokio::test]
async fn failed_truncated_or_cyclic_history_never_becomes_authoritative_empty(
    #[case] status: StatusCode,
    #[case] body: Value,
    #[case] budget: usize,
) {
    let (api, _state, server) = api(body, status, budget).await;
    assert!(
        api.history(address!("2222222222222222222222222222222222222222"))
            .await
            .is_err()
    );
    server.abort();
}
