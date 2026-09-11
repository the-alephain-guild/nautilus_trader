# 期货合约 (Futures Contract)

`FuturesContract` 表示一份有到期日、在交易所交易的期货合约，它定义了标的物、上市（激活）时间、到期时间、币种、合约乘数以及手数。

常见示例包括股指期货、商品期货、利率期货以及货币期货。

## 字段 (Fields)

| 字段                 | Rust 类型            | Python 类型          | 必填/默认值 | 说明                  |
| ------------------ | ------------------ | ------------------ | ------ | ------------------- |
| `instrument_id`    | `InstrumentId`     | `InstrumentId`     | 必填     | 在 Rust 中存储为 `id`。   |
| `raw_symbol`       | `Symbol`           | `Symbol`           | 必填     | 交易场所原生代码。           |
| `asset_class`      | `AssetClass`       | `AssetClass`       | 必填     | 标的物的资产类别。           |
| `exchange`         | `Option<Ustr>`     | `str \| None`      | `None` | 已知时填交易所 MIC 或场所代码。  |
| `underlying`       | `Ustr`             | `str`              | 必填     | 标的资产、指数或产品。         |
| `activation_ns`    | `UnixNanos`        | `int`              | 必填     | 合约激活（上市）时间戳。        |
| `expiration_ns`    | `UnixNanos`        | `int`              | 必填     | 合约到期时间戳。            |
| `currency`         | `Currency`         | `Currency`         | 必填     | 报价与结算币种。            |
| `price_precision`  | `u8`               | `int`              | 必填     | 价格允许的小数位数。          |
| `price_increment`  | `Price`            | `Price`            | 必填     | 最小有效价格步长。           |
| `size_precision`   | `u8`               | `int`              | `0`    | 期货以整数合约张数交易。        |
| `size_increment`   | `Quantity`         | `Quantity`         | `1`    | 最小合约数量步长。           |
| `multiplier`       | `Quantity`         | `Quantity`         | 必填     | 合约乘数。               |
| `lot_size`         | `Quantity`         | `Quantity`         | 必填     | 取整后的手数或合约批量大小。      |
| `margin_init`      | `Option<Decimal>`  | `Decimal \| None`  | `0`    | 初始保证金率。             |
| `margin_maint`     | `Option<Decimal>`  | `Decimal \| None`  | `0`    | 维持保证金率。             |
| `maker_fee`        | `Option<Decimal>`  | `Decimal \| None`  | `0`    | 挂单（maker）费率，负值表示返佣。 |
| `taker_fee`        | `Option<Decimal>`  | `Decimal \| None`  | `0`    | 吃单（taker）费率，负值表示返佣。 |
| `max_quantity`     | `Option<Quantity>` | `Quantity \| None` | `None` | 最大下单数量。             |
| `min_quantity`     | `Option<Quantity>` | `Quantity \| None` | `1`    | 最小下单数量。             |
| `max_price`        | `Option<Price>`    | `Price \| None`    | `None` | 最大有效报价或下单价格。        |
| `min_price`        | `Option<Price>`    | `Price \| None`    | `None` | 最小有效报价或下单价格。        |
| `tick_scheme_name` | N/A                | `str \| None`      | `None` | 已注册的可变 tick 方案名称。   |
| `info`             | `Option<Params>`   | `dict \| None`     | `None` | 适配器元数据。             |
| `ts_event`         | `UnixNanos`        | `int`              | 必填     | 事件时间戳（纳秒）。          |
| `ts_init`          | `UnixNanos`        | `int`              | 必填     | 初始化时间戳（纳秒）。         |

*注意：Python 构造函数使用 `instrument_id`；Rust 将同一值存储为 `id`。*

## 行为 (Behavior)

- `FuturesContract` 的工具类别为 `Future`。
- 它永远不是反向（inverse）合约。成本、结算与报价币种均使用 `currency`。
- 它以整数合约张数交易，size 精度为 `0`、size 步长为 `1`。
- 对于标的币种与结算币种可能不同的有到期日加密货币期货，请使用 `CryptoFuture`。

## 示例 (Example)

```rust tab="Rust"
use chrono::{TimeZone, Utc};
use nautilus_core::UnixNanos;
use nautilus_model::{
    enums::AssetClass,
    identifiers::{InstrumentId, Symbol},
    instruments::FuturesContract,
    types::{Currency, Price, Quantity},
};
use ustr::Ustr;

let activation = Utc.with_ymd_and_hms(2021, 9, 10, 0, 0, 0).unwrap();
let expiration = Utc.with_ymd_and_hms(2021, 12, 17, 0, 0, 0).unwrap();

let esz21 = FuturesContract::new(
    InstrumentId::from("ESZ21.GLBX"),
    Symbol::from("ESZ21"),
    AssetClass::Index,
    Some(Ustr::from("XCME")),
    Ustr::from("ES"),
    UnixNanos::from(activation.timestamp_nanos_opt().unwrap() as u64),
    UnixNanos::from(expiration.timestamp_nanos_opt().unwrap() as u64),
    Currency::from("USD"),
    2,
    Price::from("0.25"),
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
from nautilus_trader.model.instruments import FuturesContract
from nautilus_trader.model.objects import Price
from nautilus_trader.model.objects import Quantity

esz21 = FuturesContract(
    instrument_id=InstrumentId.from_str("ESZ21.GLBX"),
    raw_symbol=Symbol("ESZ21"),
    asset_class=AssetClass.INDEX,
    exchange="XCME",
    underlying="ES",
    currency=USD,
    price_precision=2,
    price_increment=Price.from_str("0.25"),
    multiplier=Quantity.from_int(1),
    lot_size=Quantity.from_int(1),
    activation_ns=pd.Timestamp("2021-09-10", tz="UTC").value,
    expiration_ns=pd.Timestamp("2021-12-17", tz="UTC").value,
    ts_event=0,
    ts_init=0,
)
```

## 适配器 (Adapters)

创建或消费 `FuturesContract` 工具的代表性适配器包括：

- [Databento](../../integrations/databento.md)，提供期货参考数据与行情数据。
- [Interactive Brokers](../../integrations/ib.md)，提供已上市的期货合约。

## 相关指南 (Related guides)

- [Continuous Futures](../continuous_futures.md) 介绍经过移仓调整的连续期货序列。
- [Crypto Future](crypto_future.md) 介绍有到期日的加密货币期货合约。
