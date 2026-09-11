//! Translation between Nautilus orders and SoDEX requests.
//!
//! # Enum values are not interchangeable
//!
//! Nautilus and the venue both encode time-in-force as small integers, and the two
//! encodings **collide on the wrong pairs**: `IOC` is 2 in Nautilus and 3 at the venue,
//! while `FOK` is 3 and 2 respectively. A numeric cast would silently turn an
//! immediate-or-cancel order into fill-or-kill and vice versa - same request shape, same
//! successful submission, entirely different execution semantics.
//!
//! Every enum is therefore mapped by name here, never by value, and a test asserts the two
//! numbering schemes actually disagree so that the mapping cannot be "simplified" into a
//! cast later.
//!
//! # Two Nautilus flags become venue values
//!
//! - `post_only` is a separate boolean in Nautilus but is the `GTX` time-in-force at the
//!   venue. It is only expressible on a resting order, so combining it with `IOC` is
//!   refused rather than silently dropped.
//! - `quote_quantity` selects the venue's `funds` field, which it accepts only on market
//!   buys.
//!
//! # Client order ids pass through unchanged
//!
//! The venue constrains them to `^[0-9a-zA-Z_-]{1,36}$`. Rather than hashing or rewriting a
//! non-conforming id - which would leave the venue, the logs and any reconciliation
//! disagreeing about what an order is called - the conversion refuses it and says why.

use nautilus_model::{
    enums::{
        OrderSide as NautilusSide, OrderStatus as NautilusStatus, OrderType as NautilusType,
        TimeInForce as NautilusTif,
    },
    events::OrderInitialized,
    identifiers::ClientOrderId as NautilusClientOrderId,
};

use crate::{
    common::enums::{ExecutionType, OrderSide, OrderStatus, OrderType, PositionSide, TimeInForce},
    http::{
        requests::{ClientOrderId, OrderItem, RequestError},
        spot::SpotOrderItem,
    },
};

/// Why a Nautilus order cannot be expressed at this venue.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OrderConversionError {
    #[error("{what} is not supported by SoDEX")]
    Unsupported { what: String },
    #[error("client order id {id:?} is not accepted: {reason}")]
    ClientOrderId { id: String, reason: String },
    #[error("post-only requires a resting order; it cannot be combined with {tif:?}")]
    PostOnlyWithNonResting { tif: NautilusTif },
    #[error("quote-denominated size is only valid for market buy orders")]
    QuoteQuantityNotMarketBuy,
    #[error("a limit order requires a price")]
    LimitWithoutPrice,
    #[error(transparent)]
    Request(#[from] RequestError),
}

/// Maps an order side.
///
/// Explicit rather than numeric even though the values happen to agree today.
#[must_use]
pub const fn map_side(side: NautilusSide) -> OrderSide {
    match side {
        NautilusSide::Buy => OrderSide::Buy,
        NautilusSide::Sell => OrderSide::Sell,
    }
}

/// The venue-shaped essentials of an order, extracted from a Nautilus event.
///
/// Separating extraction from mapping keeps the semantic work - which enum becomes which,
/// which flag combinations are legal - testable without constructing a 34-field engine
/// event, and gives modify and replace a shared vocabulary later.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderSpec {
    pub cl_ord_id: ClientOrderId,
    pub side: OrderSide,
    pub order_type: OrderType,
    pub time_in_force: TimeInForce,
    /// Decimal string, as the venue expects.
    pub quantity: String,
    /// Limit price, or a slippage bound on a market order.
    pub price: Option<String>,
    /// Size is denominated in the quote asset (the venue's `funds`).
    pub quote_quantity: bool,
    pub reduce_only: bool,
}

impl OrderSpec {
    /// Extracts the venue-relevant fields from a Nautilus order event.
    ///
    /// # Errors
    ///
    /// Returns [`OrderConversionError`] when the order uses a type, time-in-force or client
    /// order id the venue cannot express.
    pub fn from_initialized(init: &OrderInitialized) -> Result<Self, OrderConversionError> {
        let order_type =
            map_order_type(init.order_type).ok_or_else(|| OrderConversionError::Unsupported {
                what: format!("{:?} orders", init.order_type),
            })?;

        Ok(Self {
            cl_ord_id: map_client_order_id(&init.client_order_id)?,
            side: map_side(init.order_side),
            order_type,
            time_in_force: map_time_in_force(init.time_in_force, init.post_only)?,
            quantity: init.quantity.to_string(),
            price: init.price.map(|p| p.to_string()),
            quote_quantity: init.quote_quantity,
            reduce_only: init.reduce_only,
        })
    }
}

