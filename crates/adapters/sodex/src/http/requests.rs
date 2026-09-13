//! Signed trading request payloads.
//!
//! Field order in these structs is part of the wire contract, not a style choice: the
//! signature commits to `keccak256` over the compact JSON, and the gateway re-marshals the
//! body through its own Go structs to verify. Reordering a field silently invalidates every
//! signature. The declaration order mirrors the venue's schema tables exactly.
//!
//! Constructors enforce the venue's placement rules up front - market orders must be IOC,
//! `funds` is market-buy only, a cancel names an order one way or the other - so an invalid
//! combination fails locally instead of costing a round trip and a rejection.

use serde::Serialize;

use crate::common::enums::{
    MarginMode, OrderModifier, OrderSide, OrderType, PositionSide, StopType, TimeInForce,
    TriggerType,
};

/// Largest batch the venue accepts for orders, cancels and replaces.
pub const MAX_BATCH: usize = 100;

/// Errors raised while building a request.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RequestError {
    #[error("client order id must match ^[0-9a-zA-Z_-]{{1,36}}$, received {0:?}")]
    ClientOrderId(String),
    #[error("batch must contain between 1 and {MAX_BATCH} items, received {0}")]
    BatchSize(usize),
    #[error("market orders must use IOC time in force, received {0}")]
    MarketTimeInForce(TimeInForce),
    #[error("{0} is not accepted by the venue for order placement")]
    Unsupported(&'static str),
    #[error("funds is only valid for market buy orders")]
    FundsOnMarketBuyOnly,
    #[error("a cancel must name the order by exactly one of order id or client order id")]
    CancelIdentification,
    #[error("a modify must name the order by its order id or its client order id")]
    UnidentifiedOrder,
    #[error("a modify must change at least one of price, quantity or stop price")]
    NothingToModify,
    #[error("leverage must be at least 1, received {0}")]
    LeverageOutOfRange(u32),
    #[error("a margin change of zero would move nothing")]
    ZeroMargin,
    #[error("margin amount {value:?} could not be read: {reason}")]
    InvalidMargin { value: String, reason: String },
    #[error(
        "{field} {value:?} carries a trailing zero, which the venue refuses as invalid; \
         pass it through `common::decimal::for_wire`"
    )]
    TrailingZero { field: &'static str, value: String },
}

/// Whether a decimal string has a fractional part ending in zero.
///
/// Integers are left alone: `"70000"` is what the venue wants, while `"0.00020"` is not.
fn has_trailing_zero(value: &str) -> bool {
    value
        .split_once('.')
        .is_some_and(|(_, fraction)| fraction.ends_with('0'))
}

/// A client-assigned order identifier.
///
/// Validated on construction because the venue rejects the whole batch on a malformed id,
/// and the constraint (`^[0-9a-zA-Z_-]{1,36}$`) is easy to violate with a UUID's hyphens
/// stripped or a symbol embedded verbatim.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct ClientOrderId(String);

impl ClientOrderId {
    /// # Errors
    ///
    /// Returns [`RequestError::ClientOrderId`] if the value is empty, longer than 36
    /// characters, or contains anything outside `[0-9a-zA-Z_-]`.
    pub fn parse(raw: impl Into<String>) -> Result<Self, RequestError> {
        let raw = raw.into();
        let valid_len = (1..=36).contains(&raw.len());
        let valid_chars = raw
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');

        if valid_len && valid_chars {
            Ok(Self(raw))
        } else {
            Err(RequestError::ClientOrderId(raw))
        }
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Builder fee attached to a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct BuilderParams {
    /// Builder account id. Must be non-zero.
    pub id: u64,
    /// Fee rate in tenths of a basis point: `10` is 1 bp.
    pub fee: u64,
}

/// One order in a batch.
///
/// Prefer the constructors over building this literally - they encode the venue's rules
/// about which fields may appear together.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OrderItem {
    #[serde(rename = "clOrdID")]
    pub cl_ord_id: ClientOrderId,
    pub modifier: OrderModifier,
    pub side: OrderSide,
    #[serde(rename = "type")]
    pub order_type: OrderType,
    #[serde(rename = "timeInForce")]
    pub time_in_force: TimeInForce,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub price: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<String>,
    /// Quote-denominated size. Market buy orders only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub funds: Option<String>,
    #[serde(rename = "stopPrice", skip_serializing_if = "Option::is_none")]
    pub stop_price: Option<String>,
    #[serde(rename = "stopType", skip_serializing_if = "Option::is_none")]
    pub stop_type: Option<StopType>,
    #[serde(rename = "triggerType", skip_serializing_if = "Option::is_none")]
    pub trigger_type: Option<TriggerType>,
    #[serde(rename = "reduceOnly")]
    pub reduce_only: bool,
    #[serde(rename = "positionSide")]
    pub position_side: PositionSide,
}

impl OrderItem {
    /// A limit order.
    ///
    /// # Errors
    ///
    /// Returns [`RequestError::Unsupported`] for a time in force the venue does not accept.
    pub fn limit(
        cl_ord_id: ClientOrderId,
        side: OrderSide,
        time_in_force: TimeInForce,
        price: impl Into<String>,
        quantity: impl Into<String>,
    ) -> Result<Self, RequestError> {
        if !time_in_force.is_supported_for_placement() {
            return Err(RequestError::Unsupported("FOK time in force"));
        }
        Ok(Self {
            cl_ord_id,
            modifier: OrderModifier::Normal,
            side,
            order_type: OrderType::Limit,
            time_in_force,
            price: Some(price.into()),
            quantity: Some(quantity.into()),
            funds: None,
            stop_price: None,
            stop_type: None,
            trigger_type: None,
            reduce_only: false,
            position_side: PositionSide::Both,
        })
    }

