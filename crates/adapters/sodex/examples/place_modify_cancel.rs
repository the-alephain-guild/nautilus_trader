//! Places a resting perps limit order, amends its price, and reads the venue back to prove it.
//!
//! The live tester only amends an order when the top of book drifts, so exercising the modify
//! route through it means waiting for the market to move and hoping it moves the right way. This
//! asks the question directly instead: place, amend, read back, cancel - no dependence on what
//! the market does while the program runs.
//!
//! **Perps only.** `POST /trade/orders/modify` answers 404 on spot, the mirror of spot's
//! batch-only routes.
//!
//! **This places a real order.** On testnet that is play money; the same program against
//! `SODEX_NETWORK=mainnet` would place a real one. Both prices sit far below the market so the
//! order rests rather than fills, and it is cancelled at the end either way.
//!
//! The amendment is verified by reading the account, not by trusting the acknowledgement: an
//! acknowledgement says the request was accepted, which is a different claim from the order now
//! resting at the new price.
//!
//! Written as one `env` invocation rather than `export`: the key then lives only for this command,
//! instead of staying in the shell's environment for everything run afterwards and every child
//! process it spawns. The leading space keeps the line out of shell history where that is enabled.
//!
//! ```text
//!  env SODEX_API_KEY_NAME=perps-key-01 \
//!      SODEX_API_PRIVATE_KEY=<key registered on the perps engine> \
//!      SODEX_ACCOUNT_ID=60366 \
//!      SODEX_WALLET_ADDRESS=0x766a478C89E5E9354b7a23922De18da6A5163b00 \
//!      cargo run -p nautilus-sodex --example place_modify_cancel
//! ```
//!
//! Optional overrides: `SODEX_SYMBOL_ID` (default 1, `BTC-USD`), `SODEX_LIMIT_PRICE` (default
//! 70000), `SODEX_MODIFIED_PRICE` (default 69000), `SODEX_QUANTITY` (default 0.0002).

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
        requests::{
            CancelItem, CancelOrderRequest, ClientOrderId, ModifyOrderRequest, NewOrderRequest,
            OrderItem,
        },
    },
};

