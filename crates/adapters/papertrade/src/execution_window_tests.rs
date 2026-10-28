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

//! Fixed authorization windows and one-way risk denial; local services only.

use nautilus_common::factories::ExecutionClientFactory;
use nautilus_model::{
    identifiers::{ClientOrderId, PositionId, TraderId},
    orders::OrderTestBuilder,
};
use rstest::rstest;
use serde_json::{Value, json};

use super::{
    tests::{cancellation, client_at, order, support},
    *,
};
use crate::{
    config::{PapertradeExecutionClientConfig, PapertradeRiskBlocker},
    factories::PapertradeExecutionClientFactory,
    journal::JournalEntry,
};

pub(super) fn bind_window(
    harness: &mut support::Harness,
    now: u64,
    risk_offset: i64,
    exit_offset: i64,
) -> PapertradeRiskBlocker {
    let mut document = serde_json::to_value(&harness.config).unwrap();
    document["execution_window"] = json!({"mode":"bounded", "risk_increase_until":now.checked_add_signed(risk_offset).unwrap(), "exit_until":now.checked_add_signed(exit_offset).unwrap()});
    harness.config = serde_json::from_value(document).unwrap();
    let blocker = PapertradeRiskBlocker::default();
    harness.config.risk_blocker = Some(blocker.clone());
    blocker
}

#[rstest]
fn freshness_requires_explicit_arming_and_uses_an_exclusive_nanosecond_bound() {
    let blocker = PapertradeRiskBlocker::default();
    assert!(blocker.allows_risk_increase_at(100));
    assert!(blocker.set_observation_valid_until_ns(200).is_err());
    blocker.require_fresh_observation();
    assert!(!blocker.allows_risk_increase_at(100));
    blocker.set_observation_valid_until_ns(200).unwrap();
    assert!(blocker.allows_risk_increase_at(199));
    assert!(!blocker.allows_risk_increase_at(200));
    blocker.require_fresh_observation();
    assert!(blocker.allows_risk_increase_at(199));
    blocker.set_observation_valid_until_ns(0).unwrap();
    assert!(!blocker.allows_risk_increase_at(100));
    blocker.set_observation_valid_until_ns(200).unwrap();
    assert!(blocker.allows_risk_increase_at(199));
    blocker.freeze();
    blocker.set_observation_valid_until_ns(300).unwrap();
    assert!(!blocker.allows_risk_increase_at(199));
}

#[rstest]
fn freshness_and_freeze_share_the_entire_cloned_runtime_state() {
    let original = PapertradeRiskBlocker::default();
    let clone = original.clone();
    original.require_fresh_observation();
    assert!(!clone.allows_risk_increase_at(10));
    clone.set_observation_valid_until_ns(20).unwrap();
    assert!(original.allows_risk_increase_at(19));
    original.freeze();
    assert!(!clone.allows_risk_increase_at(19));
}

#[rstest]
#[case(false, 0, false)]
#[case(false, 1, true)]
#[case(true, 0, true)]
#[tokio::test]
async fn freshness_is_rechecked_by_the_actual_order_sender(
    #[case] closing: bool,
    #[case] remaining_ns: u64,
    #[case] allowed: bool,
) {
    let _serial = support::SERIAL.lock().await;
    let mut harness = support::Harness::new().await;
    let now = get_atomic_clock_realtime().get_time_ns().as_u64() / 1_000_000_000;
    let blocker = bind_window(&mut harness, now, 7200, 10800);
    let (mut client, _, _) = client_at(&harness, now);
    client.start().unwrap();
    client.connect().await.unwrap();
    let (_, command) = if closing {
        close_order(&harness, &client)
    } else {
        order(&harness)
    };
    legacy_prepared(&client, &command, closing, now);
    blocker.require_fresh_observation();
    blocker
        .set_observation_valid_until_ns(now * 1_000_000_000 + remaining_ns)
        .unwrap();
    let result = client.inner.submit_prepared(command.client_order_id).await;
    client.disconnect().await.unwrap();
    assert_eq!(result.is_ok(), allowed, "{result:?}");
    assert_eq!(
        harness.source.intent_requests.load(Ordering::SeqCst),
        usize::from(allowed)
    );
}

