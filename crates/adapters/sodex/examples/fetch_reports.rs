//! Builds reconciliation reports from the live account, needing no credentials.
//!
//! The account reads are unsigned, so this verifies the whole reconciliation path - read, decode,
//! map, precision - against real venue responses without holding a key. What it cannot verify is
//! fills: an account that has never traded answers `[]`, and that is the one remaining gap.
//!
//! ```text
//! SODEX_WALLET_ADDRESS=0x... cargo run -p nautilus-sodex --example fetch_reports
//! ```

use std::env;

use nautilus_common::providers::InstrumentProvider;
use nautilus_core::UnixNanos;
use nautilus_model::{identifiers::AccountId, instruments::Instrument};
use nautilus_sodex::{
    common::Market,
    execution::{fill_report, order_status_report},
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
    let wallet = env::var("SODEX_WALLET_ADDRESS")
        .unwrap_or_else(|_| "0x766a478C89E5E9354b7a23922De18da6A5163b00".to_string());

    let client = SodexHttpClient::new_public(network, market)?;
    let mut provider = SodexInstrumentProvider::new(network, market)?;
    provider.load_all(None).await?;

    println!("{network:?} {market:?}  wallet {wallet}");
    println!();

    let balances = client.account_balances(&wallet).await?;
    println!(
        "balances at block {} ({} coins)",
        balances.block_height,
        balances.balances.len()
    );
    for balance in &balances.balances {
        // Free is derived: the locked part is reserved against open orders and is not spendable.
        println!(
            "  {:<8} total={:<16} locked={:<12}",
            balance.coin, balance.total, balance.locked
        );
    }
    println!();

    let open = client.open_orders(&wallet).await?;
    let history = client.order_history(&wallet).await?;
    println!(
        "orders: {} open, {} in history",
        open.orders.len(),
        history.len()
    );

    let account_id = AccountId::from(format!("{}-probe", provider.venue()));
    let mut built = 0_usize;

    for record in open.orders.iter().chain(history.iter()) {
        let instrument_id = instrument_id_for(&record.symbol, provider.venue());
        let Some(instrument) = provider.store().find(&instrument_id) else {
            println!("  {instrument_id} is not in the loaded set - skipped");
            continue;
        };

        match order_status_report(
            record,
            account_id,
            instrument_id,
            instrument.price_precision(),
            instrument.size_precision(),
            UnixNanos::default(),
        ) {
            Ok(report) => {
                built += 1;
                println!(
                    "  {:<12} {:?} {:?} qty={} filled={} px={:?} avg={:?} status={:?}",
                    record.cl_ord_id,
                    report.order_side,
                    report.order_type,
                    report.quantity,
                    report.filled_qty,
                    report.price,
                    report.avg_px,
                    report.order_status,
                );
            }
            Err(e) => println!("  order {} could not be reported: {e}", record.order_id),
        }
    }

    println!();
    let trades = client.account_trades(&wallet).await?;
    println!("fills: {} records", trades.len());
    for trade in &trades {
        let Some(instrument) = provider
            .store()
            .find(&instrument_id_for(&trade.symbol, provider.venue()))
        else {
            println!("  trade {} names an unloaded instrument", trade.trade_id);
            continue;
        };

        match fill_report(
            trade,
            account_id,
            instrument.id(),
            instrument.price_precision(),
            instrument.size_precision(),
            UnixNanos::default(),
        ) {
            // The liquidity side and the fee currency are the venue's own here, not inferred -
            // and the fee coin is the base asset on a buy, which is why it is printed.
            Ok(report) => println!(
                "  trade {:<10} {:?} {:?} qty={} px={} fee={} liquidity={:?}",
                report.trade_id,
                report.order_side,
                report.client_order_id.map(|id| id.to_string()),
                report.last_qty,
                report.last_px,
                report.commission,
                report.liquidity_side,
            ),
            Err(e) => println!("  trade {} could not be reported: {e}", trade.trade_id),
        }
    }

    println!();
    println!("built {built} order status reports from live venue responses");
    Ok(())
}
