# Databento

NautilusTrader 提供了一个用于集成（Integration） [Databento](https://databento.com/) API
以及 [Databento 二进制编码（DBN）](https://databento.com/docs/standards-and-conventions/databento-binary-encoding)格式数据的适配器（Adapter）。
Databento 纯粹是一个市场数据提供商，因此该适配器不包含执行（Execution）客户端，
不过你可以将它与沙盒环境配对以进行模拟执行。
你也可以将 Databento 数据与 Interactive Brokers 执行配合使用，
或者计算传统资产类别的信号用于加密货币交易。

此适配器支持：

- 从 DBN 文件加载历史数据，并解码为 Nautilus 对象，用于回测（Backtest）或写入数据目录。
- 请求历史数据并解码为 Nautilus 对象，以支持实时交易和回测。
- 订阅实时数据源并解码为 Nautilus 对象，以支持实时交易和沙盒环境。

:::tip
[Databento](https://databento.com/signup) 为新账户注册提供 125 美元的免费数据额度。
Databento 目前允许将这些额度用于历史数据，或抵扣订阅计划首月的费用。

精心规划请求的话，这足够用于测试和评估。请在请求数据前检查
[/metadata.get_cost](https://databento.com/docs/api-reference-historical/metadata/metadata-get-cost)
端点。
:::

## 概述

适配器使用 [databento-rs](https://crates.io/crates/databento) crate，
这是 Databento 提供的官方 Rust 客户端库。

:::info
你**无需**单独安装 `databento`。适配器会被编译为静态库，并在构建过程中自动链接。
:::

以下适配器类可用：

- `DatabentoDataLoader`：从文件加载 DBN 数据。
- `DatabentoInstrumentProvider`：通过 Databento HTTP API 获取最新或历史的金融工具（Instrument）定义。
- `DatabentoHistoricalClient`：通过 Databento HTTP API 获取历史市场数据。
- `DatabentoLiveClient`：通过 Databento 的原始 TCP API 订阅实时数据源。
- `DatabentoDataClient`：用于实时交易节点的 `LiveMarketDataClient` 实现。

:::info
大多数用户会配置一个实时交易节点（如下所述），而不会直接使用这些组件。
:::

## 示例

参见[实时示例](https://github.com/nautechsystems/nautilus_trader/tree/develop/examples/live/databento/)。

## Databento 文档

参见 [Databento 新用户指南](https://databento.com/docs/quickstart/new-user-guides)。
请将其与本集成指南一并参阅。

## Databento 二进制编码（DBN）

Databento 二进制编码（DBN）是一种针对标准化（Normalization）市场数据的极快消息编码与存储格式。
[DBN 规范](https://databento.com/docs/standards-and-conventions/databento-binary-encoding)
包含一个自描述的元数据头，以及一组固定的结构体定义，
用于标准化市场数据的规范化方式。

适配器将 DBN 数据解码为 Nautilus 对象。同一个 Rust 解码器负责处理：

- 从磁盘加载并解码 DBN 文件。
- 实时解码历史数据和实时数据。

## 支持的 Schema

以下 Databento schema 受 NautilusTrader 支持：

| Databento schema                                                             | Nautilus 数据类型                    | 描述            |
| :--------------------------------------------------------------------------- | :------------------------------- | :------------ |
| [MBO](https://databento.com/docs/schemas-and-data-formats/mbo)               | `OrderBookDelta`                 | 按订单聚合（L3）。    |
| [MBP_1](https://databento.com/docs/schemas-and-data-formats/mbp-1)           | `(QuoteTick, TradeTick \| None)` | 按价格聚合（L1）。    |
| [MBP_10](https://databento.com/docs/schemas-and-data-formats/mbp-10)         | `OrderBookDepth10`               | 市场深度（L2）。     |
| [BBO_1S](https://databento.com/docs/schemas-and-data-formats/bbo-1s)         | `QuoteTick`                      | 1 秒最优买卖价。     |
| [BBO_1M](https://databento.com/docs/schemas-and-data-formats/bbo-1m)         | `QuoteTick`                      | 1 分钟最优买卖价。    |
| [CMBP_1](https://databento.com/docs/schemas-and-data-formats/cmbp-1)         | `(QuoteTick, TradeTick \| None)` | 跨交易场所整合的 MBP。 |
| [CBBO_1S](https://databento.com/docs/schemas-and-data-formats/cbbo-1s)       | `QuoteTick`                      | 整合的 1 秒 BBO。  |
| [CBBO_1M](https://databento.com/docs/schemas-and-data-formats/cbbo-1m)       | `QuoteTick`                      | 整合的 1 分钟 BBO。 |
| [TCBBO](https://databento.com/docs/schemas-and-data-formats/tcbbo)           | `(QuoteTick, TradeTick)`         | 交易采样的整合 BBO。  |
| [TBBO](https://databento.com/docs/schemas-and-data-formats/tbbo)             | `(QuoteTick, TradeTick)`         | 交易采样的最优买卖价。   |
| [TRADES](https://databento.com/docs/schemas-and-data-formats/trades)         | `TradeTick`                      | 成交 tick。      |
| [OHLCV_1S](https://databento.com/docs/schemas-and-data-formats/ohlcv-1s)     | `Bar`                            | 1 秒 K 线。      |
| [OHLCV_1M](https://databento.com/docs/schemas-and-data-formats/ohlcv-1m)     | `Bar`                            | 1 分钟 K 线。     |
| [OHLCV_1H](https://databento.com/docs/schemas-and-data-formats/ohlcv-1h)     | `Bar`                            | 1 小时 K 线。     |
| [OHLCV_1D](https://databento.com/docs/schemas-and-data-formats/ohlcv-1d)     | `Bar`                            | 日 K 线。        |
| [DEFINITION](https://databento.com/docs/schemas-and-data-formats/definition) | `Instrument`（多种类型）               | 金融工具定义。       |
| [IMBALANCE](https://databento.com/docs/schemas-and-data-formats/imbalance)   | `DatabentoImbalance`             | 拍卖不平衡数据。      |
| [STATISTICS](https://databento.com/docs/schemas-and-data-formats/statistics) | `DatabentoStatistics`            | 市场统计数据。       |
| [STATUS](https://databento.com/docs/schemas-and-data-formats/status)         | `InstrumentStatus`               | 市场状态更新。       |

:::note
Databento 还提供参考类 schema 的文档，包括公司行为、调整因子和证券主数据。
本适配器目前仅将上表所列的 schema 映射到 Nautilus 数据类型。Databento 的 DBN crate
还暴露了 `ohlcv-eod`；Nautilus 在适配器层面保留了一个针对日 K 线的覆盖项，
而公开的 Databento schema 文档将 `ohlcv-1d` 列为日级 OHLCV。
:::

:::info
对于不支持的 `instrument_class` 值（`'I'` 指数、`'B'` 债券、`'X'` 外汇即期），
其金融工具定义会被跳过并发出警告，而不会中止整个批次。会发出指数的发布者包括
CGIF.TITANIUM (110)、IEX Options (108) 和 MEMX MX2 (109)。如果你需要 Nautilus 对这些类型进行建模，请提交 issue。

`stat_type` 值超出已建模范围（当前为 1-20）的统计消息同样会被跳过并发出警告。
这包括交易场所专用值 `VenueSpecificVolume1` (10001) 和 `VenueSpecificPrice1` (10002)，
它们超出了持久化所使用的 `u8` Arrow 列宽。
:::

### Schema 选择注意事项

- **TBBO 和 TCBBO**：交易采样数据源，将每笔交易与该交易生效*之前*的 BBO 配对。
  当你需要将交易与同时期的报价对齐而无需管理两个数据流时使用。
- **MBP-1 和 CMBP-1（L1）**：事件级更新，仅在交易事件时发送成交数据。
  当你需要一份完整的最优价事件流时选择它们。若需报价与成交对齐，优先选择 TBBO 或 TCBBO。
- **MBP-10（L2）**：前 10 个价格层级加成交数据。适合需要深度感知、
  但无需完整 MBO 数据的策略。包含每个价格层级的订单数量。
- **MBO（L3）**：逐笔订单事件，用于队列位置建模和精确的订单簿重建。
  在节点初始化时开始订阅，以获得正确的回放上下文。
- **BBO_1S/BBO_1M 和 CBBO_1S/CBBO_1M**：固定间隔（1 秒或 1 分钟）采样的最优价更新。
  适配器仅为这些 schema 发出 `QuoteTick`。
  适合用于监控、价差和低成本信号。它们不适合微观结构方面的工作。
- **TRADES**：仅成交数据。与 MBP-1（`include_trades=True`）配对使用，或使用 TBBO
  或 TCBBO 获取带成交的报价上下文。
- **OHLCV**：从成交数据聚合而来的 K 线。用于更长时间框架的分析。
  设置 `bars_timestamp_on_close=True` 以使用收盘时间戳。
- **Imbalance、statistics 和 status**：交易场所运营数据。通过
  `subscribe_data` 配合携带 `instrument_id` 元数据的 `DataType` 进行订阅。

:::tip
整合类 schema（CMBP_1、CBBO_1S、CBBO_1M、TCBBO）将数据跨多个交易场所聚合。
对于跨交易场所分析很有用。
:::

:::info
另请参阅 Databento 的 [Schemas and data formats](https://databento.com/docs/schemas-and-data-formats) 指南。
:::

## 实时订阅的 Schema 选择

Nautilus 订阅方法按如下方式映射到 Databento schema：

| Nautilus 订阅方法                   | 默认 Schema | 可用的 Databento Schema                                                  | Nautilus 数据类型      |
| :------------------------------ | :-------- | :-------------------------------------------------------------------- | :----------------- |
| `subscribe_quote_ticks()`       | `mbp-1`   | `mbp-1`、`bbo-1s`、`bbo-1m`、`cmbp-1`、`cbbo-1s`、`cbbo-1m`、`tbbo`、`tcbbo` | `QuoteTick`        |
| `subscribe_trade_ticks()`       | `trades`  | `trades`、`tbbo`、`tcbbo`、`mbp-1`、`cmbp-1`                              | `TradeTick`        |
| `subscribe_order_book_depth()`  | `mbp-10`  | `mbp-10`                                                              | `OrderBookDepth10` |
| `subscribe_order_book_deltas()` | `mbo`     | `mbo`                                                                 | `OrderBookDeltas`  |
| `subscribe_bars()`              | 视情况而定     | `ohlcv-1s`、`ohlcv-1m`、`ohlcv-1h`、`ohlcv-1d`                           | `Bar`              |

:::note
以下示例假设你处于 `Strategy` 或 `Actor` 上下文中，其中 `self` 拥有订阅方法。
请导入所需的类型：

```python
from nautilus_trader.adapters.databento import DATABENTO_CLIENT_ID
from nautilus_trader.model import BarType
from nautilus_trader.model.enums import BookType
from nautilus_trader.model.identifiers import InstrumentId
```

:::

### 报价订阅（MBP 与 L1）

```python
# 默认 MBP-1 报价（可能包含成交数据）
self.subscribe_quote_ticks(instrument_id, client_id=DATABENTO_CLIENT_ID)

# 显式指定 MBP-1 schema
self.subscribe_quote_ticks(
    instrument_id=instrument_id,
    params={"schema": "mbp-1"},
    client_id=DATABENTO_CLIENT_ID,
)

# 1 秒 BBO 快照（适配器仅发出 QuoteTick）
self.subscribe_quote_ticks(
    instrument_id=instrument_id,
    params={"schema": "bbo-1s"},
    client_id=DATABENTO_CLIENT_ID,
)

# 跨交易场所整合的报价
self.subscribe_quote_ticks(
    instrument_id=instrument_id,
    params={"schema": "cbbo-1s"},  # 或 "cmbp-1" 用于整合的 MBP
    client_id=DATABENTO_CLIENT_ID,
)

# 交易采样的 BBO（包含报价和成交数据）
self.subscribe_quote_ticks(
    instrument_id=instrument_id,
    params={"schema": "tbbo"},  # 将在消息总线上接收 QuoteTick 和 TradeTick
    client_id=DATABENTO_CLIENT_ID,
)
```

### 成交订阅

```python
# 仅成交 tick
self.subscribe_trade_ticks(instrument_id, client_id=DATABENTO_CLIENT_ID)

# 来自 MBP-1 数据源的成交（仅在发生交易事件时）
self.subscribe_trade_ticks(
    instrument_id=instrument_id,
    params={"schema": "mbp-1"},
    client_id=DATABENTO_CLIENT_ID,
)

# 交易采样数据（包含交易时刻的报价）
self.subscribe_trade_ticks(
    instrument_id=instrument_id,
    params={"schema": "tbbo"},  # 同时在交易事件时提供报价
    client_id=DATABENTO_CLIENT_ID,
)
```

### 订单簿深度订阅（MBP 与 L2）

```python
# 订阅前 10 个层级的市场深度
self.subscribe_order_book_depth(
    instrument_id=instrument_id,
    depth=10,  # 自动选择 MBP-10 schema
)

# depth 参数对于 Databento 必须为 10
# 将接收 OrderBookDepth10 更新
```

### 订单簿增量订阅（MBO 与 L3）

```python
# 订阅完整的订单簿更新（按订单聚合）
self.subscribe_order_book_deltas(
    instrument_id=instrument_id,
    book_type=BookType.L3_MBO,  # 使用 MBO schema
)

# 在节点启动时进行 MBO 订阅，以便 Databento 能够从会话开始回放
```

### K 线订阅

```python
# 订阅 1 分钟 K 线（自动使用 ohlcv-1m schema）
self.subscribe_bars(bar_type=BarType.from_str(f"{instrument_id}-1-MINUTE-LAST-EXTERNAL"))

# 订阅 1 秒 K 线（自动使用 ohlcv-1s schema）
self.subscribe_bars(bar_type=BarType.from_str(f"{instrument_id}-1-SECOND-LAST-EXTERNAL"))

# 订阅 1 小时 K 线（自动使用 ohlcv-1h schema）
self.subscribe_bars(bar_type=BarType.from_str(f"{instrument_id}-1-HOUR-LAST-EXTERNAL"))

# 订阅日 K 线（自动使用 ohlcv-1d schema）
self.subscribe_bars(bar_type=BarType.from_str(f"{instrument_id}-1-DAY-LAST-EXTERNAL"))

# 使用适配器的收盘（end-of-day）覆盖项订阅日 K 线
self.subscribe_bars(
    bar_type=BarType.from_str(f"{instrument_id}-1-DAY-LAST-EXTERNAL"),
    params={"schema": "ohlcv-eod"},
)
```

### 自定义数据类型订阅

Imbalance、statistics 和 status 数据需要使用通用的 `subscribe_data` 方法：

```python
from nautilus_trader.adapters.databento import DATABENTO_CLIENT_ID
from nautilus_trader.adapters.databento import DatabentoImbalance
from nautilus_trader.adapters.databento import DatabentoStatistics
from nautilus_trader.model import DataType

# 订阅 imbalance 数据
self.subscribe_data(
    data_type=DataType(DatabentoImbalance, metadata={"instrument_id": instrument_id}),
    client_id=DATABENTO_CLIENT_ID,
)

# 订阅 statistics 数据
self.subscribe_data(
    data_type=DataType(DatabentoStatistics, metadata={"instrument_id": instrument_id}),
    client_id=DATABENTO_CLIENT_ID,
)

# 订阅金融工具状态更新
from nautilus_trader.model.data import InstrumentStatus

self.subscribe_data(
    data_type=DataType(InstrumentStatus, metadata={"instrument_id": instrument_id}),
    client_id=DATABENTO_CLIENT_ID,
)
```

## 金融工具 ID 和符号体系

Databento 市场数据包含一个 `instrument_id` 字段：在大多数情况下，这是由发布者分配的数字 ID，
当发布者未提供时则由 Databento 合成。Databento 仅保证该 ID 在指定的某一天内唯一。
这与 Nautilus 的 `InstrumentId` 不同，后者是由符号 + 交易场所组成、以句号分隔的字符串：`"{symbol}.{venue}"`。

解码器将 Databento 的 `raw_symbol` 映射为 Nautilus 的 `symbol`，并使用定义消息中的
[ISO 10383 市场标识码](https://www.iso20022.org/market-identifier-codes)作为 Nautilus 的 `venue`。

Databento 用*数据集 ID*（dataset ID）来标识数据集，这与交易场所标识符不同。
详情参见 [Databento 数据集命名约定](https://databento.com/docs/api-reference-historical/basics/datasets)。

对于 CME Globex MDP 3.0（`GLBX.MDP3`），以下交易所都归入 `GLBX` 交易场所下。
金融工具的 `exchange` 字段决定了具体的映射：

- `CBCM`：XCME-XCBT 跨交易所价差
- `NYUM`：XNYM-DUMX 跨交易所价差
- `XCBT`：芝加哥期货交易所（CBOT）
- `XCEC`：商品交易中心（COMEX）
- `XCME`：芝加哥商品交易所（CME）
- `XFXS`：CME FX Link 价差
- `XNYM`：纽约商品交易所（NYMEX）

:::info
其他交易场所 MIC 可以在 [metadata.list_publishers](https://databento.com/docs/api-reference-historical/metadata/metadata-list-publishers) 端点响应的 `venue` 字段中找到。
:::

## 时间戳

Databento 数据包含以下时间戳字段：

- `ts_event`：撮合引擎接收时间戳，表示为自 UNIX 纪元以来的纳秒数。
- `ts_in_delta`：撮合引擎发送时间戳，表示为 `ts_recv` 之前的纳秒数。
- `ts_recv`：采集服务器接收时间戳，表示为自 UNIX 纪元以来的纳秒数。
- `ts_out`：Databento 发送时间戳（仅实时）。

Nautilus 数据至少需要两个时间戳（由 `Data` 契约要求）：

- `ts_event`：数据事件发生的 UNIX 时间戳（纳秒）。
- `ts_init`：数据实例创建的 UNIX 时间戳（纳秒）。

解码器将 Databento 的 `ts_recv` 映射为 Nautilus 的 `ts_event`。该时间戳更可靠，
并且按 Databento 符号单调递增。例外是 `DatabentoImbalance` 和 `DatabentoStatistics`，
它们携带所有时间戳字段，因为它们是适配器专用的类型。

:::info
更多信息请参阅以下 Databento 文档：

- [Databento standards and conventions - timestamps](https://databento.com/docs/standards-and-conventions/common-fields-enums-types#timestamps)
- [Databento timestamping guide](https://databento.com/docs/architecture/timestamping-guide)

:::

## 数据类型

本节将 Databento schema 映射到 Nautilus 数据类型。

:::info
参见 Databento 的 [schemas and data formats](https://databento.com/docs/schemas-and-data-formats)。
:::

### 金融工具定义

Databento 对所有金融工具类别使用单一 schema。解码器会将每一类映射到相应的 Nautilus `Instrument` 类型。

| Databento 金融工具类别 | 代码  | Nautilus 金融工具类型   |
| ---------------- | --- | ----------------- |
| Stock            | `K` | `Equity`          |
| Future           | `F` | `FuturesContract` |
| Call             | `C` | `OptionContract`  |
| Put              | `P` | `OptionContract`  |
| Future spread    | `S` | `FuturesSpread`   |
| Option spread    | `T` | `OptionSpread`    |
| Mixed spread     | `M` | `OptionSpread`    |
| FX spot          | `X` | `CurrencyPair`    |
| Bond             | `B` | 尚未支持              |

### 价格精度

Databento 的原始价格是按 1e-9 缩放的定点整数。适配器从定义消息中金融工具的 tick size 推导出价格精度。

对于实时数据源，数据源处理器会维护一个按金融工具划分的精度映射，该映射由到达的
`InstrumentDefMsg` 记录填充。市场数据处理器按以下顺序解析精度：

1. 针对 Databento 记录 `instrument_id` 的 `InstrumentDefMsg` 元数据。
2. 由 Python 订阅路径传入的已缓存金融工具精度。
3. 传入直连实时客户端的显式 `price_precisions`。
4. USD 的默认精度 2。

回退映射在符号映射之后以 Databento 记录的 `instrument_id` 为键，
因此父级、连续合约及其他非原始符号体系的请求，在定义元数据到达之前，
仍可使用已缓存的或显式提供的精度。

对于具有非标准 tick size 的金融工具（例如以 1/256 等分数 tick 的国债期货），
**金融工具定义必须在市场数据之前到达**才能获得正确的精度。请在订阅市场数据之前或同时，
为你的金融工具订阅 `DEFINITION` schema。

对于历史请求和基于文件的加载，精度按记录、依以下顺序解析：

1. 调用时显式传入的 `price_precision` 参数。
2. 由加载定义填充的按符号缓存（文件加载器上的 `load_instruments`、
   历史客户端上的 `get_range_instruments`），或由显式的
   `set_price_precision(symbol, precision)` 调用填充。

Python 数据客户端会在每次请求前，从金融工具提供器为历史客户端缓存预填充数据，
因此已加载的金融工具无需额外配置。当精度无法解析时，加载会以一个明确的错误失败，
而不是悄悄地回退到 USD 精度。

:::tip
Python 适配器会在市场数据之前自动订阅金融工具定义，并将已缓存的金融工具精度作为回退传入，
因此精度映射无需额外配置即可填充。对于直接使用 Rust 客户端的情形，
请在市场数据之前订阅 `DEFINITION` schema，或传入显式的精度回退值。
:::

### MBO（按订单聚合）

MBO 是 Databento 提供的最高粒度数据，代表完整的订单簿深度。某些消息包含成交数据。
解码器会产生一个 `OrderBookDelta`，并可选地产生一个 `TradeTick`。

实时客户端会缓冲 MBO 消息，直到看到 `F_LAST` 标志，然后将一个 `OrderBookDeltas` 容器传递给处理器。

客户端还会在回放启动序列期间，将订单簿快照缓冲到 `OrderBookDeltas` 中。

### MBP-1（按价格聚合，最优价）

MBP-1 代表最优价的报价和成交。某些消息携带成交数据。
解码器会产生一个 `QuoteTick`，当消息为成交时还会产生一个 `TradeTick`。

### TBBO 和 TCBBO（带成交的最优价）

TBBO 和 TCBBO 在每条消息中同时提供报价和成交数据。两种 schema 每条消息都会发出
`QuoteTick` 和 `TradeTick`，比分别订阅报价和成交更高效。TCBBO 提供跨交易场所整合的数据。

#### 成交 ID 的派生（CMBP-1 和 TCBBO）

CMBP-1 和 TCBBO schema 不发布原生的成交标识符。解码器通过对金融工具 ID、
`ts_event`、`ts_recv`、价格、数量以及成交的主动方（aggressor side）进行 FNV-1a 哈希，
派生出一个确定性的 `TradeId`。同一个交易场所事件在多次回放中会产生相同的成交 ID，
因此下游的去重逻辑保持完好。两笔逻辑上不同但字段完全相同的成交会发生碰撞；
这与交易场所本身无法区分它们的情况一致。

### OHLCV（K 线聚合）

Databento 将 K 线消息的时间戳标记在该间隔的**开盘**时刻。
解码器会将 `ts_event` 标准化到 K 线的**收盘**时刻（原始 `ts_event` + 间隔）。

### Imbalance 和 Statistics

`imbalance` 和 `statistics` schema 没有内置的 Nautilus 对应类型。
适配器在 Rust 中定义了 `DatabentoImbalance` 和 `DatabentoStatistics`。

PyO3 绑定在 Python 中暴露了这些类型。它们的属性是 PyO3 对象，
可能无法与期望 Cython 类型的方法一起使用。请参阅 API 参考了解 PyO3 到 Cython 的转换方法。

将 PyO3 `Price` 转换为 Cython `Price`：

```python
price = Price.from_raw(pyo3_price.raw, pyo3_price.precision)
```

请求和订阅这些类型需要使用通用的 `subscribe_data` 方法。为 `AAPL.XNAS` 订阅 `imbalance`：

```python
from nautilus_trader.adapters.databento import DATABENTO_CLIENT_ID
from nautilus_trader.adapters.databento import DatabentoImbalance
from nautilus_trader.model import DataType

instrument_id = InstrumentId.from_str("AAPL.XNAS")
self.subscribe_data(
    data_type=DataType(DatabentoImbalance, metadata={"instrument_id": instrument_id}),
    client_id=DATABENTO_CLIENT_ID,
)
```

请求 `ES.FUT` 父符号（所有活跃的 E-mini S&P 500 期货）前一天的 `statistics`：

```python
from nautilus_trader.adapters.databento import DATABENTO_CLIENT_ID
from nautilus_trader.adapters.databento import DatabentoStatistics
from nautilus_trader.model import DataType

instrument_id = InstrumentId.from_str("ES.FUT.GLBX")
metadata = {
    "instrument_id": instrument_id,
    "start": "2024-03-06",
}
self.request_data(
    data_type=DataType(DatabentoStatistics, metadata=metadata),
    client_id=DATABENTO_CLIENT_ID,
)
```

### 数据目录持久化

这两种类型都支持用于数据目录存储的 Arrow 序列化。当你导入适配器包时，Arrow 序列化器会自动注册。

#### 写入数据目录

```python
from nautilus_trader.adapters.databento import DatabentoDataLoader
from nautilus_trader.model.identifiers import InstrumentId
from nautilus_trader.persistence.catalog import ParquetDataCatalog

catalog = ParquetDataCatalog.from_env()
loader = DatabentoDataLoader()

imbalances = loader.from_dbn_file(
    path="aapl-imbalance.dbn.zst",
    instrument_id=InstrumentId.from_str("AAPL.XNAS"),
    as_legacy_cython=False,  # Databento 专用类型必需
)

catalog.write_data(imbalances)
```

#### 从数据目录读取

```python
from nautilus_trader.adapters.databento import DatabentoImbalance

results = catalog.query(DatabentoImbalance, identifiers=["AAPL.XNAS"])

for imbalance in results:
    print(imbalance.ref_price)  # DatabentoImbalance 字段
```

:::warning
数据目录持久化支持写入和查询这些类型，但目前还不支持通过 `BacktestNode`
或 `BacktestEngine` 对它们进行流式处理。若要使用 imbalance 或 statistics 数据进行回测，
请直接查询数据目录，并在你的策略或分析代码中处理结果。
:::

#### 在 Rust 中编码和解码

`nautilus_databento::arrow` 模块提供 Arrow record batch 的编码和解码。请启用 `arrow` 特性标志。

```rust
use nautilus_databento::arrow::imbalance::{
    decode_imbalance_batch,
    imbalance_to_arrow_record_batch,
};

let batch = imbalance_to_arrow_record_batch(imbalances)?;

let metadata = batch.schema().metadata().clone();
let decoded = decode_imbalance_batch(&metadata, batch)?;
```

`statistics` 模块遵循相同的模式，使用 `decode_statistics_batch` 和 `statistics_to_arrow_record_batch`。

## 性能考虑

使用 DBN 数据进行回测时有两种选择：

- 将数据存储为 DBN（`.dbn.zst`）文件，每次运行时解码为 Nautilus 对象。
- 将 DBN 文件一次性转换为 Nautilus 对象并写入数据目录（Nautilus Parquet 格式）。

DBN 解码器是经过优化的 Rust 实现，但只写入一次数据目录能带来最佳的回测性能。

[DataFusion](https://arrow.apache.org/datafusion/) 能以高吞吐量从磁盘流式读取 Nautilus Parquet 数据，
至少比每次运行时解码 DBN 快一个数量级。

:::note
性能基准测试目前正在开发中。
:::

## 加载 DBN 数据

`DatabentoDataLoader` 类负责加载 DBN 文件并将记录转换为 Nautilus 对象。两个主要用途：

- 将数据传递给 `BacktestEngine.add_data` 用于回测。
- 将数据写入 `ParquetDataCatalog`，以便通过 `BacktestNode` 进行流式处理。

### DBN 数据到 BacktestEngine

加载 DBN 数据并传递给 `BacktestEngine`。引擎需要一个金融工具。
本示例使用 `TestInstrumentProvider`（从 DBN 文件解析出的金融工具同样可用）。
数据涵盖纳斯达克上一个月的 TSLA 成交：

```python
# 添加金融工具
TSLA_NASDAQ = TestInstrumentProvider.equity(symbol="TSLA")
engine.add_instrument(TSLA_NASDAQ)

# 将数据解码为 Cython 对象
loader = DatabentoDataLoader()
trades = loader.from_dbn_file(
    path=TEST_DATA_DIR / "databento" / "temp" / "tsla-xnas-20240107-20240206.trades.dbn.zst",
    instrument_id=TSLA_NASDAQ.id,
)

# 添加数据
engine.add_data(trades)
```

### DBN 数据到 ParquetDataCatalog

加载 DBN 数据并写入 `ParquetDataCatalog`。设置 `as_legacy_cython=False` 以将数据解码为 PyO3 对象。

### 加载金融工具

**重要**：在将市场数据加载到目录之前，先从 DEFINITION schema 文件加载金融工具定义。
目录需要先有金融工具才能存储市场数据。市场数据文件不包含金融工具定义。

```python
# 初始化目录接口
# （将使用 `NAUTILUS_PATH` 环境变量作为路径）
catalog = ParquetDataCatalog.from_env()

loader = DatabentoDataLoader()

# 步骤 1：首先加载金融工具定义
# 从 Databento 获取你的金融工具的 DEFINITION schema 文件
instruments = loader.from_dbn_file(
    path=TEST_DATA_DIR / "databento" / "temp" / "tsla-xnas-definition.dbn.zst",
    as_legacy_cython=False,  # 使用 PyO3 以获得最佳性能
)

# 将金融工具写入目录
catalog.write_data(instruments)

# 步骤 2：现在加载并写入市场数据
instrument_id = InstrumentId.from_str("TSLA.XNAS")

# 将成交数据解码为 PyO3 对象
trades = loader.from_dbn_file(
    path=TEST_DATA_DIR / "databento" / "temp" / "tsla-xnas-20240107-20240206.trades.dbn.zst",
    instrument_id=instrument_id,
    as_legacy_cython=False,  # 这是写入目录的优化
)

# 写入市场数据
catalog.write_data(trades)
```

#### 为回测加载多种数据类型

始终先加载金融工具，再加载市场数据：

```python
from nautilus_trader.adapters.databento.loaders import DatabentoDataLoader
from nautilus_trader.model.identifiers import InstrumentId
from nautilus_trader.persistence.catalog import ParquetDataCatalog

catalog = ParquetDataCatalog.from_env()
loader = DatabentoDataLoader()

# 步骤 1：从 DEFINITION 文件加载金融工具定义
instruments = loader.from_dbn_file(
    path="equity-definitions.dbn.zst",
    as_legacy_cython=False,
)
catalog.write_data(instruments)

# 步骤 2：加载市场数据（MBO、trades、quotes 等）
instrument_id = InstrumentId.from_str("AAPL.XNAS")

# 加载 MBO 订单簿增量
deltas = loader.from_dbn_file(
    path="aapl-mbo.dbn.zst",
    instrument_id=instrument_id,  # 可选，但可提高性能
    as_legacy_cython=False,
)
catalog.write_data(deltas)

# 加载成交数据
trades = loader.from_dbn_file(
    path="aapl-trades.dbn.zst",
    instrument_id=instrument_id,
    as_legacy_cython=False,
)
catalog.write_data(trades)

# 验证金融工具已在目录中
print(catalog.instruments())  # 显示你已加载的金融工具
```

:::tip
调用 `catalog.instruments()` 进行验证。返回空列表意味着你需要先加载 DEFINITION 文件。
:::

:::info
通过 Databento API 或 CLI，为你的符号和日期范围下载 DEFINITION schema 文件。
详情参见 [Databento 文档](https://databento.com/docs/api-reference-historical/timeseries/timeseries-get-range)。
:::

:::info
另请参阅[数据概念指南](../concepts/data.md)。
:::

### 历史加载器选项

`from_dbn_file` 的参数：

- `instrument_id`：通过跳过符号查找来加快解码速度。
- `price_precision`：应用于每条读取记录的覆盖值。省略时，
  加载器会从其缓存（由 `load_instruments` 或 `set_price_precision` 填充）中按符号解析精度；
  若无法解析则加载失败。
- `include_trades`：对于 MBP-1/CMBP-1 schema，当存在成交数据时，
  `True` 会同时发出 `QuoteTick` 和 `TradeTick`。
- `as_legacy_cython`：对于 IMBALANCE/STATISTICS schema 设置为 `False`（必需），
  或为获得更好的目录写入性能而设置为 `False`。

:::warning
IMBALANCE 和 STATISTICS schema 要求 `as_legacy_cython=False`（仅 PyO3 类型）。
设置为 `True` 会引发 `ValueError`。
:::

### 加载整合数据

整合类 schema 跨多个交易场所聚合数据：

```python
# 加载整合的 MBP-1 报价
loader = DatabentoDataLoader()
cmbp_quotes = loader.from_dbn_file(
    path="consolidated.cmbp-1.dbn.zst",
    instrument_id=InstrumentId.from_str("AAPL.XNAS"),
    include_trades=True,  # 如果可用，同时包含报价和成交数据
    as_legacy_cython=True,
)

# 加载整合的 BBO 报价
cbbo_quotes = loader.from_dbn_file(
    path="consolidated.cbbo-1s.dbn.zst",
    instrument_id=InstrumentId.from_str("AAPL.XNAS"),
    as_legacy_cython=False,  # 使用 PyO3 以获得更好性能
)

# 加载 TCBBO（交易采样整合 BBO），同时带报价和成交
# include_trades=True 加载报价，include_trades=False 加载成交
tcbbo_quotes = loader.from_dbn_file(
    path="consolidated.tcbbo.dbn.zst",
    instrument_id=InstrumentId.from_str("AAPL.XNAS"),
    include_trades=True,  # 加载报价
    as_legacy_cython=True,
)

tcbbo_trades = loader.from_dbn_file(
    path="consolidated.tcbbo.dbn.zst",
    instrument_id=InstrumentId.from_str("AAPL.XNAS"),
    include_trades=False,  # 加载成交
    as_legacy_cython=True,
)
```

:::tip
避免对同一金融工具同时订阅 TBBO/TCBBO 和单独的成交数据源。
这些 schema 已包含成交数据。重复订阅会浪费成本并产生重复数据。
:::

## 实时客户端架构

`DatabentoDataClient` 封装了其他 Databento 适配器类。每个数据集使用两个 `DatabentoLiveClient` 实例：

- 一个用于 MBO（订单簿增量）实时数据源
- 一个用于所有其他实时数据源

:::warning
请在节点启动时进行某数据集的所有 MBO 订阅，以便从会话开始回放。
启动后到达的订阅会被客户端记录为错误并被忽略。

此限制不适用于其他 schema。
:::

单个 `DatabentoHistoricalClient` 同时为 `DatabentoInstrumentProvider`
和 `DatabentoDataClient` 提供历史请求服务。

## 配置

在你的 `TradingNode` 客户端配置中添加一个 `DATABENTO` 部分：

```python
from nautilus_trader.adapters.databento import DATABENTO
from nautilus_trader.live.node import TradingNode

config = TradingNodeConfig(
    data_clients={
        DATABENTO: {
            "api_key": None,  # 'DATABENTO_API_KEY' 环境变量
            "http_gateway": None,  # 覆盖默认的 HTTP 历史网关
            "live_gateway": None,  # 覆盖默认的原始 TCP 实时网关
            "instrument_provider": InstrumentProviderConfig(load_all=True),
            "instrument_ids": None,  # 启动时加载的 Nautilus 金融工具 ID
            "parent_symbols": None,  # 启动时加载的 Databento 父符号
        },
    },
)
```

创建 `TradingNode` 并注册工厂：

```python
from nautilus_trader.adapters.databento.factories import DatabentoLiveDataClientFactory
from nautilus_trader.live.node import TradingNode

# 使用配置创建实时交易节点
node = TradingNode(config=config)

# 向节点注册客户端工厂
node.add_data_client_factory(DATABENTO, DatabentoLiveDataClientFactory)

# 构建节点
node.build()
```

### 配置参数

| 选项                        | 默认值    | 描述                                                                                    |
| ------------------------- | ------ | ------------------------------------------------------------------------------------- |
| `api_key`                 | `None` | Databento API 密钥。为 `None` 时回退到 `DATABENTO_API_KEY` 环境变量。                              |
| `http_gateway`            | `None` | 历史 HTTP 网关覆盖，用于测试自定义端点。                                                               |
| `live_gateway`            | `None` | 原始 TCP 实时网关覆盖，通常仅用于测试。                                                                |
| `use_exchange_as_venue`   | `True` | 使用交易所 MIC 作为 Nautilus 交易场所（例如 `XCME`）。为 `False` 时保留默认的 GLBX 映射。                       |
| `timeout_initial_load`    | `15.0` | 在继续之前，等待每个数据集的金融工具定义加载完成的秒数。                                                          |
| `mbo_subscriptions_delay` | `3.0`  | 启用 MBO/L3 流之前缓冲的秒数，以便初始快照按顺序回放。                                                       |
| `bars_timestamp_on_close` | `True` | 在收盘时为 K 线打时间戳（`ts_event`/`ts_init`）。为 `False` 时在开盘时打时间戳。                              |
| `reconnect_timeout_mins`  | `10`   | 放弃前尝试重连的分钟数。为 `None` 时无限重试。参见[连接稳定性](#连接稳定性)。                                         |
| `venue_dataset_map`       | `None` | Nautilus 交易场所到 Databento 数据集代码的可选映射。                                                  |
| `parent_symbols`          | `None` | 可选的 `{dataset: {parent symbols}}`，用于预加载定义树（例如 `{"GLBX.MDP3": {"ES.FUT", "ES.OPT"}}`）。 |
| `instrument_ids`          | `None` | 启动时预加载定义的 Nautilus `InstrumentId` 值。                                                  |

:::tip
建议使用环境变量来管理凭证。
:::

### 连接稳定性

实时客户端会在以下情况自动重连：

- **网络中断**：临时的连接问题。
- **网关重启**：Databento 每周日的维护。参见
  [维护计划](https://databento.com/docs/api-reference-live/basics#maintenance-schedule)。
- **市场收盘**：在非交易时段结束的会话。

#### 重连策略

退避策略取决于超时配置：

**有超时**（默认 10 分钟）：

- 指数退避，上限为 **60 秒**。
- 模式：1 秒、2 秒、4 秒、8 秒、16 秒、32 秒、60 秒、60 秒……（带抖动）。
- 在超时窗口内快速重连。

**无超时**（`reconnect_timeout_mins=None`）：

- 指数退避，上限为 **10 分钟**。
- 模式：1 秒、2 秒、4 秒、8 秒、16 秒、32 秒、64 秒、128 秒、256 秒、512 秒、600 秒、600 秒……（带抖动）。
- 适合无人值守系统度过夜间收盘和计划维护。

所有重连都包括：

- **抖动**：随机延迟（最多 1 秒），以防止同步重连风暴。
- **自动重新订阅**：重连后恢复所有活跃订阅。
- **周期重置**：每次成功会话（>60 秒）重置超时计时器。

#### 超时配置

`reconnect_timeout_mins` 参数控制客户端尝试重连的时长：

**默认（10 分钟）**：适合大多数使用场景。

- 处理瞬态网络问题。
- 安全度过计划中的网关重启。
- 在市场收盘时停止重试，避免夜间空转。
- 较长时间的中断需要手动干预。

:::warning
设置 `reconnect_timeout_mins=None` 会无限重试。仅用于必须在无人值守情况下
度过夜间市场收盘的系统。这可能掩盖持续性的配置或认证问题。
:::

#### 定期维护

Databento 每周日重启其实时网关（所有客户端将断开连接）：

| 数据集         | 维护时间（UTC） |
| ----------- | --------- |
| CME Globex  | 09:30     |
| 所有 ICE 交易场所 | 09:45     |
| 所有其他数据集     | 10:30     |

默认的 10 分钟超时可处理典型的重启。对于无人值守系统，
使用 `reconnect_timeout_mins=None` 或更长的值。详情参见
[Databento Maintenance Schedule](https://databento.com/docs/api-reference-live/basics/maintenance-schedule)。

## 贡献

:::info
如需贡献，请参阅
[贡献指南](https://github.com/nautechsystems/nautilus_trader/blob/develop/CONTRIBUTING.md)。
:::
