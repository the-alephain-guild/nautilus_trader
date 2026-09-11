# 数据测试规范 (Data Testing Spec)

本节定义了一套严格的测试矩阵，用于通过 `DataTester` actor 验证适配器的数据功能。Python
（`nautilus_trader.test_kit.strategies.tester_data`）和 Rust
（`nautilus_testkit::testers`）都提供了 `DataTester`。每个测试用例都由一个带前缀的 ID
（例如 TC-D01）标识，并按功能分组。

**每个适配器都必须通过与其所支持数据类型相匹配的那部分测试。**

测试组按从最基础到最派生的数据顺序排列：先是工具 (instruments) 和原始订单簿数据，
然后是报价 (quotes)、成交 (trades)、K 线 (bars) 以及衍生品数据。
通过第 1–4 组的适配器即被视为符合基线数据要求。

请将适配器特定的数据行为（自定义频道、节流、快照语义等）记录在适配器自己的指南中，
而不是这里。

## 前置条件

运行数据测试之前：

- 目标 instrument 可用，且可通过 instrument provider 加载。
- 当所测数据所在的交易场所要求鉴权时，通过环境变量（`{VENUE}_API_KEY`、`{VENUE}_API_SECRET`）
  设置 API 凭据。
- 如果交易场所提供 demo/testnet 模式，请使用为该环境创建的凭据。Demo 凭据与生产
  API key 通常是分开的、不可互换的；使用错误的凭据会产生鉴权错误（例如 HTTP 401）。

**Python 节点设置**：

旧示例仍在使用 `nautilus_trader.live.node.TradingNode`，但新的、基于 Rust 的
PyO3 适配器应优先使用 `nautilus_trader.live.LiveNode`。当你需要在节点构建之前注册
适配器客户端工厂时，请使用 `LiveNode.builder(...)`。

```python
from nautilus_trader.common import Environment
from nautilus_trader.live import LiveDataEngineConfig, LiveNode
from nautilus_trader.model import TraderId

node = (
    LiveNode.builder("TESTER-001", TraderId("TESTER-001"), Environment.SANDBOX)
    .with_data_engine_config(LiveDataEngineConfig(time_bars_build_with_no_updates=False))
    .add_data_client(None, adapter_data_client_factory, data_client_config)
    .build()
)

node.add_actor_from_config(importable_actor_config)
# Register remaining components, then start or run
```

**Rust 节点设置**（参考：`crates/adapters/{adapter}/examples/node_data_tester.rs`）：

```rust
use nautilus_testkit::testers::{DataTester, DataTesterConfig};

let tester_config = DataTesterConfig::new(client_id, vec![instrument_id])
    .with_subscribe_quotes(true);
let tester = DataTester::new(tester_config);
node.add_actor(tester)?;
node.run().await?;
```

下面的每个组都以一张概览表开始，随后是详细的测试卡片。
测试 ID 采用带间隔的编号方式，以便插入新用例时无需重新编号。

---

## 第 1 组：Instruments

在测试市场数据流之前，先验证 instrument 的加载与订阅。

| TC     | 名称              | 描述                       | 何时跳过               |
| ------ | --------------- | ------------------------ | ------------------ |
| TC-D01 | 请求 instruments  | 加载一个交易场所的全部 instruments。 | 永不。                |
| TC-D02 | 订阅 instrument   | 订阅 instrument 更新。        | 不支持 instrument 订阅。 |
| TC-D03 | 加载指定 instrument | 按 ID 加载单个 instrument。    | 永不。                |

### TC-D01: 请求 instruments

| 字段       | 值                                             |
| -------- | --------------------------------------------- |
| **前置条件** | 适配器已连接。                                       |
| **动作**   | DataTester 在启动时请求该交易场所的全部 instruments。        |
| **事件序列** | `on_instruments` 回调接收到 instrument 列表。         |
| **通过标准** | 至少接收到一个 instrument；每个都具有有效的 symbol、价格精度和数量增量。 |
| **何时跳过** | 永不。                                           |

**Python 配置：**

```python
DataTesterConfig(
    instrument_ids=[instrument_id],
    request_instruments=True,
)
```

**Rust 配置：**

```rust
DataTesterConfig::new(client_id, vec![instrument_id])
    .with_request_instruments(true)
```

### TC-D02: 订阅 instrument

