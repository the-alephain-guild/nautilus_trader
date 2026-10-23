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

use std::time::Duration;

use nautilus_core::time::get_atomic_clock_realtime;
use nautilus_model::identifiers::TraderId;
use nautilus_papertrade::factories::PapertradeExecutionClientFactory;

use super::support::Harness;

pub(crate) struct RetainedCache {
    pub(crate) orders: Vec<nautilus_model::orders::OrderAny>,
    pub(crate) positions: Vec<nautilus_model::position::Position>,
}

impl RetainedCache {
    pub(crate) fn capture(node: &nautilus_live::node::LiveNode) -> Self {
        let cache = node.kernel().cache();
        let cache = cache.borrow();
        Self {
            orders: cache
                .orders(None, None, None, None, None)
                .into_iter()
                .map(|o| o.clone())
                .collect(),
            positions: cache
                .positions(None, None, None, None, None)
                .into_iter()
                .map(|p| p.cloned())
                .collect(),
        }
    }
    pub(crate) fn restore(&self, node: &nautilus_live::node::LiveNode) {
        let cache = node.kernel().cache();
        let mut cache = cache.borrow_mut();
        for order in &self.orders {
            cache.add_order(order.clone(), None, None, false).unwrap();
        }
        for position in &self.positions {
            cache
                .add_position_without_order(position, nautilus_model::enums::OmsType::Hedging)
                .unwrap();
        }
        cache.clear_index();
        cache.build_index();
    }
}

pub(crate) fn account_node(harness: &Harness) -> nautilus_live::node::LiveNode {
    use nautilus_common::enums::Environment;
    use nautilus_papertrade::{reports, wire::TradingState};
    let node =
        nautilus_live::node::LiveNode::builder(TraderId::from("TESTER-001"), Environment::Live)
            .unwrap()
            .with_reconciliation(true)
            .with_timeout_connection(5)
            .with_timeout_reconciliation(5)
            .with_timeout_portfolio(3)
            .with_timeout_disconnection_secs(2)
            .with_delay_post_stop_secs(0)
            .with_delay_shutdown_secs(0)
            .add_exec_client(
                None,
                Box::new(PapertradeExecutionClientFactory),
                Box::new(harness.config.clone()),
            )
            .unwrap()
            .build()
            .unwrap();
    let markets: TradingState =
        serde_json::from_value(harness.source.trading.lock().clone()).unwrap();
    for market in &markets.instruments {
        node.kernel()
            .cache()
            .borrow_mut()
            .add_instrument(
                reports::instrument(market, get_atomic_clock_realtime().get_time_ns()).unwrap(),
            )
            .unwrap();
    }
    let now = get_atomic_clock_realtime().get_time_ns();
    node.kernel()
        .cache()
        .borrow_mut()
        .add_quote(nautilus_model::data::QuoteTick::new(
            nautilus_model::identifiers::InstrumentId::from("BTC-USD-PERP.PAPERTRADE"),
            nautilus_model::types::Price::from("83000.0"),
            nautilus_model::types::Price::from("83000.0"),
            nautilus_model::types::Quantity::from(0),
            nautilus_model::types::Quantity::from(0),
            now,
            now,
        ))
        .unwrap();
    node
}

pub(crate) async fn run_account_node(node: &mut nautilus_live::node::LiveNode) {
    let handle = node.handle();
    let observe = async {
        let started = tokio::time::timeout(Duration::from_secs(10), async {
            while !handle.is_running() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await;
        handle.stop();
        started
    };
    let (run, started) = tokio::join!(node.run(), observe);
    run.unwrap();
    started.expect("LiveNode did not finish startup reconciliation");
}
