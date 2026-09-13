//! Verifies the trading-domain signing path against the live venue.
//!
//! Sends a `scheduleCancel` with no timestamp, which clears any pending scheduled cancel.
//! If none is pending the call changes nothing, so this exercises signature verification,
//! nonce acceptance and header encoding without placing an order or touching a position.
//!
//! This is the Layer 3 gate: the venue publishes no expected signature bytes, so agreement
//! with the documentation cannot establish that a signature will be accepted. Only a live
//! request can.
//!
//! # Usage
//!
//! Written as one `env` invocation rather than `export`: the key then lives only for this command,
//! instead of staying in the shell's environment for everything run afterwards and every child
//! process it spawns. The leading space keeps the line out of shell history where that is enabled.
//!
//! ```text
//!  env SODEX_API_KEY_NAME=api-key-01 \
//!      SODEX_API_PRIVATE_KEY=<from register_api_key> \
//!      SODEX_ACCOUNT_ID=60366 \
//!      SODEX_NETWORK=testnet \
//!      cargo run -p nautilus-sodex --example verify_signing
//! ```

use std::env;

use nautilus_network::http::Method;
use nautilus_sodex::{
    common::{
        Market,
        credential::{ApiKeyName, ApiPrivateKey},
    },
    http::{Network, SodexHttpClient, requests::ScheduleCancelRequest},
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

    // Selectable because API keys are registered per engine: the same key can be known to
    // one engine and unknown to the other, and this probe is the cheapest way to tell which.
    let market = match env::var("SODEX_MARKET").as_deref() {
        Ok("spot") => Market::Spot,
        _ => Market::Perps,
    };

    let key = ApiPrivateKey::parse(&key_hex)?;
    let name = ApiKeyName::parse(&key_name)?;
    let client = SodexHttpClient::with_credentials(network, market, name, &key)?;

    let body = ScheduleCancelRequest::clear(account_id);
    let request = client.build_signed(
        Method::POST,
        ScheduleCancelRequest::ENDPOINT,
        ScheduleCancelRequest::ACTION,
        &body,
    )?;

    println!("network:   {network:?}");
    println!("market:    {market:?}");
    println!("account:   {account_id}");
    println!("key name:  {key_name}");
    println!("url:       {}", request.url);
    println!("body:      {}", request.body_str());
    println!("signature: {}...", &request.headers["X-API-Sign"][..12]);
    println!();

    // scheduleCancel documents no endpoint-specific payload, so an accepted request returns
    // an empty data field. Treating that as an error would report a working signature as a
    // failure - which is exactly what happened on the first run of this probe.
    match client.send_optional::<serde_json::Value>(request).await {
        Ok(data) => {
            println!("ACCEPTED - trading-domain signing verified end to end");
            println!("response data: {data:?}");
        }
        Err(e) => {
            // A rejection here is still informative: a signature error means the signing
            // path is wrong, while a business error means the signature was accepted and
            // the request merely had nothing to do.
            println!("REJECTED: {e}");
            println!();
            println!("If the message mentions a signature, signer or recovery id, the signing");
            println!("path is at fault. Anything else means the signature was accepted.");
            return Err(e.into());
        }
    }

    Ok(())
}