/// Maps an order type.
///
/// The venue offers only market and limit. Stop and trailing variants exist there as order
/// *modifiers* on perps rather than as types, so they are refused here instead of being
/// approximated by a plain limit - which would place an order that executes immediately
/// instead of waiting for its trigger.
#[must_use]
pub const fn map_order_type(order_type: NautilusType) -> Option<OrderType> {
    match order_type {
        NautilusType::Market => Some(OrderType::Market),
        NautilusType::Limit => Some(OrderType::Limit),
        _ => None,
    }
}

/// Maps time-in-force, resolving `post_only` into the venue's `GTX`.
///
/// # Errors
///
/// Returns [`OrderConversionError`] for a time-in-force the venue does not accept, or for
/// post-only combined with a non-resting one.
pub fn map_time_in_force(
    tif: NautilusTif,
    post_only: bool,
) -> Result<TimeInForce, OrderConversionError> {
    if post_only {
        // GTX means "expire if it would fill immediately", which only makes sense for an
        // order intended to rest. Pairing it with IOC asks for both at once.
        return match tif {
            NautilusTif::Gtc => Ok(TimeInForce::Gtx),
            other => Err(OrderConversionError::PostOnlyWithNonResting { tif: other }),
        };
    }

    match tif {
        NautilusTif::Gtc => Ok(TimeInForce::Gtc),
        NautilusTif::Ioc => Ok(TimeInForce::Ioc),
        other => Err(OrderConversionError::Unsupported {
            what: format!("{other:?} time in force"),
        }),
    }
}

/// Validates and carries over a client order id.
///
/// # Errors
///
/// Returns [`OrderConversionError::ClientOrderId`] when the id violates the venue's pattern.
pub fn map_client_order_id(
    id: &NautilusClientOrderId,
) -> Result<ClientOrderId, OrderConversionError> {
    ClientOrderId::parse(id.as_str()).map_err(|e| OrderConversionError::ClientOrderId {
        id: id.to_string(),
        reason: e.to_string(),
    })
}

/// Builds a spot order item from a Nautilus order.
///
/// # Errors
///
/// Returns [`OrderConversionError`] when the order cannot be expressed at this venue.
pub fn to_spot_order(
    spec: &OrderSpec,
    symbol_id: u64,
) -> Result<SpotOrderItem, OrderConversionError> {
    if spec.reduce_only {
        // Spot has no positions to reduce; accepting the flag would imply a guarantee the
        // venue cannot make.
        return Err(OrderConversionError::Unsupported {
            what: "reduce-only on spot".to_string(),
        });
    }

    if spec.quote_quantity {
        if spec.order_type != OrderType::Market || spec.side != OrderSide::Buy {
            return Err(OrderConversionError::QuoteQuantityNotMarketBuy);
        }
        return Ok(SpotOrderItem {
            symbol_id,
            cl_ord_id: spec.cl_ord_id.clone(),
            side: spec.side,
            order_type: spec.order_type,
            time_in_force: spec.time_in_force,
            price: None,
            quantity: None,
            funds: Some(spec.quantity.clone()),
        });
    }

    let price = match spec.order_type {
        OrderType::Limit => Some(
            spec.price
                .clone()
                .ok_or(OrderConversionError::LimitWithoutPrice)?,
        ),
        // A market order may still carry a price as a slippage bound.
        OrderType::Market => spec.price.clone(),
    };

    Ok(SpotOrderItem {
        symbol_id,
        cl_ord_id: spec.cl_ord_id.clone(),
        side: spec.side,
        order_type: spec.order_type,
        time_in_force: spec.time_in_force,
        price,
        quantity: Some(spec.quantity.clone()),
        funds: None,
    })
}

/// Builds a perps order item from a Nautilus order.
///
/// # Errors
///
/// Returns [`OrderConversionError`] when the order cannot be expressed at this venue.
pub fn to_perps_order(spec: &OrderSpec) -> Result<OrderItem, OrderConversionError> {
    let cl_ord_id = spec.cl_ord_id.clone();

    let mut item = match spec.order_type {
        OrderType::Limit => {
            let price = spec
                .price
                .clone()
                .ok_or(OrderConversionError::LimitWithoutPrice)?;
            OrderItem::limit(
                cl_ord_id,
                spec.side,
                spec.time_in_force,
                price,
                spec.quantity.clone(),
            )?
        }
        OrderType::Market => {
            if spec.quote_quantity {
                if spec.side != OrderSide::Buy {
                    return Err(OrderConversionError::QuoteQuantityNotMarketBuy);
                }
                OrderItem::market_buy_with_funds(cl_ord_id, spec.quantity.clone())
            } else {
                let mut market = OrderItem::market(cl_ord_id, spec.side, spec.quantity.clone());
                if let Some(price) = &spec.price {
                    market = market.with_price_bound(price.clone());
                }
                market
            }
        }
    };

    if spec.reduce_only {
        item = item.reduce_only();
    }
    item.position_side = PositionSide::Both;

    item.validate()?;
    Ok(item)
}

