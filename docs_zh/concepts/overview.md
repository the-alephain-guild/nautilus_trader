# 概述 (Overview)

## 简介

NautilusTrader 是一个开源、生产级、Rust 原生的引擎，面向多资产、多交易场所 (multi-venue) 的交易系统。

该系统在单一的事件驱动 (event-driven) 架构中横跨研究、确定性模拟和实盘执行三个环节，其中 Python 充当策略逻辑、配置和编排的控制平面 (control plane)。

这种分离既提供了编译型交易引擎的高性能与安全性，又保留了 Python 在系统组合和策略开发上的灵活性。交易系统也可以完全用 Rust 编写，以满足关键任务级的工作负载。

相同的执行语义和确定性时间模型在研究系统和实盘系统中保持一致运行。策略从研究环境部署到生产环境无需修改任何代码，从而实现研究到实盘的对等性 (research-to-live parity)，并减少通常会带来部署风险的差异。

NautilusTrader 与资产类别无关。任何具备 REST API 或 WebSocket 数据流的交易场所都可以通过模块化的适配器 (adapter) 进行集成。当前的集成涵盖加密货币交易所（CEX 和 DEX）、传统市场（外汇、股票、期货、期权）以及博彩交易所。

## 特性

- **快速**: Rust 核心，采用 [tokio](https://crates.io/crates/tokio) 实现异步网络通信。
- **可靠**: 由 Rust 支撑的类型安全和线程安全，并可选 Redis 支持的状态持久化。
- **可移植**: 可在 Linux、macOS 和 Windows 上运行。支持 Docker 部署。
- **灵活**: 模块化适配器可集成任何 REST API 或 WebSocket 数据流。
- **高级**: 有效期类型 (Time in force) `IOC`、`FOK`、`GTC`、`GTD`、`DAY`、`AT_THE_OPEN`、`AT_THE_CLOSE`，高级订单 (order) 类型和条件触发器。执行 (execution) 指令 `post-only`、`reduce-only` 和冰山订单。关联订单包括 `OCO`、`OUO`、`OTO`。
- **可定制**: 用户自定义组件 (component)，或利用[缓存 (cache)](cache.md) 和[消息总线 (message bus)](message_bus.md) 从零组装完整系统。
- **回测**: 使用历史报价 (quote) Tick、成交 (trade) Tick、K线 (Bar)、订单簿和自定义数据，支持多交易场所、多金融工具 (instrument) 和多策略的纳秒级精度同步运行。
- **实盘**: 研究和实盘部署之间使用完全相同的策略实现。
- **多交易场所**: 可同时在多个交易场所上运行做市和跨场所策略。
- **AI 训练**: 引擎速度足够快，可用于训练 AI 交易代理 (RL/ES)。

## 为什么选择 NautilusTrader？

交易策略研究通常在 Python 中使用向量化方法进行，而生产交易系统则使用编译型语言中的事件驱动架构单独构建。

NautilusTrader 消除了这种分离。

Rust 原生核心为研究和实盘执行同时提供了确定性的事件驱动运行时，而 Python 充当控制平面。相同的架构、执行语义和时间模型在两个环境中运行，使策略无需重新实现即可从研究环境迁移到生产环境。

Python 绑定通过 [PyO3](https://pyo3.rs) 提供，并正在持续从 Cython 迁移。安装时无需 Rust 工具链。

## 使用场景

该软件包有三个主要使用场景：

- 在历史数据上回测交易系统（`backtest`）。
- 使用实时数据和虚拟执行模拟交易系统（`sandbox`）。
- 将交易系统部署到真实账户或纸上交易账户进行实盘交易（`live`）。

项目代码库提供了实现上述功能的软件层框架。默认的 `backtest` 和 `live` 系统实现位于各自命名的子包中。`sandbox` 环境可以使用 sandbox 适配器构建。

:::note

- 所有示例都将使用这些默认的系统实现。
- 我们认为交易策略是端到端交易系统的子组件，这些系统包括应用层和基础设施层。

:::

## 分布式

该平台可集成到更大的分布式系统中。
几乎所有配置 (configuration) 和领域对象都可以使用 JSON、MessagePack 或 Apache Arrow (Feather) 进行序列化 (serialization)，以便通过网络通信。

## 公共核心

公共系统核心被所有节点 (node) [环境上下文](architecture.md#environment-contexts)（`backtest`、`sandbox` 和 `live`）所使用。
用户定义的 `Actor`、`Strategy` 和 `ExecAlgorithm` 组件在这些环境上下文中被一致地管理。

## 回测

将数据直接提供给 `BacktestEngine`，或通过更高层级的 `BacktestNode` 和 `ParquetDataCatalog` 提供数据，然后以纳秒级精度将数据通过系统运行。

## 实盘交易

`TradingNode` 从多个数据和执行客户端接收数据和事件，支持演示/纸上交易账户和真实账户。通过在单个[事件循环](https://docs.python.org/3/library/asyncio-eventloop.html)上异步运行可以实现高性能，
还可以选择使用 [uvloop](https://github.com/MagicStack/uvloop) 实现（适用于 Linux 和 macOS）以获得额外的吞吐量。

## 领域模型

该平台具有一个交易领域模型，包括各种值类型（如 `Price` 和 `Quantity`），以及更复杂的实体（如 `Order` 和 `Position` 对象），这些对象用于聚合多个事件以确定状态。

## 时间戳

平台内所有时间戳均以 UTC 纳秒精度记录。

时间戳字符串遵循 ISO 8601 (RFC 3339) 格式，具有 9 位（纳秒）或 3 位（毫秒）的小数精度，（但主要使用纳秒），始终保留所有数字包括尾随零。
这些可以在日志 (logging) 消息和对象的调试/显示输出中看到。

时间戳字符串由以下部分组成：

- 完整日期组件始终存在：`YYYY-MM-DD`。
- 日期和时间组件之间使用 `T` 分隔符。
- 始终使用纳秒精度（9 位小数），或在某些情况下（如 GTD 过期时间）使用毫秒精度（3 位小数）。
- 始终使用 UTC 时区，以 `Z` 后缀标识。

示例：`2024-01-05T15:30:45.123456789Z`

有关完整规范，请参阅 [RFC 3339: Date and Time on the Internet](https://datatracker.ietf.org/doc/html/rfc3339)。

## UUID

平台使用通用唯一标识符 (UUID) 第 4 版 (RFC 4122) 作为唯一标识符。
我们的高性能实现利用 `uuid` crate 在从字符串解析时进行正确性验证，确保输入的 UUID 符合规范。

有效的 UUID v4 由以下部分组成：

- 32 个十六进制数字，分为 5 组显示。
- 各组之间用连字符分隔：`8-4-4-4-12` 格式。
- 第 4 版标识（由第三组以 "4" 开头表示）。
- RFC 4122 变体标识（由第四组以 "8"、"9"、"a" 或 "b" 开头表示）。

示例：`2d89666b-1a1e-4a75-b193-4eb3b454c757`

有关完整规范，请参阅 [RFC 4122: A Universally Unique Identifier (UUID) URN Namespace](https://datatracker.ietf.org/doc/html/rfc4122)。

## 数据 (data) 类型

以下市场数据类型可以请求历史数据，也可以在交易场所/数据提供商提供且已在集成适配器中实现的情况下，作为实时流进行订阅。

- `OrderBookDelta` (L1/L2/L3)
- `OrderBookDeltas` (容器类型)
- `OrderBookDepth10` (每侧固定 10 档深度)
- `QuoteTick`
- `TradeTick`
- `Bar`
- `Instrument`
- `InstrumentStatus`
- `InstrumentClose`

以下 `PriceType` 选项可用于 K线聚合：

- `BID`
- `ASK`
- `MID`
- `LAST`

## K线聚合

以下 `BarAggregation` 方法可用：

- `MILLISECOND`
- `SECOND`
- `MINUTE`
- `HOUR`
- `DAY`
- `WEEK`
- `MONTH`
- `YEAR`
- `TICK`
- `VOLUME`
- `VALUE`（又称美元 K线）
- `RENKO`（基于价格的砖形图）
- `TICK_IMBALANCE`
- `TICK_RUNS`
- `VOLUME_IMBALANCE`
- `VOLUME_RUNS`
- `VALUE_IMBALANCE`
- `VALUE_RUNS`

上述列出的所有聚合方法均已实现内部聚合。
信息驱动的聚合方法需要 `TradeTick` 数据。

:::note K 线聚合方法分类

NautilusTrader 支持的聚合方法可按触发维度分为四类：

| 类别        | 方法                                                                                           | 触发条件                         |
| --------- | -------------------------------------------------------------------------------------------- | ---------------------------- |
| **时间驱动**  | `MILLISECOND`、`SECOND`、`MINUTE`、`HOUR`、`DAY`、`WEEK`、`MONTH`、`YEAR`                           | 固定时间间隔结束时触发                  |
| **成交量驱动** | `TICK`、`VOLUME`、`VALUE`（美元 K 线）                                                              | 累积到指定 Tick 数 / 成交量 / 成交金额时触发 |
| **价格驱动**  | `RENKO`                                                                                      | 价格移动指定幅度时触发（砖形图）             |
| **信息驱动**  | `TICK_IMBALANCE`、`TICK_RUNS`、`VOLUME_IMBALANCE`、`VOLUME_RUNS`、`VALUE_IMBALANCE`、`VALUE_RUNS` | 基于订单流不平衡程度触发                 |

信息驱动类型来自量化金融研究（微观结构分析），适合检测市场参与者的知情/非知情交易行为，需要 `TradeTick` 数据才能聚合。
:::

价格类型和 K线聚合可以通过 `BarSpecification` 以任意方式组合，步长 >= 1。
这允许为实盘交易聚合替代 K线。

## 账户类型

以下账户类型在实盘和回测环境中均可用：

- `Cash` 单币种（基础货币）
- `Cash` 多币种
- `Margin` 单币种（基础货币）
- `Margin` 多币种
- `Betting` 单币种

## 订单类型

以下订单类型可用（取决于交易场所是否支持）：

- `MARKET`
- `LIMIT`
- `STOP_MARKET`
- `STOP_LIMIT`
- `MARKET_TO_LIMIT`
- `MARKET_IF_TOUCHED`
- `LIMIT_IF_TOUCHED`
- `TRAILING_STOP_MARKET`
- `TRAILING_STOP_LIMIT`

## 值类型

以下值类型由 128 位或 64 位原始整数值支持，具体取决于编译时使用的[精度模式](../getting_started/installation.md#precision-mode)。

- `Price`
- `Quantity`
- `Money`

### 高精度模式 (128 位)

当 `high-precision` 特性标志被**启用**时（默认），值使用以下规格：

| 类型         | 原始存储   | 最大精度 | 最小值                 | 最大值                |
| :--------- | :----- | :--- | :------------------ | :----------------- |
| `Price`    | `i128` | 16   | -17,014,118,346,046 | 17,014,118,346,046 |
| `Money`    | `i128` | 16   | -17,014,118,346,046 | 17,014,118,346,046 |
| `Quantity` | `u128` | 16   | 0                   | 34,028,236,692,093 |

### 标准精度模式 (64 位)

当 `high-precision` 特性标志被**禁用**时，值使用以下规格：

| 类型         | 原始存储  | 最大精度 | 最小值            | 最大值            |
| :--------- | :---- | :--- | :------------- | :------------- |
| `Price`    | `i64` | 9    | -9,223,372,036 | 9,223,372,036  |
| `Money`    | `i64` | 9    | -9,223,372,036 | 9,223,372,036  |
| `Quantity` | `u64` | 9    | 0              | 18,446,744,073 |

:::tip 精度模式选择建议

| 场景                 | 推荐模式          | 原因                      |
| ------------------ | ------------- | ----------------------- |
| 加密货币交易（精度可能 > 9 位） | **128 位**（默认） | 部分极低价代币的价格精度超过 9 位      |
| 外汇/股票交易（精度 ≤ 5 位）  | 64 位可用        | 外汇报价通常 4-5 位小数，64 位完全足够 |
| AI 强化学习训练（速度优先）    | 64 位          | 更低内存占用，算术运算更快           |
| 高量值合约（如 BTC 大额交易）  | **128 位**     | 避免大数值与高精度同时出现时的整数溢出     |

大多数用户应保持默认的 128 位高精度模式。仅在明确了解精度限制且有充分性能理由时才切换到 64 位模式。
:::
