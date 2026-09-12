# 概念 (Concepts)

这些指南介绍了 NautilusTrader 的核心组件、架构和设计。

## [概述](overview.md)

平台的主要功能和预期使用场景。

## [架构](architecture.md)

支撑平台的基础原则、结构和设计。

## [Actor](actors.md)

`Actor` 是与交易系统交互的基础组件。
本节涵盖了其功能和实现细节。

## [策略](strategies.md)

如何使用 `Strategy` 组件实现交易策略。

## [金融工具](instruments/index.md)

可交易资产和合约的金融工具 (Instruments) 定义。

## [合成工具](synthetics.md)

由用户自定义的金融工具，其价格通过对组成工具的价格求值数值表达式计算得出。

## [连续期货](continuous_futures.md)

通过显式的展期表 (roll table) 将连续的期货合约拼接为一条经过调整的 bar 序列，
涵盖四种调整模式、请求与订阅流程，以及 bar 中点 (mid-bar) 的展期边界策略。

## [值类型](value_types.md)

平台中通用的不可变数值类型 (`Price`、`Quantity`、`Money`)，
包括它们的算术行为、精度处理和各类型特有的约束。

## [数据](data.md)

面向交易领域的内置数据 (data) 类型，以及如何处理自定义数据。

## [事件](events.md)

驱动系统运行的事件类型：订单事件、持仓事件、账户事件和时间事件。
涵盖处理器分发、从订单成交到持仓事件的因果链，以及如何从订单追溯到持仓。

## [事件溯源](event_sourcing.md)

针对影响状态的消息的持久化事件存储日志，包括捕获边界、关联头 (correlation headers)、
重放模式、恢复锚点 (recovery anchors) 和验证器 (verifier) 行为。

## [期权](options.md)

期权金融工具类型、交易场所提供的 Greeks 流式数据、带行权价区间过滤的
期权链订阅，以及快照聚合。

## [Greeks](greeks.md)

期权 Greeks（delta、gamma、vega、theta）来自两条路径：通过 Rust/PyO3 的 `OptionGreeks`
类型获取交易场所提供的实时 Greeks，以及使用本地 `GreeksCalculator` 进行 Black-Scholes
计算，支持冲击情景 (shock scenarios)、beta 加权和投资组合聚合。

## [自定义数据](custom_data.md)

自定义数据系统在 Python 和 Rust 中的工作方式：注册、持久化、Arrow 编码，
以及通过 actor 和策略进行的运行时路由。

## [订单簿](order_book.md)

高性能订单簿、自有订单跟踪、用于净流动性的过滤视图，以及二元市场 (binary market) 支持。

## [执行](execution.md)

跨多个策略和交易场所 (venue) 同时进行的交易执行和订单管理（每个实例），
包括所涉及的组件以及执行消息（命令和事件）的流转方式。

## [订单](orders/index.md)

可用的订单类型、支持的执行指令、高级订单类型，以及模拟订单。

## [持仓](positions.md)

持仓生命周期、从订单成交中的聚合、盈亏 (PnL) 计算，以及净额持仓管理系统 (netting OMS)
配置中的持仓快照。

## [缓存](cache.md)

`Cache` 是所有交易相关数据的核心内存存储。
本节涵盖了其功能和最佳实践。

## [消息总线](message_bus.md)

`MessageBus` 实现了组件之间的解耦消息传递，支持点对点、发布/订阅
和请求/响应模式。

## [账务核算](accounting.md)

账户类型（现金、保证金、博彩）、`AccountBalance` 和 `MarginBalance` 数据模型、
按金融工具与账户整体两种保证金作用域、策略查询 API、内置保证金模型，
以及各实盘交易场所通用的适配器约定。

## [投资组合](portfolio.md)

`Portfolio` 跟踪跨策略和金融工具的所有持仓，提供持仓、风险 (risk) 敞口
和绩效的统一视图。

## [报告](reports.md)

执行报告、投资组合分析、盈亏核算，以及回测后分析。

## [日志](logging.md)

为回测和实盘交易提供的高性能日志，使用 Rust 实现。

## [回测](backtesting.md)

使用特定系统实现，在历史数据上运行模拟交易。

## [可视化](visualization.md)

用于分析回测结果的交互式分析面板 (tearsheets)，包括图表、主题、
自定义选项，以及通过可扩展的图表注册机制创建的自定义可视化。

## [配置](configuration.md)

配置结构体在 Python 和 Rust 中的工作方式：默认值解析、`T` 与 `Option<T>`
约定、构建器模式 (builder patterns)，以及各适配器和引擎共享的通用字段。

## [实盘交易](live.md)

无需修改代码即可将经过回测的策略实时部署，以及回测与实盘交易之间的关键差异。

## [插件](plugins.md)

由实盘节点加载的 Rust 插件系统，涵盖 C-ABI 边界、清单 (manifest) 校验、
插入点接口（自定义数据、actor、策略）、宿主回调路由、配置，
以及从 `dlopen` 到适配器注册的完整生命周期。

## [适配器](adapters.md)

为数据提供商和交易场所开发集成适配器 (adapter) 的要求和最佳实践。

## [Rust](rust.md)

直接使用 `crates/` 实现，在纯 Rust 中编写 actor、策略，并运行回测和实盘交易。

## [确定性模拟测试 (DST)](dst.md)

确保按 seed 可重放执行的确定性契约、实现该契约的源码级接缝 (seams)、
强制执行该契约的预提交钩子 (pre-commit hook)，以及已知的作用域边界。

:::note
如果这些指南与 [API 参考](../../docs/api_reference/index.md) 之间存在差异，则以 API 参考为准。
:::
