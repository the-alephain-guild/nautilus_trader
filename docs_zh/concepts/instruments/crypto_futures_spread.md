# 加密期货价差 (Crypto Futures Spread)

`CryptoFuturesSpread` 表示交易所定义的、基于加密期货的价差策略。交易所将该策略作为单个工具发布，它拥有自己的符号、策略类型、精度、增量以及到期时间。

典型例子包括挂牌交易的加密期货日历价差 (calendar spread)。

## 字段

| 字段                    | Rust 类型            | Python 类型          | 必填/默认值 | 说明                    |
| --------------------- | ------------------ | ------------------ | ------ | --------------------- |
| `instrument_id`       | `InstrumentId`     | `InstrumentId`     | 必填     | 在 Rust 中以 `id` 存储。    |
| `raw_symbol`          | `Symbol`           | `Symbol`           | 必填     | 交易所原生符号。              |
| `underlying`          | `Currency`         | `Currency`         | 必填     | 该策略所跟踪的加密资产。          |
| `quote_currency`      | `Currency`         | `Currency`         | 必填     | 用于报价的货币。              |
| `settlement_currency` | `Currency`         | `Currency`         | 必填     | 用于结算盈亏与费用的货币。         |
| `is_inverse`          | `bool`             | `bool`             | 必填     | 当头寸规模/成本计算为反向时为 True。 |
| `strategy_type`       | `Ustr`             | `str`              | 必填     | 交易所策略类型，例如日历价差。       |
| `activation_ns`       | `UnixNanos`        | `int`              | 必填     | 策略激活时间戳。              |
| `expiration_ns`       | `UnixNanos`        | `int`              | 必填     | 策略到期时间戳。              |
| `price_precision`     | `u8`               | `int`              | 必填     | 价格允许的小数位数。            |
| `size_precision`      | `u8`               | `int`              | 必填     | 订单数量允许的小数位数。          |
| `price_increment`     | `Price`            | `Price`            | 必填     | 最小有效价格步进。             |
| `size_increment`      | `Quantity`         | `Quantity`         | 必填     | 最小有效数量步进。             |
| `multiplier`          | `Quantity`         | `Quantity`         | `1`    | 策略乘数。                 |
| `lot_size`            | `Quantity`         | `Quantity`         | `1`    | 取整后的手数或挂牌单位。          |
| `max_quantity`        | `Option<Quantity>` | `Quantity \| None` | `None` | 最大订单数量。               |
| `min_quantity`        | `Option<Quantity>` | `Quantity \| None` | `None` | 最小订单数量。               |
| `max_notional`        | `Option<Money>`    | `Money \| None`    | `None` | 最大订单名义价值。             |
| `min_notional`        | `Option<Money>`    | `Money \| None`    | `None` | 最小订单名义价值。             |
| `max_price`           | `Option<Price>`    | `Price \| None`    | `None` | 最大有效报价或订单价格。          |
| `min_price`           | `Option<Price>`    | `Price \| None`    | `None` | 最小有效报价或订单价格。          |
| `margin_init`         | `Option<Decimal>`  | `Decimal \| None`  | `0`    | 初始保证金率。               |
| `margin_maint`        | `Option<Decimal>`  | `Decimal \| None`  | `0`    | 维持保证金率。               |
| `maker_fee`           | `Option<Decimal>`  | `Decimal \| None`  | `0`    | Maker 费率。负值表示返佣。      |
| `taker_fee`           | `Option<Decimal>`  | `Decimal \| None`  | `0`    | Taker 费率。负值表示返佣。      |
| `tick_scheme_name`    | N/A                | `str \| None`      | `None` | 已注册的可变 tick 方案名称。     |
| `info`                | `Option<Params>`   | `dict \| None`     | `None` | 适配器元数据。               |
| `ts_event`            | `UnixNanos`        | `int`              | 必填     | 事件时间戳（纳秒）。            |
| `ts_init`             | `UnixNanos`        | `int`              | 必填     | 初始化时间戳（纳秒）。           |

