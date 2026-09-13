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
//! # Why this does not go through `build_add_api_key`
//!
//! Relaxing the local guard on a hunch would weaken a deliberate constraint and force a rewrite of
//! the test asserting it, with no evidence either way. So the request is assembled here from the
//! same public pieces the library uses, and the guard and its test are left alone.
//!
//! That raises an obvious objection: if this program signs the request itself, a refusal might mean
//! the signature is wrong rather than the mask. So it first builds a request the guard *does* allow,
//! through the library, and checks its own assembly of that same request matches byte for byte -
//! body and signature both, using the nonce the library chose. Only then is the mask swapped. A
//! refusal after that is the venue answering about permissions, not about signing. Getting this
//! backwards is how `API key not found` once came to mean "your payload was malformed".
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
    let name = ApiKeyName::parse(&format!("permprobe-{stamp}"))?;
    let generated = generate_api_key()?;

    println!("network {network:?}  {market:?}  account {account_id}");
    println!("probe key name:   {}", name.as_str());
    println!("probe key address {:?}", generated.public_key);
    println!();

    // Step one: a mask the guard allows, built by the library.
    let supported = DisabledPermissions::cancel_only();
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
    println!("assembly matches the library byte for byte on a supported mask (mask 13)");
    println!("so a refusal below is the venue answering about permissions, not about signing");
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
            println!("If the message names permissions or the mask, the local guard is right and");
            println!("this venue offers no key that trades without also being able to withdraw -");
            println!("record that, because it decides what a delegated key can be narrowed to.");
            println!("If it names a signature or a key, stop: step two says otherwise, so read it");
            println!("again before concluding anything.");
        }
    }

    Ok(())
}
