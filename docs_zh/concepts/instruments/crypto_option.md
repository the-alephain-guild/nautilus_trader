# 加密期权 (Crypto Option)

`CryptoOption` 表示以加密资产为标的的看跌或看涨期权。它定义了期权类型、执行价格、激活时间、到期时间、报价货币、结算货币以及合约规模。

例如加密衍生品交易场所上的 BTC 和 ETH 期权。

## 字段 (Fields)

| 字段                    | Rust 类型            | Python 类型          | 必填/默认值 | 说明                 |
| --------------------- | ------------------ | ------------------ | ------ | ------------------ |
| `instrument_id`       | `InstrumentId`     | `InstrumentId`     | 必填     | 在 Rust 中以 `id` 存储。 |
| `raw_symbol`          | `Symbol`           | `Symbol`           | 必填     | 交易场所原生代码。          |
| `underlying`          | `Currency`         | `Currency`         | 必填     | 期权追踪的加密资产。         |
| `quote_currency`      | `Currency`         | `Currency`         | 必填     | 用于为权利金报价的货币。       |
| `settlement_currency` | `Currency`         | `Currency`         | 必填     | 用于结算盈亏和费用的货币。      |
| `is_inverse`          | `bool`             | `bool`             | 必填     | 当规模/计价为反向时为 True。  |
| `option_kind`         | `OptionKind`       | `OptionKind`       | 必填     | 看跌或看涨。             |
| `strike_price`        | `Price`            | `Price`            | 必填     | 期权执行价格。            |
| `activation_ns`       | `UnixNanos`        | `int`              | 必填     | 合约激活时间戳。           |
| `expiration_ns`       | `UnixNanos`        | `int`              | 必填     | 合约到期时间戳。           |
| `price_precision`     | `u8`               | `int`              | 必填     | 价格允许的小数位数。         |
| `size_precision`      | `u8`               | `int`              | 必填     | 订单数量允许的小数位数。       |
| `price_increment`     | `Price`            | `Price`            | 必填     | 最小有效价格步长。          |
| `size_increment`      | `Quantity`         | `Quantity`         | 必填     | 最小有效数量步长。          |
| `multiplier`          | `Quantity`         | `Quantity`         | `1`    | 合约乘数。              |
| `lot_size`            | `Quantity`         | `Quantity`         | `1`    | 取整后的手数或交易板单位。      |
| `max_quantity`        | `Option<Quantity>` | `Quantity \| None` | `None` | 最大订单数量。            |
| `min_quantity`        | `Option<Quantity>` | `Quantity \| None` | `None` | 最小订单数量。            |
| `max_notional`        | `Option<Money>`    | `Money \| None`    | `None` | 最大订单名义价值。          |
| `min_notional`        | `Option<Money>`    | `Money \| None`    | `None` | 最小订单名义价值。          |
| `max_price`           | `Option<Price>`    | `Price \| None`    | `None` | 最大有效报价或订单价格。       |
| `min_price`           | `Option<Price>`    | `Price \| None`    | `None` | 最小有效报价或订单价格。       |
| `margin_init`         | `Option<Decimal>`  | `Decimal \| None`  | `0`    | 初始保证金率。            |
| `margin_maint`        | `Option<Decimal>`  | `Decimal \| None`  | `0`    | 维持保证金率。            |
| `maker_fee`           | `Option<Decimal>`  | `Decimal \| None`  | `0`    | 挂单方费率。负值表示返佣。      |
| `taker_fee`           | `Option<Decimal>`  | `Decimal \| None`  | `0`    | 吃单方费率。负值表示返佣。      |
| `tick_scheme_name`    | N/A                | `str \| None`      | `None` | 已注册的可变 tick 方案名称。  |
| `info`                | `Option<Params>`   | `dict \| None`     | `None` | 适配器元数据。            |
| `ts_event`            | `UnixNanos`        | `int`              | 必填     | 事件时间戳（纳秒）。         |
| `ts_init`             | `UnixNanos`        | `int`              | 必填     | 初始化时间戳（纳秒）。        |

