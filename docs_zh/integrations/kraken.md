# Kraken

Kraken 提供广泛数字资产的现货和衍生品交易。本集成（Integration）连接到 Kraken Pro，支持 Kraken 现货（Spot）和 Kraken 衍生品（期货）市场的实时市场数据接入和订单执行。

## 概述 (Overview)

该适配器（Adapter）使用 Rust 实现，提供 Python 绑定以便在基于 Python 的工作流中使用。它不需要外部 Kraken 客户端库——核心组件编译为静态库并在构建过程中自动链接。

本指南假设交易者正在同时设置实时市场数据推送和交易执行。Kraken 适配器包含多个组件，可以根据使用场景组合或单独使用。

- `KrakenSpotRawHttpClient` 和 `KrakenFuturesRawHttpClient`：底层 HTTP API 连接。
- `KrakenSpotHttpClient` 和 `KrakenFuturesHttpClient`：带有金融工具（Instrument）缓存和对账支持的高层 HTTP 客户端。
- `KrakenInstrumentProvider`：金融工具解析和加载功能。
- `KrakenDataClient`：市场数据推送管理器。
- `KrakenExecutionClient`：账户管理和交易执行网关。
- `KrakenLiveDataClientFactory`：Kraken 数据客户端工厂（由交易节点构建器使用）。
- `KrakenLiveExecClientFactory`：Kraken 执行客户端工厂（由交易节点构建器使用）。

:::note
大多数用户只需为实时交易节点定义配置（如下所示），无需直接使用这些底层组件。
:::

## 示例 (Examples)

您可以在 [examples/live/kraken] 目录中找到实时示例脚本。

[examples/live/kraken]: https://github.com/nautechsystems/nautilus_trader/tree/develop/examples/live/kraken/

## Kraken 文档 (Kraken documentation)

Kraken 为用户提供了详尽的文档：

