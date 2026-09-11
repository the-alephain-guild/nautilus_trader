//! Account, order, fill and position reads.
//!
//! These four endpoints are what makes unattended running possible: Nautilus reconciles through
//! request/response reports, so without them the engine cannot learn about an order it did not
//! place, a fill that happened while it was disconnected, or what the account actually holds.
//!
//! # Addressed by the master wallet, and nothing else will do
//!
//! The path segment is the **account's wallet address**, not the numeric account id and not the
//! API key's address. The other two do not fail; they are worse than that:
//!
//! - the numeric id answers `invalid parameter: userAddress`, which at least is an error;
//! - **the API key's own address answers `200` with an empty account** —
//!   `{"blockTime":0,"blockHeight":0,"balances":[]}`.
//!
//! That second case is the dangerous one. A client pointed at the wrong address would read
//! "no balance, no open orders, no positions" and reconciliation would take that as a flat
//! account, with nothing anywhere reporting a problem. A new account is legitimately empty too,
//! so emptiness cannot be treated as the error — which is why the execution client instead
//! proves the configured wallet is the right one by checking that it lists the key the client
//! signs with. See [`ApiKeyEntry`].
//!
//! # Unsigned
//!
//! All four are plain `GET`s with no signature. The venue's own documentation draws the line at
//! actions: address limits "apply to actions only (never to queries)". So a data-only or
//! read-only deployment needs no credential to reconcile — only the wallet address, which is
//! public information.

use serde::Deserialize;

use super::{ClientError, SodexHttpClient};
use crate::common::enums::{OrderSide, OrderStatus, OrderType, TimeInForce};

/// One coin balance.
///
/// `total` includes `locked`, so the free amount is the difference. Both are decimal strings at
/// the coin's own precision, which can be 18 places.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct CoinBalance {
    /// The venue's numeric coin id.
    pub id: u64,
    pub coin: String,
    /// Everything held, including what is locked behind open orders.
    pub total: String,
    /// Reserved against open orders, and therefore not available to trade.
    pub locked: String,
}

/// The balances response, with the chain position it was read at.
///
/// `block_height` matters for reconciliation: two reads at the same height describe the same
/// state, and a read at a lower height than one already seen is stale rather than a change.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct BalancesSnapshot {
    #[serde(rename = "blockTime")]
    pub block_time_ms: u64,
    #[serde(rename = "blockHeight")]
    pub block_height: u64,
    pub balances: Vec<CoinBalance>,
}

/// An order as the venue reports it, on the open list and in history alike.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct OrderRecord {
    pub symbol: String,
    #[serde(rename = "orderID")]
    pub order_id: u64,
    #[serde(rename = "clOrdID")]
    pub cl_ord_id: String,
    /// Typed at the boundary rather than carried as text, so every consumer is spared the
    /// question of what `"CANCELED"` means and no call site can spell it a second way.
    pub side: OrderSide,
    #[serde(rename = "type")]
    pub order_type: OrderType,
    #[serde(rename = "timeInForce")]
    pub time_in_force: TimeInForce,
    pub price: String,
    #[serde(rename = "origQty")]
    pub orig_qty: String,
    pub status: OrderStatus,
    #[serde(rename = "executedQty")]
    pub executed_qty: String,
    /// Quote-asset value filled so far, from which an average fill price is derived.
    #[serde(rename = "executedValue")]
    pub executed_value: String,
    #[serde(rename = "marginFrozen")]
    pub margin_frozen: Option<String>,
    #[serde(rename = "createdAt")]
    pub created_at_ms: u64,
    #[serde(rename = "updatedAt")]
    pub updated_at_ms: u64,
}

/// The open-orders response.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct OpenOrders {
    #[serde(rename = "blockTime")]
    pub block_time_ms: u64,
    #[serde(rename = "blockHeight")]
    pub block_height: u64,
    pub orders: Vec<OrderRecord>,
}

/// The positions response, perps only.
///
/// Spot does not serve this path at all, which is correct rather than a gap: spot holds
/// balances and has no positions to report.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Positions {
    #[serde(rename = "blockTime")]
    pub block_time_ms: u64,
    #[serde(rename = "blockHeight")]
    pub block_height: u64,
    /// Left untyped on purpose.
    ///
    /// The shape has not been observed: reading it requires an open perps position, and the
    /// testnet account holds no perps balance to open one with. Typing it from the spot order
    /// shape by analogy is exactly the move that has already cost this integration a day — see
    /// the adapter's record of contract details that only a live link revealed. Callers get the
    /// raw value and the knowledge that it is unverified.
    pub positions: Vec<serde_json::Value>,
}

