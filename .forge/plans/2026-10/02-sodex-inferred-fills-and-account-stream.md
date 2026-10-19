# 02 - SoDEX 成交以真实回报进入引擎：适配器主动送出真实成交、推断成交不再记错手续费、接上 accountUpdate

> **Status**: 定稿，实施待授权（经两轮独立评审；未实施）
> **Created**: 2026-10-19
> **Revised**: 2026-10-19（按独立评审的 1 条 HIGH、3 条 MEDIUM、6 条 LOW 修订，修订对照见文末「修订记录」）；同日第二次修订：Task 0 的范围扩大到源码测试与集成测试；同日第三次修订：按复核补上送出范围、送出条件、两种请求的区分与测试网细节，定稿
> **Project**: nautilus_trader (fork) — `crates/adapters/sodex`（Guild 自维护的场所适配器）
> **For Claude**: Use `/forge:execute` to implement this plan.

## 上下文 (Context)

本计划源于 2026-10-19 在 SoDEX 现货测试网上的一次实测。实测用的是 Guild 的测试网验证账户，引擎为 `guild-v2.0.0rc5+sodex.3`，即 `2d1e432cd1`。完整证据与分析在 Guild 内部的验证记录里，不随本仓库发布；下文引用的日志文件名、`post-trades.json` 都指那份证据。维护方当天决定在三处分别立项，本计划是落在 fork 的那一处。

**现象**（实证，日志行号指上述证据里的 runner 日志）：

- 挂单之后才在场所成交的限价单，运行中只由周期检查补出一条推断成交（`run2-main-runner.log:278`，`Generated inferred fill for dfa0b9e8…`；`:280`，`OrderFilled`）。
  - 成交号由引擎生成（`a21180f0-175c-51d4-8a8f-579e909f51e0`）。
  - 手续费记为 `0.0807738750000000 vUSDC`，等于名义额 × 0.00065。
- 场所上同一笔单是 2 笔成交：9461715 成交 0.00149，手续费 9.685E-7 vBTC；9461716 成交 0.00001，手续费 3.5E-9 vBTC（`post-trades.json`）。
- IOC 单拿到了真实成交：`run2-after-restart-runner.log:327` 为 `Found adcdf557… via targeted order status query: CANCELED`，`:329` 的成交号是 9461725，手续费以 vBTC 计，与场所一致。
- 引擎账户余额在一轮运行之内从不变化：`run2-main-runner.log` 里 27 次 `Updated AccountState`，vBTC 都是 `0.0000093500000000`。
- 下游后果：下游的事实签发对每条 `OrderFilled` 原样签发成交与手续费事实。补出的成交带着错误的成交号、笔数和手续费币种进入签名事实，与按场所成交采集的账本证据对不上。下游如何处理不在本计划内，由下游各自的计划承接。

**根因**（实证，均附 `文件:行号`；除注明外，`manager.rs` 指 `crates/live/src/execution/manager.rs`，`client.rs` 指 `crates/adapters/sodex/src/execution/client.rs`）：

1. **执行客户端没有推送通道，成交只能靠周期检查**。
   - 适配器只实现了行情频道（`crates/adapters/sodex/src/websocket/messages.rs:93`、`:119`、`:122`）。
   - 交易所**有** `accountUpdate` 频道，但订阅参数还没试出来：`crates/adapters/sodex/examples/probe_channels.rs:18-20` 原文为「`accountUpdate` exists but refused all eighteen parameter shapes tried as `invalid params`. Its selector is still unknown, which is why this adapter reports no fills」。
   - `crates/adapters/sodex/examples/probe_account_stream.rs` 已写好第二阶段（7 个字段的全部非空子集，共 127 次请求），尚无运行记录。
2. **周期检查对「已成交」的单只会补出推断成交，不会去取真实成交**。
   - 适配器的 `generate_order_status_reports`（`client.rs:770`）调用 `collect_order_reports`（`:1585`），它同时读挂单列表与历史（`:1586-1598`），已成交的单以 `FILLED` 状态报告返回；核心在 `manager.rs:4726-4738` 对 `PartiallyFilled | Filled` 且成交量增加的报告补出推断成交，成交号由 `create_inferred_reconciliation_trade_id`（`crates/execution/src/reconciliation/ids.rs:71`）生成。
   - 核心只对「已撤销」「已过期」且成交量增加的报告去取真实成交：`manager.rs:2361-2368` 把这类报告转为定向候选，`:5578` 判断后在 `:5591` 调 `generate_fill_reports`；判断函数在 `:5662-5667`，原文为 `matches!(report.order_status, OrderStatus::Canceled | OrderStatus::Expired) && report.filled_qty > filled_qty`。IOC 之所以拿到真实成交，是因为适配器交出了历史里「已撤销且有成交」的那份报告。
   - 在 `open_check_open_only=true` 下（下游配置如此），从挂单列表消失的订单只记一条 debug 日志（`manager.rs:2388-2417`，`:2392` 原文 `if self.config.open_check_open_only {`），不会入队定向查询；入队定向查询的 `prepare_missing_order_query`（`:3704`）与 `open_check_missing_retries`（`:3732`）只在另一分支起作用。所以「让适配器遵守 `open_only`」不能把成交改成真实回报，反而会让挂单后成交的订单在运行中一直收不到成交，IOC 的真实成交也会一起丢掉。