    /// A market order sized in the base asset.
    ///
    /// Time in force is fixed to IOC because the venue requires it; exposing it as a
    /// parameter would only allow constructing something that gets rejected.
    #[must_use]
    pub fn market(cl_ord_id: ClientOrderId, side: OrderSide, quantity: impl Into<String>) -> Self {
        Self {
            cl_ord_id,
            modifier: OrderModifier::Normal,
            side,
            order_type: OrderType::Market,
            time_in_force: TimeInForce::Ioc,
            price: None,
            quantity: Some(quantity.into()),
            funds: None,
            stop_price: None,
            stop_type: None,
            trigger_type: None,
            reduce_only: false,
            position_side: PositionSide::Both,
        }
    }

    /// A market buy sized in the quote asset.
    ///
    /// Separate from [`OrderItem::market`] because `funds` is buy-only and mutually
    /// exclusive with `quantity`; one constructor taking both would have to reject half its
    /// own argument space.
    #[must_use]
    pub fn market_buy_with_funds(cl_ord_id: ClientOrderId, funds: impl Into<String>) -> Self {
        Self {
            cl_ord_id,
            modifier: OrderModifier::Normal,
            side: OrderSide::Buy,
            order_type: OrderType::Market,
            time_in_force: TimeInForce::Ioc,
            price: None,
            quantity: None,
            funds: Some(funds.into()),
            stop_price: None,
            stop_type: None,
            trigger_type: None,
            reduce_only: false,
            position_side: PositionSide::Both,
        }
    }

    /// Marks the order reduce-only.
    #[must_use]
    pub fn reduce_only(mut self) -> Self {
        self.reduce_only = true;
        self
    }

    /// Adds a price bound to a market order for slippage protection.
    #[must_use]
    pub fn with_price_bound(mut self, price: impl Into<String>) -> Self {
        self.price = Some(price.into());
        self
    }

    /// Checks the combination against the venue's documented placement rules.
    ///
    /// # Errors
    ///
    /// Returns [`RequestError`] describing the first rule violated.
    pub fn validate(&self) -> Result<(), RequestError> {
        if !self.time_in_force.is_supported_for_placement() {
            return Err(RequestError::Unsupported("FOK time in force"));
        }
        if !self.position_side.is_supported_for_placement() {
            return Err(RequestError::Unsupported("hedge-mode position side"));
        }
        if self
            .trigger_type
            .is_some_and(|t| !t.is_supported_for_placement())
        {
            return Err(RequestError::Unsupported("last/index price trigger"));
        }
        if self.order_type == OrderType::Market && self.time_in_force != TimeInForce::Ioc {
            return Err(RequestError::MarketTimeInForce(self.time_in_force));
        }
        if self.funds.is_some()
            && !(self.order_type == OrderType::Market && self.side == OrderSide::Buy)
        {
            return Err(RequestError::FundsOnMarketBuyOnly);
        }

        // Checked here rather than left to each construction site, because the venue's answer to a
        // trailing zero is `quantity is invalid` - an error that says nothing about formatting and
        // sent every order from the engine to rejection until it was traced.
        for (field, value) in [
            ("quantity", self.quantity.as_deref()),
            ("price", self.price.as_deref()),
            ("stopPrice", self.stop_price.as_deref()),
            ("funds", self.funds.as_deref()),
        ] {
            if let Some(value) = value
                && has_trailing_zero(value)
            {
                return Err(RequestError::TrailingZero {
                    field,
                    value: value.to_string(),
                });
            }
        }

        Ok(())
    }
}

/// Body of `POST /trade/leverage`, perps only.
///
/// Both routes in this pair answer `404` on spot, which is consistent: spot here holds balances and
/// carries neither leverage nor margin.
///
/// Field names and order come from the official SDK's `UpdateLeverageRequest`. `marginMode` rides as
/// the venue's integer, like every other enum in a request body on this venue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UpdateLeverageRequest {
    #[serde(rename = "accountID")]
    pub account_id: u64,
    #[serde(rename = "symbolID")]
    pub symbol_id: u64,
    pub leverage: u32,
    #[serde(rename = "marginMode")]
    pub margin_mode: MarginMode,
}

