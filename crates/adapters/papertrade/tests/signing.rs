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

//! Independent EIP-712 vectors and semantic boundary tests.

use std::str::FromStr;

use alloy::signers::local::PrivateKeySigner;
use alloy_primitives::{Address, B256, U256, address};
use nautilus_papertrade::{
    CancelIntent, Close, OpenPosition, PapertradeSigner, RegisterSessionKey, SignedIntent,
    margin_to_raw,
};
use rstest::rstest;
use rust_decimal::Decimal;
use serde_json::{Value, json};

const NOW: u64 = 1_700_000_000;
const USER: Address = address!("2222222222222222222222222222222222222222");

fn signer() -> PapertradeSigner {
    PapertradeSigner::new(PrivateKeySigner::from_bytes(&B256::repeat_byte(17)).unwrap())
}

fn open() -> OpenPosition {
    OpenPosition {
        user: USER,
        instrumentId: 0,
        isLong: true,
        size: 12_345_000_000_000_000_000,
        leverage: 100,
        useDebt: false,
        nonce: U256::from(123_456),
        deadline: NOW + 3_600,
    }
}

fn vectors() -> Value {
    serde_json::from_str(include_str!("../test_data/signing_vectors.json")).unwrap()
}

fn assert_vector(intent: &SignedIntent, name: &str, is_cancel: bool) {
    let vectors = vectors();
    let vector = &vectors[name];
    assert_eq!(format!("{:#x}", intent.id()), vector["intentId"]);
    let fields = if is_cancel {
        intent.body()
    } else {
        &intent.body()["signed"]
    };
    for field in ["v", "r", "s"] {
        assert_eq!(fields[field], vector[field], "{name} {field}");
    }
}

#[rstest]
fn signatures_match_independent_eth_account_vectors() {
    let signer = signer();
    let opened = signer.sign_open(&open(), NOW).unwrap();
    assert_vector(&opened, "OpenPosition", false);
    assert_eq!(opened.body()["signed"]["size"], "12345000000000000000");
    assert_eq!(opened.body()["signed"]["instrumentId"], "0");
    assert_eq!(opened.body()["signed"]["isLong"], true);
    assert_eq!(opened.path(), "/intents");

    let close = Close {
        user: USER,
        positionIds: vec![U256::from(1), U256::from(1) << 200],
        nonce: U256::from(123_457),
        deadline: NOW + 3_600,
    };
    assert_vector(&signer.sign_close(&close, NOW).unwrap(), "Close", false);

    let register = RegisterSessionKey {
        user: signer.address(),
        key: USER,
        expiry: NOW + 86_400,
        feeAddress: Address::ZERO,
        nonce: U256::from(123_458),
        deadline: NOW + 3_600,
    };
    assert_vector(
        &signer.sign_registration(&register, NOW).unwrap(),
        "RegisterSessionKey",
        false,
    );

    let cancel = CancelIntent {
        user: USER,
        intentId: opened.id(),
        deadline: U256::from(NOW + 3_600),
    };
    let cancelled = signer.sign_cancel(&cancel, &opened, NOW).unwrap();
    assert_vector(&cancelled, "CancelIntent", true);
    assert_eq!(
        cancelled.path(),
        format!("/intents/{:#x}/cancel", opened.id())
    );
    assert_eq!(cancelled.action(), "openPosition");
    assert!(cancelled.body().get("intentId").is_none());
    assert!(cancelled.body().get("action").is_none());
}

#[rstest]
#[case("1", "1000000000000000000")]
#[case("12.345", "12345000000000000000")]
#[case("0.000000000000000001", "1")]
#[case("1.0000000000000000001", "")]
#[case("0", "")]
#[case("-1", "")]
#[case("79228162514264337593543950335", "")]
fn margin_conversion_is_exact(#[case] input: &str, #[case] expected: &str) {
    let result = margin_to_raw(Decimal::from_str(input).unwrap());
    if expected.is_empty() {
        assert!(result.is_err());
    } else {
        assert_eq!(result.unwrap().to_string(), expected);
    }
}

#[rstest]
#[case(0)]
#[case(NOW)]
#[case(NOW + 3_601)]
fn refuses_expired_and_overlong_intents(#[case] deadline: u64) {
    let mut intent = open();
    intent.deadline = deadline;
    assert!(signer().sign_open(&intent, NOW).is_err());
}

#[rstest]
fn refuses_invalid_opens() {
    let signer = signer();
    let mut intent = open();
    intent.size = 0;
    assert!(signer.sign_open(&intent, NOW).is_err());
    intent = open();
    intent.leverage = 0;
    assert!(signer.sign_open(&intent, NOW).is_err());
    intent = open();
    intent.user = Address::ZERO;
    assert!(signer.sign_open(&intent, NOW).is_err());
}

#[rstest]
#[case(vec![])]
#[case(vec![U256::from(1), U256::from(1)])]
#[case((0..13).map(U256::from).collect())]
fn refuses_invalid_close_batches(#[case] position_ids: Vec<U256>) {
    let intent = Close {
        user: USER,
        positionIds: position_ids,
        nonce: U256::from(1),
        deadline: NOW + 1,
    };
    assert!(signer().sign_close(&intent, NOW).is_err());
}

#[rstest]
fn registration_requires_master_authority_and_bounded_expiry() {
    let signer = signer();
    let mut intent = RegisterSessionKey {
        user: USER,
        key: signer.address(),
        expiry: NOW + 86_400,
        feeAddress: Address::ZERO,
        nonce: U256::from(1),
        deadline: NOW + 3_600,
    };
    assert!(signer.sign_registration(&intent, NOW).is_err());
    intent.user = signer.address();
    intent.expiry = NOW + 31 * 86_400;
    assert!(signer.sign_registration(&intent, NOW).is_err());
    intent.expiry = NOW + 3_600;
    assert!(signer.sign_registration(&intent, NOW).is_err());
    intent.expiry = NOW + 86_400;
    intent.key = Address::ZERO;
    assert!(signer.sign_registration(&intent, NOW).is_err());
}

#[rstest]
fn cancellation_binds_target_and_user() {
    let signer = signer();
    let target = signer.sign_open(&open(), NOW).unwrap();
    let mut cancel = CancelIntent {
        user: USER,
        intentId: B256::ZERO,
        deadline: U256::from(NOW + 100),
    };
    assert!(signer.sign_cancel(&cancel, &target, NOW).is_err());
    cancel.intentId = target.id();
    cancel.user = signer.address();
    assert!(signer.sign_cancel(&cancel, &target, NOW).is_err());
}

#[rstest]
fn signature_capabilities_and_private_key_are_absent_from_debug() {
    let signer = signer();
    let intent = signer.sign_open(&open(), NOW).unwrap();
    let debug = format!("{signer:?} {intent:?}");
    assert!(!debug.contains("1111111111111111111111111111111111111111111111111111111111111111"));
    assert!(!debug.contains(intent.body()["signed"]["r"].as_str().unwrap()));
}

#[rstest]
fn signing_the_same_intent_preserves_identity_and_wire_bytes() {
    let signer = signer();
    let first = signer.sign_open(&open(), NOW).unwrap();
    let second = signer.sign_open(&open(), NOW + 10).unwrap();
    assert_eq!(first.id(), second.id());
    assert_eq!(first.body(), second.body());
    let mut changed = open();
    changed.nonce += U256::from(1);
    assert_ne!(first.id(), signer.sign_open(&changed, NOW).unwrap().id());
    assert_eq!(first.body()["action"], json!("openPosition"));
}
