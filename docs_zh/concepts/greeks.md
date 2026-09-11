# 希腊字母 (Greeks)

Nautilus 提供两条处理期权希腊字母（Greeks，即期权价格对市场变量变化的敏感度）的路径：

1. **场所提供的希腊字母 (Venue-provided Greeks, Rust/PyO3)**：通过 `OptionGreeks`
   数据类型和期权链聚合系统，从 Deribit、Bybit、OKX 等场所实时流式传入的希腊字母。
2. **本地希腊字母计算器 (Local Greeks calculator, Cython/Python 与 Rust/PyO3)**：`GreeksCalculator`
   类基于缓存的市场数据计算 Black-Scholes 希腊字母，支持组合聚合、冲击情景
   (shock scenarios) 和 beta 加权。

这两条路径既可以各自独立使用，也可以协同工作。场所提供的希腊字母通过数据订阅系统到达，
无需任何本地计算。本地计算器则覆盖那些不提供希腊字母流的场所、回测，以及自定义调整
（冲击、beta 加权、百分比希腊字母）。

## 场所提供的希腊字母 (Venue-provided Greeks, Rust/PyO3)

### OptionGreeks

`OptionGreeks` 类型表示场所为单个期权合约提供的敏感度。它是一个 Rust 原生类型，
通过 PyO3 暴露给 Python。

| 字段                 | 类型                 | 描述                                  |
| ------------------ | ------------------ | ----------------------------------- |
| `instrument_id`    | `InstrumentId`     | 这些希腊字母所对应的期权合约。                     |
| `convention`       | `GreeksConvention` | 希腊字母的计价单位约定 (numeraire convention)。 |
| `delta`            | `float`            | 期权价格相对每单位标的的变化率。                    |
| `gamma`            | `float`            | delta 相对每单位标的的变化率。                  |
| `vega`             | `float`            | 对隐含波动率变化 1% 的敏感度。                   |
| `theta`            | `float`            | 每日时间衰减（dV/dt / 365.25）。             |
| `rho`              | `float`            | 对利率变化的敏感度。                          |
| `mark_iv`          | `float` 或 None     | 标记隐含波动率 (mark implied volatility)。  |
| `bid_iv`           | `float` 或 None     | 买价隐含波动率。                            |
| `ask_iv`           | `float` 或 None     | 卖价隐含波动率。                            |
| `underlying_price` | `float` 或 None     | 计算时刻的标的价格。                          |
| `open_interest`    | `float` 或 None     | 合约的未平仓量 (open interest)。            |
| `ts_event`         | `int`              | 事件的 UNIX 时间戳（纳秒）。                   |
| `ts_init`          | `int`              | 初始化时的 UNIX 时间戳（纳秒）。                 |

从 actor 或 strategy 中订阅：

```python
self.subscribe_option_greeks(instrument_id, client_id=ClientId("DERIBIT"))
```

处理更新：

```python
def on_option_greeks(self, greeks: OptionGreeks) -> None:
    self.log.info(f"delta={greeks.delta:.4f} gamma={greeks.gamma:.6f}")
```

完整的订阅 API（包括期权链聚合、行权价区间过滤和快照模式）请参阅
[期权 (Options)](options.md) 指南。

### 持久化与回放 (Persistence and replay)

`OptionGreeks` 是 `Data` 枚举的原生成员，因此它会持久化到数据目录 (data catalog)，
并在回测中作为内置市场数据（而非自定义数据）回放。写入和查询都使用标准的目录 API：

```python
catalog.write_data(greeks)  # greeks: list[OptionGreeks]
greeks = catalog.query(data_cls=OptionGreeks)
```

在回放期间，持久化的希腊字母会通过与实时数据相同的 `on_option_greeks` 处理函数
到达订阅它的 actor 或 strategy。它们还会驱动期权链聚合：当策略订阅 `OptionChainSlice`
时，回测数据引擎会为每个期权工具将回放的 `OptionGreeks` 与回放的 `QuoteTick`
BBO 更新连接起来。`underlying_price` 字段用于初始化 ATM，`delta` 则通过
`StrikeRange.delta(target, tolerance)` 支持基于 delta 的行权价选择。

### 核心 schema 与自定义数据 (Core schema versus custom data)

原生 `OptionGreeks` 的字段构成了规范的核心 schema：五个标准希腊字母
（`delta`、`gamma`、`vega`、`theta`、`rho`），加上隐含波动率、标的价格、未平仓量和
计价约定。这些字段名是稳定的。