3. **推断成交的手续费口径错误**。
   - `client.rs:709` 的 `calculate_commission` 按名义额在计价币上计费，`:748-754` 原文为「Fees are charged on notional in the quote asset on both engines」。
   - 同一个适配器的 `crates/adapters/sodex/src/http/account_reads.rs:225-229` 记录了现货买单从**基础币**扣手续费。实测再次印证：买单手续费 = 成交量 × 0.00065，以 vBTC 计（9.685E-7 = 0.00149 × 0.00065）；一笔卖单的手续费 = 名义额 × 0.00065，以 vUSDC 计（0.00097 × 82816 × 0.00065 = 0.052215488）。
   - 接口 `calculate_commission`（`crates/common/src/clients/execution.rs:414`）的参数里没有订单方向，适配器无法据此选币种；按方向改写需要改上游核心，本计划不做。
   - 既有测试 `client.rs:2178-2183` 的 `commission_is_denominated_in_the_cost_currency` 钉住的正是旧口径（注释原文「Both engines charge on notional in the quote asset」）。
4. **运行中余额不更新**：账户是场所上报型（日志中 `is_reported=true`），引擎不按成交推算余额；适配器只在连接时（`client.rs:674`）和收到 `QueryAccount` 时（`:763`）发布余额。

**Step 1.5 契约核对**：

- 本计划**只改适配器**（`crates/adapters/sodex/`）和它的示例、测试；不改 `crates/common`、`crates/live`、`crates/execution` 里任何已有文件。若某项实证结论是「只改适配器做不到」，停下回报，不改上游核心。
- 对外契约是 NT 的 `FillReport`、`OrderStatusReport`、`AccountState`，字段不变。适配器送出报告用已有的 `ExecutionEventEmitter::send_fill_report`（`crates/live/src/execution/emitter.rs:533`）与 `send_order_with_fills`（`:538`）；引擎在运行中分别以 `reconcile_fill_report`、`reconcile_order_with_fills` 处理（`crates/execution/src/engine/mod.rs:1015-1020`）。适配器已持有一个发送器（`client.rs:118`）。

## 目标 (Goal)

SoDEX 现货订单的每一笔场所成交，都以场所的成交号、数量和手续费（含币种）进入引擎，运行中的账户余额随成交刷新。在做到之前，任何仍会走推断成交的路径不得记下币种错误的手续费：宁可推迟对账并报错，也不记错。

## 架构 (Architecture)

1. **适配器主动送出真实成交（Task 2）**：适配器在周期检查读到某张订单的成交量增加时，读取账户成交，把这张单的状态与真实成交一起用 `send_order_with_fills` 送进引擎；被送出的这份终态记录不再作为普通状态报告返回给周期检查，免得核心对它补出推断成交。送出有三个限定：**只送引擎缓存里存在、且缓存中的已成交量小于历史成交量的订单**（这也是「成交量增加」的基准，并消除每轮重复送出）；**只在这张单的账户成交之和等于历史成交量时送出**，否则本轮不送、也不交出终态记录；**只在周期检查时送出，启动时的全量对账保持原样**。送出成交之后，调用已有的 `publish_account_state`（`client.rs:1655`）刷新余额。`find_order_report`（`client.rs:1641-1652`，定向查询与「提交结果不明」的查找都靠它）保持读挂单与历史两份列表，不跟着改。
2. **推断成交不再记错手续费（Task 1）**：现货的推断成交，`calculate_commission` 返回错误。接口文档允许这样做（`crates/common/src/clients/execution.rs:405-408` 原文「An error means the venue formula applies but its result could not be represented, so callers must not substitute a zero or generic commission for it」）；核心随之推迟这张单的对账并记 error（`manager.rs:2383` 原文「Deferring reconciliation for {client_order_id}: venue commission calculation failed: {e}」，另见 `:4883`、`:5144`）。永续合约保持按名义额计。**Task 1 必须与 Task 2 一起上线**：只上 Task 1，挂单后才成交的订单会每轮报错推迟、一直停在待对账；两者同属一个发布单元。
3. **接上 accountUpdate（Task 3、Task 4）**：先跑探针查明订阅参数，再实现订阅；做成之后，成交与余额由推送实时到达，Task 2 的轮询成为兜底。

## 关键设计决策 (Key Design Decisions)

