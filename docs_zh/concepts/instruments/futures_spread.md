# 期货价差 (Futures Spread)

`FuturesSpread` 表示由交易所定义的、包含多于一条腿的期货策略，例如跨期价差 (calendar spread) 或跨品种价差 (inter-commodity spread)。交易场所负责定义该策略、符号、最小变动价位以及到期日。

典型的例子包括挂牌交易的期货跨期价差，以及交易所支持的各类价差市场。

## 字段 (Fields)

| 字段                 | Rust 类型            | Python 类型          | 必填/默认值 | 说明                         |
| ------------------ | ------------------ | ------------------ | ------ | -------------------------- |
| `instrument_id`    | `InstrumentId`     | `InstrumentId`     | 必填     | 在 Rust 中以 `id` 存储。         |
| `raw_symbol`       | `Symbol`           | `Symbol`           | 必填     | 交易场所的原生符号。                 |
| `asset_class`      | `AssetClass`       | `AssetClass`       | 必填     | 底层策略的资产类别。                 |
| `exchange`         | `Option<Ustr>`     | `str \| None`      | `None` | 已知时的交易所 MIC 或场所代码。         |
| `underlying`       | `Ustr`             | `str`              | 必填     | 底层产品或产品族。                  |
| `strategy_type`    | `Ustr`             | `str`              | 必填     | 交易场所的策略类型，例如跨期 (calendar)。 |
| `activation_ns`    | `UnixNanos`        | `int`              | 必填     | 策略生效时间戳。                   |
| `expiration_ns`    | `UnixNanos`        | `int`              | 必填     | 策略到期时间戳。                   |
| `currency`         | `Currency`         | `Currency`         | 必填     | 报价及结算货币。                   |
| `price_precision`  | `u8`               | `int`              | 必填     | 价格允许的小数位数。                 |
| `price_increment`  | `Price`            | `Price`            | 必填     | 最小有效价格步长。                  |
| `size_precision`   | `u8`               | `int`              | `0`    | 期货价差以整数张合约交易。              |
| `size_increment`   | `Quantity`         | `Quantity`         | `1`    | 最小合约数量步长。                  |
| `multiplier`       | `Quantity`         | `Quantity`         | 必填     | 策略乘数。                      |
| `lot_size`         | `Quantity`         | `Quantity`         | 必填     | 取整后的手数或合约手数。               |
| `margin_init`      | `Option<Decimal>`  | `Decimal \| None`  | `0`    | 初始保证金率。                    |
| `margin_maint`     | `Option<Decimal>`  | `Decimal \| None`  | `0`    | 维持保证金率。                    |
| `maker_fee`        | `Option<Decimal>`  | `Decimal \| None`  | `0`    | 挂单费率。负值表示返佣。               |
| `taker_fee`        | `Option<Decimal>`  | `Decimal \| None`  | `0`    | 吃单费率。负值表示返佣。               |
| `max_quantity`     | `Option<Quantity>` | `Quantity \| None` | `None` | 最大订单数量。                    |
| `min_quantity`     | `Option<Quantity>` | `Quantity \| None` | `1`    | 最小订单数量。                    |
| `max_price`        | `Option<Price>`    | `Price \| None`    | `None` | 最大有效报价或订单价格。               |
| `min_price`        | `Option<Price>`    | `Price \| None`    | `None` | 最小有效报价或订单价格。               |
| `tick_scheme_name` | N/A                | `str \| None`      | `None` | 已注册的可变 tick 方案名称。          |
| `info`             | `Option<Params>`   | `dict \| None`     | `None` | 适配器元数据。                    |
| `ts_event`         | `UnixNanos`        | `int`              | 必填     | 事件时间戳（纳秒）。                 |
| `ts_init`          | `UnixNanos`        | `int`              | 必填     | 初始化时间戳（纳秒）。                |

*注意：Python 构造函数使用 `instrument_id`；Rust 将同一值存储为 `id`。*

## 行为 (Behavior)

- `FuturesSpread` 的工具类别 (instrument class) 为 `FuturesSpread`。
- 交易场所将该价差作为单一可交易工具发布。
- 它以整数张合约交易，size precision 为 `0`，size increment 为 `1`。
- 当某个策略需要场所特定的腿 (leg) 细节时，请使用来自适配器元数据的腿数据。

## 示例 (Example)

```rust tab="Rust"
use chrono::{TimeZone, Utc};
use nautilus_core::UnixNanos;
use nautilus_model::{
    enums::AssetClass,
    identifiers::{InstrumentId, Symbol},
    instruments::FuturesSpread,
    types::{Currency, Price, Quantity},
};
use ustr::Ustr;

let activation = Utc.with_ymd_and_hms(2022, 6, 21, 13, 30, 0).unwrap();
let expiration = Utc.with_ymd_and_hms(2024, 6, 21, 13, 30, 0).unwrap();

let es_spread = FuturesSpread::new(
    InstrumentId::from("ESM4-ESU4.GLBX"),
    Symbol::from("ESM4-ESU4"),
    AssetClass::Index,
    Some(Ustr::from("XCME")),
    Ustr::from("ES"),
    Ustr::from("EQ"),
    UnixNanos::from(activation.timestamp_nanos_opt().unwrap() as u64),
    UnixNanos::from(expiration.timestamp_nanos_opt().unwrap() as u64),
    Currency::from("USD"),
    2,
    Price::from("0.01"),
    Quantity::from("1"),
    Quantity::from("1"),
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
import pandas as pd

from nautilus_trader.model.currencies import USD
from nautilus_trader.model.enums import AssetClass
from nautilus_trader.model.identifiers import InstrumentId
from nautilus_trader.model.identifiers import Symbol
from nautilus_trader.model.instruments import FuturesSpread
from nautilus_trader.model.objects import Price
from nautilus_trader.model.objects import Quantity

es_spread = FuturesSpread(
    instrument_id=InstrumentId.from_str("ESM4-ESU4.GLBX"),
    raw_symbol=Symbol("ESM4-ESU4"),
    asset_class=AssetClass.INDEX,
    exchange="XCME",
    underlying="ES",
    strategy_type="EQ",
    activation_ns=pd.Timestamp("2022-06-21T13:30:00", tz="UTC").value,
    expiration_ns=pd.Timestamp("2024-06-21T13:30:00", tz="UTC").value,
    currency=USD,
    price_precision=2,
    price_increment=Price.from_str("0.01"),
    multiplier=Quantity.from_int(1),
    lot_size=Quantity.from_int(1),
    ts_event=0,
    ts_init=0,
)
```

## 适配器 (Adapters)

会创建或消费 `FuturesSpread` 工具的代表性适配器包括：

- [Databento](../../integrations/databento.md)，用于挂牌交易的期货价差市场。
- [Interactive Brokers](../../integrations/ib.md)，用于交易所定义的期货策略。

## 相关指南 (Related guides)

- [期货合约 (Futures Contract)](futures_contract.md) 介绍单腿期货。
- [连续期货 (Continuous Futures)](../continuous_futures.md) 介绍经过移仓调整的期货序列。