| 字段       | 值                                              |
| -------- | ---------------------------------------------- |
| **前置条件** | 适配器已连接，instrument 已加载。                         |
| **动作**   | DataTester 订阅 instrument 更新。                   |
| **事件序列** | `on_instrument` 回调接收到 instrument。              |
| **通过标准** | 接收到的 instrument 具有正确的 `instrument_id` 和有效的各字段。 |
| **何时跳过** | 适配器不支持 instrument 订阅。                          |

**Python 配置：**

```python
DataTesterConfig(
    instrument_ids=[instrument_id],
    subscribe_instrument=True,
)
```

**Rust 配置：**

```rust
DataTesterConfig::new(client_id, vec![instrument_id])
    .with_subscribe_instrument(true)
```

### TC-D03: 加载指定 instrument

| 字段       | 值                                                         |
| -------- | --------------------------------------------------------- |
| **前置条件** | 适配器已连接。                                                   |
| **动作**   | 通过 instrument provider 按 `InstrumentId` 加载指定的 instrument。 |
| **事件序列** | 加载后 instrument 在 cache 中可用。                               |
| **通过标准** | instrument 已加载，具有正确的 ID、价格精度、数量增量和交易规则。                   |
| **何时跳过** | 永不。                                                       |

**注意事项：**

- 这会直接测试 instrument provider 的 `load` / `load_async` 方法。
- 验证 instrument 已被缓存，且可通过 `self.cache.instrument(instrument_id)` 获取。

---

## 第 2 组：Order book

测试订单簿的订阅模式与快照请求。

| TC     | 名称               | 描述                         | 何时跳过          |
| ------ | ---------------- | -------------------------- | ------------- |
| TC-D10 | 订阅订单簿 deltas     | 流式接收 `OrderBookDeltas` 更新。 | 不支持订单簿。       |
| TC-D11 | 按间隔订阅订单簿         | 周期性的 `OrderBook` 快照。       | 不支持订单簿。       |
| TC-D12 | 订阅订单簿深度          | `OrderBookDepth10` 快照。     | 不支持订单簿深度。     |
| TC-D13 | 请求订单簿快照          | 一次性的订单簿快照请求。               | 不支持订单簿快照。     |
| TC-D14 | 由 deltas 维护本地订单簿 | 从 delta 流构建本地订单簿。          | 不支持订单簿。       |
| TC-D15 | 请求历史订单簿 deltas   | 历史订单簿 deltas 请求。           | 不支持历史 deltas。 |

### TC-D10: 订阅订单簿 deltas

| 字段       | 值                                                   |
| -------- | --------------------------------------------------- |
| **前置条件** | 适配器已连接，instrument 已加载。                              |
| **动作**   | DataTester 订阅订单簿 deltas。                            |
| **事件序列** | 在 `on_order_book_deltas` 中接收到 `OrderBookDeltas` 事件。 |
| **通过标准** | 接收到的 deltas 具有有效的 instrument ID；至少一个 delta 包含买/卖更新。 |
| **何时跳过** | 适配器不支持订单簿数据。                                        |

**Python 配置：**

```python
DataTesterConfig(
    instrument_ids=[instrument_id],
    subscribe_book_deltas=True,
    book_type=BookType.L2_MBP,
)
```

**Rust 配置：**

```rust
DataTesterConfig::new(client_id, vec![instrument_id])
    .with_subscribe_book_deltas(true)
    .with_book_type(BookType::L2_MBP)
```

### TC-D11: 按间隔订阅订单簿

| 字段       | 值                                            |
| -------- | -------------------------------------------- |
| **前置条件** | 适配器已连接，instrument 已加载。                       |
| **动作**   | DataTester 订阅周期性的订单簿快照。                      |
| **事件序列** | 在 `on_order_book` 中按配置的间隔接收到 `OrderBook` 事件。 |
| **通过标准** | 接收到带有买/卖档位的订单簿快照；更新大约按配置的间隔到达。               |
| **何时跳过** | 适配器不支持订单簿数据。                                 |

**Python 配置：**

```python
DataTesterConfig(
    instrument_ids=[instrument_id],
    subscribe_book_at_interval=True,
    book_type=BookType.L2_MBP,
    book_depth=10,
    book_interval_ms=1000,
)
```

**Rust 配置：**

