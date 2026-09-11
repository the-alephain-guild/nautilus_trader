# Rust

Nautilus 在 `crates/` 目录下提供了一套完整的 Rust 实现。
你可以完全脱离 Python，用 Rust 编写 actor、策略，运行回测，以及进行实盘交易。
领域模型在所有路径间共享，而 v2 PyO3 路径则可以直接在 Rust 引擎上运行
Python 策略。

:::warning
Rust API 正处于活跃开发阶段。方法签名和 trait 要求
可能在不同版本之间发生变化。
:::

## 系统实现 (System implementations)

Nautilus 提供三种实现。理解每一种各自所处的阶段，有助于
你为自己的使用场景选出最合适的那一种。

- **v1 legacy**：位于 `nautilus_trader/` 下的 Cython/Python 类。功能
  完整，组件覆盖面最广。
- **v2 Rust**：位于 `crates/` 下的纯 Rust 实现。无需 Python 即可运行。
- **v2 PyO3**：Python 用户组件（actor、策略）通过 PyO3 绑定运行在
  Rust 内核之上。它把 Python 的便利性与
  Rust 引擎的性能结合在一起。

### 能力矩阵 (Capability matrix)

| 组件                   | v1 legacy (Cython) | v2 Rust | v2 PyO3 (Python on Rust) |
| -------------------- | ------------------ | ------- | ------------------------ |
| Strategy             | ✓                  | ✓       | ✓                        |
| Actor                | ✓                  | ✓       | ✓                        |
| DataEngine           | ✓                  | ✓       | ✓                        |
| ExecutionEngine      | ✓                  | ✓       | ✓                        |
| RiskEngine           | ✓                  | ✓       | ✓                        |
| BacktestEngine       | ✓                  | ✓       | ✓                        |
| BacktestNode         | ✓                  | ✓       | ✓                        |
| LiveNode             | ✓                  | ✓       | ✓                        |
| OrderEmulator        | ✓                  | ✓       | ✓                        |
| Matching engine      | ✓                  | ✓       | ✓                        |
| Portfolio            | ✓                  | ✓       | ✓                        |
| Accounts             | ✓                  | ✓       | ✓                        |
| Cache                | ✓                  | ✓       | ✓                        |
| MessageBus           | ✓                  | ✓       | ✓                        |
| Data catalog         | ✓                  | ✓       | ✓                        |
| Indicators           | ✓                  | ✓       | ✓                        |
| Exec algorithms      | TWAP               | TWAP    | TWAP                     |
| Controller           | ✓                  | -       | -                        |
| Tearsheets           | ✓                  | -       | -                        |
| Config serialization | ✓                  | -       | -                        |

### 适配器 (Adapters)

| 适配器                 | v1 legacy (Cython) | v2 Rust | v2 PyO3 |
| ------------------- | ------------------ | ------- | ------- |
| Architect AX        | ✓                  | ✓       | ✓       |
| Betfair             | ✓                  | ✓       | ✓       |
| Binance             | ✓                  | ✓       | ✓       |
| BitMEX              | ✓                  | ✓       | ✓       |
| Bybit               | ✓                  | ✓       | ✓       |
| Databento           | ✓                  | ✓       | ✓       |
| Deribit             | ✓                  | ✓       | ✓       |
| dYdX                | ✓                  | ✓       | ✓       |
| Hyperliquid         | ✓                  | ✓       | ✓       |
| Interactive Brokers | ✓                  | -       | -       |
| Kraken              | ✓                  | ✓       | ✓       |
| OKX                 | ✓                  | ✓       | ✓       |
| Polymarket          | ✓                  | ✓       | ✓       |
| Sandbox             | ✓                  | ✓       | ✓       |
| Tardis              | ✓                  | ✓       | ✓       |

### 如何选择路径 (Choosing a path)

- **v1 legacy** 是当前功能最完整的实现。如果你需要
  Controller、tearsheets、Interactive Brokers 或配置序列化，就用它。
- **v2 Rust** 无需 Python 运行时即可提供原生性能。所有核心
  交易功能都已具备。它适合对延迟敏感的
  部署，或偏好编译型语言的团队。
- **v2 PyO3**：Python 用户组件（actor、策略）运行在
  Rust 内核引擎之上，数据处理和执行享有 Rust 性能，
  同时保留了 Python 的编写体验。

