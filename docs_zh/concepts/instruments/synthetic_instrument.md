# 合成工具 (Synthetic Instrument)

`SyntheticInstrument` 表示一种本地工具，其价格来自一个基于其他工具的公式。它适用于价差、篮子、比率以及其他派生价格——这些价格应当以工具的形式出现在系统中。

例如 `(BTC.BINANCE + LTC.BINANCE) / 2.0`，以及由各成分工具价格构建的比率型交易对。

## 字段 (Fields)

| 字段                | Rust 类型             | Python 类型            | 必填/默认值 | 说明                         |
| ----------------- | ------------------- | -------------------- | ------ | -------------------------- |
| `symbol`          | `Symbol`            | `Symbol`             | 必填     | 与场所 `SYNTH` 搭配使用的合成符号。     |
| `id`              | `InstrumentId`      | `InstrumentId`       | 派生     | 由 `symbol.SYNTH` 构成的工具 ID。 |
| `price_precision` | `u8`                | `int`                | 必填     | 合成价格允许的小数位数。               |
| `price_increment` | `Price`             | `Price`              | 派生     | 由精度推导出的最小价格步长。             |
| `components`      | `Vec<InstrumentId>` | `list[InstrumentId]` | 必填     | 公式所使用的成分工具。                |
| `formula`         | `String`            | `str`                | 必填     | 基于成分 ID 的数值表达式。            |
| `ts_event`        | `UnixNanos`         | `int`                | 必填     | 事件时间戳（纳秒）。                 |
| `ts_init`         | `UnixNanos`         | `int`                | 必填     | 初始化时间戳（纳秒）。                |

*注意：Python 由 `symbol` 和 `SYNTH` 场所构造工具 ID。Rust 将同一个值存储为 `id`。*

## 行为 (Behavior)

- `SyntheticInstrument` 是 Nautilus 本地的工具，不代表某个可在场所下单的市场。
- 它始终使用合成场所 `SYNTH`。
- Python 要求至少提供两个成分工具 ID。
- 在对象生效之前，公式必须能针对所提供的成分标识符成功编译。
- 它没有场所限制、保证金、费用、订单簿，也没有适配器特有的元数据。

## 示例 (Example)

```rust tab="Rust"
use nautilus_core::UnixNanos;
use nautilus_model::{
    identifiers::{InstrumentId, Symbol},
    instruments::SyntheticInstrument,
};

let synthetic = SyntheticInstrument::new(
    Symbol::from("BTC-LTC"),
    2,
    vec![
        InstrumentId::from("BTC.BINANCE"),
        InstrumentId::from("LTC.BINANCE"),
    ],
    "(BTC.BINANCE + LTC.BINANCE) / 2.0",
    UnixNanos::default(),
    UnixNanos::default(),
);
```

```python tab="Python"
from nautilus_trader.model.identifiers import InstrumentId
from nautilus_trader.model.identifiers import Symbol
from nautilus_trader.model.instruments import SyntheticInstrument

synthetic = SyntheticInstrument(
    symbol=Symbol("BTC-LTC"),
    price_precision=2,
    components=[
        InstrumentId.from_str("BTC.BINANCE"),
        InstrumentId.from_str("LTC.BINANCE"),
    ],
    formula="(BTC.BINANCE + LTC.BINANCE) / 2.0",
    ts_event=0,
    ts_init=0,
)
```

## 适配器 (Adapters)

`SyntheticInstrument` 仅在本地存在。它从成分工具中派生价格，而这些成分工具可以来自任何已加载到系统中的适配器。

## 相关指南 (Related guides)

- [合成工具 (Synthetics)](../synthetics.md) 介绍由公式派生的工具与合成 Bar。
- [数据 (Data)](../data.md) 解释引用工具的市场数据。
