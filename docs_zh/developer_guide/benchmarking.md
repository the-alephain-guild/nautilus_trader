# 基准测试 (Benchmarking)

本文档是编写和运行 NautilusTrader 基准测试的实践参考。它涵盖工具细节、目录布局、
示例代码、本地执行以及火焰图性能分析。

关于策略层面的内容（我们对什么进行基准测试、何时进行、以何种严谨程度进行、如何与 CI 关联），
请参阅仓库根目录下的 [`/BENCHMARKING.md`](../../BENCHMARKING.md)。

---

## 工具概览

NautilusTrader 使用两个互补的 Rust 基准测试框架：

| 框架                                                           | 测量内容                           | 何时优先选择                     |
| ------------------------------------------------------------ | ------------------------------ | -------------------------- |
| [**Criterion**](https://docs.rs/criterion/latest/criterion/) | 带置信区间的实际运行时间（wall-clock time）  | 任何 ≥ 100 ns 的代码；绝对测量；对比比较。 |
| [**iai**](https://docs.rs/iai/latest/iai/)                   | 已退役的 CPU 指令数（通过 Cachegrind 统计） | 低于 100 ns 的函数；CI 回归检测。     |

大多数热路径代码都能从两者中获益。Criterion 给出用户可见的数字；iai 给出无噪声的回归信号。

:::note
iai 是确定性的（不受系统噪声影响），但结果是机器特定的。请将其用于 CI 内的回归检测，而非跨机器比较。
:::

---

## 目录布局

每个 crate 将其基准测试放在本地的 `benches/` 文件夹中：

```text
crates/<crate_name>/
└── benches/
    ├── foo_criterion.rs
    └── foo_iai.rs
```

在 crate 的 `Cargo.toml` 中显式注册每个基准测试，这样 `cargo bench` 才能发现它们：

```toml
[[bench]]
name = "foo_criterion"
path = "benches/foo_criterion.rs"
harness = false

[[bench]]
name = "foo_iai"
path = "benches/foo_iai.rs"
harness = false
```

若要将该 crate 纳入夜间 CI 性能工作流，请把它添加到工作区 `Makefile` 中的
`cargo-ci-benches` 配方（recipe）。

---

## 编写 Criterion 基准测试

1. **在计时循环之外完成初始化。** 所有在迭代之间不会变化的工作都应放在外围代码中，
   或放在 `iter_batched_ref` 的 setup 闭包里，而不是放进传给 `iter` 的函数体内。
2. **用 `black_box` 包装输入**，以免优化器把它们折叠消除掉。
3. **对会发生变更（mutating）的基准测试使用 `iter_batched_ref`。** 它会把输入的 `Drop`
   排除在计时区域之外——否则在持有大型结构体的基准测试中，`Drop` 会主导整个测量结果。
4. **为按规模参数化的分组添加 `Throughput::Elements(n)`**，让 Criterion 报告每个元素的吞吐量。
5. **注释意图。** 说明该基准测试在测量什么（热路径、最坏情况、缓存冷启动情况），
   让未来的读者理解一旦它发生回归意味着什么。

```rust
use std::hint::black_box;

use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};

const SIZES: &[usize] = &[10, 100, 1_000];

fn bench_my_op(c: &mut Criterion) {
    let mut group = c.benchmark_group("module/my_op");

    for &n in SIZES {
        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
            b.iter_batched_ref(
                || populate(n),
                |state| state.run(black_box(n)),
                BatchSize::SmallInput,
            );
        });
    }

    group.finish();
}

criterion_group!(benches, bench_my_op);
criterion_main!(benches);
```

---

## 编写 iai 基准测试

`iai` 要求函数不带参数。请保持函数足够小，使指令计数有意义，
同时让函数之外的改动不会渗入测量结果。

```rust
use std::hint::black_box;

fn bench_add() -> i64 {
    let a = black_box(123);
    let b = black_box(456);
    a + b
}

iai::main!(bench_add);
```

在不同运行之间会变化的初始化（内存分配、随机数、系统调用）会以误导性的方式
抬高指令计数。iai 最适合用于纯粹、无内存分配的函数。

---

## 本地运行基准测试

| 目标                | 命令                                                                   |
| ----------------- | -------------------------------------------------------------------- |
| 某个 crate 中的全部基准测试 | `cargo bench -p nautilus-execution`                                  |
| 单个基准测试模块          | `cargo bench -p nautilus-execution --bench matching_core`            |
| 按名称模式运行某个特定基准测试   | `cargo bench -p nautilus-execution --bench matching_core -- iterate` |
| 快速冒烟运行（低采样数）      | `cargo bench ... -- --quick`                                         |
| 全部 CI 跟踪的基准测试     | `make cargo-ci-benches`                                              |

Criterion 将 HTML 报告写入 `target/criterion/`。打开
`target/criterion/report/index.html` 即可查看。该报告包含每个基准测试的小提琴图
（violin plot）、置信区间，以及与上一次运行保存的基线（baseline）的对比。

---

## 生成火焰图

`cargo-flamegraph` 为单个基准测试生成采样的调用栈分析。当某个基准测试出现回归，
但难以判断是哪个内部调用导致时，它非常有用。

1. 每台机器安装一次：

   ```bash
   cargo install flamegraph
   ```

2. 使用 `bench` profile 运行特定基准测试：

   ```bash
   cargo flamegraph --bench matching -p nautilus-common --profile bench
   ```

3. 在浏览器中打开 `flamegraph.svg` 并放大查看热路径。

### Linux

`perf` 必须可用。在 Debian/Ubuntu 上：

```bash
sudo apt install linux-tools-common linux-tools-$(uname -r)
```

如果 `perf_event_paranoid` 阻止了运行：

```bash
sudo sh -c 'echo 1 > /proc/sys/kernel/perf_event_paranoid'
```

值为 `1` 通常就足够了。之后请把它改回 `2`（默认值），或通过 `/etc/sysctl.conf` 使其持久化。

### macOS

`DTrace` 需要 root 权限，因此 `cargo flamegraph` 必须使用 `sudo` 运行。

:::warning
使用 `sudo` 运行会在 `target/` 中创建 root 所有的文件，导致后续 `cargo` 命令出现权限错误。
你可能需要手动删除 root 所有的文件，或运行 `sudo cargo clean`。
:::

```bash
sudo cargo flamegraph --bench matching -p nautilus-common --profile bench
```

`bench` profile 保留了完整的调试符号，因此火焰图能以可读的函数名渲染，
而不会让生产二进制文件膨胀（生产构建仍然使用 `panic = "abort"` 并通过 `[profile.release]` 构建）。

> **注意** 基准测试二进制文件使用工作区 `Cargo.toml` 中定义的自定义 `[profile.bench]` 编译。
> 该 profile 继承自 `release` 并设置 `debug = "full"`，在保留完全优化的*同时*保留调试符号，
> 使 `cargo flamegraph` 或 `perf` 等工具能够生成人类可读的调用栈信息。

---

## 模板

可直接复制的起始文件位于 [`docs/dev_templates/`](../dev_templates/)：

- **Criterion**：[`criterion_template.rs`](../dev_templates/criterion_template.rs)
- **iai**：[`iai_template.rs`](../dev_templates/iai_template.rs)

将模板复制到目标 crate 的 `benches/`，调整导入和分组名称，在 `Cargo.toml` 中注册，即可开始测量。