| 问题                  | 决策                                                       | 理由                                                                                                                                                                |
| ------------------- | -------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 真实成交由谁送进引擎          | 适配器在读到成交量增加时，用已有的发送器主动送出                                 | 核心的周期检查对 `FILLED` 只补推断成交（根因 2）；改核心违反「只改适配器」                                                                                                                       |
| 送哪些订单               | 只送引擎缓存里存在、缓存已成交量小于历史成交量的订单                               | 引擎不认识的订单经成交包送进去，会在 `crates/execution/src/engine/mod.rs:1554-1556` 被建成外部订单与持仓；周期检查对未知订单本来就跳过（`manager.rs:2338-2343`）                                               |
| 成交只到一部分时            | 不送，等账户成交之和等于历史成交量                                        | 成交包应用之后，引擎按状态报告补齐差额（`engine/mod.rs:1607-1612` → `crates/execution/src/reconciliation/orders.rs:77-84`，手续费为 `None`），补出的推断成交不经过 `calculate_commission`，Task 1 的保护失效 |
| 周期检查与全量对账怎么分        | 只在周期检查时送出；本设计只支持 `open_check_open_only=true`             | 全量对账的请求 `open_only` 固定为 `false`（`crates/common/src/clients/execution.rs:326-331`）；`open_only=false` 的部署无法只凭这个参数区分两者，处理见 Task 2 Step 0 第 7 条                       |
| 实施分支放在哪里            | 放在单独的 worktree 里，不在主检出上检出                                | 测试网验收 C 项授权推送实施分支之后，若它检出在主检出上，此后每次提交都会被每晚的自动推送推上去，绕过逐次授权                                                                                                          |
| 是否遵守 `open_only`    | 不以此为手段。被送出真实成交的那份终态记录不再作为状态报告返回；其余行为保持                   | 遵守 `open_only` 会让核心对缺失订单只记 debug，成交丢失（根因 2）                                                                                                                       |
| 现货推断成交的手续费          | 返回错误，推迟对账加报错；永续不变                                        | 接口拿不到方向，只改适配器时这是最接近正确的做法；维护方 2026-10-19 采纳「只改适配器：算手续费时报错，推迟对账」                                                                                                    |
| Task 1 与 Task 2 的关系 | 同一个发布单元，不单独发布 Task 1                                     | 只有 Task 1 时，挂单后成交的订单会一直停在待对账                                                                                                                                      |
| 运行中余额               | 送出真实成交后刷新一次余额；接上推送后由推送更新                                 | 不让余额目标押在一个尚未查明的订阅参数上                                                                                                                                              |
| 推送的订阅参数             | 先跑探针，查明之后再写解析代码；没有真实成交时的载荷就不写解析器                         | 照着推测写，曾是这个适配器最坏的一类故障（`probe_channels.rs`）                                                                                                                         |
| 测试网                 | 三项都列为待授权，各自写成可直接批准的请求                                    | 测试网用真实凭据与 Guild 的验证账户，每次单独授权                                                                                                                                      |
| 示例与测试里的账户号          | 示例改为必须由环境变量给出；源码测试与集成测试里的换成占位值；单独一个小提交，放在实施第一步；已推送的历史不改写 | 维护方 2026-10-19 采纳「当前文件里去掉，改读环境变量」与「一并换成中性占位值」                                                                                                                     |

## 承载决策 (Capability Hosting Decision)

| 能力                | plan mode? | hook? | CLAUDE.md? | 现有 skill flag? | 新 skill/command? | 决策               |
| ----------------- | ---------- | ----- | ---------- | -------------- | ---------------- | ---------------- |
| SoDEX 成交与账户状态的正确性 | 否          | 否     | 否          | 无              | 否                | 适配器代码 + 失败模式测试承载 |

## 文件清单 (File Inventory)

| 文件路径                                                                                                                                                                                                                                | 操作             | 描述                                                                     |
| ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------- | ---------------------------------------------------------------------- |
| `crates/adapters/sodex/examples/*.rs`、`examples/README.md`；`src/config.rs`、`src/execution/client.rs`、`src/execution/reports.rs`、`src/http/account.rs`、`src/http/requests.rs`、`src/http/spot.rs` 的测试模块；`tests/integration/python.rs` | Modify（Task 0） | 示例改读环境变量；测试数据里的账户号换成占位值 `12345`（位置清单见 Task 0）                          |
| `crates/adapters/sodex/src/execution/client.rs`                                                                                                                                                                                     | Modify         | `calculate_commission` 对现货返回错误；周期检查读到成交量增加时送出真实成交并刷新余额；推送订阅（Task 4）    |
| `crates/adapters/sodex/src/execution/client.rs` 中的既有测试 `commission_is_denominated_in_the_cost_currency`（`:2178-2183`）                                                                                                               | Modify         | 改写为新口径：现货返回错误、永续按名义额计                                                  |
| `crates/adapters/sodex/src/execution/reports.rs`                                                                                                                                                                                    | Modify（可能）     | 由历史记录与账户成交组装 `OrderStatusReport` 加 `FillReport`                        |
| `crates/adapters/sodex/src/websocket/messages.rs`、`websocket/client.rs`                                                                                                                                                             | Modify（Task 4） | `accountUpdate` 订阅与载荷类型                                                |
| `crates/adapters/sodex/examples/probe_account_stream.rs`                                                                                                                                                                            | 运行（Task 3）     | 第二阶段探测                                                                 |
| `crates/adapters/sodex/Cargo.toml`                                                                                                                                                                                                  | Modify         | 开发依赖加 `axum`（工作区已有，`Cargo.toml:364`），供假 HTTP 服务使用；其余所需依赖在 Step 0 实证后补齐 |
| `crates/adapters/sodex/tests/integration/periodic_check.rs`（新建）                                                                                                                                                                     | Create         | 驱动真实周期检查的端到端失败模式测试                                                     |
| `crates/adapters/sodex/tests/integration/main.rs`                                                                                                                                                                                   | Modify         | 挂上新测试模块                                                                |
| `.forge/plans/2026-10/02-sodex-inferred-fills-and-account-stream.md`                                                                                                                                                                | Modify         | 本计划                                                                    |
| `.forge/README.md`                                                                                                                                                                                                                  | Modify         | 计划索引与范围说明                                                              |

