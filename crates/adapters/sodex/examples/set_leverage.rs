//! Sets leverage and margin mode on one perps instrument, and optionally moves margin.
//!
//! Neither action has a Nautilus trait method - the `ExecutionClient` trait carries no notion of
//! leverage - so they are reached through the HTTP client, the way the API key actions are. This
//! program is how they get exercised.
//!
//! **Perps only.** Both routes answer `404` on spot, which is consistent: spot here holds balances
//! and carries neither leverage nor margin.
//!
//! **This changes account state.** Leverage applies to the instrument, not to one order, so it
//! affects every position opened afterwards. The venue may also refuse a reduction while a position
//! is open - that arrives as a venue error rather than as a local failure.
//!
//! `SODEX_MARGIN_AMOUNT` additionally moves margin against the instrument's isolated position. The
//! **sign convention is unobserved**: the SDK types the amount as a plain decimal and nothing says
//! whether a negative amount withdraws. Whoever runs this first should record what it does - that
//! is the one open question in this pair.
//!
//! Written as one `env` invocation rather than `export`: the key then lives only for this command,
//! instead of staying in the shell's environment for everything run afterwards and every child
//! process it spawns. The leading space keeps the line out of shell history where that is enabled.
//!
//! `SODEX_SYMBOL_ID=1` is BTC-USD, `SODEX_MARGIN_MODE` takes `cross` or `isolated`, and
//! `SODEX_MARGIN_AMOUNT` is optional - adding it moves margin as well as setting leverage.
//!
//! ```text
//!  env SODEX_API_KEY_NAME=perps-key-01 \
//!      SODEX_API_PRIVATE_KEY=<key registered on the perps engine> \
//!      SODEX_ACCOUNT_ID=60366 \
//!      SODEX_SYMBOL_ID=1 \
//!      SODEX_LEVERAGE=20 \
//!      SODEX_MARGIN_MODE=cross \
//!      cargo run -p nautilus-sodex --example set_leverage
//! ```

use std::env;

use nautilus_sodex::{
    common::{
        Market,
        credential::{ApiKeyName, ApiPrivateKey},
        enums::MarginMode,
    },
    http::{
        Network, SodexHttpClient,
        requests::{UpdateLeverageRequest, UpdateMarginRequest},
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
    let leverage: u32 = env::var("SODEX_LEVERAGE")
        .unwrap_or_else(|_| "20".to_string())
        .parse()?;
    let margin_mode = match env::var("SODEX_MARGIN_MODE").as_deref() {
        Ok("isolated") => MarginMode::Isolated,
        _ => MarginMode::Cross,
    };
    let network = match env::var("SODEX_NETWORK").as_deref() {
        Ok("mainnet") => Network::Mainnet,
        _ => Network::Testnet,
    };

    let key = ApiPrivateKey::parse(&key_hex)?;
    let name = ApiKeyName::parse(&key_name)?;
    let client = SodexHttpClient::with_credentials(network, Market::Perps, name, &key)?;

    println!("network {network:?}  account {account_id}  symbol {symbol_id}");
    println!("setting leverage {leverage}x in {margin_mode:?} margin mode");

    let request = UpdateLeverageRequest::new(account_id, symbol_id, leverage, margin_mode)?;
    println!("payload: {}", serde_json::to_string(&request)?);

    client.update_leverage(&request).await?;
    println!("accepted - leverage now applies to every position opened on this instrument");

    if let Ok(amount) = env::var("SODEX_MARGIN_AMOUNT") {
        println!();
        println!("moving margin by {amount}");

        let request = UpdateMarginRequest::new(account_id, symbol_id, amount)?;
        println!("payload: {}", serde_json::to_string(&request)?);

        client.update_margin(&request).await?;
        println!("accepted - record whether a negative amount withdraws, which is undocumented");
    }

    Ok(())
}