由于不存在唯一完整的希腊字母形态，因此场所特定或模型特定的值，例如 `vanna`、`volga`、
`charm`、校准输入或曲面元数据，应归入[自定义数据 (custom data)](custom_data.md)，
而不是原生类型。可选的场所字段是可空的 (nullable)。而解释这些值所必需的字段，例如
`convention`，则是非空的，并带有默认值。

### 底层 Rust 类型 (Underlying Rust types)

核心 Rust 实现位于 `crates/model/src/data/greeks.rs`：

- `OptionGreekValues`：一个普通结构体，包含 `delta`、`gamma`、`vega`、`theta`、`rho`
  字段。它实现了 `Add` 和 `Mul<f64>` 以支持聚合。
- `OptionGreeks`（位于 `crates/model/src/data/option_chain.rs`）：用 `instrument_id`、
  `convention`、隐含波动率字段和时间戳包装 `OptionGreekValues`。它实现了
  `Deref<Target = OptionGreekValues>`，因此你可以直接访问希腊字母字段。
- `HasGreeks` trait：提供一个返回 `OptionGreekValues` 的 `greeks()` 方法。
  `OptionGreekValues` 和 `OptionGreeks` 都实现了它。

### Black-Scholes 函数 (Rust/PyO3)

从 `crates/model/src/data/greeks.rs` 暴露给 Python 的底层定价函数：

```python
from nautilus_trader.model import (
    black_scholes_greeks,
    imply_vol,
    imply_vol_and_greeks,
    refine_vol_and_greeks,
)

# 在已知波动率的情况下计算希腊字母
result = black_scholes_greeks(s=100.0, r=0.05, b=0.0, vol=0.20, is_call=True, k=100.0, t=0.25)
# result.delta, result.gamma, result.vega, result.theta, result.price, result.vol

# 从市场价格反推波动率，然后计算希腊字母
result = imply_vol_and_greeks(s=100.0, r=0.05, b=0.0, is_call=True, k=100.0, t=0.25, price=5.0)

# 从一个起始波动率估计值开始精化（收敛更快）
result = refine_vol_and_greeks(
    s=100.0, r=0.05, b=0.0, is_call=True, k=100.0, t=0.25, target_price=5.0, initial_vol=0.18
)
```

这些函数返回的 `BlackScholesGreeksResult` 包含：`price`、`vol`、`delta`、`gamma`、
`vega`、`theta` 和 `itm_prob`。

**约定 (Conventions)：**

- Vega 按 0.01 缩放（对波动率变化 1 个百分点的敏感度）。
- Theta 按 1/365.25 缩放（每日衰减）。
- 美式期权在希腊字母计算中按欧式期权定价。

## 本地希腊字母计算器 (Local Greeks calculators)

### GreeksCalculator

`nautilus_trader/model/greeks.pyx` 中的旧版 Cython `GreeksCalculator` 类基于缓存的
市场数据计算 Black-Scholes 希腊字母。还有一个 PyO3 计算器从
`nautilus_trader.common.GreeksCalculator` 暴露出来，供 v2 接口使用。二者都使用
缓存 (cache) 和时钟 (clock)，并且都可以从 actor 或 strategy 中访问。

```python
from nautilus_trader.model.greeks import GreeksCalculator

# 通常在 on_start() 中创建
calculator = GreeksCalculator(cache=self.cache, clock=self.clock)
```

#### 工具希腊字母 (Instrument Greeks)

为单个工具（期权或标的）按数量 1 计算希腊字母：

```python
greeks = calculator.instrument_greeks(
    instrument_id=option_id,
    flat_interest_rate=0.0425,  # 在缓存中没有收益率曲线时使用
)
# 返回 GreeksData 或 None
```

该计算器：

1. 在缓存中查找该工具及其标的。
2. 获取当前价格（优先 MID，回退到 LAST）。
3. 从缓存中查找收益率曲线（回退到 `flat_interest_rate`）。
4. 使用 `imply_vol_and_greeks` 从市场价格反推波动率。
5. 返回一个包含全部计算结果的 `GreeksData` 对象。

对于非期权工具（期货、股票），计算器返回的 `GreeksData` 带有 `delta=1`（或 beta 加权的
delta），且没有 gamma/vega/theta。

