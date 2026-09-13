//! Opens one real perps position on testnet so the position wire shape can be read rather than
//! invented.
//!
//! `generate_position_status_reports` is the last unimplemented report on the perps engine, and it
//! cannot be written from the documentation: the positions endpoint answers an empty list on an
//! account that holds nothing, so its populated shape is unobservable until something is held.
//!
//! Typing it by analogy to the order or balance shape is the mistake this integration has already
//! paid for repeatedly - the perps balance carries `collateral` where spot carries `locked`, and
//! assuming otherwise failed the whole perps account read. So this causes a position and reads it.
//!
//! Both routes are printed, because they disagree in a way that matters: `/positions` answers
//! `{"positions": []}` when flat while `/state` answers `"P": null`, so whichever one the reports
//! are built on has to handle its own empty form.
//!
//! `SODEX_SIDE=sell` opens a short instead of a long, and that run is not optional. A long reports
//! `size` unsigned with `positionSide: "BOTH"`, which says nothing about direction - so how a short
//! is expressed cannot be inferred from a long, and Nautilus needs a signed quantity either way.
//!
//! **This places two real orders**: a minimum-size market buy to open and a reduce-only market
//! sell to close. On testnet that is play money. The close runs even when the read fails, because
//! leaving an unintended position behind is the one outcome this program must not produce.
//!
//! Set `SODEX_HOLD_ONLY=1` to skip the close and leave the position open for further inspection.
//! Whatever is left then has to be flattened by hand.
//!
//! Written as one `env` invocation rather than `export`: the key then lives only for this command,
//! instead of staying in the shell's environment for everything run afterwards and every child
//! process it spawns. The leading space keeps the line out of shell history where that is enabled.
//!
//! `SODEX_SYMBOL_ID=1` is BTC-USD; `SODEX_SIDE=sell` opens the short instead.
//!
//! ```text
//!  env SODEX_API_KEY_NAME=perps-key-01 \
//!      SODEX_API_PRIVATE_KEY=<key registered on the perps engine> \
//!      SODEX_ACCOUNT_ID=60366 \
//!      SODEX_WALLET_ADDRESS=0x766a478C89E5E9354b7a23922De18da6A5163b00 \
//!      SODEX_SYMBOL_ID=1 \
//!      SODEX_QUANTITY=0.0002 \
//!      SODEX_SIDE=buy \
//!      cargo run -p nautilus-sodex --example observe_position
//! ```

use std::{env, time::Duration};

use nautilus_network::http::Method;
use nautilus_sodex::{
    common::{
        Market,
        credential::{ApiKeyName, ApiPrivateKey},
        enums::{OrderSide, PositionSide},
    },
    http::{
        Network, OrderAck, SodexHttpClient, align_batch,
        requests::{ClientOrderId, NewOrderRequest, OrderItem},
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
    let wallet = env::var("SODEX_WALLET_ADDRESS").map_err(|_| "SODEX_WALLET_ADDRESS is not set")?;
    let network = match env::var("SODEX_NETWORK").as_deref() {
        Ok("mainnet") => Network::Mainnet,
        _ => Network::Testnet,
    };
    let symbol_id: u64 = env::var("SODEX_SYMBOL_ID")
        .unwrap_or_else(|_| "1".to_string())
        .parse()?;
    let quantity = env::var("SODEX_QUANTITY").unwrap_or_else(|_| "0.0002".to_string());
    let hold_only = env::var("SODEX_HOLD_ONLY").is_ok();
    let (open_side, close_side) = match env::var("SODEX_SIDE").as_deref() {
        Ok("sell") => (OrderSide::Sell, OrderSide::Buy),
        _ => (OrderSide::Buy, OrderSide::Sell),
    };

    let key = ApiPrivateKey::parse(&key_hex)?;
    let name = ApiKeyName::parse(&key_name)?;
    let client = SodexHttpClient::with_credentials(network, Market::Perps, name, &key)?;

    println!("network {network:?}  account {account_id}  symbol {symbol_id}  qty {quantity}");
    println!("opening with {open_side:?}, reading the position, then closing with {close_side:?}");
    println!();

    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_millis();

    let opened = place_market(
        &client, account_id, symbol_id, open_side, &quantity, false, stamp,
    )
    .await;

    // The venue settles on-chain, so the position needs a moment to appear.
    tokio::time::sleep(Duration::from_secs(4)).await;

    println!();
    println!("== raw /accounts/{{wallet}}/positions ==");
    print_raw(&client, &format!("/accounts/{wallet}/positions")).await;

    println!();
    println!("== raw /accounts/{{wallet}}/state ==");
    print_raw(&client, &format!("/accounts/{wallet}/state")).await;

    let closed = if hold_only {
        println!();
        println!("SODEX_HOLD_ONLY set - leaving the position open; flatten it by hand");
        Ok(())
    } else {
        println!();
        // Reduce-only so this can only ever shrink the position, never open the opposite one.
        place_market(
            &client,
            account_id,
            symbol_id,
            close_side,
            &quantity,
            true,
            stamp + 1,
        )
        .await
    };

    if !hold_only {
        tokio::time::sleep(Duration::from_secs(4)).await;
        println!();
        println!("== raw /accounts/{{wallet}}/positions after closing ==");
        print_raw(&client, &format!("/accounts/{wallet}/positions")).await;
    }

    opened?;
    closed?;
    Ok(())
}

/// Prints an unsigned account read verbatim, so the field names are the venue's own.
async fn print_raw(client: &SodexHttpClient, path: &str) {
    match client.get_public::<serde_json::Value>(path, None).await {
        Ok(value) => match serde_json::to_string_pretty(&value) {
            Ok(text) => println!("{text}"),
            Err(e) => println!("!! could not render {path}: {e}"),
        },
        Err(e) => println!("!! could not read {path}: {e}"),
    }
}

async fn place_market(
    client: &SodexHttpClient,
    account_id: u64,
    symbol_id: u64,
    side: OrderSide,
    quantity: &str,
    reduce_only: bool,
    stamp: u128,
) -> Result<(), Box<dyn std::error::Error>> {
    let label = ClientOrderId::parse(format!("posprobe-{stamp}"))?;
    let mut order = OrderItem::market(label, side, quantity);
    order.reduce_only = reduce_only;
    order.position_side = PositionSide::Both;

    let request = NewOrderRequest::new(account_id, symbol_id, vec![order])?;
    let submitted = request.client_order_ids();

    let signed = client.build_signed(
        Method::POST,
        NewOrderRequest::ENDPOINT,
        NewOrderRequest::ACTION,
        &request,
    )?;
    let acks: Vec<OrderAck> = client.send(signed).await?;

    match align_batch(&submitted, acks)?.first() {
        Some(ack) if ack.is_success() => {
            println!("{side:?} accepted - venue order id {:?}", ack.order_id);
            Ok(())
        }
        Some(ack) => {
            println!("{side:?} REJECTED - code {} : {:?}", ack.code, ack.error);
            Err(format!("{side:?} rejected: {:?}", ack.error).into())
        }
        None => Err("venue returned no acknowledgement".into()),
    }
}
