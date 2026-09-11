# 加密期权价差 (Crypto Option Spread)

`CryptoOptionSpread` 表示一种由交易所定义、基于加密期权的策略。交易场所将该策略发布为单一工具，拥有自己的符号、策略类型、精度、最小变动单位以及到期时间。

典型示例包括加密衍生品交易场所上挂牌的 BTC 或 ETH 期权组合。

## 字段 (Fields)

| 字段                    | Rust 类型            | Python 类型          | 必填/默认值 | 说明                  |
| --------------------- | ------------------ | ------------------ | ------ | ------------------- |
| `instrument_id`       | `InstrumentId`     | `InstrumentId`     | 必填     | 在 Rust 中以 `id` 存储。  |
| `raw_symbol`          | `Symbol`           | `Symbol`           | 必填     | 交易场所原生符号。           |
| `underlying`          | `Currency`         | `Currency`         | 必填     | 该策略追踪的加密资产。         |
| `quote_currency`      | `Currency`         | `Currency`         | 必填     | 用于为权利金报价的货币。        |
| `settlement_currency` | `Currency`         | `Currency`         | 必填     | 用于结算盈亏与费用的货币。       |
| `is_inverse`          | `bool`             | `bool`             | 必填     | 当计价/计费方式为反向时为 True。 |
| `strategy_type`       | `Ustr`             | `str`              | 必填     | 交易场所策略类型，例如垂直价差。    |
| `activation_ns`       | `UnixNanos`        | `int`              | 必填     | 策略激活时间戳。            |
| `expiration_ns`       | `UnixNanos`        | `int`              | 必填     | 策略到期时间戳。            |
| `price_precision`     | `u8`               | `int`              | 必填     | 价格允许的小数位数。          |
| `size_precision`      | `u8`               | `int`              | 必填     | 订单数量允许的小数位数。        |
| `price_increment`     | `Price`            | `Price`            | 必填     | 最小有效价格变动步长。         |
| `size_increment`      | `Quantity`         | `Quantity`         | 必填     | 最小有效数量变动步长。         |
| `multiplier`          | `Quantity`         | `Quantity`         | `1`    | 策略乘数。               |
| `lot_size`            | `Quantity`         | `Quantity`         | `1`    | 取整后的手数或交易单位。        |
| `max_quantity`        | `Option<Quantity>` | `Quantity \| None` | `None` | 最大订单数量。             |
| `min_quantity`        | `Option<Quantity>` | `Quantity \| None` | `None` | 最小订单数量。             |
| `max_notional`        | `Option<Money>`    | `Money \| None`    | `None` | 最大订单名义价值。           |
| `min_notional`        | `Option<Money>`    | `Money \| None`    | `None` | 最小订单名义价值。           |
| `max_price`           | `Option<Price>`    | `Price \| None`    | `None` | 最大有效报价或订单价格。        |
| `min_price`           | `Option<Price>`    | `Price \| None`    | `None` | 最小有效报价或订单价格。        |
| `margin_init`         | `Option<Decimal>`  | `Decimal \| None`  | `0`    | 初始保证金率。             |
| `margin_maint`        | `Option<Decimal>`  | `Decimal \| None`  | `0`    | 维持保证金率。             |
| `maker_fee`           | `Option<Decimal>`  | `Decimal \| None`  | `0`    | 挂单费率。负值表示返佣。        |
| `taker_fee`           | `Option<Decimal>`  | `Decimal \| None`  | `0`    | 吃单费率。负值表示返佣。        |
| `tick_scheme_name`    | N/A                | `str \| None`      | `None` | 已注册的可变最小变动方案名称。     |
| `info`                | `Option<Params>`   | `dict \| None`     | `None` | 适配器元数据。             |
| `ts_event`            | `UnixNanos`        | `int`              | 必填     | 事件时间戳（纳秒）。          |
| `ts_init`             | `UnixNanos`        | `int`              | 必填     | 初始化时间戳（纳秒）。         |

*注意：Python 构造函数使用 `instrument_id`；Rust 将同一个值存储为 `id`。*

## 行为 (Behavior)

- `CryptoOptionSpread` 的资产类别为 `Cryptocurrency`，工具类别为
  `OptionSpread`。
- 交易场所将该价差发布为单一可交易工具。
- 根据所设定的货币组合，该策略可以是线性 (linear)、反向 (inverse) 或 quanto 类型。
- 当适配器提供时，将交易场所特定的腿（leg）细节存储在 `info` 中。

## 示例 (Example)

```rust tab="Rust"
use chrono::{TimeZone, Utc};
use nautilus_core::UnixNanos;
use nautilus_model::{
    identifiers::{InstrumentId, Symbol},
    instruments::CryptoOptionSpread,
    types::{Currency, Price, Quantity},
};
use rust_decimal_macros::dec;
use ustr::Ustr;

let activation = Utc.with_ymd_and_hms(2026, 5, 12, 0, 0, 0).unwrap();
let expiration = Utc.with_ymd_and_hms(2026, 5, 19, 8, 0, 0).unwrap();

let btc_spread = CryptoOptionSpread::new(
    InstrumentId::from("BTC-CS-19MAY26-70000_75000.DERIBIT"),
    Symbol::from("BTC-CS-19MAY26-70000_75000"),
    Currency::from("BTC"),
    Currency::from("USD"),
    Currency::from("BTC"),
    false,
    Ustr::from("CS"),
    UnixNanos::from(activation.timestamp_nanos_opt().unwrap() as u64),
    UnixNanos::from(expiration.timestamp_nanos_opt().unwrap() as u64),
    4,
    1,
    Price::from("0.0001"),
    Quantity::from("0.1"),
    Some(Quantity::from("1")),
    None,
    None,
    Some(Quantity::from("0.1")),
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
from nautilus_trader.model.instruments import CryptoOptionSpread
from nautilus_trader.model.objects import Price
from nautilus_trader.model.objects import Quantity

btc_spread = CryptoOptionSpread(
    instrument_id=InstrumentId.from_str("BTC-CS-19MAY26-70000_75000.DERIBIT"),
    raw_symbol=Symbol("BTC-CS-19MAY26-70000_75000"),
    underlying=BTC,
    quote_currency=USD,
    settlement_currency=BTC,
    is_inverse=False,
    strategy_type="CS",
    activation_ns=pd.Timestamp("2026-05-12T00:00:00", tz="UTC").value,
    expiration_ns=pd.Timestamp("2026-05-19T08:00:00", tz="UTC").value,
    price_precision=4,
    size_precision=1,
    price_increment=Price.from_str("0.0001"),
    size_increment=Quantity.from_str("0.1"),
    min_quantity=Quantity.from_str("0.1"),
    maker_fee=Decimal("0.0003"),
    taker_fee=Decimal("0.0003"),
    ts_event=0,
    ts_init=0,
)
```

## 适配器 (Adapters)

会创建或消费 `CryptoOptionSpread` 工具的代表性适配器包括：

- [Deribit](../../integrations/deribit.md)，用于加密期权组合。
- [OKX](../../integrations/okx.md)，用于加密期权价差市场。

## 相关指南 (Related guides)

- [加密期权 (Crypto Option)](crypto_option.md) 介绍单腿加密期权。
- [期权价差 (Option Spread)](option_spread.md) 介绍非加密期权价差。
