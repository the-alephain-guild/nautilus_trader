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
Run a pared-down adaptive martingale on SoDEX, driven by two bar feeds.

**With DRY_RUN = False this submits real orders.** On testnet that is play money; the same
program against ``Network.MAINNET`` would spend real funds. Start with DRY_RUN = True, which
exercises the whole decision path — warmup, indicators, regime, entry and exit conditions,
position sizing — and logs what it would have submitted without touching the account.

The shape is a two-timeframe trend follower with a decaying pyramid. A slow feed classifies the
regime from a SuperTrend and two moving averages; a fast feed looks for a pullback entry inside a
bullish regime and adds progressively smaller layers as price falls further. Three independent
paths close the position: a trailing take-profit once the move is in profit, a hard stop on the
average entry, and a forced exit when the regime turns bearish. A gap beyond a threshold exits
immediately, and a cooldown after any exit keeps the next entry from firing on the same whipsaw.

What this deliberately does **not** carry, relative to a full implementation:

- Sizing is a configured notional per layer rather than a fraction of account equity. Equity on a
  multi-currency cash account is not a single number, and resolving it is a separate problem from
  the one this example is meant to exercise.
- Entries and exits are market orders. Quoting would roughly halve the fee, at the cost of
  unfilled-order bookkeeping that is the bulk of a production execution layer.
- One instrument, one direction (long). No portfolio-level heat, no cross-instrument netting.

Two venue behaviours shape the rest, both established by live observation rather than documented:

- A spot buy's fee is taken from the **base** asset received, so holdings end up short of the
  filled quantity and selling that quantity is refused for insufficient balance. The exit
  therefore clamps to the free base balance on a cash account.
- Bars arrive while still forming and the venue never sets its own closed flag, so the data client
  releases a bar only once its successor starts. Every bar reaching ``on_bar`` is final, and the
  first live one arrives a full interval after subscribing — which is why warmup comes from a
  history request rather than from waiting.

