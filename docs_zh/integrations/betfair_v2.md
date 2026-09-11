# Betfair v2

Betfair Rust adapter 正处于积极的功能对齐 (parity) 工作中。本页跟踪当前的 Rust 行为，
以及从 [Betfair](betfair.md) 稳定版指南切换过来的计划。

本页镜像了 [Betfair](betfair.md) 主要章节的顺序。当 Rust adapter 成为首选的 Betfair 路径时，
这个文件只需做少量编辑即可替换 `betfair.md`，而不必整篇重写。

## 适用范围 (Scope)

- 本页的真理源 (source of truth)：`crates/adapters/betfair`
- 当前的稳定版指南：[Betfair](betfair.md)
- 本页的目的：跟踪当前的 Rust 接口、已知差距以及切换路径

## 当前 Rust 状态

| 领域       | 当前 Rust 行为                                                                              | 与当前 `betfair.md` 的差异               | 切换工作                                |
| -------- | --------------------------------------------------------------------------------------- | ---------------------------------- | ----------------------------------- |
| 订单类型     | `MARKET` 仅支持 `AT_THE_CLOSE`；`LIMIT` 在收盘流程中支持 BSP。                                       | 稳定版指南在此领域仍是 Python 形态。             | 确定最终的 Betfair 市价单模型。                |
| 批量操作     | `SubmitOrderList` 和 `BatchCancelOrders` 已实现。                                            | 稳定版指南曾将这些标记为不支持。                   | 保留并推广。                              |
| 对账范围     | `reconcile_market_ids_only` 使用 `reconcile_market_ids`；否则回退到 `stream_market_ids_filter`。 | 稳定版指南称流过滤和对账是分开的。                  | 决定 Rust 是保留还是移除这种耦合。                |
| 全量镜像缓存检查 | Rust 在启动时以及每次流重连时使用 `generate_mass_status()`；没有 `check_cache_against_order_image`。      | 稳定版指南描述了 Python 的全量镜像缓存检查。         | 补齐对齐，或将 Rust 路径文档化为最终方案。            |
| 重连后暂停    | 在对账进行中时，`submit_order` 和 `submit_order_list` 会发出 `OrderDenied STREAM_RECONCILING`。      | Python 在重连期间保持交易。                  | 一旦 `betfair.md` 切换，将其推广为 Rust 默认行为。 |
| 外部订单过滤   | `ignore_external_orders` 仅跳过没有 `rfo` 的 OCM 更新。                                          | Python 还在全量镜像缓存检查期间使用它。            | 确定最终的过滤行为。                          |
| 配置接口     | 没有 `certs_dir`、没有 `instrument_config`，keep alive 固定，heartbeat 值为必填。                     | 稳定版指南仍记录了 Python 的配置接口。            | 决定是补齐对齐还是认可 Rust 接口。                |
| SSL 证书   | 流客户端目前硬编码 `certs_dir=None`。                                                             | 稳定版指南记录了证书配置和 `BETFAIR_CERTS_DIR`。 | 补齐支持，或从未来的指南中移除。                    |

## 订单能力 (Orders capability)

### 订单类型

| 订单类型                   | 是否支持 | 说明                                                        |
| ---------------------- | ---- | --------------------------------------------------------- |
| `MARKET`               | ✓*   | Rust 仅支持 `AT_THE_CLOSE`，它映射到 Betfair 的 `MARKET_ON_CLOSE`。 |
| `LIMIT`                | ✓    | Rust 支持常规限价单以及收盘 BSP 限价单。                                 |
| `STOP_MARKET`          | -    | 不支持。                                                      |
| `STOP_LIMIT`           | -    | 不支持。                                                      |
| `MARKET_IF_TOUCHED`    | -    | 不支持。                                                      |
| `LIMIT_IF_TOUCHED`     | -    | 不支持。                                                      |
| `TRAILING_STOP_MARKET` | -    | 不支持。                                                      |

### 有效期 (Time in force)

