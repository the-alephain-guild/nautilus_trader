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
//! - **the API key's own address answers `200` with an empty account** -
//!   `{"blockTime":0,"blockHeight":0,"balances":[]}`.
//!
//! That second case is the dangerous one. A client pointed at the wrong address would read
//! "no balance, no open orders, no positions" and reconciliation would take that as a flat
//! account, with nothing anywhere reporting a problem. A new account is legitimately empty too,
//! so emptiness cannot be treated as the error - which is why the execution client instead
//! proves the configured wallet is the right one by checking that it lists the key the client
//! signs with. See [`ApiKeyEntry`].
//!
//! # Unsigned
//!
//! All four are plain `GET`s with no signature. The venue's own documentation draws the line at
//! actions: address limits "apply to actions only (never to queries)". So a data-only or
//! read-only deployment needs no credential to reconcile - only the wallet address, which is
//! public information.

use nautilus_network::http::Method;
use serde::Deserialize;

use super::{
    ClientError, SodexHttpClient,
    requests::{
        CancelTwapOrderRequest, NewTwapOrderRequest, TransferAssetRequest, UpdateLeverageRequest,
        UpdateMarginRequest,
    },
};
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
    /// Everything held, including whatever is withheld from trading.
    pub total: String,
    /// Spot only: reserved against open orders.
    ///
    /// Optional because the two engines do not share a balance shape. Spot answers
    /// `{id, coin, total, locked}`; perps answers `{id, coin, total, collateral, marginRatio,
    /// price}` with no `locked` field at all, which made a required field here fail the whole
    /// perps read with `missing field 'locked'` - and that read is the first thing the execution
    /// client does, so the perps engine could not connect.
    ///
    /// The two names are kept apart rather than folded into one "withheld" field: the venue
    /// distinguishes them because they are different mechanisms, and collapsing them would assert
    /// an equivalence this crate has not established.
    pub locked: Option<String>,
    /// Perps only: posted as margin against open positions.
    pub collateral: Option<String>,
}

impl CoinBalance {
    /// The portion not available to trade, whichever engine reported it.
    ///
    /// # Errors
    ///
    /// Returns an error when neither field is present. Treating that as zero would report the
    /// whole balance as free and overstate buying power, which is the wrong way to fail.
    pub fn withheld(&self) -> anyhow::Result<&str> {
        self.locked
            .as_deref()
            .or(self.collateral.as_deref())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "balance for {} carries neither `locked` nor `collateral`",
                    self.coin
                )
            })
    }
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

/// One open position, as the perps engine reports it.
///
/// # Direction lives in the sign of `size`, not in `position_side`
///
/// `position_side` read `BOTH` on both a long and a short, which is what one-way mode reports and
/// says nothing about direction. What differed was the sign: a long answered `"0.0002"` and a short
/// `"-0.0002"`. Both were observed on testnet rather than inferred from each other, because
/// Nautilus needs a signed quantity and getting the sign backwards would report every short as a
/// long.
///
/// # This response is not a superset of `/accounts/{wallet}/state`
///
/// The same positions appear in `state` under `P` with abbreviated keys, and the two carry
/// different fields rather than the same data twice. `state` adds unrealized P&L (`ur`) and the
/// liquidation price (`lp`, non-zero only where liquidation is reachable - it read `0` on the long
/// and `570018.06` on the short). This response adds `active`, `is_taken_over` and
/// `take_over_price`. Neither route alone carries everything.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PositionRecord {
    /// The venue's own position id.
    pub id: u64,
    pub symbol: String,
    /// Signed: positive is long, negative is short.
    pub size: String,
    #[serde(rename = "avgEntryPrice")]
    pub avg_entry_price: String,
    #[serde(rename = "avgClosePrice")]
    pub avg_close_price: String,
    /// Always `BOTH` on the observed runs; direction is carried by the sign of `size`.
    #[serde(rename = "positionSide")]
    pub position_side: String,
    pub leverage: u32,
    #[serde(rename = "marginMode")]
    pub margin_mode: String,
    #[serde(rename = "initialMargin")]
    pub initial_margin: String,
    #[serde(rename = "maxSize")]
    pub max_size: String,
    #[serde(rename = "cumOpenCost")]
    pub cum_open_cost: String,
    #[serde(rename = "cumClosedSize")]
    pub cum_closed_size: String,
    #[serde(rename = "cumTradingFee")]
    pub cum_trading_fee: String,
    #[serde(rename = "realizedPnL")]
    pub realized_pnl: String,
    /// Whether the venue still considers the position open.
    ///
    /// Observed `true` on every entry the list returned; a closed position disappeared from the
    /// list entirely rather than appearing with `false`. Read rather than assumed, so an inactive
    /// entry can be skipped instead of reported as something the account still holds.
    pub active: bool,
    #[serde(rename = "isTakenOver")]
    pub is_taken_over: bool,
    #[serde(rename = "takeOverPrice")]
    pub take_over_price: String,
    #[serde(rename = "createdAt")]
    pub created_at_ms: u64,
    #[serde(rename = "updatedAt")]
    pub updated_at_ms: u64,
}

