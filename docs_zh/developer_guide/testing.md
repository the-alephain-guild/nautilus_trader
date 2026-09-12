# 测试 (Testing)

我们的自动化测试（automated tests）作为交易平台的可执行规范。
一套健康的测试套件记录了预期行为，让贡献者有信心进行重构，并在问题到达生产环境之前捕获回归缺陷。
测试同时也是活的示例，有助于阐明复杂流程并提供快速的 CI 反馈，使问题尽早浮现。

测试套件涵盖以下类别：

- 单元测试（Unit tests）
- 集成测试（Integration tests）
- 验收测试（Acceptance tests）
- 性能测试（Performance tests）
- 基于属性的测试（Property-based tests）
- 模糊测试（Fuzzing）
- 内存泄漏测试（Memory leak tests）

## 测试策略

测试与运行时契约构成同一套设计体系。
[契约式设计 (Design by contract)](rust.md#契约式设计) 阶梯尽可能将不变量推入类型系统；而下面的测试阶梯则通过更大的输入空间和更丰富的执行模型，逐级处理剩余的未知项。
每一层都将覆盖范围扩展到下层无法触及的输入或执行状态。

并非每个模块都需要每一种技术。在添加测试或 `debug_assert!` 语句之前，请用本节内容来决定应当应用哪些层。

### 机制阶梯 (Mechanism ladder)

运行时契约在 [Rust 指南](rust.md#契约式设计) 中有所介绍：优先使用类型系统，然后在 API 边界使用 `nautilus_core::correctness` 中的 `check_*`，再对内部不变量使用 `debug_assert!`，最后对可靠性关键或需要始终生效的检查使用 `assert!`。

测试各层遵循平行的逐级升级。从能证明关键问题的最低层开始；只有当下层不再能检测到回归，或输入空间增长到超出手工挑选的用例范围时，才向上攀登。

| 层级      | 触发条件                                      |
| ------- | ----------------------------------------- |
| 单元测试    | 单个函数或状态转换具有一个小而可枚举的用例集合。                  |
| 参数化测试   | 同一形态在离散输入（订单方向、状态、金融工具）上重复出现。             |
| 基于属性的测试 | 某个不变量必须对人脑无法枚举的一整类输入都成立。                  |
| 集成测试    | 多个模块通过真实（非 mock）的引擎或运行时进行交互。              |
| 模糊测试    | 不可信或对抗性的字节流穿过解析器、解码器或线格式（wire-format）处理器。 |
| 规范验收测试  | 行为依赖于实时交易场所契约（参见 `spec_exec_testing.md`）。 |
| 确定性仿真   | 正确性依赖于任务调度、超时或挂钟时间（wall-clock）顺序。         |
| 形式化验证   | 一个纯函数具有清晰的不变量和有界的输入空间，值得进行证明。             |

形式化验证这一档是前瞻性的：工作区中尚未落地任何 Kani 或 Prusti 测试框架。该行记录的是当未来采用某个验证器时的升级条件，而非当前的义务。

### 投影规则 (Projection rule)

模块的形态决定了哪些层值得投入。并非每个模块都需要完整的阶梯。
请在模块粒度（而非 crate 粒度）上应用此规则：一个适配器 crate 既包含纯解析器，也包含受 I/O 约束的客户端循环，而每一行规则适用于其中不同的部分。

| 模块形态            | 适用的层               | 示例               |
| --------------- | ------------------ | ---------------- |
| 纯函数，不变量清晰       | 单元、参数化、属性、模糊       | 对账内核、组合数学运算      |
| 纯函数，未声明不变量      | 单元、参数化、属性、模糊       | 编解码器、适配器解析器、格式化器 |
| 有状态，同步          | 单元、参数化、针对状态转换的属性测试 | Cache、订单簿        |
| 有状态，异步          | 单元、集成、确定性仿真        | 实时引擎、执行管理器       |
| 受 I/O 约束，交易场所契约 | 集成、规范验收、边界模糊       | 适配器客户端循环         |

### 何时不应添加覆盖

- 仅在测试能够到达的地方添加 `debug_assert!`。Release 构建会剥离该检查，因此一个未被运行的断言没有任何信号。一个有针对性的单元测试即可算作测试框架；而 proptest 或模糊测试框架则能放大该信号。
- 当不变量横跨一整类输入时，优先使用 proptest 而非手写的边界用例测试。对于已知的交易场所异常情况，以及作为已收缩反例的回归复现器，有针对性的单元测试仍然有效。
- 不要把一张实时规范验收卡片重复实现为集成测试。改为链接到它。
- 不要用断言语言或框架保证的测试来凑覆盖率（如在 `Some(..)` 之后断言 `Option::is_some`，在 `push` 之后断言 `Vec::len`）。

### DST 就绪 (DST readiness)

确定性仿真测试（Deterministic simulation testing，DST）要求运行时不存在隐式的非确定性。在将某个模块提升到在 DST 下运行之前，请验证以下各项：

- 时间、任务、运行时和信号原语都经由 `nautilus_common::live::dst` 路由，而不是直接使用 `tokio`。挂钟时间读取经由 `nautilus_core::time` 中的接缝（seam），而不是在调用处使用 `SystemTime::now()`。
- 依赖迭代顺序的状态映射使用 `IndexMap` 或 `IndexSet`，而不是默认的哈希集合。
- 控制平面路径上的每个 `tokio::select!` 都设置 `biased`，以固定 poll 顺序。
- 没有任何对 `Instant::now()`、`SystemTime::now()`、`tokio::signal::ctrl_c`、`std::thread::spawn` 或 `tokio::task::spawn_blocking` 的调用绕过接缝。阻塞线程和 OS 线程原语会以与隐式时钟读取相同的方式破坏 madsim 的确定性。
- 对重放（replay）敏感的 ID（`trade_id`、`venue_order_id`）是其输入的纯函数；参见 `crates/execution/src/reconciliation/ids.rs`。其他对账路径上的临时事件 UUID 不需要是确定性的。

`crates/common/src/live/dst.rs` 中的 `surface` 探针仅固定了 re-export 的形态；它并不检查调用方是否实际使用了接缝。强制约束依靠代码评审来完成。每当有新的异步模块进入工作区，或现有模块新增了控制平面调度时，都应运行此审计。

## 基于属性的测试

属性测试验证逻辑对*所有*有效输入都成立，而不仅仅是手工挑选的示例。
我们在 Rust 中使用 [`proptest`](https://altsysrq.github.io/proptest-book/intro.html) 来强制约束不变量。

- **用例：** 核心领域类型（`Price`、`Quantity`、`UnixNanos`）、记账引擎、撮合引擎和状态机。
- **不变量示例：**
  - 往返序列化：`parse(to_string(value)) == value`
  - 逆运算：`(A + B) - B == A`
  - 传递性：`若 A < B 且 B < C，则 A < C`

## 模糊测试

模糊测试向系统引入非结构化或恶意数据，以验证它能够优雅地失败。

- **用例：** 网络边界、交易所数据解析器（JSON、FIX、WebSocket feed）和复杂状态机。
- **目标：** 当遇到格式错误的数据时，系统返回 `Result::Err`，而绝不 panic、挂起或泄漏内存。

在构建或修改核心类型时，编写属性测试以覆盖数学边界。

性能测试有助于持续改进性能关键组件。

使用 [pytest](https://docs.pytest.org) 运行测试，它是我们的主要测试运行器。
使用参数化测试和 fixture（如 `@pytest.mark.parametrize`）以避免重复代码并提升清晰度。

## 运行测试

### v1 旧版 Python 测试

v1 旧版测试套件位于仓库根目录的 `tests/` 下，测试基于 Cython 的包。从仓库根目录：

```bash
make pytest
# 或
uv run --active --no-sync pytest --new-first --failed-first
```

### Python 测试

Python 测试套件位于 `python/tests/` 下，测试由 Rust 支持的 PyO3 包。它需要一个已构建的扩展模块（`make build-debug-v2`），并使用其自身位于 `python/.venv/` 下的虚拟环境。

对于 v2 路径中新的实时适配器示例和文档，优先使用 `nautilus_trader.live.LiveNode`。`nautilus_trader.live.node.TradingNode` 仍然是根目录 `tests/` 套件和较旧示例所使用的旧版 v1/Cython 运行时。

```bash
make pytest-v2
```

该 Makefile 目标会将某些测试模块隔离到独立的 pytest 进程中，以避免全局 Rust 状态冲突。请使用 `make pytest-v2` 而不是直接调用 pytest。

本地 `make pytest-v2` 运行使用来自 `make build-debug-v2` 的调试扩展。
CI 的 `build-v2` 测试 release wheel。
不要在 `python/tests/` 中编写用 `pytest.raises(BaseException)` 或类似宽泛捕获在进程内探测 Rust panic 路径的用例。
这类测试可能在对调试构建时看似通过，却会在对 release wheel 时中止解释器。
对于容易中止的 PyO3 或 FFI 方法，请验证 Python 签名和参数名，或将该调用隔离到子进程中。

运行性能测试：

```bash
make test-performance
# 或
uv run --active --no-sync pytest tests/performance_tests --benchmark-disable-gc --codspeed
```

`--benchmark-disable-gc` 标志可防止垃圾回收影响结果。性能测试应单独运行（不与单元测试混合），以避免干扰。

### Rust 测试

```bash
make cargo-test
# 或
cargo nextest run --workspace --features "python,ffi,high-precision,defi" --cargo-profile nextest
```

#### 使用可选特性进行测试

使用 `EXTRA_FEATURES` 来包含可选特性，如 `capnp` 或 `hypersync`：

```bash
# 使用 capnp 特性测试
make cargo-test EXTRA_FEATURES="capnp"

# 使用多个特性测试
make cargo-test EXTRA_FEATURES="capnp hypersync"

# hypersync 的旧版简写
make cargo-test HYPERSYNC=true

# 使用特性测试特定 crate
make cargo-test-crate-nautilus-serialization FEATURES="capnp"
```

### IDE 集成

- **PyCharm**：右键点击测试文件夹或文件 -> "Run pytest"。
- **VS Code**：使用 Python Test Explorer 扩展。

## 测试风格

### 通用

- 以测试所验证的内容命名测试函数；不需要在名称中编码预期的断言。
- 当文档字符串有助于阐明设置、场景或预期时，添加文档字符串。
- **合并断言**：尽可能先执行所有 setup/act 步骤，然后统一断言，避免 act-assert-act 反模式。
- 在测试中使用 `unwrap`、`expect` 或直接的 `panic!`/`assert` 调用；此处清晰和简洁比防御性错误处理更重要。
- 不要捕获日志输出来对日志消息进行断言。测试中的日志捕获很脆弱，因为日志器是全局状态、测试执行顺序是非确定性的，而且当日志措辞改变时断言会失效。相反，应验证该日志消息所反映的可观测行为（返回值、状态变化、副作用）。

### Python 测试（`python/tests/`）

使用 **pytest 风格的自由函数和 fixture**。不要使用测试类。

- 将每个测试编写为独立的 `def test_*()` 函数。
- 使用 `@pytest.fixture` 进行共享设置（金融工具、引擎实例、数据）。当需要清理时优先使用 `yield` 形式的 fixture（如 `engine.dispose()`）。
- 使用 `@pytest.mark.parametrize` 来覆盖多个输入，而无需重复测试主体。
- 从 `nautilus_trader.model` 导入模型类型，而不是从 `nautilus_trader.core.nautilus_pyo3`。
- 测试 provider 位于 `python/tests/providers.py`。使用 `TestInstrumentProvider` 和 `TestDataProvider` 获取常用金融工具和数据。
- 对依赖未完成特性的测试，使用 `@pytest.mark.skip(reason="WIP: <description>")` 标记，而不是删除它们。

### v1 旧版 Python 测试（`tests/`）

v1 旧版测试套件混用测试类和自由函数。新加入此套件的测试可以遵循任一模式，但对于新文件，优先使用带 fixture 的自由函数。

### Rust

有关 Rust 特定的测试约定（模块结构、`#[rstest]`、参数化），请参阅 [Rust 指南](rust.md#测试约定)。

## 等待异步效果

等待后台工作完成时，优先使用 `nautilus_trader.test_kit.functions` 中的轮询辅助函数 `await eventually(...)` 和 `nautilus_common::testing` 中的 `wait_until_async(...)`，而不是任意的 sleep。它们能更快地暴露失败，并减少 CI 中的不稳定性，因为它们会在条件满足时立即停止，或在超时时给出有用的错误。

## Mock

优先使用手写的 stub（返回固定值），而不是 mock 框架。仅在需要断言调用次数/参数或模拟复杂状态变化时使用 `MagicMock`。避免对被测试的对象本身进行 mock。

## 代码覆盖率

我们使用 `coverage` 生成覆盖率报告，并发布到 [codecov](https://about.codecov.io/)。

目标是在不牺牲适当错误处理或对架构造成"测试引发的损害"的前提下实现高覆盖率。

某些分支在不修改生产行为的情况下无法测试。
例如，防御性 if-else 块中的最终条件可能只在遇到意外值时才触发；保留这些检查以便未来的更改在需要时可以测试它们。

设计时的异常也可能不切实际地进行测试，因此 100% 覆盖率不是目标。

## 排除代码覆盖

我们使用 `pragma: no cover` 注释来[排除代码覆盖](https://coverage.readthedocs.io/en/coverage-4.3.3/excluding.html)，以避免冗余测试。
典型示例包括：

- 断言抽象方法在被调用时抛出 `NotImplementedError`。
- 断言 if-else 块中无法测试的最终条件检查（如上所述）。

此类测试维护成本高，因为它们必须跟踪重构但提供的价值很小。
确保抽象方法的具体实现保持完全覆盖。
当 `pragma: no cover` 不再适用时移除它，并将其使用限制在上述情况。

## 调试 Rust 测试

使用默认测试配置来调试 Rust 测试。

要使用调试符号运行完整套件以便后续分析，运行 `make cargo-test-debug` 而非 `make cargo-test`。

在 IntelliJ IDEA 中，为参数化 `#[rstest]` 用例调整运行配置，使其读取 `test --package nautilus-model --lib data::bar::tests::test_get_time_bar_start::case_1`
（移除 `-- --exact` 并追加 `::case_n`，其中 `n` 从 1 开始）。此解决方法与[此处](https://github.com/rust-lang/rust-analyzer/issues/8964#issuecomment-871592851)说明的行为一致。

在 VS Code 中，你可以直接选择特定的测试用例进行调试。

## Python + Rust 混合调试

此工作流允许你在 VS Code 的 Jupyter notebook 中同时调试 Python 和 Rust 代码。

### 设置

安装以下 VS Code 扩展：Rust Analyzer、CodeLLDB、Python、Jupyter。

### 步骤 0：以调试符号编译 `nautilus_trader`

   ```bash
   cd nautilus_trader && make build-debug-pyo3
   ```

### 步骤 1：设置调试配置

```python
from nautilus_trader.test_kit.debug_helpers import setup_debugging

setup_debugging()
```

此命令会创建所需的 VS Code 调试配置，并为 Python 调试器启动 `debugpy` 服务器。

默认情况下，`setup_debugging()` 期望 `.vscode` 文件夹位于 `nautilus_trader` 根目录的上一级。
如果你的工作区布局不同，请调整目标位置。

### 步骤 2：设置断点

- **Python 断点：** 在 VS Code 中的 Python 源文件中设置。
- **Rust 断点：** 在 VS Code 中的 Rust 源文件中设置。

### 步骤 3：启动混合调试

1. 在 VS Code 中选择 **"Debug Jupyter + Rust (Mixed)"** 配置。
2. 启动调试（F5）或点击绿色运行箭头。
3. Python 和 Rust 调试器都会附加到你的 Jupyter 会话。

### 步骤 4：执行代码

运行调用 Rust 函数的 Jupyter notebook 单元格。调试器会在 Python 和 Rust 代码的断点处停下。

### 可用配置

`setup_debugging()` 创建以下 VS Code 配置：

- **`Debug Jupyter + Rust (Mixed)`** - Jupyter notebook 的混合调试。
- **`Jupyter Mixed Debugging (Python)`** - notebook 的纯 Python 调试。
- **`Rust Debugger (for Jupyter debugging)`** - notebook 的纯 Rust 调试。

### 示例

打开并运行示例 notebook：`debug_mixed_jupyter.ipynb`。

### 参考

- [PyO3 调试](https://pyo3.rs/v0.25.1/debugging.html?highlight=deb#debugging-from-jupyter-notebooks)

## 数据类型测试

每种数据类型都会流经平台的多个层。下表展示了现有类型在何处被测试，以便新类型可以遵循相同的模式。

### 测试层矩阵

| 层                      | 位置                                          | 覆盖内容                              |
| ---------------------- | ------------------------------------------- | --------------------------------- |
| DataEngine subscribe   | `crates/data/tests/engine.rs`               | 引擎正确处理订阅/取消订阅命令。                  |
| DataEngine publish     | `crates/data/tests/engine.rs`               | 引擎将已发布的数据路由到消息总线。                 |
| DataActor subscribe    | `crates/common/src/actor/tests.rs`          | Actor 通过类型化发布订阅并接收数据。             |
| DataActor unsubscribe  | `crates/common/src/actor/tests.rs`          | Actor 在取消订阅后停止接收数据。               |
| PyO3 actor dispatch    | `crates/common/src/python/actor.rs`         | Rust 处理器分派到 Python 的 `on_*` 方法。   |
| Python Actor subscribe | `tests/unit_tests/common/test_actor.py`     | Python actor 订阅；命令计数递增。           |
| Python Actor unsub     | `tests/unit_tests/common/test_actor.py`     | Python actor 取消订阅；订阅列表清空。         |
| Backtest client        | `nautilus_trader/backtest/data_client.pyx`  | 回测客户端覆盖基类的 subscribe/unsubscribe。 |
| Adapter live tests     | `docs/developer_guide/spec_data_testing.md` | 实时数据验收测试（DataTester）。             |

### 各数据类型的覆盖情况

下表展示了每种数据类型在哪些层有测试覆盖。
在添加新类型时将其用作检查清单。

| 数据类型                | Engine | Actor (Rust) | PyO3 dispatch | Actor (Python) | Backtest client | Adapter spec |
| ------------------- | ------ | ------------ | ------------- | -------------- | --------------- | ------------ |
| `InstrumentAny`     | ✓      | ✓            | ✓             | ✓              | ✓               | ✓            |
| `OrderBookDeltas`   | ✓      | ✓            | ✓             | ✓              | ✓               | ✓            |
| `OrderBook`         | ✓      | ✓            | ✓             | ✓              | ✓               | ✓            |
| `QuoteTick`         | ✓      | ✓            | ✓             | ✓              | ✓               | ✓            |
| `TradeTick`         | ✓      | ✓            | ✓             | ✓              | ✓               | ✓            |
| `Bar`               | ✓      | ✓            | ✓             | ✓              | ✓               | ✓            |
| `MarkPriceUpdate`   | ✓      | ✓            | ✓             | ✓              | ✓               | ✓            |
| `IndexPriceUpdate`  | ✓      | ✓            | ✓             | ✓              | ✓               | ✓            |
| `FundingRateUpdate` | ✓      | ✓            | ✓             | ✓              | ✓               | ✓            |
| `InstrumentStatus`  | ✓      | ✓            | ✓             | ✓              | ✓               | ✓            |
| `InstrumentClose`   | ✓      | ✓            | ✓             | ✓              | ✓               | ✓            |
| `OptionGreeks`      | ✓      | ✓            | ✓             | ✓              | ✓               | ✓            |
| `OptionChainSlice`  | -      | ✓            | ✓             | ✓              | -               | ✓            |
| `CustomData`        | ✓      | ✓            | ✓             | ✓              | ✓               | -            |

`OptionChainSlice` 由 DataEngine 的 `OptionChainManager` 从各金融工具的 greeks 和报价订阅中组装而成。它没有自己的引擎订阅命令或回测客户端覆盖。

### 添加新数据类型

在引入新数据类型时，在每一层添加测试：

1. **DataEngine**（`crates/data/tests/engine.rs`）：添加 `test_execute_subscribe_<type>` 和 `test_execute_unsubscribe_<type>` 测试。遵循现有订阅测试中的模式：注册客户端、构建命令、调用 `engine.execute`、断言订阅列表。

2. **DataActor Rust**（`crates/common/src/actor/tests.rs`）：
   - 向 `TestDataActor` 添加 `received_<type>: Vec<Type>` 字段。
   - 在 `DataActor` trait 实现中实现 `on_<type>` 处理器。
   - 添加 `test_subscribe_and_receive_<type>` 和 `test_unsubscribe_<type>` 测试。
   - 对于使用 `TypedHandler` 路由的类型，使用类型化发布函数（`msgbus::publish_<type>`），而不是 `publish_any`。

3. **PyO3 actor dispatch**（`crates/common/src/python/actor.rs`）：
   - 添加调用 `py_self.call_method1("on_<type>", ...)` 的 `dispatch_on_<type>` 方法。
   - 在 `DataActor` trait 实现中添加调用该分派方法的 `on_<type>`。
   - 在 `#[pymethods]` 块中添加 `#[pyo3(name = "on_<type>")]` 方法。
   - 向 `RustTestDataActor` 包装器和内联 Python 测试类添加 `on_<type>`。
   - 添加处理器测试和分派测试。

4. **Python Actor**（`tests/unit_tests/common/test_actor.py`）：
   - 添加 `test_subscribe_<type>` 和 `test_unsubscribe_<type>` 测试。
   - 断言 `actor.subscribed_<type>()` 在订阅后返回预期条目，并在取消订阅后为空。

5. **Backtest client**（`nautilus_trader/backtest/data_client.pyx`）：如果基类 `MarketDataClient` 对该方法抛出 `NotImplementedError`，则覆盖 `subscribe_<type>` 和 `unsubscribe_<type>`。

6. **文档**：向 `actors.md` 的回调表、`strategies.md` 的处理器签名、`adapters.md` 的订阅方法存根以及 `spec_data_testing.md` 的测试卡片添加条目。

:::tip
在所有六层中搜索一个现有类型（如 `instrument_close` 或 `funding_rate`），以找到上述模式的具体示例。
:::
