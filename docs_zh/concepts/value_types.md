# 值类型 (Value Types)

NautilusTrader 提供了用于表示核心交易概念的专用值类型：
`Price`（价格）、`Quantity`（数量）和 `Money`（货币金额）。这些类型在内部使用定点算术 (fixed-point arithmetic)
以确保在不同平台和环境中实现高性能且确定性的计算。

## 概览

| 类型         | 用途              | 有符号 | 货币  |
| ---------- | --------------- | --- | --- |
| `Quantity` | 交易数量、订单金额、持仓规模。 | 否   | -   |
| `Price`    | 市场价格、报价、价格水平。   | 是   | -   |
| `Money`    | 货币金额、盈亏、账户余额。   | 是   | 是   |

## 不可变性

所有值类型均为**不可变 (immutable)**。一旦构造完成，值便无法更改。
算术运算不会修改原始对象。

```python
from nautilus_trader.model.objects import Quantity

qty1 = Quantity(100, precision=0)
qty2 = Quantity(50, precision=0)

# 这将创建一个新的 Quantity；qty1 和 qty2 保持不变
result = qty1 + qty2

print(qty1)  # 100
print(qty2)  # 50
print(result)  # 150
```

这种设计带来以下优势：

- **线程安全**：不可变值无需同步即可在线程间安全共享。
- **可预测性**：值不会意外改变，使调试更为简单。
- **可哈希性**：不可变类型可用作字典键和集合元素。

## 算术运算

值类型支持标准算术运算符（`+`、`-`、`*`、`/`、`%`、`//`）
以及一元运算符（`-`、`+`、`abs`）。返回类型取决于运算符
和操作数类型。

### 同类型二元运算

对相同值类型进行加法和减法运算时，返回该类型本身，从而保留
其领域含义（价格加价格仍然是价格）：

| 运算                    | 结果         |
| --------------------- | ---------- |
| `Quantity + Quantity` | `Quantity` |
| `Quantity - Quantity` | `Quantity` |
| `Price + Price`       | `Price`    |
| `Price - Price`       | `Price`    |
| `Money + Money`       | `Money`    |
| `Money - Money`       | `Money`    |

```python
from nautilus_trader.model.objects import Price

price1 = Price(100.50, precision=2)
price2 = Price(0.25, precision=2)

result = price1 + price2  # 返回 Price(100.75, precision=2)
print(type(result))  # <class 'Price'>
```

两个相同类型的值之间的乘法、除法、整除和取模运算返回 `Decimal`：

| 运算               | 结果        |
| ---------------- | --------- |
| `Price * Price`  | `Decimal` |
| `Price / Price`  | `Decimal` |
| `Price // Price` | `Decimal` |
| `Price % Price`  | `Decimal` |

同样的规则也适用于 `Quantity` 和 `Money`。

这些运算之所以不返回原始类型，是因为结果具有不同的量纲含义。
价格乘以价格得到的是“价格的平方”，而非价格。数量除以数量得到的是
一个无量纲的比率，而非数量。返回 `Decimal` 使得单位的变化变得明确，
并防止结果被误解为具有原始单位的值。

### 一元运算

当结果对该类型有效时，一元运算符会保留其值类型：

| 运算         | `Price`   | `Quantity` | `Money`   |
| ---------- | --------- | ---------- | --------- |
| `-x` (neg) | `Price`   | `Decimal`  | `Money`   |
| `+x` (pos) | `Price`   | `Quantity` | `Money`   |
| `abs(x)`   | `Price`   | `Quantity` | `Money`   |
| `int(x)`   | `int`     | `int`      | `int`     |
| `float(x)` | `float`   | `float`    | `float`   |
| `round(x)` | `Decimal` | `Decimal`  | `Decimal` |

`Quantity.__neg__` 返回 `Decimal` 而非 `Quantity`，因为 `Quantity` 是
无符号的，无法表示负值。

```python
from nautilus_trader.model.objects import Price, Quantity, Money
from nautilus_trader.model.currencies import USD

price = Price(100.50, precision=2)
print(-price)  # -100.50
print(type(-price))  # <class 'Price'>

money = Money(-50.00, USD)
print(abs(money))  # 50.00 USD
print(type(abs(money)))  # <class 'Money'>

qty = Quantity(10, precision=0)
print(+qty)  # 10
print(type(+qty))  # <class 'Quantity'>
```

### 混合类型运算

