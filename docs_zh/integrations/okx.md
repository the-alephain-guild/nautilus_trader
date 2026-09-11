# OKX

OKX 成立于 2017 年，是一家加密货币交易所，提供现货（Spot）、保证金（Margin）、永续合约（Perpetual Swap）、期货（Futures）、期权（Options）、价差（Spread）和事件合约（Event Contract）交易。本集成（Integration）支持在 OKX 上进行实时市场数据接入和订单执行。

## 概述

该适配器（Adapter）使用 Rust 实现，提供可选的 Python 绑定以便在基于 Python 的工作流中使用。它不需要外部 OKX 客户端库——核心组件编译为静态库并在构建过程中自动链接。

## 示例

实时示例脚本位于
[examples/live/okx](https://github.com/nautechsystems/nautilus_trader/tree/develop/examples/live/okx/)。

### 产品支持

| 产品   | 金融工具来源                | 数据  | 执行  | 备注                           |
| ---- | --------------------- | --- | --- | ---------------------------- |
| 现货   | `public/instruments`  | 是   | 是   | 现货交易对。                       |
| 保证金  | `public/instruments`  | 是   | 是   | 带保证金或杠杆的现货金融工具。              |
| 永续合约 | `public/instruments`  | 是   | 是   | 线性和反向合约。                     |
| 期货   | `public/instruments`  | 是   | 是   | 有到期日的期货合约。                   |
| 期权   | `public/instruments`  | 是   | 是   | 限价式订单执行。                     |
| 价差   | `sprd/spreads`        | 是   | 是   | 业务 WS 上的快照、报价、成交。            |
| 事件合约 | `event-contract/*` 端点 | 是   | 是   | 解析为 Nautilus `BinaryOption`。 |

相关 OKX 文档：

- [获取金融工具](https://www.okx.com/docs-v5/en/#public-data-rest-api-get-instruments)。
- [获取价差（公共）](https://www.okx.com/docs-v5/en/#spread-trading-rest-api-get-spreads-public)。
- [价差交易下单](https://www.okx.com/docs-v5/en/#spread-trading-rest-api-place-order)。
- [事件合约系列](https://www.okx.com/docs-v5/en/#public-data-rest-api-get-series)。

:::note
**期权支持**：适配器支持期权市场数据、交易所提供的希腊字母（Greeks）（`subscribe_option_greeks`），以及期权金融工具的订单执行。详见下方的
[期权交易](#期权交易options-trading)章节，以及订阅模式相关的
[期权](../concepts/options.md)指南。
:::

:::info
**金融工具乘数**：对于衍生品（`SWAP`、`FUTURES`、`OPTION`），金融工具乘数计算为 OKX 的 `ctMult` 和 `ctVal` 字段的乘积。这使持仓规模与 OKX 合约大小和价值保持一致。
:::

OKX 适配器包含多个组件，可以单独或组合使用：

- `OKXHttpClient`：底层 HTTP API 连接。
- `OKXWebSocketClient`：底层 WebSocket API 连接。
- `OKXInstrumentProvider`：金融工具解析和加载功能。
- `OKXDataClient`：市场数据推送管理器。
- `OKXExecutionClient`：账户管理和交易执行网关。
- `OKXLiveDataClientFactory`：OKX 数据客户端工厂（由交易节点构建器使用）。
- `OKXLiveExecClientFactory`：OKX 执行客户端工厂（由交易节点构建器使用）。

:::note
大多数用户只需为实时交易节点定义配置（如下所示），无需直接使用这些底层组件。
:::

## 符号体系

OKX 对不同金融工具类型使用特定的符号约定。在 Nautilus 中引用金融工具时添加 `.OKX` 后缀，例如 `BTC-USDT.OKX`。

### 按金融工具类型的符号格式

#### 现货（SPOT）

格式：`{基础货币}-{报价货币}`

示例：

- `BTC-USDT` - 比特币兑 USDT (Tether)
- `BTC-USDC` - 比特币兑 USDC
- `ETH-USDT` - 以太坊兑 USDT
- `SOL-USDT` - Solana 兑 USDT

在策略中订阅现货比特币 USD：

```python
InstrumentId.from_str("BTC-USDT.OKX")  # USDT 计价的现货
InstrumentId.from_str("BTC-USDC.OKX")  # USDC 计价的现货
```

#### SWAP（永续合约）

格式：`{基础货币}-{报价货币}-SWAP`

示例：

- `BTC-USDT-SWAP` - 比特币永续合约（线性，USDT 保证金）
- `BTC-USD-SWAP` - 比特币永续合约（反向，币本位保证金）
- `ETH-USDT-SWAP` - 以太坊永续合约（线性）
- `ETH-USD-SWAP` - 以太坊永续合约（反向）

线性 vs 反向合约：

- **线性**（USDT 保证金）：使用 USDT 等稳定币作为保证金。
- **反向**（币本位保证金）：使用基础加密货币作为保证金。

#### 期货（FUTURES，交割期货）

格式：`{基础货币}-{报价货币}-{YYMMDD}`

示例：

- `BTC-USD-251226` - 2025 年 12 月 26 日到期的比特币期货
- `ETH-USD-251226` - 2025 年 12 月 26 日到期的以太坊期货
- `BTC-USD-250328` - 2025 年 3 月 28 日到期的比特币期货

注意：期货通常为反向合约（币本位保证金）。

#### 价差（SPREADS）

格式：`{第一腿金融工具ID}_{第二腿金融工具ID}`

示例：

- `BTC-USDT_BTC-USDT-SWAP` - BTC-USDT 现货与 BTC-USDT 永续合约之间的价差
- `ETH-USD-SWAP_ETH-USD-231229` - ETH-USD 永续合约与交割期货之间的价差

在数据客户端上设置 `load_spreads=True`，即可从 OKX
[获取价差（公共）](https://www.okx.com/docs-v5/en/#spread-trading-rest-api-get-spreads-public)
端点加载实时 OKX 价差金融工具。适配器将每个 OKX `sprdId` 映射为带 `.OKX` 交易所后缀的
Nautilus 价差金融工具 ID。

价差金融工具简要说明：

- 价差市场数据通过 OKX 业务 WebSocket 推送：报价（`sprd-bbo-tbt`）、成交（`sprd-public-trades`）
  和 5 档订单簿快照（`sprd-books5`）。价差没有增量订单簿频道，因此每次 `sprd-books5` 更新都是
  通过订单簿订阅交付的完整快照（标记为快照，而非增量 L2 增量数据）。
- 当前 OKX 实时价差发现会返回现货、永续和期货腿组合。
- 如果 OKX 通过同一价差端点暴露期权腿，解析器可以表示带期权腿的价差定义。
- OKX 期权 RFQ 和大宗交易工作流与 Nitro 价差订单簿 API 是分开的，不通过此价差路径路由。

#### 期权（OPTIONS）

格式：`{基础货币}-{报价货币}-{YYMMDD}-{行权价}-{类型}`

示例：

- `BTC-USD-251226-100000-C` - 比特币看涨期权，行权价 $100,000，2025 年 12 月 26 日到期
- `BTC-USD-251226-100000-P` - 比特币看跌期权，行权价 $100,000，2025 年 12 月 26 日到期
- `ETH-USD-251226-4000-C` - 以太坊看涨期权，行权价 $4,000，2025 年 12 月 26 日到期

其中：

- `C` = 看涨期权（Call）
- `P` = 看跌期权（Put）

#### 事件（EVENTS）

OKX 事件合约金融工具 ID 使用 OKX 金融工具 API 返回的市场 ID。适配器将这些市场表示为
Nautilus `BinaryOption` 金融工具。

示例：

- `BTC-ABOVE-DAILY-260224-1600-65000` - `BTC-ABOVE-DAILY` 系列中的事件合约市场。

### 常见问题

**Q：如何订阅现货比特币 USD？**
A：使用 `BTC-USDT.OKX` 订阅 USDT 保证金现货，或使用 `BTC-USDC.OKX` 订阅 USDC 保证金现货。

**Q：BTC-USDT-SWAP 和 BTC-USD-SWAP 有什么区别？**
A：`BTC-USDT-SWAP` 是线性永续合约（USDT 保证金），而 `BTC-USD-SWAP` 是反向永续合约（BTC 保证金）。

**Q：如何知道应该使用哪种合约类型？**
A：查看配置中的 `contract_types` 参数：

- 线性合约：`OKXContractType.LINEAR`。
- 反向合约：`OKXContractType.INVERSE`。

**Q：如何加载事件合约？**
A：使用 `OKXInstrumentType.EVENTS`。要限定加载范围，通过 `instrument_families` 传入 OKX `seriesId` 值，例如
`BTC-ABOVE-DAILY`。

## 订单功能

以下是 OKX 上线性永续合约产品支持的订单类型、执行指令和有效时间选项。

### WebSocket 订单识别

OKX WebSocket 订单操作使用 `instIdCode`（数字金融工具标识符），而非字符串 `instId` 参数。
适配器从启动期间获取的金融工具定义中解析 `instIdCode` 值，并在整个会话生命周期内缓存它们。
如果金融工具缓存为空（例如因为引导启动失败），订单提交将以清晰的错误失败。

### 客户端订单 ID 要求

:::note
OKX 对客户端订单 ID 有特定要求：

- **不允许使用连字符**：OKX 不接受客户端订单 ID 中的连字符（`-`）。
- 最大长度：32 个字符。
- 允许的字符：仅限字母和数字。

配置策略时，请确保设置：

```python
use_hyphens_in_client_order_ids = False
```

:::

### 订单类型

| 订单类型                   | 线性永续合约 | 备注                       |
| ---------------------- | ------ | ------------------------ |
| `MARKET`               | ✓      | 以市场价格立即执行。支持报价数量。        |
| `MARKET_TO_LIMIT`      | ✓      | 转换为 IOC 限价单的市价单。         |
| `LIMIT`                | ✓      | 以指定价格或更优价格执行。            |
| `STOP_MARKET`          | ✓      | 通过 OKX 算法订单实现的条件市价单。     |
| `STOP_LIMIT`           | ✓      | 通过 OKX 算法订单实现的条件限价单。     |
| `MARKET_IF_TOUCHED`    | ✓      | 通过 OKX 算法订单实现的条件市价单。     |
| `LIMIT_IF_TOUCHED`     | ✓      | 通过 OKX 算法订单实现的条件限价单。     |
| `TRAILING_STOP_MARKET` | ✓      | 通过 OKX 高级算法订单实现的追踪止损市价单。 |

:::info
**条件订单**：`STOP_MARKET`、`STOP_LIMIT`、`MARKET_IF_TOUCHED`、`LIMIT_IF_TOUCHED` 和
`TRAILING_STOP_MARKET` 使用 OKX 算法订单。`TRAILING_STOP_MARKET` 路径使用 OKX 的高级算法订单
API（`move_order_stop`），取消时需要 `cancel-advance-algos` 端点。
:::

### 价差订单

OKX 价差金融工具使用独立的价差交易订单簿和 API 系列。执行客户端当前按价差金融工具 ID 路由价差订单，
例如 `ETH-USD-SWAP_ETH-USD-231229.OKX`，通过 HTTP `/api/v5/sprd/*` 端点。

适配器使用 OKX 的价差 REST 端点进行提交、取消、批量取消、订单状态和成交报告。它订阅 OKX 业务 WebSocket
[`sprd-orders` 频道](https://www.okx.com/docs-v5/en/#spread-trading-websocket-private-channel-order-channel)
以获取实时价差订单更新。

OKX `sprd-orders` WebSocket 更新不包含手续费字段。从该频道发出的实时价差成交报告使用零手续费；
来自 REST [`sprd/trades` 端点](https://www.okx.com/docs-v5/en/#spread-trading-rest-api-get-trades)
的历史和对账成交报告包含 OKX 手续费数据。

支持的价差订单指令：

- 带 GTC 有效时间的 `LIMIT`。
- 带 IOC 有效时间的 `LIMIT`。
- 带 post-only 执行的 `LIMIT`。

OKX 价差交易 API 路径不支持价差订单列表、条件订单、FOK 有效时间和修改请求。

相关 OKX 文档：

- [价差下单](https://www.okx.com/docs-v5/en/#spread-trading-rest-api-place-order)。
- [价差订单详情](https://www.okx.com/docs-v5/en/#spread-trading-rest-api-get-order-details)。
- [价差订单频道](https://www.okx.com/docs-v5/en/#spread-trading-websocket-private-channel-order-channel)。

### 现货保证金交易的数量语义

在使用现货保证金交易（`use_spot_margin=True`）时，OKX 根据订单方向对订单数量有不同的解释：

- **限价**订单将 `quantity` 解释为基础货币单位数量。
- **市价卖出**订单也使用基础货币单位数量。
- **市价买入**订单将 `quantity` 解释为报价名义金额（例如 USDT）。

:::warning
**提交现货保证金市价买入订单时**，请在订单上设置 `quote_quantity=True`（或预先计算以报价货币计价的金额）。
OKX 执行客户端会拒绝以基础货币计价的现货保证金市价买入订单，以防止意外成交。

**在首次成交时**，订单数量将从报价数量自动更新为实际收到的基础货币数量，以反映已执行的交易。
:::

```python
# 使用报价数量的现货保证金市价买入（花费 100 USDT）
order = strategy.order_factory.market(
    instrument_id=instrument_id,
    order_side=OrderSide.BUY,
    quantity=instrument.make_qty(100.0),
    quote_quantity=True,  # 解释为 USDT 名义金额
)
strategy.submit_order(order)
```

### 执行指令

| 指令            | 线性永续合约 | 备注     |
| ------------- | ------ | ------ |
| `post_only`   | ✓      | 仅限限价单。 |
| `reduce_only` | ✓      | 仅限衍生品。 |

### 有效时间

| 有效时间  | 线性永续合约 | 备注                            |
| ----- | ------ | ----------------------------- |
| `GTC` | ✓      | 撤销前有效（Good Till Canceled）。    |
| `FOK` | ✓      | 全部成交或撤销（Fill or Kill）。        |
| `IOC` | ✓      | 立即成交或撤销（Immediate or Cancel）。 |
| `GTD` | -      | *无原生 OKX 订单有效时间。*             |

:::note
**GTD（到期前有效）有效时间**：OKX 通过 `expTime` 支持请求过期，但那是请求超时，而非原生订单过期指令。

如果您需要 GTD 功能，请使用 Nautilus 的策略管理 GTD 特性。它通过在指定到期时间取消订单来处理订单过期。
:::

### 批量操作

| 操作   | 线性永续合约 | 备注            |
| ---- | ------ | ------------- |
| 批量提交 | ✓      | 在单个请求中提交多个订单。 |
| 批量修改 | ✓      | 在单个请求中修改多个订单。 |
| 批量取消 | ✓      | 在单个请求中取消多个订单。 |

### 持仓管理

| 功能    | 线性永续合约 | 备注                 |
| ----- | ------ | ------------------ |
| 查询持仓  | ✓      | 实时持仓更新。            |
| 持仓模式  | ✓      | 净持仓 vs 多/空模式（见下文）。 |
| 杠杆控制  | ✓      | 按金融工具动态调整杠杆。       |
| 保证金模式 | ✓      | 支持现金、逐仓和全仓模式。      |

#### 持仓模式

OKX 为衍生品交易支持两种持仓模式：

- **净持仓模式**（Netting）：每个金融工具一个持仓。买入和卖出订单相互抵消。这是默认模式，推荐大多数交易者使用。
- **多/空模式**（Hedging）：同一金融工具的多头和空头持仓分开。此模式支持同时持有多头和空头敞口。

:::note
持仓模式必须通过 OKX 网页或 App 界面配置，适用于全账户。适配器会检测当前持仓模式并相应处理持仓报告。
:::

### 交易模式和保证金配置

OKX 的统一账户系统支持现货和衍生品交易的不同交易模式。适配器根据您的配置和金融工具类型确定正确的交易模式。

:::note
**重要**：请首先通过 OKX 网页或 App 界面配置账户模式。API 无法首次设置账户模式。
:::

有关 OKX 账户模式和保证金的更多详细信息，请参阅
[OKX 账户模式文档](https://www.okx.com/docs-v5/en/#overview-account-mode)。

#### 交易模式概述

OKX 支持多种账户模式。对于订单，适配器从您的配置中选择 `cash`、`isolated` 或 `cross` 交易模式之一：

| 模式             | 用途         | 杠杆  | 借贷  | 配置                             |
| -------------- | ---------- | --- | --- | ------------------------------ |
| **`cash`**     | 不带杠杆的现货交易。 | -   | -   | `use_spot_margin=False` 时为默认值。 |
| **`isolated`** | 现货保证金或衍生品。 | ✓   | ✓   | `margin_mode=ISOLATED`。        |
| **`cross`**    | 现货保证金或衍生品。 | ✓   | ✓   | `margin_mode=CROSS`。           |

#### 基于配置的交易模式选择

适配器从以下条件中选择交易模式：

1. **金融工具类型**（`SPOT` vs 其他 OKX 金融工具类型）。
2. **配置设置**（`SPOT` 使用 `use_spot_margin`，其他使用 `margin_mode`）。

##### 现货交易

```python
# 不带杠杆的简单现货交易（使用 'cash' 模式）
exec_clients = {
    OKX: OKXExecClientConfig(
        instrument_types=(OKXInstrumentType.SPOT,),
        use_spot_margin=False,  # 默认 - 简单现货
        # ... 其他配置
    ),
}

# 带保证金/杠杆的现货交易（使用 'isolated' 或 'cross' 模式）
exec_clients = {
    OKX: OKXExecClientConfig(
        instrument_types=(OKXInstrumentType.SPOT,),
        use_spot_margin=True,  # 为现货启用保证金交易
        margin_mode=OKXMarginMode.ISOLATED,  # 或 CROSS 用于共享保证金
        # ... 其他配置
    ),
}
```

##### 非现货交易

```python
# 逐仓保证金的衍生品（默认 - 使用 'isolated' 模式）
exec_clients = {
    OKX: OKXExecClientConfig(
        instrument_types=(OKXInstrumentType.SWAP,),
        margin_mode=OKXMarginMode.ISOLATED,  # 或省略 - ISOLATED 为默认值
        # ... 其他配置
    ),
}

# 全仓保证金的衍生品（使用 'cross' 模式）
exec_clients = {
    OKX: OKXExecClientConfig(
        instrument_types=(OKXInstrumentType.SWAP,),
        margin_mode=OKXMarginMode.CROSS,  # 在所有持仓间共享保证金
        # ... 其他配置
    ),
}
```

##### 混合现货和衍生品交易

当同时交易现货和衍生品金融工具时，适配器根据交易的金融工具按订单确定交易模式：

```python
# 混合现货 + SWAP 配置
exec_clients = {
    OKX: OKXExecClientConfig(
        instrument_types=(OKXInstrumentType.SPOT, OKXInstrumentType.SWAP),
        use_spot_margin=True,  # 仅适用于现货订单
        margin_mode=OKXMarginMode.CROSS,  # 仅适用于 SWAP 订单
        # ... 其他配置
    ),
}
```

**工作原理：**

- **现货订单**使用 `cross` 模式，因为 `use_spot_margin=True` 且 `margin_mode=CROSS`。
- **SWAP 订单**使用 `cross` 模式，因为 `margin_mode=CROSS`。
- 每个订单根据其金融工具类型获得正确的 `tdMode`。
- 无需手动干预。

这支持跨金融工具类型、使用不同保证金配置进行交易的策略，例如：

- 现货-期货套利策略。
- 结合现货和永续合约的 Delta 中性策略。
- 跨现货和衍生品市场的做市策略。

:::warning
**手动交易模式覆盖**：您可以使用 `params={"td_mode": "..."}` 按订单覆盖交易模式。这会绕过适配器的选择，
当该值与金融工具类型不匹配时（例如对现货金融工具使用 `isolated`）可能导致订单被拒绝。

仅在无法通过配置满足的需求时才使用手动覆盖。
:::

#### 基于配置方式的优势

- **类型安全**：配置在启动时、下单之前验证。
- **自动化**：适配器根据金融工具类型和意图选择模式。
- **清晰明了**：字段名称说明用途，例如 `use_spot_margin` 与 `td_mode`。
- **安全可靠**：不兼容的组合在到达 OKX 之前被拒绝。
- **向后兼容**：默认值保持现有行为。

### 订单查询

| 功能     | 线性永续合约 | 备注        |
| ------ | ------ | --------- |
| 查询未结订单 | ✓      | 列出所有活动订单。 |
| 查询历史订单 | ✓      | 历史订单数据。   |
| 订单状态更新 | ✓      | 实时订单状态变更。 |
| 交易历史   | ✓      | 执行和成交报告。  |

### 关联订单

| 功能     | 线性永续合约 | 备注                        |
| ------ | ------ | ------------------------- |
| 订单列表   | ✓      | 通过 WS 批量；仅限常规订单。          |
| OCO 订单 | ✓      | 二选一订单（One-Cancels-Other）。 |
| 括号订单   | ✓      | 止损 + 止盈组合。                |
| 条件订单   | ✓      | 止损和触价限价订单。                |

#### 条件订单架构

条件订单（OKX 算法订单）使用混合架构：

- **提交**：HTTP REST API（`/api/v5/trade/order-algo`）。
- **状态更新**：WebSocket 业务端点（`/ws/v5/business`）的 `orders-algo` 频道。
- **取消**：带算法订单 ID 跟踪的 HTTP REST API。

此设计确保：

- 通过 HTTP 即时获得提交确认。
- 通过 WebSocket 获得实时状态更新。
- 通过算法订单 ID 映射进行正确的订单生命周期管理。

#### 支持的条件订单类型

| 订单类型                   | 触发类型              | 备注          |
| ---------------------- | ----------------- | ----------- |
| `STOP_MARKET`          | Last, Mark, Index | 触发时以市价执行。   |
| `STOP_LIMIT`           | Last, Mark, Index | 触发时下限价单。    |
| `MARKET_IF_TOUCHED`    | Last, Mark, Index | 价格触及时以市价执行。 |
| `LIMIT_IF_TOUCHED`     | Last, Mark, Index | 价格触及时下限价单。  |
| `TRAILING_STOP_MARKET` | Last, Mark, Index | 带回调比例的追踪止损。 |

#### 触发价格类型

条件订单支持不同的触发价格来源：

- **最新价**（`TriggerType.LAST_PRICE`）：使用最新成交价（默认）。
- **标记价**（`TriggerType.MARK_PRICE`）：使用标记价格。
- **指数价**（`TriggerType.INDEX_PRICE`）：使用标的指数价格。

```python
# 示例：使用标记价触发的止损
stop_order = order_factory.stop_market(
    instrument_id=instrument_id,
    order_side=OrderSide.SELL,
    quantity=Quantity.from_str("0.1"),
    trigger_price=Price.from_str("45000.0"),
    trigger_type=TriggerType.MARK_PRICE,  # 使用标记价作为触发
)
strategy.submit_order(stop_order)
```

## 风险管理

### 强平和 ADL 事件处理

OKX 适配器检测交易所发起的风险管理事件：

- **强平订单**：当交易所强制平仓某个持仓时，适配器检测强平类别并记录带有订单详情的警告。
  这些订单继续通过正常的订单和成交管道处理。
- **自动减仓（ADL）**：当 OKX 关闭您的持仓以抵消对手方的强平时，适配器检测并记录带有持仓详情的 ADL 事件。

检测由订单记录上的 `category` 字段驱动。已识别的取值为：

| `category`            | 含义      |
| --------------------- | ------- |
| `full_liquidation`    | 全部持仓强平。 |
| `partial_liquidation` | 部分持仓强平。 |
| `adl`                 | 自动减仓平仓。 |
| `delivery`            | 到期合约交割。 |
| `normal` / 其他取值       | 常规订单流程。 |

检测在两条路径上运行：

- WebSocket `orders` 频道（实时订单/成交更新）。
- HTTP `GET /api/v5/trade/orders-history` 和 `orders-history-archive`
  （用于对账和冷启动批量状态）。

:::info
**强平和 ADL 事件以 WARNING 级别记录**，包含订单 ID、金融工具和状态等详情。请监控这些日志，作为风险管理流程的一部分。

适配器处理这些交易所生成的订单，发出相关的 `OrderFilled` 事件并更新持仓。您的策略代码无需单独的处理路径。
:::

上游参考：

- [订单频道和 `category` 字段](https://www.okx.com/docs-v5/en/#order-book-trading-trade-ws-order-channel)
- [自动减仓机制](https://www.okx.com/help/okx-contract-auto-deleveraging-adl)
- [强平机制](https://www.okx.com/help/introduction-to-liquidation)

## 期权交易（Options trading）

OKX 适配器支持交易期权（`OPTION` 金融工具类型），与其他衍生品存在一些差异。OKX 期权是以标的加密货币结算的反向合约。
完整的 API 详情请参阅
[OKX 期权交易文档](https://www.okx.com/docs-v5/en/#order-book-trading-trade-post-place-order)。

### 支持的订单类型

仅支持限价式订单。OKX 不允许期权市价单。

| 订单类型     | 是否支持 | 备注                |
| -------- | ---- | ----------------- |
| `LIMIT`  | ✓    | 标准限价单。            |
| `MARKET` | -    | 在到达 API 之前被适配器拒绝。 |

期权支持 FOK 和 IOC 有效时间。OKX 对期权 FOK 订单使用专用的 `op_fok` 订单类型；适配器自动处理此映射。

期权不支持条件/算法订单（`STOP_MARKET`、`STOP_LIMIT`、`MARKET_IF_TOUCHED`、`LIMIT_IF_TOUCHED`、
`TRAILING_STOP_MARKET`），这些订单会被拒绝。

### 定价模式

期权订单可以用三种互斥的方式定价。通过订单 `params` 传入定价模式：

| 模式    | 参数       | 描述                    |
| ----- | -------- | --------------------- |
| Price | （默认）     | 以合约货币计的标准限价。          |
| USD   | `px_usd` | 以美元计价的价格。             |
| IV    | `px_vol` | 以隐含波动率计价（1.0 = 100%）。 |

```python
# 以美元定价
order = strategy.order_factory.limit(
    instrument_id=InstrumentId.from_str("BTC-USD-250328-50000-C.OKX"),
    order_side=OrderSide.BUY,
    quantity=Quantity.from_int(1),
    price=Price.from_str("0"),  # 占位符；px_usd 优先
    params={"px_usd": "100.5"},
)

# 以隐含波动率定价
order = strategy.order_factory.limit(
    instrument_id=InstrumentId.from_str("BTC-USD-250328-50000-C.OKX"),
    order_side=OrderSide.BUY,
    quantity=Quantity.from_int(1),
    price=Price.from_str("0"),  # 占位符；px_vol 优先
    params={"px_vol": "0.55"},
)
```

修改订单时，同样的 `px_usd` 或 `px_vol` 参数可以传给修改命令，以原始定价模式修订价格。

### 期权希腊字母（Greeks）

OKX 在 `opt-summary` 频道上发布两套并行的希腊字母：

- **Black-Scholes（`BLACK_SCHOLES`）**：以美元计价的希腊字母。与 Deribit 和 Bybit 适配器使用的约定一致。
- **价格调整（`PRICE_ADJUSTED`）**：以标的币单位计价的希腊字母。与 OKX 的原生合约约定一致。

默认情况下，适配器在每个 `opt-summary` tick 上同时发出两者。每个发出的 `OptionGreeks` 都带有一个
`convention` 字段，取值为 `GreeksConvention.BLACK_SCHOLES` 或 `GreeksConvention.PRICE_ADJUSTED`，
因此接收方可以按消息分支处理。

要缩小数据流范围，在订阅时传入 `params["greeks_convention"]`：

- 单个字符串：`"BLACK_SCHOLES"` 或 `"PRICE_ADJUSTED"`（不区分大小写）。
- 字符串列表：`["BLACK_SCHOLES", "PRICE_ADJUSTED"]`。
- 省略：适配器同时发出两者。

未知的条目会记录警告并被跳过。如果每个请求的条目都未知，适配器会回退为同时发出两者。

```python
# 默认（两种约定，接收方分支处理）
self.subscribe_option_greeks(instrument_id)


def on_option_greeks(self, greeks: OptionGreeks) -> None:
    if greeks.convention == GreeksConvention.BLACK_SCHOLES:
        self._handle_bs(greeks)
    else:
        self._handle_pa(greeks)
```

```python
# 缩小为单一约定
self.subscribe_option_greeks(
    instrument_id,
    params={"greeks_convention": "PRICE_ADJUSTED"},
)
```

```python
# 显式列表（同时列出两者时等同于默认）
self.subscribe_option_greeks(
    instrument_id,
    params={"greeks_convention": ["BLACK_SCHOLES", "PRICE_ADJUSTED"]},
)
```

:::note
数据引擎按 `instrument_id` 对期权希腊字母订阅去重，因此如果一个节点上的两个 actor 用不同的单一约定订阅同一金融工具，
只有第一个会到达适配器。第二个 actor 会得到第一个 actor 的约定集合。变通方法：任一 actor 都可以不带 `params`
（或带完整列表）订阅，以接收两条数据流，并在本地按 `greeks.convention` 过滤。
:::

### 持仓希腊字母

适配器从 OKX 持仓数据中暴露持仓级别的 Black-Scholes 希腊字母（`delta_bs`、`gamma_bs`、`theta_bs`、`vega_bs`）。
这些可通过标准持仓报告管道获取。

### 限制

- `reduce_only` 不适用于期权，会被自动剥离。
- 持仓方向默认为 `Net`。

### 配置

期权需要 `instrument_families` 配置参数来限定要加载哪些标的：

```python
config = TradingNodeConfig(
    data_clients={
        OKX: OKXDataClientConfig(
            instrument_types=(OKXInstrumentType.OPTION,),
            instrument_families=("BTC-USD", "ETH-USD"),
            instrument_provider=InstrumentProviderConfig(load_all=True),
        ),
    },
    exec_clients={
        OKX: OKXExecClientConfig(
            instrument_types=(OKXInstrumentType.OPTION,),
            instrument_families=("BTC-USD", "ETH-USD"),
            margin_mode=OKXMarginMode.CROSS,
        ),
    },
)
```

## 事件合约

OKX 通过 `instType=EVENTS` 暴露预测市场合约。适配器将这些金融工具加载为 Nautilus `BinaryOption` 金融工具，
并在金融工具的 `info` 字段中保留 OKX 元数据，例如 `seriesId`、`instCategory`、`instIdCode`、`state` 和 `ruleType`。

### 加载事件合约金融工具

在数据或执行客户端配置中使用 `OKXInstrumentType.EVENTS`。`instrument_families` 设置映射到事件合约的
OKX `seriesId` 值。当省略 `instrument_families` 时，适配器先请求事件合约系列列表，然后为每个系列请求金融工具。

```python
config = TradingNodeConfig(
    data_clients={
        OKX: OKXDataClientConfig(
            instrument_types=(OKXInstrumentType.EVENTS,),
            instrument_families=("BTC-ABOVE-DAILY",),
            instrument_provider=InstrumentProviderConfig(load_all=True),
        ),
    },
    exec_clients={
        OKX: OKXExecClientConfig(
            instrument_types=(OKXInstrumentType.EVENTS,),
            instrument_families=("BTC-ABOVE-DAILY",),
            margin_mode=OKXMarginMode.CROSS,
        ),
    },
)
```

### 事件合约市场数据

底层 HTTP 客户端暴露 OKX 的公共事件合约发现端点：

- `request_event_contract_series`。
- `request_event_contract_events`。
- `request_event_contract_markets`。

底层 WebSocket 客户端通过 `subscribe_event_contract_markets` 和 `unsubscribe_event_contract_markets`
支持 `event-contract-markets` 频道。该频道发布市场状态和地板行权价（floor-strike）生成更新，没有初始快照，
且不包含 `instId`，因此适配器将其作为原始交易所 JSON 转发。

:::note
OKX 的标准市场数据端点为 `EVENTS` 返回 YES 侧数据。当策略需要两种结果时，从 YES 侧价格推导 NO 侧价格。
:::

### 事件合约交易

提交事件合约订单时，通过订单 `params` 传入 OKX 事件结果：

```python
order = strategy.order_factory.limit(
    instrument_id=InstrumentId.from_str("BTC-ABOVE-DAILY-260224-1600-65000.OKX"),
    order_side=OrderSide.BUY,
    quantity=Quantity.from_int(1),
    price=Price.from_str("0.42"),
    params={"outcome": "yes"},
)
strategy.submit_order(order)
```

OKX 对 `EVENTS` 订单要求 `outcome`。它还要求非 post-only 事件合约订单和修改请求带有 `speedBump=1`。
适配器在发送订单前验证 `outcome`，并在未提供时为非 post-only 事件订单将 `speedBump` 默认设为 `1`。

结算成交以 OKX 订单类别 `delivery` 到达。适配器在实时订单更新和对账期间识别此类别。

上游参考：

- [事件合约 REST 端点](https://www.okx.com/docs-v5/en/#public-data-rest-api-get-series)。
- [WS 频道](https://www.okx.com/docs-v5/en/#public-data-websocket-event-contract-markets-channel)。
- [下单请求字段](https://www.okx.com/docs-v5/en/#order-book-trading-trade-post-place-order)。

## 身份验证

要使用 OKX 适配器，请在 OKX 账户中创建 API 凭证：

1. 登录您的 OKX 账户并导航到 API 管理页面。
2. 创建一个具有交易和数据访问所需权限的新 API 密钥。
3. 记下您的 API 密钥、密钥（Secret Key）和密码短语（Passphrase）。

您可以通过环境变量提供这些凭证：

```bash
export OKX_API_KEY="your_api_key"
export OKX_API_SECRET="your_api_secret"
export OKX_API_PASSPHRASE="your_passphrase"
```

或者直接在配置中传递（不推荐在生产环境中使用）。

## 模拟交易

OKX 提供模拟交易环境，用于在不使用真实资金的情况下测试策略。

### 设置模拟账户

1. 在 [okx.com](https://www.okx.com) 登录您的 OKX 账户。
2. 导航到 **交易** > **模拟交易**。
3. 进入模拟交易中的 **个人中心**。
4. 选择 **模拟交易 API** 并创建新的 API 密钥。
5. 记下您的模拟 API 密钥、密钥和密码短语。

您可以通过环境变量提供模拟凭证：

```bash
export OKX_API_KEY="your_demo_api_key"
export OKX_API_SECRET="your_demo_api_secret"
export OKX_API_PASSPHRASE="your_demo_passphrase"
```

### 配置

在客户端配置中设置 `environment=OKXEnvironment.DEMO`：

```python
from nautilus_trader.core.nautilus_pyo3 import OKXEnvironment

config = TradingNodeConfig(
    data_clients={
        OKX: OKXDataClientConfig(
            environment=OKXEnvironment.DEMO,
            # ... 其他配置
        ),
    },
    exec_clients={
        OKX: OKXExecClientConfig(
            environment=OKXEnvironment.DEMO,
            # ... 其他配置
        ),
    },
)
```

启用模拟模式后：

- REST API 请求包含 `x-simulated-trading: 1` 请求头。
- WebSocket 连接使用模拟端点（`wspap.okx.com`）。

:::note
模拟 API 密钥与生产密钥是分开的。请通过模拟交易界面为模拟交易创建 API 密钥。生产 API 密钥在模拟模式下不起作用。
:::

## 资金费率

适配器从
[资金费率频道](https://www.okx.com/docs-v5/en/#public-data-websocket-funding-rate-channel)
WebSocket 流接收资金费率数据。OKX 在每条消息中同时提供 `fundingTime` 和 `nextFundingTime`，
适配器将 `interval` 计算为这两个值之间的差。

对于历史资金费率请求，适配器根据
[获取资金费率历史](https://www.okx.com/docs-v5/en/#public-data-rest-api-get-funding-rate-history)
端点返回的连续资金时间戳计算间隔。

## 速率限制

适配器在为 REST 和 WebSocket 调用保持合理默认值的同时，执行 OKX 的逐端点配额限制。

### REST 限制

- 内部全局桶：每秒 250 个请求。
- 端点特定配额见下表，在可用情况下与 OKX 已发布的限制保持一致。

### WebSocket 限制

- 连接建立：每秒 3 个请求（每个 IP）。
- 订阅操作（订阅/取消订阅/登录）：每连接每小时 480 个请求。
- 订单操作桶见下表，在可用情况下与 OKX 已发布的限制保持一致。

| 操作键            | 限制（请求/秒） | 备注                         |
| -------------- | -------- | -------------------------- |
| `order`        | 30       | OKX 每 2 秒 60 个请求。          |
| `cancel`       | 30       | OKX 每 2 秒 60 个请求。          |
| `amend`        | 30       | OKX 每 2 秒 60 个请求。          |
| `batch-order`  | 7        | OKX 每 2 秒 300 个订单，对满批向下取整。 |
| `batch-cancel` | 7        | OKX 每 2 秒 300 个订单，对满批向下取整。 |
| `batch-amend`  | 7        | OKX 每 2 秒 300 个订单，对满批向下取整。 |
| `mass-cancel`  | 2        | OKX 每 2 秒 5 个请求，向下取整。      |
| `algo-order`   | 10       | OKX 每 2 秒 20 个请求。          |
| `algo-cancel`  | 1        | OKX 每 2 秒 20 个订单，对满批向下取整。  |

:::warning
OKX 执行逐端点和逐账户的配额限制。超出限制将导致 HTTP 429 响应和对该密钥的临时限流。
:::

| 键 / 端点                                  | 限制（请求/秒） | 备注                      |
| --------------------------------------- | -------- | ----------------------- |
| `okx:global`                            | 250      | 适配器级别的共享桶。              |
| `/api/v5/account/set-position-mode`     | 2        | OKX 每 2 秒 5 个请求，向下取整。   |
| `/api/v5/account/balance`               | 5        | OKX 每 2 秒 10 个请求。       |
| `/api/v5/account/trade-fee`             | 2        | OKX 每 2 秒 5 个请求，向下取整。   |
| `/api/v5/account/positions`             | 5        | OKX 每 2 秒 10 个请求。       |
| `/api/v5/account/positions-history`     | 5        | OKX 每 2 秒 10 个请求。       |
| `/api/v5/public/instruments`            | 10       | OKX 每 2 秒 20 个请求。       |
| `/api/v5/public/position-tiers`         | 5        | OKX 每 2 秒 10 个请求。       |
| `/api/v5/public/event-contract/series`  | 5        | OKX 每 2 秒 10 个请求。       |
| `/api/v5/public/event-contract/events`  | 5        | OKX 每 2 秒 10 个请求。       |
| `/api/v5/public/event-contract/markets` | 5        | OKX 每 2 秒 10 个请求。       |
| `/api/v5/public/opt-summary`            | 10       | OKX 每 2 秒 20 个请求。       |
| `/api/v5/public/time`                   | 5        | OKX 每 2 秒 10 个请求。       |
| `/api/v5/public/mark-price`             | 5        | OKX 每 2 秒 10 个请求。       |
| `/api/v5/public/funding-rate-history`   | 5        | OKX 每 2 秒 10 个请求。       |
| `/api/v5/market/index-tickers`          | 10       | OKX 每 2 秒 20 个请求。       |
| `/api/v5/market/books`                  | 20       | OKX 每 2 秒 40 个请求。       |
| `/api/v5/market/candles`                | 20       | OKX 每 2 秒 40 个请求。       |
| `/api/v5/market/history-candles`        | 10       | OKX 每 2 秒 20 个请求。       |
| `/api/v5/market/history-trades`         | 10       | OKX 每 2 秒 20 个请求。       |
| `/api/v5/sprd/spreads`                  | 10       | OKX 每 2 秒 20 个请求。       |
| `/api/v5/sprd/order`                    | 10       | OKX 每 2 秒 20 个请求。       |
| `/api/v5/sprd/cancel-order`             | 10       | OKX 每 2 秒 20 个请求。       |
| `/api/v5/sprd/mass-cancel`              | 5        | OKX 每 2 秒 10 个请求。       |
| `/api/v5/sprd/orders-pending`           | 5        | OKX 每 2 秒 10 个请求。       |
| `/api/v5/sprd/orders-history`           | 10       | OKX 每 2 秒 20 个请求。       |
| `/api/v5/sprd/trades`                   | 10       | OKX 每 2 秒 20 个请求。       |
| `/api/v5/trade/order`                   | 30       | OKX 每 2 秒 60 个请求。       |
| `/api/v5/trade/cancel-batch-orders`     | 7        | OKX 每 2 秒 300 个订单，向下取整。 |
| `/api/v5/trade/orders-pending`          | 30       | OKX 每 2 秒 60 个请求。       |
| `/api/v5/trade/orders-history`          | 20       | OKX 每 2 秒 40 个请求。       |
| `/api/v5/trade/fills`                   | 30       | OKX 每 2 秒 60 个请求。       |
| `/api/v5/trade/order-algo`              | 10       | OKX 每 2 秒 20 个请求。       |
| `/api/v5/trade/cancel-algos`            | 1        | OKX 每 2 秒 20 个订单。       |
| `/api/v5/trade/cancel-advance-algos`    | 1        | 高级算法取消的保守配额桶。           |
| `/api/v5/trade/amend-algos`             | 10       | OKX 每 2 秒 20 个请求。       |
| `/api/v5/trade/orders-algo-pending`     | 10       | OKX 每 2 秒 20 个请求。       |
| `/api/v5/trade/orders-algo-history`     | 10       | OKX 每 2 秒 20 个请求。       |

所有键都包含 `okx:global` 桶。URL 在速率限制前会被标准化（移除查询字符串），因此不同过滤条件的请求共享同一配额。

对于基于订单的取消配额，适配器使用假设满批大小的请求级别桶：常规批量取消每请求 20 个订单，
算法取消每请求 10 个订单。OKX 当前的公开文档不再列出 `/api/v5/trade/cancel-advance-algos` 的速率限制，
但适配器仍保留一个端点特定的桶，因为 HTTP 客户端可能调用该遗留路径。

:::info
参阅 [OKX 速率限制文档](https://www.okx.com/docs-v5/en/#rest-api-rate-limit)。
:::

## 配置

### 配置选项

OKX 数据客户端提供以下配置选项：

#### 数据客户端

| 选项                                 | 默认值                         | 描述                           |
| ---------------------------------- | --------------------------- | ---------------------------- |
| `instrument_types`                 | `(OKXInstrumentType.SPOT,)` | 要加载的 OKX 金融工具类型。             |
| `contract_types`                   | `None`                      | 要加载的合约样式。                    |
| `load_spreads`                     | `False`                     | 加载实时价差金融工具。                  |
| `instrument_families`              | `None`                      | 系列或事件 `seriesId` 值。          |
| `base_url_http`                    | `None`                      | OKX REST 端点的覆盖。              |
| `base_url_ws_public`               | `None`                      | 公共 WebSocket URL 的覆盖。        |
| `base_url_ws_business`             | `None`                      | 业务 WebSocket URL 的覆盖。        |
| `api_key`                          | `None`                      | 未设置时回退到 `OKX_API_KEY`。       |
| `api_secret`                       | `None`                      | 未设置时回退到 `OKX_API_SECRET`。    |
| `api_passphrase`                   | `None`                      | 回退到 `OKX_API_PASSPHRASE`。    |
| `environment`                      | `None`                      | 环境枚举（`LIVE` 或 `DEMO`）。       |
| `http_timeout_secs`                | `60`                        | REST 市场数据请求超时时间。             |
| `max_retries`                      | `3`                         | 可恢复 REST 错误的重试次数。            |
| `retry_delay_initial_ms`           | `1,000`                     | 重试前的初始延迟。                    |
| `retry_delay_max_ms`               | `10,000`                    | 最大指数退避延迟。                    |
| `update_instruments_interval_mins` | `60`                        | 后台金融工具刷新间隔。                  |
| `vip_level`                        | `None`                      | 按 VIP 等级启用更深的订单簿。            |
| `proxy_url`                        | `None`                      | 可选的 HTTP 和 WebSocket 代理 URL。 |
| `transport_backend`                | `Sockudo`                   | WebSocket 传输后端。              |

支持的数据客户端 `instrument_types` 取值为 `SPOT`、`MARGIN`、`SWAP`、`FUTURES`、`OPTION` 和 `EVENTS`。

`instrument_families` 对 `OPTION` 为必填，对 `FUTURES`、`SWAP` 和 `EVENTS` 为可选，对 `SPOT` 和 `MARGIN` 则被忽略。
对于 `EVENTS`，传入 OKX `seriesId` 值，例如 `BTC-ABOVE-DAILY`。价差金融工具使用 `load_spreads` 而非
`instrument_types`，因为 OKX 从 `/api/v5/sprd/spreads` 提供它们。

OKX 执行客户端提供以下配置选项：

#### 执行客户端

| 选项                               | 默认值                         | 描述                           |
| -------------------------------- | --------------------------- | ---------------------------- |
| `instrument_types`               | `(OKXInstrumentType.SPOT,)` | 可交易的 OKX 金融工具类型。             |
| `contract_types`                 | `None`                      | 要加载的可交易合约样式。                 |
| `load_spreads`                   | `False`                     | 加载实时价差金融工具。                  |
| `instrument_families`            | `None`                      | 系列或事件 `seriesId` 值。          |
| `base_url_http`                  | `None`                      | OKX 交易 REST 端点的覆盖。           |
| `base_url_ws_private`            | `None`                      | 私有 WebSocket URL 的覆盖。        |
| `base_url_ws_business`           | `None`                      | 业务 WebSocket URL 的覆盖。        |
| `api_key`                        | `None`                      | 未设置时回退到 `OKX_API_KEY`。       |
| `api_secret`                     | `None`                      | 未设置时回退到 `OKX_API_SECRET`。    |
| `api_passphrase`                 | `None`                      | 回退到 `OKX_API_PASSPHRASE`。    |
| `environment`                    | `None`                      | 环境枚举（`LIVE` 或 `DEMO`）。       |
| `margin_mode`                    | `None`                      | 保证金模式（`ISOLATED` 或 `CROSS`）。 |
| `use_spot_margin`                | `False`                     | 启用现货式保证金或杠杆。                 |
| `http_timeout_secs`              | `60`                        | REST 交易请求超时时间。               |
| `use_fills_channel`              | `False`                     | 订阅成交频道（VIP5+）。               |
| `use_mm_mass_cancel`             | `False`                     | 使用做市商批量取消端点。                 |
| `max_retries`                    | `3`                         | 可恢复 REST 错误的重试次数。            |
| `retry_delay_initial_ms`         | `1,000`                     | 重试前的初始延迟。                    |
| `retry_delay_max_ms`             | `10,000`                    | 最大指数退避延迟。                    |
| `use_spot_cash_position_reports` | `False`                     | 从钱包生成 SPOT 现金持仓。             |
| `proxy_url`                      | `None`                      | 可选的 HTTP 和 WebSocket 代理 URL。 |
| `transport_backend`              | `Sockudo`                   | WebSocket 传输后端。              |

支持的执行客户端 `instrument_types` 取值为 `SPOT`、`MARGIN`、`SWAP`、`FUTURES`、`OPTION` 和 `EVENTS`。

`instrument_families` 对执行客户端的含义与数据客户端相同。价差金融工具使用 OKX 价差 ID 而非 `instrument_types`；
在数据客户端上用 `load_spreads=True` 加载它们，对于仅执行的 Python v1 节点，还需在执行客户端上加载后再进行交易。

### EEA 端点覆盖

通过 EEA 门户注册的 OKX 账户使用 EEA API 基础设施。适配器默认使用全球 OKX 端点，
因此 EEA 账户应设置显式的 REST 和 WebSocket 端点覆盖。

| 配置字段                   | Live 基址                    | Demo 基址                       | WebSocket 路径      |
| ---------------------- | -------------------------- | ----------------------------- | ----------------- |
| `base_url_http`        | `https://eea.okx.com`      | `https://eea.okx.com`         |                   |
| `base_url_ws_public`   | `wss://wseea.okx.com:8443` | `wss://wseeapap.okx.com:8443` | `/ws/v5/public`   |
| `base_url_ws_private`  | `wss://wseea.okx.com:8443` | `wss://wseeapap.okx.com:8443` | `/ws/v5/private`  |
| `base_url_ws_business` | `wss://wseea.okx.com:8443` | `wss://wseeapap.okx.com:8443` | `/ws/v5/business` |

对于 WebSocket 字段，将同一行的基址和路径拼接起来。

数据客户端配置使用 `base_url_ws_public`，执行客户端配置使用 `base_url_ws_private`。EEA 账户还必须在两个 v2
配置上设置 `base_url_ws_business`，因为 v2 不会从公共或私有覆盖推导业务 WebSocket URL。

Python v1 实时配置暴露 `base_url_ws` 而非拆分的 WebSocket 字段。对于这些配置，在 `OKXDataClientConfig` 上
将 `base_url_ws` 设为公共 EEA WebSocket URL，在 `OKXExecClientConfig` 上设为私有 EEA WebSocket URL；
每个客户端会从该值推导其业务 WebSocket URL。

当前官方端点列表请参阅 [OKX EEA API 文档](https://my.okx.com/docs-v5/en/)。

以下是使用 OKX 数据和执行客户端的实时交易节点配置示例：

```python
from nautilus_trader.adapters.okx import OKX
from nautilus_trader.adapters.okx import OKXDataClientConfig, OKXExecClientConfig
from nautilus_trader.adapters.okx.factories import (
    OKXLiveDataClientFactory,
    OKXLiveExecClientFactory,
)
from nautilus_trader.config import InstrumentProviderConfig, TradingNodeConfig
from nautilus_trader.core.nautilus_pyo3 import OKXContractType
from nautilus_trader.core.nautilus_pyo3 import OKXEnvironment
from nautilus_trader.core.nautilus_pyo3 import OKXInstrumentType
from nautilus_trader.core.nautilus_pyo3 import OKXMarginMode
from nautilus_trader.live.node import TradingNode

config = TradingNodeConfig(
    ...,
    data_clients={
        OKX: OKXDataClientConfig(
            api_key=None,  # 将使用 OKX_API_KEY 环境变量
            api_secret=None,  # 将使用 OKX_API_SECRET 环境变量
            api_passphrase=None,  # 将使用 OKX_API_PASSPHRASE 环境变量
            base_url_http=None,
            base_url_ws=None,
            environment=OKXEnvironment.LIVE,
            instrument_provider=InstrumentProviderConfig(load_all=True),
            instrument_types=(OKXInstrumentType.SWAP,),
            contract_types=(OKXContractType.LINEAR,),
        ),
    },
    exec_clients={
        OKX: OKXExecClientConfig(
            api_key=None,
            api_secret=None,
            api_passphrase=None,
            base_url_http=None,
            base_url_ws=None,
            environment=OKXEnvironment.LIVE,
            instrument_provider=InstrumentProviderConfig(load_all=True),
            instrument_types=(OKXInstrumentType.SWAP,),
            contract_types=(OKXContractType.LINEAR,),
        ),
    },
)
node = TradingNode(config=config)
node.add_data_client_factory(OKX, OKXLiveDataClientFactory)
node.add_exec_client_factory(OKX, OKXLiveExecClientFactory)
node.build()
```

## 贡献

:::info
如需额外功能或为 OKX 适配器做贡献，请参阅我们的
[贡献指南](https://github.com/nautechsystems/nautilus_trader/blob/develop/CONTRIBUTING.md)。
:::
