# 期权合约 (Option Contract)

`OptionContract` 表示一份基于非加密标的的挂牌看跌或看涨期权。它定义了期权类型、行权价、生效时间、到期时间、计价货币、乘数以及最小交易单位。

典型示例包括股票期权、指数期权和期货期权。

## 字段 (Fields)

| Field              | Rust type          | Python type        | Required/default | Notes              |
| ------------------ | ------------------ | ------------------ | ---------------- | ------------------ |
| `instrument_id`    | `InstrumentId`     | `InstrumentId`     | Required         | 在 Rust 中以 `id` 存储。 |
| `raw_symbol`       | `Symbol`           | `Symbol`           | Required         | 交易场所的原生代码。         |
| `asset_class`      | `AssetClass`       | `AssetClass`       | Required         | 标的的资产类别。           |
| `exchange`         | `Option<Ustr>`     | `str \| None`      | `None`           | 已知时为交易所 MIC 或场所代码。 |
| `underlying`       | `Ustr`             | `str`              | Required         | 标的资产、期货或指数。        |
| `option_kind`      | `OptionKind`       | `OptionKind`       | Required         | 看跌或看涨。             |
| `strike_price`     | `Price`            | `Price`            | Required         | 期权行权价。             |
| `activation_ns`    | `UnixNanos`        | `int`              | Required         | 合约生效时间戳。           |
| `expiration_ns`    | `UnixNanos`        | `int`              | Required         | 合约到期时间戳。           |
| `currency`         | `Currency`         | `Currency`         | Required         | 权利金报价与结算货币。        |
| `price_precision`  | `u8`               | `int`              | Required         | 价格允许的小数位数。         |
| `price_increment`  | `Price`            | `Price`            | Required         | 最小有效价格步长。          |
| `size_precision`   | `u8`               | `int`              | `0`              | 期权以整数张数交易。         |
| `size_increment`   | `Quantity`         | `Quantity`         | `1`              | 最小合约数量步长。          |
| `multiplier`       | `Quantity`         | `Quantity`         | Required         | 合约乘数。              |
| `lot_size`         | `Quantity`         | `Quantity`         | Required         | 取整后的手数或合约手数。       |
| `margin_init`      | `Option<Decimal>`  | `Decimal \| None`  | `0`              | 初始保证金率。            |
| `margin_maint`     | `Option<Decimal>`  | `Decimal \| None`  | `0`              | 维持保证金率。            |
| `maker_fee`        | `Option<Decimal>`  | `Decimal \| None`  | `0`              | Maker 费率。负值表示返佣。   |
| `taker_fee`        | `Option<Decimal>`  | `Decimal \| None`  | `0`              | Taker 费率。负值表示返佣。   |
| `max_quantity`     | `Option<Quantity>` | `Quantity \| None` | `None`           | 最大下单数量。            |
| `min_quantity`     | `Option<Quantity>` | `Quantity \| None` | `1`              | 最小下单数量。            |
| `max_price`        | `Option<Price>`    | `Price \| None`    | `None`           | 最大有效报价或下单价格。       |
| `min_price`        | `Option<Price>`    | `Price \| None`    | `None`           | 最小有效报价或下单价格。       |
| `tick_scheme_name` | N/A                | `str \| None`      | `None`           | 已注册的可变 tick 方案名称。  |
| `info`             | `Option<Params>`   | `dict \| None`     | `None`           | 适配器元数据。            |
| `ts_event`         | `UnixNanos`        | `int`              | Required         | 事件时间戳（纳秒）。         |
| `ts_init`          | `UnixNanos`        | `int`              | Required         | 初始化时间戳（纳秒）。        |

*注意：Python 构造函数使用 `instrument_id`；Rust 将同一值存储为 `id`。*

## 行为 (Behavior)

- `OptionContract` 的金融工具类别为 `Option`。
- 它以整数张数交易，数量精度为 `0`，数量步长为 `1`。
- 期权类型与行权价共同决定收益结构形态。
- 当标的与结算均为加密货币时，请使用 `CryptoOption`。

## 示例 (Example)

```rust tab="Rust"
use chrono::{TimeZone, Utc};
use nautilus_core::UnixNanos;
use nautilus_model::{
    enums::{AssetClass, OptionKind},
    identifiers::{InstrumentId, Symbol},
    instruments::OptionContract,
    types::{Currency, Price, Quantity},
};
use ustr::Ustr;

let activation = Utc.with_ymd_and_hms(2021, 9, 17, 0, 0, 0).unwrap();
let expiration = Utc.with_ymd_and_hms(2021, 12, 17, 0, 0, 0).unwrap();

let aapl_call = OptionContract::new(
    InstrumentId::from("AAPL211217C00150000.OPRA"),
    Symbol::from("AAPL211217C00150000"),
    AssetClass::Equity,
    Some(Ustr::from("GMNI")),
    Ustr::from("AAPL"),
    OptionKind::Call,
    Price::from("150.00"),
    Currency::from("USD"),
    UnixNanos::from(activation.timestamp_nanos_opt().unwrap() as u64),
    UnixNanos::from(expiration.timestamp_nanos_opt().unwrap() as u64),
    2,
    Price::from("0.01"),
    Quantity::from("100"),
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
from nautilus_trader.model.enums import OptionKind
from nautilus_trader.model.identifiers import InstrumentId
from nautilus_trader.model.identifiers import Symbol
from nautilus_trader.model.instruments import OptionContract
from nautilus_trader.model.objects import Price
from nautilus_trader.model.objects import Quantity

aapl_call = OptionContract(
    instrument_id=InstrumentId.from_str("AAPL211217C00150000.OPRA"),
    raw_symbol=Symbol("AAPL211217C00150000"),
    asset_class=AssetClass.EQUITY,
    exchange="GMNI",
    underlying="AAPL",
    option_kind=OptionKind.CALL,
    strike_price=Price.from_str("150.00"),
    currency=USD,
    price_precision=2,
    price_increment=Price.from_str("0.01"),
    multiplier=Quantity.from_int(100),
    lot_size=Quantity.from_int(1),
    activation_ns=pd.Timestamp("2021-09-17", tz="UTC").value,
    expiration_ns=pd.Timestamp("2021-12-17", tz="UTC").value,
    ts_event=0,
    ts_init=0,
)
```

## 适配器 (Adapters)

创建或消费 `OptionContract` 金融工具的代表性适配器包括：

- [Databento](../../integrations/databento.md)，用于挂牌期权数据。
- [Interactive Brokers](../../integrations/ib.md)，用于挂牌期权合约。

## 相关指南 (Related guides)

- [Options](../options.md) 涵盖期权数据、希腊字母（Greeks）以及期权链订阅。
- [Crypto Option](crypto_option.md) 涵盖加密期权合约。
