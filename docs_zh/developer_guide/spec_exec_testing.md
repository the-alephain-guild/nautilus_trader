# 执行测试规范 (Execution Testing Spec)

本节定义了一套严格的测试矩阵，用于借助 Rust 的 `ExecTester` 策略验证适配器的执行
功能。Python 侧把它作为内置策略暴露，经 `nautilus_trader.testkit.ExecTesterConfig`
配置；Rust 侧从 `nautilus_testkit::testers` 导入。每个测试用例都用带前缀的 ID
（例如 TC-E01）标识，并按功能分组。

**每个适配器都必须通过与其所支持能力相匹配的那一部分测试。**

测试从简单（单个市价单）逐步推进到复杂（括号单、修改链、拒单处理）。通过
第 1-5 组的适配器即被视为达到基线合规。应首先使用
[数据测试规范](spec_data_testing.md) 验证数据连通性。

请把适配器特有的行为（某个交易场所如何模拟市价单、如何处理 TIF 选项等）
记录在该适配器自己的指南里，而不是这里。每份适配器指南都应包含一张能力
矩阵，标明它支持哪些订单类型、time-in-force 选项、操作和标志。

## 前提条件 (Prerequisites)

在运行执行测试之前：

- 拥有带有效 API 凭据的 demo/testnet 账户（推荐，但非必需）。
- 账户已注资，对测试用的合约和数量具有足够的保证金。
- 目标合约可用，并可通过 instrument provider 加载。
- 已设置环境变量：`{VENUE}_API_KEY`、`{VENUE}_API_SECRET`（或对应的 sandbox 变体）。
- 如果交易场所提供 demo/testnet 模式，请使用为该环境创建的凭据。Demo 与生产
  API 密钥通常是分开的、不可互换；使用错误的凭据会产生认证错误（例如 HTTP 401）。
- 绕过风险引擎（`LiveRiskEngineConfig(bypass=True)`），以避免干扰。
- 启用对账（reconciliation），以验证状态一致性。

**Python 节点设置**：

旧的示例仍使用 `nautilus_trader.live.node.TradingNode`，但新的、由 Rust 支撑的
PyO3 适配器应优先使用 `nautilus_trader.live.LiveNode`。当你需要在节点构建之前
注册适配器客户端工厂时，请使用 `LiveNode.builder(...)`。

```python
from nautilus_trader.common import Environment
from nautilus_trader.live import LiveExecEngineConfig, LiveNode, LiveRiskEngineConfig
from nautilus_trader.model import TraderId

node = (
    LiveNode.builder("TESTER-001", TraderId("TESTER-001"), Environment.SANDBOX)
    .with_risk_engine_config(LiveRiskEngineConfig(bypass=True))
    .with_exec_engine_config(LiveExecEngineConfig(reconciliation=True))
    .add_exec_client(None, adapter_exec_client_factory, exec_client_config)
    .build()
)

node.add_strategy_from_config(importable_strategy_config)
# Register remaining components, then start or run
```

**Rust 节点设置**（参考：`crates/adapters/{adapter}/examples/node_exec_tester.rs`）：

```rust
use nautilus_testkit::testers::{ExecTester, ExecTesterConfig};

let tester_config = ExecTesterConfig::new(strategy_id, instrument_id, client_id, order_qty);
let tester = ExecTester::new(tester_config);
node.add_strategy(tester)?;
node.run().await?;
```

## 基本冒烟测试 (Basic smoke test)

一项可以随时运行的快速健全性检查，例如在修改适配器后或在开发迭代之间运行。
该 tester 在启动时用一个市价单开仓，下一个买入和一个卖出 post-only 限价单，
等待 30 秒，然后停止（取消未成交订单并平仓）。

**Python 配置：**

```python
ExecTesterConfig(
    instrument_id=instrument_id,
    order_qty=Decimal("0.001"),
    open_position_on_start_qty=Decimal("0.001"),
    enable_limit_buys=True,
    enable_limit_sells=True,
    use_post_only=True,
)
```

**Rust 配置：**

```rust
ExecTesterConfig::new(strategy_id, instrument_id, client_id, dec!(0.001))
    .with_open_position_on_start_qty(Some(dec!(0.001)))
    .with_enable_limit_buys(true)
    .with_enable_limit_sells(true)
    .with_use_post_only(true)
```

**预期行为：**

1. 启动时：市价单成交，开仓。
2. 在距最优买价/卖价 `tob_offset_ticks`（默认 500 ticks）处下两个限价单。
3. 策略空转 30 秒。检查日志中是否有错误、被拒订单或断连。
4. 停止时：取消未成交的限价单，并用市价单平仓。

**通过标准：** 日志中无错误，仓位干净地开仓和平仓，限价单被交易场所确认。

---

下面每一组都以一张汇总表开头，随后是详细的测试卡片。
测试 ID 采用间隔编号，以便插入新条目时无需重新编号。

---

## 第 1 组：市价单 (Group 1: Market orders)

测试市价单的提交与成交。市价单应立即执行。

| TC     | 名称              | 描述                           | 跳过条件          |
| ------ | --------------- | ---------------------------- | ------------- |
| TC-E01 | 市价 BUY - 提交并成交  | 通过市价买入开多头仓位。                 | 不支持市价单。       |
| TC-E02 | 市价 SELL - 提交并成交 | 通过市价卖出开空头仓位。                 | 不支持市价单。       |
| TC-E03 | 带 IOC TIF 的市价单  | 显式使用 IOC time in force 的市价单。 | 不支持 IOC。      |
| TC-E04 | 带 FOK TIF 的市价单  | 显式使用 FOK time in force 的市价单。 | 不支持 FOK。      |
| TC-E05 | 带 quote 数量的市价单  | 使用计价货币数量的市价单。                | 不支持 quote 数量。 |
| TC-E06 | 通过市价单平仓         | 停止时用市价单平掉一个未平仓位。             | 不支持市价单。       |

### TC-E01: 市价 BUY - 提交并成交

| 字段       | 值                                                                           |
| -------- | --------------------------------------------------------------------------- |
| **前提条件** | 适配器已连接，合约已加载，行情数据流通，无未平仓位。                                                  |
| **操作**   | ExecTester 通过 `open_position_on_start_qty` 开一个多头仓位。                         |
| **事件序列** | `OrderInitialized` -> `OrderSubmitted` -> `OrderAccepted` -> `OrderFilled`。 |
| **通过标准** | 仓位以 side=LONG 开出，数量与配置相符，成交价在市场区间内，`AccountState` 已更新。                      |
| **跳过条件** | 适配器不支持市价单。                                                                  |

**注意事项：**

- 一些适配器将市价单模拟为激进的限价 IOC 单（查阅适配器指南）。
- 从策略视角看到的事件序列，无论交易场所采用何种机制都应相同。
- 成交价应落在近期的买/卖价差之内。
- 部分成交是合法的；请验证累计成交数量与订单数量相符。

**Python 配置：**

```python
ExecTesterConfig(
    instrument_id=instrument_id,
    order_qty=Decimal("0.01"),
    open_position_on_start_qty=Decimal("0.01"),
    enable_limit_buys=False,
    enable_limit_sells=False,
)
```

**Rust 配置：**

```rust
ExecTesterConfig::new(strategy_id, instrument_id, client_id, Quantity::from("0.01"))
    .with_open_position_on_start(Some(Decimal::new(1, 2)))
    .with_enable_limit_buys(false)
    .with_enable_limit_sells(false)
```

### TC-E02: 市价 SELL - 提交并成交

| 字段       | 值                                                                           |
| -------- | --------------------------------------------------------------------------- |
| **前提条件** | 适配器已连接，合约已加载，行情数据流通，无未平仓位。                                                  |
| **操作**   | ExecTester 通过为负的 `open_position_on_start_qty` 开一个空头仓位。                      |
| **事件序列** | `OrderInitialized` -> `OrderSubmitted` -> `OrderAccepted` -> `OrderFilled`。 |
| **通过标准** | 仓位以 side=SHORT 开出，数量与配置相符，成交价在市场区间内。                                        |
| **跳过条件** | 适配器不支持市价单或卖空。                                                               |

**Python 配置：**

```python
ExecTesterConfig(
    instrument_id=instrument_id,
    order_qty=Decimal("0.01"),
    open_position_on_start_qty=Decimal("-0.01"),
    enable_limit_buys=False,
    enable_limit_sells=False,
)
```

**Rust 配置：**

```rust
ExecTesterConfig::new(strategy_id, instrument_id, client_id, Quantity::from("0.01"))
    .with_open_position_on_start(Some(Decimal::new(-1, 2)))
    .with_enable_limit_buys(false)
    .with_enable_limit_sells(false)
```

### TC-E03: 带 IOC TIF 的市价单

| 字段       | 值                                                                           |
| -------- | --------------------------------------------------------------------------- |
| **前提条件** | 适配器已连接，合约已加载，行情数据流通。                                                        |
| **操作**   | 以 `open_position_time_in_force=IOC` 开仓。                                     |
| **事件序列** | `OrderInitialized` -> `OrderSubmitted` -> `OrderAccepted` -> `OrderFilled`。 |
| **通过标准** | 与 TC-E01 相同；订单上显式设置了 IOC TIF。                                               |
| **跳过条件** | 不支持 IOC。                                                                    |

**Python 配置：**

```python
ExecTesterConfig(
    instrument_id=instrument_id,
    order_qty=Decimal("0.01"),
    open_position_on_start_qty=Decimal("0.01"),
    open_position_time_in_force=TimeInForce.IOC,
    enable_limit_buys=False,
    enable_limit_sells=False,
)
```

**Rust 配置：**

```rust
let mut config = ExecTesterConfig::new(strategy_id, instrument_id, client_id, Quantity::from("0.01"))
    .with_open_position_on_start(Some(Decimal::new(1, 2)))
    .with_enable_limit_buys(false)
    .with_enable_limit_sells(false);
config.open_position_time_in_force = TimeInForce::Ioc;
```

### TC-E04: 带 FOK TIF 的市价单

| 字段       | 值                                                                           |
| -------- | --------------------------------------------------------------------------- |
| **前提条件** | 适配器已连接，合约已加载，行情数据流通。                                                        |
| **操作**   | 以 `open_position_time_in_force=FOK` 开仓。                                     |
| **事件序列** | `OrderInitialized` -> `OrderSubmitted` -> `OrderAccepted` -> `OrderFilled`。 |
| **通过标准** | 与 TC-E01 相同；订单上显式设置了 FOK TIF。                                               |
| **跳过条件** | 不支持 FOK。                                                                    |

**注意事项：**

- FOK 要求整个数量可立即成交，否则订单被取消。
- 使用较小的测试数量，以确保盘口深度足以完整成交。

**Python 配置：**

