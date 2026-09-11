# 二元期权 (Binary Option)

`BinaryOption` 表示一种二元结果工具，它根据某个条件是否为真，结算为固定的收益。它可以用于建模预测市场、二元期权，或场所特定的"是/否"合约。

典型例子包括预测市场的结果以及二元事件合约。

## 字段 (Fields)

| 字段                 | Rust 类型            | Python 类型          | 必填/默认值 | 说明                 |
| ------------------ | ------------------ | ------------------ | ------ | ------------------ |
| `instrument_id`    | `InstrumentId`     | `InstrumentId`     | 必填     | 在 Rust 中以 `id` 存储。 |
| `raw_symbol`       | `Symbol`           | `Symbol`           | 必填     | 场所原生交易代码。          |
| `asset_class`      | `AssetClass`       | `AssetClass`       | 必填     | 结果市场的资产类别。         |
| `currency`         | `Currency`         | `Currency`         | 必填     | 报价与结算货币。           |
| `activation_ns`    | `UnixNanos`        | `int`              | 必填     | 合约激活时间戳。           |
| `expiration_ns`    | `UnixNanos`        | `int`              | 必填     | 合约到期时间戳。           |
| `price_precision`  | `u8`               | `int`              | 必填     | 价格允许的小数位数。         |
| `size_precision`   | `u8`               | `int`              | 必填     | 订单数量允许的小数位数。       |
| `price_increment`  | `Price`            | `Price`            | 必填     | 最小有效价格步进。          |
| `size_increment`   | `Quantity`         | `Quantity`         | 必填     | 最小有效数量步进。          |
| `outcome`          | `Option<Ustr>`     | `str \| None`      | `None` | 场所提供时的结果标签。        |
| `description`      | `Option<Ustr>`     | `str \| None`      | `None` | 人类可读的市场描述。         |
| `max_quantity`     | `Option<Quantity>` | `Quantity \| None` | `None` | 最大订单数量。            |
| `min_quantity`     | `Option<Quantity>` | `Quantity \| None` | `None` | 最小订单数量。            |
| `max_notional`     | `Option<Money>`    | N/A                | 仅 Rust | 最大订单名义价值。          |
| `min_notional`     | `Option<Money>`    | N/A                | 仅 Rust | 最小订单名义价值。          |
| `max_price`        | `Option<Price>`    | N/A                | 仅 Rust | 最大有效报价或订单价格。       |
| `min_price`        | `Option<Price>`    | N/A                | 仅 Rust | 最小有效报价或订单价格。       |
| `margin_init`      | `Option<Decimal>`  | N/A                | 仅 Rust | 初始保证金率。            |
| `margin_maint`     | `Option<Decimal>`  | N/A                | 仅 Rust | 维持保证金率。            |
| `maker_fee`        | `Option<Decimal>`  | `Decimal \| None`  | `0`    | 挂单方费率。负值表示返佣。      |
| `taker_fee`        | `Option<Decimal>`  | `Decimal \| None`  | `0`    | 吃单方费率。负值表示返佣。      |
| `tick_scheme_name` | N/A                | `str \| None`      | `None` | 已注册的可变 tick 方案名称。  |
| `info`             | `Option<Params>`   | `dict \| None`     | `None` | 适配器元数据。            |
| `ts_event`         | `UnixNanos`        | `int`              | 必填     | 事件时间戳（纳秒）。         |
| `ts_init`          | `UnixNanos`        | `int`              | 必填     | 初始化时间戳（纳秒）。        |

*注意：Python 构造函数使用 `instrument_id`；Rust 将同一值存储为 `id`。*

## 行为 (Behavior)

- `BinaryOption` 的工具类别为 `BinaryOption`。
- 它永远不是反向 (inverse) 的，并且乘数和手数 (lot size) 均为一。
- 许多场所将二元结果在零到一之间报价，但允许的价格范围与 tick 大小由场所定义。
- `outcome` 和 `description` 为合约提供人类可读的上下文。

## 示例 (Example)

```rust tab="Rust"
use chrono::{TimeZone, Utc};
use nautilus_core::UnixNanos;
use nautilus_model::{
    enums::AssetClass,
    identifiers::{InstrumentId, Symbol, Venue},
    instruments::BinaryOption,
    types::{Currency, Price, Quantity},
};
use rust_decimal_macros::dec;
use ustr::Ustr;

let raw_symbol = Symbol::from(
    "0x12a0cb60174abc437bf1178367c72d11f069e1a3add20b148fb0ab4279b772b2-92544998123698303655208967887569360731013655782348975589292031774495159624905",
);
let expiration = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();

let yes_outcome = BinaryOption::new(
    InstrumentId::new(raw_symbol, Venue::from("POLYMARKET")),
    raw_symbol,
    AssetClass::Alternative,
    Currency::from("USDC"),
    UnixNanos::default(),
    UnixNanos::from(expiration.timestamp_nanos_opt().unwrap() as u64),
    3,
    2,
    Price::from("0.001"),
    Quantity::from("0.01"),
    Some(Ustr::from("Yes")),
    Some(Ustr::from("Will the outcome of this market be 'Yes'?")),
    None,
    Some(Quantity::from("5")),
    None,
    None,
    None,
    None,
    None,
    None,
    Some(dec!(0)),
    Some(dec!(0)),
    None,
    UnixNanos::default(),
    UnixNanos::default(),
);
```

```python tab="Python"
from decimal import Decimal

import pandas as pd

from nautilus_trader.model.currencies import USDC
from nautilus_trader.model.enums import AssetClass
from nautilus_trader.model.identifiers import InstrumentId
from nautilus_trader.model.identifiers import Symbol
from nautilus_trader.model.identifiers import Venue
from nautilus_trader.model.instruments import BinaryOption
from nautilus_trader.model.objects import Price
from nautilus_trader.model.objects import Quantity

raw_symbol = Symbol(
    "0x12a0cb60174abc437bf1178367c72d11f069e1a3add20b148fb0ab4279b772b2-92544998123698303655208967887569360731013655782348975589292031774495159624905",
)
price_increment = Price.from_str("0.001")
size_increment = Quantity.from_str("0.01")

yes_outcome = BinaryOption(
    instrument_id=InstrumentId(symbol=raw_symbol, venue=Venue("POLYMARKET")),
    raw_symbol=raw_symbol,
    asset_class=AssetClass.ALTERNATIVE,
    currency=USDC,
    price_precision=price_increment.precision,
    size_precision=size_increment.precision,
    price_increment=price_increment,
    size_increment=size_increment,
    activation_ns=0,
    expiration_ns=pd.Timestamp("2024-01-01", tz="UTC").value,
    min_quantity=Quantity.from_int(5),
    maker_fee=Decimal(0),
    taker_fee=Decimal(0),
    outcome="Yes",
    description="Will the outcome of this market be 'Yes'?",
    ts_event=0,
    ts_init=0,
)
```

## 适配器 (Adapters)

创建或消费 `BinaryOption` 工具的代表性适配器包括：

- [Hyperliquid](../../integrations/hyperliquid.md)，用于二元市场与预测类市场。
- [OKX](../../integrations/okx.md)，用于场所定义的二元结果产品。
- [Polymarket](../../integrations/polymarket.md)，用于预测市场结果。

## 相关指南 (Related guides)

- [订单簿 (Order Book)](../order_book.md) 介绍二元市场的订单簿行为。
- [数据 (Data)](../data.md) 解释引用工具的市场数据。
