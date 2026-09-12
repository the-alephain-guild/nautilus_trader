# 报告 (Reports)

本指南介绍 `ReportProvider` 类提供的投资组合分析和报告功能，以及这些报告如何用于盈亏 (PnL) 核算和回测 (backtest) 运行后分析。

## 概述

NautilusTrader 中的 `ReportProvider` 类从交易数据生成结构化分析报告，将原始订单 (order)、成交 (fill)、持仓 (position) 和账户 (account) 状态转换为 pandas DataFrame 以供分析和可视化。这些报告可帮助你评估策略 (strategy) 绩效、分析执行质量并核验盈亏核算。

报告可以通过两种方式生成：

- **Trader 辅助方法**（推荐）：便捷方法，如 `trader.generate_orders_report()`。
- **直接使用 ReportProvider**：可对数据选择和过滤进行更精细的控制。

报告在回测和实盘交易环境中提供一致的分析能力，支持可靠的绩效评估和策略比较。

## 可用报告

`ReportProvider` 类提供多个静态方法来从交易数据生成报告。每个报告返回一个具有特定列和索引的 pandas DataFrame，便于分析。

### 订单报告

生成所有订单的完整视图：

```python
# 使用 Trader 辅助方法（推荐）
orders_report = trader.generate_orders_report()

# 或直接使用 ReportProvider
from nautilus_trader.analysis import ReportProvider

orders = cache.orders()
orders_report = ReportProvider.generate_orders_report(orders)
```

**返回 `pd.DataFrame`，关键列包括：**

| 列                 | 描述                      |
| ----------------- | ----------------------- |
| `client_order_id` | 索引 - 唯一订单标识符。           |
| `instrument_id`   | 交易金融工具 (instrument)。    |
| `strategy_id`     | 创建该订单的策略。               |
| `trader_id`       | Trader 标识符。             |
| `account_id`      | 账户标识符（如已分配）。            |
| `venue_order_id`  | 交易场所分配的订单 ID（如已被接受）。    |
| `side`            | BUY 或 SELL。             |
| `type`            | MARKET、LIMIT 等。         |
| `status`          | 当前订单状态。                 |
| `quantity`        | 原始订单数量（字符串）。            |
| `filled_qty`      | 已成交数量（字符串）。             |
| `price`           | 限价（取决于订单类型）。            |
| `avg_px`          | 平均成交价格（如已成交）。           |
| `time_in_force`   | 有效期 (time-in-force) 指令。 |
| `ts_init`         | 订单初始化时间戳（Unix 纳秒）。      |
| `ts_last`         | 最后更新时间戳（Unix 纳秒）。       |

附加列因订单类型而异（例如 stop 订单的 `trigger_price`、GTD 订单的 `expire_time`）。完整字段列表请参阅 `Order.to_dict()`。

### 订单成交报告

提供已成交订单的汇总（每个订单一行）：

```python
# 使用 Trader 辅助方法（推荐）
fills_report = trader.generate_order_fills_report()

# 或直接使用 ReportProvider
orders = cache.orders()
fills_report = ReportProvider.generate_order_fills_report(orders)
```

此报告仅包含 `filled_qty > 0` 的订单，列与订单报告相同，但仅筛选已执行的订单。注意此报告中 `ts_init` 和 `ts_last` 已转换为 datetime 对象，便于分析。

### 成交明细报告

详细记录每笔成交事件（每笔成交一行）：

```python
# 使用 Trader 辅助方法（推荐）
fills_report = trader.generate_fills_report()

# 或直接使用 ReportProvider
orders = cache.orders()
fills_report = ReportProvider.generate_fills_report(orders)
```

**返回 `pd.DataFrame`，关键列包括：**

