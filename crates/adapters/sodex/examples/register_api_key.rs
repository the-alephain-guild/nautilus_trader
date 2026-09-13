//! Registers a fresh SoDEX API key using the master wallet.
//!
//! The venue's UI exposes no API key management, so registration goes through the
//! `addAPIKey` endpoint, which the master wallet must sign. This runs that once.
//!
//! # Usage
//!
//! Written as one `env` invocation rather than `export`: the key then lives only for this command,
//! instead of staying in the shell's environment for everything run afterwards and every child
//! process it spawns. The leading space keeps the line out of shell history where that is enabled.
//!
//! `SODEX_NETWORK` takes `testnet` or `mainnet`, `SODEX_MARKET` takes `perps` or `spot`, and the
//! account id is the `aid` from `/accounts/{address}/state`.
//!
//! ```text
//!  env SODEX_MASTER_PRIVATE_KEY=<exported from Settings -> Export Email Wallet> \
//!      SODEX_ACCOUNT_ID=60366 \
//!      SODEX_API_KEY_NAME=api-key-01 \
//!      SODEX_NETWORK=testnet \
//!      SODEX_MARKET=perps \
//!      cargo run -p nautilus-sodex --example register_api_key
//! ```
//!
//! # One key, both engines
//!
//! Spot and perps hold separate key sets, so a key registered on one is unknown to the
//! other. To use a single private key for both, run this twice with `SODEX_MARKET` set each
//! way; on the second run set `SODEX_API_PRIVATE_KEY` to the key from the first, and its
//! address is registered again rather than a new keypair being generated.
//!
//! A generated API private key is printed once and never written anywhere. Store it before
//! the terminal scrolls away; losing it means revoking the key and registering another.
//!
//! The master key is read from the environment and used only to sign this one action. It is
//! never logged, never sent (only the signature it produces is), and should go back offline
//! afterwards - it can authorize withdrawals, while the key being registered cannot even
//! read account data.

use std::env;

use nautilus_sodex::{
    common::{
        Market,
        credential::{ApiKeyName, ApiPrivateKey, MasterPrivateKey},
    },
    http::{
        Network,
        account::{AccountClient, NO_EXPIRY, generate_api_key},
    },
    signing::ExchangeSigner,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let master_hex =
        env::var("SODEX_MASTER_PRIVATE_KEY").map_err(|_| "SODEX_MASTER_PRIVATE_KEY is not set")?;
    let account_id: u64 = env::var("SODEX_ACCOUNT_ID")
        .map_err(|_| "SODEX_ACCOUNT_ID is not set")?
        .parse()?;
    let key_name = env::var("SODEX_API_KEY_NAME").unwrap_or_else(|_| "api-key-01".to_string());
    let network = match env::var("SODEX_NETWORK").as_deref() {
        Ok("mainnet") => Network::Mainnet,
        _ => Network::Testnet,
    };

    let master = MasterPrivateKey::parse(&master_hex)?;
    let name = ApiKeyName::parse(&key_name)?;

    // Each engine holds its own key set, so the market decides which set this registration
    // lands in - not merely which gateway path is used.
    let market = match env::var("SODEX_MARKET").as_deref() {
        Ok("spot") => Market::Spot,
        _ => Market::Perps,
    };
    let client = AccountClient::new(network, market, &master)?;

    println!("network:        {network:?}");
    println!("market:         {market:?}");
    println!("account id:     {account_id}");
    println!("master address: {:?}", client.master_address());
    println!("key name:       {key_name}");
    println!();

    // Reuse an existing key when one is supplied, so the same private key can be registered
    // on the second engine instead of ending up with a separate key per market.
    let existing = env::var("SODEX_API_PRIVATE_KEY").ok();
    let (public_key, generated_secret) = match existing {
        Some(hex) => {
            let key = ApiPrivateKey::parse(&hex)?;
            let address = ExchangeSigner::new(&key, market, network.chain_id())?.address();
            println!("reusing existing API key address: {address:?}");
            (address, None)
        }
        None => {
            let generated = generate_api_key()?;
            println!("generated API key address: {:?}", generated.public_key);
            (generated.public_key, Some(generated.private_key))
        }
    };

    let request = client.build_add_api_key(
        account_id, &name, public_key, NO_EXPIRY,
        None, // all permissions enabled; pass a DisabledPermissions mask to restrict
    )?;

    println!("submitting to {} ...", request.url);
    let response: Option<serde_json::Value> = client.send(request).await?;
    println!("venue accepted the registration: {response:?}");

    match generated_secret {
        Some(secret) => {
            println!();
            println!("=== store this now; it is not recoverable ===");
            println!("SODEX_API_KEY_NAME={key_name}");
            println!("SODEX_API_PRIVATE_KEY={}", secret.as_hex());
            println!("=== the master key can go back offline ===");
        }
        None => {
            println!();
            println!("Existing key registered on {market:?}; nothing new to store.");
        }
    }

    Ok(())
}