/// Reads back the price the venue currently holds for one order.
async fn resting_price(
    client: &SodexHttpClient,
    wallet: &str,
    order_id: u64,
) -> Result<Option<String>, Box<dyn std::error::Error>> {
    let open = client.open_orders(wallet).await?;
    Ok(open
        .orders
        .into_iter()
        .find(|record| record.order_id == order_id)
        .map(|record| record.price))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let key_hex =
        env::var("SODEX_API_PRIVATE_KEY").map_err(|_| "SODEX_API_PRIVATE_KEY is not set")?;
    let key_name = env::var("SODEX_API_KEY_NAME").unwrap_or_else(|_| "perps-key-01".to_string());
    let account_id: u64 = env::var("SODEX_ACCOUNT_ID")
        .map_err(|_| "SODEX_ACCOUNT_ID is not set")?
        .parse()?;
    let wallet = env::var("SODEX_WALLET_ADDRESS").map_err(|_| "SODEX_WALLET_ADDRESS is not set")?;
    let symbol_id: u64 = env::var("SODEX_SYMBOL_ID")
        .unwrap_or_else(|_| "1".to_string())
        .parse()?;
    let quantity = env::var("SODEX_QUANTITY").unwrap_or_else(|_| "0.0002".to_string());
    let price = env::var("SODEX_LIMIT_PRICE").unwrap_or_else(|_| "70000".to_string());
    let amended = env::var("SODEX_MODIFIED_PRICE").unwrap_or_else(|_| "69000".to_string());
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
    let label = ClientOrderId::parse(format!("modprobe-{stamp}"))?;

    // Single variable between runs. The venue refused to amend a post-only order with
    // `OrderCannotBeModified` - a business error, so the request was understood and declined -
    // and post-only is the one thing that distinguishes it from an ordinary resting limit order.
    let time_in_force = match env::var("SODEX_TIME_IN_FORCE").as_deref() {
        Ok("gtc") => TimeInForce::Gtc,
        _ => TimeInForce::Gtx,
    };

    println!("{network:?} Perps  account {account_id}  symbol {symbol_id}  {time_in_force:?}");
    println!("placing {quantity} @ {price}, to be amended to {amended}");
    println!();

    let mut item = OrderItem::limit(
        label.clone(),
        OrderSide::Buy,
        time_in_force,
        price.clone(),
        quantity,
    )?;
    item.position_side = PositionSide::Both;

    let request = NewOrderRequest::new(account_id, symbol_id, vec![item])?;
    let submitted = request.client_order_ids();
    let signed = client.build_signed(
        Method::POST,
        NewOrderRequest::ENDPOINT,
        NewOrderRequest::ACTION,
        &request,
    )?;

    let acks: Vec<OrderAck> = client.send(signed).await?;
    let ack = align_batch(&submitted, acks)?
        .into_iter()
        .next()
        .ok_or("venue returned no acknowledgement")?;

    if !ack.is_success() {
        return Err(format!("placement refused - code {} : {:?}", ack.code, ack.error).into());
    }
    let order_id = ack.order_id.ok_or("accepted without an order id")?;
    println!("resting as order {order_id}");

    let before = resting_price(&client, &wallet, order_id)
        .await?
        .ok_or("the accepted order is not in the open set")?;
    println!("venue holds it at {before}");
    println!();

    // Quantity is left alone: amending one field at a time keeps the reading unambiguous when the
    // venue refuses, since only the price can be at fault.
    // Two more single variables, because the venue refuses with `OrderCannotBeModified` - a
    // business error, so the request is understood and declined. The SDK's only stated rules are
    // that exactly one identifier and at least one changed field must be present, and both hold
    // either way here.
    let by_client_id = env::var("SODEX_MODIFY_BY").as_deref() == Ok("cl_ord_id");
    let change_quantity = env::var("SODEX_MODIFY_FIELD").as_deref() == Ok("quantity");

    let modify = ModifyOrderRequest::new(
        account_id,
        symbol_id,
        if by_client_id { None } else { Some(order_id) },
        by_client_id.then(|| label.as_str().to_string()),
        (!change_quantity).then(|| amended.clone()),
        // A quantity change needs a value that differs from the resting one, or nothing is asked.
        change_quantity.then(|| "0.0003".to_string()),
        None,
    )?;
    println!("payload: {}", serde_json::to_string(&modify)?);

    let signed = client.build_signed(
        Method::POST,
        ModifyOrderRequest::ENDPOINT,
        ModifyOrderRequest::ACTION,
        &modify,
    )?;

    // `send_optional` because the venue answers an amend with an empty body on success, which the
    // execution client reads the same way.
    let outcome = client.send_optional::<Vec<OrderAck>>(signed).await;
    let after = resting_price(&client, &wallet, order_id).await?;
    if change_quantity {
        println!("(amending quantity, so the price read back is expected to be unchanged)");
    }

    // Cancelled before the verdict is reported, so a failed amendment does not leave the order
    // resting while the program exits.
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
    println!();

    match outcome? {
        Some(acks) => match acks.first() {
            Some(ack) if !ack.is_success() => {
                return Err(
                    format!("amendment refused - code {} : {:?}", ack.code, ack.error).into(),
                );
            }
            _ => println!("venue acknowledged the amendment"),
        },
        None => println!("venue acknowledged the amendment with an empty body"),
    }

    match after {
        Some(ref now) if now == &amended => {
            println!("venue now holds it at {now} - the amendment took effect");
            Ok(())
        }
        Some(now) => {
            Err(format!("the venue accepted the amendment but holds {now}, not {amended}").into())
        }
        None => Err("the order left the open set during the amendment".into()),
    }
}
