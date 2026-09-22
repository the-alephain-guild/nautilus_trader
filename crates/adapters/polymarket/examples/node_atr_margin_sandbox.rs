// -------------------------------------------------------------------------------------------------
//  Copyright (C) 2015-2026 Nautech Systems Pty Ltd. All rights reserved.
//  https://nautechsystems.io
//
//  Licensed under the GNU Lesser General Public License Version 3.0 (the "License");
//  You may not use this file except in compliance with the License.
//  You may obtain a copy of the License at https://www.gnu.org/licenses/lgpl-3.0.en.html
//
//  Unless required by applicable law or agreed to in writing, software
//  distributed under the License is distributed on an "AS IS" BASIS,
//  WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
//  See the License for the specific language governing permissions and
//  limitations under the License.
// -------------------------------------------------------------------------------------------------

//! Paper run of the ATR-normalized margin strategy against live venue data.
//!
//! Live market data drives a simulated matching engine, so orders are never sent to the
//! venue and no funds are at risk. This is the configuration for accumulating the
//! out-of-sample window count a decision needs, while also exercising the execution
//! assumptions a replay cannot: whether resting orders fill at all, and at what price.
//!
//! Run with:
//! ```text
//! cargo run -p nautilus-polymarket --example node_atr_margin_sandbox
//! ```

mod atr_margin;

use atr_margin::{AtrMarginBinary, AtrMarginBinaryConfig};
use log::LevelFilter;
use nautilus_common::{enums::Environment, logging::logger::LoggerConfig};
use nautilus_live::node::LiveNode;
use nautilus_model::{
    enums::{AccountType, BookType, OmsType},
    identifiers::{AccountId, StrategyId, TraderId},
    types::{Currency, Money, Quantity},
};
use nautilus_polymarket::{
    common::consts::{POLYMARKET, POLYMARKET_VENUE, PUSD},
    config::{
        PolymarketDataClientConfig, PolymarketInstrumentProviderConfig,
        PolymarketUpDownEventSlugConfig,
    },
    factories::PolymarketDataClientFactory,
};
use nautilus_sandbox::{SandboxExecutionClientConfig, SandboxExecutionClientFactory};
use nautilus_trading::strategy::StrategyConfig;
use rust_decimal::Decimal;

const TRADER_ID: &str = "ATR-MARGIN-PAPER-001";
const ACCOUNT_ID: &str = "POLYMARKET-PAPER-001";
const NODE_NAME: &str = "ATR-MARGIN-SANDBOX";
const STRATEGY_ID: &str = "ATR-MARGIN-BINARY-001";

/// Settlement feed symbol. Must match the feed the markets resolve against.
const REFERENCE_SYMBOL: &str = "btc/usd";
/// Venue TWAP window; the venue publishes 30 and 60.
const REFERENCE_WINDOW_SECONDS: u64 = 60;
/// Underlying asset code in the up/down slug prefix.
const ASSET: &str = "btc";
/// Market interval in minutes.
const INTERVAL_MINS: u64 = 5;
/// Rolling count of periods to keep loaded, so the next window is present before it opens.
const PERIODS: u64 = 4;

/// Polymarket rejects limit orders below five shares.
const ORDER_QTY: &str = "5";
const STARTING_BALANCE: f64 = 10_000.0;

/// Seconds before expiration at which the decision is taken.
///
/// This is the horizon the exploratory study found most sample-efficient: earlier
/// decisions saw a monotonically rising failure rate, and later ones priced the
/// remaining edge away.
const DECIDE_BEFORE_EXPIRY_SECS: u64 = 75;

/// Entry price ceiling.
///
/// The break-even win rate equals the entry price, so an entry at 0.95 must be right
/// 95% of the time merely to break even. Anything above this bound is refused.
const MAX_ENTRY_PRICE: f64 = 0.95;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();

    let trader_id = TraderId::from(TRADER_ID);
    let account_id = AccountId::from(ACCOUNT_ID);

    // The adapter builds the rolling up/down slugs itself, so each new five-minute
    // market is discovered without the strategy enumerating them.
    let event_slug_builder = PolymarketUpDownEventSlugConfig {
        assets: vec![ASSET.to_string()],
        interval_mins: INTERVAL_MINS,
        periods: PERIODS,
        start_offset_periods: 0,
    };
    let instrument_config = PolymarketInstrumentProviderConfig {
        event_slug_builder: Some(event_slug_builder),
        use_gamma_markets: true,
        ..Default::default()
    };
    let data_config = PolymarketDataClientConfig {
        instrument_config: Some(instrument_config),
        update_instruments_interval_mins: Some(1),
        ..Default::default()
    };

    let pusd = Currency::from(PUSD);
    let sandbox_config = SandboxExecutionClientConfig {
        account_id,
        venue: *POLYMARKET_VENUE,
        starting_balances: vec![Money::new(STARTING_BALANCE, pusd)],
        base_currency: Some(pusd),
        oms_type: OmsType::Netting,
        account_type: AccountType::Cash,
        default_leverage: Decimal::ONE,
        leverages: ahash::AHashMap::new(),
        book_type: BookType::L2_MBP,
        fee_model: None,
        fill_model: None,
        frozen_account: false,
        bar_execution: false,
        trade_execution: true,
        reject_stop_orders: true,
        support_gtd_orders: true,
        support_contingent_orders: false,
        use_position_ids: true,
        use_random_ids: false,
        use_reduce_only: false,
        // Honouring queue position is what makes a resting order's fill contingent on
        // the flow ahead of it, which is the assumption a replay cannot check.
        queue_position: true,
        liquidity_consumption: true,
        bar_adaptive_high_low_ordering: false,
        use_market_order_acks: false,
        oto_full_trigger: false,
        price_protection_points: 0,
    };

    let strategy_config = AtrMarginBinaryConfig::builder()
        .base(StrategyConfig {
            strategy_id: Some(StrategyId::from(STRATEGY_ID)),
            order_id_tag: Some("001".to_string()),
            use_uuid_client_order_ids: true,
            ..Default::default()
        })
        .reference_symbol(REFERENCE_SYMBOL.to_string())
        .reference_window_seconds(REFERENCE_WINDOW_SECONDS)
        .trade_size(Quantity::from(ORDER_QTY))
        .decide_before_expiry_secs(DECIDE_BEFORE_EXPIRY_SECS)
        .max_entry_price(MAX_ENTRY_PRICE)
        // Of the four rules only the thick-lead one had a positive point estimate; the
        // others were negative or indistinguishable, so they stay off by default.
        .thick_lead_only(true)
        .post_only(true)
        .scale_atr_by_remaining(true)
        .build();

    let log_config = LoggerConfig {
        stdout_level: LevelFilter::Info,
        ..Default::default()
    };

    let mut node = LiveNode::builder(trader_id, Environment::Sandbox)?
        .with_name(NODE_NAME.to_string())
        .with_logging(log_config)
        .with_load_state(false)
        .with_save_state(false)
        .add_data_client(
            None,
            Box::new(PolymarketDataClientFactory),
            Box::new(data_config),
        )?
        .add_simulated_exec_client(
            Some(POLYMARKET.to_string()),
            Box::new(SandboxExecutionClientFactory::new()),
            Box::new(sandbox_config),
        )?
        .with_delay_post_stop_secs(2)
        .build()?;

    node.add_strategy(AtrMarginBinary::from_config(strategy_config))?;
    node.run().await?;

    Ok(())
}
