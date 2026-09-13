//! Cancels every order resting on one engine, then reads the account again to prove they are gone.
//!
//! Written because stopping the live node left two orders resting on perps: the shutdown wrote no
//! log line and cancelled nothing, so the account needs a cleanup path that does not depend on the
//! node's shutdown working. The same request reaches the multi-item cancel route, which no program
//! had sent with more than one order in it.
//!
//! The two engines cancel through different routes, the mirror of the asymmetry in batch
//! submission: perps through `DELETE /trade/orders` with action `cancelOrder`, spot through
//! `/trade/orders/batch` with action `batchCancelOrder`, where each cancellation additionally
//! carries a client order id labeling the cancellation itself.
//!
//! **This cancels everything resting on the selected engine**, not only what one program placed.
//! It does not touch positions: a resting order and an open position are different things, and
//! flattening a position is a trade rather than a cleanup.
//!
//! The acknowledgement is not taken as proof. The venue is read a second time, and the program
//! exits non-zero if anything is still resting - an acknowledgement says the request was accepted,
//! not that the book is clear.
//!
//! Written as one `env` invocation rather than `export`: the key then lives only for this command,
//! instead of staying in the shell's environment for everything run afterwards and every child
//! process it spawns. The leading space keeps the line out of shell history where that is enabled.
//!
//! ```text
//!  env SODEX_API_KEY_NAME=perps-key-01 \
//!      SODEX_API_PRIVATE_KEY=<key registered on that engine> \
//!      SODEX_ACCOUNT_ID=60366 \
//!      SODEX_MARKET=perps \
//!      cargo run -p nautilus-sodex --example cancel_open_orders
//! ```

use std::{
    env,
    time::{SystemTime, UNIX_EPOCH},
};

use nautilus_common::providers::InstrumentProvider;
use nautilus_network::http::Method;
use nautilus_sodex::{
    common::{
        Market,
        credential::{ApiKeyName, ApiPrivateKey},
    },
    http::{
        Network, OrderAck, SodexHttpClient,
        requests::{CancelItem, CancelOrderRequest, ClientOrderId, MAX_BATCH},
        spot::{SpotCancelItem, SpotCancelOrderRequest},
    },
    providers::{SodexInstrumentProvider, instrument_id_for},
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let key_hex =
        env::var("SODEX_API_PRIVATE_KEY").map_err(|_| "SODEX_API_PRIVATE_KEY is not set")?;
    let account_id: u64 = env::var("SODEX_ACCOUNT_ID")
        .map_err(|_| "SODEX_ACCOUNT_ID is not set")?
        .parse()?;
    let market = match env::var("SODEX_MARKET").as_deref() {
        Ok("perps") => Market::Perps,
        _ => Market::Spot,
    };
    let key_name = env::var("SODEX_API_KEY_NAME").unwrap_or_else(|_| match market {
        Market::Perps => "perps-key-01".to_string(),
        Market::Spot => "api-key-01".to_string(),
    });
    let network = match env::var("SODEX_NETWORK").as_deref() {
        Ok("mainnet") => Network::Mainnet,
        _ => Network::Testnet,
    };
    let wallet = env::var("SODEX_WALLET_ADDRESS").map_err(|_| "SODEX_WALLET_ADDRESS is not set")?;

    let key = ApiPrivateKey::parse(&key_hex)?;
    let name = ApiKeyName::parse(&key_name)?;
    let client = SodexHttpClient::with_credentials(network, market, name, &key)?;

    // The numeric symbol id a cancel needs is not in the order record, which names the symbol in
    // text, so the instrument listing supplies the mapping.
    let mut provider = SodexInstrumentProvider::new(network, market)?;
    provider.load_all(None).await?;

    println!("{network:?} {market:?}  account {account_id}  wallet {wallet}");

    let open = client.open_orders(&wallet).await?;
    if open.orders.is_empty() {
        println!("nothing resting - no cancel sent");
        return Ok(());
    }

    println!("{} resting:", open.orders.len());
    let mut items = Vec::with_capacity(open.orders.len());
    for record in &open.orders {
        let instrument_id = instrument_id_for(&record.symbol, provider.venue());
        let Some(symbol_id) = provider.symbol_id(&instrument_id) else {
            return Err(format!("{} is not in the loaded instrument set", record.symbol).into());
        };
        println!(
            "  {:<34} {:?} {} @ {}  order id {}",
            record.cl_ord_id, record.side, record.orig_qty, record.price, record.order_id
        );
        items.push((symbol_id, record.order_id));
    }
    println!();

    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();

    for (batch, chunk) in items.chunks(MAX_BATCH).enumerate() {
        let acks: Vec<OrderAck> = match market {
            Market::Perps => {
                let cancels = chunk
                    .iter()
                    .map(|(symbol_id, order_id)| CancelItem::by_order_id(*symbol_id, *order_id))
                    .collect();
                let request = CancelOrderRequest::new(account_id, cancels)?;
                println!("payload: {}", serde_json::to_string(&request)?);
                let signed = client.build_signed(
                    Method::DELETE,
                    CancelOrderRequest::ENDPOINT,
                    CancelOrderRequest::ACTION,
                    &request,
                )?;
                client.send(signed).await?
            }
            Market::Spot => {
                let mut cancels = Vec::with_capacity(chunk.len());
                for (index, (symbol_id, order_id)) in chunk.iter().enumerate() {
                    let label = ClientOrderId::parse(format!("cancel-{stamp}-{batch}-{index}"))?;
                    cancels.push(SpotCancelItem::by_order_id(*symbol_id, label, *order_id));
                }
                let request = SpotCancelOrderRequest::new(account_id, cancels)?;
                println!("payload: {}", serde_json::to_string(&request)?);
                let signed = client.build_signed(
                    Method::DELETE,
                    SpotCancelOrderRequest::ENDPOINT,
                    SpotCancelOrderRequest::ACTION,
                    &request,
                )?;
                client.send(signed).await?
            }
        };

        for ack in &acks {
            if ack.is_success() {
                println!(
                    "  cancelled {:?} (order id {:?})",
                    ack.cl_ord_id, ack.order_id
                );
            } else {
                println!(
                    "  refused {:?} - code {} : {:?}",
                    ack.cl_ord_id, ack.code, ack.error
                );
            }
        }
    }
    println!();

    let remaining = client.open_orders(&wallet).await?;
    if remaining.orders.is_empty() {
        println!("verified against the venue: nothing resting");
        return Ok(());
    }

    for record in &remaining.orders {
        println!(
            "  still resting: {} {:?} {} @ {}",
            record.cl_ord_id, record.side, record.orig_qty, record.price
        );
    }
    Err(format!("{} order(s) survived the cancel", remaining.orders.len()).into())
}
