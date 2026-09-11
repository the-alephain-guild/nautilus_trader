# 确定性仿真测试 (DST)

**确定性仿真测试 (Deterministic simulation testing, DST)** 在一个由种子控制的运行时下运行 NautilusTrader，
从而让对时序敏感的执行行为能够仅凭一个整数就实现按位可复现。本指南
解释 DST 是什么、NautilusTrader 如何支持它、这种支持提供了哪些保证，以及
这些保证在何处终止。

目标是发布一份外部用户和审计者都能验证的契约：NautilusTrader 所声称的
确定性有源码级证据支撑，并在提交时由一个在持续集成中运行的 pre-commit
钩子强制执行。

## 引言

### DST 是什么

DST 是一种针对并发系统的测试技术。单个种子完全决定一次执行，
包括任务调度、定时器触发和随机值。在相同种子、二进制文件
和配置下进行的两次运行会产生完全相同的可观测行为。当某个属性失败时，
种子就是复现手段：相同的种子每次都会重放该次失败。

异步运行时中的调度决策来自环境进程状态：任务唤醒顺序、
定时器精度、线程调度、哈希种子。这些都不受测试框架控制，
这正是为什么一个在 CI 中偶尔浮现的竞态通常很难按需复现。DST
用一个带种子的伪随机序列替换这些环境来源，因此交错执行顺序成为
种子的函数。

