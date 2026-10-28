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

//! Read-only public API and account stream, using the shared HTTP transport.

use std::{
    collections::HashSet,
    sync::Arc,
    time::{Duration, Instant},
};

use alloy_primitives::{Address, B256};
use anyhow::{Result, ensure};
use nautilus_network::http::{HttpClient, HttpRedirectPolicy, HttpResponseStream, Method};
use serde::Deserialize;
use serde_json::{Value, json};
use url::Url;

use crate::{
    stream::{SseDecoder, WalletState},
    wire::{AccountSnapshot, CloseRecord, TradingState},
};

#[derive(Debug, Clone)]
pub struct PapertradeApi {
    http: HttpClient,
    origin: Url,
    rpc: Url,
    reference: Url,
    max_pages: usize,
}

const WALLET_IDLE_TIMEOUT: Duration = Duration::from_secs(35);

#[derive(Debug, Clone)]
pub(crate) struct WalletActivity(Arc<parking_lot::Mutex<Instant>>);

impl WalletActivity {
    fn new() -> Self {
        Self(Arc::new(parking_lot::Mutex::new(Instant::now())))
    }
    fn received(&self) {
        *self.0.lock() = Instant::now();
    }
    pub(crate) fn is_fresh(&self) -> bool {
        self.0.lock().elapsed() < WALLET_IDLE_TIMEOUT
    }
}

#[derive(Debug)]
pub struct WalletStream {
    response: HttpResponseStream,
    decoder: SseDecoder,
    state: WalletState,
    pub(crate) activity: WalletActivity,
}