/// The positions response, perps only.
///
/// Spot does not serve this path at all, which is correct rather than a gap: spot holds
/// balances and has no positions to report.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Positions {
    #[serde(rename = "blockTime")]
    pub block_time_ms: u64,
    #[serde(rename = "blockHeight")]
    pub block_height: u64,
    /// Empty as `[]` here. Note that `state` reports the same emptiness as `"P": null`, so a
    /// consumer reading that route instead has to treat null as empty rather than as absent.
    pub positions: Vec<PositionRecord>,
}

/// One fill, as the venue reports it.
///
/// # The fee is charged in the asset you receive
///
/// `fee_coin` is not the quote currency: a buy pays its fee in the **base** asset, deducted from
/// what arrives. Observed on testnet - a market buy of `0.001` vBTC credited `0.00099935`, and the
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
/// the list does not contain the signer's address, the client is pointed at the wrong account -
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
}

/// The account's own fee rates, which need not be the instrument's defaults.
///
/// Measured 2026-09-14: at tier 0 they matched the instrument exactly - perps `0.00012`/`0.0004`,
/// spot `0.00035`/`0.00065`. The three tier fields are why this endpoint exists anyway: an account
/// that trades volume, stakes, or earns a maker rebate stops matching, and a commission computed
/// from the instrument's default would then be wrong in the direction that compounds.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct FeeRates {
    #[serde(rename = "makerFeeRate")]
    pub maker: String,
    #[serde(rename = "takerFeeRate")]
    pub taker: String,
    #[serde(rename = "feeTier")]
    pub fee_tier: u32,
    #[serde(rename = "stakingTier")]
    pub staking_tier: u32,
    #[serde(rename = "makerRebateTier")]
    pub maker_rebate_tier: u32,
}

impl SodexHttpClient {
    /// Reads the account's maker and taker fee rates.
    ///
    /// Unsigned, like the other account reads.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on transport, status or decoding failure.
    pub async fn fee_rate(&self, wallet: &str) -> Result<FeeRates, ClientError> {
        self.get_public(&format!("/accounts/{wallet}/fee-rate"), None)
            .await
    }
}

impl SodexHttpClient {
    /// Moves assets between accounts, including between this account's two engines.
    ///
    /// **This moves funds.** The permission mask that would withhold it does not bind at this
    /// venue, so an API key registered without one carries this authority whether or not anyone
    /// meant it to.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on signing, transport or venue rejection.
    pub async fn transfer_asset(&self, request: &TransferAssetRequest) -> Result<(), ClientError> {
        let signed = self.build_signed(
            Method::POST,
            TransferAssetRequest::ENDPOINT,
            TransferAssetRequest::ACTION,
            request,
        )?;

        let _: Option<serde_json::Value> = self.send_optional(signed).await?;
        Ok(())
    }

    /// Places a TWAP order, which the venue slices over the given minutes itself.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on signing, transport or venue rejection.
    pub async fn new_twap_order(
        &self,
        request: &NewTwapOrderRequest,
    ) -> Result<Option<serde_json::Value>, ClientError> {
        let signed = self.build_signed(
            Method::POST,
            NewTwapOrderRequest::ENDPOINT,
            NewTwapOrderRequest::ACTION,
            request,
        )?;

        // Returned rather than discarded, unlike the leverage and margin actions: a TWAP has an
        // id that must be known to cancel it, and nothing has observed where the venue puts it.
        // Whoever runs this first should record the shape.
        self.send_optional(signed).await
    }

    /// Cancels a running TWAP order.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on signing, transport or venue rejection.
    pub async fn cancel_twap_order(
        &self,
        request: &CancelTwapOrderRequest,
    ) -> Result<(), ClientError> {
        let signed = self.build_signed(
            Method::DELETE,
            CancelTwapOrderRequest::ENDPOINT,
            CancelTwapOrderRequest::ACTION,
            request,
        )?;

        let _: Option<serde_json::Value> = self.send_optional(signed).await?;
        Ok(())
    }

    /// Sets leverage and margin mode for one perps instrument.
    ///
    /// Signed with the trading domain, like an order - not the universal domain the API key actions
    /// use. The route answers `404` on spot, so calling this on a spot client is a configuration
    /// error rather than a venue refusal.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on transport, status or decoding failure. The venue may also refuse
    /// the change itself - reducing leverage under an open position, for one - and that arrives as
    /// a status or envelope error rather than as a local validation failure.
    pub async fn update_leverage(
        &self,
        request: &UpdateLeverageRequest,
    ) -> Result<(), ClientError> {
        let signed = self.build_signed(
            Method::POST,
            UpdateLeverageRequest::ENDPOINT,
            UpdateLeverageRequest::ACTION,
            request,
        )?;

        // Discarded rather than typed: the response shape is unobserved, and these two actions
        // answer the question they were asked by succeeding. A later reader who sees a payload worth
        // having should type it then, with the payload in hand.
        let _: Option<serde_json::Value> = self.send_optional(signed).await?;
        Ok(())
    }

    /// Moves margin against one isolated perps position.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on transport, status or decoding failure.
    pub async fn update_margin(&self, request: &UpdateMarginRequest) -> Result<(), ClientError> {
        let signed = self.build_signed(
            Method::POST,
            UpdateMarginRequest::ENDPOINT,
            UpdateMarginRequest::ACTION,
            request,
        )?;

        let _: Option<serde_json::Value> = self.send_optional(signed).await?;
        Ok(())
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