```python
ExecTesterConfig(
    instrument_id=instrument_id,
    order_qty=Decimal("0.01"),
    open_position_on_start_qty=Decimal("0.01"),
    open_position_time_in_force=TimeInForce.FOK,
    enable_limit_buys=False,
    enable_limit_sells=False,
)
```

**Rust 配置：**

```rust
let mut config = ExecTesterConfig::new(strategy_id, instrument_id, client_id, Quantity::from("0.01"))
    .with_open_position_on_start(Some(Decimal::new(1, 2)))
    .with_enable_limit_buys(false)
    .with_enable_limit_sells(false);
config.open_position_time_in_force = TimeInForce::Fok;
```

### TC-E05: 带 quote 数量的市价单

| 字段       | 值                                                                           |
| -------- | --------------------------------------------------------------------------- |
| **前提条件** | 适配器已连接，合约已加载，适配器支持 quote 数量。                                                |
| **操作**   | 以 `use_quote_quantity=True` 开仓，数量以计价货币计。                                    |
| **事件序列** | `OrderInitialized` -> `OrderSubmitted` -> `OrderAccepted` -> `OrderFilled`。 |
| **通过标准** | 订单以计价货币数量提交；成交数量以基础货币计。                                                     |
| **跳过条件** | 适配器不支持 quote 数量订单。                                                          |

**Python 配置：**

```python
ExecTesterConfig(
    instrument_id=instrument_id,
    order_qty=Decimal("100.0"),  # Quote currency amount
    open_position_on_start_qty=Decimal("100.0"),
    use_quote_quantity=True,
    enable_limit_buys=False,
    enable_limit_sells=False,
)
```

**Rust 配置：**

```rust
ExecTesterConfig::new(strategy_id, instrument_id, client_id, Quantity::from("100"))
    .with_open_position_on_start(Some(Decimal::from(100)))
    .with_use_quote_quantity(true)
    .with_enable_limit_buys(false)
    .with_enable_limit_sells(false)
```

### TC-E06: 停止时通过市价单平仓

| 字段       | 值                                                                                |
| -------- | -------------------------------------------------------------------------------- |
| **前提条件** | 来自 TC-E01 或 TC-E02 的未平仓位。                                                        |
| **操作**   | 停止策略；ExecTester 通过市价单平仓。                                                         |
| **事件序列** | `OrderInitialized` -> `OrderSubmitted` -> `OrderAccepted` -> `OrderFilled`（平仓单）。 |
| **通过标准** | 仓位已平（净数量 = 0），无残留未成交订单。                                                          |
| **跳过条件** | 适配器不支持市价单。                                                                       |

**注意事项：**

- 该测试自然地接续 TC-E01 或 TC-E02，作为同一会话的一部分。
- `close_positions_on_stop=True` 是默认值。
- 平仓单应位于仓位的相反方向。

**Python 配置：**

```python
ExecTesterConfig(
    instrument_id=instrument_id,
    order_qty=Decimal("0.01"),
    open_position_on_start_qty=Decimal("0.01"),
    close_positions_on_stop=True,
    enable_limit_buys=False,
    enable_limit_sells=False,
)
```

**Rust 配置：**

```rust
ExecTesterConfig::new(strategy_id, instrument_id, client_id, Quantity::from("0.01"))
    .with_open_position_on_start(Some(Decimal::new(1, 2)))
    .with_close_positions_on_stop(true)
    .with_enable_limit_buys(false)
    .with_enable_limit_sells(false)
```

---

## 第 2 组：限价单 (Group 2: Limit orders)

测试限价单的提交、被接受，以及在各种 time-in-force 选项下的行为。

| TC     | 名称               | 描述                           | 跳过条件     |
| ------ | ---------------- | ---------------------------- | -------- |
| TC-E10 | 限价 BUY GTC       | 在 TOB 下方下 GTC 限价买单，验证被接受。    | 从不跳过。    |
| TC-E11 | 限价 SELL GTC      | 在 TOB 上方下 GTC 限价卖单，验证被接受。    | 从不跳过。    |
| TC-E12 | 限价 BUY 与 SELL 配对 | 同时下两侧订单，验证两者均被接受。            | 从不跳过。    |
| TC-E13 | 限价 IOC 激进成交      | 在激进价格上下限价 IOC 单，预期成交。        | 不支持 IOC。 |
| TC-E14 | 限价 IOC 被动不成交     | 在远离市场处下限价 IOC 单，预期取消。        | 不支持 IOC。 |
| TC-E15 | 限价 FOK 成交        | 在激进价格上下限价 FOK 单，预期成交。        | 不支持 FOK。 |
| TC-E16 | 限价 FOK 不成交       | 在远离市场处下限价 FOK 单，预期取消。        | 不支持 FOK。 |
| TC-E17 | 限价 GTD           | 带到期时间的限价单，验证被接受。             | 不支持 GTD。 |
| TC-E18 | 限价 GTD 到期        | 等待 GTD 到期，验证 `OrderExpired`。 | 不支持 GTD。 |
| TC-E19 | 限价 DAY           | 带 DAY TIF 的限价单，验证被接受。        | 不支持 DAY。 |

### TC-E10: 限价 BUY GTC - 提交并被接受

| 字段       | 值                                                          |
| -------- | ---------------------------------------------------------- |
| **前提条件** | 适配器已连接，合约已加载，报价流通。                                         |
| **操作**   | ExecTester 在 `best_bid - tob_offset_ticks` 处下一个限价买单。       |
| **事件序列** | `OrderInitialized` -> `OrderSubmitted` -> `OrderAccepted`。 |
| **通过标准** | 订单在交易场所挂出，价格、数量、side=BUY、TIF=GTC 均正确。                      |
| **跳过条件** | 从不跳过。                                                      |

**注意事项：**

- `tob_offset_ticks`（默认 500）将订单下在远离市场处，以避免意外成交。
- 验证订单以 `OrderStatus.ACCEPTED` 出现在缓存中。
- 在被显式取消之前，订单应保持挂出状态。

**Python 配置：**

```python
ExecTesterConfig(
    instrument_id=instrument_id,
    order_qty=Decimal("0.01"),
    enable_limit_buys=True,
    enable_limit_sells=False,
)
```

**Rust 配置：**

```rust
ExecTesterConfig::new(strategy_id, instrument_id, client_id, Quantity::from("0.01"))
    .with_enable_limit_buys(true)
    .with_enable_limit_sells(false)
```

### TC-E11: 限价 SELL GTC - 提交并被接受

| 字段       | 值                                                          |
| -------- | ---------------------------------------------------------- |
| **前提条件** | 适配器已连接，合约已加载，报价流通。                                         |
| **操作**   | ExecTester 在 `best_ask + tob_offset_ticks` 处下一个限价卖单。       |
| **事件序列** | `OrderInitialized` -> `OrderSubmitted` -> `OrderAccepted`。 |
| **通过标准** | 订单在交易场所挂出，价格、数量、side=SELL、TIF=GTC 均正确。                     |
| **跳过条件** | 从不跳过。                                                      |

**Python 配置：**

```python
ExecTesterConfig(
    instrument_id=instrument_id,
    order_qty=Decimal("0.01"),
    enable_limit_buys=False,
    enable_limit_sells=True,
)
```

**Rust 配置：**

```rust
ExecTesterConfig::new(strategy_id, instrument_id, client_id, Quantity::from("0.01"))
    .with_enable_limit_buys(false)
    .with_enable_limit_sells(true)
```

### TC-E12: 限价 BUY 与 SELL 配对

| 字段       | 值                                                                    |
| -------- | -------------------------------------------------------------------- |
| **前提条件** | 适配器已连接，合约已加载，报价流通。                                                   |
| **操作**   | ExecTester 同时下一个限价买单和一个限价卖单。                                         |
| **事件序列** | 两条独立序列：各自 `OrderInitialized` -> `OrderSubmitted` -> `OrderAccepted`。 |
| **通过标准** | 两个订单都在交易场所挂出，买单在买价下方，卖单在卖价上方。                                        |
| **跳过条件** | 从不跳过。                                                                |

**Python 配置：**

```python
ExecTesterConfig(
    instrument_id=instrument_id,
    order_qty=Decimal("0.01"),
    enable_limit_buys=True,
    enable_limit_sells=True,
)
```

**Rust 配置：**

```rust
ExecTesterConfig::new(strategy_id, instrument_id, client_id, Quantity::from("0.01"))
    .with_enable_limit_buys(true)
    .with_enable_limit_sells(true)
```

### TC-E13: 限价 IOC 激进成交

| 字段       | 值                                                                           |
| -------- | --------------------------------------------------------------------------- |
| **前提条件** | 适配器已连接，合约已加载，报价流通。                                                          |
| **操作**   | 在等于或高于最优卖价（激进价格）处提交一个限价买入 IOC 单。                                            |
| **事件序列** | `OrderInitialized` -> `OrderSubmitted` -> `OrderAccepted` -> `OrderFilled`。 |
| **通过标准** | 订单立即成交；仓位开出。                                                                |
| **跳过条件** | 适配器不支持 IOC TIF。                                                             |

**注意事项：**

- 该测试需要手动创建订单或进行适配器特有的配置，因为 ExecTester 默认的限价单
  下单使用 GTC TIF。
- 未能立即成交的 IOC 订单会被交易场所取消。

### TC-E14: 限价 IOC 被动 - 不成交

| 字段       | 值                                                                             |
| -------- | ----------------------------------------------------------------------------- |
| **前提条件** | 适配器已连接，合约已加载，报价流通。                                                            |
| **操作**   | 在远低于市场（被动价格）处提交一个限价买入 IOC 单。                                                  |
| **事件序列** | `OrderInitialized` -> `OrderSubmitted` -> `OrderAccepted` -> `OrderCanceled`。 |
| **通过标准** | 订单被交易场所立即取消，无成交。                                                              |
| **跳过条件** | 适配器不支持 IOC TIF。                                                               |

**注意事项：**

- 交易场所应取消未成交的 IOC 订单；验证收到 `OrderCanceled` 事件（而非 `OrderExpired`）。

### TC-E15: 限价 FOK 成交

| 字段       | 值                                                                           |
| -------- | --------------------------------------------------------------------------- |
| **前提条件** | 适配器已连接，合约已加载，报价流通，盘口深度充足。                                                   |
| **操作**   | 在激进价格上提交一个限价买入 FOK 单，数量在 top-of-book 深度之内。                                  |
| **事件序列** | `OrderInitialized` -> `OrderSubmitted` -> `OrderAccepted` -> `OrderFilled`。 |
| **通过标准** | 订单在单个成交事件中完全成交。                                                             |
| **跳过条件** | 适配器不支持 FOK TIF。                                                             |

**注意事项：**

- FOK 要求整个数量可成交；使用较小的数量以确保盘口深度足够。

### TC-E16: 限价 FOK 不成交