```rust
DataTesterConfig::new(client_id, vec![instrument_id])
    .with_subscribe_book_at_interval(true)
    .with_book_type(BookType::L2_MBP)
    .with_book_depth(Some(NonZeroUsize::new(10).unwrap()))
    .with_book_interval_ms(NonZeroUsize::new(1000).unwrap())
```

### TC-D12: 订阅订单簿深度

| 字段       | 值                                                   |
| -------- | --------------------------------------------------- |
| **前置条件** | 适配器已连接，instrument 已加载。                              |
| **动作**   | DataTester 订阅 `OrderBookDepth10` 快照。                |
| **事件序列** | 在 `on_order_book_depth` 中接收到 `OrderBookDepth10` 事件。 |
| **通过标准** | 接收到最多 10 档买/卖的深度快照；价格排序正确。                          |
| **何时跳过** | 适配器不支持订单簿深度订阅。                                      |

**Python 配置：**

```python
DataTesterConfig(
    instrument_ids=[instrument_id],
    subscribe_book_depth=True,
    book_type=BookType.L2_MBP,
    book_depth=10,
)
```

**Rust 配置：** 尚不支持。订单簿深度订阅在 Rust 版 `DataTester` 中仍是 TODO。

### TC-D13: 请求订单簿快照

| 字段       | 值                       |
| -------- | ----------------------- |
| **前置条件** | 适配器已连接，instrument 已加载。  |
| **动作**   | DataTester 请求一次性的订单簿快照。 |
| **事件序列** | 通过历史数据回调接收到订单簿快照。       |
| **通过标准** | 快照包含买/卖档位，价格和数量均有效。     |
| **何时跳过** | 适配器不支持订单簿快照请求。          |

**Python 配置：**

```python
DataTesterConfig(
    instrument_ids=[instrument_id],
    request_book_snapshot=True,
    book_depth=10,
)
```

**Rust 配置：**

```rust
DataTesterConfig::new(client_id, vec![instrument_id])
    .with_request_book_snapshot(true)
    .with_book_depth(Some(NonZeroUsize::new(10).unwrap()))
```

### TC-D14: 由 deltas 维护本地订单簿

| 字段       | 值                                                           |
| -------- | ----------------------------------------------------------- |
| **前置条件** | 适配器已连接，instrument 已加载，订单簿 deltas 正在流式传输。                    |
| **动作**   | DataTester 以 `manage_book=True` 订阅 deltas；从 delta 流构建本地订单簿。 |
| **事件序列** | `OrderBookDeltas` 被应用到本地 `OrderBook`；按配置的深度记录订单簿日志。         |
| **通过标准** | 本地订单簿能从 deltas 正确构建；买档递减、卖档递增；首个快照之后订单簿非空。                  |
| **何时跳过** | 适配器不支持订单簿数据。                                                |

**注意事项：**

- 受管订单簿会把每个 delta 应用到由 actor 维护的 `OrderBook` 实例上。
- 使用 `book_levels_to_print` 控制日志的详细程度。

**Python 配置：**

```python
DataTesterConfig(
    instrument_ids=[instrument_id],
    subscribe_book_deltas=True,
    manage_book=True,
    book_type=BookType.L2_MBP,
    book_levels_to_print=10,
)
```

**Rust 配置：**

```rust
DataTesterConfig::new(client_id, vec![instrument_id])
    .with_subscribe_book_deltas(true)
    .with_manage_book(true)
    .with_book_type(BookType::L2_MBP)
```

### TC-D15: 请求历史订单簿 deltas

| 字段       | 值                           |
| -------- | --------------------------- |
| **前置条件** | 适配器已连接，instrument 已加载。      |
| **动作**   | DataTester 请求历史订单簿 deltas。  |
| **事件序列** | 通过回调接收到历史 deltas。           |
| **通过标准** | 接收到的 deltas 具有有效的时间戳和订单簿动作。 |
| **何时跳过** | 适配器不支持历史订单簿 delta 请求。       |

**Python 配置：**

```python
DataTesterConfig(
    instrument_ids=[instrument_id],
    request_book_deltas=True,
)
```

**Rust 配置：** 尚不支持。历史订单簿 delta 请求在 Rust 版 `DataTester` 中仍是 TODO。

---

## 第 3 组：Quotes

测试报价 tick 的订阅与历史请求。

