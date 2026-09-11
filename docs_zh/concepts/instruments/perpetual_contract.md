# 永续合约 (Perpetual Contract)

`PerpetualContract` 表示一种跨资产类别的通用永续期货合约。
当某个交易场所提供的永续掉期没有被专门建模为 `CryptoPerpetual` 时，请使用它。

典型例子包括非加密类的永续合约，以及交易场所特有的合成掉期。

## 字段 (Fields)

| 字段                    | Rust 类型            | Python 类型          | 必填/默认值 | 说明                    |
| --------------------- | ------------------ | ------------------ | ------ | --------------------- |
| `instrument_id`       | `InstrumentId`     | `InstrumentId`     | 必填     | 在 Rust 中以 `id` 存储。    |
| `raw_symbol`          | `Symbol`           | `Symbol`           | 必填     | 交易场所原生符号。             |
| `underlying`          | `Ustr`             | `str`              | 必填     | 标的资产或参考市场。            |
| `asset_class`         | `AssetClass`       | `AssetClass`       | 必填     | 标的的资产类别。              |
| `base_currency`       | `Option<Currency>` | `Currency \| None` | `None` | 基础货币，反向合约必填。          |
| `quote_currency`      | `Currency`         | `Currency`         | 必填     | 用于报价价格的货币。            |
| `settlement_currency` | `Currency`         | `Currency`         | 必填     | 用于结算盈亏与手续费的货币。        |
| `is_inverse`          | `bool`             | `bool`             | 必填     | 当头寸规模/成本计算为反向时为 True。 |
| `price_precision`     | `u8`               | `int`              | 必填     | 价格允许的小数位数。            |
| `size_precision`      | `u8`               | `int`              | 必填     | 下单数量允许的小数位数。          |
| `price_increment`     | `Price`            | `Price`            | 必填     | 最小有效价格步长。             |
| `size_increment`      | `Quantity`         | `Quantity`         | 必填     | 最小有效数量步长。             |
| `multiplier`          | `Quantity`         | `Quantity`         | `1`    | 合约乘数。                 |
| `lot_size`            | `Quantity`         | `Quantity`         | `1`    | 取整后的手数或交易板块规模。        |
| `max_quantity`        | `Option<Quantity>` | `Quantity \| None` | `None` | 最大下单数量。               |
| `min_quantity`        | `Option<Quantity>` | `Quantity \| None` | `None` | 最小下单数量。               |
| `max_notional`        | `Option<Money>`    | `Money \| None`    | `None` | 最大下单名义价值。             |
| `min_notional`        | `Option<Money>`    | `Money \| None`    | `None` | 最小下单名义价值。             |
| `max_price`           | `Option<Price>`    | `Price \| None`    | `None` | 最大有效报价或下单价格。          |
| `min_price`           | `Option<Price>`    | `Price \| None`    | `None` | 最小有效报价或下单价格。          |
| `margin_init`         | `Option<Decimal>`  | `Decimal \| None`  | `0`    | 初始保证金率。               |
| `margin_maint`        | `Option<Decimal>`  | `Decimal \| None`  | `0`    | 维持保证金率。               |
| `maker_fee`           | `Option<Decimal>`  | `Decimal \| None`  | `0`    | Maker 手续费率。负值表示返佣。    |
| `taker_fee`           | `Option<Decimal>`  | `Decimal \| None`  | `0`    | Taker 手续费率。负值表示返佣。    |
| `tick_scheme_name`    | N/A                | `str \| None`      | `None` | 已注册的可变 tick 方案名称。     |
| `info`                | `Option<Params>`   | `dict \| None`     | `None` | 适配器元数据。               |
| `ts_event`            | `UnixNanos`        | `int`              | 必填     | 事件时间戳（纳秒）。            |
| `ts_init`             | `UnixNanos`        | `int`              | 必填     | 初始化时间戳（纳秒）。           |

*注意：Python 构造函数使用 `instrument_id`；Rust 将同一值以 `id` 存储。*

## 行为 (Behavior)

- `PerpetualContract` 的工具类别 (instrument class) 为 `Swap`。
- 它没有激活时间戳，也没有到期时间戳。
- 反向合约需要提供基础货币。
- 线性合约通常以报价货币结算。
- 当基础资产为某种货币的加密永续合约，请使用 `CryptoPerpetual`。

## 示例 (Example)

```rust tab="Rust"
use nautilus_core::UnixNanos;
use nautilus_model::{
    enums::AssetClass,
    identifiers::{InstrumentId, Symbol},
    instruments::PerpetualContract,
    types::{Currency, Price, Quantity},
};
use rust_decimal_macros::dec;
use ustr::Ustr;

let eurusd_perp = PerpetualContract::new(
    InstrumentId::from("EURUSD-PERP.AX"),
    Symbol::from("EURUSD-PERP"),
    Ustr::from("EURUSD"),
    AssetClass::FX,
    Some(Currency::from("EUR")),
    Currency::from("USD"),
    Currency::from("USD"),
    false,
    5,
    0,
    Price::from("0.00001"),
    Quantity::from("1"),
    None,
    None,
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

from nautilus_trader.model.currencies import EUR
from nautilus_trader.model.currencies import USD
from nautilus_trader.model.enums import AssetClass
from nautilus_trader.model.identifiers import InstrumentId
from nautilus_trader.model.identifiers import Symbol
from nautilus_trader.model.instruments import PerpetualContract
from nautilus_trader.model.objects import Price
from nautilus_trader.model.objects import Quantity

eurusd_perp = PerpetualContract(
    instrument_id=InstrumentId.from_str("EURUSD-PERP.AX"),
    raw_symbol=Symbol("EURUSD-PERP"),
    underlying="EURUSD",
    asset_class=AssetClass.FX,
    base_currency=EUR,
    quote_currency=USD,
    settlement_currency=USD,
    is_inverse=False,
    price_precision=5,
    size_precision=0,
    price_increment=Price.from_str("0.00001"),
    size_increment=Quantity.from_int(1),
    margin_init=Decimal("0.03"),
    margin_maint=Decimal("0.03"),
    maker_fee=Decimal("0.00002"),
    taker_fee=Decimal("0.00002"),
    ts_event=0,
    ts_init=0,
)
```

## 适配器 (Adapters)

创建或消费 `PerpetualContract` 工具的代表性适配器包括：

- [Architect AX](../../integrations/architect_ax.md)，用于交易场所定义的永续合约。

## 相关指南 (Related guides)

- [加密永续合约 (Crypto Perpetual)](crypto_perpetual.md) 介绍加密永续期货。
- [数据 (Data)](../data.md) 介绍标记价格、指数价格和资金费率更新。
