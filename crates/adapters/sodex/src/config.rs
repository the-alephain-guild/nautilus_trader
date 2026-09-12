//! Client configuration.
//!
//! # Spot and perps are separate venues
//!
//! The two engines differ in seven ways that no parameter can paper over: the EIP-712 domain
//! name, the REST batch path, the action names hashed into signatures, the API key set, the
//! order item shape, the account balances, and the reference price used for limit bounds
//! (`lastTradePrice` on spot, `markPrice` on perps).
//!
//! Modeling them as one venue would put a "which engine is this" branch at the head of
//! nearly every method in the execution client, with the branch condition carrying all seven
//! differences at once. They are therefore [`SODEX_SPOT`] and [`SODEX_PERPS`], and a client
//! binds to one engine at construction - the same discipline the HTTP client already
//! follows, which has already prevented signing a perps action under the spot domain.
//!
//! # Credentials
//!
//! Market data needs none: the venue serves it unsigned. Only the execution client takes
//! credentials, and only ever an API key - never the master wallet, which can authorize
//! withdrawals and belongs offline. Values fall back to the environment so a deployment can
//! keep secrets out of config files.

use nautilus_core::string::secret::SecretString;
use nautilus_model::identifiers::{InstrumentId, Venue};
use serde::{Deserialize, Serialize};

use crate::{common::Market, http::Network};

/// Registry key for this adapter's client factories.
///
/// One key covers both engines: a factory reads [`SodexDataClientConfig::market`] to decide
/// which venue the client it builds belongs to, so a node registers the factory once and
/// creates a spot and a perps client from it.
pub const SODEX: &str = "SODEX";

/// Venue identifier for the spot engine.
pub const SODEX_SPOT: &str = "SODEX_SPOT";

/// Venue identifier for the perpetuals engine.
pub const SODEX_PERPS: &str = "SODEX_PERPS";

/// Environment variable holding the API key's *name* (not its address).
pub const ENV_API_KEY_NAME: &str = "SODEX_API_KEY_NAME";

/// Environment variable holding the API key's private key.
pub const ENV_API_PRIVATE_KEY: &str = "SODEX_API_PRIVATE_KEY";

/// Environment variable holding the numeric account id.
///
/// One value serves both engines. `/accounts/{address}/state` answered the same `aid` on spot and
/// on perps for the same wallet, so there is no second id to hunt for - unlike the API keys, which
/// really are registered per engine. Worth stating because the key being per-engine invites the
/// assumption that the id is too, and that assumption sends you looking for something that does
/// not exist.
pub const ENV_ACCOUNT_ID: &str = "SODEX_ACCOUNT_ID";

/// Environment variable holding the account's wallet address.
pub const ENV_WALLET_ADDRESS: &str = "SODEX_WALLET_ADDRESS";

/// The venue for a given engine.
#[must_use]
pub fn venue_for(market: Market) -> Venue {
    match market {
        Market::Spot => Venue::from(SODEX_SPOT),
        Market::Perps => Venue::from(SODEX_PERPS),
    }
}

/// Errors raised while resolving configuration.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConfigError {
    #[error("missing {what}: set it in the config or the {env} environment variable")]
    Missing {
        what: &'static str,
        env: &'static str,
    },
    #[error("{env} is not a valid account id: {value:?}")]
    InvalidAccountId { env: &'static str, value: String },
    #[error(
        "{env} is not a wallet address: {value:?} - expected 0x followed by 40 hex digits. The \
         account reads are addressed by the master wallet, and a wrong address answers with an \
         empty account rather than an error"
    )]
    InvalidWalletAddress { env: &'static str, value: String },
}

/// Configuration for the market-data client.
///
/// Deliberately credential-free. Reading market data does not require a key, so a data-only
/// deployment holds no secret at all.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(module = "nautilus_trader.adapters.sodex", from_py_object)
)]
#[cfg_attr(
    feature = "python",
    pyo3_stub_gen::derive::gen_stub_pyclass(module = "nautilus_trader.adapters.sodex")
)]
pub struct SodexDataClientConfig {
    /// Which gateway to reach.
    pub network: Network,
    /// Which engine this client serves.
    pub market: Market,
    /// Restricts instrument loading to these ids. Empty means load everything.
    #[serde(default)]
    pub instrument_ids: Vec<InstrumentId>,
    /// How often to reload the venue's instrument listing, in minutes.
    ///
    /// A young venue lists and delists pairs while a session is running, and nothing pushes
    /// that: without a reload, a long-running process keeps trading against the listing it saw
    /// at startup and never learns of a new pair - or that one it holds a symbol id for has
    /// gone. `None` or `0` disables the reload, which is only appropriate for a short run.
    #[serde(default = "default_instrument_refresh_mins")]
    pub update_instruments_interval_mins: Option<u64>,
    /// HTTP timeout in seconds.
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
}

