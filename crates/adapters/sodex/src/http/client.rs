//! REST client.
//!
//! # Signing and the request body differ on purpose
//!
//! The signature commits to `keccak256` over `{"type": …, "params": …}`, but the HTTP body
//! carries **only the `params` object** — the venue documents this explicitly:
//!
//! > the HTTP request body contains only the params object (without the type wrapper),
//! > using the same field order and types as the signing payload.
//!
//! Sending the wrapped form, or hashing the unwrapped one, both fail verification with the
//! same opaque message. [`SignedRequest`] is produced without touching the network so this
//! step is testable on its own rather than only observable as a rejection.

use std::{collections::HashMap, num::NonZeroU32, sync::Arc, time::Duration};

use nautilus_core::{consts::NAUTILUS_USER_AGENT, time::get_atomic_clock_realtime};
use nautilus_network::{
    http::{HttpClient, HttpClientError, HttpResponse, Method},
    ratelimiter::quota::Quota,
    retry::{RetryConfig, RetryError, RetryManager},
};
use serde::Serialize;
use tokio_util::sync::CancellationToken;

use super::{
    Network,
    models::{ApiResponse, EnvelopeError},
    ratelimit::{
        DEFAULT_ENDPOINT_WEIGHT, OrderRateLimiter, RateLimited, WeightBudget, await_order_quota,
        order_rate_limiter,
    },
    requests::RequestError,
};
use crate::{
    common::{
        Market,
        credential::{ApiKeyName, ApiPrivateKey, CredentialError},
    },
    signing::{
        NonceGenerator,
        signers::{ExchangeSigner, SigningError, payload_hash},
    },
};

/// Header carrying the API key's *name*.
pub const HEADER_API_KEY: &str = "X-API-Key";
/// Header carrying the typed signature.
pub const HEADER_API_SIGN: &str = "X-API-Sign";
/// Header carrying the nonce.
pub const HEADER_API_NONCE: &str = "X-API-Nonce";

/// Default HTTP timeout when a caller does not specify one.
pub const DEFAULT_TIMEOUT_SECS: u64 = 30;

/// Ceiling on requests per second, as a backstop against a runaway loop.
///
/// Not the venue's limit — the venue meters weight and order counts, neither of which is a
/// request rate. This only stops this client from flooding a single endpoint faster than any
/// legitimate use would; the real budgets are enforced by
/// [`WeightBudget`] and [`await_order_quota`].
const REQUESTS_PER_SECOND_BACKSTOP: u32 = 40;

