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

**With ``SODEX_DRY_RUN=false`` this submits real orders.** On testnet that is play money; the
same program against ``Network.MAINNET`` would spend real funds. The default is a dry run, which
connects and subscribes without submitting anything.

Credentials come from the environment - ``SODEX_ACCOUNT_ID``, ``SODEX_API_KEY_NAME``,
``SODEX_API_PRIVATE_KEY`` - rather than being written here. The key must be a registered API
key, never the master wallet: the master key can authorize withdrawals and belongs offline.

Reconciliation is **on**. The adapter reads the account's balances, open orders and order
history, and Nautilus infers fills from those reports - so positions, average prices and fees do
get reconciled, including orders this client did not place.

Either engine, selected by ``MARKET``. The two are separate venues on this adapter, so the
instrument id, the client id and the order size all move with that one constant. Perps differs in
two ways that matter here: orders carry a one-way position side, and ``reduce_only`` is honored
(spot refuses it), so ``close_positions_on_stop`` flattens rather than placing an opposing trade.

One limit remains, and it is granularity rather than capability: the venue's per-fill endpoint
answers ``[]`` on an account that has never traded, so its wire shape is unobserved and this
adapter does not parse it. Fills therefore arrive at reconciliation cadence rather than per
trade, each carrying a synthetic trade id instead of the venue's own.

"""

from __future__ import annotations

import os

from nautilus_trader.adapters.sodex import SODEX_PERPS
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


def _dry_run_from_env() -> bool:
    """
    Read ``SODEX_DRY_RUN``, defaulting to a dry run.

    Scoped to one invocation rather than written into this file: ``env SODEX_DRY_RUN=false``
    arms exactly the run it prefixes, where a constant edited to arm it stays armed until
    somebody remembers to put it back - and the next run is usually started by someone who
    did not do the editing.

    An unrecognized value is refused rather than read as either answer. A typo that silently
    means "no orders" only wastes a run; one that silently means "orders" spends money.

    """
    raw = os.environ.get("SODEX_DRY_RUN", "true").strip().lower()

    if raw in ("true", "1", "yes", "on"):
        return True
    if raw in ("false", "0", "no", "off"):
        return False

    raise SystemExit(f"SODEX_DRY_RUN must be a boolean, not {raw!r}")


# WARNING: `SODEX_DRY_RUN=false` submits orders to the configured network.
DRY_RUN = _dry_run_from_env()
NETWORK = Network.TESTNET

# Which engine to reach. `SODEX_MARKET=perps` switches everything venue-specific below, because
# spot and perps are two separate venues here and the parts have to move together: a perps market
# paired with a spot instrument id is rejected rather than routed to the wrong engine, which is the
# behavior to want but an annoying way to find out you edited only half the configuration.
#
# This and `DRY_RUN` both come from the environment, so one command describes a whole run and
# nothing about the last one is left behind in the file.
MARKET = Market.PERPS if os.environ.get("SODEX_MARKET", "spot").lower() == "perps" else Market.SPOT

if MARKET == Market.SPOT:
    VENUE_NAME = SODEX_SPOT
    INSTRUMENT_ID = InstrumentId.from_str(f"vBTC_vUSDC.{SODEX_SPOT}")
    ORDER_QTY = "0.001"
else:
    VENUE_NAME = SODEX_PERPS
    # `BTC-USD` takes a 0.00001 step but also enforces a 10 vUSDC minimum notional, so near
    # 77,000 the smallest accepted size is 0.00013. This clears it with room to spare, and at
    # the default 20x leverage it posts well under a dollar of margin.
    INSTRUMENT_ID = InstrumentId.from_str(f"BTC-USD.{SODEX_PERPS}")
    ORDER_QTY = "0.0002"

TRADER_ID = TraderId.from_str("TESTER-001")
STRATEGY_ID = StrategyId.from_str("EXEC_TESTER-001")


def main() -> None:
    """
    Run the example.
    """
    node = (
        LiveNode.builder("SODEX-EXEC-TESTER-001", TRADER_ID, Environment.LIVE)
        # On: the account reads give the engine order status, and it infers fills from them.
        .with_reconciliation(reconciliation=True)
        # Bypassed deliberately: this program exists to observe what the adapter does with an
        # order, and a pre-trade rejection would answer a different question. The strategy runs
        # (`paper_trading.py`, and anything headed for production) leave it on.
        .with_risk_engine_config(LiveRiskEngineConfig(bypass=True))
        .add_data_client(
            VENUE_NAME,
            SodexDataClientFactory(),
            SodexDataClientConfig(network=NETWORK, market=MARKET),
        )
        .add_exec_client(
            VENUE_NAME,
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
            client_id=ClientId.from_str(VENUE_NAME),
            order_qty=Quantity.from_str(ORDER_QTY),
            subscribe_quotes=True,
            subscribe_trades=True,
            enable_limit_buys=True,
            enable_limit_sells=True,
            # The venue expresses post-only as its GTX time-in-force, which the adapter maps
            # by name rather than by value - the two numbering schemes disagree on IOC and FOK.
            use_post_only=True,
            cancel_orders_on_stop=True,
            # Works now that fills are accounted for through reconciliation, though a position
            # opened moments before the stop may not have been reconciled yet.
            close_positions_on_stop=True,
            dry_run=DRY_RUN,
            log_data=False,
        ),
    )

    node.run()


if __name__ == "__main__":
    main()
