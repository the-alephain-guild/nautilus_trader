//! Lists the API keys each engine holds for a wallet, needing no credentials.
//!
//! This is the verification tool for every key operation: a registration, a revocation or an
//! expiry is only confirmed when the venue's own list agrees. The read is unsigned, so it works
//! without holding any key at all - including after revoking the only key there was.
//!
//! Both engines are listed, because spot and perps keep **separate key sets** under one account
//! id. A key registered through one gateway is invisible to the other, which answers requests
//! signed by it with `API key not found` - an error that reads like a credential problem and is
//! really an engine mix-up.
//!
//! The raw response is printed alongside the parsed rows. The parsed form drops whatever this
//! adapter does not model, and one of the open questions about this venue is whether a key's
//! permission mask is visible at all: `ApiKeyEntry` has no field for it, so only the raw JSON can
//! say whether the venue withholds that information or the adapter merely ignores it.
//!
//! ```text
//! SODEX_WALLET_ADDRESS=0x... cargo run -p nautilus-sodex --example list_api_keys
//! ```

use std::env;

use nautilus_sodex::{
    common::Market,
    http::{Network, SodexHttpClient, account_reads::ApiKeyEntry},
};

/// Renders `expiresAt`, which the venue documents as Unix milliseconds with `0` meaning never.
fn expiry(expires_at_ms: u64) -> String {
    if expires_at_ms == 0 {
        return "never".to_string();
    }

    let now_ms = u64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis()),
    )
    .unwrap_or(u64::MAX);

    // Read as a duration from now rather than as a date, because that is what the open question
    // needs: a key set to expire in ten minutes that reads as decades away says the unit is not
    // milliseconds.
    if expires_at_ms > now_ms {
        format!(
            "{}s from now ({expires_at_ms})",
            (expires_at_ms - now_ms) / 1000
        )
    } else {
        format!("{}s ago ({expires_at_ms})", (now_ms - expires_at_ms) / 1000)
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let network = match env::var("SODEX_NETWORK").as_deref() {
        Ok("mainnet") => Network::Mainnet,
        _ => Network::Testnet,
    };
    let wallet = env::var("SODEX_WALLET_ADDRESS")
        .unwrap_or_else(|_| "0x766a478C89E5E9354b7a23922De18da6A5163b00".to_string());

    println!("{network:?}  wallet {wallet}");

    for market in [Market::Spot, Market::Perps] {
        let client = SodexHttpClient::new_public(network, market)?;
        println!();
        println!("=== {market:?} ===");

        let keys: Vec<ApiKeyEntry> = client.api_keys(&wallet).await?;
        if keys.is_empty() {
            println!("  no keys registered");
        }
        for key in &keys {
            println!(
                "  {:<28} {:<4} {}  expires {}",
                key.name,
                key.key_type,
                key.public_key,
                expiry(key.expires_at_ms)
            );
        }

        let raw: serde_json::Value = client
            .get_public(&format!("/accounts/{wallet}/api-keys"), None)
            .await?;
        println!("  raw: {raw}");
    }

    Ok(())
}