*注意：Python 构造函数使用 `instrument_id`；Rust 将同一值以 `id` 存储。*

## 行为

- `CryptoFuturesSpread` 的资产类别 (asset class) 为 `Cryptocurrency`，工具类别 (instrument class) 为
  `FuturesSpread`。
- 交易所将该价差作为单个可交易工具发布。
- 该策略可以是线性 (linear)、反向 (inverse) 或 quanto，取决于货币组合的设置。
- 当适配器提供交易所专有的腿 (leg) 细节时，将其存储在 `info` 中。

## 示例

```rust tab="Rust"
use chrono::{TimeZone, Utc};
use nautilus_core::UnixNanos;
use nautilus_model::{
    identifiers::{InstrumentId, Symbol},
    instruments::CryptoFuturesSpread,
    types::{Currency, Price, Quantity},
};
use rust_decimal_macros::dec;
use ustr::Ustr;

let activation = Utc.with_ymd_and_hms(2026, 5, 12, 0, 0, 0).unwrap();
let expiration = Utc.with_ymd_and_hms(2026, 5, 19, 8, 0, 0).unwrap();

let btc_spread = CryptoFuturesSpread::new(
    InstrumentId::from("BTC-FS-19MAY26_PERP.DERIBIT"),
    Symbol::from("BTC-FS-19MAY26_PERP"),
    Currency::from("BTC"),
    Currency::from("USD"),
    Currency::from("BTC"),
    false,
    Ustr::from("FS"),
    UnixNanos::from(activation.timestamp_nanos_opt().unwrap() as u64),
    UnixNanos::from(expiration.timestamp_nanos_opt().unwrap() as u64),
    1,
    0,
    Price::from("0.5"),
    Quantity::from("1"),
    Some(Quantity::from("10")),
    Some(Quantity::from("1")),
    None,
    Some(Quantity::from("1")),
    None,
    None,
    None,
    None,
    None,
    None,
    Some(dec!(0.0003)),
    Some(dec!(0.0003)),
    None,
    UnixNanos::default(),
    UnixNanos::default(),
);
```

```python tab="Python"
from decimal import Decimal

import pandas as pd

from nautilus_trader.model.currencies import BTC
from nautilus_trader.model.currencies import USD
from nautilus_trader.model.identifiers import InstrumentId
from nautilus_trader.model.identifiers import Symbol
from nautilus_trader.model.instruments import CryptoFuturesSpread
from nautilus_trader.model.objects import Price
from nautilus_trader.model.objects import Quantity

btc_spread = CryptoFuturesSpread(
    instrument_id=InstrumentId.from_str("BTC-FS-19MAY26_PERP.DERIBIT"),
    raw_symbol=Symbol("BTC-FS-19MAY26_PERP"),
    underlying=BTC,
    quote_currency=USD,
    settlement_currency=BTC,
    is_inverse=False,
    strategy_type="FS",
    activation_ns=pd.Timestamp("2026-05-12T00:00:00", tz="UTC").value,
    expiration_ns=pd.Timestamp("2026-05-19T08:00:00", tz="UTC").value,
    price_precision=1,
    size_precision=0,
    price_increment=Price.from_str("0.5"),
    size_increment=Quantity.from_int(1),
    multiplier=Quantity.from_int(10),
    lot_size=Quantity.from_int(1),
    min_quantity=Quantity.from_int(1),
    maker_fee=Decimal("0.0003"),
    taker_fee=Decimal("0.0003"),
    ts_event=0,
    ts_init=0,
)
```

## 适配器

会创建或消费 `CryptoFuturesSpread` 工具的代表性适配器包括：

- [Deribit](../../integrations/deribit.md)，用于加密期货组合 (combo)。
- [OKX](../../integrations/okx.md)，用于加密期货价差市场。

## 相关指南

- [加密期货 (Crypto Future)](crypto_future.md) 介绍单腿的定期加密期货。
- [期货价差 (Futures Spread)](futures_spread.md) 介绍非加密类的期货价差。