impl UpdateLeverageRequest {
    /// Path this request must be posted to. Perps only.
    pub const ENDPOINT: &'static str = "/trade/leverage";

    /// Action name for the signing payload.
    pub const ACTION: &'static str = "updateLeverage";

    /// Builds a leverage change.
    ///
    /// # Errors
    ///
    /// Returns [`RequestError::LeverageOutOfRange`] for zero leverage, which is not a position with
    /// no leverage but a meaningless request. The venue's own per-instrument ceiling
    /// (`maxLeverage`, 40 on perps BTC-USD) is not checked here: it lives on the instrument, and a
    /// request type that guessed it would drift from the listing.
    pub fn new(
        account_id: u64,
        symbol_id: u64,
        leverage: u32,
        margin_mode: MarginMode,
    ) -> Result<Self, RequestError> {
        if leverage == 0 {
            return Err(RequestError::LeverageOutOfRange(leverage));
        }

        Ok(Self {
            account_id,
            symbol_id,
            leverage,
            margin_mode,
        })
    }
}

/// Body of `POST /trade/margin`, perps only.
///
/// Moves margin against one isolated position. The **sign convention is unobserved**: the SDK types
/// it as a plain decimal and neither the route nor the documentation says whether a negative amount
/// withdraws. So this carries the caller's string through unchanged rather than normalizing it, and
/// whoever first runs it should record what a negative amount does.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UpdateMarginRequest {
    #[serde(rename = "accountID")]
    pub account_id: u64,
    #[serde(rename = "symbolID")]
    pub symbol_id: u64,
    pub amount: String,
}

impl UpdateMarginRequest {
    /// Path this request must be posted to. Perps only.
    pub const ENDPOINT: &'static str = "/trade/margin";

    /// Action name for the signing payload.
    pub const ACTION: &'static str = "updateMargin";

    /// Builds a margin change.
    ///
    /// # Errors
    ///
    /// Returns [`RequestError::ZeroMargin`] for an amount that parses to zero, which would spend a
    /// signed request and a rate-limit slot to move nothing.
    pub fn new(
        account_id: u64,
        symbol_id: u64,
        amount: impl Into<String>,
    ) -> Result<Self, RequestError> {
        let amount = amount.into();
        match amount.parse::<rust_decimal::Decimal>() {
            Ok(value) if value.is_zero() => return Err(RequestError::ZeroMargin),
            Ok(_) => {}
            Err(e) => {
                return Err(RequestError::InvalidMargin {
                    value: amount,
                    reason: e.to_string(),
                });
            }
        }

        // Parsing to a non-zero decimal is not enough, because the venue reads the string rather
        // than a number. Applied by the same rule as an order's fields, not from a measurement of
        // this route: a local refusal of `"12.30"` costs the caller a rewrite to `"12.3"`, which
        // is the cheaper way to be wrong. Ordered after the zero check so `"0.0"` still reads as
        // an attempt to move nothing.
        if has_trailing_zero(&amount) {
            return Err(RequestError::TrailingZero {
                field: "amount",
                value: amount,
            });
        }

        Ok(Self {
            account_id,
            symbol_id,
            amount,
        })
    }
}

/// Body of `POST /trade/orders/modify`, perps only.
///
/// The route exists on perps and answers `404` on spot, so an amend has no spot equivalent: there,
/// the engine has to cancel and replace. Worth knowing before reaching for it, because on a venue
/// that settles on-chain a cancel-replace costs a second round trip and gives up queue position.
///
/// Field names and their order come from the official SDK's `ModifyOrderRequest`, not from this
/// adapter's reading of the documentation. The signing digest is compact JSON in declaration order,
/// so one renamed or reordered key produces `API key not found` - an error naming credentials for a
/// payload fault, which this integration has already paid for once.
///
/// Every mutable field is optional, and the venue takes what is sent: omitting the price amends
/// only the quantity. At least one of `order_id` or `cl_ord_id` has to identify the order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ModifyOrderRequest {
    #[serde(rename = "accountID")]
    pub account_id: u64,
    #[serde(rename = "symbolID")]
    pub symbol_id: u64,
    #[serde(rename = "orderID", skip_serializing_if = "Option::is_none")]
    pub order_id: Option<u64>,
    #[serde(rename = "clOrdID", skip_serializing_if = "Option::is_none")]
    pub cl_ord_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub price: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<String>,
    #[serde(rename = "stopPrice", skip_serializing_if = "Option::is_none")]
    pub stop_price: Option<String>,
}

