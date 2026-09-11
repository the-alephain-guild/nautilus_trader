# 合成工具 (Synthetics)

合成工具 (synthetic instruments) 是在本地定义的工具，其价格由其他工具派生而来。
它们可以组合来自单一交易场所或多个交易场所的成分工具，并以合成交易场所代码 `SYNTH`
将结果暴露为一个标准的 Nautilus 工具。

合成工具适用于以下场景：

- 让 `Actor` 与 `Strategy` 组件能够订阅报价 (quote) 或成交 (trade) 数据流。
- 由派生价格触发模拟订单 (emulated orders)。
- 由合成报价或成交构造 K 线 (bars)。

合成工具不能被直接交易。它们存在于平台本地，充当分析工具。未来 Nautilus 可能会支持基于
合成工具行为来交易其成分工具。

## 公式语言 (Formula language)

每个合成工具都定义一条派生公式。Nautilus 使用其内置的数值表达式引擎对该公式求值，
并将最终的数值结果转换为合成工具的 `Price`。

### 支持的语法 (Supported syntax)

公式可以直接引用成分工具的 `InstrumentId` 值，包括含有 `/` 和 `-` 的 ID。

| 构造    | 示例                                          | 说明                          |
| ----- | ------------------------------------------- | --------------------------- |
| 成分引用  | `BTCUSDT.BINANCE`                           | 使用原始的 `InstrumentId` 文本。    |
| 成分引用  | `AUD/USD.SIM`                               | 含有 `/` 的 ID 是合法的。           |
| 成分引用  | `ETH-USDT-SWAP.OKX`                         | 含有 `-` 的 ID 是合法的。           |
| 数值字面量 | `1`、`0.5`、`1.2e-3`                          | 按 `f64` 语义求值。               |
| 布尔字面量 | `true`、`false`                              | 用于条件和逻辑表达式。                 |
| 括号    | `(a + b) / 2`                               | 使用括号覆盖运算优先级。                |
| 一元运算符 | `-x`、`!flag`                                | 一元 `-` 对数值取负。一元 `!` 对布尔值取反。 |
| 二元运算符 | `+ - * / % ^`、`== !=`、`< <= > >=`、`&& \|\|` | 算术运算为数值运算。逻辑运算符为布尔运算。       |
| 局部赋值  | `spread = a - b; spread / 2`                | 语句从左到右执行。公式必须以一个值结尾。        |
| 注释    | `// line`、`/* block */`                     | 注释会被忽略。                     |

:::note
新公式应使用原始的 `InstrumentId` 值。为了向后兼容，将成分 ID 中 `-` 替换为 `_`
的公式仍然被接受。
:::

### 运算符优先级 (Operator precedence)

表达式引擎按以下顺序对运算符求值，从最高优先级到最低优先级：

| 级别  | 运算符               | 说明                       |
| --- | ----------------- | ------------------------ |
| 最高  | `^`               | 幂运算。右结合。                 |
|     | 一元 `-`、一元 `!`     | `-2 ^ 2` 求值为 `-(2 ^ 2)`。 |
|     | `*`、`/`、`%`       | 乘法、除法和取模。                |
|     | `+`、`-`           | 加法和减法。                   |
|     | `<`、`<=`、`>`、`>=` | 数值比较。                    |
|     | `==`、`!=`         | 相等与不等。两侧必须为相同类型。         |
| 最低  | `&&`、`\|\|`       | 布尔运算符。                   |

赋值不是表达式运算符。用 `;` 分隔语句，并让最后一条语句成为你希望合成工具产出的值。

### 内置函数 (Built-in functions)

| 函数      | 签名                                     | 说明                          |
| ------- | -------------------------------------- | --------------------------- |
| `abs`   | `abs(x)`                               | 绝对值。                        |
| `ceil`  | `ceil(x)`                              | 向上取整。                       |
| `floor` | `floor(x)`                             | 向下取整。                       |
| `round` | `round(x)`                             | 按 Rust `f64` 规则四舍五入到最近的整数。  |
| `min`   | `min(x1, x2, ...)`                     | 接受一个或多个数值参数。                |
| `max`   | `max(x1, x2, ...)`                     | 接受一个或多个数值参数。                |
| `if`    | `if(condition, when_true, when_false)` | 条件必须为布尔值。两个分支类型相同。仅对所选分支求值。 |

### 类型规则 (Type rules)

- 成分输入是数值。
- 算术运算符要求数值操作数并返回数值结果。
- `<`、`<=`、`>`、`>=` 要求数值操作数并返回布尔结果。
- `==` 和 `!=` 接受任意匹配的类型（两侧均为数值或均为布尔），并返回布尔结果。
- `&&`、`||` 和一元 `!` 要求布尔操作数。
- `&&` 和 `||` 会短路求值。右侧仅在需要时才求值。
- 局部变量必须先赋值后使用。
- 局部变量名必须以字母或 `_` 开头，随后由字母、数字或 `_` 组成。
- 公式的最终结果必须为数值。以赋值结尾或产出布尔结果的公式对合成工具而言是无效的。

### 限制 (Limits)

表达式引擎强制执行以下编译期限制。超出限制的公式会在构造时产生清晰的错误。

| 限制   | 值   | 描述            |
| ---- | --- | ------------- |
| 栈深度  | 32  | 求值栈上中间值的最大数量。 |
| 局部变量 | 16  | 不同局部变量名的最大数量。 |

