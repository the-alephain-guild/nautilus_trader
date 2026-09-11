//! Streams live market data and reports what each frame is worth to a strategy.
//!
//! Needs no credentials - the venue serves market data unsigned. Subscribes to all three
//! channels this adapter implements and prints every push, marking which candles are safe to
//! trade on and which are still forming.
//!
//! It exercises the parts of the client that unit tests cannot: the connection, the venue's
//! acknowledgement shapes (which differ per channel), whether the keepalive is accepted, and
//! whether symbols come back spelled the way they were sent.
//!
//! # Usage
//!
//! ```text
//! cargo run -p nautilus-sodex --example stream_market_data
//! ```
//!
//! Optional overrides: `SODEX_NETWORK` (default testnet), `SODEX_MARKET` (`spot` or `perps`,
//! default spot), `SODEX_SYMBOL` (default `vBTC_vUSDC`), `SODEX_INTERVAL` (default `1m`),
//! `SODEX_SECONDS` (how long to listen, default 90).
//!
//! Trades are sparse on testnet. To see them, point it at mainnet - still a public read:
//!
//! ```text
//! SODEX_NETWORK=mainnet cargo run -p nautilus-sodex --example stream_market_data
//! ```

use std::{env, time::Duration};

use nautilus_sodex::{
    common::Market,
    http::Network,
    websocket::{SodexWebSocketClient, SodexWsEvent, Subscription},
};

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
    let symbol = env::var("SODEX_SYMBOL").unwrap_or_else(|_| "vBTC_vUSDC".to_string());
    let interval = env::var("SODEX_INTERVAL").unwrap_or_else(|_| "1m".to_string());
    let seconds: u64 = env::var("SODEX_SECONDS")
        .unwrap_or_else(|_| "90".into())
        .parse()?;

    let client = SodexWebSocketClient::new(network, market);
    println!("connecting to {network:?} {market:?} …");
    let mut events = client.connect().await?;
    println!("connected");

    let subscriptions = [
        Subscription::candles(&symbol, &interval),
        Subscription::trades(&symbol),
        Subscription::ticker(&symbol),
    ];
    for subscription in &subscriptions {
        client.subscribe(subscription.clone()).await?;
        println!("subscribed to {}", subscription.channel());
    }
    println!("listening for {seconds}s on {symbol}");
    println!();

    let deadline = tokio::time::sleep(Duration::from_secs(seconds));
    tokio::pin!(deadline);

    let (mut forming, mut closed, mut trades, mut tickers) = (0_u32, 0_u32, 0_u32, 0_u32);

    loop {
        tokio::select! {
            () = &mut deadline => break,
            event = events.recv() => match event {
                Some(SodexWsEvent::Candle(candle)) => {
                    if candle.is_final() {
                        closed += 1;
                        println!(
                            "CLOSED  {} {} open={} o={} h={} l={} c={} v={}",
                            candle.symbol, candle.interval, candle.open_time_ms,
                            candle.open, candle.high, candle.low, candle.close, candle.volume,
                        );
                    } else {
                        forming += 1;
                        println!(
                            "forming {} {} open={} c={} (bar still open - not tradable)",
                            candle.symbol, candle.interval, candle.open_time_ms, candle.close,
                        );
                    }
                }
                Some(SodexWsEvent::Trade(trade)) => {
                    trades += 1;
                    println!(
                        "trade   {} {:?} p={} q={} id={}",
                        trade.symbol, trade.side, trade.price, trade.quantity, trade.trade_id,
                    );
                }
                Some(SodexWsEvent::Ticker(ticker)) => {
                    tickers += 1;
                    println!(
                        "quote   {} bid={} x {} / ask={} x {}",
                        ticker.symbol,
                        ticker.bid_price, ticker.bid_quantity,
                        ticker.ask_price, ticker.ask_quantity,
                    );
                }
                Some(SodexWsEvent::RequestRejected { op, subscription, reason }) => {
                    println!("REJECTED {op:?} {subscription:?}: {reason}");
                }
                Some(SodexWsEvent::Reconnected) => {
                    println!("-- reconnected, subscriptions replayed --");
                }
                None => {
                    println!("stream ended");
                    break;
                }
            },
        }
    }

    println!();
    println!(
        "{forming} forming pushes, {closed} closed bars, {trades} trades, {tickers} quotes in \
         {seconds}s"
    );
    if closed == 0 {
        println!(
            "No bar carried the venue's closed flag. That is expected - across two full bar \
             periods on both engines it was never set, which is why the data client releases a \
             bar when its successor starts instead of waiting for the flag."
        );
    }

    for subscription in &subscriptions {
        client.unsubscribe(subscription).await?;
    }
    client.close().await;
    println!("closed");

    Ok(())
}
