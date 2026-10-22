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

use nautilus_core::UnixNanos;
use nautilus_model::identifiers::AccountId;
use nautilus_papertrade::{
    reports,
    wire::{AccountSnapshot, CloseRecord, TradingState},
};
use rstest::rstest;
use rust_decimal::Decimal;
use serde_json::Value;

#[rstest]
fn hedged_source_positions_keep_independent_ids_and_quantities() {
    let snapshot: AccountSnapshot =
        serde_json::from_str(include_str!("../test_data/live_wallet.json")).unwrap();
    let state: TradingState =
        serde_json::from_str(include_str!("../test_data/live_trading.json")).unwrap();
    let a = AccountId::from("PAPERTRADE-001");
    let now = UnixNanos::from(1);
    let first =
        reports::position_report(&snapshot.open_positions[0], &state.instruments[0], a, now)
            .unwrap();
    let second =
        reports::position_report(&snapshot.open_positions[1], &state.instruments[0], a, now)
            .unwrap();
    assert_ne!(first.venue_position_id, second.venue_position_id);
    assert!(first.signed_decimal_qty > Decimal::ZERO && second.signed_decimal_qty < Decimal::ZERO);
    let balances = reports::balances(&snapshot).unwrap();
    assert_eq!(balances.len(), 1);
    assert_eq!(
        balances[0].total.as_decimal(),
        Decimal::from(1_000_000) + snapshot.balance.decimal().unwrap().trunc_with_scale(6)
    );
}
#[rstest]
fn normal_and_liquidated_payoffs_reconcile_to_source_net_pnl() {
    let value: Value =
        serde_json::from_str(include_str!("../test_data/live_history.json")).unwrap();
    let state: TradingState =
        serde_json::from_str(include_str!("../test_data/live_trading.json")).unwrap();
    let mut liquidations = 0;
    for row in value["items"].as_array().unwrap() {
        let record: CloseRecord = serde_json::from_value(row.clone()).unwrap();
        if record.kind == "Liquidated" {
            liquidations += 1;
        }
        let market = state
            .instruments
            .iter()
            .find(|m| m.id == record.instrument_id)
            .unwrap();
        let fill = reports::close_fill(
            &record,
            market,
            AccountId::from("PAPERTRADE-001"),
            None,
            None,
            UnixNanos::from(1),
        )
        .unwrap();
        let p = reports::historical_position(&record);
        let entry = reports::market_price(market, p.entry_price).unwrap();
        let gross = fill.last_qty.as_decimal()
            * (fill.last_px.as_decimal() - entry)
            * if record.is_long {
                Decimal::ONE
            } else {
                -Decimal::ONE
            };
        assert!(
            (gross - fill.commission.as_decimal() - record.net_pnl().unwrap()).abs()
                <= Decimal::new(1, 6)
        );
        assert!(fill.trade_id.to_string().len() <= 36);
        let repeated = reports::close_fill(
            &record,
            market,
            AccountId::from("PAPERTRADE-001"),
            None,
            None,
            UnixNanos::from(2),
        )
        .unwrap();
        assert_eq!(fill.trade_id, repeated.trade_id);
    }
    assert!(liquidations > 0);
}