## 实现任务 (Tasks)

实施分支检出在单独的 worktree 里，不在主检出上（理由见「关键设计决策」最后一行）。

### Task 0: 示例、源码测试与集成测试都不再带真实账户号（单独一个小提交）

**Files**：当前含该账户号的 15 个示例文件，即 `crates/adapters/sodex/examples/` 下的 `README.md`、`cancel_open_orders.rs`、`observe_fill.rs`、`observe_position.rs`、`place_and_cancel.rs`、`place_modify_cancel.rs`、`probe_account_stream.rs`、`probe_channels.rs`（两处）、`probe_key_permissions.rs`、`probe_limit_quantity.rs`、`probe_rest_endpoints.rs`、`register_api_key.rs`、`revoke_api_key.rs`、`set_leverage.rs`、`verify_signing.rs`。

- 每处默认值改为必须由环境变量 `SODEX_ACCOUNT_ID` 给出，缺失时报错退出，并在报错里说明如何查到账户号（已有示例 `352dbf5c91` 加过查询账户号的办法）。
- README 改为说明这个环境变量，不写具体账户号。
- 已推送的历史不改写。
- **源码与集成测试里的同一数字**（维护方 2026-10-19 采纳「一并换成中性占位值」）：按 `git grep` 列出的全部位置如下，共 55 行。它们全部位于各文件的 `#[cfg(test)]` 测试模块或集成测试里，生产代码里没有；文档注释里也没有出现。
  - `crates/adapters/sodex/src/config.rs:406`、`:414`（配置序列化往返）；
  - `crates/adapters/sodex/src/execution/client.rs:1911`、`:1925`、`:1939`、`:1954`、`:1974`、`:1985`、`:2021`、`:2041`（下单、撤单请求组装）；
  - `crates/adapters/sodex/src/execution/reports.rs:467`、`:552`、`:712`、`:819`、`:886`、`:1063`、`:1075`（报告里的 `AccountId`，形如 `SODEX_SPOT-<账户号>`）；
  - `crates/adapters/sodex/src/http/account.rs:399`、`:411`、`:421`、`:432`、`:453`、`:469`、`:480`、`:490`、`:504`、`:506`、`:517`、`:520`（API Key 的登记与撤销请求）；
  - `crates/adapters/sodex/src/http/requests.rs:1060`、`:1066`、`:1091`、`:1095`、`:1239`、`:1246`、`:1259`、`:1269`、`:1279`、`:1288`、`:1293`、`:1324`、`:1329`、`:1338`、`:1350`、`:1357`、`:1361`、`:1499`、`:1500`、`:1503`、`:1506`（各类请求的 JSON 序列化）；
  - `crates/adapters/sodex/src/http/spot.rs:324`、`:342`、`:350`、`:458`（现货批量下单请求）；
  - `crates/adapters/sodex/tests/integration/python.rs:49`（常量 `TEST_ACCOUNT_ID`，`:142`、`:190` 经常量使用）。

  实施时以当时 `git grep` 的结果为准，行号如有漂移，先更新这张清单。
- **占位值**：统一换成 `12345`。它一看就是示例值，而且与这些测试里出现的其他数字（`symbolID` 1、`id` 7、杠杆 20、各订单号等）都不相同，字段写错位置时测试仍能分辨。
- **断言了这个数字本身的测试**：夹具与断言一起换。例如 `config.rs:414` 断言往返后的 `account_id` 等于这个数字；`requests.rs`、`spot.rs` 与 `client.rs:1974` 期望的 JSON 字符串里含 `"accountID":<账户号>`；`reports.rs` 里的报告标识形如 `SODEX_SPOT-<账户号>`。
  - **为什么不会削弱测试**：这些测试验证的是「给定的账户号原样出现在序列化结果、往返结果或报告标识里」，账户号本身取什么值无关紧要。把输入与期望同时换成同一个占位值，比较的仍是「输入的那个值到了输出」，判别力不变。
  - 签名类测试（`account.rs` 的登记与撤销请求）只断言签名前缀与请求体字段，不比对依赖账户号的签名或摘要常量。这几个测试模块里没有长十六进制期望值，已用 `git grep` 核对，所以换值不需要重算任何金样。
  - 证明办法：换值前后测试数相同；再做一次变异，把某个请求结构的 `accountID` 字段改名，换值前后都应有同样的测试转红。
- **验收**：
  - 示例、源码、集成测试三处扫描账户号，结果必须为 0：`git grep -c <账户号> -- crates/adapters/sodex/examples crates/adapters/sodex/src crates/adapters/sodex/tests` 无输出；
  - `cargo build -p nautilus-sodex --examples` 通过；任取一个示例在不设环境变量时运行，确认报错退出；
  - `cargo test -p nautilus-sodex` 通过，测试数与换值之前相同；上面的变异在换值前后都转红。
- **提交**：`chore(sodex): read the account id from the environment and use a placeholder in tests`。

### Task 1: 现货推断成交不再给出计价币手续费（与 Task 2 同一发布单元）