## 项目配置 (Project setup)

Nautilus 各 crate 已发布到
[crates.io](https://crates.io/crates/nautilus-backtest)。将它们添加到你的
`Cargo.toml` 中：

```toml
[dependencies]
nautilus-backtest = "0.59"
nautilus-common = "0.59"
nautilus-execution = "0.59"
nautilus-model = { version = "0.59", features = ["stubs"] }
nautilus-trading = { version = "0.59", features = ["examples"] }

anyhow = "1"
log = "0.4"
```

对于实盘交易，请添加 live crate 以及你所交易场所对应的适配器：

```toml
[dependencies]
nautilus-live = "0.59"
nautilus-okx = "0.59"
```

若要跟踪最新的开发分支，请将所有 Nautilus 依赖都指向
同一个 git 来源，以避免 crates.io 版本与 git 版本之间的类型不匹配：

```toml
[dependencies]
nautilus-backtest = { git = "https://github.com/nautechsystems/nautilus_trader.git", branch = "develop" }
nautilus-common = { git = "https://github.com/nautechsystems/nautilus_trader.git", branch = "develop" }
nautilus-execution = { git = "https://github.com/nautechsystems/nautilus_trader.git", branch = "develop" }
nautilus-model = { git = "https://github.com/nautechsystems/nautilus_trader.git", branch = "develop", features = ["stubs"] }
nautilus-trading = { git = "https://github.com/nautechsystems/nautilus_trader.git", branch = "develop", features = ["examples"] }
```

最低支持的 Rust 版本（MSRV）为 **1.96.0**。

### 特性标志 (Feature flags)

| 标志               | Crate               | 效果                                      |
| ---------------- | ------------------- | --------------------------------------- |
| `high-precision` | `nautilus-model`    | 16 位定点精度（默认为 9 位）。加密货币场景必需。             |
| `stubs`          | `nautilus-model`    | 测试用 instrument stub（`audusd_sim` 等）。    |
| `examples`       | `nautilus-trading`  | 示例策略（`EmaCross`、`GridMarketMaker`）。     |
| `streaming`      | `nautilus-backtest` | 通过 `BacktestNode` 实现基于 catalog 的数据流式传输。 |
| `defi`           | `nautilus-model`    | DeFi 数据类型。隐含启用 `high-precision`。        |

:::tip
标准的 9 位精度可以满足大多数传统金融工具的需要。
对于价格可能带有很多位小数（例如 `0.00000001`）的加密货币交易场所，
请启用 `high-precision`。
:::

## Actors

actor 接收市场数据、自定义数据/信号以及系统事件，但不负责管理订单。
请实现 `DataActor` trait，并通过 `Deref`/`DerefMut` 把你的结构体绑定到
`DataActorCore`。你的结构体还必须实现 `Debug`（这是 blanket
`Component` impl 的要求）。内核会直接在你的结构体上提供订阅方法、缓存
访问和时钟访问。

### 处理器方法 (Handler methods)

重写 `DataActor` trait 上的任意处理器，即可接收对应的
数据或事件。所有处理器都有默认的空操作实现，因此你只需
重写自己需要的部分。

| 处理器                    | 接收内容                |
| ---------------------- | ------------------- |
| `on_start`             | actor 已启动。          |
| `on_stop`              | actor 已停止。          |
| `on_quote`             | `QuoteTick`         |
| `on_trade`             | `TradeTick`         |
| `on_bar`               | `Bar`               |
| `on_book_deltas`       | `OrderBookDeltas`   |
| `on_book`              | `OrderBook`（按间隔）    |
| `on_instrument`        | `InstrumentAny`     |
| `on_mark_price`        | `MarkPriceUpdate`   |
| `on_index_price`       | `IndexPriceUpdate`  |
| `on_funding_rate`      | `FundingRateUpdate` |
| `on_option_greeks`     | `OptionGreeks`      |
| `on_option_chain`      | `OptionChainSlice`  |
| `on_instrument_status` | `InstrumentStatus`  |
| `on_order_filled`      | `OrderFilled`       |
| `on_order_canceled`    | `OrderCanceled`     |
| `on_time_event`        | `TimeEvent`         |

如需逐步操作的讲解，请参阅
[编写一个 Actor (Rust)](../how_to/write_rust_actor.md) how-to 指南。
如需完整示例，请参阅
[`BookImbalanceActor`](https://github.com/nautechsystems/nautilus_trader/tree/develop/crates/trading/src/examples/actors/imbalance)。

## Strategies

策略在 actor 的基础上增加了订单管理。请同时实现
`DataActor`（用于数据处理）和 `Strategy`（用于访问
`StrategyCore`）。`StrategyCore` 封装了 `DataActorCore`，并额外提供
`OrderFactory`、`OrderManager` 以及组合集成能力。

### 订单管理 (Order management)

`Strategy` trait 通过 `StrategyCore` 提供以下订单方法：

| 方法                    | 动作                           |
| --------------------- | ---------------------------- |
| `submit_order`        | 向交易场所提交一笔新订单。                |
| `submit_order_list`   | 提交一组关联订单（contingent orders）。 |
| `modify_order`        | 修改价格、数量或触发价格。                |
| `cancel_order`        | 取消某一笔指定订单。                   |
| `cancel_orders`       | 取消经过筛选的一组订单。                 |
| `cancel_all_orders`   | 取消某个 instrument 的所有订单。       |
| `close_position`      | 以市价单平掉一个持仓。                  |
| `close_all_positions` | 平掉所有未平仓持仓。                   |

`OrderFactory`（通过 `self.core.order_factory()` 访问）负责构建订单
对象：`market`、`limit`、`stop_market`、`stop_limit`、
`market_if_touched`、`limit_if_touched` 以及 `trailing_stop_market`。

如需逐步操作的讲解，请参阅
[编写一个 Strategy (Rust)](../how_to/write_rust_strategy.md) how-to 指南。
如需完整示例，请参阅
[`EmaCross`](https://github.com/nautechsystems/nautilus_trader/tree/develop/crates/trading/src/examples/strategies/ema_cross)
和
[`GridMarketMaker`](https://github.com/nautechsystems/nautilus_trader/tree/develop/crates/trading/src/examples/strategies/grid_mm)。

### 运行 Rust 组件 (Running Rust components)

Rust 策略和 actor 可以通过三种路径运行。下面的示例
使用的是策略，但相同的模式同样适用于 actor，分别通过
`add_actor`（纯 Rust）和 `add_native_actor`（来自 Python）。

#### 纯 Rust (Pure Rust)

用 Rust 编写你的策略和 `main` 函数，然后通过 `cargo build`
构建出一个独立的二进制文件。这条路径无需任何 Python 运行时。

```rust
let strategy = GridMarketMaker::new(config);
node.add_strategy(strategy)?;
node.run().await?;
```

完整讲解请参阅 [运行实盘交易 (Rust)](../how_to/run_rust_live_trading.md)。

#### 来自 Python 的原生配置 (Native config from Python)

向 `add_native_strategy` 传入一个类型名称和配置，即可从 Python
注册一个内置的 Rust 策略。Rust 一侧会构造该策略
并将其注册到引擎。Python 提供配置；所有
执行都发生在 Rust 中。

```python
from nautilus_trader.core.nautilus_pyo3.trading import GridMarketMakerConfig

config = GridMarketMakerConfig(
    instrument_id=InstrumentId.from_str("BTC-USDT-SWAP.OKX"),
    max_position=Quantity.from_str("10.0"),
    trade_size=Quantity.from_str("0.1"),
    num_levels=5,
    grid_step_bps=15,
)

node.add_native_strategy("GridMarketMaker", config)
```

内置策略配置：

| 配置                           | 策略                     |
| ---------------------------- | ---------------------- |
| `CompositeMarketMakerConfig` | `CompositeMarketMaker` |
| `DeltaNeutralVolConfig`      | `DeltaNeutralVol`      |
| `EmaCrossConfig`             | `EmaCross`             |
| `ExecTesterConfig`           | `ExecTester`           |
| `GridMarketMakerConfig`      | `GridMarketMaker`      |
| `HurstVpinDirectionalConfig` | `HurstVpinDirectional` |

内置 actor 配置（通过 `add_native_actor`）：

| 配置                         | Actor                |
| -------------------------- | -------------------- |
| `BookImbalanceActorConfig` | `BookImbalanceActor` |
| `DataTesterConfig`         | `DataTester`         |

从源码编译的用户可以将自己的原生组件添加到这条
路径中。添加一个 `#[pyclass]` 配置、一个 `register_*` 函数，以及在
`native_strategy_register` 或 `native_actor_register` 中添加一个 match 分支。
这样该组件就可以从 Python 使用，而无需在类型本身上添加 PyO3 包装。

#### 插件加载 (Plugin loading)

对于构建为 `cdylib` crate 的 Rust 组件，使用 `add_plugin` 或
`LiveNodeConfig.plugins`。插件清单（manifest）会提供组件类别，因此
宿主只需要库路径、清单中的类型名称以及实例配置即可。

## 回测 (Backtesting)

如需两套 API 的带注解讲解，请参阅
[运行一次回测 (Rust)](../how_to/run_rust_backtest.md) how-to 指南。

### `BacktestEngine`（低层 API）

构造引擎，添加交易场所和 instrument，加载数据，注册
策略，然后运行。完整的可运行示例如下：

```bash
cargo run -p nautilus-backtest --features examples --example engine-ema-cross
```

源码：
[`crates/backtest/examples/engine_ema_cross.rs`](https://github.com/nautechsystems/nautilus_trader/tree/develop/crates/backtest/examples/engine_ema_cross.rs)

### `BacktestNode`（高层 API）

从 `ParquetDataCatalog` 加载数据，并支持以可配置的
分块大小进行流式传输。需要在 `nautilus-backtest` 上启用 `streaming`
特性。完整的可运行示例如下：

```bash
cargo run -p nautilus-backtest --features examples,streaming --example node-ema-cross
```

源码：
[`crates/backtest/examples/node_ema_cross.rs`](https://github.com/nautechsystems/nautilus_trader/tree/develop/crates/backtest/examples/node_ema_cross.rs)

## 实盘交易 (Live trading)

如需带注解的讲解，请参阅
[运行实盘交易 (Rust)](../how_to/run_rust_live_trading.md) how-to 指南。

`LiveNode` 通过适配器客户端连接到真实的交易场所。构建器
模式用于配置数据客户端和执行客户端，随后 `run()` 启动异步
事件循环。每个适配器都提供自己的工厂类型和配置类型。

| 适配器          | 示例                                       |
| ------------ | ---------------------------------------- |
| Architect AX | `crates/adapters/architect_ax/examples/` |
| Betfair      | `crates/adapters/betfair/examples/`      |
| Binance      | `crates/adapters/binance/examples/`      |
| BitMEX       | `crates/adapters/bitmex/examples/`       |
| Blockchain   | `crates/adapters/blockchain/examples/`   |
| Bybit        | `crates/adapters/bybit/examples/`        |
| Databento    | `crates/adapters/databento/examples/`    |
| Deribit      | `crates/adapters/deribit/examples/`      |
| dYdX         | `crates/adapters/dydx/examples/`         |
| Hyperliquid  | `crates/adapters/hyperliquid/examples/`  |
| Kraken       | `crates/adapters/kraken/examples/`       |
| OKX          | `crates/adapters/okx/examples/`          |
| Polymarket   | `crates/adapters/polymarket/examples/`   |
| Sandbox      | `crates/adapters/sandbox/examples/`      |
| Tardis       | `crates/adapters/tardis/examples/`       |

大多数适配器都包含 `node_data_tester.rs` 和 `node_exec_tester.rs`
示例。它们针对实盘交易场所测试数据请求、流式传输以及订单执行。

## 相关指南 (Related guides)

- [编写一个 Actor (Rust)](../how_to/write_rust_actor.md) - 逐步讲解的 actor 教程。
- [编写一个 Strategy (Rust)](../how_to/write_rust_strategy.md) - 逐步讲解的策略教程。
- [运行一次回测 (Rust)](../how_to/run_rust_backtest.md) - BacktestEngine 和 BacktestNode 的用法。
- [运行实盘交易 (Rust)](../how_to/run_rust_live_trading.md) - LiveNode 的配置与交易场所连接。
- [架构](architecture.md) - 系统设计以及数据/执行流。
- [Actors](actors.md) - actor 概念（同时适用于 Python 和 Rust）。
- [Strategies](strategies.md) - 策略概念与处理器参考。
- [Events](events.md) - 事件类型与处理器分发。
- [Backtesting](backtesting.md) - 回测概念与撮合引擎行为。
