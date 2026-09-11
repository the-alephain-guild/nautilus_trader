# 加密永续合约 (Crypto Perpetual)

`CryptoPerpetual` 表示一种加密永续期货合约，也称为永续掉期 (perpetual swap)。它没有到期日，跟踪某个加密基础资产，并以加密货币、稳定币或其他场所定义的结算货币进行结算。

例如 `ETHUSDT-PERP.BINANCE`、`XBTUSD.BITMEX` 以及 `BTC-USD-SWAP.OKX`。

## 字段 (Fields)

| 字段                    | Rust 类型            | Python 类型          | 必填/默认值 | 说明                 |
| --------------------- | ------------------ | ------------------ | ------ | ------------------ |
| `instrument_id`       | `InstrumentId`     | `InstrumentId`     | 必填     | 在 Rust 中存储为 `id`。  |
| `raw_symbol`          | `Symbol`           | `Symbol`           | 必填     | 场所原生交易代码。          |
| `base_currency`       | `Currency`         | `Currency`         | 必填     | 基础加密资产。            |
| `quote_currency`      | `Currency`         | `Currency`         | 必填     | 价格计价货币。            |
| `settlement_currency` | `Currency`         | `Currency`         | 必填     | 用于结算盈亏和费用的货币。      |
| `is_inverse`          | `bool`             | `bool`             | 必填     | 当采用反向计量/计价时为 True。 |
| `price_precision`     | `u8`               | `int`              | 必填     | 价格允许的小数位数。         |
| `size_precision`      | `u8`               | `int`              | 必填     | 订单数量允许的小数位数。       |
| `price_increment`     | `Price`            | `Price`            | 必填     | 最小有效价格步长。          |
| `size_increment`      | `Quantity`         | `Quantity`         | 必填     | 最小有效数量步长。          |
| `ts_event`            | `UnixNanos`        | `int`              | 必填     | 事件时间戳（纳秒）。         |
| `ts_init`             | `UnixNanos`        | `int`              | 必填     | 初始化时间戳（纳秒）。        |
| `multiplier`          | `Quantity`         | `Quantity`         | `1`    | 合约乘数。              |
| `lot_size`            | `Quantity`         | `Quantity`         | `1`    | 取整后的手数或交易板块大小。     |
| `max_quantity`        | `Option<Quantity>` | `Quantity \| None` | `None` | 最大订单数量。            |
| `min_quantity`        | `Option<Quantity>` | `Quantity \| None` | `None` | 最小订单数量。            |
| `max_notional`        | `Option<Money>`    | `Money \| None`    | `None` | 最大订单名义价值。          |
| `min_notional`        | `Option<Money>`    | `Money \| None`    | `None` | 最小订单名义价值。          |
| `max_price`           | `Option<Price>`    | `Price \| None`    | `None` | 最大有效报价或订单价格。       |
| `min_price`           | `Option<Price>`    | `Price \| None`    | `None` | 最小有效报价或订单价格。       |
| `margin_init`         | `Option<Decimal>`  | `Decimal \| None`  | `0`    | 初始保证金率。            |
| `margin_maint`        | `Option<Decimal>`  | `Decimal \| None`  | `0`    | 维持保证金率。            |
| `maker_fee`           | `Option<Decimal>`  | `Decimal \| None`  | `0`    | Maker 费率。负值表示返佣。   |
| `taker_fee`           | `Option<Decimal>`  | `Decimal \| None`  | `0`    | Taker 费率。负值表示返佣。   |
| `tick_scheme_name`    | N/A                | `str \| None`      | `None` | 已注册的可变 tick 方案名称。  |
| `info`                | `Option<Params>`   | `dict \| None`     | `None` | 适配器元数据。            |

*注意：Python 构造函数使用 `instrument_id`；Rust 将同一值存储为 `id`。*

## 行为 (Behavior)

- `CryptoPerpetual` 的资产类别为 `Cryptocurrency`，金融工具类别为 `Swap`。
- 它没有生效或到期时间戳。
- 线性 (linear) 合约通常设置 `is_inverse=False`，并以计价货币结算。
- 反向 (inverse) 合约设置 `is_inverse=True`，通常以基础货币结算。
- Quanto 合约以第三种货币结算，该货币既不同于基础货币也不同于计价货币。
- 成本货币：对于反向合约为基础货币，对于 quanto 合约为结算货币，其余情况为计价货币。

