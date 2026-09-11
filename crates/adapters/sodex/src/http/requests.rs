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
    OrderModifier, OrderSide, OrderType, PositionSide, StopType, TimeInForce, TriggerType,
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
        Ok(())
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
