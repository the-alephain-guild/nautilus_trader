//! Asks the venue which stream channels exist, rather than guessing them into the adapter.
//!
//! The candle channel is the only one this adapter implements, because it is the only one
//! whose name and payload were documented. Order and fill streams would complete the
//! execution client, but a channel name invented here would fail at run time in a way that
//! reads like a credential problem - so this probe subscribes to candidate names and prints
//! exactly what the venue says about each.
//!
//! Read-only: a subscribe that is refused changes nothing, and a subscribe that is accepted
//! is dropped as soon as the probe ends.
//!
//! # What it found
//!
//! Run against testnet spot on 2026-09-10, sweeping 42 candidate names:
//!
//! - `candle`, `trade`, `ticker` exist. `trade` and `ticker` take `symbols` as an array,
//!   unlike `candle`, which takes one `symbol` plus an `interval`.
//! - `accountUpdate` exists but refused all eighteen parameter shapes tried as
//!   `invalid params`. Its selector is still unknown, which is why this adapter reports no
//!   fills - see the crate documentation.
//! - Everything else answered `unknown channel`, including `order`, `orders`, `fill`,
//!   `fills`, `position`, `positions`, `depth` and `orderbook`. The venue publishes no order
//!   book channel.
//!
//! The distinction that makes this work: the venue validates parameters *before* the channel
//! name, so `invalid params` means the name exists and `unknown channel` means it does not.
//!
//! Without `SODEX_CHANNEL` it sweeps channel names; with it, parameter shapes for that one
//! channel. "unknown channel" means the name does not exist; "invalid params" means it does
//! and the shape was wrong, which is the reply worth chasing.
//!
//! ```text
//! SODEX_ACCOUNT_ID=60366 cargo run -p nautilus-sodex --example probe_channels
//! SODEX_CHANNEL=accountUpdate cargo run -p nautilus-sodex --example probe_channels
//! ```

use std::{env, sync::Arc, time::Duration};

use nautilus_network::websocket::{
    WebSocketClient, WebSocketConfig, channel_epoch_message_handler,
};
use nautilus_sodex::{common::Market, http::Network, websocket::stream_url};
use tokio_tungstenite::tungstenite::Message;

/// Names to try, with the shape each is asked for.
const CANDIDATES: &[&str] = &[
    // Market data
    "candle",
    "trade",
    "ticker",
    "depth",
    "orderbook",
    "books",
    "book",
    "bbo",
    "level2",
    "markPrice",
    "indexPrice",
    "fundingRate",
    "funding",
    "liquidation",
    "openInterest",
    // Account and order streams
    "user",
    "userData",
    "userTrade",
    "userTrades",
    "userOrder",
    "userOrders",
    "private",
    "account",
    "accountUpdate",
    "balance",
    "balances",
    "wallet",
    "asset",
    "assets",
    "order",
    "orders",
    "openOrder",
    "openOrders",
    "orderUpdate",
    "orderStatus",
    "myTrade",
    "myTrades",
    "fill",
    "fills",
    "execution",
    "executions",
    "deal",
    "deals",
    "position",
    "positions",
    "positionUpdate",
    "margin",
    "notification",
    "activity",
];

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
    let account_id = env::var("SODEX_ACCOUNT_ID").unwrap_or_else(|_| "60366".to_string());
    let symbol = env::var("SODEX_SYMBOL").unwrap_or_else(|_| "vBTC_vUSDC".to_string());

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

    println!("probing {network:?} {market:?} with accountID={account_id} symbol={symbol}");
    println!();

    let account: u64 = account_id.parse()?;
    let mut id = 0_u64;
    let send = |id: &mut u64, params: serde_json::Value| {
        *id += 1;
        println!("{:>3} -> {params}", *id);
        serde_json::json!({ "op": "subscribe", "id": *id, "params": params }).to_string()
    };

    if let Ok(list) = env::var("SODEX_LISTEN") {
        // Subscribe to several channels at once and print their pushes interleaved, which is
        // how one channel's frames can be interpreted against another's.
        for channel in list.split(',') {
            client
                .send_text(
                    send(
                        &mut id,
                        serde_json::json!({ "channel": channel, "symbols": [symbol.clone()] }),
                    ),
                    None,
                )
                .await?;
        }
    } else if let Ok(channel) = env::var("SODEX_CHANNEL") {
        // Parameter sweep for one channel. "invalid params" means the venue knows the
        // channel and rejected the shape, which is the signal worth chasing.
        for extra in [
            serde_json::json!({}),
            serde_json::json!({ "symbols": [symbol.clone()] }),
            serde_json::json!({ "accountID": account }),
            serde_json::json!({ "accountIDs": [account] }),
            serde_json::json!({ "accountID": account, "symbols": [symbol.clone()] }),
            serde_json::json!({ "accountID": account, "symbol": symbol.clone() }),
            serde_json::json!({ "accountID": account, "coin": "USDC" }),
            serde_json::json!({ "accountID": account, "coins": ["USDC"] }),
            serde_json::json!({ "accountID": account, "type": "order" }),
            serde_json::json!({ "accountID": account, "types": ["order"] }),
            serde_json::json!({ "accountID": account, "market": "spot" }),
            serde_json::json!({ "accountID": account, "event": "order" }),
            serde_json::json!({ "accountID": account, "events": ["order"] }),
            serde_json::json!({ "accountID": account, "topics": ["order"] }),
            serde_json::json!({ "accountID": account, "topic": "order" }),
            serde_json::json!({ "userID": account }),
            serde_json::json!({ "uid": account }),
            serde_json::json!({ "subAccountID": account }),
            serde_json::json!({ "accountID": account, "pushInterval": "1000ms" }),
        ] {
            let mut params = serde_json::json!({ "channel": channel });
            if let (Some(target), Some(extra)) = (params.as_object_mut(), extra.as_object()) {
                for (key, value) in extra {
                    target.insert(key.clone(), value.clone());
                }
            }
            client.send_text(send(&mut id, params), None).await?;
        }
    } else {
        for channel in CANDIDATES {
            // The venue validates parameters before the channel name, so a shape it cannot
            // parse hides whether the channel exists. Four shapes are tried per name: keyed
            // by account (numeric, as the venue's own struct declares) and keyed by symbol,
            // singular as the candle channel uses and plural as `trade` asked for.
            for params in [
                serde_json::json!({ "channel": channel, "accountID": account }),
                serde_json::json!({ "channel": channel, "symbol": symbol }),
                serde_json::json!({ "channel": channel, "symbols": [symbol.clone()] }),
                serde_json::json!({
                    "channel": channel, "accountID": account, "symbols": [symbol.clone()]
                }),
            ] {
                client.send_text(send(&mut id, params), None).await?;
            }
        }
    }
    println!();

    // Collect for a few seconds; acks arrive out of order, so they are keyed by the id above.
    let listen_secs: u64 = env::var("SODEX_SECONDS")
        .unwrap_or_else(|_| "10".into())
        .parse()?;
    let deadline = tokio::time::sleep(Duration::from_secs(listen_secs));
    tokio::pin!(deadline);

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
                    println!("{text}");
                }
                Some(_) => {}
                None => break,
            },
        }
    }

    client.disconnect().await;
    Ok(())
}
