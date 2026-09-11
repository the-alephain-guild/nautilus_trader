# 可视化 (Visualization)

NautilusTrader 提供交互式 HTML 分析报告页 (tearsheet)，通过基于 Plotly 构建的可扩展可视化系统分析回测 (backtest) 结果。你只需少量代码即可生成报告，并能添加自定义图表 (chart) 和主题。

## 概述

可视化系统由三部分组成：

1. **图表注册表 (Chart Registry)** - 解耦的图表定义，可通过自定义可视化进行扩展。
2. **主题系统 (Theme System)** - 使用内置和自定义主题实现一致的样式。
3. **配置 (Configuration)** - 声明式地指定渲染内容和显示方式。

所有可视化输出都是独立的 HTML 文件，可以在任何现代浏览器中查看、与利益相关方共享或归档以备将来参考。

:::note
可视化系统需要 `plotly>=6.3.1`。使用以下命令安装：

```bash
uv pip install "nautilus_trader[visualization]"
```

或

```bash
uv pip install "plotly>=6.3.1"
```

:::

## 分析报告页 (Tearsheet)

分析报告页是一份绩效报告，将多个图表和统计数据组合成单个交互式可视化。报告页在完成回测运行后生成，提供策略 (strategy) 绩效的即时视觉反馈。

### 快速开始

使用默认设置生成分析报告页：

```python
from nautilus_trader.analysis import create_tearsheet
from nautilus_trader.backtest.engine import BacktestEngine

# 运行回测后
engine.run()

# 生成分析报告页
create_tearsheet(
    engine=engine,
    output_path="backtest_results.html",
)
```

这会生成一个包含所有默认图表的 HTML 文件，使用浅色主题和自动布局。在浏览器中打开 `backtest_results.html` 即可查看交互式报告页。

### 自定义

控制显示哪些图表及其样式：

```python
from nautilus_trader.analysis import TearsheetConfig
from nautilus_trader.analysis import TearsheetDrawdownChart
from nautilus_trader.analysis import TearsheetEquityChart
from nautilus_trader.analysis import TearsheetRunInfoChart
from nautilus_trader.analysis import TearsheetStatsTableChart

config = TearsheetConfig(
    charts=[
        TearsheetRunInfoChart(),
        TearsheetStatsTableChart(),
        TearsheetEquityChart(),
        TearsheetDrawdownChart(),
    ],
    theme="nautilus_dark",
    height=2000,
)

create_tearsheet(
    engine=engine,
    output_path="custom_tearsheet.html",
    config=config,
)
```

### 货币筛选

对于多货币回测，可按特定货币筛选统计数据：

```python
from nautilus_trader.model.currencies import USD

create_tearsheet(
    engine=engine,
    output_path="usd_only.html",
    currency=USD,  # Currency 对象，仅显示 USD 统计数据
)
```

当 `currency` 为 `None`（默认值）时，所有货币的统计数据会在报告页中分别显示。

## 可用图表

报告页可以包含以下内置图表的任意组合：

| 图表名称              | 类型  | 描述                                         |
| ----------------- | --- | ------------------------------------------ |
| `run_info`        | 表格  | 运行元数据和账户余额。                                |
| `stats_table`     | 表格  | 绩效统计数据（盈亏、收益率、通用指标）。                       |
| `equity`          | 折线图 | 随时间变化的累计收益率，可选基准对比。                        |
| `drawdown`        | 面积图 | 从权益曲线 (equity curve) 峰值的回撤 (drawdown) 百分比。 |
| `monthly_returns` | 热力图 | 按年份组织的月度组合收益率百分比。                          |
| `distribution`    | 直方图 | 单次收益值的分布情况。                                |
| `rolling_sharpe`  | 折线图 | 60 天滚动夏普比率。                                |
| `yearly_returns`  | 柱状图 | 年度收益率百分比。                                  |
| `bars_with_fills` | K线图 | 价格 K线 (bar)（OHLC）叠加订单 (order) 成交标记。        |

所有图表都注册在图表注册表中，并通过 `TearsheetConfig.charts` 里的图表对象进行配置（每个图表对象映射到一个内置图表名称）。

### 运行信息表

`run_info` 图表显示回测运行的关键元数据：

- 运行 ID、开始时间、完成时间
- 回测周期（起止日期）
- 处理的总迭代次数
- 事件、订单和持仓 (position) 数量
- 账户起始和结束余额（按货币）

此表格默认显示在左上方位置。

### 绩效统计表

`stats_table` 图表显示按类别组织的绩效指标 (indicator)：

- **盈亏统计**（按货币）：总盈亏、胜率、盈利因子等。
- **收益率统计**：夏普比率、索提诺比率、最大回撤等。
- **通用统计**：总交易次数、平均交易持续时间等。

