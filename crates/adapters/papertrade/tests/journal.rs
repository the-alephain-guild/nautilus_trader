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

use alloy::signers::local::PrivateKeySigner;
use alloy_primitives::{Address, B256, U256, address};
use nautilus_common::messages::execution::SubmitOrder;
use nautilus_core::{UUID4, UnixNanos};
use nautilus_model::{
    enums::{OrderStatus, OrderType},
    identifiers::{ClientOrderId, InstrumentId, TraderId},
    orders::OrderTestBuilder,
    types::Quantity,
};
use nautilus_papertrade::{OpenPosition, PapertradeSigner, journal::Journal};
use rstest::rstest;

const WALLET: Address = address!("2222222222222222222222222222222222222222");
fn command(id: &str) -> SubmitOrder {
    let order = OrderTestBuilder::new(OrderType::Market)
        .instrument_id(InstrumentId::from("BTC-USD-PERP.PAPERTRADE"))
        .client_order_id(ClientOrderId::from(id))
        .quantity(Quantity::from(1000))
        .build();
    SubmitOrder::from_order(
        &order,
        TraderId::from("TESTER-001"),
        None,
        None,
        UUID4::new(),
        UnixNanos::from(1),
    )
}
fn sign(nonce: U256) -> anyhow::Result<nautilus_papertrade::SignedIntent> {
    PapertradeSigner::new(PrivateKeySigner::from_bytes(&B256::repeat_byte(17)).unwrap()).sign_open(
        &OpenPosition {
            user: WALLET,
            instrumentId: 0,
            isLong: true,
            size: 10_000_000_000_000_000_000,
            leverage: 100,
            useDebt: false,
            nonce,
            deadline: 1_700_003_600,
        },
        1_700_000_000,
    )
}
#[rstest]
fn restart_retains_request_and_duplicate_command_never_allocates_a_new_intent() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("journal.redb");
    let cmd = command("PT-1");
    let first = {
        let journal = Journal::open(&path, WALLET, "PAPERTRADE-001").unwrap();
        let (first, new) = journal.prepare(&cmd, None, 100, sign).unwrap();
        assert!(new);
        let (same, new) = journal
            .prepare(&cmd, None, 100, |_| panic!("duplicate was re-signed"))
            .unwrap();
        assert!(!new);
        assert_eq!(same.intent_id, first.intent_id);
        first
    };
    let journal = Journal::open(&path, WALLET, "PAPERTRADE-001").unwrap();
    let (same, new) = journal
        .prepare(&cmd, None, 200, |_| panic!("restart was re-signed"))
        .unwrap();
    assert!(!new);
    assert_eq!(same.body, first.body);
    let (second, new) = journal.prepare(&command("PT-2"), None, 201, sign).unwrap();
    assert!(new);
    assert_eq!(second.nonce.0, first.nonce.0 + U256::from(1));
}
#[rstest]
fn journal_rejects_another_account_and_concurrent_process_owner() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("journal.redb");
    let journal = Journal::open(&path, WALLET, "PAPERTRADE-001").unwrap();
    assert!(Journal::open(&path, WALLET, "PAPERTRADE-001").is_err());
    drop(journal);
    assert!(Journal::open(&path, Address::ZERO, "PAPERTRADE-001").is_err());
}
#[rstest]
fn late_acknowledgement_does_not_erase_a_terminal_state() {
    let dir = tempfile::tempdir().unwrap();
    let journal =
        Journal::open(&dir.path().join("journal.redb"), WALLET, "PAPERTRADE-001").unwrap();
    let (entry, _) = journal.prepare(&command("PT-1"), None, 100, sign).unwrap();
    let mut terminal = entry.clone();
    terminal.status = OrderStatus::Rejected;
    journal.update(&terminal).unwrap();
    let mut late = entry;
    late.status = OrderStatus::Accepted;
    journal.update(&late).unwrap();
    assert_eq!(journal.entries().unwrap()[0].status, OrderStatus::Rejected);
}

#[rstest]
#[case("instrumentId")]
#[case("isLong")]
#[case("entryPriceRaw")]
#[case("marginRaw")]
#[case("leverage")]
#[case("openedAtMs")]
fn conflicting_close_identity_is_rejected_without_poisoning_the_journal(#[case] field: &str) {
    use nautilus_papertrade::wire::{CloseRecord, VenuePosition};
    use serde_json::{Value, json};
    let source: Value =
        serde_json::from_str(include_str!("../test_data/live_wallet.json")).unwrap();
    let position: VenuePosition =
        serde_json::from_value(source["openPositions"][0].clone()).unwrap();
    let value = json!({"positionId":position.position_id,"instrumentId":position.instrument_id,
        "symbol":"BTC","priceScaleRaw":"10","isLong":position.is_long,
        "entryPriceRaw":position.entry_price,"marginRaw":position.margin,"leverage":position.leverage,
        "openedAtMs":position.opened_at_ms,"timestampMs":position.opened_at_ms+1000,
        "txHash":B256::repeat_byte(5),"kind":"PositionClosed","exitPriceRaw":position.entry_price,
        "adjustedPnlRaw":"0","userPaidFeeRaw":"0"});
    let correct: CloseRecord = serde_json::from_value(value.clone()).unwrap();
    let mut changed = value;
    changed[field] = match field {
        "isLong" => json!(!position.is_long),
        "instrumentId" => json!(position.instrument_id + 1),
        "entryPriceRaw" => json!((position.entry_price.0 + U256::from(1)).to_string()),
        "marginRaw" => json!((position.margin.0 + U256::from(1)).to_string()),
        "leverage" => json!(position.leverage + 1),
        "openedAtMs" => json!(position.opened_at_ms + 1),
        _ => unreachable!(),
    };
    let conflict: CloseRecord = serde_json::from_value(changed).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let journal =
        Journal::open(&dir.path().join("journal.redb"), WALLET, "PAPERTRADE-001").unwrap();
    journal
        .remember(std::slice::from_ref(&position), &[])
        .unwrap();
    assert!(
        journal.remember(&[], &[conflict]).is_err(),
        "Changed position economics were accepted as a close"
    );
    assert!(journal.closes().unwrap().is_empty());
    journal.remember(&[], std::slice::from_ref(&correct)).unwrap();
    assert_eq!(journal.closes().unwrap(), vec![correct]);
}
