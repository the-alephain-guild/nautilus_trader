# Rust

[Rust](https://www.rust-lang.org/learn) 编程语言非常适合实现平台和系统的关键核心（mission-critical core）。
其强类型系统、所有权模型（ownership model）和编译时检查从根本上消除了内存（memory）错误和数据竞争（data race），
同时零成本抽象（zero-cost abstraction）和无垃圾回收器的特性提供了接近 C 语言的性能——这对高频交易工作负载至关重要。

## Cargo 清单约定

- 在 `[dependencies]` 中，先按字母顺序列出内部 crate（`nautilus-*`），插入一个空行，然后按字母顺序列出外部必需依赖，再插入一个空行，最后按字母顺序列出可选依赖（带有 `optional = true` 的）。保留依赖项的行内注释。
- 在每个构建 Python 产物的 `extension-module` 特性列表中添加 `"python"`，使其紧邻 `"pyo3/extension-module"`，以便完整的 Python 栈一目了然。
- 当清单单独分组适配器时（例如 `crates/pyo3`），将 `# Adapters` 块紧接在内部 crate 列表下方，以便下游使用者能快速浏览适配器覆盖范围。
- 始终在 `[dev-dependencies]` 和 `[build-dependencies]` 部分前保留一个空行。
- 当特性或依赖集发生变化时，对相关清单应用相同的布局，以避免 crate 之间的偏差。
- 对 `bin/` 源文件使用 snake_case 文件名（例如 `bin/ws_data.rs`），并在每个 `[[bin]]` 部分中反映这些路径。
- 保持 `[[bin]] name` 条目使用 kebab-case（例如 `name = "hyperliquid-ws-data"`），以便编译后的二进制文件保留其预期的 CLI 名称。

## 版本管理指南

- 对共享依赖使用工作区继承（workspace inheritance）（例如 `serde = { workspace = true }`）。
- 仅对不属于工作区的 crate 特有依赖直接固定版本。
- 将工作区提供的依赖分组放在 crate 专用依赖之前，以便继承关系易于审查。
- 保持相关依赖对齐：`capnp`/`capnpc`（精确版本）、`arrow`/`parquet`（major.minor）、
  `datafusion`/`object_store` 以及 `dydx-proto`/`prost`/`tonic`。pre-commit 会强制执行这一点。
- 仅适配器使用的依赖应放在工作区 `Cargo.toml` 的 "Adapter dependencies" 部分。
  pre-commit 会阻止核心 crate 使用它们。

## 特性标志约定

- 优先使用增量特性标志——启用一个特性不应破坏现有功能。
- 使用描述性的标志名称来说明启用了什么功能。
- 在 crate 级别的文档中记录每个特性，以便使用者了解其切换的内容。
- 常见模式：
  - `high-precision`：切换值类型支撑（64 位或 128 位整数），以支持需要更高精度的领域。
  - `default = []`：保持默认值最小化。
  - `python`：启用 Python 绑定。
  - `extension-module`：构建 Python 扩展模块（始终包含 `python`）。
  - `ffi`：启用 C FFI 绑定。
  - `stubs`：暴露测试桩（testing stubs）。

## 构建配置

为避免开发过程中不必要的重新构建，请在不同构建目标之间对齐 cargo 特性、配置文件（profile）和标志。
Cargo 的构建缓存以特性、配置文件和标志的精确组合为键——任何不匹配都会触发完全重新构建。

### 对齐目标（测试和代码检查）

| 目标                         | 特性                               | 配置文件      | `--all-targets` | `--no-deps` | 用途      |
| -------------------------- | -------------------------------- | --------- | --------------- | ----------- | ------- |
| `cargo-test`               | `ffi,python,high-precision,defi` | `nextest` | ✓（隐式）           | 不适用         | 运行测试。   |
| `cargo-clippy`（pre-commit） | `ffi,python,high-precision,defi` | `nextest` | ✓               | 不适用         | 检查所有代码。 |

这些目标共享相同的特性集和配置文件，使 cargo 能在代码检查和测试之间复用编译产物，无需重新构建。
`nextest` 配置文件的使用与大多数核心维护者使用 cargo-nextest 运行测试的工作流程保持一致。

### 文档构建

文档使用 `make docs-rust` 单独构建，其运行：

```bash
cargo +nightly doc --all-features --no-deps --workspace
```

此命令使用 nightly 工具链和 `--all-features`，而非上述对齐的特性集，因此不会与测试/代码检查共享构建产物。

### 独立目标（Python 扩展构建）

| 目标            | 特性                         | 配置文件      | 说明                    |
| ------------- | -------------------------- | --------- | --------------------- |
| `build`       | 包含 `extension-module` + 子集 | `release` | 需要不同的特性以构建 PyO3 扩展模块。 |
| `build-debug` | 包含 `extension-module` + 子集 | `dev`     | 需要不同的特性以构建 PyO3 扩展模块。 |

Python 扩展构建有意使用不同的特性（`extension-module` 是必需的），因此会触发重新构建。这是预期且不可避免的。

### 需要避免的重新构建触发因素

以下任何不匹配都会导致完全重新构建：

- 不同的特性组合（例如 `--features "a,b"` vs `--features "a,c"`）。
- 不同的 `--no-default-features` 用法（启用/禁用默认特性）。
- 不同的配置文件（例如 `dev` vs `nextest` vs `release`）。

添加新的构建目标或修改现有目标时，请与测试/代码检查组保持对齐，以维持快速增量构建。

### 生成的 FFI 绑定与精度模式

当启用 `ffi` 特性时，`nautilus-model` 构建脚本会重新生成 `nautilus_trader/core/includes/model.h` 和
`nautilus_trader/core/rust/model.pxd`。这些文件编码了生成的 C/Cython 绑定是否使用高精度。
已提交的生成文件使用高精度。使用 `ffi` 编译 `nautilus-model` 的本地 cargo 命令应当
包含 `high-precision` 特性，或者避免重新生成这些文件。

使用 `BASE_FEATURES` 的 make 目标（例如 `make build-debug-v2`）已经包含
`high-precision`。漂移风险主要来自启用 `ffi` 但未使用对齐特性集的临时 cargo 命令。

对于不包含完整对齐特性集的窄范围检查，可使用 Rust 特性。在命令中保留环境变量覆盖，
以防陈旧的 shell 值强制使用标准精度绑定：

```fish
env HIGH_PRECISION=true cargo check -p nautilus-model --features ffi,python,high-precision
```

在提交 FFI 相关工作之前，验证这些生成文件没有发生漂移：

```fish
git diff -- nautilus_trader/core/includes/model.h nautilus_trader/core/rust/model.pxd
```

如果它们的更改仅仅是因为某个命令在未使用高精度的情况下运行，请使用 `HIGH_PRECISION=true`
重新运行该 cargo 命令。不要手动编辑生成的文件。

## 模块组织

- 保持模块专注于单一职责。
- 定义子模块时使用 `mod.rs` 作为模块根。
- 优先使用相对扁平的层次结构而非深层嵌套，以保持路径可管理。
- 为方便使用，从 crate 根重新导出（re-export）常用项。

## 代码风格和约定

### 文件头要求

所有 Rust 文件必须包含标准化的版权头：

```rust
// -------------------------------------------------------------------------------------------------
//  Copyright (C) 2015-2026 Nautech Systems Pty Ltd. All rights reserved.
//  https://nautechsystems.io
//
//  Licensed under the GNU Lesser General Public License Version 3.0 (the "License");
//  You may not use this file except in compliance with the License.
//  You may obtain a copy of the License at https://www.gnu.org/licenses/lgpl-3.0.en.html
//
//  Unless required by applicable law or agreed to in writing, software
//  distributed under the License is distributed on an "AS IS" BASIS,
//  WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
//  See the License for the specific language governing permissions and
//  limitations under the License.
// -------------------------------------------------------------------------------------------------
```

:::info[自动化执行]
`check_copyright_year.sh` pre-commit 钩子会验证版权头是否包含当前年份。
:::

### 代码格式化

运行 `make format` 时，rustfmt 会自动处理导入格式化。
该工具将导入分为几组（标准库、外部 crate、本地导入），并在每组内按字母顺序排序。

在此部分中，遵循以下间距规则：

- 在**函数之间留一个空行**（包括测试）——这提高了可读性，并与 `rustfmt` 的默认行为一致。
- 在**每个文档注释**（`///` 或 `//!`）**上方留一个空行**，使注释与前面的代码块清晰分离。

#### 字符串格式化

优先使用内联格式字符串而非位置参数：

```rust
// 推荐 - 使用变量名的内联格式
anyhow::bail!("Failed to subtract {n} months from {datetime}");

// 而非 - 位置参数
anyhow::bail!("Failed to subtract {} months from {}", n, datetime);
```

这使消息更具可读性和自文档性，尤其是当存在多个变量时。

### 类型限定

在代码中遵循以下类型限定约定：

- **anyhow**：始终完全限定 `anyhow` 宏（`anyhow::bail!`、`anyhow::anyhow!`）和 Result 类型（`anyhow::Result<T>`）。
- **Nautilus 领域类型**：不要完全限定 Nautilus 领域类型。导入后直接使用（例如 `Symbol`、`InstrumentId`、`Price`）。
- **tokio**：通常完全限定 `tokio` 类型，因为它们在标准库和其他 crate 中可能有等效项（例如 `tokio::spawn`、`tokio::time::timeout`）。

```rust
use nautilus_model::identifiers::Symbol;

pub fn process_symbol(symbol: Symbol) -> anyhow::Result<()> {
    if !symbol.is_valid() {
        anyhow::bail!("Invalid symbol: {symbol}");
    }

    tokio::spawn(async move {
        // 异步处理 symbol
    });

    Ok(())
}
```

:::info[自动化执行]
`check_anyhow_usage.sh` pre-commit 钩子会自动执行这些 anyhow 约定。
:::

### 日志记录

- 完全限定日志宏以明确后端：
  - 对所有 Rust 组件使用 `log::…`（`log::debug!`、`log::info!`、`log::warn!` 等）。
- 消息以大写字母开头，优先使用完整句子，省略末尾句号（例如 `"Processing batch"`，而非 `"Processing batch."`）。

:::info[自动化执行]
`check_logging_macro_usage.sh` pre-commit 钩子会强制使用完全限定的日志宏。
:::

### 错误处理

一致地使用结构化错误处理模式：

1. **主要模式**：对可能失败的函数使用 `anyhow::Result<T>`：

   ```rust
   pub fn calculate_balance(&mut self) -> anyhow::Result<Money> {
       // 实现
   }
   ```

2. **自定义错误类型**：对领域特定错误使用 `thiserror`：

   ```rust
   #[derive(Error, Debug)]
   pub enum NetworkError {
       #[error("Connection failed: {0}")]
       ConnectionFailed(String),
       #[error("Timeout occurred")]
       Timeout,
   }
   ```

3. **错误传播**：使用 `?` 运算符进行简洁的错误传播。

4. **错误创建**：优先使用 `anyhow::bail!` 进行带错误的提前返回：

   ```rust
   // 推荐 - 使用 bail! 进行提前返回
   pub fn process_value(value: i32) -> anyhow::Result<i32> {
       if value < 0 {
           anyhow::bail!("Value cannot be negative: {value}");
       }
       Ok(value * 2)
   }

   // 而非 - 冗长的 return 语句
   if value < 0 {
       return Err(anyhow::anyhow!("Value cannot be negative: {value}"));
   }
   ```

   **注意**：使用 `anyhow::bail!` 进行提前返回，但在闭包上下文中（如 `ok_or_else()`）无法提前返回时使用 `anyhow::anyhow!`。

5. **错误上下文**：`.context()` 消息使用小写字母以支持错误链式调用（专有名词/缩写除外）：

   ```rust
   // 好的 - 小写字母自然衔接
   parse_timestamp(value).context("failed to parse timestamp")?;

   // 例外 - 专有名词保持大写
   connect().context("BitMEX websocket did not become active")?;
   ```

:::info[自动化执行]
`check_error_conventions.sh` 和 `check_anyhow_usage.sh` pre-commit 钩子会强制执行这些错误处理模式。
:::

### 异步模式

使用一致的 async/await 模式：

1. **异步函数命名**：不需要特殊后缀；优先使用自然名称。
2. **Tokio 用法**：完全限定 tokio 类型（例如 `tokio::time::timeout`）。spawn 规则参见 [适配器运行时模式](#适配器运行时模式)。
3. **错误处理**：从异步函数返回 `anyhow::Result` 以与同步约定保持一致。
4. **取消安全性（cancellation safety）**：说明函数是否是取消安全的，以及取消时哪些不变量仍然成立。
5. **流处理**：使用 `tokio_stream`（或 `futures::Stream`）处理异步迭代器，以明确反压（back-pressure）。
6. **超时模式**：使用超时（`tokio::time::timeout`）包装网络或长时间运行的 await，并传播或处理超时错误。

### 适配器运行时模式

适配器 crate（位于 `crates/adapters/` 下）由于 Python FFI 兼容性，在派生（spawn）异步任务时需要特殊处理：

1. **使用 `get_runtime().spawn()` 而非 `tokio::spawn()`**：当从 Python 线程（它们没有 Tokio 上下文）调用时，`tokio::spawn()` 会 panic，因为它依赖线程本地存储。全局运行时模式提供了一个可从任何线程访问的显式引用。

   ```rust
   use nautilus_common::live::get_runtime;

   // 正确 - 可从 Python 线程工作
   get_runtime().spawn(async move {
       // 异步工作
   });

   // 错误 - 从 Python 线程会 panic
   tokio::spawn(async move {
       // 异步工作
   });
   ```

2. **使用更短的导入路径**：从 `live` 模块的重新导出处导入 `get_runtime`，而非完整路径：

   ```rust
   // 推荐 - 通过重新导出的更短路径
   use nautilus_common::live::get_runtime;

   // 避免 - 不必要的冗长
   use nautilus_common::live::runtime::get_runtime;
   ```

3. **使用 `get_runtime().block_on()` 进行同步到异步的桥接**：当同步代码需要在适配器中调用异步函数时：

   ```rust
   fn sync_method(&self) -> anyhow::Result<()> {
       get_runtime().block_on(self.async_implementation())
   }
   ```

4. **在首次使用前安装自定义运行时**：拥有 `main()` 的 Rust 原生二进制文件可以在
   `LiveNode::build()` 或任何适配器/客户端使用之前调用 `set_runtime()`。使用
   `tokio::runtime::Builder::new_multi_thread().enable_all()` 构建自定义运行时；
   当前线程（current-thread）运行时以及没有 I/O 或定时器驱动的运行时不满足适配器假设。
   如果启用了 `python` 特性，请在构建运行时之前准备好 Python，或保留默认初始化器。

5. **测试不受此约束**：使用 `#[tokio::test]` 的测试代码会创建自己的运行时上下文，因此
   `tokio::spawn()` 可以正确工作。强制执行钩子会跳过测试文件和测试模块。

:::info[自动化执行]
`check_tokio_usage.sh` pre-commit 钩子会自动执行这些适配器运行时模式。
:::

### 属性模式

一致的属性使用和排序：

```rust
#[repr(C)]
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(module = "nautilus_trader.model")
)]
#[cfg_attr(
    feature = "python",
    pyo3_stub_gen::derive::gen_stub_pyclass(module = "nautilus_trader.model")
)]
pub struct Symbol(Ustr);
```

对于具有大量 derive 属性的枚举：

```rust
#[repr(C)]
#[derive(
    Copy,
    Clone,
    Debug,
    Display,
    Hash,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    AsRefStr,
    FromRepr,
    EnumIter,
    EnumString,
)]
#[strum(ascii_case_insensitive)]
#[strum(serialize_all = "SCREAMING_SNAKE_CASE")]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(
        frozen,
        eq,
        eq_int,
        module = "nautilus_trader.model",
        from_py_object,
        rename_all = "SCREAMING_SNAKE_CASE",
    )
)]
#[cfg_attr(
    feature = "python",
    pyo3_stub_gen::derive::gen_stub_pyclass_enum(module = "nautilus_trader.model")
)]
pub enum AccountType {
    /// 仅包含无杠杆现金资产的账户。
    Cash = 1,
    /// 使用账户资产作为抵押品进行保证金交易的账户。
    Margin = 2,
}
```

### 类型桩注解

Python 类型桩（`.pyi` 文件）使用
[pyo3-stub-gen](https://github.com/Jij-Inc/pyo3-stub-gen) 从 Rust 源代码生成。每个
暴露给 Python 的类型和函数都需要匹配的桩注解，以使生成的桩与绑定保持同步。

**注解类型：**

| PyO3 构造         | 桩注解                                            |
| --------------- | ---------------------------------------------- |
| `#[pyclass]`    | `pyo3_stub_gen::derive::gen_stub_pyclass`      |
| 枚举 `#[pyclass]` | `pyo3_stub_gen::derive::gen_stub_pyclass_enum` |
| `#[pymethods]`  | `pyo3_stub_gen::derive::gen_stub_pymethods`    |
| `#[pyfunction]` | `pyo3_stub_gen::derive::gen_stub_pyfunction`   |

**放置规则：**

- 在结构体和枚举上，使用 `#[cfg_attr(feature = "python", ...)]`，并将桩注解
  放在 `pyo3::pyclass` 属性的正下方。
- 在 `#[pymethods]` impl 块上，将 `#[pyo3_stub_gen::derive::gen_stub_pymethods]`
  放在 `#[pymethods]` 的正下方。
- 在函数上，将桩注解放在 `#[pyfunction]` 的正上方、所有文档注释之后。
  使用完全限定路径而非导入它。

```rust
/// Converts a list of `Bar` into Arrow IPC bytes.
#[pyo3_stub_gen::derive::gen_stub_pyfunction(module = "nautilus_trader.serialization")]
#[pyfunction(name = "bars_to_arrow")]
pub fn py_bars_to_arrow(data: Vec<Bar>) -> PyResult<Py<PyBytes>> {
    // ...
}
```

```rust
#[pymethods]
#[pyo3_stub_gen::derive::gen_stub_pymethods]
impl AccountState {
    #[staticmethod]
    #[pyo3(name = "from_dict")]
    pub fn py_from_dict(values: &Bound<'_, PyDict>) -> PyResult<Self> {
        // ...
    }
}
```

**Module 参数**：设置 `module = "nautilus_trader.<package>"` 以匹配导入该类型的
Python 包。例如，model 类型使用 `nautilus_trader.model`，序列化函数使用
`nautilus_trader.serialization`。

**Cargo.toml**：将 `pyo3-stub-gen` 添加为可选依赖，并将其包含在 `python`
特性列表中：

```toml
[features]
python = ["pyo3", "pyo3-stub-gen"]

[dependencies]
pyo3-stub-gen = { workspace = true, optional = true }
```

**重新生成桩**：更改注解后运行 `make py-stubs-v2`（或 `python python/generate_stubs.py`）。
后处理器会处理 `py_` 前缀剥离、`@property`/`@staticmethod`/`@classmethod` 装饰、
关键字转义、去重以及 ruff 格式化。

### 构造函数模式

一致地使用 `new()` vs `new_checked()` 约定：

```rust
/// Creates a new [`Symbol`] instance with correctness checking.
///
/// # Errors
///
/// Returns an error if `value` is not a valid string.
///
/// # Notes
///
/// PyO3 requires a `Result` type for proper error handling and stacktrace printing in Python.
pub fn new_checked<T: AsRef<str>>(value: T) -> CorrectnessResult<Self> {
    // 实现
}

/// Creates a new [`Symbol`] instance.
///
/// # Panics
///
/// Panics if `value` is not a valid string.
pub fn new<T: AsRef<str>>(value: T) -> Self {
    Self::new_checked(value).expect_display(FAILED)
}
```

始终对 `CorrectnessResult` 上的 `.expect_display()` 消息使用 `FAILED` 常量，
并导入提供它的 trait：

```rust
use nautilus_core::correctness::{CorrectnessResult, CorrectnessResultExt, FAILED};
```

### 类型转换模式

对于从字符串解析的类型，同时提供可能失败和不可能失败的转换：

1. **`FromStr`**：通过 `.parse()` 或 `from_str()` 进行可能失败的解析。返回 `Result`。

2. **`From<T: AsRef<str>>`**：符合人体工程学的不可能失败的转换，直接接受 `&str`、`String`、`Cow<str>` 等，无需 `.as_str()`。

```rust
impl FromStr for Symbol {
    type Err = SymbolParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // 解析逻辑
    }
}

impl<T: AsRef<str>> From<T> for Symbol {
    fn from(value: T) -> Self {
        Self::from_str(value.as_ref()).expect(FAILED)
    }
}
```

**设计说明**：`From` 实现在输入无效时可能 panic。这是为了 API 人体工程学而有意为之。需要错误处理时使用 `FromStr` / `.parse()`。`From` 实现为已知输入有效的场景提供便利。

**约束**：此模式不能用于自身实现 `AsRef<str>` 的类型（例如字符串包装类型），因为它会与一揽子实现 `impl<T> From<T> for T` 冲突。对于此类类型，应改为分别提供 `From<&str>` 和 `From<String>` 实现。

### 常量和命名约定

常量使用 SCREAMING_SNAKE_CASE 并配以描述性名称：

```rust
/// Number of nanoseconds in one second.
pub const NANOSECONDS_IN_SECOND: u64 = 1_000_000_000;

/// Bar specification for 1-minute last price bars.
pub const BAR_SPEC_1_MINUTE_LAST: BarSpecification = BarSpecification {
    step: NonZero::new(1).unwrap(),
    aggregation: BarAggregation::Minute,
    price_type: PriceType::Last,
};
```

### 哈希集合

三个考量驱动哈希集合的选择：

- **迭代顺序确定性**（主要筛选条件）
- **性能**
- **线程安全性**

先回答确定性问题，然后从剩余选项中基于性能进行选择。

#### 迭代顺序确定性

`AHash` 在每个进程中随机化其哈希器，因此 `AHashMap` / `AHashSet`
的迭代顺序在不同运行之间会有所不同。当某个集合的迭代顺序在确定性
仿真测试（deterministic simulation testing，DST）路径上馈入可观测状态
（消息总线上发出的事件、公共方法返回的有序 `Vec`、有种子的 RNG 被消耗的
顺序、下游效应触发的顺序）时，请改用 `indexmap` crate 中的 `IndexMap` /
`IndexSet`。它们保留插入顺序，并且是 `AHash*` 集合的直接替换。

```rust
use indexmap::{IndexMap, IndexSet};

// 按插入顺序迭代；跨运行确定性
let mut commissions: IndexMap<Currency, Money> = IndexMap::new();
let mut subscribed: IndexSet<InstrumentId> = IndexSet::new();
```

pre-commit 钩子 `check-dst-conventions` 在 `crates/live/src/manager.rs` 和
`crates/execution/src/matching_engine/engine.rs` 中强制使用 `IndexMap` /
`IndexSet`，因为这两个文件经审计被认定对成交排序和对账（reconciliation）
具有承重作用。其他调用点会逐个审查；已关闭的调用点和剩余允许的模式
列在 [../concepts/dst.md](../concepts/dst.md) 的 "Implementation notes" 下。

当集合是**仅查找**的（没有 `.iter()`、`.values()`、`.keys()`、
`.into_iter()`、`.drain()` 或 `for x in map { ... }`）时，迭代顺序无关紧要，
基于性能考量 `AHashMap` / `AHashSet` 是正确的选择。边界情况
（例如克隆映射并让调用者迭代的公共 getter）应对照清单的分类规则进行审查。

#### 性能

对于迭代顺序不馈入可观测状态的查找密集型热路径，优先使用 `AHashMap` / `AHashSet`
而非标准库：

```rust
use ahash::{AHashMap, AHashSet};

let mut symbols: AHashSet<Symbol> = AHashSet::new();
let mut prices: AHashMap<InstrumentId, Price> = AHashMap::new();
```

对于非性能关键、非迭代敏感的场景（工厂注册表、配置映射、测试夹具），
标准 `HashMap` / `HashSet` 是可接受的，并且因其简洁性而常常更受青睐：

```rust
use std::collections::{HashMap, HashSet};

let mut symbols: HashSet<Symbol> = HashSet::new();
let mut prices: HashMap<InstrumentId, Price> = HashMap::new();
```

**为什么使用 `ahash`？**

- **卓越的性能**：AHash 在可用时使用 AES-NI 硬件指令，与默认的 SipHash 相比提供 2-3 倍更快的哈希速度。
- **低碰撞率**：尽管是非加密哈希，AHash 对典型数据提供了出色的分布和低碰撞率。
- **直接替换**：与标准库集合完全兼容的 API。

**何时使用标准 `HashMap`/`HashSet`：**

- **非性能关键的代码**：对于性能不关键的简单场景（例如工厂注册表、配置映射、测试夹具），标准 `HashMap`/`HashSet` 是可接受的，甚至因其简洁性而更受青睐。
- **需要加密安全性**：当哈希洪泛攻击是隐患时（例如处理网络协议中的不可信用户输入），使用标准 `HashMap`。
- **网络客户端**：对于面向网络的组件，优先使用标准 `HashMap`，因为安全考虑优先于性能收益。
- **外部库边界**：与期望标准 `HashMap` 的外部库交互时使用它（例如 Arrow 序列化元数据）。

#### AHashMap vs IndexMap 微基准测试

下面的数字来自 `crates/core/benches/hash_map.rs`（release
配置文件）。时间为每次操作的耗时；比率是 `IndexMap` 相对于
`AHashMap` 的比值（低于 1.0 的值有利于 `IndexMap`）。

| 模式                    | 大小   | AHashMap | IndexMap | 比率    |
| --------------------- | ---: | -------: | -------: | ----: |
| Insert（构建映射）          |    4 |  40.8 ns |  49.8 ns | 1.22x |
| Insert（构建映射）          |   32 | 192.4 ns | 348.2 ns | 1.81x |
| Insert（构建映射）          |  256 |  1.01 us |  2.74 us | 2.72x |
| Lookup（随机 get）        |    4 |  2.56 ns |  9.36 ns | 3.66x |
| Lookup（随机 get）        |   32 |  2.49 ns |  7.95 ns | 3.19x |
| Lookup（随机 get）        |  256 |  3.00 ns |  9.48 ns | 3.16x |
| `.values().collect()` |    4 |  8.08 ns |  6.61 ns | 0.82x |
| `.values().collect()` |   32 |  22.8 ns |  14.8 ns | 0.65x |
| `.values().collect()` |  256 |   145 ns |   109 ns | 0.75x |
| `.keys().collect()`   |    4 |  7.90 ns |  6.24 ns | 0.79x |
| `.keys().collect()`   |   32 |  23.0 ns |  12.6 ns | 0.55x |
| `.keys().collect()`   |  256 |   145 ns |   101 ns | 0.70x |
| Clone                 |    4 |  8.48 ns |  17.8 ns | 2.10x |
| Clone                 |   32 |  25.3 ns |  62.5 ns | 2.47x |
| Clone                 |  256 |  71.0 ns |   247 ns | 3.48x |
| Entry accumulate      |    4 |   122 ns |   159 ns | 1.30x |
| Entry accumulate      |   32 |   439 ns |  1.10 us | 2.51x |
| Entry accumulate      |  256 |  2.21 us |  7.83 us | 3.54x |

对于单键移除，`IndexMap` 提供两个方法：`shift_remove`
以 `O(n)` 成本保留插入顺序；`swap_remove` 是 `O(1)`，但
会将最后一个条目交换到被移除的槽位，从而破坏迭代顺序。

| 模式         | 大小   | AHashMap.remove | IndexMap.shift_remove | IndexMap.swap_remove |
| ---------- | ---: | --------------: | --------------------: | -------------------: |
| Remove one |    4 |         9.89 ns |               37.8 ns |              37.1 ns |
| Remove one |   32 |         62.0 ns |                117 ns |              53.4 ns |
| Remove one |  256 |         70.3 ns |                355 ns |               269 ns |

如何阅读此表：

- `AHashMap` 在纯查找上大约快 3 倍。在迭代顺序不流入可观测状态的热查找
  路径上保留 `AHashMap`。
- `IndexMap` 在 `.values().collect()` 和 `.keys().collect()` 上快 25% 到
  45%。在迭代驱动可观测状态的地方，切换到 `IndexMap` 既是确定性的胜利，
  也是一个小的性能胜利。
- `IndexMap` 在 insert、clone 和 entry-modify-or-insert 上慢 1.3 到 3.5 倍。
  在构建密集或每次成交累积的路径上保留 `AHashMap`。
- 当移除后迭代顺序无关紧要时，优先使用 `swap_remove` 而非 `shift_remove`；
  它与 `AHashMap` 移除保持竞争力。

### 线程安全哈希映射模式

`AHashMap` 不是线程安全的。将其包装在 `Arc` 中仅能跨线程共享指针，但不能协调修改。仅在映射构建后不可变时使用 `Arc<AHashMap>`，否则需添加适当的同步机制。

```rust
// 避免：多线程修改时的数据竞争
let cache = Arc::new(AHashMap::new());
let cache_clone = Arc::clone(&cache);
tokio::spawn(async move {
    cache_clone.insert(key, value);  // 数据竞争
});
cache.insert(other_key, other_value);  // 数据竞争
```

**模式：**

1. **构建后不可变** — 构建映射一次，然后以只读方式共享：

   ```rust
   let mut map = AHashMap::new();
   map.insert(key1, value1);
   map.insert(key2, value2);
   let shared_map = Arc::new(map);  // 现在不可变

   // 多线程可以安全读取
   let map_clone = Arc::clone(&shared_map);
   tokio::spawn(async move {
       if let Some(value) = map_clone.get(&key1) {
           // 安全的只读访问
       }
   });
   ```

2. **并发读写** — 使用 `DashMap`：

   ```rust
   use dashmap::DashMap;

   let cache: Arc<DashMap<K, V>> = Arc::new(DashMap::new());

   // 多线程可以安全地并发读写
   cache.insert(key, value);
   if let Some(entry) = cache.get(&key) {
       // 安全的并发访问
   }
   ```

   `DashMap` 内部使用分片（sharding）和细粒度锁实现高效的并发访问。

3. **单线程热路径** — 在单线程上下文中使用普通的 `AHashMap`：

   ```rust
   struct Handler {
       instruments: AHashMap<Ustr, InstrumentAny>,
   }

   impl Handler {
       async fn next(&mut self) -> Option<()> {
           // Handler 运行在单个任务上，无并发访问
           self.instruments.insert(key, value);
           Ok(())
       }
   }
   ```

**决策树：**

1. 迭代顺序在 DST 路径上可观测？使用 `IndexMap<K, V>` / `IndexSet<T>`
2. 否则，按访问模式：
   - 构建后不可变：使用 `Arc<AHashMap<K, V>>`
   - 需要并发访问：使用 `Arc<DashMap<K, V>>`
   - 单线程访问：使用普通的 `AHashMap<K, V>`

### 共享可变性存储

从 Cython 移植的代码经常在修改容器中的值之前将其克隆出来。
这种模式会产生静默陈旧（silent staleness）：本地克隆在另一个代码路径对
规范条目应用事件的那一刻就与之产生分歧。

仅当以下三个条件全部成立时，才使用 `Rc<RefCell<T>>`（单线程）或
`Arc<RwLock<T>>`（多线程）存储：

- 值在插入后会被修改。
- 多个持有者需要观察彼此的写入。
- 句柄必须比容器的借用作用域存活得更久。

`Cache` 中的订单在内部使用这种形态进行每键借用跟踪。存储为
`AHashMap<ClientOrderId, SharedCell<OrderAny>>`；智能指针泄漏保持
在内部。公共访问器返回隐藏它的作用域 newtype：`Cache::order`
返回 `OrderRef<'_>`（读借用），`Cache::order_mut` 返回 `OrderRefMut<'_>`
（独占写借用，需要 `&mut Cache`），而 `Cache::order_owned` 在值必须
跨越边界时返回一个拥有所有权的 `OrderAny` 快照。引擎在分发事件前
丢弃借用，并在事件后重新读取缓存以获取事件后状态，这使得分发
保持为干净的事务边界。

`Cache::order_mut` 接受 `&mut Cache`，这意味着接收 `CacheView`
（仅暴露不可变缓存借用）的策略和适配器无法触及它。订单修改
保留给直接持有缓存的数据引擎和执行引擎；类型系统强制执行该契约。

否则，优先使用更简单的形态：

- 读为主且只设置一次：`Rc<T>` 或 `Arc<T>`（无内部可变性）。
- 拥有所有权的快照对调用者足够：存储 `T`，读取时克隆。
- 单一所有者，无修改：普通字段。

采用 `Rc<RefCell<T>>` 之前值得权衡的成本：

- 每次访问都要支付运行时借用检查。
- 智能指针类型在写边界处泄漏。
- 误用会在运行时 panic 而非编译失败。
- `Rc<RefCell<T>>` 是 `!Send` 的；跨线程存储需要 `Arc<RwLock<T>>`
  （或读取罕见时使用 `Arc<Mutex<T>>`）。

**决策树：**

1. 可变、多观察者，且句柄比容器借用存活得更久？
   - 单线程：`Rc<RefCell<T>>`。
   - 多线程：`Arc<RwLock<T>>`（或读取罕见时使用 `Arc<Mutex<T>>`）。
2. 读为主且只设置一次：`Rc<T>` 或 `Arc<T>`。
3. 拥有所有权的快照即可：存储 `T`，读取时克隆。
4. 单一所有者，无修改：普通字段。

### 重新导出模式

按字母顺序组织重新导出，并将其放在 lib.rs 文件末尾：

```rust
// Re-exports
pub use crate::{
    nanos::UnixNanos,
    time::AtomicTime,
    uuid::UUID4,
};

// Module-level re-exports
pub use crate::identifiers::{
    account_id::AccountId,
    actor_id::ActorId,
    client_id::ClientId,
};
```

### 文档标准

所有文档注释使用第三人称陈述语气（例如 "Returns the account ID" 而非 "Return the account ID"）。

#### 章节标题大小写

Rustdoc 章节标题使用 Title Case，与 Rust 标准库约定保持一致：

- `# Examples`
- `# Errors`
- `# Panics`
- `# Safety`
- `# Notes`
- `# Thread Safety`
- `# Feature Flags`

#### 模块级文档

所有模块必须有以简短描述开头的模块级文档：

```rust
//! Functions for correctness checks similar to the *design by contract* philosophy.
//!
//! This module provides validation checking of function or method conditions.
//!
//! A condition is a predicate which must be true just prior to the execution of
//! some section of code - for correct behavior as per the design specification.
```

对于带有特性标志的模块，清晰地记录它们：

```rust
//! # Feature flags
//!
//! This crate provides feature flags to control source code inclusion during compilation,
//! depending on the intended use case:
//!
//! - `ffi`: Enables the C foreign function interface (FFI) from [cbindgen](https://github.com/mozilla/cbindgen).
//! - `python`: Enables Python bindings from [PyO3](https://pyo3.rs).
//! - `extension-module`: Builds as a Python extension module (used with `python`).
//! - `stubs`: Enables type stubs for use in testing scenarios.
```

#### 字段文档

所有结构体和枚举字段必须有以句号结尾的文档：

```rust
pub struct Currency {
    /// The currency code as an alpha-3 string (e.g., "USD", "EUR").
    pub code: Ustr,
    /// The currency decimal precision.
    pub precision: u8,
    /// The ISO 4217 currency code.
    pub iso4217: u16,
    /// The full name of the currency.
    pub name: Ustr,
    /// The currency type, indicating its category (e.g. Fiat, Crypto).
    pub currency_type: CurrencyType,
}
```

#### 函数文档

为所有公共函数记录：

- 目的和行为
- 输入参数用法说明
- 错误条件（如适用）
- Panic 条件（如适用）

```rust
/// Returns a reference to the `AccountBalance` for the specified currency, or `None` if absent.
///
/// # Panics
///
/// Panics if `currency` is `None` and `self.base_currency` is `None`.
pub fn base_balance(&self, currency: Option<Currency>) -> Option<&AccountBalance> {
    // 实现
}
```

#### 错误和 Panic 文档格式

对于单行的错误和 Panic 文档，使用句首大写并遵循以下约定：

```rust
/// Returns a reference to the `AccountBalance` for the specified currency, or `None` if absent.
///
/// # Errors
///
/// Returns an error if the currency conversion fails.
///
/// # Panics
///
/// Panics if `currency` is `None` and `self.base_currency` is `None`.
pub fn base_balance(&self, currency: Option<Currency>) -> anyhow::Result<Option<&AccountBalance>> {
    // 实现
}
```

对于多行的错误和 Panic 文档，使用句首大写、列表项和句号结尾：

```rust
/// Calculates the unrealized profit and loss for the position.
///
/// # Errors
///
/// Returns an error if:
/// - The market price for the instrument cannot be found.
/// - The conversion rate calculation fails.
/// - Invalid position state is encountered.
///
/// # Panics
///
/// This function panics if:
/// - The instrument ID is invalid or uninitialized.
/// - Required market data is missing from the cache.
/// - Internal state consistency checks fail.
pub fn calculate_unrealized_pnl(&self, market_price: Price) -> anyhow::Result<Money> {
    // 实现
}
```

#### Safety 文档格式

对于 Safety 文档，使用 `SAFETY:` 前缀后跟简短描述，说明为什么 unsafe 操作是有效的：

```rust
/// Creates a new instance from raw components without validation.
///
/// # Safety
///
/// The caller must ensure that all input parameters are valid and properly initialized.
pub unsafe fn from_raw_parts(ptr: *const u8, len: usize) -> Self {
    // SAFETY: Caller guarantees ptr is valid and len is correct
    Self {
        data: std::slice::from_raw_parts(ptr, len),
    }
}
```

对于内联 unsafe 块，在 unsafe 代码正上方使用 `SAFETY:` 注释：

```rust
impl Send for MessageBus {
    fn send(&self) {
        // SAFETY: Message bus is not meant to be passed between threads
        unsafe {
            // unsafe operation here
        }
    }
}
```

## Python 绑定

Python 绑定通过 [PyO3](https://pyo3.rs) 提供，允许用户直接在 Python 中导入 NautilusTrader crate，无需 Rust 工具链。

### PyO3 命名约定

**通过 PyO3** 将 Rust 函数暴露给 Python 时：

1. Rust 符号**必须**以 `py_*` 为前缀，以在 Rust 代码库中明确其用途。
2. 使用 `#[pyo3(name = "…")]` 属性发布**不带** `py_` 前缀的 *Python* 名称，以保持 Python API 的整洁。

```rust
#[pyo3(name = "do_something")]
pub fn py_do_something() -> PyResult<()> {
    // …
}
```

:::info[自动化执行]
`check_pyo3_conventions.sh` pre-commit 钩子会强制 PyO3 函数使用 `py_` 前缀。
:::

### PyO3 枚举约定

暴露给 Python 的枚举应使用以下 `pyclass` 属性：

- `frozen`：枚举是不可变的值类型。
- `eq, eq_int`：启用与其他枚举实例和整数判别值的相等性比较。
- `rename_all = "SCREAMING_SNAKE_CASE"`：标准化 Python 变体名称。
- `from_py_object`：启用从 Python 对象的转换。

:::warning[不要对 `eq_int` 枚举使用 `hash` pyclass 属性]
PyO3 自动生成的 `__hash__` 使用 Rust 的 `DefaultHasher`，它产生的值与
Python 的 `hash()` 对等效整数的结果不同。由于 `eq_int` 使得 `MyEnum.VARIANT == 1`
为真，哈希契约（`a == b` 蕴含 `hash(a) == hash(b)`）将被违反。请改为
提供一个直接返回判别值的手动 `__hash__`：
:::

```rust
#[pymethods]
impl MyEnum {
    const fn __hash__(&self) -> isize {
        *self as isize
    }
}
```

### 测试约定

- 使用 `mod tests` 作为标准测试模块名称，除非需要专门分区。
- 一致地使用 `#[rstest]` 属性，这种标准化减少了认知负担。
- 在 Rust 测试中**不要**使用 Arrange、Act、Assert 分隔注释。

:::info[自动化执行]
`check_testing_conventions.sh` pre-commit 钩子会强制使用 `#[rstest]` 而非 `#[test]`。
:::

#### 参数化测试

一致地使用 `rstest` 属性，对于参数化测试：

```rust
#[rstest]
#[case("AUDUSD", false)]
#[case("AUD/USD", false)]
#[case("CL.FUT", true)]
fn test_symbol_is_composite(#[case] input: &str, #[case] expected: bool) {
    let symbol = Symbol::new(input);
    assert_eq!(symbol.is_composite(), expected);
}
```

#### 测试规格（bon builders）

对于具有许多构造参数的事件，规范的测试构建器是一个
流式规格（fluent spec），它与事件一起定义在 `events/<event>/spec/<name>.rs` 下
（参考实现见 `crates/model/src/events/order/spec/filled.rs`）。用
`#[cfg(any(test, feature = "stubs"))]` 门控该 spec 模块，使其对 crate 内测试
以及通过 `stubs` 特性选择启用的下游 crate 可用，但在生产构建中编译排除。
spec 不得被生产代码引用。

为什么使用自定义 spec 而非带 `builder(default)` 的 `derive_builder::Builder`：
后者会绕过生产构造函数，因此后续添加的不变量不会被测试覆盖。spec 在每次
`build()` 时都会流经生产构造函数。

剖析：

- 派生 `bon::Builder` 时使用 `finish_fn = into_spec`，使生成的 finish
  方法不与自定义 `build()` 冲突。
- 用字面量或 `TestDefault::test_default()` 调用将每个必需字段标记为
  `#[builder(default = ...)]`。将可选字段保留为不带默认值的 `Option<T>`，
  以便调用者要么设置它们，要么接受 `None`。
- 将事件 ID 字段默认设置为 `crate::stubs` 中的 `test_uuid()`。这会产生
  不同的、可复现的 UUID，而无需调用者管理状态。
- 在生成的构建器上实现 `build()`，使其调用 `into_spec()` 并转发到
  生产构造函数（例如 `OrderFilled::new`）。返回类型是事件本身，而非
  `Result`，因为 spec 默认值在构造上是有效的。

调用者用法：

```rust
let fill = OrderFilledSpec::builder()
    .last_qty(Quantity::from(50_000))
    .trade_id(TradeId::from("TRADE-1"))
    .build();
```

仅覆盖测试关心的字段；其余采用 spec 默认值。
不要在 `build()` 后写 `.unwrap()`。

确定性：在 `cargo nextest` 下，每个测试运行在一个全新的进程中，因此
每线程的 UUID 序列会自动重置。在普通的 `cargo test` 下，在任何比较
跨抽取的 UUID 序列的测试开始处调用 `crate::stubs` 中的
`reset_test_uuid_rng()`。

用 spec 模块中的单个测试固定 spec 默认值，使任何字段的意外漂移
在那里浮现，而非在下游测试中表现为静默的行为变化。

#### 基于属性的测试

对基于属性的测试使用 `proptest` crate。将它们放在一个单独的
`property_tests` 模块中（而非 `mod tests` 内部），以使确定性单元
测试与随机化属性测试分离：

```rust
#[cfg(test)]
mod property_tests {
    use proptest::prelude::*;
    use rstest::rstest;

    use super::*;

    // Define strategies for generating test inputs
    fn my_strategy() -> impl Strategy<Value = MyType> {
        prop_oneof![
            Just(MyType::VariantA),
            Just(MyType::VariantB),
        ]
    }

    fn value_strategy() -> impl Strategy<Value = f64> {
        prop_oneof![
            -1000.0..1000.0,
            Just(0.0),
        ]
    }

    // Group all property tests inside the proptest! macro
    proptest! {
        #[rstest]
        fn prop_construction_roundtrip(
            value in value_strategy(),
            variant in my_strategy()
        ) {
            // Test invariants that should hold for all generated inputs
        }
    }
}
```

约定：

- 将模块命名为 `property_tests`，与 `mod tests` 分离。
- 导入 `proptest::prelude::*` 和 `rstest::rstest`。
- 定义返回 `impl Strategy<Value = T>` 的策略函数。
- 使用 `prop_oneof!` 将值范围与边界情况组合。
- 使用 `prop_filter_map` 过滤无效组合。
- 测试名称以 `prop_` 为前缀。
- 用 `#[rstest]` 标记 `proptest!` 内的每个测试。

#### 测试命名

使用描述场景的测试名称：

```rust
fn test_sma_with_no_inputs()
fn test_sma_with_single_input()
fn test_symbol_is_composite()
```

### 框式横幅注释

不要使用框式横幅或分隔注释。如果代码需要视觉分隔，
可考虑将其拆分为单独的模块或文件。请改用：

- 传达用途的清晰函数名称。
- 用于逻辑分组的模块结构（`mod tests { mod fixtures { } }`）。
- 用于分组相关方法的 impl 块。
- 用于语义文档的文档注释（`///`）。
- IDE 导航和代码折叠。

应避免的模式：

```rust
// ============================================================================
// Some Section
// ============================================================================

// ========== Test Fixtures ==========
```

## Rust-Python 内存管理

使用 PyO3 绑定时，理解和避免 Rust 的 `Arc` 引用计数与 Python 垃圾回收器之间的引用循环（reference cycle）至关重要。
本节记录了在 Rust 回调持有结构体中处理 Python 对象的最佳实践。

### 引用循环问题

**问题**：在回调持有结构体中使用 `Arc<PyObject>` 会创建循环引用：

1. **Rust `Arc` 持有 Python 对象** -> 增加 Python 引用计数。
2. **Python 对象可能引用 Rust 对象** -> 创建循环。
3. **双方都无法被垃圾回收** -> 内存泄漏。

**有问题的模式示例**：

```rust
// AVOID: This creates reference cycles
struct CallbackHolder {
    handler: Option<Arc<PyObject>>,  // ✗ Arc wrapper causes cycles
}
```

### 解决方案：基于 GIL 的克隆

**解决方案**：使用普通的 `PyObject` 并通过 `clone_py_object()` 进行适当的基于 GIL 的克隆：

```rust
use nautilus_core::python::clone_py_object;

// CORRECT: Use plain PyObject without Arc wrapper
struct CallbackHolder {
    handler: Option<PyObject>,  // ✓ No Arc wrapper
}

// Manual Clone implementation using clone_py_object
impl Clone for CallbackHolder {
    fn clone(&self) -> Self {
        Self {
            handler: self.handler.as_ref().map(clone_py_object),
        }
    }
}
```

### 最佳实践

#### 1. 使用 `clone_py_object()` 克隆 Python 对象

```rust
// When cloning Python callbacks
let cloned_callback = clone_py_object(&original_callback);

// In manual Clone implementations
self.py_handler.as_ref().map(clone_py_object)
```

#### 2. 从回调持有结构体中移除 `#[derive(Clone)]`

```rust
// BEFORE: Automatic derive causes issues with PyObject
#[derive(Clone)]  // ✗ Remove this
struct Config {
    handler: Option<PyObject>,
}

// AFTER: Manual implementation with proper cloning
struct Config {
    handler: Option<PyObject>,
}

impl Clone for Config {
    fn clone(&self) -> Self {
        Self {
            // Clone regular fields normally
            url: self.url.clone(),
            // Use clone_py_object for Python objects
            handler: self.handler.as_ref().map(clone_py_object),
        }
    }
}
```

#### 3. 更新函数签名以接受 `PyObject`

```rust
// BEFORE: Arc wrapper in function signatures
fn spawn_task(handler: Arc<PyObject>) { ... }  // ✗

// AFTER: Plain PyObject
fn spawn_task(handler: PyObject) { ... }  // ✓
```

#### 4. 创建 Python 回调时避免 `Arc::new()`

```rust
// BEFORE: Wrapping in Arc
let callback = Arc::new(py_function);  // ✗

// AFTER: Use directly
let callback = py_function;  // ✓
```

### 为什么这有效

`clone_py_object()` 函数：

- **在执行克隆操作前获取 Python GIL**。
- **通过 `clone_ref()` 使用 Python 原生引用计数**。
- **避免干扰 Python GC 的 Rust Arc 包装**。
- **通过适当的 GIL 管理维护线程安全**。

这种方法允许 Rust 和 Python 垃圾回收器正确工作，消除引用循环导致的内存泄漏。

## 契约式设计

契约式设计（design by contract）规定了函数与其调用者之间的义务：

- **前置条件（Preconditions）**：函数要求调用者满足什么。
- **后置条件（Postconditions）**：函数作为回报保证什么。
- **不变量（Invariants）**：其类型在多次调用之间维护哪些属性。

优先依赖类型系统。所有权、生命周期、`Send`/`Sync`、`Result`/`Option`、
穷尽匹配、newtype 和可见性在编译期编码了大多数契约，且在运行时零成本。
仅在类型系统无法做到的地方使用运行时检查。

对于大多数前置条件，使用 `nautilus_core::correctness` 模块：它是
项目的契约式设计机制，应作为默认选择。`check_*`
函数（`check_predicate_true`、`check_valid_string_ascii`、
`check_positive_u64`、`check_in_range_inclusive_f64`、`check_equal_usize`、
`check_key_in_map`，……）返回一个带类型的 `CorrectnessResult<()>`，其
`CorrectnessError` 变体命名了每种违规类型。将 `new_checked()`（可能失败，返回
`CorrectnessResult`）与通过 `.expect_display(FAILED)` panic 的 `new()`
包装器配对用于已验证类型；这是 [构造函数模式](#构造函数模式)
约定，并产生以 `Condition failed: ...` 为前缀的 panic 消息。

对于正确性模块未建模的*内部*不变量，使用 `debug_assert!`（以及
`debug_assert_eq!`/`_ne!`）：字段关系、单调序列、CAS 后置条件、
编码/解码往返、可证明在范围内的索引，以及对信任上游验证的内部
辅助函数的前置条件。release 构建会剥离该检查，因此切勿对公共 API
输入使用 `debug_assert!`。对于 `unsafe` 代码，对正确性关键（soundness-critical）
的前置条件（null、对齐、出处）使用始终开启的 `assert!`，并将
`debug_assert!` 保留给由设计维护的热路径前置条件。

选择机制：

| 情况                                 | 使用                                        |
| ---------------------------------- | ----------------------------------------- |
| 针对命名前置条件的公共 API 输入                 | `nautilus_core::correctness` 中的 `check_*` |
| 已验证的构造函数（可能失败 + panic 配对）          | `new_checked()` / `new()`                 |
| 可恢复的非验证错误（I/O、解析、网络）               | `Result<T, DomainError>`                  |
| 编译器无法证明的内部不变量                      | `debug_assert!`                           |
| 无匹配 `CorrectnessError` 的始终开启的内部不变量 | `assert!`                                 |
| 正确性关键的 `unsafe` 前置条件               | `assert!`（始终开启）                           |
| 由设计维护的热路径 `unsafe` 前置条件            | `debug_assert!` 加上记录在案的 `Safety` 子句       |

风格：

- 将 `debug_assert!` 消息以 `Invariant:` 为前缀，并陈述正面规则，
  而非失败：`debug_assert!(next > last, "Invariant: time is strictly monotonic across CAS")`。
- `Condition failed: ...`（来自 `FAILED` 常量）标记调用者提供的
  输入违规；`Invariant: ...` 标记内部契约 bug。
- 将断言放在不变量首次被假设的地方。当不变量跨越热循环成立时，
  在边界处断言一次，而非在循环内部。

## 常见反模式

1. **避免在热路径中使用 `.clone()`** — 优先借用或通过 `Arc` 共享所有权。
2. **避免在生产代码中使用 `.unwrap()`** — 通常使用 `?` 传播错误或将其映射为领域错误，但对锁中毒（lock poisoning）使用 unwrap 是可接受的，因为它表示应快速中止的严重程序状态。
3. **当 `&str` 足够时避免使用 `String`** — 在紧密循环中最小化分配。
4. **避免暴露内部可变性（interior mutability）** — 将 mutex/`RefCell` 隐藏在安全 API 之后。
5. **避免在 `Result<T, E>` 中使用大型结构体** — 对大型错误载荷使用 Box（`Box<dyn Error + Send + Sync>`）。

## Unsafe Rust

为了实现 Cython 和 Rust 之间的互操作，编写 `unsafe` Rust 代码是必要的。能够跨越安全 Rust 的边界正是使得实现 Rust 语言本身许多最基本特性成为可能的原因，正如 C 和 C++ 被用来实现它们自己的标准库一样。

我们会非常谨慎地使用 Rust 的 `unsafe` 功能——它启用了一小组额外的语言特性，从而改变了接口与调用者之间的契约，将保证正确性的部分责任从 Rust 编译器转移到了我们身上。目标是实现 `unsafe` 功能的优势，同时避免*任何*未定义行为（undefined behavior）。
Rust 语言设计者对未定义行为的定义可以在[语言参考](https://doc.rust-lang.org/stable/reference/behavior-considered-undefined.html)中找到。

### 安全策略

为保持正确性，任何 `unsafe` Rust 的使用都必须遵循我们的策略：

- 如果一个函数调用是 `unsafe` 的，其文档中*必须*有一个 `Safety` 部分，解释该函数为什么是 `unsafe` 的，涵盖函数期望调用者维护的不变量，以及如何履行该契约中的义务。
- 在文档注释的 Safety 部分记录每个函数为什么是 `unsafe` 的，并为所有 `unsafe` 块编写单元测试。
- 始终包含 `SAFETY:` 注释，解释为什么 unsafe 操作是有效的。
- **Crate 级别的 lint** — 每个暴露 FFI 符号的 crate 都启用 `#![deny(unsafe_op_in_unsafe_fn)]`。即使在 `unsafe fn` 内部，每个指针解引用或其他危险操作也必须包装在自己的 `unsafe { … }` 块中。
- **CVec 契约** — 对于跨 FFI 边界的原始向量，请阅读 [FFI 内存契约](ffi.md)。外部代码成为分配的所有者，并且**必须**恰好调用一次匹配的 `vec_drop_*` 函数。

### Unsafe 代码类别

代码库在以下类别中使用 unsafe Rust：

1. **FFI 边界** — 用于 C 互操作的原始指针操作。参见 [FFI 文档](ffi.md)。
2. **内部可变性** — 使用 `UnsafeCell` 实现具有受控访问模式的线程本地注册表。
3. **Unsafe Send/Sync** — 类型本身不是线程安全的，但通过运行时不变量满足 trait 约束（例如架构保证的单线程访问）。

### Unsafe Send/Sync 要求

不安全地实现 `Send` 或 `Sync` 时：

1. 精确记录哪些字段违反了 trait 要求。
2. 解释确保安全的运行时机制（例如单线程事件循环）。
3. 包含一个 `WARNING` 声明，说明违反不变量是未定义行为。
4. 优先使用运行时强制执行（断言、`Result` 返回）而非仅依赖文档保证。

```rust
// SAFETY: Contains Rc<RefCell<...>> which is not thread-safe.
// Single-threaded access guaranteed by the backtest engine architecture.
// WARNING: Actually sending across threads is undefined behavior.
#[allow(unsafe_code)]
unsafe impl Send for BacktestDataClient {}
```

### 纵深防御

当 unsafe 代码依赖于不变量时，添加防御机制：

- **类型验证**：在转换前进行运行时类型检查（例如 `TypeId` 比较）。
- **调试断言**：在调试构建中及早捕获内存损坏。
- **RAII 守卫**：确保在正常返回和 panic 路径上都进行清理。
- **运行时检查**：当不变量被违反时快速失败，而非继续不安全地执行。

### 运行时不变量

若干核心子系统依赖运行时不变量而非编译期保证。测试验证下面前三个
契约。守卫使用规则按约定强制执行。任何触及 `UnsafeCell`、
注册表、`unsendable` 或 live-node 线程的 PR 都应确认不变量测试
仍然通过。

#### 线程本地注册表

actor 注册表、组件注册表和消息总线各自使用
`thread_local!` 存储。在一个线程上注册的对象永远不会从另一个线程
可见。live node 事件循环运行在单个线程上，所有注册表和消息总线
访问都发生在该线程上。

`LiveNodeHandle` 是唯一预期的跨线程控制面。它使用
`Arc<AtomicBool>` 进行停止信号传递，使用 `Arc<AtomicU8>` 表示状态，
两者都使用 `Ordering::Relaxed`。

#### Actor 注册表 vs 组件注册表

两个注册表都在线程本地映射中存储 `Rc<UnsafeCell<dyn Trait>>`，但
在处理别名访问的方式上有所不同：

| 属性   | Actor 注册表              | 组件注册表                  |
| ---- | ---------------------- | ---------------------- |
| 别名   | 允许（多个守卫）               | 禁止（`BorrowGuard` + 集合） |
| 重入访问 | 是，回调所需                 | 否，生命周期操作是顺序的           |
| 错误处理 | 查找失败时 panic 或返回 `None` | 出错时返回 `anyhow::Result` |
| 守卫类型 | `ActorRef<T>`（Rc 支撑）   | 栈本地 `BorrowGuard`      |

actor 注册表选择重入访问而非别名防止，因为
消息处理器经常回调进注册表以查找其他
actor。组件注册表可以强制严格别名，因为生命周期
操作（start、stop、reset、dispose）是非重入的。

#### `ActorRef` 使用规则

`ActorRef` 守卫必须：

- 在单个同步作用域内获取和丢弃。
- 永不存储在结构体字段中。
- 永不跨越 `.await` 点持有。
- 永不发送到另一个线程。

规范模式在闭包中捕获 actor 的 `Ustr` ID，并在每次回调触发时
查找该 actor：

```rust
let actor_id = actor.actor_id().inner();
let handler = TypedHandler::from(move |quote: &QuoteTick| {
    if let Some(mut actor) = try_get_actor_unchecked::<MyActor>(&actor_id) {
        actor.handle_quote(quote);
    }
});
```

## 工具配置

项目使用多种工具保障代码质量：

- **rustfmt**：自动代码格式化（参见 `rustfmt.toml`）。
- **clippy**：代码检查和最佳实践（参见 `clippy.toml`）。
  在抑制 `missing_panics_doc` 或 `missing_errors_doc` 时，包含一个 `reason`
  解释为什么该 lint 不适用：

  ```rust
  #[allow(clippy::missing_panics_doc, reason = "mutex poisoning is not expected")]
  ```

- **cbindgen**：FFI 的 C 头文件生成。

## Rust 版本管理

项目通过 `rust-toolchain.toml` 固定到特定 Rust 版本。

**保持工具链与 CI 同步：**

```bash
rustup update       # 更新到最新稳定版 Rust
rustup show         # 验证正确的工具链处于活动状态
```

如果 pre-commit 在本地通过但在 CI 中失败，清除 prek 缓存并重新运行：

```bash
prek clean    # 清除缓存环境
make pre-commit     # 重新运行所有检查
```

这确保您使用与 CI 相同的 Rust 和 clippy 版本。

## 资源

- [The Rustonomicon](https://doc.rust-lang.org/nomicon/) — Unsafe Rust 的暗黑艺术。
- [The Rust Reference - Unsafety](https://doc.rust-lang.org/stable/reference/unsafety.html)。
- [Safe Bindings in Rust - Russell Johnston](https://www.abubalay.com/blog/2020/08/22/safe-bindings-in-rust)。
- [Google - Rust and C interoperability](https://www.chromium.org/Home/chromium-security/memory-safety/rust-and-c-interoperability/)。

## Cap'n Proto 序列化

`nautilus-serialization` crate 提供了可选的 Cap'n Proto 序列化支持，用于高效的数据交换。
此特性是可选启用的，以避免标准构建需要 Cap'n Proto 编译器。

### 安装 Cap'n Proto

在使用 schema 之前安装 Cap'n Proto 编译器。所需版本在仓库根目录的 `tools.toml` 文件中指定。

有关各平台的详细安装说明，请参阅[环境设置](environment_setup.md#capn-proto)指南。

:::warning
Ubuntu 默认的 `capnproto` 包版本过旧。Linux 用户必须从源代码安装。
:::

验证安装：

```bash
capnp --version  # 应与 tools.toml 中的版本匹配
```

### Schema 开发工作流

Schema 文件位于 `crates/serialization/schemas/capnp/`：

- `common/` - 基础类型、标识符、枚举。
- `commands/` - 交易命令。
- `events/` - 订单和持仓事件。
- `data/` - 市场数据类型。

修改 schema 时：

1. 在相应子目录中编辑 `.capnp` schema 文件。
2. 重新生成 Rust 绑定：

   ```bash
   make regen-capnp
   # 或
   ./scripts/regen-capnp.sh
   ```

3. 审查更改：

   ```bash
   git diff crates/serialization/generated/capnp
   ```

4. 如需要，更新 `crates/serialization/src/capnp/conversions.rs` 中的转换。
5. 运行测试：

   ```bash
   make cargo-test EXTRA_FEATURES="capnp"
   ```

### 生成的代码

生成的 Rust 文件被检入 `crates/serialization/generated/capnp/`，原因如下：

- **docs.rs 兼容性**：文档构建环境缺少 Cap'n Proto 编译器。
- **贡献者便利性**：大多数开发者在标准开发中不需要安装 capnp。
- **构建可重现性**：确保跨环境一致的代码生成。

当启用 `capnp` 特性时，生成的文件会在构建期间通过 `build.rs` 自动创建，但我们将它们提交到仓库以支持在未安装编译器的情况下进行构建。

### 验证 Schema 一致性

在提交 schema 更改之前，确保生成的文件是最新的：

```bash
make check-capnp-schemas
```

此目标会：

1. 如果未安装 `capnp` 则带警告跳过（对本地开发可接受）。
2. 如果重新生成出错（例如版本不匹配）则失败。
3. 重新生成 schema，并在生成的文件与已提交版本不同时失败。

CI 会自动运行此检查以捕获漂移（capnp 在 CI 中始终安装）。

### 使用 capnp 特性进行测试

```bash
# 使用 capnp 运行工作区测试
make cargo-test EXTRA_FEATURES="capnp"

# 使用 capnp 运行特定 crate 测试
make cargo-test-crate-nautilus-serialization FEATURES="capnp"

# 运行特定测试
cargo test -p nautilus-serialization --features capnp test_price_roundtrip
```

### Schema 演进指南

演进 schema 时：

- **仅进行增量更改**：在末尾添加新字段。
- **永远不要移除字段**：在注释中标记已弃用的字段。
- **永远不要复用字段编号**：即使在弃用之后。
- **测试往返兼容性**：确保新旧版本可以互操作。

Cap'n Proto 的演进规则允许 schema 更改而不破坏二进制兼容性，但您必须遵循这些约束以维护前向/后向兼容性。