**Files**：`client.rs`（`calculate_commission` 与既有测试 `:2178-2183`）

- **改法**：现货（`Market::Spot`）返回错误，错误信息说明「现货买单手续费以基础币计、接口拿不到方向，等待真实成交」；永续保持现有按名义额计。
- **失败模式测试（先写，先红）**：
  1. 现货推断成交：0.00149 @ 82845 → 返回错误（现状给出 vUSDC，测试应转红）；
  2. 永续推断成交：口径不变（回归保护）；
  3. 改写既有测试 `commission_is_denominated_in_the_cost_currency`，按新口径断言：现货返回错误，永续以计价币计。
- **变异**：把现货分支改回返回计价币手续费，第 1 条转红；把永续分支也改成返回错误，第 2 条转红。
- **上线约束**：不单独发布。只有 Task 2 一起完成，挂单后才成交的订单才不会停在待对账；完成报告里要写明两者同在一个发布单元。

### Task 2: 适配器在运行中主动送出真实成交，并刷新余额

**Files**：`client.rs`、可能 `reports.rs`；新建 `tests/integration/periodic_check.rs`；`Cargo.toml`

- **Step 0，实证（做不到就停下回报，不改上游核心）**：
  1. 适配器在 `generate_order_status_reports` 里发出 `send_order_with_fills` 之后，与它返回给周期检查的报告，谁先被引擎处理。读 `crates/live` 的执行报告通道与 `manager.rs` 周期检查的处理顺序，写出调用链与 `文件:行号`。目标顺序是：真实成交先应用，周期检查再看到时订单的已成交量已经追平，不再补出推断成交。
  2. 成交接口滞后时怎么办：场所在链上结算，订单历史可能已显示成交，而账户成交还没出现。定一个等待规则：本轮不送出、也不把这份终态记录交给周期检查，下一轮再试；写明重试上限，以及超过上限后的处理（warn 日志，不静默）。
  3. 订单最终一定到达 `FILLED`（或带成交的 `CANCELED`），不会因为终态记录被拿走而卡住。
  4. 从适配器测试里驱动真实周期检查是否可行：需要哪些开发依赖、能否只新增测试文件而不改上游已有测试文件。夹具要按运行时节点的方式，把适配器发送器的通道接到一个 `ExecutionEngine`（引擎在 `engine/mod.rs:1015-1020` 消费执行报告），否则断言 1、2、4 观察不到任何事件；写明这样接线是否仍在「只新增测试文件」的范围内。
  5. 送出范围：确认适配器在 `generate_order_status_reports` 里能读到引擎缓存。适配器的 `core`（`client.rs:115`）是 `ExecutionClientCore`，它带着 `cache: CacheView`（`crates/execution/src/client/core.rs:50`）。以「缓存里存在、缓存已成交量小于历史成交量」作为送出判据；引擎不认识的订单一律不送。
  6. 送出条件：确认能按订单把账户成交与历史记录对上（`clOrdID`、`orderID`），并以「账户成交之和等于历史成交量」为送出条件；只到一部分时本轮不送、不交出终态记录，与第 2 条的重试上限合并为同一规则。
  7. 两种请求的区分：周期检查的请求由 `manager.rs:2088-2091` 构造，`open_only` 取配置值；启动时的全量对账由默认的 `generate_mass_status` 构造，`open_only` 固定为 `false`，并同时请求成交报告（`crates/common/src/clients/execution.rs:326-336`）。
     - 在 `open_check_open_only=true` 下，以 `cmd.open_only` 为判据：`true` 是周期检查，可以送出；`false` 是全量对账，返回值保持原样、不送出。
     - 在 `open_check_open_only=false` 下，只凭这个参数分不出两者。周期检查仍会拿到 `FILLED` 记录、补推断成交；加上 Task 1 的报错，这张单会每轮推迟，一直停在待对账。所以本设计**只支持 `open_check_open_only=true`**。
     - 防护：实证适配器能否覆盖 `generate_mass_status`（适配器内的改动，不改接口），以便标记「本次是全量对账」。能做到时，全量对账之外再收到 `open_only=false` 的请求，即可判定部署配置不受支持，记一条 error 日志 `sodex_open_only_required`，并在文档写明。做不到时，至少在适配器文档写明这项部署要求，并请下游在部署配置里固定 `open_check_open_only=true`（下游现状即如此）。
- **改法**（按 Step 0 的结论落实）：
  - 周期检查（`open_only=true`）读到某张订单在引擎缓存里存在、且缓存已成交量小于历史成交量时，读账户成交，按 `clOrdID` 与 `orderID` 取这张单的全部成交，组装 `FillReport`（成交号、数量、价格、手续费与币种均取场所原值），连同状态报告用 `send_order_with_fills` 送出；
  - 只在这张单的账户成交之和等于历史成交量时送出；不等时本轮不送、也不交出终态记录，下一轮再试；
  - 这份终态记录不再作为普通状态报告返回；
  - 全量对账（`open_only=false`）的返回值保持原样，不送出；
  - 送出之后调用 `publish_account_state` 刷新余额；
  - `find_order_report` 不改。
