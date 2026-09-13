//! Account-level requests: registering and revoking API keys.
//!
//! These are the only actions signed by the master wallet, and they are one-off setup rather
//! than part of a trading loop. [`AccountClient`] is therefore separate from
//! [`SodexHttpClient`](super::client::SodexHttpClient): a trading process constructs the
//! latter and never holds the key that owns the account. That is a difference in blast radius
//! rather than in kind: the master wallet cannot be revoked, while an API key can - but an API
//! key registered without a permission mask still withholds nothing.
//!
//! # API keys are registered per engine
//!
//! Spot and perps keep **separate key sets** even though they share an account id. A key
//! registered through the perps gateway is not visible to spot, which rejects requests
//! signed by it with "API key not found". Registering the same public key on both engines
//! is what lets one private key sign for both - that is a deliberate second registration,
//! not something the first one implies.
//!
//! `GET /{engine}/accounts/{address}/api-keys` lists what a given engine actually holds,
//! which is the quickest way to tell this failure from a signing problem.

use std::collections::HashMap;

use alloy_primitives::Address;
use nautilus_network::http::{HttpClient, HttpClientError, Method};
use serde::Serialize;

use super::{
    Network,
    client::{ClientError, HEADER_API_NONCE, HEADER_API_SIGN, SignedRequest},
    models::ApiResponse,
};
use crate::{
    common::{
        Market,
        credential::{ApiKeyName, ApiPrivateKey, MasterPrivateKey},
        enums::DisabledPermissions,
    },
    signing::{ExchangeSigner, NonceGenerator, universal::UniversalSigner},
};

/// Header selecting the signature domain's chain id.
///
/// Required only by account-level actions, and its value must equal the `chainId` the
/// signature's domain used.
pub const HEADER_API_CHAIN: &str = "X-API-Chain";

/// API key type. Only EVM keys are currently supported.
pub const API_KEY_TYPE_EVM: u8 = 1;

/// Never expires.
pub const NO_EXPIRY: u64 = 0;

/// Body of the add-API-key request.
///
/// Field order mirrors the venue's schema table. Note `type` here versus `keyType` in the
/// signed struct - the venue documents that difference as deliberate.
#[derive(Debug, Clone, Serialize)]
pub struct AddApiKeyRequest {
    #[serde(rename = "accountID")]
    pub account_id: u64,
    pub name: String,
    #[serde(rename = "type")]
    pub key_type: u8,
    #[serde(rename = "publicKey")]
    pub public_key: String,
    #[serde(rename = "expiresAt")]
    pub expires_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permissions: Option<u64>,
}

/// Body of the revoke request.
#[derive(Debug, Clone, Serialize)]
pub struct RevokeApiKeyRequest {
    #[serde(rename = "accountID")]
    pub account_id: u64,
    pub name: String,
}

/// A freshly generated signing credential.
///
/// The private key never leaves this value unless the caller takes it out; nothing here
/// logs, serializes or transmits it.
#[derive(Debug)]
pub struct GeneratedApiKey {
    /// Register this with the venue.
    pub public_key: Address,
    /// Keep this locally; it signs trading actions.
    pub private_key: ApiPrivateKey,
}

/// Generates a fresh secp256k1 keypair for use as an API key.
///
/// The venue registers the address and the trading process holds the private key, so the
/// master wallet is needed only to authorize the registration.
///
/// # Errors
///
/// Returns [`ClientError::Credential`] if the generated key cannot be re-parsed, which would
/// indicate a broken key generator rather than user error.
pub fn generate_api_key() -> Result<GeneratedApiKey, ClientError> {
    let signer = alloy::signers::local::PrivateKeySigner::random();
    let public_key = signer.address();
    let hex = alloy_primitives::hex::encode(signer.to_bytes());
    let private_key = ApiPrivateKey::parse(&hex)?;

    Ok(GeneratedApiKey {
        public_key,
        private_key,
    })
}

/// Client for master-wallet-signed account actions.
#[derive(Debug)]
pub struct AccountClient {
    http: HttpClient,
    network: Network,
    market: Market,
    signer: UniversalSigner,
    /// Same master key, exchange domain. Only `revokeAPIKey` needs this combination.
    revocation_signer: ExchangeSigner,
    nonces: NonceGenerator,
}

