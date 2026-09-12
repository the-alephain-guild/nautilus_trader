# BitMEX

BitMEX (Bitcoin Mercantile Exchange) 成立于 2014 年，是一个加密货币衍生品（Derivatives）
交易平台，提供现货（Spot）、永续合约（Perpetual Contracts）、传统期货（Futures）、预测市场
（Prediction Markets）及其他高级交易产品。本集成（Integration）支持 BitMEX 的实时市场数据
接入和订单执行（Order Execution）。

## 概览

本适配器（Adapter）使用 Rust 实现，并提供可选的 Python 绑定，方便在基于 Python 的工作流中使用。
它不依赖外部 BitMEX 客户端库——核心组件编译为静态库，并在构建过程中自动链接。

## 示例

你可以在[这里](https://github.com/nautechsystems/nautilus_trader/tree/develop/examples/live/bitmex/)找到实时示例脚本。

## 组件

本指南假设交易者正在配置实时市场数据馈送和交易执行。
BitMEX 适配器包含多个组件，可以根据使用场景一起使用或单独使用。

- `BitmexHttpClient`：底层 HTTP API 连接。
- `BitmexWebSocketClient`：底层 WebSocket API 连接。
- `BitmexInstrumentProvider`：金融工具（Instrument）解析和加载功能。
- `BitmexDataClient`：市场数据馈送管理器。
- `BitmexExecutionClient`：账户管理和交易执行网关。
- `BitmexLiveDataClientFactory`：BitMEX 数据客户端工厂（由交易节点构建器使用）。
- `BitmexLiveExecClientFactory`：BitMEX 执行客户端工厂（由交易节点构建器使用）。

:::note
大多数用户只需为实时交易节点定义配置（如下所示），
不需要直接使用这些底层组件。
:::

## BitMEX 文档

BitMEX 为用户提供了全面的文档：

- [BitMEX API Explorer](https://www.bitmex.com/app/restAPI) - 交互式 API 文档。
- [BitMEX API Documentation](https://www.bitmex.com/app/apiOverview) - 完整的 API 参考。
- [BitMEX Exchange Rules](https://www.bitmex.com/exchange-rules) - 官方交易所规则和法规。
- [Contract Guides](https://www.bitmex.com/app/contract) - 详细的合约规格。
- [Spot Trading Guide](https://www.bitmex.com/app/spotGuide) - 现货交易概览。
- [Perpetual Contracts Guide](https://www.bitmex.com/app/perpetualContractsGuide) - 永续合约说明。
- [Futures Contracts Guide](https://www.bitmex.com/app/futuresGuide) - 传统期货信息。

建议你在使用本 NautilusTrader 集成指南时，同时参考 BitMEX 文档。

## 产品支持

| 产品类型      | 数据馈送 | 交易  | 备注                          |
| --------- | ---- | --- | --------------------------- |
| 现货        | ✓    | ✓   | 交易对有限，与衍生品共享统一钱包。           |
| 永续合约      | ✓    | ✓   | 提供反向和线性合约。                  |
| 股票永续合约    | -    | -   | *尚未支持*。目前仅在测试网（Testnet）上可用。 |
| 期货        | ✓    | ✓   | 传统固定到期合约。                   |
| Quanto 期货 | ✓    | ✓   | 以与标的资产不同的货币结算。              |
| 预测市场      | ✓    | ✓   | 基于事件的合约，0-100 定价，USDT 结算。   |
| 期权        | -    | -   | *BitMEX 未提供*。               |

:::note
BitMEX 已停止其期权（Options）产品，以专注于核心衍生品和现货业务。
:::

### 现货交易

- 直接的代币/币种交易，即时结算。
- 主要交易对包括 XBT/USDT、ETH/USDT、ETH/XBT。
- 额外的山寨币交易对（LINK、SOL、UNI、APE、AXS、BMEX 对 USDT）。

### 衍生品

- **永续合约**：反向合约（如 XBTUSD）和线性合约（如 ETHUSDT）。
- **传统期货**：固定到期日合约。
- **Quanto 期货**：以与标的资产不同的货币结算的合约。
- **预测市场**：基于事件的衍生品（如 P_FTXZ26、P_SBFJAILZ26），允许交易者对加密货币、金融和其他事件的结果进行投机。无杠杆（Leverage），定价 0-100，USDT 结算。
- **股票永续合约**：基于权益的永续合约（如 SPYUSDT、CRCLUSDT）。*目前仅在测试网上可用；本适配器尚未支持。*

### 金融工具类型代码 (CFI)

BitMEX 使用遵循 ISO 10962 标准的 CFI（金融工具分类，Classification of Financial Instruments）代码。
适配器可识别以下金融工具类型代码：

| 代码       | 类型     | 状态  | 描述                      |
| -------- | ------ | --- | ----------------------- |
| `FFWCSX` | 永续合约   | 已支持 | 基于加密货币的永续合约（如 XBTUSD）。  |
| `FFWCSF` | 永续外汇   | 已支持 | 基于外汇的永续合约。              |
| `FFCCSX` | 期货     | 已支持 | 固定到期的日历期货。              |
| `FFICSX` | 预测市场   | 已支持 | 基于事件的预测合约。              |
| `IFXXXP` | 现货     | 已支持 | 现货交易对。                  |
| `FFSCSX` | 股票永续合约 | 不支持 | 基于股票/权益的永续合约。仅限测试网。     |
| `SRMCSX` | 互换利率   | 不支持 | 基于收益率的互换产品（历史产品）。       |
| `MR****` | 指数     | 参考  | BitMEX 指数（不可交易，用于价格参考）。 |

详情请参阅 [BitMEX Typ Values](https://support.bitmex.com/hc/en-gb/articles/6299296145565-What-are-the-Typ-Values-for-Instrument-endpoint)。

## 代码命名规则

BitMEX 对其交易代码使用特定的命名约定。理解这一约定对于正确识别和交易金融工具至关重要。

### 代码格式

BitMEX 代码通常遵循以下模式：

- **现货交易对**：基础货币 + 计价货币（如 `XBT/USDT`、`ETH/USDT`）。
- **永续合约**：基础货币 + 计价货币（如 `XBTUSD`、`ETHUSD`）。
- **期货合约**：基础货币 + 到期代码（如 `XBTM24`、`ETHH25`）。
- **Quanto 合约**：非 USD 结算合约的特殊命名。
- **预测市场**：`P_` 前缀 + 事件标识符 + 到期代码（如 `P_POWELLK26`、`P_FTXZ26`）。

:::info
BitMEX 使用 `XBT` 作为比特币的代码而非 `BTC`。这遵循了 ISO 4217
货币代码标准，其中 "X" 表示非主权货币。XBT 和 BTC 指的是同一资产——比特币。
:::

### 到期代码

期货合约使用标准的期货月份代码：

- `F` = 一月
- `G` = 二月
- `H` = 三月
- `J` = 四月
- `K` = 五月
- `M` = 六月
- `N` = 七月
- `Q` = 八月
- `U` = 九月
- `V` = 十月
- `X` = 十一月
- `Z` = 十二月

后跟年份（如 `24` 表示 2024 年，`25` 表示 2025 年）。

### NautilusTrader 金融工具 ID

在 NautilusTrader 中，BitMEX 金融工具使用原生的 BitMEX 代码直接标识，
并结合交易场所（Venue）标识符：

```python
from nautilus_trader.model.identifiers import InstrumentId

# 现货交易对（注意：代码中不包含斜杠）
spot_id = InstrumentId.from_str("XBTUSDT.BITMEX")  # XBT/USDT 现货
eth_spot_id = InstrumentId.from_str("ETHUSDT.BITMEX")  # ETH/USDT 现货

# 永续合约
perp_id = InstrumentId.from_str("XBTUSD.BITMEX")  # 比特币永续合约（反向）
linear_perp_id = InstrumentId.from_str("ETHUSDT.BITMEX")  # 以太坊永续合约（线性）

# 期货合约（2024年6月）
futures_id = InstrumentId.from_str("XBTM24.BITMEX")  # 比特币期货，2024年6月到期

# 预测市场合约
prediction_id = InstrumentId.from_str(
    "P_XBTETFV23.BITMEX"
)  # 比特币 ETF SEC 审批预测，2023年10月到期
```

:::note
NautilusTrader 中的 BitMEX 现货代码不包含 BitMEX UI 中显示的斜杠 (/)。
请使用 `XBTUSDT` 而非 `XBT/USDT`。
:::

### 数量缩放

BitMEX 以*合约*单位报告现货和衍生品的数量。每个合约的实际资产大小是
交易所特定的，并在金融工具定义中公布：

- `lotSize` —— 可交易的最小合约数量。
- `underlyingToPositionMultiplier` —— 每单位标的资产对应的合约数量。

例如，SOL/USDT 现货金融工具 (`SOLUSDT`) 公布了 `lotSize = 1000` 和
`underlyingToPositionMultiplier = 10000`，这意味着一个合约代表 `1 / 10000 = 0.0001`
SOL，最小订单量（`lotSize * contract_size`）为 `0.1` SOL。适配器现在直接从
这些字段推导合约大小，并相应地缩放入站市场数据和出站订单，因此 Nautilus 中的
数量始终以基础单位（SOL、ETH 等）表示。

有关这些字段的详细信息，请参阅 BitMEX API 文档：<https://www.bitmex.com/app/apiOverview#Instrument-Properties>。

## 订单能力

BitMEX 集成支持以下订单类型和执行功能。

### 订单类型

| 订单类型                   | 支持  | 备注                                |
| ---------------------- | --- | --------------------------------- |
| `MARKET`               | ✓   | 以当前市场价格立即执行。不支持报价数量。              |
| `LIMIT`                | ✓   | 仅以指定价格或更优价格执行。                    |
| `STOP_MARKET`          | ✓   | 支持（设置 `trigger_price`）。           |
| `STOP_LIMIT`           | ✓   | 支持（设置 `price` 和 `trigger_price`）。 |
| `MARKET_IF_TOUCHED`    | ✓   | 支持（设置 `trigger_price`）。           |
| `LIMIT_IF_TOUCHED`     | ✓   | 支持（设置 `price` 和 `trigger_price`）。 |
| `TRAILING_STOP_MARKET` | ✓   | 支持（设置 `trailing_offset`）。仅价格偏移类型。 |

### 执行指令

| 指令            | 支持  | 备注                                                 |
| ------------- | --- | -------------------------------------------------- |
| `post_only`   | ✓   | 通过 `LIMIT` 订单上的 `ParticipateDoNotInitiate` 执行指令支持。 |
| `reduce_only` | ✓   | 通过 `ReduceOnly` 执行指令支持。                            |

:::note
Post-only 订单如果会穿越价差（Spread），BitMEX 会取消该订单而非拒绝。
集成将这些情况表示为 `due_post_only=True` 的拒绝，以便策略能一致地处理。
:::

### 触发类型

BitMEX 支持多种参考价格来评估以下订单的止损/条件触发：

- `STOP_MARKET`
- `STOP_LIMIT`
- `MARKET_IF_TOUCHED`
- `LIMIT_IF_TOUCHED`

选择与你的策略和/或风险偏好匹配的触发类型。

| 参考价格  | Nautilus `TriggerType` | BitMEX 值     | 备注                          |
| ----- | ---------------------- | ------------ | --------------------------- |
| 最新成交价 | `LAST_PRICE`           | `LastPrice`  | BitMEX 默认值；基于最新成交价触发。       |
| 标记价格  | `MARK_PRICE`           | `MarkPrice`  | 推荐用于许多止损场景，以减少因价格尖峰导致的止损触发。 |
| 指数价格  | `INDEX_PRICE`          | `IndexPrice` | 跟踪外部指数；对某些合约有用。             |

- 如果未提供 `trigger_type`，BitMEX 将使用其场所默认值（`LastPrice`）。
- 这些触发参考由交易所评估；订单在被触发之前保持挂单状态。

**示例**：

```python
from nautilus_trader.model.enums import TriggerType

order = self.order_factory.stop_market(
    instrument_id=instrument_id,
    order_side=order_side,
    quantity=qty,
    trigger_price=trigger,
    trigger_type=TriggerType.MARK_PRICE,  # 使用 BitMEX 标记价格作为参考
)
```

`ExecTester` 示例配置也演示了在 `examples/live/bitmex/bitmex_exec_tester.py` 中设置 `stop_trigger_type=TriggerType.MARK_PRICE`。

### 追踪止损

BitMEX 支持追踪止损订单，当市场朝有利方向移动时自动调整止损价格。适配器将
`TRAILING_STOP_MARKET` 订单映射到 BitMEX 的挂钩订单（Pegged Orders），使用
`TrailingStopPeg` 价格类型。

**限制：**

- 仅支持 `PRICE` 追踪偏移类型（绝对价格偏移，而非基点或 tick）。
- 偏移符号自动处理：卖出止损使用负偏移，买入止损使用正偏移。
- 触发类型可与追踪止损组合使用以实现额外控制。

**示例**：

```python
from nautilus_trader.model.enums import TrailingOffsetType

order = self.order_factory.trailing_stop_market(
    instrument_id=instrument_id,
    order_side=OrderSide.SELL,
    quantity=qty,
    trailing_offset=Decimal("100"),  # $100 追踪偏移
    trailing_offset_type=TrailingOffsetType.PRICE,
    trigger_type=TriggerType.LAST_PRICE,  # 可选
)
```

:::note
BitMEX 随着市场移动定期更新追踪止损价格。
当市场朝触发水平移动时，止损价格会冻结。
有关当前更新节奏的详情，请参阅 [BitMEX API 文档](https://www.bitmex.com/app/perpetualContractsGuide)。
:::

### 挂钩订单

BitMEX 支持自动跟踪参考价格的挂钩订单（Pegged Orders，BBO）。适配器通过
`submit_order` 上的 `params` 字典支持挂钩订单，这会在交易所端将订单类型
覆盖为 `Pegged`。

| 挂钩价格类型        | 描述                         |
| ------------- | -------------------------- |
| `PrimaryPeg`  | 挂钩到最优买价（买入）或最优卖价（卖出）。      |
| `MarketPeg`   | 挂钩到对手方（买入挂钩最优卖价，卖出挂钩最优买价）。 |
| `MidPricePeg` | 挂钩到买卖价之间的中间价。              |
| `LastPeg`     | 挂钩到最新成交价。                  |

**要求**：

- 底层订单必须是 `LIMIT` 订单。其他订单类型将被拒绝。
- `peg_price_type` 是必需的；`peg_offset_value` 是可选的（默认为 0）。
- `peg_offset_value` 可以为负值（如卖方偏移）或小数。

**示例**：

```python
# 挂钩到最优买价，零偏移（BBO）
order = self.order_factory.limit(
    instrument_id=instrument_id,
    order_side=OrderSide.BUY,
    quantity=qty,
    price=price,  # LIMIT 订单必需，但会被挂钩覆盖
)
self.submit_order(order, params={"peg_price_type": "PrimaryPeg", "peg_offset_value": "0"})

# 挂钩到中间价，偏移 -0.5
self.submit_order(order, params={"peg_price_type": "MidPricePeg", "peg_offset_value": "-0.5"})
```

:::note
构造 `LimitOrder` 时仍然需要 `price` 字段，但 BitMEX 对挂钩订单会忽略它，
而是持续跟踪参考价格加偏移。
:::

### 有效期

| 有效期   | 支持  | 备注                                      |
| ----- | --- | --------------------------------------- |
| `GTC` | ✓   | 撤销前有效（Good Till Canceled，默认）。           |
| `GTD` | -   | *BitMEX 不支持*。                           |
| `FOK` | ✓   | 全部成交否则取消（Fill or Kill）—— 完全成交或取消。       |
| `IOC` | ✓   | 立即成交否则取消（Immediate or Cancel）—— 允许部分成交。 |
| `DAY` | ✓   | 在 UTC 00:00 到期（BitMEX 交易日边界）。           |

:::note
`DAY` 订单在 UTC 时间 12:00am 到期，这标志着 BitMEX 交易日的边界（当日交易时段结束）。
详情请参阅 [BitMEX Exchange Rules](https://www.bitmex.com/exchange-rules) 和 [API 文档](https://www.bitmex.com/api/explorer/)。
:::

### 高级订单功能

| 功能    | 支持  | 备注                                               |
| ----- | --- | ------------------------------------------------ |
| 订单修改  | ✓   | 修改价格、数量和触发价格。                                    |
| 条件单组合 | ✓   | 使用 `contingency_type` 和 `linked_order_ids`。      |
| 冰山订单  | ✓   | 使用 `display_qty`。                                |
| 追踪止损  | ✓   | 使用 `trailing_offset`。仅价格偏移类型。                    |
| 挂钩订单  | ✓   | 使用带 `peg_price_type` 的 `params`。参见[挂钩订单](#挂钩订单)。 |

### 批量操作

| 操作   | 支持  | 备注            |
| ---- | --- | ------------- |
| 批量提交 | -   | *BitMEX 不支持*。 |
| 批量修改 | -   | *BitMEX 不支持*。 |
| 批量取消 | ✓   | 单次请求取消多个订单。   |

### 持仓管理

| 功能    | 支持  | 备注                          |
| ----- | --- | --------------------------- |
| 查询持仓  | ✓   | REST 和通过 WebSocket 的实时持仓更新。 |
| 全仓保证金 | ✓   | 默认保证金模式。                    |
| 逐仓保证金 | ✓   |                             |

### 订单查询

| 功能      | 支持  | 备注                       |
| ------- | --- | ------------------------ |
| 查询未完成订单 | ✓   | 列出所有活跃订单。                |
| 查询订单历史  | ✓   | 历史订单数据。                  |
| 订单状态更新  | ✓   | 通过 WebSocket 实时推送订单状态变化。 |
| 交易历史    | ✓   | 执行和成交报告。                 |

### 强平和 ADL 处理

BitMEX 通过 `execution` 频道上的 `execType` 字段呈现强制平仓的成交：

| `execType`    | 含义                                           |
| ------------- | -------------------------------------------- |
| `Trade`       | 正常执行（用户或 taker 发起）。                          |
| `Liquidation` | 持仓被强平引擎强制平仓。BitMEX 对自动减仓（ADL）和对手方强平成交都使用此代码。 |
| `Bankruptcy`  | 账户破产；持仓针对保险基金平仓。                             |
| `Settlement`  | 计划内的合约结算。                                    |
| `Funding`     | 对未平仓持仓的资金费结算。                                |

适配器将 `Liquidation` 和 `Bankruptcy` 通过标准的 `FillReport` 路径路由，
并在破产执行时记录警告。BitMEX 的公开 API **不**在 `execType` 中区分
自动减仓和对手方强平；两者都显示为 `Liquidation`。ADL 平仓的持仓通常可以
通过零佣金以及本地缓存中缺少对应订单来识别（引擎会为其创建一个外部订单）。

上游参考：

- [`/execution` 字段定义](https://support.bitmex.com/hc/en-gb/articles/6205689858077--execution-field-definitions)
- [自动减仓概览](https://support.bitmex.com/hc/en-gb/articles/18589621443357-What-is-Auto-Deleveraging)
- [强平概览](https://support.bitmex.com/hc/en-gb/articles/360003188434-Liquidations)

## 市场数据

- 订单簿增量：仅 `L2_MBP`；`depth` 为 0（完整订单簿）或 25。
- 订单簿 depth10 快照：通过 `orderBook10` 频道提供固定的 10 档。
- 通过 WebSocket 支持报价、成交和金融工具更新。
- 在适用的情况下支持资金费率、标记价格和指数价格。
- 通过 REST 进行历史请求：
  - 成交 Tick 数据，支持可选的 `start`、`end` 和 `limit` 过滤器（每次调用最多 1,000 条结果）。
  - K 线（`1m`、`5m`、`1h`、`1d`），用于外部聚合的 LAST 价格，包括可选的部分 K 线。

:::note
BitMEX 每次 REST 响应上限为 1,000 行，需要通过 `start`/`startTime` 手动分页。当前适配器仅返回第一页；更广泛的分页支持计划在未来更新中提供。
:::

### 成交 ID 推导

成交 Tick 和成交使用场所提供的 `trdMatchID`（UUID）作为 `TradeId`。当场所
省略 `trdMatchID` 时（分桶成交或某些执行类型），执行路径回退到场所的
`execID`；市场数据解析器回退到对代码、`ts_event`、价格、数量和方向的
确定性 FNV-1a 哈希。同一场所事件在重放（Replay）中产生相同的成交 ID，
保持下游去重完整。

## 连接管理

### HTTP Keep-Alive

BitMEX 适配器使用 HTTP keep-alive 以获得最佳性能：

- **连接池**：连接自动池化和复用。
- **Keep-alive 超时**：90 秒（与 BitMEX 服务器端超时匹配）。
- **自动重连**：失败的连接自动重新建立。
- **SSL 会话缓存**：减少后续请求的握手开销。

此配置通过维护持久连接并避免为每个请求建立新连接的开销，确保与 BitMEX 服务器的低延迟通信。

### 请求认证和过期

BitMEX 使用 `api-expires` 头部进行请求认证以防止重放攻击：

- 签名请求包含一个 `api-expires` Unix 时间戳，设置为当前时间往后 `recv_window_ms / 1000` 秒（默认 10 秒）。
- 一旦该时间戳过期，BitMEX 将拒绝任何请求，因此请将延迟保持在配置的窗口内。

## 资金费率

适配器从 [Funding](https://www.bitmex.com/app/wsAPI#Funding)
WebSocket 流接收资金费率数据。BitMEX 在每条消息中返回一个 `fundingInterval`
日期时间字段，适配器读取其小时和分钟来计算 `FundingRateUpdate` 上的 `interval` 字段。

## 限流

BitMEX 实施双层限流（Rate Limiting）系统：

### REST 限制

- **突发限制**：认证用户每秒 10 个请求（适用于下单、修改和取消端点）。
- **滚动分钟限制**：认证用户每分钟 120 个请求（未认证用户每分钟 30 个请求）。
- **订单上限**：每个交易对 200 个未完成订单和 10 个止损单；超过这些上限将触发交易所端拒绝。

适配器使用配置的 `max_requests_per_second` 和 `max_requests_per_minute` 值在本地执行这些配额。

### WebSocket 限制

- 连接请求：遵循交易所指导（目前每个 IP 每秒 3 个连接）。
- 私有流需要认证；如果超过限制，适配器会自动重连。

:::warning
超过 BitMEX 限流限制会返回 HTTP 429，并可能触发临时 IP 封禁；持续的 4xx/5xx 错误可能延长封锁期。
:::

### 可配置限流

如果你的账户拥有与默认值不同的限制，可以配置限流参数：

| 参数                        | 默认值（认证） | 默认值（未认证） | 描述              |
| ------------------------- | ------- | -------- | --------------- |
| `max_requests_per_second` | 10      | 10       | 每秒最大请求数（突发限制）。  |
| `max_requests_per_minute` | 120     | 30       | 每分钟最大请求数（滚动窗口）。 |

:::info
有关限流的更多详情，请参阅 [BitMEX API 限流文档](https://www.bitmex.com/app/restAPI#Limits)。
:::

:::warning
**取消广播器限流注意事项**

取消广播器（当 `canceller_pool_size > 1` 时）将每个取消请求并行分发到多个独立的 HTTP 客户端。每个客户端维护自己的限流器，这意味着有效请求速率会乘以池大小。

**示例**：当 `canceller_pool_size=3` 且 `max_requests_per_second=10` 时，单次取消操作消耗 **3 个请求**（每个客户端一个），如果快速取消，可能达到 **每秒 30 个请求**。

由于 BitMEX 在**账户级别**（而非每个连接）执行限流，广播器可能会导致你超过交易所默认的每秒 10 个请求突发和每分钟 120 个请求滚动窗口限制。

**缓解措施**：按比例降低 `max_requests_per_second` 和 `max_requests_per_minute`（除以 `canceller_pool_size`），或调整池大小本身（参见[取消广播器配置](#取消广播器)）。
未来版本可能支持池内共享限流器。
:::

### 限流头部

BitMEX 通过响应头部公开当前配额：

- `x-ratelimit-limit`：当前窗口内允许的总请求数。
- `x-ratelimit-remaining`：触发限流前的剩余请求数。
- `x-ratelimit-reset`：配额重置的 UNIX 时间戳。
- `retry-after`：收到 429 响应后需等待的秒数。

## 提交广播器

BitMEX 执行客户端包含一个提交广播器（Submit Broadcaster），通过并行请求分发提供更高的市场单和限价单被目标价格接受的保证，以较低的最小延迟换取重复提交的风险。

### 概念

订单提交是时间关键型操作——当策略决定建仓时，任何延迟都可能导致错失机会或不利定价。提交广播器通过以下方式解决这个问题：

- **并行分发**：提交请求同时广播到多个独立的 HTTP 客户端实例。
- **首次成功短路**：第一个成功响应获胜，最小化到接受确认的延迟。
- **共享 client_order_id**：所有传输使用相同的 `client_order_id`。BitMEX 以 "duplicate clOrdID" 拒绝重复提交（被跟踪为预期拒绝）。
- **延迟与重复的权衡**：接受潜在重复成交的风险（如果多个传输在拒绝之前成功），换取更低的最小延迟和更高的接受保证。

这种架构通过跨多个网络路径并行化来降低订单接受的最小延迟。

### 用法

提交广播器是可选启用的，通过提交订单时的 `submit_tries` 参数控制。默认情况下，订单通过单个 HTTP 客户端提交。要启用广播：

```python
# 单次提交（默认行为）
self.submit_order(order)

# 广播到 2 个并行 HTTP 客户端以实现冗余
self.submit_order(order, params={"submit_tries": 2})

# 广播到 3 个并行 HTTP 客户端（推荐最大值）
self.submit_order(order, params={"submit_tries": 3})
```

**要点**：

- `submit_tries` 必须是正整数。
- 仅当 `submit_tries > 1` 时才会进行广播。默认提交通过单个 HTTP 客户端进行。
- 如果 `submit_tries` 超过 `submitter_pool_size`，将被限制为池大小并发出警告。
- 所有传输使用相同的 `client_order_id`；BitMEX 将重复提交作为预期拒绝处理。

### 健康监控

广播器池中的每个 HTTP 客户端维护健康指标：

- 成功提交将客户端标记为健康。
- 失败请求增加错误计数器。
- 后台健康检查定期验证客户端连接。
- 降级的客户端被跟踪但保留在池中以维持容错性。

广播器公开的指标包括总提交数、成功提交数、失败提交数和预期拒绝数，用于运营监控和调试。

#### 跟踪指标

| 指标                   | 类型      | 描述                                       |
| -------------------- | ------- | ---------------------------------------- |
| `total_submits`      | `u64`   | 发起的提交操作总数。                               |
| `successful_submits` | `u64`   | 成功收到 BitMEX 确认的提交操作数。                    |
| `failed_submits`     | `u64`   | 池中所有 HTTP 客户端均失败（无健康客户端或所有请求失败）的提交操作数。   |
| `expected_rejects`   | `u64`   | 检测到的预期拒绝模式数（如并行提交导致的重复 clOrdID）。         |
| `healthy_clients`    | `usize` | 池中当前健康的 HTTP 客户端数量（通过最近健康检查的客户端）。        |
| `total_clients`      | `usize` | 池中配置的 HTTP 客户端总数（`submitter_pool_size`）。 |

这些指标可以通过 `SubmitBroadcaster` 实例上的 `get_metrics()` 方法以编程方式访问。

### 配置

提交广播器通过执行客户端配置进行设置：

| 选项                     | 默认值    | 描述                                             |
| ---------------------- | ------ | ---------------------------------------------- |
| `submitter_pool_size`  | `None` | HTTP 客户端池大小。`None` 解析为 1（单个客户端，无冗余）。           |
| `submitter_proxy_urls` | `None` | 可选的代理 URL 列表，用于提交广播器的路径多样性。*尚未通过 Python 集成接入。* |

**配置示例**：

```python
from nautilus_trader.adapters.bitmex.config import BitmexExecClientConfig

exec_config = BitmexExecClientConfig(
    api_key="YOUR_API_KEY",
    api_secret="YOUR_API_SECRET",
    submitter_pool_size=3,  # 推荐的冗余池大小
)
```

:::tip
对于没有更高限流额度的高频交易策略，请权衡使用提交广播器的优势与可能触及限流限制的风险，因为每个客户端有独立的限流预算。
默认 `submitter_pool_size=None` 会禁用广播器。推荐设置 `submitter_pool_size=3` 将每个提交请求广播到 3 个并行 HTTP 客户端以实现容错，这会消耗每次提交操作 3 倍的限流配额，但在网络或交易所问题面前提供更高的保证。
:::

广播器在执行客户端连接时自动启动，断开时自动停止。仅当 `submit_tries > 1` 时，提交操作才通过广播器路由；默认提交直接使用单个 HTTP 客户端。

## 取消广播器

BitMEX 执行客户端包含一个取消广播器（Cancel Broadcaster），通过并行请求分发提供容错的订单取消。

### 概念

订单取消是时间关键型操作——当策略决定取消订单时，任何延迟或失败都可能导致意外成交、滑点或不希望的持仓暴露。取消广播器通过以下方式解决这个问题：

- **并行分发**：取消请求同时广播到多个独立的 HTTP 客户端实例。
- **首次成功短路**：第一个成功响应获胜，剩余的进行中请求立即中止。
- **容错性**：如果一个 HTTP 客户端遇到网络问题、DNS 故障或连接超时，池中的其他客户端继续处理。
- **幂等成功处理**：表示订单已被取消的响应（如 "orderID not found" 或类似的幂等状态）被视为成功而非失败，防止不必要的错误传播。

这种架构确保单个网络路径故障或慢连接不会阻塞关键的取消操作，提高实时交易中风险管理和持仓控制的可靠性。

### 健康监控

广播器池中的每个 HTTP 客户端维护健康指标：

- 成功取消将客户端标记为健康。
- 失败请求增加错误计数器。
- 后台健康检查定期验证客户端连接。
- 降级的客户端被跟踪但保留在池中以维持容错性。

广播器公开的指标包括总取消数、成功取消数、失败取消数、预期拒绝数（已取消的订单）和幂等成功数，用于运营监控和调试。

#### 跟踪指标

| 指标                     | 类型      | 描述                                       |
| ---------------------- | ------- | ---------------------------------------- |
| `total_cancels`        | `u64`   | 发起的取消操作总数（包括单个、批量和全部取消请求）。               |
| `successful_cancels`   | `u64`   | 成功收到 BitMEX 确认的取消操作数。                    |
| `failed_cancels`       | `u64`   | 池中所有 HTTP 客户端均失败（无健康客户端或所有请求失败）的取消操作数。   |
| `expected_rejects`     | `u64`   | 检测到的预期拒绝模式数（如 post-only 订单拒绝）。           |
| `idempotent_successes` | `u64`   | 幂等成功响应数（订单已取消、订单未找到、因状态无法取消）。            |
| `healthy_clients`      | `usize` | 池中当前健康的 HTTP 客户端数量（通过最近健康检查的客户端）。        |
| `total_clients`        | `usize` | 池中配置的 HTTP 客户端总数（`canceller_pool_size`）。 |

这些指标可以通过 `CancelBroadcaster` 实例上的 `get_metrics()` 方法以编程方式访问。

### 配置

取消广播器通过执行客户端配置进行设置：

| 选项                     | 默认值    | 描述                                             |
| ---------------------- | ------ | ---------------------------------------------- |
| `canceller_pool_size`  | `None` | HTTP 客户端池大小。`None` 解析为 1（单个客户端，无冗余）。           |
| `canceller_proxy_urls` | `None` | 可选的代理 URL 列表，用于取消广播器的路径多样性。*尚未通过 Python 集成接入。* |

**配置示例**：

```python
from nautilus_trader.adapters.bitmex.config import BitmexExecClientConfig

exec_config = BitmexExecClientConfig(
    api_key="YOUR_API_KEY",
    api_secret="YOUR_API_SECRET",
    canceller_pool_size=3,  # 推荐的冗余池大小
)
```

:::tip
对于没有更高限流额度的高频交易策略，请权衡使用取消广播器的优势与可能触及限流限制的风险，因为每个客户端有独立的限流预算。
默认 `canceller_pool_size=None` 会禁用广播器。推荐设置 `canceller_pool_size=3` 将每个取消请求广播到 3 个并行 HTTP 客户端以实现容错，这会消耗每次取消操作 3 倍的限流配额，但在网络或交易所问题面前提供更高的保证。
:::

广播器在执行客户端连接时自动启动，断开时自动停止。所有取消操作（`cancel_order`、`cancel_all_orders`、`batch_cancel_orders`）自动通过广播器路由，无需对策略代码进行任何更改。

## 死人开关

适配器支持 BitMEX 的[死人开关](https://www.bitmex.com/app/restAPI#OrdercancelAllAfter)
（`cancelAllAfter`），它提供自动订单取消作为防范连接故障的安全网。

### 工作原理

启用后，会在 BitMEX 上设置一个服务器端计时器。如果计时器到期而未被刷新，
BitMEX 将取消该账户上的**所有**未完成订单。适配器通过发送周期性的心跳请求
保持计时器存活。如果适配器失去连接（网络故障、进程崩溃等），心跳停止，
BitMEX 将在配置的超时后取消订单。

流程：

1. 在**连接**时，适配器使用配置的超时（毫秒）调用
   `POST /api/v1/order/cancelAllAfter` 来武装服务器端计时器。
2. 后台任务以 `timeout / 4`（最少 1 秒）的**刷新间隔**发送相同的请求，
   以在计时器到期前不断重置它。
3. 在**断开**时，适配器等待后台心跳任务完全关闭，然后以 `timeout=0`
   调用 `cancelAllAfter` 来**解除**服务器端计时器的武装。

例如，在 60 秒超时的情况下，适配器每 15 秒发送一次心跳。
如果连续四次心跳失败（60 秒失去连接），BitMEX 将取消所有未完成订单。

### 断开顺序

在断开期间解除死人开关的武装需要仔细的顺序控制。解除武装的请求
（`timeout=0`）应该是最后一个到达 BitMEX 的 `cancelAllAfter` 调用。如果在解除
武装之后处理了一个进行中的心跳，它将重新武装服务器端计时器，订单可能会在超时
到期后被意外取消，即使适配器已经优雅地断开连接。

适配器在两种实现中都缓解了这个问题：

- **Rust**：心跳任务立即停止（abort + await），这样断开连接就不会因等待
  sleep 或 HTTP 超时而停滞。然后在任务退出后发送解除武装的请求。
- **Python**：心跳任务被取消并 await，确保协程在发送解除武装请求之前
  完全展开。

在强制停止的场景中（如通过 `stop()` 关闭进程），心跳任务被中止而不解除武装。
这是有意为之的，因为当进程意外退出时，服务器端计时器提供了所期望的安全行为。

:::note
每次心跳消耗一个 REST 限流令牌。60 秒的超时大约从 120/分钟的预算中
使用每分钟 4 个请求。
:::

### 配置

通过在执行客户端配置上设置 `deadmans_switch_timeout_secs` 来启用死人开关：

```python
from nautilus_trader.adapters.bitmex.config import BitmexExecClientConfig

exec_config = BitmexExecClientConfig(
    api_key="YOUR_API_KEY",
    api_secret="YOUR_API_SECRET",
    deadmans_switch_timeout_secs=60,  # 失去连接 60 秒后取消所有订单
)
```

启用后，适配器在连接时记录：

```
Starting dead man's switch: timeout=60s, refresh_interval=15s
```

并在断开时记录：

```
Disarming dead man's switch
```

:::tip
**60 秒**的超时是推荐的起点。较短的超时提供更快的保护，但对短暂的网络抖动
更敏感。较长的超时对短暂中断更宽容，但在真正发生故障时会使订单暴露更长时间。
:::

:::warning
死人开关适用于账户上的**所有**未完成订单，而不仅仅是适配器下的订单。
如果其他系统在同一账户上下单，启用死人开关也会影响那些订单。
:::

## 配置

### API 凭证

BitMEX API 凭证可以直接在配置中提供，也可以通过环境变量提供：

- `BITMEX_API_KEY`：用于生产环境的 BitMEX API 密钥（API Key）。
- `BITMEX_API_SECRET`：用于生产环境的 BitMEX API 密钥密文（API Secret）。
- `BITMEX_TESTNET_API_KEY`：用于测试网的 BitMEX API 密钥（API Key）。
- `BITMEX_TESTNET_API_SECRET`：用于测试网的 BitMEX API 密钥密文（API Secret）。

生成 API 密钥的步骤：

1. 登录你的 BitMEX 账户。
2. 导航到 Account & Security -> API Keys。
3. 创建具有适当权限的新 API 密钥。
4. 对于测试网，使用 [testnet.bitmex.com](https://testnet.bitmex.com)。

:::note
**测试网 API 端点**：

- REST API：`https://testnet.bitmex.com/api/v1`
- WebSocket：`wss://ws.testnet.bitmex.com/realtime`

当配置了 `environment=BitmexEnvironment.TESTNET` 时，适配器会自动将请求路由到正确的端点。
:::

### 数据客户端配置选项

BitMEX 数据客户端提供以下配置选项：

| 选项                                 | 默认值       | 描述                                               |
| ---------------------------------- | --------- | ------------------------------------------------ |
| `api_key`                          | `None`    | 可选的 API 密钥；如果为 `None`，从 `environment` 所选的环境加载。   |
| `api_secret`                       | `None`    | 可选的 API 密钥密文；如果为 `None`，从 `environment` 所选的环境加载。 |
| `environment`                      | `None`    | 环境枚举（`MAINNET` 或 `TESTNET`）。                     |
| `base_url_http`                    | `None`    | REST 基础 URL 覆盖（默认为生产环境）。                         |
| `base_url_ws`                      | `None`    | WebSocket 基础 URL 覆盖（默认为生产环境）。                    |
| `http_timeout_secs`                | `60`      | 应用于 HTTP 调用的请求超时。                                |
| `max_retries`                      | `3`       | HTTP 调用的最大重试次数。                                  |
| `retry_delay_initial_ms`           | `1,000`   | 重试之间的初始退避延迟（毫秒）。                                 |
| `retry_delay_max_ms`               | `10,000`  | 重试之间的最大退避延迟（毫秒）。                                 |
| `recv_window_ms`                   | `10,000`  | 签名请求的过期窗口（毫秒）。参见[请求认证](#请求认证和过期)。                |
| `update_instruments_interval_mins` | `60`      | 金融工具目录刷新间隔（分钟）。                                  |
| `max_requests_per_second`          | `10`      | 适配器对 REST 调用执行的突发限流。                             |
| `max_requests_per_minute`          | `120`     | 适配器对 REST 调用执行的滚动分钟限流。                           |
| `proxy_url`                        | `None`    | 可选的 HTTP 和 WebSocket 传输代理 URL。                   |
| `transport_backend`                | `Sockudo` | WebSocket 传输后端。                                  |

### 执行客户端配置选项

BitMEX 执行客户端提供以下配置选项：

| 选项                             | 默认值       | 描述                                                  |
| ------------------------------ | --------- | --------------------------------------------------- |
| `api_key`                      | `None`    | 可选的 API 密钥；如果为 `None`，从 `environment` 所选的环境加载。      |
| `api_secret`                   | `None`    | 可选的 API 密钥密文；如果为 `None`，从 `environment` 所选的环境加载。    |
| `environment`                  | `None`    | 环境枚举（`MAINNET` 或 `TESTNET`）。                        |
| `base_url_http`                | `None`    | REST 基础 URL 覆盖（默认为生产环境）。                            |
| `base_url_ws`                  | `None`    | WebSocket 基础 URL 覆盖（默认为生产环境）。                       |
| `http_timeout_secs`            | `60`      | 应用于 HTTP 调用的请求超时。                                   |
| `max_retries`                  | `3`       | HTTP 调用的最大重试次数。                                     |
| `retry_delay_initial_ms`       | `1,000`   | 重试之间的初始退避延迟（毫秒）。                                    |
| `retry_delay_max_ms`           | `10,000`  | 重试之间的最大退避延迟（毫秒）。                                    |
| `recv_window_ms`               | `10,000`  | 签名请求的过期窗口（毫秒）。参见[请求认证](#请求认证和过期)。                   |
| `max_requests_per_second`      | `10`      | 适配器对 REST 调用执行的突发限流。                                |
| `max_requests_per_minute`      | `120`     | 适配器对 REST 调用执行的滚动分钟限流。                              |
| `deadmans_switch_timeout_secs` | `None`    | 死人开关的超时秒数。`None` 表示禁用。参见[死人开关](#死人开关)。              |
| `canceller_pool_size`          | `None`    | 取消广播器池中的 HTTP 客户端数量。`None` 解析为 1。参见[取消广播器](#取消广播器)。 |
| `submitter_pool_size`          | `None`    | 提交广播器池中的 HTTP 客户端数量。`None` 解析为 1。参见[提交广播器](#提交广播器)。 |
| `proxy_url`                    | `None`    | 可选的 HTTP 和 WebSocket 传输代理 URL。                      |
| `submitter_proxy_urls`         | `None`    | 可选的代理 URL 列表，用于提交广播器路径多样性。*尚未通过 Python 集成接入。*       |
| `canceller_proxy_urls`         | `None`    | 可选的代理 URL 列表，用于取消广播器路径多样性。*尚未通过 Python 集成接入。*       |
| `transport_backend`            | `Sockudo` | WebSocket 传输后端。                                     |

### 配置示例

BitMEX 实时交易的典型配置包括测试网和主网选项：

```python
from nautilus_trader.adapters.bitmex.config import BitmexDataClientConfig
from nautilus_trader.adapters.bitmex.config import BitmexExecClientConfig
from nautilus_trader.adapters.bitmex import BitmexEnvironment

# 使用环境变量（推荐）
testnet_data_config = BitmexDataClientConfig(
    environment=BitmexEnvironment.TESTNET,
)

# 使用显式凭证
mainnet_data_config = BitmexDataClientConfig(
    api_key="YOUR_API_KEY",  # 或使用 os.getenv("BITMEX_API_KEY")
    api_secret="YOUR_API_SECRET",  # 或使用 os.getenv("BITMEX_API_SECRET")
    environment=BitmexEnvironment.MAINNET,
)

mainnet_exec_config = BitmexExecClientConfig(
    api_key="YOUR_API_KEY",
    api_secret="YOUR_API_SECRET",
    environment=BitmexEnvironment.MAINNET,
)
```

## 交易注意事项

### 条件订单

BitMEX 执行适配器现在将 Nautilus 条件订单列表映射到交易所的
原生 `clOrdLinkID`/`contingencyType` 机制。当引擎提交
`ContingencyType::Oco` 或 `ContingencyType::Oto` 订单时，适配器将：

- 在 BitMEX 上创建/维护链接的订单组，使子止损单和目标单继承
  父订单的状态。
- 传播订单列表更新和取消，以便条件关联的对等订单与
  当前持仓状态保持一致。
- 显示包含适当条件元数据的执行报告，使策略级别的
  跟踪无需额外的手动配置。

这意味着常见的条件单组合流程（入场 + 止损 + 止盈）和多腿止损结构现在可以
直接由 BitMEX 管理，而不是在客户端模拟。定义策略时，继续使用 Nautilus 的
`OrderList`/`ContingencyType` 抽象——适配器会自动处理所需的 BitMEX 配置。

### 合约规格

- **反向合约**：以加密货币结算（如 XBTUSD 以 XBT 结算）。
- **线性合约**：以稳定币结算（如 ETHUSDT 以 USDT 结算）。
- **合约大小**：因金融工具而异，请仔细检查规格。
- **最小价格变动**：最小价格增量因合约而异。

### 保证金要求

- 初始保证金要求因合约和市场条件而异。
- 维持保证金通常低于初始保证金。
- 当维持保证金要求不满足时会发生强平。
- BitMEX 支持逐仓保证金和全仓保证金模式。
- 风险限额可根据持仓大小进行调整，详见 [Exchange Rules](https://www.bitmex.com/exchange-rules)。

### 手续费

- **Maker 手续费**：通常为负值（提供流动性的返佣）。
- **Taker 手续费**：消耗流动性的正向手续费。
- **资金费率**：每 8 小时适用于永续合约。
- **预测市场手续费**：Maker 0.00%，Taker 0.25%（不允许使用杠杆）。

## 贡献

:::info
如需更多功能或为 BitMEX 适配器做贡献，请参阅我们的
[贡献指南](https://github.com/nautechsystems/nautilus_trader/blob/develop/CONTRIBUTING.md)。
:::