与其他数值类型运算时，返回类型遵循 Python 的
[数值塔 (numeric tower)](https://docs.python.org/3/library/numbers.html) 约定。基本
原则是运算结果向更通用的类型扩展：`float` 运算返回 `float`，
而 `int` 和 `Decimal` 运算返回 `Decimal` 以保持精度。

这一规则适用于全部六种二元运算符（`+`、`-`、`*`、`/`、`//`、`%`），
且在两个方向上均成立（`值 op 标量` 和 `标量 op 值`）：

| 左操作数      | 右操作数      | 结果类型      |
| --------- | --------- | --------- |
| 值类型       | `int`     | `Decimal` |
| 值类型       | `float`   | `float`   |
| 值类型       | `Decimal` | `Decimal` |
| `int`     | 值类型       | `Decimal` |
| `float`   | 值类型       | `float`   |
| `Decimal` | 值类型       | `Decimal` |

```python
from decimal import Decimal
from nautilus_trader.model.objects import Quantity

qty = Quantity(100, precision=0)

# Quantity + int -> Decimal
result1 = qty + 50
print(type(result1))  # <class 'decimal.Decimal'>

# Quantity + float -> float
result2 = qty + 50.5
print(type(result2))  # <class 'float'>

# Quantity + Decimal -> Decimal
result3 = qty + Decimal("50")
print(type(result3))  # <class 'decimal.Decimal'>
```

## 精度处理

每种值类型都存储一个精度字段，表示小数位数。
精度在构造时设定且不可变。不存在“未指定”的精度。

### 定点表示

值类型在内部存储为按全局固定精度缩放的整数
（例如，在高精度模式下为 10^16），而非浮点数。`precision`
字段记录构造时所用的小数位数，用于控制显示格式
和序列化，但底层原始值始终使用全局缩放比例。

```python
from nautilus_trader.model.objects import Price

p1 = Price(1.23, precision=2)  # 显示为 "1.23"
p2 = Price(1.230, precision=3)  # 显示为 "1.230"

p1 == p2  # True：底层值相同
str(p1)  # "1.23"
str(p2)  # "1.230"
```

**精度控制的是显示，而非身份。** 两个小数值相同但
精度不同的价格是相等的。`precision` 字段决定字符串格式
以及显示多少位小数，但相等性是基于底层数值判定的。

**市场数据序列化使用精度元数据。** 当市场数据类型（报价、
成交、订单簿增量）被写入 Parquet 或 Arrow 格式时，精度会存储在
文件元数据中，以便能够正确解码这些值。单个文件内的所有市场数据值
必须共享相同的精度。

:::note
如果某个交易场所更改了某个金融工具的最小变动价位 (tick size)（从而改变了其精度），
那么更改前后写入的数据文件将具有不同的精度元数据，不应
被合并到同一个文件中。
:::

关于金融工具级别的精度如何约束有效的价格和数量，请参阅
金融工具指南的 [精度 (Precision)](instruments/index.md#precision) 部分。

### 算术精度

对不同精度的值进行算术运算时，结果采用
操作数中的最大精度。

```python
from nautilus_trader.model.objects import Price

price1 = Price(100.5, precision=1)  # 1 位小数
price2 = Price(0.125, precision=3)  # 3 位小数

result = price1 + price2
print(result)  # 100.625
print(result.precision)  # 3（1 和 3 中的最大值）
```

## 类型特定约束

### Quantity

`Quantity` 表示非负数量。尝试创建负数量或从较小数量中减去较大数量将引发错误：

```python
from nautilus_trader.model.objects import Quantity

# 这将引发 ValueError: Quantity cannot be negative
qty = Quantity(-100, precision=0)

# 这也会引发 ValueError
qty1 = Quantity(50, precision=0)
qty2 = Quantity(100, precision=0)
result = qty1 - qty2  # 结果为 -50，这是无效的
```

### Money

`Money` 值包含货币信息。`Money` 值之间的加法和减法运算要求货币一致：

```python
from nautilus_trader.model.objects import Money
from nautilus_trader.model.currencies import USD, EUR

usd_amount = Money(100.00, USD)
eur_amount = Money(50.00, EUR)

# 这可以正常工作——货币相同
result = usd_amount + Money(25.00, USD)

# 这将引发 ValueError——货币不匹配
result = usd_amount + eur_amount
```

## 常用模式

### 累积值

由于值类型是不可变的，通过重新赋值来累积：

```python
from nautilus_trader.model.objects import Money
from nautilus_trader.model.currencies import USD

total = Money(0.00, USD)
amounts = [Money(100.00, USD), Money(50.00, USD), Money(25.00, USD)]

for amount in amounts:
    total = total + amount  # 重新赋值为新的 Money 实例

print(total)  # 175.00 USD
```

### 转换为其他类型

值类型提供转换方法：

```python
from nautilus_trader.model.objects import Price

price = Price(123.456, precision=3)

# 转换为 Decimal（保留精度）
decimal_value = price.as_decimal()

# 转换为 float
float_value = price.as_double()

# 转换为字符串
string_value = str(price)  # "123.456"
```

### 从字符串创建

从字符串表示解析值类型：

```python
from nautilus_trader.model.objects import Quantity, Price, Money

qty = Quantity.from_str("100.5")
price = Price.from_str("99.95")
money = Money.from_str("1000.00 USD")
```