impl AccountClient {
    /// Creates a client that signs with the master wallet.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] if the key cannot be parsed or the HTTP client cannot be built.
    pub fn new(
        network: Network,
        market: Market,
        master_key: &MasterPrivateKey,
    ) -> Result<Self, ClientError> {
        let signer = UniversalSigner::for_network(master_key, network.chain_id())?;
        let revocation_signer =
            ExchangeSigner::for_master_revocation(master_key, market, network.chain_id())?;

        let http = HttpClient::builder()
            .headers(HashMap::from([(
                "Accept".to_string(),
                "application/json".to_string(),
            )]))
            .rate_limiters(Vec::new())
            .timeout_secs(30)
            .build()
            .map_err(|e: HttpClientError| ClientError::Transport(e.to_string()))?;

        Ok(Self {
            http,
            network,
            market,
            signer,
            revocation_signer,
            nonces: NonceGenerator::new(),
        })
    }

    /// The master wallet address the venue will recover from the signature.
    #[must_use]
    pub fn master_address(&self) -> Address {
        self.signer.address()
    }

    /// Prepares a signed add-API-key request without sending it.
    ///
    /// `permissions` is a mask of *withheld* permissions; pass `None` to enable everything.
    /// The venue does not accept a permissioned key that leaves both `TRADE` and `CANCEL`
    /// enabled, so such a mask is refused here rather than at the gateway.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on signing or serialization failure, or if the permission
    /// mask is one the venue rejects.
    pub fn build_add_api_key(
        &self,
        account_id: u64,
        name: &ApiKeyName,
        public_key: Address,
        expires_at: u64,
        permissions: Option<DisabledPermissions>,
    ) -> Result<SignedRequest, ClientError> {
        if let Some(mask) = permissions
            && !mask.is_disabled(DisabledPermissions::TRADE)
            && !mask.is_disabled(DisabledPermissions::CANCEL)
        {
            return Err(ClientError::Transport(
                "a permissioned key must disable TRADE, CANCEL, or both".to_string(),
            ));
        }

        let nonce = self.nonces.next();
        let network_chain_id = self.network.chain_id();

        let signature = match permissions {
            None => self.signer.sign_add_api_key(
                network_chain_id,
                nonce,
                account_id,
                name.as_str(),
                API_KEY_TYPE_EVM,
                public_key,
                expires_at,
            )?,
            Some(mask) => self.signer.sign_add_permissioned_api_key(
                network_chain_id,
                nonce,
                account_id,
                name.as_str(),
                API_KEY_TYPE_EVM,
                public_key,
                expires_at,
                mask.as_mask(),
            )?,
        };

        let body = AddApiKeyRequest {
            account_id,
            name: name.as_str().to_string(),
            key_type: API_KEY_TYPE_EVM,
            public_key: format!("{public_key:#x}"),
            expires_at,
            permissions: permissions.map(DisabledPermissions::as_mask),
        };

        Ok(SignedRequest {
            method: Method::POST,
            url: self.url_for("/accounts/api-keys"),
            headers: self.headers(&signature, nonce),
            body: serde_json::to_vec(&body)?,
        })
    }

    /// Prepares a signed revoke request without sending it.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on signing or serialization failure.
    pub fn build_revoke_api_key(
        &self,
        account_id: u64,
        name: &ApiKeyName,
    ) -> Result<SignedRequest, ClientError> {
        // Revocation is the one action signed with the master key under the *exchange*
        // domain: it appears in the venue's trading-action list, so it commits to an
        // ExchangeAction with the 0x01 prefix, while the key table still requires the master
        // wallet because it changes the API key set. Signing it under the universal domain
        // like addAPIKey would be rejected.
        let nonce = self.nonces.next();
        let body = RevokeApiKeyRequest {
            account_id,
            name: name.as_str().to_string(),
        };

        let digest = crate::signing::payload_hash("revokeAPIKey", &body)?;
        let signature = self.revocation_signer.sign_action(digest, nonce)?;

        // No X-API-Chain here: that header belongs to the universal domain only.
        let mut headers = HashMap::from([
            ("Content-Type".to_string(), "application/json".to_string()),
            ("Accept".to_string(), "application/json".to_string()),
            (
                HEADER_API_SIGN.to_string(),
                alloy_primitives::hex::encode_prefixed(&signature),
            ),
            (HEADER_API_NONCE.to_string(), nonce.to_string()),
        ]);
        headers.shrink_to_fit();

        Ok(SignedRequest {
            method: Method::DELETE,
            url: self.url_for("/accounts/api-keys"),
            headers,
            body: serde_json::to_vec(&body)?,
        })
    }

