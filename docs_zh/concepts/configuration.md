# 配置 (Configuration)

NautilusTrader 在整个平台中使用带类型的配置结构体 (config struct)。
每个组件（数据客户端、执行客户端、引擎、策略）都有一个专属的配置结构体，用于控制其行为。

## 设计原则

### 默认值在配置边界处解析

配置结构体为那些总是存在合理默认值的字段携带具体的值。
超时、重试次数、退避延迟和心跳间隔都是诸如 `u64` 或 `u32` 这样的普通类型，默认值已经内置其中。
下游代码接收到的是已解析好的值，不会重复执行设置默认值的逻辑。

### Option 表示语义上的缺失，而非"使用默认值"

`Option<T>` 字段只在 `None` 承载真实含义时才会出现：某项功能被关闭、某个回看窗口 (lookback window) 无界，或者某个值在运行时从环境中继承而来。
如果一个字段总是解析为一个具体的值，它就不会被包裹在 `Option` 中。

这一区别使得配置的语义在类型层面就一目了然。一个普通的 `u64` 字段总是有值。
而一个 `Option<u64>` 字段则可能缺失，消费它的代码会针对这种情况进行分支处理。

### 默认值的唯一真理源

每个配置结构体都通过 `bon::Builder`，借助 `#[builder(default = value)]` 注解在一处定义默认值。
`Default` 实现委托给 builder（`Self::builder().build()`），因此不存在第二份可能与之失同步的默认值副本。

### 配置解码遇到未知字段时失败

配置解码在遇到未知字段时会快速失败。Nautilus 将多余的键视为 bug，而非无害的输入。
这能在节点或客户端以错误的设置启动之前，捕获拼写错误、配置改名后遗留的旧名称以及复制粘贴的失误。

## Python 配置

Python 配置类（msgspec 结构体）接受 `None` 作为可选参数的值。
对于普通的 `T` 字段，`None` 表示"使用默认值"。对于 `Option<T>` 字段，
`None` 则保留该字段可选的含义（禁用、无界等）。

所有 Python 配置类都继承自 `NautilusConfig`，它在底层的 `msgspec.Struct` 上设置了
`forbid_unknown_fields=True`。如今未知的键会在解码期间抛出
`msgspec.ValidationError`。

```python
from nautilus_trader.adapters.bybit.config import BybitDataClientConfig

# 全部使用默认值：60 秒超时、3 次重试等
config = BybitDataClientConfig()

# 仅覆盖超时设置
config = BybitDataClientConfig(http_timeout_secs=30)

# 禁用 instrument 状态轮询
config = BybitDataClientConfig(instrument_status_poll_secs=None)
```

## Rust 配置

所有配置结构体都派生 [`bon::Builder`](https://bon-rs.com)，它会生成
一个类型安全的 builder，对必填字段进行编译期检查。带有
`#[builder(default = value)]` 的字段可以在 builder 调用中省略，此时它们将
使用所声明的默认值。构造配置有三种等价的方式：

使用 Serde 反序列化的 Rust 配置结构体还设置了
`#[serde(deny_unknown_fields)]`。如今未知的键会导致反序列化失败，而不再被
忽略。

```rust
// Builder：只设置与默认值不同的部分
let config = BybitDataClientConfig::builder()
    .http_timeout_secs(30)
    .build();

// 结构体字面量配合默认值展开
let config = BybitDataClientConfig {
    http_timeout_secs: 30,
    ..Default::default()
};

// 全部使用默认值
let config = BybitDataClientConfig::default();
```

对于未指定的字段，这三种方式产生完全相同的结果。

## 通用配置字段

大多数适配器配置共享一组通用字段：

| 字段                                 | 类型    | 默认值    | 用途                 |
| ---------------------------------- | ----- | ------ | ------------------ |
| `http_timeout_secs`                | `u64` | 60     | REST 请求超时。         |
| `max_retries`                      | `u32` | 3      | 最大重试次数。            |
| `retry_delay_initial_ms`           | `u64` | 1,000  | 初始退避延迟。            |
| `retry_delay_max_ms`               | `u64` | 10,000 | 最大退避延迟。            |
| `heartbeat_interval_secs`          | `u64` | 视情况    | WebSocket 保活间隔。    |
| `recv_window_ms`                   | `u64` | 视情况    | 签名请求的过期窗口。         |
| `update_instruments_interval_mins` | 视情况   | 视情况    | 周期性 instrument 刷新。 |

适配器专属的字段（限速、轮询间隔、保证金模式）记录在每个适配器各自的集成指南中。

## 引擎配置

引擎配置（`LiveExecEngineConfig`、`DataEngineConfig` 等）遵循相同的
模式。诸如 `reconciliation`、`inflight_check_interval_ms` 和
`open_check_threshold_ms` 这样的字段都是带有 builder 默认值的普通类型。真正可选的
功能则使用 `Option<T>`：

```python
from nautilus_trader.config import LiveExecEngineConfig

config = LiveExecEngineConfig(
    reconciliation=True,
    open_check_interval_secs=30.0,  # 启用未成交订单轮询
    open_check_lookback_mins=60,  # 回看 60 分钟
    # position_check_interval_secs=None  # 默认禁用
)
```
