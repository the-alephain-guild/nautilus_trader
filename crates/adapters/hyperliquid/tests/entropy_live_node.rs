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

#[path = "support/scenario.rs"]
mod scenario;
#[path = "support/entropy.rs"]
mod support;
use nautilus_model::{enums::OrderStatus, orders::Order};
use rstest::rstest;
use rust_decimal::Decimal;
use scenario::{Retained, economics, history, live_round_trip};
use serde_json::json;
use support::{Harness, SERIAL, run_to_running};

#[tokio::test]
async fn entropy_live_node_starts_from_an_empty_account_cache() {
    let _serial = SERIAL.lock().await;
    let harness = Harness::new(false).await;
    let mut node = harness.node().await;
    assert!(
        node.kernel()
            .cache()
            .borrow()
            .accounts_all_owned()
            .is_empty()
    );
    run_to_running(&mut node).await;
    assert_eq!(node.kernel().cache().borrow().accounts_all_owned().len(), 1);
    assert!(harness.source.actions.lock().is_empty());
    node.dispose();
}

#[rstest]
#[case::io_first(true, 110000)]
#[case::io_after_another_deployer(false, 120000)]
#[tokio::test]
async fn entropy_live_node_routes_orders_and_replays_rest_without_duplicate_economics(
    #[case] io_first: bool,
    #[case] expected_asset: u64,
) {
    let _serial = SERIAL.lock().await;
    let harness = Harness::new(io_first).await;
    let (retained, _, fills, ids) = live_round_trip(&harness).await;
    {
        let actions = harness.source.actions.lock();
        assert_eq!(actions.len(), 2);
        for (i, signed) in actions.iter().enumerate() {
            assert!(signed["signature"].is_object());
            assert!(signed["nonce"].is_u64());
            assert_eq!(signed["action"]["orders"].as_array().unwrap().len(), 1);
            let request = &signed["action"]["orders"][0];
            assert_eq!(request["a"], expected_asset);
            assert_eq!(request["b"], i == 0);
            assert_eq!(request["r"], i == 1);
            assert_eq!(request["c"], fills[i]["cloid"]);
            assert_eq!(
                request["s"].as_str().unwrap().parse::<Decimal>().unwrap(),
                Decimal::from(2)
            );
        }
    }
    *harness.source.fills.lock() = vec![fills[1].clone(), fills[0].clone(), fills[0].clone()];
    *harness.source.history.lock() = vec![history(&fills[1]), history(&fills[0])];
    let mut restarted = harness.node().await;
    retained.restore(&restarted);
    run_to_running(&mut restarted).await;
    assert_eq!(economics(&restarted), ids);
    assert_eq!(
        harness.source.actions.lock().len(),
        2,
        "Restart resubmitted orders"
    );
    for kind in ["frontendOpenOrders", "clearinghouseState"] {
        assert!(
            harness
                .source
                .queries
                .lock()
                .iter()
                .any(|q| q["type"] == kind && q["dex"] == "io")
        );
    }
    restarted.dispose();
}

#[rstest]
#[case::missing_orders(true)]
#[case::orders_lagging_behind_retained_position(false)]
#[tokio::test]
async fn entropy_live_node_recovers_retained_positions_with_missing_or_lagging_orders(
    #[case] missing: bool,
) {
    let _serial = SERIAL.lock().await;
    let harness = Harness::new(false).await;
    let (mut retained, accepted, fills, ids) = live_round_trip(&harness).await;
    retained.orders = if missing { Vec::new() } else { accepted };
    *harness.source.fills.lock() = vec![fills[1].clone(), fills[0].clone(), fills[1].clone()];
    *harness.source.history.lock() = vec![history(&fills[1]), history(&fills[0])];
    let mut recovered = harness.node().await;
    retained.restore(&recovered);
    {
        let cache = recovered.kernel().cache();
        let cache = cache.borrow();
        assert_eq!(cache.positions(None, None, None, None, None).len(), 1);
        assert_eq!(
            cache.orders(None, None, None, None, None).len(),
            if missing { 0 } else { 2 }
        );
        for order in &retained.orders {
            assert_eq!(order.status(), OrderStatus::Accepted);
            assert!(order.trade_ids().is_empty());
        }
    }
    run_to_running(&mut recovered).await;
    assert_eq!(economics(&recovered), ids);
    let healed = Retained::capture(&recovered);
    recovered.dispose();
    drop(recovered);
    let mut restarted = harness.node().await;
    healed.restore(&restarted);
    run_to_running(&mut restarted).await;
    assert_eq!(economics(&restarted), ids);
    assert_eq!(
        harness.source.actions.lock().len(),
        2,
        "Recovery resubmitted an order"
    );
    restarted.dispose();
}

#[rstest]
#[case::conflicting_cloid("cloid")]
#[case::other_deployer_same_base_symbol("coin")]
#[case::opposite_side("side")]
#[tokio::test]
async fn entropy_live_node_rejects_conflicting_retained_order_identity(#[case] field: &str) {
    let _serial = SERIAL.lock().await;
    let harness = Harness::new(false).await;
    let (mut retained, _, fills, ids) = live_round_trip(&harness).await;
    retained.orders.clear();
    let mut rows = vec![history(&fills[1]), history(&fills[0])];
    rows[1]["order"][field] = json!(match field {
        "cloid" => "0x22222222222222222222222222222222",
        "coin" => "xyz:SNDK",
        "side" => "A",
        _ => unreachable!(),
    });
    *harness.source.fills.lock() = fills;
    *harness.source.history.lock() = rows;
    let mut recovered = harness.node().await;
    retained.restore(&recovered);
    let result = tokio::time::timeout(std::time::Duration::from_secs(5), recovered.run()).await;
    assert!(!recovered.handle().is_running());
    let result = result.expect("Identity conflict did not terminate startup");
    let error = result.expect_err("Conflicting history must not authorize recovery");
    assert!(
        format!("{error:#}").contains("conflicts with retained"),
        "Unexpected startup error: {error:#}"
    );
    {
        let cache = recovered.kernel().cache();
        let cache = cache.borrow();
        assert!(cache.orders(None, None, None, None, None).is_empty());
        let positions = cache.positions(None, None, None, None, None);
        assert_eq!(positions.len(), 1);
        let position = positions[0].as_ref();
        assert_eq!(
            position
                .events
                .iter()
                .map(|e| e.trade_id)
                .collect::<Vec<_>>(),
            ids
        );
        assert_eq!(position.events.len(), 2);
        assert_eq!(
            position.realized_pnl.unwrap().as_decimal(),
            Decimal::from(20)
        );
        assert_eq!(position.commissions()[0].as_decimal(), Decimal::new(5, 1));
    }
    assert_eq!(harness.source.actions.lock().len(), 2);
    recovered.dispose();
}
