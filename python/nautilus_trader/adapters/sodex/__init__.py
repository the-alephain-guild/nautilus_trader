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
Integration adapter for the SoDEX exchange.

SoDEX is an on-chain orderbook DEX on ValueChain. Orders are authenticated with an offline
EIP-712 signature over plain REST rather than by broadcasting transactions, so the integration
behaves more like a centralized venue than an AMM.

Spot and perps are modelled as two venues, ``SODEX_SPOT`` and ``SODEX_PERPS``, because they
differ in ways no parameter papers over: the signing domain, the batch endpoint, the action
names hashed into signatures, the API key set, the order item shape, the balances, and the
reference price used for limit bounds. A client binds to one engine through its configuration's
``market``, and instrument ids must carry the matching venue.

Market data needs no credentials; the venue serves it unsigned. Execution credentials resolve
from the configuration or, preferably, from ``SODEX_ACCOUNT_ID``, ``SODEX_API_KEY_NAME`` and
``SODEX_API_PRIVATE_KEY`` — keeping the key out of config files and out of any traceback that
prints its arguments. The key is a registered API key, never the master wallet, which can
authorize withdrawals and belongs offline.

Reconciliation works. The execution client reads the account's balances, open orders and order
history, and the engine infers fills from those reports — so positions, average prices and fees
reconcile, including for orders this client did not place.

Known limitation: granularity rather than capability. The venue's per-fill endpoint answers an
empty list on an account that has never traded, so its wire shape is unobserved and goes
unparsed. Fills therefore arrive at reconciliation cadence rather than per trade, each carrying a
synthetic trade id instead of the venue's own.
"""

from nautilus_trader._fixup import fixup_module_names
from nautilus_trader._libnautilus.sodex import *  # noqa: F403 (undefined-local-with-import-star)


__all__ = [
    "Market",
    "Network",
    "SODEX",
    "SODEX_PERPS",
    "SODEX_PERPS_VENUE",
    "SODEX_SPOT",
    "SODEX_SPOT_VENUE",
    "SodexDataClientConfig",
    "SodexDataClientFactory",
    "SodexExecClientConfig",
    "SodexExecutionClientFactory",
]

fixup_module_names(globals(), __name__)
del fixup_module_names
