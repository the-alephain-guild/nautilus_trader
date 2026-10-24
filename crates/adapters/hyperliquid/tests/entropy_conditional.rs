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
#[allow(dead_code, reason = "Shared helpers include standard-limit cases")]
mod scenario;
#[path = "support/entropy.rs"]
mod support;
use nautilus_core::UUID4;
use nautilus_model::{
    enums::{OrderSide, OrderType, TimeInForce, TriggerType},
    identifiers::{ClientOrderId, StrategyId, TraderId},
    orders::{LimitIfTouchedOrder, Order, OrderAny},
    types::{Price, Quantity},
};
use scenario::{Retained, economics, history, live_round_trip_with_close};
use serde_json::json;
use support::{Harness, INSTRUMENT, SERIAL, run_to_running};

#[tokio::test]
async fn filled_limit_if_touched_recovers_without_an_open_history_row() {
    let _serial = SERIAL.lock().await;
    let harness = Harness::new(false).await;
    let order = OrderAny::LimitIfTouched(LimitIfTouchedOrder::new(
        TraderId::from("TESTER-001"),
        StrategyId::from("ENTROPY-001"),
        INSTRUMENT.into(),
        ClientOrderId::from("ENTROPY-CLOSE"),
        OrderSide::Sell,
        Quantity::from("2.00"),
        Price::from("109.50"),
        Price::from("110.00"),
        TriggerType::Default,
        TimeInForce::Gtc,
        None,
        false,
        true,
        false,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        UUID4::new(),
        nautilus_core::time::get_atomic_clock_realtime().get_time_ns(),
    ));
    let (mut retained, _, fills, ids) = live_round_trip_with_close(&harness, order).await;
    retained.orders.clear();
    let mut close = history(&fills[1]);
    close["order"]["orderType"] = json!("Take Profit Limit");
    close["order"]["limitPx"] = json!("109.50");
    close["order"]["triggerPx"] = json!("110.00");
    close["order"]["isTrigger"] = json!(false);
    *harness.source.history.lock() = vec![close, history(&fills[0])];
    *harness.source.fills.lock() = vec![fills[1].clone(), fills[0].clone()];
    let check = |node: &nautilus_live::node::LiveNode| {
        assert_eq!(economics(node), ids);
        let cache = node.kernel().cache();
        let cache = cache.borrow();
        let close = cache.order(&ClientOrderId::from("ENTROPY-CLOSE")).unwrap();
        assert_eq!(close.order_type(), OrderType::LimitIfTouched);
        assert_eq!(close.price(), Some(Price::from("109.50")));
        assert_eq!(close.trigger_price(), Some(Price::from("110.00")));
    };
    let mut recovered = harness.node().await;
    retained.restore(&recovered);
    run_to_running(&mut recovered).await;
    check(&recovered);
    let healed = Retained::capture(&recovered);
    recovered.dispose();
    drop(recovered);
    let mut restarted = harness.node().await;
    healed.restore(&restarted);
    run_to_running(&mut restarted).await;
    check(&restarted);
    assert_eq!(harness.source.actions.lock().len(), 2);
    restarted.dispose();
}