/// Failures from the REST layer.
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("credentials required for signed requests")]
    CredentialsRequired,
    #[error(transparent)]
    Credential(#[from] CredentialError),
    #[error(transparent)]
    Signing(#[from] SigningError),
    #[error(transparent)]
    Request(#[from] RequestError),
    #[error(transparent)]
    RateLimited(#[from] RateLimited),
    #[error("serialization failed: {0}")]
    Serialize(#[from] serde_json::Error),
    #[error("transport failed: {0}")]
    Transport(String),
    #[error("venue returned HTTP {status}: {body}")]
    Status { status: u16, body: String },
}

impl From<HttpClientError> for ClientError {
    fn from(error: HttpClientError) -> Self {
        Self::Transport(error.to_string())
    }
}

/// A fully prepared signed request, before it touches the network.
///
/// Separated from sending so the signing and encoding rules can be asserted directly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedRequest {
    pub method: Method,
    pub url: String,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

impl SignedRequest {
    /// The body as text, for assertions and diagnostics.
    #[must_use]
    pub fn body_str(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

/// Wall-clock milliseconds, for the rolling weight window.
fn now_millis() -> u64 {
    get_atomic_clock_realtime().get_time_ms()
}

/// Credentials for signed endpoints.
#[derive(Debug)]
struct Credentials {
    key_name: ApiKeyName,
    signer: ExchangeSigner,
    nonces: NonceGenerator,
}

/// REST client for one network and market.
///
/// Bound to a single market because the EIP-712 domain differs between spot and perps; a
/// client that could switch would be able to sign a perps action under the spot domain.
#[derive(Debug)]
pub struct SodexHttpClient {
    http: HttpClient,
    network: Network,
    market: Market,
    credentials: Option<Credentials>,
    weights: std::sync::Mutex<WeightBudget>,
    /// Order-count pacing, shared so one account's clients draw on one allowance.
    orders: Arc<OrderRateLimiter>,
    retry: RetryManager<ClientError>,
    cancellation: CancellationToken,
}

impl SodexHttpClient {
    /// A client for unsigned market-data endpoints.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError::Transport`] if the underlying HTTP client cannot be built.
    pub fn new_public(network: Network, market: Market) -> Result<Self, ClientError> {
        Self::public_with_options(network, market, DEFAULT_TIMEOUT_SECS, None)
    }

    /// A market-data client with an explicit timeout and proxy.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError::Transport`] if the underlying HTTP client cannot be built.
    pub fn public_with_options(
        network: Network,
        market: Market,
        timeout_secs: u64,
        proxy_url: Option<String>,
    ) -> Result<Self, ClientError> {
        Ok(Self {
            http: Self::build_http(timeout_secs, proxy_url)?,
            network,
            market,
            credentials: None,
            weights: std::sync::Mutex::new(WeightBudget::new()),
            orders: order_rate_limiter(),
            retry: Self::build_retry(),
            cancellation: CancellationToken::new(),
        })
    }

    /// A client that can sign trading actions.
    ///
    /// Takes an [`ApiPrivateKey`] rather than the master key: trading actions must be signed
    /// by a registered API key, and the master wallet is expected to stay offline.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] if the key cannot be parsed into a signer or the HTTP client
    /// cannot be built.
    pub fn with_credentials(
        network: Network,
        market: Market,
        key_name: ApiKeyName,
        key: &ApiPrivateKey,
    ) -> Result<Self, ClientError> {
        Self::signed_with_options(network, market, key_name, key, DEFAULT_TIMEOUT_SECS, None)
    }

    /// A signing client with an explicit timeout and proxy.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] if the key cannot be parsed into a signer or the HTTP client
    /// cannot be built.
    pub fn signed_with_options(
        network: Network,
        market: Market,
        key_name: ApiKeyName,
        key: &ApiPrivateKey,
        timeout_secs: u64,
        proxy_url: Option<String>,
    ) -> Result<Self, ClientError> {
        let signer = ExchangeSigner::new(key, market, network.chain_id())?;
        Ok(Self {
            http: Self::build_http(timeout_secs, proxy_url)?,
            network,
            market,
            credentials: Some(Credentials {
                key_name,
                signer,
                nonces: NonceGenerator::new(),
            }),
            weights: std::sync::Mutex::new(WeightBudget::new()),
            orders: order_rate_limiter(),
            retry: Self::build_retry(),
            cancellation: CancellationToken::new(),
        })
    }

    /// Cancels every in-flight retry loop, so a shutdown does not wait out a backoff.
    pub fn shutdown(&self) {
        self.cancellation.cancel();
    }

    /// The address the venue will recover from this client's signatures.
    ///
    /// Not cosmetic: it is what proves a configured wallet address is the account this client
    /// actually signs for, which the account reads cannot establish on their own — a wrong
    /// address answers with an empty account rather than an error.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError::CredentialsRequired`] on an unsigned client.
    pub fn signing_address(&self) -> Result<alloy_primitives::Address, ClientError> {
        self.credentials
            .as_ref()
            .map(|credentials| credentials.signer.address())
            .ok_or(ClientError::CredentialsRequired)
    }

    /// Whether this client can sign.
    #[must_use]
    pub const fn can_sign(&self) -> bool {
        self.credentials.is_some()
    }

    /// Absolute URL for a market-scoped path such as `/trade/orders`.
    #[must_use]
    pub fn url_for(&self, path: &str) -> String {
        format!("{}{path}", self.network.market_base(self.market))
    }

    /// Prepares a signed request without sending it.
    ///
    /// `action_type` is the venue's action name (`newOrder`, `cancelOrder`, …) that goes into
    /// the signing payload but **not** into the body.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError::CredentialsRequired`] without credentials, or a signing or
    /// serialization failure.
    pub fn build_signed<P: Serialize>(
        &self,
        method: Method,
        path: &str,
        action_type: &str,
        params: &P,
    ) -> Result<SignedRequest, ClientError> {
        let credentials = self
            .credentials
            .as_ref()
            .ok_or(ClientError::CredentialsRequired)?;

        let digest = payload_hash(action_type, params)?;
        let nonce = credentials.nonces.next();
        let signature = credentials.signer.sign_action(digest, nonce)?;

        let mut headers = HashMap::new();
        headers.insert("Content-Type".to_string(), "application/json".to_string());
        headers.insert("Accept".to_string(), "application/json".to_string());
        headers.insert(
            HEADER_API_KEY.to_string(),
            credentials.key_name.as_str().to_string(),
        );
        headers.insert(
            HEADER_API_SIGN.to_string(),
            alloy_primitives::hex::encode_prefixed(&signature),
        );
        headers.insert(HEADER_API_NONCE.to_string(), nonce.to_string());

        // Only `params` goes on the wire; the `{type, params}` envelope exists solely to be
        // hashed. Serializing the envelope here would break verification.
        let body = serde_json::to_vec(params)?;

        Ok(SignedRequest {
            method,
            url: self.url_for(path),
            headers,
            body,
        })
    }

    /// Reserves request weight against the rolling per-IP budget.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError::RateLimited`] carrying how long to wait.
    pub fn reserve_weight(&self, weight: u32, now_ms: u64) -> Result<(), ClientError> {
        self.weights
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .try_consume(weight, now_ms)
            .map_err(ClientError::from)
    }

    /// Books weight the venue charges after a response, such as history row counts.
    pub fn record_weight(&self, weight: u32, now_ms: u64) {
        self.weights
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .record(weight, now_ms);
    }

    /// Sends a prepared request and decodes the envelope.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError::Status`] for a non-success HTTP status, or a transport,
    /// decoding or venue-level error.
    pub async fn send<T: serde::de::DeserializeOwned>(
        &self,
        request: SignedRequest,
    ) -> Result<T, ClientError> {
        self.send_weighted(request, DEFAULT_ENDPOINT_WEIGHT, 0).await
    }

    /// Sends a prepared write, declaring its cost on both metered axes.
    ///
    /// `orders` is the number of orders the request places, which is **not** the same as its
    /// request weight: a batch of `N` orders costs one request's weight but `N` against the
    /// order-count allowance. Passing it lets the client pace instead of being rejected; pass
    /// `0` for a write that places none, such as a cancel.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError::Status`] for a non-success HTTP status, or a transport, decoding
    /// or venue-level error. Transient failures are not retried here — see
    /// [`Self::write_is_retryable`] for why a write whose outcome is unknown must not be
    /// repeated.
    pub async fn send_weighted<T: serde::de::DeserializeOwned>(
        &self,
        request: SignedRequest,
        weight: u32,
        orders: u32,
    ) -> Result<T, ClientError> {
        await_order_quota(&self.orders, orders).await;
        let response = self
            .dispatch(&request, weight, Self::write_is_retryable)
            .await?;

        Self::decode(response)
    }

    /// Sends a prepared request against an endpoint that may return no payload.
    ///
    /// Several trading endpoints — `scheduleCancel`, `updateLeverage`, `updateMargin`,
    /// `modifyOrder` — document "no endpoint-specific data". For those, an absent `data` is
    /// the success case, not the missing-payload error [`send`](Self::send) reports.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on transport, status or venue-level failure. An accepted
    /// request that returns nothing yields `Ok(None)`.
    pub async fn send_optional<T: serde::de::DeserializeOwned>(
        &self,
        request: SignedRequest,
    ) -> Result<Option<T>, ClientError> {
        let response = self
            .dispatch(&request, DEFAULT_ENDPOINT_WEIGHT, Self::write_is_retryable)
            .await?;

        let status = response.status.as_u16();
        if !(200..300).contains(&status) {
            return Err(ClientError::Status {
                status,
                body: String::from_utf8_lossy(&response.body).into_owned(),
            });
        }

        let envelope: ApiResponse<T> = serde_json::from_slice(&response.body)?;
        match envelope.into_result() {
            Ok(data) => Ok(Some(data)),
            Err(EnvelopeError::MissingData) => Ok(None),
            Err(other) => Err(ClientError::Transport(other.to_string())),
        }
    }

    /// Sends an unsigned GET against a market-data path.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on transport, status or decoding failure.
    pub async fn get_public<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        params: Option<&HashMap<String, Vec<String>>>,
    ) -> Result<T, ClientError> {
        let url = self.url_for(path);
        let keys = Self::rate_limit_keys(&url);
        let headers = HashMap::from([("Accept".to_string(), "application/json".to_string())]);

        let attempt = || async {
            self.reserve_weight(DEFAULT_ENDPOINT_WEIGHT, now_millis())?;
            self.http
                .request(
                    Method::GET,
                    url.clone(),
                    params,
                    Some(headers.clone()),
                    None,
                    None,
                    Some(keys.clone()),
                )
                .await
                .map_err(ClientError::from)
        };

        // Reads carry no side effect, so the full retry policy applies.
        let response = self
            .retry
            .invocation(
                &url,
                attempt,
                Self::read_is_retryable,
                |error: RetryError| match error {
                    RetryError::Canceled => {
                        ClientError::Transport("client is shutting down".to_string())
                    }
                    other => ClientError::Transport(other.to_string()),
                },
            )
            .retry_delay(&Self::retry_delay)
            .cancellation_token(&self.cancellation)
            .execute()
            .await?;

        Self::decode(response)
    }

    /// Sends one prepared request, charging its weight and retrying per `retryable`.
    ///
    /// Weight is reserved inside the attempt, not around it: each attempt costs the venue's
    /// budget whether or not it succeeds, and reserving once outside would under-count a retry.
    async fn dispatch(
        &self,
        request: &SignedRequest,
        weight: u32,
        retryable: fn(&ClientError) -> bool,
    ) -> Result<HttpResponse, ClientError> {
        let keys = Self::rate_limit_keys(&request.url);

        let attempt = || async {
            self.reserve_weight(weight, now_millis())?;
            self.http
                .request(
                    request.method.clone(),
                    request.url.clone(),
                    None,
                    Some(request.headers.clone()),
                    Some(request.body.clone()),
                    None,
                    Some(keys.clone()),
                )
                .await
                .map_err(ClientError::from)
        };

        self.retry
            .invocation(
                &request.url,
                attempt,
                retryable,
                |error: RetryError| match error {
                    RetryError::Canceled => {
                        ClientError::Transport("client is shutting down".to_string())
                    }
                    other => ClientError::Transport(other.to_string()),
                },
            )
            .retry_delay(&Self::retry_delay)
            .cancellation_token(&self.cancellation)
            .execute()
            .await
    }

    fn decode<T: serde::de::DeserializeOwned>(
        response: HttpResponse,
    ) -> Result<T, ClientError> {
        let status = response.status.as_u16();
        let body = response.body;

        if !(200..300).contains(&status) {
            return Err(ClientError::Status {
                status,
                body: String::from_utf8_lossy(&body).into_owned(),
            });
        }

        let envelope: ApiResponse<T> = serde_json::from_slice(&body)?;
        envelope
            .into_result()
            .map_err(|e| ClientError::Transport(e.to_string()))
    }

    fn build_http(timeout_secs: u64, proxy_url: Option<String>) -> Result<HttpClient, ClientError> {
        let backstop = Quota::per_second(
            NonZeroU32::new(REQUESTS_PER_SECOND_BACKSTOP).expect("a non-zero literal"),
        )
        .expect("a one-second period is a valid replenish interval");

        HttpClient::builder()
            .headers(HashMap::from([
                ("Accept".to_string(), "application/json".to_string()),
                (
                    "User-Agent".to_string(),
                    NAUTILUS_USER_AGENT.to_string(),
                ),
            ]))
            .default_quota(backstop)
            .timeout_secs(timeout_secs)
            .maybe_proxy_url(proxy_url)
            .build()
            .map_err(ClientError::from)
    }

    /// Retry policy for transient transport failures.
    ///
    /// A reset connection or a gateway 5xx is not a decision the venue made about the order,
    /// so giving up on the first one turns a network hiccup into a missed trade. What must
    /// *not* be retried is anything the venue answered deliberately — a rejection, a bad
    /// signature, an unknown symbol — because repeating those only burns the weight budget.
    /// [`Self::is_transient`] draws that line.
    fn build_retry() -> RetryManager<ClientError> {
        RetryManager::new(RetryConfig {
            max_retries: 3,
            initial_delay_ms: 200,
            max_delay_ms: 5_000,
            backoff_factor: 2.0,
            jitter_ms: 250,
            operation_timeout_ms: Some(30_000),
            immediate_first: false,
            max_elapsed_ms: Some(60_000),
        })
    }

    /// Whether a **read** is worth another attempt.
    ///
    /// Reads have no side effect, so anything short of a deliberate venue decision can be
    /// repeated.
    fn read_is_retryable(error: &ClientError) -> bool {
        match error {
            // The request never reached a venue decision.
            ClientError::Transport(_) => true,
            // 5xx is the gateway failing; 429 is it asking for a pause.
            ClientError::Status { status, .. } => *status >= 500 || *status == 429,
            // Our own budget said wait. Retrying after the carried delay is the whole point.
            ClientError::RateLimited(_) => true,
            // Everything else is a decision: a rejection, a bad signature, a malformed body.
            _ => false,
        }
    }

    /// Whether a **write** is worth another attempt.
    ///
    /// Far narrower than [`Self::read_is_retryable`], and deliberately so. A transport failure
    /// on a write does not say whether the venue received it: the request may have been
    /// processed and only the response lost. Repeating it could place a second order.
    ///
    /// The only safe repeat is a failure raised **before anything was sent** — the weight
    /// budget refusing to let the request out. Everything else is left to the caller, which
    /// knows whether its action is safe to repeat.
    ///
    /// This is tighter than it has to be, and the reason is a gap elsewhere: the adapter has
    /// no order-status query yet, so after an ambiguous write there is no way to ask the venue
    /// what happened. When that query exists, a write retry can be made safe by reconciling
    /// first, and this predicate can widen.
    fn write_is_retryable(error: &ClientError) -> bool {
        matches!(error, ClientError::RateLimited(_))
    }

    /// How long a failure itself says to wait, when it knows.
    ///
    /// The weight budget computes the exact moment capacity returns, so the retry loop should
    /// use that rather than its own backoff curve, which would either wake too early and burn
    /// another rejection or too late and lose the slot.
    fn retry_delay(error: &ClientError) -> Option<Duration> {
        match error {
            ClientError::RateLimited(limited) => {
                Some(Duration::from_millis(limited.retry_after_ms))
            }
            _ => None,
        }
    }

    /// Rate-limit keys for a URL, most specific first, as the shared client documents.
    ///
    /// Derived from the path after the API version so that spot and perps share one bucket per
    /// logical endpoint rather than splitting it by market, and so the host does not become
    /// part of the key.
    fn rate_limit_keys(url: &str) -> Vec<String> {
        let path = url
            .split_once("/api/v1/")
            .map_or(url, |(_, rest)| rest)
            .trim_start_matches('/');

        match path.split_once('/') {
            Some((head, _)) => vec![path.to_string(), head.to_string()],
            None => vec![path.to_string()],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        common::enums::OrderSide,
        http::requests::{ClientOrderId, NewOrderRequest, OrderItem},
    };

    fn client() -> SodexHttpClient {
        let key = ApiPrivateKey::parse(&"2".repeat(64)).unwrap();
        let name = ApiKeyName::parse("api-key-01").unwrap();
        SodexHttpClient::with_credentials(Network::Testnet, Market::Perps, name, &key).unwrap()
    }

    fn order() -> NewOrderRequest {
        NewOrderRequest::new(
            12345,
            1,
            vec![OrderItem::market(
                ClientOrderId::parse("my-order-1").unwrap(),
                OrderSide::Buy,
                "0.001",
            )],
        )
        .unwrap()
    }

    #[test]
    fn body_carries_params_only_without_the_signing_envelope() {
        // The venue hashes {type, params} but expects only params on the wire. Sending the
        // envelope would fail verification with an unhelpful message.
        let request = client()
            .build_signed(Method::POST, "/trade/orders", "newOrder", &order())
            .unwrap();

        let body = request.body_str();

        // Note the assertion is on the envelope's own keys, not on `"type"` alone: an order
        // item legitimately carries a `type` field, so a bare substring check would fail on
        // correct output.
        assert!(!body.contains(r#""type":"newOrder""#), "envelope leaked: {body}");
        assert!(!body.contains(r#""params":"#), "envelope leaked: {body}");
        assert!(body.starts_with(r#"{"accountID":12345"#), "{body}");
    }

    #[test]
    fn body_matches_the_signed_params_byte_for_byte() {
        let request = client()
            .build_signed(Method::POST, "/trade/orders", "newOrder", &order())
            .unwrap();

        // Any divergence between what was hashed and what is sent breaks verification.
        assert_eq!(request.body, serde_json::to_vec(&order()).unwrap());
    }

    #[test]
    fn api_key_header_carries_the_name_not_the_address() {
        // The venue lists this confusion as the most common integration error.
        let request = client()
            .build_signed(Method::POST, "/trade/orders", "newOrder", &order())
            .unwrap();

        assert_eq!(request.headers.get(HEADER_API_KEY).unwrap(), "api-key-01");
        assert!(!request.headers[HEADER_API_KEY].starts_with("0x"));
    }

    #[test]
    fn signature_header_is_hex_with_the_exchange_prefix() {
        let request = client()
            .build_signed(Method::POST, "/trade/orders", "newOrder", &order())
            .unwrap();

        let signature = request.headers.get(HEADER_API_SIGN).unwrap();
        assert!(signature.starts_with("0x01"), "{signature}");
        // 0x + 66 bytes (1 prefix + 65 signature) as hex.
        assert_eq!(signature.len(), 2 + 66 * 2);
    }

    #[test]
    fn every_request_draws_a_fresh_nonce() {
        let client = client();

        let first = client
            .build_signed(Method::POST, "/trade/orders", "newOrder", &order())
            .unwrap();
        let second = client
            .build_signed(Method::POST, "/trade/orders", "newOrder", &order())
            .unwrap();

        let a: u64 = first.headers[HEADER_API_NONCE].parse().unwrap();
        let b: u64 = second.headers[HEADER_API_NONCE].parse().unwrap();
        assert!(b > a, "nonces must advance: {a} then {b}");
    }

    #[test]
    fn identical_payloads_produce_different_signatures_via_the_nonce() {
        // The nonce is inside the signed struct, so replaying a body is not enough.
        let client = client();
        let first = client
            .build_signed(Method::POST, "/trade/orders", "newOrder", &order())
            .unwrap();
        let second = client
            .build_signed(Method::POST, "/trade/orders", "newOrder", &order())
            .unwrap();

        assert_eq!(first.body, second.body);
        assert_ne!(
            first.headers[HEADER_API_SIGN],
            second.headers[HEADER_API_SIGN]
        );
    }

    #[test]
    fn public_client_refuses_to_sign() {
        let public = SodexHttpClient::new_public(Network::Testnet, Market::Perps).unwrap();

        assert!(!public.can_sign());
        assert!(matches!(
            public.build_signed(Method::POST, "/trade/orders", "newOrder", &order()),
            Err(ClientError::CredentialsRequired)
        ));
    }

    #[test]
    fn urls_are_scoped_to_the_configured_network_and_market() {
        assert_eq!(
            client().url_for("/trade/orders"),
            "https://testnet-gw.sodex.dev/api/v1/perps/trade/orders"
        );
    }

    #[test]
    fn weight_budget_is_shared_across_calls() {
        let client = client();

        client.reserve_weight(1100, 1_000).unwrap();
        assert!(matches!(
            client.reserve_weight(200, 1_000),
            Err(ClientError::RateLimited(_))
        ));
    }

    #[test]
    fn after_the_fact_weight_is_recorded_against_the_same_budget() {
        let client = client();

        client.reserve_weight(1000, 1_000).unwrap();
        client.record_weight(200, 1_000);

        assert!(matches!(
            client.reserve_weight(1, 1_000),
            Err(ClientError::RateLimited(_))
        ));
    }
}

#[cfg(test)]
mod transport_policy_tests {
    use super::*;

    fn limited() -> ClientError {
        ClientError::RateLimited(RateLimited {
            axis: crate::http::Axis::IpWeight,
            needed: 20,
            available: 0,
            retry_after_ms: 1_500,
        })
    }

    #[test]
    fn a_read_retries_what_never_reached_a_venue_decision() {
        assert!(SodexHttpClient::read_is_retryable(&ClientError::Transport(
            "connection reset".to_string()
        )));
        assert!(SodexHttpClient::read_is_retryable(&ClientError::Status {
            status: 503,
            body: String::new()
        }));
        assert!(SodexHttpClient::read_is_retryable(&ClientError::Status {
            status: 429,
            body: String::new()
        }));
    }

    #[test]
    fn a_read_does_not_retry_a_decision_the_venue_made() {
        // Repeating a rejection cannot change it and burns the weight budget doing so.
        assert!(!SodexHttpClient::read_is_retryable(&ClientError::Status {
            status: 400,
            body: "invalid request body".to_string()
        }));
        assert!(!SodexHttpClient::read_is_retryable(
            &ClientError::CredentialsRequired
        ));
    }

    #[test]
    fn a_write_does_not_retry_an_ambiguous_transport_failure() {
        // This is the whole point of splitting the predicates. A transport failure on a write
        // does not say whether the venue received it, so repeating it could place a second
        // order — and there is no order-status query yet to find out which happened.
        assert!(!SodexHttpClient::write_is_retryable(
            &ClientError::Transport("connection reset".to_string())
        ));
        assert!(!SodexHttpClient::write_is_retryable(&ClientError::Status {
            status: 503,
            body: String::new()
        }));
    }

    #[test]
    fn a_write_retries_only_a_refusal_raised_before_anything_was_sent() {
        // The weight budget rejects locally, so nothing reached the venue and the repeat is
        // provably free of side effects.
        assert!(SodexHttpClient::write_is_retryable(&limited()));
    }

    #[test]
    fn a_rate_limited_failure_carries_its_own_wait() {
        // The budget knows exactly when capacity returns; the backoff curve does not. Waking
        // early would spend another rejection, waking late would lose the slot.
        assert_eq!(
            SodexHttpClient::retry_delay(&limited()),
            Some(Duration::from_millis(1_500))
        );
        assert_eq!(
            SodexHttpClient::retry_delay(&ClientError::Transport("x".to_string())),
            None
        );
    }

    #[test]
    fn rate_limit_keys_ignore_the_host_and_the_market_segment() {
        // Spot and perps must share one bucket per logical endpoint: they are one venue behind
        // one IP budget, and splitting the key would let the pair double the rate.
        let spot = SodexHttpClient::rate_limit_keys(
            "https://testnet-gw.sodex.dev/api/v1/spot/trade/orders",
        );
        let perps = SodexHttpClient::rate_limit_keys(
            "https://mainnet-gw.sodex.dev/api/v1/perps/trade/orders",
        );

        assert_eq!(spot, vec!["spot/trade/orders".to_string(), "spot".to_string()]);
        assert_eq!(
            perps,
            vec!["perps/trade/orders".to_string(), "perps".to_string()]
        );
        assert_ne!(spot, perps);
    }

    #[test]
    fn the_configured_timeout_reaches_the_transport() {
        // It previously did not: the builder hardcoded 30 seconds and the config field was
        // dead, so a deployment asking for a shorter timeout silently got the default.
        let client = SodexHttpClient::public_with_options(Network::Testnet, Market::Spot, 5, None);

        assert!(client.is_ok());
    }
}
