# 货币对 (Currency Pair)

`CurrencyPair` 表示以 `BASE/QUOTE` 形式报价的现货或现金市场。基础货币 (base currency)
是被买入或卖出的资产，而报价货币 (quote currency) 为一单位基础货币定价。Nautilus 用此类型
表示法币外汇对和加密现货对。

示例包括 `EUR/USD.SIM`、`BTCUSDT.BINANCE` 和 `ETH/USD.KRAKEN`。

## 字段 (Fields)

| 字段                 | Rust 类型            | Python 类型          | 必填/默认值 | 说明                    |
| ------------------ | ------------------ | ------------------ | ------ | --------------------- |
| `instrument_id`    | `InstrumentId`     | `InstrumentId`     | 必填     | 在 Rust 中存储为 `id`。     |
| `raw_symbol`       | `Symbol`           | `Symbol`           | 必填     | 交易场所的原生符号。            |
| `base_currency`    | `Currency`         | `Currency`         | 必填     | 被买入或卖出的资产。            |
| `quote_currency`   | `Currency`         | `Currency`         | 必填     | 用于为基础资产定价的货币。         |
| `price_precision`  | `u8`               | `int`              | 必填     | 价格允许的小数位数。            |
| `size_precision`   | `u8`               | `int`              | 必填     | 订单数量允许的小数位数。          |
| `price_increment`  | `Price`            | `Price`            | 必填     | 最小有效价格步长。             |
| `size_increment`   | `Quantity`         | `Quantity`         | 必填     | 最小有效数量步长。             |
| `ts_event`         | `UnixNanos`        | `int`              | 必填     | 事件时间戳（纳秒）。            |
| `ts_init`          | `UnixNanos`        | `int`              | 必填     | 初始化时间戳（纳秒）。           |
| `multiplier`       | `Quantity`         | `Quantity`         | `1`    | 合约乘数。                 |
| `lot_size`         | `Option<Quantity>` | `Quantity \| None` | `None` | 取整后的手数或板块规模。          |
| `max_quantity`     | `Option<Quantity>` | `Quantity \| None` | `None` | 最大订单数量。               |
| `min_quantity`     | `Option<Quantity>` | `Quantity \| None` | `None` | 最小订单数量。               |
| `max_notional`     | `Option<Money>`    | `Money \| None`    | `None` | 最大订单名义价值。             |
| `min_notional`     | `Option<Money>`    | `Money \| None`    | `None` | 最小订单名义价值。             |
| `max_price`        | `Option<Price>`    | `Price \| None`    | `None` | 最大有效报价或订单价格。          |
| `min_price`        | `Option<Price>`    | `Price \| None`    | `None` | 最小有效报价或订单价格。          |
| `margin_init`      | `Option<Decimal>`  | `Decimal \| None`  | `0`    | 初始保证金率。               |
| `margin_maint`     | `Option<Decimal>`  | `Decimal \| None`  | `0`    | 维持保证金率。               |
| `maker_fee`        | `Option<Decimal>`  | `Decimal \| None`  | `0`    | 挂单 (maker) 费率。负值表示返佣。 |
| `taker_fee`        | `Option<Decimal>`  | `Decimal \| None`  | `0`    | 吃单 (taker) 费率。负值表示返佣。 |
| `tick_scheme_name` | N/A                | `str \| None`      | `None` | 已注册的可变 tick 方案名称。     |
| `info`             | `Option<Params>`   | `dict \| None`     | `None` | 适配器元数据。               |

*注意：Python 构造函数使用 `instrument_id`；Rust 将同一值存储为 `id`。*

## 行为 (Behavior)

- `CurrencyPair` 的合约类别为 `Spot`。
- 它没有到期日、行权价、期权类型或衍生品标的字段。
- 它永远不是反向合约。其结算货币和成本货币均为报价货币。
- 法币外汇对和加密现货对都使用此类型。

:::warning
不要仅因为某些品种的符号看起来像货币对，就把有到期日的期货、掉期或期权建模为 `CurrencyPair`。
请使用特定的衍生品类型，以使成本货币、结算货币、到期日和名义价值计算与交易场所一致。
:::

## 示例 (Example)

```rust tab="Rust"
use nautilus_core::UnixNanos;
use nautilus_model::{
    identifiers::{InstrumentId, Symbol},
    instruments::{CurrencyPair, InstrumentAny},
    types::{Currency, Money, Price, Quantity},
};
use rust_decimal_macros::dec;

let btcusdt = CurrencyPair::new(
    InstrumentId::from("BTCUSDT.BINANCE"),
    Symbol::from("BTCUSDT"),
    Currency::from("BTC"),
    Currency::from("USDT"),
    2,
    6,
    Price::from("0.01"),
    Quantity::from("0.000001"),
    None,
    None,
    None,
    Some(Quantity::from("0.000001")),
    None,
    Some(Money::from("10.00 USDT")),
    Some(Price::from("1000000.00")),
    Some(Price::from("0.01")),
    Some(dec!(0.001)),
    Some(dec!(0.001)),
    Some(dec!(0.001)),
    Some(dec!(0.001)),
    None,
    UnixNanos::default(),
    UnixNanos::default(),
);

let instrument = InstrumentAny::CurrencyPair(btcusdt);
```

```python tab="Python"
from decimal import Decimal

from nautilus_trader.model.currencies import BTC
from nautilus_trader.model.currencies import USDT
from nautilus_trader.model.identifiers import InstrumentId
from nautilus_trader.model.identifiers import Symbol
from nautilus_trader.model.instruments import CurrencyPair
from nautilus_trader.model.objects import Money
from nautilus_trader.model.objects import Price
from nautilus_trader.model.objects import Quantity

btcusdt = CurrencyPair(
    instrument_id=InstrumentId.from_str("BTCUSDT.BINANCE"),
    raw_symbol=Symbol("BTCUSDT"),
    base_currency=BTC,
    quote_currency=USDT,
    price_precision=2,
    size_precision=6,
    price_increment=Price.from_str("0.01"),
    size_increment=Quantity.from_str("0.000001"),
    ts_event=0,
    ts_init=0,
    min_quantity=Quantity.from_str("0.000001"),
    min_notional=Money.from_str("10.00 USDT"),
    max_price=Price.from_str("1000000.00"),
    min_price=Price.from_str("0.01"),
    margin_init=Decimal("0.001"),
    margin_maint=Decimal("0.001"),
    maker_fee=Decimal("0.001"),
    taker_fee=Decimal("0.001"),
)
```

## 适配器 (Adapters)

创建或消费 `CurrencyPair` 合约的代表性适配器包括：

- [Binance](../../integrations/binance.md)，用于现货市场。
- [Kraken](../../integrations/kraken.md)，用于现货市场。
- [OKX](../../integrations/okx.md)，用于现货市场。
- [Tardis](../../integrations/tardis.md)，用于现货元数据。
- [Interactive Brokers](../../integrations/ib.md)，用于外汇现金合约。
- [Hyperliquid](../../integrations/hyperliquid.md)，用于现货资产。

## 相关指南 (Related guides)

- [Data](../data.md) 介绍引用合约的市场数据。
- [Execution](../execution.md) 介绍使用合约精度的订单校验。
- [Value types](../value_types.md) 介绍 `Price`、`Quantity` 和 `Money`。