| 有效期            | 是否支持 | 说明                                                     |
| -------------- | ---- | ------------------------------------------------------ |
| `GTC`          | ✓    | 映射到 Betfair 的 `PERSIST`。                               |
| `DAY`          | ✓    | 映射到 Betfair 的 `LAPSE`。                                 |
| `FOK`          | ✓    | 映射到 Betfair 的 `FILL_OR_KILL`。                          |
| `IOC`          | ✓    | 映射到 `FILL_OR_KILL`，并设置 `min_fill_size=0`。              |
| `AT_THE_CLOSE` | ✓    | 用于 Betfair BSP 的 `LIMIT_ON_CLOSE` 和 `MARKET_ON_CLOSE`。 |

Rust 目前还接受 `AT_THE_OPEN` 模式下的 `LIMIT` 订单，并将其通过 Betfair 的
`LIMIT_ON_CLOSE` 指令路由。请将其视为当前行为，而非已定型的公开契约。

### 批量操作

| 操作   | 是否支持 | 说明                         |
| ---- | ---- | -------------------------- |
| 批量提交 | ✓    | 通过 `SubmitOrderList` 实现。   |
| 批量修改 | -    | 不支持。                       |
| 批量取消 | ✓    | 通过 `BatchCancelOrders` 实现。 |

## 执行控制流 (Execution control flow)

启动时：

1. 连接 HTTP 客户端并获取初始账户资金。
2. 从缓存的订单初始化 OCM 状态。
3. 连接 Betfair 执行流并订阅订单更新。
4. 从 `listCurrentOrders` 生成启动时的批量状态 (mass status)。
5. 将订单和成交报告对账 (reconcile) 进执行引擎。

