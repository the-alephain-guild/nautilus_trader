# Deribit

Deribit 成立于 2016 年，是一家专注于比特币和以太坊期权（Options）与期货（Futures）的加密货币衍生品交易所。
按交易量计算，它是规模最大的加密期权交易所之一，也是加密衍生品交易的领先平台。

此集成（Integration）支持 Deribit 的实时市场数据接入和订单执行。

## 概述 (Overview)

此适配器（Adapter）使用 Rust 实现，并提供可选的 Python 绑定以用于基于 Python 的工作流。
Deribit 通过 HTTP 和 WebSocket 两种传输方式使用 JSON-RPC 2.0 协议。
WebSocket 是订阅和实时数据的首选方式。

官方 Deribit API 参考文档可在 [docs.deribit.com](https://docs.deribit.com/) 查阅。

Deribit 适配器包含多个组件，可根据你的使用场景一起使用或单独使用：

- `DeribitHttpClient`：底层 HTTP API 连接（基于 HTTP 的 JSON-RPC）。
- `DeribitWebSocketClient`：底层 WebSocket API 连接（基于 WebSocket 的 JSON-RPC）。
- `DeribitInstrumentProvider`：金融工具的解析与加载功能。
- `DeribitDataClient`：市场数据源管理器。
- `DeribitExecutionClient`：账户管理与交易执行网关。
- `DeribitLiveDataClientFactory`：Deribit 数据客户端工厂（由交易节点构建器使用）。
- `DeribitLiveExecClientFactory`：Deribit 执行客户端工厂（由交易节点构建器使用）。

:::note
大多数用户只需为实时交易节点定义一份配置（如下文所示），
无需直接操作这些底层组件。
:::

### 产品支持 (Product support)

| 产品类型              | 数据源 | 交易  | 备注                           |
| ----------------- | --- | --- | ---------------------------- |
| Perpetual Futures | ✓   | ✓   | BTC-PERPETUAL、ETH-PERPETUAL。 |
| Dated Futures     | ✓   | ✓   | 具有固定到期日的期货。                  |
| Options           | ✓   | ✓   | BTC 与 ETH 期权。                |
| Spot              | ✓   | ✓   | BTC_USDC、ETH_USDC 交易对。       |
| Future Combos     | ✓   | ✓   | 期货的日历价差（calendar spread）。    |
| Option Combos     | ✓   | ✓   | 期权价差策略。                      |

## 符号体系 (Symbology)

Deribit 针对不同的金融工具类型使用特定的符号约定。
引用所有金融工具 ID 时都应带上 `.DERIBIT` 后缀
（例如，BTC 永续合约为 `BTC-PERPETUAL.DERIBIT`）。

### 永续期货 (Perpetual Futures)

格式：`{Currency}-PERPETUAL`

示例：

- `BTC-PERPETUAL` —— 比特币永续掉期合约
- `ETH-PERPETUAL` —— 以太坊永续掉期合约

在策略中订阅 BTC 永续合约：

```python
InstrumentId.from_str("BTC-PERPETUAL.DERIBIT")
```

### 到期期货 (Dated Futures)

格式：`{Currency}-{DDMMMYY}`

示例：

- `BTC-28MAR25` —— 2025 年 3 月 28 日到期的比特币期货
- `ETH-27JUN25` —— 2025 年 6 月 27 日到期的以太坊期货

```python
InstrumentId.from_str("BTC-28MAR25.DERIBIT")
```

### 期权 (Options)

格式：`{Currency}-{DDMMMYY}-{Strike}-{Type}`

示例：

- `BTC-28MAR25-100000-C` —— 比特币看涨期权，行权价 100,000 美元，2025 年 3 月 28 日到期
- `BTC-28MAR25-80000-P` —— 比特币看跌期权，行权价 80,000 美元，2025 年 3 月 28 日到期
- `ETH-28MAR25-4000-C` —— 以太坊看涨期权，行权价 4,000 美元

其中：

- `C` = 看涨期权（Call）
- `P` = 看跌期权（Put）

```python
InstrumentId.from_str("BTC-28MAR25-100000-C.DERIBIT")
```

### 现货 (Spot)

格式：`{BaseCurrency}_{QuoteCurrency}`

示例：

- `BTC_USDC` —— 比特币兑 USDC
- `ETH_USDC` —— 以太坊兑 USDC

```python
InstrumentId.from_str("BTC_USDC.DERIBIT")
```

### 期货组合 (Future combos)

格式：`{Currency}-FS-{LegA}_{LegB}`

各腿（leg）为到期期货或永续合约（在组合名称中以 `PERP` 表示，尽管独立的金融工具是
`BTC-PERPETUAL`）。组合在其最早到期的腿到期时一同到期。

示例：

- `BTC-FS-25DEC26_PERP` —— 2026 年 12 月期货与永续合约之间的日历价差。
- `BTC-FS-22MAY26_19MAY26` —— 两个到期期货之间的跨月价差。

```python
InstrumentId.from_str("BTC-FS-25DEC26_PERP.DERIBIT")
```

适配器将期货组合建模为 `CryptoFuturesSpread`，以美元计价，价格为各腿之间的价差，
结算货币为加密货币，并根据上游的 `instrument_type` 设置 `is_inverse`。

### 期权组合 (Option combos)

格式：`{Currency}-{Strategy}-{DDMMMYY}-{Strikes}`

策略代码包括 CS（看涨价差，call spread）、PS（看跌价差，put spread）、STRG（宽跨式，strangle）、
STRD（跨式，straddle）、BOX（盒式，box）以及 RR（风险逆转，risk reversal）。strikes 部分用 `_`
分隔多个行权价。

示例：

- `BTC-CS-19MAY26-70000_75000` —— 2026 年 5 月 19 日到期的 70k / 75k 看涨价差。
- `BTC-STRG-29MAY26-72000_80000` —— 72k / 80k 宽跨式。
- `BTC-STRD-29MAY26-77000` —— 77k 跨式。
- `BTC-BOX-25DEC26-58000_60000` —— 58k / 60k 盒式。

```python
InstrumentId.from_str("BTC-STRG-29MAY26-72000_80000.DERIBIT")
```

适配器将期权组合建模为 `CryptoOptionSpread`，在 Deribit 的反向期权（inverse-option）约定下以基础货币计价；
分数形式的 `size_increment`（例如 `0.1`）会被完整保留至端到端。

## 可交易到期日 (Traded expirations)

Deribit 通过 `public/get_expirations` HTTP 端点公开当前活跃的可交易到期日。
期权链加载器可以使用高层 HTTP 客户端刷新活跃的期权系列，而无需扫描每一个金融工具。

```rust tab="Rust"
use nautilus_deribit::http::models::DeribitCurrency;

let expirations = client
    .request_option_expirations(DeribitCurrency::BTC)
    .await?;
```

```python tab="Python"
from nautilus_trader.adapters.deribit import DeribitCurrency
from nautilus_trader.adapters.deribit import DeribitHttpClient

client = DeribitHttpClient()
expirations = await client.request_option_expirations(DeribitCurrency.BTC)
```

该高层方法仅返回期权到期日。如需更底层的 Rust 请求，可使用 `GetExpirationsParams` 调用
`client.inner().get_expirations(...)`。对于诸如 `BTC` 这样的具体货币，Deribit 返回按货币分键的结果；
而对于 `currency=any`，则直接返回按种类（kind）分键的结果——适配器对两种形态都做了处理。

## 组合金融工具 (Combo instruments)

当 `product_types` 包含 `DeribitProductType.OptionCombo` 或 `DeribitProductType.FutureCombo` 时，
金融工具提供器会加载组合。Deribit 在 `/public/get_combos` 上公开每个活跃组合的腿构成，
并在标准的 `/public/get_instruments?kind=option_combo|future_combo` 响应中提供组合的交易元数据
（tick size、合约规模、到期日、最小交易数量）。

### 成交发布 (Trade publishing)

Deribit 会将每笔组合成交发布两次：

- 在组合的成交频道（`trades.{combo_name}.{interval}`）上：包含父级成交以及描述每条腿成交的
  `legs[]` 数组。
- 在每条腿各自的成交频道（`trades.{leg_instrument}.{interval}`）上：该腿的独立成交，
  带有 `combo_id` 和 `combo_trade_id` 标记，指回其父级。

因此，普通期权或期货的订阅者会在其已有的成交流上看到源自组合的成交，
而组合本身的订阅者则会看到组合层级的成交。适配器不会将组合父级消息扇出（fan out）为额外的腿级 tick；
它将上游的父级消息和每条腿的消息作为针对各自 `InstrumentId` 的独立 `TradeTick` 转发，
因此同时订阅组合及某条底层腿的订阅者，对该笔组合成交只会看到一个组合 tick 加一个腿 tick，
而非针对同一金融工具的重复 tick。

要让 Deribit 数据客户端在订阅组合成交的同时打开真实的腿成交频道，
请向 `subscribe_trade_ticks` 传入 `params={"subscribe_combo_legs": True}`。
当取消订阅该组合成交流时，Nautilus 也会一并关闭由此选项打开的腿订阅。

Deribit 本身就会按腿发布大宗交易（block trade）和大宗询价（Block RFQ），因此适配器会将它们
通过标准的 1:1 成交路径转发。关于源自大宗交易和 RFQ 的成交如何在最终的 `TradeTick` 上被标记，
请参见 [成交 ID 溯源](#成交-id-溯源-trade-id-provenance)。

### 历史组合成交 (Historical combo trades)

标准的按金融工具查询成交端点接受组合金融工具名称。如需在一次调用中扫描某一产品种类的全部组合，
可通过 `DeribitHttpClient::inner()` 使用 `get_last_trades_by_currency`：

```rust
use nautilus_deribit::http::{
    models::{DeribitCurrency, DeribitProductType},
    query::GetLastTradesByCurrencyParams,
};

let params = GetLastTradesByCurrencyParams::builder()
    .currency(DeribitCurrency::BTC)
    .kind(DeribitProductType::FutureCombo)
    .count(50_u32)
    .include_old(true)
    .build()?;
let resp = client.inner().get_last_trades_by_currency(params).await?;
```

每个返回的 `DeribitPublicTrade` 都携带 `legs: Option<Vec<DeribitTradeLeg>>`，
以及用于关联每条腿成交的 `combo_id` 和 `combo_trade_id` 字段。

## 成交 ID 溯源 (Trade ID provenance)

当成交源自大宗询价（Block RFQ）、大宗交易（block trade）或组合时，适配器发出的公共 `TradeTick`
会在场内成交 ID 前加上前缀。需要将这些成交与普通成交区分开的策略，可对 `TradeTick.trade_id`
做前缀模式匹配。原始的 Deribit `trade_id` 会保留在前缀之后，因此与 Deribit 自身 ID 的对账
只需去掉前缀即可。

| 前缀       | 来源字段             | 含义                   |
| -------- | ---------------- | -------------------- |
| `RFQ-`   | `block_rfq_id`   | 成交源自大宗询价（Block RFQ）。 |
| `BLK-`   | `block_trade_id` | 成交为（非 RFQ 的）大宗交易。    |
| `COMBO-` | `combo_id`       | 父级源自组合金融工具的腿级成交。     |
| *无前缀*    | （以上均非）           | 标准成交。                |

当存在多个标记时的优先级：`RFQ-` > `BLK-` > `COMBO-`。在 Deribit 上，大宗询价本身也是大宗交易，
因此 RFQ 标记优先；以大宗交易形式执行的组合会被标记为 `BLK-`，因为大宗流是更重要的对账信号。

这仅适用于公共成交（`TradeTick`）。`FillReport.trade_id` 保持不变，因此与 `get_user_trades_*`
的对账仍可正常工作。

:::note
这是一种单向约定。在此版本之前捕获的回放数据没有前缀。
在不同版本之间存储并比较 `trade_id` 字符串的策略，应在新数据一侧去掉前缀，
或仅对已知是升级后捕获的数据按前缀进行过滤。
:::

## 订单簿订阅 (Order book subscriptions)

Deribit 提供两种类型的订单簿数据源，各自适用于不同的使用场景。

### 原始数据源（逐笔）(Raw feeds, tick-by-tick)

原始（raw）频道将每一次更新作为单独的消息推送。订阅原始订单簿后，
你会在订单簿中每一次订单插入、更新或删除时都收到通知。

- 需要经过认证的连接（防止滥用的保护措施）。
- 当你在高频交易（HFT）或做市中需要每一次价格档位变动时使用。
- 消息量更高。

### 聚合数据源（批量）(Aggregated feeds, batched)

聚合（aggregated）频道按固定间隔（例如每 100ms）批量推送更新。
这会将多次订单簿变动合并到单条消息中。

- 无需认证即可使用。
- 推荐用于大多数使用场景。
- 消息量更低，更易于处理。
- 默认间隔：100ms。

### 订阅参数 (Subscription parameters)

Nautilus 适配器通过订阅参数同时支持两种数据源类型：

| 参数         | 取值                   | 备注                                          |
| ---------- | -------------------- | ------------------------------------------- |
| `interval` | `raw`、`100ms`、`agg2` | 默认：`100ms`。`agg2` 以约 1 秒的间隔批量推送。`raw` 需要认证。 |
| `depth`    | `1`、`10`、`20`        | 默认：`10`。每一侧的价格档位数量。                         |

```python
from nautilus_trader.model.identifiers import InstrumentId

instrument_id = InstrumentId.from_str("BTC-PERPETUAL.DERIBIT")

# Default: 100ms aggregated feed (no authentication required)
strategy.subscribe_order_book_deltas(instrument_id)

# Raw feed (requires API credentials)
strategy.subscribe_order_book_deltas(
    instrument_id,
    params={"interval": "raw"},
)
```

:::note
原始订单簿数据源需要经过认证的 WebSocket 连接。在订阅原始数据源之前，请确保已配置 API 凭据。
:::

:::tip
对大多数策略而言，默认的 100ms 聚合数据源在显著降低消息开销的同时已能提供足够的粒度。
仅在确实需要逐笔精度时才使用原始数据源。
:::

### 序列缺口恢复 (Sequence gap recovery)

适配器会在每一次订单簿更新上跟踪 `change_id` / `prev_change_id` 序列号。
当检测到缺口（消息丢失）时，适配器会自动执行以下操作：

1. 丢弃受影响金融工具的所有入站增量（delta）。
2. 取消订阅该订单簿频道。
3. 重新订阅以获取新的快照。
4. 快照到达后恢复正常处理。

在重新同步期间，策略不会收到过期或不完整的订单簿更新。

## 订单能力 (Orders capability)

下面列出 Deribit 支持的订单类型、执行指令以及有效期（time-in-force）选项。

### 订单类型 (Order types)

| 订单类型          | 是否支持 | 备注            |
| ------------- | ---- | ------------- |
| `MARKET`      | ✓    | 以市场价格立即执行。    |
| `LIMIT`       | ✓    | 以指定价格或更优价格执行。 |
| `STOP_MARKET` | ✓    | 触发后转为市价的条件单。  |
| `STOP_LIMIT`  | ✓    | 触发后转为限价的条件单。  |

### 执行指令 (Execution instructions)

| 指令            | 是否支持 | 备注                                        |
| ------------- | ---- | ----------------------------------------- |
| `post_only`   | ✓    | 若订单会吃掉流动性则被拒绝。使用 `reject_post_only=true`。 |
| `reduce_only` | ✓    | 订单只能减少现有持仓。                               |

### 有效期 (Time in force)

| 有效期   | 是否支持 | 备注                                   |
| ----- | ---- | ------------------------------------ |
| `GTC` | ✓    | 撤单前一直有效（`good_til_cancelled`）。       |
| `GTD` | ✓    | 当日有效——在 UTC 8:00 到期（`good_til_day`）。 |
| `IOC` | ✓    | 立即成交否则取消（`immediate_or_cancel`）。     |
| `FOK` | ✓    | 全部成交否则取消（`fill_or_kill`）。            |

:::note
**Deribit 上的 GTD**：与其他允许 GTD 接受任意到期时间的交易所不同，
Deribit 的 `good_til_day` 始终在当日或次日的 UTC 8:00 到期。自定义到期时间会被记录为警告，
订单将采用交易所固定的到期行为。
:::

### 触发类型 (Trigger types)

条件单（止损单）支持不同的触发价格来源：

| 触发类型          | 是否支持 | 备注                  |
| ------------- | ---- | ------------------- |
| `last_price`  | ✓    | 使用最新成交价（默认）。        |
| `mark_price`  | ✓    | 使用标记价格（mark price）。 |
| `index_price` | ✓    | 使用底层指数价格。           |

```python
# Example: Stop loss using mark price trigger
stop_order = order_factory.stop_market(
    instrument_id=instrument_id,
    order_side=OrderSide.SELL,
    quantity=Quantity.from_str("0.1"),
    trigger_price=Price.from_str("45000.0"),
    trigger_type=TriggerType.MARK_PRICE,  # Use mark price for trigger
)
strategy.submit_order(stop_order)
```

### 批量操作 (Batch operations)

| 操作   | 是否支持 | 备注      |
| ---- | ---- | ------- |
| 批量提交 | -    | *尚未实现*。 |
| 批量修改 | -    | *尚未实现*。 |
| 批量取消 | -    | *尚未实现*。 |

### Post-only 行为 (Post-only behavior)

Deribit 提供两种 post-only 模式：

1. **价格调整（Deribit 默认）**：如果一个 post-only 订单会穿越价差并成交，
   Deribit 会自动将价格调整到价差内的一个 tick。
2. **拒绝模式**：如果订单会穿越价差，则立即被拒绝。

为获得确定性行为，Nautilus 适配器使用**拒绝模式**（`reject_post_only=true`）。
如果一个 post-only 订单会吃掉流动性，它会以错误码 `11054` 被拒绝，并发出一个
`due_post_only` 标志设为 `true` 的 `OrderRejected` 事件。

这使得策略能够区分：

- 因 post-only 违规（试图吃掉流动性）而被拒绝的订单。
- 因其他原因（保证金不足、价格无效等）而被拒绝的订单。

### 订单修改 (Order modification)

适配器使用 Deribit 原生的 `private/edit` 端点，而非取消并重下（cancel-and-replace）。
这带来若干优势：

| 优势      | 说明                    |
| ------- | --------------------- |
| 单次请求    | 执行更快、延迟低于「取消 + 重新下单」。 |
| 保留队列优先级 | 仅减少数量或保持同一价格时维持原有位置。  |
| 保留成交历史  | 部分成交仍与同一订单 ID 关联。     |

**队列优先级规则：**

- **仅减少数量**：保留队列位置。
- **同一价格**：保留队列位置。
- **增加数量或修改价格**：丢失队列位置（视为新订单）。

### 持仓管理 (Position management)

| 功能    | 是否支持 | 备注                              |
| ----- | ---- | ------------------------------- |
| 查询持仓  | ✓    | 实时持仓更新。                         |
| 持仓模式  | -    | Deribit 仅使用净持仓（net position）模式。 |
| 杠杆控制  | -    | 杠杆在账户层级通过 UI 设置。                |
| 保证金模式 | -    | 通过 Deribit UI 设置使用组合保证金。        |

### 订单查询 (Order querying)

| 功能     | 是否支持 | 备注        |
| ------ | ---- | --------- |
| 查询未结订单 | ✓    | 列出所有活跃订单。 |
| 查询订单历史 | ✓    | 历史订单数据。   |
| 订单状态更新 | ✓    | 实时订单状态变化。 |
| 成交历史   | ✓    | 执行与成交报告。  |

### 关联订单 (Contingent orders)

| 功能           | 是否支持 | 备注           |
| ------------ | ---- | ------------ |
| 订单列表         | -    | *不支持*。       |
| OCO 订单       | -    | *不支持*。       |
| 括号单（Bracket） | -    | *不支持*。       |
| 条件单          | ✓    | 止损市价单与止损限价单。 |

### 强平处理 (Liquidation handling)

Deribit 会标记任何由强制平仓（liquidation）触发的成交。在 `user.trades` 数据流和
`private/get_user_trades_*` 端点上，可选的 `liquidation` 字段指示哪一侧正在被强平：

| 取值     | 含义             |
| ------ | -------------- |
| `"M"`  | 做市方（Maker）被强平。 |
| `"T"`  | 吃单方（Taker）被强平。 |
| `"MT"` | 双方都被强平。        |
| 缺省     | 正常（非强平）成交。     |

适配器会为每一笔带强平标记的成交记录一条警告，包含金融工具、成交 ID、订单 ID 和被强平的一侧，
随后通过正常流程发出 `FillReport`。Deribit 不运行与「强平 + 保险基金 / 组合保证金」流程相区别的
ADL（自动减仓）机制，因此没有单独的 ADL 信号需要暴露。

上游参考：

- [`user.trades.{instrument_name}.{interval}` 频道](https://docs.deribit.com/#user-trades-instrument_name-interval)
- [强平文档](https://support.deribit.com/hc/en-us/articles/25944769313309-Liquidations)

## 资金费率 (Funding rates)

Deribit 持续（每隔几秒）结算资金费用，而非像大多数其他交易所那样按固定间隔结算。
对于 Deribit，`FundingRateUpdate` 上的 `interval` 字段为 `None`，因为这种连续模型
无法映射到一个离散周期。

## Deribit 专属数据 (Deribit specific data)

适配器会从 Deribit 的 `deribit_volatility_index.{index_name}` WebSocket 频道发出
`DeribitVolatilityIndex` 自定义数据。Deribit 提供诸如 `btc_usd` 和 `eth_usd` 之类的
波动率指数数据流。

| 字段           | 类型      | 说明                            |
| ------------ | ------- | ----------------------------- |
| `index_name` | `str`   | Deribit 波动率指数名称，例如 `btc_usd`。 |
| `volatility` | `float` | 当前波动率指数值。                     |
| `ts_event`   | `int`   | 更新发生时的 UNIX 时间戳（纳秒）。          |
| `ts_init`    | `int`   | 对象构建时的 UNIX 时间戳（纳秒）。          |

在 actor 或策略中使用 `DataType(DeribitVolatilityIndex)` 进行订阅。
`index_name` 元数据键是必需的：

```python
from nautilus_trader.adapters.deribit.constants import DERIBIT_CLIENT_ID
from nautilus_trader.adapters.deribit.data import DeribitVolatilityIndex
from nautilus_trader.model.data import DataType

self.subscribe_data(
    data_type=DataType(DeribitVolatilityIndex, metadata={"index_name": "btc_usd"}),
    client_id=DERIBIT_CLIENT_ID,
)
```

## 速率限制 (Rate limiting)

Deribit 使用基于额度（credit）的速率限制系统。每个 API 请求都会消耗额度，而额度会随时间补充。
适配器会强制执行这些配额，以防止违反速率限制。

### REST 限制 (REST limits)

| 桶 / 键             | 限制             | 备注                   |
| ----------------- | -------------- | -------------------- |
| `deribit:global`  | 20 次/秒（突发 100） | 所有 REST 请求的默认桶。      |
| `deribit:orders`  | 5 次/秒（突发 20）   | 撮合引擎操作（买入、卖出、修改、取消）。 |
| `deribit:account` | 5 次/秒          | 账户信息端点。              |

### WebSocket 限制 (WebSocket limits)

| 操作      | 限制           | 备注                         |
| ------- | ------------ | -------------------------- |
| 订阅/取消订阅 | 3 次/秒（突发 10） | 订阅操作。                      |
| 订单操作    | 5 次/秒（突发 20） | 通过 WebSocket 的买入、卖出、修改、取消。 |

:::note
Nautilus 适配器使用 WebSocket（而非 REST）提交订单以获得更低的延迟。
订单操作由 `DERIBIT_WS_ORDER_QUOTA`（5 次/秒，突发 20）进行速率限制。
:::

### 基于额度系统的详细说明 (Credit-based system details)

Deribit 使用一套精巧的、基于额度的速率限制系统，额度会以固定速率持续补充。
每一秒，额度都会「滴入」（drip）回你的子账户的额度池中。

**非撮合引擎请求：**

| 参数     | 取值          | 备注               |
| ------ | ----------- | ---------------- |
| 每次请求成本 | 500 额度      | 每次 API 调用都会消耗额度。 |
| 最大池容量  | 50,000 额度   | 允许 100 次请求的突发。   |
| 补充速率   | 10,000 额度/秒 | 约 20 次/秒的持续请求。   |

**撮合引擎请求（默认档位）：**

| 参数   | 取值    | 备注          |
| ---- | ----- | ----------- |
| 持续速率 | 5 次/秒 | 持续速率限制。     |
| 突发容量 | 20 次  | 触发限流前的最大突发。 |

做市商和高交易量交易者可根据 7 天交易量档位获得更高的撮合引擎限制。

Nautilus 适配器使用令牌桶（token bucket）速率限制器实现这一机制，其配置为：

- `DERIBIT_HTTP_REST_QUOTA`：20 次/秒，突发 100（非撮合 REST）
- `DERIBIT_HTTP_ORDER_QUOTA`：5 次/秒，突发 20（撮合引擎 REST）
- `DERIBIT_WS_ORDER_QUOTA`：5 次/秒，突发 20（撮合引擎 WebSocket）
- `DERIBIT_WS_SUBSCRIPTION_QUOTA`：3 次/秒，突发 10（订阅/取消订阅）

更多详情，请参见 [速率限制文章](https://support.deribit.com/hc/en-us/articles/25944617523357-Rate-Limits)。

:::warning
当你超出允许的配额时，Deribit 会返回错误码 `10028`（too_many_requests）。
反复违反可能导致暂时性限流。
:::

## 连接管理 (Connection management)

### 平台限制 (Platform limits)

| 限制                   | 取值  |
| -------------------- | --- |
| 每个 IP 的最大连接数         | 32  |
| 每个 API key 的最大会话数    | 16  |
| 每个（子）账户的最大 API key 数 | 8   |

### 基于会话的认证 (Session-based authentication)

适配器为数据客户端和执行客户端使用**独立的 WebSocket 会话**，每个会话都有各自的认证范围：

| 客户端   | 会话名称                 | 用途                 |
| ----- | -------------------- | ------------------ |
| 数据客户端 | `nautilus-data`      | 市场数据订阅（原始数据源需要认证）。 |
| 执行客户端 | `nautilus-execution` | 订单操作（买入、卖出、修改、取消）。 |

**认证流程：**

1. WebSocket 连接到 Deribit。
2. 客户端使用 `client_signature` 授权类型并带会话范围进行认证。
3. 令牌（token）在到期时间的 80% 处自动刷新（持续刷新循环）。
4. 重新连接时，重新认证会以指数退避（exponential backoff）重试（最多 3 次）。
   若所有尝试都失败，则仅恢复公共频道的订阅。

这种基于会话的方式带来：

- 按客户端类型独立管理令牌。
- 隔离的故障域（数据认证失败不影响执行）。
- 在 Deribit 的会话日志中留下清晰的审计轨迹。

### 最佳实践 (Best practices)

适配器遵循 Deribit 的
[推荐连接实践](https://support.deribit.com/hc/en-us/articles/25944603459613)：

1. **使用 WebSocket 订阅**获取实时数据，而非 REST 轮询，从而减少请求数、降低延迟并减少速率限制消耗。
2. **在提供凭据时认证所有连接**。已认证用户可享受更高的速率限制，并且更不容易被 IP 限流。
3. **实现心跳**（30 秒间隔）以维持连接健康并尽早检测断连。
4. **自动处理重连**，并伴随重新认证和订阅恢复。

:::tip
即使仅用于访问公共数据，也应始终提供 API 凭据。已认证连接拥有更高的速率限制，
并且在高负载期间 Deribit 会先联系已认证客户端再施加限制。
:::

:::note
适配器使用 30 秒的心跳间隔，这是 Deribit 推荐的 30-60 秒区间的下限。
更频繁的心跳可能触发更严格的速率限制。
:::

## 认证 (Authentication)

Deribit 对私有端点使用基于 API key 的认证，并采用 HMAC-SHA256 签名。

创建 API 凭据：

1. 登录你在 [deribit.com](https://www.deribit.com)（或测试网 [test.deribit.com](https://test.deribit.com)）的 Deribit 账户。
2. 导航至 **Account** -> **API**。
3. 点击 **Add new key** 并配置权限：
   - 启用 **read** 以访问市场数据
   - 启用 **trade** 以执行订单
   - 如需访问账户余额，启用 **wallet**
4. 记下你的 **Client ID**（API key）和 **Client Secret**（API secret）。

:::warning
妥善保管你的 API secret。切勿分享，也不要将其提交到版本控制中。
:::

### API key 范围 (API key scopes)

Deribit 上的每个 API key 都被分配了一个默认访问范围（scope），用于定义其最大权限。
在 [创建 API key](https://support.deribit.com/hc/en-us/articles/26268257333661) 时配置合适的权限：

| 范围                 | 所需用途        |
| ------------------ | ----------- |
| `account:read`     | 账户信息、组合数据。  |
| `trade:read`       | 查看订单和持仓。    |
| `trade:read_write` | 下单、修改和取消订单。 |
| `wallet:read`      | 查看余额和交易历史。  |

**交易所需的推荐最小范围：** `account:read`、`trade:read_write`、`wallet:read`

:::tip
遵循最小权限原则。对于仅访问数据（市场数据，不交易）的场景，
请创建一个不带 `trade:read_write` 的只读 key。
:::

## 测试网 (Testnet)

Deribit 提供一个测试网（testnet）环境，可在不使用真实资金的情况下测试策略。
要使用测试网，请在客户端配置中设置 `environment=DeribitEnvironment.TESTNET`：

```python
from nautilus_trader.core.nautilus_pyo3 import DeribitEnvironment

config = TradingNodeConfig(
    data_clients={
        DERIBIT: DeribitDataClientConfig(
            environment=DeribitEnvironment.TESTNET,
            # ... other config
        ),
    },
    exec_clients={
        DERIBIT: DeribitExecClientConfig(
            environment=DeribitEnvironment.TESTNET,
            # ... other config
        ),
    },
)
```

启用测试网模式时：

- HTTP 请求使用 `https://test.deribit.com`。
- WebSocket 连接使用 `wss://test.deribit.com/ws/api/v2`。
- 从 `DERIBIT_TESTNET_API_KEY` 和 `DERIBIT_TESTNET_API_SECRET` 环境变量加载凭据。

:::note
测试网 API key 与生产 key 是分开的。请通过 [test.deribit.com](https://test.deribit.com) 的测试网界面
专门为测试网创建 API key。
:::

## 配置 (Configuration)

### 数据客户端配置选项 (Data client configuration options)

| 选项                                 | 默认值       | 说明                                                              |
| ---------------------------------- | --------- | --------------------------------------------------------------- |
| `api_key`                          | `None`    | Deribit API key；省略时从环境变量加载。                                     |
| `api_secret`                       | `None`    | Deribit API secret；省略时从环境变量加载。                                  |
| `product_types`                    | `None`    | 要加载的产品类型（Future、Option、Spot 等）。若为 `None`，默认为 Future。            |
| `environment`                      | `None`    | 环境枚举（`MAINNET` 或 `TESTNET`）。                                    |
| `base_url_http`                    | `None`    | 覆盖 HTTP REST 基础 URL。                                            |
| `base_url_ws`                      | `None`    | 覆盖 WebSocket 基础 URL。                                            |
| `proxy_url`                        | `None`    | HTTP 和 WebSocket 传输的可选代理 URL。                                   |
| `http_timeout_secs`                | `60`      | REST 调用的请求超时（秒）。                                                |
| `max_retries`                      | `3`       | 可恢复错误的最大重试次数。                                                   |
| `retry_delay_initial_ms`           | `1,000`   | 重试前的初始延迟（毫秒）。                                                   |
| `retry_delay_max_ms`               | `10,000`  | 重试之间的最大延迟（毫秒）。                                                  |
| `update_instruments_interval_mins` | `60`      | 金融工具刷新之间的间隔（分钟）。                                                |
| `auto_load_missing_instruments`    | `False`   | 在订阅时惰性加载未缓存的金融工具；参见 [订阅时惰性加载](#订阅时惰性加载-lazy-load-on-subscribe)。 |
| `transport_backend`                | `Sockudo` | WebSocket 传输后端。                                                 |

#### 订阅时惰性加载 (Lazy-load on subscribe)

`subscribe_*` 命令会在发送 WebSocket 订阅之前先在本地缓存中查找金融工具，
以便处理器能够解析入站帧。在 `auto_load_missing_instruments = False`（默认）的情况下，
对一个未被预加载的金融工具（因为配置的 `product_types`）发起订阅会直接返回错误，
而不是悄然成功却在处理器处丢弃后续的帧。

将 `auto_load_missing_instruments = True` 设置为 `True`，则会改为在首次订阅时通过 HTTP 获取该金融工具，
为 WebSocket 处理器缓存填充数据，然后再转发订阅。HTTP 失败会被记录，并跳过该 WebSocket 订阅。

### 执行客户端配置选项 (Execution client configuration options)

| 选项                       | 默认值       | 说明                                                   |
| ------------------------ | --------- | ---------------------------------------------------- |
| `api_key`                | `None`    | Deribit API key；省略时从环境变量加载。                          |
| `api_secret`             | `None`    | Deribit API secret；省略时从环境变量加载。                       |
| `product_types`          | `None`    | 要加载的产品类型（Future、Option、Spot 等）。若为 `None`，默认为 Future。 |
| `environment`            | `None`    | 环境枚举（`MAINNET` 或 `TESTNET`）。                         |
| `base_url_http`          | `None`    | 覆盖 HTTP REST 基础 URL。                                 |
| `base_url_ws`            | `None`    | 覆盖 WebSocket 基础 URL。                                 |
| `proxy_url`              | `None`    | HTTP 和 WebSocket 传输的可选代理 URL。                        |
| `http_timeout_secs`      | `60`      | REST 调用的请求超时（秒）。                                     |
| `max_retries`            | `3`       | 可恢复错误的最大重试次数。                                        |
| `retry_delay_initial_ms` | `1,000`   | 重试前的初始延迟（毫秒）。                                        |
| `retry_delay_max_ms`     | `10,000`  | 重试之间的最大延迟（毫秒）。                                       |
| `transport_backend`      | `Sockudo` | WebSocket 传输后端。                                      |

### 生产配置 (Production configuration)

下面是一个使用 Deribit 数据客户端和执行客户端的实时交易节点配置示例：

```python
from nautilus_trader.adapters.deribit import DERIBIT
from nautilus_trader.adapters.deribit import DeribitDataClientConfig
from nautilus_trader.adapters.deribit import DeribitExecClientConfig
from nautilus_trader.adapters.deribit import DeribitLiveDataClientFactory
from nautilus_trader.adapters.deribit import DeribitLiveExecClientFactory
from nautilus_trader.config import InstrumentProviderConfig
from nautilus_trader.config import TradingNodeConfig
from nautilus_trader.core.nautilus_pyo3 import DeribitEnvironment
from nautilus_trader.core.nautilus_pyo3 import DeribitProductType
from nautilus_trader.live.node import TradingNode

config = TradingNodeConfig(
    ...,  # Omitted
    data_clients={
        DERIBIT: DeribitDataClientConfig(
            api_key=None,  # Uses DERIBIT_API_KEY env var
            api_secret=None,  # Uses DERIBIT_API_SECRET env var
            product_types=(DeribitProductType.Future,),
            environment=DeribitEnvironment.MAINNET,
            instrument_provider=InstrumentProviderConfig(load_all=True),
        ),
    },
    exec_clients={
        DERIBIT: DeribitExecClientConfig(
            api_key=None,
            api_secret=None,
            product_types=(DeribitProductType.Future,),
            environment=DeribitEnvironment.MAINNET,
            instrument_provider=InstrumentProviderConfig(load_all=True),
        ),
    },
)

node = TradingNode(config=config)
node.add_data_client_factory(DERIBIT, DeribitLiveDataClientFactory)
node.add_exec_client_factory(DERIBIT, DeribitLiveExecClientFactory)
node.build()
```

### API 凭据 (API credentials)

向 Deribit 客户端提供凭据有多种方式。
你可以将对应的值传递给配置对象，或设置以下环境变量：

Deribit 实时（生产）客户端：

- `DERIBIT_API_KEY`
- `DERIBIT_API_SECRET`

Deribit 测试网客户端：

- `DERIBIT_TESTNET_API_KEY`
- `DERIBIT_TESTNET_API_SECRET`

:::tip
我们建议使用环境变量来管理你的凭据。
:::

### 产品类型 (Product types)

`product_types` 配置选项控制加载哪些 Deribit 产品族。
通过 `DeribitProductType` 枚举可用的选项：

- `DeribitProductType.Future` —— 永续期货和到期期货。
- `DeribitProductType.Option` —— 看涨期权和看跌期权。
- `DeribitProductType.Spot` —— 现货交易对。
- `DeribitProductType.FutureCombo` —— 期货价差金融工具。
- `DeribitProductType.OptionCombo` —— 期权价差金融工具。

加载多种产品类型的示例：

```python
from nautilus_trader.core.nautilus_pyo3 import DeribitProductType

config = DeribitDataClientConfig(
    product_types=(
        DeribitProductType.Future,
        DeribitProductType.Option,
    ),
    # ... other config
)
```

### 基础 URL 覆盖 (Base URL overrides)

可以为 HTTP 和 WebSocket API 覆盖默认的基础 URL：

| 环境  | HTTP URL                   | WebSocket URL                      |
| --- | -------------------------- | ---------------------------------- |
| 生产  | `https://www.deribit.com`  | `wss://www.deribit.com/ws/api/v2`  |
| 测试网 | `https://test.deribit.com` | `wss://test.deribit.com/ws/api/v2` |

## 服务器基础设施 (Server infrastructure)

Deribit 的撮合引擎位于 **Equinix LD4，Slough，英国**。对于延迟敏感的策略，
可考虑在伦敦或其附近托管。Deribit 直接面向机构客户提供托管（colocation）和交叉连接（cross-connect）选项。

对于大多数通过互联网连接的用户，适配器内置的重试逻辑、心跳监控和自动重连处理
能够提供可靠的连接。

更多详情，请参见 [服务器基础设施文章](https://support.deribit.com/hc/en-us/articles/25944617582877)。

## 贡献 (Contributing)

:::info
如需更多功能或为 Deribit 适配器做贡献，请参阅我们的
[贡献指南](https://github.com/nautechsystems/nautilus_trader/blob/develop/CONTRIBUTING.md)。
:::