*注意：Python 构造函数使用 `instrument_id`；Rust 将相同的值存储为 `id`。*

## 行为 (Behavior)

- `CryptoOption` 的资产类别为 `Cryptocurrency`，金融工具类别为 `Option`。
- 期权类型和执行价格定义了收益形态。
- 根据所设置的货币组合，合约可以是线性 (linear)、反向 (inverse) 或 quanto。
- 对于非加密的挂牌期权，请使用 `OptionContract`。

## 示例 (Example)

```rust tab="Rust"
use chrono::{TimeZone, Utc};
use nautilus_core::UnixNanos;
use nautilus_model::{
    enums::OptionKind,
    identifiers::{InstrumentId, Symbol},
    instruments::CryptoOption,
    types::{Currency, Money, Price, Quantity},
};
use rust_decimal_macros::dec;

let activation = Utc.with_ymd_and_hms(2022, 12, 22, 0, 0, 0).unwrap();
let expiration = Utc.with_ymd_and_hms(2023, 1, 13, 8, 0, 0).unwrap();

let btc_option = CryptoOption::new(
    InstrumentId::from("BTC-13JAN23-16000-P.DERIBIT"),
    Symbol::from("BTC-13JAN23-16000-P"),
    Currency::from("BTC"),
    Currency::from("USD"),
    Currency::from("BTC"),
    false,
    OptionKind::Put,
    Price::from("16000.00"),
    UnixNanos::from(activation.timestamp_nanos_opt().unwrap() as u64),
    UnixNanos::from(expiration.timestamp_nanos_opt().unwrap() as u64),
    2,
    1,
    Price::from("0.01"),
    Quantity::from("0.1"),
    Some(Quantity::from("1")),
    Some(Quantity::from("1")),
    Some(Quantity::from("9000")),
    Some(Quantity::from("0.1")),
    None,
    Some(Money::from("10.00 USD")),
    None,
    None,
    Some(dec!(0)),
    Some(dec!(0)),
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
from nautilus_trader.model.enums import OptionKind
from nautilus_trader.model.identifiers import InstrumentId
from nautilus_trader.model.identifiers import Symbol
from nautilus_trader.model.instruments import CryptoOption
from nautilus_trader.model.objects import Money
from nautilus_trader.model.objects import Price
from nautilus_trader.model.objects import Quantity

btc_option = CryptoOption(
    instrument_id=InstrumentId.from_str("BTC-13JAN23-16000-P.DERIBIT"),
    raw_symbol=Symbol("BTC-13JAN23-16000-P"),
    underlying=BTC,
    quote_currency=USD,
    settlement_currency=BTC,
    is_inverse=False,
    option_kind=OptionKind.PUT,
    strike_price=Price.from_str("16000.00"),
    activation_ns=pd.Timestamp("2022-12-22", tz="UTC").value,
    expiration_ns=pd.Timestamp("2023-01-13T08:00:00", tz="UTC").value,
    price_precision=2,
    size_precision=1,
    price_increment=Price.from_str("0.01"),
    size_increment=Quantity.from_str("0.1"),
    max_quantity=Quantity.from_str("9000"),
    min_quantity=Quantity.from_str("0.1"),
    min_notional=Money(10.00, USD),
    margin_init=Decimal(0),
    margin_maint=Decimal(0),
    maker_fee=Decimal("0.0003"),
    taker_fee=Decimal("0.0003"),
    ts_event=0,
    ts_init=0,
)
```

## 适配器 (Adapters)

创建或消费 `CryptoOption` 金融工具的代表性适配器包括：

- [Bybit](../../integrations/bybit.md)，用于加密期权。
- [Deribit](../../integrations/deribit.md)，用于加密期权。
- [OKX](../../integrations/okx.md)，用于加密期权。
- [Tardis](../../integrations/tardis.md)，用于加密期权元数据。

## 相关指南 (Related guides)

- [期权 (Options)](../options.md) 涵盖期权数据、希腊字母 (Greeks) 和期权链订阅。
- [加密期权价差 (Crypto Option Spread)](crypto_option_spread.md) 涵盖交易所定义的加密期权价差。