impl ModifyOrderRequest {
    /// Path this request must be posted to. Perps only.
    pub const ENDPOINT: &'static str = "/trade/orders/modify";

    /// Action name for the signing payload.
    pub const ACTION: &'static str = "modifyOrder";

    /// Builds an amend.
    ///
    /// # Errors
    ///
    /// Returns [`RequestError::UnidentifiedOrder`] if neither id is given - the venue would have
    /// nothing to amend - or [`RequestError::NothingToModify`] if no field would change, which would
    /// spend a signed request and a rate-limit slot to ask for nothing.
    pub fn new(
        account_id: u64,
        symbol_id: u64,
        order_id: Option<u64>,
        cl_ord_id: Option<String>,
        price: Option<String>,
        quantity: Option<String>,
        stop_price: Option<String>,
    ) -> Result<Self, RequestError> {
        if order_id.is_none() && cl_ord_id.is_none() {
            return Err(RequestError::UnidentifiedOrder);
        }
        if price.is_none() && quantity.is_none() && stop_price.is_none() {
            return Err(RequestError::NothingToModify);
        }

        // The same backstop an order item carries, on a route an amend reaches through its own
        // construction site. What was measured is narrower than what this guards: the venue
        // refused `"0.00020"` as a quantity where `"0.0002"` was accepted byte-for-byte
        // otherwise. The rule is applied to the other decimal fields rather than waiting to be
        // bitten by each one - being wrong here refuses a value locally that can be written
        // without the zero, while being wrong the other way sent every order to rejection.
        for (field, value) in [
            ("price", price.as_deref()),
            ("quantity", quantity.as_deref()),
            ("stopPrice", stop_price.as_deref()),
        ] {
            if let Some(value) = value
                && has_trailing_zero(value)
            {
                return Err(RequestError::TrailingZero {
                    field,
                    value: value.to_string(),
                });
            }
        }

        Ok(Self {
            account_id,
            symbol_id,
            order_id,
            cl_ord_id,
            price,
            quantity,
            stop_price,
        })
    }
}

/// Body of `POST /trade/orders`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NewOrderRequest {
    #[serde(rename = "accountID")]
    pub account_id: u64,
    #[serde(rename = "symbolID")]
    pub symbol_id: u64,
    pub orders: Vec<OrderItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub builder: Option<BuilderParams>,
}

impl NewOrderRequest {
    /// Path this request must be posted to.
    ///
    /// Perps batches post here directly; the spot equivalent is `/trade/orders/batch`, and
    /// posting a perps-shaped batch to spot's `/trade/orders` is rejected. Binding the path
    /// to the request type keeps the two from being crossed.
    pub const ENDPOINT: &'static str = "/trade/orders";

    /// Action name for the signing payload.
    pub const ACTION: &'static str = "newOrder";

    /// Builds a batch, validating size and every order.
    ///
    /// # Errors
    ///
    /// Returns [`RequestError::BatchSize`] outside 1..=[`MAX_BATCH`], or the first order-level
    /// violation found.
    pub fn new(
        account_id: u64,
        symbol_id: u64,
        orders: Vec<OrderItem>,
    ) -> Result<Self, RequestError> {
        if orders.is_empty() || orders.len() > MAX_BATCH {
            return Err(RequestError::BatchSize(orders.len()));
        }
        for order in &orders {
            order.validate()?;
        }
        Ok(Self {
            account_id,
            symbol_id,
            orders,
            builder: None,
        })
    }

    /// Attaches a builder fee to every order in the batch.
    #[must_use]
    pub fn with_builder(mut self, builder: BuilderParams) -> Self {
        self.builder = Some(builder);
        self
    }

    /// The client order ids in submission order, for aligning the response.
    #[must_use]
    pub fn client_order_ids(&self) -> Vec<String> {
        self.orders
            .iter()
            .map(|o| o.cl_ord_id.as_str().to_string())
            .collect()
    }
}

/// Body of `POST /trade/orders/schedule-cancel`.
///
/// A dead-man switch: at the scheduled time the venue cancels every open order. Omitting the
/// timestamp clears any pending schedule, which makes that form a harmless idempotent
/// no-op - useful as a probe that exercises the full trading-domain signing path without
/// placing or touching an order.
///
/// The venue requires a scheduled time at least 5 seconds out, and counts triggers against a
/// daily limit of 10; clearing does not consume one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ScheduleCancelRequest {
    #[serde(rename = "accountID")]
    pub account_id: u64,
    #[serde(rename = "scheduledTimestamp", skip_serializing_if = "Option::is_none")]
    pub scheduled_timestamp: Option<u64>,
}