- **失败模式测试**（新建 `tests/integration/periodic_check.rs`）：
  - **驱动方式**：用真实的 `ExecutionManager` 周期检查路径（`check_open_orders`，`manager.rs:2234`；定向查询 `request_targeted_order_reports`）。SoDEX 客户端接一个用 `axum` 起的假 HTTP 服务，数据按 2026-10-19 的真实形状造：一单两笔成交，`9461715` 成交 0.00149、手续费 9.685E-7 vBTC，`9461716` 成交 0.00001、手续费 3.5E-9 vBTC。配置 `open_check_open_only=true`，与下游一致。夹具里这张订单先处于 `ACCEPTED`（实测那张单就是撤单被拒后回到挂着的状态），不能停在 `SUBMITTED`：停在 `SUBMITTED` 时，在途检查（`manager.rs:1810` 起的 `check_inflight_orders`）会走单笔查询补出推断成交，「到达 `FILLED`」那条断言就分不出新旧写法。轮数固定为 N = 3，新旧写法用同一个 N。适配器发送器的通道按 Step 0 第 4 条接到引擎。
  - **断言**：
    1. 恰好 2 条 `OrderFilled`，成交号分别为 `9461715`、`9461716`，手续费币种为 vBTC；
    2. 没有一条 `OrderFilled` 的成交号是推断格式（按 `create_inferred_reconciliation_trade_id` 的格式判定，不靠匹配日志文本）；
    3. 订单在 N = 3 轮周期检查之内到达 `FILLED`；
    4. 送出成交之后，账户状态的 vBTC 余额等于假服务的 `balances` 值（对应运行中余额刷新）；
    5. 成交接口滞后：
       - 5a：第一轮历史显示已成交、账户成交为空，本轮不得出现推断成交，下一轮账户成交出现后送出真实成交；超过重试上限时有 warn 日志；
       - 5b：两笔成交只出现一笔，本轮没有 `OrderFilled`，也没有推断成交；下一轮两笔都出现后，恰好送出 2 条；
    6. IOC 回归：已撤销且有成交的单仍能拿到真实成交（对应证据 `run2-after-restart-runner.log:327-329`）；
    7. 历史里有一张引擎不认识、带成交的订单：没有任何 `OrderFilled`，也没有建出外部订单；
    8. 全量对账的请求（`open_only=false`）：返回值与修订前相同，不送出成交包。
  - **变异**（每条写出预期转红的断言）：
    - 把适配器改回「总是返回历史记录、不主动送成交」：断言 2 转红；
    - 把适配器改成原计划的写法，即「遵守 `open_only`、只返回挂单列表、也不送成交」：断言 3 转红；
    - 把成交号映射改成引擎生成的 ID：断言 1 转红；
    - 去掉送出后的余额刷新：断言 4 转红；
    - 在账户成交为空时照样交出终态记录：断言 5a 转红；
    - 只到一部分成交时照样送出：断言 5b 转红；
    - 去掉「缓存里存在」的判据：断言 7 转红；
    - 全量对账时也送出：断言 8 转红。
- **验收**：测试先红后绿，红的原因正是要修的缺陷；`cargo test -p nautilus-sodex`、`cargo clippy -p nautilus-sodex --all-targets -- -D warnings` 通过。
- **提交**：`fix(sodex): send a filled order's venue trades to the engine instead of leaving the fill to inference`。Task 1 与 Task 2 的提交在同一次发布之前都要到位。

### Task 3: 查明 accountUpdate 的订阅参数（需授权，见待授权项 A）

**Files**：`crates/adapters/sodex/examples/probe_account_stream.rs`（运行；参数形状若需补，Modify）

- 先跑第一阶段，再跑 `SODEX_PHASE=subsets`；把每次请求与场所的原样回答存进证据。
- **判据**：
  - 找到被接受的订阅：记下参数形状，以及订阅后在运行时长内收到的载荷原样（可能为空）。
  - 127 个子集全被拒：写明「这些字段的任何组合都不是选择器」，Task 4 停止并回报。
- **提交**：`docs(sodex): record the accountUpdate subscription the venue accepts`（或记录否定结论）。

### Task 4: 实现 accountUpdate 订阅与解析

**前置**：Task 3 找到了参数形状，并且拿到了至少一份**真实成交时的载荷**（待授权项 B）。拿不到真实载荷就不写解析器。

**Files**：`websocket/messages.rs`、`websocket/client.rs`、`client.rs`

- **失败模式测试**：
  1. 用真实载荷做夹具，解析出的成交回报与 `account_trades` 里同一笔成交逐字段一致，包括成交号、数量、价格、手续费、手续费币种；
  2. 解析出的账户状态与 `balances` 一致；
  3. 断线重连之后，补取断线期间的成交，不重复也不遗漏；
  4. 同一笔成交既由推送到达、又由 Task 2 的轮询到达时，只产生一条 `OrderFilled`；
  5. 未知字段或缺字段时报错，不静默丢弃。
- **变异**：去掉去重，第 4 条转红；把缺字段改成取默认值，第 5 条转红。
- **验收**：测试先红后绿；`cargo test -p nautilus-sodex`、clippy 通过；测试网验收见待授权项 C。
- **提交**：`feat(sodex): stream fills and balances from accountUpdate`。

### Task 5: 收尾