| 字段       | 值                                                                             |
| -------- | ----------------------------------------------------------------------------- |
| **前提条件** | 适配器已连接，合约已加载，报价流通。                                                            |
| **操作**   | 在被动价格（远低于市场）提交一个限价买入 FOK 单。                                                   |
| **事件序列** | `OrderInitialized` -> `OrderSubmitted` -> `OrderAccepted` -> `OrderCanceled`。 |
| **通过标准** | 订单被交易场所立即取消，无成交。                                                              |
| **跳过条件** | 适配器不支持 FOK TIF。                                                               |

### TC-E17: 限价 GTD - 提交并被接受

| 字段       | 值                                                          |
| -------- | ---------------------------------------------------------- |
| **前提条件** | 适配器已连接，合约已加载，报价流通。                                         |
| **操作**   | 下一个设置了 `order_expire_time_delta_mins` 的限价买单（例如 60 分钟）。     |
| **事件序列** | `OrderInitialized` -> `OrderSubmitted` -> `OrderAccepted`。 |
| **通过标准** | 订单以 GTD TIF 被接受，且到期时间戳正确。                                  |
| **跳过条件** | 适配器不支持 GTD TIF。                                            |

**Python 配置：**

```python
ExecTesterConfig(
    instrument_id=instrument_id,
    order_qty=Decimal("0.01"),
    order_expire_time_delta_mins=60,
    enable_limit_buys=True,
    enable_limit_sells=False,
)
```

**Rust 配置：**

```rust
let mut config = ExecTesterConfig::new(strategy_id, instrument_id, client_id, Quantity::from("0.01"))
    .with_enable_limit_buys(true)
    .with_enable_limit_sells(false);
config.order_expire_time_delta_mins = Some(60);
```

### TC-E18: 限价 GTD 到期

| 字段       | 值                                     |
| -------- | ------------------------------------- |
| **前提条件** | 来自 TC-E17 的未成交 GTD 限价单（或使用极短的到期时间）。   |
| **操作**   | 等待 GTD 到期时间流逝。                        |
| **事件序列** | `OrderExpired`。                       |
| **通过标准** | 订单转入 expired 状态；收到 `OrderExpired` 事件。 |
| **跳过条件** | 适配器不支持 GTD TIF。                       |

**注意事项：**

- 使用较短的 `order_expire_time_delta_mins`（例如 1-2 分钟）以避免长时间等待。
- 一些交易场所可能将到期报告为取消；验证适配器将其映射为 `OrderExpired`。

### TC-E19: 限价 DAY - 提交并被接受

| 字段       | 值                                                          |
| -------- | ---------------------------------------------------------- |
| **前提条件** | 适配器已连接，合约已加载，市场处于交易时段。                                     |
| **操作**   | 提交一个带 DAY TIF 的限价买单。                                       |
| **事件序列** | `OrderInitialized` -> `OrderSubmitted` -> `OrderAccepted`。 |
| **通过标准** | 订单以 DAY TIF 被接受；将在交易日结束时被自动取消。                             |
| **跳过条件** | 适配器不支持 DAY TIF。                                            |

**注意事项：**

- DAY 订单在 7×24 小时的加密交易场所与传统市场上的行为可能不同。
- 验证在交易时段之外提交时的行为（若适用）。

---

## 第 3 组：止损单与条件单 (Group 3: Stop and conditional orders)

测试止损单与条件单类型。这些订单驻留在交易场所，直到触发条件被满足。
支持交易场所原生条件单的适配器还应验证：未触发的触发单会出现在重启对账中，
而不只是出现在普通的未成交订单端点里。

| TC     | 名称                   | 描述                 | 跳过条件               |
| ------ | -------------------- | ------------------ | ------------------ |
| TC-E20 | StopMarket BUY       | 在卖价上方下止损买单，验证被接受。  | 不支持 `STOP_MARKET`。 |
| TC-E21 | StopMarket SELL      | 在买价下方下止损卖单，验证被接受。  | 不支持 `STOP_MARKET`。 |
| TC-E22 | StopLimit BUY        | 带触发价 + 限价的止损限价买单。  | 不支持 `STOP_LIMIT`。  |
| TC-E23 | StopLimit SELL       | 带触发价 + 限价的止损限价卖单。  | 不支持 `STOP_LIMIT`。  |
| TC-E24 | MarketIfTouched BUY  | 在买价下方下 MIT 买单。     | 不支持 `MIT`。         |
| TC-E25 | MarketIfTouched SELL | 在卖价上方下 MIT 卖单。     | 不支持 `MIT`。         |
| TC-E26 | LimitIfTouched BUY   | 带触发价 + 限价的 LIT 买单。 | 不支持 `LIT`。         |
| TC-E27 | LimitIfTouched SELL  | 带触发价 + 限价的 LIT 卖单。 | 不支持 `LIT`。         |

### TC-E20: StopMarket BUY

| 字段       | 值                                                          |
| -------- | ---------------------------------------------------------- |
| **前提条件** | 适配器已连接，合约已加载，报价流通。                                         |
| **操作**   | ExecTester 在当前卖价上方下一个止损市价买单。                               |
| **事件序列** | `OrderInitialized` -> `OrderSubmitted` -> `OrderAccepted`。 |
| **通过标准** | 止损单在交易场所被接受，触发价正确且 side=BUY。                               |
| **跳过条件** | 适配器不支持 `StopMarket` 订单。                                    |

**注意事项：**

- 触发价应在当前卖价上方 `stop_offset_ticks` 处。
- 订单不应立即触发（触发价在市场上方）。
- 对于使用长生命周期触发签名的交易场所，验证触发单的签名到期采用该场所的
  触发单窗口，而非普通订单的到期。
- 验证触发与成交需要市场移动，而这在测试期间可能不会发生。
- 被接受后，重启或强制对账，并验证当交易场所将触发单保存在单独端点时，
  该订单仍以未成交订单报告的形式出现。

**Python 配置：**

```python
ExecTesterConfig(
    instrument_id=instrument_id,
    order_qty=Decimal("0.01"),
    enable_limit_buys=False,
    enable_limit_sells=False,
    enable_stop_buys=True,
    enable_stop_sells=False,
    stop_order_type=OrderType.STOP_MARKET,
)
```

**Rust 配置：**

```rust
ExecTesterConfig::new(strategy_id, instrument_id, client_id, Quantity::from("0.01"))
    .with_enable_limit_buys(false)
    .with_enable_limit_sells(false)
    .with_enable_stop_buys(true)
    .with_enable_stop_sells(false)
    .with_stop_order_type(OrderType::StopMarket)
```

### TC-E21: StopMarket SELL

| 字段       | 值                                                          |
| -------- | ---------------------------------------------------------- |
| **前提条件** | 适配器已连接，合约已加载，报价流通。                                         |
| **操作**   | ExecTester 在当前买价下方下一个止损市价卖单。                               |
| **事件序列** | `OrderInitialized` -> `OrderSubmitted` -> `OrderAccepted`。 |
| **通过标准** | 止损单在交易场所被接受，触发价正确且 side=SELL。                              |
| **跳过条件** | 适配器不支持 `StopMarket` 订单。                                    |

**Python 配置：**

```python
ExecTesterConfig(
    instrument_id=instrument_id,
    order_qty=Decimal("0.01"),
    enable_limit_buys=False,
    enable_limit_sells=False,
    enable_stop_buys=False,
    enable_stop_sells=True,
    stop_order_type=OrderType.STOP_MARKET,
)
```

**Rust 配置：**

```rust
ExecTesterConfig::new(strategy_id, instrument_id, client_id, Quantity::from("0.01"))
    .with_enable_limit_buys(false)
    .with_enable_limit_sells(false)
    .with_enable_stop_buys(false)
    .with_enable_stop_sells(true)
    .with_stop_order_type(OrderType::StopMarket)
```

### TC-E22: StopLimit BUY

| 字段       | 值                                                          |
| -------- | ---------------------------------------------------------- |
| **前提条件** | 适配器已连接，合约已加载，报价流通。                                         |
| **操作**   | ExecTester 下一个止损限价买单，触发价在卖价上方并带限价偏移。                       |
| **事件序列** | `OrderInitialized` -> `OrderSubmitted` -> `OrderAccepted`。 |
| **通过标准** | 止损限价单被接受，触发价、限价正确且 side=BUY。                               |
| **跳过条件** | 适配器不支持 `StopLimit` 订单。                                     |

**注意事项：**

- 需要设置 `stop_limit_offset_ticks`，作为限价相对触发价的偏移。

**Python 配置：**

```python
ExecTesterConfig(
    instrument_id=instrument_id,
    order_qty=Decimal("0.01"),
    enable_limit_buys=False,
    enable_limit_sells=False,
    enable_stop_buys=True,
    enable_stop_sells=False,
    stop_order_type=OrderType.STOP_LIMIT,
    stop_limit_offset_ticks=50,
)
```

**Rust 配置：**

```rust
let mut config = ExecTesterConfig::new(strategy_id, instrument_id, client_id, Quantity::from("0.01"))
    .with_enable_limit_buys(false)
    .with_enable_limit_sells(false)
    .with_enable_stop_buys(true)
    .with_enable_stop_sells(false)
    .with_stop_order_type(OrderType::StopLimit);
config.stop_limit_offset_ticks = Some(50);
```

### TC-E23: StopLimit SELL

| 字段       | 值                                                          |
| -------- | ---------------------------------------------------------- |
| **前提条件** | 适配器已连接，合约已加载，报价流通。                                         |
| **操作**   | ExecTester 下一个止损限价卖单，触发价在买价下方。                             |
| **事件序列** | `OrderInitialized` -> `OrderSubmitted` -> `OrderAccepted`。 |
| **通过标准** | 止损限价单被接受，触发价、限价正确且 side=SELL。                              |
| **跳过条件** | 适配器不支持 `StopLimit` 订单。                                     |

**Python 配置：**

```python
ExecTesterConfig(
    instrument_id=instrument_id,
    order_qty=Decimal("0.01"),
    enable_limit_buys=False,
    enable_limit_sells=False,
    enable_stop_buys=False,
    enable_stop_sells=True,
    stop_order_type=OrderType.STOP_LIMIT,
    stop_limit_offset_ticks=50,
)
```

**Rust 配置：**

```rust
let mut config = ExecTesterConfig::new(strategy_id, instrument_id, client_id, Quantity::from("0.01"))
    .with_enable_limit_buys(false)
    .with_enable_limit_sells(false)
    .with_enable_stop_buys(false)
    .with_enable_stop_sells(true)
    .with_stop_order_type(OrderType::StopLimit);
config.stop_limit_offset_ticks = Some(50);
```

### TC-E24: MarketIfTouched BUY

