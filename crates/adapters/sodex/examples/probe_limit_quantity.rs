//! Asks the venue which part of a perps limit order it calls an invalid quantity.
//!
//! Every order the execution client sent on perps came back `quantity is invalid`, while a market
//! order of the same size had been accepted days earlier. Comparing the two payloads left exactly
//! two candidates, because four fields differ and only two of them can plausibly carry that error:
//!
//! ```text
//! LIMIT  : "type":1,"timeInForce":4,"price":"76763","quantity":"0.00020"
//! MARKET : "type":2,"timeInForce":3,                "quantity":"0.0002"
//! ```
//!
//! So either the trailing zero is refused - Nautilus formats a quantity at the instrument's
//! precision, the earlier market order carried a hand-written string - or limit orders have a
//! constraint market orders do not. One request separates them: send the limit order with the
//! untrailed string and see which way it goes.
//!
//! The order rests far from the market and is cancelled immediately afterwards, so nothing is
//! expected to fill. `SODEX_QUANTITY` is the variable under test; change only it between runs.
//!
//! Written as one `env` invocation rather than `export`: the key then lives only for this command,
//! instead of staying in the shell's environment for everything run afterwards and every child
//! process it spawns. The leading space keeps the line out of shell history where that is enabled.
//!
//! `SODEX_SYMBOL_ID=1` is BTC-USD. The limit price defaults to well below the market so a buy
//! cannot fill; `SODEX_LIMIT_PRICE` overrides it.
//!
//! ```text
//!  env SODEX_API_KEY_NAME=perps-key-01 \
//!      SODEX_API_PRIVATE_KEY=<key registered on the perps engine> \
//!      SODEX_ACCOUNT_ID=60366 \
//!      SODEX_SYMBOL_ID=1 \
//!      SODEX_QUANTITY=0.0002 \
//!      cargo run -p nautilus-sodex --example probe_limit_quantity
//! ```

use std::env;

use nautilus_network::http::Method;
use nautilus_sodex::{
    common::{
        Market,
        credential::{ApiKeyName, ApiPrivateKey},
        enums::{OrderSide, PositionSide, TimeInForce},
    },
    http::{
        Network, OrderAck, SodexHttpClient, align_batch,
        requests::{CancelItem, CancelOrderRequest, ClientOrderId, NewOrderRequest, OrderItem},
    },
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let key_hex =
        env::var("SODEX_API_PRIVATE_KEY").map_err(|_| "SODEX_API_PRIVATE_KEY is not set")?;
    let key_name = env::var("SODEX_API_KEY_NAME").unwrap_or_else(|_| "perps-key-01".to_string());
    let account_id: u64 = env::var("SODEX_ACCOUNT_ID")
        .map_err(|_| "SODEX_ACCOUNT_ID is not set")?
        .parse()?;
    let symbol_id: u64 = env::var("SODEX_SYMBOL_ID")
        .unwrap_or_else(|_| "1".to_string())
        .parse()?;
    let quantity = env::var("SODEX_QUANTITY").unwrap_or_else(|_| "0.0002".to_string());
    let price = env::var("SODEX_LIMIT_PRICE").unwrap_or_else(|_| "70000".to_string());
    let network = match env::var("SODEX_NETWORK").as_deref() {
        Ok("mainnet") => Network::Mainnet,
        _ => Network::Testnet,
    };

    let key = ApiPrivateKey::parse(&key_hex)?;
    let name = ApiKeyName::parse(&key_name)?;
    let client = SodexHttpClient::with_credentials(network, Market::Perps, name, &key)?;

    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_millis();
    let label = ClientOrderId::parse(format!("qtyprobe-{stamp}"))?;

    // Identical to what the execution client builds, except that the quantity string is the one
    // under test rather than whatever Nautilus formatted.
    let mut item = OrderItem::limit(
        label.clone(),
        OrderSide::Buy,
        TimeInForce::Gtx,
        price.clone(),
        quantity.clone(),
    )?;
    item.position_side = PositionSide::Both;

    let request = NewOrderRequest::new(account_id, symbol_id, vec![item])?;
    let submitted = request.client_order_ids();

    println!("quantity under test: {quantity:?}   limit price: {price}");
    println!("payload: {}", serde_json::to_string(&request)?);
    println!();

    let signed = client.build_signed(
        Method::POST,
        NewOrderRequest::ENDPOINT,
        NewOrderRequest::ACTION,
        &request,
    )?;

    match client.send::<Vec<OrderAck>>(signed).await {
        Ok(acks) => match align_batch(&submitted, acks)?.first() {
            Some(ack) if ack.is_success() => {
                println!("ACCEPTED - venue order id {:?}", ack.order_id);
                println!("so the trailing zero was the problem, not the order type");

                if let Some(order_id) = ack.order_id {
                    let cancel = CancelOrderRequest::new(
                        account_id,
                        vec![CancelItem::by_order_id(symbol_id, order_id)],
                    )?;
                    let signed = client.build_signed(
                        Method::DELETE,
                        CancelOrderRequest::ENDPOINT,
                        CancelOrderRequest::ACTION,
                        &cancel,
                    )?;
                    let _: Vec<OrderAck> = client.send(signed).await?;
                    println!("cancelled, nothing left resting");
                }
            }
            Some(ack) => {
                println!("REJECTED - code {} : {:?}", ack.code, ack.error);
                println!("so the quantity string is not the cause; look at the limit order itself");
            }
            None => println!("venue returned no acknowledgement"),
        },
        Err(e) => {
            println!("REJECTED at transport: {e}");
            println!("so the quantity string is not the cause; look at the limit order itself");
        }
    }

    Ok(())
}