impl SodexDataClientConfig {
    /// A data client for one engine on one network.
    #[must_use]
    pub fn new(network: Network, market: Market) -> Self {
        Self {
            network,
            market,
            instrument_ids: Vec::new(),
            update_instruments_interval_mins: default_instrument_refresh_mins(),
            timeout_secs: default_timeout_secs(),
        }
    }

    /// The venue these instruments belong to.
    #[must_use]
    pub fn venue(&self) -> Venue {
        venue_for(self.market)
    }
}

/// Configuration for the execution client.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(module = "nautilus_trader.adapters.sodex", from_py_object)
)]
#[cfg_attr(
    feature = "python",
    pyo3_stub_gen::derive::gen_stub_pyclass(module = "nautilus_trader.adapters.sodex")
)]
pub struct SodexExecClientConfig {
    /// Which gateway to reach.
    pub network: Network,
    /// Which engine this client trades on.
    pub market: Market,
    /// Numeric account id. Falls back to [`ENV_ACCOUNT_ID`].
    #[serde(default)]
    pub account_id: Option<u64>,
    /// Name of the registered API key. Falls back to [`ENV_API_KEY_NAME`].
    ///
    /// This is the key's *name*, not its address - the venue documents that confusion as the
    /// most common integration error.
    #[serde(default)]
    pub api_key_name: Option<String>,
    /// Private key of the registered API key. Falls back to [`ENV_API_PRIVATE_KEY`].
    #[serde(default)]
    pub api_private_key: Option<SecretString>,
    /// The account's **master wallet** address. Falls back to [`ENV_WALLET_ADDRESS`].
    ///
    /// Public information, not a secret - it addresses the account's reads and signs nothing.
    /// Required all the same, and for an uncomfortable reason: the reads are keyed by this
    /// address, and pointing them at the API key's address instead does not fail. It answers
    /// `200` with an empty account, which reconciliation would read as "flat, nothing open".
    /// The execution client therefore proves at startup that this wallet lists the key it signs
    /// with, rather than trusting the value.
    #[serde(default)]
    pub wallet_address: Option<String>,
    /// How often to reload the venue's instrument listing, in minutes.
    ///
    /// A young venue lists and delists pairs while a session is running, and nothing pushes
    /// that: without a reload, a long-running process keeps trading against the listing it saw
    /// at startup and never learns of a new pair - or that one it holds a symbol id for has
    /// gone. `None` or `0` disables the reload, which is only appropriate for a short run.
    #[serde(default = "default_instrument_refresh_mins")]
    pub update_instruments_interval_mins: Option<u64>,
    /// HTTP timeout in seconds.
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
}

impl SodexExecClientConfig {
    /// An execution client config that resolves credentials from the environment.
    #[must_use]
    pub fn from_env(network: Network, market: Market) -> Self {
        Self {
            network,
            market,
            account_id: None,
            api_key_name: None,
            api_private_key: None,
            wallet_address: None,
            update_instruments_interval_mins: default_instrument_refresh_mins(),
            timeout_secs: default_timeout_secs(),
        }
    }

    /// The venue this client trades on.
    #[must_use]
    pub fn venue(&self) -> Venue {
        venue_for(self.market)
    }

    /// Resolves the account id from the config or the environment.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError`] when absent from both, or unparsable.
    pub fn resolve_account_id(&self) -> Result<u64, ConfigError> {
        if let Some(id) = self.account_id {
            return Ok(id);
        }
        let raw = env_value(ENV_ACCOUNT_ID).ok_or(ConfigError::Missing {
            what: "account id",
            env: ENV_ACCOUNT_ID,
        })?;
        raw.parse().map_err(|_| ConfigError::InvalidAccountId {
            env: ENV_ACCOUNT_ID,
            value: raw,
        })
    }

    /// Resolves the API key name from the config or the environment.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::Missing`] when absent from both.
    pub fn resolve_api_key_name(&self) -> Result<String, ConfigError> {
        self.api_key_name
            .clone()
            .or_else(|| env_value(ENV_API_KEY_NAME))
            .ok_or(ConfigError::Missing {
                what: "API key name",
                env: ENV_API_KEY_NAME,
            })
    }

    /// Resolves the API private key from the config or the environment.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::Missing`] when absent from both.
    pub fn resolve_api_private_key(&self) -> Result<SecretString, ConfigError> {
        if let Some(key) = &self.api_private_key {
            return Ok(key.clone());
        }
        env_value(ENV_API_PRIVATE_KEY)
            .map(SecretString::from)
            .ok_or(ConfigError::Missing {
                what: "API private key",
                env: ENV_API_PRIVATE_KEY,
            })
    }