| 字段       | 值                                                          |
| -------- | ---------------------------------------------------------- |
| **前提条件** | 适配器已连接，合约已加载，报价流通。                                         |
| **操作**   | 下一个 MIT 买单，触发价在当前买价下方（逢低买入）。                               |
| **事件序列** | `OrderInitialized` -> `OrderSubmitted` -> `OrderAccepted`。 |
| **通过标准** | MIT 单在交易场所被接受，触发价正确。                                       |
| **跳过条件** | 适配器不支持 `MarketIfTouched` 订单。                               |

### TC-E25: MarketIfTouched SELL

| 字段       | 值                                                          |
| -------- | ---------------------------------------------------------- |
| **前提条件** | 适配器已连接，合约已加载，报价流通。                                         |
| **操作**   | 下一个 MIT 卖单，触发价在当前卖价上方（逢高卖出）。                               |
| **事件序列** | `OrderInitialized` -> `OrderSubmitted` -> `OrderAccepted`。 |
| **通过标准** | MIT 单在交易场所被接受，触发价正确。                                       |
| **跳过条件** | 适配器不支持 `MarketIfTouched` 订单。                               |

### TC-E26: LimitIfTouched BUY

| 字段       | 值                                                          |
| -------- | ---------------------------------------------------------- |
| **前提条件** | 适配器已连接，合约已加载，报价流通。                                         |
| **操作**   | 下一个 LIT 买单，触发价在买价下方并带限价偏移。                                 |
| **事件序列** | `OrderInitialized` -> `OrderSubmitted` -> `OrderAccepted`。 |
| **通过标准** | LIT 单被接受，触发价与限价正确。                                         |
| **跳过条件** | 适配器不支持 `LimitIfTouched` 订单。                                |

### TC-E27: LimitIfTouched SELL

| 字段       | 值                                                          |
| -------- | ---------------------------------------------------------- |
| **前提条件** | 适配器已连接，合约已加载，报价流通。                                         |
| **操作**   | 下一个 LIT 卖单，触发价在卖价上方并带限价偏移。                                 |
| **事件序列** | `OrderInitialized` -> `OrderSubmitted` -> `OrderAccepted`。 |
| **通过标准** | LIT 单被接受，触发价与限价正确。                                         |
| **跳过条件** | 适配器不支持 `LimitIfTouched` 订单。                                |

---

## 第 4 组：订单修改 (Group 4: Order modification)

测试订单修改（amend）以及撤单重提（cancel-replace）工作流。

| TC     | 名称           | 描述                    | 跳过条件       |
| ------ | ------------ | --------------------- | ---------- |
| TC-E30 | 修改限价 BUY 价格  | 将未成交的限价买单 amend 到新价格。 | 不支持修改。     |
| TC-E31 | 修改限价 SELL 价格 | 将未成交的限价卖单 amend 到新价格。 | 不支持修改。     |
| TC-E32 | 撤单重提限价 BUY   | 撤销限价买单并以新价格重新提交。      | 从不跳过。      |
| TC-E33 | 撤单重提限价 SELL  | 撤销限价卖单并以新价格重新提交。      | 从不跳过。      |
| TC-E34 | 修改止损触发价      | amend 止损单的触发价。        | 不支持修改或无止损。 |
| TC-E35 | 撤单重提止损单      | 撤销止损单并以新触发价重新提交。      | 不支持止损单。    |
| TC-E36 | 修改被拒         | 在不支持修改的适配器上发起修改。      | 适配器支持修改。   |

### TC-E30: 修改限价 BUY 价格

| 字段       | 值                                                                          |
| -------- | -------------------------------------------------------------------------- |
| **前提条件** | 来自 TC-E10 的未成交 GTC 限价买单。                                                   |
| **操作**   | 随市场移动，ExecTester 将限价买单修改到新价格（`modify_orders_to_maintain_tob_offset=True`）。 |
| **事件序列** | `OrderPendingUpdate` -> `OrderUpdated`。                                    |
| **通过标准** | 记录到带新价格的 `OrderUpdated` 事件；订单退出 `PendingUpdate`。                           |
| **跳过条件** | 适配器不支持订单修改。                                                                |

**注意事项：**

- 需要市场移动来触发 ExecTester 的订单维护逻辑。
- 当订单价格偏离目标 TOB 偏移时，触发修改。
- 验证 `OrderUpdated` 日志显示预期价格。如果该事件始终未到达，订单将停留在
  `PendingUpdate`，且 tester 会停止对其进行修改。

**Python 配置：**

```python
ExecTesterConfig(
    instrument_id=instrument_id,
    order_qty=Decimal("0.01"),
    enable_limit_buys=True,
    enable_limit_sells=False,
    modify_orders_to_maintain_tob_offset=True,
)
```

**Rust 配置：**

```rust
let mut config = ExecTesterConfig::new(strategy_id, instrument_id, client_id, Quantity::from("0.01"))
    .with_enable_limit_buys(true)
    .with_enable_limit_sells(false);
config.modify_orders_to_maintain_tob_offset = true;
```

### TC-E31: 修改限价 SELL 价格

| 字段       | 值                                                |
| -------- | ------------------------------------------------ |
| **前提条件** | 来自 TC-E11 的未成交 GTC 限价卖单。                         |
| **操作**   | 随市场移动，ExecTester 将限价卖单修改到新价格。                    |
| **事件序列** | `OrderPendingUpdate` -> `OrderUpdated`。          |
| **通过标准** | 记录到带新价格的 `OrderUpdated` 事件；订单退出 `PendingUpdate`。 |
| **跳过条件** | 适配器不支持订单修改。                                      |

**Python 配置：**

```python
ExecTesterConfig(
    instrument_id=instrument_id,
    order_qty=Decimal("0.01"),
    enable_limit_buys=False,
    enable_limit_sells=True,
    modify_orders_to_maintain_tob_offset=True,
)
```

**Rust 配置：**

```rust
let mut config = ExecTesterConfig::new(strategy_id, instrument_id, client_id, Quantity::from("0.01"))
    .with_enable_limit_buys(false)
    .with_enable_limit_sells(true);
config.modify_orders_to_maintain_tob_offset = true;
```

### TC-E32: 撤单重提限价 BUY

| 字段       | 值                                                                                                     |
| -------- | ----------------------------------------------------------------------------------------------------- |
| **前提条件** | 未成交的 GTC 限价买单。                                                                                        |
| **操作**   | 随市场移动，ExecTester 撤销限价买单并以新价格重新提交。                                                                     |
| **事件序列** | `OrderPendingCancel` -> `OrderCanceled` -> `OrderInitialized` -> `OrderSubmitted` -> `OrderAccepted`。 |
| **通过标准** | 原订单被撤销，新订单以更新后的价格被接受。                                                                                 |
| **跳过条件** | 从不跳过（撤单重提始终可用）。                                                                                       |

**注意事项：**

- 当适配器不支持原生修改时，这是通用的替代方案。
- 缓存中有两个不同的订单：被撤销的原单和新的替代单。

**Python 配置：**

```python
ExecTesterConfig(
    instrument_id=instrument_id,
    order_qty=Decimal("0.01"),
    enable_limit_buys=True,
    enable_limit_sells=False,
    cancel_replace_orders_to_maintain_tob_offset=True,
)
```

**Rust 配置：**

```rust
let mut config = ExecTesterConfig::new(strategy_id, instrument_id, client_id, Quantity::from("0.01"))
    .with_enable_limit_buys(true)
    .with_enable_limit_sells(false);
config.cancel_replace_orders_to_maintain_tob_offset = true;
```

### TC-E33: 撤单重提限价 SELL

| 字段       | 值                                                                                                     |
| -------- | ----------------------------------------------------------------------------------------------------- |
| **前提条件** | 未成交的 GTC 限价卖单。                                                                                        |
| **操作**   | 随市场移动，ExecTester 撤销限价卖单并以新价格重新提交。                                                                     |
| **事件序列** | `OrderPendingCancel` -> `OrderCanceled` -> `OrderInitialized` -> `OrderSubmitted` -> `OrderAccepted`。 |
| **通过标准** | 原订单被撤销，新订单以更新后的价格被接受。                                                                                 |
| **跳过条件** | 从不跳过。                                                                                                 |

**Python 配置：**

```python
ExecTesterConfig(
    instrument_id=instrument_id,
    order_qty=Decimal("0.01"),
    enable_limit_buys=False,
    enable_limit_sells=True,
    cancel_replace_orders_to_maintain_tob_offset=True,
)
```

**Rust 配置：**

```rust
let mut config = ExecTesterConfig::new(strategy_id, instrument_id, client_id, Quantity::from("0.01"))
    .with_enable_limit_buys(false)
    .with_enable_limit_sells(true);
config.cancel_replace_orders_to_maintain_tob_offset = true;
```

### TC-E34: 修改止损触发价

| 字段       | 值                                                                       |
| -------- | ----------------------------------------------------------------------- |
| **前提条件** | 来自 TC-E20 或 TC-E22 的未成交止损单。                                             |
| **操作**   | 随市场移动，ExecTester 修改止损触发价（`modify_stop_orders_to_maintain_offset=True`）。 |
| **事件序列** | `OrderPendingUpdate` -> `OrderUpdated`。                                 |
| **通过标准** | 记录到带新触发价的 `OrderUpdated` 事件；订单退出 `PendingUpdate`。                       |
| **跳过条件** | 适配器不支持原生止损修改，或不支持止损单。                                                   |

**注意事项：**

- 一些交易场所允许限价单修改，但拒绝触发单替换。对于这类适配器，
  跳过 TC-E34 并改为运行 TC-E35。

**Python 配置：**

```python
ExecTesterConfig(
    instrument_id=instrument_id,
    order_qty=Decimal("0.01"),
    enable_stop_buys=True,
    modify_stop_orders_to_maintain_offset=True,
)
```

**Rust 配置：**

```rust
let mut config = ExecTesterConfig::new(strategy_id, instrument_id, client_id, Quantity::from("0.01"))
    .with_enable_stop_buys(true);
config.modify_stop_orders_to_maintain_offset = true;
```

### TC-E35: 撤单重提止损单

| 字段       | 值                                                                                                     |
| -------- | ----------------------------------------------------------------------------------------------------- |
| **前提条件** | 未成交的止损单。                                                                                              |
| **操作**   | ExecTester 撤销止损单并以新触发价重新提交。                                                                           |
| **事件序列** | `OrderPendingCancel` -> `OrderCanceled` -> `OrderInitialized` -> `OrderSubmitted` -> `OrderAccepted`。 |
| **通过标准** | 原止损单被撤销，新止损单以更新后的触发价被接受。                                                                              |
| **跳过条件** | 不支持止损单。                                                                                               |

**注意事项：**

- 对于不支持原生触发单替换的交易场所，这是必需的路径。
- 新止损单被接受后，重启或强制对账，并验证恰好剩下一个当前触发单。

**Python 配置：**

