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
Stream SoDEX market data with the built-in DataTester actor.

Connects to the SoDEX testnet and subscribes for the configured instrument, logging everything
received. Needs no credentials — the venue serves market data unsigned — and places no orders.

Two details are specific to this venue and worth noticing in the output:

- Bars appear one interval late. The venue never sets the `closed` flag on its candle pushes,
  so the adapter releases a bar when its successor starts instead of waiting for a flag that
  never comes. Waiting for it would mean receiving nothing at all, with a healthy-looking
  connection.
- Quotes are periodic samples of the top of book, not every change. The venue publishes no
  dedicated quote channel; they are derived from its ticker, which pushes on its own cadence.

There is no order book channel at this venue, so `subscribe_book_deltas` is deliberately absent
rather than set to False-by-omission: it would never produce data.

"""

from __future__ import annotations

from nautilus_trader.adapters.sodex import SODEX_SPOT
from nautilus_trader.adapters.sodex import SodexDataClientConfig
from nautilus_trader.adapters.sodex import SodexDataClientFactory
from nautilus_trader.adapters.sodex import SodexMarket
from nautilus_trader.adapters.sodex import SodexNetwork
from nautilus_trader.common import Environment
from nautilus_trader.live import LiveNode
from nautilus_trader.model import BarType
from nautilus_trader.model import ClientId
from nautilus_trader.model import InstrumentId
from nautilus_trader.model import TraderId
from nautilus_trader.testkit import DataTesterConfig


TRADER_ID = TraderId.from_str("TESTER-001")

# The two engines do not share symbol names: spot lists `vBTC_vUSDC`, perps lists `BTC-USD`.
# An instrument id must carry the venue of the engine its client is bound to.
INSTRUMENT_ID = InstrumentId.from_str(f"vBTC_vUSDC.{SODEX_SPOT}")
BAR_TYPE = BarType.from_str(f"{INSTRUMENT_ID}-1-MINUTE-LAST-EXTERNAL")


def main() -> None:
    """
    Run the example.
    """
    node = (
        LiveNode.builder("SODEX-DATA-TESTER-001", TRADER_ID, Environment.LIVE)
        .add_data_client(
            # Named for the engine rather than left to default. One factory serves both
            # engines, so two clients would otherwise share the id `SODEX` and collide.
            SODEX_SPOT,
            SodexDataClientFactory(),
            SodexDataClientConfig(
                network=SodexNetwork.TESTNET,
                market=SodexMarket.SPOT,
            ),
        )
        .build()
    )
    node.add_builtin_actor(
        "DataTester",
        DataTesterConfig(
            client_id=ClientId.from_str(SODEX_SPOT),
            instrument_ids=[INSTRUMENT_ID],
            bar_types=[BAR_TYPE],
            subscribe_quotes=True,
            subscribe_trades=True,
            request_instruments=True,
            request_bars=True,
            log_data=True,
        ),
    )

    node.run()


if __name__ == "__main__":
    main()