#[rstest]
#[case(false, true)]
#[case(true, false)]
#[tokio::test]
async fn freshness_preserves_cancel_risk_classification(
    #[case] closing: bool,
    #[case] allowed: bool,
) {
    let _serial = support::SERIAL.lock().await;
    let mut harness = support::Harness::new().await;
    let now = get_atomic_clock_realtime().get_time_ns().as_u64() / 1_000_000_000;
    let blocker = bind_window(&mut harness, now, 7200, 10800);
    let (mut client, _, _) = client_at(&harness, now);
    client.start().unwrap();
    client.connect().await.unwrap();
    let (_, command) = if closing {
        close_order(&harness, &client)
    } else {
        order(&harness)
    };
    let entry = legacy_prepared(&client, &command, closing, now);
    let cancel = cancellation(&client, &entry, now);
    blocker.require_fresh_observation();
    let result = client.inner.cancel_prepared(&cancel, entry, now).await;
    client.disconnect().await.unwrap();
    assert_eq!(result.is_ok(), allowed, "{result:?}");
    assert_eq!(
        harness.source.intent_requests.load(Ordering::SeqCst),
        usize::from(allowed)
    );
}

#[rstest]
#[case(true)]
#[case(false)]
#[tokio::test]
async fn freshness_is_sampled_after_blocking_admission_reads(#[case] account_lock: bool) {
    let _serial = support::SERIAL.lock().await;
    let mut harness = support::Harness::new().await;
    let now = get_atomic_clock_realtime().get_time_ns().as_u64() / 1_000_000_000;
    let blocker = bind_window(&mut harness, now, 7200, 10800);
    let (mut client, clock, _) = client_at(&harness, now);
    client.start().unwrap();
    client.connect().await.unwrap();
    let (_, command) = order(&harness);
    let signed = legacy_prepared(&client, &command, false, now)
        .signed()
        .unwrap();
    blocker.require_fresh_observation();
    let until = now * 1_000_000_000 + 100;
    blocker.set_observation_valid_until_ns(until).unwrap();
    let held = (
        account_lock.then(|| client.inner.snapshot.write()),
        (!account_lock).then(|| client.inner.stream_activity.write()),
    );
    let inner = client.inner.clone();
    let (entered, entering) = std::sync::mpsc::channel();
    let (completed, completion) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        entered.send(()).unwrap();
        completed
            .send(
                inner
                    .submission_time(&signed, false)
                    .map_err(|e| e.to_string()),
            )
            .unwrap();
    });
    entering
        .recv_timeout(std::time::Duration::from_secs(3))
        .unwrap();
    let blocked = completion.recv_timeout(std::time::Duration::from_millis(50));
    clock.set_time(nautilus_core::UnixNanos::from(until));
    drop(held);
    let result = completion
        .recv_timeout(std::time::Duration::from_secs(3))
        .unwrap();
    worker.join().unwrap();
    client.disconnect().await.unwrap();
    assert!(matches!(
        blocked,
        Err(std::sync::mpsc::RecvTimeoutError::Timeout)
    ));
    assert!(result.unwrap_err().contains("fresh observation"));
    assert_eq!(harness.source.intent_requests.load(Ordering::SeqCst), 0);
}

pub(super) fn close_order(
    harness: &support::Harness,
    client: &PapertradeExecutionClient,
) -> (OrderAny, SubmitOrder) {
    let position = client
        .inner
        .snapshot
        .read()
        .as_ref()
        .unwrap()
        .open_positions[0]
        .clone();
    let trading = client.inner.trading.read();
    let market = trading
        .as_ref()
        .unwrap()
        .instruments
        .iter()
        .find(|market| market.id == position.instrument_id)
        .unwrap();
    let order = OrderTestBuilder::new(OrderType::Market)
        .instrument_id(reports::instrument_id(market))
        .client_order_id(ClientOrderId::from("WINDOW-CLOSE"))
        .side(if position.is_long {
            OrderSide::Sell
        } else {
            OrderSide::Buy
        })
        .quantity(reports::quantity(&position, market).unwrap())
        .reduce_only(true)
        .time_in_force(TimeInForce::Gtc)
        .build();
    harness
        .cache
        .borrow_mut()
        .add_order(order.clone(), None, None, false)
        .unwrap();
    let command = SubmitOrder::from_order(
        &order,
        TraderId::from("TESTER-001"),
        None,
        Some(PositionId::from(format!("PT-{}", position.position_id.0))),
        nautilus_core::UUID4::new(),
        UnixNanos::from(1),
    );
    (order, command)
}

pub(super) fn legacy_prepared(
    client: &PapertradeExecutionClient,
    command: &SubmitOrder,
    closing: bool,
    now: u64,
) -> JournalEntry {
    let signer = client.inner.signer.as_ref().unwrap();
    let block = client
        .inner
        .trading
        .read()
        .as_ref()
        .unwrap()
        .indexed_through_block;
    let target = closing.then(|| {
        client
            .inner
            .snapshot
            .read()
            .as_ref()
            .unwrap()
            .open_positions[0]
            .position_id
    });
    // A predecessor/library writer can persist a signed request without the new window.
    // The actual sender must still check it, independently of the current prepare path.
    client
        .inner
        .journal
        .prepare(command, target, block, |nonce| {
            if let Some(target) = target {
                signer.sign_close(
                    &Close {
                        user: client.inner.wallet,
                        positionIds: vec![target.0],
                        nonce,
                        deadline: now + 3600,
                    },
                    now,
                )
            } else {
                signer.sign_open(
                    &OpenPosition {
                        user: client.inner.wallet,
                        instrumentId: 0,
                        isLong: true,
                        size: margin_to_raw(rust_decimal::Decimal::from(100))?,
                        leverage: client.inner.config.leverage,
                        useDebt: false,
                        nonce,
                        deadline: now + 3600,
                    },
                    now,
                )
            }
        })
        .unwrap()
        .0
}

