//! Enumerates the venue's own subscription struct by provoking type errors.
//!
//! The account stream is the one thing standing between this adapter and unattended running:
//! without it the execution client never learns that an order filled. The channel exists —
//! `probe_channels` found `accountUpdate` — but eighteen guessed parameter shapes were all
//! refused as `invalid params`, which says nothing about *which* parameters it wants.
//!
//! This asks the venue instead of guessing again. Its gateway unmarshals into Go structs and
//! reports failures verbatim, naming the struct field and its type:
//!
//! ```text
//! json: cannot unmarshal string into Go struct field SubscriptionParams.accountID of type uint64
//! ```
//!
//! So a candidate field sent with a deliberately wrong type is a question the venue answers:
//!
//! - the error names `SubscriptionParams.<field>` → **the field exists**, and its Go type is
//!   stated outright;
//! - any other error → Go ignored the field, because `encoding/json` discards unknown keys, and
//!   the request fell through to the channel's own validation.
//!
//! Read-only: every request here is a malformed subscribe that the venue rejects. Nothing is
//! subscribed and no order is touched.
//!
//! Once the field set is known it is closed, so the second phase is no longer guessing: it
//! enumerates **every non-empty subset** of those fields with plausible values. 127 requests is
//! a bounded experiment that either finds the selector or proves no subset of the struct is one.
//!
//! ```text
//! cargo run -p nautilus-sodex --example probe_account_stream          # phase 1: which fields exist
//! SODEX_PHASE=subsets cargo run -p nautilus-sodex --example probe_account_stream
//! ```

use std::{collections::BTreeMap, env, sync::Arc, time::Duration};

use nautilus_network::websocket::{
    WebSocketClient, WebSocketConfig, channel_epoch_message_handler,
};
use nautilus_sodex::{common::Market, http::Network, websocket::stream_url};
use tokio_tungstenite::tungstenite::Message;

/// Field names to ask about. A wrong-typed value is sent for each.
const CANDIDATE_FIELDS: &[&str] = &[
    // Already known to exist, kept as the positive control: if this one stops being named, the
    // whole technique has broken and every "absent" below is meaningless.
    "accountID",
    "symbol",
    "symbols",
    "interval",
    "pushInterval",
    // Plausible account-stream selectors.
    "subAccountID",
    "userID",
    "uid",
    "address",
    "wallet",
    "apiKey",
    "token",
    "auth",
    "signature",
    "nonce",
    "topic",
    "topics",
    "event",
    "events",
    "type",
    "types",
    "category",
    "filter",
    "filters",
    "coin",
    "coins",
    "currency",
    "currencies",
    "market",
    "markets",
    "engine",
    "depth",
    "level",
    "limit",
    "snapshot",
];