/// One fill, as the venue reports it.
///
/// # The fee is charged in the asset you receive
///
/// `fee_coin` is not the quote currency: a buy pays its fee in the **base** asset, deducted from
/// what arrives. Observed on testnet — a market buy of `0.001` vBTC credited `0.00099935`, and the
/// difference is exactly the reported `0.00000065` fee. A sell is expected to pay in the quote
/// asset by the same rule, but that has not been observed, so `fee_coin` is read from the response
/// rather than derived from the side.
///
/// This matters beyond bookkeeping: **the proceeds of a buy are smaller than the quantity
/// ordered**, so selling back the amount you asked for is rejected for insufficient balance. Any
/// flattening logic has to sell what was received.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct TradeRecord {
    #[serde(rename = "tradeID")]
    pub trade_id: u64,
    #[serde(rename = "orderID")]
    pub order_id: u64,
    #[serde(rename = "clOrdID")]
    pub cl_ord_id: String,
    pub symbol: String,
    pub side: OrderSide,
    pub price: String,
    pub quantity: String,
    /// Fee amount, denominated in [`Self::fee_coin`] rather than in the quote asset.
    pub fee: String,
    /// Which asset the fee was taken from.
    #[serde(rename = "feeCoin")]
    pub fee_coin: String,
    /// Whether this side provided liquidity.
    ///
    /// Reported, which is why a fill report carries a real liquidity side while a fill *inferred*
    /// from an order record cannot.
    #[serde(rename = "isMaker")]
    pub is_maker: bool,
    /// Fill time, milliseconds.
    pub time: u64,
}

/// One registered API key, as the venue lists it.
///
/// Used to prove a configured wallet address is the account the client actually signs for: if
/// the list does not contain the signer's address, the client is pointed at the wrong account —
/// or the key was registered on the other engine, which is its own documented trap.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ApiKeyEntry {
    pub name: String,
    #[serde(rename = "type")]
    pub key_type: String,
    /// The key's address, lowercase hex.
    #[serde(rename = "publicKey")]
    pub public_key: String,
    /// Unix milliseconds, or `0` for a key that does not expire.
    #[serde(rename = "expiresAt")]
    pub expires_at_ms: u64,
}

impl SodexHttpClient {
    /// Reads the account's coin balances.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on transport, status or decoding failure.
    pub async fn account_balances(&self, wallet: &str) -> Result<BalancesSnapshot, ClientError> {
        self.get_public(&format!("/accounts/{wallet}/balances"), None)
            .await
    }

    /// Reads the account's currently open orders.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on transport, status or decoding failure.
    pub async fn open_orders(&self, wallet: &str) -> Result<OpenOrders, ClientError> {
        self.get_public(&format!("/accounts/{wallet}/orders"), None)
            .await
    }

    /// Reads the account's order history, which is where a terminal order ends up.
    ///
    /// An order that has left the open list is only visible here, so reconciling a single order
    /// means consulting both.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on transport, status or decoding failure.
    pub async fn order_history(&self, wallet: &str) -> Result<Vec<OrderRecord>, ClientError> {
        self.get_public(&format!("/accounts/{wallet}/orders/history"), None)
            .await
    }

    /// Reads the account's fills.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on transport, status or decoding failure.
    pub async fn account_trades(&self, wallet: &str) -> Result<Vec<TradeRecord>, ClientError> {
        self.get_public(&format!("/accounts/{wallet}/trades"), None)
            .await
    }

    /// Reads the account's open positions. Perps only; spot does not serve the path.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on transport, status or decoding failure.
    pub async fn account_positions(&self, wallet: &str) -> Result<Positions, ClientError> {
        self.get_public(&format!("/accounts/{wallet}/positions"), None)
            .await
    }

    /// Lists the API keys registered for a wallet on this engine.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on transport, status or decoding failure.
    pub async fn api_keys(&self, wallet: &str) -> Result<Vec<ApiKeyEntry>, ClientError> {
        self.get_public(&format!("/accounts/{wallet}/api-keys"), None)
            .await
    }
}
