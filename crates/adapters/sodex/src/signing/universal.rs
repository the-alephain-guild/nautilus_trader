//! Account-level signing under the universal domain.
//!
//! Registering and revoking API keys changes who may trade, so the venue requires those
//! actions to be signed by the master wallet under a separate EIP-712 domain named
//! `universal`, with signatures prefixed `0x02` instead of `0x01`.
//!
//! # The domain chain id is not the network
//!
//! Two different chain ids appear in one signature and they mean different things:
//!
//! - `domain.chainId` must equal whatever is sent in the `X-API-Chain` header. The venue
//!   states it "may be any uint64; it identifies the signature domain and does not select
//!   the Sodex network".
//! - `message.chainID` is what actually selects mainnet (286623) or testnet (138565).
//!
//! Setting the domain value from the network id is a reasonable convention and is what
//! [`UniversalSigner::for_network`] does, but the two remain distinct fields and the header
//! must agree with the domain, not with the message.
//!
//! # `type` on the wire is `keyType` in the signature
//!
//! The venue documents this asymmetry directly: "The REST request JSON and the EIP-712 typed
//! data intentionally use different field names for the API key type. In request payloads
//! this field is `type`, while in the typed signing structure it is `keyType`."

use std::str::FromStr;

use alloy::{
    signers::{SignerSync, local::PrivateKeySigner},
    sol_types::{Eip712Domain, SolStruct, eip712_domain},
};
use alloy_primitives::{Address, Bytes};

use super::signers::SigningError;
use crate::common::{SignatureKind, credential::MasterPrivateKey};

alloy::sol! {
    struct UserSignedAddAPIKeyAction {
        uint64 chainID;
        uint64 nonce;
        uint64 accountID;
        string name;
        uint8 keyType;
        bytes publicKey;
        uint64 expiresAt;
    }
}

alloy::sol! {
    struct UserSignedAddPermissionedAPIKeyAction {
        uint64 chainID;
        uint64 nonce;
        uint64 accountID;
        string name;
        uint8 keyType;
        bytes publicKey;
        uint64 expiresAt;
        uint64 permissions;
    }
}

/// Signs account-level actions with the master wallet.
///
/// Held separately from the trading signer and constructed from [`MasterPrivateKey`], so the
/// two cannot be confused: a process that only trades never builds one of these.
#[derive(Debug, Clone)]
pub struct UniversalSigner {
    signer: PrivateKeySigner,
    domain: Eip712Domain,
    api_chain: u64,
}

impl UniversalSigner {
    /// Creates a signer with an explicit domain chain id.
    ///
    /// # Errors
    ///
    /// Returns [`SigningError::Signer`] if the key cannot be parsed.
    pub fn new(key: &MasterPrivateKey, api_chain: u64) -> Result<Self, SigningError> {
        let signer = PrivateKeySigner::from_str(key.as_hex())
            .map_err(|e| SigningError::Signer(e.to_string()))?;

        let domain = eip712_domain! {
            name: "universal",
            version: "1",
            chain_id: api_chain,
            verifying_contract: Address::ZERO,
        };

        Ok(Self {
            signer,
            domain,
            api_chain,
        })
    }

    /// Creates a signer whose domain chain id mirrors the network's chain id.
    ///
    /// # Errors
    ///
    /// Returns [`SigningError::Signer`] if the key cannot be parsed.
    pub fn for_network(key: &MasterPrivateKey, chain_id: u64) -> Result<Self, SigningError> {
        Self::new(key, chain_id)
    }

    /// The value that must be sent as `X-API-Chain`.
    ///
    /// The venue rejects the signature if the header and `domain.chainId` disagree, so this
    /// is exposed rather than left for the caller to reconstruct.
    #[must_use]
    pub const fn api_chain(&self) -> u64 {
        self.api_chain
    }

    /// The master wallet's address.
    #[must_use]
    pub fn address(&self) -> Address {
        self.signer.address()
    }

    /// Signs an `addAPIKey` action, returning the value for `X-API-Sign`.
    ///
    /// `public_key` is the EVM address of the key being registered - the address whose
    /// private key will then sign trading actions.
    ///
    /// # Errors
    ///
    /// Returns [`SigningError::Sign`] if the digest cannot be signed.
    // One parameter per field of `UserSignedAddAPIKeyAction`. A wrapper struct would just
    // restate that type while putting a layer between the call and what gets signed.
    #[allow(clippy::too_many_arguments)]
    pub fn sign_add_api_key(
        &self,
        network_chain_id: u64,
        nonce: u64,
        account_id: u64,
        name: &str,
        key_type: u8,
        public_key: Address,
        expires_at: u64,
    ) -> Result<Vec<u8>, SigningError> {
        let action = UserSignedAddAPIKeyAction {
            chainID: network_chain_id,
            nonce,
            accountID: account_id,
            name: name.to_string(),
            keyType: key_type,
            publicKey: Bytes::from(public_key.to_vec()),
            expiresAt: expires_at,
        };

        self.finish(action.eip712_signing_hash(&self.domain))
    }

    /// Signs an `addPermissionedAPIKey` action.
    ///
    /// `permissions` is a mask whose set bits **disable** the corresponding permission.
    ///
    /// # Errors
    ///
    /// Returns [`SigningError::Sign`] if the digest cannot be signed.
    #[allow(clippy::too_many_arguments)]
    pub fn sign_add_permissioned_api_key(
        &self,
        network_chain_id: u64,
        nonce: u64,
        account_id: u64,
        name: &str,
        key_type: u8,
        public_key: Address,
        expires_at: u64,
        permissions: u64,
    ) -> Result<Vec<u8>, SigningError> {
        let action = UserSignedAddPermissionedAPIKeyAction {
            chainID: network_chain_id,
            nonce,
            accountID: account_id,
            name: name.to_string(),
            keyType: key_type,
            publicKey: Bytes::from(public_key.to_vec()),
            expiresAt: expires_at,
            permissions,
        };

        self.finish(action.eip712_signing_hash(&self.domain))
    }

