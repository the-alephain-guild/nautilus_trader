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
Test explicit, read-only SoDEX execution account identity.
"""

import pytest

from nautilus_trader.adapters.sodex import Market
from nautilus_trader.adapters.sodex import Network
from nautilus_trader.adapters.sodex import SodexExecClientConfig


@pytest.mark.parametrize("network", [Network.MAINNET, Network.TESTNET])
@pytest.mark.parametrize("market", [Market.SPOT, Market.PERPS])
@pytest.mark.parametrize(
    ("account_id", "wallet_address"),
    [
        (None, None),
        (0, "0x" + "aB" * 20),
        (2**64 - 1, "0x" + "12" * 20),
    ],
)
def test_explicit_account_identity_is_read_without_resolution(
    network: Network,
    market: Market,
    account_id: int | None,
    wallet_address: str | None,
) -> None:
    """
    Test identity getters retain explicit fields without environment resolution.
    """
    config = SodexExecClientConfig(
        network=network,
        market=market,
        account_id=account_id,
        wallet_address=wallet_address,
    )

    assert config.account_id == account_id
    assert config.wallet_address == wallet_address
    assert config.network == network
    assert config.market == market
    assert config.account_id == account_id


@pytest.mark.parametrize(
    ("field", "replacement"),
    [("account_id", 99), ("wallet_address", "0x" + "34" * 20)],
)
def test_account_identity_properties_are_read_only(field: str, replacement: object) -> None:
    """
    Test identity cannot be changed after native config construction.
    """
    wallet = "0x" + "12" * 20
    config = SodexExecClientConfig(account_id=42, wallet_address=wallet)

    assert config.account_id == 42
    assert config.wallet_address == wallet
    with pytest.raises(AttributeError):
        setattr(config, field, replacement)
    assert config.account_id == 42
    assert config.wallet_address == wallet


@pytest.mark.parametrize("field", ["api_private_key", "api_key_name"])
def test_account_identity_does_not_expose_credentials(field: str) -> None:
    """
    Test public account readback does not add credential getters.
    """
    config = SodexExecClientConfig(account_id=42, wallet_address="0x" + "12" * 20)

    assert not hasattr(config, field)
