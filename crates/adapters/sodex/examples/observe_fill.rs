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
//! # The sell cannot be the same size as the buy
//!
//! A first version sold the quantity it had ordered and was rejected for insufficient balance. The
//! venue charges a buy's fee in the **base** asset, deducted from what arrives: ordering `0.001`
//! vBTC credits `0.00099935`. So the flattening sell reads the balance and sells what is actually
//! held, rounded down to the venue's step size — selling a hair more is the difference between
//! flattening and leaving a position behind.
//!
//! Set `SODEX_FLATTEN_ONLY=1` to skip the buy and only sell what the account already holds, which
//! is how a leftover from an earlier run gets cleaned up.
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
use rust_decimal::Decimal;
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
    let base_coin = env::var("SODEX_BASE_COIN").unwrap_or_else(|_| "vBTC".to_string());
    // The venue rejects a size off its step grid, so the held amount is rounded down to it.
    let step_size = env::var("SODEX_STEP_SIZE").unwrap_or_else(|_| "0.00001".to_string());

    let key = ApiPrivateKey::parse(&key_hex)?;
    let name = ApiKeyName::parse(&key_name)?;
    let client = SodexHttpClient::with_credentials(network, Market::Spot, name, &key)?;

    println!("network {network:?}  account {account_id}  symbol {symbol_id}  qty {quantity}");
    println!("placing a market BUY, then a market SELL to flatten");
    println!();

    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_millis();

    let flatten_only = env::var("SODEX_FLATTEN_ONLY").is_ok();

    let bought = if flatten_only {
        println!("SODEX_FLATTEN_ONLY set — skipping the buy");
        Ok(())
    } else {
        place_market(
            &client,
            account_id,
            symbol_id,
            OrderSide::Buy,
            &quantity,
            stamp,
        )
        .await
    };

    // Always attempt the flattening sell, whatever the buy reported: an unintended position is
    // the one outcome this program must not leave behind.
    //
    // And sell what is *held*, not what was ordered. A buy's fee comes out of the base asset it
    // credits, so the two differ and selling the ordered amount is rejected outright.
    let sold = {
        // The venue settles on-chain, so the balance needs a moment to reflect the buy.
        tokio::time::sleep(Duration::from_secs(3)).await;

        match held_base(&client, &wallet, &base_coin, &step_size).await {
            Ok(Some(sellable)) => {
                println!("holding {sellable} {base_coin} — selling that, not the ordered size");
                place_market(
                    &client,
                    account_id,
                    symbol_id,
                    OrderSide::Sell,
                    &sellable,
                    stamp + 1,
                )
                .await
            }
            Ok(None) => {
                println!("no sellable {base_coin} balance — nothing to flatten");
                Ok(())
            }
            Err(e) => {
                println!("!! could not read the balance to flatten: {e}");
                println!("!! CHECK FOR A LEFTOVER {base_coin} POSITION");
                Err(e)
            }
        }
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

/// The sellable balance of one coin, rounded **down** to the venue's step size.
///
/// Down, not nearest: rounding up asks to sell more than is held, which the venue rejects for
/// insufficient balance — the exact failure this function exists to avoid.
async fn held_base(
    client: &SodexHttpClient,
    wallet: &str,
    coin: &str,
    step_size: &str,
) -> Result<Option<String>, Box<dyn std::error::Error>> {
    let snapshot = client.account_balances(wallet).await?;
    let Some(balance) = snapshot.balances.iter().find(|entry| entry.coin == coin) else {
        return Ok(None);
    };

    let total: Decimal = balance.total.parse()?;
    let locked: Decimal = balance.locked.parse()?;
    let step: Decimal = step_size.parse()?;
    let free = total - locked;

    if step.is_zero() || free <= Decimal::ZERO {
        return Ok(None);
    }

    let steps = (free / step).floor();
    let sellable = steps * step;

    if sellable <= Decimal::ZERO {
        return Ok(None);
    }
    Ok(Some(sellable.normalize().to_string()))
}
