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

use alloy_primitives::{Address, B256, address, b256};
use nautilus_papertrade::{
    chain,
    wire::{CloseRecord, Raw},
};
use rstest::rstest;
use serde_json::{Value, json};

fn receipt() -> Value {
    serde_json::from_str(include_str!("../test_data/live_receipt.json")).unwrap()
}
#[rstest]
fn observed_close_batch_binds_individual_intents_and_exchange_effects() {
    let receipt = receipt();
    let id = b256!("bb96a1ff0cca1f172ef94a56aaf05b66129be7568634e5abb24c4c16a43c4fce");
    assert_eq!(chain::intent_result(&receipt, id).unwrap(), Some(true));
    assert_eq!(chain::intent_result(&receipt, B256::ZERO).unwrap(), None);
    let history: Value =
        serde_json::from_str(include_str!("../test_data/live_history.json")).unwrap();
    for row in history["items"].as_array().unwrap().iter().take(3) {
        let record: CloseRecord = serde_json::from_value(row.clone()).unwrap();
        chain::verify_close(
            &receipt,
            &record,
            address!("a23dadf056158c5ff1081654dbf8e167530fbf71"),
        )
        .unwrap();
        assert!(chain::verify_close(&receipt, &record, Address::ZERO).is_err());
    }
}
#[rstest]
fn observed_open_return_data_recovers_position_without_a_wallet_frame() {
    let receipt: Value =
        serde_json::from_str(include_str!("../test_data/live_open_receipt.json")).unwrap();
    let id = b256!("adfae189b04060dcbef2af7322b00447844d8119b819e1820308e38e86339614");
    let wallet = address!("19ea302761c7ca11d4252f7e9ad2ca199d0e0380");
    let mut body = json!({"signed":{"instrumentId":"0","isLong":true,"useDebt":false,
        "size":"500000000000000000000000","leverage":"20"}});
    let position = chain::recovered_open(&receipt, id, wallet, &body).unwrap();
    assert_eq!(position.position_id.to_string(), "1671873");
    assert_eq!(position.entry_price.to_string(), "830005");
    assert_eq!(position.intent_id, Some(id));
    body["signed"]["leverage"] = json!("21");
    assert!(chain::recovered_open(&receipt, id, wallet, &body).is_err());
    let _zero = Raw::default();
}
#[rstest]
fn transaction_success_does_not_hide_missing_or_removed_intent_results() {
    let mut receipt = receipt();
    let id = b256!("bb96a1ff0cca1f172ef94a56aaf05b66129be7568634e5abb24c4c16a43c4fce");
    let log = receipt["logs"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|l| l["topics"][3] == json!(id))
        .unwrap();
    log["removed"] = json!(true);
    assert!(chain::intent_result(&receipt, id).is_err());
}
