# 账务 (Accounting)

账务子系统负责追踪平台所交互的每一个账户的余额、保证金和盈亏（PnL）。本指南涵盖数据模型、策略所使用的查询 API，以及适配器作者为了在各交易场所之间保持一致性必须遵循的约定。

它同样适用于回测和实盘交易。关于回测专用的配置（初始余额、各交易场所的保证金模型选择），请参阅 [回测](backtesting.md)。

## 账户类型 (Account types)

当你为实盘交易或回测把某个交易场所挂接到引擎时，需要通过 `account_type` 在三种账务模式中选择一种：

| 账户类型    | 典型用例                | 引擎锁定的内容                              |
| ------- | ------------------- | ------------------------------------ |
| Cash    | 现货交易（如 BTC/USDT、股票） | 待成交订单将开仓的每一个持仓的名义价值（notional value）。 |
| Margin  | 衍生品或任何允许使用杠杆的产品     | 每个订单的初始保证金，加上未平仓持仓的维持保证金。            |
| Betting | 体育博彩、做市庄家           | 交易场所要求的下注额（stake）；无杠杆。               |

### 现金账户 (Cash accounts)

现金账户对交易进行全额结算；没有杠杆，因此也不存在保证金的概念。锁定余额反映的是为待成交订单预留的名义价值。

### 保证金账户 (Margin accounts)

