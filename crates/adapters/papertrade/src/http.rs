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

//! Relayer acknowledgements with explicit uncertain outcomes and no automatic resubmission.

use std::{collections::HashMap, num::NonZeroU32};

use alloy_primitives::B256;
use anyhow::{Result, ensure};
use nautilus_core::string::secret::SecretString;
use nautilus_network::{
    http::{HttpClient, HttpRedirectPolicy, Method},
    ratelimiter::quota::Quota,
};
use serde::Deserialize;
use serde_json::Value;
use url::Url;

use crate::signing::SignedIntent;

/// Relayer lifecycle labels; none alone establishes a Nautilus fill or spendable PnL.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum IntentState {
    Queued,
    Prepared,
    Submitted,
    Confirmed,
    Failed,
    Reverted,
    Expired,
    Cancelled,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IntentReceipt {
    pub intent_id: B256,
    pub action: String,
    pub state: IntentState,
    pub queue_revision: Option<String>,
}

/// Evidence from one submission attempt. Unknown must be reconciled using the same ID.
#[derive(Debug, Clone)]
pub enum SubmissionOutcome {
    Acknowledged(IntentReceipt),
    /// Includes transport failures, non-success HTTP status, and unbound/malformed replies.
    /// An error response does not prove a prior or concurrent attempt was rejected.
    Unknown {
        intent_id: B256,
        http_status: Option<u16>,
    },
}

/// Low-level HTTP transport for requests prepared by [crate::PapertradeSigner].
///
/// No funds move until the caller explicitly invokes submit. The frontend build ID is an
/// explicit input because the official origin can reject clients using a stale wire contract.
/// This client performs no application retry and never creates a replacement intent.
#[derive(Debug, Clone)]
pub struct PapertradeRelayerClient {
    origin: Url,
    build_id: String,
    http: HttpClient,
}

impl PapertradeRelayerClient {
    pub fn new(build_id: String) -> Result<Self> {
        Self::with_origin(Url::parse("https://exchange.papertrade.xyz/")?, build_id)
    }

    /// Selects an explicit origin, including loopback HTTP for an integration harness.
    /// All transports created here reject redirects to keep signatures on this origin.
    pub fn with_origin(origin: Url, build_id: String) -> Result<Self> {
        let quota = NonZeroU32::new(1).ok_or_else(|| anyhow::anyhow!("Invalid request quota"))?;
        let http = HttpClient::builder()
            .timeout_secs(35)
            .maybe_default_quota(Quota::per_second(quota))
            .redirect_policy(HttpRedirectPolicy::Reject)
            .build()?;
        ensure!(
            origin.scheme() == "https"
                || (origin.scheme() == "http"
                    && matches!(origin.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"))),
            "Relayer origin must use HTTPS or loopback HTTP"
        );
        ensure!(
            origin.username().is_empty()
                && origin.password().is_none()
                && origin.path() == "/"
                && origin.query().is_none()
                && origin.fragment().is_none(),
            "Relayer origin must not contain credentials, a path, query, or fragment"
        );
        ensure!(
            !build_id.is_empty()
                && build_id.len() <= 128
                && build_id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-'),
            "Invalid frontend build identifier"
        );
        Ok(Self {
            origin,
            build_id,
            http,
        })
    }

    /// Returns a relayer acknowledgement, not an execution or fill report.
    ///
    /// The caller supplies current Unix seconds; stale requests fail before transmission.
    /// Signature bodies use the shared transport's secret-body path and redirects are
    /// treated as uncertain replies.
    pub async fn submit(&self, intent: &SignedIntent, now: u64) -> Result<SubmissionOutcome> {
        ensure!(intent.deadline() > now, "Intent expired before submission");
        let body = SecretString::from(serde_json::to_string(intent.body())?);
        let headers = HashMap::from([
            ("Accept".to_owned(), "application/json".to_owned()),
            ("Content-Type".to_owned(), "application/json".to_owned()),
            ("X-Papertrade-Build".to_owned(), self.build_id.clone()),
        ]);
        let response = self
            .http
            .request_with_secret_body(
                Method::POST,
                self.origin.join(&intent.path())?.to_string(),
                None,
                Some(headers),
                body,
                Some(35),
                None,
            )
            .await;
        let Ok(response) = response else {
            return Ok(SubmissionOutcome::Unknown {
                intent_id: intent.id(),
                http_status: None,
            });
        };
        let status = response.status.as_u16();
        let receipt = (200..300)
            .contains(&status)
            .then(|| parse_receipt(&response.body, intent))
            .transpose();
        Ok(match receipt {
            Ok(Some(receipt)) => SubmissionOutcome::Acknowledged(receipt),
            _ => SubmissionOutcome::Unknown {
                intent_id: intent.id(),
                http_status: Some(status),
            },
        })
    }
}

fn parse_receipt(body: &[u8], intent: &SignedIntent) -> Result<IntentReceipt> {
    let value: Value = serde_json::from_slice(body)?;
    ensure!(
        value.get("error").is_none() && value.get("ok") != Some(&Value::Bool(false)),
        "Relayer returned an error"
    );
    let receipt: IntentReceipt = serde_json::from_value(value)?;
    ensure!(
        receipt.intent_id == intent.id() && receipt.action == intent.action(),
        "Relayer receipt does not match submitted intent"
    );
    if let Some(revision) = &receipt.queue_revision {
        ensure!(
            revision == "0"
                || (!revision.starts_with('0')
                    && !revision.is_empty()
                    && revision.bytes().all(|b| b.is_ascii_digit())),
            "Invalid queue revision"
        );
    }
    Ok(receipt)
}
