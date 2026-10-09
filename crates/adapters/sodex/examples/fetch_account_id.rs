//! Looks up the account id a wallet holds at the venue, needing no credentials.
//!
//! Every signed request carries the numeric account id, `SODEX_ACCOUNT_ID`, which the venue assigns
//! to a wallet. The account state read is unsigned, so the id can be found from the wallet address
//! alone - before any API key exists, which is when registering the first one needs it.
//!
//! Both engines are asked. API keys are registered per engine, which invites the assumption that
//! the account id is too; the venue has answered the same id on both for one wallet. Asking both
//! shows that still holds, and the program refuses to print an id if it ever stops holding.
//!
//! Unlike `list_api_keys`, the wallet has no default. The id printed here is what later signed
//! requests are built with, and an id read from some other wallet is the wrong id, not a sample.
//!
//! ```text
//! SODEX_WALLET_ADDRESS=0x... cargo run -p nautilus-sodex --example fetch_account_id
//! ```

use std::env;

use nautilus_sodex::{
    common::Market,
    http::{Network, SodexHttpClient},
};

async fn account_id(
    network: Network,
    market: Market,
    wallet: &str,
) -> Result<u64, Box<dyn std::error::Error>> {
    let client = SodexHttpClient::new_public(network, market)?;
    let state: serde_json::Value = client
        .get_public(&format!("/accounts/{wallet}/state"), None)
        .await?;
    let aid = state
        .get("aid")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| format!("{market:?} account state carries no numeric aid: {state}"))?;
    Ok(aid)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let network = match env::var("SODEX_NETWORK").as_deref() {
        Ok("mainnet") => Network::Mainnet,
        _ => Network::Testnet,
    };
    let wallet = env::var("SODEX_WALLET_ADDRESS").map_err(|_| "SODEX_WALLET_ADDRESS is not set")?;

    println!("{network:?}  wallet {wallet}");
    println!();

    let spot = account_id(network, Market::Spot, &wallet).await?;
    println!("Spot   aid {spot}");
    let perps = account_id(network, Market::Perps, &wallet).await?;
    println!("Perps  aid {perps}");

    if spot != perps {
        return Err(format!(
            "the engines disagree on the account id (spot {spot}, perps {perps}); \
             resolve which one signed requests expect before using either"
        )
        .into());
    }

    println!();
    println!("SODEX_ACCOUNT_ID={spot}");
    Ok(())
}