:::note
资金费支付不属于金融工具的字段。它们以数据形式到达，例如 `FundingRateUpdate`，并引用金融工具 ID。
:::

## 示例 (Example)

```rust tab="Rust"
use nautilus_core::UnixNanos;
use nautilus_model::{
    identifiers::{InstrumentId, Symbol},
    instruments::{CryptoPerpetual, InstrumentAny},
    types::{Currency, Money, Price, Quantity},
};
use rust_decimal_macros::dec;

let ethusdt_perp = CryptoPerpetual::new(
    InstrumentId::from("ETHUSDT-PERP.BINANCE"),
    Symbol::from("ETHUSDT"),
    Currency::from("ETH"),
    Currency::from("USDT"),
    Currency::from("USDT"),
    false,
    2,
    3,
    Price::from("0.01"),
    Quantity::from("0.001"),
    None,
    None,
    Some(Quantity::from("10000.000")),
    Some(Quantity::from("0.001")),
    None,
    Some(Money::from("10.00 USDT")),
    Some(Price::from("15000.00")),
    Some(Price::from("1.00")),
    Some(dec!(1.0)),
    Some(dec!(0.35)),
    Some(dec!(0.0002)),
    Some(dec!(0.0004)),
    None,
    UnixNanos::default(),
    UnixNanos::default(),
);

let instrument = InstrumentAny::CryptoPerpetual(ethusdt_perp);
```

```python tab="Python"
from decimal import Decimal

from nautilus_trader.model.currencies import ETH
from nautilus_trader.model.currencies import USDT
from nautilus_trader.model.identifiers import InstrumentId
from nautilus_trader.model.identifiers import Symbol
from nautilus_trader.model.instruments import CryptoPerpetual
from nautilus_trader.model.objects import Money
from nautilus_trader.model.objects import Price
from nautilus_trader.model.objects import Quantity

ethusdt_perp = CryptoPerpetual(
    instrument_id=InstrumentId.from_str("ETHUSDT-PERP.BINANCE"),
    raw_symbol=Symbol("ETHUSDT"),
    base_currency=ETH,
    quote_currency=USDT,
    settlement_currency=USDT,
    is_inverse=False,
    price_precision=2,
    size_precision=3,
    price_increment=Price.from_str("0.01"),
    size_increment=Quantity.from_str("0.001"),
    ts_event=0,
    ts_init=0,
    max_quantity=Quantity.from_str("10000.000"),
    min_quantity=Quantity.from_str("0.001"),
    min_notional=Money.from_str("10.00 USDT"),
    max_price=Price.from_str("15000.00"),
    min_price=Price.from_str("1.00"),
    margin_init=Decimal("1.0"),
    margin_maint=Decimal("0.35"),
    maker_fee=Decimal("0.0002"),
    taker_fee=Decimal("0.0004"),
)
```

## 适配器 (Adapters)

创建或消费 `CryptoPerpetual` 金融工具的代表性适配器包括：

- [Binance](../../integrations/binance.md)，用于 USD-M 和 COIN-M 永续期货。
- [BitMEX](../../integrations/bitmex.md)，用于反向和线性永续合约。
- [Bybit](../../integrations/bybit.md)，用于线性和反向永续产品。
- [dYdX](../../integrations/dydx.md)，用于永续市场。
- [Hyperliquid](../../integrations/hyperliquid.md)，用于永续市场。
- [Kraken](../../integrations/kraken.md)，用于期货场所的永续市场。
- [OKX](../../integrations/okx.md)，用于掉期市场。
- [Tardis](../../integrations/tardis.md)，用于加密永续合约元数据。

## 相关指南 (Related guides)

- [数据 (Data)](../data.md) 涵盖标记价格、指数价格和资金费率更新。
- [期权 (Options)](../options.md) 涵盖期权特有的金融工具类型。
- [执行 (Execution)](../execution.md) 解释订单到达场所之前的精度和名义价值检查。
