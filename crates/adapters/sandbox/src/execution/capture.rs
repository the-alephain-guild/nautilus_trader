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

use std::{
    cell::Cell,
    collections::BTreeSet,
    fmt,
    rc::{Rc, Weak},
};

use aws_lc_rs::digest::{SHA256, digest};
use nautilus_common::clients::{
    ExecutionAccountCaptureBalance, ExecutionAccountCaptureError, ExecutionAccountCaptureOrder,
    ExecutionAccountCapturePosition, ExecutionAccountCaptureProbe, ExecutionAccountCaptureSource,
    ExecutionAccountSnapshot,
};
use nautilus_model::{
    accounts::{Account, AccountAny},
    enums::OrderStatus,
    orders::Order,
    types::Currency,
};

use super::{SandboxExecutionClient, SandboxInner};

pub(super) struct SandboxAccountCaptureProbe {
    source: ExecutionAccountCaptureSource,
    owner: Weak<std::cell::RefCell<SandboxInner>>,
    retired: Weak<Cell<bool>>,
    connected: Weak<Cell<bool>>,
}

impl SandboxAccountCaptureProbe {
    pub(super) fn new(client: &SandboxExecutionClient) -> anyhow::Result<Self> {
        let core = client.core.try_borrow()?;
        let currency_definitions = client
            .config
            .starting_balances
            .iter()
            .map(|money| currency_definition(money.currency))
            .collect::<Vec<_>>();
        let configuration_json = serde_json::to_string(&serde_json::json!({
            "client_config":&client.config, "starting_currency_definitions":currency_definitions,
            "base_currency_definition":client.config.base_currency.map(currency_definition),
        }))?;
        let configuration_sha256 = digest(&SHA256, configuration_json.as_bytes())
            .as_ref()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        Ok(Self {
            source: ExecutionAccountCaptureSource {
                incarnation: client.capture_incarnation,
                client_id: core.client_id,
                account_id: core.account_id,
                venue: core.venue,
                account_type: client.config.account_type.to_string(),
                api_revision: "execution-account-capture-v1".to_owned(),
                model_revision: "sandbox-account-state-v1".to_owned(),
                runtime_version: env!("CARGO_PKG_VERSION").to_owned(),
                configuration_json,
                configuration_sha256,
            },
            owner: Rc::downgrade(&client.inner),
            retired: Rc::downgrade(&client.capture_retired),
            connected: Rc::downgrade(&client.capture_connected),
        })
    }
}

impl fmt::Debug for SandboxAccountCaptureProbe {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SandboxAccountCaptureProbe")
            .field("source", &self.source)
            .finish()
    }
}

impl ExecutionAccountCaptureProbe for SandboxAccountCaptureProbe {
    fn source(&self) -> &ExecutionAccountCaptureSource {
        &self.source
    }