| 列                 | 描述                     |
| ----------------- | ---------------------- |
| `client_order_id` | 索引 - 订单标识符。            |
| `trade_id`        | 唯一交易/成交标识符。            |
| `venue_order_id`  | 交易场所 (venue) 分配的订单 ID。 |
| `instrument_id`   | 交易金融工具。                |
| `strategy_id`     | 创建该订单的策略。              |
| `account_id`      | 账户标识符。                 |
| `position_id`     | 关联的持仓 ID（如适用）。         |
| `order_side`      | BUY 或 SELL。            |
| `order_type`      | 订单类型（MARKET、LIMIT 等）。  |
| `last_px`         | 成交执行价格（字符串）。           |
| `last_qty`        | 成交执行数量（字符串）。           |
| `currency`        | 成交币种。                  |
| `liquidity_side`  | MAKER 或 TAKER。         |
| `commission`      | 手续费金额和币种。              |
| `ts_event`        | 成交时间戳（datetime）。       |
| `ts_init`         | 初始化时间戳（datetime）。      |

完整字段列表请参阅 `OrderFilled.to_dict()`。

### 持仓报告

包含快照在内的持仓分析：

```python
# 使用 Trader 辅助方法（推荐）
# 自动包含 NETTING OMS 的快照
positions_report = trader.generate_positions_report()

# 或直接使用 ReportProvider
positions = cache.positions()
snapshots = cache.position_snapshots()  # 用于 NETTING OMS
positions_report = ReportProvider.generate_positions_report(
    positions=positions, snapshots=snapshots
)
```

**返回 `pd.DataFrame`，关键列包括：**

| 列                  | 描述                       |
| ------------------ | ------------------------ |
| `position_id`      | 索引 - 唯一持仓标识符。            |
| `instrument_id`    | 交易金融工具。                  |
| `strategy_id`      | 管理该持仓的策略。                |
| `trader_id`        | Trader 标识符。              |
| `account_id`       | 账户标识符。                   |
| `opening_order_id` | 开仓的订单 ID。                |
| `closing_order_id` | 平仓的订单 ID。                |
| `entry`            | 入场方向（BUY 或 SELL）。        |
| `side`             | 持仓方向（LONG、SHORT 或 FLAT）。 |
| `quantity`         | 当前持仓规模。                  |
| `peak_qty`         | 达到的最大规模。                 |
| `avg_px_open`      | 平均入场价格。                  |
| `avg_px_close`     | 平均出场价格（如已平仓）。            |
| `commissions`      | 已付手续费列表。                 |
| `realized_pnl`     | 已实现盈亏。                   |
| `realized_return`  | 收益率 (return) 百分比。        |
| `ts_init`          | 持仓初始化时间戳。                |
| `ts_opened`        | 开仓时间戳（datetime）。         |
| `ts_last`          | 最后更新时间戳。                 |
| `ts_closed`        | 平仓时间戳（datetime 或 NA）。    |
| `duration_ns`      | 持仓持续时间（纳秒）。              |
| `is_snapshot`      | 是否为历史快照。                 |

### 账户报告

跟踪账户余额 (balance) 和保证金随时间的变化：

```python
# 使用 Trader 辅助方法（推荐）
# 需要 venue 参数
from nautilus_trader.model.identifiers import Venue

venue = Venue("BINANCE")
account_report = trader.generate_account_report(venue)

# 或直接使用 ReportProvider
account = cache.account(account_id)
account_report = ReportProvider.generate_account_report(account)
```

**返回 `pd.DataFrame`，列包括：**

| 列               | 描述                   |
| --------------- | -------------------- |
| `ts_event`      | 索引 - 账户状态变更时间戳。      |
| `account_id`    | 账户标识符。               |
| `account_type`  | 账户类型（如 SPOT、MARGIN）。 |
| `base_currency` | 账户基础货币。              |
| `total`         | 总余额（字符串）。            |
| `free`          | 可用余额（字符串）。           |
| `locked`        | 订单锁定的余额（字符串）。        |
| `currency`      | 余额币种。                |
| `reported`      | 余额是否由交易场所报告。         |
| `margins`       | 保证金信息（列表，如适用）。       |
| `info`          | 交易场所特定的附加信息。         |

每一行代表一条余额条目；含多种货币的账户在每个账户状态事件下会产生多行。

