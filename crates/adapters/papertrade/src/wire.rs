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

//! Exact wire types for observed public account, market and history responses.

use std::fmt;

use alloy_primitives::{Address, B256, U256};
use anyhow::{Result, ensure};
use rust_decimal::Decimal;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

/// A canonical decimal uint256. JSON numbers and hexadecimal amounts are rejected.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Raw(pub U256);

impl Raw {
    pub fn decimal(self) -> Result<Decimal> {
        scaled_decimal(&self.to_string(), 18)
    }
}
impl fmt::Display for Raw {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
impl Serialize for Raw {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}
impl<'de> Deserialize<'de> for Raw {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let text = String::deserialize(d)?;
        if text.is_empty()
            || (text.len() > 1 && text.starts_with('0'))
            || !text.bytes().all(|b| b.is_ascii_digit())
        {
            return Err(serde::de::Error::custom(
                "Expected a canonical decimal uint256",
            ));
        }
        U256::from_str_radix(&text, 10)
            .map(Self)
            .map_err(serde::de::Error::custom)
    }
}

pub fn scaled_decimal(raw: &str, scale: usize) -> Result<Decimal> {
    let (negative, digits) = raw.strip_prefix('-').map_or((false, raw), |s| (true, s));
    ensure!(
        !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()),
        "Invalid amount"
    );
    ensure!(
        digits == "0" || !digits.starts_with('0'),
        "Noncanonical amount"
    );
    ensure!(!(negative && digits == "0"), "Noncanonical signed zero");
    let digits = format!("{digits:0>width$}", width = scale + 1);
    let split = digits.len() - scale;
    Ok(Decimal::from_str_exact(&format!(
        "{}{}.{}",
        if negative { "-" } else { "" },
        &digits[..split],
        &digits[split..]
    ))?)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(try_from = "Value", into = "Value")]