此表格默认显示在右上方位置。

### 权益曲线

`equity` 图表绘制回测期间的累计收益率。当向 `create_tearsheet()` 提供 `benchmark_returns` 时，基准线会叠加显示以供对比。

```python
import pandas as pd

# 加载基准收益率（例如来自市场指数）
# 索引应为 datetime，与策略收益率时间范围对齐
benchmark_returns = pd.read_csv("sp500_returns.csv", index_col=0, parse_dates=True)["return"]

create_tearsheet(
    engine=engine,
    output_path="with_benchmark.html",
    benchmark_returns=benchmark_returns,
    benchmark_name="S&P 500",
)
```

基准序列按原样绘制；请确保索引与策略的收益日期对齐以获得准确对比。

## 主题

主题控制图表的视觉样式，包括颜色、字体和背景。NautilusTrader 提供四个内置主题：

| 主题名称            | 描述                        | 使用场景     |
| --------------- | ------------------------- | -------- |
| `plotly_white`  | 简洁的浅色主题，深灰色标题。            | 默认，专业报告。 |
| `plotly_dark`   | 深色背景，标准 Plotly 配色。        | 低光环境。    |
| `nautilus`      | NautilusTrader 品牌配色的浅色主题。 | 官方浅色模式。  |
| `nautilus_dark` | 青色/蓝绿色特征配色的深色主题。          | 官方深色模式。  |

### 选择主题

在 `TearsheetConfig` 中指定主题：

```python
config = TearsheetConfig(theme="nautilus_dark")
create_tearsheet(engine=engine, config=config)
```

### 自定义主题

注册自定义主题以在所有可视化中保持一致的品牌风格：

```python
from nautilus_trader.analysis import register_theme

register_theme(
    name="corporate",
    template="plotly_white",  # 基础 Plotly 模板
    colors={
        "primary": "#003366",  # 海军蓝
        "positive": "#2e8b57",  # 海绿色
        "negative": "#c41e3a",  # 红衣主教红
        "neutral": "#808080",  # 灰色
        "background": "#ffffff",  # 白色
        "grid": "#e5e5e5",  # 浅灰色
        # 可选的表格颜色（省略时将提供默认值）
        "table_section": "#e5e5e5",
        "table_row_odd": "#f8f8f8",
        "table_row_even": "#ffffff",
        "table_text": "#000000",
    },
)

# 使用自定义主题
config = TearsheetConfig(theme="corporate")
```

主题系统会根据 `background` 和 `grid` 颜色自动为 `table_*` 颜色提供合理的默认值，确保与引入表格专用颜色之前注册的主题向后兼容。

## 配置

`TearsheetConfig` 类提供对报告页生成的声明式控制：

```python
from nautilus_trader.analysis import GridLayout
from nautilus_trader.analysis import TearsheetConfig
from nautilus_trader.analysis import TearsheetDrawdownChart
from nautilus_trader.analysis import TearsheetEquityChart
from nautilus_trader.analysis import TearsheetStatsTableChart

config = TearsheetConfig(
    charts=[
        TearsheetEquityChart(),
        TearsheetDrawdownChart(),
        TearsheetStatsTableChart(),
    ],
    theme="nautilus_dark",
    title="Q4 2024 Strategy Performance",
    height=1800,
    include_benchmark=True,
    benchmark_name="SPY",
    layout=GridLayout(
        rows=2,
        cols=2,
        heights=[0.60, 0.40],
        vertical_spacing=0.08,
        horizontal_spacing=0.12,
    ),
)
```

### 配置参数

| 参数                  | 类型                     | 默认值              | 描述                             |
| ------------------- | ---------------------- | ---------------- | ------------------------------ |
| `charts`            | `list[TearsheetChart]` | 所有内置图表           | 要包含的图表对象列表（按顺序）。               |
| `theme`             | `str`                  | `"plotly_white"` | 样式主题名称。                        |
| `layout`            | `GridLayout`           | `None`（自动计算）     | 自定义子图网格布局。                     |
| `title`             | `str`                  | 自动生成（含策略/时间）     | 报告页标题。                         |
| `include_benchmark` | `bool`                 | `True`           | 提供基准时是否显示。                     |
| `benchmark_name`    | `str`                  | `"Benchmark"`    | 基准的显示名称。                       |
| `height`            | `int`                  | `1500`           | 总高度（像素）。                       |
| `show_logo`         | `bool`                 | `True`           | 显示 NautilusTrader 标志（预留供将来使用）。 |

当 `layout` 为 `None` 时，网格维度和行高根据图表数量自动计算。对于 8 个图表（默认值），使用 4x2 网格，行高为 `[0.50, 0.22, 0.16, 0.12]`，为顶行表格提供更多空间。

