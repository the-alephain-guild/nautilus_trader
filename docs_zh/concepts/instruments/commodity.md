# 商品 (Commodity)

`Commodity` 表示现货商品市场，例如黄金、白银、原油或其他以某种货币计价的实物资产。它建模的是现货市场，而非有到期日的期货合约。

示例包括 `XAUUSD.IDEALPRO` 以及特定交易场所的商品现货代码。

## 字段 (Fields)

| 字段                 | Rust 类型            | Python 类型          | 是否必填/默认值 | 说明                     |
| ------------------ | ------------------ | ------------------ | -------- | ---------------------- |
| `instrument_id`    | `InstrumentId`     | `InstrumentId`     | 必填       | 在 Rust 中以 `id` 存储。     |
| `raw_symbol`       | `Symbol`           | `Symbol`           | 必填       | 交易场所的原生代码。             |
| `asset_class`      | `AssetClass`       | `AssetClass`       | 必填       | 商品资产分类。                |
| `quote_currency`   | `Currency`         | `Currency`         | 必填       | 为商品计价所用的货币。            |
| `price_precision`  | `u8`               | `int`              | 必填       | 价格允许的小数位数。             |
| `size_precision`   | `u8`               | `int`              | 必填       | 订单数量允许的小数位数。           |
| `price_increment`  | `Price`            | `Price`            | 必填       | 最小有效价格步长。              |
| `size_increment`   | `Quantity`         | `Quantity`         | 必填       | 最小有效数量步长。              |
| `ts_event`         | `UnixNanos`        | `int`              | 必填       | 事件时间戳（纳秒）。             |
| `ts_init`          | `UnixNanos`        | `int`              | 必填       | 初始化时间戳（纳秒）。            |
| `base_currency`    | N/A                | `Currency \| None` | `None`   | 仅 Python 的基础资产货币（如已知）。 |
| `lot_size`         | `Option<Quantity>` | `Quantity \| None` | `None`   | 圆整后的手数或挂牌单位。           |
| `max_quantity`     | `Option<Quantity>` | `Quantity \| None` | `None`   | 最大订单数量。                |
| `min_quantity`     | `Option<Quantity>` | `Quantity \| None` | `None`   | 最小订单数量。                |
| `max_notional`     | `Option<Money>`    | `Money \| None`    | `None`   | 最大订单名义价值。              |
| `min_notional`     | `Option<Money>`    | `Money \| None`    | `None`   | 最小订单名义价值。              |
| `max_price`        | `Option<Price>`    | `Price \| None`    | `None`   | 最大有效报价或订单价格。           |
| `min_price`        | `Option<Price>`    | `Price \| None`    | `None`   | 最小有效报价或订单价格。           |
| `margin_init`      | `Option<Decimal>`  | `Decimal \| None`  | `0`      | 初始保证金率。                |
| `margin_maint`     | `Option<Decimal>`  | `Decimal \| None`  | `0`      | 维持保证金率。                |
| `maker_fee`        | `Option<Decimal>`  | `Decimal \| None`  | `0`      | 挂单方费率。负值表示返佣。          |
| `taker_fee`        | `Option<Decimal>`  | `Decimal \| None`  | `0`      | 吃单方费率。负值表示返佣。          |
| `tick_scheme_name` | N/A                | `str \| None`      | `None`   | 已注册的可变 tick 方案名称。      |
| `info`             | `Option<Params>`   | `dict \| None`     | `None`   | 适配器元数据。                |

*注意：Python 构造函数使用 `instrument_id`；Rust 将同一个值存储为 `id`。*

## 行为 (Behavior)

- `Commodity` 的合约类别为 `Spot`。
- 它允许负价格：电力或原油等现货市场可能在零以下成交，且 `RiskEngine` 在订单提交和修改时都接受负价格。
- 它从不是反向（inverse）合约，其成本货币即为计价货币。
- 它没有激活时间戳、到期日、行权价、期权类型或结算货币字段。
- 对于有到期日的交易所挂牌商品期货，请使用 `FuturesContract`。

## 示例 (Example)

```rust tab="Rust"
use nautilus_core::UnixNanos;
use nautilus_model::{
    enums::AssetClass,
    identifiers::{InstrumentId, Symbol},
    instruments::Commodity,
    types::{Currency, Price, Quantity},
};

let gold = Commodity::new(
    InstrumentId::from("GOLD.COMEX"),
    Symbol::from("GOLD"),
    AssetClass::Commodity,
    Currency::from("USD"),
    2,
    0,
    Price::from("0.01"),
    Quantity::from("1"),
    Some(Quantity::from("1")),
    None,
    None,
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
from nautilus_trader.model.currencies import USD
from nautilus_trader.model.enums import AssetClass
from nautilus_trader.model.identifiers import InstrumentId
from nautilus_trader.model.identifiers import Symbol
from nautilus_trader.model.instruments import Commodity
from nautilus_trader.model.objects import Price
from nautilus_trader.model.objects import Quantity

gold = Commodity(
    instrument_id=InstrumentId.from_str("GOLD.COMEX"),
    raw_symbol=Symbol("GOLD"),
    asset_class=AssetClass.COMMODITY,
    quote_currency=USD,
    price_precision=2,
    price_increment=Price.from_str("0.01"),
    size_precision=0,
    size_increment=Quantity.from_int(1),
    lot_size=Quantity.from_int(1),
    ts_event=0,
    ts_init=0,
)
```

## 适配器 (Adapters)

会创建或消费 `Commodity` 合约的代表性适配器包括：

- [Interactive Brokers](../../integrations/ib.md)，用于现货商品和金属合约。

## 相关指南 (Related guides)

- [期货合约 (Futures Contract)](futures_contract.md) 涵盖以商品为标的的有到期日期货。
- [数据 (Data)](../data.md) 解释引用合约的市场数据。
