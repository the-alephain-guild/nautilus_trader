# 投资组合 (Portfolio)

`Portfolio` 是管理和跟踪交易节点或回测 (backtest) 中所有活跃策略 (strategy) 持仓 (position) 的核心枢纽。
它整合来自多个金融工具 (instrument) 的持仓数据，提供持仓、风险敞口和整体表现的统一视图。

## 货币转换 (Currency Conversion)

`Portfolio` 支持对盈亏和敞口计算进行自动货币转换，
允许你以首选货币查看结果。当跨多个结算货币不同的金融工具交易，
或管理基础货币不同的多个账户时，此功能尤为实用。

### 支持的转换

以下投资组合查询方法支持货币转换：

- `realized_pnl()` / `realized_pnls()` - 将已实现盈亏转换为目标货币。
- `unrealized_pnl()` / `unrealized_pnls()` - 将未实现盈亏转换为目标货币。
- `total_pnl()` / `total_pnls()` - 将总盈亏转换为目标货币。
- `net_exposure()` / `net_exposures()` - 将净风险敞口转换为目标货币。

所有方法均接受可选的 `target_currency` 参数以指定所需的输出货币。

### 单账户行为

查询单个账户且未指定 `target_currency` 时，`Portfolio`
会自动将值转换为该账户的基础货币：

```python
# 以账户基础货币（如 USD）返回敞口
exposure = portfolio.net_exposures(venue=BINANCE, account_id=account_id)
```

### 多账户行为

同时查询多个账户时，行为取决于查询所有金融工具（`net_exposures()`）
还是单个金融工具（`net_exposure()`）：

**`net_exposures()`（所有金融工具）：**

- **相同基础货币**：自动转换为公共基础货币。
- **不同基础货币**：返回包含多种货币的字典，每种货币已转换为对应账户的基础货币。
  提供 `target_currency` 可获得单货币结果。

**`net_exposure()`（跨账户的单个金融工具）：**

- **不同基础货币**：除非提供 `target_currency`，否则返回 `None`。

```python
# 场景 1：多账户，均以 USD 为基础货币
exposures = portfolio.net_exposures(venue=BINANCE)
# 返回 {USD: Money(...)}

# 场景 2：多账户，基础货币不同（USD 和 EUR）
exposures = portfolio.net_exposures(venue=BINANCE)
# 返回 {USD: Money(...), EUR: Money(...)}

# 强制跨账户使用单一货币
exposures = portfolio.net_exposures(venue=BINANCE, target_currency=USD)
# 返回 {USD: Money(...)}
```

### 转换失败处理

当提供 `target_currency` 但货币转换失败时，行为取决于方法类型：

- **单值方法**（`realized_pnl`、`unrealized_pnl`、`total_pnl`、`net_exposure`）：
  返回 `None` 并记录错误，以防止返回不正确的值。
- **返回字典的方法**（`realized_pnls`、`unrealized_pnls`、`total_pnls`、`net_exposures`）：
  跳过转换失败的金融工具，但返回转换成功的结果。

:::warning
使用 `target_currency` 进行跨币种聚合时，请确保汇率数据可用。
:::

### 转换价格类型

将敞口转换为目标货币时，`Portfolio` 根据持仓构成使用不同的价格类型：

- **全多头持仓**：使用 `BID` 价格（对多头敞口保守估计）。
- **全空头持仓**：使用 `ASK` 价格（对空头敞口保守估计）。
- **多空混合持仓**：使用 `MID` 价格（多空并存时取中间价）。

这确保转换反映了真实市场条件：多头持仓以买价平仓，空头持仓以卖价回补。
对于混合持仓，中间价提供中性估值。

如果在投资组合配置中启用了 `use_mark_xrates`，则对于混合持仓和一般转换，
`MARK` 价格将替代 `MID` 价格。

## 权益与盯市估值 (Equity and Mark-to-market)

`Portfolio` 提供三个拉取式 (pull-style) 查询，用于持续对投资组合估值。
每个查询都返回按币种区分的结果，键为相应账户的基础货币或原生结算货币。

| 方法                                 | 返回内容                  |
| ---------------------------------- | --------------------- |
| `mark_values(venue, account_id)`   | 未平仓持仓的带符号盯市 (MTM) 总额。 |
| `equity(venue, account_id)`        | 结合余额和持仓估值的总权益。        |
| `missing_price_instruments(venue)` | 当前被标记为无法定价的金融工具。      |

