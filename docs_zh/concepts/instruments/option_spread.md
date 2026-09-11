# 期权价差 (Option Spread)

`OptionSpread` 表示由交易所定义、包含多于一条腿 (leg) 的期权策略。
交易场所会把该策略作为单个金融工具发布，并赋予它自己的代码 (symbol)、最小变动价位、
到期日和执行规则。

典型示例包括挂牌的垂直价差、日历价差以及其他期权策略。

## 字段 (Fields)

| 字段                 | Rust 类型            | Python 类型          | 必填/默认  | 说明                 |
| ------------------ | ------------------ | ------------------ | ------ | ------------------ |
| `instrument_id`    | `InstrumentId`     | `InstrumentId`     | 必填     | 在 Rust 中存储为 `id`。  |
| `raw_symbol`       | `Symbol`           | `Symbol`           | 必填     | 交易场所的原生代码。         |
| `asset_class`      | `AssetClass`       | `AssetClass`       | 必填     | 底层策略的资产类别。         |
| `exchange`         | `Option<Ustr>`     | `str \| None`      | `None` | 已知时填交易所 MIC 或场所代码。 |
| `underlying`       | `Ustr`             | `str`              | 必填     | 标的资产、期货或指数。        |
| `strategy_type`    | `Ustr`             | `str`              | 必填     | 交易场所的策略类型，例如垂直价差。  |
| `activation_ns`    | `UnixNanos`        | `int`              | 必填     | 策略生效时间戳。           |
| `expiration_ns`    | `UnixNanos`        | `int`              | 必填     | 策略到期时间戳。           |
| `currency`         | `Currency`         | `Currency`         | 必填     | 权利金报价与结算货币。        |
| `price_precision`  | `u8`               | `int`              | 必填     | 价格允许的小数位数。         |
| `price_increment`  | `Price`            | `Price`            | 必填     | 最小有效价格步长。          |
| `size_precision`   | `u8`               | `int`              | `0`    | 期权价差按整数张合约交易。      |
| `size_increment`   | `Quantity`         | `Quantity`         | `1`    | 最小合约数量步长。          |
| `multiplier`       | `Quantity`         | `Quantity`         | 必填     | 策略乘数。              |
| `lot_size`         | `Quantity`         | `Quantity`         | 必填     | 取整后的手数或合约手数。       |
| `margin_init`      | `Option<Decimal>`  | `Decimal \| None`  | `0`    | 初始保证金率。            |
| `margin_maint`     | `Option<Decimal>`  | `Decimal \| None`  | `0`    | 维持保证金率。            |
| `maker_fee`        | `Option<Decimal>`  | `Decimal \| None`  | `0`    | 做市方费率。负值表示返佣。      |
| `taker_fee`        | `Option<Decimal>`  | `Decimal \| None`  | `0`    | 吃单方费率。负值表示返佣。      |
| `max_quantity`     | `Option<Quantity>` | `Quantity \| None` | `None` | 最大订单数量。            |
| `min_quantity`     | `Option<Quantity>` | `Quantity \| None` | `1`    | 最小订单数量。            |
| `max_price`        | `Option<Price>`    | `Price \| None`    | `None` | 最大有效报价或订单价格。       |
| `min_price`        | `Option<Price>`    | `Price \| None`    | `None` | 最小有效报价或订单价格。       |
| `tick_scheme_name` | N/A                | `str \| None`      | `None` | 已注册的可变最小变动价位方案名称。  |
| `info`             | `Option<Params>`   | `dict \| None`     | `None` | 适配器元数据。            |
| `ts_event`         | `UnixNanos`        | `int`              | 必填     | 事件时间戳，单位纳秒。        |
| `ts_init`          | `UnixNanos`        | `int`              | 必填     | 初始化时间戳，单位纳秒。       |

*注：Python 构造函数使用 `instrument_id`；Rust 把同一个值存储为 `id`。*

## 行为 (Behavior)

- `OptionSpread` 的金融工具类别为 `OptionSpread`。
- 交易场所把该价差作为单个可交易的金融工具发布。
- 它按整数张合约交易，size precision 为 `0`，size increment 为 `1`。
- 当适配器提供交易场所特定的腿 (leg) 明细时，将其存储到 `info` 中。

## 示例 (Example)

```rust tab="Rust"
use chrono::{TimeZone, Utc};
use nautilus_core::UnixNanos;
use nautilus_model::{
    enums::AssetClass,
    identifiers::{InstrumentId, Symbol},
    instruments::OptionSpread,
    types::{Currency, Price, Quantity},
};
use ustr::Ustr;

let activation = Utc.with_ymd_and_hms(2023, 11, 6, 20, 54, 7).unwrap();
let expiration = Utc.with_ymd_and_hms(2024, 2, 23, 22, 59, 0).unwrap();

let sr3_spread = OptionSpread::new(
    InstrumentId::from("UD:U$: GN 2534559.GLBX"),
    Symbol::from("UD:U$: GN 2534559"),
    AssetClass::FX,
    Some(Ustr::from("XCME")),
    Ustr::from("SR3"),
    Ustr::from("GN"),
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
from nautilus_trader.model.instruments import OptionSpread
from nautilus_trader.model.objects import Price
from nautilus_trader.model.objects import Quantity

sr3_spread = OptionSpread(
    instrument_id=InstrumentId.from_str("UD:U$: GN 2534559.GLBX"),
    raw_symbol=Symbol("UD:U$: GN 2534559"),
    asset_class=AssetClass.FX,
    exchange="XCME",
    underlying="SR3",
    strategy_type="GN",
    activation_ns=pd.Timestamp("2023-11-06T20:54:07", tz="UTC").value,
    expiration_ns=pd.Timestamp("2024-02-23T22:59:00", tz="UTC").value,
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

会创建或消费 `OptionSpread` 金融工具的代表性适配器包括：

- [Databento](../../integrations/databento.md)，用于挂牌的期权价差市场。
- [Interactive Brokers](../../integrations/ib.md)，用于交易所定义的期权策略。

## 相关指南 (Related guides)

- [期权合约 (Option Contract)](option_contract.md) 介绍单腿期权合约。
- [期权 (Options)](../options.md) 介绍期权数据、希腊字母 (Greeks) 与期权链订阅。