```python
ExecTesterConfig(
    instrument_id=instrument_id,
    order_qty=Decimal("0.01"),
    enable_stop_buys=True,
    cancel_replace_stop_orders_to_maintain_offset=True,
)
```

**Rust 配置：**

```rust
let mut config = ExecTesterConfig::new(strategy_id, instrument_id, client_id, Quantity::from("0.01"))
    .with_enable_stop_buys(true);
config.cancel_replace_stop_orders_to_maintain_offset = true;
```

### TC-E36: 修改被拒

| 字段       | 值                                            |
| -------- | -------------------------------------------- |
| **前提条件** | 未成交的限价单，且适配器不支持修改。                           |
| **操作**   | 尝试修改该订单（以编程方式，而非通过 ExecTester 的自动维护）。        |
| **事件序列** | `OrderModifyRejected`。                       |
| **通过标准** | 修改尝试导致带原因的 `OrderModifyRejected` 事件；原订单保持不变。 |
| **跳过条件** | 适配器支持订单修改。                                   |

**注意事项：**

- 该测试检验的是适配器的拒绝路径，而非 ExecTester 的撤单重提逻辑。
- 拒绝原因应表明不支持修改。

---

## 第 5 组：订单取消 (Group 5: Order cancellation)

测试订单取消工作流。

| TC     | 名称       | 描述                  | 跳过条件     |
| ------ | -------- | ------------------- | -------- |
| TC-E40 | 取消单个限价单  | 取消一个未成交的限价单。        | 从不跳过。    |
| TC-E41 | 停止时全部取消  | 策略停止时取消所有未成交订单（默认）。 | 从不跳过。    |
| TC-E42 | 停止时逐个取消  | 停止时逐个取消订单。          | 从不跳过。    |
| TC-E43 | 停止时批量取消  | 停止时通过批量 API 取消订单。   | 不支持批量取消。 |
| TC-E44 | 取消已取消的订单 | 取消一个非未成交状态的订单。      | 从不跳过。    |

### TC-E40: 取消单个限价单

| 字段       | 值                                        |
| -------- | ---------------------------------------- |
| **前提条件** | 来自 TC-E10 或 TC-E11 的未成交 GTC 限价单。         |
| **操作**   | 停止策略；ExecTester 取消该未成交的限价单。              |
| **事件序列** | `OrderPendingCancel` -> `OrderCanceled`。 |
| **通过标准** | 订单状态转入 CANCELED；无残留未成交订单。                |
| **跳过条件** | 从不跳过。                                    |

**注意事项：**

- `cancel_orders_on_stop=True`（默认）在策略停止时触发取消。
- 验证 `OrderCanceled` 事件包含正确的 `venue_order_id`。

**Python 配置：**

```python
ExecTesterConfig(
    instrument_id=instrument_id,
    order_qty=Decimal("0.01"),
    enable_limit_buys=True,
    enable_limit_sells=False,
    cancel_orders_on_stop=True,
)
```

**Rust 配置：**

```rust
ExecTesterConfig::new(strategy_id, instrument_id, client_id, Quantity::from("0.01"))
    .with_enable_limit_buys(true)
    .with_enable_limit_sells(false)
    .with_cancel_orders_on_stop(true)
```

### TC-E41: 停止时全部取消

| 字段       | 值                                              |
| -------- | ---------------------------------------------- |
| **前提条件** | 多个未成交订单（来自 TC-E12 的限价买单 + 限价卖单）。               |
| **操作**   | 以 `cancel_orders_on_stop=True`（默认）停止策略。        |
| **事件序列** | 对每个订单：`OrderPendingCancel` -> `OrderCanceled`。 |
| **通过标准** | 所有未成交订单被取消；无残留未成交订单。                           |
| **跳过条件** | 从不跳过。                                          |

**Python 配置：**

```python
ExecTesterConfig(
    instrument_id=instrument_id,
    order_qty=Decimal("0.01"),
    enable_limit_buys=True,
    enable_limit_sells=True,
    cancel_orders_on_stop=True,
)
```

**Rust 配置：**

```rust
ExecTesterConfig::new(strategy_id, instrument_id, client_id, Quantity::from("0.01"))
    .with_enable_limit_buys(true)
    .with_enable_limit_sells(true)
    .with_cancel_orders_on_stop(true)
```

### TC-E42: 停止时逐个取消

| 字段       | 值                                                 |
| -------- | ------------------------------------------------- |
| **前提条件** | 多个未成交订单。                                          |
| **操作**   | 以 `use_individual_cancels_on_stop=True` 停止。       |
| **事件序列** | 对每个订单单独的 `OrderPendingCancel` -> `OrderCanceled`。 |
| **通过标准** | 每个订单被单独取消；所有订单到达 CANCELED 状态。                     |
| **跳过条件** | 从不跳过。                                             |

**Python 配置：**

```python
ExecTesterConfig(
    instrument_id=instrument_id,
    order_qty=Decimal("0.01"),
    enable_limit_buys=True,
    enable_limit_sells=True,
    use_individual_cancels_on_stop=True,
)
```

**Rust 配置：**

```rust
let mut config = ExecTesterConfig::new(strategy_id, instrument_id, client_id, Quantity::from("0.01"))
    .with_enable_limit_buys(true)
    .with_enable_limit_sells(true);
config.use_individual_cancels_on_stop = true;
```

### TC-E43: 停止时批量取消

| 字段       | 值                                                 |
| -------- | ------------------------------------------------- |
| **前提条件** | 多个未成交订单，适配器支持批量取消。                                |
| **操作**   | 以 `use_batch_cancel_on_stop=True` 停止。             |
| **事件序列** | 对所有订单的批量 `OrderPendingCancel` -> `OrderCanceled`。 |
| **通过标准** | 所有订单通过单个批量请求被取消；全部到达 CANCELED 状态。                 |
| **跳过条件** | 适配器不支持批量取消。                                       |

**Python 配置：**

```python
ExecTesterConfig(
    instrument_id=instrument_id,
    order_qty=Decimal("0.01"),
    enable_limit_buys=True,
    enable_limit_sells=True,
    use_batch_cancel_on_stop=True,
)
```

**Rust 配置：**

```rust
ExecTesterConfig::new(strategy_id, instrument_id, client_id, Quantity::from("0.01"))
    .with_enable_limit_buys(true)
    .with_enable_limit_sells(true)
    .with_use_batch_cancel_on_stop(true)
```

### TC-E44: 取消已取消的订单

| 字段       | 值                                        |
| -------- | ---------------------------------------- |
| **前提条件** | 一个先前已取消的订单（来自 TC-E40）。                   |
| **操作**   | 再次尝试取消同一个订单。                             |
| **事件序列** | `OrderCancelRejected`。                   |
| **通过标准** | 取消尝试被拒绝；收到带原因的 `OrderCancelRejected` 事件。 |
| **跳过条件** | 从不跳过。                                    |

**注意事项：**

- 该测试检验适配器对无效取消请求的错误处理。
- 拒绝原因应表明订单不处于可取消状态。

---

## 第 6 组：括号单 (Group 6: Bracket orders)

测试括号单的提交（入场 + 止盈 + 止损）。

| TC     | 名称                 | 描述                       | 跳过条件       |
| ------ | ------------------ | ------------------------ | ---------- |
| TC-E50 | 括号 BUY             | 入场限价买 + TP 限价卖 + SL 止损卖。 | 不支持括号单。    |
| TC-E51 | 括号 SELL            | 入场限价卖 + TP 限价买 + SL 止损买。 | 不支持括号单。    |
| TC-E52 | 括号入场成交后激活          | 验证入场成交后 TP/SL 变为激活。      | 不支持括号单。    |
| TC-E53 | 带 post-only 入场的括号单 | 入场单使用 post-only 标志。      | 不支持括号或 PO。 |

### TC-E50: 括号 BUY

| 字段       | 值                                                                                                                               |
| -------- | ------------------------------------------------------------------------------------------------------------------------------- |
| **前提条件** | 适配器已连接，合约已加载，报价流通。                                                                                                              |
| **操作**   | ExecTester 提交一个括号单：入场限价买 + 止盈卖 + 止损卖。                                                                                           |
| **事件序列** | 入场：`OrderInitialized` -> `OrderSubmitted` -> `OrderAccepted`；TP 与 SL：`OrderInitialized` -> `OrderSubmitted` -> `OrderAccepted`。 |
| **通过标准** | 创建并接受三个订单：入场在买价下方，TP 在卖价上方，SL 在入场价下方。                                                                                           |
| **跳过条件** | 适配器不支持括号单。                                                                                                                      |

**Python 配置：**

```python
ExecTesterConfig(
    instrument_id=instrument_id,
    order_qty=Decimal("0.01"),
    enable_brackets=True,
    bracket_entry_order_type=OrderType.LIMIT,
    bracket_offset_ticks=500,
    enable_limit_buys=True,
    enable_limit_sells=False,
)
```

**Rust 配置：**

```rust
ExecTesterConfig::new(strategy_id, instrument_id, client_id, Quantity::from("0.01"))
    .with_enable_brackets(true)
    .with_bracket_entry_order_type(OrderType::Limit)
    .with_bracket_offset_ticks(500)
    .with_enable_limit_buys(true)
    .with_enable_limit_sells(false)
```

### TC-E51: 括号 SELL

| 字段       | 值                                     |
| -------- | ------------------------------------- |
| **前提条件** | 适配器已连接，合约已加载，报价流通。                    |
| **操作**   | ExecTester 提交括号单：入场限价卖 + TP 买 + SL 买。 |
| **事件序列** | 与 TC-E50 相同的模式，但用于卖出方向。               |
| **通过标准** | 在卖出方向创建并接受三个订单。                       |
| **跳过条件** | 适配器不支持括号单。                            |

### TC-E52: 括号入场成交激活 TP/SL

| 字段       | 值                                             |
| -------- | --------------------------------------------- |
| **前提条件** | 来自 TC-E50 且入场单成交的括号单。                         |
| **操作**   | 入场单成交；验证关联的 TP 与 SL 订单激活。                     |
| **事件序列** | 入场：`OrderFilled`；TP 与 SL 从关联（contingent）转为激活。 |
| **通过标准** | 入场成交后，TP 与 SL 订单在交易场所上生效。                     |
| **跳过条件** | 适配器不支持括号单。                                    |

**注意事项：**

- 这需要入场单确实成交，可能需要激进的定价。
- TP/SL 的激活机制因交易场所而异（有些立即激活，有些是 OCA 组）。

### TC-E53: 带 post-only 入场的括号单

| 字段       | 值                                                |
| -------- | ------------------------------------------------ |
| **前提条件** | 适配器支持括号单和 post-only。                             |
| **操作**   | 以 `use_post_only=True` 提交括号单（应用于入场和 TP）。         |
| **事件序列** | 与 TC-E50 相同，入场带 post-only 标志。                    |
| **通过标准** | 入场和 TP 订单作为 post-only（maker）被接受；SL 不是 post-only。 |
| **跳过条件** | 不支持括号单或不支持 post-only。                            |