"""

from __future__ import annotations

from decimal import Decimal
from enum import StrEnum
from typing import Any

from nautilus_trader.adapters.sodex import SODEX_SPOT
from nautilus_trader.adapters.sodex import Market
from nautilus_trader.adapters.sodex import Network
from nautilus_trader.adapters.sodex import SodexDataClientConfig
from nautilus_trader.adapters.sodex import SodexDataClientFactory
from nautilus_trader.adapters.sodex import SodexExecClientConfig
from nautilus_trader.adapters.sodex import SodexExecutionClientFactory
from nautilus_trader.common import Environment
from nautilus_trader.common import LogColor
from nautilus_trader.config import LiveRiskEngineConfig
from nautilus_trader.config import StrategyConfig
from nautilus_trader.indicators import AverageTrueRange
from nautilus_trader.indicators import ExponentialMovingAverage
from nautilus_trader.live import LiveNode
from nautilus_trader.model import Bar
from nautilus_trader.model import BarType
from nautilus_trader.model import ClientId
from nautilus_trader.model import ClientOrderId
from nautilus_trader.model import InstrumentId
from nautilus_trader.model import OrderFilled
from nautilus_trader.model import OrderRejected
from nautilus_trader.model import OrderSide
from nautilus_trader.model import PositionClosed
from nautilus_trader.model import Quantity
from nautilus_trader.model import StrategyId
from nautilus_trader.model import TraderId
from nautilus_trader.trading import Strategy


class SuperTrend:
    """
    ATR-banded trend direction over bars.

    The engine ships no SuperTrend, and 2.0 removed the Python indicator base class. The actor
    bridges any object answering ``initialized`` and ``handle_bar``, so this wraps the engine's
    own ATR rather than reimplementing a range average.

    """

    def __init__(self, period: int, multiplier: float) -> None:
        """
        Initialize the instance.
        """
        self._atr = AverageTrueRange(period)
        self._multiplier = multiplier
        self._upper = 0.0
        self._lower = 0.0
        self._prev_close = 0.0
        self._trend = 0
        self._count = 0

    @property
    def initialized(self) -> bool:
        """
        Whether the bands and the direction are both established.
        """
        return self._atr.initialized and self._count > 0

    @property
    def trend(self) -> int:
        """
        Direction: 1 bullish, -1 bearish, 0 undetermined.
        """
        return self._trend

    @property
    def value(self) -> float:
        """
        The band currently acting as support or resistance.
        """
        return self._lower if self._trend == 1 else self._upper

    def handle_bar(self, bar: Bar) -> None:
        """
        Update from a bar.
        """
        self._atr.handle_bar(bar)
        close = float(bar.close)

        if not self._atr.initialized:
            self._prev_close = close
            return

        mid = (float(bar.high) + float(bar.low)) / 2.0
        band = self._multiplier * self._atr.value
        basic_upper = mid + band
        basic_lower = mid - band

        if self._count == 0:
            upper, lower = basic_upper, basic_lower
            trend = 1 if close >= mid else -1
        else:
            # A band only tightens towards price; it widens again only once price has closed
            # through it, which is what makes a flip sticky rather than per-bar noise.
            upper = (
                basic_upper
                if basic_upper < self._upper or self._prev_close > self._upper
                else self._upper
            )
            lower = (
                basic_lower
                if basic_lower > self._lower or self._prev_close < self._lower
                else self._lower
            )
            trend = (
                (-1 if close < lower else 1) if self._trend == 1 else (1 if close > upper else -1)
            )

        self._upper = upper
        self._lower = lower
        self._trend = trend
        self._prev_close = close
        self._count += 1

    def reset(self) -> None:
        """
        Clear all state.
        """
        self._atr.reset()
        self._upper = 0.0
        self._lower = 0.0
        self._prev_close = 0.0
        self._trend = 0
        self._count = 0


class Regime(StrEnum):
    """
    Trend classification from the slow feed.
    """

    BULL_STRONG = "BULL_STRONG"
    BULL_WEAK = "BULL_WEAK"
    NEUTRAL = "NEUTRAL"
    BEAR_WEAK = "BEAR_WEAK"
    BEAR_STRONG = "BEAR_STRONG"


class MartingaleState(StrEnum):
    """
    Position lifecycle.
    """

    IDLE = "IDLE"
    SCALING = "SCALING"


BULLISH = (Regime.BULL_STRONG, Regime.BULL_WEAK)


class AdaptiveMartingaleConfig(StrategyConfig):
    """
    Configuration for the adaptive martingale strategy.
    """

    def __init__(
        self,
        *,
        instrument_id: InstrumentId,
        regime_bar_type: BarType,
        signal_bar_type: BarType,
        base_notional: Decimal,
        max_total_notional: Decimal,
        client_id: ClientId | None = None,
        supertrend_period: int = 10,
        supertrend_multiplier: float = 3.0,
        ema_fast_period: int = 21,
        ema_slow_period: int = 55,
        atr_period: int = 14,
        pullback_threshold: float = 0.015,
        layer_spacing_pct: float = 0.02,
        pyramid_factors: tuple[float, ...] = (1.0, 0.7, 0.5, 0.35),
        trailing_tp_activation: float = 0.08,
        trailing_tp_distance: float = 0.03,
        hard_stop_pct: float = 0.15,
        volatility_threshold: float = 0.08,
        regime_exit_confirm_bars: int = 3,
        exit_cooldown_bars: int = 6,
        gap_threshold: float = 0.05,
        warmup_bars: int = 200,
        dry_run: bool = True,
        close_positions_on_stop: bool = True,
        **_kwargs: Any,
    ) -> None:
        """
        Initialize the instance.
        """
        super().__init__()
        self.instrument_id = instrument_id
        self.regime_bar_type = regime_bar_type
        self.signal_bar_type = signal_bar_type
        self.base_notional = base_notional
        self.max_total_notional = max_total_notional
        self.client_id = client_id
        self.supertrend_period = supertrend_period
        self.supertrend_multiplier = supertrend_multiplier
        self.ema_fast_period = ema_fast_period
        self.ema_slow_period = ema_slow_period
        self.atr_period = atr_period
        self.pullback_threshold = pullback_threshold
        self.layer_spacing_pct = layer_spacing_pct
        self.pyramid_factors = pyramid_factors
        self.trailing_tp_activation = trailing_tp_activation
        self.trailing_tp_distance = trailing_tp_distance
        self.hard_stop_pct = hard_stop_pct
        self.volatility_threshold = volatility_threshold
        self.regime_exit_confirm_bars = regime_exit_confirm_bars
        self.exit_cooldown_bars = exit_cooldown_bars
        self.gap_threshold = gap_threshold
        self.warmup_bars = warmup_bars
        self.dry_run = dry_run
        self.close_positions_on_stop = close_positions_on_stop


class AdaptiveMartingale(Strategy):
    """
    A two-timeframe trend follower that scales into drawdown with decaying layer sizes.
    """

    def __init__(self, config: AdaptiveMartingaleConfig) -> None:
        """
        Initialize the instance.
        """
        super().__init__(config)
        self._config = config
        self.instrument: Any | None = None

        self._supertrend = SuperTrend(
            config.supertrend_period,
            config.supertrend_multiplier,
        )
        self._ema_fast_regime = ExponentialMovingAverage(config.ema_fast_period)
        self._ema_slow_regime = ExponentialMovingAverage(config.ema_slow_period)
        self._ema_signal = ExponentialMovingAverage(config.ema_fast_period)
        self._atr_signal = AverageTrueRange(config.atr_period)

        self._regime = Regime.NEUTRAL
        self._state = MartingaleState.IDLE
        self._layer_order_ids: list[ClientOrderId] = []
        self._layer_prices: list[float] = []
        self._layer_qtys: list[float] = []
        self._avg_entry = 0.0
        self._peak_price = 0.0
        self._trailing_active = False
        self._bear_confirm_count = 0
        self._cooldown_remaining = 0
        self._prev_signal_close = 0.0
        self._order_in_flight = False

    def on_start(self) -> None:
        """
        On start.
        """
        self.instrument = self.cache.instrument(self._config.instrument_id)
        if self.instrument is None:
            # Raising rather than calling `self.stop()`: stopping from inside `on_start` re-enters
            # the actor while it is still mutably borrowed, and the real error is then buried under
            # a `RuntimeError: Already borrowed` from the failed stop. A strategy with no
            # instrument can do nothing useful, so failing the start is also the honest outcome.
            raise RuntimeError(f"Could not find instrument for {self._config.instrument_id}")

        regime_bar_type = self._config.regime_bar_type
        signal_bar_type = self._config.signal_bar_type

        # The actor feeds registered indicators from both live bars and history responses, keyed
        # by bar type — so warmup needs no hand-feeding, and hand-feeding would double-count.
        self.register_indicator_for_bars(regime_bar_type, self._supertrend)
        self.register_indicator_for_bars(regime_bar_type, self._ema_fast_regime)
        self.register_indicator_for_bars(regime_bar_type, self._ema_slow_regime)
        self.register_indicator_for_bars(signal_bar_type, self._ema_signal)
        self.register_indicator_for_bars(signal_bar_type, self._atr_signal)

        self.request_bars(
            regime_bar_type,
            limit=self._config.warmup_bars,
            client_id=self._config.client_id,
        )
        self.request_bars(
            signal_bar_type,
            limit=self._config.warmup_bars,
            client_id=self._config.client_id,
        )

        self.subscribe_bars(regime_bar_type, client_id=self._config.client_id)
        self.subscribe_bars(signal_bar_type, client_id=self._config.client_id)

        log_msg = (
            f"started dry_run={self._config.dry_run} "
            f"regime={regime_bar_type} signal={signal_bar_type} "
            f"base_notional={self._config.base_notional} "
            f"max_total_notional={self._config.max_total_notional}"
        )
        self.log.info(log_msg, LogColor.BLUE)

    def on_historical_bars(self, bars: list[Bar]) -> None:
        """
        On a warmup response.
        """
        if not bars:
            self.log.warning("warmup response carried no bars")
            return

        # The indicators have already consumed these. Reporting readiness is what separates
        # "warmup arrived and the indicators are armed" from "no bars ever came back" — two
        # states that otherwise look identical: a run that never decides anything.
        log_msg = (
            f"warmup_received bar_type={bars[0].bar_type} count={len(bars)} "
            f"regime_ready={self._regime_ready()} signal_ready={self._signal_ready()}"
        )
        self.log.info(log_msg, LogColor.CYAN)

    def on_bar(self, bar: Bar) -> None:
        """
        On a bar.
        """
        if bar.bar_type == self._config.regime_bar_type:
            self._update_regime(bar)
        elif bar.bar_type == self._config.signal_bar_type:
            self._on_signal_bar(bar)

    def _regime_ready(self) -> bool:
        return (
            self._supertrend.initialized
            and self._ema_fast_regime.initialized
            and self._ema_slow_regime.initialized
        )

    def _signal_ready(self) -> bool:
        return self._ema_signal.initialized and self._atr_signal.initialized

    def _update_regime(self, bar: Bar) -> None:
        if not self._regime_ready():
            return

        close = float(bar.close)
        fast = self._ema_fast_regime.value
        slow = self._ema_slow_regime.value
        above_fast = close > fast
        above_slow = close > slow
        trend = self._supertrend.trend

        if trend == 1:
            if above_fast and above_slow:
                regime = Regime.BULL_STRONG
            elif above_fast or above_slow:
                regime = Regime.BULL_WEAK
            else:
                regime = Regime.NEUTRAL
        elif trend == -1:
            if not above_fast and not above_slow:
                regime = Regime.BEAR_STRONG
            elif not above_fast and close > slow * 0.95:
                regime = Regime.BEAR_WEAK
            else:
                regime = Regime.NEUTRAL
        else:
            regime = Regime.NEUTRAL

        if regime != self._regime:
            log_msg = (
                f"regime_change from={self._regime.value} to={regime.value} "
                f"supertrend={trend} close={close:.4f} fast={fast:.4f} slow={slow:.4f}"
            )
            self.log.info(log_msg, LogColor.MAGENTA)
        self._regime = regime

    def _on_signal_bar(self, bar: Bar) -> None:
        if not self._signal_ready():
            return

        close = float(bar.close)
        gap = self._gap_pct(close)
        self._prev_signal_close = close
        decision = self._decide(close, gap)

        # One line per signal bar, naming the branch actually taken. Without it, "evaluated and
        # declined" and "never reached" look identical from outside — and the quiet case is the
        # common one, since most bars are holds.
        log_msg = (
            f"signal_bar decision={decision} state={self._state.value} "
            f"regime={self._regime.value} close={close:.4f} "
            f"ema={self._ema_signal.value:.4f} atr_pct={self._atr_signal.value / close:.4f} "
            f"gap={gap:.4f} layers={len(self._layer_prices)} cooldown={self._cooldown_remaining}"
        )
        self.log.info(log_msg)

        if decision == "gap_exit":
            self._exit("gap_protection")
        elif decision.startswith("exit_"):
            self._exit(decision.removeprefix("exit_"))
        elif decision == "enter":
            self._enter(0, close)
        elif decision == "scale":
            self._enter(len(self._layer_prices), close)

    def _decide(self, close: float, gap: float) -> str:
        """
        Name the branch this bar takes, without acting on it.

        Separating the choice from the action is what lets the choice be logged as made rather
        than inferred from whichever side effect happened to follow.
        """
        if abs(gap) > self._config.gap_threshold:
            return "gap_exit" if self._state == MartingaleState.SCALING else "hold_gap"

        if self._cooldown_remaining > 0:
            self._cooldown_remaining -= 1

        if self._state == MartingaleState.SCALING:
            reason = self._exit_reason(close)
            if reason is not None:
                return f"exit_{reason}"
            return "scale" if self._should_scale(close) else "hold_scaling"

        if self._cooldown_remaining > 0:
            return "hold_cooldown"

        blocker = self._open_blocker(close)
        return "enter" if blocker is None else f"hold_{blocker}"

    def _gap_pct(self, close: float) -> float:
        if self._prev_signal_close <= 0.0:
            return 0.0
        return (close - self._prev_signal_close) / self._prev_signal_close

    def _open_blocker(self, close: float) -> str | None:
        """
        Return the first condition blocking a first layer, or `None` if none does.
        """
        if self._regime not in BULLISH:
            return "regime"

        reference = self._ema_signal.value
        if close >= reference * (1.0 - self._config.pullback_threshold):
            return "no_pullback"

        if self._atr_signal.value / close > self._config.volatility_threshold:
            return "volatility"

        return None

    def _should_scale(self, close: float) -> bool:
        if len(self._layer_prices) >= len(self._config.pyramid_factors):
            return False
        if self._regime not in BULLISH:
            return False
        return close <= self._avg_entry * (1.0 - self._config.layer_spacing_pct)

    def _exit_reason(self, close: float) -> str | None:
        if self._avg_entry <= 0.0:
            return None

        if self._regime == Regime.BEAR_STRONG:
            self._bear_confirm_count = 0
            return "regime_exit"
        if self._regime == Regime.BEAR_WEAK:
            self._bear_confirm_count += 1
            if self._bear_confirm_count >= self._config.regime_exit_confirm_bars:
                return "regime_exit"
        else:
            self._bear_confirm_count = 0

        pnl_pct = (close - self._avg_entry) / self._avg_entry
        if pnl_pct <= -self._config.hard_stop_pct:
            return "hard_stop"

        if pnl_pct >= self._config.trailing_tp_activation:
            self._trailing_active = True
        if self._trailing_active:
            self._peak_price = max(self._peak_price, close)
            drawdown = (self._peak_price - close) / self._peak_price
            if drawdown >= self._config.trailing_tp_distance:
                return "trailing_tp"

        return None

    def _committed_notional(self) -> Decimal:
        total = Decimal(0)
        for price, qty in zip(self._layer_prices, self._layer_qtys, strict=True):
            total += Decimal(str(price)) * Decimal(str(qty))
        return total

    def _layer_quantity(self, layer: int, price: float) -> Quantity | None:
        if self.instrument is None:
            return None

        factor = Decimal(str(self._config.pyramid_factors[layer]))
        notional = self._config.base_notional * factor
        committed = self._committed_notional()
        if committed + notional > self._config.max_total_notional:
            log_msg = (
                f"layer_blocked_exposure layer={layer} committed={committed} "
                f"requested={notional} cap={self._config.max_total_notional}"
            )
            self.log.info(log_msg)
            return None

        qty = self.instrument.make_qty(float(notional) / price, round_down=True)
        if qty.as_double() <= 0.0:
            log_msg = (
                f"layer_quantity_rounds_to_zero layer={layer} notional={notional} price={price}"
            )
            self.log.warning(log_msg)
            return None
        return qty

    def _enter(self, layer: int, price: float) -> None:
        if self._order_in_flight:
            return

        qty = self._layer_quantity(layer, price)
        if qty is None:
            return

        log_msg = (
            f"entry_signal layer={layer} price={price:.4f} qty={qty} "
            f"regime={self._regime.value} avg_entry={self._avg_entry:.4f}"
        )
        self.log.info(log_msg, LogColor.GREEN)
        if self._config.dry_run:
            return

        order = self.order_factory.market(
            instrument_id=self._config.instrument_id,
            order_side=OrderSide.BUY,
            quantity=qty,
        )
        self._order_in_flight = True
        self.submit_order(order, client_id=self._config.client_id)

    def _exit(self, reason: str) -> None:
        if self._order_in_flight:
            return

        qty = self._exit_quantity()
        if qty is None:
            # Nothing sellable, so the position is flat in every sense that matters here.
            # Resetting keeps a stale SCALING state from blocking every future entry.
            log_msg = f"exit_without_holdings reason={reason}"
            self.log.warning(log_msg)
            self._reset_position_state()
            return

        log_msg = (
            f"exit_signal reason={reason} qty={qty} layers={len(self._layer_prices)} "
            f"avg_entry={self._avg_entry:.4f}"
        )
        self.log.info(log_msg, LogColor.YELLOW)
        if self._config.dry_run:
            self._reset_position_state()
            return

        order = self.order_factory.market(
            instrument_id=self._config.instrument_id,
            order_side=OrderSide.SELL,
            quantity=qty,
            reduce_only=True,
        )
        self._order_in_flight = True
        self.submit_order(order, client_id=self._config.client_id)

    def _exit_quantity(self) -> Quantity | None:
        if self.instrument is None:
            return None

        positions = self.cache.positions_open(
            instrument_id=self._config.instrument_id,
            strategy_id=self.strategy_id,
        )
        if positions:
            qty = positions[0].quantity
        elif self._config.dry_run:
            # No position exists because nothing was submitted. Reporting the intended size keeps
            # the dry run from silently skipping the exit leg it is meant to demonstrate.
            qty = self.instrument.make_qty(sum(self._layer_qtys), round_down=True)
        else:
            return None

        account = self.portfolio.account(self._config.instrument_id.venue)
        if account is not None and account.is_cash_account():
            # A buy's fee comes out of the base asset received, so holdings fall short of the
            # filled quantity and selling that quantity is refused for insufficient balance.
            free = account.balance_free(self.instrument.base_currency)
            if free is not None:
                held = self.instrument.make_qty(free.as_double(), round_down=True)
                if held.as_double() < qty.as_double():
                    log_msg = f"exit_clamped_to_holdings position={qty} held={held}"
                    self.log.info(log_msg)
                    qty = held

        if qty.as_double() <= 0.0:
            return None
        return qty

    def on_order_filled(self, event: OrderFilled) -> None:
        """
        On an order fill.
        """
        # An order can fill across several events, so the in-flight guard must hold until the order
        # itself is done. Releasing on the first partial would let the next bar submit a second
        # order while this one is still working.
        order = self.cache.order(event.client_order_id)
        if order is None or order.leaves_qty.as_double() <= 0.0:
            self._order_in_flight = False

        if event.order_side == OrderSide.SELL:
            log_msg = f"exit_filled qty={event.last_qty} price={event.last_px}"
            self.log.info(log_msg, LogColor.YELLOW)
            return

        price = float(event.last_px)
        qty = event.last_qty.as_double()

        # A layer is an order, not a fill. Paper trading showed one market buy arriving as 0.00001
        # then 0.00102, which appended two layers for a single order — consuming a pyramid factor
        # early and overstating exposure. Keying on the client order id folds partials back into
        # the layer they belong to, at their volume-weighted price.
        if self._layer_order_ids and self._layer_order_ids[-1] == event.client_order_id:
            index = len(self._layer_prices) - 1
            held = self._layer_qtys[index]
            self._layer_prices[index] = (self._layer_prices[index] * held + price * qty) / (
                held + qty
            )
            self._layer_qtys[index] = held + qty
        else:
            self._layer_order_ids.append(event.client_order_id)
            self._layer_prices.append(price)
            self._layer_qtys.append(qty)

        index = len(self._layer_prices) - 1
        self._avg_entry = self._weighted_average()
        self._state = MartingaleState.SCALING
        self._peak_price = max(self._peak_price, price)

        log_msg = (
            f"layer_filled layer={index} fill_price={price:.4f} fill_qty={qty} "
            f"layer_price={self._layer_prices[index]:.4f} layer_qty={self._layer_qtys[index]} "
            f"avg_entry={self._avg_entry:.4f}"
        )
        self.log.info(log_msg, LogColor.GREEN)

    def on_order_rejected(self, event: OrderRejected) -> None:
        """
        On an order rejection.
        """
        self._order_in_flight = False
        log_msg = f"order_rejected reason={event.reason}"
        self.log.error(log_msg)

    def on_position_closed(self, event: PositionClosed) -> None:
        """
        On a position close.
        """
        log_msg = f"position_closed realized_pnl={event.realized_pnl}"
        self.log.info(log_msg, LogColor.YELLOW)
        self._reset_position_state()

    def _weighted_average(self) -> float:
        total_qty = sum(self._layer_qtys)
        if total_qty <= 0.0:
            return 0.0
        pairs = zip(self._layer_prices, self._layer_qtys, strict=True)
        return sum(p * q for p, q in pairs) / total_qty

    def _reset_position_state(self) -> None:
        self._state = MartingaleState.IDLE
        self._layer_order_ids.clear()
        self._layer_prices.clear()
        self._layer_qtys.clear()
        self._avg_entry = 0.0
        self._peak_price = 0.0
        self._trailing_active = False
        self._bear_confirm_count = 0
        self._cooldown_remaining = self._config.exit_cooldown_bars

    def on_stop(self) -> None:
        """
        On stop.
        """
        self.cancel_all_orders(self._config.instrument_id)
        if self._config.close_positions_on_stop and not self._config.dry_run:
            self.close_all_positions(self._config.instrument_id)

    def on_reset(self) -> None:
        """
        On reset.
        """
        self._supertrend.reset()
        self._ema_fast_regime.reset()
        self._ema_slow_regime.reset()
        self._ema_signal.reset()
        self._atr_signal.reset()
        self._regime = Regime.NEUTRAL
        self._prev_signal_close = 0.0
        self._order_in_flight = False
        self._reset_position_state()
        self._cooldown_remaining = 0


# WARNING: With DRY_RUN = False this submits orders to the configured network.
DRY_RUN = True
NETWORK = Network.TESTNET
MARKET = Market.SPOT
CLIENT_ID = ClientId.from_str(SODEX_SPOT)
TRADER_ID = TraderId.from_str("TESTER-001")
STRATEGY_ID = StrategyId.from_str("ADAPTIVE-MARTINGALE-001")
INSTRUMENT_ID = InstrumentId.from_str(f"vBTC_vUSDC.{SODEX_SPOT}")

# The production shape is a daily regime with a 4-hour signal. Both are configurable precisely so
# a verification run can use minutes and observe the whole loop inside one session.
REGIME_BAR_TYPE = BarType.from_str(f"{INSTRUMENT_ID}-5-MINUTE-LAST-EXTERNAL")
SIGNAL_BAR_TYPE = BarType.from_str(f"{INSTRUMENT_ID}-1-MINUTE-LAST-EXTERNAL")

BASE_NOTIONAL = Decimal(80)
MAX_TOTAL_NOTIONAL = Decimal(250)


def main() -> None:
    """
    Run the example.
    """
    node = (
        LiveNode.builder("SODEX-ADAPTIVE-MARTINGALE-001", TRADER_ID, Environment.LIVE)
        .with_reconciliation(reconciliation=True)
        .with_risk_engine_config(LiveRiskEngineConfig(bypass=True))
        .add_data_client(
            SODEX_SPOT,
            SodexDataClientFactory(),
            SodexDataClientConfig(network=NETWORK, market=MARKET),
        )
        .add_exec_client(
            SODEX_SPOT,
            SodexExecutionClientFactory(),
            SodexExecClientConfig(network=NETWORK, market=MARKET),
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
                dry_run=DRY_RUN,
            ),
        ),
    )

    node.run()


if __name__ == "__main__":
    main()