pub struct VenuePosition {
    pub position_id: Raw,
    pub instrument_id: u32,
    pub is_long: bool,
    pub use_debt: bool,
    pub entry_price: Raw,
    pub margin: Raw,
    pub leverage: u32,
    pub bust_price: Raw,
    pub opened_at_ms: u64,
    pub intent_id: Option<B256>,
}
type PositionTuple = (Raw, u32, bool, bool, Raw, Raw, u32, Raw, u64, Option<B256>);
impl TryFrom<Value> for VenuePosition {
    type Error = anyhow::Error;
    fn try_from(v: Value) -> Result<Self> {
        let (
            position_id,
            instrument_id,
            is_long,
            use_debt,
            entry_price,
            margin,
            leverage,
            bust_price,
            opened_at_ms,
            intent_id,
        ): PositionTuple = serde_json::from_value(v)?;
        ensure!(
            position_id.0 > U256::ZERO
                && entry_price.0 > U256::ZERO
                && margin.0 > U256::ZERO
                && leverage > 0,
            "Invalid position"
        );
        Ok(Self {
            position_id,
            instrument_id,
            is_long,
            use_debt,
            entry_price,
            margin,
            leverage,
            bust_price,
            opened_at_ms,
            intent_id,
        })
    }
}
impl From<VenuePosition> for Value {
    fn from(p: VenuePosition) -> Self {
        serde_json::json!([
            p.position_id,
            p.instrument_id,
            p.is_long,
            p.use_debt,
            p.entry_price,
            p.margin,
            p.leverage,
            p.bust_price,
            p.opened_at_ms,
            p.intent_id
        ])
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(try_from = "Value", into = "Value")]
pub struct Operation {
    pub intent_id: B256,
    pub action: String,
    pub wire: Value,
}
impl TryFrom<Value> for Operation {
    type Error = anyhow::Error;
    fn try_from(v: Value) -> Result<Self> {
        let a = v
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("Invalid operation"))?;
        ensure!(a.len() == 10, "Invalid operation tuple length");
        let intent_id = serde_json::from_value(a[0].clone())?;
        let action = a[1]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Invalid operation action"))?
            .to_owned();
        Ok(Self {
            intent_id,
            action,
            wire: v,
        })
    }
}
impl From<Operation> for Value {
    fn from(p: Operation) -> Self {
        p.wire
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Funds {
    pub reserved_balance: Raw,
    pub reserved_queued: Raw,
    pub available_balance: Raw,
    pub available_queued: Raw,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountSnapshot {
    pub address: Address,
    pub account_revision: u64,
    pub indexed_through_block: u64,
    pub indexed_through_timestamp: u64,
    pub wallet_revision: u64,
    pub queue_revision: Raw,
    pub session_keys: Vec<(Address, Raw)>,
    pub balance: Raw,
    pub queued: Raw,
    pub locked: Raw,
    pub consumable_debt: Raw,
    pub funds: Funds,
    pub open_positions: Vec<VenuePosition>,
    pub operations: Vec<Operation>,
    pub lifetime: Value,
    pub paper: Value,
    pub queue_node_count: u64,
    #[serde(default)]
    pub intent_outcomes: Vec<(B256, String)>,
}
impl AccountSnapshot {
    pub fn validate(&self, wallet: Address) -> Result<()> {
        ensure!(self.address == wallet, "Account wallet mismatch");
        ensure!(
            self.account_revision > 0 && self.wallet_revision <= self.indexed_through_block,
            "Invalid account revision"
        );
        ensure!(
            self.indexed_through_timestamp <= u64::MAX / 1_000_000_000,
            "Account timestamp overflow"
        );
        let mut ids = std::collections::HashSet::new();
        let mut locked = U256::ZERO;
        for p in &self.open_positions {
            ensure!(
                p.opened_at_ms <= self.indexed_through_timestamp * 1000
                    && p.opened_at_ms <= u64::MAX / 1_000_000,
                "Position timestamp exceeds coverage"
            );
            ensure!(ids.insert(p.position_id), "Duplicate venue position");
            locked = locked
                .checked_add(p.margin.0)
                .ok_or_else(|| anyhow::anyhow!("Locked overflow"))?;
        }
        ensure!(
            locked == self.locked.0,
            "Locked margin disagrees with positions"
        );
        ensure!(
            self.funds.available_balance.0
                == self.balance.0.saturating_sub(self.funds.reserved_balance.0)
                && self.funds.available_queued.0
                    == self.queued.0.saturating_sub(self.funds.reserved_queued.0),
            "Available funds disagree with reservations"
        );
        let mut keys = std::collections::HashSet::new();
        ensure!(
            self.session_keys.iter().all(|(key, _)| keys.insert(*key)),
            "Duplicate session key"
        );
        let mut operations = std::collections::HashSet::new();
        ensure!(
            self.operations
                .iter()
                .all(|o| operations.insert(o.intent_id)),
            "Duplicate intent"
        );
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(try_from = "Value", into = "Value")]
pub struct Market {
    pub id: u32,
    pub symbol: String,
    pub asset_index: u32,
    pub price_scale: Raw,
    pub active: bool,
    pub paused: bool,
    pub terminal: bool,
    pub max_leverage: u32,
    pub max_position_notional: Raw,
    pub long_oi: Raw,
    pub short_oi: Raw,
    pub long_cap: Raw,
    pub short_cap: Raw,
    pub frozen_bid: Option<Raw>,
    pub frozen_ask: Option<Raw>,
    pub wire: Value,
}
impl TryFrom<Value> for Market {
    type Error = anyhow::Error;
    fn try_from(v: Value) -> Result<Self> {
        let a = v
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("Invalid market"))?;
        ensure!(a.len() == 23, "Unknown market tuple version");
        let market = Self {
            id: serde_json::from_value(a[0].clone())?,
            symbol: serde_json::from_value(a[1].clone())?,
            asset_index: serde_json::from_value(a[2].clone())?,
            price_scale: serde_json::from_value(a[3].clone())?,
            active: serde_json::from_value(a[4].clone())?,
            paused: serde_json::from_value(a[5].clone())?,
            terminal: serde_json::from_value(a[6].clone())?,
            frozen_bid: serde_json::from_value(a[9].clone())?,
            frozen_ask: serde_json::from_value(a[10].clone())?,
            max_leverage: serde_json::from_value(a[14].clone())?,
            max_position_notional: serde_json::from_value(a[15].clone())?,
            long_oi: serde_json::from_value(a[19].clone())?,
            short_oi: serde_json::from_value(a[20].clone())?,
            long_cap: serde_json::from_value(a[21].clone())?,
            short_cap: serde_json::from_value(a[22].clone())?,
            wire: v.clone(),
        };
        ensure!(
            !market.symbol.is_empty()
                && market.symbol.bytes().all(|b| b.is_ascii_alphanumeric())
                && market.price_scale.0 > U256::ZERO
                && market.max_leverage > 0,
            "Invalid market identity"
        );
        Ok(market)
    }
}
impl From<Market> for Value {
    fn from(m: Market) -> Self {
        m.wire
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TradingState {
    pub schema_version: u32,
    pub indexed_through_block: u64,
    pub trading_paused: bool,
    pub minimum_trade_size_raw: Raw,
    pub minimum_open_notional_raw: Raw,
    pub instruments: Vec<Market>,
    pub intake_controls: Option<(u64, Raw, Vec<String>)>,
    #[serde(default)]
    pub minimum_leverage: u32,
    #[serde(default)]
    pub maximum_leverage: u32,
}
impl TradingState {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.schema_version == 1 && !self.instruments.is_empty(),
            "Unknown trading schema"
        );
        let mut ids = std::collections::HashSet::new();
        let mut symbols = std::collections::HashSet::new();
        ensure!(
            self.instruments
                .iter()
                .all(|m| ids.insert(m.id) && symbols.insert(&m.symbol)),
            "Duplicate market identity"
        );
        Ok(())
    }
    pub fn action_enabled(&self, action: &str) -> bool {
        !self
            .intake_controls
            .as_ref()
            .is_some_and(|(_, _, disabled)| disabled.iter().any(|a| a == action))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CloseRecord {
    pub timestamp_ms: u64,
    pub tx_hash: B256,
    pub position_id: Raw,
    pub instrument_id: u32,
    pub symbol: String,
    pub price_scale_raw: Raw,
    pub is_long: bool,
    pub entry_price_raw: Raw,
    pub margin_raw: Raw,
    pub leverage: u32,
    pub kind: String,
    pub opened_at_ms: u64,
    pub exit_price_raw: Raw,
    pub adjusted_pnl_raw: String,
    pub user_paid_fee_raw: Raw,
    #[serde(default)]
    pub builder_fee_raw: Raw,
}
impl CloseRecord {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            matches!(self.kind.as_str(), "PositionClosed" | "Liquidated"),
            "Unknown close kind"
        );
        ensure!(
            self.margin_raw.0 > U256::ZERO
                && self.entry_price_raw.0 > U256::ZERO
                && self.price_scale_raw.0 > U256::ZERO
                && self.leverage > 0,
            "Invalid close values"
        );
        ensure!(
            self.opened_at_ms <= self.timestamp_ms && self.timestamp_ms <= u64::MAX / 1_000_000,
            "Invalid close chronology"
        );
        let _ = self.net_pnl()?;
        Ok(())
    }
    pub(crate) fn validate_position(&self, position: &VenuePosition) -> Result<()> {
        ensure!(
            self.position_id == position.position_id
                && self.instrument_id == position.instrument_id
                && self.is_long == position.is_long
                && self.entry_price_raw == position.entry_price
                && self.margin_raw == position.margin
                && self.leverage == position.leverage
                && self.opened_at_ms == position.opened_at_ms,
            "Close history disagrees with observed position identity"
        );
        Ok(())
    }
    pub fn net_pnl(&self) -> Result<Decimal> {
        Ok(scaled_decimal(&self.adjusted_pnl_raw, 18)?
            - self.user_paid_fee_raw.decimal()?
            - self.builder_fee_raw.decimal()?)
    }
}
