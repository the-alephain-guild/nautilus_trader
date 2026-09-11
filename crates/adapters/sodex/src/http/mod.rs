//! REST transport for SoDEX.
//!
//! # Endpoint layout
//!
//! Three base paths per network, all under the same gateway host:
//!
//! | Scope  | Path              | Auth     |
//! |--------|-------------------|----------|
//! | Public | `/api/v1`         | none     |
//! | Spot   | `/api/v1/spot`    | signed   |
//! | Perps  | `/api/v1/perps`   | signed   |
//!
//! Market-data reads are unsigned; writes carry `X-API-Key`, `X-API-Sign` and `X-API-Nonce`.
//! Omitting `X-API-Key` tells the venue to verify against the master wallet instead of a
//! registered key — this adapter does not use that path, since it would require the master
//! key in a trading process.

pub mod account;
pub mod account_reads;
pub mod client;
pub mod models;
pub mod orders;
pub mod ratelimit;
pub mod requests;
pub mod spot;

pub use account::{AccountClient, AddApiKeyRequest, generate_api_key};
pub use account_reads::{
    ApiKeyEntry, BalancesSnapshot, CoinBalance, OpenOrders, OrderRecord, Positions,
};
pub use client::{ClientError, SignedRequest, SodexHttpClient};
pub use models::{ApiResponse, EnvelopeError};
pub use orders::{AlignError, OrderAck, align_batch};
pub use ratelimit::{Axis, BatchCost, RateLimited, WeightBudget};
pub use requests::{CancelOrderRequest, ClientOrderId, NewOrderRequest, OrderItem, RequestError};

use crate::common::Market;

/// Gateway host for the two networks.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(
        // Prefixed for the same reason as `SodexMarket`: `Network` is far too generic a name
        // to put into a strategy's namespace through a star import.
        name = "SodexNetwork",
        module = "nautilus_trader.adapters.sodex",
        eq,
        eq_int,
        frozen,
        from_py_object,
        rename_all = "SCREAMING_SNAKE_CASE"
    )
)]
#[cfg_attr(
    feature = "python",
    pyo3_stub_gen::derive::gen_stub_pyclass_enum(module = "nautilus_trader.adapters.sodex")
)]
pub enum Network {
    Mainnet,
    /// Testnet, the default: a configuration that forgets to state a network must not end up
    /// signing against mainnet with real funds.
    #[default]
    Testnet,
}

impl Network {
    /// Base URL up to and including `/api/v1`, without a trailing slash.
    #[must_use]
    pub const fn public_base(self) -> &'static str {
        match self {
            Self::Mainnet => "https://mainnet-gw.sodex.dev/api/v1",
            Self::Testnet => "https://testnet-gw.sodex.dev/api/v1",
        }
    }

    /// WebSocket base, without a trailing slash.
    #[must_use]
    pub const fn ws_base(self) -> &'static str {
        match self {
            Self::Mainnet => "wss://mainnet-gw.sodex.dev/ws",
            Self::Testnet => "wss://testnet-gw.sodex.dev/ws",
        }
    }

    /// The chain id this network signs under.
    ///
    /// Pairing the two here keeps a testnet URL from being signed with the mainnet chain id,
    /// which would fail verification with no useful diagnostic.
    #[must_use]
    pub const fn chain_id(self) -> u64 {
        match self {
            Self::Mainnet => crate::common::CHAIN_ID_MAINNET,
            Self::Testnet => crate::common::CHAIN_ID_TESTNET,
        }
    }

    /// Base URL for one market's REST endpoints.
    #[must_use]
    pub fn market_base(self, market: Market) -> String {
        format!("{}/{}", self.public_base(), market.path_segment())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::{CHAIN_ID_MAINNET, CHAIN_ID_TESTNET};

    #[test]
    fn market_base_uses_the_rest_path_segment_not_the_domain_name() {
        // perps signs under domain "futures" but is reached at "/perps"; conflating the two
        // yields a 404 that looks like a routing bug rather than a naming one.
        assert_eq!(
            Network::Testnet.market_base(Market::Perps),
            "https://testnet-gw.sodex.dev/api/v1/perps"
        );
        assert_eq!(Market::Perps.domain_name(), "futures");
    }

    #[test]
    fn network_binds_url_and_chain_id_together() {
        assert_eq!(Network::Mainnet.chain_id(), CHAIN_ID_MAINNET);
        assert_eq!(Network::Testnet.chain_id(), CHAIN_ID_TESTNET);
        assert!(Network::Testnet.public_base().contains("testnet"));
        assert!(Network::Mainnet.public_base().contains("mainnet"));
    }

    #[test]
    fn spot_and_perps_differ_only_in_the_trailing_segment() {
        let spot = Network::Mainnet.market_base(Market::Spot);
        let perps = Network::Mainnet.market_base(Market::Perps);

        assert!(spot.ends_with("/spot"));
        assert!(perps.ends_with("/perps"));
        assert_eq!(
            spot.trim_end_matches("/spot"),
            perps.trim_end_matches("/perps")
        );
    }
}
