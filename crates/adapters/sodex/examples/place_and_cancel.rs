//! Places a resting limit order on spot and cancels it, verifying the full order contract.
//!
//! The order is priced far below the market so it rests on the book instead of filling, and
//! it is cancelled immediately. That exercises what the signing probe could not: symbol
//! resolution, the price and lot filters, notional minimums, order id round-tripping and the
//! batch response alignment.
//!
//! **This places a real order.** On testnet that is play money, but the same program against
//! `SODEX_NETWORK=mainnet` would place a real one. The price bound below is what keeps it
//! from filling - do not raise it toward the market to "make sure it works".
//!
//! # Usage
//!
//! ```text
//! export SODEX_API_KEY_NAME=api-key-01
//! export SODEX_API_PRIVATE_KEY=<from register_api_key>
//! export SODEX_ACCOUNT_ID=60366
//! export SODEX_NETWORK=testnet
//! cargo run -p nautilus-sodex --example place_and_cancel
//! ```
//!
//! Optional overrides: `SODEX_SYMBOL_ID` (default 1, `vBTC_vUSDC`), `SODEX_LIMIT_PRICE`
//! (default 40000), `SODEX_QUANTITY` (default 0.001).

use std::env;

use nautilus_network::http::Method;
use nautilus_sodex::{
    common::{
        Market,
        credential::{ApiKeyName, ApiPrivateKey},
        enums::{OrderSide, TimeInForce},
    },
    http::{
        Network, OrderAck, SodexHttpClient, align_batch,
        requests::ClientOrderId,
        spot::{SpotCancelItem, SpotCancelOrderRequest, SpotNewOrderRequest, SpotOrderItem},
    },
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let key_hex =
        env::var("SODEX_API_PRIVATE_KEY").map_err(|_| "SODEX_API_PRIVATE_KEY is not set")?;
    let key_name = env::var("SODEX_API_KEY_NAME").unwrap_or_else(|_| "api-key-01".to_string());
    let account_id: u64 = env::var("SODEX_ACCOUNT_ID")
        .map_err(|_| "SODEX_ACCOUNT_ID is not set")?
        .parse()?;
    let network = match env::var("SODEX_NETWORK").as_deref() {
        Ok("mainnet") => Network::Mainnet,
        _ => Network::Testnet,
    };
    let symbol_id: u64 = env::var("SODEX_SYMBOL_ID")
        .unwrap_or_else(|_| "1".into())
        .parse()?;
    let price = env::var("SODEX_LIMIT_PRICE").unwrap_or_else(|_| "40000".into());
    let quantity = env::var("SODEX_QUANTITY").unwrap_or_else(|_| "0.001".into());

    let key = ApiPrivateKey::parse(&key_hex)?;
    let name = ApiKeyName::parse(&key_name)?;
    let client = SodexHttpClient::with_credentials(network, Market::Spot, name, &key)?;

    // A client order id must match ^[0-9a-zA-Z_-]{1,36}$, so the timestamp is used bare.
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_millis();
    let order_id_label = ClientOrderId::parse(format!("probe-{stamp}"))?;

    println!("network:  {network:?}   account: {account_id}   symbol: {symbol_id}");
    println!("resting buy {quantity} @ {price} (far below market; must not fill)");
    println!();

    // ---- place ----------------------------------------------------------------------
    let order = SpotOrderItem::limit(
        symbol_id,
        order_id_label.clone(),
        OrderSide::Buy,
        TimeInForce::Gtc,
        &price,
        &quantity,
    )?;
    let request = SpotNewOrderRequest::new(account_id, vec![order])?;
    let submitted = request.client_order_ids();

    let signed = client.build_signed(
        Method::POST,
        SpotNewOrderRequest::ENDPOINT,
        SpotNewOrderRequest::ACTION,
        &request,
    )?;
    println!("POST {}", signed.url);
    println!("body {}", signed.body_str());

    let acks: Vec<OrderAck> = client.send(signed).await?;
    let aligned = align_batch(&submitted, acks)?;

    let placed = match aligned.first() {
        Some(ack) if ack.is_success() => {
            println!("PLACED - venue order id {:?}", ack.order_id);
            ack.clone()
        }
        Some(ack) => {
            println!("REJECTED - code {} : {:?}", ack.code, ack.error);
            return Err(format!("order rejected: {:?}", ack.error).into());
        }
        None => return Err("venue returned no acknowledgement".into()),
    };

    let venue_order_id = placed
        .order_id
        .ok_or("accepted order carried no venue order id")?;

    // ---- cancel ---------------------------------------------------------------------
    // Always attempt the cancel: leaving a resting order behind is the one outcome this
    // program must not produce.
    println!();
    let cancel_label = ClientOrderId::parse(format!("cancel-{stamp}"))?;
    let cancel = SpotCancelOrderRequest::new(
        account_id,
        vec![SpotCancelItem::by_order_id(
            symbol_id,
            cancel_label,
            venue_order_id,
        )],
    )?;

    let signed = client.build_signed(
        Method::DELETE,
        SpotCancelOrderRequest::ENDPOINT,
        SpotCancelOrderRequest::ACTION,
        &cancel,
    )?;
    println!("DELETE {}", signed.url);
    println!("body   {}", signed.body_str());

    match client.send::<Vec<OrderAck>>(signed).await {
        Ok(acks) => match acks.first() {
            Some(ack) if ack.is_success() => {
                println!("CANCELLED - order {venue_order_id} removed");
                println!();
                println!("Full order contract verified: place, acknowledge, cancel.");
            }
            Some(ack) => {
                println!("CANCEL REJECTED - code {} : {:?}", ack.code, ack.error);
                println!();
                println!("!! ORDER {venue_order_id} MAY STILL BE RESTING - cancel it in the UI !!");
            }
            None => {
                println!("!! no cancel acknowledgement; check order {venue_order_id} in the UI !!");
            }
        },
        Err(e) => {
            println!("CANCEL FAILED: {e}");
            println!();
            println!("!! ORDER {venue_order_id} MAY STILL BE RESTING - cancel it in the UI !!");
            return Err(e.into());
        }
    }

    Ok(())
}