| TC     | 名称     | 描述                        | 何时跳过     |
| ------ | ------ | ------------------------- | -------- |
| TC-D20 | 订阅报价   | 验证启动后 `QuoteTick` 事件正常流动。 | 永不。      |
| TC-D21 | 请求历史报价 | 请求历史报价 tick。              | 不支持历史报价。 |

### TC-D20: 订阅报价

| 字段       | 值                                          |
| -------- | ------------------------------------------ |
| **前置条件** | 适配器已连接，instrument 已加载。                     |
| **动作**   | DataTester 在启动时订阅报价。                       |
| **事件序列** | 在 `on_quote_tick` 中接收到 `QuoteTick` 事件。     |
| **通过标准** | 至少接收到一个 `QuoteTick`，具有有效的买/卖价格和数量；买价 < 卖价。 |
| **何时跳过** | 永不。                                        |

**Python 配置：**

```python
DataTesterConfig(
    instrument_ids=[instrument_id],
    subscribe_quotes=True,
)
```

**Rust 配置：**

```rust
DataTesterConfig::new(client_id, vec![instrument_id])
    .with_subscribe_quotes(true)
```

### TC-D21: 请求历史报价

| 字段       | 值                                  |
| -------- | ---------------------------------- |
| **前置条件** | 适配器已连接，instrument 已加载。             |
| **动作**   | DataTester 请求历史报价 tick。            |
| **事件序列** | 通过 `on_historical_data` 回调接收到历史报价。 |
| **通过标准** | 接收到的报价具有有效的时间戳、买/卖价格和数量。           |
| **何时跳过** | 适配器不支持历史报价请求。                      |

**Python 配置：**

```python
DataTesterConfig(
    instrument_ids=[instrument_id],
    request_quotes=True,
    requests_start_delta=pd.Timedelta(hours=1),
)
```

---

## 第 4 组：Trades

测试成交 tick 的订阅与历史请求。

| TC     | 名称     | 描述                        | 何时跳过     |
| ------ | ------ | ------------------------- | -------- |
| TC-D30 | 订阅成交   | 验证启动后 `TradeTick` 事件正常流动。 | 永不。      |
| TC-D31 | 请求历史成交 | 请求历史成交 tick。              | 不支持历史成交。 |

### TC-D30: 订阅成交

| 字段       | 值                                      |
| -------- | -------------------------------------- |
| **前置条件** | 适配器已连接，instrument 已加载。                 |
| **动作**   | DataTester 在启动时订阅成交。                   |
| **事件序列** | 在 `on_trade_tick` 中接收到 `TradeTick` 事件。 |
| **通过标准** | 至少接收到一个 `TradeTick`，具有有效的价格、数量和主动方向。   |
| **何时跳过** | 永不。                                    |

**Python 配置：**

```python
DataTesterConfig(
    instrument_ids=[instrument_id],
    subscribe_trades=True,
)
```

**Rust 配置：**

```rust
DataTesterConfig::new(client_id, vec![instrument_id])
    .with_subscribe_trades(true)
```

### TC-D31: 请求历史成交

| 字段       | 值                                  |
| -------- | ---------------------------------- |
| **前置条件** | 适配器已连接，instrument 已加载。             |
| **动作**   | DataTester 请求历史成交 tick。            |
| **事件序列** | 通过 `on_historical_data` 回调接收到历史成交。 |
| **通过标准** | 接收到的成交具有有效的时间戳、价格、数量和成交 ID。        |
| **何时跳过** | 适配器不支持历史成交请求。                      |

**Python 配置：**

```python
DataTesterConfig(
    instrument_ids=[instrument_id],
    request_trades=True,
    requests_start_delta=pd.Timedelta(hours=1),
)
```

**Rust 配置：**

```rust
DataTesterConfig::new(client_id, vec![instrument_id])
    .with_request_trades(true)
```

---

## 第 5 组：Bars

测试 K 线的订阅与历史请求。

| TC     | 名称       | 描述                  | 何时跳过       |
| ------ | -------- | ------------------- | ---------- |
| TC-D40 | 订阅 K 线   | 验证启动后 `Bar` 事件正常流动。 | 不支持 K 线。   |
| TC-D41 | 请求历史 K 线 | 请求历史 OHLCV K 线。     | 不支持历史 K 线。 |

