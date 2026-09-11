//! Signer construction and action signing.

use std::str::FromStr;

use alloy::{
    signers::{SignerSync, local::PrivateKeySigner},
    sol_types::{Eip712Domain, SolStruct, eip712_domain},
};
use alloy_primitives::{Address, B256, keccak256};
use serde::Serialize;

use crate::common::{
    Market, SignatureKind,
    credential::{ApiPrivateKey, MasterPrivateKey},
};

// The typed struct bound into every trading-action signature. `payloadHash` commits to the
// JSON body; `nonce` is replayed-protected by the gateway's per-signer nonce window.
alloy::sol! {
    struct ExchangeAction {
        bytes32 payloadHash;
        uint64 nonce;
    }
}

/// Errors raised while signing.
#[derive(Debug, thiserror::Error)]
pub enum SigningError {
    #[error("failed to construct signer from private key: {0}")]
    Signer(String),
    #[error("failed to serialize signing payload: {0}")]
    Serialize(#[from] serde_json::Error),
    #[error("failed to sign digest: {0}")]
    Sign(String),
}

/// The `{type, params}` envelope that gets hashed.
///
/// Declared as a struct rather than built as a map so that `type` reliably precedes
/// `params` in the serialized output.
#[derive(Serialize)]
struct SigningPayload<'a, T: Serialize> {
    #[serde(rename = "type")]
    action_type: &'a str,
    params: &'a T,
}

/// Computes `keccak256` over the compact JSON encoding of `{type, params}`.
///
/// `params` must be a concrete `Serialize` type whose field order mirrors the
/// corresponding Go struct — see the [module docs](super) for why a
/// [`serde_json::Value`] cannot be used here: its key order is decided by a Cargo feature
/// this crate does not control.
///
/// # Errors
///
/// Returns [`SigningError::Serialize`] if `params` cannot be serialized.
pub fn payload_hash<T: Serialize>(action_type: &str, params: &T) -> Result<B256, SigningError> {
    let payload = SigningPayload {
        action_type,
        params,
    };
    // `to_string` (not `to_string_pretty`) yields the compact form the venue hashes.
    let json = serde_json::to_string(&payload)?;
    Ok(keccak256(json.as_bytes()))
}

/// Signs trading actions with a registered API key.
///
/// Deliberately accepts only [`ApiPrivateKey`]: account-level actions belong to the master
/// wallet and use a different domain and prefix, so they cannot be signed through this type.
#[derive(Debug, Clone)]
pub struct ExchangeSigner {
    signer: PrivateKeySigner,
    domain: Eip712Domain,
}

impl ExchangeSigner {
    /// Creates a signer bound to one market and one chain.
    ///
    /// The binding is intentional: `domain.name` distinguishes spot from perps and
    /// `domain.chainId` distinguishes mainnet from testnet, so a signer cannot be
    /// accidentally reused across either boundary.
    ///
    /// # Errors
    ///
    /// Returns [`SigningError::Signer`] if the key cannot be parsed into a signer.
    pub fn new(key: &ApiPrivateKey, market: Market, chain_id: u64) -> Result<Self, SigningError> {
        Self::from_hex(key.as_hex(), market, chain_id)
    }

    /// Creates a signer from the master wallet, for the one action that requires it.
    ///
    /// `revokeAPIKey` sits in the venue's trading-action list — it commits to an
    /// [`ExchangeAction`] under the `spot`/`futures` domain with the `0x01` prefix — yet the
    /// "which key signs what" table requires the **master wallet** to sign it, because it
    /// changes the API key set itself. That makes it the only combination of master key and
    /// exchange domain, and this constructor exists solely for it.
    ///
    /// # Errors
    ///
    /// Returns [`SigningError::Signer`] if the key cannot be parsed into a signer.
    pub fn for_master_revocation(
        key: &MasterPrivateKey,
        market: Market,
        chain_id: u64,
    ) -> Result<Self, SigningError> {
        Self::from_hex(key.as_hex(), market, chain_id)
    }

    fn from_hex(hex: &str, market: Market, chain_id: u64) -> Result<Self, SigningError> {
        let signer =
            PrivateKeySigner::from_str(hex).map_err(|e| SigningError::Signer(e.to_string()))?;

        let domain = eip712_domain! {
            name: market.domain_name(),
            version: "1",
            chain_id: chain_id,
            verifying_contract: Address::ZERO,
        };

        Ok(Self { signer, domain })
    }

