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
Paper-trade the adaptive martingale on live SoDEX data with simulated execution.

Nothing here reaches the venue's trading surface. Market data is served unsigned, so the data
client needs no credentials; orders go to the sandbox adapter, which runs the engine's own
`OrderMatchingEngine` against the arriving data. No API key is read, nothing is signed, and no
funds - real or testnet - are at risk.

Mainnet rather than testnet, and that choice is forced. Every one of the venue's 33 tradable
testnet spot symbols is frozen: sampling ten minutes of 1-minute bars gives a single distinct
close on 22 of them and no bars at all on the other 11. A strategy whose entry depends on a
pullback and whose volatility gate divides by ATR cannot evaluate anything there - ATR collapses
to zero and no condition can ever become true. Mainnet quotes move, so the decision path is
exercised rather than merely executed.

`vETH_vUSDC` because it is the only mainnet pair here with both movement and depth. Two hours of
1-minute bars, measured when this was written:

| pair            | mean abs 1-min move | mean bar range | 2h span | 2h volume       |
|-----------------|---------------------|----------------|---------|-----------------|
| `vHYPE_vUSDC`   | 4.8 bp              | 5.8 bp         | 1.70%   | 2.84 HYPE       |
| `vSOL_vUSDC`    | 3.6 bp              | 4.2 bp         | 1.01%   | 0.287 SOL       |
| `vETH_vUSDC`    | 2.9 bp              | 3.6 bp         | 0.77%   | 12.37 ETH       |
| `vBTC_vUSDC`    | 2.4 bp              | 2.8 bp         | 0.59%   | 0.0052 BTC      |

HYPE moves most, and on depth it is unusable: an 80 USDC order is about 3.5 HYPE, which exceeds
that pair's entire two-hour volume. The same order is roughly 0.26% of ETH's, so ETH is the one
where a simulated fill resembles a reachable one. Switching pairs is a one-line change, and the
trade-off above is the thing to weigh when doing it.

Two limits worth holding in view when reading the results:

- **Fills are still optimistic.** The matching engine prices against bars and trades without
  modeling the book, so queue position and depth cost nothing here. Treat fills as proof the
  plumbing works, not as evidence the size is tradable.
- **The risk engine caps one order, not total exposure.** It is on here, but 2.0's risk engine
  carries a per-order notional cap, a free-balance check and a submit rate limit - there is no
  cumulative or per-day notional ceiling. The pyramid's total is bounded by the strategy's own
  ``max_total_notional``, which is strategy state rather than an engine guarantee. Anything that
  has to hold across restarts or across strategies needs a separate mechanism.
- **Simulated fees are the engine's, not the venue's.** The sandbox applies its own fee model. The
  live execution client charges the venue's own maker/taker rates taken from the instrument, and
  the venue deducts a spot buy's fee from the base asset received rather than the quote. Paper
  P&L will therefore not match live P&L.