### TC-D40: 订阅 K 线

| 字段       | 值                                                                   |
| -------- | ------------------------------------------------------------------- |
| **前置条件** | 适配器已连接，instrument 已加载，K 线类型已配置。                                     |
| **动作**   | DataTester 订阅已配置 `BarType` 的 K 线。                                   |
| **事件序列** | 在 `on_bar` 中接收到 `Bar` 事件。                                           |
| **通过标准** | 至少接收到一个 `Bar`，具有有效的 OHLCV 值；high >= low、high >= open、high >= close。 |
| **何时跳过** | 适配器不支持 K 线订阅。                                                       |

**Python 配置：**

```python
DataTesterConfig(
    instrument_ids=[instrument_id],
    bar_types=[BarType.from_str("BTCUSDT-PERP.VENUE-1-MINUTE-LAST-EXTERNAL")],
    subscribe_bars=True,
)
```

**Rust 配置：**

```rust
DataTesterConfig::new(client_id, vec![instrument_id])
    .with_bar_types(vec![bar_type])
    .with_subscribe_bars(true)
```

### TC-D41: 请求历史 K 线

| 字段       | 值                                   |
| -------- | ----------------------------------- |
| **前置条件** | 适配器已连接，instrument 已加载，K 线类型已配置。     |
| **动作**   | DataTester 请求已配置 `BarType` 的历史 K 线。 |
| **事件序列** | 通过回调接收到历史 K 线。                      |
| **通过标准** | 接收到的 K 线具有有效的 OHLCV 值和递增的时间戳。       |
| **何时跳过** | 适配器不支持历史 K 线请求。                     |

**Python 配置：**

```python
DataTesterConfig(
    instrument_ids=[instrument_id],
    bar_types=[BarType.from_str("BTCUSDT-PERP.VENUE-1-MINUTE-LAST-EXTERNAL")],
    request_bars=True,
    requests_start_delta=pd.Timedelta(hours=1),
)
```

**Rust 配置：**

```rust
DataTesterConfig::new(client_id, vec![instrument_id])
    .with_bar_types(vec![bar_type])
    .with_request_bars(true)
```

---

## 第 6 组：衍生品数据

测试衍生品特有的数据流：标记价格 (mark prices)、指数价格 (index prices) 和资金费率 (funding rates)。

| TC     | 名称       | 描述                      | 何时跳过   |
| ------ | -------- | ----------------------- | ------ |
| TC-D50 | 订阅标记价格   | `MarkPriceUpdate` 事件。   | 非衍生品。  |
| TC-D51 | 订阅指数价格   | `IndexPriceUpdate` 事件。  | 非衍生品。  |
| TC-D52 | 订阅资金费率   | `FundingRateUpdate` 事件。 | 非永续合约。 |
| TC-D53 | 请求历史资金费率 | 历史资金费率数据。               | 非永续合约。 |

### TC-D50: 订阅标记价格

| 字段       | 值                                                    |
| -------- | ---------------------------------------------------- |
| **前置条件** | 适配器已连接，衍生品 instrument 已加载。                           |
| **动作**   | DataTester 订阅标记价格更新。                                 |
| **事件序列** | 在 `on_mark_price` 中接收到 `MarkPriceUpdate` 事件。         |
| **通过标准** | 至少接收到一个 `MarkPriceUpdate`，具有有效的 instrument ID 和标记价格。 |
| **何时跳过** | instrument 不是衍生品，或适配器不提供标记价格。                        |

**Python 配置：**

```python
DataTesterConfig(
    instrument_ids=[instrument_id],
    subscribe_mark_prices=True,
)
```

**Rust 配置：**

```rust
DataTesterConfig::new(client_id, vec![instrument_id])
    .with_subscribe_mark_prices(true)
```

### TC-D51: 订阅指数价格

| 字段       | 值                                                     |
| -------- | ----------------------------------------------------- |
| **前置条件** | 适配器已连接，衍生品 instrument 已加载。                            |
| **动作**   | DataTester 订阅指数价格更新。                                  |
| **事件序列** | 在 `on_index_price` 中接收到 `IndexPriceUpdate` 事件。        |
| **通过标准** | 至少接收到一个 `IndexPriceUpdate`，具有有效的 instrument ID 和指数价格。 |
| **何时跳过** | instrument 不是衍生品，或适配器不提供指数价格。                         |