    /// The EVM address derived from the signing key.
    ///
    /// This is what the venue registered as the API key's public key, and what it recovers
    /// from a submitted signature to authenticate the request.
    #[must_use]
    pub fn address(&self) -> Address {
        self.signer.address()
    }

    /// Signs an action, returning the 66-byte value for the `X-API-Sign` header:
    /// the `0x01` type prefix followed by the 65-byte signature.
    ///
    /// # Errors
    ///
    /// Returns [`SigningError::Sign`] if the digest cannot be signed.
    pub fn sign_action(&self, payload_hash: B256, nonce: u64) -> Result<Vec<u8>, SigningError> {
        let action = ExchangeAction {
            payloadHash: payload_hash,
            nonce,
        };
        let digest = action.eip712_signing_hash(&self.domain);
        let signature = self
            .signer
            .sign_hash_sync(&digest)
            .map_err(|e| SigningError::Sign(e.to_string()))?;

        // `as_rsy`, not `as_bytes`: the gateway recovers the signer from a raw recovery id
        // of 0 or 1, while `as_bytes` emits the EIP-155 style `27 + parity`. Sending the
        // latter is rejected with "Invalid recovery ID: bad recovery id" — a distinction the
        // venue's documentation never states, since it publishes no expected signature bytes.
        let raw = signature.as_rsy();
        let mut out = Vec::with_capacity(1 + raw.len());
        out.push(SignatureKind::Exchange.prefix());
        out.extend_from_slice(&raw);
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::CHAIN_ID_TESTNET;

    use crate::{
        common::enums::OrderSide,
        http::requests::{ClientOrderId, NewOrderRequest, OrderItem},
    };

    /// The venue's worked example, built from the production request types.
    ///
    /// Deliberately not a local copy of the payload shape: a duplicate would drift from the
    /// real types, and these tests would then keep passing against a payload the adapter no
    /// longer sends.
    fn doc_example_params() -> NewOrderRequest {
        let cl_ord_id = ClientOrderId::parse("my-order-1").expect("valid id");
        NewOrderRequest::new(
            12345,
            1,
            vec![OrderItem::market(cl_ord_id, OrderSide::Buy, "0.001")],
        )
        .expect("valid batch")
    }

    /// Pins the serialized form against the worked example in the venue's signing guide.
    ///
    /// This is the whole field-order contract in one assertion: key order, omitted
    /// `omitempty` fields, quoted `DecimalString`, and present-at-zero-value non-optionals.
    /// If a future edit reorders a field or drops a `skip_serializing_if`, this fails here
    /// rather than as an opaque signature rejection from the gateway.
    #[test]
    fn signing_payload_matches_venue_example_byte_for_byte() {
        let expected = r#"{"type":"newOrder","params":{"accountID":12345,"symbolID":1,"orders":[{"clOrdID":"my-order-1","modifier":1,"side":1,"type":2,"timeInForce":3,"quantity":"0.001","reduceOnly":false,"positionSide":1}]}}"#;

        let payload = SigningPayload {
            action_type: "newOrder",
            params: &doc_example_params(),
        };
        let actual = serde_json::to_string(&payload).unwrap();

        assert_eq!(actual, expected);
    }

    /// Guards the trap the module docs describe, without asserting the trap's shape.
    ///
    /// An earlier version of this test asserted that a `Value` round-trip *reorders* keys, and
    /// it failed the moment the crate was built with the `python` feature — which enables
    /// `serde_json/preserve_order` transitively and makes the round-trip order-preserving.
    /// That failure was the test doing its job in the wrong place: the hazard is not that
    /// `Value` sorts, it is that **whether it sorts depends on a feature this crate does not
    /// control**, so the same payload could hash two ways in two builds.
    ///
    /// What is pinned here is therefore the property that holds in every configuration: the
    /// signing payload is produced by serializing the concrete type directly, in declaration
    /// order, and that encoding is stable regardless of how `Value` happens to behave.
    #[test]
    fn the_signing_encoding_is_taken_from_the_typed_struct_not_from_a_value() {
        let direct = serde_json::to_string(&doc_example_params()).unwrap();

        // Declaration order, which is the venue's contract.
        assert!(
            direct.starts_with(r#"{"accountID""#),
            "the typed encoding must lead with the first declared field, got {direct}"
        );

        // And the digest is computed from that encoding, not from a re-encoded `Value`.
        let via_value: serde_json::Value = serde_json::from_str(&direct).unwrap();
        let reordered = serde_json::to_string(&via_value).unwrap();
        let digest_direct = payload_hash("newOrder", &doc_example_params()).unwrap();
        let digest_of_direct_text =
            keccak256(format!(r#"{{"type":"newOrder","params":{direct}}}"#).as_bytes());

        assert_eq!(
            digest_direct, digest_of_direct_text,
            "payload_hash must hash the typed encoding verbatim"
        );

        // `reordered` may or may not equal `direct` depending on the feature set; either way it
        // is not what gets signed. Asserting only that the digest follows the typed form keeps
        // this test meaningful in both configurations.
        let _ = reordered;
    }

    #[test]
    fn signature_carries_exchange_prefix_and_is_66_bytes() {
        let key = ApiPrivateKey::parse(&"2".repeat(64)).unwrap();
        let signer = ExchangeSigner::new(&key, Market::Perps, CHAIN_ID_TESTNET).unwrap();

        let hash = payload_hash("newOrder", &doc_example_params()).unwrap();
        let sig = signer.sign_action(hash, 1_760_373_925_001).unwrap();

        assert_eq!(sig.len(), 66, "1 prefix byte + 65 signature bytes");
        assert_eq!(sig[0], 0x01);
    }

    /// The gateway rejected `27 + parity` with "Invalid recovery ID: bad recovery id" on the
    /// first live attempt. Nothing in the venue's documentation states which convention it
    /// wants — it publishes no expected signature bytes — so this test carries the finding.
    #[test]
    fn recovery_id_is_raw_zero_or_one_not_eip155_v() {
        let key = ApiPrivateKey::parse(&"2".repeat(64)).unwrap();
        let signer = ExchangeSigner::new(&key, Market::Perps, CHAIN_ID_TESTNET).unwrap();

        // Sign enough distinct nonces to hit both parities; either one alone would pass a
        // weaker assertion by luck.
        let seen: Vec<u8> = (0..32)
            .map(|nonce| {
                let hash = payload_hash("newOrder", &doc_example_params()).unwrap();
                let sig = signer.sign_action(hash, 1_760_373_925_000 + nonce).unwrap();
                sig[65]
            })
            .collect();

        for recovery_id in &seen {
            assert!(
                *recovery_id <= 1,
                "recovery id {recovery_id} is EIP-155 style; the gateway needs 0 or 1"
            );
        }
        assert!(seen.contains(&0) && seen.contains(&1), "both parities exercised");
    }

    /// Spot and perps sign under different domains, so the same payload must not produce
    /// the same signature. This is the kind of mistake that only surfaces as a gateway
    /// rejection, so it is pinned here.
    #[test]
    fn market_selects_a_distinct_domain() {
        let key = ApiPrivateKey::parse(&"2".repeat(64)).unwrap();
        let hash = payload_hash("newOrder", &doc_example_params()).unwrap();
        let nonce = 1_760_373_925_001;

        let spot = ExchangeSigner::new(&key, Market::Spot, CHAIN_ID_TESTNET)
            .unwrap()
            .sign_action(hash, nonce)
            .unwrap();
        let perps = ExchangeSigner::new(&key, Market::Perps, CHAIN_ID_TESTNET)
            .unwrap()
            .sign_action(hash, nonce)
            .unwrap();

        assert_ne!(spot, perps);
    }

    /// Same for mainnet vs testnet: the chain id is part of the domain.
    #[test]
    fn chain_id_selects_a_distinct_domain() {
        use crate::common::CHAIN_ID_MAINNET;

        let key = ApiPrivateKey::parse(&"2".repeat(64)).unwrap();
        let hash = payload_hash("newOrder", &doc_example_params()).unwrap();
        let nonce = 1_760_373_925_001;

        let mainnet = ExchangeSigner::new(&key, Market::Perps, CHAIN_ID_MAINNET)
            .unwrap()
            .sign_action(hash, nonce)
            .unwrap();
        let testnet = ExchangeSigner::new(&key, Market::Perps, CHAIN_ID_TESTNET)
            .unwrap()
            .sign_action(hash, nonce)
            .unwrap();

        assert_ne!(mainnet, testnet);
    }
}