FoundationDB 从大约 2009 年起就将这一模式应用于一个生产环境的分布式数据库；
在 Rust 生态中，[madsim](https://github.com/madsim-rs/madsim) 拦截 `tokio`
原语以提供一个确定性调度器。

DST 所瞄准的 bug 是那些能逃过单元测试、集成测试、属性测试和验收
测试的 bug：通道唤醒顺序、关闭时的排空竞态、启动顺序、对账
顺序、恢复路径正确性。它们全都涉及其他测试层无法穷尽覆盖、
而确定性调度器却能系统性探索的交错执行。

### 本指南涵盖的内容

NautilusTrader 的 DST 支持有两个组成部分：

- **契约**：运行时在种子控制的执行下保证什么，以及在哪些条件下保证。
- **强制执行**：实现该契约的源码级接缝 (seam)，以及让这些接缝保持就位的
  pre-commit 钩子。

## 目标

- **种子可复现的执行**，针对 NautilusTrader 运行时的覆盖范围内部分。
- **诚实的范围**。契约列出哪些被覆盖、哪些未被覆盖。不会悄然回退到
  真实的挂钟时间或无种子的 RNG；削弱该保证的条件均被逐项列出。
- **源码层面的强制执行**。一个 pre-commit 钩子会让那些向 DST 路径
  添加被禁止模式的提交失败，因此契约无需依赖审查者的注意力即可保持为真。
- **最小必要的插桩**。这些接缝仅在契约要求的地方把时间、任务调度和随机性
  路由到一个确定性来源；其余一切照常运行。

## 方法

`madsim` 只对那些经由其别名子模块（`time`、`task`、`runtime`、`signal`）
路由的 `tokio` 原语做确定化处理。挂钟读取、单调时钟读取、RNG 取数、哈希
迭代以及 `select!` 轮询完全绕过 `tokio`，需要各自专属的接缝。第 1 层
把别名子模块替换为 `madsim`；第 2 层提供这些接缝。

### 第 1 层：运行时替换

在 `nautilus-common` 的 `simulation` Cargo 特性下，当设置了
`RUSTFLAGS="--cfg madsim"` 时，四个 `tokio` 子模块会经由 `madsim` 路由：

- `time`（定时器、间隔、单调 `Instant`）。
- `task`（异步任务的派生与 join）。
- `runtime`（运行时构建器和句柄）。
- `signal`（进程信号，例如 `ctrl_c`；re-export 可用，但调用点的采用
  是部分的，详见 [范围边界](#signal-handling) 中的说明）。

这些 re-export 位于 `nautilus_common::live::dst`。`time`、`task`
和 `runtime` 的 DST 路径调用点从这个模块导入，而不是直接从 `tokio` 导入，
因此对于完全路由的那些原语，切换该特性会在一处切换异步运行时。在常规
构建下，这些 re-export 解析为真实的 `tokio`。在 `simulation` + `cfg(madsim)`
下，它们解析为 `madsim` 的确定性对应物。

`tokio` 提供的其余一切（`sync`、`io`、作为宏的 `select!`、`fs`、`net`）无条件
使用真实的 `tokio`。传递依赖的 crate（`tokio-tungstenite`、`tokio-rustls`、`reqwest`）
不受影响。

### 第 2 层：非确定性替换

异步运行时之外的非确定性通过显式接缝重定向：

- **挂钟读取**经由 `nautilus_core::time::duration_since_unix_epoch`。在仿真下
  这会路由到 `madsim::time::TimeHandle::try_current()`，为订单和成交时间戳
  保留 Unix-epoch 语义。当在 madsim 运行时之外被调用时（纯
  `#[rstest]` 测试体），它会回退到 `SystemTime::now()`，而后者在 `cfg(madsim)`
  下经 libc 拦截到与常规构建相同的真实系统调用。仿真下的生产路径
  始终运行在某个运行时内部，因此它们继续接收虚拟时间。
- **单调读取**经由 `nautilus_common::live::dst::time::Instant`。该类型在常规
  构建下解析为 `tokio::time::Instant`（以便兼容 `tokio::test(start_paused)`
  测试辅助），在仿真下解析为 `madsim::time::Instant`。
- **网络局部单调读取**经由 `nautilus_network::dst::time`。该 crate 在依赖图中
  位于 `nautilus-common` 之下，并暴露一个语义相同的局部 re-export 模块。
- **哈希迭代顺序**在对账管理器和订单撮合引擎中使用
  `IndexMap` 和 `IndexSet`，而非 `AHashMap` 和 `AHashSet`。`AHash` 每个进程
  都随机化其哈希器；在迭代顺序驱动下游事件发布或者驱动有种子的
  `FillModel` RNG 被消耗的顺序之处，需要按插入顺序迭代。
- **`tokio::select!` 轮询顺序**在 DST 路径上的每个生产站点都使用 `biased;`
  修饰符。无偏的 `select!` 会按一个未被拦截的 RNG 选定的顺序轮询各分支。

## 确定性契约

在以下条件下，由 `(seed, binary hash, configuration hash)` 标识的一次运行，在
同一平台上会产生按位完全相同的：

1. 异步任务的调度顺序。
2. 定时器触发（虚拟单调时钟与虚拟挂钟）。
3. 来自 `madsim::rand` 的 RNG 输出。
4. `tokio` 原语上的通道投递顺序。

### 必需条件

契约仅在以下全部为真时才成立：

1. `simulation` Cargo 特性处于激活状态且设置了 `RUSTFLAGS="--cfg madsim"`。两者
   都是必需的。该特性激活确定性运行时；该 cfg 标志激活 `madsim` 的
   libc 级 `clock_gettime` 和 `getrandom` 拦截。缺一另一会悄然
   回退到真实的 `tokio`，并在不报错的情况下破坏确定性。
2. DST 路径上的每个 `tokio::select!` 调用点都使用 `biased;` 修饰符。
3. 单调时间读取经由 DST 接缝（`nautilus_common::live::dst::time` 或
   `nautilus_network::dst::time`），而非直接经由 `std::time::Instant::now`。
4. 挂钟时间读取经由 `nautilus_core::time::duration_since_unix_epoch`。
5. 随机性经由 `madsim::rand`。`rand::thread_rng`、`rand::rng()`、`fastrand`、
   `getrandom` 和 `OsRng` 不被拦截。
6. 对迭代顺序敏感的集合使用 `IndexMap` 或 `IndexSet`，而非 `AHashMap` 或
   `AHashSet`。
7. `tokio::task::LocalSet` 的构造在仿真下被 cfg 屏蔽。`madsim` 不
   提供 `LocalSet`；`spawn_local` 无需它即可工作。
8. `tokio::task::spawn_blocking` 调用点被 cfg 屏蔽或移除。阻塞调用会
   逃出确定性调度器。

## 静态强制执行

静态强制执行有两层：

- `clippy.toml` 和 `[workspace.lints.clippy]` 中的 Clippy 策略阻止那些在
  整个 workspace 的 DST 契约下无效的 API：直接的 `getrandom::{fill,u32,u64}` 调用以及
  `tokio::task::LocalSet`。
- 一个名为 `check-dst-conventions` 的 pre-commit 钩子强制执行 Clippy 无法
  干净地表达的、限定作用域的、感知路径的、感知 cfg 的结构化检查。

该钩子位于 `.pre-commit-hooks/check_dst_conventions.sh`，作为标准
pre-commit 套件的一部分运行，并在持续集成中运行。它覆盖 16 个范围内的 workspace crate，
并在检测到以下任意情况时让提交失败：

- 裸的 `std::time::Instant::now()`、`SystemTime::now()` 或 `chrono::Utc::now()` 读取，
  包括当所在文件从 `std::time`（或对 `Utc` 而言从 `chrono`）导入该类型时的
  无前缀形式。
- 未经 cfg 屏蔽的裸 RNG 用法（`rand::thread_rng`、`rand::rng()`、`fastrand::`、`getrandom::`、`OsRng`）
  或 `Uuid::new_v4()`。
- 在前三行内缺少 `biased;` 的 `tokio::select!` 块。
- 缺少前置 `#[cfg(test)]`、`#[cfg(not(madsim))]` 或
  `#[cfg(not(all(feature = "simulation", madsim)))]` 属性的 `std::thread::spawn`、
  `std::thread::Builder::new` 或 `tokio::task::spawn_blocking` 调用。
- 在 DST 路径上对迭代顺序敏感的文件中出现 `AHashMap` 或 `AHashSet`。完整的
  文件集合正在审计中；强制执行目前覆盖 `crates/live/src/manager.rs` 和
  `crates/execution/src/matching_engine/engine.rs`，并随着更多文件被审查而扩展。
- 绕过 `nautilus_network::net` 的、直接的 `tokio::net::TcpStream::connect` /
  `tokio::net::TcpListener::bind` 触达。该接缝在常规构建下 re-export `tokio::net`
  类型，在 `turmoil` 特性下切换到 `turmoil::net`，因此所有 TCP 入口点共享
  单一的、受 cfg 屏蔽的切换点。

该钩子支持两种例外形式：

- 在特定行上的内联 `// dst-ok` 标记，通常附带一个简短理由（例如，
  仅用于日志、不影响状态的挂钟计时）。
- 在钩子脚本自身中的一个小的文件级允许列表，用于代码库审计中被归类为
  无需处理的站点（cache 模块中的日志计时，DeFi 模块中的进度报告）。

测试文件、`tests/`、`python/` 和 `ffi/` 目录下的文件，以及位于内联
`#[cfg(test)]` 模块内的行都被排除，因为它们不属于 DST 路径。

### 范围内的 crate

该钩子应用于 `nautilus-live` 传递闭包中的 16 个 workspace crate：

- `analysis`、`common`、`core`、`cryptography`、`data`、`execution`、`indicators`、`live`、
  `model`、`network`、`persistence`、`portfolio`、`risk`、`serialization`、`system`、`trading`。

适配器 crate 和基础设施 crate（Redis、Postgres）不在范围内。它们的 DST
适用性需要在进入 DST 路径之前经过单独的审计。

## 网络种子浸泡测试

`nautilus-network` 的 Turmoil 测试使用两层：

- 固定种子测试在每夜测试套件中运行。它们以可复现的种子覆盖 connect、reconnect、partition、
  reconnect 期间 close、backoff 期间 close 以及重复的 server-drop 场景。
- 一个被忽略的 reconnect 浸泡测试会持续扫描 Turmoil 种子直到被停止，或直到
  `NAUTILUS_TURMOIL_SOAK_COUNT` 个种子已运行完毕。当启用 `transport-sockudo` 时，每个种子
  先运行 Tungstenite WebSocket 后端、再运行 Sockudo 后端，因此两个后端看到
  相同的调度搜索路径。

使用以下命令运行持续浸泡：

```bash
scripts/soak-network-turmoil.sh
```

使用以下命令运行有界浸泡：

```bash
env NAUTILUS_TURMOIL_SOAK_COUNT=100 scripts/soak-network-turmoil.sh
```

该浸泡测试使用确定性种子扫描、随机节点顺序、随机化的链路延迟、重复的
服务端 drop、reconnect 状态循环，以及精确的应用消息顺序检查。它
不启用 Turmoil 的 `fail_rate`：对 TCP 而言，这会在没有重传模型的情况下断开链路，
从而会在一个保序测试中夸大客户端投递契约。

这些 Turmoil 测试针对模拟网络运行，且不被限定到 Linux。若干真实的
本地回环套接字和 WebSocket 单元测试使用 `target_os = "linux"` 以保证 CI 稳定性，因此 macOS
本地运行不会执行那部分主机 TCP 覆盖。在把整个网络测试集视为已覆盖之前，
请使用 macOS 进行 Turmoil 种子扫描，并使用 Linux CI 或一台 Linux 工作站。

## 实现说明

DST 审计在本仓库中产生的具体改动。在调查某条代码路径是否在 DST 路径上、
以及它今天如何路由时，请以此为起点。

### 迭代顺序接缝

因为迭代顺序在 DST 路径上可观测，从 `AHashMap` / `AHashSet` 翻转为
`IndexMap` / `IndexSet` 的生产站点：

- **撮合引擎** (`crates/execution/src/matching_engine/engine.rs`)：九个字段
  （`execution_bar_types`、`execution_bar_deltas`、`account_ids`、`cached_filled_qty`、
  `bid_consumption`、`ask_consumption`、`queue_ahead`、`queue_excess`、`queue_pending`）。
  迭代式移除使用 `.shift_remove()`。关闭
  [#3914](https://github.com/nautechsystems/nautilus_trader/issues/3914)。
- **对账管理器** (`crates/live/src/manager.rs`)：受钩子强制执行，外加
  `ReconciliationResult.orders` 和 `ReconciliationResult.fills`。
- **Account trait** (`crates/model/src/accounts/`)：`balances`、`balances_total`、
  `balances_free`、`balances_locked`、`starting_balances` 的返回值。`BaseAccount`
  和 `MarginAccount` 上的存储字段均为 `IndexMap`。
- **持仓事件** (`crates/model/src/position.rs`)：`Position::commissions` 翻转
  为 `IndexMap`（在 `events/position/snapshot.rs` 中经由 `.values()` 消费）。
- **组合聚合** (`crates/portfolio/src/portfolio.rs`)：`unrealized_pnls`、
  `realized_pnls`、`net_positions` 存储；`accumulate_mark_values` 构建
  `IndexMap<Currency, f64>`。
- **数据引擎** (`crates/data/src/engine/`)：`book_snapshot_counts`、`bar_aggregators`、
  `BookSnapshotInfos`。迭代式移除使用 `.shift_remove()`。
- **执行引擎** (`crates/execution/src/engine/`)：`ExecutionEngine.clients`，外加
  `get_clients_for_orders()` 中的 `client_ids` / `venues` 累加器。
- **交易算法** (`crates/trading/src/algorithm/core.rs`)：
  `strategy_event_handlers`（驱动有序的 `msgbus::unsubscribe_*` 扇出）。
- **分析器** (`crates/analysis/src/analyzer.rs`)：`account_balances`、
  `account_balances_starting`。
- **Cache API** (`crates/common/src/cache/mod.rs`)：`get_orders_for_ids` 和
  `get_positions_for_ids` 在返回前会按 `client_order_id` / `position_id` 对其
  `Vec` 返回值排序。存储仍保留在 `AHashSet`（集合语义）。

范围内 crate 中剩余的 `AHashMap` / `AHashSet` 站点要么是仅查找的，要么处于
并发共享所有权包装（`Arc<DashMap>`、`AtomicMap`）之后，要么馈入
可交换的聚合。任何驱动可观测迭代顺序的新增范围内站点
都是一次回归，逐区域审计会对此加以防范。

### 时间接缝

仍留在 DST 路径上的 `Instant::now` / `SystemTime::now` 调用点，要么
位于 `#[cfg(test)]` 内，要么在钩子中被文件级允许，要么带有内联 `// dst-ok`
标记及理由：

- `crates/common/src/testing.rs:81,108` `wait_until` / `wait_until_async`
- `crates/execution/src/engine/mod.rs:822,847` 初始化日志计时
- `crates/common/src/cache/mod.rs:569,904,3895` 日志与审计计时（文件级允许）
- `crates/model/src/defi/reporting.rs:59,123` 进度日志（文件级允许）
- `crates/core/src/time.rs` 接缝定义站点（文件级允许）

`chrono::Utc::now` 在范围内 crate 中被钩子禁止。剩余的调用点是
日志桥接和写入器（在"日志运行于真实的 OS 线程"下被排除范围）。
`crates/core/src/datetime.rs::is_within_last_24_hours` 辅助函数曾经从
非日志路径触达 `chrono::Utc::now`；它现在经由
`nautilus_core::time::nanos_since_unix_epoch()` 路由，并直接以 `u64` 纳秒比较。

### 随机性接缝

DST 路径上的生产 RNG 站点：

- `crates/core/src/uuid.rs::UUID4::new()` 当在仿真下的 madsim 运行时内部被调用时
  经由 `madsim::rand::thread_rng()` 路由，在运行时之外（以及在常规构建上）
  回退到 `rand::rng()`。仿真下的生产路径始终运行在
  运行时内部，因此它们消耗有种子的字节；在 `cfg(madsim)` 下的纯
  `#[rstest]` 测试使用主机 RNG。可从 `nautilus-common` 和 `nautilus-risk` 中的
  订单与事件工厂触达。
- `crates/execution/src/models/fill.rs::default_std_rng()` 以相同方式路由。当未提供种子时，
  从 `ProbabilisticFillState::new()` 调用。有种子时，
  `StdRng::seed_from_u64` 在构造上即是确定性的。
- `crates/execution/src/matching_engine/ids_generator.rs:167,179` 为 `use_random_ids`
  路径使用 `nautilus_core::UUID4::new()`。默认 ID 方案
  （`{venue}-{raw_id}-{count}`）在不使用它时是确定性的。

带标记允许：`crates/network/src/backoff.rs:105` 用于 reconnect 抖动，
`// dst-ok`（传输层）。

### Tokio 子模块拆分

`madsim` 对 `time`、`task`、`runtime` 和 `signal` 做别名处理。其他 tokio 子模块
（`sync`、`io`、`select!`、`fs`、`net`）在仿真下保留在真实的 tokio 上。进一步扩展
该替换将需要让 `tokio-tungstenite`、`tokio-rustls` 和
`reqwest` 针对垫片化的 `tokio::net::TcpStream` 重新构建，审计认为这过于
侵入性而予以排除。

直接触碰真实 `tokio::net` / `tokio::io` 的范围内站点：

- `crates/network/src/net.rs:37` re-export `tokio::net::{TcpListener, TcpStream}`
- `crates/network/src/socket/client.rs:46,356` `tokio::io::{AsyncReadExt, AsyncWriteExt}`
- `crates/network/src/tls.rs:22` `tokio::io::{AsyncRead, AsyncWrite}`
- `crates/network/src/websocket/types.rs:26,29` 别名 `MaybeTlsStream<tokio::net::TcpStream>`

即使在仿真下，这些也运行在真实套接字上。`tokio::sync` 上的通道投递顺序
保持确定性，因为发送方任务和接收方任务由 madsim 执行器调度，
即便通道实现本身是真实的。

### 裸线程逃逸规则

钩子的规则 4 禁止在三种逃逸情况之外的裸线程派生：

- `#[cfg(test)]` 测试模块。
- `#[cfg(not(madsim))]` 或 `#[cfg(not(all(feature = "simulation", madsim)))]` 生产
  站点（例如日志写入器线程）。
- 内联 `// dst-ok` 标记。

`tokio::task::LocalSet` 和 `tokio::task::spawn_blocking` 在 `madsim` 下
不受支持。代码库审计在范围内 crate 中没有发现两者的任何生产站点；
新增站点必须带有 cfg 屏蔽或 `// dst-ok` 标记。

### 仿真下的日志测试

日志写入器线程在仿真下被 cfg 屏蔽；在 `cfg(madsim)` 下日志
事件被丢弃。初始化文件日志写入器的测试要么会挂起、要么会针对
一个空日志文件断言，因此受影响的子模块在模块边界处被屏蔽：

- `crates/common/src/logging/logger.rs::tests::serial_tests`（八个测试）。
- `crates/common/src/logging/macros.rs::tests`（两个测试）。

`logger.rs::tests::sim_tests::test_init_under_madsim_skips_writer_thread_and_forces_bypass`
在仿真下运行，并固定该被屏蔽行为。

## 范围边界

该契约刻意保持狭窄。以下削弱之处是显式声明的，而非疏漏。

### Python 不在 DST 范围内

DST 运行于一个原生 Rust 测试框架之下。DST 运行期间不会启动任何 Python 解释器。
`crates/*/src/python/` 下的 PyO3 绑定、`ffi/` 目录，以及
`nautilus_trader/` 下的 Python 包都作为一项政策（而非弱点）被排除在契约之外。任何
仅可从 Python 调用路径触达的代码都不在范围内；任何可从原生 DST 框架触达的
Rust 路径都必须满足契约，即便相同的类型同时也被导出到 Python。

`check-dst-conventions` 钩子通过在范围内 crate 中跳过 `/python/` 和 `/ffi/` 路径来
编码这项政策。这些路径之后的时钟、RNG 和线程调用点不适用于契约。

DST 的首要目标是 Rust 引擎自身的可靠性：订单生命周期、
对账、撮合、风控和执行状态机。用户策略的确定性重放是一个
更晚的、次要的目标，一旦策略以 Rust 编写或通过一个 Rust 原生测试框架运行，
该目标就变得可用。在此期间，一个调用
`time.time()`、发出任意网络请求或依赖线程调度的 Python 策略可能会在两次运行间
改变其命令流；Rust 内核会确定性地处理这个变化的流，
但从一个 Python 入口点的端到端重放无法保证。

### 平台限定

`madsim` 对 `clock_gettime` 和 `getrandom` 的 libc 覆盖是平台特定的。
不声称跨平台的按位可复现性。一个在 Linux x86_64 上复现某次失败的种子
可能无法在 macOS aarch64 上复现。

### 非别名依赖会悄然逃逸

任何通过非别名路径触达操作系统的依赖（直接的 `libc` 调用、绕过 `std::net`、
使用 `fastrand` 或 `OsRng` 的 crate）都会逃出仿真器而不引发任何错误。
范围内 crate 已经过审计；适配器 crate 和基础设施 crate 在进入 DST 路径之前
需要各自的审计。

### 传输层 I/O 不被仿真

`tokio-tungstenite`、`tokio-rustls`、`reqwest`、`redis` 和 `sqlx` 在内部使用真实的 `tokio`。
在仿真下，WebSocket 和 HTTP I/O 运行在真实网络上。这是有意的：
初始目标是订单生命周期确定性，而非传输故障注入。传输层
确定性将需要逐 crate 的 `madsim` 垫片，而这些垫片目前并不存在。

驱动真实本地回环套接字的测试模块（`crates/network/src/socket/client.rs::tests`、
`::rust_tests`；`crates/network/src/websocket/client.rs::tests`、`::rust_tests`；
`crates/network/tests/websocket_proxy.rs`）在
`all(feature = "simulation", madsim)` 下被 cfg 屏蔽，因为它们的生产代码路径触达
`dst::time::*`（madsim 时间原语），而后者从一个
`#[tokio::test]` 运行时调用时会 panic。重试测试模块（`crates/network/src/retry.rs::tests`、
`::proptest_tests`）在仿真下运行：每个测试属性都通过 `cfg_attr` 在
`#[tokio::test(start_paused = true)]` 和 `#[madsim::test]` 之间切换，时间读取和
sleep 经由 `crate::dst::time` 路由，显式的虚拟时间推进则经由
一个受 `cfg` 屏蔽的 `advance_clock` 辅助函数，因此同一段测试体同时覆盖两种运行时。

### 信号处理

`nautilus_common::live::dst::signal` 暴露一个被路由的 `ctrl_c` re-export。
`crates/live/src/node.rs` 的运行循环经由它路由，因此由 `ctrl_c` 驱动的节点关闭
在 `cfg(madsim)` 下可从测试代码经由 `madsim::runtime::Handle::send_ctrl_c` 注入。
适配器二进制入口点仍直接调用 `tokio::signal::ctrl_c`，并保持被排除范围。

### 日志运行于真实的 OS 线程

日志子系统经由 `std::thread::Builder` 派生一个写入器线程，并使用
`std::sync::mpsc`。在仿真下，该线程不被派生，日志事件被丢弃。
日志输出在确定性契约之外：该写入器只写，从不读取或改变
仿真状态。

### 适配器

适配器 crate 不在初始 DST 契约的范围内。每个适配器都有自己的一组
`chrono::Utc::now`、`SystemTime::now`、`Uuid::new_v4` 和传输层调用点。一个
进入 DST 路径的适配器必须在其行为能被契约覆盖之前，针对直接的时钟、RNG 和传输用法
接受审计。

### 进程全局惰性状态在首次调用时消耗 RNG 字节

契约在单次运行时运行内成立。少数几个进程全局惰性
初始化会在首次调用时消耗 RNG 字节，这对于那些在一个进程内
运行两次有种子执行并比较其 trace 的框架很重要。

- `Ustr::from()` interner 在首次使用时分配并为其内部 map 设种子。
- `ahash::RandomState` 在首次实例创建时经由 `getrandom`（`madsim` 在
  `cfg(madsim)` 下会挂钩它）为自身设种子。

单运行时测试（每个种子一次测试体调用、全新进程）不受影响：
该消耗是有种子执行的一部分，并确定性地复现。

那些在一个进程内调用测试体两次以 diff trace 的相同种子等价性框架
会在两次运行间看到漂移。运行 1 支付惰性初始化成本并消耗
RNG 字节；运行 2 继承了预热状态，并从 RNG 序列中的一个
不同偏移处开始。

变通办法：在比较之前于运行时之外预热进程全局状态，
例如在进程启动时调用一次 `Ustr::from("")` 并构造一个
`ahash::RandomState`。这样两次运行都从预热后的状态开始，
并以完全相同的方式消耗 RNG 序列。

### 适配器工厂不再暴露 `Rc<RefCell<Cache>>`

提交 `f0ea66da15`（"Standardize adapter cache access via `CacheView`"）将
`DataClientFactory::create` 和 `ExecutionClientFactory::create` 上的可变
`Rc<RefCell<Cache>>` 参数替换为一个 `CacheView`。`CacheView` 暴露
`borrow()` 用于读取访问，但不暴露内部的 `Rc` 句柄。

这阻碍了那些需要在工厂内部内联构造一个 `OrderMatchingEngine` 的
DST 风格框架工厂，因为
`OrderMatchingEngine::new` 仍然接收 `Rc<RefCell<Cache>>`，而且没有
公共访问器可以从 `CacheView` 恢复该句柄。

变通办法：

- 将框架消费者固定到 `f0ea66da15` 之前的某个提交。
- 重构框架，使其在工厂之外（在那里 kernel 的 `Cache` 句柄仍可触达）
  构建 `OrderMatchingEngine`，并将构造好的引擎传入客户端。

一个更长期的逃逸口子要么是在 `CacheView` 上为内部句柄提供一个访问器，
要么是一个接受 `CacheView` 的 `OrderMatchingEngine` 替代构造器。两者
今天都不在代码树中。

## 与其他测试层的关系

DST 是对现有测试的补充；它不替代其中任何一个。

| 层           | 覆盖                | 与 DST 的关系           |
| ----------- | ----------------- | ------------------- |
| 单元测试        | 纯逻辑、计算、解析器、转换器。   | 不变。                 |
| 集成测试        | 组件交互、I/O 边界。      | 不变。DST 并行运行，而非取而代之。 |
| 基于属性的测试     | 输入域上的不变量（解析器、往返）。 | 不变。                 |
| 验收测试        | 端到端回测和实盘场景。       | 不变。                 |
| 确定性仿真 (DST) | 异步时序、调度、恢复正确性。    | 增加种子可重放的探索。         |

DST 的独特价值在于异步并发与状态机正确性的交集。
诸如"某条消息在关闭时在特定唤醒顺序下被丢弃"或"当迭代顺序反转时
某个对账事件丢失"这样的 bug 是其目标类别。对于其他任何东西，
预先存在的测试层才是合适的工具。

## 状态

截至本仓库的当前状态：

- 第 1 层（运行时替换）已实现。`nautilus_common::live::dst` 为 `time`、`task`、
  `runtime` 和 `signal` 暴露被路由的 re-export。`time`、`task` 和
  `runtime` 的生产调用点经由该接缝路由；信号调用点的采用是部分的（见"范围边界"
  下的"信号处理"）。
- 第 2 层（非确定性替换）已在 16 个范围内 crate 中实现。接缝
  为挂钟时间、单调时间、随机性和迭代顺序而存在。审计
  闭包和剩余的被允许调用点在"实现说明"下逐项列出。
- 经由 `check-dst-conventions` 的静态强制执行在 pre-commit 和 CI 中处于激活状态。该钩子
  覆盖承重条件；`// dst-ok` 标记约定在有正当理由时允许逐行的
  例外。
- `cfg(madsim)` 下的构建与测试冒烟门经由 `dst` 工作流运行
  （`.github/workflows/dst.yml`，调用 `make cargo-test-sim`）。它以
  `--features simulation` 编译范围内 crate，并运行今天每个 sim 兼容的测试。
  消费 `nautilus-model` 类型的 crate（`nautilus-common`、`nautilus-execution`）
  还会以 `--features "simulation,high-precision"` 运行第二轮，以便那些经接缝路由的
  代码路径在两种定点宽度下（`QuantityRaw` / `PriceRaw` 作为
  `u64` 对比 `u128`）都被执行。
  - 全部的 `nautilus-common`。这一轮在传播 `nautilus-core/simulation` 的情况下编译，
    因此对于套件中的每个测试都选择显式的 `wall_clock_now` cfg 分支。纯 `#[rstest]`
    测试运行在 madsim 运行时之外，并经由接缝的 `SystemTime::now()` 回退路由
    （即 madsim 的 libc 垫片在运行时之外采取的相同路径）。这一轮中的
    `live::dst::tests::test_dst_wall_clock_advances_with_virtual_time`
    测试使用 `#[madsim::test]`，并断言 `nanos_since_unix_epoch`
    随 `madsim::time::sleep` 推进，因此虚拟挂钟行为在 common 这一轮上
    被端到端验证。
  - 全部的 `nautilus-network`（受传输约束的测试模块在源码层被屏蔽）。包括
    针对 sleep / timeout 虚拟时间和速率限制器的接缝固定测试，
    外加在虚拟时间下执行 backoff 计时的重试套件。
  - 全部的 `nautilus-execution`。撮合引擎、成交模型和执行引擎
    状态机在确定性调度器下以有种子的 RNG 运行。
  - `nautilus-core` 中的跨 crate 接缝固定测试（`wall_clock_now` 虚拟
    时间）。每一轮以它自己 crate 的 `--features simulation` 运行，并在适用处使用
    `#[madsim::test]`，因此显式 cfg 分支和虚拟时间
    都被验证。

  这些合在一起捕捉受 cfg 屏蔽的 DST 接缝中的漂移，并在确定性调度器下
  执行范围内的状态机；它尚未端到端地
  执行确定性。
- 端到端运行时验证（针对某条范围内代码路径的相同种子 diff）不在
  本仓库的范围内。结构化条件（规则 1 到规则 6）已被强制执行；
  关于一个种子在多次运行间复现完全相同可观测行为的声称，从接缝设计来看
  是合理的，但尚未由一个回归门验证。

## 延伸阅读

- `.pre-commit-hooks/check_dst_conventions.sh` 完整定义了这五条强制执行规则，并
  记录了 `// dst-ok` 标记约定。
- 外部参考：[FoundationDB 测试
  哲学](https://apple.github.io/foundationdb/testing.html)、[TigerBeetle 仿真
  测试博客文章](https://tigerbeetle.com/blog/)，以及
  关于确定性运行时的 [madsim 仓库](https://github.com/madsim-rs/madsim)。