## 盈亏核算注意事项

准确的盈亏核算需要仔细考虑以下几个因素：

### 基于持仓的盈亏

- **已实现盈亏**：在持仓部分或全部平仓时计算。
- **未实现盈亏**：使用当前价格按市值计算。
- **手续费影响**：仅在以结算货币计价时纳入。

:::warning
盈亏计算取决于 OMS 类型。在 `NETTING` OMS 中，持仓快照在持仓重新开启时保留历史盈亏。请始终在报告中包含快照，以确保总盈亏计算的准确性。在 `HEDGING` OMS 中，不使用快照，因为每个持仓都有唯一 ID，且不会重新开启。
:::

### 多币种核算

处理多币种时：

- 每个持仓以其结算货币跟踪盈亏。
- 投资组合汇总需要进行货币转换。
- 手续费币种可能与结算货币不同。

```python
# 跨持仓访问盈亏
for position in positions:
    realized = position.realized_pnl  # 以结算货币计
    unrealized = position.unrealized_pnl(last_price)

    # 处理多币种汇总（示意）
    # 注意：货币转换需要用户提供的汇率
    if position.settlement_currency != base_currency:
        # 从数据源获取转换汇率
        # rate = get_exchange_rate(position.settlement_currency, base_currency)
        # realized_converted = realized.as_double() * rate
        pass
```

### 快照注意事项

对于 `NETTING` OMS：

```python
from nautilus_trader.model.objects import Money

# 包含快照以获得完整盈亏（按币种）
pnl_by_currency = {}

# 添加当前持仓的盈亏
for position in cache.positions(instrument_id=instrument_id):
    if position.realized_pnl:
        currency = position.realized_pnl.currency
        if currency not in pnl_by_currency:
            pnl_by_currency[currency] = 0.0
        pnl_by_currency[currency] += position.realized_pnl.as_double()

# 添加历史快照的盈亏
for snapshot in cache.position_snapshots(instrument_id=instrument_id):
    if snapshot.realized_pnl:
        currency = snapshot.realized_pnl.currency
        if currency not in pnl_by_currency:
            pnl_by_currency[currency] = 0.0
        pnl_by_currency[currency] += snapshot.realized_pnl.as_double()

# 为每种货币创建 Money 对象
total_pnls = [Money(amount, currency) for currency, amount in pnl_by_currency.items()]
```

## 回测运行后分析

回测完成后，可以通过各种报告和投资组合分析器进行分析。

### 访问回测结果

```python
# 回测运行后
engine.run(start=start_time, end=end_time)

# 使用 Trader 辅助方法生成报告
orders_report = engine.trader.generate_orders_report()
positions_report = engine.trader.generate_positions_report()
fills_report = engine.trader.generate_fills_report()

# 或直接访问数据进行自定义分析
orders = engine.cache.orders()
positions = engine.cache.positions()
snapshots = engine.cache.position_snapshots()
```

### 投资组合统计 (statistics)

投资组合分析器提供绩效指标 (metric)：

```python
# 访问投资组合分析器
portfolio = engine.portfolio

# 获取不同类别的统计数据
stats_pnls = portfolio.analyzer.get_performance_stats_pnls()
stats_returns = portfolio.analyzer.get_performance_stats_returns()
stats_general = portfolio.analyzer.get_performance_stats_general()
```