"""

from __future__ import annotations

from decimal import Decimal

from adaptive_martingale import AdaptiveMartingale
from adaptive_martingale import AdaptiveMartingaleConfig

from nautilus_trader.adapters.sandbox import SandboxExecutionClientConfig
from nautilus_trader.adapters.sandbox import SandboxExecutionClientFactory
from nautilus_trader.adapters.sodex import SODEX_SPOT
from nautilus_trader.adapters.sodex import Market
from nautilus_trader.adapters.sodex import Network
from nautilus_trader.adapters.sodex import SodexDataClientConfig
from nautilus_trader.adapters.sodex import SodexDataClientFactory
from nautilus_trader.common import Environment
from nautilus_trader.config import LiveRiskEngineConfig
from nautilus_trader.live import LiveNode
from nautilus_trader.model import AccountId
from nautilus_trader.model import AccountType
from nautilus_trader.model import BarType
from nautilus_trader.model import ClientId
from nautilus_trader.model import Currency
from nautilus_trader.model import CurrencyType
from nautilus_trader.model import InstrumentId
from nautilus_trader.model import Money
from nautilus_trader.model import OmsType
from nautilus_trader.model import StrategyId
from nautilus_trader.model import TraderId
from nautilus_trader.model import Venue


NETWORK = Network.MAINNET
MARKET = Market.SPOT
CLIENT_ID = ClientId.from_str(SODEX_SPOT)
VENUE = Venue.from_str(SODEX_SPOT)
TRADER_ID = TraderId.from_str("PAPER-001")
ACCOUNT_ID = AccountId.from_str(f"{SODEX_SPOT}-SANDBOX-001")
STRATEGY_ID = StrategyId.from_str("ADAPTIVE-MARTINGALE-PAPER-001")
INSTRUMENT_ID = InstrumentId.from_str(f"vETH_vUSDC.{SODEX_SPOT}")

# The production shape is a daily regime with a 4-hour signal. Minutes here so a session can watch
# the loop turn over rather than waiting out a day.
REGIME_BAR_TYPE = BarType.from_str(f"{INSTRUMENT_ID}-5-MINUTE-LAST-EXTERNAL")
SIGNAL_BAR_TYPE = BarType.from_str(f"{INSTRUMENT_ID}-1-MINUTE-LAST-EXTERNAL")

BASE_NOTIONAL = Decimal(80)
MAX_TOTAL_NOTIONAL = Decimal(250)
STARTING_BALANCE = 5_000.0

# The largest order the strategy can legitimately place is one base notional: the pyramid factors
# are (1.0, 0.7, 0.5, 0.35), so every layer after the first is smaller. A cap at 1.5x that never
# fires in normal operation and stops a sizing bug from reaching the venue as one enormous order.
MAX_ORDER_NOTIONAL = BASE_NOTIONAL * Decimal("1.5")

# The venue's quote coin is in no standard currency table, and the data client only registers it at
# connect - after the sandbox configuration below needs it to denominate a starting balance.
# Registering it here, at the engine's full fixed-point width, is what keeps the two definitions
# identical: the adapter reuses an already-registered code rather than replacing it, so a narrower
# precision chosen here would silently become the ledger's precision. That matters because this
# venue's fees carry ten decimal places, and eight would truncate them.
QUOTE_CURRENCY = Currency("vUSDC", 16, 0, "vUSDC", CurrencyType.CRYPTO)
Currency.register(QUOTE_CURRENCY, False)


def main() -> None:
    """
    Run the example.
    """
    node = (
        LiveNode.builder("SODEX-ADAPTIVE-MARTINGALE-PAPER", TRADER_ID, Environment.SANDBOX)
        # Nothing to reconcile against: the account exists only inside the matching engine.
        .with_reconciliation(reconciliation=False)
        # Not bypassed. Bypassing skips the per-order notional cap, the free-balance check and the
        # submit rate limit at once, and a paper run whose risk path differs from production's is
        # not rehearsing production. It costs nothing here: market orders price off the LAST bars
        # this strategy already subscribes to, so the engine can value them.
        .with_risk_engine_config(
            LiveRiskEngineConfig(
                bypass=False,
                max_notional_per_order={str(INSTRUMENT_ID): MAX_ORDER_NOTIONAL},
            ),
        )
        .add_data_client(
            SODEX_SPOT,
            SodexDataClientFactory(),
            SodexDataClientConfig(network=NETWORK, market=MARKET),
        )
        .add_simulated_exec_client(
            SODEX_SPOT,
            SandboxExecutionClientFactory(),
            SandboxExecutionClientConfig(
                venue=VENUE,
                starting_balances=[Money(STARTING_BALANCE, QUOTE_CURRENCY)],
                account_id=ACCOUNT_ID,
                # Spot on this venue is a multi-currency cash account: a buy spends the quote coin
                # and credits the base one, and nothing can be sold that is not held. Modeling it
                # as margin would let the strategy take positions the live venue would refuse.
                account_type=AccountType.CASH,
                oms_type=OmsType.NETTING,
                # This venue publishes no order book channel, so bars and trades are all the
                # matching engine has to price a fill against.
                bar_execution=True,
                trade_execution=True,
            ),
        )
        .build()
    )
    node.add_strategy(
        AdaptiveMartingale(
            AdaptiveMartingaleConfig(
                strategy_id=STRATEGY_ID,
                instrument_id=INSTRUMENT_ID,
                client_id=CLIENT_ID,
                regime_bar_type=REGIME_BAR_TYPE,
                signal_bar_type=SIGNAL_BAR_TYPE,
                base_notional=BASE_NOTIONAL,
                max_total_notional=MAX_TOTAL_NOTIONAL,
                warmup_bars=200,
                # Every threshold below is a fraction of price, so it has to be rescaled with the
                # bar interval - the defaults are calibrated for 4-hour bars. A 1.5% pullback is a
                # normal day on that cadence and an impossibility on this one: ETH's whole two-hour
                # range was 0.77%, so the default would hold every bar forever and the run would
                # prove nothing. These are set against the measured minute statistics above, in
                # multiples of a single minute move (2.9 bp) rather than round numbers.
                pullback_threshold=0.0010,
                layer_spacing_pct=0.0015,
                trailing_tp_activation=0.0025,
                trailing_tp_distance=0.0010,
                hard_stop_pct=0.0060,
                gap_threshold=0.0080,
                # Observed atr_pct sits near 3 bp, so 30 bp blocks a genuine spike while leaving
                # the gate live. The 8% default could never fire here, making it dead code.
                volatility_threshold=0.0030,
                # Orders are submitted for real - to the matching engine, not to the venue.
                dry_run=False,
            ),
        ),
    )

    node.run()


if __name__ == "__main__":
    main()
