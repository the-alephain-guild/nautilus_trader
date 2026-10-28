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

//! Account-bound final admission over local venue fixtures.

use super::{
    tests::{cancellation, client_at, order, support},
    window_tests::{bind_window, close_order, legacy_prepared},
    *,
};
use crate::config::PapertradeRiskBlocker;
use rstest::rstest;

async fn wait_initial_source_poll(client: &PapertradeExecutionClient, harness: &support::Harness) {
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            if harness.source.trading_requests.load(Ordering::SeqCst) >= 2
                && client.inner.sync.try_lock().is_ok()
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
}

#[test]
fn account_bound_observations_cannot_use_an_unbound_or_deadline_only_update() {
    let blocker = PapertradeRiskBlocker::default();
    let hash = alloy_primitives::B256::repeat_byte(1);
    assert!(
        blocker
            .set_account_observation_valid_until_ns(hash, 200)
            .is_err()
    );
    blocker.require_fresh_observation();
    blocker.set_observation_valid_until_ns(200).unwrap();
    blocker.require_account_observation();
    assert!(!blocker.allows_risk_increase_at(100));
    assert!(blocker.set_observation_valid_until_ns(300).is_err());
    blocker.set_observation_valid_until_ns(0).unwrap();
    blocker
        .set_account_observation_valid_until_ns(hash, 200)
        .unwrap();
    blocker.require_account_observation();
    assert!(blocker.set_observation_valid_until_ns(300).is_err());
    blocker.freeze();
    blocker
        .set_account_observation_valid_until_ns(hash, 300)
        .unwrap();
    assert!(blocker.is_frozen());
}

#[rstest]
#[case(false, true)]
#[case(true, false)]
#[tokio::test]
async fn account_bound_actual_sender_rejects_a_newly_accepted_snapshot(
    #[case] changed: bool,
    #[case] allowed: bool,
) {
    let _serial = support::SERIAL.lock().await;
    let mut harness = support::Harness::new().await;
    let now = get_atomic_clock_realtime().get_time_ns().as_u64() / 1_000_000_000;
    let blocker = bind_window(&mut harness, now, 7200, 10800);
    blocker.require_account_observation();
    harness.config.poll_interval_ms = 60_000;
    let (mut client, clock, _) = client_at(&harness, now);
    client.start().unwrap();
    client.connect().await.unwrap();
    wait_initial_source_poll(&client, &harness).await;
    let mut snapshot = client.inner.snapshot.read().clone().unwrap();
    let original_hash = alloy_primitives::keccak256(serde_json::to_vec(&snapshot).unwrap());
    blocker
        .set_account_observation_valid_until_ns(original_hash, (now + 60) * 1_000_000_000)
        .unwrap();
    if changed {
        snapshot.account_revision += 1;
        snapshot.indexed_through_timestamp += 1;
        snapshot.balance.0 -= U256::from(30_000_000_000_000_000_000_u128);
        snapshot.funds.available_balance.0 = snapshot
            .balance
            .0
            .saturating_sub(snapshot.funds.reserved_balance.0);
        clock.set_time(nautilus_core::UnixNanos::from((now + 1) * 1_000_000_000));
        crate::reconciliation::synchronize(&client.inner, snapshot)
            .await
            .unwrap();
    }
    let (_, command) = order(&harness);
    legacy_prepared(&client, &command, false, now + u64::from(changed));
    let result = client.inner.submit_prepared(command.client_order_id).await;
    client.disconnect().await.unwrap();
    assert_eq!(result.is_ok(), allowed, "{result:?}");
    assert_eq!(
        harness.source.intent_requests.load(Ordering::SeqCst),
        usize::from(allowed)
    );
}