## 自定义图表

注册表模式让你能够添加自定义图表。图表是将轨迹 (trace) 渲染到 Plotly 图形对象上的函数。

### 注册自定义图表

```python
from nautilus_trader.analysis.tearsheet import register_chart
import plotly.graph_objects as go


def my_custom_chart(returns, output_path=None, title="Custom Chart", theme="plotly_white"):
    """
    创建自定义可视化。

    此函数签名与内置图表函数保持一致。
    """
    from nautilus_trader.analysis.themes import get_theme

    theme_config = get_theme(theme)

    # 创建可视化
    fig = go.Figure()
    fig.add_trace(
        go.Scatter(
            x=returns.index,
            y=returns.cumsum(),
            mode="lines",
            name="Custom Metric",
            line={"color": theme_config["colors"]["primary"]},
        )
    )

    fig.update_layout(
        title=title,
        template=theme_config["template"],
        xaxis_title="Date",
        yaxis_title="Value",
    )

    if output_path:
        fig.write_html(output_path)

    return fig


# 注册图表以供独立使用（通过 `get_chart()` / `list_charts()`）
register_chart("my_custom", my_custom_chart)
```

### 报告页集成

要实现完整的报告页集成并正确放置在网格中，可使用底层注册方式。

:::warning
`_register_tearsheet_chart` 函数是内部 API，可能在不同版本之间发生变化。在大多数使用场景下，应优先使用 `register_chart` 注册独立图表，或将新的内置图表贡献到上游。
:::

```python
from nautilus_trader.analysis import TearsheetConfig
from nautilus_trader.analysis import TearsheetCustomChart
from nautilus_trader.analysis import TearsheetEquityChart
from nautilus_trader.analysis import TearsheetStatsTableChart
from nautilus_trader.analysis.tearsheet import _register_tearsheet_chart


def _render_my_metric(fig, row, col, returns, theme_config, **kwargs):
    """
    将自定义指标直接渲染到子图上。

    参数
    ----------
    fig : go.Figure
        要添加轨迹的图形对象。
    row : int
        子图行位置。
    col : int
        子图列位置。
    returns : pd.Series
        来自分析器的策略收益率。
    theme_config : dict
        主题配置字典。
    **kwargs : dict
        额外参数（stats_pnls、stats_returns、benchmark_returns 等）。
    """
    metric_values = returns.rolling(30).std() * 100  # 示例指标

    fig.add_trace(
        go.Scatter(
            x=returns.index,
            y=metric_values,
            mode="lines",
            name="30-Day Volatility",
            line={"color": theme_config["colors"]["neutral"]},
        ),
        row=row,
        col=col,
    )

    fig.update_xaxes(title_text="Date", row=row, col=col)
    fig.update_yaxes(title_text="Volatility (%)", row=row, col=col)


# 注册以在报告页中使用
_register_tearsheet_chart(
    name="volatility",
    subplot_type="scatter",
    title="Rolling Volatility (30-day)",
    renderer=_render_my_metric,
)

# 现在可以在 TearsheetConfig.charts 中使用 "volatility"：
config = TearsheetConfig(
    charts=[
        TearsheetStatsTableChart(),
        TearsheetEquityChart(),
        TearsheetCustomChart(chart="volatility"),
    ],
)
```

渲染器函数接收所有必要数据（收益率、统计数据、主题配置），并直接渲染到指定的子图位置。

## 离线分析

当你有预计算的统计数据但没有 `BacktestEngine` 实例时，可使用底层 API：

```python
from nautilus_trader.analysis.tearsheet import create_tearsheet_from_stats

# 加载预计算数据（结构与 PortfolioAnalyzer 输出匹配）
stats_pnls = {"USD": {"PnL (total)": 1500.0, "Win Rate": 0.55, ...}}  # 按货币
stats_returns = {"Sharpe Ratio (252 days)": 1.2, "Max Drawdown": -0.15, ...}
stats_general = {"Avg Winner": 100.0, "Avg Loser": -50.0, ...}
returns = pd.Series(...)  # 带 datetime 索引的日收益率

create_tearsheet_from_stats(
    stats_pnls=stats_pnls,
    stats_returns=stats_returns,
    stats_general=stats_general,
    returns=returns,
    output_path="offline_analysis.html",
)
```

字典键应与 `PortfolioAnalyzer.get_performance_stats_*()` 返回的键匹配。

此方法适用于：

- 分析分别存储的多次回测运行结果。
- 使用预计算指标比较策略。
- 与外部分析管道集成。

## 最佳实践

### 图表选择

- 探索性分析时使用默认图表，查看所有可用指标。
- 了解策略关注的指标后自定义图表。
- 移除不相关的图表以减少视觉混乱和文件大小。

