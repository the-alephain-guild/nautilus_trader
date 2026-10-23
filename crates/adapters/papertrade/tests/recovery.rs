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

use std::{sync::atomic::Ordering, time::Duration};

use nautilus_model::identifiers::TraderId;
use serde_json::{Value, json};
mod support;
use support::{Harness, SERIAL, closed_history, mark_attempted, prepared_open};

#[tokio::test]
async fn failed_wallet_reconnect_never_restores_old_coverage_on_poll_ticks() {
    let _serial = SERIAL.lock().await;
    let harness = Harness::new().await;
    let mut client = harness.client();
    client.start().unwrap();
    client.connect().await.unwrap();
    harness.source.unavailable.store(true, Ordering::SeqCst);
    let address = harness.source.wallet.borrow()["address"].clone();
    harness.source.wallet.send(json!({"address":address,"baseAccountRevision":999,"accountRevision":1000,"state":{},"addedPositions":[],"removedPositionIds":[],"addedOperations":[],"removedOperationIds":[]})).unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        while client.is_connected() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    for _ in 0..100 {
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(
            !client.is_connected(),
            "A poll tick revived the invalidated account snapshot"
        );
        assert!(client.generate_mass_status(None).await.is_err());
    }
    client.disconnect().await.unwrap();
}

#[tokio::test]
async fn silent_wallet_expires_despite_successful_polling() {
    let _serial = SERIAL.lock().await;
    let harness = Harness::new().await;
    let mut client = harness.client();
    client.start().unwrap();
    client.connect().await.unwrap();
    harness.source.unavailable.store(true, Ordering::SeqCst);
    // The original response stays open but sends neither frames nor heartbeats.
    tokio::time::timeout(Duration::from_secs(38), async {
        while client.is_connected() {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("The idle deadline was reset by polling");
    assert!(client.generate_mass_status(None).await.is_err());
    client.disconnect().await.unwrap();
}

#[tokio::test]
async fn restart_rejects_regressed_coverage_and_keeps_the_durable_checkpoint() {
    let _serial = SERIAL.lock().await;
    let harness = Harness::new().await;
    let current = harness.source.wallet.borrow().clone();
    let mut client = harness.client();
    client.start().unwrap();
    client.connect().await.unwrap();
    client.disconnect().await.unwrap();
    drop(client);
    let mut stale = current.clone();
    stale["indexedThroughBlock"] = json!(current["indexedThroughBlock"].as_u64().unwrap() - 1);
    stale["indexedThroughTimestamp"] =
        json!(current["indexedThroughTimestamp"].as_u64().unwrap() - 1);
    harness.source.wallet.send_replace(stale);
    let mut restarted = harness.client();
    restarted.start().unwrap();
    assert!(
        restarted.connect().await.is_err(),
        "Restart accepted coverage older than its checkpoint"
    );
    assert!(!restarted.is_connected());
    restarted.disconnect().await.unwrap();
    drop(restarted);
    harness.source.wallet.send_replace(current);
    let mut recovered = harness.client();
    recovered.start().unwrap();
    recovered.connect().await.unwrap();
    assert!(recovered.is_connected());
    recovered.disconnect().await.unwrap();
}

#[tokio::test]
async fn failed_position_reconciliation_does_not_advance_restart_checkpoint() {
    let _serial = SERIAL.lock().await;
    let harness = Harness::new().await;
    let current = harness.source.wallet.borrow().clone();
    let mut client = harness.client();
    client.start().unwrap();
    client.connect().await.unwrap();
    client.disconnect().await.unwrap();
    drop(client);
    let mut missing = current.clone();
    missing["indexedThroughBlock"] = json!(current["indexedThroughBlock"].as_u64().unwrap() + 10);
    missing["indexedThroughTimestamp"] =
        json!(current["indexedThroughTimestamp"].as_u64().unwrap() + 10);
    missing["walletRevision"] = missing["indexedThroughBlock"].clone();
    missing["openPositions"] = json!([]);
    missing["locked"] = json!("0");
    harness.source.wallet.send_replace(missing);
    let mut failed = harness.client();
    failed.start().unwrap();
    assert!(failed.connect().await.is_err());
    failed.disconnect().await.unwrap();
    drop(failed);
    harness.source.wallet.send_replace(current);
    let mut recovered = harness.client();
    recovered.start().unwrap();
    recovered
        .connect()
        .await
        .expect("A failed reconciliation poisoned the restart checkpoint");
    recovered.disconnect().await.unwrap();
}

#[tokio::test]
async fn delayed_close_receipt_never_publishes_an_external_identity_for_a_local_fill() {
    use alloy::signers::local::PrivateKeySigner;
    use alloy_primitives::{B256, U256};
    use nautilus_common::messages::execution::SubmitOrder;
    use nautilus_core::{UUID4, UnixNanos};
    use nautilus_model::{
        enums::{OrderSide, OrderStatus, OrderType},
        identifiers::{ClientOrderId, InstrumentId},
        orders::OrderTestBuilder,
    };
    use nautilus_papertrade::{
        journal::Journal,
        reports,
        signing::{Close, PapertradeSigner},
        wire::{TradingState, VenuePosition},
    };
    let _serial = SERIAL.lock().await;
    let harness = Harness::new().await;
    let mut source = harness.source.wallet.borrow().clone();
    let position: VenuePosition =
        serde_json::from_value(source["openPositions"][0].clone()).unwrap();
    let markets: TradingState =
        serde_json::from_value(harness.source.trading.lock().clone()).unwrap();
    let mut client = harness.client();
    client.start().unwrap();
    client.connect().await.unwrap();
    client.disconnect().await.unwrap();
    drop(client);
    let order = OrderTestBuilder::new(OrderType::Market)
        .instrument_id(InstrumentId::from("BTC-USD-PERP.PAPERTRADE"))
        .client_order_id(ClientOrderId::from("PT-CLOSE-RECOVERY"))
        .side(OrderSide::Sell)
        .quantity(reports::quantity(&position, &markets.instruments[0]).unwrap())
        .reduce_only(true)
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
        None,
        UUID4::new(),
        UnixNanos::from(1),
    );
    let now = source["indexedThroughTimestamp"].as_u64().unwrap();
    let signer =
        PapertradeSigner::new(PrivateKeySigner::from_bytes(&B256::repeat_byte(17)).unwrap());
    let journal = Journal::open(
        &harness.config.journal_path,
        source["address"].as_str().unwrap().parse().unwrap(),
        "PAPERTRADE-001",
    )
    .unwrap();
    let (entry, _) = journal
        .prepare(
            &command,
            Some(position.position_id),
            source["indexedThroughBlock"].as_u64().unwrap(),
            |nonce| {
                signer.sign_close(
                    &Close {
                        user: source["address"].as_str().unwrap().parse().unwrap(),
                        positionIds: vec![position.position_id.0],
                        nonce,
                        deadline: now + 3600,
                    },
                    now,
                )
            },
        )
        .unwrap();
    journal
        .begin_submission(entry.command.client_order_id)
        .unwrap()
        .unwrap();
    drop(journal);
    let tx = B256::repeat_byte(31);
    let block = B256::repeat_byte(32);
    let exit = position.entry_price.0 + U256::from(100);
    *harness.source.history.lock() = json!({"items":[{
        "positionId":position.position_id,"instrumentId":position.instrument_id,"symbol":"BTC","priceScaleRaw":"10","isLong":position.is_long,"entryPriceRaw":position.entry_price,"marginRaw":position.margin,"leverage":position.leverage,"openedAtMs":position.opened_at_ms,"timestampMs":now*1000,"txHash":tx,"kind":"PositionClosed","exitPriceRaw":exit.to_string(),"adjustedPnlRaw":"0","userPaidFeeRaw":"0"
    }],"nextCursor":null});
    source["openPositions"].as_array_mut().unwrap().remove(0);
    source["locked"] = json!(source["openPositions"][0][5].as_str().unwrap());
    source["walletRevision"] = source["indexedThroughBlock"].clone();
    source["accountRevision"] = json!(2);
    harness.source.wallet.send_replace(source.clone());
    let mut recovered = harness.client();
    recovered.start().unwrap();
    assert!(
        recovered.connect().await.is_err(),
        "Missing attribution evidence was published as an external close"
    );
    let user: alloy_primitives::Address = source["address"].as_str().unwrap().parse().unwrap();
    let log = json!({"address":nautilus_papertrade::chain::BATCH_EXECUTOR,"topics":[nautilus_papertrade::chain::RESULT_TOPIC,B256::ZERO,B256::ZERO,entry.intent_id],"transactionHash":tx,"blockHash":block,"blockNumber":format!("0x{:x}",source["indexedThroughBlock"].as_u64().unwrap()),"removed":false,"data":format!("0x{:064x}",U256::from(1))});
    let close = json!({"address":nautilus_papertrade::signing::EXCHANGE,"topics":["0x87902ed2451849f465fc4d36a01d7f5c67b53af12f63ee85d2814a2d582f89bf",format!("0x{:064x}",position.position_id.0),format!("0x{:064x}",U256::from_be_slice(user.as_slice()))],"transactionHash":tx,"blockHash":block,"removed":false,"data":format!("0x{exit:064x}{}","0".repeat(256))});
    *harness.source.receipt.lock() = Some(
        json!({"status":"0x1","transactionHash":tx,"blockHash":block,"blockNumber":log["blockNumber"],"logs":[log,close]}),
    );
    recovered.connect().await.unwrap();
    let mass = recovered.generate_mass_status(None).await.unwrap().unwrap();
    let closes: Vec<_> = mass
        .fill_reports()
        .into_values()
        .flatten()
        .filter(|f| {
            f.venue_position_id == Some(reports::position_id(&position))
                && f.order_side == OrderSide::Sell
        })
        .collect();
    assert_eq!(closes.len(), 1);
    assert_eq!(closes[0].client_order_id, Some(command.client_order_id));
    assert_eq!(
        closes[0].venue_order_id.to_string(),
        format!("{:#x}", entry.intent_id)
    );
    assert!(
        mass.order_reports()
            .values()
            .any(|r| r.client_order_id == Some(command.client_order_id)
                && r.order_status == OrderStatus::Filled)
    );
    recovered.disconnect().await.unwrap();
}

#[rstest::rstest]
#[case(false)]
#[case(true)]
#[tokio::test]
async fn newer_chain_evidence_cannot_complete_an_older_empty_account_snapshot(
    #[case] account_covers_execution: bool,
) {
    use alloy_primitives::b256;
    use nautilus_papertrade::wire::VenuePosition;
    let _serial = SERIAL.lock().await;
    let harness = Harness::new().await;
    let mut source = harness.source.wallet.borrow().clone();
    let mut position: VenuePosition =
        serde_json::from_value(source["openPositions"][0].clone()).unwrap();
    source["openPositions"] = json!([]);
    source["locked"] = json!("0");
    let covered = source["indexedThroughBlock"].as_u64().unwrap();
    harness.source.wallet.send_replace(source.clone());
    let now = source["indexedThroughTimestamp"].as_u64().unwrap();
    let entry = prepared_open(&harness, &position, now, now + 3600);
    mark_attempted(&harness, &entry);
    let mut receipt: Value =
        serde_json::from_str(include_str!("../test_data/live_open_receipt.json")).unwrap();
    let old_id = b256!("adfae189b04060dcbef2af7322b00447844d8119b819e1820308e38e86339614");
    let execution_block = covered + u64::from(!account_covers_execution);
    receipt["blockNumber"] = json!(format!("0x{execution_block:x}"));
    for log in receipt["logs"].as_array_mut().unwrap() {
        log["blockNumber"] = json!(format!("0x{execution_block:x}"));
        if log["topics"][3] == json!(old_id) {
            log["topics"][3] = json!(entry.intent_id);
        }
    }
    *harness.source.receipt.lock() = Some(receipt);
    harness.source.trading.lock()["indexedThroughBlock"] = json!(covered + 2);
    let mut client = harness.client();
    client.start().unwrap();
    let connected = client.connect().await;
    if account_covers_execution {
        assert!(
            connected.is_err(),
            "Covered execution disappeared from the account without terminal history"
        );
    } else {
        connected.unwrap();
        let mass = client.generate_mass_status(None).await.unwrap().unwrap();
        assert!(
            !mass.reports_complete(),
            "Future chain fill was paired with authoritative empty positions"
        );
        assert!(mass.fill_reports().is_empty());
        assert!(mass.position_reports().is_empty());
    }
    client.disconnect().await.unwrap();
    drop(client);
    position.intent_id = Some(entry.intent_id);
    source["openPositions"] = json!([position]);
    source["locked"] = json!(position.margin);
    source["indexedThroughBlock"] = json!(covered + 2);
    source["walletRevision"] = json!(covered + 1);
    source["accountRevision"] = json!(2);
    harness.source.wallet.send_replace(source);
    let mut restarted = harness.client();
    restarted.start().unwrap();
    restarted.connect().await.unwrap();
    let mass = restarted.generate_mass_status(None).await.unwrap().unwrap();
    assert!(mass.reports_complete());
    assert_eq!(
        mass.position_reports()
            .values()
            .map(Vec::len)
            .sum::<usize>(),
        1
    );
    assert_eq!(mass.fill_reports().values().map(Vec::len).sum::<usize>(), 1);
    assert_eq!(
        mass.fill_reports()
            .values()
            .flatten()
            .next()
            .unwrap()
            .client_order_id,
        Some(entry.command.client_order_id)
    );
    restarted.disconnect().await.unwrap();
}

#[rstest::rstest]
#[case("symbol",json!("ETH_CHANGED"))]
#[case("priceScaleRaw",json!("10"))]
#[tokio::test]
async fn invalid_history_market_does_not_poison_recovery(
    #[case] field: &str,
    #[case] invalid: Value,
) {
    let _serial = SERIAL.lock().await;
    let harness = Harness::new().await;
    let mut history: Value =
        serde_json::from_str(include_str!("../test_data/live_history.json")).unwrap();
    let correct = history.clone();
    history["items"][0][field] = invalid;
    *harness.source.history.lock() = history;
    let mut client = harness.client();
    client.start().unwrap();
    assert!(client.connect().await.is_err());
    client.disconnect().await.unwrap();
    drop(client);
    *harness.source.history.lock() = correct;
    let mut recovered = harness.client();
    recovered.start().unwrap();
    recovered
        .connect()
        .await
        .expect("Invalid history was persisted before validation");
    recovered.disconnect().await.unwrap();
}

#[tokio::test]
async fn restart_rejects_a_durable_order_that_never_started_submission() {
    use nautilus_model::enums::OrderStatus;
    use nautilus_papertrade::wire::VenuePosition;
    let _serial = SERIAL.lock().await;
    let harness = Harness::new().await;
    let mut source = harness.source.wallet.borrow().clone();
    let position: VenuePosition =
        serde_json::from_value(source["openPositions"][0].clone()).unwrap();
    source["openPositions"] = json!([]);
    source["locked"] = json!("0");
    harness.source.wallet.send_replace(source.clone());
    let now = source["indexedThroughTimestamp"].as_u64().unwrap();
    let entry = prepared_open(&harness, &position, now, now + 3600);
    // Drop/reopen after the real durable prepare, before any transport invocation.
    let mut client = harness.client();
    client.start().unwrap();
    client.connect().await.unwrap();
    let mass = client.generate_mass_status(None).await.unwrap().unwrap();
    let order = mass
        .order_reports()
        .into_values()
        .find(|r| r.client_order_id == Some(entry.command.client_order_id))
        .unwrap();
    assert_eq!(
        order.order_status,
        OrderStatus::Rejected,
        "An unsent request blocked recovery forever"
    );
    assert!(mass.reports_complete());
    assert!(mass.fill_reports().is_empty());
    client.submit_order(entry.command).unwrap();
    client.disconnect().await.unwrap();
}

#[rstest::rstest]
#[case(false)]
#[case(true)]
#[tokio::test]
async fn ambiguous_attempt_expires_only_after_complete_chain_coverage_past_its_deadline(
    #[case] legacy_expired: bool,
) {
    use nautilus_model::enums::OrderStatus;
    use nautilus_papertrade::wire::VenuePosition;
    let _serial = SERIAL.lock().await;
    let harness = Harness::new().await;
    let mut source = harness.source.wallet.borrow().clone();
    let position: VenuePosition =
        serde_json::from_value(source["openPositions"][0].clone()).unwrap();
    source["openPositions"] = json!([]);
    source["locked"] = json!("0");
    harness.source.wallet.send_replace(source.clone());
    let now = source["indexedThroughTimestamp"].as_u64().unwrap();
    let deadline = now - 3600;
    harness.source.trading.lock()["indexedThroughBlock"] = source["indexedThroughBlock"].clone();
    let entry = prepared_open(&harness, &position, now - 7200, deadline);
    mark_attempted(&harness, &entry);
    if legacy_expired {
        support::write_legacy_expired(&harness.config.journal_path, &entry);
    }
    *harness.source.block_timestamp.lock() = Some(deadline - 1);
    let mut client = harness.client();
    client.start().unwrap();
    client.connect().await.unwrap();
    let mass = client.generate_mass_status(None).await.unwrap().unwrap();
    assert!(
        !mass.reports_complete(),
        "Local time alone terminated an ambiguous intent"
    );
    assert!(
        mass.order_reports()
            .values()
            .any(|r| r.client_order_id == Some(entry.command.client_order_id)
                && r.order_status == OrderStatus::Submitted)
    );
    client.disconnect().await.unwrap();
    drop(client);
    *harness.source.block_timestamp.lock() = None;
    let mut unavailable = harness.client();
    unavailable.start().unwrap();
    assert!(
        unavailable.connect().await.is_err(),
        "Missing block evidence was treated as expiry"
    );
    unavailable.disconnect().await.unwrap();
    drop(unavailable);
    *harness.source.block_timestamp.lock() = Some(now);
    let mut restarted = harness.client();
    restarted.start().unwrap();
    restarted.connect().await.unwrap();
    let mass = restarted.generate_mass_status(None).await.unwrap().unwrap();
    assert!(
        mass.order_reports()
            .values()
            .any(|r| r.client_order_id == Some(entry.command.client_order_id)
                && r.order_status == OrderStatus::Expired),
        "Complete post-deadline execution coverage did not resolve the unknown order"
    );
    assert!(mass.reports_complete());
    assert!(mass.fill_reports().is_empty());
    restarted.disconnect().await.unwrap();
}

#[rstest::rstest]
#[case::chain(false, "chain")]
#[case::legacy_expired(true, "chain")]
#[case::account_snapshot(false, "snapshot")]
#[case::account_patch(false, "patch")]
#[case::other_order_failure(false, "multi_order")]
#[case::process_kill(false, "process_kill")]
#[tokio::test]
async fn successful_close_cannot_expire_while_history_is_delayed_across_restart(
    #[case] legacy_expired: bool,
    #[case] success_source: &str,
) {
    use alloy::signers::local::PrivateKeySigner;
    use alloy_primitives::{B256, U256};
    use nautilus_common::messages::execution::SubmitOrder;
    use nautilus_core::{UUID4, UnixNanos};
    use nautilus_model::{
        enums::{OrderSide, OrderStatus, OrderType},
        identifiers::{ClientOrderId, InstrumentId},
        orders::OrderTestBuilder,
    };
    use nautilus_papertrade::{
        journal::Journal,
        reports,
        signing::{Close, PapertradeSigner},
        wire::{TradingState, VenuePosition},
    };
    if let Ok(child) = std::env::var("PAPERTRADE_TEST_INTERRUPTED_SYNC") {
        use nautilus_common::{
            cache::Cache, factories::ExecutionClientFactory,
            live::runner::replace_exec_event_sender,
        };
        use nautilus_papertrade::{
            config::{PAPERTRADE, PapertradeExecutionClientConfig},
            factories::PapertradeExecutionClientFactory,
        };
        let child: Value = serde_json::from_str(&child).unwrap();
        let mut config = PapertradeExecutionClientConfig::new(
            "PAPERTRADE-001".into(),
            child["wallet"].as_str().unwrap().into(),
            child["path"].as_str().unwrap().into(),
            "test-build".into(),
        );
        config.origin = child["origin"].as_str().unwrap().into();
        config.rpc_url = format!("{}rpc", config.origin);
        config.session_private_key = Some(format!("0x{}", "11".repeat(32)).into());
        let (sender, _events) = tokio::sync::mpsc::unbounded_channel();
        replace_exec_event_sender(sender);
        let cache = std::rc::Rc::new(std::cell::RefCell::new(Cache::default()));
        let mut client = PapertradeExecutionClientFactory
            .create(
                TraderId::from("TESTER-001"),
                PAPERTRADE,
                &config,
                cache.into(),
            )
            .unwrap();
        client.start().unwrap();
        client.connect().await.unwrap();
        panic!("Parent must terminate the child at the blocked coverage RPC");
    }
    let _serial = SERIAL.lock().await;
    let mut harness = Harness::new().await;
    let mut source = harness.source.wallet.borrow().clone();
    let position: VenuePosition =
        serde_json::from_value(source["openPositions"][0].clone()).unwrap();
    let markets: TradingState =
        serde_json::from_value(harness.source.trading.lock().clone()).unwrap();
    let order = OrderTestBuilder::new(OrderType::Market)
        .instrument_id(InstrumentId::from("BTC-USD-PERP.PAPERTRADE"))
        .client_order_id(ClientOrderId::from("PT-SUCCESS-CLOSE"))
        .side(OrderSide::Sell)
        .quantity(reports::quantity(&position, &markets.instruments[0]).unwrap())
        .reduce_only(true)
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
        None,
        UUID4::new(),
        UnixNanos::from(1),
    );
    let now = source["indexedThroughTimestamp"].as_u64().unwrap();
    let wallet = source["address"].as_str().unwrap().parse().unwrap();
    let signer =
        PapertradeSigner::new(PrivateKeySigner::from_bytes(&B256::repeat_byte(17)).unwrap());
    let journal = Journal::open(&harness.config.journal_path, wallet, "PAPERTRADE-001").unwrap();
    let (entry, _) = journal
        .prepare(
            &command,
            Some(position.position_id),
            source["indexedThroughBlock"].as_u64().unwrap(),
            |nonce| {
                signer.sign_close(
                    &Close {
                        user: wallet,
                        positionIds: vec![position.position_id.0],
                        nonce,
                        deadline: now - 3600,
                    },
                    now - 7200,
                )
            },
        )
        .unwrap();
    journal
        .begin_submission(entry.command.client_order_id)
        .unwrap()
        .unwrap();
    drop(journal);
    harness.source.trading.lock()["indexedThroughBlock"] = source["indexedThroughBlock"].clone();
    *harness.source.block_timestamp.lock() = Some(now);
    let tx = B256::repeat_byte(31);
    let block = B256::repeat_byte(32);
    let height = format!("0x{:x}", source["indexedThroughBlock"].as_u64().unwrap());
    let result = json!({"address":nautilus_papertrade::chain::BATCH_EXECUTOR,
        "topics":[nautilus_papertrade::chain::RESULT_TOPIC,B256::ZERO,B256::ZERO,entry.intent_id],
        "transactionHash":tx,"blockHash":block,"blockNumber":height,"removed":false,
        "data":format!("0x{:064x}",U256::from(1))});
    let exit = position.entry_price.0 + U256::from(100);
    let close = json!({"address":nautilus_papertrade::signing::EXCHANGE,
        "topics":["0x87902ed2451849f465fc4d36a01d7f5c67b53af12f63ee85d2814a2d582f89bf",format!("0x{:064x}",position.position_id.0),format!("0x{:064x}",U256::from_be_slice(wallet.as_slice()))],
        "transactionHash":tx,"blockHash":block,"blockNumber":height,"removed":false,
        "data":format!("0x{exit:064x}{}","0".repeat(256))});
    let receipt = json!({"status":"0x1","transactionHash":tx,"blockHash":block,"blockNumber":height,"logs":[result,close]});
    if legacy_expired {
        support::write_legacy_expired(&harness.config.journal_path, &entry);
    }
    if matches!(success_source, "snapshot" | "multi_order" | "process_kill") {
        source["intentOutcomes"] = json!([[entry.intent_id, "succeeded"]]);
        harness.source.wallet.send_replace(source.clone());
        if matches!(success_source, "multi_order" | "process_kill") {
            let other = prepared_open(&harness, &position, now - 7200, now - 3600);
            mark_attempted(&harness, &other);
            *harness.source.block_timestamp.lock() = None;
        }
    } else if success_source == "patch" {
        *harness.source.block_timestamp.lock() = Some(now - 3601);
    } else {
        *harness.source.receipt.lock() = Some(receipt.clone());
    }
    for reconnect in 0..2 {
        if success_source == "process_kill" && reconnect == 0 {
            use std::process::{Command, Stdio};
            harness.source.block_paused.store(true, Ordering::SeqCst);
            let mut child = Command::new(std::env::current_exe().unwrap())
                .args(["--exact", std::thread::current().name().unwrap(), "--nocapture"])
                .env("PAPERTRADE_TEST_INTERRUPTED_SYNC", json!({"wallet":source["address"], "path":harness.config.journal_path, "origin":harness.config.origin}).to_string())
                .stdout(Stdio::null()).stderr(Stdio::inherit()).spawn().unwrap();
            let reached = tokio::time::timeout(Duration::from_secs(8), async {
                while harness.source.block_calls.load(Ordering::SeqCst) == 0 {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await;
            child.kill().unwrap();
            let status = child.wait().unwrap();
            harness.source.block_paused.store(false, Ordering::SeqCst);
            reached.unwrap();
            assert!(!status.success());
            #[cfg(unix)]
            {
                use std::os::unix::process::ExitStatusExt;
                assert_eq!(status.signal(), Some(9));
            }
            let journal =
                Journal::open(&harness.config.journal_path, wallet, "PAPERTRADE-001").unwrap();
            let saved = journal
                .entries()
                .unwrap()
                .into_iter()
                .find(|e| e.intent_id == entry.intent_id)
                .unwrap();
            assert_eq!(
                saved.scanned_through,
                source["indexedThroughBlock"].as_u64().unwrap()
            );
            assert!(
                saved.execution_succeeded,
                "Interrupted synchronization advanced coverage without retaining stream success"
            );
            drop(journal);
            *harness.source.block_timestamp.lock() = Some(now);
            source["intentOutcomes"] = json!([]);
            source["accountRevision"] = json!(3);
            harness.source.wallet.send_replace(source.clone());
            continue;
        }
        let mut client = harness.client();
        client.start().unwrap();
        if success_source == "multi_order" && reconnect == 0 {
            assert!(client.connect().await.is_err());
            client.disconnect().await.unwrap();
            drop(client);
            let journal =
                Journal::open(&harness.config.journal_path, wallet, "PAPERTRADE-001").unwrap();
            let saved = journal
                .entries()
                .unwrap()
                .into_iter()
                .find(|e| e.intent_id == entry.intent_id)
                .unwrap();
            assert!(
                saved.execution_succeeded,
                "Another order failure discarded an observed success"
            );
            drop(journal);
            *harness.source.block_timestamp.lock() = Some(now);
            source["intentOutcomes"] = json!([]);
            source["accountRevision"] = json!(3);
            harness.source.wallet.send_replace(source.clone());
            continue;
        }
        client.connect().await.unwrap();
        if success_source == "patch" && reconnect == 0 {
            harness.source.wallet.send_replace(json!({
                "address":source["address"], "baseAccountRevision":1, "accountRevision":2,
                "state":{}, "intentOutcomes":[[entry.intent_id,"succeeded"]],
                "addedPositions":[], "removedPositionIds":[], "addedOperations":[], "removedOperationIds":[]
            }));
            tokio::time::timeout(Duration::from_secs(3), async {
                loop {
                    if let Some(nautilus_common::messages::ExecutionEvent::Account(state)) =
                        harness._events.recv().await
                        && state
                            .info
                            .as_ref()
                            .and_then(|info| info.get("papertrade_account_revision"))
                            == Some(&json!(2))
                    {
                        break;
                    }
                }
            })
            .await
            .unwrap();
            *harness.source.block_timestamp.lock() = Some(now);
        }
        let mass = client.generate_mass_status(None).await.unwrap().unwrap();
        let report = mass
            .order_reports()
            .into_values()
            .find(|r| r.client_order_id == Some(command.client_order_id))
            .unwrap();
        assert!(
            matches!(
                report.order_status,
                OrderStatus::Submitted | OrderStatus::Accepted
            ),
            "A successful close was terminated before its history arrived, reconnect={reconnect}"
        );
        assert!(!mass.reports_complete());
        client.disconnect().await.unwrap();
        drop(client);
        let journal =
            Journal::open(&harness.config.journal_path, wallet, "PAPERTRADE-001").unwrap();
        let saved = journal.entries().unwrap().pop().unwrap();
        assert!(saved.execution_succeeded);
        assert_eq!(
            saved.scanned_through,
            source["indexedThroughBlock"].as_u64().unwrap()
        );
        drop(journal);
        // The next scan starts after this receipt; success must survive in the journal.
        *harness.source.receipt.lock() = None;
        if success_source != "chain" {
            source["intentOutcomes"] = json!([]);
            source["accountRevision"] = json!(3);
            harness.source.wallet.send_replace(source.clone());
        }
    }
    *harness.source.history.lock() = json!({"items":[{"positionId":position.position_id,
        "instrumentId":position.instrument_id,"symbol":"BTC","priceScaleRaw":"10",
        "isLong":position.is_long,"entryPriceRaw":position.entry_price,"marginRaw":position.margin,
        "leverage":position.leverage,"openedAtMs":position.opened_at_ms,"timestampMs":now*1000,
        "txHash":tx,"kind":"PositionClosed","exitPriceRaw":exit.to_string(),"adjustedPnlRaw":"0","userPaidFeeRaw":"0"}],"nextCursor":null});
    source["openPositions"].as_array_mut().unwrap().remove(0);
    source["locked"] = source["openPositions"][0][5].clone();
    source["accountRevision"] = json!(4);
    source["walletRevision"] = source["indexedThroughBlock"].clone();
    harness.source.wallet.send_replace(source);
    *harness.source.receipt.lock() = Some(receipt);
    let mut recovered = harness.client();
    recovered.start().unwrap();
    recovered.connect().await.unwrap();
    let mass = recovered.generate_mass_status(None).await.unwrap().unwrap();
    assert!(mass.reports_complete());
    assert!(
        mass.order_reports()
            .values()
            .any(|r| r.client_order_id == Some(command.client_order_id)
                && r.order_status == OrderStatus::Filled)
    );
    recovered.disconnect().await.unwrap();
}

#[tokio::test]
async fn deferred_external_history_is_retried_after_local_open_is_proven_expired() {
    use nautilus_model::enums::OrderStatus;
    use nautilus_papertrade::wire::VenuePosition;
    let _serial = SERIAL.lock().await;
    let harness = Harness::new().await;
    let mut source = harness.source.wallet.borrow().clone();
    let position: VenuePosition =
        serde_json::from_value(source["openPositions"][0].clone()).unwrap();
    let now = source["indexedThroughTimestamp"].as_u64().unwrap();
    source["openPositions"] = json!([]);
    source["locked"] = json!("0");
    harness.source.wallet.send_replace(source.clone());
    harness.source.trading.lock()["indexedThroughBlock"] = source["indexedThroughBlock"].clone();
    let entry = prepared_open(&harness, &position, now - 7200, now - 3600);
    mark_attempted(&harness, &entry);
    *harness.source.block_timestamp.lock() = Some(now - 3601);
    *harness.source.history.lock() =
        json!({"items":[closed_history(&position, now * 1000)],"nextCursor":null});
    let mut client = harness.client();
    client.start().unwrap();
    client.connect().await.unwrap();
    let mass = client.generate_mass_status(None).await.unwrap().unwrap();
    assert!(!mass.reports_complete());
    assert!(mass.fill_reports().is_empty());
    *harness.source.block_timestamp.lock() = Some(now);
    let recovered = tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            let mass = client.generate_mass_status(None).await.unwrap().unwrap();
            if mass.reports_complete() {
                break mass;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    client.disconnect().await.unwrap();
    let mass = recovered.unwrap();
    assert_eq!(mass.fill_reports().values().map(Vec::len).sum::<usize>(), 2);
    assert!(
        mass.order_reports()
            .values()
            .any(|r| r.client_order_id == Some(entry.command.client_order_id)
                && r.order_status == OrderStatus::Expired)
    );
}

#[tokio::test]
async fn retained_close_history_cannot_be_replayed_with_an_older_snapshot_after_failed_round() {
    use nautilus_papertrade::wire::VenuePosition;
    let _serial = SERIAL.lock().await;
    let harness = Harness::new().await;
    let old = harness.source.wallet.borrow().clone();
    let positions: Vec<VenuePosition> =
        serde_json::from_value(old["openPositions"].clone()).unwrap();
    let mut client = harness.client();
    client.start().unwrap();
    client.connect().await.unwrap();
    client.disconnect().await.unwrap();
    drop(client);
    let mut newer = old.clone();
    newer["accountRevision"] = json!(2);
    newer["indexedThroughBlock"] = json!(old["indexedThroughBlock"].as_u64().unwrap() + 10);
    newer["walletRevision"] = newer["indexedThroughBlock"].clone();
    newer["indexedThroughTimestamp"] = json!(old["indexedThroughTimestamp"].as_u64().unwrap() + 10);
    newer["openPositions"] = json!([]);
    newer["locked"] = json!("0");
    let time = newer["indexedThroughTimestamp"].as_u64().unwrap() * 1000;
    *harness.source.history.lock() =
        json!({"items":[closed_history(&positions[0],time)],"nextCursor":null});
    harness.source.wallet.send_replace(newer.clone());
    let mut failed = harness.client();
    failed.start().unwrap();
    assert!(
        failed.connect().await.is_err(),
        "Missing close for the second position must defer publication"
    );
    failed.disconnect().await.unwrap();
    drop(failed);
    harness.source.wallet.send_replace(old);
    *harness.source.history.lock() = json!({"items":[],"nextCursor":null});
    let mut stale = harness.client();
    stale.start().unwrap();
    assert!(
        stale.connect().await.is_err(),
        "Persisted future close was combined with an older open position"
    );
    stale.disconnect().await.unwrap();
    drop(stale);
    harness.source.wallet.send_replace(newer);
    *harness.source.history.lock() = json!({"items":positions.iter().map(|p|closed_history(p,time)).collect::<Vec<_>>(),"nextCursor":null});
    let mut recovered = harness.client();
    recovered.start().unwrap();
    recovered.connect().await.unwrap();
    let mass = recovered.generate_mass_status(None).await.unwrap().unwrap();
    assert!(mass.reports_complete());
    assert!(mass.position_reports().is_empty());
    assert_eq!(mass.fill_reports().values().map(Vec::len).sum::<usize>(), 4);
    recovered.disconnect().await.unwrap();
}

#[tokio::test]
async fn slow_history_cannot_keep_a_silent_account_tradeable_past_the_idle_deadline() {
    use nautilus_common::messages::{ExecutionEvent, execution::SubmitOrder};
    use nautilus_core::{UUID4, UnixNanos};
    use nautilus_model::{
        enums::OrderType,
        events::OrderEventAny,
        identifiers::{ClientOrderId, InstrumentId},
        orders::{Order, OrderTestBuilder},
        types::Quantity,
    };
    let _serial = SERIAL.lock().await;
    let mut harness = Harness::new().await;
    let mut client = harness.client();
    client.start().unwrap();
    client.connect().await.unwrap();
    let calls = harness.source.history_calls.load(Ordering::SeqCst);
    harness
        .source
        .history_delay_ms
        .store(19_000, Ordering::SeqCst);
    let mut snapshot = harness.source.wallet.borrow().clone();
    snapshot["accountRevision"] = json!(2);
    snapshot["indexedThroughBlock"] = json!(snapshot["indexedThroughBlock"].as_u64().unwrap() + 1);
    snapshot["walletRevision"] = snapshot["indexedThroughBlock"].clone();
    snapshot["balance"] = json!("50000000000000000000");
    snapshot["funds"]["availableBalance"] = snapshot["balance"].clone();
    harness.source.wallet.send_replace(snapshot);
    tokio::time::timeout(Duration::from_secs(3), async {
        while harness.source.history_calls.load(Ordering::SeqCst) <= calls {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    harness.source.unavailable.store(true, Ordering::SeqCst);
    // Two independently bounded pages keep reconciliation busy beyond the stream deadline.
    tokio::time::sleep(Duration::from_secs(36)).await;
    let connected = client.is_connected();
    let reports_unavailable = client.generate_mass_status(None).await.is_err();
    let order = OrderTestBuilder::new(OrderType::Market)
        .instrument_id(InstrumentId::from("BTC-USD-PERP.PAPERTRADE"))
        .client_order_id(ClientOrderId::from("PT-STALE-ACCOUNT"))
        .quantity(Quantity::from(1660))
        .quote_quantity(true)
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
        None,
        UUID4::new(),
        UnixNanos::from(1),
    );
    client.submit_order(command).unwrap();
    let mut denied = false;
    while let Ok(event) = harness._events.try_recv() {
        if let ExecutionEvent::Order(OrderEventAny::Denied(event)) = event {
            denied |= event.client_order_id == order.client_order_id()
                && event.reason.contains("unavailable");
        }
    }
    client.disconnect().await.unwrap();
    assert!(
        !connected,
        "Slow reconciliation kept a silent account connected past its deadline"
    );
    assert!(reports_unavailable);
    assert!(denied, "An order was admitted using the stale cash balance");
}
