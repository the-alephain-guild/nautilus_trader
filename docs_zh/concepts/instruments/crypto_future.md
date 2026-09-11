# 加密期货 (Crypto Future)

`CryptoFuture` 表示一份有到期日的加密货币期货合约。它跟踪一种加密标的资产，以某种报价货币计价，以某种结算货币结算，并在固定的时间戳到期。

典型例子包括加密衍生品交易所上有到期日的 BTC 或 ETH 期货。

## 字段 (Fields)

| 字段                    | Rust 类型            | Python 类型          | 必填/默认值 | 说明                    |
| --------------------- | ------------------ | ------------------ | ------ | --------------------- |
| `instrument_id`       | `InstrumentId`     | `InstrumentId`     | 必填     | 在 Rust 中以 `id` 存储。    |
| `raw_symbol`          | `Symbol`           | `Symbol`           | 必填     | 交易所原生符号。              |
| `underlying`          | `Currency`         | `Currency`         | 必填     | 该合约所跟踪的加密资产。          |
| `quote_currency`      | `Currency`         | `Currency`         | 必填     | 用于报价的货币。              |
| `settlement_currency` | `Currency`         | `Currency`         | 必填     | 用于结算盈亏和费用的货币。         |
| `is_inverse`          | `bool`             | `bool`             | 必填     | 为 True 时，计量/计价采用反向方式。 |
| `activation_ns`       | `UnixNanos`        | `int`              | 必填     | 合约激活时间戳。              |
| `expiration_ns`       | `UnixNanos`        | `int`              | 必填     | 合约到期时间戳。              |
| `price_precision`     | `u8`               | `int`              | 必填     | 价格允许的小数位数。            |
| `size_precision`      | `u8`               | `int`              | 必填     | 订单数量允许的小数位数。          |
| `price_increment`     | `Price`            | `Price`            | 必填     | 最小有效价格步进。             |
| `size_increment`      | `Quantity`         | `Quantity`         | 必填     | 最小有效数量步进。             |
| `multiplier`          | `Quantity`         | `Quantity`         | `1`    | 合约乘数。                 |
| `lot_size`            | `Quantity`         | `Quantity`         | `1`    | 取整后的手数或板块规模。          |
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

*注意：Python 构造函数使用 `instrument_id`；Rust 将同一值存储为 `id`。*

## 行为 (Behavior)

- `CryptoFuture` 的资产类别为 `Cryptocurrency`，工具类别为 `Future`。
- 线性合约通常设置 `is_inverse=False`，并以报价货币结算。
- 反向合约设置 `is_inverse=True`，通常以标的货币结算。
- Quanto 合约以第三种货币结算，该货币既不同于标的货币也不同于报价货币。
- 对于没有到期日的加密衍生品，请使用 `CryptoPerpetual`。

## 示例 (Example)

```rust tab="Rust"
use chrono::{TimeZone, Utc};
use nautilus_core::UnixNanos;
use nautilus_model::{
    identifiers::{InstrumentId, Symbol},
    instruments::CryptoFuture,
    types::{Currency, Money, Price, Quantity},
};

let activation = Utc.with_ymd_and_hms(2024, 1, 8, 0, 0, 0).unwrap();
let expiration = Utc.with_ymd_and_hms(2024, 3, 29, 0, 0, 0).unwrap();

let btcusdt_future = CryptoFuture::new(
    InstrumentId::from("BTCUSDT-240329.BINANCE"),
    Symbol::from("BTCUSDT-240329"),
    Currency::from("BTC"),
    Currency::from("USDT"),
    Currency::from("USDT"),
    false,
    UnixNanos::from(activation.timestamp_nanos_opt().unwrap() as u64),
    UnixNanos::from(expiration.timestamp_nanos_opt().unwrap() as u64),
    2,
    6,
    Price::from("0.01"),
    Quantity::from("0.000001"),
    None,
    None,
    Some(Quantity::from("9000.0")),
    Some(Quantity::from("0.000001")),
    None,
    Some(Money::from("10.00 USDT")),
    Some(Price::from("1000000.00")),
    Some(Price::from("0.01")),
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

from nautilus_trader.model.currencies import BTC
from nautilus_trader.model.currencies import USDT
from nautilus_trader.model.identifiers import InstrumentId
from nautilus_trader.model.identifiers import Symbol
from nautilus_trader.model.instruments import CryptoFuture
from nautilus_trader.model.objects import Money
from nautilus_trader.model.objects import Price
from nautilus_trader.model.objects import Quantity

btcusdt_future = CryptoFuture(
    instrument_id=InstrumentId.from_str("BTCUSDT-240329.BINANCE"),
    raw_symbol=Symbol("BTCUSDT-240329"),
    underlying=BTC,
    quote_currency=USDT,
    settlement_currency=USDT,
    is_inverse=False,
    activation_ns=pd.Timestamp("2024-01-08", tz="UTC").value,
    expiration_ns=pd.Timestamp("2024-03-29", tz="UTC").value,
    price_precision=2,
    size_precision=6,
    price_increment=Price.from_str("0.01"),
    size_increment=Quantity.from_str("0.000001"),
    max_quantity=Quantity.from_str("9000"),
    min_quantity=Quantity.from_str("0.000001"),
    min_notional=Money(10.00, USDT),
    max_price=Price.from_str("1000000.00"),
    min_price=Price.from_str("0.01"),
    ts_event=0,
    ts_init=0,
)
```

## 适配器 (Adapters)

创建或消费 `CryptoFuture` 工具的代表性适配器包括：

- [BitMEX](../../integrations/bitmex.md)，用于反向和线性的有到期日期货。
- [Bybit](../../integrations/bybit.md)，用于加密期货市场。
- [Deribit](../../integrations/deribit.md)，用于有到期日的加密期货。
- [OKX](../../integrations/okx.md)，用于有到期日的加密期货。
- [Tardis](../../integrations/tardis.md)，用于加密期货元数据。

## 相关指南 (Related guides)

- [加密永续合约](crypto_perpetual.md) 介绍永续加密期货。
- [期货合约](futures_contract.md) 介绍非加密类期货合约。
