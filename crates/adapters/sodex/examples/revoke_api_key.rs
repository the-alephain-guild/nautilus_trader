//! Revokes an API key with the master wallet, and proves from the venue that it stopped working.
//!
//! This is the emergency brake for a delegated key, and until now it existed only as code: the
//! request type and its builder are unit-tested for shape, but nothing had ever sent one. A brake
//! that has never been pulled is not a brake, so this exists to be exercised once on a throwaway
//! key before any key is handed to an unattended process.
//!
//! Revocation is the one action signed by the **master wallet** under the exchange domain: it sits
//! in the venue's trading-action list, yet it changes the API key set, so the key table requires
//! the owner. The master key is therefore needed here, used for this one action, and belongs back
//! offline afterwards - it can authorize withdrawals.
//!
//! # What it proves, and how
//!
//! Supplying `SODEX_API_PRIVATE_KEY` - the private key of the key being revoked - turns this into
//! a measurement rather than an assertion:
//!
//! 1. **Before.** A signed no-op is sent with that key. It must be accepted. Without this step the
//!    run proves nothing: a probe that fails for any other reason would read as a successful
//!    revocation.
//! 2. **Revoke.** Signed by the master wallet.
//! 3. **After.** The same no-op must now be refused. Retried briefly, because a key set the venue
//!    has just changed need not propagate instantly, and one immediate refusal-free attempt would
//!    be reported as a failed revocation.
//!
//! The no-op is `scheduleCancel` with no timestamp, which clears any pending dead-man schedule.
//! It costs one unit of weight, touches no order, and exercises the whole trading-domain signing
//! path - the cheapest request that can tell "this key works" from "this key does not".
//!
//! Omitting `SODEX_API_PRIVATE_KEY` still revokes, but then the venue's acknowledgement is the
//! only evidence, which is the weaker claim: it says the request was accepted, not that the key
//! is dead.
//!
//! **Revoking a key that something is using kills it mid-flight.** A running node signs every
//! order, cancel and amend with it.
//!
//! Written as one `env` invocation rather than `export`: the keys then live only for this command,
//! instead of staying in the shell's environment for everything run afterwards and every child
//! process it spawns. The leading space keeps the line out of shell history where that is enabled.
//!
//! ```text
//!  env SODEX_MASTER_PRIVATE_KEY=<exported from Settings -> Export Email Wallet> \
//!      SODEX_ACCOUNT_ID=60366 \
//!      SODEX_API_KEY_NAME=throwaway-key-01 \
//!      SODEX_API_PRIVATE_KEY=<the key being revoked, to prove it stops working> \
//!      SODEX_MARKET=perps \
//!      cargo run -p nautilus-sodex --example revoke_api_key
//! ```

use std::{env, time::Duration};

use nautilus_network::http::Method;
use nautilus_sodex::{
    common::{
        Market,
        credential::{ApiKeyName, ApiPrivateKey, MasterPrivateKey},
    },
    http::{Network, SodexHttpClient, account::AccountClient, requests::ScheduleCancelRequest},
};

/// How long the key may take to stop working after the venue accepts the revocation.
const PROPAGATION_ATTEMPTS: u32 = 6;
const PROPAGATION_WAIT: Duration = Duration::from_secs(1);

/// Sends the cheapest signed request that distinguishes a live key from a dead one.
async fn probe(client: &SodexHttpClient, account_id: u64) -> Result<(), String> {
    let body = ScheduleCancelRequest::clear(account_id);
    let request = client
        .build_signed(
            Method::POST,
            ScheduleCancelRequest::ENDPOINT,
            ScheduleCancelRequest::ACTION,
            &body,
        )
        .map_err(|e| e.to_string())?;

    client
        .send_optional::<serde_json::Value>(request)
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let master_hex =
        env::var("SODEX_MASTER_PRIVATE_KEY").map_err(|_| "SODEX_MASTER_PRIVATE_KEY is not set")?;
    let account_id: u64 = env::var("SODEX_ACCOUNT_ID")
        .map_err(|_| "SODEX_ACCOUNT_ID is not set")?
        .parse()?;
    let key_name = env::var("SODEX_API_KEY_NAME").map_err(|_| "SODEX_API_KEY_NAME is not set")?;
    let network = match env::var("SODEX_NETWORK").as_deref() {
        Ok("mainnet") => Network::Mainnet,
        _ => Network::Testnet,
    };
    // Each engine holds its own key set, so the market decides which set this revocation lands
    // in - a key revoked on perps is untouched on spot.
    let market = match env::var("SODEX_MARKET").as_deref() {
        Ok("spot") => Market::Spot,
        _ => Market::Perps,
    };

    let master = MasterPrivateKey::parse(&master_hex)?;
    let name = ApiKeyName::parse(&key_name)?;
    let account = AccountClient::new(network, market, &master)?;

    println!("network:        {network:?}");
    println!("market:         {market:?}");
    println!("account id:     {account_id}");
    println!("master address: {:?}", account.master_address());
    println!("revoking key:   {key_name}");
    println!();

    // Built before the revocation so the same client serves both probes, and so a malformed key
    // is reported before anything is changed at the venue.
    let probe_client = match env::var("SODEX_API_PRIVATE_KEY") {
        Ok(hex) => {
            let key = ApiPrivateKey::parse(&hex)?;
            Some(SodexHttpClient::with_credentials(
                network,
                market,
                name.clone(),
                &key,
            )?)
        }
        Err(_) => {
            println!("SODEX_API_PRIVATE_KEY is unset, so the venue's acknowledgement will be the");
            println!("only evidence - which says the request was accepted, not that the key is");
            println!("dead. Supply it to measure the revocation instead.");
            println!();
            None
        }
    };

    if let Some(ref client) = probe_client {
        match probe(client, account_id).await {
            Ok(()) => println!("before: the key works - so a later refusal will mean something"),
            Err(e) => {
                println!("before: the key is ALREADY refused - {e}");
                return Err(
                    "nothing to measure: the key did not work before the revocation, so \
                            its failure afterwards would prove nothing. Check the key name and \
                            engine, or revoke without SODEX_API_PRIVATE_KEY to skip the \
                            measurement."
                        .into(),
                );
            }
        }
    }

    let request = account.build_revoke_api_key(account_id, &name)?;
    println!();
    println!("{} {}", request.method, request.url);
    println!("body: {}", request.body_str());

    let response: Option<serde_json::Value> = account.send(request).await?;
    println!("venue accepted the revocation: {response:?}");
    println!();

    let Some(client) = probe_client else {
        println!("The master key can go back offline.");
        return Ok(());
    };

    for attempt in 1..=PROPAGATION_ATTEMPTS {
        match probe(&client, account_id).await {
            Err(e) => {
                println!("after:  the key is refused - {e}");
                println!();
                println!(
                    "Revocation verified end to end against the venue, in {attempt} attempt(s)."
                );
                println!("The master key can go back offline.");
                return Ok(());
            }
            Ok(()) => {
                println!(
                    "after:  attempt {attempt} still accepted; waiting for the key set to settle"
                );
                if attempt < PROPAGATION_ATTEMPTS {
                    tokio::time::sleep(PROPAGATION_WAIT).await;
                }
            }
        }
    }

    Err(format!(
        "the venue accepted the revocation but still honors {key_name} after \
         {PROPAGATION_ATTEMPTS} attempts - treat the key as live and revoke it through the venue's \
         own interface"
    )
    .into())
}
