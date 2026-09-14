//! Asks the venue whether an API key can be allowed to trade while being denied withdrawals.
//!
//! This adapter refuses that combination locally. `build_add_api_key` requires a permissioned key
//! to withhold `TRADE`, `CANCEL` or both, and the test covering it says "the venue does not support
//! this combination" - but that claim cites no source, and nothing has ever asked the venue. The
//! answer decides how much a delegated key can be narrowed:
//!
//! - **Accepted** - a key that trades but cannot move funds is registrable, the local guard is
//!   stricter than the venue, and every key handed to an unattended process should carry that mask.
//! - **Refused for the permissions** - the claim is confirmed and worth recording: at this venue
//!   any key that can trade can also withdraw, so the only controls left are expiry, revocation and
//!   keeping the key out of the process that trades.
//!
//! # Three steps, because a refusal on its own means nothing here
//!
//! The venue recovers a signer address from the digest and looks it up, so **a payload it did not
//! expect comes back as `API key not found`** - an error naming credentials for what is really a
//! mismatch. A refused mask and an unrecognized action type are therefore indistinguishable from
//! the message alone, and this venue has already charged once for that confusion.
//!
//! So:
//!
//! 1. **Control.** A mask the adapter considers supported (`cancel_only`, 13) is built by the
//!    library and actually sent. No permissioned key has ever been registered on this account, so
//!    this path is unproven - if it fails, the adapter's permissioned action type does not match
//!    the venue's, every permissioned registration is broken, and nothing below can be read as
//!    being about permissions.
//! 2. **Mirror.** Only then is the hand assembly used here compared with the library's, byte for
//!    byte, on that same supported mask and with the nonce the library chose. Relaxing the local
//!    guard on a hunch would have meant rewriting a test with no evidence either way, so the guard
//!    and its test are left alone and the request is assembled from public pieces instead.
//! 3. **Under test.** The combination the guard refuses locally is sent. After the first two
//!    steps, its answer is the venue speaking about the mask.
//!
//! **This registers a real key if the venue accepts it**, and revokes it immediately afterwards so
//! nothing is left behind. The keypair is generated here and its private half is never printed:
//! the point is the venue's answer, not a usable credential.
//!
//! Written as one `env` invocation rather than `export`: the key then lives only for this command,
//! instead of staying in the shell's environment for everything run afterwards and every child
//! process it spawns. The leading space keeps the line out of shell history where that is enabled.
//!
//! ```text
//!  env SODEX_MASTER_PRIVATE_KEY=<exported from Settings -> Export Email Wallet> \
//!      SODEX_ACCOUNT_ID=60366 \
//!      SODEX_MARKET=perps \
//!      cargo run -p nautilus-sodex --example probe_key_permissions
//! ```

use std::{collections::HashMap, env};

use nautilus_sodex::{
    common::{
        Market,
        credential::{ApiKeyName, MasterPrivateKey},
        enums::DisabledPermissions,
    },
    http::{
        Network, SignedRequest,
        account::{
            API_KEY_TYPE_EVM, AccountClient, AddApiKeyRequest, HEADER_API_CHAIN, NO_EXPIRY,
            generate_api_key,
        },
        client::{HEADER_API_NONCE, HEADER_API_SIGN},
    },
    signing::{NonceGenerator, UniversalSigner},
};

/// Everything a registration request needs except the two things under test.
///
/// Held together rather than passed each time, because the whole point is that two requests differ
/// only in their nonce and their mask: anything else differing would make the comparison below
/// meaningless.
struct Registration<'a> {
    signer: &'a UniversalSigner,
    template: &'a SignedRequest,
    network: Network,
    account_id: u64,
    name: &'a ApiKeyName,
    public_key: alloy_primitives::Address,
    expires_at: u64,
}

