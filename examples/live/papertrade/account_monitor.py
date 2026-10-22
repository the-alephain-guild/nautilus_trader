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

"""Connect the native Papertrade clients without installing a trading strategy."""

import argparse
from pathlib import Path

from nautilus_trader.adapters.papertrade import (
    PapertradeDataClientConfig,
    PapertradeDataClientFactory,
    PapertradeExecutionClientConfig,
    PapertradeExecutionClientFactory,
)
from nautilus_trader.common import Environment
from nautilus_trader.live import LiveNode
from nautilus_trader.model import AccountId, TraderId


def main() -> None:
    """Read account state using an already registered session key from the environment."""
    parser = argparse.ArgumentParser()
    parser.add_argument("--wallet", required=True)
    parser.add_argument("--build-id", required=True)
    parser.add_argument("--journal", required=True, type=Path)
    args = parser.parse_args()
    args.journal.parent.mkdir(parents=True, exist_ok=True)
    node = (
        LiveNode.builder("PAPERTRADE-ACCOUNT", TraderId("TESTER-001"), Environment.LIVE)
        .with_reconciliation(True)
        .add_data_client(
            None, PapertradeDataClientFactory(), PapertradeDataClientConfig()
        )
        .add_exec_client(
            None,
            PapertradeExecutionClientFactory(),
            PapertradeExecutionClientConfig(
                AccountId("PAPERTRADE-001"),
                args.wallet,
                str(args.journal),
                args.build_id,
            ),
        )
        .build()
    )
    node.run()


if __name__ == "__main__":
    main()