impl ScheduleCancelRequest {
    /// Path this request must be posted to.
    pub const ENDPOINT: &'static str = "/trade/orders/schedule-cancel";

    /// Action name for the signing payload.
    pub const ACTION: &'static str = "scheduleCancel";

    /// Clears any pending scheduled cancel.
    #[must_use]
    pub const fn clear(account_id: u64) -> Self {
        Self {
            account_id,
            scheduled_timestamp: None,
        }
    }

    /// Arms the dead-man switch for a given millisecond timestamp.
    #[must_use]
    pub const fn at(account_id: u64, scheduled_timestamp: u64) -> Self {
        Self {
            account_id,
            scheduled_timestamp: Some(scheduled_timestamp),
        }
    }
}

/// One cancel in a batch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CancelItem {
    #[serde(rename = "symbolID")]
    pub symbol_id: u64,
    #[serde(rename = "orderID", skip_serializing_if = "Option::is_none")]
    pub order_id: Option<u64>,
    #[serde(rename = "clOrdID", skip_serializing_if = "Option::is_none")]
    pub cl_ord_id: Option<ClientOrderId>,
}

impl CancelItem {
    /// Cancels by venue order id.
    #[must_use]
    pub const fn by_order_id(symbol_id: u64, order_id: u64) -> Self {
        Self {
            symbol_id,
            order_id: Some(order_id),
            cl_ord_id: None,
        }
    }

    /// Cancels by client order id.
    #[must_use]
    pub const fn by_client_order_id(symbol_id: u64, cl_ord_id: ClientOrderId) -> Self {
        Self {
            symbol_id,
            order_id: None,
            cl_ord_id: Some(cl_ord_id),
        }
    }

    /// Checks that exactly one identifier is present.
    ///
    /// # Errors
    ///
    /// Returns [`RequestError::CancelIdentification`] if both or neither is set.
    pub const fn validate(&self) -> Result<(), RequestError> {
        match (self.order_id.is_some(), self.cl_ord_id.is_some()) {
            (true, false) | (false, true) => Ok(()),
            _ => Err(RequestError::CancelIdentification),
        }
    }
}

/// Body of `DELETE /trade/orders`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CancelOrderRequest {
    #[serde(rename = "accountID")]
    pub account_id: u64,
    pub cancels: Vec<CancelItem>,
}

impl CancelOrderRequest {
    /// Path this request must be sent to, with `DELETE`.
    pub const ENDPOINT: &'static str = "/trade/orders";

    /// Action name for the signing payload.
    pub const ACTION: &'static str = "cancelOrder";