**Python 配置：**

```python
DataTesterConfig(
    instrument_ids=[instrument_id],
    subscribe_index_prices=True,
)
```

**Rust 配置：**

```rust
DataTesterConfig::new(client_id, vec![instrument_id])
    .with_subscribe_index_prices(true)
```

### TC-D52: 订阅资金费率

| 字段       | 值                                                    |
| -------- | ---------------------------------------------------- |
| **前置条件** | 适配器已连接，永续合约 instrument 已加载。                          |
| **动作**   | DataTester 订阅资金费率更新。                                 |
| **事件序列** | 在 `on_funding_rate` 中接收到 `FundingRateUpdate` 事件。     |
| **通过标准** | 至少接收到一个 `FundingRateUpdate`，具有有效的 instrument ID 和费率。 |
| **何时跳过** | instrument 不是永续合约，或适配器不提供资金费率。                       |

**Python 配置：**

```python
DataTesterConfig(
    instrument_ids=[instrument_id],
    subscribe_funding_rates=True,
)
```

**Rust 配置：**

```rust
DataTesterConfig::new(client_id, vec![instrument_id])
    .with_subscribe_funding_rates(true)
```

### TC-D53: 请求历史资金费率

| 字段       | 值                                  |
| -------- | ---------------------------------- |
| **前置条件** | 适配器已连接，永续合约 instrument 已加载。        |
| **动作**   | DataTester 请求历史资金费率（默认回溯 7 天）。     |
| **事件序列** | 通过回调接收到历史资金费率。                     |
| **通过标准** | 接收到的资金费率具有有效的时间戳和费率值。              |
| **何时跳过** | instrument 不是永续合约，或适配器不支持历史资金费率请求。 |

**Python 配置：**

```python
DataTesterConfig(
    instrument_ids=[instrument_id],
    request_funding_rates=True,
)
```

**Rust 配置：**

```rust
DataTesterConfig::new(client_id, vec![instrument_id])
    .with_request_funding_rates(true)
```

---

## 第 7 组：Instrument status

测试 instrument 状态与收盘事件的订阅。

| TC     | 名称               | 描述                     | 何时跳过   |
| ------ | ---------------- | ---------------------- | ------ |
| TC-D60 | 订阅 instrument 状态 | `InstrumentStatus` 事件。 | 不支持状态。 |
| TC-D61 | 订阅 instrument 收盘 | `InstrumentClose` 事件。  | 不支持收盘。 |

### TC-D60: 订阅 instrument 状态

| 字段       | 值                                                    |
| -------- | ---------------------------------------------------- |
| **前置条件** | 适配器已连接，instrument 已加载。                               |
| **动作**   | DataTester 订阅 instrument 状态更新。                       |
| **事件序列** | 在 `on_instrument_status` 中接收到 `InstrumentStatus` 事件。 |
| **通过标准** | 接收到的状态事件具有有效的 `MarketStatusAction`（例如 `Trading`）。    |
| **何时跳过** | 适配器不支持 instrument 状态订阅。                              |

**注意事项：**

- 状态事件可能仅在状态发生变化时才触发（例如交易暂停 -> 恢复）。
- 在正常交易时段内，订阅时可能会收到一个 `Trading` 状态。

**Python 配置：**

```python
DataTesterConfig(
    instrument_ids=[instrument_id],
    subscribe_instrument_status=True,
)
```

**Rust 配置：**

```rust
DataTesterConfig::new(client_id, vec![instrument_id])
    .with_subscribe_instrument_status(true)
```

### TC-D61: 订阅 instrument 收盘

| 字段       | 值                                                  |
| -------- | -------------------------------------------------- |
| **前置条件** | 适配器已连接，instrument 已加载。                             |
| **动作**   | DataTester 订阅 instrument 收盘事件。                     |
| **事件序列** | 在 `on_instrument_close` 中接收到 `InstrumentClose` 事件。 |
| **通过标准** | 接收到的收盘事件具有有效的收盘价格和收盘类型。                            |
| **何时跳过** | 适配器不支持 instrument 收盘订阅。                            |

**注意事项：**

- 对于传统市场，收盘事件通常在交易时段结束时触发。
- 对于 7×24 小时的加密交易场所，除非适配器合成一个每日收盘，否则可能不会触发。