    fn capture(&self) -> Result<ExecutionAccountSnapshot, ExecutionAccountCaptureError> {
        use ExecutionAccountCaptureError::{
            AccountUnavailable, Borrowed, InconsistentState, Retired, UnsupportedConfiguration,
        };

        if self.retired.upgrade().is_none_or(|retired| retired.get()) {
            return Err(Retired);
        }
        if self
            .connected
            .upgrade()
            .is_none_or(|connected| !connected.get())
        {
            return Err(AccountUnavailable);
        }
        let owner = self.owner.upgrade().ok_or(Retired)?;
        let inner = owner.try_borrow().map_err(|_| Borrowed)?;
        if inner.config.frozen_account {
            return Err(UnsupportedConfiguration);
        }
        let mut initial_codes = BTreeSet::new();
        if inner
            .config
            .starting_balances
            .iter()
            .any(|money| !initial_codes.insert(money.currency.code))
        {
            return Err(UnsupportedConfiguration);
        }
        let cache = inner.cache.try_borrow().map_err(|_| Borrowed)?;
        let state = cache.capture_account_state(&self.source.account_id)?;
        if state.account.account_type().to_string() != self.source.account_type {
            return Err(InconsistentState);
        }
        match (state.account.base_currency(), inner.config.base_currency) {
            (Some(current), Some(configured)) if same_denomination(current, configured) => {}
            (None, None) => {}
            _ => return Err(InconsistentState),
        }
        let account_balances = state.account.balances();
        if account_balances.iter().any(|(currency, balance)| {
            !same_denomination(*currency, balance.currency)
                || !same_denomination(*currency, balance.total.currency)
                || !same_denomination(*currency, balance.locked.currency)
                || !same_denomination(*currency, balance.free.currency)
        }) {
            return Err(InconsistentState);
        }
        if inner.config.starting_balances.iter().any(|initial| {
            account_balances
                .keys()
                .find(|currency| currency.code == initial.currency.code)
                .is_none_or(|currency| !same_denomination(*currency, initial.currency))
        }) {
            return Err(InconsistentState);
        }
        let mut balances = account_balances
            .into_values()
            .map(|balance| {
                let currency = balance.total.currency;
                ExecutionAccountCaptureBalance {
                    asset_code: currency.code.to_string(),
                    asset_precision: currency.precision,
                    asset_type: currency.currency_type.to_string(),
                    asset_iso4217: currency.iso4217,
                    total: exact_fixed(&balance.total.raw().to_string()),
                    locked: exact_fixed(&balance.locked.raw().to_string()),
                    free: exact_fixed(&balance.free.raw().to_string()),
                }
            })
            .collect::<Vec<_>>();
        balances.sort_by(|left, right| left.asset_code.cmp(&right.asset_code));
        if initial_codes.iter().any(|code| {
            !balances
                .iter()
                .any(|balance| balance.asset_code == code.as_str())
        }) {
            return Err(InconsistentState);
        }
        let mut resting = BTreeSet::new();
        for engine in inner.matching_engines.values() {
            if engine.account_capture_has_pending_work(&self.source.account_id)? {
                return Err(ExecutionAccountCaptureError::PendingNativeState);
            }
            for order in engine.get_open_orders() {
                if !resting.insert(order.client_order_id) {
                    return Err(InconsistentState);
                }
            }
        }
        if resting.iter().any(|id| {
            !state
                .orders
                .iter()
                .any(|order| order.client_order_id() == *id)
        }) {
            return Err(InconsistentState);
        }
        let orders = state
            .orders
            .iter()
            .map(|order| {
                let in_matching_engine = resting.contains(&order.client_order_id());
                ExecutionAccountCaptureOrder {
                    client_order_id: order.client_order_id().to_string(),
                    venue_order_id: order.venue_order_id().map(|id| id.to_string()),
                    instrument_id: order.instrument_id().to_string(),
                    order_type: order.order_type().to_string(),
                    side: order.order_side().to_string(),
                    status: order.status().to_string(),
                    quantity: exact_fixed(&order.quantity().raw().to_string()),
                    filled_quantity: exact_fixed(&order.filled_qty().raw().to_string()),
                    price: order
                        .price()
                        .map(|price| exact_fixed(&price.raw().to_string())),
                    trigger_price: order
                        .trigger_price()
                        .map(|price| exact_fixed(&price.raw().to_string())),
                    in_matching_engine,
                    pending: !in_matching_engine
                        || matches!(
                            order.status(),
                            OrderStatus::PendingUpdate | OrderStatus::PendingCancel
                        )
                        || order.is_inflight(),
                }
            })
            .collect();
        let positions = state
            .positions
            .iter()
            .map(|position| ExecutionAccountCapturePosition {
                position_id: position.id.to_string(),
                instrument_id: position.instrument_id.to_string(),
                side: position.side.to_string(),
                quantity: exact_fixed(&position.quantity.raw().to_string()),
                settlement_currency: position.settlement_currency.code.to_string(),
            })
            .collect();
        let no_borrowing_model =
            matches!(&state.account, AccountAny::Cash(account) if !account.allow_borrowing);
        let observed_at = inner
            .clock
            .try_borrow()
            .map_err(|_| Borrowed)?
            .timestamp_ns();
        Ok(ExecutionAccountSnapshot {
            source: self.source.clone(),
            observed_at,
            balances,
            positions,
            orders,
            no_borrowing_model,
        })
    }
}

fn same_denomination(left: Currency, right: Currency) -> bool {
    // Currency's global equality intentionally compares only code, capture must
    // also bind the native precision and asset classification to this amount.
    left.code == right.code
        && left.precision == right.precision
        && left.currency_type == right.currency_type
        && left.iso4217 == right.iso4217
}

fn currency_definition(currency: Currency) -> serde_json::Value {
    serde_json::json!({"code":currency.code.as_str(), "precision":currency.precision,
    "currency_type":currency.currency_type.to_string(), "iso4217":currency.iso4217})
}

fn exact_fixed(raw: &str) -> String {
    let (sign, digits) = raw
        .strip_prefix('-')
        .map_or(("", raw), |digits| ("-", digits));
    let width = usize::from(nautilus_model::types::fixed::FIXED_PRECISION);
    let padded = format!("{:0>width$}", digits, width = width + 1);
    let point = padded.len() - width;
    let mut value = format!("{}{}.{}", sign, &padded[..point], &padded[point..]);
    while value.ends_with('0') {
        value.pop();
    }
    if value.ends_with('.') {
        value.pop();
    }
    if value == "-0" { "0".to_owned() } else { value }
}