    /// Builds a cancel batch, validating size and every item.
    ///
    /// # Errors
    ///
    /// Returns [`RequestError::BatchSize`] outside 1..=[`MAX_BATCH`], or
    /// [`RequestError::CancelIdentification`] for an ambiguously identified cancel.
    pub fn new(account_id: u64, cancels: Vec<CancelItem>) -> Result<Self, RequestError> {
        if cancels.is_empty() || cancels.len() > MAX_BATCH {
            return Err(RequestError::BatchSize(cancels.len()));
        }
        for cancel in &cancels {
            cancel.validate()?;
        }
        Ok(Self {
            account_id,
            cancels,
        })
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    fn id(value: &str) -> ClientOrderId {
        ClientOrderId::parse(value).unwrap()
    }

    /// The venue's worked signing example, reproduced from production types.
    ///
    /// This is the field-order contract: key order, omitted optionals, quoted decimals, and
    /// non-optional fields present at their zero value. A reordered field or a dropped
    /// `skip_serializing_if` fails here rather than as an opaque signature rejection.
    /// Pinned for the same reason every other request body here is: the signing digest is compact
    /// JSON in the SDK's declaration order, and `marginMode` rides as the venue's integer rather
    /// than its string, which is how every enum travels in a request on this venue.
    #[rstest]
    fn a_leverage_change_serializes_to_the_sdk_shape() {
        let request = UpdateLeverageRequest::new(60366, 1, 20, MarginMode::Cross).unwrap();

        let json = serde_json::to_string(&request).unwrap();

        assert_eq!(
            json,
            r#"{"accountID":60366,"symbolID":1,"leverage":20,"marginMode":2}"#
        );
    }

    #[rstest]
    fn isolated_mode_rides_as_its_own_integer() {
        let request = UpdateLeverageRequest::new(1, 2, 3, MarginMode::Isolated).unwrap();

        assert!(
            serde_json::to_string(&request)
                .unwrap()
                .contains(r#""marginMode":1"#)
        );
    }

    /// Zero is not "no leverage", it is a meaningless request, so it fails before it is signed.
    #[rstest]
    fn zero_leverage_is_refused() {
        let error = UpdateLeverageRequest::new(1, 2, 0, MarginMode::Cross).unwrap_err();

        assert!(matches!(error, RequestError::LeverageOutOfRange(0)));
    }

    #[rstest]
    fn a_margin_change_serializes_to_the_sdk_shape() {
        let request = UpdateMarginRequest::new(60366, 1, "1.5").unwrap();

        let json = serde_json::to_string(&request).unwrap();

        assert_eq!(json, r#"{"accountID":60366,"symbolID":1,"amount":"1.5"}"#);
    }

    /// The amount is carried through verbatim rather than normalized, because the sign convention is
    /// unobserved: nothing says whether a negative amount withdraws margin.
    #[rstest]
    fn a_negative_margin_amount_is_passed_through_unchanged() {
        let request = UpdateMarginRequest::new(1, 2, "-0.75").unwrap();

        assert_eq!(request.amount, "-0.75");
    }

    #[rstest]
    fn a_margin_change_of_zero_is_refused() {
        for zero in ["0", "0.0", "-0.00"] {
            let error = UpdateMarginRequest::new(1, 2, zero).unwrap_err();
            assert!(
                matches!(error, RequestError::ZeroMargin),
                "{zero} was accepted"
            );
        }
    }

    #[rstest]
    fn an_unreadable_margin_amount_is_refused() {
        let error = UpdateMarginRequest::new(1, 2, "plenty").unwrap_err();

        assert!(matches!(error, RequestError::InvalidMargin { .. }));
    }

    /// The signing digest is compact JSON in the SDK's declaration order, so this pins the exact
    /// bytes. A renamed or reordered key yields `API key not found` - an error naming credentials
    /// for a payload fault, which this integration has already paid for once.
    #[rstest]
    fn a_modify_serializes_to_the_sdk_shape() {
        let request = ModifyOrderRequest::new(
            12345,
            1,
            Some(2_772_119_007),
            None,
            Some("77280".to_string()),
            Some("0.0002".to_string()),
            None,
        )
        .unwrap();

        let json = serde_json::to_string(&request).unwrap();

        assert_eq!(
            json,
            r#"{"accountID":12345,"symbolID":1,"orderID":2772119007,"price":"77280","quantity":"0.0002"}"#
        );
    }

    /// Absent fields are omitted rather than sent as null: the venue amends what it is given, so a
    /// null price would be a different request from no price.
    #[rstest]
    fn an_unchanged_field_is_omitted_entirely() {
        let request =
            ModifyOrderRequest::new(1, 2, Some(3), None, None, Some("0.5".to_string()), None)
                .unwrap();

        let json = serde_json::to_string(&request).unwrap();

        assert_eq!(
            json,
            r#"{"accountID":1,"symbolID":2,"orderID":3,"quantity":"0.5"}"#
        );
    }

    #[rstest]
    fn a_modify_can_name_its_target_by_client_order_id() {
        let request = ModifyOrderRequest::new(
            1,
            2,
            None,
            Some("my-order-1".to_string()),
            Some("100".to_string()),
            None,
            None,
        )
        .unwrap();

        let json = serde_json::to_string(&request).unwrap();

        assert_eq!(
            json,
            r#"{"accountID":1,"symbolID":2,"clOrdID":"my-order-1","price":"100"}"#
        );
    }

    /// Neither id means the venue has nothing to amend, so this fails here rather than spending a
    /// signed request to find out.
    #[rstest]
    fn a_modify_without_either_id_is_refused() {
        let error = ModifyOrderRequest::new(1, 2, None, None, Some("100".to_string()), None, None)
            .unwrap_err();

        assert!(matches!(error, RequestError::UnidentifiedOrder));
    }

    /// An amend that changes nothing would spend a signed request and a rate-limit slot to ask for
    /// the state the order is already in.
    #[rstest]
    fn a_modify_that_changes_nothing_is_refused() {
        let error = ModifyOrderRequest::new(1, 2, Some(3), None, None, None, None).unwrap_err();

        assert!(matches!(error, RequestError::NothingToModify));
    }

    #[rstest]
    fn a_modify_with_a_trailing_zero_is_refused() {
        // The venue reads these by string form, and every engine-formatted price carries that zero
        // whenever the value uses fewer decimals than the instrument allows.
        let error =
            ModifyOrderRequest::new(1, 2, Some(3), None, Some("76562.0".to_string()), None, None)
                .unwrap_err();

        assert!(
            matches!(error, RequestError::TrailingZero { field: "price", .. }),
            "{error:?}"
        );
    }

    #[rstest]
    fn a_margin_amount_with_a_trailing_zero_is_refused() {
        let error = UpdateMarginRequest::new(1, 2, "12.30").unwrap_err();

        assert!(
            matches!(
                error,
                RequestError::TrailingZero {
                    field: "amount",
                    ..
                }
            ),
            "{error:?}"
        );
    }

    #[rstest]
    fn new_order_request_matches_the_venue_signing_example_byte_for_byte() {
        let expected = r#"{"accountID":12345,"symbolID":1,"orders":[{"clOrdID":"my-order-1","modifier":1,"side":1,"type":2,"timeInForce":3,"quantity":"0.001","reduceOnly":false,"positionSide":1}]}"#;

        let request = NewOrderRequest::new(
            12345,
            1,
            vec![OrderItem::market(id("my-order-1"), OrderSide::Buy, "0.001")],
        )
        .unwrap();

        assert_eq!(serde_json::to_string(&request).unwrap(), expected);
    }

    #[rstest]
    fn client_order_id_enforces_the_documented_pattern() {
        assert!(ClientOrderId::parse("my-order_1").is_ok());
        assert!(ClientOrderId::parse("a".repeat(36)).is_ok());

        assert!(ClientOrderId::parse("").is_err());
        assert!(ClientOrderId::parse("a".repeat(37)).is_err());
        assert!(ClientOrderId::parse("BTC-USD:1").is_err(), "colon");
        assert!(ClientOrderId::parse("order 1").is_err(), "space");
    }

    #[rstest]
    fn market_orders_are_ioc_and_carry_no_price() {
        let order = OrderItem::market(id("m1"), OrderSide::Sell, "1.5");

        assert_eq!(order.time_in_force, TimeInForce::Ioc);
        assert!(order.price.is_none());
        order.validate().unwrap();
    }

    #[rstest]
    fn limit_order_rejects_unsupported_time_in_force() {
        let err =
            OrderItem::limit(id("l1"), OrderSide::Buy, TimeInForce::Fok, "100", "1").unwrap_err();

        assert_eq!(err, RequestError::Unsupported("FOK time in force"));
    }

    #[rstest]
    fn funds_is_refused_outside_market_buy() {
        let mut sell = OrderItem::market(id("f1"), OrderSide::Sell, "1");
        sell.quantity = None;
        sell.funds = Some("100".to_string());

        assert_eq!(sell.validate(), Err(RequestError::FundsOnMarketBuyOnly));

        // The market-buy constructor is the supported path and must pass.
        OrderItem::market_buy_with_funds(id("f2"), "100")
            .validate()
            .unwrap();
    }

    #[rstest]
    fn market_order_with_non_ioc_tif_is_refused() {
        let mut order = OrderItem::market(id("m2"), OrderSide::Buy, "1");
        order.time_in_force = TimeInForce::Gtc;

        assert_eq!(
            order.validate(),
            Err(RequestError::MarketTimeInForce(TimeInForce::Gtc))
        );
    }

    #[rstest]
    fn hedge_mode_position_side_is_refused() {
        let mut order = OrderItem::market(id("h1"), OrderSide::Buy, "1");
        order.position_side = PositionSide::Long;

        assert_eq!(
            order.validate(),
            Err(RequestError::Unsupported("hedge-mode position side"))
        );
    }

    #[rstest]
    fn unsupported_trigger_types_are_refused() {
        let mut order = OrderItem::market(id("t1"), OrderSide::Buy, "1");
        order.trigger_type = Some(TriggerType::LastPrice);

        assert_eq!(
            order.validate(),
            Err(RequestError::Unsupported("last/index price trigger"))
        );

        order.trigger_type = Some(TriggerType::MarkPrice);
        order.validate().unwrap();
    }

    #[rstest]
    fn batch_bounds_are_enforced() {
        assert_eq!(
            NewOrderRequest::new(1, 1, vec![]).unwrap_err(),
            RequestError::BatchSize(0)
        );

        let too_many: Vec<OrderItem> = (0..=MAX_BATCH)
            .map(|i| OrderItem::market(id(&format!("o{i}")), OrderSide::Buy, "1"))
            .collect();
        assert_eq!(
            NewOrderRequest::new(1, 1, too_many).unwrap_err(),
            RequestError::BatchSize(MAX_BATCH + 1)
        );

        let exactly_max: Vec<OrderItem> = (0..MAX_BATCH)
            .map(|i| OrderItem::market(id(&format!("o{i}")), OrderSide::Buy, "1"))
            .collect();
        assert!(NewOrderRequest::new(1, 1, exactly_max).is_ok());
    }

    #[rstest]
    fn client_order_ids_come_back_in_submission_order() {
        // These feed align_batch, so the order has to survive intact.
        let request = NewOrderRequest::new(
            1,
            1,
            vec![
                OrderItem::market(id("first"), OrderSide::Buy, "1"),
                OrderItem::market(id("second"), OrderSide::Sell, "2"),
            ],
        )
        .unwrap();

        assert_eq!(request.client_order_ids(), vec!["first", "second"]);
    }

    #[rstest]
    fn clearing_a_scheduled_cancel_omits_the_timestamp() {
        // Presence of the field is what distinguishes arming from clearing, so an
        // always-serialized `null` would arm the dead-man switch instead of clearing it.
        let clear = serde_json::to_string(&ScheduleCancelRequest::clear(60366)).unwrap();
        assert_eq!(clear, r#"{"accountID":60366}"#);

        let armed =
            serde_json::to_string(&ScheduleCancelRequest::at(60366, 1_760_373_925_000)).unwrap();
        assert_eq!(
            armed,
            r#"{"accountID":60366,"scheduledTimestamp":1760373925000}"#
        );
    }

    #[rstest]
    fn cancel_must_name_the_order_exactly_one_way() {
        CancelItem::by_order_id(1, 99).validate().unwrap();
        CancelItem::by_client_order_id(1, id("c1"))
            .validate()
            .unwrap();

        let both = CancelItem {
            symbol_id: 1,
            order_id: Some(99),
            cl_ord_id: Some(id("c1")),
        };
        assert_eq!(both.validate(), Err(RequestError::CancelIdentification));

        let neither = CancelItem {
            symbol_id: 1,
            order_id: None,
            cl_ord_id: None,
        };
        assert_eq!(neither.validate(), Err(RequestError::CancelIdentification));
    }

    #[rstest]
    fn cancel_request_omits_the_unused_identifier() {
        let request = CancelOrderRequest::new(7, vec![CancelItem::by_order_id(1, 99)]).unwrap();
        let json = serde_json::to_string(&request).unwrap();

        assert_eq!(
            json,
            r#"{"accountID":7,"cancels":[{"symbolID":1,"orderID":99}]}"#
        );
        assert!(!json.contains("clOrdID"));
    }

    /// Several cancels travel in one request, which is the point of batching them: the venue charges
    /// `1 + floor(N / 40)` weight for a batch against 1 per separate cancel.
    #[rstest]
    fn a_batch_cancel_carries_every_target_in_one_request() {
        let request = CancelOrderRequest::new(
            7,
            vec![
                CancelItem::by_order_id(1, 99),
                CancelItem::by_order_id(2, 100),
                CancelItem::by_client_order_id(1, id("my-order-1")),
            ],
        )
        .unwrap();

        let json = serde_json::to_string(&request).unwrap();

        assert_eq!(
            json,
            r#"{"accountID":7,"cancels":[{"symbolID":1,"orderID":99},{"symbolID":2,"orderID":100},{"symbolID":1,"clOrdID":"my-order-1"}]}"#
        );
    }

    /// Mixed instruments in one batch: the symbol rides on each item, not on the request, so there is
    /// no reason to split a withdrawal by instrument.
    #[rstest]
    fn a_batch_cancel_may_span_instruments() {
        let request = CancelOrderRequest::new(
            7,
            vec![
                CancelItem::by_order_id(1, 99),
                CancelItem::by_order_id(5, 101),
            ],
        )
        .unwrap();

        let json = serde_json::to_string(&request).unwrap();

        assert!(json.contains(r#"{"symbolID":1,"orderID":99}"#));
        assert!(json.contains(r#"{"symbolID":5,"orderID":101}"#));
    }

    #[rstest]
    fn builder_is_omitted_unless_attached() {
        let plain =
            NewOrderRequest::new(1, 1, vec![OrderItem::market(id("b1"), OrderSide::Buy, "1")])
                .unwrap();
        assert!(!serde_json::to_string(&plain).unwrap().contains("builder"));

        let with_builder = plain.with_builder(BuilderParams { id: 1234, fee: 10 });
        let json = serde_json::to_string(&with_builder).unwrap();
        assert!(json.contains(r#""builder":{"id":1234,"fee":10}"#), "{json}");
    }

    #[rstest]
    fn reduce_only_and_price_bound_compose_onto_a_market_order() {
        let order = OrderItem::market(id("r1"), OrderSide::Sell, "1")
            .reduce_only()
            .with_price_bound("90000");

        assert!(order.reduce_only);
        assert_eq!(order.price.as_deref(), Some("90000"));
        order.validate().unwrap();
    }
}