**冲击情景 (Shock scenarios)**：对现货、波动率或时间施加假设性的变化：

```python
greeks = calculator.instrument_greeks(
    instrument_id=option_id,
    spot_shock=10.0,  # 标的 +10 点
    vol_shock=0.02,  # 波动率绝对值 +2%
    time_to_expiry_shock=1 / 365,  # 向前滚动一天
)
```

**波动率更新 (Volatility update)**：从缓存的起始点精化隐含波动率，以加快收敛：

```python
greeks = calculator.instrument_greeks(
    instrument_id=option_id,
    update_vol=True,  # 使用缓存的波动率作为起始点
    cache_greeks=True,  # 存储结果供下次迭代使用
)
```

**Beta 加权希腊字母 (Beta-weighted Greeks)**：以某个指数为基准表达 delta 和 gamma：

```python
greeks = calculator.instrument_greeks(
    instrument_id=option_id,
    index_instrument_id=InstrumentId.from_str("SPX.CBOE"),
    beta_weights={underlying_id: 1.15},
    percent_greeks=True,
)
```

**时间加权 vega (Time-weighted vega)**：跨不同到期日对 vega 进行归一化：

```python
greeks = calculator.instrument_greeks(
    instrument_id=option_id,
    vega_time_weight_base=30,  # 归一化到 30 天 vega
)
```

#### 组合希腊字母 (Portfolio Greeks)

跨所有符合过滤条件的未平仓持仓聚合希腊字母：

```python
portfolio = calculator.portfolio_greeks(
    underlyings=["AAPL", "MSFT"],
    venue=Venue("CBOE"),
    strategy_id=StrategyId("DELTA_HEDGE-001"),
    flat_interest_rate=0.0425,
    index_instrument_id=InstrumentId.from_str("SPX.CBOE"),
    beta_weights=beta_dict,
    percent_greeks=True,
)
# 返回 PortfolioGreeks: pnl, price, delta, gamma, vega, theta
```

过滤条件：

- `underlyings`：符号前缀列表（例如 `["AAPL"]` 匹配 AAPL 股票及所有 AAPL 期权）。
- `venue`：限制为单个场所。
- `instrument_id`：限制为单个工具。
- `strategy_id`：限制为单个策略。
- `side`：按持仓方向过滤（LONG、SHORT）。
- `greeks_filter`：一个可调用对象，对每个持仓接收 `PortfolioGreeks`；返回
  `True` 表示包含该持仓。

### GreeksData

在旧版 Python 接口上，`GreeksData` 是一个 Python 自定义数据类
（`@customdataclass`），它携带单个工具希腊字母计算的完整上下文。它继承自 `Data`，
并支持 Arrow 序列化、缓存存储和目录持久化。v2/PyO3 接口从 Rust 暴露相同的核心字段。

| 字段                 | 类型             | 描述                                   |
| ------------------ | -------------- | ------------------------------------ |
| `instrument_id`    | `InstrumentId` | 该工具。                                 |
| `is_call`          | `bool`         | 看涨为 True，看跌为 False。                  |
| `strike`           | `float`        | 行权价。                                 |
| `expiry`           | `int`          | 到期日，以 YYYYMMDD 整数表示。                 |
| `expiry_in_days`   | `int`          | 距到期的天数。                              |
| `expiry_in_years`  | `float`        | 距到期的年数（天数 / 365.25）。                 |
| `multiplier`       | `float`        | 合约乘数。                                |
| `quantity`         | `float`        | 持仓数量（来自 `instrument_greeks` 时始终为 1）。 |
| `underlying_price` | `float`        | 计算中使用的标的价格。                          |
| `interest_rate`    | `float`        | 使用的利率。                               |
| `cost_of_carry`    | `float`        | 持有成本（r - 股息率；期货为 0）。                 |
| `vol`              | `float`        | 隐含波动率。                               |
| `pnl`              | `float`        | 相对于持仓入场的盈亏（如果提供了持仓）。                 |
| `price`            | `float`        | 模型价格。                                |
| `delta`            | `float`        | Delta。                               |
| `gamma`            | `float`        | Gamma。                               |
| `vega`             | `float`        | Vega（dV / 波动率变化 1%）。                 |
| `theta`            | `float`        | Theta（每日衰减）。                         |
| `itm_prob`         | `float`        | 实值概率 (in-the-money probability)。     |

