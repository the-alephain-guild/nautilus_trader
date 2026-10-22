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

use alloy_primitives::Address;
use nautilus_papertrade::stream::{SseDecoder, WalletState};
use rstest::rstest;
use serde_json::{Value, json};

fn snapshot() -> Value {
    serde_json::from_str(include_str!("../test_data/live_wallet.json")).unwrap()
}

#[rstest]
fn observed_nonempty_snapshot_preserves_hedged_positions() {
    let raw = snapshot();
    let wallet: Address = raw["address"].as_str().unwrap().parse().unwrap();
    let mut state = WalletState::new(wallet);
    assert!(state.apply(&raw).unwrap());
    let account = state.current().unwrap();
    assert_eq!(account.open_positions.len(), 2);
    assert!(account.open_positions[0].is_long);
    assert!(!account.open_positions[1].is_long);
    assert_eq!(account.open_positions[0].position_id.to_string(), "1671873");
    assert_eq!(account.locked.to_string(), "1000000000000000000000000");
    assert!(!state.apply(&raw).unwrap());
}

#[rstest]
#[case("wallet")]
#[case("locked")]
#[case("funds")]
#[case("duplicate")]
fn invalid_snapshot_does_not_replace_valid_state(#[case] fault: &str) {
    let raw = snapshot();
    let mut state = WalletState::new(raw["address"].as_str().unwrap().parse().unwrap());
    state.apply(&raw).unwrap();
    let mut changed = raw.clone();
    changed["accountRevision"] = json!(2);
    match fault {
        "wallet" => changed["address"] = json!(Address::ZERO),
        "locked" => changed["locked"] = json!("0"),
        "funds" => changed["funds"]["availableBalance"] = json!("1"),
        "duplicate" => changed["openPositions"][1] = changed["openPositions"][0].clone(),
        _ => unreachable!(),
    }
    assert!(state.apply(&changed).is_err());
    assert!(state.current().is_none());
    assert!(state.apply(&raw).unwrap());
    assert_eq!(state.current().unwrap().account_revision, 1);
}

#[rstest]
fn patches_require_exact_base_and_a_fresh_snapshot_after_disconnect() {
    let raw = snapshot();
    let mut state = WalletState::new(raw["address"].as_str().unwrap().parse().unwrap());
    state.apply(&raw).unwrap();
    let mut patch = json!({
        "address": raw["address"], "baseAccountRevision": 1, "accountRevision": 2,
        "state": {}, "addedPositions": [], "removedPositionIds": [],
        "addedOperations": [], "removedOperationIds": [],
        "intentOutcomes": [], "closedPositions": []
    });
    assert!(state.apply(&patch).unwrap());
    assert!(!state.apply(&patch).unwrap());
    patch["baseAccountRevision"] = json!(3);
    patch["accountRevision"] = json!(4);
    assert!(state.apply(&patch).is_err());
    state.disconnect();
    assert!(state.current().is_none());
    assert!(state.apply(&patch).is_err());
    assert!(state.apply(&raw).unwrap());
}

#[rstest]
fn sse_handles_arbitrary_chunk_boundaries_and_bounds_memory() {
    let bytes = format!(
        ": connected\r\n\r\nevent: wallet\r\ndata: {}\r\n\r\n",
        snapshot()
    );
    let mut decoder = SseDecoder::default();
    let mut events = Vec::new();
    for b in bytes.as_bytes() {
        events.extend(decoder.push(&[*b]).unwrap());
    }
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event, "wallet");
    assert_eq!(
        serde_json::from_str::<Value>(&events[0].data).unwrap(),
        snapshot()
    );
    assert!(
        SseDecoder::default()
            .push(&vec![b'a'; 8 * 1024 * 1024 + 1])
            .is_err()
    );
}
