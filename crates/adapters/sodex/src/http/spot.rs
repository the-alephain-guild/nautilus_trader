//! Spot trading request payloads.
//!
//! Spot and perps are not the same shape, and two differences are easy to get wrong because
//! both forms compile and serialize cleanly:
//!
//! - **`symbolID` lives on the order item**, not on the request. A spot batch can therefore
//!   span symbols, while a perps batch names one symbol for all of its orders.
//! - **A spot cancel carries two client order ids.** `clOrdID` identifies the *cancellation
//!   request itself* and is required; `origClOrdID` names the order being cancelled. Perps
//!   uses `clOrdID` for the target directly, so porting a perps cancel across would cancel
//!   nothing while looking correct.
//!
//! - **The batch endpoints live at a different path.** Spot batches go to
//!   `/trade/orders/batch`, while the perps batch endpoint is `/trade/orders`. Spot's
//!   `/trade/orders` is a *single-order* endpoint, so posting a batch there is rejected for
//!   missing the top-level `symbolID`, `clOrdID`, `side`, `type` and `timeInForce` - the
//!   fields it wanted flat rather than nested. The path is therefore bound to the request
//!   type as [`SpotNewOrderRequest::ENDPOINT`] instead of left to the caller.
//!
//! Spot order items also lack the perps-only fields - no modifier, stop, trigger,
//! reduce-only or position side - since those describe positions, which spot does not have.
//!
//! - **The action names differ too.** Spot's batch actions are `batchNewOrder` and
//!   `batchCancelOrder`, where perps uses `newOrder` and `cancelOrder`. The action name is
//!   hashed into the signature, so borrowing the perps name produces a valid signature over
//!   the wrong digest.
//!
//! # Why a wrong action name reports "API key not found"
//!
//! The gateway recovers the signer's address from the digest and signature, then looks up an
//! API key by that address. ECDSA recovery does not fail on a wrong digest - it returns a
//! *different* address, and no key is registered for it. So a mismatched payload hash
//! surfaces as a missing key rather than as a signature error, pointing at credentials when
//! the credentials are fine.
//!
//! Distinguishing the two: if a cheap signed call such as `scheduleCancel` succeeds with the
//! same key on the same engine, the key is registered and working, and the fault is in the
//! failing request's payload or action name.
//!
//! One more asymmetry worth knowing when pricing orders: spot's limit price bounds are
//! computed from `lastTradePrice`, while perps uses `markPrice`.
//!
//! Field order mirrors the venue's schema tables, for the same signing reason as perps.

use serde::Serialize;

use super::requests::{BuilderParams, ClientOrderId, MAX_BATCH, RequestError};
use crate::common::enums::{OrderSide, OrderType, TimeInForce};

/// One spot order. The venue calls this `BatchNewOrderItem`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SpotOrderItem {
    #[serde(rename = "symbolID")]
    pub symbol_id: u64,
    #[serde(rename = "clOrdID")]
    pub cl_ord_id: ClientOrderId,
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
}