保证金账户支持需要抵押品的金融工具，例如期货或带杠杆的加密货币永续合约。它们追踪账户余额，为未平仓订单和持仓预留保证金，并对每个金融工具应用可配置的杠杆。保证金按两种作用域（scope）追踪，详见下文的 [保证金作用域](#margin-scopes)。

**关键术语**：

- **杠杆 (Leverage)**：相对于账户净值放大敞口。更高的杠杆会同时提高潜在收益和风险。
- **初始保证金 (Initial margin)**：提交订单时预留的抵押品。
- **维持保证金 (Maintenance margin)**：保持一个未平仓持仓所需的最低抵押品。
- **锁定余额 (Locked balance)**：作为抵押品预留、不可用于新订单的资金。

:::note
只减仓（reduce-only）订单不会计入现金账户的 `balance_locked`，也不会增加保证金账户的初始保证金，因为它们只能减少敞口。
:::

### 博彩账户 (Betting accounts)

博彩账户专门用于那些你下注一定金额以赢取或损失固定赔付的交易场所（预测市场、体育博彩）。引擎只锁定交易场所要求的下注额；杠杆和保证金均不适用。

## 余额模型 (Balance model)

一个 `AccountBalance` 持有同一币种下的三个数值：

- `total`：交易场所报告的总余额数字（视交易场所而定，可能是钱包余额、净清算价值或保证金余额）。
- `locked`：针对未平仓订单和持仓预留的金额。
- `free`：可用于新订单的金额（`total - locked`）。

不变式 `total == locked + free` 在币种精度下必须始终成立。

Python 的 `AccountBalance(total, locked, free)` 构造函数要求一次性提供全部三个字段。用 Rust 编写的适配器代码另有两个派生构造函数，能集中强制执行该不变式；当交易场所只报告三个数值中的两个时，应优先使用它们，而非 `AccountBalance::new`：

| Rust 辅助方法                               | 何时使用                                                  |
| --------------------------------------- | ----------------------------------------------------- |
| `AccountBalance::from_total_and_locked` | 交易场所报告 total 和 locked；`free` 被推导并钳制到 `[0, total]` 区间。 |
| `AccountBalance::from_total_and_free`   | 交易场所报告 total 和 free；`locked` 被推导并钳制。                  |
| `AccountBalance::new`                   | 三个数值都已知且一致（测试、透传场景）。                                  |

当 `total >= 0` 时，这些辅助方法会把推导出的字段钳制到 `[0, total]` 区间，因此交易场所舍入造成的瞬时溢出绝不会让账户陷入损坏状态。

## 保证金作用域 (Margin scopes)

一个 `MarginBalance` 有四个字段：`initial`、`maintenance`、`currency`，以及一个用于选择两种作用域之一的 `Optional[InstrumentId]`。

### 单金融工具作用域 (Per-instrument scope)

`MarginBalance.instrument_id` 被设置为一个具体的金融工具。在以下情况使用它：

- 逐仓保证金（每个持仓单独的抵押品）。
- 回测或计算得出的保证金，此时 `AccountsManager` 会按金融工具从未平仓订单和持仓本地推导保证金。

### 账户级作用域 (Account-wide scope)

`MarginBalance.instrument_id` 为 `None`。该条目以其 `currency`（抵押品币种）为键。用于那些按每种抵押品币种报告单一聚合值的全仓（cross-margin）交易场所。一个交易场所可能发出一个账户级条目（单抵押品全仓），也可能发出多个（每种抵押品币各一个）。

两种作用域在同一个 `MarginAccount` 上以各自独立的内部存储共存。一个 `AccountState` 事件可以携带任意一种或两种作用域的条目，而 `MarginAccount.apply()` 会根据 `instrument_id` 是否被设置，把每个条目路由到正确的存储。

:::note
`MarginAccount.apply()` 会用传入事件**替换**两个存储。它不会与先前的状态合并。发出部分快照的适配器必须在每次更新时都包含所有当前有效的保证金条目，否则这些条目会被丢弃，直到下一次完整快照到来。余额列表同样会被替换。
:::

## 策略查询 API (Strategy query API)

使用与交易场所报告形态相匹配的查询。如果交易场所按金融工具报告保证金，就用 `InstrumentId` 查询。如果它按账户级报告保证金，就用 `Currency` 查询。

| 你想要的数值的作用域   | 使用                                                                                              |
| ------------ | ----------------------------------------------------------------------------------------------- |
| 单金融工具保证金（逐仓） | `margin(id)` / `margin_init(id)` / `margin_maint(id)`                                           |
| 单一抵押品的账户级保证金 | `margin_for_currency(ccy)` / `margin_init_for_currency(ccy)` / `margin_maint_for_currency(ccy)` |
| 跨两种作用域的合计总额  | `total_margin_init(ccy)` / `total_margin_maint(ccy)`                                            |

点查询在条目不存在时返回 `None`；总额查询始终返回一个 `Money`（若无匹配则为该币种的零值）。

:::note
下面列出的名称是 `MarginAccount` 上的 Python / Cython API。使用 `nautilus-model` crate 的 Rust 策略调用的是 `account_margin(&currency)`、`account_initial_margin(&currency)`、`account_maintenance_margin(&currency)`、`total_initial_margin(currency)` 和 `total_maintenance_margin(currency)`：同样按 `Option<InstrumentId>` 拆分，只是方法名不同。
:::

### 单金融工具查询（`MarginAccount`）

- `margin(instrument_id) -> MarginBalance | None`
- `margin_init(instrument_id) -> Money | None`
- `margin_maint(instrument_id) -> Money | None`
- `margins() -> dict[InstrumentId, MarginBalance]`（所有单金融工具条目）
- `margins_init() -> dict[InstrumentId, Money]`
- `margins_maint() -> dict[InstrumentId, Money]`

这些方法只能看到单金融工具存储。在全仓交易场所上，它们会返回空字典或 `None`。此时请使用下面的账户级查询。

### 账户级查询（`MarginAccount`）

- `margin_for_currency(currency) -> MarginBalance | None`
- `margin_init_for_currency(currency) -> Money | None`
- `margin_maint_for_currency(currency) -> Money | None`
- `account_margins() -> dict[Currency, MarginBalance]`（所有账户级条目）
- `account_margins_init() -> dict[Currency, Money]`
- `account_margins_maint() -> dict[Currency, Money]`

### 合计总额（`MarginAccount`）

对于给定币种，下列方法会对单金融工具条目和账户级条目求和：

- `total_margin_init(currency) -> Money`
- `total_margin_maint(currency) -> Money`

当策略所交易的交易场所可能同时出现两种作用域时（例如逐仓持仓与全仓抵押品并存），这些方法非常有用。

### 清除账户级条目

- `clear_account_margin(currency)` 会移除给定抵押品币种的账户级条目，并触发一次余额重算。针对单金融工具条目的对应方法是 `clear_margin(instrument_id)`。

这些是系统方法；适配器代码通过 `MarginAccount.apply()` 隐式调用它们。策略不应需要直接使用它们。

### 组合层查询 (Portfolio-level queries)

保证金查询：

- `portfolio.margins_init(venue=..., account_id=...) -> dict[InstrumentId, Money]`
- `portfolio.margins_maint(venue=..., account_id=...) -> dict[InstrumentId, Money]`

这两个方法镜像了 `MarginAccount.margins_init` / `margins_maint`，且只返回单金融工具条目。对于全仓交易场所上的账户级数据，请通过 `portfolio.account(venue).margin_init_for_currency(ccy)` 直接查询账户。

盈亏（PnL）、敞口、按市价计值（mark-to-market）和净值查询都接受 `venue` 以及一个可选的 `account_id`，以便对多账户交易场所进行作用域限定：

- `portfolio.unrealized_pnls(venue=..., account_id=...) -> dict[Currency, Money]`
- `portfolio.realized_pnls(venue=..., account_id=...) -> dict[Currency, Money]`
- `portfolio.total_pnls(venue=..., account_id=...) -> dict[Currency, Money]`
- `portfolio.net_exposures(venue=..., account_id=...) -> dict[Currency, Money]`
- `portfolio.mark_values(venue=..., account_id=...) -> dict[Currency, Money]`
- `portfolio.equity(venue=..., account_id=...) -> dict[Currency, Money]`
- `portfolio.missing_price_instruments(venue) -> list[InstrumentId]`

关于净值公式、价格回退链、基准币种换算行为，以及只告警一次的缺失价格追踪器，请参阅 [组合指南](portfolio.md#equity-and-mark-to-market)。

### 实操示例 (Worked examples)

单抵押品全仓（一个账户级条目）：

```python
usdc_margin = margin_account.margin_init_for_currency(USDC)
usdc_total = margin_account.total_margin_init(USDC)
```

逐币种全仓（每种抵押品币各一个条目）：

```python
for ccy, margin_balance in margin_account.account_margins().items():
    print(ccy, margin_balance.initial, margin_balance.maintenance)
```

## 保证金模型 (Margin models)

NautilusTrader 为计算路径（回测，以及为对账而以 `calculate_account_state=True` 运行的实盘策略）提供了灵活的保证金计算模型。来自交易场所的已报告保证金会直接流入 `_account_margins` 或 `_margins`，不经过任何模型。

### 概览

不同交易场所对待杠杆的方式各不相同：

- **传统券商**（Interactive Brokers、TD Ameritrade）：保证金百分比固定，与杠杆无关。
- **加密货币交易所**（Binance 等）：杠杆可能会降低保证金要求。

两个内置模型都使用金融工具的 `margin_init` 和 `margin_maint` 字段，按名义价值的百分比来计算保证金。它们的唯一区别在于杠杆是否会降低预留额。对于具有真正逐合约固定保证金的交易场所（CME / ICE），请设置 `instrument.margin_init` 和 `margin_maint`，使百分比能还原出期望的美元金额，或者实现一个 [自定义模型](#custom-models)。

### HEDGING 模式下的净额计算

在 `OmsType.HEDGING` 下，每一笔成交都会开立自己的 `Position`，因此一个账户可以为同一金融工具持有多个未平仓子持仓。账户管理器会按 `ts_opened` 顺序把这些子持仓净额合并到一个假想的 NETTING 持仓上，然后对得到的净有符号数量和平均开仓价运行一次保证金模型。

该重放过程遵循与 `Position.apply` 相同的规则：同方向成交产生数量加权平均开仓价，反方向成交按现有均价部分平仓，而穿越零点的成交会让剩余部分采用翻转成交的价格。共享同一 `ts_opened` 的子持仓按 `(ts_opened, position_id)` 顺序折叠，从而使结果不依赖于缓存的迭代顺序。

对于相同的成交序列，HEDGING 账户和 NETTING 账户计算出的维持保证金相同；该要求随净经济敞口而缩放。

### 可用的模型 (Available models)

#### `StandardMarginModel`

使用固定百分比、不除以杠杆，与传统券商的行为一致。

```python
# Fixed percentages - leverage ignored
margin = notional * instrument.margin_init
```

- 初始保证金：`notional_value * instrument.margin_init`
- 维持保证金：`notional_value * instrument.margin_maint`

**适用场景**：传统券商（Interactive Brokers）、有固定保证金要求的外汇券商。

#### `LeveragedMarginModel`

将保证金要求除以杠杆。

```python
# Leverage reduces margin requirements
adjusted_notional = notional / leverage
margin = adjusted_notional * instrument.margin_init
```

- 初始保证金：`(notional_value / leverage) * instrument.margin_init`
- 维持保证金：`(notional_value / leverage) * instrument.margin_maint`

**适用场景**：随杠杆降低保证金的加密货币交易所，以及杠杆会影响保证金要求的交易场所。

### 默认行为 (Default behavior)

`MarginAccount` 默认使用 `LeveragedMarginModel`。可通过代码覆盖：

```python
from nautilus_trader.backtest.models import LeveragedMarginModel
from nautilus_trader.backtest.models import StandardMarginModel
from nautilus_trader.test_kit.stubs.execution import TestExecStubs

account = TestExecStubs.margin_account()

# Traditional broker behavior
account.set_margin_model(StandardMarginModel())

# Or the leveraged model (default)
account.set_margin_model(LeveragedMarginModel())
```

### 实操示例：EUR/USD

- **金融工具**：EUR/USD
- **数量**：100,000 EUR
- **价格**：1.10000
- **名义价值**：$110,000
- **杠杆**：50x
- **`instrument.margin_init`**：3%

| 模型        | 计算                     | 结果     | 百分比   |
| --------- | ---------------------- | ------ | ----- |
| Standard  | $110,000 × 0.03        | $3,300 | 3.00% |
| Leveraged | ($110,000 ÷ 50) × 0.03 | $66    | 0.06% |

在一个 $10,000 的账户上：标准模型会阻止这笔交易，而杠杆模型则允许它。

### 自定义模型 (Custom models)

继承 `MarginModel`，并通过 `MarginModelConfig` 接收配置：

```python
from decimal import Decimal

from nautilus_trader.backtest.config import MarginModelConfig
from nautilus_trader.backtest.models import MarginModel
from nautilus_trader.model.objects import Money


class RiskAdjustedMarginModel(MarginModel):
    def __init__(self, config: MarginModelConfig) -> None:
        self.risk_multiplier = Decimal(str(config.config.get("risk_multiplier", 1.0)))
        self.use_leverage = config.config.get("use_leverage", False)

    def calculate_margin_init(
        self, instrument, quantity, price, leverage, use_quote_for_inverse=False
    ):
        notional = instrument.notional_value(quantity, price, use_quote_for_inverse)

        if self.use_leverage:
            adjusted = notional.as_decimal() / leverage
        else:
            adjusted = notional.as_decimal()

        margin = adjusted * instrument.margin_init * self.risk_multiplier
        return Money(margin, instrument.quote_currency)

    def calculate_margin_maint(
        self, instrument, side, quantity, price, leverage, use_quote_for_inverse=False
    ):
        return self.calculate_margin_init(
            instrument, quantity, price, leverage, use_quote_for_inverse
        )
```

关于通过 `BacktestVenueConfig` 和 `MarginModelConfig` 对保证金模型进行回测级配置，请参阅 [回测](backtesting.md#margin-models) 的保证金模型一节。

## 适配器约定 (Adapter convention)

实盘适配器把交易场所的响应转换为 `AccountBalance` 和 `MarginBalance` 实例。适配器作者必须遵循的约定如下：

### 构建 `AccountBalance`

优先使用派生辅助方法，从而集中强制执行钳制以及 `total == locked + free` 不变式。手工计算三个字段并把它们传给 `AccountBalance::new` 只适用于三个数值都已是权威值的透传路径（例如测试）。

### 构建 `MarginBalance`

选择与交易场所所报告内容相匹配的作用域：

| 交易场所报告        | 作用域   | 发出方式                                           |
| ------------- | ----- | ---------------------------------------------- |
| 单金融工具（逐仓持仓）   | 单金融工具 | `MarginBalance::new(initial, maint, Some(id))` |
| 按抵押品的单一聚合（全仓） | 账户级   | `MarginBalance::new(initial, maint, None)`     |
| 多个聚合，每种抵押品一个  | 账户级   | 每种币种一个 `MarginBalance`，且 `instrument_id=None`  |

:::note
不使用合成的 `ACCOUNT.{VENUE}` 或 `ACCOUNT-{COIN}.{VENUE}` 形式的 `InstrumentId` 占位符。账户级条目携带 `instrument_id=None`，并以 `currency` 为键。
:::

## 相关指南 (Related guides)

- [回测](backtesting.md)：初始余额、`MarginModelConfig`，以及回测专用的账户设置。
- [组合](portfolio.md)：组合层的盈亏、敞口和币种换算。
- [持仓](positions.md)：持仓生命周期、聚合和盈亏。
- [适配器](adapters.md)：适配器作者的要求和最佳实践。