#[rstest]
#[case(json!({"mode":"bounded","risk_increase_until":1700000000,"exit_until":1700010000}))]
#[case(json!({"mode":"unbounded","ignored":true}))]
#[case(json!({"mode":"unknown"}))]
#[case(json!({"mode":"bounded","risk_increase_until":0,"exit_until":1700010000}))]
#[case(json!({"mode":"bounded","risk_increase_until":1700000000,"exit_until":0}))]
#[case(json!({"mode":"bounded","risk_increase_until":1700010001,"exit_until":1700010000}))]
#[case(json!({"mode":"bounded","risk_increase_until":1700000000,"exit_until":18446744074_u64}))]
#[case(json!({"mode":"bounded","risk_increase_until":true,"exit_until":1700010000}))]
#[case(json!({"mode":"bounded","risk_increase_until":1700000000.0,"exit_until":1700010000}))]
#[case(json!({"mode":"bounded","risk_increase_until":"1700000000","exit_until":1700010000}))]
#[case(json!({"mode":"bounded","risk_increase_until":-1,"exit_until":1700010000}))]
#[case(json!({"mode":"bounded","risk_increase_until":1700000000}))]
#[case(json!({"mode":"bounded","risk_increase_until":1700000000,"exit_until":1700010000,"ignored":true}))]
#[tokio::test]
async fn native_window_invalid_or_unbound_configuration_is_refused_before_storage(
    #[case] window: Value,
) {
    let _serial = support::SERIAL.lock().await;
    let harness = support::Harness::new().await;
    let mut document = serde_json::to_value(&harness.config).unwrap();
    document["execution_window"] = window;
    if let Ok(config) = serde_json::from_value::<PapertradeExecutionClientConfig>(document) {
        let result = PapertradeExecutionClientFactory.create(
            TraderId::from("TESTER-001"),
            "PAPERTRADE",
            &config,
            harness.cache.clone().into(),
        );
        assert!(
            result.is_err(),
            "bounded runtime capability must not disappear from config"
        );
    }
    assert!(!harness.config.journal_path.exists());
}

#[rstest]
#[case(Value::Null)]
#[case(Value::Bool(false))]
#[case(json!({"frozen":true}))]
#[tokio::test]
async fn native_window_json_cannot_restore_a_runtime_blocker(#[case] value: Value) {
    let _serial = support::SERIAL.lock().await;
    let harness = support::Harness::new().await;
    let mut document = serde_json::to_value(&harness.config).unwrap();
    document["risk_blocker"] = value;
    assert!(serde_json::from_value::<PapertradeExecutionClientConfig>(document).is_err());
    assert!(!harness.config.journal_path.exists());
}

#[rstest]
#[case(false, 3600, 7200, false)]
#[case(false, 3601, 7200, true)]
#[case(false, 3599, 7200, false)]
#[case(true, -1, 3600, false)]
#[case(true, -1, 3601, true)]
#[case(true, -1, 3599, false)]
#[tokio::test]
async fn native_window_prepare_keeps_the_full_signature_inside_its_exclusive_bound(
    #[case] closing: bool,
    #[case] risk: i64,
    #[case] exit: i64,
    #[case] allowed: bool,
) {
    let _serial = support::SERIAL.lock().await;
    let mut harness = support::Harness::new().await;
    let now = get_atomic_clock_realtime().get_time_ns().as_u64() / 1_000_000_000;
    bind_window(&mut harness, now, risk, exit);
    let (mut client, _, _) = client_at(&harness, now);
    client.start().unwrap();
    client.connect().await.unwrap();
    let (order, command) = if closing {
        close_order(&harness, &client)
    } else {
        order(&harness)
    };
    let result = client.prepare_order(&command, &order);
    let entries = client.inner.journal.entries().unwrap();
    client.disconnect().await.unwrap();
    assert_eq!(result.is_ok(), allowed, "{result:?}");
    assert_eq!(entries.len(), usize::from(allowed));
}