impl SpotOrderItem {
    /// A limit order.
    ///
    /// # Errors
    ///
    /// Returns [`RequestError::Unsupported`] for a time in force the venue does not accept.
    pub fn limit(
        symbol_id: u64,
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
            symbol_id,
            cl_ord_id,
            side,
            order_type: OrderType::Limit,
            time_in_force,
            price: Some(price.into()),
            quantity: Some(quantity.into()),
            funds: None,
        })
    }

    /// A market order sized in the base asset.
    #[must_use]
    pub fn market(
        symbol_id: u64,
        cl_ord_id: ClientOrderId,
        side: OrderSide,
        quantity: impl Into<String>,
    ) -> Self {
        Self {
            symbol_id,
            cl_ord_id,
            side,
            order_type: OrderType::Market,
            time_in_force: TimeInForce::Ioc,
            price: None,
            quantity: Some(quantity.into()),
            funds: None,
        }
    }

    /// Checks the combination against the venue's placement rules.
    ///
    /// # Errors
    ///
    /// Returns [`RequestError`] describing the first rule violated.
    pub fn validate(&self) -> Result<(), RequestError> {
        if !self.time_in_force.is_supported_for_placement() {
            return Err(RequestError::Unsupported("FOK time in force"));
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

/// Body of `POST /spot/trade/orders`. The venue calls this `BatchNewOrderRequest`.
///
/// Unlike the perps form there is no request-level symbol: each order names its own.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SpotNewOrderRequest {
    #[serde(rename = "accountID")]
    pub account_id: u64,
    pub orders: Vec<SpotOrderItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub builder: Option<BuilderParams>,
}

impl SpotNewOrderRequest {
    /// Path this request must be posted to.
    ///
    /// Not `/trade/orders` - that is spot's single-order endpoint and rejects a batch.
    pub const ENDPOINT: &'static str = "/trade/orders/batch";

    /// Action name for the signing payload.
    ///
    /// `batchNewOrder`, not `newOrder` - spot's batch form is its own action. The name is
    /// hashed into the signature, so using the perps name yields a digest the venue cannot
    /// match. See the module docs for why that surfaces as "API key not found".
    pub const ACTION: &'static str = "batchNewOrder";

    /// Builds a batch, validating size and every order.
    ///
    /// # Errors
    ///
    /// Returns [`RequestError::BatchSize`] outside 1..=[`MAX_BATCH`], or the first
    /// order-level violation.
    pub fn new(account_id: u64, orders: Vec<SpotOrderItem>) -> Result<Self, RequestError> {
        if orders.is_empty() || orders.len() > MAX_BATCH {
            return Err(RequestError::BatchSize(orders.len()));
        }
        for order in &orders {
            order.validate()?;
        }
        Ok(Self {
            account_id,
            orders,
            builder: None,
        })
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

/// One spot cancellation. The venue calls this `BatchCancelOrderItem`.
///
/// Note the two ids: `cl_ord_id` labels this cancellation, `orig_cl_ord_id` names the order
/// to cancel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SpotCancelItem {
    #[serde(rename = "symbolID")]
    pub symbol_id: u64,
    /// Identifier for this cancellation request. Required.
    #[serde(rename = "clOrdID")]
    pub cl_ord_id: ClientOrderId,
    /// Venue order id to cancel.
    #[serde(rename = "orderID", skip_serializing_if = "Option::is_none")]
    pub order_id: Option<u64>,
    /// Client order id of the order to cancel.
    #[serde(rename = "origClOrdID", skip_serializing_if = "Option::is_none")]
    pub orig_cl_ord_id: Option<ClientOrderId>,
}

impl SpotCancelItem {
    /// Cancels by venue order id.
    #[must_use]
    pub const fn by_order_id(symbol_id: u64, request_id: ClientOrderId, order_id: u64) -> Self {
        Self {
            symbol_id,
            cl_ord_id: request_id,
            order_id: Some(order_id),
            orig_cl_ord_id: None,
        }
    }

    /// Cancels by the original client order id.
    #[must_use]
    pub const fn by_client_order_id(
        symbol_id: u64,
        request_id: ClientOrderId,
        target: ClientOrderId,
    ) -> Self {
        Self {
            symbol_id,
            cl_ord_id: request_id,
            order_id: None,
            orig_cl_ord_id: Some(target),
        }
    }

    /// Checks that exactly one target identifier is present.
    ///
    /// # Errors
    ///
    /// Returns [`RequestError::CancelIdentification`] if both or neither is set.
    pub const fn validate(&self) -> Result<(), RequestError> {
        match (self.order_id.is_some(), self.orig_cl_ord_id.is_some()) {
            (true, false) | (false, true) => Ok(()),
            _ => Err(RequestError::CancelIdentification),
        }
    }
}

/// Body of `DELETE /spot/trade/orders`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SpotCancelOrderRequest {
    #[serde(rename = "accountID")]
    pub account_id: u64,
    pub cancels: Vec<SpotCancelItem>,
}

impl SpotCancelOrderRequest {
    /// Path this request must be sent to, with `DELETE`.
    pub const ENDPOINT: &'static str = "/trade/orders/batch";

    /// Action name for the signing payload. `batchCancelOrder`, not `cancelOrder`.
    pub const ACTION: &'static str = "batchCancelOrder";

    /// Builds a cancel batch, validating size and every item.
    ///
    /// # Errors
    ///
    /// Returns [`RequestError`] for a bad batch size or an ambiguously identified cancel.
    pub fn new(account_id: u64, cancels: Vec<SpotCancelItem>) -> Result<Self, RequestError> {
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

    #[rstest]
    fn order_item_carries_its_own_symbol() {
        // The perps form puts symbolID on the request; putting it there for spot would drop
        // it from every item and leave the venue unable to route the order.
        let request = SpotNewOrderRequest::new(
            60366,
            vec![
                SpotOrderItem::limit(
                    1,
                    id("a"),
                    OrderSide::Buy,
                    TimeInForce::Gtc,
                    "40000",
                    "0.001",
                )
                .unwrap(),
            ],
        )
        .unwrap();

        let json = serde_json::to_string(&request).unwrap();
        assert_eq!(
            json,
            r#"{"accountID":60366,"orders":[{"symbolID":1,"clOrdID":"a","side":1,"type":1,"timeInForce":1,"price":"40000","quantity":"0.001"}]}"#
        );
    }

    #[rstest]
    fn a_spot_batch_may_span_symbols() {
        // Consequence of the per-item symbol; a perps batch cannot do this.
        let request = SpotNewOrderRequest::new(
            60366,
            vec![
                SpotOrderItem::market(1, id("btc"), OrderSide::Buy, "0.001"),
                SpotOrderItem::market(2, id("eth"), OrderSide::Sell, "0.5"),
            ],
        )
        .unwrap();

        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains(r#""symbolID":1"#));
        assert!(json.contains(r#""symbolID":2"#));
    }

    #[rstest]
    fn cancel_distinguishes_its_own_id_from_the_target() {
        // clOrdID labels the cancellation; origClOrdID names the order. Conflating them
        // produces a request that is accepted in shape but cancels nothing.
        let cancel = SpotCancelItem::by_client_order_id(1, id("cancel-1"), id("order-7"));
        cancel.validate().unwrap();

        let json = serde_json::to_string(&cancel).unwrap();
        assert_eq!(
            json,
            r#"{"symbolID":1,"clOrdID":"cancel-1","origClOrdID":"order-7"}"#
        );
    }

    #[rstest]
    fn cancel_by_order_id_omits_the_original_client_id() {
        let cancel = SpotCancelItem::by_order_id(1, id("cancel-1"), 987);
        let json = serde_json::to_string(&cancel).unwrap();

        assert_eq!(json, r#"{"symbolID":1,"clOrdID":"cancel-1","orderID":987}"#);
        assert!(!json.contains("origClOrdID"));
    }

    #[rstest]
    fn cancel_must_name_exactly_one_target() {
        let both = SpotCancelItem {
            symbol_id: 1,
            cl_ord_id: id("c"),
            order_id: Some(1),
            orig_cl_ord_id: Some(id("o")),
        };
        assert_eq!(both.validate(), Err(RequestError::CancelIdentification));

        let neither = SpotCancelItem {
            symbol_id: 1,
            cl_ord_id: id("c"),
            order_id: None,
            orig_cl_ord_id: None,
        };
        assert_eq!(neither.validate(), Err(RequestError::CancelIdentification));
    }

    #[rstest]
    fn spot_items_have_no_position_fields() {
        // Spot has no positions, so reduceOnly and positionSide must not appear; sending
        // them would be a shape the venue does not expect.
        let json =
            serde_json::to_string(&SpotOrderItem::market(1, id("m"), OrderSide::Buy, "0.001"))
                .unwrap();

        assert!(!json.contains("reduceOnly"));
        assert!(!json.contains("positionSide"));
        assert!(!json.contains("modifier"));
    }

    #[rstest]
    fn market_orders_are_ioc() {
        let order = SpotOrderItem::market(1, id("m"), OrderSide::Buy, "0.001");

        assert_eq!(order.time_in_force, TimeInForce::Ioc);
        order.validate().unwrap();
    }

    #[rstest]
    fn batch_bounds_are_enforced() {
        assert_eq!(
            SpotNewOrderRequest::new(1, vec![]).unwrap_err(),
            RequestError::BatchSize(0)
        );
    }

    /// Action names are hashed into the signature, and the perps names are not
    /// interchangeable with the spot ones. Values taken from the official Go SDK
    /// (`spot/types/batch_new_order_request.go`, `perps/types/new_order_request.go`).
    #[rstest]
    fn spot_batch_actions_are_not_the_perps_action_names() {
        use crate::http::requests::{CancelOrderRequest, NewOrderRequest};

        assert_eq!(SpotNewOrderRequest::ACTION, "batchNewOrder");
        assert_eq!(SpotCancelOrderRequest::ACTION, "batchCancelOrder");

        assert_eq!(NewOrderRequest::ACTION, "newOrder");
        assert_eq!(CancelOrderRequest::ACTION, "cancelOrder");

        assert_ne!(SpotNewOrderRequest::ACTION, NewOrderRequest::ACTION);
        assert_ne!(SpotCancelOrderRequest::ACTION, CancelOrderRequest::ACTION);
    }

    /// The action name is part of the hashed envelope, so swapping it changes the digest and
    /// therefore the signature - which is why the wrong name cannot be shrugged off.
    #[rstest]
    fn action_name_changes_the_signing_digest() {
        use crate::signing::payload_hash;

        let request = SpotNewOrderRequest::new(
            60366,
            vec![SpotOrderItem::market(1, id("a"), OrderSide::Buy, "0.001")],
        )
        .unwrap();

        let correct = payload_hash(SpotNewOrderRequest::ACTION, &request).unwrap();
        let perps_name = payload_hash("newOrder", &request).unwrap();

        assert_ne!(correct, perps_name);
    }

    #[rstest]
    fn spot_batches_target_the_batch_path_not_the_single_order_one() {
        // Posting to /trade/orders reaches spot's single-order endpoint, which rejects the
        // batch for missing the flat fields it expects. This was a live failure, not a
        // hypothetical.
        use crate::http::requests::{CancelOrderRequest, NewOrderRequest};

        assert_eq!(SpotNewOrderRequest::ENDPOINT, "/trade/orders/batch");
        assert_eq!(SpotCancelOrderRequest::ENDPOINT, "/trade/orders/batch");

        // And the perps paths deliberately differ.
        assert_eq!(NewOrderRequest::ENDPOINT, "/trade/orders");
        assert_eq!(CancelOrderRequest::ENDPOINT, "/trade/orders");
        assert_ne!(SpotNewOrderRequest::ENDPOINT, NewOrderRequest::ENDPOINT);
    }
}
