//! Credential types for SoDEX.
//!
//! The venue separates account ownership from trading authority, and this module keeps that
//! separation visible in the type system: [`MasterPrivateKey`] and [`ApiPrivateKey`] are
//! distinct types even though both wrap a secp256k1 key, so signing a trading action with
//! the master wallet fails to compile rather than failing at the gateway.

use std::fmt::Debug;

/// Placeholder shown instead of key material in debug output.
const REDACTED: &str = "<redacted>";

/// Errors raised while constructing credentials.
#[derive(Debug, thiserror::Error)]
pub enum CredentialError {
    #[error("private key must be 32 bytes of hex (64 hex chars, optional 0x prefix), received {0}")]
    KeyLength(usize),
    #[error("private key contains non-hex characters")]
    KeyEncoding,
    #[error(
        "API key name must match ^[0-9a-zA-Z_-]{{1,36}}$ and must not be `default`, received {0:?}"
    )]
    KeyName(String),
}

/// A 32-byte secp256k1 private key, stored as hex without the `0x` prefix.
#[derive(Clone)]
struct PrivateKeyHex(String);

impl PrivateKeyHex {
    fn parse(raw: &str) -> Result<Self, CredentialError> {
        let stripped = raw.strip_prefix("0x").unwrap_or(raw);
        if stripped.len() != 64 {
            return Err(CredentialError::KeyLength(stripped.len()));
        }
        if !stripped.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(CredentialError::KeyEncoding);
        }
        Ok(Self(stripped.to_ascii_lowercase()))
    }
}

impl Debug for PrivateKeyHex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(REDACTED)
    }
}

/// The key that owns the account. Signs only `addAPIKey`, `revokeAPIKey` and
/// `approveBuilderFee`; holding it grants full control including withdrawals, so it should
/// live offline and never reach a trading process.
#[derive(Clone, Debug)]
pub struct MasterPrivateKey(PrivateKeyHex);

impl MasterPrivateKey {
    /// # Errors
    ///
    /// Returns [`CredentialError`] if the key is not 32 bytes of hex.
    pub fn parse(raw: &str) -> Result<Self, CredentialError> {
        PrivateKeyHex::parse(raw).map(Self)
    }

    #[must_use]
    pub fn as_hex(&self) -> &str {
        &self.0.0
    }
}

/// The key registered via `addAPIKey`. Signs trading actions and nothing else - it cannot
/// query account data, and it can be revoked without moving funds.
#[derive(Clone, Debug)]
pub struct ApiPrivateKey(PrivateKeyHex);

impl ApiPrivateKey {
    /// # Errors
    ///
    /// Returns [`CredentialError`] if the key is not 32 bytes of hex.
    pub fn parse(raw: &str) -> Result<Self, CredentialError> {
        PrivateKeyHex::parse(raw).map(Self)
    }

    #[must_use]
    pub fn as_hex(&self) -> &str {
        &self.0.0
    }
}

/// The human-readable name of a registered API key.
///
/// This is what travels in the `X-API-Key` header - despite the header's name it carries
/// the *name*, not the public key. Getting this wrong is listed by the venue as the most
/// common integration mistake, so the distinction is encoded as its own type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApiKeyName(String);

impl ApiKeyName {
    /// # Errors
    ///
    /// Returns [`CredentialError::KeyName`] if the name is empty, longer than 36 chars,
    /// contains characters outside `[0-9a-zA-Z_-]`, or is the reserved value `default`.
    pub fn parse(raw: &str) -> Result<Self, CredentialError> {
        let valid_len = (1..=36).contains(&raw.len());
        let valid_chars = raw
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
        if !valid_len || !valid_chars || raw == "default" {
            return Err(CredentialError::KeyName(raw.to_string()));
        }
        Ok(Self(raw.to_string()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[rstest]
    fn parses_key_with_and_without_prefix() {
        let bare = "2".repeat(64);
        let prefixed = format!("0x{bare}");
        assert_eq!(
            ApiPrivateKey::parse(&bare).unwrap().as_hex(),
            ApiPrivateKey::parse(&prefixed).unwrap().as_hex()
        );
    }

    #[rstest]
    fn rejects_wrong_length_key() {
        assert!(matches!(
            ApiPrivateKey::parse("0xdeadbeef"),
            Err(CredentialError::KeyLength(8))
        ));
    }

    #[rstest]
    fn rejects_non_hex_key() {
        let bad = "z".repeat(64);
        assert!(matches!(
            ApiPrivateKey::parse(&bad),
            Err(CredentialError::KeyEncoding)
        ));
    }

    #[rstest]
    fn debug_output_hides_key_material() {
        let key = ApiPrivateKey::parse(&"2".repeat(64)).unwrap();
        let rendered = format!("{key:?}");
        assert!(rendered.contains(REDACTED), "{rendered}");
        assert!(!rendered.contains("22222"), "{rendered}");
    }

    #[rstest]
    fn rejects_reserved_and_malformed_key_names() {
        assert!(ApiKeyName::parse("default").is_err());
        assert!(ApiKeyName::parse("").is_err());
        assert!(ApiKeyName::parse(&"a".repeat(37)).is_err());
        assert!(ApiKeyName::parse("has space").is_err());
        assert!(ApiKeyName::parse("api-key_01").is_ok());
    }
}