#[rstest]
#[case(false, false, false)]
#[case(true, false, true)]
#[case(false, true, true)]
#[case(true, true, false)]
#[tokio::test]
async fn account_bound_sync_lock_refuses_only_risk_increases(
    #[case] closing: bool,
    #[case] cancelling: bool,
    #[case] allowed: bool,
) {
    let _serial = support::SERIAL.lock().await;
    let mut harness = support::Harness::new().await;
    let now = get_atomic_clock_realtime().get_time_ns().as_u64() / 1_000_000_000;
    let blocker = bind_window(&mut harness, now, 7200, 10800);
    blocker.require_account_observation();
    let (mut client, _, _) = client_at(&harness, now);
    client.start().unwrap();
    client.connect().await.unwrap();
    let snapshot = client.inner.snapshot.read().clone().unwrap();
    let original_hash = alloy_primitives::keccak256(serde_json::to_vec(&snapshot).unwrap());
    blocker
        .set_account_observation_valid_until_ns(original_hash, (now + 60) * 1_000_000_000)
        .unwrap();
    let (_, command) = if closing {
        close_order(&harness, &client)
    } else {
        order(&harness)
    };
    let entry = legacy_prepared(&client, &command, closing, now);
    let signed = if cancelling {
        cancellation(&client, &entry, now)
    } else {
        entry.signed().unwrap()
    };
    let held = client.inner.sync.lock().await;
    blocker
        .set_account_observation_valid_until_ns(original_hash, (now + 60) * 1_000_000_000)
        .unwrap();
    let result = client.inner.submission_time(&signed, cancelling);
    drop(held);
    client.disconnect().await.unwrap();
    assert_eq!(result.is_ok(), allowed, "{result:?}");
    assert_eq!(harness.source.intent_requests.load(Ordering::SeqCst), 0);
}

#[rstest]
#[case(false)]
#[case(true)]
#[tokio::test]
async fn account_bound_failed_or_cancelled_apply_stays_closed_until_success(
    #[case] cancelled: bool,
) {
    let _serial = support::SERIAL.lock().await;
    let mut harness = support::Harness::new().await;
    let now = get_atomic_clock_realtime().get_time_ns().as_u64() / 1_000_000_000;
    let blocker = bind_window(&mut harness, now, 7200, 10800);
    blocker.require_account_observation();
    harness.config.poll_interval_ms = 60_000;
    let (mut client, _, _) = client_at(&harness, now);
    client.start().unwrap();
    client.connect().await.unwrap();
    wait_initial_source_poll(&client, &harness).await;
    let snapshot = client.inner.snapshot.read().clone().unwrap();
    let hash = alloy_primitives::keccak256(serde_json::to_vec(&snapshot).unwrap());
    blocker
        .set_account_observation_valid_until_ns(hash, (now + 60) * 1_000_000_000)
        .unwrap();
    if cancelled {
        harness
            .source
            .trading_delay_ms
            .store(5000, Ordering::SeqCst);
        assert!(
            tokio::time::timeout(
                std::time::Duration::from_millis(30),
                crate::reconciliation::synchronize(&client.inner, snapshot.clone())
            )
            .await
            .is_err()
        );
        harness.source.trading_delay_ms.store(0, Ordering::SeqCst);
    } else {
        let mut invalid = snapshot.clone();
        invalid.balance.0 -= U256::from(1);
        assert!(
            crate::reconciliation::synchronize(&client.inner, invalid)
                .await
                .is_err()
        );
    }
    let (_, command) = order(&harness);
    let entry = legacy_prepared(&client, &command, false, now);
    blocker
        .set_account_observation_valid_until_ns(hash, (now + 60) * 1_000_000_000)
        .unwrap();
    assert!(
        client
            .inner
            .submission_time(&entry.signed().unwrap(), false)
            .is_err()
    );
    let (_, close) = close_order(&harness, &client);
    let closing = legacy_prepared(&client, &close, true, now);
    assert!(
        client
            .inner
            .submission_time(&closing.signed().unwrap(), false)
            .is_ok()
    );
    assert_eq!(harness.source.intent_requests.load(Ordering::SeqCst), 0);
    crate::reconciliation::synchronize(&client.inner, snapshot)
        .await
        .unwrap();
    assert!(
        client
            .inner
            .submission_time(&entry.signed().unwrap(), false)
            .is_ok()
    );
    client.disconnect().await.unwrap();
}

