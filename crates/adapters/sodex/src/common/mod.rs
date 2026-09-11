//! Shared types and constants for the SoDEX adapter.

pub mod credential;
pub mod decimal;
pub mod enums;

/// ValueChain mainnet, used as `message.chainID` and as the trading-action EIP-712 `chainId`.
pub const CHAIN_ID_MAINNET: u64 = 286623;

/// ValueChain testnet.
pub const CHAIN_ID_TESTNET: u64 = 138565;

/// Which orderbook an action targets. Also selects the EIP-712 domain name, which is why
/// this is not merely cosmetic: signing a perps action under the `spot` domain produces a
/// signature the gateway will reject.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(
        // Prefixed for the Python surface: this module is imported with `*`, and a bare
        // `Market` would shadow whatever else a strategy has by that name.
        name = "SodexMarket",
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
pub enum Market {
    /// Spot, the default: it holds no leverage and has no liquidation, so a configuration
    /// that forgets to state a market cannot silently open a leveraged position.
    #[default]
    Spot,
    Perps,
}

impl Market {
    /// The EIP-712 `domain.name` for this market.
    ///
    /// Note the asymmetry with the REST path segment: perps actions sign under `futures`,
    /// not `perps`.
    #[must_use]
    pub const fn domain_name(self) -> &'static str {
        match self {
            Self::Spot => "spot",
            Self::Perps => "futures",
        }
    }

    /// The REST path segment for this market (`/api/v1/{segment}`).
    #[must_use]
    pub const fn path_segment(self) -> &'static str {
        match self {
            Self::Spot => "spot",
            Self::Perps => "perps",
        }
    }
}

/// Leading byte identifying the signature family. The gateway rejects raw 65-byte
/// signatures that carry no prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignatureKind {
    /// Trading actions signed under the `spot`/`futures` domain.
    Exchange,
    /// Account-level actions (`addAPIKey`, `revokeAPIKey`, `approveBuilderFee`)
    /// signed under the `universal` domain by the master wallet.
    Universal,
}

impl SignatureKind {
    #[must_use]
    pub const fn prefix(self) -> u8 {
        match self {
            Self::Exchange => 0x01,
            Self::Universal => 0x02,
        }
    }
}