---

## 第 7 组：订单标志 (Group 7: Order flags)

测试订单级别的标志和特殊参数。

| TC     | 名称             | 描述                           | 跳过条件             |
| ------ | -------------- | ---------------------------- | ---------------- |
| TC-E60 | PostOnly 被接受   | 下在远离 TOB 处的 post-only 限价单。   | 不支持 post-only。   |
| TC-E61 | 平仓时 ReduceOnly | 用 reduce-only 标志平仓。          | 不支持 reduce-only。 |
| TC-E62 | 展示数量           | 可见数量 < 总量的冰山单。               | 不支持展示数量。         |
| TC-E63 | 自定义订单参数        | 通过 `order_params` 传入适配器特有参数。 | 不适用。             |

### TC-E60: PostOnly 被接受

| 字段       | 值                                                          |
| -------- | ---------------------------------------------------------- |
| **前提条件** | 适配器已连接，合约已加载，报价流通。                                         |
| **操作**   | ExecTester 在被动价格下一个 `use_post_only=True` 的限价买单。            |
| **事件序列** | `OrderInitialized` -> `OrderSubmitted` -> `OrderAccepted`。 |
| **通过标准** | 订单作为 maker 单被接受；post-only 标志被交易场所确认。                       |
| **跳过条件** | 适配器不支持 post-only 标志。                                       |

**Python 配置：**

```python
ExecTesterConfig(
    instrument_id=instrument_id,
    order_qty=Decimal("0.01"),
    enable_limit_buys=True,
    enable_limit_sells=False,
    use_post_only=True,
)
```

**Rust 配置：**

```rust
ExecTesterConfig::new(strategy_id, instrument_id, client_id, Quantity::from("0.01"))
    .with_enable_limit_buys(true)
    .with_enable_limit_sells(false)
    .with_use_post_only(true)
```

### TC-E61: 平仓时 ReduceOnly

| 字段       | 值                                                                                          |
| -------- | ------------------------------------------------------------------------------------------ |
| **前提条件** | 未平仓位（来自 TC-E01）。                                                                           |
| **操作**   | 以 `reduce_only_on_stop=True` 停止策略；平仓单使用 reduce-only 标志。                                    |
| **事件序列** | `OrderInitialized` -> `OrderSubmitted` -> `OrderAccepted` -> `OrderFilled`（带 reduce-only）。 |
| **通过标准** | 平仓单带有 reduce-only 标志；仓位完全平掉。                                                               |
| **跳过条件** | 适配器不支持 reduce-only 标志。                                                                     |

**Python 配置：**

```python
ExecTesterConfig(
    instrument_id=instrument_id,
    order_qty=Decimal("0.01"),
    open_position_on_start_qty=Decimal("0.01"),
    reduce_only_on_stop=True,
    close_positions_on_stop=True,
    enable_limit_buys=False,
    enable_limit_sells=False,
)
```

**Rust 配置：**

```rust
ExecTesterConfig::new(strategy_id, instrument_id, client_id, Quantity::from("0.01"))
    .with_open_position_on_start(Some(Decimal::new(1, 2)))
    .with_reduce_only_on_stop(true)
    .with_close_positions_on_stop(true)
    .with_enable_limit_buys(false)
    .with_enable_limit_sells(false)
```

### TC-E62: 展示数量（冰山单）

| 字段       | 值                                                          |
| -------- | ---------------------------------------------------------- |
| **前提条件** | 适配器已连接，适配器支持展示数量。                                          |
| **操作**   | 下一个 `order_display_qty` < `order_qty` 的限价单。                |
| **事件序列** | `OrderInitialized` -> `OrderSubmitted` -> `OrderAccepted`。 |
| **通过标准** | 订单以设置了展示数量被接受；盘口上仅可见展示数量。                                  |
| **跳过条件** | 适配器不支持展示数量 / 冰山单。                                          |

**Python 配置：**

```python
ExecTesterConfig(
    instrument_id=instrument_id,
    order_qty=Decimal("1.0"),
    order_display_qty=Decimal("0.1"),
    enable_limit_buys=True,
    enable_limit_sells=False,
)
```

**Rust 配置：**

```rust
let mut config = ExecTesterConfig::new(strategy_id, instrument_id, client_id, Quantity::from("1.0"))
    .with_enable_limit_buys(true)
    .with_enable_limit_sells(false);
config.order_display_qty = Some(Quantity::from("0.1"));
```

### TC-E63: 自定义订单参数

| 字段       | 值                                                          |
| -------- | ---------------------------------------------------------- |
| **前提条件** | 适配器已连接，适配器接受附加参数。                                          |
| **操作**   | 下一个带 `order_params` 字典（含适配器特有参数）的订单。                       |
| **事件序列** | `OrderInitialized` -> `OrderSubmitted` -> `OrderAccepted`。 |
| **通过标准** | 订单被接受；适配器特有参数被透传到交易场所。                                     |
| **跳过条件** | 不适用（适配器特有）。                                                |

**注意事项：**

- `order_params` 字典对 ExecTester 是不透明的，会被透传给适配器。
- 关于支持的参数，请查阅适配器指南。

---

## 第 8 组：拒单处理 (Group 8: Rejection handling)

测试适配器正确处理并报告订单拒绝。

| TC     | 名称            | 描述                         | 跳过条件             |
| ------ | ------------- | -------------------------- | ---------------- |
| TC-E70 | PostOnly 拒绝   | 会穿越价差的 post-only 订单。       | 不支持 post-only。   |
| TC-E71 | ReduceOnly 拒绝 | 没有可减仓位的 reduce-only 订单。    | 不支持 reduce-only。 |
| TC-E72 | 不支持的订单类型      | 提交适配器不支持的订单类型。             | 从不跳过。            |
| TC-E73 | 不支持的 TIF      | 提交带不支持的 time in force 的订单。 | 从不跳过。            |
| TC-E74 | 提交模糊失败        | 提交时的传输、超时或发送失败。            | 无 mock 路径。       |
| TC-E75 | 取消模糊失败        | 取消时的传输、超时或发送失败。            | 无取消。             |
| TC-E76 | 修改模糊失败        | 修改时的传输、超时或发送失败。            | 无修改。             |
| TC-E77 | 批量模糊失败        | 没有逐单结果的整批失败。               | 无批量。             |
| TC-E78 | 逐单批量拒绝        | 批量响应中带有显式的逐单拒绝。            | 无批量。             |

TC-E74 至 TC-E78 在下面被集中规定，因为它们通常需要一个 mock 的 HTTP 或
WebSocket 边界，而非真实的交易场所。

### 模糊结果失败

这些用例证明：当交易场所的结果未知时，适配器的请求失败不会转化为终态拒绝事件。
通过标准还定义了本地准备失败（local prepare-failure）的豁免情形：当一条命令已知
尚未发送，且可归因于单个 cancel 或 modify 命令时，适配器可以发出对应的拒绝事件。

**通过标准：**

- 来自传输错误、超时、WebSocket 发送失败、重试耗尽或响应解析失败的提交失败，
  不应发出 `OrderRejected`。
- 来自传输错误、超时、WebSocket 发送失败、重试耗尽或整请求服务端失败的取消失败，
  不应发出 `OrderCancelRejected`。
- 来自传输错误、超时、WebSocket 发送失败、重试耗尽或整请求服务端失败的修改失败，
  不应发出 `OrderModifyRejected`。
- 当适配器能将失败归因于单个 cancel 命令时，证明该命令无法被发送的本地取消准备
  失败可以发出 `OrderCancelRejected`。
- 当适配器能将失败归因于单个 modify 命令时，证明该命令无法被发送的本地修改准备
  失败可以发出 `OrderModifyRejected`。
- 当交易场所没有返回逐单结果时，整批请求失败不应每个订单发一条拒绝。
- 显式的逐单交易场所拒绝仍应发出对应的拒绝事件，并带有交易场所的原因。

订单将保持在相应的在途（in-flight）状态，直到交易场所更新、查询结果或对账过程
将其解决。

### TC-E70: PostOnly 拒绝

| 字段       | 值                                                                        |
| -------- | ------------------------------------------------------------------------ |
| **前提条件** | 适配器已连接，合约已加载，报价流通。                                                       |
| **操作**   | ExecTester 将 post-only 订单下在盘口错误的一侧（`test_reject_post_only=True`），使其穿越价差。 |
| **事件序列** | `OrderInitialized` -> `OrderSubmitted` -> `OrderRejected`。               |
| **通过标准** | 交易场所拒绝订单；`OrderRejected.due_post_only=true`；原因点明 post-only 违规。           |
| **跳过条件** | 适配器不支持 post-only 标志。                                                     |

**注意事项：**

- ExecTester 的 `test_reject_post_only` 模式会故意将订单定价为穿越价差。
- 一些交易场所可能部分成交而非拒绝；行为因场所而异。
- 对 post-only 穿越拒绝发出 `OrderRejected` 的适配器，应设置
  `due_post_only=true`，以便策略将其与其他交易场所拒绝区分开。

**Python 配置：**

```python
ExecTesterConfig(
    instrument_id=instrument_id,
    order_qty=Decimal("0.01"),
    enable_limit_buys=True,
    enable_limit_sells=False,
    use_post_only=True,
    test_reject_post_only=True,
)
```

**Rust 配置：**

```rust
ExecTesterConfig::new(strategy_id, instrument_id, client_id, Quantity::from("0.01"))
    .with_enable_limit_buys(true)
    .with_enable_limit_sells(false)
    .with_use_post_only(true)
    .with_test_reject_post_only(true)
```

### TC-E71: ReduceOnly 拒绝

| 字段       | 值                                                                                                                   |
| -------- | ------------------------------------------------------------------------------------------------------------------- |
| **前提条件** | 适配器已连接，该合约无未平仓位。                                                                                                    |
| **操作**   | 当不存在可减仓位时，ExecTester 通过 `test_reject_reduce_only=True` 和 `open_position_on_start_qty`，以 `reduce_only=True` 开一个市价仓位。 |
| **事件序列** | `OrderInitialized` -> `OrderSubmitted` -> `OrderRejected`。                                                          |
| **通过标准** | 订单被拒绝；`OrderRejected` 事件的原因表明 reduce-only 违规。                                                                       |
| **跳过条件** | 适配器不支持 reduce-only 标志。                                                                                              |

**注意事项：**

- `test_reject_reduce_only` 标志仅应用于通过 `open_position_on_start_qty` 提交的
  开仓市价单。
- 运行该测试前，验证该合约不存在先前的仓位。

**Python 配置：**

