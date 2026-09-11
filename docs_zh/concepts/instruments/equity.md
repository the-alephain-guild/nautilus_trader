# 股票 (Equity)

`Equity` 表示一只挂牌股票、ETF 或类似的现货市场证券。Nautilus 对那些以整数单位交易、用单一货币报价且没有合约到期日的金融工具使用这一类型。

例子包括 `AAPL.XNAS`、`MSFT.XNAS` 以及各交易场所特有的 ETF 代码。

## 字段 (Fields)

| 字段                 | Rust 类型            | Python 类型          | 必填/默认值    | 说明                 |
| ------------------ | ------------------ | ------------------ | --------- | ------------------ |
| `instrument_id`    | `InstrumentId`     | `InstrumentId`     | 必填        | 在 Rust 中以 `id` 存储。 |
| `raw_symbol`       | `Symbol`           | `Symbol`           | 必填        | 交易场所原生代码。          |
| `currency`         | `Currency`         | `Currency`         | 必填        | 报价与结算货币。           |
| `price_precision`  | `u8`               | `int`              | 必填        | 价格允许的小数位数。         |
| `price_increment`  | `Price`            | `Price`            | 必填        | 最小有效价格步长。          |
| `lot_size`         | `Option<Quantity>` | `Quantity`         | 必填/Python | 交易板手数或整股手数。        |
| `ts_event`         | `UnixNanos`        | `int`              | 必填        | 事件时间戳，单位纳秒。        |
| `ts_init`          | `UnixNanos`        | `int`              | 必填        | 初始化时间戳，单位纳秒。       |
| `isin`             | `Option<Ustr>`     | `str \| None`      | `None`    | 已知时填写的国际证券识别码。     |
| `max_quantity`     | `Option<Quantity>` | `Quantity \| None` | `None`    | 最大订单数量。            |
| `min_quantity`     | `Option<Quantity>` | `Quantity \| None` | `None`    | 最小订单数量。            |
| `max_price`        | `Option<Price>`    | N/A                | 仅 Rust    | 最大有效报价或订单价格。       |
| `min_price`        | `Option<Price>`    | N/A                | 仅 Rust    | 最小有效报价或订单价格。       |
| `margin_init`      | `Option<Decimal>`  | `Decimal \| None`  | `0`       | 初始保证金率。            |
| `margin_maint`     | `Option<Decimal>`  | `Decimal \| None`  | `0`       | 维持保证金率。            |
| `maker_fee`        | `Option<Decimal>`  | `Decimal \| None`  | `0`       | 挂单方费率。负值表示返佣。      |
| `taker_fee`        | `Option<Decimal>`  | `Decimal \| None`  | `0`       | 吃单方费率。负值表示返佣。      |
| `tick_scheme_name` | N/A                | `str \| None`      | `None`    | 已注册的可变 tick 方案名称。  |
| `info`             | `Option<Params>`   | `dict \| None`     | `None`    | 适配器元数据。            |

*注意：Python 构造函数使用 `instrument_id`；Rust 将同一值以 `id` 存储。*

## 行为 (Behavior)

- `Equity` 的资产类别为 `Equity`，金融工具类别为 `Spot`。
- 数量精度始终为零，因此订单使用整股数量。
- 乘数 (multiplier) 与数量增量 (size increment) 均为一。
- 它没有基础货币、到期日、行权价、期权类型，也没有反向计价 (inverse costing) 标志。
- 仅在交易场所公布价格上下限时才使用价格限制。

## 示例 (Example)

```rust tab="Rust"
use nautilus_core::UnixNanos;
use nautilus_model::{
    identifiers::{InstrumentId, Symbol},
    instruments::Equity,
    types::{Currency, Price, Quantity},
};
use ustr::Ustr;

let aapl = Equity::new(
    InstrumentId::from("AAPL.XNAS"),
    Symbol::from("AAPL"),
    Some(Ustr::from("US0378331005")),
    Currency::from("USD"),
    2,
    Price::from("0.01"),
    Some(Quantity::from("100")),
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    UnixNanos::default(),
    UnixNanos::default(),
);
```

```python tab="Python"
from nautilus_trader.model.currencies import USD
from nautilus_trader.model.identifiers import InstrumentId
from nautilus_trader.model.identifiers import Symbol
from nautilus_trader.model.instruments import Equity
from nautilus_trader.model.objects import Price
from nautilus_trader.model.objects import Quantity

aapl = Equity(
    instrument_id=InstrumentId.from_str("AAPL.XNAS"),
    raw_symbol=Symbol("AAPL"),
    isin="US0378331005",
    currency=USD,
    price_precision=2,
    price_increment=Price.from_str("0.01"),
    lot_size=Quantity.from_int(100),
    ts_event=0,
    ts_init=0,
)
```

## 适配器 (Adapters)

创建或消费 `Equity` 金融工具的代表性适配器包括：

- [Databento](../../integrations/databento.md)，用于挂牌的美国股票和 ETF。
- [Interactive Brokers](../../integrations/ib.md)，用于挂牌的股票合约。

## 相关指南 (Related guides)

- [数据](../data.md) 介绍了引用金融工具的市场数据。
- [值类型](../value_types.md) 介绍了 `Price`、`Quantity` 和 `Money`。
