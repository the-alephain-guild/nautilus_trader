# Lighter

[Lighter](https://lighter.xyz) 是一个面向现货和永续合约的去中心化中央限价订单簿 (central-limit-order-book) 交易所。该交易场所通过以太坊零知识 rollup 进行结算，而撮合与排序则在链下运行。

NautilusTrader 的 Lighter 适配器由 `nautilus-lighter` crate 实现。它提供了 Rust 数据与执行客户端、带类型的 REST 和 WebSocket 模型，以及一个内置的 L2 交易签名器，用于该交易场所的 Schnorr / ECgFp5 签名流程。

## 概览 (Overview)

该适配器由以下主要组件构成：

- `LighterRawHttpClient`：面向公开和账户端点的底层 REST 客户端。
- `LighterHttpClient`：领域客户端，负责将 instrument、成交、订单簿、订单和账户状态解析为 Nautilus 模型类型。
- `LighterWebSocketClient`：可重连的 WebSocket 客户端，用于公开行情流和私有账户流。
- `LighterDataClient`：Nautilus 数据客户端，用于 instrument、成交、报价以及 L2 MBP 订单簿。
- `LighterExecutionClient`：Nautilus 执行客户端，用于账户流、订单提交、修改、撤销以及对账报告。
- `LighterDataClientFactory` 和 `LighterExecutionClientFactory`：live-node 工厂装配。

Python 接口面被刻意收窄。Python 扩展暴露了配置、环境选择、工厂类和 integrator 撤销；数据与执行客户端则通过 Rust trait 接口面消费。

## 示例 (Examples)

该适配器包含 Python v2 和 Rust live-node 示例。Python 示例位于
[`python/examples/lighter/`](https://github.com/nautechsystems/nautilus_trader/tree/develop/python/examples/lighter/)，
默认采用 dry build：它们构建节点、注册 tester，然后退出，除非传入 `--run`。

```fish
cd python
.venv/bin/python examples/lighter/data_tester.py --lighter-environment testnet
.venv/bin/python examples/lighter/exec_tester.py --lighter-environment testnet
```

传入 `--run` 以连接到 Lighter。除非同时传入 `--live-orders`，否则执行 tester 始终保持在 `dry_run` 模式。

```fish
cd python
.venv/bin/python examples/lighter/data_tester.py \
    --lighter-environment mainnet \
    --instrument BTC-PERP.LIGHTER \
    --run
.venv/bin/python examples/lighter/exec_tester.py \
    --lighter-environment mainnet \
    --instrument DOGE-PERP.LIGHTER \
    --run
```

Rust 示例位于 `crates/adapters/lighter/examples/` 下，并会立即运行：

```fish
cargo run --example lighter-data-tester --package nautilus-lighter --features examples
cargo run --example lighter-exec-tester --package nautilus-lighter --features examples
```

:::warning
示例可能连接到真实的交易场所。在启用了实时订单流的情况下，执行示例如果指向已注资的 mainnet 账户，可能会提交真实订单。运行前请检查所选的 instrument、数量和环境。
:::

## 产品支持 (Product support)

| 产品类型                     | 数据源 | 交易  | 备注                                       |
| ------------------------ | --- | --- | ---------------------------------------- |
| 现货 (Spot)                | ✓   | ✓   | 使用 Lighter market index 2048-4094 的现货市场。 |
| 永续合约 (Perpetual futures) | ✓   | ✓   | 使用 Lighter market index 0-254 的线性永续市场。   |
| 定期合约 (Dated futures)     | -   | -   | *不支持*。                                   |
| 期权 (Options)             | -   | -   | *不支持*。                                   |

## 限制 (Limitations)

当前适配器的范围被刻意设计得比该交易场所的完整交易接口面更窄：

- 分组订单列表 (grouped order lists)、OCO/OTO 组、bracket 订单、TWAP、追踪止损 (trailing stops) 以及 iceberg 显示数量均未实现。
- 原生批量提交 (native batch submit) 和原生批量撤销 (native batch cancel) 仅为相互独立的订单操作而设计。批量提交在一个 `sendTxBatch` 中发送相互独立的 `L2CreateOrder` 交易，批量撤销则在一个 `sendTxBatch` 中签署相互独立的 `L2CancelOrder` 交易。两者每批均上限为 15 笔交易。
- 分组的交易场所订单仍不在支持范围内：批量提交不使用 `CreateGroupedOrders`，也不提供原子化的 OCO/OTO 或 bracket 分组。
- `CancelAllOrders` 使用所请求 instrument 的缓存挂单。适配器不使用 Lighter 原生的账户级全撤交易，因为它可能影响无关的市场。
- 现货交易支持市价单和限价单。条件性止损和止盈订单仅限于永续市场。
- 账户状态和持仓报告来自私有 WebSocket 流。`query_account` 和持仓状态生成会重放最新缓存的流状态。
- 无作用域的订单对账被限定在已配置或已观察到的活跃市场范围内，以避免在标准 REST 配额下触发覆盖整个交易场所的全量扩散。
- 历史成交请求对主账户和子账户均需要凭证。

## 符号体系 (Symbology)

Lighter 通过数值型的 `market_index` 值来标识市场。适配器从 `GET /api/v1/orderBookDetails` 引导出该映射，然后将原始的交易场所符号转换为 Nautilus `InstrumentId`。

| 交易场所产品 | Nautilus 符号格式         | 示例                 | 备注              |
| ------ | --------------------- | ------------------ | --------------- |
| 永续合约   | `{BASE}-PERP.LIGHTER` | `BTC-PERP.LIGHTER` | 原始交易场所符号 `BTC`。 |
| 现货     | `{BASE}-SPOT.LIGHTER` | `ETH-SPOT.LIGHTER` | 原始交易场所符号 `ETH`。 |

后缀用于区分共享同一交易场所符号的现货和永续挂牌。出站请求会去掉后缀，并使用缓存的 `market_index`。

## 环境 (Environments)

| 环境      | REST URL                              | WebSocket URL                              | Chain ID |
| ------- | ------------------------------------- | ------------------------------------------ | -------- |
| Mainnet | `https://mainnet.zklighter.elliot.ai` | `wss://mainnet.zklighter.elliot.ai/stream` | 304      |
| Testnet | `https://testnet.zklighter.elliot.ai` | `wss://testnet.zklighter.elliot.ai/stream` | 300      |

在数据和执行配置中使用 `LighterEnvironment::Mainnet` 或 `LighterEnvironment::Testnet`。也可以通过 URL 覆盖来指向私有网关或本地测试 fixture。

## Integrator 归属 (Integrator attribution)

提交的创建和修改订单交易会在 Lighter 的 `L2TxAttributes` 中携带 NautilusTrader integrator 账户索引。这有助于我们衡量该集成的真实使用情况，并据此安排后续维护的优先级。Maker 和 taker 的 integrator 费用均设为零，因此该归属不会带来任何交易成本。

在这些属性能够附加到订单之前，Lighter 要求先进行一次 `ApproveIntegrator` 批准。在启动期间，执行客户端会为已配置的 L2 账户提交所需的**零费用 (zero-fee)** 批准。

### 撤销批准 (Revoking the approval)

在离开该适配器时，把撤销作为清理手段使用。它会发送相同的 `ApproveIntegrator` 交易，但 `approval_expiry = 0` 且每一项 max fee 均设为零；下一次执行客户端启动时会重新记录一次零费用批准。

```bash
export LIGHTER_API_KEY_INDEX=0
export LIGHTER_API_SECRET=REPLACE_ME
export LIGHTER_ACCOUNT_INDEX=123456
cargo run -p nautilus-lighter --bin lighter-integrator-revoke           # mainnet
cargo run -p nautilus-lighter --bin lighter-integrator-revoke testnet   # testnet
```

脚本源码：
[`crates/adapters/lighter/bin/integrator_revoke.rs`](https://github.com/nautechsystems/nautilus_trader/blob/develop/crates/adapters/lighter/bin/integrator_revoke.rs)。

```python
# Python (PyO3 绑定) - 读取与 Rust bin 相同的环境变量
from nautilus_trader.core.nautilus_pyo3 import revoke_lighter_integrator
from nautilus_trader.core.nautilus_pyo3 import LighterEnvironment

await revoke_lighter_integrator()  # mainnet (默认)
await revoke_lighter_integrator(LighterEnvironment.TESTNET)  # testnet
```

Rust 脚本会打印该操作的摘要，并在签名或发送之前暂停等待按下 Enter 键；如果摘要中有任何看起来不对的地方，请在此之前用 `Ctrl+C` 中止。Python 绑定不会提示：请在调用前自行检查当前生效的环境变量。

## 数据订阅 (Data subscriptions)

| 数据类型                         | 订阅   | 快照  | 历史  | Nautilus 类型         | 备注                            |
| ---------------------------- | ---- | --- | --- | ------------------- | ----------------------------- |
| Instrument 元数据               | 缓存重放 | ✓   | -   | `InstrumentAny`     | 从 `orderBookDetails` 加载。      |
| 成交 Tick (Trade ticks)        | ✓    | -   | ✓   | `TradeTick`         | WebSocket 成交；历史 REST 成交需要鉴权。  |
| 报价 Tick (Quote ticks)        | ✓    | -   | -   | `QuoteTick`         | 最优买卖 ticker 流。                |
| 订单簿增量 (Order book deltas)    | ✓    | ✓   | -   | `OrderBookDeltas`   | 仅 `L2_MBP`。                   |
| 订单簿 depth10                  | ✓    | ✓   | -   | `OrderBookDepth10`  | 从维护的订单簿状态得到的实时前 10 档视图。       |
| 订单簿快照 (Order book snapshots) | -    | ✓   | -   | `OrderBook`         | REST 快照，最大深度 250。             |
| 标记价格 (Mark prices)           | ✓    | -   | -   | `MarkPriceUpdate`   | 永续市场统计流。                      |
| 指数价格 (Index prices)          | ✓    | -   | -   | `IndexPriceUpdate`  | 市场和现货统计流。                     |
| 资金费率 (Funding rates)         | ✓    | -   | ✓   | `FundingRateUpdate` | 当前预估值及 REST 小时级历史。            |
| Bar                          | ✓    | -   | ✓   | `Bar`               | WebSocket K 线流；用于回填的 REST 历史。 |
| Instrument 状态                | REST | ✓   | -   | `InstrumentStatus`  | `active` / `inactive` 快照。     |

订单簿增量和 depth10 订阅仅接受 `BookType::L2_MBP`。其他订单簿类型会在订阅前返回错误。

WebSocket 订单簿仅从 `subscribed/order_book` 初始化。如果在该快照到达之前先收到了 `update/order_book`，适配器会将其丢弃并等待真正的快照，因为增量更新并不包含完整的可见订单簿。

Depth10 订阅与增量使用相同的 WebSocket `order_book` 流。适配器在每次接受快照或增量更新后都会发出一份刷新后的前 10 档视图。

Bar 订阅使用交易场所的 `candle/{market_id}/{resolution}` WebSocket 频道。Lighter 大约每 500 毫秒批量推送一次当前未收盘 bar 的进行中更新；适配器仅在 K 线起始时间戳前进时才发出一个 Nautilus `Bar`，因此消费者每个已收盘周期只会看到一个事件。进行中缓存会在重连和取消订阅时被清空。

该数据流支持 `1m`、`5m`、`15m`、`30m`、`1h`、`4h`、`12h` 和 `1d`。`1w` 仅可通过 `request_bars` 经 REST 获取；订阅 `1-WEEK` bar 类型会返回错误。

Instrument 状态订阅会在可用时重放最新缓存的 `orderBookDetails` 状态，否则获取一份 REST 快照。Lighter 不暴露 WebSocket 状态变更流。

资金费率订阅使用 `market_stats.current_funding_rate`，它是 Lighter 对即将到来的支付所做的预估。历史资金费率请求使用 `/api/v1/fundings`（`1h` 分辨率），并将已结算的行映射为 `FundingRateUpdate`（`interval=60`）。REST 的 `direction` 字段控制符号：`long` 保持为正，因为多头支付空头；而 `short` 被映射为负，因为空头支付多头。适配器在公开资金费率历史中不使用账户特定的 `positionFunding` 载荷。

成交订阅使用公开的 WebSocket 成交流。历史成交请求使用 `/api/v1/trades`；实时 mainnet 测试表明该端点会拒绝未鉴权的请求。当存在可用凭证时，数据客户端会为该请求铸造一个 Lighter 鉴权 token。在没有凭证的情况下，数据客户端会记录一条警告并拒绝该请求。

### 不支持的数据请求 (Unsupported data requests)

`request_quotes` 未实现。Lighter 通过 WebSocket `ticker` 流暴露最优买卖盘数据，但适配器可用的 REST 端点并不提供能够安全映射到 `QuoteTick` 的带时间戳报价快照或报价历史。

`request_book_depth` 未实现。已记录的 REST 订单簿端点不提供用于 `OrderBookDepth10.ts_event` 的交易场所事件时间戳；请使用 `subscribe_book_depth10` 获取实时 depth10 快照，或使用 `request_book_snapshot` 获取一份 REST `OrderBook` 快照。

## 订单能力 (Orders capability)

### 订单标识 (Order identification)

Lighter 使用一个数值型的交易场所订单索引以及一个由调用方提供的 `client_order_index`。适配器从 Nautilus 的 `ClientOrderId` 派生出 Lighter 的 `client_order_index`，并维护一个本地映射，以便私有 WebSocket 报告能够恢复原始的客户端订单 ID。

在所需映射可用时，查询路径既可以使用 Nautilus 客户端订单 ID，也可以使用数值型的交易场所订单 ID。

### 订单类型 (Order types)

| 订单类型                   | 永续  | 现货  | 备注                              |
| ---------------------- | --- | --- | ------------------------------- |
| `MARKET`               | ✓   | ✓   | 上限由缓存的对手方报价 + 滑点派生。             |
| `LIMIT`                | ✓   | ✓   | 需要限价。                           |
| `STOP_MARKET`          | ✓   | -   | 仅永续；上限由 `trigger_price` + 滑点派生。 |
| `STOP_LIMIT`           | ✓   | -   | 仅永续；映射到 Lighter 止损限价单。          |
| `MARKET_IF_TOUCHED`    | ✓   | -   | 仅永续；上限由 `trigger_price` + 滑点派生。 |
| `LIMIT_IF_TOUCHED`     | ✓   | -   | 仅永续；映射到 Lighter 止盈限价单。          |
| `MARKET_TO_LIMIT`      | -   | -   | *不支持*。                          |
| `TRAILING_STOP_MARKET` | -   | -   | *不支持*。                          |
| `TRAILING_STOP_LIMIT`  | -   | -   | *不支持*。                          |
| `TWAP`                 | -   | -   | *不支持*；无 Nautilus 映射。            |

条件性订单类型仅对永续市场可用。现货条件性订单会在本地被拒绝，因为 Lighter 会在交易场所侧拒绝它们。条件性订单类型必须包含 `trigger_price`。如果触发价缺失，`STOP_MARKET` 和 `MARKET_IF_TOUCHED` 会被提前拒绝；如果触发价在该 instrument 的价格精度下被截断为 `0` ticks，则所有条件性类型都会被拒绝。

Lighter 的市价类订单在传输层上需要一个最差可接受的 `price` 字段。适配器会自动派生它：`MARKET` 订单读取缓存的对手方 `QuoteTick`（买入读 ask，卖出读 bid）；`STOP_MARKET` 和 `MARKET_IF_TOUCHED` 使用订单的 `trigger_price`。该基准价会按 `market_order_slippage_bps`（默认 50 bps = 0.5%）放宽，并在该 instrument 的价格精度下做保守取整（买入向上取整，卖出向下取整）。在策略尚未订阅报价之前提交的 `MARKET` 订单会被拒绝，并附带清晰的错误信息。可通过 `SubmitOrder.params["market_order_slippage_bps"]` 按单覆盖。

### 关联订单 (Contingent orders)

| 特性                           | 永续  | 现货  | 备注                                             |
| ---------------------------- | --- | --- | ---------------------------------------------- |
| 止损市价 (Stop-loss market)      | ✓   | -   | `STOP_MARKET` 映射到 Lighter `STOP_LOSS`。         |
| 止损限价 (Stop-loss limit)       | ✓   | -   | `STOP_LIMIT` 映射到 Lighter `STOP_LOSS_LIMIT`。    |
| 止盈市价 (Take-profit market)    | ✓   | -   | `MARKET_IF_TOUCHED` 映射到 Lighter `TAKE_PROFIT`。 |
| 止盈限价 (Take-profit limit)     | ✓   | -   | `LIMIT_IF_TOUCHED` 映射到 `TAKE_PROFIT_LIMIT`。    |
| 触发价 (Trigger price)          | ✓   | -   | 每个受支持的条件性订单都需要。                                |
| 触发价类型 (Trigger price type)   | -   | -   | *不支持*；无触发来源选择器。                                |
| 分组订单列表 (Grouped order lists) | -   | -   | *不支持*。                                         |
| OCO / OTO 订单                 | -   | -   | *不支持*。                                         |
| Bracket 订单                   | -   | -   | *不支持*。                                         |
| `CreateGroupedOrders`        | -   | -   | *不支持*；原生批处理使用相互独立的交易。                          |

### 订单选项 (Order options)

| 选项               | 永续  | 现货  | 备注                                     |
| ---------------- | --- | --- | -------------------------------------- |
| `post_only`      | ✓   | ✓   | 映射到 Lighter 的 post-only time-in-force。 |
| `reduce_only`    | ✓   | -   | 透传到 `CreateOrder`；仅用于减少已有持仓。           |
| `quote_quantity` | -   | -   | *不支持*；请改为提交 base 数量。                   |
| `display_qty`    | -   | -   | *不支持*；Lighter 不暴露 iceberg 显示数量字段。      |

### 适配器订单参数 (Adapter order params)

| 参数                                      | 永续  | 现货  | 备注                       |
| --------------------------------------- | --- | --- | ------------------------ |
| `market_order_slippage_bps`             | ✓   | ✓   | 覆盖市价类上限的配置默认值。           |
| 通过 `SubmitOrder.params` 传 `post_only`   | -   | -   | *不支持*；请使用 Nautilus 订单标志。 |
| 通过 `SubmitOrder.params` 传 `reduce_only` | -   | -   | *不支持*；请使用 Nautilus 订单标志。 |

### 有效期 (Time in force)

| 有效期            | 永续  | 现货  | 备注                                       |
| -------------- | --- | --- | ---------------------------------------- |
| `GTC`          | ✓   | ✓   | 限价类使用 `GoodTillTime`；市价类使用 `IOC`。        |
| `DAY`          | ✓   | ✓   | 限价类和条件性订单使用正的订单到期时间。                     |
| `GTD`          | ✓   | ✓   | 限价类和条件性订单使用所提供的 Nautilus 到期时间。           |
| `IOC`          | ✓   | ✓   | 普通 `MARKET`/`LIMIT` 使用到期 `0`；条件限价使用触发到期。 |
| `FOK`          | -   | -   | *不支持*。                                   |
| `AT_THE_OPEN`  | -   | -   | *不支持*。                                   |
| `AT_THE_CLOSE` | -   | -   | *不支持*。                                   |

对于 `MARKET`、`STOP_MARKET` 和 `MARKET_IF_TOUCHED`，适配器会把传输层有效期映射为 Lighter `ImmediateOrCancel`，因为该交易场所会拒绝以 `GoodTillTime` 发送的市价类订单。普通 `MARKET` 订单设置 `OrderExpiry = 0`。条件市价订单（`STOP_MARKET` 和 `MARKET_IF_TOUCHED`）保留一个正的 `OrderExpiry`，以便触发器可以挂着等待，传输层的 `ImmediateOrCancel` 仅在触发器触发后才生效。Nautilus 的 `IOC` 无法为条件市价订单表示，因此适配器会在本地拒绝它并附带清晰的错误信息。条件限价订单（`STOP_LIMIT` 和 `LIMIT_IF_TOUCHED`）可以使用 Nautilus `IOC`：触发器以一个正的 `OrderExpiry` 挂着等待，子限价订单在触发后使用 Lighter `ImmediateOrCancel`。

当未提供显式的 GTD 到期时间时，限价类的 `GTC`、`DAY` 和 `GTD` 订单默认采用当前时间加 28 天。条件性 `GTC`、`DAY` 以及限价类 `IOC` 订单使用相同的默认到期时间。该交易场所会拒绝把 `-1` 作为这些 TIF 的到期值，视其为无效。实时测试还表明，过短的 GTD 到期时间可能会被排序器以 `21711 invalid expiry` 拒绝；在实时 GTD 测试中请使用一个交易场所可接受的到期时间范围。

### 执行指令 (Execution instructions)

| 指令            | 永续  | 现货  | 备注                             |
| ------------- | --- | --- | ------------------------------ |
| `post_only`   | ✓   | ✓   | 覆盖 TIF 并发送 Lighter `PostOnly`。 |
| `reduce_only` | ✓   | -   | 针对已有衍生品持仓的减仓标志。                |

请在限价类订单上使用 `post_only`。适配器不会合成只做 maker 的市价单。实时 mainnet 测试确认了用于平掉永续持仓的 `reduce_only=true`。无效的减仓开仓可能会被 Lighter 丢弃而不产生交易场所订单报告；适配器会将它们对账为 `INFLIGHT_TIMEOUT`，而非交易场所提供的拒绝原因。

### 高级订单特性 (Advanced order features)

| 特性                        | 永续  | 现货  | 备注                               |
| ------------------------- | --- | --- | -------------------------------- |
| 订单修改 (Order modification) | ✓   | ✓   | 在活跃订单上修改数量、价格和触发价。               |
| Bracket 订单                | -   | -   | *不支持*。                           |
| Iceberg 订单                | -   | -   | *不支持*。                           |
| 追踪止损 (Trailing stops)     | -   | -   | *不支持*。                           |
| 钉住订单 (Pegged orders)      | -   | -   | *不支持*。                           |
| TWAP 订单                   | -   | -   | *不支持*；无 Nautilus 映射。             |
| 杠杆更新 (Leverage update)    | ✓   | -   | 仅永续；提交一笔签名的 `UpdateLeverage` 交易。 |
| 原生全撤 (Native cancel-all)  | -   | -   | *不支持*；适配器按 instrument 限定全撤范围。    |
| 死手开关 (Dead man's switch)  | -   | -   | *不支持*。                           |

### 订单操作 (Order operations)

| 操作                           | 永续  | 现货  | 备注                                       |
| ---------------------------- | --- | --- | ---------------------------------------- |
| 提交订单 (Submit order)          | ✓   | ✓   | 通过 WebSocket 发送一笔签名的 `L2CreateOrder` 交易。 |
| 提交订单列表 (Submit order list)   | ✓   | ✓   | 仅批处理相互独立的 `L2CreateOrder` 交易。            |
| 修改订单 (Modify order)          | ✓   | ✓   | 发送一笔签名的 `ModifyOrder`；报告可能会重述接受状态。       |
| 撤销订单 (Cancel order)          | ✓   | ✓   | 发送一笔签名的 `L2CancelOrder` 交易。              |
| 撤销所有订单 (Cancel all orders)   | ✓   | ✓   | 遍历所请求 instrument 的缓存挂单。                  |
| 设置杠杆 (Set leverage)          | ✓   | -   | 仅永续；提交一笔签名的 `UpdateLeverage` 交易。         |
| 批量撤销订单 (Batch cancel orders) | ✓   | ✓   | 仅批处理相互独立的 `L2CancelOrder` 交易。            |
| 原生批量提交 (Native batch submit) | ✓   | ✓   | 使用一个 `sendTxBatch`，上限为 15 笔创建交易。         |
| 原生批量撤销 (Native batch cancel) | ✓   | ✓   | 使用一个 `sendTxBatch`，上限为 15 笔撤销交易。         |
| 查询订单 (Query order)           | ✓   | ✓   | 需要凭证和 REST 查询。                           |
| 查询账户 (Query account)         | ✓   | ✓   | 重放最新的私有 WebSocket 账户状态。                  |
| 批量状态 (Mass status)           | ✓   | ✓   | 限定在来自 WS 和 REST 报告的账户活跃市场范围内。            |

原生交易场所的 `CancelAllOrders` 交易是账户级的。适配器刻意按 instrument 撤销缓存挂单，以避免触及无关的市场。

`SubmitOrderList` 和 `BatchCancelOrders` 对相互独立的操作使用 `sendTxBatch`。它们不创建分组的交易场所订单，不提供原子化的 OCO/OTO 或 bracket 语义，也不对有作用域的撤销使用账户级的 `CancelAllOrders`。

`sendTxBatch` 响应暴露一个顶层 API 代码和一个 `tx_hash` 列表；它不暴露逐订单的 API 拒绝字段。一个成功的批处理响应会将已签名的交易排入队列，随后私有账户流会报告最终的逐订单提交、撤销、成交和拒绝结果。

`UpdateLeverage` 通过 `LighterExecutionClient::update_leverage(instrument_id, initial_margin_fraction, margin_mode)` 暴露。`initial_margin_fraction` 以交易场所 ticks（1e-4 分数）表示：`500` 表示 5% 初始保证金（20 倍杠杆），`1000` 表示 10%（10 倍），依此类推。

`UpdateLeverage` 在本仓库中没有 oracle 测试向量；其 body 字段顺序对照上游签名器的 cgo 头文件进行固定，并通过向 Lighter mainnet 提交一笔被排序器接受的签名交易验证了其传输格式。

### 订单查询与对账 (Order querying and reconciliation)

| 特性                            | 永续  | 现货  | 备注                                  |
| ----------------------------- | --- | --- | ----------------------------------- |
| 查询挂单 (Query open orders)      | ✓   | ✓   | REST `accountActiveOrders`，按市场限定范围。 |
| 查询订单历史 (Query order history)  | ✓   | ✓   | REST `accountInactiveOrders`，带游标分页。 |
| 订单状态更新 (Order status updates) | ✓   | ✓   | 私有 WebSocket 订单流加上状态报告。             |
| 成交历史 (Trade history)          | ✓   | ✓   | REST `trades`；账户历史需要凭证。             |
| 成交报告 (Fill reports)           | ✓   | ✓   | REST 和私有 WebSocket 成交载荷。            |
| 持仓报告 (Position reports)       | ✓   | -   | 仅永续；重放缓存的持仓流。                       |
| 账户状态 (Account state)          | ✓   | ✓   | 重放缓存的合并账户状态快照。                      |
| 批量状态 (Mass status)            | ✓   | ✓   | 组合订单、成交和缓存持仓。                       |

## 账户与持仓管理 (Account and position management)

已鉴权的执行客户端会订阅以下私有流：

- `account_all_orders`：订单状态报告。
- `account_all_trades`：成交报告。
- `account_all_positions`：持仓快照。
- `account_all_assets`：逐资产余额快照（现货余额加上永续抵押品）。
- `user_stats`：永续账户保证金汇总（抵押品和可用余额）。

适配器会将 `account_all_assets` 和 `user_stats` 合并为单一账户状态，并且仅在两个流都交付了它们的第一帧之后才发出该状态。

执行客户端在连接之前需要凭证，因为私有账户流和 nonce 刷新是必需的。可以在没有凭证的情况下构造一个客户端，但在 `private_key`、`account_index` 和 `api_key_index` 解析出来之前，实时执行不会连接。

永续持仓以净额 (netting) 模式报告：每个市场一个持仓。现货余额通过账户资产状态到达，而非持仓报告。每一帧 `account_all_positions` 都被视为一份完整的交易场所快照。如果新的一帧省略了某个先前缓存的市场，适配器会为该 instrument 发出一个平仓持仓报告；一个空的 `positions` 映射会清空所有缓存的永续持仓，并为每个发出平仓报告。

| 特性                                   | 永续  | 现货  | 备注                                           |
| ------------------------------------ | --- | --- | -------------------------------------------- |
| 账户余额 (Account balances)              | ✓   | ✓   | 合并资产 + `user_stats`，查询时从缓存重放。                |
| 持仓快照 (Position snapshots)            | ✓   | -   | 仅永续；`account_all_positions` 流。               |
| 净额持仓 (Netting positions)             | ✓   | -   | 每个永续市场一个 Nautilus 持仓。                        |
| 全仓保证金 (Cross margin)                 | ✓   | -   | 通过 `LighterPositionMarginMode::Cross` 透传。    |
| 逐仓保证金 (Isolated margin)              | ✓   | -   | 通过 `LighterPositionMarginMode::Isolated` 透传。 |
| 杠杆更新 (Leverage updates)              | ✓   | -   | 签名的 `UpdateLeverage` 交易。                     |
| 现货保证金 / 借贷 (Spot margin / borrowing) | -   | -   | *不支持*。                                       |
| 充值 / 提现 (Deposits / withdrawals)     | -   | -   | 请使用交易场所工具或交易适配器之外的 Lighter API。              |

## 强平与 ADL 处理 (Liquidation and ADL handling)

| 事件或字段                                | 支持  | 备注                  |
| ------------------------------------ | --- | ------------------- |
| 强平成交 (Liquidation trades)            | ✓   | 账户成交行可解析为成交，没有特殊事件。 |
| 自动减仓成交 (Deleverage trades)           | ✓   | 账户成交行可解析为成交，没有特殊事件。 |
| 强平价格报告 (Liquidation price reporting) | -   | *不支持*；报告中省略此字段。     |
| ADL 事件流 (ADL event stream)           | -   | *不支持*。              |

## 资金费率 (Funding rates)

永续 `market_stats` 帧会发出 `MarkPriceUpdate`、`IndexPriceUpdate` 和 `FundingRateUpdate` 事件。现货 `spot_market_stats` 帧会发出 `IndexPriceUpdate` 事件。

历史资金费率请求使用公开的 `/api/v1/fundings` 端点，并为已结算的小时级行发出 `FundingRateUpdate` 响应。

## 账户层级 (Account tiers)

Lighter 为每个账户分配一个层级，用于管控延迟、速率限制和费用。Standard 是零费用的默认层级；更高的层级在交易场所上是选择性加入的，以费用换取更低的延迟和更高的吞吐。执行客户端在连接时（通过 `GET /api/v1/account`）检测该层级，并以蓝色记录它，对于此适配器尚不识别的任何层级，还会包含原始的 `account_type` 代码。检测仅供参考：适配器从不自行提高速率限制，因为更高的交易场所限制需要向 Lighter 注册调用方 IP，所以仅凭更高的层级本身并不能保证更高的限制对你的连接已经生效。

| 层级       | 延迟 (maker / taker) | REST 加权限制       | `sendTx` 限制          | 费用 (maker / taker)        | 备注                |
| -------- | ------------------ | --------------- | -------------------- | ------------------------- | ----------------- |
| Standard | 200 ms / 300 ms    | 60 req/min      | 60 req/min           | 0 / 0                     | 零费用的默认层级。         |
| Premium  | 0 ms / 140-200 ms  | 24,000 req/min  | 4,000-40,000 req/min | 0.28-0.40 / 1.96-2.80 bps | 最低延迟；随质押的 LIT 扩展。 |
| Plus     | 200 ms / 300 ms    | 120,000 req/min | 8,000 req/min        | 0.5 / 0.5 bps             | 提高了限制，标准延迟。       |
| Builder  | -                  | 240,000 req/min | -                    | -                         | 最高的 REST 吞吐。      |

Premium 的延迟、费用和 `sendTx` 吞吐随质押的 LIT 扩展，且该表可能变化；请参阅 Lighter 文档获取当前数据。要实际使用更高层级的限制，需向 Lighter 注册调用方 IP 并显式设置配额（参见 [速率限制](#rate-limiting)）。

## 速率限制 (Rate limiting)

Lighter 对 IP 地址和 L1 地址都施加速率限制。执行客户端在连接时检测账户层级并记录它（参见 [账户层级](#account-tiers)），但它不会自动提高限制。默认情况下，两个客户端都使用保守的标准账户配额。要使用更高层级的吞吐，需向 Lighter 注册调用方 IP，并在客户端配置中显式设置配额：

- `rest_quota_per_min`：REST 读取桶配额，以每分钟请求数计。不设置则保持 60 req/min。在数据和执行客户端上均可用。
- `sendtx_quota_per_min`：交易配额，以每分钟请求数计，在一个与读取分开计量的桶中计量。不设置则保持标准的 60 req/min，与 `rest_quota_per_min` 相互独立。仅执行客户端可用。

该交易场所在一个桶中跨两种传输方式对每个账户的交易进行计量。执行客户端用一个跨它提交所用两条路径共享的限流器来强制执行 `sendtx_quota_per_min`：WebSocket `sendTx` 路径（单笔订单提交、撤销、修改、杠杆）和 HTTP `sendTx` / `sendTxBatch` 端点（原生批量提交/撤销以及启动时的 integrator 批准）。因此它们的合计速率始终保持在那一个交易场所限制之下。

| 范围                               | 交易场所限制               | 适配器行为                                   |
| -------------------------------- | -------------------- | --------------------------------------- |
| REST，标准账户                        | 60 req/min           | 默认；设置 `rest_quota_per_min` 以覆盖。         |
| REST，premium 账户                  | 24,000 加权 req/min    | 已记录；设置 `rest_quota_per_min` 以使用它。       |
| REST，plus 账户                     | 120,000 加权 req/min   | 已记录；设置 `rest_quota_per_min` 以使用它。       |
| REST，builder 账户                  | 240,000 加权 req/min   | 已记录；设置 `rest_quota_per_min` 以使用它。       |
| `sendTx` / `sendTxBatch`，标准      | 60 req/min           | 单笔使用 `sendTx`；批处理使用 `sendTxBatch`。      |
| `sendTx` / `sendTxBatch`，plus    | 8,000 req/min        | 设置 `sendtx_quota_per_min` 以使用它。         |
| `sendTx` / `sendTxBatch`，premium | 4,000-40,000 req/min | 设置 `sendtx_quota_per_min`（随质押的 LIT 扩展）。 |
| 默认交易类型限制                         | 40 req/min           | 适用于未被成交量配额覆盖的交易类型。                      |
| `L2UpdateLeverage` 交易限制          | 40 req/min           | 与 `update_leverage` 相关。                 |
| 待处理订单 (Pending orders)           | 500/账户，16/市场         | 交易场所限制；适配器不会预先计数。                       |
| 活跃订单 (Active orders)             | 1,500/账户，1,000/市场    | 交易场所限制；适配器不会预先计数。                       |

| 端点或传输方式                         | 限制       | 备注                            |
| ------------------------------- | -------- | ----------------------------- |
| `/api/v1/trades`                | 100 行    | 适配器在对账时按此上限分页。                |
| `/api/v1/accountInactiveOrders` | 100 行    | 适配器在此上限下跟随 `next_cursor`。     |
| `/api/v1/orderBookOrders`       | 250 档    | 快照深度被钳制到交易场所上限。               |
| `/api/v1/candles`               | 500 行    | 适配器将 REST bar 分页上限设为此交易场所最大值。 |
| WebSocket 连接数                   | 200 / IP | 交易场所限制。                       |
| WebSocket 订阅数 / 连接              | 500      | 交易场所限制。                       |
| WebSocket 唯一账户数 / 连接            | 500      | 交易场所限制。                       |
| WebSocket 连接数 / 分钟              | 80       | 交易场所限制。                       |
| WebSocket 客户端消息数 / 分钟           | 200      | 不含 `sendTx` 和 `sendTxBatch`。  |
| WebSocket 在途消息数                 | 50       | 不含 `sendTx` 和 `sendTxBatch`。  |
| `sendTxBatch` 批大小               | 15 笔交易   | 适用于原生 HTTP 提交和撤销批处理。          |
| WebSocket keepalive             | 2 分钟     | 适配器每 30 秒发送一次心跳。              |
| WebSocket 出站命令队列                | 1000     | 适配器从此队列深度开始施加背压。              |

Premium 成交量配额是针对 `L2CreateOrder`、`L2CancelAllOrders`、`L2ModifyOrder` 和 `L2CreateGroupedOrders` 的一个独立交易场所约束。适配器不检查剩余配额；如果某个策略依赖于 premium 或 plus 限制，请使用交易场所账户工具。

## 连接管理 (Connection management)

WebSocket 客户端每 30 秒发送一次心跳，并以从 250 毫秒到最多 30 秒的指数退避进行重连。私有账户订阅使用 Lighter 鉴权 token，其最大 TTL 为 8 小时；适配器在到期前 15 分钟刷新 token 并重新订阅账户频道。

在执行重连时，适配器会通过 `GET /api/v1/nextNonce` 刷新 nonce 基线，然后才恢复签名交易的分发。

在一个会话内，适配器在本地管理交易 nonce：交易场所确认会推进分配窗口，而被拒绝或失败的交易会回滚或触发一次来自 `GET /api/v1/nextNonce` 的重新同步，因此订单流无需重连即可从 nonce 失步中恢复。

`LighterExecutionClient::connect()` 在返回之前会等待最多 30 秒，以让每一个账户流（`account_all_orders`、`account_all_trades`、`account_all_positions`、`account_all_assets`、`user_stats`）交付其第一帧。Lighter 没有用于账户或持仓状态的 REST 端点，因此 WebSocket 帧是唯一的真理源：更早返回会让策略与交易场所的初始状态产生竞争，并发现交易场所订单 id 查找表或持仓缓存为空。该门控会在每次连接尝试开始时清除任何上一会话的持仓和账户缓存，以便重连周期观察到的是新会话的帧，而非陈旧数据。相比之下，透明的 WebSocket 重连不会重新进入 `connect()`：它们会保留缓存的持仓，直到下一帧 `account_all_positions` 到达，然后应用相同的完整快照替换规则。

## API 凭证 (API credentials)

Lighter 签名需要全部三个凭证值：

- 账户索引 (Account index)：数值型的 Lighter 账户标识符。
- API key 索引 (API key index)：数值型的 API key 槽位，`0..=254`。索引 `0..=3` 保留给 Lighter 桌面/移动客户端。
- API 私钥 (API private key)：40 字节的十六进制私钥，可带或不带 `0x` 前缀。

配置值优先。当配置字段被省略时，适配器会根据所选环境读取环境变量。

| 环境      | API key 索引                      | API 私钥                       | 账户索引                            |
| ------- | ------------------------------- | ---------------------------- | ------------------------------- |
| Mainnet | `LIGHTER_API_KEY_INDEX`         | `LIGHTER_API_SECRET`         | `LIGHTER_ACCOUNT_INDEX`         |
| Testnet | `LIGHTER_TESTNET_API_KEY_INDEX` | `LIGHTER_TESTNET_API_SECRET` | `LIGHTER_TESTNET_ACCOUNT_INDEX` |

执行会拒绝不完整的凭证。数据客户端可以在没有凭证的情况下运行公开流和公开 REST 端点；诸如 `request_trades` 这类已鉴权的数据请求在三个值都可用时会使用相同的值。

## 配置 (Configuration)

### 数据客户端配置选项 (Data client configuration options)

| 选项                                 | 默认        | 描述                                         |
| ---------------------------------- | --------- | ------------------------------------------ |
| `base_url_http`                    | `None`    | 可选的 REST URL 覆盖。                           |
| `base_url_ws`                      | `None`    | 可选的 WebSocket URL 覆盖。                      |
| `proxy_url`                        | `None`    | 用于 HTTP 和 WebSocket 的可选代理 URL。             |
| `environment`                      | `Mainnet` | `LighterEnvironment::Mainnet` 或 `Testnet`。 |
| `account_index`                    | `None`    | 用于已鉴权 REST 数据的 Lighter 账户索引。               |
| `api_key_index`                    | `None`    | 用于已鉴权 REST 数据的 Lighter API key 槽位。         |
| `private_key`                      | `None`    | 用于 REST 鉴权 token 的十六进制私钥。                  |
| `http_timeout_secs`                | `60`      | HTTP 请求超时，单位秒。                             |
| `ws_timeout_secs`                  | `30`      | WebSocket 连接超时，单位秒。                        |
| `update_instruments_interval_mins` | `60`      | Instrument 元数据刷新间隔，单位分钟。                   |
| `transport_backend`                | Default   | WebSocket 传输后端。                            |

### 执行客户端配置选项 (Execution client configuration options)

| 选项                          | 默认        | 描述                                               |
| --------------------------- | --------- | ------------------------------------------------ |
| `trader_id`                 | 必填        | Nautilus trader 标识符。                             |
| `account_id`                | 必填        | 该交易场所的 Nautilus 账户标识符。                           |
| `account_index`             | `None`    | Lighter 账户索引。                                    |
| `api_key_index`             | `None`    | Lighter API key 槽位。                              |
| `private_key`               | `None`    | 用于鉴权和 L2 交易签名的十六进制私钥。                            |
| `base_url_http`             | `None`    | 可选的 REST URL 覆盖。                                 |
| `base_url_ws`               | `None`    | 可选的 WebSocket URL 覆盖。                            |
| `proxy_url`                 | `None`    | 用于 HTTP 和 WebSocket 的可选代理 URL。                   |
| `environment`               | `Mainnet` | `LighterEnvironment::Mainnet` 或 `Testnet`。       |
| `http_timeout_secs`         | `60`      | HTTP 请求超时，单位秒。                                   |
| `ws_timeout_secs`           | `30`      | WebSocket 连接超时，单位秒。                              |
| `active_markets`            | `[]`      | 无作用域对账期间要轮询的 Lighter market ID。                  |
| `market_order_slippage_bps` | `50`      | 用于 `MARKET` / `STOP_MARKET` / `MIT` 的滑点上限 (bps)。 |
| `transport_backend`         | Default   | WebSocket 传输后端。                                  |

### 配置示例 (Configuration example)

```rust
use nautilus_lighter::{
    common::enums::LighterEnvironment,
    config::{LighterDataClientConfig, LighterExecClientConfig},
};

let data_config = LighterDataClientConfig {
    environment: LighterEnvironment::Testnet,
    ..Default::default()
};

let exec_config = LighterExecClientConfig::builder()
    .trader_id(trader_id)
    .account_id(account_id)
    .environment(LighterEnvironment::Testnet)
    .active_markets(vec![0])
    .build();
```

上面的执行配置从匹配的 testnet 环境变量中解析凭证。直接设置 `account_index`、`api_key_index` 和 `private_key` 可覆盖环境变量查找。把 `active_markets` 设为那些应在冷启动对账期间检查挂单的交易场所 market ID。

## 官方文档 (Official documentation)

- 交易与签名：<https://apidocs.lighter.xyz/docs/trading>
- API keys：<https://apidocs.lighter.xyz/docs/api-keys>
- 速率限制：<https://apidocs.lighter.xyz/docs/rate-limits>
- 成交量配额：<https://apidocs.lighter.xyz/docs/volume-quota-program>
- 数据结构、常量和错误：<https://apidocs.lighter.xyz/docs/data-structures-constants-and-errors>
- REST OpenAPI：<https://raw.githubusercontent.com/elliottech/lighter-python/main/openapi.json>
- WebSocket 参考：<https://apidocs.lighter.xyz/docs/websocket-reference>

## 贡献 (Contributing)

:::info
如需更多特性或要为 Lighter 适配器做贡献，请参阅我们的
[贡献指南](https://github.com/nautechsystems/nautilus_trader/blob/develop/CONTRIBUTING.md)。
:::