```python
ExecTesterConfig(
    instrument_id=instrument_id,
    order_qty=Decimal("0.01"),
    open_position_on_start_qty=Decimal("0.01"),
    test_reject_reduce_only=True,
    enable_limit_buys=False,
    enable_limit_sells=False,
)
```

**Rust 配置：**

```rust
ExecTesterConfig::new(strategy_id, instrument_id, client_id, Quantity::from("0.01"))
    .with_open_position_on_start(Some(Decimal::new(1, 2)))
    .with_test_reject_reduce_only(true)
    .with_enable_limit_buys(false)
    .with_enable_limit_sells(false)
```

### TC-E72: 不支持的订单类型

| 字段       | 值                                    |
| -------- | ------------------------------------ |
| **前提条件** | 适配器已连接，订单类型不在适配器支持集合内。               |
| **操作**   | 提交一个适配器不支持的订单类型。                     |
| **事件序列** | `OrderDenied`（适配器在提交前拒绝）。            |
| **通过标准** | 订单在到达交易场所前被拒绝；带原因的 `OrderDenied` 事件。 |
| **跳过条件** | 从不跳过（每个适配器都有可供测试的不支持订单类型）。           |

**注意事项：**

- `OrderDenied` 发生在适配器层面，在订单到达交易场所之前。
- 这与来自交易场所的 `OrderRejected` 不同。
- 通过配置一个适配器不支持的止损单类型来测试。

### TC-E73: 不支持的 TIF

| 字段       | 值                                    |
| -------- | ------------------------------------ |
| **前提条件** | 适配器已连接，TIF 不在适配器支持集合内。               |
| **操作**   | 提交一个带适配器不支持的 TIF 的订单。                |
| **事件序列** | `OrderDenied`（适配器在提交前拒绝）。            |
| **通过标准** | 订单在到达交易场所前被拒绝；带原因的 `OrderDenied` 事件。 |
| **跳过条件** | 从不跳过（每个适配器都有可供测试的不支持 TIF 选项）。        |

**注意事项：**

- 与 TC-E72 类似，但针对 time-in-force 选项。
- 使用 Nautilus 枚举中适配器不映射的 TIF 值进行测试。

---

## 第 9 组：生命周期（启动/停止） (Group 9: Lifecycle (start/stop))

测试策略在启动和停止时的生命周期行为及状态管理。

| TC     | 名称       | 描述                | 跳过条件    |
| ------ | -------- | ----------------- | ------- |
| TC-E80 | 启动时开仓    | 策略启动时立即开仓。        | 不支持市价单。 |
| TC-E81 | 停止时取消订单  | 策略停止时取消所有未成交订单。   | 从不跳过。   |
| TC-E82 | 停止时平仓    | 策略停止时平掉未平仓位。      | 不支持市价单。 |
| TC-E83 | 停止时退订    | 策略停止时退订数据源。       | 不支持退订。  |
| TC-E84 | 对账未成交订单  | 对账来自先前会话的现有未成交订单。 | 从不跳过。   |
| TC-E85 | 对账已成交订单  | 对账来自先前会话的已成交订单。   | 从不跳过。   |
| TC-E86 | 对账未平多头仓位 | 对账现有的未平多头仓位。      | 从不跳过。   |
| TC-E87 | 对账未平空头仓位 | 对账现有的未平空头仓位。      | 从不跳过。   |

### TC-E80: 启动时开仓

| 字段       | 值                                                                           |
| -------- | --------------------------------------------------------------------------- |
| **前提条件** | 适配器已连接，合约已加载，无现有仓位。                                                         |
| **操作**   | 策略以设置好的 `open_position_on_start_qty` 启动。                                    |
| **事件序列** | `OrderInitialized` -> `OrderSubmitted` -> `OrderAccepted` -> `OrderFilled`。 |
| **通过标准** | 启动时开仓；在限价单维护开始之前，市价单已被提交并成交。                                                |
| **跳过条件** | 适配器不支持市价单。                                                                  |

**Python 配置：**

```python
ExecTesterConfig(
    instrument_id=instrument_id,
    order_qty=Decimal("0.01"),
    open_position_on_start_qty=Decimal("0.01"),
)
```

**Rust 配置：**

```rust
ExecTesterConfig::new(strategy_id, instrument_id, client_id, Quantity::from("0.01"))
    .with_open_position_on_start(Some(Decimal::new(1, 2)))
```

### TC-E81: 停止时取消订单

| 字段       | 值                                                 |
| -------- | ------------------------------------------------- |
| **前提条件** | 策略会话中产生的未成交限价单。                                   |
| **操作**   | 以 `cancel_orders_on_stop=True`（默认）停止策略。           |
| **事件序列** | 对每个未成交订单：`OrderPendingCancel` -> `OrderCanceled`。 |
| **通过标准** | 所有策略持有的未成交订单在停止时被取消。                              |
| **跳过条件** | 从不跳过。                                             |

### TC-E82: 停止时平仓

| 字段       | 值                                                                               |
| -------- | ------------------------------------------------------------------------------- |
| **前提条件** | 策略会话中产生的未平仓位。                                                                   |
| **操作**   | 以 `close_positions_on_stop=True`（默认）停止策略。                                       |
| **事件序列** | 平仓单：`OrderInitialized` -> `OrderSubmitted` -> `OrderAccepted` -> `OrderFilled`。 |
| **通过标准** | 所有策略持有的仓位被平掉；净仓位 = 0。                                                           |
| **跳过条件** | 适配器不支持市价单。                                                                      |

### TC-E83: 停止时退订

| 字段       | 值                                 |
| -------- | --------------------------------- |
| **前提条件** | 活跃的数据订阅（报价、成交、盘口）。                |
| **操作**   | 以 `can_unsubscribe=True`（默认）停止策略。 |
| **事件序列** | 数据订阅被移除。                          |
| **通过标准** | 停止后不再收到数据事件；干净断连。                 |
| **跳过条件** | 适配器不支持退订。                         |

**Python 配置：**

```python
ExecTesterConfig(
    instrument_id=instrument_id,
    order_qty=Decimal("0.01"),
    can_unsubscribe=True,
)
```

**Rust 配置：**

```rust
ExecTesterConfig::new(strategy_id, instrument_id, client_id, Quantity::from("0.01"))
    .with_can_unsubscribe(true)
```

### TC-E84: 对账未成交订单

| 字段       | 值                                                                      |
| -------- | ---------------------------------------------------------------------- |
| **前提条件** | 交易场所上有一个或多个来自先前会话的未成交限价单。                                              |
| **操作**   | 以 `reconciliation=True` 启动节点。                                          |
| **事件序列** | 为每个未成交订单生成 `OrderStatusReport`。                                        |
| **通过标准** | 每个未成交订单都被加载进缓存，且 `venue_order_id`、status=ACCEPTED、价格、数量、side 和订单类型均正确。 |
| **跳过条件** | 从不跳过。                                                                  |

**注意事项：**

- 从先前的测试会话保留未成交的限价单（停止时不取消）。
- 使用 `external_order_claims` 认领该合约，以便适配器为其对账订单。
- 验证对账到的订单数量与交易场所报告的数量相符。

### TC-E85: 对账已成交订单

| 字段       | 值                                                                |
| -------- | ---------------------------------------------------------------- |
| **前提条件** | 交易场所上有一个或多个来自先前会话的已成交订单。                                         |
| **操作**   | 以 `reconciliation=True` 启动节点。                                    |
| **事件序列** | 为每笔历史成交生成 `FillReport`。                                          |
| **通过标准** | 每个已成交订单都被加载进缓存，且 `venue_order_id`、status=FILLED、成交价、成交数量和手续费均正确。 |
| **跳过条件** | 从不跳过。                                                            |

**注意事项：**

- 需要在先前会话中成交过的订单。
- 验证成交价、数量和手续费与交易场所报告的值相符。
- 一些适配器可能只报告回溯窗口内的成交。

### TC-E86: 对账未平多头仓位

| 字段       | 值                                      |
| -------- | -------------------------------------- |
| **前提条件** | 交易场所上有一个来自先前会话的未平多头仓位。                 |
| **操作**   | 以 `reconciliation=True` 启动节点。          |
| **事件序列** | 为该多头仓位生成 `PositionStatusReport`。       |
| **通过标准** | 仓位被加载进缓存，且合约、side=LONG、数量和入场价均与交易场所相符。 |
| **跳过条件** | 从不跳过。                                  |

**注意事项：**

- 在先前会话中开一个多头仓位，并在不平仓的情况下停止策略
  （`close_positions_on_stop=False`）。
- 验证对账到的仓位数量和平均入场价与交易场所相符。
- 对账后，策略应能够管理或平掉该仓位。

### TC-E87: 对账未平空头仓位

| 字段       | 值                                       |
| -------- | --------------------------------------- |
| **前提条件** | 交易场所上有一个来自先前会话的未平空头仓位。                  |
| **操作**   | 以 `reconciliation=True` 启动节点。           |
| **事件序列** | 为该空头仓位生成 `PositionStatusReport`。        |
| **通过标准** | 仓位被加载进缓存，且合约、side=SHORT、数量和入场价均与交易场所相符。 |
| **跳过条件** | 从不跳过。                                   |

**注意事项：**

- 在先前会话中开一个空头仓位，并在不平仓的情况下停止策略
  （`close_positions_on_stop=False`）。
- 验证对账到的仓位数量和平均入场价与交易场所相符。
- 对账后，策略应能够管理或平掉该仓位。

---

## 第 10 组：期权交易 (Group 10: Options trading)

测试期权特有的执行行为。期权合约通常与线性衍生品有不同的约束：交易场所
可能限制订单类型、支持其他定价模式，或禁止条件单。具体限制因场所而异；
请查阅适配器指南。

这些测试需要一个 `CryptoOption` 合约。使用一个具有合理流动性的虚值（OTM）
期权来获得成交。

| TC      | 名称         | 描述                              | 跳过条件        |
| ------- | ---------- | ------------------------------- | ----------- |
| TC-E90  | 限价 BUY 期权  | 在期权合约上下一个限价买单。                  | 不支持期权。      |
| TC-E91  | 限价 SELL 期权 | 在期权合约上下一个限价卖单。                  | 不支持期权。      |
| TC-E92  | 带替代定价的限价单  | 通过 `order_params` 以适配器特有定价下限价单。 | 不支持替代定价。    |
| TC-E94  | 不支持的订单类型被拒 | 提交一个适配器对期权拒绝的订单类型。              | 不支持期权。      |
| TC-E96  | 条件单被拒      | 在期权上提交一个止损/条件单；预期被拒。            | 不支持期权。      |
| TC-E99  | FOK 限价期权   | 在期权合约上下一个 FOK 限价单。              | 不支持 FOK 期权。 |
| TC-E100 | 取消期权订单     | 取消期权合约上一个未成交的限价单。               | 不支持期权。      |
| TC-E101 | 对账期权仓位     | 对账来自先前会话的未平期权仓位。                | 不支持期权。      |

