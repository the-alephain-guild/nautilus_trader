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
"""
Exercise SoDEX order submission with the built-in ExecTester strategy.

**With DRY_RUN = False this submits real orders.** On testnet that is play money; the same
program against ``Network.MAINNET`` would spend real funds. Start with DRY_RUN = True, which
connects and subscribes without submitting anything.

Credentials come from the environment — ``SODEX_ACCOUNT_ID``, ``SODEX_API_KEY_NAME``,
``SODEX_API_PRIVATE_KEY`` — rather than being written here. The key must be a registered API
key, never the master wallet: the master key can authorize withdrawals and belongs offline.

Two limits of this adapter shape what the tester can do, and both are worth knowing before
reading its output:

- **No fills are reported.** The venue's account stream exists but its subscription parameters
  are not yet known, so the client learns that an order was accepted and never that it filled.
  A position opened here will not appear in the engine's position state.
- **No reconciliation.** There is no order-status query, so orders this client did not place —
  or fills that happened while it was disconnected — stay invisible. Reconciliation is left off
  for that reason rather than enabled and silently doing nothing.

Because of the first point, ``close_positions_on_stop`` cannot work: the engine believes it
holds no position. Cancelling resting orders on stop does work, and is enabled.

"""

from __future__ import annotations

from decimal import Decimal

from nautilus_trader.adapters.sodex import SODEX_SPOT
from nautilus_trader.adapters.sodex import Market
from nautilus_trader.adapters.sodex import Network
from nautilus_trader.adapters.sodex import SodexDataClientConfig
from nautilus_trader.adapters.sodex import SodexDataClientFactory
from nautilus_trader.adapters.sodex import SodexExecClientConfig
from nautilus_trader.adapters.sodex import SodexExecutionClientFactory
from nautilus_trader.common import Environment
from nautilus_trader.config import LiveRiskEngineConfig
from nautilus_trader.live import LiveNode
from nautilus_trader.model import ClientId
from nautilus_trader.model import InstrumentId
from nautilus_trader.model import Quantity
from nautilus_trader.model import StrategyId
from nautilus_trader.model import TraderId
from nautilus_trader.testkit import ExecTesterConfig


# WARNING: With DRY_RUN = False this submits orders to the configured network.
DRY_RUN = True
NETWORK = Network.TESTNET
MARKET = Market.SPOT
TRADER_ID = TraderId.from_str("TESTER-001")
STRATEGY_ID = StrategyId.from_str("EXEC_TESTER-001")
INSTRUMENT_ID = InstrumentId.from_str(f"vBTC_vUSDC.{SODEX_SPOT}")
ORDER_QTY = "0.001"


def main() -> None:
    """
    Run the example.
    """
    node = (
        LiveNode.builder("SODEX-EXEC-TESTER-001", TRADER_ID, Environment.LIVE)
        # Left off deliberately: reconciliation needs an order-status query this adapter does
        # not have, so enabling it would look like a safety net while providing none.
        .with_reconciliation(reconciliation=False)
        .with_risk_engine_config(LiveRiskEngineConfig(bypass=True))
        .add_data_client(
            SODEX_SPOT,
            SodexDataClientFactory(),
            SodexDataClientConfig(network=NETWORK, market=MARKET),
        )
        .add_exec_client(
            SODEX_SPOT,
            SodexExecutionClientFactory(),
            # Every credential resolves from the environment when left unset.
            SodexExecClientConfig(network=NETWORK, market=MARKET),
        )
        .build()
    )
    node.add_builtin_strategy(
        "ExecTester",
        ExecTesterConfig(
            strategy_id=STRATEGY_ID,
            instrument_id=INSTRUMENT_ID,
            client_id=ClientId.from_str(SODEX_SPOT),
            order_qty=Quantity.from_str(ORDER_QTY),
            subscribe_quotes=True,
            subscribe_trades=True,
            enable_limit_buys=True,
            enable_limit_sells=True,
            # The venue expresses post-only as its GTX time-in-force, which the adapter maps
            # by name rather than by value — the two numbering schemes disagree on IOC and FOK.
            use_post_only=True,
            cancel_orders_on_stop=True,
            # Cannot work without fill reports: the engine believes it holds no position.
            close_positions_on_stop=False,
            dry_run=DRY_RUN,
            log_data=False,
        ),
    )

    node.run()


if __name__ == "__main__":
    main()
