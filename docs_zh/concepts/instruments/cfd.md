# 差价合约 (Cfd)

`Cfd` 表示一种差价合约 (contract for difference)，它跟踪某个标的资产，但不转移对标的资产的所有权。交易场所定义计价货币、精度、增量、限额、保证金以及手续费。

例如外汇、股票、指数和大宗商品上的 CFD 合约。

## 字段 (Fields)

| 字段                 | Rust 类型            | Python 类型          | 必填/默认值 | 说明                   |
| ------------------ | ------------------ | ------------------ | ------ | -------------------- |
| `instrument_id`    | `InstrumentId`     | `InstrumentId`     | 必填     | 在 Rust 中以 `id` 存储。   |
| `raw_symbol`       | `Symbol`           | `Symbol`           | 必填     | 交易场所原生代码。            |
| `asset_class`      | `AssetClass`       | `AssetClass`       | 必填     | 标的资产的资产类别。           |
| `base_currency`    | `Option<Currency>` | `Currency \| None` | `None` | 当 CFD 跟踪某基准货币时的基准货币。 |
| `quote_currency`   | `Currency`         | `Currency`         | 必填     | 用于报价和计价的货币。          |
| `price_precision`  | `u8`               | `int`              | 必填     | 价格允许的小数位数。           |
| `size_precision`   | `u8`               | `int`              | 必填     | 订单数量允许的小数位数。         |
| `price_increment`  | `Price`            | `Price`            | 必填     | 最小有效价格步长。            |
| `size_increment`   | `Quantity`         | `Quantity`         | 必填     | 最小有效数量步长。            |
| `lot_size`         | `Option<Quantity>` | `Quantity \| None` | `None` | 取整后的手数或交易板单位。        |
| `max_quantity`     | `Option<Quantity>` | `Quantity \| None` | `None` | 最大订单数量。              |
| `min_quantity`     | `Option<Quantity>` | `Quantity \| None` | `None` | 最小订单数量。              |
| `max_notional`     | `Option<Money>`    | `Money \| None`    | `None` | 最大订单名义价值。            |
| `min_notional`     | `Option<Money>`    | `Money \| None`    | `None` | 最小订单名义价值。            |
| `max_price`        | `Option<Price>`    | `Price \| None`    | `None` | 最大有效报价或订单价格。         |
| `min_price`        | `Option<Price>`    | `Price \| None`    | `None` | 最小有效报价或订单价格。         |
| `margin_init`      | `Option<Decimal>`  | `Decimal \| None`  | `0`    | 初始保证金率。              |
| `margin_maint`     | `Option<Decimal>`  | `Decimal \| None`  | `0`    | 维持保证金率。              |
| `maker_fee`        | `Option<Decimal>`  | `Decimal \| None`  | `0`    | 挂单方手续费率，负值表示返佣。      |
| `taker_fee`        | `Option<Decimal>`  | `Decimal \| None`  | `0`    | 吃单方手续费率，负值表示返佣。      |
| `tick_scheme_name` | N/A                | `str \| None`      | `None` | 已注册的可变 tick 方案名称。    |
| `info`             | `Option<Params>`   | `dict \| None`     | `None` | 适配器元数据。              |
| `ts_event`         | `UnixNanos`        | `int`              | 必填     | 事件时间戳，单位为纳秒。         |
| `ts_init`          | `UnixNanos`        | `int`              | 必填     | 初始化时间戳，单位为纳秒。        |

*注意：Python 构造函数使用 `instrument_id`；Rust 将同一个值存储为 `id`。*

## 行为 (Behavior)

- `Cfd` 的工具类别 (instrument class) 为 `Cfd`。
- 它从不是反向合约 (inverse)，并且使用值为 1 的乘数 (multiplier)。
- 它没有激活时间戳、到期时间戳、行权价或期权类型。
- 当某个交易场所同时提供现金工具和 CFD 时，请使用其原始市场类型。

## 示例 (Example)

```rust tab="Rust"
use nautilus_core::UnixNanos;
use nautilus_model::{
    enums::AssetClass,
    identifiers::{InstrumentId, Symbol},
    instruments::Cfd,
    types::{Currency, Price, Quantity},
};
use rust_decimal_macros::dec;

let audusd = Cfd::new(
    InstrumentId::from("AUDUSD.OANDA"),
    Symbol::from("AUD/USD"),
    AssetClass::FX,
    Some(Currency::from("AUD")),
    Currency::from("USD"),
    5,
    0,
    Price::from("0.00001"),
    Quantity::from("1"),
    Some(Quantity::from("1000")),
    None,
    None,
    None,
    None,
    None,
    None,
    Some(dec!(0.03)),
    Some(dec!(0.03)),
    Some(dec!(0.00002)),
    Some(dec!(0.00002)),
    None,
    UnixNanos::default(),
    UnixNanos::default(),
);
```

```python tab="Python"
from decimal import Decimal

from nautilus_trader.model.currencies import AUD
from nautilus_trader.model.currencies import USD
from nautilus_trader.model.enums import AssetClass
from nautilus_trader.model.identifiers import InstrumentId
from nautilus_trader.model.identifiers import Symbol
from nautilus_trader.model.instruments import Cfd
from nautilus_trader.model.objects import Price
from nautilus_trader.model.objects import Quantity

audusd = Cfd(
    instrument_id=InstrumentId.from_str("AUDUSD.OANDA"),
    raw_symbol=Symbol("AUD/USD"),
    asset_class=AssetClass.FX,
    base_currency=AUD,
    quote_currency=USD,
    price_precision=5,
    price_increment=Price.from_str("0.00001"),
    size_precision=0,
    size_increment=Quantity.from_int(1),
    lot_size=Quantity.from_int(1000),
    margin_init=Decimal("0.03"),
    margin_maint=Decimal("0.03"),
    maker_fee=Decimal("0.00002"),
    taker_fee=Decimal("0.00002"),
    ts_event=0,
    ts_init=0,
)
```

## 适配器 (Adapters)

创建或消费 `Cfd` 工具的代表性适配器包括：

- [Interactive Brokers](../../integrations/ib.md)，用于 CFD 合约。

## 相关指南 (Related guides)

- [货币对 (Currency Pair)](currency_pair.md) 涵盖现金外汇和加密货币现货交易对。
- [大宗商品 (Commodity)](commodity.md) 涵盖现货大宗商品工具。