### TC-E90: 限价 BUY 期权

| 字段       | 值                                                          |
| -------- | ---------------------------------------------------------- |
| **前提条件** | 适配器已连接，期权合约已加载，报价流通。                                       |
| **操作**   | ExecTester 在期权上以被动价格下一个限价买单。                               |
| **事件序列** | `OrderInitialized` -> `OrderSubmitted` -> `OrderAccepted`。 |
| **通过标准** | 订单被交易场所接受，且合约、side、价格和数量均正确。                               |
| **跳过条件** | 适配器不支持期权交易。                                                |

**Python 配置：**

```python
ExecTesterConfig(
    instrument_id=instrument_id,  # CryptoOption instrument
    order_qty=Decimal("1"),
    enable_limit_buys=True,
    enable_limit_sells=False,
    tob_offset_ticks=500,
)
```

### TC-E91: 限价 SELL 期权

| 字段       | 值                                                          |
| -------- | ---------------------------------------------------------- |
| **前提条件** | 适配器已连接，期权合约已加载，报价流通。                                       |
| **操作**   | ExecTester 在期权上以被动价格下一个限价卖单。                               |
| **事件序列** | `OrderInitialized` -> `OrderSubmitted` -> `OrderAccepted`。 |
| **通过标准** | 订单被交易场所接受，且合约、side、价格和数量均正确。                               |
| **跳过条件** | 适配器不支持期权交易。                                                |

**Python 配置：**

```python
ExecTesterConfig(
    instrument_id=instrument_id,  # CryptoOption instrument
    order_qty=Decimal("1"),
    enable_limit_buys=False,
    enable_limit_sells=True,
    tob_offset_ticks=500,
)
```

### TC-E92: 带替代定价的限价单

| 字段       | 值                                                          |
| -------- | ---------------------------------------------------------- |
| **前提条件** | 适配器已连接，期权合约已加载。                                            |
| **操作**   | 通过 `order_params` 以适配器特有定价下一个限价单。                          |
| **事件序列** | `OrderInitialized` -> `OrderSubmitted` -> `OrderAccepted`。 |
| **通过标准** | 订单被接受；交易场所确认该替代定价模式。                                       |
| **跳过条件** | 适配器不支持期权的替代定价模式。                                           |

**注意事项：**

- 当替代定价生效时，订单对象上的 `price` 字段可能只是占位符。关于支持的
  参数键，请查阅适配器指南。
- 示例：OKX 支持 `px_usd`（USD 价格）和 `px_vol`（隐含波动率）。
- 在交易场所响应中验证定价模式被正确反映。

**Python 配置：**

```python
ExecTesterConfig(
    instrument_id=instrument_id,  # CryptoOption instrument
    order_qty=Decimal("1"),
    enable_limit_buys=True,
    enable_limit_sells=False,
    order_params={"px_usd": "100.5"},  # Adapter-specific pricing key
)
```

### TC-E94: 期权的不支持订单类型被拒

| 字段       | 值                                                                    |
| -------- | -------------------------------------------------------------------- |
| **前提条件** | 适配器已连接，期权合约已加载。                                                      |
| **操作**   | 提交一个交易场所对期权不支持的订单类型（例如市价单）。                                          |
| **事件序列** | 取决于适配器：`OrderDenied`（提交前）或 `OrderSubmitted` -> `OrderRejected`（提交后）。 |
| **通过标准** | 订单不成交。拒绝或被拒原因引用了该不支持的订单类型。                                           |
| **跳过条件** | 适配器不支持期权。                                                            |

**注意事项：**

- 确切的拒绝点因适配器而异。一些适配器在提交前本地拒绝；另一些则提交并
  转达交易场所的拒绝。
- ExecTester 可在期权合约上通过 `open_position_on_start_qty` 触发市价单。
  一些不支持的类型（例如 `MarketToLimit`）需要手动或编程方式提交。
- 测试适配器记录的每一种不支持的类型。

### TC-E96: 期权的条件单被拒

| 字段       | 值                                                                    |
| -------- | -------------------------------------------------------------------- |
| **前提条件** | 适配器已连接，期权合约已加载。                                                      |
| **操作**   | 在期权合约上提交一个条件单。                                                       |
| **事件序列** | 取决于适配器：`OrderDenied`（提交前）或 `OrderSubmitted` -> `OrderRejected`（提交后）。 |
| **通过标准** | 订单不成交。原因引用了不支持的条件单类型。                                                |
| **跳过条件** | 适配器不支持期权，或适配器为期权支持条件单。                                               |

**注意事项：**

- 测试适配器记录为期权不支持的每一种条件单类型
  （例如 `STOP_MARKET`、`STOP_LIMIT`、`MARKET_IF_TOUCHED`、`LIMIT_IF_TOUCHED`、
  `TRAILING_STOP_MARKET`）。
- ExecTester 可在期权合约上通过 `enable_stop_buys`/`enable_stop_sells` 配合
  `stop_order_type` 触发条件单。

### TC-E99: FOK 限价期权

| 字段       | 值                                                                                             |
| -------- | --------------------------------------------------------------------------------------------- |
| **前提条件** | 适配器已连接，期权合约已加载，盘口深度充足。                                                                        |
| **操作**   | 在期权合约上以 `TimeInForce::Fok` 下一个限价单。                                                            |
| **事件序列** | `OrderInitialized` -> `OrderSubmitted` -> `OrderAccepted` -> `OrderFilled` 或 `OrderCanceled`。 |
| **通过标准** | 订单完全成交或被取消。无部分成交。                                                                             |
| **跳过条件** | 适配器不支持期权的 FOK。                                                                                |

**注意事项：**

- 一些交易场所为期权 FOK 订单使用专门的订单类型（例如 OKX 使用 `op_fok`）。
  适配器会透明地处理该映射。
- 对于正向用例，使用较小的数量和激进的定价来获得成交。

### TC-E100: 取消期权订单

| 字段       | 值                                        |
| -------- | ---------------------------------------- |
| **前提条件** | 来自 TC-E90 或 TC-E91 的未成交限价单。              |
| **操作**   | 取消该未成交的限价单。                              |
| **事件序列** | `OrderPendingCancel` -> `OrderCanceled`。 |
| **通过标准** | 订单被取消；不再出现在交易场所的未成交订单中。                  |
| **跳过条件** | 适配器不支持期权。                                |

### TC-E101: 对账期权仓位

| 字段       | 值                                |
| -------- | -------------------------------- |
| **前提条件** | 来自先前会话的未平期权仓位。                   |
| **操作**   | 以 `reconciliation=True` 启动节点。    |
| **事件序列** | 为该期权仓位生成 `PositionStatusReport`。 |
| **通过标准** | 仓位被加载进缓存，且合约、side、数量和入场价均正确。     |
| **跳过条件** | 适配器不支持期权。                        |

**注意事项：**

- 在先前会话中开一个期权仓位，并在不平仓的情况下停止
  （`close_positions_on_stop=False`）。
- 验证对账到的仓位与交易场所报告的状态相符。

---

## ExecTester 配置参考 (ExecTester configuration reference)

所有 `ExecTesterConfig` 参数的快速参考。所示默认值针对 Python 配置；
Rust builder 使用等价的默认值。

| 参数                                              | 类型           | 默认值         | 影响的组       |
| ----------------------------------------------- | ------------ | ----------- | ---------- |
| `instrument_id`                                 | InstrumentId | *必填*        | 全部         |
| `order_qty`                                     | Decimal      | *必填*        | 全部         |
| `order_display_qty`                             | Decimal?     | None        | 2, 7       |
| `order_expire_time_delta_mins`                  | PositiveInt? | None        | 2          |
| `order_params`                                  | dict?        | None        | 7, 10      |
| `client_id`                                     | ClientId?    | None        | 全部         |
| `subscribe_quotes`                              | bool         | True        |            |
| `subscribe_trades`                              | bool         | True        |            |
| `subscribe_book`                                | bool         | False       |            |
| `book_type`                                     | BookType     | L2_MBP      |            |
| `book_depth`                                    | PositiveInt? | None        |            |
| `book_interval_ms`                              | PositiveInt  | 1000        |            |
| `book_levels_to_print`                          | PositiveInt  | 10          |            |
| `open_position_on_start_qty`                    | Decimal?     | None        | 1, 9       |
| `open_position_time_in_force`                   | TimeInForce  | GTC         | 1          |
| `enable_limit_buys`                             | bool         | True        | 2, 4, 5, 6 |
| `enable_limit_sells`                            | bool         | True        | 2, 4, 5, 6 |
| `enable_stop_buys`                              | bool         | False       | 3, 4       |
| `enable_stop_sells`                             | bool         | False       | 3, 4       |
| `limit_time_in_force`                           | TimeInForce? | None        | 2, 6       |
| `tob_offset_ticks`                              | PositiveInt  | 500         | 2, 4       |
| `stop_order_type`                               | OrderType    | STOP_MARKET | 3          |
| `stop_offset_ticks`                             | PositiveInt  | 100         | 3          |
| `stop_limit_offset_ticks`                       | PositiveInt? | None        | 3          |
| `stop_time_in_force`                            | TimeInForce? | None        | 3          |
| `stop_trigger_type`                             | TriggerType? | None        | 3          |
| `enable_brackets`                               | bool         | False       | 6          |
| `bracket_entry_order_type`                      | OrderType    | LIMIT       | 6          |
| `bracket_offset_ticks`                          | PositiveInt  | 500         | 6          |
| `modify_orders_to_maintain_tob_offset`          | bool         | False       | 4          |
| `modify_stop_orders_to_maintain_offset`         | bool         | False       | 4          |
| `cancel_replace_orders_to_maintain_tob_offset`  | bool         | False       | 4          |
| `cancel_replace_stop_orders_to_maintain_offset` | bool         | False       | 4          |
| `use_post_only`                                 | bool         | False       | 2, 6, 7, 8 |
| `use_quote_quantity`                            | bool         | False       | 1, 7       |
| `emulation_trigger`                             | TriggerType? | None        | 2, 3       |
| `cancel_orders_on_stop`                         | bool         | True        | 5, 9       |
| `close_positions_on_stop`                       | bool         | True        | 9          |
| `close_positions_time_in_force`                 | TimeInForce? | None        | 9          |
| `reduce_only_on_stop`                           | bool         | True        | 7, 9       |
| `use_individual_cancels_on_stop`                | bool         | False       | 5          |
| `use_batch_cancel_on_stop`                      | bool         | False       | 5          |
| `dry_run`                                       | bool         | False       |            |
| `log_data`                                      | bool         | True        |            |
| `test_reject_post_only`                         | bool         | False       | 8          |
| `test_reject_reduce_only`                       | bool         | False       | 8          |
| `can_unsubscribe`                               | bool         | True        | 9          |