    /// Sends a prepared account request.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on transport, status, or venue-level failure.
    pub async fn send<T: serde::de::DeserializeOwned>(
        &self,
        request: SignedRequest,
    ) -> Result<Option<T>, ClientError> {
        let response = self
            .http
            .request(
                request.method,
                request.url,
                None,
                Some(request.headers),
                Some(request.body),
                None,
                None,
            )
            .await
            .map_err(|e| ClientError::Transport(e.to_string()))?;

        let status = response.status.as_u16();
        if !(200..300).contains(&status) {
            return Err(ClientError::Status {
                status,
                body: String::from_utf8_lossy(&response.body).into_owned(),
            });
        }

        // These endpoints document no endpoint-specific payload, so an absent `data` is a
        // success rather than the error it would be for a query.
        let envelope: ApiResponse<T> = serde_json::from_slice(&response.body)?;
        if let Some(error) = envelope.error {
            return Err(ClientError::Transport(error));
        }
        Ok(envelope.data)
    }

    fn url_for(&self, path: &str) -> String {
        format!("{}{path}", self.network.market_base(self.market))
    }

    fn headers(&self, signature: &[u8], nonce: u64) -> HashMap<String, String> {
        HashMap::from([
            ("Content-Type".to_string(), "application/json".to_string()),
            ("Accept".to_string(), "application/json".to_string()),
            (
                HEADER_API_SIGN.to_string(),
                alloy_primitives::hex::encode_prefixed(signature),
            ),
            (HEADER_API_NONCE.to_string(), nonce.to_string()),
            // Must equal the domain chain id the signature used, not the network id per se.
            (
                HEADER_API_CHAIN.to_string(),
                self.signer.api_chain().to_string(),
            ),
        ])
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    fn client() -> AccountClient {
        let key = MasterPrivateKey::parse(&"3".repeat(64)).unwrap();
        AccountClient::new(Network::Testnet, Market::Perps, &key).unwrap()
    }

    fn name() -> ApiKeyName {
        ApiKeyName::parse("api-key-01").unwrap()
    }

    #[rstest]
    fn generated_keys_are_distinct_and_usable() {
        let a = generate_api_key().unwrap();
        let b = generate_api_key().unwrap();

        assert_ne!(a.public_key, b.public_key);
        assert_ne!(a.private_key.as_hex(), b.private_key.as_hex());
        assert_eq!(a.private_key.as_hex().len(), 64);
    }

    #[rstest]
    fn generated_private_key_derives_the_reported_address() {
        // If these disagreed, the venue would register an address whose key we do not hold,
        // and every subsequent signature would fail authentication.
        use crate::{common::Market, signing::ExchangeSigner};

        let generated = generate_api_key().unwrap();
        let signer = ExchangeSigner::new(
            &generated.private_key,
            Market::Perps,
            crate::common::CHAIN_ID_TESTNET,
        )
        .unwrap();

        assert_eq!(signer.address(), generated.public_key);
    }

    #[rstest]
    fn add_request_carries_the_chain_header_matching_the_signing_domain() {
        // The venue rejects the signature when X-API-Chain and domain.chainId disagree.
        let client = client();
        let request = client
            .build_add_api_key(60366, &name(), Address::ZERO, NO_EXPIRY, None)
            .unwrap();

        assert_eq!(
            request.headers[HEADER_API_CHAIN],
            crate::common::CHAIN_ID_TESTNET.to_string()
        );
    }

    #[rstest]
    fn add_request_signature_uses_the_universal_prefix() {
        let request = client()
            .build_add_api_key(60366, &name(), Address::ZERO, NO_EXPIRY, None)
            .unwrap();

        assert!(request.headers[HEADER_API_SIGN].starts_with("0x02"));
    }

    #[rstest]
    fn add_request_body_uses_type_not_key_type() {
        // The signed struct calls this keyType; the body must call it type.
        let request = client()
            .build_add_api_key(60366, &name(), Address::ZERO, NO_EXPIRY, None)
            .unwrap();

        let body = request.body_str();
        assert!(body.contains(r#""type":1"#), "{body}");
        assert!(!body.contains("keyType"), "{body}");
    }

    #[rstest]
    fn permissions_are_omitted_unless_requested() {
        let plain = client()
            .build_add_api_key(60366, &name(), Address::ZERO, NO_EXPIRY, None)
            .unwrap();
        assert!(!plain.body_str().contains("permissions"));

        let restricted = client()
            .build_add_api_key(
                60366,
                &name(),
                Address::ZERO,
                NO_EXPIRY,
                Some(DisabledPermissions::cancel_only()),
            )
            .unwrap();
        assert!(restricted.body_str().contains(r#""permissions":13"#));
    }

    #[rstest]
    fn permissioned_key_leaving_trade_and_cancel_enabled_is_refused() {
        // The venue does not support this combination; failing here saves a round trip and
        // gives a reason instead of a gateway error code.
        let only_withdraw_disabled =
            DisabledPermissions::none().disabling(DisabledPermissions::WITHDRAW);

        let result = client().build_add_api_key(
            60366,
            &name(),
            Address::ZERO,
            NO_EXPIRY,
            Some(only_withdraw_disabled),
        );

        assert!(result.is_err());
    }

    #[rstest]
    fn requests_target_the_documented_path() {
        let request = client()
            .build_add_api_key(60366, &name(), Address::ZERO, NO_EXPIRY, None)
            .unwrap();

        assert_eq!(
            request.url,
            "https://testnet-gw.sodex.dev/api/v1/perps/accounts/api-keys"
        );
    }

    #[rstest]
    fn revoke_uses_delete_and_names_the_key() {
        let request = client().build_revoke_api_key(60366, &name()).unwrap();

        assert_eq!(request.method, Method::DELETE);
        assert!(request.body_str().contains(r#""name":"api-key-01""#));
    }

    #[rstest]
    fn revoke_signs_under_the_exchange_domain_not_the_universal_one() {
        // revokeAPIKey is a trading-domain action signed by the master key, so it carries
        // the 0x01 prefix and no X-API-Chain. Signing it like addAPIKey would be rejected.
        let request = client().build_revoke_api_key(60366, &name()).unwrap();

        assert!(
            request.headers[HEADER_API_SIGN].starts_with("0x01"),
            "{}",
            request.headers[HEADER_API_SIGN]
        );
        assert!(!request.headers.contains_key(HEADER_API_CHAIN));
    }

    #[rstest]
    fn add_and_revoke_use_different_signature_families() {
        let client = client();
        let add = client
            .build_add_api_key(60366, &name(), Address::ZERO, NO_EXPIRY, None)
            .unwrap();
        let revoke = client.build_revoke_api_key(60366, &name()).unwrap();

        assert!(add.headers[HEADER_API_SIGN].starts_with("0x02"));
        assert!(revoke.headers[HEADER_API_SIGN].starts_with("0x01"));
    }

    #[rstest]
    fn each_account_action_draws_a_fresh_nonce() {
        let client = client();

        let first = client
            .build_add_api_key(60366, &name(), Address::ZERO, NO_EXPIRY, None)
            .unwrap();
        let second = client
            .build_add_api_key(60366, &name(), Address::ZERO, NO_EXPIRY, None)
            .unwrap();

        let a: u64 = first.headers[HEADER_API_NONCE].parse().unwrap();
        let b: u64 = second.headers[HEADER_API_NONCE].parse().unwrap();
        assert!(b > a);
    }
}