- 计划 Status 改为 ✅，填写完成报告；完成报告写明 Task 1 与 Task 2 同在一个发布单元。
- `.forge/README.md` 索引更新。
- 更新适配器文档里已过时的说法：`client.rs:5-9` 说「Fill reports are not [verified]」，2026-10-19 的实测已经有真实成交。
- 版本与发布：wheel 由 `.github/workflows/guild-wheels.yml` 构建；是否发布 `sodex.4` 由维护方定，不在本计划内。

## 待授权项 (Pending Authorization)

三项都要在 Guild 的测试网验证账户上进行，账户号不写入本仓库，由授权方在授权时确认。该账户里保留着一小笔基础币余额，作为回归样本：

- 不为了「清理」而卖出它；
- 允许测试交易改变它的数值，但每一项都要在跑前、跑后用只读余额接口记录 vBTC 与 vUSDC；
- 按场所的账户成交逐笔核对「起始 + 买入 − 卖出 − 以 vBTC 计的手续费 = 收尾」，差额必须为 0，不为 0 就照实报告。

### A：在测试网运行 accountUpdate 探针（Task 3）

- **做什么**：只读的订阅探测，不下单，不改余额。
  - 第一阶段：4 个频道 × 36 个候选字段，共 144 次畸形订阅，场所会全部拒绝。
  - 第二阶段：127 次取值合理的订阅。**其中一个可能被接受，也就是真的订阅上账户推送**，订阅只读，探针结束即断开。
- **运行时长**：每次默认 45 秒（`SODEX_SECONDS`）。
- **账户**：经环境变量 `SODEX_ACCOUNT_ID` 给出（Task 0 之后不再有默认值）。
- **证据**：每次请求与回答的原样记录；若订阅成功，记录收到的载荷。
- **余额核对**：不下单，跑前跑后的余额应完全相同。

### B：下两笔小单，录下真实成交时的推送载荷（Task 4 前置）

- **交易对**：`vBTC_vUSDC`。
- **笔数**：2 笔，均为 IOC 限价单，必须成交，否则录不到载荷。一笔买、一笔卖，买卖两侧的载荷都要有，因为两者手续费币种不同。
- **金额上限**：单笔名义额不超过 10 vUSDC，合计不超过 20 vUSDC。卖单从账户已持有的 vBTC 中卖出不低于最小下单量的数量，不卖低于步长的零头。
- **前置**：Task 3 已查明订阅参数，Task 4 的订阅代码可以收录载荷。
- **证据**：两笔订单在推送里收到的载荷原样、同一时间窗的账户成交，以及跑前跑后的余额与账本核对。

### C：三段完成后的测试网验收（Task 1、2、4 一起）

- **构建交付**：
  1. 在本计划的实施分支上手动触发 `.github/workflows/guild-wheels.yml`（它支持 `workflow_dispatch`；上传发布的步骤只在 `guild-v` 开头的 tag 上运行，见该文件 `:125`），只取构建产物里的 Linux wheel，不打 tag，不创建 Release。构建产物会在 Actions 里保留 14 天（该文件 `:122`，`retention-days: 14`）；fork 是公开仓库，任何登录 GitHub 的人都能下载，内容与推上去的分支源码相同。手动触发要求实施分支已推上 fork，这次推送也属于本项授权的一部分。实施分支检出在单独的 worktree 里，不在主检出上，免得此后的提交被每晚的自动推送带上去。
  2. 用这个 wheel 构建一个本地 runner 镜像，单独打标签，不推送。具体命令以下游 runner 仓库当时的本地镜像构建入口为准，实施时写进证据。
  3. 由本地运行工具以开发工具链指向这个镜像，此前的测试网验证用过同一做法。
  4. 验收时核对镜像里的引擎版本与 wheel 的 sha256，避免在旧构建上「验收通过」。
- **交易对**：`vBTC_vUSDC`。
- **笔数**：最多 4 笔买单（3 笔挂在卖一、撤单之前成交的限价单，1 笔 IOC），加 1 笔清理卖单。清理卖单只卖本轮买入的数量（扣除以基础币计的手续费后，向下取整到步长），不碰跑前已有的余额。
- **金额上限**：单笔名义额不超过 150 vUSDC，买入累计不超过 150 vUSDC。验证账户 2026-10-19 收尾时的 vUSDC 约 178，上限据此设定；开跑前以快照为准，余额低于上限时不开跑。
- **判据**：
  - 每一笔场所成交在引擎里都有一条 `OrderFilled`，成交号等于场所 `tradeID`，手续费币种与场所 `feeCoin` 一致；
  - 运行中没有 `Generated inferred fill`；
  - 运行中引擎余额随成交变化；
  - 余额账本差额为 0。
- **证据**：runner 日志、逐笔对照表、跑前跑后的快照与账本核对。

## 验证清单 (Verification)

- [ ] Task 0：示例、源码、集成测试三处账户号扫描为 0；示例可编译，缺环境变量时报错退出；测试数不变
- [ ] Task 1、Task 2、Task 4 的失败模式测试都经历过先红后绿，红的原因正是要修的缺陷
- [ ] 上面逐条写出的变异各跑一次，预期的断言转红
- [ ] `cargo test -p nautilus-sodex`、`cargo clippy -p nautilus-sodex --all-targets -- -D warnings` 通过
- [ ] 没有改 `crates/common`、`crates/live`、`crates/execution` 里任何已有文件（`git diff --stat` 核对）
- [ ] 每一处引用的上游行为都有 `文件:行号` 证据
- [ ] 待授权项未经授权不执行