### 主题使用

- 专业报告和演示使用 `plotly_white`。
- 官方材料或低光环境查看使用 `nautilus_dark`。
- 创建自定义主题以匹配内部规范或个人偏好。

### 性能考虑

- 报告页 HTML 文件将所有数据内联包含，长时间回测可能达到几兆字节。
- 考虑为不同分析时间范围生成单独的报告页。
- 对于非常大的数据集，使用单独的图表函数代替完整报告页。

### 自定义统计集成

自定义图表与在 `PortfolioAnalyzer` 中注册的[自定义统计](reports.md)配合使用效果最佳。这确保你的可视化显示的指标与系统其余部分一致地计算：

```python
from nautilus_trader.analysis.statistic import PortfolioStatistic


class MyCustomStatistic(PortfolioStatistic):
    """用于专业策略分析的自定义指标。"""

    def calculate_from_returns(self, returns):
        # 你的计算逻辑
        return custom_metric_value


# 注册到分析器
analyzer.register_statistic(MyCustomStatistic())

# 现在可在 stats_returns 中供自定义图表使用
```

## API 层级

可视化系统提供两个 API 层级：

### 高级 API

推荐用于大多数使用场景：

```python
create_tearsheet(engine=engine, config=config)
```

自动从 `BacktestEngine` 提取数据，生成所有配置的图表，并产出完整的 HTML 报告页。

### 底层 API

用于高级自定义或离线分析：

```python
create_tearsheet_from_stats(
    stats_pnls=stats_pnls,
    stats_returns=stats_returns,
    stats_general=stats_general,
    returns=returns,
    run_info=run_info,
    account_info=account_info,
    config=config,
)
```

提供对数据输入的细粒度控制，并允许分析预计算的统计数据。

### 独立图表函数

单个图表函数可以独立使用，为自定义分析工作流生成单一用途的 HTML 可视化或 Plotly 图形。

#### 带成交标记的价格 K线

`create_bars_with_fills` 函数生成叠加订单成交标记的 K线图，用于在价格走势中直观分析策略执行情况。它既可独立使用，也可包含在报告页中：

```python
from nautilus_trader.analysis import create_bars_with_fills
from nautilus_trader.analysis import create_tearsheet
from nautilus_trader.analysis import TearsheetBarsWithFillsChart
from nautilus_trader.analysis import TearsheetConfig
from nautilus_trader.analysis import TearsheetEquityChart
from nautilus_trader.analysis import TearsheetStatsTableChart
from nautilus_trader.model.data import BarType

# 独立使用
bar_type = BarType.from_str("ESM4.XCME-1-MINUTE-LAST-EXTERNAL")
fig = create_bars_with_fills(
    engine=engine,
    bar_type=bar_type,
    title="ES Futures - Entry/Exit Analysis",
)
fig.show()  # 在 Jupyter 中显示
fig.write_html("bars_with_fills.html")  # 或保存为文件

# 包含在报告页中
config = TearsheetConfig(
    charts=[
        TearsheetStatsTableChart(),
        TearsheetEquityChart(),
        TearsheetBarsWithFillsChart(
            bar_type="ESM4.XCME-1-MINUTE-LAST-EXTERNAL",
            title="Bars with Fills",
        ),
    ],
)
create_tearsheet(engine=engine, config=config)

# 在同一报告页中包含多个带成交标记的 K线图
config = TearsheetConfig(
    charts=[
        TearsheetStatsTableChart(),
        TearsheetEquityChart(),
        TearsheetBarsWithFillsChart(
            bar_type=f"{instrument.id}-5-MINUTE-MID-INTERNAL",
            title=f"Bars with Order Fills - {instrument.id}",
        ),
        TearsheetBarsWithFillsChart(
            bar_type=f"{other_instrument.id}-5-MINUTE-MID-INTERNAL",
            title=f"Bars with Order Fills - {other_instrument.id}",
        ),
    ],
)
create_tearsheet(engine=engine, config=config)
```

该可视化以 K线显示 OHLC 价格走势，并用三角形标记表示订单成交（绿色向上三角形表示买入，红色向下三角形表示卖出）。需要额外配置的图表（如 `bar_type`）直接在图表对象上接收这些参数（例如 `TearsheetBarsWithFillsChart(bar_type=...)`）。

其他单独的图表函数包括 `create_equity_curve`、`create_drawdown_chart`、`create_monthly_returns_heatmap` 等。完整列表请参阅 API 参考文档。

## 相关指南

- [回测](backtesting.md) - 了解如何运行生成报告页的回测。
- [报告](reports.md) - 理解报告页中显示的底层统计数据。
- [投资组合](portfolio.md) - 探索投资组合跟踪和绩效指标。