#[rstest]
#[case(false, 3600, 7200, false, false)]
#[case(false, 3601, 7200, false, true)]
#[case(false, 3601, 7200, true, false)]
#[case(true, -1, 3600, false, false)]
#[case(true, -1, 3601, false, true)]
#[case(true, -1, 3601, true, true)]
#[tokio::test]
async fn native_window_actual_order_sender_rechecks_window_and_frozen_state(
    #[case] closing: bool,
    #[case] risk: i64,
    #[case] exit: i64,
    #[case] frozen: bool,
    #[case] allowed: bool,
) {
    let _serial = support::SERIAL.lock().await;
    let mut harness = support::Harness::new().await;
    let now = get_atomic_clock_realtime().get_time_ns().as_u64() / 1_000_000_000;
    let blocker = bind_window(&mut harness, now, risk, exit);
    let (mut client, _, _) = client_at(&harness, now);
    client.start().unwrap();
    client.connect().await.unwrap();
    let (_order, command) = if closing {
        close_order(&harness, &client)
    } else {
        order(&harness)
    };
    let entry = legacy_prepared(&client, &command, closing, now);
    if frozen {
        blocker.freeze();
        blocker.freeze();
    }
    let result = client.inner.submit_prepared(command.client_order_id).await;
    let persisted = client.inner.journal.entries().unwrap().remove(0);
    client.disconnect().await.unwrap();
    assert_eq!(persisted.body, entry.body);
    assert_eq!(persisted.intent_id, entry.intent_id);
    assert_eq!(result.is_ok(), allowed, "{result:?}");
    assert_eq!(
        harness.source.intent_requests.load(Ordering::SeqCst),
        usize::from(allowed)
    );
    assert_eq!(persisted.send_attempted, allowed);
}

#[rstest]
#[case(false, -1, 3600, false, false)]
#[case(false, -1, 3601, false, true)]
#[case(false, -1, 3601, true, true)]
#[case(true, 3600, 7200, false, false)]
#[case(true, 3601, 7200, false, true)]
#[case(true, 3601, 7200, true, false)]
#[tokio::test]
async fn native_window_cancelling_a_close_is_not_an_exit_permission(
    #[case] closing: bool,
    #[case] risk: i64,
    #[case] exit: i64,
    #[case] frozen: bool,
    #[case] allowed: bool,
) {
    let _serial = support::SERIAL.lock().await;
    let mut harness = support::Harness::new().await;
    let now = get_atomic_clock_realtime().get_time_ns().as_u64() / 1_000_000_000;
    let blocker = bind_window(&mut harness, now, risk, exit);
    let (mut client, _, _) = client_at(&harness, now);
    client.start().unwrap();
    client.connect().await.unwrap();
    let (_order, command) = if closing {
        close_order(&harness, &client)
    } else {
        order(&harness)
    };
    let entry = legacy_prepared(&client, &command, closing, now);
    let cancel = cancellation(&client, &entry, now);
    if frozen {
        blocker.freeze();
    }
    let result = client
        .inner
        .cancel_prepared(&cancel, entry.clone(), now)
        .await;
    let persisted = client.inner.journal.entries().unwrap().remove(0);
    client.disconnect().await.unwrap();
    assert_eq!(result.is_ok(), allowed, "{result:?}");
    assert_eq!(
        harness.source.intent_requests.load(Ordering::SeqCst),
        usize::from(allowed)
    );
    assert_eq!(persisted.status, entry.status);
}

#[tokio::test]
async fn native_window_expiry_and_risk_freeze_do_not_disable_observation() {
    let _serial = support::SERIAL.lock().await;
    let mut harness = support::Harness::new().await;
    let now = get_atomic_clock_realtime().get_time_ns().as_u64() / 1_000_000_000;
    let blocker = bind_window(&mut harness, now, -7200, -3600);
    blocker.freeze();
    harness.config.read_only = true;
    let (mut client, _, _) = client_at(&harness, now);
    client.start().unwrap();
    client.connect().await.unwrap();
    assert!(client.is_connected());
    assert!(client.generate_mass_status(None).await.unwrap().is_some());
    assert_eq!(harness.source.intent_requests.load(Ordering::SeqCst), 0);
    client.disconnect().await.unwrap();
}

#[test]
fn native_window_a_runtime_blocker_cannot_be_lost_through_config_serialization() {
    let mut config = PapertradeExecutionClientConfig::new(
        nautilus_model::identifiers::AccountId::from("PAPERTRADE-001"),
        "0x2222222222222222222222222222222222222222".into(),
        "fixture.redb".into(),
        "test-build".into(),
    );
    let blocker = PapertradeRiskBlocker::default();
    config.risk_blocker = Some(blocker.clone());
    let clone = config.clone();
    blocker.freeze();
    assert!(clone.risk_blocker.as_ref().unwrap().is_frozen());
    assert!(serde_json::to_value(&config).is_err());
    assert!(serde_json::to_value(&clone).is_err());
}