在每次流重连时，相同的批量状态对账会在最近的时间窗口上运行，并且 adapter 会暂停
新的增加敞口的命令，直到对账派发完成。
参见 [重连后对账](#post-reconnect-reconciliation)。

当前 Rust 说明：

- `stream_market_ids_filter` 过滤实时 OCM 更新。
- `reconcile_market_ids_only=True` 使用显式的 `reconcile_market_ids`。
- 当 `reconcile_market_ids_only=False` 且 `reconcile_market_ids` 未设置时，Rust 目前
  在启动对账时回退到 `stream_market_ids_filter`。
- Rust 尚未实现 Python 的 `check_cache_against_order_image` 全量镜像缓存检查。
- `ignore_external_orders=True` 目前仅跳过没有 `rfo` 的 OCM 更新。

## 会话管理与重连 (Session management and reconnection)

Betfair 会话每 12-24 小时过期一次。Rust adapter 通过三种机制自动处理会话恢复：

| 机制             | 触发条件                         | 动作                                              |
| -------------- | ---------------------------- | ----------------------------------------------- |
| 周期性 keep‑alive | 每 10 小时。                     | 续期会话令牌，推送到所有流的 watch 通道。                        |
| keep‑alive 回退  | keep‑alive 返回 `LoginFailed`。 | 通过 `reconnect()` 完整重新登录，向流推送新令牌。                |
| 流重连            | 断线后收到 `Connection` 消息。       | 先尝试 keep‑alive，在 `LoginFailed` 时回退到重新登录，更新认证信息。 |

keep-alive 期间出现的瞬时错误（网络超时、5xx 响应）会被记录并跳过。现有的会话令牌
会被保留，下一个 keep-alive 间隔会重试。只有 `LoginFailed` 错误（会话过期）才会触发
完整的重新登录。

数据客户端和执行客户端运行完全相同的重连逻辑。每个客户端都会启动：

- 一个 **keep-alive 任务**，周期性刷新会话，并将更新后的认证字节推送到流的 watch 通道。
- 一个 **重连处理器 (reconnect handler)**，在流重连后监听 `Connection` 消息，刷新会话，
  并推送新令牌。

流客户端将认证字节存储在 `tokio::sync::watch` 通道中。`post_reconnection` 闭包在每次
TCP 重连时从该通道读取，因此由 keep-alive 任务或重连处理器刷新的令牌会在下一次连接
尝试时被取用。

当 race 流处于活动状态时，数据客户端的重连处理器也会更新 race 流的认证信息。

## 重连后对账 (Post-reconnect reconciliation) {#post-reconnect-reconciliation}

当 Betfair 执行流重连时，adapter 假设在断线间隙期间缓存可能已经与场所 (venue) 状态
发生偏离（特别是，成交可能在重连后的流镜像到达之前就已完成并从未匹配账本上滚落）。
因此，在允许策略增加新敞口之前，它会在最近的时间窗口上运行一次批量状态对账。

| 步骤  | 触发条件                      | 动作                                                              |
| --- | ------------------------- | --------------------------------------------------------------- |
| 1   | 流断线后的第二条 `Connection` 消息。 | OCM 处理器置起 `pending_resync` 和 `is_reconciling`，向后台任务发送重连信号。      |
| 2   | 重连任务收到信号。                 | 重新置起 `is_reconciling`，使排队中的第二次重连在其自身迭代期间也会暂停。                   |
| 3   | 重连任务主体。                   | 刷新会话，更新流认证，获取 `getAccountFunds`，并调用 `listCurrentOrders` 取订单和成交。 |
| 4   | 批量状态构建完成。                 | 作为 `ExecutionReport::MassStatus` 派发，使引擎对账进缓存。                   |
| 5   | 迭代结束。                     | 清除 `is_reconciling`。失败的迭代也会清除它（fail‑open，与 Nautilus 其余部分一致）。    |

当 `is_reconciling` 被置起时：

- `submit_order` 和 `submit_order_list` 会发出 `OrderDenied`，原因为
  `STREAM_RECONCILING: post-reconnect reconciliation in progress, retry once it completes`。
- `cancel_order`、`batch_cancel_orders` 和 `modify_order` 原样通过，使策略在该窗口期间
  始终可以减少敞口。
- 在断线间隙期间到达的缓冲 OCM 会在下一个策略命令时通过 `process_pending_resync` 排空
  （使用独立的 `pending_resync` 标志）。

如果客户端在对账仍在进行中时断开连接，`clear_resync_state` 会清除 `is_reconciling`，
使后续的连接/提交周期能够干净地开始。

批量状态获取的回看窗口为 `stream_gap_recovery_lookback_mins`（默认 `10`）。它应当
充裕地超过预期最长的重连时长，使在断线间隙中完成的成交仍能被捕获。

## Tick 方案与定价 (Tick scheme and pricing)

Betfair 使用分层 tick 方案，不同价格区间的增量各不相同：

| 价格区间           | Tick 大小 |
| -------------- | ------- |
| 1.01 - 2.00    | 0.01    |
| 2.00 - 3.00    | 0.02    |
| 3.00 - 4.00    | 0.05    |
| 4.00 - 6.00    | 0.10    |
| 6.00 - 10.00   | 0.20    |
| 10.00 - 20.00  | 0.50    |
| 20.00 - 30.00  | 1.00    |
| 30.00 - 50.00  | 2.00    |
| 50.00 - 100.00 | 5.00    |
| 100.00 - 1000  | 10.00   |

最低价格为 1.01，最高价格为 1000.00。

## 订单修改 (Order modification)

- 价格和数量无法原子地同时更改；这些操作需要分开进行。
- 价格修改使用 `ReplaceOrders`（取消 + 以新价格下新单）。
- 数量减少使用带 `size_reduction` 参数的 `CancelOrders`。
- 不支持数量增加；请改为提交一个新订单。

一次替换 (replace) 操作会同时为原订单生成取消事件、为替换订单生成接受事件。adapter
跟踪待处理的替换以抑制合成的取消事件。

## 订单流成交处理 (Order stream fill handling)

执行客户端处理来自 Betfair Exchange Streaming API 的订单更新。有两个配置选项控制
更新如何被过滤：

- `stream_market_ids_filter`：在市场层级过滤（提前退出，静默跳过）。
- `ignore_external_orders`：在订单层级过滤（跳过没有 `rfo` 的 OCM 更新）。

### 成交处理

adapter 在处理来自流的成交时会处理若干边界情况：

- **增量成交**：Betfair 报告累计已匹配数量。adapter 通过跟踪每个订单上次已知的已成交
  数量来计算增量成交。
- **超额成交保护**：会超过订单数量的成交将被拒绝。
- **竞态条件**：当流成交在 HTTP 订单响应之前到达时，adapter 会立即缓存场所订单 ID，
  以确保正确的订单匹配。
- **网络错误恢复**：当 HTTP 订单提交因网络错误（超时、连接重置）失败时，订单可能仍已
  在场所上下单。adapter 会将订单保持在 SUBMITTED 状态，并保留客户订单引用，使流在
  重连时可以确认该订单。API 错误（Betfair 明确拒绝的情况）则立即拒绝。
- **间隙窗口成交**：在流断线期间完成并从未匹配账本上滚落的成交，会由重连后的批量状态
  对账恢复；参见 [重连后对账](#post-reconnect-reconciliation)。

## 限速 (Rate limiting)

adapter 使用独立的限速桶 (rate limit bucket)，使账户状态轮询和对账不会限制订单下单：

| 桶       | 默认值  | 端点                                            |
| ------- | ---- | --------------------------------------------- |
| General | 5/s  | 账户状态、对账、keep‑alive。                           |
| Orders  | 20/s | `placeOrders`、`replaceOrders`、`cancelOrders`。 |

订单状态和成交报告查询在遇到会话错误时，会在刷新会话后重试一次。`TOO_MANY_REQUESTS`
错误会在延迟 5 秒后重试。

## 市场版本价格保护 (Market version price protection)

当 `use_market_version=True` 时，每个订单请求都会包含 adapter 最后看到的市场版本。如果
在 Betfair 处理该订单时市场已超出该版本，Betfair 会让该投注失效 (lapse)，而不是将其
与已变更的账本进行匹配。

adapter 从 instrument 的 `info` 字典中读取市场版本，该字典由 Exchange Streaming API 的
`MarketDefinition` 更新填充。在收到第一个 `MarketDefinition` 之前提交的订单不会包含版本。

## 自定义数据类型 (Custom data types)

Rust adapter 通过市场流和 race 流发出与 Python adapter 相同的自定义数据类型。当订阅
市场时，所有自定义数据会自动流入。

| 类型                         | 流      | 描述                     |
| -------------------------- | ------ | ---------------------- |
| `BetfairTicker`            | Market | 最后成交价、成交量、BSP 指标。      |
| `BetfairStartingPrice`     | Market | 市场收盘后已实现的 BSP。         |
| `BetfairSequenceCompleted` | Market | 标记一个市场变更序列的结束。         |
| `BetfairOrderVoided`       | Order  | 作废订单的详情（作废数量、价格、方向）。   |
| `BetfairRaceRunnerData`    | Race   | 每个参赛者的实时 GPS 跟踪 (TPD)。 |
| `BetfairRaceProgress`      | Race   | 分段时间、跑位顺序、跨栏数据。        |

Race 数据需要 Total Performance Data (TPD) 覆盖，以及一个具有 TPD 访问权限的 Betfair
API key。通过 `subscribe_race_data=True` 启用。

## 多节点部署 (Multi-node deployment)

当多个交易节点跨不同市场共享单个 Betfair 账户时：

1. 将 `stream_market_ids_filter` 设置为仅包含该节点的市场。
2. 设置 `ignore_external_orders=True` 以抑制关于来自其他节点的订单的警告。
3. 设置 `reconcile_market_ids_only=True` 以限制对账范围。

## 当前 Rust 配置 (Current Rust configuration)

### 数据客户端配置

| 选项                                  | 默认值      | 说明                      |
| ----------------------------------- | -------- | ----------------------- |
| `account_currency`                  | 必填       | Betfair 账户币种。           |
| `username`                          | `None`   | 回退到 `BETFAIR_USERNAME`。 |
| `password`                          | `None`   | 回退到 `BETFAIR_PASSWORD`。 |
| `app_key`                           | `None`   | 回退到 `BETFAIR_APP_KEY`。  |
| `proxy_url`                         | `None`   | HTTP 请求的可选代理 URL。       |
| `request_rate_per_second`           | `5`      | General HTTP 限速。        |
| `default_min_notional`              | `None`   | 可选的最小名义金额覆盖。            |
| `event_type_ids`                    | `None`   | 可选的导航过滤器。               |
| `event_type_names`                  | `None`   | 可选的导航过滤器。               |
| `event_ids`                         | `None`   | 可选的导航过滤器。               |
| `country_codes`                     | `None`   | 可选的导航过滤器。               |
| `market_types`                      | `None`   | 可选的导航过滤器。               |
| `market_ids`                        | `None`   | 可选的导航过滤器。               |
| `min_market_start_time`             | `None`   | 可选的导航过滤器。               |
| `max_market_start_time`             | `None`   | 可选的导航过滤器。               |
| `stream_host`                       | `None`   | 可选的流主机覆盖。               |
| `stream_port`                       | `None`   | 可选的流端口覆盖。               |
| `stream_heartbeat_ms`               | `5,000`  | 目前在 Rust 中为必填。          |
| `stream_idle_timeout_ms`            | `60,000` | 重连前的空闲超时。               |
| `stream_reconnect_delay_initial_ms` | `2,000`  | 初始重连延迟。                 |
| `stream_reconnect_delay_max_ms`     | `30,000` | 最大重连延迟。                 |
| `stream_use_tls`                    | `True`   | 流连接使用 TLS。              |
| `stream_conflate_ms`                | `None`   | 显式的合并 (conflation) 设置。  |
| `subscription_delay_secs`           | `3`      | 第一次市场订阅前的延迟。            |
| `subscribe_race_data`               | `False`  | 订阅 RCM 更新。              |

Rust 尚未暴露 `certs_dir` 或 `instrument_config`。Rust 还使用固定的 36,000 秒
keep-alive 间隔。

### 执行客户端配置

| 选项                                  | 默认值           | 说明                                     |
| ----------------------------------- | ------------- | -------------------------------------- |
| `trader_id`                         | `TRADER-001`  | 客户端核心的 Trader ID。                      |
| `account_id`                        | `BETFAIR-001` | 客户端核心的 Account ID。                     |
| `account_currency`                  | `GBP`         | Betfair 账户币种。                          |
| `username`                          | `None`        | 回退到 `BETFAIR_USERNAME`。                |
| `password`                          | `None`        | 回退到 `BETFAIR_PASSWORD`。                |
| `app_key`                           | `None`        | 回退到 `BETFAIR_APP_KEY`。                 |
| `proxy_url`                         | `None`        | HTTP 请求的可选代理 URL。                      |
| `request_rate_per_second`           | `5`           | General HTTP 限速。                       |
| `order_request_rate_per_second`     | `20`          | 订单端点限速。                                |
| `stream_host`                       | `None`        | 可选的流主机覆盖。                              |
| `stream_port`                       | `None`        | 可选的流端口覆盖。                              |
| `stream_heartbeat_ms`               | `5,000`       | 目前在 Rust 中为必填。                         |
| `stream_idle_timeout_ms`            | `60,000`      | 重连前的空闲超时。                              |
| `stream_reconnect_delay_initial_ms` | `2,000`       | 初始重连延迟。                                |
| `stream_reconnect_delay_max_ms`     | `30,000`      | 最大重连延迟。                                |
| `stream_use_tls`                    | `True`        | 流连接使用 TLS。                             |
| `stream_market_ids_filter`          | `None`        | 可选的实时 OCM 市场过滤器。                       |
| `ignore_external_orders`            | `False`       | 仅跳过没有 `rfo` 的 OCM 更新。                  |
| `calculate_account_state`           | `True`        | 目前在 Rust 中控制周期性账户状态轮询的开关。              |
| `request_account_state_secs`        | `300`         | 账户资金的轮询间隔。                             |
| `reconcile_market_ids_only`         | `False`       | 当为 `True` 时，使用 `reconcile_market_ids`。 |
| `reconcile_market_ids`              | `None`        | 显式的启动对账市场 ID。                          |
| `use_market_version`                | `False`       | 为下单和替换请求附加市场版本。                        |
| `stream_gap_recovery_lookback_mins` | `10`          | 重连后批量状态对账的回看窗口。                        |

Rust 尚未暴露 `certs_dir` 或 `instrument_config`。

## 切换计划 (Cutover plan)

在 Rust adapter 成为首选的 Betfair 路径之前，将本页用作过渡跟踪器。

切换时：

1. 决定 Rust 是保留其当前的对账过滤行为，还是匹配 Python 的拆分方式。
2. 决定 Rust 是否添加证书配置以及其他 Python 配置字段。
3. 决定 Rust 是保留仅限 BSP 的 `MARKET` 订单，还是添加 Python 的激进限价 (aggressive-limit) 路径。
4. 将此文件提升为 `betfair.md`。
5. 将任何剩余的仅限 Python 的说明移入一份简短的遗留说明或发布说明。
