//! Produces one real fill on testnet so the fill wire shape can be read rather than invented.
//!
//! The account trades endpoint is the only route to fill reporting, and fill reporting is what
//! stands between this adapter and unattended running. But the endpoint answers `[]` on an
//! account that has never traded, so its shape cannot be learned by reading it — only by causing
//! a fill and then reading it.
//!
//! Typing it by analogy to the order shape is the move this integration has already paid for
//! once: every contract detail it got wrong came from assuming one endpoint resembled another.
//!
//! **This places two real orders**: a minimum-size market buy and a market sell to flatten. On
//! testnet that is play money. Against `SODEX_NETWORK=mainnet` it would not be, which is why the
//! sell runs even when the read fails — leaving an unintended position behind is the one outcome
//! this program must not produce.
//!
//! ```text
//! export SODEX_API_KEY_NAME=api-key-01
//! export SODEX_API_PRIVATE_KEY=<registered key>
//! export SODEX_ACCOUNT_ID=60366
//! export SODEX_WALLET_ADDRESS=0x766a478C89E5E9354b7a23922De18da6A5163b00
//! cargo run -p nautilus-sodex --example observe_fill
//! ```

use std::{env, time::Duration};

use nautilus_network::http::Method;
use nautilus_sodex::{
    common::{
        Market,
        credential::{ApiKeyName, ApiPrivateKey},
        enums::OrderSide,
    },
    http::{
        Network, OrderAck, SodexHttpClient, align_batch,
        requests::ClientOrderId,
        spot::{SpotNewOrderRequest, SpotOrderItem},
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
    let wallet = env::var("SODEX_WALLET_ADDRESS")
        .map_err(|_| "SODEX_WALLET_ADDRESS is not set")?;
    let network = match env::var("SODEX_NETWORK").as_deref() {
        Ok("mainnet") => Network::Mainnet,
        _ => Network::Testnet,
    };
    let symbol_id: u64 = env::var("SODEX_SYMBOL_ID")
        .unwrap_or_else(|_| "1".into())
        .parse()?;
    let quantity = env::var("SODEX_QUANTITY").unwrap_or_else(|_| "0.001".into());

    let key = ApiPrivateKey::parse(&key_hex)?;
    let name = ApiKeyName::parse(&key_name)?;
    let client = SodexHttpClient::with_credentials(network, Market::Spot, name, &key)?;

    println!("network {network:?}  account {account_id}  symbol {symbol_id}  qty {quantity}");
    println!("placing a market BUY, then a market SELL to flatten");
    println!();

    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_millis();

    let bought = place_market(&client, account_id, symbol_id, OrderSide::Buy, &quantity, stamp)
        .await;

    // Always attempt the flattening sell, whatever the buy reported: an unintended position is
    // the one outcome this program must not leave behind.
    let sold = if bought.is_ok() {
        // The venue settles on-chain, so the balance needs a moment to reflect the buy.
        tokio::time::sleep(Duration::from_secs(3)).await;
        place_market(
            &client,
            account_id,
            symbol_id,
            OrderSide::Sell,
            &quantity,
            stamp + 1,
        )
        .await
    } else {
        println!("buy did not fill, so nothing to flatten");
        Ok(())
    };

    tokio::time::sleep(Duration::from_secs(3)).await;

    println!();
    println!("== raw /accounts/{{wallet}}/trades ==");
    let trades: serde_json::Value = client
        .get_public(&format!("/accounts/{wallet}/trades"), None)
        .await?;
    println!("{}", serde_json::to_string_pretty(&trades)?);

    println!();
    println!("== raw /accounts/{{wallet}}/balances ==");
    let balances: serde_json::Value = client
        .get_public(&format!("/accounts/{wallet}/balances"), None)
        .await?;
    println!("{}", serde_json::to_string_pretty(&balances)?);

    bought?;
    sold?;
    Ok(())
}

async fn place_market(
    client: &SodexHttpClient,
    account_id: u64,
    symbol_id: u64,
    side: OrderSide,
    quantity: &str,
    stamp: u128,
) -> Result<(), Box<dyn std::error::Error>> {
    let label = ClientOrderId::parse(format!("fillprobe-{stamp}"))?;
    let order = SpotOrderItem::market(symbol_id, label, side, quantity);
    let request = SpotNewOrderRequest::new(account_id, vec![order])?;
    let submitted = request.client_order_ids();

    let signed = client.build_signed(
        Method::POST,
        SpotNewOrderRequest::ENDPOINT,
        SpotNewOrderRequest::ACTION,
        &request,
    )?;
    let acks: Vec<OrderAck> = client.send(signed).await?;

    match align_batch(&submitted, acks)?.first() {
        Some(ack) if ack.is_success() => {
            println!("{side:?} accepted — venue order id {:?}", ack.order_id);
            Ok(())
        }
        Some(ack) => {
            println!("{side:?} REJECTED — code {} : {:?}", ack.code, ack.error);
            Err(format!("{side:?} rejected: {:?}", ack.error).into())
        }
        None => Err("venue returned no acknowledgement".into()),
    }
}
