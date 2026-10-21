#!/usr/bin/env python3
# -------------------------------------------------------------------------------------------------
#  Copyright (C) 2015-2026 Nautech Systems Pty Ltd. All rights reserved.
#  https://nautechsystems.io
#
#  Licensed under the GNU Lesser General Public License Version 3.0 (the "License");
#  You may not use this file except in compliance with the License.
#  You may obtain a copy of the License at https://www.gnu.org/licenses/lgpl-3.0.en.html
#
#  Unless required by applicable law or agreed to in writing, software
#  distributed under the License is distributed on an "AS IS" BASIS,
#  WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
#  See the License for the specific language governing permissions and
#  limitations under the License.
# -------------------------------------------------------------------------------------------------
"""
Test Entropy io: HIP-3 routing with the native Hyperliquid execution client.

This example connects to Hyperliquid mainnet with DRY_RUN enabled. It submits no
orders and sends no shutdown cancel or close commands in that mode. Changing DRY_RUN
to False enables real Entropy orders using the configured Hyperliquid account.
Market availability, minimum notional, margin mode, and maximum leverage come from
the current venue metadata; this example does not fund or configure the account.

"""

from __future__ import annotations

from decimal import Decimal

from nautilus_trader.adapters.hyperliquid import (
    HyperliquidDataClientConfig,
    HyperliquidDataClientFactory,
    HyperliquidEnvironment,
    HyperliquidExecutionClientConfig,
    HyperliquidExecutionClientFactory,
)
from nautilus_trader.common import Environment
from nautilus_trader.live import LiveNode
from nautilus_trader.model import (
    AccountId,
    ClientId,
    InstrumentId,
    Quantity,
    StrategyId,
    TimeInForce,
    TraderId,
)
from nautilus_trader.testkit import ExecTesterConfig

# Enabling execution also enables shutdown cancel/close commands for this test account
DRY_RUN = True
HYPERLIQUID = "HYPERLIQUID"
TRADER_ID = TraderId.from_str("TESTER-001")
ACCOUNT_ID = AccountId.from_str("HYPERLIQUID-001")
STRATEGY_ID = StrategyId.from_str("EXEC_TESTER-001")
INSTRUMENT_ID = InstrumentId.from_str(f"io:SNDK-USD-PERP.{HYPERLIQUID}")
ORDER_QTY = "1"
TOB_OFFSET_TICKS = 500


def main() -> None:
    """
    Run the example.
    """
    node = (
        LiveNode.builder("ENTROPY-EXEC-TESTER-001", TRADER_ID, Environment.LIVE)
        .with_reconciliation(reconciliation=True)
        .add_data_client(
            None,
            HyperliquidDataClientFactory(),
            HyperliquidDataClientConfig(environment=HyperliquidEnvironment.MAINNET),
        )
        .add_exec_client(
            None,
            HyperliquidExecutionClientFactory(),
            HyperliquidExecutionClientConfig(
                account_id=ACCOUNT_ID,
                environment=HyperliquidEnvironment.MAINNET,
            ),
        )
        .build()
    )
    node.add_builtin_strategy(
        "ExecTester",
        ExecTesterConfig(
            strategy_id=STRATEGY_ID,
            instrument_id=INSTRUMENT_ID,
            client_id=ClientId.from_str(HYPERLIQUID),
            external_order_instrument_ids=[INSTRUMENT_ID],
            order_qty=Quantity.from_str(ORDER_QTY),
            subscribe_quotes=True,
            subscribe_trades=True,
            open_position_on_start_qty=Decimal(ORDER_QTY),
            open_position_on_first_quote=True,
            open_position_time_in_force=TimeInForce.IOC,
            enable_limit_buys=True,
            enable_limit_sells=True,
            tob_offset_ticks=TOB_OFFSET_TICKS,
            use_post_only=True,
            cancel_orders_on_stop=not DRY_RUN,
            close_positions_on_stop=not DRY_RUN,
            reduce_only_on_stop=False,
            dry_run=DRY_RUN,
            log_data=False,
        ),
    )

    node.run()


if __name__ == "__main__":
    main()