impl PapertradeApi {
    pub fn new(origin: Url, rpc: Url, max_pages: usize) -> Result<Self> {
        ensure!(max_pages > 0, "History page budget must be positive");
        let http = HttpClient::builder()
            .redirect_policy(HttpRedirectPolicy::Reject)
            .build()?;
        Ok(Self {
            http,
            origin,
            rpc,
            reference: Url::parse("https://api.hyperliquid.xyz/info")?,
            max_pages,
        })
    }
    #[must_use]
    pub fn with_reference(mut self, reference: Url) -> Self {
        self.reference = reference;
        self
    }
    async fn get(&self, path: &str) -> Result<Value> {
        let response = self
            .http
            .request(
                Method::GET,
                self.origin.join(path)?.to_string(),
                None,
                None,
                None,
                Some(20),
                None,
            )
            .await?;
        ensure!(
            response.status.is_success(),
            "Public API HTTP {}",
            response.status.as_u16()
        );
        Ok(serde_json::from_slice(&response.body)?)
    }
    pub async fn trading(&self) -> Result<TradingState> {
        let state: TradingState = serde_json::from_value(self.get("/state/trading").await?)?;
        state.validate()?;
        Ok(state)
    }
    pub async fn wallet_stream(&self, wallet: Address) -> Result<WalletStream> {
        let url = self
            .origin
            .join(&format!("/state/user/live?wallet={wallet:#x}"))?;
        let response = tokio::time::timeout(
            Duration::from_secs(20),
            self.http.get_stream(url.to_string()),
        )
        .await??;
        ensure!(
            response.status().is_success(),
            "Wallet stream HTTP {}",
            response.status()
        );
        Ok(WalletStream {
            response,
            decoder: SseDecoder::default(),
            state: WalletState::new(wallet),
            activity: WalletActivity::new(),
        })
    }
    pub async fn snapshot(&self, wallet: Address) -> Result<AccountSnapshot> {
        self.wallet_stream(wallet).await?.next().await
    }
    pub async fn history(&self, wallet: Address) -> Result<Vec<CloseRecord>> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Page {
            items: Vec<CloseRecord>,
            next_cursor: Option<String>,
        }
        let mut cursor: Option<String> = None;
        let mut cursors = HashSet::new();
        let mut records = std::collections::BTreeMap::new();
        for _ in 0..self.max_pages {
            let mut url = self
                .origin
                .join(&format!("/query/accounts/{wallet:#x}/trades"))?;
            let mut pairs = vec![("filter", "all".to_owned()), ("limit", "75".to_owned())];
            if let Some(c) = &cursor {
                pairs.push(("cursor", c.clone()));
            }
            pairs.sort_by_key(|p| p.0);
            url.query_pairs_mut()
                .extend_pairs(pairs.iter().map(|(k, v)| (*k, v.as_str())));
            let page: Page = serde_json::from_value(
                self.get(&format!(
                    "{}?{}",
                    url.path(),
                    url.query().unwrap_or_default()
                ))
                .await?,
            )?;
            for record in page.items {
                record.validate()?;
                if let Some(previous) = records.insert(record.position_id, record.clone()) {
                    ensure!(
                        previous == record,
                        "Conflicting close history for a position"
                    );
                }
            }
            match page.next_cursor {
                None => return Ok(records.into_values().collect()),
                Some(next) => {
                    ensure!(
                        !next.is_empty() && cursors.insert(next.clone()),
                        "History cursor repeated"
                    );
                    cursor = Some(next);
                }
            }
        }
        anyhow::bail!("History page budget exhausted; coverage is incomplete")
    }
    pub(crate) async fn block_timestamp(&self, number: u64) -> Result<u64> {
        let height = format!("0x{number:x}");
        let response = self.http.request(Method::POST,self.rpc.to_string(),None,
            Some(std::collections::HashMap::from([("Content-Type".into(),"application/json".into())])),
            Some(serde_json::to_vec(&json!({"jsonrpc":"2.0","id":1,"method":"eth_getBlockByNumber","params":[height,false]}))?),
            Some(20),None).await?;
        ensure!(response.status.is_success(), "Block RPC HTTP failure");
        let envelope: Value = serde_json::from_slice(&response.body)?;
        ensure!(
            envelope.get("error").is_none() && envelope["result"]["number"] == height,
            "Missing or mismatched coverage block"
        );
        let block = &envelope["result"];
        let hash: B256 = serde_json::from_value(block["hash"].clone())?;
        ensure!(!hash.is_zero(), "Unmined coverage block");
        let timestamp = block["timestamp"]
            .as_str()
            .and_then(|s| s.strip_prefix("0x"))
            .ok_or_else(|| anyhow::anyhow!("Missing coverage block timestamp"))?;
        Ok(u64::from_str_radix(timestamp, 16)?)
    }
    pub async fn receipt(&self, tx_hash: B256) -> Result<Value> {
        let body = serde_json::to_vec(
            &json!({"jsonrpc":"2.0","id":1,"method":"eth_getTransactionReceipt","params":[tx_hash]}),
        )?;
        let response = self
            .http
            .request(
                Method::POST,
                self.rpc.to_string(),
                None,
                Some(std::collections::HashMap::from([(
                    "Content-Type".into(),
                    "application/json".into(),
                )])),
                Some(body),
                Some(20),
                None,
            )
            .await?;
        ensure!(
            response.status.is_success(),
            "Receipt RPC HTTP {}",
            response.status.as_u16()
        );
        let envelope: Value = serde_json::from_slice(&response.body)?;
        ensure!(
            envelope.get("error").is_none() && envelope["result"].is_object(),
            "Receipt unavailable"
        );
        let receipt = envelope["result"].clone();
        ensure!(
            receipt["transactionHash"] == json!(tx_hash) && receipt["status"] == "0x1",
            "Receipt identity or status mismatch"
        );
        Ok(receipt)
    }
    pub async fn intent_receipts(&self, ids: &[B256], from: u64, to: u64) -> Result<Vec<Value>> {
        if ids.is_empty() || from > to {
            return Ok(Vec::new());
        }
        ensure!(
            (to - from) / 1000 < self.max_pages as u64,
            "Intent log coverage exceeds configured budget"
        );
        let mut hashes = HashSet::new();
        for start in (from..=to).step_by(1000) {
            let end = start.saturating_add(999).min(to);
            let body = serde_json::to_vec(
                &json!({"jsonrpc":"2.0","id":1,"method":"eth_getLogs","params":[{
                "address":crate::chain::BATCH_EXECUTOR,"fromBlock":format!("0x{start:x}"),
                "toBlock":format!("0x{end:x}"),"topics":[crate::chain::RESULT_TOPIC,null,null,ids]}]}),
            )?;
            let response = self
                .http
                .request(
                    Method::POST,
                    self.rpc.to_string(),
                    None,
                    Some(std::collections::HashMap::from([(
                        "Content-Type".into(),
                        "application/json".into(),
                    )])),
                    Some(body),
                    Some(20),
                    None,
                )
                .await?;
            ensure!(response.status.is_success(), "Intent RPC HTTP failure");
            let v: Value = serde_json::from_slice(&response.body)?;
            ensure!(v.get("error").is_none(), "Intent log RPC failed");
            for log in v["result"]
                .as_array()
                .ok_or_else(|| anyhow::anyhow!("Missing intent logs"))?
            {
                ensure!(log["removed"] == false, "Removed intent result");
                hashes.insert(serde_json::from_value::<B256>(
                    log["transactionHash"].clone(),
                )?);
            }
        }
        let mut receipts = Vec::new();
        for hash in hashes {
            receipts.push(self.receipt(hash).await?);
        }
        Ok(receipts)
    }
    pub(crate) async fn reference_request(&self, payload: &Value) -> Result<Value> {
        let body = serde_json::to_vec(payload)?;
        let response = self
            .http
            .request(
                Method::POST,
                self.reference.to_string(),
                None,
                Some(std::collections::HashMap::from([(
                    "Content-Type".into(),
                    "application/json".into(),
                )])),
                Some(body),
                Some(10),
                None,
            )
            .await?;
        ensure!(
            response.status.is_success(),
            "Reference API HTTP {}",
            response.status.as_u16()
        );
        ensure!(
            response.body.len() <= 2 * 1024 * 1024,
            "Reference response exceeds size limit"
        );
        Ok(serde_json::from_slice(&response.body)?)
    }
    pub async fn reference_mid(&self, symbol: &str) -> Result<(rust_decimal::Decimal, u64)> {
        let v = self
            .reference_request(&json!({"type":"l2Book","coin":symbol}))
            .await?;
        ensure!(v["coin"] == symbol, "Reference symbol mismatch");
        let bid: rust_decimal::Decimal = v["levels"][0][0]["px"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing reference bid"))?
            .parse()?;
        let ask: rust_decimal::Decimal = v["levels"][1][0]["px"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing reference ask"))?
            .parse()?;
        ensure!(
            bid > rust_decimal::Decimal::ZERO && ask >= bid,
            "Invalid reference BBO"
        );
        Ok((
            (bid + ask) / rust_decimal::Decimal::from(2),
            v["time"]
                .as_u64()
                .ok_or_else(|| anyhow::anyhow!("Missing reference time"))?,
        ))
    }
}

impl WalletStream {
    /// A disconnect discards this stream; the next stream always requires a full snapshot.
    pub async fn next(&mut self) -> Result<AccountSnapshot> {
        loop {
            let chunk = tokio::time::timeout(WALLET_IDLE_TIMEOUT, self.response.chunk())
                .await??
                .ok_or_else(|| anyhow::anyhow!("Wallet stream closed"))?;
            self.activity.received();
            let mut changed = false;
            for event in self.decoder.push(&chunk)? {
                if event.event == "wallet" {
                    let frame: Value = serde_json::from_str(&event.data)?;
                    changed = self.state.apply(&frame)? || changed;
                } else if event.event == "heartbeat" {
                    ensure!(event.data == "null", "Invalid heartbeat");
                } else if event.event == "liveness" {
                    let v: Value = serde_json::from_str(&event.data)?;
                    ensure!(
                        v["heartbeatIntervalMs"]
                            .as_u64()
                            .is_some_and(|x| x > 0 && x <= 60_000),
                        "Invalid liveness"
                    );
                }
            }
            if changed {
                return self
                    .state
                    .current()
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("No current snapshot"));
            }
        }
    }
}