:::info
有关可用统计数据和创建自定义指标的详细信息，请参阅[投资组合指南](portfolio.md#投资组合统计)。该指南涵盖：

- 内置统计类别（基于盈亏、收益率、持仓、订单）。
- 使用 `PortfolioStatistic` 创建自定义统计。
- 注册和使用自定义指标。

:::

### 可视化

NautilusTrader 通过 Plotly 提供交互式分析报表 (tearsheet) 和图表：

```python
from nautilus_trader.analysis import create_tearsheet

# 回测运行后
engine.run()

# 生成交互式 HTML 分析报表
create_tearsheet(engine, output_path="tearsheet.html")
```

这将创建一个交互式 HTML 报告，包含：

- 权益 (equity) 曲线
- 回撤 (drawdown) 分析
- 月度收益率热力图
- 绩效统计表
- 收益率分布

要进行更精细的控制，可生成单独的图表：

```python
from nautilus_trader.analysis import create_equity_curve

returns = engine.portfolio.analyzer.returns()
fig = create_equity_curve(returns, title="My Strategy Equity")
fig.show()  # 在浏览器中显示
fig.write_image("equity.png")  # 导出为 PNG（需要 kaleido）
```

安装可视化依赖：

```bash
uv pip install "nautilus_trader[visualization]"
```

## 报告生成模式

:::note 报告生成时机对比

| 场景          | 报告生成时机                 | 数据完整性          |
| ----------- | ---------------------- | -------------- |
| **回测完成后**   | `engine.run()` 返回后立即生成 | 完整（所有历史数据已处理）  |
| **实盘交易中**   | 随时可生成（如定时调用）           | 部分（仅已完成的持仓和订单） |
| **实盘交易结束后** | `node.stop()` 后生成      | 完整（含当天所有数据）    |

在实盘交易**进行中**生成持仓报告时，未平仓持仓会以 `LONG`/`SHORT` 状态出现，`avg_px_close` 和 `ts_closed` 为 NA。若需评估未平仓持仓的实时盈亏，使用 `position.unrealized_pnl(current_price)`。
:::

### 实盘交易

在实盘交易期间，定期生成报告：

```python
import pandas as pd


class ReportingActor(Actor):
    def on_start(self):
        # 设置定期报告调度
        self.clock.set_timer(
            name="generate_reports",
            interval=pd.Timedelta(minutes=30),
            callback=self.generate_reports,
        )

    def generate_reports(self, event):
        # 生成并记录报告
        positions_report = self.trader.generate_positions_report()

        # 保存或传输报告
        positions_report.to_csv(f"positions_{event.ts_event}.csv")
```

### 绩效分析

用于回测分析：

```python
import pandas as pd

# 运行回测
engine.run(start=start_time, end=end_time)

# 收集结果
positions_closed = engine.cache.positions_closed()
stats_pnls = engine.portfolio.analyzer.get_performance_stats_pnls()
stats_returns = engine.portfolio.analyzer.get_performance_stats_returns()
stats_general = engine.portfolio.analyzer.get_performance_stats_general()

# 创建汇总字典
results = {
    "total_positions": len(positions_closed),
    "pnl_total": stats_pnls.get("PnL (total)"),
    "sharpe_ratio": stats_returns.get("Sharpe Ratio (252 days)"),
    "profit_factor": stats_general.get("Profit Factor"),
    "win_rate": stats_general.get("Win Rate"),
}

# 显示结果
results_df = pd.DataFrame([results])
print(results_df.T)  # 转置以纵向显示
```

:::info
报告从内存数据结构生成。对于大规模分析或长时间运行的系统，建议将报告持久化到数据库以实现高效查询。有关持久化选项，请参阅 [Cache 指南](cache.md)。
:::

## 与其他组件的集成

`ReportProvider` 与以下系统组件协同工作：

- **Cache**：报告的所有交易数据（订单、持仓、账户）的来源。
- **Portfolio**：使用报告进行绩效分析和指标计算。
- **BacktestEngine**：利用报告进行运行后分析和可视化。
- **持仓快照**：对于 `NETTING` OMS 中准确的盈亏报告至关重要。

## 总结

`ReportProvider` 将订单、成交、持仓和账户状态生成为结构化的 DataFrame，用于分析和可视化。要在 `NETTING` OMS 中获得准确的总盈亏，请在生成报告时包含持仓快照。

## 相关指南

- [可视化](visualization.md) - 从回测结果创建交互式分析报表和图表。
- [投资组合](portfolio.md) - 投资组合统计和绩效指标。
- [回测](backtesting.md) - 运行生成报告的回测。
- [Cache](cache.md) - 存储报告数据的缓存系统。
