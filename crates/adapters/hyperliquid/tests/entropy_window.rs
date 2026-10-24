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

#[path = "support/priced.rs"]
mod priced;
#[path = "support/scenario.rs"]
#[allow(
    dead_code,
    reason = "Shared scenario helpers include other recovery paths"
)]
mod scenario;
#[path = "support/entropy.rs"]
mod support;
use nautilus_model::{enums::OrderStatus, orders::Order};
use rstest::rstest;
use rust_decimal::Decimal;
use scenario::{Retained, history};
use serde_json::{Value, json};
use support::{Harness, SERIAL, run_to_running};

fn truncated(fills: &[Value]) -> Vec<Value> {
    let mut rows = vec![fills[2].clone(), fills[1].clone()];
    // A real endpoint-sized window includes delisted markets absent from current
    // metadata; their rows must not certify the missing Entropy fill.
    for index in 0..1998 {
        let mut row = fills[2].clone();
        row["coin"] = json!("DELISTED");
        row["oid"] = json!(100000 + index);
        row["tid"] = json!(100000 + index);
        rows.push(row);
    }
    rows
}

#[rstest]
#[case::retained_proof(true)]
#[case::missing_proof(false)]
#[tokio::test]
async fn truncated_fill_window_never_infers_execution_at_the_limit(#[case] proof: bool) {
    let _serial = SERIAL.lock().await;
    let harness = Harness::new(false).await;
    let (mut retained, accepted, fills, ids) =
        priced::round_trip(&harness, ["90.00", "95.00"], "115.00").await;
    let mut open = history(&fills[1]);
    open["order"]["limitPx"] = json!("100.00");
    let mut close = history(&fills[2]);
    close["order"]["limitPx"] = json!("110.00");
    *harness.source.history.lock() = vec![close, open];
    *harness.source.fills.lock() = truncated(&fills);

    if proof {
        retained.orders.clear();
    } else {
        retained.orders = accepted;
        retained.positions.clear();
    }
    let mut recovered = harness.node().await;
    retained.restore(&recovered);
    if proof {
        run_to_running(&mut recovered).await;
    } else {
        let result = tokio::time::timeout(std::time::Duration::from_secs(5), recovered.run()).await;
        let error = result
            .expect("Incomplete fill coverage was admitted into Running")
            .expect_err("Missing execution proof must fail startup");
        assert!(format!("{error:#}").contains("fill coverage"));
        {
            let cache = recovered.kernel().cache();
            let cache = cache.borrow();
            assert!(cache.positions(None, None, None, None, None).is_empty());
            assert_eq!(cache.orders(None, None, None, None, None).len(), 2);
            for order in cache.orders(None, None, None, None, None) {
                assert_eq!(order.status(), OrderStatus::Accepted);
                assert!(order.trade_ids().is_empty());
                assert!(order.commissions().is_empty());
            }
        }
        recovered.dispose();
        drop(recovered);
        *harness.source.fills.lock() = fills.clone();
        recovered = harness.node().await;
        retained.restore(&recovered);
        run_to_running(&mut recovered).await;
    }
    priced::check(
        &recovered,
        &ids,
        Decimal::new(925, 1),
        Decimal::from(115),
        Decimal::from(45),
    );
    let healed = Retained::capture(&recovered);
    recovered.dispose();
    drop(recovered);
    *harness.source.fills.lock() = truncated(&fills);
    let mut restarted = harness.node().await;
    healed.restore(&restarted);
    run_to_running(&mut restarted).await;
    priced::check(
        &restarted,
        &ids,
        Decimal::new(925, 1),
        Decimal::from(115),
        Decimal::from(45),
    );
    assert_eq!(harness.source.actions.lock().len(), 2);
    restarted.dispose();
}
