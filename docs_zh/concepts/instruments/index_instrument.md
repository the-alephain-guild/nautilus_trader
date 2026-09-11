# 指数工具 (Index Instrument)

`IndexInstrument` 表示一个参考指数，例如股票指数、波动率指数或基准价格序列。它携带精度和增量元数据，使 Nautilus 能够一致地存储和路由价格，但它本身并不是一个可直接交易的合约。

示例包括 `SPX.XCBO`、`VIX.XCBO` 以及特定交易场所的参考指数。

## 字段 (Fields)

| 字段                 | Rust 类型          | Python 类型      | 必填/默认值 | 说明                |
| ------------------ | ---------------- | -------------- | ------ | ----------------- |
| `instrument_id`    | `InstrumentId`   | `InstrumentId` | 必填     | 在 Rust 中存储为 `id`。 |
| `raw_symbol`       | `Symbol`         | `Symbol`       | 必填     | 交易场所原生代码。         |
| `currency`         | `Currency`       | `Currency`     | 必填     | 报价值所用的参考货币。       |
| `price_precision`  | `u8`             | `int`          | 必填     | 价格允许的小数位数。        |
| `size_precision`   | `u8`             | `int`          | 必填     | 数量允许的小数位数。        |
| `price_increment`  | `Price`          | `Price`        | 必填     | 最小有效价格步长。         |
| `size_increment`   | `Quantity`       | `Quantity`     | 必填     | 最小有效数量步长。         |
| `ts_event`         | `UnixNanos`      | `int`          | 必填     | 事件时间戳（纳秒）。        |
| `ts_init`          | `UnixNanos`      | `int`          | 必填     | 初始化时间戳（纳秒）。       |
| `tick_scheme_name` | N/A              | `str \| None`  | `None` | 已注册的可变 tick 方案名称。 |
| `info`             | `Option<Params>` | `dict \| None` | `None` | 适配器元数据。           |

*注意：Python 构造函数使用 `instrument_id`；Rust 将同一个值存储为 `id`。*

## 行为 (Behavior)

- `IndexInstrument` 的资产类别为 `Index`，工具类别为 `Spot`。
- 它是一个参考工具，不应用于订单提交。
- 它没有限额、保证金、费用、合约乘数、到期日或结算货币。
- 对于以指数作为标的的可交易衍生品，请使用期权或期货类型。

## 示例 (Example)

```rust tab="Rust"
use nautilus_core::UnixNanos;
use nautilus_model::{
    identifiers::{InstrumentId, Symbol},
    instruments::IndexInstrument,
    types::{Currency, Price, Quantity},
};

let spx = IndexInstrument::new(
    InstrumentId::from("SPX.XCBO"),
    Symbol::from("SPX"),
    Currency::from("USD"),
    2,
    0,
    Price::from("0.01"),
    Quantity::from("1"),
    None,
    UnixNanos::default(),
    UnixNanos::default(),
);
```

```python tab="Python"
from nautilus_trader.model.currencies import USD
from nautilus_trader.model.identifiers import InstrumentId
from nautilus_trader.model.identifiers import Symbol
from nautilus_trader.model.instruments import IndexInstrument
from nautilus_trader.model.objects import Price
from nautilus_trader.model.objects import Quantity

spx = IndexInstrument(
    instrument_id=InstrumentId.from_str("SPX.XCBO"),
    raw_symbol=Symbol("SPX"),
    currency=USD,
    price_precision=2,
    size_precision=0,
    price_increment=Price.from_str("0.01"),
    size_increment=Quantity.from_str("1"),
    ts_event=0,
    ts_init=0,
)
```

## 适配器 (Adapters)

创建或消费 `IndexInstrument` 工具的代表性适配器包括：

- [Interactive Brokers](../../integrations/ib.md)，用于参考指数。
- [Databento](../../integrations/databento.md)，用于参考数据源。

## 相关指南 (Related guides)

- [期权合约 (Option Contract)](option_contract.md) 介绍以指数为标的的上市期权。
- [期货合约 (Futures Contract)](futures_contract.md) 介绍指数期货。