#[tokio::test]
async fn account_bound_checkpoint_commit_cannot_reuse_the_previous_memory_snapshot() {
    let _serial = support::SERIAL.lock().await;
    let mut harness = support::Harness::new().await;
    let now = get_atomic_clock_realtime().get_time_ns().as_u64() / 1_000_000_000;
    let blocker = bind_window(&mut harness, now, 7200, 10800);
    blocker.require_account_observation();
    harness.config.poll_interval_ms = 60_000;
    let (mut client, clock, _) = client_at(&harness, now);
    client.start().unwrap();
    client.connect().await.unwrap();
    wait_initial_source_poll(&client, &harness).await;
    let probe = client.journal_identity_probe().unwrap();
    let mut snapshot = client.inner.snapshot.read().clone().unwrap();
    let original_hash = alloy_primitives::keccak256(serde_json::to_vec(&snapshot).unwrap());
    blocker
        .set_account_observation_valid_until_ns(original_hash, (now + 60) * 1_000_000_000)
        .unwrap();
    let (_, command) = order(&harness);
    let entry = legacy_prepared(&client, &command, false, now);
    let (_, close) = close_order(&harness, &client);
    let closing = legacy_prepared(&client, &close, true, now);
    snapshot.account_revision += 1;
    snapshot.indexed_through_timestamp += 1;
    snapshot.balance.0 -= U256::from(30_000_000_000_000_000_000_u128);
    snapshot.funds.available_balance.0 = snapshot
        .balance
        .0
        .saturating_sub(snapshot.funds.reserved_balance.0);
    let new_hash = alloy_primitives::keccak256(serde_json::to_vec(&snapshot).unwrap());
    clock.set_time(nautilus_core::UnixNanos::from((now + 1) * 1_000_000_000));
    // The original publication mutex pauses real reconciliation after its durable commit.
    let lock_owner = client.inner.clone();
    let (acquired, acquisition) = std::sync::mpsc::channel();
    let (release, release_lock) = std::sync::mpsc::channel();
    let locker = std::thread::spawn(move || {
        let _held = lock_owner.sent_orders.lock();
        acquired.send(()).unwrap();
        release_lock
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap();
    });
    acquisition
        .recv_timeout(std::time::Duration::from_secs(3))
        .unwrap();
    let inner = client.inner.clone();
    let worker = std::thread::spawn(move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(crate::reconciliation::synchronize(&inner, snapshot))
    });
    let committed = tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            let observed: serde_json::Value =
                serde_json::from_str(&probe.account_observation_json().unwrap()).unwrap();
            if observed["account_snapshot_keccak256"] == serde_json::json!(new_hash) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        }
    })
    .await
    .is_ok();
    let memory_hash = alloy_primitives::keccak256(
        serde_json::to_vec(client.inner.snapshot.read().as_ref().unwrap()).unwrap(),
    );
    blocker
        .set_account_observation_valid_until_ns(original_hash, (now + 60) * 1_000_000_000)
        .unwrap();
    let denied = client
        .inner
        .submission_time(&entry.signed().unwrap(), false)
        .is_err();
    let exit_allowed = client
        .inner
        .submission_time(&closing.signed().unwrap(), false)
        .is_ok();
    release.send(()).unwrap();
    locker.join().unwrap();
    let synchronized = worker.join().unwrap();
    client.disconnect().await.unwrap();
    assert!(committed);
    assert_eq!(memory_hash, original_hash);
    assert!(denied);
    assert!(exit_allowed);
    assert!(synchronized.is_ok(), "{synchronized:?}");
    assert_eq!(harness.source.intent_requests.load(Ordering::SeqCst), 0);
}