- [Kraken API 文档](https://docs.kraken.com/api/)
- [Kraken 现货 REST API](https://docs.kraken.com/api/docs/guides/spot-rest-intro)
- [Kraken 期货 REST API](https://docs.kraken.com/api/docs/futures-api)

请结合本 NautilusTrader 集成指南参考 Kraken 文档。

## 产品 (Products)

Kraken 支持两个主要产品类别：

| 产品类型      | 支持  | 备注                             |
| --------- | --- | ------------------------------ |
| 现货        | ✓   | 支持保证金（Margin）的标准加密货币交易对。       |
| 期货（永续）    | ✓   | 反向（`PI_`）和 USD 保证金（`PF_`）永续合约。 |
| 期货（交割/弹性） | ✓   | 固定到期（`FI_`）和弹性（`FF_`）合约。       |

:::note
**双产品部署**：当同时配置 `SPOT` 和 `FUTURES` 产品类型时，适配器会查询两个 API 并合并账户状态。这使执行引擎能够查看跨两个市场的抵押品。
:::

## K 线数据流 (Bar streaming)

### 支持的时间间隔

Kraken 适配器通过 WebSocket 支持现货市场的实时 K 线（OHLC）数据流。以下时间间隔可用：

| 时间间隔  | BarType 规格       |
| ----- | ---------------- |
| 1 分钟  | `1-MINUTE-LAST`  |
| 5 分钟  | `5-MINUTE-LAST`  |
| 15 分钟 | `15-MINUTE-LAST` |
| 30 分钟 | `30-MINUTE-LAST` |
| 1 小时  | `1-HOUR-LAST`    |
| 4 小时  | `4-HOUR-LAST`    |
| 1 天   | `1-DAY-LAST`     |
| 1 周   | `1-WEEK-LAST`    |
| 15 天  | `15-DAY-LAST`    |

:::note
**期货限制**：Kraken 期货不支持通过 WebSocket 进行 K 线数据流推送。请改用 `request_bars()` 获取历史 K 线数据。
:::

### K 线发射延迟

Kraken 的 WebSocket OHLC 频道在每笔交易时推送*当前*（未完成的）K 线更新。与某些交易所（如 Binance）不同，Kraken 不提供 "is_closed" 指标来表示 K 线已完成。

为避免发射部分/未完成的 K 线，适配器缓冲当前 K 线，仅在下一个 K 线周期开始时（即收到具有新 `interval_begin` 时间戳的消息时）发射它。这意味着：

- K 线的发射延迟最多为一个 K 线周期。
- 对于 1 分钟 K 线，最大延迟约为 1 分钟。
- 发射的 K 线数据是完整且最终的。

我们选择这种方式而非基于定时器的发射，因为：

- 基于定时器的发射可能错过 K 线关闭前的最后一次更新。
- Kraken 的更新不保证在精确的间隔边界到达。
- 缓冲以延迟为代价确保数据完整性。

:::warning
如果 K 线延迟对您的策略至关重要，请考虑使用逐笔成交数据并使用 `BarAggregator` 在本地聚合 K 线。
:::

:::tip
对于大多数使用场景，我们建议使用 `INTERNAL` K 线聚合（订阅逐笔成交并在本地聚合 K 线）而非 `EXTERNAL` 交易所提供的 K 线：

- K 线完成后立即发射，无缓冲延迟。
- 跨所有交易所行为一致，简化多交易场所策略。

:::

## 符号体系 (Symbology)

### 比特币符号格式（BTC vs XBT）

Kraken 在其各个 API 中使用不同的比特币符号约定：

| 市场  | 符号格式  | 示例                 | 备注                     |
| --- | ----- | ------------------ | ---------------------- |
| 现货  | `BTC` | `BTC/USD.KRAKEN`   | 适配器在加载时将 XBT 规范化为 BTC。 |
| 期货  | `XBT` | `PI_XBTUSD.KRAKEN` | 使用 Kraken 的原生 XBT 格式。  |

:::note
Kraken 的 REST API 为比特币返回 `XBT`（遵循 ISO 4217 关于超国家货币的约定），但其 WebSocket v2 API 要求使用 `BTC` 格式。适配器在加载金融工具时会自动将现货符号规范化为 `BTC`，无论 XBT 出现在基础货币（例如 `XBT/USD` 转为 `BTC/USD`）还是报价货币（例如 `ETH/XBT` 转为 `ETH/BTC`）。期货则保留 Kraken 的原生 `XBT` 格式。
:::

### 现货市场

NautilusTrader 对 Kraken 现货金融工具符号使用 ISO 4217-A3 格式，提供跨交易所的标准化表示。适配器在内部处理到 Kraken 原生格式的转换。

**金融工具 ID 格式：**

```python
InstrumentId.from_str("BTC/USD.KRAKEN")  # 现货 BTC/USD
InstrumentId.from_str("ETH/USD.KRAKEN")  # 现货 ETH/USD
InstrumentId.from_str("SOL/USD.KRAKEN")  # 现货 SOL/USD
InstrumentId.from_str("BTC/USDT.KRAKEN")  # 现货 BTC/USDT
InstrumentId.from_str("ETH/BTC.KRAKEN")  # 现货 ETH/BTC（从 ETH/XBT 规范化而来）
```

### 期货市场

Kraken 期货金融工具使用带有前缀的特定命名约定：

- `PI_` - 永续反向合约（例如 `PI_XBTUSD`）
- `PF_` - 永续固定保证金合约（例如 `PF_XBTUSD`）
- `FI_` - 固定到期反向合约（例如 `FI_XBTUSD_230929`）
- `FF_` - 弹性期货合约

**金融工具 ID 格式：**

```python
InstrumentId.from_str("PI_XBTUSD.KRAKEN")  # 永续反向 BTC
InstrumentId.from_str("PI_ETHUSD.KRAKEN")  # 永续反向 ETH
InstrumentId.from_str("PF_XBTUSD.KRAKEN")  # 永续固定保证金 BTC
```

## 数据功能 (Data capability)

### 订阅（实时）

| 数据类型                | 现货  | 期货  | 备注                               |
| ------------------- | --- | --- | -------------------------------- |
| `QuoteTick`         | ✓   | ✓   | 派生自 ticker 频道。                   |
| `TradeTick`         | ✓   | ✓   |                                  |
| `OrderBookDeltas`   | ✓   | ✓   | 现货 L2/L3 和期货 L2 更新。              |
| `OrderBookDepth10`  | -   | -   | 使用 `OrderBookDeltas` 并设置深度 `10`。 |
| `Bar`               | ✓   | -   | 现货 WS OHLC 频道。见 K 线小节。           |
| `MarkPriceUpdate`   | -   | ✓   | 来自期货 ticker 推送。                  |
| `IndexPriceUpdate`  | -   | ✓   | 来自期货 ticker 推送。                  |
| `FundingRateUpdate` | -   | ✓   | 仅永续合约。                           |
| `InstrumentStatus`  | ✓   | ✓   | Python 适配器轮询金融工具刷新。              |

### 请求（历史）

| 数据类型                | 现货  | 期货  | 备注                       |
| ------------------- | --- | --- | ------------------------ |
| `TradeTick`         | ✓   | ✓   |                          |
| `Bar`               | ✓   | ✓   |                          |
| `OrderBook`（快照）     | ✓   | ✓   | 通过 HTTP depth 端点。        |
| `FundingRateUpdate` | -   | ✓   | 客户端侧 start/end/limit 过滤。 |

## L3 订单簿（market-by-order）

Kraken 通过 `wss://ws-l3.kraken.com/v2` 上的 WebSocket v2 `level3` 频道公开现货逐订单订单簿数据。这提供了交易场所订单 ID、逐订单数量以及真正的增量事件（`add`、`modify`、`delete`）。适配器将每个交易场所订单 ID 哈希成 NautilusTrader 使用的 `u64` `BookOrder.order_id` 字段。

### 前置条件

L3 订阅需要现货 API 凭证，因为 Kraken 的 `level3` 频道是经过身份验证的。在 `KrakenDataClientConfig` 中设置，或通过 `KRAKEN_SPOT_API_KEY` 和 `KRAKEN_SPOT_API_SECRET` 设置：

```python
from nautilus_trader.adapters.kraken.config import KrakenDataClientConfig

config = KrakenDataClientConfig(
    api_key="YOUR_KEY",
    api_secret="YOUR_SECRET",
)
```

然后使用 `book_type=BookType.L3_MBO` 订阅：

```python
from nautilus_trader.model.enums import BookType

await client.subscribe_book_deltas(
    instrument_id=instrument_id,
    book_type=BookType.L3_MBO,
    depth=1000,  # 有效值: 10, 100, 1000
)
```

有效深度为 `10`、`100` 和 `1000`。深度 `0` 将使用 `1000`。

### CRC32 校验和验证

默认情况下，当 Kraken 提供校验和时，适配器会在每个 L3 快照和更新上验证 CRC32 校验和。在不匹配时，它会发射一个 `Clear` delta、清空本地 L3 状态、刷新认证令牌，并重新订阅，使 Kraken 发送一份全新的快照。要禁用验证以进行基准测试：

```python
config = KrakenDataClientConfig(
    api_key="...",
    api_secret="...",
    validate_l3_checksum=False,
)
```

### 存储建议

`OrderBookDelta` 在其 Arrow schema 中已携带 `order_id: u64`，因此 L3 数据在 `ParquetDataCatalog` 中的存储方式与 L2 完全相同。L3 每个金融工具产生的事件数量明显多于 L2。推荐设置：

- 使用较小的块大小（例如 `chunk_size=50_000`）以加快并行读取。
- 在 catalog 配置中启用 `zstd` 压缩。
- 使用按金融工具的路径分区（默认启用）。

## 订单功能 (Orders capability)

### 订单类型

| 订单类型                   | 现货  | 期货  | 备注                                  |
| ---------------------- | --- | --- | ----------------------------------- |
| `MARKET`               | ✓   | ✓   | 以市场价格立即执行。                          |
| `LIMIT`                | ✓   | ✓   | 以指定价格或更优价格执行。                       |
| `STOP_MARKET`          | ✓   | ✓   | 条件市价单（止损）。                          |
| `MARKET_IF_TOUCHED`    | ✓   | ✓   | 条件市价单（止盈）。                          |
| `STOP_LIMIT`           | ✓   | ✓   | 条件限价单（止损限价）。                        |
| `LIMIT_IF_TOUCHED`     | ✓   | ✓   | 映射到带 `limit_price` 的 `take_profit`。 |
| `TRAILING_STOP_MARKET` | ✓   | -   | 带 `trailing_offset` 的追踪止损。          |
| `TRAILING_STOP_LIMIT`  | ✓   | -   | 带 `limit_offset` 的追踪止损限价。           |

### 有效时间

| 有效时间  | 现货  | 期货  | 备注                            |
| ----- | --- | --- | ----------------------------- |
| `GTC` | ✓   | ✓   | 撤销前有效（Good Till Canceled）。    |
| `GTD` | ✓   | -   | 到期前有效（仅现货，需要 `expire_time`）。  |
| `IOC` | ✓   | ✓   | 立即成交或撤销（Immediate or Cancel）。 |
| `FOK` | ✓   | -   | 仅现货限价单。                       |

:::note
**市价单**本质上是立即执行的，不支持有效时间设置。`IOC` 仅适用于限价类订单。
:::

### 执行指令

| 指令               | 现货  | 期货  | 备注                                       |
| ---------------- | --- | --- | ---------------------------------------- |
| `post_only`      | ✓   | ✓   | 适用于限价单。                                  |
| `reduce_only`    | ✓   | ✓   | 现货需要 `spot_account_type=Margin`（仅保证金订单）。 |
| `quote_quantity` | ✓   | -   | 仅现货。以报价货币计的成交量（`viqc`）。                  |
| `display_qty`    | ✓   | -   | 仅现货。冰山订单（`displayvol`）。                  |

### 触发类型

条件订单（止损、止盈、追踪止损）在现货上支持触发价格参考：

| 触发类型          | 现货  | 期货  | 备注          |
| ------------- | --- | --- | ----------- |
| `LAST_PRICE`  | ✓   | ✓   | 默认。最新成交价。   |
| `INDEX_PRICE` | ✓   | ✓   | 更广泛的市场指数价格。 |
| `MARK_PRICE`  | -   | ✓   | 仅期货。        |

:::note
适配器在提交时拒绝不支持的触发类型（例如 `BID_ASK`），而不是静默地强制转换它们。
:::

### 批量操作

| 操作   | 现货  | 期货  | 备注                       |
| ---- | --- | --- | ------------------------ |
| 批量提交 | ✓   | ✓   | 现货每批 15 个订单。期货每批 10 个。   |
| 批量修改 | -   | ✓   | 仅期货 HTTP 辅助方法。执行会逐条发送命令。 |
| 批量取消 | ✓   | ✓   | 自动分块为每批 50 个。            |

:::note
**取消所有订单**：

- 不支持按订单方向筛选；无论方向如何，所有订单都将被取消。
- 现货：取消所有交易对的所有未结订单。
- 期货：需要 `instrument_id`；仅取消该交易对的订单。

:::

### 持仓管理

| 功能    | 现货  | 期货  | 备注                                 |
| ----- | --- | --- | ---------------------------------- |
| 查询持仓  | ✓   | ✓   | 现货保证金通过 `OpenPositions`；现货现金需选择启用。 |
| 持仓模式  | -   | -   | 每个金融工具单一持仓。                        |
| 杠杆控制  | ✓   | ✓   | 现货分级；按订单 `params={"leverage": N}`。 |
| 保证金模式 | ✓   | ✓   | 现货/期货全仓保证金；现货无逐仓保证金。               |

### 订单查询

| 功能     | 现货  | 期货  | 备注                     |
| ------ | --- | --- | ---------------------- |
| 查询未结订单 | ✓   | ✓   | 列出所有活动订单。              |
| 查询历史订单 | ✓   | ✓   | 支持分页的历史订单数据。           |
| 订单状态更新 | ✓   | ✓   | 通过 WebSocket 实时更新订单状态。 |
| 交易历史   | ✓   | ✓   | 成交和填充报告。               |

### 条件订单 (Contingent orders)

| 功能     | 现货  | 期货  | 备注       |
| ------ | --- | --- | -------- |
| 订单列表   | -   | -   | *不支持*。   |
| OCO 订单 | -   | -   | *不支持*。   |
| 组合订单   | -   | -   | *不支持*。   |
| 条件订单   | ✓   | ✓   | 止损和止盈订单。 |

## 订单路由（现货）

Rust 现货执行客户端默认通过 Kraken 已认证的 WebSocket v2 trade 频道路由 `submit_order`、`modify_order`、`cancel_order` 和 `submit_order_list`，并以 REST 作为回退。Python 实时 `KrakenExecutionClient` 目前无论下方旋钮如何设置都通过 REST 路由所有订单；只有在使用 Rust 执行客户端（`KrakenSpotExecutionClient`）时——通过 Rust 工厂或直接构造 pyo3 暴露的 `nautilus_trader.core.nautilus_pyo3.kraken.KrakenExecClientConfig`——WebSocket trade 路由才会生效。

### 通过 REST 路由的订单形态

某些现货订单形态始终通过 REST 路由。它们分为两类：Kraken 的 WS v2 API 完全不支持的形态，以及 WS API 支持但本适配器尚未编码的形态。

**Kraken WS v2 限制：**

| 形态        | 原因                                         |
| --------- | ------------------------------------------ |
| 不支持的触发类型  | `triggers.reference` 仅接受 `last` 和 `index`。 |
| 混合符号的订单列表 | `batch_add` 要求单一共享符号。                      |

**本适配器尚未编码（后续工作，当前走 REST）：**

| 形态                | 备注                                                            |
| ----------------- | ------------------------------------------------------------- |
| `FOK` 有效时间        | 可编码为 `FOK` 有效时间，但构建器路由到 REST。                                 |
| 追踪止损 / 止损限价       | 可通过 `triggers.price` + `triggers.price_type` 编码，但构建器路由到 REST。 |
| 冰山（`display_qty`） | 可编码为 `order_type: "iceberg"` + `display_qty`，但构建器路由到 REST。    |
| 报价数量订单            | 买入市价的报价数量映射到 `cash_order_qty`；目前路由到 REST。                     |

逐次调用的 `params={"use_ws_trade": False}` 覆盖会强制单个命令走 REST，无论配置的默认值如何。可在 `SubmitOrder`、`ModifyOrder`、`CancelOrder` 或 `SubmitOrderList` 上设置它。

### WebSocket 请求超时

当 WebSocket 往返超过 `ws_request_timeout_secs`（默认 `5`）时，调度器将命令结果视为未知，并使订单保持在其当前的传输中（in-flight）状态：

- 提交 / batch_add：调度器可能通过同一 WebSocket 发送一个尽力而为的补偿性 `cancel_order`，以免延迟的交易场所接受导致遗留一个孤儿订单。
- 修改：订单保持在 `PENDING_UPDATE`。
- 取消：订单保持在 `PENDING_CANCEL`。

超时本身不会发射 `OrderRejected`、`OrderModifyRejected` 或 `OrderCancelRejected`。如果交易场所实际上接受了命令，WebSocket 订单更新或实时执行对账引擎（`open_check_interval_secs`）就是恢复路径。

:::tip
将 `ws_request_timeout_secs` 设置得明显高于您观察到的往返延迟（默认 `5` 大约是典型值的 25 倍），这样超时只会在真正的网络故障下触发。
:::

### Rust 侧配置旋钮

Rust 的 `KrakenExecClientConfig`（及其 pyo3 包装器）暴露：

| 选项                        | 默认值    | 描述                           |
| ------------------------- | ------ | ---------------------------- |
| `use_ws_trade`            | `True` | 当 trade 频道处于活动状态时通过 WS 路由订单。 |
| `ws_request_timeout_secs` | `5`    | 在将命令结果标记为未知之前的 WS 往返超时。      |

这些选项未在 Python 实时 `KrakenExecClientConfig` 上暴露，因为 Python 实时执行客户端尚未支持它们。

## 对账 (Reconciliation)

Kraken 适配器为现货和期货市场提供对账功能，允许交易者在启动时或运行期间将本地状态与交易所状态同步。

### 现货对账

**订单状态报告：**

- 未结订单：获取所有当前活动的订单。
- 已关闭订单：获取支持分页的历史订单。
- 时间范围查询：支持按开始/结束时间戳筛选。

**成交报告：**

- 交易历史：获取支持分页的成交历史。
- 时间范围查询：支持按开始/结束时间戳筛选。
- 所有成交类型：市价单、限价单和条件订单成交。

**保证金持仓报告**（当 `spot_account_type=Margin` 时）：

- 未结持仓：从 `POST /0/private/OpenPositions` 获取，并按（交易对，方向）聚合为 `PositionStatusReport` 条目。
- 合成 FLAT 清理：如果本地缓存中有一个未结的现货保证金持仓，但它不再出现在交易场所上（Kraken 从 `OpenPositions` 中省略已关闭的持仓），适配器会在下一个持仓检查 tick 发射一个合成的 FLAT 报告，使引擎对账为已关闭。
- 保证金余额：在账户状态刷新的同时调用 `POST /0/private/TradeBalance`；已用保证金填充 `MarginBalance.initial`，其余指标流入 `AccountState.info`（见现货保证金交易）。

### 期货对账

**订单状态报告：**

- 未结订单：获取所有当前活动的期货订单。
- 历史订单：当 `open_only=False` 时获取已关闭和已成交的订单。
- 订单事件：通过 `/api/history/v2/orders` 端点获取完整的订单生命周期历史。

**成交报告：**

- 成交历史：获取所有成交报告。
- 时间筛选：按开始/结束时间戳进行客户端筛选（解析 RFC3339 时间戳）。
- 所有成交类型：包含手续费信息的 Maker 和 Taker 成交。

**持仓状态报告：**

- 未结持仓：获取所有活动的期货持仓。
- 实时数据：包含未实现资金费用、平均价格和持仓规模。

:::note
**期货时间筛选**：Kraken 期货成交端点不支持服务端时间范围筛选。适配器通过解析 `fillTime` 字段并与请求的开始/结束时间戳进行比较来实现客户端筛选。
:::

### 现货持仓报告（现金模式）

在现金模式下，Kraken 适配器可以选择性地将钱包余额报告为现货金融工具的持仓状态报告。此功能默认禁用，必须通过配置显式启用。保证金模式账户应保持其禁用，转而依赖 `OpenPositions`（见现货保证金交易）。

**工作原理：**

- 启用后，钱包余额被转换为 `PositionStatusReport` 对象。
- 正余额报告为多头（`LONG`）持仓。
- 仅报告与配置的报价货币匹配的金融工具（默认：`USDT`）。
- 这防止了同一资产在多个报价货币下出现重复报告（例如 BTC/USD、BTC/USDT、BTC/EUR）。

**配置：**

```python
exec_clients = {
    KRAKEN: {
        "use_spot_position_reports": True,
        "spot_positions_quote_currency": "USDT",  # 默认值
    },
}
```

:::warning
**谨慎使用**：如果您的策略未设计为处理现货持仓，启用现货持仓报告可能导致意外行为。例如，预期平仓的策略可能会尝试卖出您的钱包持有量。
:::

## 现货保证金交易 (Spot margin trading)

Kraken 现货支持在部分交易对上进行杠杆交易。每个交易对的可用性和有效杠杆分级由 Kraken 在金融工具端点上以 `AssetPairInfo.leverage_buy` 和 `leverage_sell` 形式公布；适配器在金融工具加载时缓存这些信息，并在订单提交前验证请求的分级。保证金交易通过 `spot_account_type` 按执行客户端启用，并支持逐订单的 `leverage` 参数。

### 配置

```python
from nautilus_trader.adapters.kraken import KrakenExecClientConfig
from nautilus_trader.model.enums import AccountType

exec_clients = {
    KRAKEN: KrakenExecClientConfig(
        spot_account_type=AccountType.MARGIN,
        default_leverage=3,  # 可选的配置级默认值
        margin_balance_asset="ZGBP",  # 可选的摘要显示资产
    ),
}
```

`margin_balance_asset` 仅控制 Kraken `TradeBalance` 端点返回的账户摘要指标（净值、可用保证金、已用保证金等）的计价单位。来自 `OpenPositions` 的逐持仓数字始终以所交易交易对的报价货币计。

### 逐订单杠杆

通过 `params` 在单个订单上覆盖配置的默认值：

```python
order = strategy.order_factory.limit(
    instrument_id=BTC_USD,
    order_side=OrderSide.BUY,
    quantity=Quantity.from_str("0.01"),
    price=Price.from_str("50000.00"),
    params={"leverage": 5},
)
```

适配器在提交前会根据该交易对的 `AssetPairInfo.leverage_buy` / `leverage_sell` 验证请求的分级；无效的分级会产生一个 `OrderDenied` 事件，且永远不会到达交易场所。

### Reduce-only

保证金订单可以携带 `reduce_only=True`；如果不存在匹配的持仓，Kraken 会拒绝该订单。现金订单会忽略该标志。

### 账户状态

当 `spot_account_type=Margin` 时，适配器会调用 Kraken 的 `TradeBalance` 端点，并在两个位置呈现结果：

- `MarginBalance.initial`：已用保证金（`m`）。
- `AccountState.info` 字典：完整的 `TradeBalance` 快照：
  - `equity`：净值
  - `free_margin`：净值减去已用保证金
  - `unrealized_pnl`：未结持仓的盈亏
  - `margin_level`：有持仓时为净值 / 已用保证金（%）
  - `trade_balance`：存入的抵押品
  - `equivalent_balance`：合并货币的钱包等值
  - `cost_basis`、`valuation`、`unexecuted_value`、`used_margin`：原始 `TradeBalance` 字段
  - `asset`：解析出的计价资产（例如 `USD`、`GBP`）

每次账户状态刷新都会发射一行 INFO 日志：

```text
Margin metrics: equity=1234.56 GBP, free_margin=1100.00, unrealized_pnl=12.34
```

策略通过 `account_state.info["equity"]` 等读取这些值。

### 持仓对账

未结的现货保证金持仓在每个 `position_check_interval_secs` tick 通过 `POST /0/private/OpenPositions` 呈现。在交易场所上已关闭但在本地缓存中仍显示为未结的持仓，会在下一次扫描时对账为 FLAT。此路径独立于 `use_spot_position_reports`（后者是从钱包派生、仅限现金模式的）。

## 资金费率 (Funding rates)

适配器从 [Ticker](https://docs.kraken.com/api/docs/futures-api/websocket/ticker) WebSocket 推送接收资金费率数据，该推送为永续期货提供 `relative_funding_rate` 和 `next_funding_rate_time`。

对于 Kraken，`FundingRateUpdate` 上的 `interval` 字段为 `None`，因为 ticker 推送不包含资金费率间隔字段，且 Kraken API 文档未指定固定的资金费率周期。

## 速率限制 (Rate limiting)

适配器实现了自动速率限制以符合 Kraken 的 API 要求。

| 端点类型        | 限制（请求/秒） | 备注              |
| ----------- | -------- | --------------- |
| 现货 REST（全局） | 5        | 现货 API 的全局速率限制。 |
| 期货 REST（全局） | 5        | 期货 API 的全局速率限制。 |

:::info
Kraken 使用基于计数器的速率限制系统，限制因等级而异：

- **入门等级（Starter）**：最大计数器 15，衰减 -0.33/秒
- **中级等级（Intermediate）**：最大计数器 20，衰减 -0.5/秒
- **专业等级（Pro）**：最大计数器 20，衰减 -1/秒

账本/交易历史调用使计数器增加 +2；其他调用增加 +1。
:::

:::warning
Kraken 可能会临时封锁超出速率限制的 IP 地址。适配器会在接近限制时自动排队请求。
:::

### 对账间隔指南

执行引擎的 `open_check_interval_secs` 和 `position_check_interval_secs` 设置会产生持续的 REST API 负载，可能耗尽 Kraken 基于计数器的速率限制，尤其是在计数器仅以 0.33/秒衰减的入门等级上。每次未结订单检查会产生 1-3 次 REST 调用（每次 +1 或 +2 计数），在较短的间隔下，计数器在衰减之前就会溢出，导致 `EAPI:Rate limit exceeded` 错误。

Kraken 的推荐设置：

```python
exec_engine = LiveExecEngineConfig(
    reconciliation=True,
    open_check_interval_secs=30.0,  # 入门等级最低 30s
    position_check_interval_secs=120.0,  # 2 分钟
)
```

具有更快计数器衰减的更高等级账户可以使用更短的间隔。如果您在日志中看到 `EAPI:Rate limit exceeded` 错误，请增大这些间隔，或减小适配器配置中的 `max_requests_per_second`。

## 配置 (Configuration)

每个客户端的产品类型必须在配置中指定。

### 数据客户端配置选项

| 选项                                 | 默认值       | 描述                                      |
| ---------------------------------- | --------- | --------------------------------------- |
| `api_key`                          | `None`    | API 密钥；省略时从环境变量加载。                      |
| `api_secret`                       | `None`    | API 密钥（secret）；省略时从环境变量加载。              |
| `environment`                      | `LIVE`    | 交易环境（`LIVE` 或 `DEMO`）；demo 仅适用于期货。      |
| `product_types`                    | `(SPOT,)` | 产品类型元组（例如 `(KrakenProductType.SPOT,)`）。 |
| `base_url_http_spot`               | `None`    | Kraken 现货 REST 基础 URL 覆盖。               |
| `base_url_http_futures`            | `None`    | Kraken 期货 REST 基础 URL 覆盖。               |
| `base_url_ws_spot`                 | `None`    | Kraken 现货 WebSocket URL 覆盖。             |
| `base_url_ws_futures`              | `None`    | Kraken 期货 WebSocket URL 覆盖。             |
| `base_url_ws_l3_spot`              | `None`    | Kraken 现货 L3 WebSocket URL 覆盖。          |
| `proxy_url`                        | `None`    | HTTP 和 WebSocket 传输的可选代理 URL。           |
| `update_instruments_interval_mins` | `60`      | 金融工具重新加载间隔；设为 `None` 可禁用重新加载。           |
| `max_retries`                      | `None`    | REST 请求的最大重试次数。                         |
| `retry_delay_initial_ms`           | `None`    | 重试间的初始延迟（毫秒）。                           |
| `retry_delay_max_ms`               | `None`    | 重试间的最大延迟（毫秒）。                           |
| `http_timeout_secs`                | `None`    | HTTP 请求超时时间（秒）。                         |
| `ws_heartbeat_secs`                | `30`      | WebSocket 心跳间隔（秒）。                      |
| `max_requests_per_second`          | `None`    | 覆盖速率限制；默认为 5 请求/秒。                      |
| `validate_l3_checksum`             | `True`    | 验证 Kraken 现货 L3 校验和，并在不匹配时重新同步。         |
| `transport_backend`                | `Sockudo` | WebSocket 传输后端。                         |

### 执行客户端配置选项

| 选项                              | 默认值       | 描述                                      |
| ------------------------------- | --------- | --------------------------------------- |
| `api_key`                       | `None`    | API 密钥；省略时从环境变量加载。                      |
| `api_secret`                    | `None`    | API 密钥（secret）；省略时从环境变量加载。              |
| `environment`                   | `LIVE`    | 交易环境（`LIVE` 或 `DEMO`）；demo 仅适用于期货。      |
| `product_types`                 | `(SPOT,)` | 产品类型元组；现货可用现金或保证金；期货使用保证金。              |
| `base_url_http_spot`            | `None`    | Kraken 现货 REST 基础 URL 覆盖。               |
| `base_url_http_futures`         | `None`    | Kraken 期货 REST 基础 URL 覆盖。               |
| `base_url_ws_spot`              | `None`    | Kraken 现货 WebSocket URL 覆盖。             |
| `base_url_ws_futures`           | `None`    | Kraken 期货 WebSocket URL 覆盖。             |
| `proxy_url`                     | `None`    | HTTP 和 WebSocket 传输的可选代理 URL。           |
| `max_retries`                   | `None`    | 订单提交/取消调用的最大重试次数。                       |
| `retry_delay_initial_ms`        | `None`    | 重试间的初始延迟（毫秒）。                           |
| `retry_delay_max_ms`            | `None`    | 重试间的最大延迟（毫秒）。                           |
| `http_timeout_secs`             | `None`    | HTTP 请求超时时间（秒）。                         |
| `ws_heartbeat_secs`             | `30`      | WebSocket 心跳间隔（秒）。                      |
| `max_requests_per_second`       | `None`    | 覆盖速率限制；默认为 5 请求/秒。                      |
| `use_spot_position_reports`     | `False`   | 将钱包余额报告为持仓；仅现金模式。                       |
| `spot_positions_quote_currency` | `"USDT"`  | 现货钱包持仓报告的报价货币筛选。                        |
| `spot_account_type`             | `CASH`    | 现货交易的账户类型；`MARGIN` 启用杠杆和报告。             |
| `default_leverage`              | `None`    | 设置后以 `"N:1"` 形式发送的默认现货保证金杠杆。            |
| `margin_balance_asset`          | `None`    | `TradeBalance` 的摘要资产；`None` 默认为 `ZUSD`。 |
| `transport_backend`             | `Sockudo` | WebSocket 传输后端。                         |

对于现货保证金，当订单没有逐订单杠杆参数时应用 `default_leverage`。`margin_balance_asset` 仅改变 `TradeBalance` 摘要的计价单位；逐持仓数字仍以该交易对的报价货币计。

### 模拟环境设置

要使用 Kraken 期货模拟（模拟交易）进行测试：

1. 在 [https://demo-futures.kraken.com](https://demo-futures.kraken.com) 注册并生成 API 凭证。
2. 使用您的模拟凭证设置环境变量：
   - `KRAKEN_FUTURES_DEMO_API_KEY`
   - `KRAKEN_FUTURES_DEMO_API_SECRET`
3. 使用 `environment=KrakenEnvironment.DEMO` 和 `product_types=(KrakenProductType.FUTURES,)` 配置适配器。

```python
from nautilus_trader.adapters.kraken import KRAKEN
from nautilus_trader.adapters.kraken import KrakenEnvironment
from nautilus_trader.adapters.kraken import KrakenProductType

config = TradingNodeConfig(
    ...,  # 省略
    data_clients={
        KRAKEN: {
            "environment": KrakenEnvironment.DEMO,
            "product_types": (KrakenProductType.FUTURES,),
        },
    },
    exec_clients={
        KRAKEN: {
            "environment": KrakenEnvironment.DEMO,
            "product_types": (KrakenProductType.FUTURES,),
        },
    },
)
```

### 生产配置

最常见的使用场景是配置实时 `TradingNode` 以包含 Kraken 数据和执行客户端。将 `KRAKEN` 部分添加到您的客户端配置中：

```python
from nautilus_trader.adapters.kraken import KRAKEN
from nautilus_trader.adapters.kraken import KrakenEnvironment
from nautilus_trader.adapters.kraken import KrakenProductType
from nautilus_trader.live.node import TradingNode

config = TradingNodeConfig(
    ...,  # 省略
    data_clients={
        KRAKEN: {
            "environment": KrakenEnvironment.LIVE,
            "product_types": (KrakenProductType.SPOT,),
        },
    },
    exec_clients={
        KRAKEN: {
            "environment": KrakenEnvironment.LIVE,
            "product_types": (KrakenProductType.SPOT,),
        },
    },
)
```

### 双产品配置（现货 + 期货）

当同时交易现货和期货市场时，包含两种产品类型：

```python
config = TradingNodeConfig(
    ...,  # 省略
    data_clients={
        KRAKEN: {
            "environment": KrakenEnvironment.LIVE,
            "product_types": (KrakenProductType.SPOT, KrakenProductType.FUTURES),
        },
    },
    exec_clients={
        KRAKEN: {
            "environment": KrakenEnvironment.LIVE,
            "product_types": (KrakenProductType.SPOT, KrakenProductType.FUTURES),
        },
    },
)
```

然后，创建 `TradingNode` 并添加客户端工厂：

```python
from nautilus_trader.adapters.kraken import KRAKEN
from nautilus_trader.adapters.kraken import KrakenLiveDataClientFactory
from nautilus_trader.adapters.kraken import KrakenLiveExecClientFactory
from nautilus_trader.live.node import TradingNode

# 使用配置实例化实时交易节点
node = TradingNode(config=config)

# 向节点注册客户端工厂
node.add_data_client_factory(KRAKEN, KrakenLiveDataClientFactory)
node.add_exec_client_factory(KRAKEN, KrakenLiveExecClientFactory)

# 最后构建节点
node.build()
```

### API 凭证

有两种方式向 Kraken 客户端提供凭证。可以将对应的 `api_key` 和 `api_secret` 值传递给配置对象，或设置以下环境变量：

| 环境变量                             | 描述                             |
| -------------------------------- | ------------------------------ |
| `KRAKEN_SPOT_API_KEY`            | Kraken 现货实时交易的 API 密钥。         |
| `KRAKEN_SPOT_API_SECRET`         | Kraken 现货实时交易的 API 密钥（secret）。 |
| `KRAKEN_FUTURES_API_KEY`         | Kraken 期货实时 API 密钥。            |
| `KRAKEN_FUTURES_API_SECRET`      | Kraken 期货实时 API 密钥（secret）。    |
| `KRAKEN_FUTURES_DEMO_API_KEY`    | Kraken 期货（模拟）的 API 密钥。         |
| `KRAKEN_FUTURES_DEMO_API_SECRET` | Kraken 期货（模拟）的 API 密钥（secret）。 |

:::note
**模拟环境**：只有 Kraken 期货提供模拟环境（`https://demo-futures.kraken.com`）用于无真实资金的测试。Kraken 现货没有模拟或测试网环境。
:::

:::tip
我们建议使用环境变量来管理您的凭证。
:::

启动交易节点时，您将立即收到凭证是否有效以及是否具有交易权限的确认。

## 贡献 (Contributing)

:::info
如需额外功能或为 Kraken 适配器做贡献，请参阅我们的
[贡献指南](https://github.com/nautechsystems/nautilus_trader/blob/develop/CONTRIBUTING.md)。
:::