`GreeksData` 通过其 `to_portfolio_greeks()` 方法扩展到组合层面，该方法将所有值乘以
合约 `multiplier`。`*` 运算符用于应用持仓数量：

```python
position_greeks = signed_qty * instrument_greeks  # 返回 PortfolioGreeks
```

### PortfolioGreeks

`PortfolioGreeks` 是 `portfolio_greeks()` 的聚合结果。它支持加法（`+`）以合并持仓，
以及标量乘法（`*`）以进行缩放：

| 字段      | 类型      | 描述        |
| ------- | ------- | --------- |
| `pnl`   | `float` | 聚合盈亏。     |
| `price` | `float` | 聚合模型价值。   |
| `delta` | `float` | 组合 delta。 |
| `gamma` | `float` | 组合 gamma。 |
| `vega`  | `float` | 组合 vega。  |
| `theta` | `float` | 组合 theta。 |

### YieldCurveData

`YieldCurveData` 存储一条利率或股息收益率曲线。`GreeksCalculator` 会按货币代码
（用于利率）或按标的工具 ID（用于股息收益率）从缓存中查找曲线。

```python
from nautilus_trader.model.greeks_data import YieldCurveData
import numpy as np

curve = YieldCurveData(
    ts_event=0,
    ts_init=0,
    curve_name="USD",
    tenors=np.array([0.25, 0.5, 1.0, 2.0]),
    interest_rates=np.array([0.04, 0.042, 0.045, 0.048]),
)

# 可调用：为给定期限插值出利率
rate = curve(0.75)  # 二次插值
```

## 在两条路径之间做选择 (Choosing between the two paths)

| 标准      | 场所提供 (`OptionGreeks`)                  | 本地计算器 (`GreeksCalculator`)               |
| ------- | -------------------------------------- | ---------------------------------------- |
| 计算      | 由场所完成                                  | 本地 Black-Scholes                         |
| 延迟      | 随市场数据一同到达                              | 按需计算                                     |
| 场所      | Deribit、Bybit、OKX                      | 任何带期权工具的场所                               |
| 冲击情景    | 不支持                                    | 现货、波动率和时间冲击                              |
| 组合聚合    | 手动（遍历 `OptionChainSlice`）              | 通过 `portfolio_greeks()` 内置               |
| Beta 加权 | 不支持                                    | 内置                                       |
| 回测支持    | 通过录制的 `OptionGreeks` 数据                | 基于任意时间点的缓存价格                             |
| 可用的希腊字母 | delta, gamma, vega, theta, rho, IV, OI | delta, gamma, vega, theta, itm_prob, vol |
| 数据类型    | `OptionGreeks` (Rust/PyO3)             | `GreeksData` / `PortfolioGreeks`         |

## 希腊字母定义 (Greek definitions)

供参考，Nautilus 计算的希腊字母如下：

| 希腊字母   | 符号  | 定义                                                |
| ------ | --- | ------------------------------------------------- |
| Delta  | `d` | 期权价格对标的价格的一阶导数（dV/dS）。                            |
| Gamma  | `g` | 期权价格对标的价格的二阶导数（d2V/dS2）。                          |
| Vega   | `v` | 对隐含波动率变化 1 个百分点的敏感度（dV/dVol）。                     |
| Theta  | `t` | 每日时间衰减：期权价格每个日历日的变化（dV/dt / 365.25）。              |
| Rho    | `r` | 对无风险利率变化的敏感度（dV/dr）。                              |
| ITM 概率 | -   | 期权到期时处于实值的概率：P(ϕS_T > ϕK)，其中看涨时 ϕ = 1，看跌时 ϕ = -1。 |

## 示例 (Examples)

完整可运行的示例可在代码仓库中找到：

- `examples/live/bybit/bybit_option_greeks.py`：订阅 Bybit 场所提供的希腊字母。
- `examples/live/deribit/deribit_option_greeks.py`：订阅 Deribit 场所提供的希腊字母。
- `examples/live/okx/okx_option_greeks.py`：订阅 OKX 场所提供的希腊字母。

## 相关指南 (Related guides)

- [期权 (Options)](options.md) - 期权工具、期权链订阅和行权价过滤。
- [数据 (Data)](data.md) - 内置数据类型、自定义数据和订阅模型。
- [Actors](actors.md) - 订阅和处理函数参考。
- [策略 (Strategies)](strategies.md) - 策略实现和处理方法。