**Python 配置：**

```python
DataTesterConfig(
    instrument_ids=[instrument_id],
    subscribe_instrument_close=True,
)
```

**Rust 配置：**

```rust
DataTesterConfig::new(client_id, vec![instrument_id])
    .with_subscribe_instrument_close(true)
```

---

## 第 8 组：期权希腊字母

测试期权希腊字母 (option greeks) 与期权链 (option chain) 的订阅。

| TC     | 名称       | 描述                                 | 何时跳过     |
| ------ | -------- | ---------------------------------- | -------- |
| TC-D62 | 订阅期权希腊字母 | 单个 instrument 的 `OptionGreeks` 数据。 | 不支持希腊字母。 |
| TC-D63 | 订阅期权链    | 某个系列的 `OptionChainSlice` 快照。       | 不支持期权链。  |

### TC-D62: 订阅期权希腊字母

| 字段       | 值                                            |
| -------- | -------------------------------------------- |
| **前置条件** | 适配器已连接，期权 instrument 已加载。                    |
| **动作**   | DataTester 订阅期权希腊字母更新。                       |
| **事件序列** | 在 `on_option_greeks` 中接收到 `OptionGreeks` 事件。 |
| **通过标准** | 接收到的希腊字母具有有效的 delta、gamma、vega、theta 值。      |
| **何时跳过** | 适配器不支持期权希腊字母订阅。                              |

**注意事项：**

- 希腊字母仅对期权 instrument 可用。
- 取值取决于交易场所的定价模型，并且可能在每次报价变动时更新。
- 某些交易场所（Bybit、Deribit）按 instrument 订阅；OKX 按 instrument 族订阅
  并过滤到所请求的那些 instruments。
- 当交易场所不提供 `rho` 时，它可能为零（Bybit、OKX）。
- `underlying_price` 和 `open_interest` 可能为 `None`，具体取决于交易场所的频道。

**Python 配置：**

```python
DataTesterConfig(
    instrument_ids=[instrument_id],
    subscribe_option_greeks=True,
)
```

**Rust 配置：**

```rust
DataTesterConfig::new(client_id, vec![instrument_id])
    .with_subscribe_option_greeks(true)
```

### TC-D63: 订阅期权链

| 字段       | 值                                               |
| -------- | ----------------------------------------------- |
| **前置条件** | 适配器已连接，该系列的期权 instruments 已加载。                  |
| **动作**   | DataTester 订阅某个系列的期权链快照。                        |
| **事件序列** | 在 `on_option_chain` 中接收到 `OptionChainSlice` 快照。 |
| **通过标准** | 快照包含与该系列匹配的 instruments 的希腊字母。                  |
| **何时跳过** | 适配器不支持期权链订阅。                                    |

**注意事项：**

- 期权链订阅由 DataEngine 管理，它会在内部为每个 instrument 创建报价和希腊字母订阅。
- ATM 相对行权价区间需要在订阅开始前先进行一次远期价格 bootstrap。
- 尚不能通过 `DataTesterConfig` 配置；需要使用 `subscribe_option_chain` 和一个
  `OptionSeriesId` 手动设置 actor。

---

## 第 9 组：Lifecycle

测试 actor 的生命周期行为：取消订阅的处理与自定义参数。

| TC     | 名称      | 描述                 | 何时跳过     |
| ------ | ------- | ------------------ | -------- |
| TC-D70 | 停止时取消订阅 | actor 停止时从数据源取消订阅。 | 不支持取消订阅。 |
| TC-D71 | 自定义订阅参数 | 适配器特定的订阅参数。        | 不适用。     |
| TC-D72 | 自定义请求参数 | 适配器特定的请求参数。        | 不适用。     |

### TC-D70: 停止时取消订阅

| 字段       | 值                                     |
| -------- | ------------------------------------- |
| **前置条件** | 存在活动的数据订阅（报价、成交、订单簿）。                 |
| **动作**   | 以 `can_unsubscribe=True`（默认）停止 actor。 |
| **事件序列** | 数据订阅被移除；不再接收到后续数据事件。                  |
| **通过标准** | 干净地取消订阅；日志中无错误；停止后无数据事件。              |
| **何时跳过** | 适配器不支持取消订阅。                           |

**Python 配置：**