多头贡献正名义价值，空头贡献负名义价值。平仓持仓 (flat positions) 将被跳过。

### 权益公式

权益结合账户余额与未平仓持仓估值，第二项根据账户类型而有所不同：

- **现金账户和投注账户**：`balances_total + Σ mark_value(未平仓持仓)`。
- **保证金账户**：`balances_total + Σ unrealized_pnl(未平仓持仓)`。

现金和投注账户路径内部使用 `mark_values()`。保证金账户路径使用与
`unrealized_pnls()` 相同的、带缓存的未实现盈亏计算管线。

### 价格回退

估值会按以下顺序向 `Cache` 请求价格，遇到第一个匹配项即停止：

1. 盯市价格 (mark price)，前提是 `PortfolioConfig` 中 `use_mark_prices=true` 且缓存中存在盯市价格。
2. 与方向匹配的报价：多头用 `BID`，空头用 `ASK`。
3. 最新成交价 (last trade price)。
4. 最近缓存的 K 线收盘价（在 `bar_updates=true` 时填充）。

如果上述四项都无法得到价格，该持仓将进入无价格追踪器 (missing-price tracker)，
并在求和时被跳过。

### 基础货币转换

当 `convert_to_account_base_currency=true`（默认值）且账户设置了 `base_currency` 时，
结算货币的值会使用来自 `Cache.get_xrate()` 的 `MID` 汇率转换为基础货币。
启用 `use_mark_xrates=true` 时，会优先使用来自 `Cache.get_mark_xrate()` 的缓存盯市汇率，
若不可用则回退到 `MID`。此时输出字典只有一个键，与基础货币匹配。

当 `convert_to_account_base_currency=false`，或账户未设置 `base_currency` 时，
结果以每个持仓的原生结算货币为键，且不进行汇率转换。

如果某个所需转换缺少汇率数据，该持仓将被视为无法定价，并通过无价格追踪器标记，
而不是悄悄地以 1.0 的汇率估值。

### 无价格追踪

该追踪器是按场所 (venue) 维护的金融工具 ID 集合，记录在最近一次
`mark_values()` 或 `equity()` 调用中无法定价的金融工具。它有两个可观察的行为：

- 当某金融工具从可定价转为不可定价时，仅触发一次警告日志，而非在之后每次调用时都触发。
  下一次转回可定价状态会清除该条目，因此未来再次掉价时会重新告警。
- 当某场所变为平仓状态（无未平仓持仓）时，其追踪器条目会被清除，以免过时的金融工具
  仍被标记。

调用 `missing_price_instruments(venue)` 可检查当前集合。

:::tip
如果 `equity()` 的结果低于你的预期，请在排查计算逻辑之前先检查
`missing_price_instruments(venue)`。某个金融工具的报价、成交和 K 线数据全部为空，
是导致估值出现静默缺口的最常见原因。
:::

### 场所与账户范围

`mark_values` 和 `equity` 接受可选的 `account_id`，用于将聚合范围限定到单个账户。
当 `account_id=None` 时，结果会聚合该场所上的每个账户。

无价格追踪器是按场所维护的。`missing_price_instruments` 只接受一个场所参数，
而带账户过滤的 `mark_values(venue, account_id)` 调用不会清除场所条目，
因此同一场所上其他账户触发的标记得以保留。

## 投资组合统计