    fn finish(&self, digest: alloy_primitives::B256) -> Result<Vec<u8>, SigningError> {
        let signature = self
            .signer
            .sign_hash_sync(&digest)
            .map_err(|e| SigningError::Sign(e.to_string()))?;

        // See the note in `signers.rs`: the recovery id must be 0 or 1, not `27 + parity`.
        let raw = signature.as_rsy();
        let mut out = Vec::with_capacity(1 + raw.len());
        out.push(SignatureKind::Universal.prefix());
        out.extend_from_slice(&raw);
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;
    use crate::common::{CHAIN_ID_MAINNET, CHAIN_ID_TESTNET, enums::DisabledPermissions};

    fn signer(api_chain: u64) -> UniversalSigner {
        let key = MasterPrivateKey::parse(&"3".repeat(64)).unwrap();
        UniversalSigner::new(&key, api_chain).unwrap()
    }

    fn public_key() -> Address {
        Address::from_str("0x3d4595c8742d0a58173a9963c05755b59a8f8256").unwrap()
    }

    fn sign_with(signer: &UniversalSigner, network_chain_id: u64) -> Vec<u8> {
        signer
            .sign_add_api_key(
                network_chain_id,
                1_760_373_925_000,
                1010,
                "api-key-01",
                1,
                public_key(),
                0,
            )
            .unwrap()
    }

    #[rstest]
    fn signature_carries_the_universal_prefix() {
        // 0x02, not the 0x01 used for trading actions. The venue rejects the wrong prefix.
        let signature = sign_with(&signer(CHAIN_ID_TESTNET), CHAIN_ID_TESTNET);

        assert_eq!(signature[0], 0x02);
        assert_eq!(signature.len(), 66);
    }

    #[rstest]
    fn domain_chain_id_is_reported_for_the_header() {
        // X-API-Chain must equal domain.chainId or verification fails, so the value the
        // signer actually used is exposed rather than reconstructed by the caller.
        let signer = signer(42);

        assert_eq!(signer.api_chain(), 42);
    }

    #[rstest]
    fn domain_chain_id_and_network_chain_id_are_independent() {
        // The domain value identifies the signature domain; the message value selects the
        // network. Changing either alone must change the signature.
        let base = signer(42);
        let other_domain = signer(43);

        let a = sign_with(&base, CHAIN_ID_TESTNET);
        let b = sign_with(&other_domain, CHAIN_ID_TESTNET);
        assert_ne!(a, b, "domain chain id must affect the signature");

        let c = sign_with(&base, CHAIN_ID_MAINNET);
        assert_ne!(a, c, "message chain id must affect the signature");
    }

    #[rstest]
    fn for_network_mirrors_the_network_chain_id_into_the_domain() {
        let key = MasterPrivateKey::parse(&"3".repeat(64)).unwrap();
        let signer = UniversalSigner::for_network(&key, CHAIN_ID_TESTNET).unwrap();

        assert_eq!(signer.api_chain(), CHAIN_ID_TESTNET);
    }

    #[rstest]
    fn permissioned_and_plain_actions_are_different_structs() {
        // Different EIP-712 type hashes, so the same inputs must not collide.
        let signer = signer(CHAIN_ID_TESTNET);

        let plain = sign_with(&signer, CHAIN_ID_TESTNET);
        let permissioned = signer
            .sign_add_permissioned_api_key(
                CHAIN_ID_TESTNET,
                1_760_373_925_000,
                1010,
                "api-key-01",
                1,
                public_key(),
                0,
                DisabledPermissions::cancel_only().as_mask(),
            )
            .unwrap();

        assert_ne!(plain, permissioned);
        assert_eq!(permissioned[0], 0x02);
    }

    #[rstest]
    fn permission_mask_changes_the_signature() {
        let signer = signer(CHAIN_ID_TESTNET);

        let sign_mask = |mask: u64| {
            signer
                .sign_add_permissioned_api_key(
                    CHAIN_ID_TESTNET,
                    1_760_373_925_000,
                    1010,
                    "api-key-01",
                    1,
                    public_key(),
                    0,
                    mask,
                )
                .unwrap()
        };

        assert_ne!(
            sign_mask(DisabledPermissions::none().as_mask()),
            sign_mask(DisabledPermissions::cancel_only().as_mask())
        );
    }

    #[rstest]
    fn recovery_id_is_raw_zero_or_one_not_eip155_v() {
        // Same gateway constraint as the exchange signer; registration failed on this first.
        let signer = signer(CHAIN_ID_TESTNET);

        let seen: Vec<u8> = (0..32)
            .map(|n| {
                let sig = signer
                    .sign_add_api_key(
                        CHAIN_ID_TESTNET,
                        1_760_373_925_000 + n,
                        1010,
                        "api-key-01",
                        1,
                        public_key(),
                        0,
                    )
                    .unwrap();
                sig[65]
            })
            .collect();

        for recovery_id in &seen {
            assert!(
                *recovery_id <= 1,
                "recovery id {recovery_id} must be 0 or 1"
            );
        }
        assert!(
            seen.contains(&0) && seen.contains(&1),
            "both parities exercised"
        );
    }

    #[rstest]
    fn master_address_is_derived_from_the_key() {
        // The venue recovers this address from the signature; if it does not match the
        // account's owner the action is rejected.
        let signer = signer(CHAIN_ID_TESTNET);

        assert_ne!(signer.address(), Address::ZERO);
    }
}
