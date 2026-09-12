# Binance

Binance 成立于 2017 年，是全球最大的加密货币交易所之一，以日交易量以及加密资产和加密衍生品的未平仓合约量衡量。

NautilusTrader 同时提供 Python 和 Rust 两种语言的 Binance 集成（integration）。Rust 适配器（adapter）支持下方列出的所有产品类型，并包含额外的特性（在文中逐项标注）。Python 适配器支持相同的产品类型。

支持的产品：

- **Binance Spot**（包括 Binance US）
- **Binance USDT 保证金期货（USDT-Margined Futures）**（永续合约和交割合约）
- **Binance 币本位期货（Coin-Margined Futures）**（永续合约和交割合约）

## 示例

- [Python 实盘示例](https://github.com/nautechsystems/nautilus_trader/tree/develop/examples/live/binance/)
- [Rust 现货示例](https://github.com/nautechsystems/nautilus_trader/tree/develop/crates/adapters/binance/examples/spot/)
- [Rust 期货示例](https://github.com/nautechsystems/nautilus_trader/tree/develop/crates/adapters/binance/examples/futures/)

## 概览

Binance 适配器包含多个组件，可组合使用或单独使用：

- `BinanceHttpClient`：底层 HTTP API 连接。
- `BinanceWebSocketClient`：底层 WebSocket API 连接。
- `BinanceInstrumentProvider`：金融工具（instrument）解析和加载。
- `BinanceSpotDataClient` / `BinanceFuturesDataClient`：市场数据源管理器。
- `BinanceSpotExecutionClient` / `BinanceFuturesExecutionClient`：账户管理和交易执行（execution）网关。
- `BinanceLiveDataClientFactory`：Binance 数据客户端工厂（由交易节点构建器使用）。
- `BinanceLiveExecClientFactory`：Binance 执行客户端工厂（由交易节点构建器使用）。

:::note
大多数用户只需配置一个实盘交易节点（如下所示），无需直接使用这些底层组件。
:::

### 产品支持

| 产品类型               | 支持  | 备注                   |
| ------------------ | --- | -------------------- |
| 现货市场（含 Binance US） | ✓   |                      |
| 保证金账户（全仓和逐仓）       | -   | *尚未实现。* 计划在 v2 版本支持。 |
| USDT 保证金期货（永续和交割）  | ✓   |                      |
| 币本位期货              | ✓   |                      |

:::note
保证金账户功能（借入、归还、逐仓保证金管理）尚未实现。
Python 适配器不会添加保证金支持。完整的保证金交易支持计划在 v2 版本提供。
:::

:::info
每个 Binance 客户端实例只处理一种产品类型。Rust 配置使用单数形式的 `product_type` 字段，实盘工厂从单个配置创建一个数据客户端或执行客户端。要在同一个节点中同时运行现货和期货，需配置具有不同 ID 的独立客户端，例如 `BINANCE_SPOT` 和 `BINANCE_FUTURES`，然后在策略订阅或提交订单时传入相匹配的 `client_id`。Python 适配器使用不同的配置字段名，但 `examples/live/binance/binance_spot_and_futures_market_maker.py` 展示了相同的多客户端 ID 路由模式。
:::

:::info 产品类型说明：保证金账户 vs. 期货合约

这三类产品在 Binance 体系中是完全独立的账户和 API，本质上有根本区别：

**保证金账户（Margin Account）= 现货市场的杠杆借贷**

你借入代币，在**现货市场**直接买卖真实资产。你实际持有代币，到期需归还借款并支付**按小时计算的利息**。

- **全仓保证金（Cross Margin）**：账户内所有资产共同作为抵押品，一个仓位爆仓会波及所有持仓。
- **逐仓保证金（Isolated Margin）**：每个交易对独立设置抵押品上限，爆仓只损失分配给该仓位的资金。

**USDT 保证金期货（USDT-M Futures）= 以 USDT 结算的衍生品合约**

你持有的是"涨跌赔付合约"，而非真实代币。保证金和盈亏均以 **USDT** 计价结算。持仓成本为**资金费率**（多空双方定期互付）而非借款利息。

**币本位期货（Coin-M Futures）= 以加密货币本身结算的衍生品合约**

与 USDT-M 期货逻辑相同，但保证金和盈亏以 **BTC / ETH 等加密货币**计价结算。常见于矿工或长期持币者，无需将资产换成 USDT 即可对冲风险。

| 维度                | 保证金账户     | USDT-M 期货 | 币本位期货       |
| ----------------- | --------- | --------- | ----------- |
| 产品性质              | 现货 + 借贷   | 衍生品合约     | 衍生品合约       |
| 是否持有真实资产          | 是         | 否         | 否           |
| 保证金 / 结算货币        | 借入的代币     | USDT      | BTC / ETH 等 |
| 持仓成本              | 借款利息（按小时） | 资金费率      | 资金费率        |
| NautilusTrader 支持 | ❌ 未实现     | ✅         | ✅           |
:::

## 数据类型

本集成包含若干自定义数据（data）类型：

- `BinanceFuturesTicker`：期货 24 小时行情数据，包含价格和统计信息。
- `BinanceBar`：附带额外成交量指标的 K 线数据，用于历史和实时场景。
- `BinanceFuturesMarkPriceUpdate`：Binance Futures 的标记价格更新。
- `BinanceFuturesLiquidation`：来自 `forceOrder` 流的期货强平事件。

完整定义请参阅 Binance [API 参考](/docs/python-api-latest/adapters/binance.html)。

## 交易代码规则

在可能的情况下，现货和期货合约均使用 Binance 原生交易代码。由于 NautilusTrader 支持多交易场所（venue）交易，因此必须区分作为现货交易对的 `BTCUSDT` 与作为永续期货合约的 `BTCUSDT`（Binance 对两者使用相同的代码）。

Nautilus 为所有永续合约代码添加 `-PERP` 后缀。例如，Binance Futures 的 `BTCUSDT` 永续合约在 Nautilus 系统内变为 `BTCUSDT-PERP`。

## 订单能力

以下表格详细说明了不同 Binance 账户类型支持的订单（order）类型、执行指令和有效期选项。

### 订单类型

| 订单类型                   | 现货  | 保证金 | USDT 期货 | 币本位期货 | 备注           |
| ---------------------- | --- | --- | ------- | ----- | ------------ |
| `MARKET`               | ✓   | -   | ✓       | ✓     | 报价数量支持：仅限现货。 |
| `LIMIT`                | ✓   | -   | ✓       | ✓     |              |
| `STOP_MARKET`          | -   | -   | ✓       | ✓     | 仅限期货。        |
| `STOP_LIMIT`           | ✓   | -   | ✓       | ✓     |              |
| `MARKET_IF_TOUCHED`    | -   | -   | ✓       | ✓     | 仅限期货。        |
| `LIMIT_IF_TOUCHED`     | ✓   | -   | ✓       | ✓     |              |
| `TRAILING_STOP_MARKET` | -   | -   | ✓       | ✓     | 仅限期货。        |

### 执行指令

| 指令            | 现货  | 保证金 | USDT 期货 | 币本位期货 | 备注            |
| ------------- | --- | --- | ------- | ----- | ------------- |
| `post_only`   | ✓   | -   | ✓       | ✓     | 请参阅下方限制条件。    |
| `reduce_only` | -   | -   | ✓       | ✓     | 仅限期货；对冲模式下禁用。 |

#### Post-only 限制

仅*限价*订单类型支持 `post_only`。

| 订单类型         | 现货  | 保证金 | USDT 期货 | 币本位期货 | 备注                                 |
| ------------ | --- | --- | ------- | ----- | ---------------------------------- |
| `LIMIT`      | ✓   | -   | ✓       | ✓     | 现货使用 `LIMIT_MAKER`，期货使用 `GTX` TIF。 |
| `STOP_LIMIT` | -   | -   | ✓       | ✓     | 仅限期货。                              |

### 有效期

| 有效期   | 现货  | 保证金 | USDT 期货 | 币本位期货 | 备注                            |
| ----- | --- | --- | ------- | ----- | ----------------------------- |
| `GTC` | ✓   | -   | ✓       | ✓     | 撤销前有效（Good Till Canceled）。    |
| `GTD` | ✓*  | -   | ✓       | ✓     | *现货会转换为 GTC 并发出警告。            |
| `FOK` | ✓   | -   | ✓       | ✓     | 全部成交或取消（Fill or Kill）。        |
| `IOC` | ✓   | -   | ✓       | ✓     | 立即成交或取消（Immediate or Cancel）。 |

### 高级订单功能

| 功能         | 现货  | 保证金 | USDT 期货 | 币本位期货 | 备注                            |
| ---------- | --- | --- | ------- | ----- | ----------------------------- |
| 订单修改       | ✓   | -   | ✓       | ✓     | 仅支持 `LIMIT` 订单的价格和数量修改。       |
| OCO 订单     | ✓   | -   | -       | -     | 现货 OCO 通过 `orderList/oco` 提交。 |
| Bracket 订单 | -   | -   | -       | -     | *计划中*。当前在提交时被拒绝。              |
| 冰山订单       | ✓   | -   | ✓       | ✓     | 将大额订单拆分为可见部分。                 |

### 批量操作

| 操作   | 现货  | 保证金 | USDT 期货 | 币本位期货 | 备注                  |
| ---- | --- | --- | ------- | ----- | ------------------- |
| 批量提交 | ✓   | -   | ✓       | ✓     | 订单逐个提交（无批量 API 调用）。 |
| 批量修改 | -   | -   | -       | -     | 尚未实现。               |
| 批量取消 | -*  | -   | ✓       | ✓     | *现货回退为逐个取消。         |

#### 取消全部订单的行为

当从策略中调用 `cancel_all_orders()` 时，适配器会同时包含处于未完成（open）和在途（inflight，即 SUBMITTED）状态的订单，以便适配器也能取消尚未被 Binance 确认的订单。

**多策略安全性**：当多个策略交易同一金融工具时，适配器会将发起请求的策略所拥有的订单与该金融工具的所有订单进行比较。如果该策略拥有全部订单，则使用单次取消全部的 API 调用。否则，会发送按策略隔离的取消请求（常规订单使用批量取消，algo 订单使用逐个取消），以避免影响其他策略。

**期货 algo 订单**：条件订单类型（`STOP_MARKET`、`STOP_LIMIT`、`TAKE_PROFIT`、`TAKE_PROFIT_MARKET`、`TRAILING_STOP_MARKET`）需要使用不同的取消端点。适配器会自动将这些订单路由到正确的端点。一旦 algo 订单触发并成为常规订单，它将使用标准取消端点。

**使用的端点**：

| 账户类型    | 常规订单                            | algo 订单（批量）                      | algo 订单（逐个）                 |
| ------- | ------------------------------- | -------------------------------- | --------------------------- |
| 现货/保证金  | `DELETE /api/v3/openOrders`     | N/A                              | N/A                         |
| USDT 期货 | `DELETE /fapi/v1/allOpenOrders` | `DELETE /fapi/v1/algoOpenOrders` | `DELETE /fapi/v1/algoOrder` |
| 币本位期货   | `DELETE /dapi/v1/allOpenOrders` | `DELETE /dapi/v1/algoOpenOrders` | `DELETE /dapi/v1/algoOrder` |

### 持仓管理

| 功能    | 现货  | 保证金 | USDT 期货 | 币本位期货 | 备注                 |
| ----- | --- | --- | ------- | ----- | ------------------ |
| 查询持仓  | -   | -   | ✓       | ✓     | 实时持仓更新。            |
| 持仓模式  | -   | -   | ✓       | ✓     | 单向 vs 对冲模式（持仓 ID）。 |
| 杠杆控制  | -   | -   | ✓       | ✓     | 按交易对动态调整杠杆。        |
| 保证金模式 | -   | -   | ✓       | ✓     | 按交易对设置全仓 vs 逐仓保证金。 |

### 风险事件

| 功能     | 现货  | 保证金 | USDT 期货 | 币本位期货 | 备注                         |
| ------ | --- | --- | ------- | ----- | -------------------------- |
| 强平处理   | -   | -   | ✓       | ✓     | 交易所强制平仓。                   |
| ADL 处理 | -   | -   | ✓       | ✓     | 自动减仓（Auto-Deleveraging）事件。 |

Binance Futures 可能在响应风险事件时触发交易所生成的订单：

- **强平（Liquidations）**：当保证金不足以维持持仓时，Binance 会以破产价格强制平仓。这些订单的客户端 ID 以 `autoclose-` 开头。
- **ADL（自动减仓）**：当保险基金耗尽时，Binance 会平掉盈利持仓以弥补亏损。这些订单使用客户端 ID 前缀 `adl_autoclose`。
- **结算（USDT-M）**：资金费率/保证金结算订单使用以 `settlement_autoclose-` 开头的客户端 ID。
- **交割（COIN-M）**：到期的交割合约自动平仓，客户端 ID 以 `delivery_autoclose-` 开头。
- **保险基金（Insurance fund）**：由保险基金接管时使用状态 `NEW_INSURANCE`（在公开更新日志中已弃用，但仍可在传输数据中观察到）。

适配器通过客户端 ID 模式检测这些特殊订单类型（在执行类型之前检查），然后：

1. 记录带有订单详情的警告日志以便监控。
2. 生成包含正确成交详情和 TAKER 流动性方向的 `FillReport`。
3. 生成用于对账（reconciliation）的 `OrderStatusReport`。

上游参考：

- [USDT-M `ORDER_TRADE_UPDATE`](https://developers.binance.com/docs/derivatives/usds-margined-futures/user-data-streams/Event-Order-Update)
- [COIN-M `ORDER_TRADE_UPDATE`](https://developers.binance.com/docs/derivatives/coin-margined-futures/user-data-streams/Event-Order-Update)

当订单尚未存在于缓存中时，执行引擎会根据运行时状态报告创建外部订单。这覆盖了首次出现的交易所生成订单（实盘强平或 ADL 事件的典型情况）。引擎会将该订单分配给任何通过 `external_order_claims` 认领了该金融工具的策略，或默认分配给 `EXTERNAL` 策略。

#### 手续费估算

当 Binance 在成交事件中省略手续费字段（`N`/`n`）时，Rust 适配器会使用计价货币以 `default_taker_fee * qty * price` 来估算手续费。这仅适用于 USD-M 线性合约。COIN-M 反向合约（inverse contracts）会回退为零手续费，因为线性公式没有考虑合约面值。请在 `BinanceExecClientConfig` 上配置 `default_taker_fee` 以匹配你的费率档位（默认值：0.0004 / 0.04%）。

#### 对冲模式持仓 ID

当启用 `use_position_ids`（默认启用）时，交易所生成的成交报告会包含一个由金融工具和持仓方向派生的 `venue_position_id`（例如 `ETHUSDT-PERP.BINANCE-LONG`）。在 `BinanceExecClientConfig` 上将 `use_position_ids` 设为 false，可对 `OmsType.HEDGING` 使用虚拟持仓。

:::note
状态报告和成交报告会作为单个 `OrderWithFills` 执行报告捆绑发出。引擎先从状态报告创建外部订单，然后应用真实成交，保留交易场所的 `trade_id` 和 `commission`。捆绑成交未覆盖的任何剩余数量，会使用状态报告 `avg_px` 推断出的成交来平仓。
:::

### 订单查询

| 功能      | 现货  | 保证金 | USDT 期货 | 币本位期货 | 备注        |
| ------- | --- | --- | ------- | ----- | --------- |
| 查询未完成订单 | ✓   | ✓   | ✓       | ✓     | 列出所有活跃订单。 |
| 查询订单历史  | ✓   | ✓   | ✓       | ✓     | 历史订单数据。   |
| 订单状态更新  | ✓   | ✓   | ✓       | ✓     | 实时订单状态变更。 |
| 成交历史    | ✓   | ✓   | ✓       | ✓     | 执行和成交报告。  |

### 关联订单（Contingent orders）

| 功能         | 现货  | 保证金 | USDT 期货 | 币本位期货 | 备注                                |
| ---------- | --- | --- | ------- | ----- | --------------------------------- |
| 订单列表       | ✓   | -   | ✓       | ✓     | 现货 OCO 列表；期货为独立批次。                |
| OCO 订单     | ✓   | -   | -       | -     | 仅限现货，通过 `orderList/oco`。          |
| Bracket 订单 | -   | -   | -       | -     | *计划中*。当前在提交时被拒绝。                  |
| 条件订单       | ✓   | ✓   | ✓       | ✓     | 停损（stop）和触价（market-if-touched）订单。 |

### 订单参数

在调用 `Strategy.submit_order`（Python）时提供 `params` 字典，或在 `SubmitOrder` 命令上设置 `Params`（Rust），即可自定义单个订单。Binance 执行客户端识别以下参数：

| 参数               | 类型     | 账户类型         | 描述                                                                                               |
| ---------------- | ------ | ------------ | ------------------------------------------------------------------------------------------------ |
| `price_match`    | `str`  | USDT/COIN 期货 | 设置 Binance 的 `priceMatch` 模式之一（参见下方价格匹配部分），将价格选择委托给交易所。不能与 `post_only` 或冰山（`display_qty`）指令组合使用。 |
| `close_position` | `bool` | USDT/COIN 期货 | 触发时平掉整个持仓（参见下方平仓部分）。仅对 `StopMarket` 和 `MarketIfTouched` 订单有效。不能与 `reduce_only` 组合使用。             |

### 价格匹配

Binance Futures 通过 `priceMatch` 参数支持 BBO（最优买卖报价）价格匹配，将价格选择委托给交易所。限价订单会动态以最优价格加入订单簿，无需指定确切的价格水平。

使用 `price_match` 时，你提交一个带有参考价格（用于本地风险检查）的限价订单，而 Binance 会根据当前市场状态和价格匹配模式确定实际的挂单价格。

#### 有效的价格匹配值

| 值             | 行为                           |
| ------------- | ---------------------------- |
| `OPPONENT`    | 加入订单簿对手方的最优价格。               |
| `OPPONENT_5`  | 加入对手方价格，但允许最多 5 个 tick 的偏移。  |
| `OPPONENT_10` | 加入对手方价格，但允许最多 10 个 tick 的偏移。 |
| `OPPONENT_20` | 加入对手方价格，但允许最多 20 个 tick 的偏移。 |
| `QUEUE`       | 加入同方向的最优价格（保持 maker）。        |
| `QUEUE_5`     | 加入同方向队列，但偏移最多 5 个 tick。      |
| `QUEUE_10`    | 加入同方向队列，但偏移最多 10 个 tick。     |
| `QUEUE_20`    | 加入同方向队列，但偏移最多 20 个 tick。     |

:::info
更多详情请参阅[官方文档](https://developers.binance.com/docs/derivatives/usds-margined-futures/trade/rest-api)。
:::

#### 事件序列

当提交带有 `price_match` 的订单时：

1. Nautilus 向 Binance 发送带有 `priceMatch` 参数的订单，但在 API 请求中省略限价。
2. Binance 接受订单并确定实际的挂单价格。
3. Nautilus 生成 `OrderAccepted` 事件。
4. 如果 Binance 接受的价格与参考价格不同，Nautilus 会生成带有实际挂单价格的 `OrderUpdated` 事件。
5. Nautilus 缓存中的订单价格现已与 Binance 接受的价格一致。

#### 示例

```python
order = strategy.order_factory.limit(
    instrument_id=InstrumentId.from_str("BTCUSDT-PERP.BINANCE"),
    order_side=OrderSide.BUY,
    quantity=Quantity.from_int(1),
    price=Price.from_str("65000"),  # 用于本地风险检查的参考价格
)

strategy.submit_order(
    order,
    params={"price_match": "QUEUE"},
)
```

:::note
如果 Binance 以不同的价格接受了订单（例如 64,995.50），你将先收到 `OrderAccepted` 事件，随后收到带有新价格的 `OrderUpdated` 事件。
:::

### 平仓（Close position）

Binance Futures 条件订单支持 `closePosition`，它会在触发时平掉整个持仓。Binance 会在触发时根据当前持仓规模在服务端解析数量。

与 `reduce_only` 不同，`closePosition` 会自适应持仓规模的变化，并且当持仓通过其他方式平掉时，Binance 会自动取消该订单。

通过 `StopMarket` 或 `MarketIfTouched` 订单的 `params` 字典传入 `close_position`。不能与 `reduce_only` 组合使用。

```rust tab="Rust"
let params = Params::from([("close_position", true.into())]);
let cmd = SubmitOrder::new(order).with_params(params);
```

```python tab="Python"
strategy.submit_order(order, params={"close_position": True})
```

:::info
当设置了 `close_position` 时，Nautilus 会在 API 请求中省略 `quantity` 和 `reduceOnly`。订单数量仅用于本地风险检查。
:::

### 追踪止损

对于 Binance 上的追踪止损市价订单：

- 使用 `activation_price`（可选）指定追踪机制的激活时机。
- 省略时，Binance 使用提交时的当前市场价格。
- 使用 `trailing_offset` 设置回调率（以基点为单位）。

:::warning
不要对追踪止损订单使用 `trigger_price`：这会导致错误失败。请改用 `activation_price`。
:::

## 关联与交易（Link & Trade）

对于通过 Binance Rust 适配器下达的每个订单，NautilusTrader 集成 ID 会被自动添加为所有系统生成的客户端订单 ID 的前缀。这通过 Binance 的 [Link and Trade](https://developers.binance.com/docs/binance_link/link-and-trade) 计划提供透明的订单归因，且无需任何用户配置。

适配器使用确定性的双向编码，将发出的 `ClientOrderId` 值压缩为紧凑格式，以适配 Binance 的 36 字符 `newClientOrderId` 限制，并在传入的订单事件到达策略之前将其解码回原始 ID。这一转换完全透明：策略在任何时候都只会看到它们原始的 `ClientOrderId` 值。

:::note
集成 ID 前缀适用于所有订单操作，包括提交、修改、取消和状态查询。在添加此支持之前下达的订单会通过透传（passthrough）解码得到妥善处理。
:::

:::info
此功能目前仅在 Rust 适配器中可用。用户可以通过在订单上传入自定义的 `client_order_id`，或移除编码调用并重新编译来选择退出。这两种方式都没有技术上的限制。
:::

### 解码客户端订单 ID

当直接查询 Binance 时（REST API、网页 UI 或你自己的 HTTP 代码），`clientOrderId` 字段包含的是编码后的形式。两个工具函数可以恢复原始的 Nautilus `ClientOrderId`：

```python
from nautilus_trader.adapters.binance import (
    decode_binance_futures_client_order_id,
    decode_binance_spot_client_order_id,
)

# 来自 Binance REST 响应或网页 UI 的编码 ID
encoded = "x-TD67BGP9-T0A4b1H2vj50H"
original = decode_binance_spot_client_order_id(encoded)
# -> "O-20260305-120000-001-001-100"

# 期货对应函数
encoded_futures = "x-aHRE4BCj-U2xK9mPqR7sT1vW3y"
original_futures = decode_binance_futures_client_order_id(encoded_futures)
```

不带 broker 前缀的字符串会原样透传，因此对任何 `clientOrderId` 值调用这些函数都是安全的。

:::note
领域级 HTTP 客户端（`BinanceSpotHttpClient`、`BinanceFuturesHttpClient`）在返回诸如 `OrderStatusReport` 等 Nautilus 类型时会自动解码。仅在适配器之外工作时才需要手动解码：直接的 REST 查询、Binance 网页 UI 或原始的交易场所模型。
:::

## 订单簿

订单簿可维护全量或部分深度。WebSocket 流的更新频率在现货和期货之间有所不同，Nautilus 会使用可用的最高频率：

- **现货 SBE 增量深度**：25ms
- **现货 JSON 增量深度**：100ms
- **期货**：0ms（无节流）

每个交易者实例的每个金融工具仅支持一个订单簿。当流订阅各不相同时，Binance 数据客户端会使用最新的订单簿数据订阅（增量或快照）。

订单簿快照重建将在以下情况触发：

- 订单簿数据的初始订阅。
- 数据 WebSocket 重新连接。

事件序列如下：

- 增量数据开始缓冲。
- 请求并等待快照。
- 快照响应解析为 `OrderBookDeltas`。
- 快照增量发送到 `DataEngine`。
- 迭代缓冲的增量，丢弃序列号不大于快照中最后一个增量的数据。
- 停止缓冲增量数据。
- 剩余增量发送到 `DataEngine`。

:::note
这一"快照加缓冲"序列适用于未指定显式深度的期货和现货 `BookDeltas` 订阅。现货的部分深度订阅会交付自包含的 top-N 快照。参见[现货市场数据模式](#现货市场数据模式)。
:::

## Binance 数据差异

`QuoteTick` 上的 `ts_event` 字段在现货和期货之间有所不同。现货不提供事件时间戳，因此适配器使用 `ts_init`（意味着 `ts_event` 和 `ts_init` 相同）。

## Binance 特定数据

随着 Binance 特定的数据流逐步可用，你可以订阅它们。

:::note
K 线、标记价格、指数价格和资金费率可以通过 Rust 适配器以常规方式订阅。下方的自定义数据订阅适用于 Python 适配器。
:::

Binance USD-M 标记价格负载可能包含一个 `ap` 移动平均字段。Rust 适配器会解析这个原始的交易场所字段，但不会将其作为领域数据或 Binance 自定义数据发出；Nautilus 的标记价格订阅会从同一数据流中发出标记价格、指数价格和资金费率更新。

### `BinanceFuturesTicker`

订阅特定期货金融工具的 24 小时行情统计：

```python
from nautilus_trader.core import nautilus_pyo3 as pyo3

client_id = pyo3.ClientId.from_str("BINANCE")

self.subscribe_data(
    data_type=pyo3.DataType(
        "BinanceFuturesTicker",
        {"instrument_id": "BTCUSDT-PERP.BINANCE"},
    ),
    client_id=client_id,
)
```

适配器会订阅该金融工具的 `@ticker` 流，并发出带有 `metadata={"instrument_id": "<instrument_id>"}` 的 `BinanceFuturesTicker` 自定义数据。行情自定义数据需要 `instrument_id`；不支持全市场行情订阅。

### `BinanceFuturesMarkPriceUpdate`

从你的 actor 或策略中订阅 `BinanceFuturesMarkPriceUpdate`（包含资金费率信息）：

```python
from nautilus_trader.adapters.binance import BinanceFuturesMarkPriceUpdate
from nautilus_trader.model import DataType
from nautilus_trader.model import ClientId

# 在你的 `on_start` 方法中
self.subscribe_data(
    data_type=DataType(
        BinanceFuturesMarkPriceUpdate, metadata={"instrument_id": self.instrument.id}
    ),
    client_id=ClientId("BINANCE"),
)
```

接收到的 `BinanceFuturesMarkPriceUpdate` 对象会传递到你的 `on_data` 方法中。请检查类型，因为此方法处理所有自定义/通用数据。

```python
from nautilus_trader.core import Data

def on_data(self, data: Data):
    # 首先检查数据类型
    if isinstance(data, BinanceFuturesMarkPriceUpdate):
        # 对数据进行处理
```

### `BinanceFuturesLiquidation`

订阅强平更新，可针对以下二者之一：

- 特定金融工具（`<symbol>@forceOrder`），或
- 所有交易代码（`!forceOrder@arr`），通过省略 `instrument_id` 实现。

```python
from nautilus_trader.core import nautilus_pyo3 as pyo3

client_id = pyo3.ClientId.from_str("BINANCE")

# 特定金融工具
self.subscribe_data(
    data_type=pyo3.DataType(
        "BinanceFuturesLiquidation",
        {"instrument_id": "BTCUSDT-PERP.BINANCE"},
    ),
    client_id=client_id,
)

# 全市场（无 instrument_id 元数据）
self.subscribe_data(
    data_type=pyo3.DataType("BinanceFuturesLiquidation"),
    client_id=client_id,
)
```

对于特定金融工具的订阅，`CustomData.data_type` 包含 `metadata={"instrument_id": "<instrument_id>"}`。对于全市场订阅，数据类型没有元数据。

当两种模式同时订阅时，全市场优先。当全市场处于活跃状态时，适配器会暂停按交易代码的强平流，并在取消订阅全市场之后恢复活跃的按交易代码流。

## 资金费率

Rust 适配器通过 `subscribe_funding_rates` 将 `FundingRateUpdate` 作为一等数据类型发出。数据来自[标记价格流（Mark Price Stream）](https://developers.binance.com/docs/derivatives/usds-margined-futures/websocket-market-streams/Mark-Price-Stream) WebSocket 端点，该端点在提供标记价格和指数价格的同时，也提供当前资金费率和下一次资金时间。三个订阅（`subscribe_mark_prices`、`subscribe_index_prices`、`subscribe_funding_rates`）共享单个 `@markPrice@1s` 流，并采用引用计数的订阅管理。

历史资金费率可通过 `request_funding_rates` 获取，它查询[获取资金费率历史（Get Funding Rate History）](https://developers.binance.com/docs/derivatives/usds-margined-futures/market-data/rest-api/Get-Funding-Rate-History) REST 端点（USD-M 为 `GET /fapi/v1/fundingRate`，COIN-M 为 `GET /dapi/v1/fundingRate`）。每一条历史记录都映射为一个 `FundingRateUpdate`，其 `ts_event` 设为资金时间。`next_funding_ns` 字段对于历史记录为 `None`，因为该端点不提供此信息。

Python 适配器通过 `BinanceFuturesMarkPriceUpdate` 自定义数据订阅暴露资金费率数据（参见下方的 [Binance 特定数据](#binance-特定数据)）。

`FundingRateUpdate` 上的 `interval` 字段对 Binance 为 `None`，因为标记价格流和资金费率历史端点都不包含资金间隔字段。Binance 通过[获取资金费率信息（Get Funding Rate Info）](https://developers.binance.com/docs/derivatives/usds-margined-futures/market-data/rest-api/Get-Funding-Rate-Info) REST 端点暴露 `fundingIntervalHours`，但适配器并不消费它。

## 金融工具状态轮询

:::info[仅限 Rust 适配器]
此功能在 Rust 数据客户端（`LiveNode`）中可用。Python 数据客户端不会轮询状态变更。
:::

适配器会定期轮询 Binance 的 `exchangeInfo`，以检测金融工具交易状态的变化。当某个交易代码在不同状态间转换时（例如从 Trading 到 Halt，或对于临近到期的期货合约从 Trading 到 Delivering），适配器会发出 `InstrumentStatus` 事件。

轮询间隔默认为 3600 秒（60 分钟），可通过数据客户端配置中的 `instrument_status_poll_secs` 进行配置。设为 `0` 可完全禁用轮询。

在初次连接时，适配器会从 exchange info 响应中填充其状态缓存而不发出事件。只有后续轮询检测到状态变更时才会发出 `InstrumentStatus` 事件。如果某个交易代码从 exchange info 中消失（例如下架或合约到期后），适配器会发出 `NotAvailableForTrading`。

### 状态映射

#### 现货

| Binance 状态       | MarketStatusAction     |
| ---------------- | ---------------------- |
| Trading          | Trading                |
| EndOfDay         | Close                  |
| Halt             | Halt                   |
| Break            | Pause                  |
| NonRepresentable | NotAvailableForTrading |

#### 期货（USD-M）

| Binance 状态     | MarketStatusAction |
| -------------- | ------------------ |
| Trading        | Trading            |
| PendingTrading | PreOpen            |
| PreTrading     | PreOpen            |
| PostTrading    | PostClose          |
| EndOfDay       | Close              |
| Halt           | Halt               |
| AuctionMatch   | Cross              |
| Break          | Pause              |

#### 期货（COIN-M）

| Binance 状态     | MarketStatusAction     |
| -------------- | ---------------------- |
| Trading        | Trading                |
| PendingTrading | PreOpen                |
| PreDelivering  | PreClose               |
| Delivering     | Close                  |
| Delivered      | Close                  |
| PreSettle      | PreClose               |
| Settling       | Close                  |
| Close          | Close                  |
| PreDelisting   | PreClose               |
| Delisting      | Suspend                |
| Down           | NotAvailableForTrading |

:::note
只有在连接时处于可交易状态的金融工具才会被跟踪。在连接时处于非交易状态的交易代码（例如连接时处于停牌状态）不会出现在金融工具缓存中，因此不会监控它们的状态转换。
:::

## 速率限制

Binance 使用基于时间间隔的速率限制系统，按固定时间窗口（每分钟，在 :00 秒重置）跟踪请求权重。每个 API 端点都有一个分配的权重成本，总权重使用量按 IP 地址跟踪。

### 全局权重限制

以下是所有端点共享的主要限制：

| 账户类型   | 权重限制  | 时间间隔 |
| ------ | ----- | ---- |
| 现货/保证金 | 6,000 | 1 分钟 |
| 期货     | 2,400 | 1 分钟 |

### 端点权重成本

某些端点每次请求的权重成本更高：

| 端点                        | 权重  | 备注              |
| ------------------------- | --- | --------------- |
| `/api/v3/order`           | 1   | 现货下单。           |
| `/api/v3/allOrders`       | 20  | 现货历史订单（开销大）。    |
| `/api/v3/klines`          | 2+  | 随 `limit` 参数缩放。 |
| `/fapi/v1/order`          | 1   | 期货下单。           |
| `/fapi/v1/allOrders`      | 20  | 期货历史订单（开销大）。    |
| `/fapi/v1/commissionRate` | 20  | 期货手续费率查询。       |
| `/fapi/v1/klines`         | 5+  | 随 `limit` 参数缩放。 |

### WebSocket API 限制

WebSocket API（用于用户数据流）与 REST API 共享相同的权重配额：

| 限制类型        | 值   | 备注                |
| ----------- | --- | ----------------- |
| 请求权重        | 共享  | 计入 REST API 权重配额。 |
| 握手          | 5   | 每次连接尝试的权重成本。      |
| Ping/pong 帧 | 5/秒 | 最大 ping/pong 速率。  |

### 适配器行为

适配器使用令牌桶速率限制器来近似 Binance 基于时间间隔的限制。这在维持正常操作吞吐量的同时，降低了违反配额的风险。

对于具有动态权重的端点（例如 `/klines` 随 `limit` 参数缩放），适配器每次调用只取一个令牌。大批量历史请求可能需要手动控制节奏。请监控 `X-MBX-USED-WEIGHT-*` 响应头以跟踪实际使用量。

:::warning
当你超过允许的权重时，Binance 会返回 HTTP 429。反复违规会触发临时 IP 封禁（对于屡次违规者，从 2 分钟逐级升级到 3 天）。
:::

:::info
要获取最新的速率限制，请查询 `/api/v3/exchangeInfo`（现货）或 `/fapi/v1/exchangeInfo`（期货），或参阅：

- [现货 API 限制](https://developers.binance.com/docs/binance-spot-api-docs/rest-api/limits)
- [期货 API 限制](https://developers.binance.com/docs/derivatives/usds-margined-futures/general-info)

:::

## 配置

:::note
下方的配置表描述的是 **Python 适配器**。Rust 适配器使用 `BinanceDataClientConfig` 和 `BinanceExecClientConfig`，字段名不同。Rust 配置选项的权威列表请参阅 Rust 源码 `crates/adapters/binance/src/config.rs`。
:::

### 数据客户端配置选项

| 选项                                 | 默认值       | 描述                                                                          |
| ---------------------------------- | --------- | --------------------------------------------------------------------------- |
| `venue`                            | `BINANCE` | 注册客户端时使用的交易场所标识符。                                                           |
| `api_key`                          | `None`    | Binance API 密钥（API key）；省略时从环境变量加载。                                         |
| `api_secret`                       | `None`    | Binance API secret；省略时从环境变量加载。                                              |
| `key_type`                         | `HMAC`    | **已弃用**：密钥类型现在会从 API secret 格式中自动检测。仅在需要强制 `RSA` 时使用。                       |
| `account_type`                     | `SPOT`    | 数据端点的账户类型（现货、保证金、USDT 期货、币本位期货）。                                            |
| `base_url_http`                    | `None`    | HTTP REST 基础 URL 覆盖。                                                        |
| `base_url_ws`                      | `None`    | WebSocket 基础 URL 覆盖。                                                        |
| `proxy_url`                        | `None`    | HTTP 和 WebSocket 传输的可选代理 URL。                                               |
| `us`                               | `False`   | 为 `True` 时将请求路由到 Binance US 端点。                                             |
| `environment`                      | `None`    | Binance 环境：`LIVE`、`TESTNET` 或 `DEMO`。为 `None` 时默认为 `LIVE`。                  |
| `update_instruments_interval_mins` | `60`      | 金融工具目录刷新间隔（分钟）。                                                             |
| `use_agg_trade_ticks`              | `False`   | 为 `True` 时订阅聚合交易 tick 而非原始交易。期货 WebSocket 订阅始终使用 `@aggTrade`，无论此标志如何。       |
| `spot_market_data_mode`            | `Sbe`     | *仅限 Rust。* 现货市场数据传输方式（`Sbe` 或 `Json`）。参见[现货市场数据模式](#现货市场数据模式)。 |
| `instrument_status_poll_secs`      | `3600`    | *仅限 Rust。* 轮询 exchange info 以检测金融工具状态变更的间隔（秒）。设为 `0` 可禁用。                   |
| `transport_backend`                | `Sockudo` | *仅限 Rust。* WebSocket 传输后端。                                                  |

### 执行客户端配置选项

| 选项                                      | 默认值       | 描述                                                                       |
| --------------------------------------- | --------- | ------------------------------------------------------------------------ |
| `venue`                                 | `BINANCE` | 注册客户端时使用的交易场所标识符。                                                        |
| `api_key`                               | `None`    | Binance API 密钥；省略时从环境变量加载。                                               |
| `api_secret`                            | `None`    | Binance API secret；省略时从环境变量加载。                                           |
| `key_type`                              | `HMAC`    | **已弃用**：密钥类型现在会从 API secret 格式中自动检测。仅在需要强制 `RSA` 时使用（仅限数据客户端，执行不支持 RSA）。 |
| `account_type`                          | `SPOT`    | 下单的账户类型（现货、保证金、USDT 期货、币本位期货）。                                           |
| `base_url_http`                         | `None`    | HTTP REST 基础 URL 覆盖。                                                     |
| `base_url_ws`                           | `None`    | WebSocket API 基础 URL 覆盖。                                                 |
| `base_url_ws_stream`                    | `None`    | WebSocket 流 URL 覆盖（期货用户数据事件交付）。                                          |
| `proxy_url`                             | `None`    | HTTP 和 WebSocket 传输的可选代理 URL。                                            |
| `us`                                    | `False`   | 为 `True` 时将请求路由到 Binance US 端点。                                          |
| `environment`                           | `None`    | Binance 环境：`LIVE`、`TESTNET` 或 `DEMO`。为 `None` 时默认为 `LIVE`。               |
| `use_gtd`                               | `True`    | 为 `False` 时将 GTD 订单重映射为 GTC 以进行本地到期管理。                                   |
| `use_reduce_only`                       | `True`    | 为 `True` 时将 `reduce_only` 指令传递给 Binance。                                 |
| `use_position_ids`                      | `True`    | 启用 Binance 对冲持仓 ID；设为 `False` 使用虚拟对冲。                                    |
| `use_trade_lite`                        | `False`   | 使用包含衍生费用的 TRADE_LITE 执行事件。                                               |
| `treat_expired_as_canceled`             | `False`   | 为 `True` 时将 `EXPIRED` 执行类型视为 `CANCELED`。                                 |
| `recv_window_ms`                        | `5,000`   | 签名 REST 请求的接收窗口（毫秒）。                                                     |
| `max_retries`                           | `None`    | 订单提交/取消/修改调用的最大重试次数。                                                     |
| `retry_delay_initial_ms`                | `None`    | 重试尝试之间的初始延迟（毫秒）。                                                         |
| `retry_delay_max_ms`                    | `None`    | 重试尝试之间的最大延迟（毫秒）。                                                         |
| `futures_leverages`                     | `None`    | 期货账户的 `BinanceSymbol` 到初始杠杆的映射。                                          |
| `futures_margin_types`                  | `None`    | `BinanceSymbol` 到期货保证金类型（逐仓/全仓）的映射。                                      |
| `use_ws_trading`                        | `True`    | 对订单操作使用 WebSocket 交易 API（现货和 USD-M 期货）。为 `False` 时使用 HTTP。               |
| `default_taker_fee`                     | `0.0004`  | 用于交易所生成成交（强平、ADL、结算）手续费估算的默认 taker 费率。                                   |
| `log_rejected_due_post_only_as_warning` | `True`    | 为 `True` 时将 post-only 拒绝记录为警告；否则记录为错误。                                   |
| `transport_backend`                     | `Sockudo` | *仅限 Rust。* WebSocket 传输后端。                                               |

最常见的用例是配置一个带有 Binance 数据和执行客户端的实盘 `TradingNode`。在你的客户端配置中添加 `BINANCE` 部分：

```python
from nautilus_trader.adapters.binance import BINANCE
from nautilus_trader.live.node import TradingNode

config = TradingNodeConfig(
    ...,  # 省略
    data_clients={
        BINANCE: {
            "api_key": "YOUR_BINANCE_API_KEY",
            "api_secret": "YOUR_BINANCE_API_SECRET",
            "account_type": "spot",  # {spot, usdt_futures, coin_futures}
            "base_url_http": None,  # 使用自定义端点覆盖
            "base_url_ws": None,  # 使用自定义端点覆盖
            "us": False,  # 客户端是否用于 Binance US
        },
    },
    exec_clients={
        BINANCE: {
            "api_key": "YOUR_BINANCE_API_KEY",
            "api_secret": "YOUR_BINANCE_API_SECRET",
            "account_type": "spot",  # {spot, usdt_futures, coin_futures}
            "base_url_http": None,  # 使用自定义端点覆盖
            "base_url_ws": None,  # 使用自定义端点覆盖
            "us": False,  # 客户端是否用于 Binance US
        },
    },
)
```

然后，创建 `TradingNode` 并添加客户端工厂：

```python
from nautilus_trader.adapters.binance import BINANCE
from nautilus_trader.adapters.binance import BinanceLiveDataClientFactory
from nautilus_trader.adapters.binance import BinanceLiveExecClientFactory
from nautilus_trader.live.node import TradingNode

# 使用配置实例化实盘交易节点
node = TradingNode(config=config)

# 向节点注册客户端工厂
node.add_data_client_factory(BINANCE, BinanceLiveDataClientFactory)
node.add_exec_client_factory(BINANCE, BinanceLiveExecClientFactory)

# 最后构建节点
node.build()
```

### 现货市场数据模式

`spot_market_data_mode`（Rust 的 `BinanceDataClientConfig`）用于选择现货数据传输方式。它仅影响现货；期货不受影响。

| 模式     | 凭证          | 报价           |
| ------ | ----------- | ------------ |
| `Sbe`  | Ed25519（必需） | `bestBidAsk` |
| `Json` | 无（公开）       | `bookTicker` |

`Sbe`（默认）使用 Binance 的 Simple Binary Encoding 流，并需要 Ed25519 密钥（参见[密钥类型](#密钥类型)）；如果没有这些密钥，客户端将拒绝连接。`Json` 使用公开流，无需凭证。完整的现货 `BookDeltas` 订阅在 `Sbe` 模式下使用 25ms 的 SBE 增量深度流，或在 `Json` 模式下使用 100ms 的公开 JSON 增量深度流，并配合 REST 快照同步。显式深度订阅使用部分订单簿快照（参见[订单簿](#订单簿)）。

:::note
在 `nautilus_trader.core.nautilus_pyo3.binance` 上以 `BinanceSpotMarketDataMode` 形式暴露给 Python；不在旧版 Python 适配器配置中。
:::

### 密钥类型

Binance 支持三种 API 密钥类型：**Ed25519**、**HMAC-SHA256** 和 **RSA**。适配器会从你的 API secret 格式中自动检测密钥类型，因此无需配置。

**强烈推荐使用 Ed25519。** Binance 推荐 Ed25519，因为它具有更优的性能和安全性。NautilusTrader 的未来版本将仅支持 Ed25519。

| 密钥类型    | 数据客户端 | 执行客户端 | 状态             |
| ------- | ----- | ----- | -------------- |
| Ed25519 | ✓     | ✓     | **推荐**         |
| HMAC    | ✓     | ✓     | 已弃用，将在未来版本中移除。 |
| RSA     | ✓     | -     | 已弃用，执行不支持。     |

:::tip
现在就切换到 Ed25519 密钥。生成一个 Ed25519 密钥对并在 Binance 注册。参见下方的[生成 Ed25519 密钥](#生成-ed25519-密钥)。
:::

:::note
Ed25519 密钥必须以未加密的 PEM 格式（base64 编码的 ASN.1/DER）提供。实现会自动从 DER 结构中提取 32 字节的种子。加密（密码保护）的 PEM 密钥不受支持。如果你的密钥已加密，请先解密：`openssl pkey -in encrypted.pem -out decrypted.pem`
:::

#### 生成 Ed25519 密钥

**方案 1：OpenSSL（推荐）**

```bash
# 生成私钥（PKCS#8 PEM 格式）
openssl genpkey -algorithm ed25519 -out binance_ed25519_private.pem

# 提取公钥
openssl pkey -in binance_ed25519_private.pem -pubout -out binance_ed25519_public.pem
```

**方案 2：Binance Key Generator**

从发布页面下载 [Binance Asymmetric Key Generator](https://github.com/binance/asymmetric-key-generator) 并运行它来生成密钥对。

**在 Binance 注册**

1. 登录 Binance 并前往 **Profile** -> **API Management**
2. 点击 **Create API** 并选择 **Self-generated**
3. 粘贴你的公钥文件内容（包括 `-----BEGIN PUBLIC KEY-----` 头/尾）
4. 配置权限（启用现货和保证金交易等）

**在 NautilusTrader 中使用**

将私钥设置为你的 API secret：

```bash
export BINANCE_API_KEY="your-api-key-from-binance"
export BINANCE_API_SECRET="$(cat binance_ed25519_private.pem)"
```

或在你的配置中直接传入 PEM 内容。

:::warning
请妥善保管你的私钥。切勿分享它或将其提交到版本控制中。
:::

### API 凭证

将凭证直接传递给配置对象，或设置相应的环境变量（按环境划分的变量参见[环境](#环境)）。

:::tip
对所有客户端使用 Ed25519 密钥。HMAC 密钥仍可用于数据客户端和执行客户端，但 Ed25519 提供更好的性能，并将在未来版本中成为唯一支持的密钥类型。参见[密钥类型](#密钥类型)。
:::

:::warning
`BINANCE_ED25519_*` 和 `BINANCE_*_ED25519_*` 环境变量在现货/保证金中已被移除。对于期货，它们已被弃用，并将在未来版本中移除。请将它们重命名为 `BINANCE_API_KEY` / `BINANCE_API_SECRET`（Ed25519 密钥现在会被自动检测）。
:::

当交易节点启动时，你会收到关于你的凭证是否有效以及是否具有交易权限的确认。

### 账户类型

使用 `BinanceAccountType` 枚举设置 `account_type`：

- `SPOT`
- `USDT_FUTURES`（以 USDT 或 BUSD 稳定币作为抵押品）
- `COIN_FUTURES`（以其他加密货币作为抵押品）

:::note
枚举中存在 `MARGIN` 和 `ISOLATED_MARGIN` 账户类型，但保证金交易尚未实现。参见[产品支持](#产品支持)。
:::

### 基础 URL 覆盖

可以覆盖 HTTP REST 和 WebSocket API 的默认基础 URL。这对于配置 API 集群，或当 Binance 为你提供了专用端点时非常有用。

### Binance US

在配置中设置 `us=True` 以使用 Binance US 端点（默认为 `False`）。US 账户可用的所有功能与标准 Binance 行为一致。

### 环境

Binance 提供三种交易环境，每种都有独立的 API 凭证和端点。`environment` 配置选项用于选择使用哪一种。

| 环境          | 配置                      | 描述                  |
| ----------- | ----------------------- | ------------------- |
| **Live**    | `environment="LIVE"`    | 使用真实资金的生产环境交易（默认）。  |
| **Demo**    | `environment="DEMO"`    | 使用模拟的现货和期货资金进行模拟交易。 |
| **Testnet** | `environment="TESTNET"` | 旧版的现货和期货测试网络。       |

#### Live（生产环境）

用于使用真实资金进行实盘交易的默认环境。使用你的主 Binance 账户凭证。

```python
config = BinanceExecClientConfig(
    api_key="YOUR_API_KEY",
    api_secret="YOUR_API_SECRET",
    account_type=BinanceAccountType.SPOT,
    # environment=BinanceEnvironment.LIVE（默认）
)
```

| 变量                   | 描述             |
| -------------------- | -------------- |
| `BINANCE_API_KEY`    | 实盘 API 密钥。     |
| `BINANCE_API_SECRET` | 实盘 API secret。 |

#### Demo 交易

在生产基础设施上使用模拟资金练习交易。Demo 账户使用与你实盘账户相同的 Binance 登录，但使用虚拟余额进行交易。

**如何获取 Demo 凭证：**

1. 在 [binance.com/en/demo-trading](https://www.binance.com/en/demo-trading) 登录。
2. 前往 **API Management** 并创建一个 demo API 密钥。
3. Demo 密钥可用于现货和期货的 demo 端点。

| 端点          | URL                        |
| ----------- | -------------------------- |
| 现货 HTTP     | `demo-api.binance.com`     |
| 现货 WS       | `demo-stream.binance.com`  |
| USD-M HTTP  | `demo-fapi.binance.com`    |
| USD-M WS    | `demo-fstream.binance.com` |
| COIN-M HTTP | `demo-dapi.binance.com`    |
| COIN-M WS   | `demo-dstream.binance.com` |

```python
config = BinanceExecClientConfig(
    api_key="YOUR_DEMO_API_KEY",
    api_secret="YOUR_DEMO_API_SECRET",
    account_type=BinanceAccountType.SPOT,
    environment=BinanceEnvironment.DEMO,
)
```

| 变量                        | 描述               |
| ------------------------- | ---------------- |
| `BINANCE_DEMO_API_KEY`    | Demo API 密钥。     |
| `BINANCE_DEMO_API_SECRET` | Demo API secret。 |

#### Testnet

一个拥有自己的用户账户、余额和订单簿的旧版测试网络。对于新的模拟交易设置，建议使用 `environment=BinanceEnvironment.DEMO`。现货测试网仍位于 `testnet.binance.vision`；期货测试网端点可能通过 Demo 交易基础设施路由。

**如何获取现货测试网凭证：**

1. 前往 [testnet.binance.vision](https://testnet.binance.vision/)。
2. 使用 GitHub 登录。
3. 生成一个 API 密钥（HMAC、RSA 或 Ed25519）。

**期货测试网：** 使用 `BinanceEnvironment.TESTNET` 的现有配置仍可继续工作，但新的期货测试应使用 `BinanceEnvironment.DEMO`。

```python
config = BinanceExecClientConfig(
    api_key="YOUR_TESTNET_API_KEY",
    api_secret="YOUR_TESTNET_API_SECRET",
    account_type=BinanceAccountType.SPOT,
    environment=BinanceEnvironment.TESTNET,
)
```

| 变量                                   | 描述                |
| ------------------------------------ | ----------------- |
| `BINANCE_TESTNET_API_KEY`            | 现货测试网 API 密钥。     |
| `BINANCE_TESTNET_API_SECRET`         | 现货测试网 API secret。 |
| `BINANCE_FUTURES_TESTNET_API_KEY`    | 期货测试网 API 密钥。     |
| `BINANCE_FUTURES_TESTNET_API_SECRET` | 期货测试网 API secret。 |

:::note
测试网凭证与你的实盘账户完全分离。市场数据和流动性与生产环境不同。
:::

### 聚合交易

Binance 提供聚合交易数据端点作为交易的替代数据源。与默认的交易端点不同，聚合交易端点可以返回 `start_time` 和 `end_time` 之间的所有 tick。

设置 `use_agg_trade_ticks=True` 以使用聚合交易（默认为 `False`）。

:::note
对于期货（USD-M 和 COIN-M），WebSocket 交易订阅始终使用 `@aggTrade`。Binance 在期货 WebSocket 上只发布聚合交易；旧版的 `@trade` 流未被记录在文档中，已被静默禁用。HTTP 的 `request_trade_ticks` 路径继续遵循 `use_agg_trade_ticks`。
:::

### 手续费率查询

默认情况下，Binance Futures 金融工具使用基于你 VIP 等级的费率层级表。对于具有负 maker 费率的做市商账户或需要精确费率时，可以启用按交易对查询手续费率：

```python
from nautilus_trader.adapters.binance import BinanceInstrumentProviderConfig

instrument_provider = BinanceInstrumentProviderConfig(
    load_all=True,
    query_commission_rates=True,  # 按交易对查询精确费率
)
```

启用后，适配器会在加载金融工具期间并行查询 Binance 的 `/fapi/v1/commissionRate` 端点以获取每个交易对的费率。这对以下情况很有用：

- 具有负 maker 费率的做市商账户。
- 具有自定义费率安排的账户。
- 用于盈亏计算的精确手续费率。

适配器使用带有速率限制的并行请求（120 请求/分钟，已考虑该端点 20 的权重）。如果查询失败，会回退到费率层级表。

### 解析器警告

如果某些 Binance 金融工具包含超出平台处理能力的字段值，则无法被解析为 Nautilus 对象。这些金融工具会被跳过并发出警告。

要抑制这些警告：

```python
from nautilus_trader.config import InstrumentProviderConfig

instrument_provider = InstrumentProviderConfig(
    load_all=True,
    log_warnings=False,
)
```

### 期货对冲模式

Binance Futures 对冲模式允许在同一金融工具上同时持有多头和空头持仓。

要使用对冲模式：

1. 在启动策略之前，先在 Binance 上配置对冲模式。
2. 在 `BinanceExecClientConfig` 中将 `use_reduce_only` 设为 `False`（默认为 `True`）。

    ```python
    from nautilus_trader.adapters.binance import BINANCE

    config = TradingNodeConfig(
        ...,  # 省略
        data_clients={
            BINANCE: BinanceDataClientConfig(
                api_key=None,  # 'BINANCE_API_KEY' 环境变量
                api_secret=None,  # 'BINANCE_API_SECRET' 环境变量
                account_type=BinanceAccountType.USDT_FUTURES,
                base_url_http=None,  # 使用自定义端点覆盖
                base_url_ws=None,  # 使用自定义端点覆盖
            ),
        },
        exec_clients={
            BINANCE: BinanceExecClientConfig(
                api_key=None,  # 'BINANCE_API_KEY' 环境变量
                api_secret=None,  # 'BINANCE_API_SECRET' 环境变量
                account_type=BinanceAccountType.USDT_FUTURES,
                base_url_http=None,  # 使用自定义端点覆盖
                base_url_ws=None,  # 使用自定义端点覆盖
                use_reduce_only=False,  # 对冲模式必须禁用
            ),
        },
    )
    ```

3. 提交订单时，在 `position_id` 中使用 `LONG` 或 `SHORT` 后缀来指示持仓方向。

    ```python
    class EMACrossHedgeMode(Strategy):
        (...,)  # 省略

        def buy(self) -> None:
            order: MarketOrder = self.order_factory.market(
                instrument_id=self.instrument_id,
                order_side=OrderSide.BUY,
                quantity=self.instrument.make_qty(self.trade_size),
                # time_in_force=TimeInForce.FOK,
            )

            # LONG 后缀被 Binance 适配器识别为多头持仓。
            position_id = PositionId(f"{self.instrument_id}-LONG")
            self.submit_order(order, position_id)

        def sell(self) -> None:
            order: MarketOrder = self.order_factory.market(
                instrument_id=self.instrument_id,
                order_side=OrderSide.SELL,
                quantity=self.instrument.make_qty(self.trade_size),
                # time_in_force=TimeInForce.FOK,
            )
            # SHORT 后缀被 Binance 适配器识别为空头持仓。
            position_id = PositionId(f"{self.instrument_id}-SHORT")
            self.submit_order(order, position_id)
    ```

## 贡献

:::info
要为 Binance 适配器做出贡献，请参阅[贡献指南](https://github.com/nautechsystems/nautilus_trader/blob/develop/CONTRIBUTING.md)。
:::