/// Maps a venue order status onto the Nautilus equivalent.
///
/// Returns `None` for `TRIGGERED`, which has no Nautilus counterpart: a triggered stop
/// becomes an ordinary working order there, so the caller decides what to emit rather than
/// having a guess made here.
#[must_use]
pub const fn map_order_status(status: OrderStatus) -> Option<NautilusStatus> {
    use nautilus_model::enums::OrderStatus as N;
    match status {
        OrderStatus::New => Some(N::Accepted),
        OrderStatus::PartiallyFilled => Some(N::PartiallyFilled),
        OrderStatus::Filled => Some(N::Filled),
        OrderStatus::Canceled => Some(N::Canceled),
        OrderStatus::Rejected => Some(N::Rejected),
        OrderStatus::Expired => Some(N::Expired),
        OrderStatus::Triggered => None,
    }
}

/// Whether an execution report represents a fill.
#[must_use]
pub const fn is_fill(execution_type: ExecutionType) -> bool {
    matches!(
        execution_type,
        ExecutionType::PartiallyFilled | ExecutionType::Filled
    )
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;
    /// The default Nautilus client order id shape: 27 characters, inside the venue's limit.
    const DEFAULT_ID: &str = "O-19700101-000000-001-001-1";

    fn spec(order_type: OrderType, side: OrderSide, tif: TimeInForce) -> OrderSpec {
        OrderSpec {
            cl_ord_id: ClientOrderId::parse(DEFAULT_ID).unwrap(),
            side,
            order_type,
            time_in_force: tif,
            quantity: "0.001".to_string(),
            price: Some("40000".to_string()),
            quote_quantity: false,
            reduce_only: false,
        }
    }

    #[rstest]
    fn nautilus_and_venue_time_in_force_numbering_actually_disagree() {
        // The reason every mapping here is by name. If this ever stops holding, the mapping
        // could be simplified - until then a cast silently swaps IOC and FOK.
        assert_eq!(NautilusTif::Ioc as u8, 2);
        assert_eq!(TimeInForce::Ioc.as_int(), 3);
        assert_eq!(NautilusTif::Fok as u8, 3);
        assert_eq!(TimeInForce::Fok.as_int(), 2);
    }

    #[rstest]
    fn ioc_maps_to_ioc_and_not_to_the_venue_value_that_shares_its_number() {
        let mapped = map_time_in_force(NautilusTif::Ioc, false).unwrap();

        assert_eq!(mapped, TimeInForce::Ioc);
        assert_ne!(mapped, TimeInForce::Fok);
    }

    #[rstest]
    fn post_only_becomes_gtx() {
        assert_eq!(
            map_time_in_force(NautilusTif::Gtc, true).unwrap(),
            TimeInForce::Gtx
        );
    }

    #[rstest]
    fn post_only_with_ioc_is_refused() {
        // Asking for "never take liquidity" and "fill immediately" at once. Dropping either
        // flag silently would change what the strategy asked for.
        assert!(matches!(
            map_time_in_force(NautilusTif::Ioc, true),
            Err(OrderConversionError::PostOnlyWithNonResting { .. })
        ));
    }

    #[rstest]
    fn unsupported_time_in_force_is_refused() {
        for tif in [NautilusTif::Fok, NautilusTif::Gtd, NautilusTif::Day] {
            assert!(
                map_time_in_force(tif, false).is_err(),
                "{tif:?} must be refused"
            );
        }
    }

    #[rstest]
    fn stop_orders_are_refused_rather_than_flattened_to_limit() {
        // A stop treated as a plain limit would execute right away instead of waiting for
        // its trigger - the most damaging possible silent substitution.
        for order_type in [
            NautilusType::StopMarket,
            NautilusType::StopLimit,
            NautilusType::TrailingStopMarket,
        ] {
            assert_eq!(map_order_type(order_type), None, "{order_type:?}");
        }
    }

    #[rstest]
    fn a_limit_order_converts_with_its_price_and_id_intact() {
        let item =
            to_spot_order(&spec(OrderType::Limit, OrderSide::Buy, TimeInForce::Gtc), 1).unwrap();

        assert_eq!(item.cl_ord_id.as_str(), DEFAULT_ID);
        assert_eq!(item.price.as_deref(), Some("40000"));
        assert_eq!(item.quantity.as_deref(), Some("0.001"));
        assert_eq!(item.side, OrderSide::Buy);
        assert_eq!(item.time_in_force, TimeInForce::Gtc);
    }

    #[rstest]
    fn the_default_nautilus_client_order_id_is_accepted_unchanged() {
        // 27 characters of [0-9A-Za-z-], inside the venue's 36-character limit - which is
        // what makes pass-through viable instead of a hashed mapping.
        let id = NautilusClientOrderId::from(DEFAULT_ID);

        assert_eq!(map_client_order_id(&id).unwrap().as_str(), DEFAULT_ID);
    }

    #[rstest]
    fn a_non_conforming_client_order_id_is_refused_not_rewritten() {
        // Rewriting would leave the venue, the logs and reconciliation disagreeing about
        // what this order is called.
        let id = NautilusClientOrderId::from("order:with:colons");

        assert!(matches!(
            map_client_order_id(&id),
            Err(OrderConversionError::ClientOrderId { .. })
        ));
    }

    #[rstest]
    fn a_limit_order_without_a_price_is_refused() {
        let mut order = spec(OrderType::Limit, OrderSide::Buy, TimeInForce::Gtc);
        order.price = None;

        assert_eq!(
            to_spot_order(&order, 1),
            Err(OrderConversionError::LimitWithoutPrice)
        );
    }

    #[rstest]
    fn quote_quantity_requires_a_market_buy() {
        let mut sell = spec(OrderType::Market, OrderSide::Sell, TimeInForce::Ioc);
        sell.quote_quantity = true;
        assert_eq!(
            to_spot_order(&sell, 1),
            Err(OrderConversionError::QuoteQuantityNotMarketBuy)
        );

        let mut buy = spec(OrderType::Market, OrderSide::Buy, TimeInForce::Ioc);
        buy.quote_quantity = true;
        let item = to_spot_order(&buy, 1).unwrap();
        assert_eq!(item.funds.as_deref(), Some("0.001"));
        assert!(item.quantity.is_none());
    }

    #[rstest]
    fn reduce_only_is_refused_on_spot_but_carried_on_perps() {
        let mut order = spec(OrderType::Market, OrderSide::Sell, TimeInForce::Ioc);
        order.reduce_only = true;

        assert!(matches!(
            to_spot_order(&order, 1),
            Err(OrderConversionError::Unsupported { .. })
        ));

        assert!(to_perps_order(&order).unwrap().reduce_only);
    }

    #[rstest]
    fn perps_orders_use_one_way_position_side() {
        // Hedge mode is documented as unsupported for placement; anything else would be
        // rejected by the venue.
        let item =
            to_perps_order(&spec(OrderType::Limit, OrderSide::Buy, TimeInForce::Gtc)).unwrap();

        assert_eq!(item.position_side, PositionSide::Both);
    }

    #[rstest]
    fn venue_statuses_map_onto_nautilus_equivalents() {
        use nautilus_model::enums::OrderStatus as N;

        assert_eq!(map_order_status(OrderStatus::New), Some(N::Accepted));
        assert_eq!(map_order_status(OrderStatus::Filled), Some(N::Filled));
        assert_eq!(map_order_status(OrderStatus::Canceled), Some(N::Canceled));
        assert_eq!(map_order_status(OrderStatus::Expired), Some(N::Expired));
    }

    #[rstest]
    fn triggered_has_no_direct_equivalent() {
        // Deliberately unmapped: a triggered stop becomes an ordinary working order in
        // Nautilus, and guessing which event to emit belongs to the caller.
        assert_eq!(map_order_status(OrderStatus::Triggered), None);
    }

    #[rstest]
    fn only_fill_execution_types_count_as_fills() {
        assert!(is_fill(ExecutionType::Filled));
        assert!(is_fill(ExecutionType::PartiallyFilled));

        for other in [
            ExecutionType::New,
            ExecutionType::Canceled,
            ExecutionType::Rejected,
            ExecutionType::Modified,
            ExecutionType::Expired,
            ExecutionType::Replaced,
        ] {
            assert!(!is_fill(other), "{other:?} is not a fill");
        }
    }
}