对于任何实际的定价公式而言，这些限制都相当宽裕。一个含 8 个成分的加权和仅使用峰值栈深度
3 且没有局部变量。

### 示例 (Examples)

```python
# 简单价差
formula = "BTCUSDT.BINANCE - ETHUSDT.BINANCE"

# 两个外汇对的平均值
formula = "(AUD/USD.SIM + NZD/USD.SIM) / 2"

# 复用一个中间值
formula = "spread = BTCUSDT.BINANCE - ETHUSDT.BINANCE; spread / 2"

# 条件输出
formula = "if(BTCUSDT.BINANCE > ETHUSDT.BINANCE, BTCUSDT.BINANCE, ETHUSDT.BINANCE)"
```

## 创建合成工具 (Creating a synthetic instrument)

在定义新的合成工具之前，请确保所有成分工具都已存在于缓存 (cache) 中。

下面的示例在一个 actor 或 strategy 中创建一个合成工具。该合成工具表示 Binance 上比特币
与以太坊现货价格之间的一个简单价差。它假定 `BTCUSDT.BINANCE` 和 `ETHUSDT.BINANCE`
已经存在于缓存中。

```python
from nautilus_trader.model.instruments import SyntheticInstrument

btcusdt_binance_id = InstrumentId.from_str("BTCUSDT.BINANCE")
ethusdt_binance_id = InstrumentId.from_str("ETHUSDT.BINANCE")

synthetic = SyntheticInstrument(
    symbol=Symbol("BTC-ETH:BINANCE"),
    price_precision=8,
    components=[
        btcusdt_binance_id,
        ethusdt_binance_id,
    ],
    formula=f"{btcusdt_binance_id} - {ethusdt_binance_id}",
    ts_event=self.clock.timestamp_ns(),
    ts_init=self.clock.timestamp_ns(),
)

self._synthetic_id = synthetic.id
self.add_synthetic(synthetic)
self.subscribe_quote_ticks(self._synthetic_id)
```

:::note
上面示例中的合成工具 `instrument_id` 为 `{symbol}.SYNTH`，最终产出
`BTC-ETH:BINANCE.SYNTH`。
:::

## 更新公式 (Updating formulas)

你可以随时更新合成工具的公式。

```python
synthetic = self.cache.synthetic(self._synthetic_id)

new_formula = "(BTCUSDT.BINANCE + ETHUSDT.BINANCE) / 2"
synthetic.change_formula(new_formula)

self.update_synthetic(synthetic)
```

## 触发工具 ID (Trigger instrument IDs)

你可以由合成价格触发模拟订单。在下面的示例中，一旦合成价格达到触发条件，合成工具便会
释放一个模拟订单。

```python
order = self.strategy.order_factory.limit(
    instrument_id=ETHUSDT_BINANCE.id,
    order_side=OrderSide.BUY,
    quantity=Quantity.from_str("1.5"),
    price=Price.from_str("30000.00000000"),
    emulation_trigger=TriggerType.DEFAULT,
    trigger_instrument_id=self._synthetic_id,
)

self.strategy.submit_order(order)
```

## 性能 (Performance)

公式在构造时编译一次，并在每个到来的成分价格 tick 上求值。表达式引擎采用一次编译、多次
求值 (compile-once/eval-many) 的架构，配合零分配 (zero-allocation) 的 f64 栈，因此求值
对 tick 处理路径只增加可以忽略不计的开销。

测量环境为 Apple M4 Pro、rustc 1.94.1、release 配置 (opt-level 3)：

### 求值（热路径）(Evaluation (hot path))

| 公式模式                                    | 耗时    |
| --------------------------------------- | ----- |
| `(A + B) / 2.0`                         | 12 ns |
| `A * 0.4 + B * 0.3 + C * 0.2 + D * 0.1` | 18 ns |
| `if(A > B, A - B, B - A)`               | 12 ns |
| `spread = A - B; mid = ...; mid + ...`  | 19 ns |
| `max(min(A, B * 20), abs(A - B))`       | 15 ns |

### 求值的规模扩展（加权和）(Evaluation scaling (weighted sum))

| 成分数量 | 耗时    |
| ---- | ----- |
| 2    | 14 ns |
| 4    | 18 ns |
| 8    | 28 ns |

### 编译（冷路径）(Compilation (cold path))

| 公式模式     | 耗时     |
| -------- | ------ |
| 简单平均     | 675 ns |
| 4 输入加权   | 1.4 us |
| 条件       | 1.0 us |
| 含局部变量    | 1.3 us |
| 含连字符的 ID | 755 ns |

## 错误处理 (Error handling)

Nautilus 在每个边界上都会校验合成工具。公式编译会拒绝未知符号、类型错误和容量溢出。
求值会在错误数据到达公式之前拒绝输入数量错误以及非有限价格 (NaN、Infinity)。

关于输入要求和异常，请参阅
[`SyntheticInstrument` API 参考](/docs/python-api-latest/model/instruments.html#nautilus_trader.model.instruments.synthetic.SyntheticInstrument)。

## 相关指南 (Related guides)

- [Instruments](instruments/) - 工具定义与各交易场所特定的工具类型。
- [Data](data.md) - 引用工具的市场数据类型。
- [Orders](orders/) - 订单可使用合成工具 ID 作为模拟触发条件。
