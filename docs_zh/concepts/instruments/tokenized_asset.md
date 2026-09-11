# 代币化资产 (Tokenized Asset)

`TokenizedAsset` 表示在加密货币交易场所上跟踪另一资产的类现货代币。
可将其用于代币化股票、代币化基金，或其他由交易场所提供代币但经济参照物为外部资产的类似工具。

典型示例包括加密货币交易场所上的代币化股票或 ETF 代码。

## 字段 (Fields)

| 字段                | Rust 类型            | Python 类型          | 必填/默认值 | 说明                    |
| ----------------- | ------------------ | ------------------ | ------ | --------------------- |
| `instrument_id`   | `InstrumentId`     | `InstrumentId`     | 必填     | 在 Rust 中存储为 `id`。     |
| `raw_symbol`      | `Symbol`           | `Symbol`           | 必填     | 交易场所原生代码。             |
| `asset_class`     | `AssetClass`       | `AssetClass`       | 必填     | 经济资产分类。               |
| `base_currency`   | `Currency`         | `Currency`         | 必填     | 代币化资产或基础代币。           |
| `quote_currency`  | `Currency`         | `Currency`         | 必填     | 用于为该代币计价的货币。          |
| `price_precision` | `u8`               | `int`              | 必填     | 价格允许的小数位数。            |
| `size_precision`  | `u8`               | `int`              | 必填     | 订单数量允许的小数位数。          |
| `price_increment` | `Price`            | `Price`            | 必填     | 最小有效价格步进。             |
| `size_increment`  | `Quantity`         | `Quantity`         | 必填     | 最小有效数量步进。             |
| `ts_event`        | `UnixNanos`        | `int`              | 必填     | 以纳秒为单位的事件时间戳。         |
| `ts_init`         | `UnixNanos`        | `int`              | 必填     | 以纳秒为单位的初始化时间戳。        |
| `isin`            | `Option<Ustr>`     | `str \| None`      | `None` | 已知时填写的国际证券识别码。        |
| `multiplier`      | `Quantity`         | `Quantity`         | `1`    | 合约乘数。                 |
| `lot_size`        | `Option<Quantity>` | `Quantity \| None` | `None` | 整手或最小交易单位。            |
| `max_quantity`    | `Option<Quantity>` | `Quantity \| None` | `None` | 最大订单数量。               |
| `min_quantity`    | `Option<Quantity>` | `Quantity \| None` | `None` | 最小订单数量。               |
| `max_notional`    | `Option<Money>`    | `Money \| None`    | `None` | 最大订单名义价值。             |
| `min_notional`    | `Option<Money>`    | `Money \| None`    | `None` | 最小订单名义价值。             |
| `max_price`       | `Option<Price>`    | `Price \| None`    | `None` | 最大有效报价或订单价格。          |
| `min_price`       | `Option<Price>`    | `Price \| None`    | `None` | 最小有效报价或订单价格。          |
| `margin_init`     | `Option<Decimal>`  | `Decimal \| None`  | `0`    | 初始保证金率。               |
| `margin_maint`    | `Option<Decimal>`  | `Decimal \| None`  | `0`    | 维持保证金率。               |
| `maker_fee`       | `Option<Decimal>`  | `Decimal \| None`  | `0`    | 挂单 (maker) 费率。负值表示返佣。 |
| `taker_fee`       | `Option<Decimal>`  | `Decimal \| None`  | `0`    | 吃单 (taker) 费率。负值表示返佣。 |
| `info`            | `Option<Params>`   | `dict \| None`     | `None` | 适配器元数据。               |

*注意：Python 构造函数使用 `instrument_id`；Rust 将同一值存储为 `id`。*

## 行为 (Behavior)

- `TokenizedAsset` 的工具类别为 `Spot`。
- 它绝不是反向合约，其成本货币为计价货币。
- 当该代币参照某只上市证券时，它可以携带 `isin`。
- 它没有生效时间戳、到期日、行权价或期权类型。

## 示例 (Example)

```rust tab="Rust"
use nautilus_core::UnixNanos;
use nautilus_model::{
    enums::AssetClass,
    identifiers::{InstrumentId, Symbol},
    instruments::TokenizedAsset,
    types::{Currency, Price, Quantity},
};
use rust_decimal_macros::dec;

let aaplx = TokenizedAsset::new(
    InstrumentId::from("AAPLx/USD.KRAKEN"),
    Symbol::from("AAPLxUSD"),
    AssetClass::Equity,
    Currency::get_or_create_crypto("AAPLx"),
    Currency::from("USD"),
    None,
    2,
    4,
    Price::from("0.01"),
    Quantity::from("0.0001"),
    None,
    None,
    None,
    Some(Quantity::from("0.0001")),
    None,
    None,
    None,
    None,
    None,
    None,
    Some(dec!(-0.0002)),
    Some(dec!(0.001)),
    None,
    UnixNanos::default(),
    UnixNanos::default(),
);
```

```python tab="Python"
from decimal import Decimal

from nautilus_trader.model.currencies import USD
from nautilus_trader.model.enums import AssetClass
from nautilus_trader.model.identifiers import InstrumentId
from nautilus_trader.model.identifiers import Symbol
from nautilus_trader.model.instruments import TokenizedAsset
from nautilus_trader.model.objects import Currency
from nautilus_trader.model.objects import Price
from nautilus_trader.model.objects import Quantity

aaplx = TokenizedAsset(
    instrument_id=InstrumentId.from_str("AAPLx/USD.KRAKEN"),
    raw_symbol=Symbol("AAPLxUSD"),
    asset_class=AssetClass.EQUITY,
    base_currency=Currency.from_str("AAPLx"),
    quote_currency=USD,
    price_precision=2,
    size_precision=4,
    price_increment=Price.from_str("0.01"),
    size_increment=Quantity.from_str("0.0001"),
    min_quantity=Quantity.from_str("0.0001"),
    maker_fee=Decimal("-0.0002"),
    taker_fee=Decimal("0.001"),
    ts_event=0,
    ts_init=0,
)
```

## 适配器 (Adapters)

会创建或消费 `TokenizedAsset` 工具的代表性适配器包括：

- [Kraken](../../integrations/kraken.md)，用于交易场所提供代币化资产的场景。

## 相关指南 (Related guides)

- [货币对 (Currency Pair)](currency_pair.md) 介绍普通的加密货币现货交易对。
- [股票 (Equity)](equity.md) 介绍上市的现金股票。
