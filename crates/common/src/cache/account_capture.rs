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

use nautilus_model::{
    accounts::AccountAny,
    identifiers::AccountId,
    orders::{Order, OrderAny},
    position::Position,
};

use super::Cache;
use crate::clients::ExecutionAccountCaptureError;

/// An account-scoped cache copy for a concrete execution owner to cross-check.
#[derive(Debug)]
pub struct AccountCaptureState {
    pub account: AccountAny,
    pub orders: Vec<OrderAny>,
    pub positions: Vec<Position>,
}

impl Cache {
    /// Copies current account state without panicking on held entity borrows.
    ///
    /// All unassigned nonterminal orders remain potential account work until
    /// execution routing assigns their account, including default client routes.
    ///
    /// # Errors
    ///
    /// Returns an error when the actual account is absent or any entity needed
    /// to enumerate the complete account is mutably borrowed.
    pub fn capture_account_state(
        &self,
        account_id: &AccountId,
    ) -> Result<AccountCaptureState, ExecutionAccountCaptureError> {
        let account = self
            .accounts
            .get(account_id)
            .ok_or(ExecutionAccountCaptureError::AccountUnavailable)?
            .try_borrow()
            .map_err(|_| ExecutionAccountCaptureError::Borrowed)?
            .clone();
        let mut orders = Vec::new();
        for cell in self.orders.values() {
            let order = cell
                .try_borrow()
                .map_err(|_| ExecutionAccountCaptureError::Borrowed)?;
            if !order.is_closed()
                && (order.account_id() == Some(*account_id) || order.account_id().is_none())
            {
                orders.push(order.clone());
            }
        }
        let mut positions = Vec::new();
        for cell in self.positions.values() {
            let position = cell
                .try_borrow()
                .map_err(|_| ExecutionAccountCaptureError::Borrowed)?;
            if position.account_id == *account_id && !position.is_closed() {
                positions.push(position.clone());
            }
        }
        orders.sort_by_key(Order::client_order_id);
        positions.sort_by_key(|position| position.id);
        Ok(AccountCaptureState {
            account,
            orders,
            positions,
        })
    }
}
