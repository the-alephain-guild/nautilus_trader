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


"""Test Entropy discovery, native venue routing, and safe example defaults."""

from unittest.mock import AsyncMock

import pytest
from unit.adapters.example_modules import capture_data_tester_main
from unit.adapters.example_modules import capture_exec_tester_main
from unit.adapters.example_modules import load_example_module

from nautilus_trader.adapters.hyperliquid import HyperliquidDataClientConfig
from nautilus_trader.adapters.hyperliquid import HyperliquidDataClientFactory
from nautilus_trader.adapters.hyperliquid import HyperliquidEnvironment
from nautilus_trader.adapters.hyperliquid import HyperliquidExecutionClientConfig
from nautilus_trader.adapters.hyperliquid import HyperliquidExecutionClientFactory
from nautilus_trader.adapters.hyperliquid import HyperliquidHttpClient
from nautilus_trader.adapters.hyperliquid.entropy import load_entropy_instruments
from nautilus_trader.common import Environment
from nautilus_trader.live import LiveNode
from nautilus_trader.model import AccountId
from nautilus_trader.model import CryptoPerpetual
from nautilus_trader.model import Currency
from nautilus_trader.model import InstrumentId
from nautilus_trader.model import Price
from nautilus_trader.model import Quantity
from nautilus_trader.model import Symbol
from nautilus_trader.model import TraderId


def instrument(coin: str, venue: str = "HYPERLIQUID") -> CryptoPerpetual:
    """Create an instrument with native identity and settlement types."""
    return CryptoPerpetual(
        instrument_id=InstrumentId.from_str(f"{coin}-USD-PERP.{venue}"),
        raw_symbol=Symbol(coin),
        base_currency=Currency.from_str(coin.rsplit(":", maxsplit=1)[-1]),
        quote_currency=Currency.from_str("USD"),
        settlement_currency=Currency.from_str("USDC"),
        is_inverse=False,
        price_precision=2,
        size_precision=4,
        price_increment=Price.from_str("0.01"),
        size_increment=Quantity.from_str("0.0001"),
        ts_event=0,
        ts_init=0,
    )


@pytest.mark.asyncio
async def test_discovery_retains_native_io_identity_and_excludes_other_markets() -> None:
    """Preserve native instrument objects while selecting Entropy markets."""
    sndk = instrument("io:SNDK")
    other = [instrument("xyz:SNDK"), instrument("BTC"), instrument("io:SNDK", "OTHER")]
    client = AsyncMock(spec=HyperliquidHttpClient)
    client.load_instrument_definitions = AsyncMock(return_value=[*other, sndk])
    result = await load_entropy_instruments(client)
    assert result == [sndk]
    assert result[0] is sndk
    assert str(result[0].id) == "io:SNDK-USD-PERP.HYPERLIQUID"
    client.load_instrument_definitions.assert_awaited_once_with(
        include_spot=False,
        include_perps=False,
        include_perps_hip3=True,
        include_outcomes=False,
    )


@pytest.mark.asyncio
async def test_missing_entropy_markets_fail_without_fallback() -> None:
    """Do not substitute another deployer's market when Entropy is absent."""
    client = AsyncMock(spec=HyperliquidHttpClient)
    client.load_instrument_definitions = AsyncMock(return_value=[instrument("xyz:SNDK")])
    with pytest.raises(RuntimeError, match="No Entropy"):
        await load_entropy_instruments(client)


@pytest.mark.asyncio
async def test_discovery_propagates_source_failure() -> None:
    """Do not treat a failed metadata request as an empty market universe."""
    client = AsyncMock(spec=HyperliquidHttpClient)
    client.load_instrument_definitions = AsyncMock(side_effect=ValueError("metadata unavailable"))
    with pytest.raises(ValueError, match="metadata unavailable"):
        await load_entropy_instruments(client)


def test_entropy_examples_preserve_hyperliquid_routing(monkeypatch: pytest.MonkeyPatch) -> None:
    """Keep order routing native and all execution effects disabled by default."""
    data = load_example_module("entropy", "data_tester")
    captured = capture_data_tester_main(monkeypatch, data)
    kwargs = captured["data_tester_kwargs"]
    assert kwargs["instrument_ids"] == [
        InstrumentId.from_str("io:SNDK-USD-PERP.HYPERLIQUID"),
    ]
    assert str(kwargs["client_id"]) == "HYPERLIQUID"
    assert kwargs["subscribe_funding_rates"] is True
    assert captured["run_called"] is True

    execution = load_example_module("entropy", "exec_tester")
    captured = capture_exec_tester_main(monkeypatch, execution)
    kwargs = captured["exec_tester_kwargs"]
    assert kwargs["instrument_id"] == InstrumentId.from_str("io:SNDK-USD-PERP.HYPERLIQUID")
    assert str(kwargs["client_id"]) == "HYPERLIQUID"
    assert kwargs["dry_run"] is True
    assert kwargs["cancel_orders_on_stop"] is False
    assert kwargs["close_positions_on_stop"] is False
    assert "risk_engine_config" not in captured
    assert captured["reconciliation"] is True


def test_native_builder_accepts_entropy_market_routing() -> None:
    """Build native clients without connecting or submitting orders."""
    node = (
        LiveNode.builder("ENTROPY-PYTEST", TraderId("TESTER-001"), Environment.LIVE)
        .add_data_client(None, HyperliquidDataClientFactory(), HyperliquidDataClientConfig())
        .add_exec_client(
            None,
            HyperliquidExecutionClientFactory(),
            HyperliquidExecutionClientConfig(
                AccountId("HYPERLIQUID-001"),
                private_key="0x" + "11" * 32,
                environment=HyperliquidEnvironment.MAINNET,
            ),
        )
        .build()
    )
    assert node.environment == Environment.LIVE
