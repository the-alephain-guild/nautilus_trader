# 投注合约 (Betting Instrument)

`BettingInstrument` 表示体育或博彩市场中的一个投注选项 (selection)。它携带赛事 (event)、赛事系列 (competition)、市场 (market) 以及投注选项的元数据，因此 Nautilus 可以把该投注选项当作一个具有价格、数量、限额、保证金和费用的合约来处理。

典型例子包括 Betfair 的对盘赔率 (match-odds) 投注选项和让分市场 (handicap market) 投注选项。

## 字段

| 字段                   | Rust 类型            | Python 类型          | 必填/默认值      | 说明                       |
| -------------------- | ------------------ | ------------------ | ----------- | ------------------------ |
| `instrument_id`      | `InstrumentId`     | N/A                | 仅 Rust      | 在 Rust 中以 `id` 存储。       |
| `raw_symbol`         | `Symbol`           | N/A                | 仅 Rust      | 原生或生成的交易场所代码。            |
| `venue_name`         | N/A                | `str`              | 仅 Python    | 用于构造合约 ID 的交易场所。         |
| `event_type_id`      | `u64`              | `int`              | 必填          | 赛事类型标识符。                 |
| `event_type_name`    | `Ustr`             | `str`              | 必填          | 赛事类型名称，例如某项运动。           |
| `competition_id`     | `u64`              | `int`              | 必填          | 赛事系列标识符。                 |
| `competition_name`   | `Ustr`             | `str`              | 必填          | 赛事系列名称。                  |
| `event_id`           | `u64`              | `int`              | 必填          | 赛事标识符。                   |
| `event_name`         | `Ustr`             | `str`              | 必填          | 赛事名称。                    |
| `event_country_code` | `Ustr`             | `str`              | 必填          | 赛事所在国家/地区代码。             |
| `event_open_date`    | `UnixNanos`        | `datetime`         | 必填          | 赛事开始时间。                  |
| `betting_type`       | `Ustr`             | `str`              | 必填          | 交易场所发布的投注类型。             |
| `market_id`          | `Ustr`             | `str`              | 必填          | 市场标识符。                   |
| `market_name`        | `Ustr`             | `str`              | 必填          | 市场名称。                    |
| `market_type`        | `Ustr`             | `str`              | 必填          | 市场类型，例如对盘赔率。             |
| `market_start_time`  | `UnixNanos`        | `datetime`         | 必填          | 市场开始时间。                  |
| `selection_id`       | `u64`              | `int`              | 必填          | 投注选项或参赛者标识符。             |
| `selection_name`     | `Ustr`             | `str`              | 必填          | 投注选项或参赛者名称。              |
| `selection_handicap` | `f64`              | `float`            | 必填          | 让分市场的让分值。                |
| `currency`           | `Currency`         | `str`              | 必填          | 报价和结算货币。                 |
| `price_precision`    | `u8`               | `int`              | 必填          | 价格允许的小数位数。               |
| `size_precision`     | `u8`               | `int`              | 必填          | 订单数量允许的小数位数。             |
| `price_increment`    | `Price`            | `Price \| None`    | 必填/Rust     | 价格步长，通常由 tick scheme 设定。 |
| `size_increment`     | `Quantity`         | `Quantity`         | 必填/Rust     | 最小数量步长。                  |
| `max_quantity`       | `Option<Quantity>` | `Quantity \| None` | `None`      | 最大订单数量。                  |
| `min_quantity`       | `Option<Quantity>` | `Quantity \| None` | `None`      | 最小订单数量。                  |
| `max_notional`       | `Option<Money>`    | `Money \| None`    | `None`      | 最大订单名义价值。                |
| `min_notional`       | `Option<Money>`    | `Money \| None`    | `None`      | 最小订单名义价值。                |
| `max_price`          | `Option<Price>`    | `Price \| None`    | `None`      | 最大有效报价或订单价格。             |
| `min_price`          | `Option<Price>`    | `Price \| None`    | `None`      | 最小有效报价或订单价格。             |
| `margin_init`        | `Option<Decimal>`  | `Decimal \| None`  | `1`         | 初始保证金率。                  |
| `margin_maint`       | `Option<Decimal>`  | `Decimal \| None`  | `1`         | 维持保证金率。                  |
| `maker_fee`          | `Option<Decimal>`  | `Decimal \| None`  | `0`         | 挂单 (maker) 费率。负值表示返佣。    |
| `taker_fee`          | `Option<Decimal>`  | `Decimal \| None`  | `0`         | 吃单 (taker) 费率。负值表示返佣。    |
| `tick_scheme_name`   | N/A                | `str \| None`      | `None`      | 已注册的可变 tick scheme 名称。   |
| `info`               | `Option<Params>`   | `dict \| None`     | `{}`/`None` | 适配器元数据。                  |
| `ts_event`           | `UnixNanos`        | `int`              | 必填          | 事件时间戳，单位为纳秒。             |
| `ts_init`            | `UnixNanos`        | `int`              | 必填          | 初始化时间戳，单位为纳秒。            |