impl Registration<'_> {
    /// Assembles the request the way `AccountClient` does, for any mask including ones it refuses.
    fn request(
        &self,
        nonce: u64,
        mask: DisabledPermissions,
    ) -> Result<SignedRequest, Box<dyn std::error::Error>> {
        let signature = self.signer.sign_add_permissioned_api_key(
            self.network.chain_id(),
            nonce,
            self.account_id,
            self.name.as_str(),
            API_KEY_TYPE_EVM,
            self.public_key,
            self.expires_at,
            mask.as_mask(),
        )?;

        let body = AddApiKeyRequest {
            account_id: self.account_id,
            name: self.name.as_str().to_string(),
            key_type: API_KEY_TYPE_EVM,
            public_key: format!("{:#x}", self.public_key),
            expires_at: self.expires_at,
            permissions: Some(mask.as_mask()),
        };

        Ok(SignedRequest {
            // Method and url come from the library's own request, so neither is restated here.
            method: self.template.method.clone(),
            url: self.template.url.clone(),
            headers: HashMap::from([
                ("Content-Type".to_string(), "application/json".to_string()),
                ("Accept".to_string(), "application/json".to_string()),
                (
                    HEADER_API_SIGN.to_string(),
                    alloy_primitives::hex::encode_prefixed(&signature),
                ),
                (HEADER_API_NONCE.to_string(), nonce.to_string()),
                (
                    HEADER_API_CHAIN.to_string(),
                    self.signer.api_chain().to_string(),
                ),
            ]),
            body: serde_json::to_vec(&body)?,
        })
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let master_hex =
        env::var("SODEX_MASTER_PRIVATE_KEY").map_err(|_| "SODEX_MASTER_PRIVATE_KEY is not set")?;
    let account_id: u64 = env::var("SODEX_ACCOUNT_ID")
        .map_err(|_| "SODEX_ACCOUNT_ID is not set")?
        .parse()?;
    let network = match env::var("SODEX_NETWORK").as_deref() {
        Ok("mainnet") => Network::Mainnet,
        _ => Network::Testnet,
    };
    let market = match env::var("SODEX_MARKET").as_deref() {
        Ok("spot") => Market::Spot,
        _ => Market::Perps,
    };

    let master = MasterPrivateKey::parse(&master_hex)?;
    let account = AccountClient::new(network, market, &master)?;
    let signer = UniversalSigner::for_network(&master, network.chain_id())?;
    let nonces = NonceGenerator::new();

    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();
    let control_name = ApiKeyName::parse(&format!("permprobe-{stamp}-a"))?;
    let name = ApiKeyName::parse(&format!("permprobe-{stamp}-b"))?;
    let generated = generate_api_key()?;

    println!("network {network:?}  {market:?}  account {account_id}");
    println!("probe key address {:?}", generated.public_key);
    println!();

    // Step one: does a permissioned registration work at all? Nothing has ever sent one.
    let supported = DisabledPermissions::cancel_only();
    let control = account.build_add_api_key(
        account_id,
        &control_name,
        generated.public_key,
        NO_EXPIRY,
        Some(supported),
    )?;
    println!(
        "control: registering {} with mask {} (cancel only)",
        control_name.as_str(),
        supported.as_mask()
    );
    println!("body: {}", control.body_str());

    match account.send::<serde_json::Value>(control).await {
        Ok(response) => {
            println!("control ACCEPTED: {response:?}");
            let revoke = account.build_revoke_api_key(account_id, &control_name)?;
            let _: Option<serde_json::Value> = account.send(revoke).await?;
            println!("control key revoked; the permissioned path works, so read the test below");
        }
        Err(e) => {
            println!("control REFUSED: {e}");
            println!();
            println!(
                "A mask this adapter considers supported was refused, so the fault is not the"
            );
            println!("mask under test. Either the permissioned action type signed here does not");
            println!("match the venue's, or permissioned registration lives somewhere other than");
            println!("this path - and `DisabledPermissions` is unusable until that is settled.");
            println!("Nothing about withdrawal permissions can be concluded from this run.");
            return Ok(());
        }
    }
    println!();

    // Step two: the same supported mask, assembled here, must match the library byte for byte.
    let template = account.build_add_api_key(
        account_id,
        &name,
        generated.public_key,
        NO_EXPIRY,
        Some(supported),
    )?;
    let nonce: u64 = template.headers[HEADER_API_NONCE].parse()?;

    // Step two: the same request, assembled here, must match it exactly.
    let registration = Registration {
        signer: &signer,
        template: &template,
        network,
        account_id,
        name: &name,
        public_key: generated.public_key,
        expires_at: NO_EXPIRY,
    };
    let mirror = registration.request(nonce, supported)?;

    if mirror.body != template.body
        || mirror.headers[HEADER_API_SIGN] != template.headers[HEADER_API_SIGN]
    {
        println!("library body: {}", template.body_str());
        println!("probe body:   {}", mirror.body_str());
        return Err(
            "this program's assembly differs from the library's, so the venue's answer \
                    would say nothing about permissions - fix the assembly first"
                .into(),
        );
    }
    println!("assembly matches the library byte for byte on the supported mask");
    println!("and the venue accepted that mask above, so what follows is about the mask alone");
    println!();

    // Step three: the combination the guard refuses locally.
    let under_test = DisabledPermissions::none()
        .disabling(DisabledPermissions::WITHDRAW)
        .disabling(DisabledPermissions::TRANSFER);

    let request = registration.request(nonces.next(), under_test)?;

    println!(
        "mask under test: {} (WITHDRAW | TRANSFER withheld)",
        under_test.as_mask()
    );
    println!("body: {}", request.body_str());
    println!();

    match account.send::<serde_json::Value>(request).await {
        Ok(response) => {
            println!("ACCEPTED: {response:?}");
            println!();
            println!("The venue registers a key that may trade but may not withdraw or transfer.");
            println!("So the local guard is stricter than the venue: a delegated key should carry");
            println!("this mask, and `build_add_api_key` should stop refusing it.");

            let revoke = account.build_revoke_api_key(account_id, &name)?;
            let _: Option<serde_json::Value> = account.send(revoke).await?;
            println!();
            println!("Probe key revoked; nothing left registered.");
        }
        Err(e) => {
            println!("REFUSED: {e}");
            println!();
            println!("The control registered a key on this same path, with the same signing and");
            println!("the same key type, differing only in the mask - so this is the venue");
            println!("refusing this mask, whatever the message says. `API key not found` is how");
            println!("it reports a payload it did not expect, because it recovers a signer from");
            println!("the digest and looks that address up.");
            println!();
            println!("Meaning: either only certain masks are accepted, or none that leaves TRADE");
            println!("enabled is. Both come to the same thing for a delegated key - it cannot be");
            println!("narrowed to trade-without-withdraw, and expiry and revocation are the only");
            println!("controls left.");
        }
    }

    Ok(())
}
