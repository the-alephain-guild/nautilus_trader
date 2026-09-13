//! Loads instruments and fetches historical bars from the live venue.
//!
//! Exercises the whole read path end to end: symbol listing, instrument construction, the
//! interval mapping, the kline request and the bar conversion. Needs no credentials, since
//! the venue serves market data unsigned.
//!
//! ```text
//! cargo run -p nautilus-sodex --example fetch_bars
//! ```
//!
//! Optional: `SODEX_NETWORK` (default testnet), `SODEX_MARKET` (default spot),
//! `SODEX_SYMBOL` (default `vBTC_vUSDC`), `SODEX_INTERVAL` (default `1h`),
//! `SODEX_START_MINS` (fetch a window that many minutes back instead of a fixed count).

use std::env;

use nautilus_common::providers::InstrumentProvider;
use nautilus_model::{data::BarSpecification, instruments::Instrument};
use nautilus_sodex::{
    common::Market,
    data::{BarRequest, drop_forming_tail, fetch_bars, interval_to_spec},
    http::{Network, SodexHttpClient},
    providers::{SodexInstrumentProvider, instrument_id_for},
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
    let interval = env::var("SODEX_INTERVAL").unwrap_or_else(|_| "1h".to_string());

    println!("network {network:?}  market {market:?}  symbol {symbol}  interval {interval}");
    println!();

    // ---- instruments -----------------------------------------------------------------
    let mut provider = SodexInstrumentProvider::new(network, market)?;
    provider.load_all(None).await?;
    println!("loaded {} tradable instruments", provider.len());

    let instrument_id = instrument_id_for(&symbol, provider.venue());
    let symbol_id = provider.symbol_id(&instrument_id).ok_or_else(|| {
        // The two engines do not share symbol names, so "not listed" is most often a
        // spot name asked of perps. Showing what is listed turns that into one step.
        let listed: Vec<String> = provider
            .store()
            .get_all()
            .keys()
            .take(8)
            .map(|id| id.symbol.to_string())
            .collect();
        format!(
            "{symbol} not listed on {market:?}; listed here: {} …",
            listed.join(", ")
        )
    })?;
    println!("{instrument_id} maps to venue symbolID {symbol_id}");

    let instrument = provider
        .store()
        .find(&instrument_id)
        .ok_or("instrument missing from store")?;
    println!(
        "  price increment {}  size increment {}",
        instrument.price_increment(),
        instrument.size_increment()
    );
    println!();

    // ---- bars ------------------------------------------------------------------------
    let spec: BarSpecification = interval_to_spec(&interval)?;
    let client = SodexHttpClient::new_public(network, market)?;

    let start_ms = match env::var("SODEX_START_MINS") {
        Ok(mins) => {
            let mins: u64 = mins.parse()?;
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_millis() as u64;
            Some(now_ms.saturating_sub(mins * 60_000))
        }
        Err(_) => None,
    };

    let request = BarRequest {
        instrument_id,
        spec,
        // From the instrument, not from each kline's text: the engine rejects a bar whose four
        // prices disagree about scale, and this venue writes one tick size several ways.
        price_precision: instrument.price_precision(),
        size_precision: instrument.size_precision(),
        // `SODEX_START_MINS` asks for a window instead of a count, which is what a node's warmup
        // request looks like: a start time and no limit. Worth covering here, because the two
        // combinations do not behave the same on this venue.
        start_ms,
        end_ms: None,
        limit: if start_ms.is_some() { None } else { Some(10) },
    };

    // Already excludes the still-forming bar: `fetch_bars` drops it, since the venue does not flag
    // one and no caller can tell from the payload. Re-checking here proves that, rather than
    // assuming it.
    let closed = fetch_bars(&client, market, &request).await?;
    println!("venue returned {} closed bars", closed.len());

    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_millis() as u64;
    if drop_forming_tail(closed.clone(), &spec, now_ms).len() != closed.len() {
        return Err("fetch_bars returned a bar whose interval has not elapsed".into());
    }
    println!();

    for bar in closed.iter().rev().take(5).collect::<Vec<_>>().iter().rev() {
        println!(
            "  {}  O {}  H {}  L {}  C {}  V {}",
            bar.ts_event, bar.open, bar.high, bar.low, bar.close, bar.volume
        );
    }

    println!();
    println!("read path verified: symbols, instrument, interval mapping, klines, bars");

    // A sanity check the venue cannot get wrong but a mapping bug easily could. Reported as an
    // error rather than asserted: this is a probe, and a named failure reads better than a panic.
    for bar in &closed {
        let broken = if bar.high < bar.low {
            Some("high below low")
        } else if bar.high < bar.open || bar.high < bar.close {
            Some("high is not the maximum")
        } else if bar.low > bar.open || bar.low > bar.close {
            Some("low is not the minimum")
        } else {
            None
        };

        if let Some(reason) = broken {
            return Err(format!(
                "OHLC invariant violated at {}: {reason} (o={} h={} l={} c={})",
                bar.ts_event, bar.open, bar.high, bar.low, bar.close
            )
            .into());
        }
    }
    println!("OHLC invariants hold across all {} bars", closed.len());

    Ok(())
}
