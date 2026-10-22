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

//! Native instruments and reports derived from venue evidence, with stable trade IDs.

use crate::{
    config::PAPERTRADE,
    wire::{AccountSnapshot, CloseRecord, Market, VenuePosition},
};
use anyhow::{Result, ensure};
use nautilus_core::UnixNanos;
use nautilus_model::{
    enums::{
        CurrencyType, LiquiditySide, OrderSide, OrderStatus, OrderType, PositionSide, TimeInForce,
    },
    identifiers::{
        AccountId, ClientOrderId, InstrumentId, PositionId, Symbol, TradeId, VenueOrderId,
    },
    instruments::{CryptoPerpetual, InstrumentAny},
    reports::{FillReport, OrderStatusReport, PositionStatusReport},
    types::{AccountBalance, Currency, Money, Price, Quantity, fixed::FIXED_PRECISION},
};
use rust_decimal::{Decimal, RoundingStrategy};
use std::str::FromStr;

pub fn instrument_id(market: &Market) -> InstrumentId {
    InstrumentId::from(format!("{}-USD-PERP.{PAPERTRADE}", market.symbol))
}
pub fn position_id(position: &VenuePosition) -> PositionId {
    PositionId::from(format!("PT-{}", position.position_id))
}
fn trade_id(kind: &str, position: crate::wire::Raw, tx: Option<alloy_primitives::B256>) -> TradeId {
    let tx = tx.map(|tx| format!("{tx:#x}")).unwrap_or_default();
    let hash = alloy_primitives::keccak256(format!("papertrade:{kind}:{position}:{tx}"));
    TradeId::from(format!("PT{kind}-{}", &format!("{hash:x}")[..32]))
}
pub fn market_price(market: &Market, raw: crate::wire::Raw) -> Result<Decimal> {
    Ok(Decimal::from_str(&raw.to_string())? / Decimal::from_str(&market.price_scale.to_string())?)
}
pub fn quantity(position: &VenuePosition, market: &Market) -> Result<Quantity> {
    let price = market_price(market, position.entry_price)?;
    ensure!(price > Decimal::ZERO, "Invalid entry price");
    let base = position
        .margin
        .decimal()?
        .checked_mul(Decimal::from(position.leverage))
        .and_then(|n| n.checked_div(price))
        .ok_or_else(|| anyhow::anyhow!("Position quantity overflow"))?;
    let base = base.round_dp_with_strategy(u32::from(FIXED_PRECISION), RoundingStrategy::ToZero);
    ensure!(base > Decimal::ZERO, "Position below native precision");
    Ok(Quantity::from_decimal_dp(base, FIXED_PRECISION)?)
}
pub fn instrument(market: &Market, now: UnixNanos) -> Result<InstrumentAny> {
    let scale = market.price_scale.to_string();
    ensure!(
        scale.starts_with('1') && scale[1..].bytes().all(|b| b == b'0'),
        "Unsupported oracle scale"
    );
    let precision = u8::try_from(scale.len() - 1)?;
    let currency = Currency::try_from_str(&market.symbol).unwrap_or_else(|| {
        Currency::new(&market.symbol, 8, 0, &market.symbol, CurrencyType::Crypto)
    });
    let usdc = Currency::USDC();
    let tick = Price::from_decimal_dp(Decimal::new(1, u32::from(precision)), precision)?;
    let step =
        Quantity::from_decimal_dp(Decimal::new(1, u32::from(FIXED_PRECISION)), FIXED_PRECISION)?;
    Ok(CryptoPerpetual::builder()
        .instrument_id(instrument_id(market))
        .raw_symbol(Symbol::from(market.symbol.as_str()))
        .base_currency(currency)
        .quote_currency(usdc)
        .settlement_currency(usdc)
        .is_inverse(false)
        .price_precision(precision)
        .size_precision(FIXED_PRECISION)
        .price_increment(tick)
        .size_increment(step)
        .margin_init(Decimal::ONE / Decimal::from(market.max_leverage))
        .maker_fee(Decimal::ZERO)
        .taker_fee(Decimal::ZERO)
        .ts_event(now)
        .ts_init(now)
        .build()?
        .into())
}
pub fn open_fill(
    position: &VenuePosition,
    market: &Market,
    account: AccountId,
    client: Option<ClientOrderId>,
    now: UnixNanos,
) -> Result<FillReport> {
    let order_id = position.intent_id.map_or_else(
        || format!("PT-OPEN-{}", position.position_id),
        |id| format!("{id:#x}"),
    );
    let precision = u8::try_from(market.price_scale.to_string().len() - 1)?;
    Ok(FillReport::new(
        account,
        instrument_id(market),
        VenueOrderId::from(order_id),
        trade_id("O", position.position_id, None),
        if position.is_long {
            OrderSide::Buy
        } else {
            OrderSide::Sell
        },
        quantity(position, market)?,
        Price::from_decimal_dp(market_price(market, position.entry_price)?, precision)?,
        Money::from_decimal(Decimal::ZERO, Currency::USDC())?,
        LiquiditySide::Taker,
        client,
        Some(position_id(position)),
        UnixNanos::from(position.opened_at_ms * 1_000_000),
        now,
        None,
    ))
}
pub fn historical_position(record: &CloseRecord) -> VenuePosition {
    VenuePosition {
        position_id: record.position_id,
        instrument_id: record.instrument_id,
        is_long: record.is_long,
        use_debt: false,
        entry_price: record.entry_price_raw,
        margin: record.margin_raw,
        leverage: record.leverage,
        bust_price: Default::default(),
        opened_at_ms: record.opened_at_ms,
        intent_id: None,
    }
}
pub fn close_fill(
    record: &CloseRecord,
    market: &Market,
    account: AccountId,
    client: Option<ClientOrderId>,
    venue_id: Option<VenueOrderId>,
    now: UnixNanos,
) -> Result<FillReport> {
    let position = historical_position(record);
    let qty = quantity(&position, market)?;
    let entry = market_price(market, record.entry_price_raw)?;
    let exit = market_price(market, record.exit_price_raw)?;
    let gross = qty.as_decimal()
        * (exit - entry)
        * if record.is_long {
            Decimal::ONE
        } else {
            -Decimal::ONE
        };
    // The settlement adjustment includes asymmetric payoff deductions and liquidation
    // caps as well as explicit fees. Raw accounting remains in the source journal.
    let adjustment = gross - record.net_pnl()?;
    let precision = u8::try_from(market.price_scale.to_string().len() - 1)?;
    Ok(FillReport::new(
        account,
        instrument_id(market),
        venue_id.unwrap_or_else(|| VenueOrderId::from(format!("PT-CLOSE-{}", record.position_id))),
        trade_id("C", record.position_id, Some(record.tx_hash)),
        if record.is_long {
            OrderSide::Sell
        } else {
            OrderSide::Buy
        },
        qty,
        Price::from_decimal_dp(exit, precision)?,
        Money::from_decimal(adjustment, Currency::USDC())?,
        LiquiditySide::Taker,
        client,
        Some(position_id(&position)),
        UnixNanos::from(record.timestamp_ms * 1_000_000),
        now,
        None,
    ))
}
pub fn order_report(fill: &FillReport, now: UnixNanos) -> OrderStatusReport {
    OrderStatusReport::new(
        fill.account_id,
        fill.instrument_id,
        fill.client_order_id,
        fill.venue_order_id,
        Some(fill.order_side),
        OrderType::Market,
        TimeInForce::Gtc,
        OrderStatus::Filled,
        fill.last_qty,
        fill.last_qty,
        fill.ts_event,
        fill.ts_event,
        now,
        None,
    )
    .with_avg_px(fill.last_px.as_decimal())
}
pub fn position_report(
    p: &VenuePosition,
    m: &Market,
    account: AccountId,
    now: UnixNanos,
) -> Result<PositionStatusReport> {
    Ok(PositionStatusReport::new(
        account,
        instrument_id(m),
        if p.is_long {
            PositionSide::Long
        } else {
            PositionSide::Short
        },
        quantity(p, m)?,
        UnixNanos::from(p.opened_at_ms * 1_000_000),
        now,
        None,
        Some(position_id(p)),
        Some(market_price(m, p.entry_price)?),
    ))
}
pub fn balances(snapshot: &AccountSnapshot) -> Result<Vec<AccountBalance>> {
    let cash_locked = snapshot
        .open_positions
        .iter()
        .filter(|p| !p.use_debt)
        .try_fold(Decimal::ZERO, |sum, p| {
            Ok::<_, anyhow::Error>(sum + p.margin.decimal()?)
        })?;
    let total = (snapshot.balance.decimal()? + cash_locked)
        .round_dp_with_strategy(6, RoundingStrategy::ToZero);
    let free = snapshot
        .funds
        .available_balance
        .decimal()?
        .round_dp_with_strategy(6, RoundingStrategy::ToZero);
    let currency = Currency::USDC();
    Ok(vec![AccountBalance::new_checked(
        Money::from_decimal(total, currency)?,
        Money::from_decimal(total - free, currency)?,
        Money::from_decimal(free, currency)?,
    )?])
}