/// Channels to ask the question of. `candle` is a second control: its required fields are known.
const CHANNELS: &[&str] = &["accountUpdate", "candle", "trade", "ticker"];

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let network = match env::var("SODEX_NETWORK").as_deref() {
        Ok("mainnet") => Network::Mainnet,
        _ => Network::Testnet,
    };
    let market = match env::var("SODEX_MARKET").as_deref() {
        Ok("perps") => Market::Perps,
        _ => Market::Spot,
    };

    let (handler, mut raw_rx) = channel_epoch_message_handler();
    let client = Arc::new(
        WebSocketClient::epoch_builder()
            .config(WebSocketConfig {
                url: stream_url(network, market),
                headers: Vec::new(),
                heartbeat_interval_secs: Some(20),
                heartbeat_payload: Some(r#"{"op":"ping"}"#.to_string()),
                connect_timeout_ms: None,
                reconnect_delay_initial_ms: None,
                reconnect_delay_max_ms: None,
                reconnect_backoff_factor: None,
                reconnect_jitter_ms: None,
                reconnect_max_attempts: None,
                heartbeat_timeout_secs: Some(40),
                idle_timeout_ms: None,
                backend: Default::default(),
                proxy_url: None,
            })
            .epoch_handler(handler)
            .connect()
            .await?,
    );

    println!("probing {network:?} {market:?}");
    println!();

    let subsets = env::var("SODEX_PHASE").as_deref() == Ok("subsets");
    let channel = env::var("SODEX_CHANNEL").unwrap_or_else(|_| "accountUpdate".to_string());
    let account: u64 = env::var("SODEX_ACCOUNT_ID")
        .unwrap_or_else(|_| "60366".into())
        .parse()?;
    let symbol = env::var("SODEX_SYMBOL").unwrap_or_else(|_| "vBTC_vUSDC".to_string());
    let quote_coin = env::var("SODEX_COIN").unwrap_or_else(|_| "vUSDC".to_string());

    // id -> a human-readable description of what was asked
    let mut asked: BTreeMap<u64, (String, String)> = BTreeMap::new();
    let mut id = 0_u64;

    if subsets {
        // The seven fields the venue named, each with a value it should accept. Enumerating all
        // non-empty subsets turns "which shape does it want" from guesswork into a finite search.
        let fields: [(&str, serde_json::Value); 7] = [
            ("accountID", serde_json::json!(account)),
            ("symbols", serde_json::json!([symbol.clone()])),
            ("symbol", serde_json::json!(symbol.clone())),
            ("coins", serde_json::json!([quote_coin.clone()])),
            ("interval", serde_json::json!("1m")),
            ("level", serde_json::json!(1)),
            ("pushInterval", serde_json::json!("1000ms")),
        ];

        for mask in 1u8..128 {
            let mut params = serde_json::Map::new();
            params.insert(
                "channel".to_string(),
                serde_json::Value::String(channel.clone()),
            );
            let mut label = Vec::new();
            for (bit, (name, value)) in fields.iter().enumerate() {
                if mask & (1 << bit) != 0 {
                    params.insert((*name).to_string(), value.clone());
                    label.push(*name);
                }
            }

            id += 1;
            asked.insert(id, (channel.clone(), label.join("+")));
            let request = serde_json::json!({
                "op": "subscribe",
                "id": id,
                "params": serde_json::Value::Object(params),
            });
            client.send_text(request.to_string(), None).await?;
        }
    } else {
        for channel in CHANNELS {
            for field in CANDIDATE_FIELDS {
                let mut params = serde_json::Map::new();
                params.insert(
                    "channel".to_string(),
                    serde_json::Value::String((*channel).to_string()),
                );
                // An object where any scalar, string or array is expected. Go cannot unmarshal
                // it into anything plausible, so a field that exists must be named in the error.
                params.insert((*field).to_string(), serde_json::json!({ "probe": true }));

                id += 1;
                asked.insert(id, ((*channel).to_string(), (*field).to_string()));
                let request = serde_json::json!({
                    "op": "subscribe",
                    "id": id,
                    "params": serde_json::Value::Object(params),
                });
                client.send_text(request.to_string(), None).await?;
            }
        }
    }

    let deadline = tokio::time::sleep(Duration::from_secs(
        env::var("SODEX_SECONDS")
            .unwrap_or_else(|_| "45".into())
            .parse()?,
    ));
    tokio::pin!(deadline);

    // channel -> field -> the venue's verdict
    let mut found: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    let mut other: BTreeMap<String, usize> = BTreeMap::new();
    let mut accepted: Vec<String> = Vec::new();
    let mut replies = 0_usize;

    loop {
        tokio::select! {
            () = &mut deadline => break,
            frame = raw_rx.recv() => match frame {
                Some((_, Message::Text(text))) => {
                    let value: serde_json::Value = match serde_json::from_str(&text) {
                        Ok(value) => value,
                        Err(_) => continue,
                    };
                    if value.get("op").and_then(serde_json::Value::as_str) == Some("pong") {
                        continue;
                    }
                    let Some(reply_id) = value.get("id").and_then(serde_json::Value::as_u64) else {
                        continue;
                    };
                    let Some((channel, field)) = asked.get(&reply_id) else {
                        continue;
                    };
                    replies += 1;
                    if value.get("success").and_then(serde_json::Value::as_bool) == Some(true) {
                        println!(
                            "ACCEPTED  {channel}  fields[{field}]  result={}",
                            value.get("result").map_or("null".to_string(), |r| r.to_string())
                        );
                        accepted.push(field.clone());
                        continue;
                    }

                    let error = value
                        .get("error")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("<no error text>");

                    // The field exists exactly when the venue names it in its own struct.
                    if let Some(rest) = error.split("SubscriptionParams.").nth(1) {
                        found
                            .entry(channel.clone())
                            .or_default()
                            .insert(field.clone(), rest.to_string());
                    } else {
                        *other.entry(error.to_string()).or_default() += 1;
                    }
                }
                Some(_) => {}
                None => break,
            },
        }
    }

    println!();
    println!("asked {} combinations, {replies} answered", asked.len());
    if subsets {
        println!();
        if accepted.is_empty() {
            println!(
                "== no subset of SubscriptionParams selects `{channel}` =="
            );
            println!(
                "   The field set is closed, so this is not an exhausted guess list: it is proof\n   \
                 that the selector needs something outside that struct — a different op, an\n   \
                 authenticated subscribe, or a field the gateway reads elsewhere."
            );
        } else {
            println!("== subsets the venue accepted ==");
            for label in &accepted {
                println!("   {label}");
            }
        }
    }
    println!();
    println!("== fields the venue names in SubscriptionParams ==");
    for (channel, fields) in &found {
        println!("  {channel}:");
        for (field, detail) in fields {
            println!("    {field:<16} {detail}");
        }
    }
    if found.is_empty() {
        println!("  none — the technique did not work here, so nothing below means 'absent'");
    }
    println!();
    println!("== other replies, by error text ==");
    for (error, count) in &other {
        println!("  {count:>3}  {error}");
    }

    client.disconnect().await;
    Ok(())
}
