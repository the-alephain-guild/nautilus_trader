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


"""Entropy market discovery using the native Hyperliquid HIP-3 adapter."""

from nautilus_trader.adapters.hyperliquid import HyperliquidHttpClient
from nautilus_trader.model import CryptoPerpetual


async def load_entropy_instruments(client: HyperliquidHttpClient) -> list[CryptoPerpetual]:
    """
    Load currently listed Entropy instruments without changing their Hyperliquid identity.

    The native client populates its asset mappings from venue metadata. Retain those
    mappings rather than calculating numeric HIP-3 asset IDs from a fixed dex index.
    This helper filters its return value, not the live node's entire instrument universe.
    """
    instruments = await client.load_instrument_definitions(
        include_spot=False,
        include_perps=False,
        include_perps_hip3=True,
        include_outcomes=False,
    )
    entropy = [
        instrument
        for instrument in instruments
        if isinstance(instrument, CryptoPerpetual)
        and str(instrument.id.venue) == "HYPERLIQUID"
        and str(instrument.id.symbol).startswith("io:")
    ]
    if not entropy:
        raise RuntimeError("No Entropy io: perpetuals found in Hyperliquid metadata")
    return entropy