NautilusTrader 提供了多种[内置投资组合统计指标](https://github.com/nautechsystems/nautilus_trader/tree/develop/crates/analysis/src/statistics)，
用于分析回测和实盘交易中投资组合的表现。

统计指标大致分为以下几类：

- 基于盈亏 (PnL) 的统计（按币种）
- 基于收益率 (Returns) 的统计
- 基于持仓的统计
- 基于订单 (order) 的统计

你也可以随时调用交易者的 `PortfolioAnalyzer` 来计算统计指标，
包括在回测*进行中*或实盘交易期间。

## 自定义统计

可以通过继承 `PortfolioStatistic` 基类并实现任意 `calculate_` 方法来定义自定义投资组合统计指标。

例如，以下是内置 `WinRate`（胜率）统计的实现：

```python
import pandas as pd
from typing import Any
from nautilus_trader.analysis.statistic import PortfolioStatistic


class WinRate(PortfolioStatistic):
    """
    根据已实现盈亏 (realized PnL) 序列计算胜率。
    """

    def calculate_from_realized_pnls(self, realized_pnls: pd.Series) -> Any | None:
        # 前置条件检查
        if realized_pnls is None or realized_pnls.empty:
            return 0.0

        # 计算统计指标
        winners = [x for x in realized_pnls if x > 0.0]
        losers = [x for x in realized_pnls if x <= 0.0]

        return len(winners) / float(max(1, (len(winners) + len(losers))))
```

:::tip 统计方法选择指南

`PortfolioStatistic` 基类提供四种数据源方法，对应不同的分析需求：

| 方法                                              | 数据来源               | 适用统计类型              |
| ----------------------------------------------- | ------------------ | ------------------- |
| `calculate_from_realized_pnls(pnls: pd.Series)` | 已实现盈亏序列（浮点数，按币种聚合） | 胜率、盈亏比、平均盈利/亏损、期望值  |
| `calculate_from_returns(returns: pd.Series)`    | 持仓收益率时间序列          | 夏普比率、最大回撤、年化收益率、波动率 |
| `calculate_from_positions(positions: list)`     | 原始持仓对象列表           | 持仓持续时间统计、持仓频率、多空比   |
| `calculate_from_orders(orders: list)`           | 原始订单对象列表           | 成交率、撤单率、订单执行质量      |

实现自定义统计时，只需重写用到的方法即可（可同时重写多个）。系统在回测结束后会调用所有已注册统计的全部 `calculate_from_*` 方法，并将结果显示在绩效报告中。
:::

然后可以将这些统计指标注册到交易者的 `PortfolioAnalyzer` 中。

```python
stat = WinRate()

# 注册到投资组合分析器
engine.portfolio.analyzer.register_statistic(stat)
```

有关所有可用方法，请参阅 [`PortfolioAnalyzer` API 参考](/docs/python-api-latest/analysis.html#nautilus_trader.analysis.analyzer.PortfolioAnalyzer)。

:::tip
请确保你的统计指标能够稳健地处理退化输入，例如 `None`、空序列或数据不足的情况。
对于未知/无法计算的值返回 `None`，或在语义上合理时返回合理的默认值（如 `0.0`，例如没有交易时的胜率）。
:::

## 收益率：持仓 vs 投资组合

分析器跟踪两个不同的收益率序列：

- **持仓收益率**（`analyzer.position_returns()`）以相对于平均开仓价的、考虑方向的价格收益率，
  衡量每个持仓的已实现收益。它反映金融工具在入场与出场之间的价格变动，
  与账户规模或杠杆无关。
- **投资组合收益率**（`analyzer.portfolio_returns()`）衡量账户总余额的每日百分比变化。
  在一个 $100,000 的账户上获得 $900 的收益，当天大约报告 0.9%。

当分析器拥有跨越至少两个不同日历日的账户状态历史时，它会自动计算投资组合收益率，
并将其用作统计、绩效报告 (tearsheet) 和月度收益率热力图的主序列。同一天内的多个快照
计为一天，因此仅有日内交易不会产生投资组合收益率。当投资组合收益率不可用时，
它会回退到持仓收益率。

便捷访问器 `analyzer.returns()` 会根据这一优先级进行解析：存在时返回投资组合收益率，
否则返回持仓收益率。

### 多币种账户

投资组合收益率需要单一币种的余额历史。当账户持有多种币种的余额时，分析器无法生成
单一收益率序列，会静默回退到持仓收益率。统计和绩效报告图表使用 `returns()`
所解析到的那个序列。

如果你需要为多币种账户计算投资组合层面的收益率，请在外部计算：先将余额转换为
公共币种，再计算百分比变化。

### 按场所计算

在回测引擎中，分析器按场所运行（`engine.pyx`）。每个场所的账户都会产生自己的
投资组合收益率序列。绩效报告会在所有已缓存账户之间聚合，从而为多场所回测
生成合并的收益率序列。

## 回测分析

回测运行结束后，引擎会将已实现盈亏、收益率、持仓和订单数据传递给每个已注册的统计指标。
所有输出结果随后将显示在绩效报告的 `Portfolio Performance` 标题下，分组如下：

- 已实现盈亏统计（按币种）
- 收益率统计（针对整个投资组合）
- 基于持仓和订单数据的综合统计（针对整个投资组合）

## 相关指南

- [持仓](positions.md) - 投资组合内的持仓跟踪。
- [报告](reports.md) - 生成投资组合分析报告。
- [可视化](visualization.md) - 可视化投资组合表现。