    /// Resolves the account's wallet address from the config or the environment.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::Missing`] when absent from both, or
    /// [`ConfigError::InvalidWalletAddress`] when it is not an address. Checking the shape here
    /// rather than letting the venue answer matters: a malformed address does not produce an
    /// error from the venue, it produces an empty account.
    pub fn resolve_wallet_address(&self) -> Result<String, ConfigError> {
        let raw = self
            .wallet_address
            .clone()
            .or_else(|| env_value(ENV_WALLET_ADDRESS))
            .ok_or(ConfigError::Missing {
                what: "wallet address",
                env: ENV_WALLET_ADDRESS,
            })?;

        let hex = raw.strip_prefix("0x").or_else(|| raw.strip_prefix("0X"));
        match hex {
            Some(hex) if hex.len() == 40 && hex.chars().all(|c| c.is_ascii_hexdigit()) => Ok(raw),
            _ => Err(ConfigError::InvalidWalletAddress {
                env: ENV_WALLET_ADDRESS,
                value: raw,
            }),
        }
    }

    /// Whether every credential can be resolved.
    ///
    /// Useful for failing at startup rather than on the first order.
    #[must_use]
    pub fn has_credentials(&self) -> bool {
        self.resolve_account_id().is_ok()
            && self.resolve_api_key_name().is_ok()
            && self.resolve_api_private_key().is_ok()
            && self.resolve_wallet_address().is_ok()
    }
}

pub(crate) const fn default_timeout_secs() -> u64 {
    30
}

/// Hourly: frequent enough that a new listing becomes tradable within the session, rare enough
/// that it costs nothing against the request budget.
///
/// The wrap is not redundant: this is a serde default for an `Option<u64>` field, where `None`
/// means "no reload", so the default has to be expressible in the field's own type.
#[allow(clippy::unnecessary_wraps)]
pub(crate) const fn default_instrument_refresh_mins() -> Option<u64> {
    Some(60)
}

/// Reads an environment variable, treating blank values as absent.
///
/// A variable exported empty is a configuration mistake, not an intentional empty
/// credential, and letting it through produces a signature failure far from its cause.
fn env_value(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[rstest]
    fn each_engine_gets_its_own_venue() {
        // The whole point of the split: an instrument on one engine is not the same
        // instrument on the other, even when the symbols look alike.
        assert_eq!(venue_for(Market::Spot).as_str(), SODEX_SPOT);
        assert_eq!(venue_for(Market::Perps).as_str(), SODEX_PERPS);
        assert_ne!(venue_for(Market::Spot), venue_for(Market::Perps));
    }

    #[rstest]
    fn data_config_carries_no_credential_fields() {
        // Market data is served unsigned, so a data-only deployment should hold no secret.
        // This is a structural assertion: the serialized form must not grow a key field.
        let config = SodexDataClientConfig::new(Network::Testnet, Market::Spot);
        let json = serde_json::to_string(&config).unwrap();

        assert!(!json.contains("key"), "{json}");
        assert!(!json.contains("secret"), "{json}");
    }

    #[rstest]
    fn explicit_config_wins_over_the_environment() {
        let config = SodexExecClientConfig {
            account_id: Some(4242),
            api_key_name: Some("explicit".to_string()),
            ..SodexExecClientConfig::from_env(Network::Testnet, Market::Perps)
        };

        assert_eq!(config.resolve_account_id().unwrap(), 4242);
        assert_eq!(config.resolve_api_key_name().unwrap(), "explicit");
    }

    #[rstest]
    fn missing_credentials_name_the_variable_to_set() {
        // The error has to say which variable, or the operator is left guessing.
        let config = SodexExecClientConfig {
            account_id: None,
            ..SodexExecClientConfig::from_env(Network::Testnet, Market::Perps)
        };

        // Only assert the shape when the variable is genuinely unset in this environment.
        if env_value(ENV_ACCOUNT_ID).is_none() {
            let err = config.resolve_account_id().unwrap_err();
            assert_eq!(
                err,
                ConfigError::Missing {
                    what: "account id",
                    env: ENV_ACCOUNT_ID
                }
            );
            assert!(err.to_string().contains(ENV_ACCOUNT_ID));
        }
    }

    #[rstest]
    fn blank_environment_values_count_as_absent() {
        // `export SODEX_ACCOUNT_ID=` is a mistake, not an intentional empty value. Letting
        // it through would surface as a parse or signing failure far from the cause.
        // SAFETY: single-threaded test, variable is restored immediately.
        unsafe {
            std::env::set_var("SODEX_TEST_BLANK", "   ");
        }
        assert_eq!(env_value("SODEX_TEST_BLANK"), None);
        unsafe {
            std::env::remove_var("SODEX_TEST_BLANK");
        }
    }

    #[rstest]
    fn config_round_trips_through_serde() {
        let config = SodexExecClientConfig {
            account_id: Some(60366),
            api_key_name: Some("api-key-01".to_string()),
            ..SodexExecClientConfig::from_env(Network::Mainnet, Market::Spot)
        };

        let json = serde_json::to_string(&config).unwrap();
        let restored: SodexExecClientConfig = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.account_id, Some(60366));
        assert_eq!(restored.venue(), venue_for(Market::Spot));
    }
}