```python
DataTesterConfig(
    instrument_ids=[instrument_id],
    subscribe_quotes=True,
    subscribe_trades=True,
    can_unsubscribe=True,
)
```

**Rust 配置：**

```rust
DataTesterConfig::new(client_id, vec![instrument_id])
    .with_subscribe_quotes(true)
    .with_subscribe_trades(true)
    .with_can_unsubscribe(true)
```

### TC-D71: 自定义订阅参数

| 字段       | 值                                       |
| -------- | --------------------------------------- |
| **前置条件** | 适配器已连接，且适配器接受额外的订阅参数。                   |
| **动作**   | 使用包含适配器特定参数的 `subscribe_params` 字典进行订阅。 |
| **事件序列** | 订阅建立，且自定义参数已生效。                         |
| **通过标准** | 数据流动，且适配器特定参数处于生效状态。                    |
| **何时跳过** | 不适用（适配器特定）。                             |

**注意事项：**

- `subscribe_params` 字典对 DataTester 是不透明的，会被透传给适配器。
- 请查阅适配器的指南了解所支持的参数。

### TC-D72: 自定义请求参数

| 字段       | 值                                     |
| -------- | ------------------------------------- |
| **前置条件** | 适配器已连接，且适配器接受额外的请求参数。                 |
| **动作**   | 使用包含适配器特定参数的 `request_params` 字典请求数据。 |
| **事件序列** | 请求被满足，且自定义参数已生效。                      |
| **通过标准** | 接收到历史数据，且适配器特定参数处于生效状态。               |
| **何时跳过** | 不适用（适配器特定）。                           |

**注意事项：**

- `request_params` 字典对 DataTester 是不透明的，会被透传给适配器。
- 请查阅适配器的指南了解所支持的参数。

---

## DataTester 配置参考

所有 `DataTesterConfig` 参数的快速参考。所示默认值针对的是 Python 配置。
注意：Rust 的 `DataTesterConfig::new` 将 `manage_book` 设为 `true`，而 Python 默认将其设为 `False`。

| 参数                            | 类型                 | 默认值    | 影响的组    |
| ----------------------------- | ------------------ | ------ | ------- |
| `instrument_ids`              | list[InstrumentId] | *必填*   | 全部      |
| `client_id`                   | ClientId?          | None   | 全部      |
| `bar_types`                   | list[BarType]?     | None   | 5       |
| `subscribe_book_deltas`       | bool               | False  | 2       |
| `subscribe_book_depth`        | bool               | False  | 2       |
| `subscribe_book_at_interval`  | bool               | False  | 2       |
| `subscribe_quotes`            | bool               | False  | 3       |
| `subscribe_trades`            | bool               | False  | 4       |
| `subscribe_mark_prices`       | bool               | False  | 6       |
| `subscribe_index_prices`      | bool               | False  | 6       |
| `subscribe_funding_rates`     | bool               | False  | 6       |
| `subscribe_bars`              | bool               | False  | 5       |
| `subscribe_instrument`        | bool               | False  | 1       |
| `subscribe_instrument_status` | bool               | False  | 7       |
| `subscribe_instrument_close`  | bool               | False  | 7       |
| `subscribe_option_greeks`     | bool               | False  | 8       |
| `subscribe_params`            | dict?              | None   | 9       |
| `can_unsubscribe`             | bool               | True   | 9       |
| `request_instruments`         | bool               | False  | 1       |
| `request_book_snapshot`       | bool               | False  | 2       |
| `request_book_deltas`         | bool               | False  | 2       |
| `request_quotes`              | bool               | False  | 3       |
| `request_trades`              | bool               | False  | 4       |
| `request_bars`                | bool               | False  | 5       |
| `request_funding_rates`       | bool               | False  | 6       |
| `request_params`              | dict?              | None   | 9       |
| `requests_start_delta`        | Timedelta?         | 1 小时   | 3, 4, 5 |
| `book_type`                   | BookType           | L2_MBP | 2       |
| `book_depth`                  | PositiveInt?       | None   | 2       |
| `book_interval_ms`            | PositiveInt        | 1000   | 2       |
| `book_levels_to_print`        | PositiveInt        | 10     | 2       |
| `manage_book`                 | bool               | False  | 2       |
| `use_pyo3_book`               | bool               | False  | 2       |
| `log_data`                    | bool               | True   | 全部      |

---
