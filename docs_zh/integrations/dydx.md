# dYdX

dYdX 是最大的去中心化加密货币衍生品交易所之一。本集成（Integration）支持与 dYdX v4 的实时市场数据接入和订单执行（Execution），dYdX v4 运行在其自有的、基于 Cosmos SDK 的应用专用区块链（dYdX Chain）上，使用 CometBFT 共识。订单簿和撮合引擎作为验证者进程的一部分在链上运行。订单以 Cosmos 交易的形式通过 gRPC 提交，并在每个区块结算。一个 Indexer 服务通过 REST 和 WebSocket API 对外暴露市场数据和账户状态。

这是基于 Rust 实现、带有 Python 绑定的适配器。

## 安装

:::note
无需额外的安装扩展项。该适配器以 Rust 实现，并在构建过程中自动编译进核心 `nautilus_trader` 包。
:::

## 示例

您可以在[此处](https://github.com/nautechsystems/nautilus_trader/tree/develop/examples/live/dydx/)找到实时示例脚本。

## 概述

该适配器以 Rust 实现，并通过 PyO3 提供 Python 绑定。它直接集成了 dYdX 的 Indexer API（REST/WebSocket，用于市场数据）以及 gRPC（用于提交 Cosmos SDK 交易），无需依赖外部客户端库。

### 产品支持

| 产品类型                    | 数据推送 | 交易  | 备注                           |
| ----------------------- | ---- | --- | ---------------------------- |
| 永续期货（Perpetual Futures） | ✓    | ✓   | 所有永续合约均以 USDC 结算。            |
| 现货（Spot）                | -    | -   | dYdX 在 Solana 上提供现货；本适配器不支持。 |
| 期权（Options）             | -    | -   | *dYdX 上不提供*。                 |

:::note
该适配器仅支持永续期货。所有市场以 USD 计价，以 USDC 结算。
:::

## 链架构

与暴露单一 REST/WebSocket API 的中心化交易所（CEX）不同，dYdX v4 运行在其**自有的、基于 Cosmos SDK 的应用专用区块链**上。这意味着每一笔交易都是一笔需要经过共识的 Cosmos 交易，适配器必须管理序列号（sequence）、gas 以及基于区块高度的过期。

### 传输层

适配器通过三个相互独立的传输层进行通信：

```
                         ┌─────────────────────────────────────────────┐
                         │              dYdX v4 Chain                  │
                         │                                             │
 ┌──────────┐  HTTP      │   ┌──────────────────────┐                  │
 │          │───────────►│   │  Indexer (read-only) │                  │
 │          │  WebSocket │   │  - REST API          │                  │
 │ Nautilus │───────────►│   │  - Streaming API     │                  │
 │ Adapter  │            │   └──────────────────────┘                  │
 │          │  gRPC      │   ┌──────────────────────┐                  │
 │          │───────────►│   │  Validator (write)   │                  │
 └──────────┘            │   │  - Cosmos Tx submit  │                  │
                         │   │  - Sequence mgmt     │                  │
                         │   └──────────────────────┘                  │
                         └─────────────────────────────────────────────┘
```

| 传输层       | 目标        | 方向  | 用途                 |
| --------- | --------- | --- | ------------------ |
| HTTP      | Indexer   | 只读  | 金融工具元数据、历史数据、账户状态。 |
| WebSocket | Indexer   | 只读  | 实时市场数据、订单/成交/持仓更新。 |
| gRPC      | Validator | 写入  | 下单、撤单及批量操作。        |

### 基于区块的结算

dYdX 区块大约每 **~0.5 秒**产生一个（实际时间会有波动）。适配器内置一个 `BlockTimeMonitor`，它跟踪从 WebSocket 推送中观测到的区块时间，以动态估算 `seconds_per_block`。该估算值用于将基于时间的订单过期转换为短期订单所需的区块高度偏移量。

## 架构

dYdX v4 适配器包含多个组件，它们既可以组合使用，也可以单独使用：

- `DydxHttpClient`：基于 Rust 的 HTTP 客户端，用于查询 Indexer REST API。
- `DydxWebSocketClient`：基于 Rust 的 WebSocket 客户端，用于接收实时市场数据和账户更新。
- `DydxGrpcClient`：基于 Rust 的 gRPC 客户端，用于提交 Cosmos SDK 交易。
- `DydxInstrumentProvider`：金融工具（Instrument）解析和加载功能。
- `DydxDataClient`：市场数据推送管理器。
- `DydxExecutionClient`：账户管理和交易执行网关。
- `DydxLiveDataClientFactory`：dYdX v4 数据客户端工厂（由交易节点构建器使用）。
- `DydxLiveExecClientFactory`：dYdX v4 执行客户端工厂（由交易节点构建器使用）。

:::note
大多数用户只需为实时交易节点定义配置（如下所示），无需直接使用这些底层组件。
:::

:::warning[首次账户激活]
dYdX v4 交易账户（子账户 0）仅在钱包首次入金或交易后才会创建。在此之前，每个 gRPC/Indexer 查询都会返回 `NOT_FOUND`，导致 `DydxExecutionClient.connect()` 失败。

在启动实时 `TradingNode` 之前，从同一钱包在同一网络（主网/测试网）上发送任意正数金额的 USDC 或其他支持的抵押品。交易最终确认后（几个区块后），重启节点，客户端即可正常连接。
:::

## 故障排除

### `StatusCode.NOT_FOUND` 未找到账户

**原因：** 钱包/子账户从未充值，因此在链上尚不存在。

**解决方法：**

1. 在正确的网络上向子账户 0 存入任意正数金额的 USDC。
2. 等待最终确认（主网约 30 秒，测试网更长）。
3. 重启 `TradingNode`；连接现在应该成功。

:::tip
在无人值守的部署中，将 `connect()` 调用包裹在指数退避循环中，以便客户端在入金到账前持续重试。
:::

## 符号体系

dYdX 对永续期货合约使用特定的符号约定。

### 符号格式

格式：`{Base}-USD-PERP`

dYdX 上的所有永续合约都：

- 以 USD 计价
- 以 USDC 结算
- 在 Nautilus 中使用 `.DYDX` 交易场所（venue）后缀

示例：

- `BTC-USD-PERP.DYDX` - 比特币永续期货
- `ETH-USD-PERP.DYDX` - 以太坊永续期货
- `SOL-USD-PERP.DYDX` - Solana 永续期货

在策略中订阅：

```python
InstrumentId.from_str("BTC-USD-PERP.DYDX")
InstrumentId.from_str("ETH-USD-PERP.DYDX")
```

:::info
添加 `-PERP` 后缀是为了与其他适配器保持一致并为未来做好准备。虽然 dYdX 目前仅支持永续合约，但这种命名约定为将来可能扩展到其他产品类型留出了空间。
:::

## 订单功能

dYdX 支持永续期货交易，提供完整的订单类型和执行功能。Rust 适配器会根据有效时间（time-in-force）和过期时间自动将订单分类为短期或长期订单，因此无需手动打标签。

### 订单类型

| 订单类型                   | 永续合约 | 备注                          |
| ---------------------- | ---- | --------------------------- |
| `MARKET`               | ✓    | 以当前最优可用价格立即执行。              |
| `LIMIT`                | ✓    |                             |
| `STOP_MARKET`          | ✓    | 止损（Stop-loss）条件订单，始终为长期订单。  |
| `STOP_LIMIT`           | ✓    | 条件订单，始终为长期订单。               |
| `MARKET_IF_TOUCHED`    | ✓    | 止盈（Take-profit）市价单，触及价格时触发。 |
| `LIMIT_IF_TOUCHED`     | ✓    | 止盈限价单，触及价格时触发。              |
| `TRAILING_STOP_MARKET` | -    | *不支持*。                      |

### 执行指令

| 指令            | 永续合约 | 备注                                                                                                |
| ------------- | ---- | ------------------------------------------------------------------------------------------------- |
| `post_only`   | ✓    | 在 LIMIT、STOP_LIMIT 和 LIMIT_IF_TOUCHED 订单上受支持。一个定价会穿越价差的 post-only 订单会被交易场所**先接受再立即取消**（而不是带原因拒绝）。 |
| `reduce_only` | ✓    | 对所有订单类型传递。dYdX 将其作为**成交时的钳制（fill-time clamp）**而非下单时的前置条件来执行：一个针对无持仓的 reduce-only 订单仍会正常成交。        |

### 有效时间选项

| 有效时间  | 永续合约 | 备注                                                                                                          |
| ----- | ---- | ----------------------------------------------------------------------------------------------------------- |
| `GTC` | ✓    | 撤销前有效（Good Till Canceled）。                                                                                  |
| `GTD` | ✓    | 到期前有效（Good Till Date）。交易场所将过期上报为一个取消事件；当订单的 `expire_time` 已过时，适配器会将其映射为 `OrderExpired`（而非 `OrderCanceled`）。 |
| `IOC` | ✓    | 立即成交或撤销（Immediate or Cancel）。                                                                               |
| `FOK` | -    | *已被 dYdX v4 弃用*。链会以 `code=48` 拒绝 FOK 订单；适配器在本地生成 `OrderDenied` 而不广播。                                        |
| `DAY` | -    | *不支持*。适配器在本地生成 `OrderDenied` 而不广播。                                                                          |

### 高级订单功能

| 功能        | 永续合约 | 备注                                                                                                                                              |
| --------- | ---- | ----------------------------------------------------------------------------------------------------------------------------------------------- |
| 订单修改      | -    | 不支持。dYdX 支持短期订单的[替换（replacement）](https://docs.dydx.xyz/concepts/trading/limit-orderbook#replacements)（相同 ID、更高的 GTB）；目前尚未以 `ModifyOrder` 形式暴露。 |
| 组合/OCO 订单 | -    | *不支持*。                                                                                                                                          |
| 冰山订单      | -    | *不支持*。                                                                                                                                          |

### 批量操作

| 操作   | 永续合约 | 备注                                                                   |
| ---- | ---- | -------------------------------------------------------------------- |
| 批量提交 | ✓    | 支持长期 `LIMIT` 订单。短期订单逐个单独提交。                                          |
| 批量修改 | -    | *不支持*。                                                               |
| 批量取消 | ✓    | 分区处理：短期订单使用 `MsgBatchCancel`（单次 gRPC 调用），长期订单使用批量的 `MsgCancelOrder`。 |

### 持仓管理

| 功能    | 永续合约 | 备注           |
| ----- | ---- | ------------ |
| 查询持仓  | ✓    | 实时持仓更新。      |
| 持仓模式  | -    | 仅支持净持仓（见下文）。 |
| 杠杆控制  | ✓    | 按市场的杠杆设置。    |
| 保证金模式 | -    | 仅支持全仓保证金。    |

:::note
dYdX 在交易场所层面支持净持仓（netting，每个金融工具一个持仓）。适配器目前仅以 `NETTING` 模式运行。对冲（hedging）支持计划在未来版本中提供。
:::

### 订单查询

| 功能     | 永续合约 | 备注        |
| ------ | ---- | --------- |
| 查询未结订单 | ✓    | 列出所有活动订单。 |
| 查询历史订单 | ✓    | 历史订单数据。   |
| 订单状态更新 | ✓    | 实时订单状态变更。 |
| 交易历史   | ✓    | 执行与成交报告。  |

### 关联订单（Contingent orders）

| 功能     | 永续合约 | 备注              |
| ------ | ---- | --------------- |
| 订单列表   | -    | *不支持*。          |
| OCO 订单 | -    | *不支持*。          |
| 组合订单   | -    | *不支持*。          |
| 条件订单   | ✓    | 止损、止盈市价单和止盈限价单。 |

### 权益层级限制（Equity tier limit）

dYdX 根据账户的权益层级（equity tier），将每个子账户的**同时未结条件订单**数量限制在一个固定值（例如标准层级为 10 个条件订单）。提交超出上限的额外条件订单会在链上以 `code=10001` 被拒绝，并伴有形如 `Opening order would exceed equity tier limit of N` 的日志消息。在下更多订单前先取消已有的条件订单，或将策略分散到多个子账户中。

### MIT 与 LIT 的往返还原（round-tripping）

dYdX 协议使用单一的 `TAKE_PROFIT` 订单类型，带有一个价格（`subticks`）和一个触发价格；它表现为触发后市价（market-on-trigger）还是触发后限价（limit-on-trigger）隐含在价格中。适配器将 Nautilus 的 `MARKET_IF_TOUCHED` 作为止盈提交，价格设为 5% 穿透（pay-through）的最坏情况价；将 `LIMIT_IF_TOUCHED` 作为止盈提交，价格设为用户的限价。两种形式都被 Indexer 返回为 `"type":"TAKE_PROFIT"`。在对账（reconciliation）时，适配器将解析出的限价与配置的穿透容差进行比较，以还原原始的 Nautilus 订单类型。如果价格在预言机（oracle）的穿透带（pay-through band）以内，则该订单对账为 `MARKET_IF_TOUCHED`；否则对账为 `LIMIT_IF_TOUCHED`。

### 清算与 ADL（自动减仓）处理

dYdX v4 应用两个先后执行的风险机制：

1. **清算（Liquidation）** 在账户跌破其维持保证金时触发。持仓在距预言机价格一个有界价差内对保险基金（insurance fund）平仓。
2. **自动减仓（Deleveraging，ADL）** 在以下情形之一触发：清算无法完全恢复抵押充足性，或一次大幅的预言机价格跳变使账户在单步内变为负值。自动减仓会将抵押不足的持仓对随机选定的反向账户平仓。

Indexer 通过每条 `Fill` 记录上的 `type` 字段（`DydxFillType`）暴露这一分类：

| `type`        | 含义                      |
| ------------- | ----------------------- |
| `LIMIT`       | 正常成交。                   |
| `LIQUIDATED`  | 清算的吃单方（taker，抵押不足）。     |
| `LIQUIDATION` | 清算的挂单方（maker，保险基金）。     |
| `DELEVERAGED` | 自动减仓的吃单方（taker，ADL 平仓）。 |
| `OFFSETTING`  | 自动减仓的挂单方（maker，反向账户）。   |

对于每一笔清算/自动减仓成交，适配器都会记录一条包含金融工具、方向、数量和价格的警告日志，然后通过正常路径发出 `FillReport`。`DydxPerpetualPositionStatus::Liquidated` 会平掉对应的持仓报告。

上游参考：

- [清算（Liquidations）](https://docs.dydx.xyz/concepts/trading/liquidations)
- [合约亏损机制（自动减仓）](https://help.dydx.trade/en/articles/166973-contract-loss-mechanisms-on-dydx-chain)

### 订单分类

dYdX 将每个订单归入三种链上类别之一。Rust 适配器会根据有效时间和过期时间自动判定类别，因此无需手动配置。

| 类别              | 下单位置 | 过期方式     | 典型用途                                     |
| --------------- | ---- | -------- | ---------------------------------------- |
| 短期（Short-term）  | 内存中  | 区块高度     | IOC/FOK，或在 40 个区块内过期的订单。                 |
| 长期（Long-term）   | 链上   | 时间戳（UTC） | 过期时间超出短期窗口（约 20 秒，按 ~0.5 秒/区块）的 GTC/GTD。 |
| 条件（Conditional） | 链上   | 时间戳（UTC） | 止损和止盈触发器。                                |

在协议层面，**所有 dYdX 订单都是限价单**。`MARKET` 订单类型是 Nautilus 提供的便利封装，适配器将其实现为一个定价远穿订单簿的激进 IOC 限价单。这意味着市价单遵循与限价单相同的 `Submitted > Accepted > Filled` 生命周期（在成交之前预期会有一个 `OrderAccepted` 事件）。

有关短期订单与有状态订单机制的完整协议层细节，请参阅 [dYdX 订单文档](https://docs.dydx.xyz/concepts/trading/orders)。

#### 短期订单

短期订单**仅存活于验证者内存中**，按区块高度过期（最多 40 个区块，按 ~0.5 秒/区块约为 ~20 秒）。它们是 dYdX 上最快的订单类型，因为它们跳过了链上存储。

**特性**：

- **IOC 和 FOK 始终为短期订单**，无论其他参数如何
- **GTD 订单**在其过期时间落在动态短期窗口（`40 个区块 × seconds_per_block`）以内时，会被自动归类为短期订单
- 使用 Good-Til-Block（GTB）而非 Cosmos SDK 序列号来做重放保护（replay protection）
- 可以**并发**广播（无信号量，使用缓存的序列号）
- 静默过期，不产生取消事件
- 不能在单笔交易中批量提交（每笔交易一个 `MsgPlaceOrder`）

#### 长期订单

长期（有状态）订单**存储在链上**，按 UTC 时间戳过期。它们在过期或被取消时会产生显式的取消事件。

**特性**：

- **GTC** 订单默认 90 天过期（协议上限为 95 天）
- **GTD** 订单使用用户提供的过期时间戳
- 需要正确的 Cosmos SDK 序列号管理（通过信号量串行化）
- 必须以递增的序列号**串行**广播
- 可以在单笔交易中批量提交

#### 条件订单

条件订单（止损、止盈）**始终存储在链上**，由验证者根据价格条件触发。

**特性**：

- 始终使用基于时间戳的过期（GTC 默认 90 天，协议上限 95 天）
- 始终使用长期广播路径（通过信号量串行化）
- 包括 `StopMarket`、`StopLimit`、`TakeProfitMarket` 和 `TakeProfitLimit`

#### 自动路由

适配器使用 `BlockTimeMonitor` 自动确定订单的存活时长：

```
max_short_term_secs = SHORT_TERM_ORDER_MAXIMUM_LIFETIME (40) × seconds_per_block
```

如果订单距过期的时间在 `max_short_term_secs` 以内，则按短期订单路由。否则按长期订单路由。无需任何手动配置。

#### MARKET 订单的实现

dYdX 没有原生的市价单类型。适配器将 `MARKET` 订单实现为激进的 **IOC 限价单**，定价为：

- **买入**：`oracle_price × (1 + 0.05)`（高于预言机价 5%）
- **卖出**：`oracle_price × (1 - 0.05)`（低于预言机价 5%）

这个 5% 的滑点缓冲（`DEFAULT_MARKET_ORDER_SLIPPAGE = 0.05`）设定了最坏情况价格（即"穿透价格 / pay-through price"）。由于订单为 IOC，未成交部分的滑点不会被消耗。该缓冲被有意设得较宽，以在剧烈波动条件下最大化成交概率。

### 客户端订单 ID 编码

dYdX 在链上要求 `u32` 的客户端 ID，但 Nautilus 使用基于字符串的 `ClientOrderId` 值（例如 `O-20260220-031943-001-000-51`）。适配器对其进行双向编码，使订单能够在多次重启之间对账，而无需持久化状态。

对于标准的 O 格式（`O-YYYYMMDD-HHMMSS-TTT-SSS-CCC`），其编码是确定性的：

| dYdX 字段           | 位数  | 内容                                         |
| ----------------- | --- | ------------------------------------------ |
| `client_id`       | 32  | `[trader:10][strategy:10][count:12]`（唯一键）。 |
| `client_metadata` | 32  | 自 2020-01-01 UTC 起的秒数（时间戳）。                |

由于编码是确定性的，适配器无需数据库或映射文件，即可将任何已对账的订单解码回其原始的 `ClientOrderId` 字符串。

非标准的 `ClientOrderId` 格式（自定义字符串、纯数字）会回退到带有内存反向映射的顺序分配。这些 ID 只能在同一会话内解码。

#### 重启碰撞防护

重启时，Nautilus 会根据已对账订单的数量重置内部订单计数器，这个数量可能低于上一会话中使用过的最高计数值（例如，如果某些订单已从 API 响应中过期）。这可能导致新订单产生与上一会话订单相同的 `client_id`，从而造成交易场所订单 UUID 重复。

适配器通过登记对账期间见到的每一个 `client_id` 来防止这种情况。如果一个新的 O 格式编码产生了一个已被使用过的 `client_id`，编码器会记录一条警告并回退到顺序分配。顺序分配也会跳过任何已登记的值。

:::note
此防护是自动的，无需用户配置。警告日志 `[ENCODER] client_id ... collides with reconciled order` 仅为信息提示。订单仍会以一个替代 ID 成功提交。
:::

## 广播与重试策略

### 短期广播

短期订单使用 Good-Til-Block（GTB）做重放保护。链的 `ClobDecorator` ante handler 会跳过对短期消息的 Cosmos SDK 序列号检查，因此：

- **无信号量**：广播完全并发
- **缓存的序列号**：无需递增或分配
- **不重试**：如果广播失败，立即失败
- 良性取消错误被视为成功（见下文）

### 长期广播

长期订单和条件订单需要正确的 Cosmos SDK 序列号管理：

- **带 1 个许可的信号量**将所有长期广播串行化
- **指数退避**：500ms -> 1s -> 2s -> 4s（最多 5 次重试）
- **10 秒的总预算**防止无限重试循环
- 在序列号不匹配时，会先**从链上重新同步序列号**再重试

### 序列号不匹配检测

| 错误码        | 来源                 | 含义             |
| ---------- | ------------------ | -------------- |
| `code=32`  | Cosmos SDK         | 账户序列号不匹配       |
| `code=104` | dYdX authenticator | 签名验证失败（与序列号相关） |

两者都会通过 `RetryManager` 触发自动重新同步 + 重试。

### 良性取消错误

短期取消操作期间出现的以下错误被视为**成功**：

| 错误码         | 含义                               |
| ----------- | -------------------------------- |
| `code=19`   | 交易已在 mempool 缓存中（重复交易）           |
| `code=9`    | memclob 中已存在 GoodTilBlock >= 的取消 |
| `code=3006` | 待取消的订单不存在（已成交/已过期/已取消）           |

### 批量取消的分区

在取消多个订单时，适配器按存活时长对它们进行分区：

1. **短期订单**：通过 `broadcast_short_term()` 发送单个 `MsgBatchCancel`
2. **长期订单**：通过 `broadcast_with_retry()` 发送批量的 `MsgCancelOrder` 消息

这确保每一组都使用合适的广播策略。

## 资金费率（Funding rates）

dYdX 永续期货使用固定的 1 小时资金费率结算周期。对于 WebSocket 和历史资金费率数据，适配器都会在所有 `FundingRateUpdate` 对象上将 `interval` 设为 `60`（分钟）。

## 速率限制

### gRPC 速率限制

适配器对 gRPC 的 `broadcast_tx` 调用进行速率限制，以防止验证者节点返回 `ResourceExhausted`（429）错误。

| 设置                           | 默认值 | 描述                             |
| ---------------------------- | --- | ------------------------------ |
| `grpc_rate_limit_per_second` | `4` | 每秒最大 gRPC 广播请求数。设为 `None` 可禁用。 |

### 提供商限制

公共 gRPC 提供商的已知速率限制：

| 提供商       | 限制               | 备注  |
| --------- | ---------------- | --- |
| Polkachu  | 300 次/分钟（~5/秒）   |     |
| KingNodes | 250 次/分钟（~4.2/秒） |     |
| AutoStake | 4 次/秒            |     |

默认值 4 次/秒较为保守，可在所有公共提供商上正常工作。

### 多 gRPC URL 回退

使用 `base_url_grpc` 覆盖主 gRPC 端点：

```python
exec_config = DydxExecClientConfig(
    base_url_grpc="https://primary-grpc.example.com:443",
    # ...
)
```

当 `base_url_grpc` 未设置时，适配器会为所选网络使用默认公共节点，并内置跨公共验证者列表的回退能力。目前 Python 配置上尚未暴露通过用户配置进行的显式多 URL 回退。

## 价格与数量量化（Quantization）

dYdX 对价格和数量使用基于整数的量化。适配器通过 `OrderMessageBuilder` 自动处理所有转换，但理解这些参数有助于调试。

### 市场参数

| 参数                            | 描述                       |
| ----------------------------- | ------------------------ |
| `atomic_resolution`           | 将人类可读的数量转换为 quantums 的指数 |
| `quantum_conversion_exponent` | 将 quantums 转换为代币的指数      |
| `step_base_quantums`          | 以 quantums 表示的最小订单数量步长   |
| `subticks_per_tick`           | 每个 tick 内的价格粒度           |

### 市价单定价

市价单使用预言机价格加上 5% 的滑点缓冲（即"穿透价格 / pay-through price"）：

- **买入**：`oracle_price × 1.05`
- **卖出**：`oracle_price × 0.95`

预言机价格从 Indexer 缓存而来，并定期刷新。

### 自动处理

所有价格和数量的量化都由 `OrderMessageBuilder` 自动处理。通过 Nautilus 提交订单时无需手动转换。

## 数据订阅

v4 适配器支持以下数据订阅：

| 数据类型                        | 订阅  | 历史请求 | 备注                         |
| --------------------------- | --- | ---- | -------------------------- |
| 成交 tick（Trade ticks）        | ✓   | ✓    |                            |
| 报价 tick（Quote ticks）        | ✓   | -    | 由订单簿最优价格（top-of-book）合成。   |
| 订单簿增量（Order book deltas）    | ✓   | -    | 仅 L2 深度。                   |
| 订单簿快照（Order book snapshots） | -   | ✓    | 通过 HTTP 请求获取一次性快照。         |
| K 线（Bars）                   | ✓   | ✓    | 见下文支持的分辨率。                 |
| 标记价格（Mark prices）           | ✓   | -    | 通过 markets 频道。             |
| 指数价格（Index prices）          | ✓   | -    | 通过 markets 频道。             |
| 资金费率（Funding rates）         | ✓   | ✓    | 实时通过 markets 频道，历史通过 HTTP。 |
| 金融工具状态（Instrument status）   | ✓   | -    | 通过 markets 频道。             |

### 支持的 K 线分辨率

| 分辨率       | dYdX K 线 |
| --------- | -------- |
| 1-MINUTE  | `1MIN`   |
| 5-MINUTE  | `5MINS`  |
| 15-MINUTE | `15MINS` |
| 30-MINUTE | `30MINS` |
| 1-HOUR    | `1HOUR`  |
| 4-HOUR    | `4HOURS` |
| 1-DAY     | `1DAY`   |

## 子账户（Subaccounts）

dYdX 支持每个钱包地址拥有多个子账户，从而能在单个钱包内隔离交易策略与风险管理。

### 关键概念

- 每个钱包地址可以拥有多个带编号的子账户（0、1、2、……、127）。
- 子账户 0 是**默认**子账户，会在首次入金时自动创建。
- 每个子账户都维护各自的：
  - 持仓
  - 未结订单
  - 抵押品余额
  - 保证金要求

### 配置

在执行客户端配置中指定子账户编号：

```python
config = TradingNodeConfig(
    exec_clients={
        "DYDX": DydxExecClientConfig(
            subaccount=0,  # 默认子账户
        ),
    },
)
```

:::note
大多数用户会使用子账户 `0`（默认）。高级用户可以为不同的子账户配置多个执行客户端，以实现策略隔离或风险隔离。
:::

## 测试网设置

dYdX 测试网（`dydx-testnet-4`）是主网的完整副本，可用于在不冒真实资金风险的情况下测试策略。当设置 `environment=DydxNetwork.TESTNET` 时，所有默认的测试网端点都会被自动解析。

### 1. 创建测试网钱包

**方式 A：通过 dYdX 测试网网页应用（最简单）**

1. 访问 [v4.testnet.dydx.exchange](https://v4.testnet.dydx.exchange)
2. 使用 MetaMask、Keplr、Phantom 或 WalletConnect 连接
3. 系统会自动生成一个 dYdX 账户
4. 导出您的助记词：点击您的地址（右上角），选择 "Export secret phrase"

**方式 B：使用已有的 secp256k1 私钥**

任何 32 字节、十六进制编码的 secp256k1 私钥都可以使用。适配器会使用 Cosmos bech32 编码自动从该密钥派生出 `dydx1...` 地址。

### 2. 为测试网账户充值

在适配器能够连接之前，子账户必须先充值（见[首次账户激活](#architecture)）。

**通过测试网网页应用：**

在 [v4.testnet.dydx.exchange](https://v4.testnet.dydx.exchange) 点击充值/入金按钮，即可自动收到测试网 USDC。

**直接通过水龙头（faucet）API：**

```bash
# 为子账户 0 充值 2000 USDC
curl -X POST https://faucet.v4testnet.dydx.exchange/faucet/tokens \
  -H "Content-Type: application/json" \
  -d '{"address": "dydx1...", "subaccountNumber": 0, "amount": 2000}'

# 充值原生代币（用于支付 gas 费用）
curl -X POST https://faucet.v4testnet.dydx.exchange/faucet/native-token \
  -H "Content-Type: application/json" \
  -d '{"address": "dydx1..."}'
```

### 3. 设置环境变量

```bash
export DYDX_TESTNET_WALLET_ADDRESS="dydx1..."
export DYDX_TESTNET_PRIVATE_KEY="0x..."  # 十六进制编码，0x 前缀可选
```

### 4. 配置交易节点

在数据客户端和执行客户端上都设置 `environment=DydxNetwork.TESTNET`：

```python
from nautilus_trader.adapters.dydx import DydxNetwork

config = TradingNodeConfig(
    ...,  # 省略
    data_clients={
        DYDX: DydxDataClientConfig(
            wallet_address=None,  # 回退到 DYDX_TESTNET_WALLET_ADDRESS 环境变量
            instrument_provider=InstrumentProviderConfig(load_all=True),
            environment=DydxNetwork.TESTNET,
        ),
    },
    exec_clients={
        DYDX: DydxExecClientConfig(
            wallet_address=None,  # 回退到 DYDX_TESTNET_WALLET_ADDRESS 环境变量
            private_key=None,  # 回退到 DYDX_TESTNET_PRIVATE_KEY 环境变量
            subaccount=0,
            instrument_provider=InstrumentProviderConfig(load_all=True),
            environment=DydxNetwork.TESTNET,
        ),
    },
)
```

### 测试网端点

默认测试网端点会被自动使用。如有需要，可在各自的配置上通过 `base_url_http`、`base_url_ws` 或 `base_url_grpc`（仅执行）进行覆盖。

| 服务        | 默认 URL                                        |
| --------- | --------------------------------------------- |
| HTTP      | `https://indexer.v4testnet.dydx.exchange`     |
| WebSocket | `wss://indexer.v4testnet.dydx.exchange/v4/ws` |
| gRPC      | `https://test-dydx-grpc.kingnodes.com:443`（主） |
| Faucet    | `https://faucet.v4testnet.dydx.exchange`      |
| 网页应用      | `https://v4.testnet.dydx.exchange`            |

### 主网端点

默认主网端点会被自动使用。如有需要，可在各自的配置上通过 `base_url_http`、`base_url_ws` 或 `base_url_grpc`（仅执行）进行覆盖。

| 服务        | 默认 URL                                       |
| --------- | -------------------------------------------- |
| HTTP      | `https://indexer.dydx.trade`                 |
| WebSocket | `wss://indexer.dydx.trade/v4/ws`             |
| gRPC      | `https://dydx-ops-grpc.kingnodes.com:443`（主） |

## 配置

通过交易节点配置来配置 dYdX 适配器。执行客户端支持对凭证使用环境变量回退。数据客户端使用公共端点，不需要钱包凭证。

### 数据客户端配置选项

| 选项                        | 默认值       | 描述                                                      |
| ------------------------- | --------- | ------------------------------------------------------- |
| `wallet_address`          | `None`    | 旧版 Python 配置字段。公共数据客户端不使用钱包凭证。                          |
| `environment`             | `None`    | `DydxNetwork.MAINNET` 或 `DydxNetwork.TESTNET`。          |
| `bars_timestamp_on_close` | `True`    | K 线的 `ts_event` 是否应为 K 线收盘时间。设为 `False` 可使用交易场所原生的开盘时间。 |
| `base_url_http`           | `None`    | HTTP API 端点覆盖。`None` 表示为所选网络选用默认值。                      |
| `base_url_ws`             | `None`    | WebSocket 端点覆盖。`None` 表示为所选网络选用默认值。                     |
| `proxy_url`               | `None`    | HTTP 和 WebSocket 传输的可选代理 URL。                           |
| `max_retries`             | `3`       | REST/WebSocket 恢复的最大重试次数。                               |
| `retry_delay_initial_ms`  | `1,000`   | 重试间的初始延迟（毫秒）。                                           |
| `retry_delay_max_ms`      | `10,000`  | 重试间的最大延迟（毫秒）。                                           |
| `transport_backend`       | `Sockudo` | WebSocket 传输后端。                                         |

### 执行客户端配置选项

| 选项                           | 默认值       | 描述                                                                        |
| ---------------------------- | --------- | ------------------------------------------------------------------------- |
| `wallet_address`             | `None`    | dYdX 钱包地址。回退到 `DYDX_WALLET_ADDRESS` / `DYDX_TESTNET_WALLET_ADDRESS` 环境变量。 |
| `subaccount`                 | `0`       | 子账户编号（0-127）。子账户 0 为默认。                                                   |
| `private_key`                | `None`    | 用于签名的十六进制编码私钥。回退到 `DYDX_PRIVATE_KEY` / `DYDX_TESTNET_PRIVATE_KEY`。        |
| `authenticator_ids`          | `None`    | 用于授权密钥交易（机构配置）的 authenticator ID 列表。                                      |
| `environment`                | `None`    | `DydxNetwork.MAINNET` 或 `DydxNetwork.TESTNET`。                            |
| `base_url_http`              | `None`    | HTTP 客户端自定义端点覆盖。`None` 表示为所选网络选用默认值。                                      |
| `base_url_ws`                | `None`    | WebSocket 客户端自定义端点覆盖。`None` 表示为所选网络选用默认值。                                 |
| `base_url_grpc`              | `None`    | gRPC 客户端自定义端点覆盖。`None` 表示为所选网络选用默认值。                                      |
| `proxy_url`                  | `None`    | HTTP 和 WebSocket 传输的可选代理 URL。                                             |
| `max_retries`                | `3`       | 提交/取消/修改订单操作的最大重试次数。                                                      |
| `retry_delay_initial_ms`     | `1,000`   | 重试间的初始延迟（毫秒）。                                                             |
| `retry_delay_max_ms`         | `10,000`  | 重试间的最大延迟（毫秒）。                                                             |
| `grpc_rate_limit_per_second` | `4`       | 每秒最大 gRPC 请求数。设为 `None` 可禁用。                                              |
| `transport_backend`          | `Sockudo` | WebSocket 传输后端。                                                           |

### 基本设置

配置一个实时 `TradingNode` 以包含 dYdX 数据和执行客户端：

```python
from nautilus_trader.adapters.dydx import DydxDataClientConfig
from nautilus_trader.adapters.dydx import DydxExecClientConfig
from nautilus_trader.adapters.dydx import DydxNetwork
from nautilus_trader.adapters.dydx.constants import DYDX
from nautilus_trader.config import InstrumentProviderConfig
from nautilus_trader.config import TradingNodeConfig

config = TradingNodeConfig(
    ...,  # 省略
    data_clients={
        DYDX: DydxDataClientConfig(
            wallet_address=None,  # 回退到环境变量
            instrument_provider=InstrumentProviderConfig(load_all=True),
            environment=DydxNetwork.MAINNET,
        ),
    },
    exec_clients={
        DYDX: DydxExecClientConfig(
            wallet_address=None,  # 回退到环境变量
            private_key=None,  # 回退到环境变量
            subaccount=0,
            instrument_provider=InstrumentProviderConfig(load_all=True),
            environment=DydxNetwork.MAINNET,
        ),
    },
)
```

然后，创建一个 `TradingNode` 并注册客户端工厂：

```python
from nautilus_trader.adapters.dydx import DydxLiveDataClientFactory
from nautilus_trader.adapters.dydx import DydxLiveExecClientFactory
from nautilus_trader.adapters.dydx.constants import DYDX
from nautilus_trader.live.node import TradingNode

node = TradingNode(config=config)

node.add_data_client_factory(DYDX, DydxLiveDataClientFactory)
node.add_exec_client_factory(DYDX, DydxLiveExecClientFactory)

node.build()
```

### API 凭证

凭证既可以通过 Python 配置（`wallet_address`、`private_key`）直接传入，也可以根据所配置的 `environment` 从环境变量中自动解析。

#### 环境变量

| 变量                            | 网络  | 描述                          |
| ----------------------------- | --- | --------------------------- |
| `DYDX_WALLET_ADDRESS`         | 主网  | Bech32 编码的钱包地址（`dydx1...`）。 |
| `DYDX_PRIVATE_KEY`            | 主网  | 用于签名的十六进制编码 secp256k1 私钥。   |
| `DYDX_TESTNET_WALLET_ADDRESS` | 测试网 | 测试网钱包地址（`dydx1...`）。        |
| `DYDX_TESTNET_PRIVATE_KEY`    | 测试网 | 测试网私钥。                      |

#### 解析优先级

1. 在 Python 配置中传入的值（如果非空）
2. 由 `environment` 选定的环境变量

### 授权密钥交易（Permissioned key trading）

#### 什么是 API 交易密钥（API Trading Keys）

API 交易密钥让您能够在不共享主钱包助记词的前提下，将交易委托给一个独立的签名密钥。该 API 密钥可以使用所有者全仓保证金账户中所有可用的保证金进行下单，但无法提取资金或转移资产。

#### 创建 API 密钥

1. 在 dYdX 网页应用中，导航至 **More > API Trading Keys**
2. 点击 **Generate New API Key**
3. 保存 **API Wallet Address** 和 **Private Key**（仅显示一次，dYdX 不会存储）
4. 点击 **Authorize API Key**（这会将该密钥作为 authenticator 在链上注册）
5. 密钥现已激活，可用于交易

有关创建和管理 API 密钥的完整细节，请参阅 [dYdX API 交易密钥指南](https://docs.dydx.xyz/concepts/trading/api-trading-keys)。

#### 适配器配置

为使用 API 交易密钥而配置适配器有两种方式：

**自动解析（推荐）：** 将 API 密钥的私钥设为 `DYDX_PRIVATE_KEY`，将所有者的钱包地址设为 `DYDX_WALLET_ADDRESS`。适配器会在连接期间检测到二者不匹配，并自动向链查询匹配的 authenticator ID，无需手动配置 ID。

```python
config = DydxExecClientConfig(
    wallet_address="dydx1owner...",  # 所有者账户（持有保证金）
    private_key="0xapikey...",  # API 交易密钥私钥
    # authenticator_ids 自动解析
)
```

**手动覆盖：** 如果您已知 authenticator ID（例如来自 dYdX 的 TypeScript 客户端），可直接传入以跳过自动解析：

```python
config = DydxExecClientConfig(
    wallet_address="dydx1owner...",
    private_key="0xapikey...",
    authenticator_ids=[1, 2],  # 跳过自动解析
)
```

:::note
API 交易密钥仅适用于**全仓保证金（cross-margin）**账户和全仓市场。不支持逐仓保证金（isolated margin）。
:::

## 订单簿

订单簿可以根据订阅维护全深度或最优价格（top-of-book）报价。交易场所不直接提供报价。相反，适配器订阅订单簿增量更新，并在最优价格或数量发生变化时为 `DataEngine` 合成报价。仅支持 L2（MBP）订单簿类型。

## 贡献

:::info
如需额外功能或为 dYdX 适配器做贡献，请参阅我们的
[贡献指南](https://github.com/nautechsystems/nautilus_trader/blob/develop/CONTRIBUTING.md)。
:::
