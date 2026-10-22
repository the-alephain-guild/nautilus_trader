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

"""Stream Papertrade reference-mid quotes with no credentials or execution client."""

from nautilus_trader.adapters.papertrade import (
    PapertradeDataClientConfig,
    PapertradeDataClientFactory,
)
from nautilus_trader.common import Environment
from nautilus_trader.live import LiveNode
from nautilus_trader.model import ClientId, InstrumentId, TraderId
from nautilus_trader.testkit import DataTesterConfig


def main() -> None:
    """Run a public data-only node."""
    node = (
        LiveNode.builder("PAPERTRADE-DATA", TraderId("TESTER-001"), Environment.LIVE)
        .add_data_client(
            None, PapertradeDataClientFactory(), PapertradeDataClientConfig()
        )
        .build()
    )
    node.add_builtin_actor(
        "DataTester",
        DataTesterConfig(
            client_id=ClientId("PAPERTRADE"),
            instrument_ids=[InstrumentId.from_str("BTC-USD-PERP.PAPERTRADE")],
            subscribe_quotes=True,
            subscribe_trades=False,
            manage_book=False,
            log_data=True,
        ),
    )
    node.run()


if __name__ == "__main__":
    main()