*注意：Python 会根据交易场所、市场、投注选项和让分字段来构造合约 ID 和原生代码 (raw symbol)。Rust 则直接以 `instrument_id` 和 `raw_symbol` 接收这两者。*

## 行为

- `BettingInstrument` 的资产类别 (asset class) 为 `Alternative`，合约类别 (instrument class) 为 `SportsBetting`。
- 每个投注选项或参赛者都被建模为独立的合约。
- 投注合约通常使用已注册的 tick scheme 来定义有效的赔率步长。
- 保证金默认为 1，因为下注通常会冻结全部本金。

## 示例

```rust tab="Rust"
use chrono::{TimeZone, Utc};
use nautilus_core::UnixNanos;
use nautilus_model::{
    identifiers::{InstrumentId, Symbol},
    instruments::BettingInstrument,
    types::{Currency, Money, Price, Quantity},
};
use rust_decimal_macros::dec;
use ustr::Ustr;

let event_open = Utc.with_ymd_and_hms(2022, 2, 7, 23, 30, 0).unwrap();
let market_start = Utc.with_ymd_and_hms(2022, 2, 7, 23, 30, 0).unwrap();

let selection = BettingInstrument::new(
    InstrumentId::from("1-123456789.BETFAIR"),
    Symbol::from("1-123456789"),
    6423,
    Ustr::from("American Football"),
    12_282_733,
    Ustr::from("NFL"),
    29_678_534,
    Ustr::from("NFL"),
    Ustr::from("GB"),
    UnixNanos::from(event_open.timestamp_nanos_opt().unwrap() as u64),
    Ustr::from("ODDS"),
    Ustr::from("1-123456789"),
    Ustr::from("AFC Conference Winner"),
    Ustr::from("SPECIAL"),
    UnixNanos::from(market_start.timestamp_nanos_opt().unwrap() as u64),
    50214,
    Ustr::from("Kansas City Chiefs"),
    0.0,
    Currency::from("GBP"),
    2,
    2,
    Price::from("0.01"),
    Quantity::from("0.01"),
    Some(Quantity::from("1000")),
    Some(Quantity::from("1")),
    Some(Money::from("10000 GBP")),
    Some(Money::from("10 GBP")),
    Some(Price::from("100.00")),
    Some(Price::from("1.00")),
    Some(dec!(1)),
    Some(dec!(1)),
    Some(dec!(0)),
    Some(dec!(0)),
    None,
    UnixNanos::default(),
    UnixNanos::default(),
);
```

```python tab="Python"
import pandas as pd

from nautilus_trader.model.currencies import GBP
from nautilus_trader.model.instruments import BettingInstrument
from nautilus_trader.model.objects import Money

selection = BettingInstrument(
    venue_name="BETFAIR",
    event_type_id=6423,
    event_type_name="American Football",
    competition_id=12282733,
    competition_name="NFL",
    event_id=29678534,
    event_name="NFL",
    event_country_code="GB",
    event_open_date=pd.Timestamp("2022-02-07 23:30:00+00:00"),
    betting_type="ODDS",
    market_id="1-123456789",
    market_name="AFC Conference Winner",
    market_type="SPECIAL",
    market_start_time=pd.Timestamp("2022-02-07 23:30:00+00:00"),
    selection_id=50214,
    selection_name="Kansas City Chiefs",
    currency="GBP",
    selection_handicap=0.0,
    price_precision=2,
    size_precision=2,
    min_notional=Money(1, GBP),
    ts_event=0,
    ts_init=0,
)
```

## 适配器

会创建或消费 `BettingInstrument` 合约的代表性适配器包括：

- [Betfair](../../integrations/betfair.md)，用于体育投注市场。
- [Betfair v2](../../integrations/betfair_v2.md)，用于体育投注市场。

## 相关指南

- [账务 (Accounting)](../accounting.md) 介绍投注账户的行为。
- [数据 (Data)](../data.md) 解释引用合约的市场数据。
