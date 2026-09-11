# 策略 (Strategies)

策略继承 `Strategy` 类，并实现其逻辑所需的方法。

**核心能力**：

- 所有 `Actor` 的能力。
- 订单 (order) 管理。

**与 Actor 的关系**：
`Strategy` 类继承自 `Actor`，这意味着策略拥有 Actor 的全部功能，外加订单管理能力。

:::tip
我们建议在深入策略开发之前，先阅读 [Actors](actors.md) 指南。
:::

策略可以在任何[环境上下文](architecture.md#environment-contexts)中添加到 Nautilus 系统，系统一旦启动，就会立即根据其逻辑开始发送命令和接收事件。

利用数据 (data) 摄取、事件处理和订单管理这些基本构建模块（我们将在下文讨论），可以构建任何类型的策略，包括方向性、动量、再平衡、配对交易、做市等。

请参阅 [`Strategy` API 参考](/docs/python-api-latest/trading.html)，获取所有可用方法。

Nautilus 交易策略由两个主要部分组成：

- 策略实现本身，通过继承 `Strategy` 类来定义。
- *可选的* 策略配置 (configuration)，通过继承 `StrategyConfig` 类来定义。

:::tip
一旦策略定义完成，相同的源代码既可用于回测 (backtest)，也可用于实盘交易 (live trading)。
:::

策略的主要能力包括：

- 历史数据请求。
- 实时数据订阅 (subscription)。
- 设置时间提醒或定时器。
- 缓存 (cache) 访问。
- 投资组合 (portfolio) 访问。
- 创建和管理订单与持仓 (position)。

## 策略实现

交易策略继承自 `Strategy`，因此你必须定义一个构造函数。至少需要初始化基类：

```python
from nautilus_trader.trading.strategy import Strategy


class MyStrategy(Strategy):
    def __init__(self) -> None:
        super().__init__()  # <-- 必须调用父类来初始化策略
```

在此基础上，你可以根据需要实现处理器 (handler)，以便根据状态 (state) 转换和事件执行相应操作。

:::warning
不要在 `__init__` 构造函数中（注册之前）调用 `clock` 和 `logger` 等组件 (component)。
这是因为系统时钟和日志子系统尚未初始化。
:::

### 处理器

处理器是 `Strategy` 类中的方法，可根据事件或状态变化执行相应操作。
这些方法以 `on_*` 为前缀。你可以根据策略的需求实现其中任意或全部处理器。

为类似类型的事件提供多个处理器，是为了让你能够控制处理粒度。
你可以用专用处理器响应特定事件，也可以用通用处理器响应一系列相关事件（使用典型的 switch 语句逻辑）。
系统按照从最具体到最通用的顺序依次调用处理器。

#### 状态动作

生命周期 (lifecycle) 状态变化会触发这些处理器。建议：

- 使用 `on_start` 方法初始化策略（例如获取金融工具 (instrument)、订阅数据）。
- 使用 `on_stop` 方法执行清理任务（例如取消未完成订单、关闭已开持仓、取消数据订阅）。

```python
def on_start(self) -> None:
def on_stop(self) -> None:
def on_resume(self) -> None:
def on_reset(self) -> None:
def on_dispose(self) -> None:
def on_degrade(self) -> None:
def on_fault(self) -> None:
def on_save(self) -> dict[str, bytes]:  # 返回用户自定义的状态字典用于保存
def on_load(self, state: dict[str, bytes]) -> None:
```

#### 数据处理

这些处理器接收数据更新，包括内置的行情数据 (market data) 和用户自定义数据。

```python
from nautilus_trader.core import Data
from nautilus_trader.model import OrderBook
from nautilus_trader.model import Bar
from nautilus_trader.model import QuoteTick
from nautilus_trader.model import TradeTick
from nautilus_trader.model import OrderBookDeltas
from nautilus_trader.model import InstrumentClose
from nautilus_trader.model import InstrumentStatus
from nautilus_trader.model import OptionChainSlice
from nautilus_trader.model import OptionGreeks
from nautilus_trader.model.instruments import Instrument

def on_order_book_deltas(self, deltas: OrderBookDeltas) -> None:
def on_order_book(self, order_book: OrderBook) -> None:
def on_quote_tick(self, tick: QuoteTick) -> None:
def on_trade_tick(self, tick: TradeTick) -> None:
def on_bar(self, bar: Bar) -> None:
def on_instrument(self, instrument: Instrument) -> None:
def on_instrument_status(self, data: InstrumentStatus) -> None:
def on_instrument_close(self, data: InstrumentClose) -> None:
def on_option_greeks(self, greeks: OptionGreeks) -> None:
def on_option_chain(self, chain: OptionChainSlice) -> None:
def on_historical_data(self, data: Data) -> None:
def on_data(self, data: Data) -> None:  # 自定义数据传递给此处理器
def on_signal(self, signal: Data) -> None:  # 自定义信号传递给此处理器
```

#### 订单管理

这些处理器接收与订单相关的事件。
`OrderEvent` 类型消息按以下顺序传递给处理器：

1. 特定处理器（例如 `on_order_accepted`、`on_order_rejected` 等）
2. `on_order_event(...)`
3. `on_event(...)`

```python
from nautilus_trader.model.events import OrderAccepted
from nautilus_trader.model.events import OrderCanceled
from nautilus_trader.model.events import OrderCancelRejected
from nautilus_trader.model.events import OrderDenied
from nautilus_trader.model.events import OrderEmulated
from nautilus_trader.model.events import OrderEvent
from nautilus_trader.model.events import OrderExpired
from nautilus_trader.model.events import OrderFilled
from nautilus_trader.model.events import OrderInitialized
from nautilus_trader.model.events import OrderModifyRejected
from nautilus_trader.model.events import OrderPendingCancel
from nautilus_trader.model.events import OrderPendingUpdate
from nautilus_trader.model.events import OrderRejected
from nautilus_trader.model.events import OrderReleased
from nautilus_trader.model.events import OrderSubmitted
from nautilus_trader.model.events import OrderTriggered
from nautilus_trader.model.events import OrderUpdated

def on_order_initialized(self, event: OrderInitialized) -> None:
def on_order_denied(self, event: OrderDenied) -> None:
def on_order_emulated(self, event: OrderEmulated) -> None:
def on_order_released(self, event: OrderReleased) -> None:
def on_order_submitted(self, event: OrderSubmitted) -> None:
def on_order_rejected(self, event: OrderRejected) -> None:
def on_order_accepted(self, event: OrderAccepted) -> None:
def on_order_canceled(self, event: OrderCanceled) -> None:
def on_order_expired(self, event: OrderExpired) -> None:
def on_order_triggered(self, event: OrderTriggered) -> None:
def on_order_pending_update(self, event: OrderPendingUpdate) -> None:
def on_order_pending_cancel(self, event: OrderPendingCancel) -> None:
def on_order_modify_rejected(self, event: OrderModifyRejected) -> None:
def on_order_cancel_rejected(self, event: OrderCancelRejected) -> None:
def on_order_updated(self, event: OrderUpdated) -> None:
def on_order_filled(self, event: OrderFilled) -> None:
def on_order_event(self, event: OrderEvent) -> None:  # 所有订单事件消息最终都会传递给此处理器
```

#### 持仓管理

这些处理器接收与持仓相关的事件。
`PositionEvent` 类型消息按以下顺序传递给处理器：

1. 特定处理器（例如 `on_position_opened`、`on_position_changed` 等）
2. `on_position_event(...)`
3. `on_event(...)`

```python
from nautilus_trader.model.events import PositionChanged
from nautilus_trader.model.events import PositionClosed
from nautilus_trader.model.events import PositionEvent
from nautilus_trader.model.events import PositionOpened

def on_position_opened(self, event: PositionOpened) -> None:
def on_position_changed(self, event: PositionChanged) -> None:
def on_position_closed(self, event: PositionClosed) -> None:
def on_position_event(self, event: PositionEvent) -> None:  # 所有持仓事件消息最终都会传递给此处理器
```

#### 通用事件处理

此处理器最终会接收到达策略的所有事件消息，包括没有其他特定处理器的事件。

```python
from nautilus_trader.core.message import Event

def on_event(self, event: Event) -> None:
```

#### 处理器示例

以下示例展示了一个典型的 `on_start` 处理器方法实现（取自 EMA 交叉策略示例）。
我们可以看到以下内容：

- 注册指标 (indicator) 以接收 K线 (Bar) 更新。
- 请求历史数据（用于填充指标）。
- 订阅实时数据。

缓存检查在实盘交易中很重要。直接订阅假设金融工具已经由 instrument provider 配置加载，或由先前的金融工具请求加载。

```python
def on_start(self) -> None:
    """
    策略启动时执行的操作。
    """
    self.instrument = self.cache.instrument(self.instrument_id)
    if self.instrument is None:
        self.log.error(f"Could not find instrument for {self.instrument_id}")
        self.stop()  # 将策略转换为 STOPPED 状态
        return

    # 注册指标以接收更新
    self.register_indicator_for_bars(self.bar_type, self.fast_ema)
    self.register_indicator_for_bars(self.bar_type, self.slow_ema)

    # 获取历史数据
    self.request_bars(self.bar_type)

    # 订阅实时数据
    self.subscribe_bars(self.bar_type)
    self.subscribe_quote_ticks(self.instrument_id)
```

### 时钟与定时器

策略可以访问一个 `Clock`，它提供了多种方法来创建不同的时间戳，以及设置时间提醒或定时器来触发 `TimeEvent`。

请参阅 [`Clock` API 参考](/docs/python-api-latest/common.html)，获取所有可用方法。

#### 当前时间戳

虽然有多种方式获取当前时间戳，以下是两个常用方法示例：

获取当前 UTC 时间戳（带时区的 `pd.Timestamp`）：

```python
import pandas as pd


now: pd.Timestamp = self.clock.utc_now()
```

获取当前 UTC 时间戳（自 UNIX 纪元以来的纳秒数）：

```python
unix_nanos: int = self.clock.timestamp_ns()
```

#### 时间提醒

可以设置时间提醒，在指定的提醒时间向 `on_event` 处理器分发一个 `TimeEvent`。在实盘环境中，可能会有几微秒的轻微延迟。

以下示例设置了一个从当前时间起一分钟后触发的时间提醒：

```python
import pandas as pd

# 从现在起一分钟后触发一个 TimeEvent
self.clock.set_time_alert(
    name="MyTimeAlert1",
    alert_time=self.clock.utc_now() + pd.Timedelta(minutes=1),
)
```

#### 定时器

可以设置连续定时器，按固定间隔生成 `TimeEvent`，直到定时器到期或被取消。

以下示例设置了一个每分钟触发一次、立即开始的定时器：

```python
import pandas as pd

# 每分钟触发一个 TimeEvent
self.clock.set_timer(
    name="MyTimer1",
    interval=pd.Timedelta(minutes=1),
)
```

### 缓存访问

交易实例的中央 `Cache` 存储数据和执行对象（订单、持仓等）。
有许多可用方法，通常带有过滤功能，这里介绍一些基本用例。

#### 获取数据

以下示例展示了如何从缓存中获取数据（假设已分配了某个金融工具 ID 属性）。
如果请求的数据不可用，这些方法返回 `None`。

```python
last_quote = self.cache.quote_tick(self.instrument_id)
last_trade = self.cache.trade_tick(self.instrument_id)
last_bar = self.cache.bar(bar_type)
```

#### 获取执行对象

以下示例展示了如何从缓存中获取单个订单和持仓对象：

```python
order = self.cache.order(client_order_id)
position = self.cache.position(position_id)
```

请参阅 [`Cache` API 参考](/docs/python-api-latest/cache.html)，获取所有可用方法。

### 投资组合访问

交易实例的中央 `Portfolio` 提供账户和持仓信息。
以下展示了可用方法的概览。

#### 账户和持仓信息

```python
import decimal

from nautilus_trader.accounting.accounts.base import Account
from nautilus_trader.model import Venue
from nautilus_trader.model import Currency
from nautilus_trader.model import Money
from nautilus_trader.model import InstrumentId

def account(self, venue: Venue) -> Account

def balances_locked(self, venue: Venue) -> dict[Currency, Money]
def margins_init(self, venue: Venue) -> dict[Currency, Money]
def margins_maint(self, venue: Venue) -> dict[Currency, Money]
def unrealized_pnls(self, venue: Venue) -> dict[Currency, Money]
def realized_pnls(self, venue: Venue) -> dict[Currency, Money]
def net_exposures(self, venue: Venue) -> dict[Currency, Money]

def unrealized_pnl(self, instrument_id: InstrumentId) -> Money
def realized_pnl(self, instrument_id: InstrumentId) -> Money
def net_exposure(self, instrument_id: InstrumentId) -> Money
def net_position(self, instrument_id: InstrumentId) -> decimal.Decimal

def is_net_long(self, instrument_id: InstrumentId) -> bool
def is_net_short(self, instrument_id: InstrumentId) -> bool
def is_flat(self, instrument_id: InstrumentId) -> bool
def is_completely_flat(self) -> bool
```

请参阅 [`Portfolio` API 参考](/docs/python-api-latest/portfolio.html)，获取所有可用方法。

#### 报告与分析

`Portfolio` 还提供了一个 `PortfolioAnalyzer`，可以接受灵活数量的数据（以适应不同的回溯窗口）。该分析器会跟踪并生成绩效指标和统计数据。

请参阅 [`PortfolioAnalyzer` API 参考](/docs/python-api-latest/analysis.html)和[投资组合统计](portfolio.md#portfolio-statistics)指南。

### 交易命令

以下交易命令可用于订单管理。
另请参阅[执行](../concepts/execution.md) (Execution) 指南，了解通过系统的完整流程。

#### 提交订单

每个 `Strategy` 的基类上都提供了一个 `OrderFactory` 作为便利工具，减少了创建不同 `Order` 对象所需的样板代码（不过如果交易者愿意，也可以直接使用 `Order.__init__(...)` 构造函数初始化这些对象）。

`SubmitOrder` 或 `SubmitOrderList` 命令将流向哪个组件执行，取决于以下条件：

- 如果指定了 `emulation_trigger`，命令将*首先*发送到 `OrderEmulator`。
- 如果指定了 `exec_algorithm_id`（且没有 `emulation_trigger`），命令将*首先*发送到相应的 `ExecAlgorithm`。
- 否则，命令将*首先*发送到 `RiskEngine`。

:::info 为什么需要 OrderEmulator？
很多交易所不原生支持高级订单类型（如 `STOP_LIMIT`、`TRAILING_STOP` 等）。`OrderEmulator` 的作用是**在本地模拟**这些订单类型：持续监控市场价格（由 `emulation_trigger` 指定监控买卖价或最新成交价），当触发条件满足时，将模拟订单转换为简单的 `MARKET` 或 `LIMIT` 订单，再提交到交易所。

以模拟 `STOP_LIMIT` 订单为例，其生命周期如下：

```
策略提交订单(emulation_trigger=LAST_PRICE)
  → RiskEngine 风险检查
  → OrderEmulator 持有订单，订阅市场数据
  → 持续监控最新成交价...
  → 价格触及 stop 价格！
  → 转换为 LIMIT 订单，释放
  → 再次经过 RiskEngine 风险检查
  → 提交到交易所执行
```

这一设计带来四个关键优势：

- **跨交易所统一**：策略代码可以使用相同的高级订单类型，无需关心交易所是否原生支持
- **回测/实盘一致**：模拟逻辑在所有环境中完全相同
- **风险双重检查**：订单在初始提交和触发释放时都经过 `RiskEngine`
- **可恢复**：模拟订单可持久化，系统崩溃后重启能恢复状态
:::

以下示例提交一个用于模拟的 `LIMIT` 买入订单（参见[模拟订单](orders/emulated.md)）：

```python
from nautilus_trader.model.enums import OrderSide
from nautilus_trader.model.enums import TriggerType
from nautilus_trader.model.orders import LimitOrder


def buy(self) -> None:
    """
    用户的简单买入方法（示例）。
    """
    order: LimitOrder = self.order_factory.limit(
        instrument_id=self.instrument_id,
        order_side=OrderSide.BUY,
        quantity=self.instrument.make_qty(self.trade_size),
        price=self.instrument.make_price(5000.00),
        emulation_trigger=TriggerType.LAST_PRICE,
    )

    self.submit_order(order)
```

:::info
你可以同时指定订单模拟和执行算法。在这种情况下，订单首先发送到 `OrderEmulator`，释放后再路由到 `ExecAlgorithm`。
:::

以下示例向 TWAP 执行算法提交一个 `MARKET` 买入订单：

```python
from nautilus_trader.model.enums import OrderSide
from nautilus_trader.model.enums import TimeInForce
from nautilus_trader.model import ExecAlgorithmId


def buy(self) -> None:
    """
    用户的简单买入方法（示例）。
    """
    order: MarketOrder = self.order_factory.market(
        instrument_id=self.instrument_id,
        order_side=OrderSide.BUY,
        quantity=self.instrument.make_qty(self.trade_size),
        time_in_force=TimeInForce.FOK,
        exec_algorithm_id=ExecAlgorithmId("TWAP"),
        exec_algorithm_params={"horizon_secs": 20, "interval_secs": 2.5},
    )

    self.submit_order(order)
```

#### 取消订单

订单可以单独取消、批量取消，或取消某个金融工具的所有订单（可选方向过滤）。

如果订单已经*关闭*或已在等待取消中，则会记录一条警告。

如果订单当前处于*开放*状态，其状态将变为 `PENDING_CANCEL`。

`CancelOrder`、`CancelAllOrders` 或 `BatchCancelOrders` 命令将流向哪个组件执行，取决于以下条件：

- 如果订单当前处于模拟状态，命令将*首先*发送到 `OrderEmulator`。
- 如果指定了 `exec_algorithm_id`（且没有 `emulation_trigger`），并且订单仍在本地系统中活跃，命令将*首先*发送到相应的 `ExecAlgorithm`。
- 否则，订单将*首先*发送到 `ExecutionEngine`。

:::info
命令离开策略后，任何托管 (managed) 的 GTD 定时器也会被取消。
:::

以下展示如何取消单个订单：

```python
self.cancel_order(order)
```

以下展示如何批量取消订单：

```python
from nautilus_trader.model.orders import Order


my_order_list: list[Order] = [order1, order2, order3]
self.cancel_orders(my_order_list)
```

以下展示如何取消所有订单：

```python
self.cancel_all_orders()
```

#### 修改订单

当订单处于模拟状态，或在交易场所上*开放*时（如果支持），可以单独修改订单。

如果订单已经*关闭*或已在等待取消中，则会记录一条警告。
如果订单当前处于*开放*状态，其状态将变为 `PENDING_UPDATE`。

:::warning
至少需要有一个值与原始订单不同，命令才有效。
:::

`ModifyOrder` 命令将流向哪个组件执行，取决于以下条件：

- 如果订单当前处于模拟状态，命令将*首先*发送到 `OrderEmulator`。
- 否则，订单将*首先*发送到 `RiskEngine`。

:::info
一旦订单处于执行算法的控制之下，策略就无法直接修改它（只能取消）。
:::

以下展示如何修改当前在交易场所上*开放*的 `LIMIT` 买入订单的数量：

```python
from nautilus_trader.model import Quantity


new_quantity: Quantity = Quantity.from_int(5)
self.modify_order(order, new_quantity)
```

:::info
价格和触发价格也可以修改（当处于模拟状态或交易场所支持时）。
:::

#### 市场退出

`market_exit()` 方法提供了一种优雅的方式，用于退出策略的所有持仓并取消所有订单。
退出完成后策略仍保持运行，如果你需要，之后可以重新入场。

```python
self.market_exit()
```

市场退出流程：

1. 取消该策略所有未成交和在途 (in-flight) 的订单。
2. 用市价单关闭所有未平仓持仓。
3. 周期性检查（间隔为 `market_exit_interval_ms`），直到所有订单都已结清且持仓全部关闭。
4. 一旦持仓归零 (flat)，或达到 `market_exit_max_attempts` 后，调用 `post_market_exit()`。

提供了两个钩子用于自定义逻辑：

- `on_market_exit()` —— 退出流程开始时调用。
- `post_market_exit()` —— 退出流程完成时调用。

```python
class MyStrategy(Strategy):
    def on_market_exit(self) -> None:
        self.log.info("Beginning market exit...")

    def post_market_exit(self) -> None:
        self.log.info("Market exit complete")
```

在市场退出期间，非 reduce-only 的订单会被自动拒绝 (denied)。对于订单列表 (order list)，如果列表中有任何一个订单不是 reduce-only，则整个列表都会被拒绝，以保持列表语义（例如带有相互依赖关系的 bracket 订单）。

要检查退出是否正在进行（例如，以跳过提交订单的逻辑），使用 `is_exiting()`：

```python
def on_quote_tick(self, tick: QuoteTick) -> None:
    if self.is_exiting():
        return  # 退出期间跳过订单逻辑
    # ... 正常的订单逻辑
```

要在策略停止时自动执行市场退出，设置 `manage_stop=True`：

```python
config = StrategyConfig(manage_stop=True)
```

启用此选项后，调用 `stop()` 会先执行市场退出，待持仓归零后再停止策略。

`StrategyConfig` 中的配置选项：

- `manage_stop`（默认值：False）—— 如果为 True，`stop()` 会在停止前执行市场退出。
- `market_exit_interval_ms`（默认值：100）—— 退出完成检查之间的间隔。
- `market_exit_max_attempts`（默认值：100）—— 完成退出前的最大检查次数。
- `market_exit_time_in_force`（默认值：None/GTC）—— 关闭市价单的有效期类型 (time in force)。
- `market_exit_reduce_only`（默认值：True）—— 关闭市价单是否应为 reduce only。

## 策略配置

独立的配置类对策略在何处以及如何实例化提供了完全的灵活性。配置可以通过网络序列化，从而支持分布式回测和远程实盘交易。

这是可选的。你可以跳过配置，直接将参数传给策略构造函数。如果你想要分布式回测或远程实盘交易，则需要定义一个配置。

以下是一个配置示例：

```python
from decimal import Decimal
from nautilus_trader.config import StrategyConfig
from nautilus_trader.model import Bar, BarType
from nautilus_trader.model import InstrumentId
from nautilus_trader.trading.strategy import Strategy


# 配置定义
class MyStrategyConfig(StrategyConfig):
    instrument_id: InstrumentId  # 示例值: "ETHUSDT-PERP.BINANCE"
    bar_type: BarType  # 示例值: "ETHUSDT-PERP.BINANCE-15-MINUTE[LAST]-EXTERNAL"
    fast_ema_period: int = 10
    slow_ema_period: int = 20
    trade_size: Decimal
    order_id_tag: str


# 策略定义
class MyStrategy(Strategy):
    def __init__(self, config: MyStrategyConfig) -> None:
        # 始终初始化父类 Strategy
        # 此后，配置已存储并可通过 `self.config` 访问
        super().__init__(config)

        # 自定义状态变量
        self.time_started = None
        self.count_of_processed_bars: int = 0

    def on_start(self) -> None:
        self.time_started = self.clock.utc_now()  # 记录策略启动时间
        self.subscribe_bars(self.config.bar_type)  # 查看如何通过 `self.config` 暴露配置数据

    def on_bar(self, bar: Bar):
        self.count_of_processed_bars += 1  # 更新已处理的 K线 计数


# 使用具体值实例化配置。通过设置：
#   - InstrumentId - 参数化策略将交易的金融工具。
#   - BarType - 参数化策略将使用的 K线 数据。
config = MyStrategyConfig(
    instrument_id=InstrumentId.from_str("ETHUSDT-PERP.BINANCE"),
    bar_type=BarType.from_str("ETHUSDT-PERP.BINANCE-15-MINUTE[LAST]-EXTERNAL"),
    trade_size=Decimal(1),
    order_id_tag="001",
)

# 将配置传递给交易策略。
strategy = MyStrategy(config=config)
```

通过 `self.config` 访问配置值。
这在两者之间提供了清晰的分离：

- 配置数据（通过 `self.config` 访问）：
  - 包含定义策略工作方式的初始设置。
  - 例如：`self.config.trade_size`、`self.config.instrument_id`

- 策略状态变量（作为直接属性）：
  - 跟踪策略的任何自定义状态。
  - 例如：`self.time_started`、`self.count_of_processed_bars`

这种分离使代码更易于理解和维护。

:::note
尽管定义交易单一金融工具的策略通常是合理的，但单个策略可以处理的金融工具数量仅受机器资源限制。
:::

### 托管 GTD 过期

策略可以为具有 GTD（*Good 'till Date*，到期前有效）有效期类型的订单管理过期。如果交易所/经纪商不支持此有效期选项，或者你出于任何原因希望由策略来管理，这会很有用。

要使用此选项，请在 `StrategyConfig` 中传入 `manage_gtd_expiry=True`。当提交具有 GTD 有效期的订单时，策略将自动启动一个内部时间提醒。
一旦内部 GTD 时间提醒到达，订单将被取消（如果尚未*关闭*）。

某些交易场所（如 Binance Futures）支持 GTD 有效期，因此在使用 `managed_gtd_expiry` 时，你应该在执行客户端配置中设置 `use_gtd=False` 以避免冲突。

### 多策略

如果你打算运行同一策略的多个实例，使用不同的配置（例如交易不同的金融工具），那么每个实例都需要一个唯一的策略 ID 和 order ID tag。

如果未提供 `strategy_id`，平台会根据策略类名和 order ID tag 构建策略 ID。该 tag 可通过 `order_id_tag` 提供；否则注册时会分配下一个数字 tag，从 `000` 开始。例如，上述配置生成的策略 ID 为 `MyStrategy-001`。

如果同时提供了 `strategy_id` 和 `order_id_tag`，Rust 会将该 tag 追加到运行时策略 ID 后面，除非该 ID 已经以这个 tag 结尾。例如，`strategy_id=MyStrategy-PRIMARY` 配合 `order_id_tag=ABC` 会变成 `MyStrategy-PRIMARY-ABC`。
如果省略了 `order_id_tag`，Rust 会使用 `strategy_id` 中以连字符分隔的最后一部分作为 order ID tag。

:::note
平台有内置的安全措施：如果两个策略共享重复的策略 ID，在注册时会抛出 `RuntimeError`，提示该策略 ID 已被注册。
:::

这样做的原因是，系统必须能够识别各种命令和事件属于哪个策略。order ID tag 还能保证同一交易者下不同策略生成的 client order ID 保持唯一。

:::info Rust 实现
Rust 将 `StrategyConfig` 视为不可变的构造输入。运行时的 `StrategyId` 携带 order ID tag，与 Python/Cython 的行为一致。这使得 actor 注册、client order ID 生成、order list ID 生成和 position ID 生成都通过 `strategy_id.get_tag()` 保持一致。

如果省略了 `strategy_id`，`order_id_tag` 会覆盖生成的后缀，例如 `MyStrategy-ABC`。
:::

请参阅 [`StrategyId` API 参考](/docs/python-api-latest/model/identifiers.html)，获取更多详情。

## 相关指南

- [Actors](actors.md) —— 策略所继承的基类。
- [Events](events.md) —— 事件类型与处理器分发。
- [Orders](orders/) —— 从策略管理的订单类型与订单管理。
- [Backtesting](backtesting.md) —— 用历史数据测试策略。