## 进度追踪 (Progress)

| Task               | Status | Completed | Notes             |
| ------------------ | ------ | --------- | ----------------- |
| 0 示例与测试去掉账户号       | 🔲     |           |                   |
| 1 现货推断成交返回错误       | 🔲     |           | 与 Task 2 同一发布单元   |
| 2 适配器主动送出真实成交并刷新余额 | 🔲     |           | Step 0 实证不成立则停下回报 |
| 3 accountUpdate 探测 | 🔲     |           | 待授权 A             |
| 4 accountUpdate 实现 | 🔲     |           | 待授权 B             |
| 5 收尾               | 🔲     |           |                   |

## 修订记录 (Revisions)

2026-10-19 第一次修订，依据独立评审（1 条 HIGH、3 条 MEDIUM、6 条 LOW）与维护方当天的两项采纳：

| 评审项 | 原写法                                        | 修订                                                                                                                                                      |
| --- | ------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------- |
| H-1 | 第二段让适配器遵守 `open_only`，以为已成交的单会走定向查询并取到真实成交 | 前提不成立（`manager.rs:2388-2417`、`:5578`、`:5662-5667`）。改为适配器主动用 `send_order_with_fills` 送出真实成交；`find_order_report` 不改；Step 0 实证先后、等待上限与到达 `FILLED`，做不到就停下回报 |
| M-1 | 第一段在「扩接口」与「在调用点改写」之间二选一，两者都改上游             | 只改适配器：现货推断成交返回错误，推迟对账；必须与 Task 2 同一发布单元；既有测试 `client.rs:2178-2183` 列入清单                                                                                 |
| M-2 | 失败模式测试拦不住 H-1，夹具与变异没有落地                    | 驱动真实周期检查、假 HTTP 服务、一单两笔成交的真实形状、`open_only=true`；逐条写出断言与变异，「原计划写法」使「到达 `FILLED`」转红；写明开发依赖                                                                |
| M-3 | 三项待授权只有一句话                                 | 每项写明账户、交易对、笔数、金额上限、时长、证据与余额账本核对；A 项写明第二阶段可能真的订阅成功；C 项写明构建交付办法                                                                                           |
| L-1 | 正文有内部编号与内部用语                               | 改为中性说法                                                                                                                                                  |
| L-2 | 示例内置账户号                                    | 新增 Task 0，示例一律改读环境变量；历史不改写。第二次修订：范围扩大到 `src/` 与 `tests/`，测试数据换成占位值                                                                                      |
| L-3 | README 写「不涉及上游源码」                          | 按实际范围改写                                                                                                                                                 |
| L-4 | 余额目标只靠 Task 4                              | 送出真实成交后刷新余额，列为 Task 2 的断言 4                                                                                                                             |
| L-5 | 原 Task 2 第 4 条在原设计下不可实现                    | 并入新 Task 2 的断言 5                                                                                                                                        |
| L-6 | 文件清单缺项                                     | 补上开发依赖、新测试文件、既有测试与示例文件                                                                                                                                  |

同日第三次修订，依据复核（0 条 HIGH、3 条 MEDIUM、2 条 LOW、4 条 INFO）：

| 复核项 | 修订                                                                                                            |
| --- | ------------------------------------------------------------------------------------------------------------- |
| N-1 | 送出范围限定为「引擎缓存里存在、缓存已成交量小于历史成交量」的订单；Step 0 第 5 条实证缓存可读；断言 7 与对应变异                                               |
| N-2 | 只在账户成交之和等于历史成交量时送出；Step 0 第 6 条；断言 5b 与对应变异                                                                   |
| N-3 | 写明以 `cmd.open_only` 区分周期检查与全量对账，本设计只支持 `open_check_open_only=true`，以及 `false` 时的后果与防护；Step 0 第 7 条；断言 8 与对应变异 |
| N-4 | 夹具写明把发送器通道接到引擎（Step 0 第 4 条）；订单先处于 `ACCEPTED`，轮数固定为 N = 3                                                     |
| N-5 | C 项的清理卖单只卖本轮买入的数量，不碰原有余额                                                                                      |
| I-7 | 「不创建发布」如实改为「不创建 Release，构建产物在 Actions 中保留 14 天，公开可下载」                                                         |
| I-9 | 实施分支放在单独的 worktree 里                                                                                          |

## 偏离与改进日志 (Deviations & Improvements)

| 类型        | 位置       | 描述                                                                                                              | 已批准              |
| --------- | -------- | --------------------------------------------------------------------------------------------------------------- | ---------------- |
| DEVIATION | 本计划的首次提交 | 首次提交 `fb80b799c5`（连同其后的表格规整提交）在推送之前被替换为 `7219189d08`：正文里有几处只适用于内部工作区的引用，不适合放进公开仓库，已改为中性说法。两次提交的计划内容相同，只有这几处措辞不同 | ✅（2026-10-19 确认） |

## 完成报告 (Close-out Report)

（执行完毕后填写）
