# 02 - SoDEX 成交以真实回报进入引擎：手续费口径、周期检查取真实成交、接上 accountUpdate

> **Status**: 🔲 Not started（只起草，待独立评审）
> **Created**: 2026-10-19
> **Project**: nautilus_trader (fork) — `crates/adapters/sodex`（Guild 自维护的场所适配器）
> **For Claude**: Use `/forge:execute` to implement this plan.

## 上下文 (Context)

本计划的来源是 2026-10-19 在 SoDEX 现货测试网上的一次实测（Guild 的测试网验证账户，引擎 `guild-v2.0.0rc5+sodex.3`，即 `2d1e432cd1`）。完整证据与分析在 Guild 内部的验证记录里，不随本仓库发布；下文引用的日志文件名、`post-trades.json` 都指那份证据。决策人当天裁定「三处都立，按建议分工」，本计划是落在 fork 的那一处。

**现象**（实证，日志行号指上述证据目录里的 runner 日志）：

- 挂单后才在场所成交的限价单，运行中只由周期检查补出一条推断成交（`run2-main-runner.log:278`，`Generated inferred fill for dfa0b9e8…`；`:280`，`OrderFilled`）。
  - 成交号是引擎生成的 UUID（`a21180f0-175c-51d4-8a8f-579e909f51e0`）。
  - 手续费记为 `0.0807738750000000 vUSDC`，等于名义额 × 0.00065。
- 场所上，同一笔单是 2 笔成交：9461715 成交 0.00149，手续费 9.685E-7 vBTC；9461716 成交 0.00001，手续费 3.5E-9 vBTC（证据 `post-trades.json`）。
- IOC 单走的是定向查询，拿到了真实成交：`run2-after-restart-runner.log:327`，`Found adcdf557… via targeted order status query`；`:329` 的成交号是 9461725，手续费 vBTC，与场所一致。
- 引擎账户余额在一轮运行之内从不变化：`run2-main-runner.log` 里 27 次 `Updated AccountState` 的 vBTC 都是 `0.0000093500000000`。
- 下游后果：custos 的事实桥对每条 `OrderFilled` 原样签发成交与手续费事实。补出的成交因此带着错误的成交号、笔数和手续费币种，进入签名事实，与 custos 按场所成交采集的账本证据对不上。下游如何处理不在本计划内，由下游各自的计划承接。

**根因**（实证，均附 `文件:行号`）：

1. **补出成交的手续费口径错误**。`crates/adapters/sodex/src/execution/client.rs:709` 的 `calculate_commission` 按名义额在计价币上计费，`:748-754` 原文为「Fees are charged on notional in the quote asset on both engines」。但同一个适配器的 `crates/adapters/sodex/src/http/account_reads.rs:225-229` 记录了现货买单从**基础币**扣手续费。这次实测再次印证：买单手续费 = 成交量 × 0.00065，以 vBTC 计（9.685E-7 = 0.00149 × 0.00065）；卖单手续费 = 名义额 × 0.00065，以 vUSDC 计（S1：0.00097 × 82816 × 0.00065 = 0.052215488）。
   - 难点：trait 方法 `calculate_commission`（`crates/common/src/clients/execution.rs:414`）的参数里没有订单方向。
   - 推断成交在 `crates/live/src/execution/manager.rs:4909` 的 `resolve_inferred_fill_commission` 里调用它，那里也没有传方向。
2. **周期检查把历史里已成交的单当状态报告返回，引擎因此补出推断成交**。
   - custos 配置的是 `open_check_open_only=True`（custos `src/custos/engines/nautilus/host.py:385`），NT 也会在请求里带上 `open_only`（`crates/common/src/messages/execution/report.rs:95`）。
   - 但 SoDEX 的 `generate_order_status_reports`（`crates/adapters/sodex/src/execution/client.rs:770`）不读这个参数，`collect_order_reports`（`:1585`）总是读挂单列表和历史两份（`:1586-1598`），已成交的单因此作为状态报告返回。
   - NT 在 `crates/execution/src/reconciliation/orders.rs:1170` 为它补出推断成交。
   - 只有订单从挂单列表消失、走定向查询时，NT 才在 `crates/live/src/execution/manager.rs:5591` 调 `generate_fill_reports` 取真实成交；IOC 走的就是这条路。
   - Binance 适配器读了 `open_only`（`crates/adapters/binance/src/spot/execution.rs:1348`），可以作参照。
3. **执行客户端没有任何推送通道**。
   - 适配器只实现了行情频道（`crates/adapters/sodex/src/websocket/messages.rs:93`、`:119`、`:122`）。
   - 交易所**有** `accountUpdate` 频道，但订阅参数还没试出来：`crates/adapters/sodex/examples/probe_channels.rs:18-20` 原文为「`accountUpdate` exists but refused all eighteen parameter shapes tried as `invalid params`. Its selector is still unknown, which is why this adapter reports no fills」。
   - `crates/adapters/sodex/examples/probe_account_stream.rs` 已经写好第二阶段：把场所报出的 7 个字段做全部非空子集，共 127 次请求。尚无它的运行记录。
   - 账户余额只在连接时（`execution/client.rs:674`）和收到 `QueryAccount` 时（`:763`）发布，而账户是场所上报型（日志中 `is_reported=true`），所以运行中余额不更新。

**Step 1.5 契约核对**：
- 本计划改动的是适配器内部，以及可能改动的 `calculate_commission` trait（见 Task 1 的决策点）。
- 对外契约是 NT 的 `FillReport`、`OrderStatusReport`、`AccountState`，字段不变。
- custos 只消费 `OrderFilled`、`AccountState` 事件，不直接依赖适配器。

## 目标 (Goal)

SoDEX 现货订单的每一笔场所成交，都以场所的成交号、数量和手续费（含币种）进入引擎；运行中的账户余额跟随场所变化。在做到之前，任何退回到推断成交的路径，手续费币种和金额也要与场所规则一致。

## 架构 (Architecture)

三段，按依赖与收益排序，每段可独立验收、独立提交：

1. **手续费口径**：推断成交的手续费按方向计——现货买单以基础币、按成交量计，卖单以计价币、按名义额计。这一段是兜底，后两段都做成之后，推断成交只在异常路径出现。
2. **周期检查取真实成交**：让 `generate_order_status_reports` 遵守 `open_only`。已成交的单不再以状态报告出现，改由 NT 走定向查询加 `generate_fill_reports` 拿到真实成交。
3. **接上 accountUpdate**：先跑完 `probe_account_stream` 的第二阶段，确认订阅参数和推送的载荷形状；再实现订阅，把推送解析为成交回报与账户状态。做成之后，成交与余额都实时到达。

## 关键设计决策 (Key Design Decisions)

| 问题                | 决策                                                         | 理由                                                                           |
| ----------------- | ---------------------------------------------------------- | ---------------------------------------------------------------------------- |
| 三段的顺序             | 手续费 → 周期检查 → 推送                                            | 前两段只动适配器、无需外部未知；第三段依赖场所未公开的订阅参数，风险最高，放最后                                     |
| 手续费需要方向而 trait 没有 | Task 1 先实证调用点能否拿到订单方向，再在「扩 trait」与「在调用点按方向改写」二者中选，结论写进偏离日志 | 扩 trait 会改 `crates/common` 与 `crates/live`，与上游分歧更大；调用点方案只动 live 一处。不预设，由实证决定 |
| 周期检查是否改 NT 核心     | 不改。只让适配器遵守 `open_only`                                     | NT 已有「缺失订单 → 定向查询 → 取成交回报」的路径，IOC 的实测已经证明它能拿到真实成交                            |
| 推送的订阅参数           | 先跑探针，查明之后再写解析代码                                            | `probe_channels.rs` 记录过照着推测写导致的最坏故障，本计划不重复                                   |
| 测试网验收             | 列为待授权项，不在本计划的执行授权内                                         | 测试网要用真实凭据与 Guild 的验证账户，每次需决策人单独授权（沿用此前测试网验证的做法）                              |

## 承载决策 (Capability Hosting Decision)

| 能力                | plan mode? | hook? | CLAUDE.md? | 现有 skill flag? | 新 skill/command? | 决策               |
| ----------------- | ---------- | ----- | ---------- | -------------- | ---------------- | ---------------- |
| SoDEX 成交与账户状态的正确性 | 否          | 否     | 否          | 无              | 否                | 适配器代码 + 失败模式测试承载 |

## 文件清单 (File Inventory)

| 文件路径                                                                 | 操作                  | 描述                                                             |
| -------------------------------------------------------------------- | ------------------- | -------------------------------------------------------------- |
| `crates/adapters/sodex/src/execution/client.rs`                      | Modify              | 手续费口径；`generate_order_status_reports` 遵守 `open_only`；推送订阅（第三段） |
| `crates/adapters/sodex/src/execution/reports.rs`                     | Modify（第二段可能）       | 只读挂单列表时的报告整理                                                   |
| `crates/adapters/sodex/src/websocket/messages.rs`                    | Modify（第三段）         | `accountUpdate` 订阅与载荷类型                                        |
| `crates/adapters/sodex/src/websocket/client.rs`                      | Modify（第三段）         | 私有频道订阅                                                         |
| `crates/common/src/clients/execution.rs`                             | Modify（视 Task 1 决策） | 若选扩 trait：`calculate_commission` 增加方向参数                        |
| `crates/live/src/execution/manager.rs`                               | Modify（视 Task 1 决策） | 推断成交手续费的调用点                                                    |
| `crates/adapters/sodex/examples/probe_account_stream.rs`             | 运行，必要时 Modify       | 第二阶段探测                                                         |
| `.forge/plans/2026-10/02-sodex-inferred-fills-and-account-stream.md` | Create              | 本计划                                                            |
| `.forge/README.md`                                                   | Modify              | 计划索引                                                           |

## 实现任务 (Tasks)

### Task 1: 推断成交的手续费按方向计

**Files**：`crates/adapters/sodex/src/execution/client.rs`，以及按决策可能涉及的 `crates/common/src/clients/execution.rs`、`crates/live/src/execution/manager.rs`

- **Step 0，实证决策点**：读 `manager.rs` 里 `resolve_inferred_fill_commission`（`:4909`）的全部调用方，确认调用处手里有没有订单（从而有方向）。把「扩 trait」与「调用点改写」两种方案的改动面列出来，选一种，写进偏离日志。
- **失败模式测试（先写，先红）**：
  1. 现货买单的推断成交：0.00149 @ 82845，taker 费率 0.00065。手续费必须是 `0.0000009685 vBTC`；现状给出的是 vUSDC，测试应转红。
  2. 现货卖单的推断成交：0.00097 @ 82816。手续费必须是 `0.052215488 vUSDC`。
  3. 方向未知时（若方案允许出现这种情况）必须返回错误，不得退回计价币。这一条对应 `common/src/clients/execution.rs:405-408`「must not substitute a zero or generic commission」的契约。
  4. 永续合约的手续费口径保持不变（回归保护）。
  5. 舍入：基础币手续费按 vBTC 的精度舍入。先查证 vBTC 在引擎里的精度，以及场所回报的手续费位数（9.685E-7 有 10 位小数），确认舍入不会改变数值；输入必须能区分「按成交量计」与「按名义额计」两种规则（教训 #57）。
- **验收**：上述测试先红后绿；`cargo test -p nautilus-sodex`、`cargo clippy -p nautilus-sodex --all-targets -- -D warnings` 通过；若改了 `common` 或 `live`，再跑这两个 crate 的测试。
- **提交**：`fix(sodex): charge an inferred spot buy's fee in the base asset`，或按实际方案写。

### Task 2: 周期检查遵守 `open_only`，已成交的单改走真实成交回报

**Files**：`crates/adapters/sodex/src/execution/client.rs`（`generate_order_status_reports` 与 `collect_order_reports`），可能涉及 `execution/reports.rs`

- **Step 0，实证**：读 `manager.rs:2380-2420`（`open_only` 模式下缺失订单的处理）与定向查询的入队逻辑，确认「已成交的单从挂单列表消失」之后，NT 一定会对它发定向查询，并在 `:5591` 取成交回报。把调用链写进计划的进度备注；若不成立，停下来回报，不改 NT 核心。
- **失败模式测试**：
  1. `open_only=true` 时，历史里已成交、挂单列表里没有的订单，不出现在状态报告里。
  2. `open_only=false`（启动对账）时，行为保持不变，仍按 `current_orders` 合并两份列表（回归保护，对应 `reports.rs:235-300` 已有的「按最新修订」规则）。
  3. 端到端（用 NT 测试替身驱动执行管理器）：一笔挂单在两次周期检查之间成交，结果应是一条或多条 `OrderFilled`，成交号等于场所 `tradeID`，并且没有 `Generated inferred fill`。用 2026-10-19 的真实形状造夹具：一单两笔成交，0.00149 与 0.00001。
  4. 成交接口暂时没返回这笔成交时（场所在链上结算，可能延迟），不得补出推断成交；要等下一轮。记录重试次数上限与超限后的行为，超限时必须有日志，不得静默。
  5. 部分成交后仍挂着的单：不在本段改变它的处理，写一条测试固定现状。
- **验收**：测试先红后绿；`cargo test -p nautilus-sodex`、clippy 通过。
- **提交**：`fix(sodex): honor open_only so a filled order is reconciled from its fills`。

### Task 3: 查明 accountUpdate 的订阅参数（探测，只读）

**Files**：`crates/adapters/sodex/examples/probe_account_stream.rs`（运行；参数形状若需补，Modify）

- **前置**：探针连测试网，**需要决策人授权才运行**（待授权项 A）。它只发送场所必定拒绝的畸形订阅，不下单、不改账户（文件头 `:22-23`）。
- **步骤**：
  1. 先跑第一阶段，记录场所报出的字段与类型；
  2. 再跑 `SODEX_PHASE=subsets`；
  3. 把每次请求与场所的原样回答存进证据目录。
- **判据**：
  - 找到被接受的订阅，就记下参数形状，以及订阅后在一段时间内收到的载荷原样（可能为空）。
  - 127 个子集全被拒，就写明「这些字段的任何组合都不是选择器」，第三段停止，并回报。
- **提交**：`docs(sodex): record the accountUpdate subscription the venue accepts`（或记录否定结论）。

### Task 4: 实现 accountUpdate 订阅与解析

**前置**：Task 3 找到了参数形状，并拿到至少一份**真实成交**时的载荷。要拿到这份载荷，需要在测试网下单，属于待授权项 B。拿不到真实载荷就不写解析器（`probe_channels.rs` 的教训）。

**Files**：`websocket/messages.rs`、`websocket/client.rs`、`execution/client.rs`

- **失败模式测试**：
  1. 用真实载荷做夹具，解析出成交回报：成交号、数量、价格、手续费、手续费币种都与 `account_trades` 的同一笔成交一致；
  2. 解析出账户状态，余额与 `balances` 接口一致；
  3. 断线重连之后，补取断线期间的成交，不重复也不遗漏；
  4. 同一笔成交既从推送到达、又从周期检查到达时，只产生一条 `OrderFilled`；
  5. 未知字段或缺字段时报错，不静默丢弃。
- **验收**：测试先红后绿；`cargo test -p nautilus-sodex`、clippy 通过；测试网验收见待授权项 C。
- **提交**：`feat(sodex): stream fills and balances from accountUpdate`。

### Task 5: 收尾

- 计划 Status 改为 ✅，填写完成报告。
- `.forge/README.md` 索引更新。
- 更新适配器文档里已过时的说法：`execution/client.rs:5-9` 说「Fill reports are not [verified]」，本次实测已经有真实成交。
- 版本号与发布：fork 的 wheel 由 `.github/workflows/guild-wheels.yml` 构建；是否发 `sodex.4` 由决策人定，不在本计划内。

## 待授权项 (Pending Authorization)

| 编号  | 内容                                                 | 需要什么                        |
| --- | -------------------------------------------------- | --------------------------- |
| A   | Task 3 在测试网运行探针（只发畸形订阅，不下单）                        | 决策人授权连测试网                   |
| B   | Task 4 前置：在测试网下一笔小单，录下 `accountUpdate` 的真实成交载荷     | 决策人授权下单，并定账户、交易对、单笔与累计上限    |
| C   | 三段完成后的测试网验收：挂单后成交的订单，引擎里的成交号、手续费币种与场所一致，运行中余额随成交变化 | 决策人授权；沿用此前测试网验证的探针夹具与余额核对办法 |

## 验证清单 (Verification)

- [ ] Task 1 到 Task 4 的失败模式测试都经历过先红后绿，红的原因正是要修的缺陷
- [ ] 关键断言各做一次变异，确认转红
- [ ] `cargo test -p nautilus-sodex` 通过；涉及的 crate 测试通过
- [ ] `cargo clippy -p nautilus-sodex --all-targets -- -D warnings` 通过
- [ ] 每一处引用的上游行为都有 `文件:行号` 证据
- [ ] 待授权项未经授权不执行

## 进度追踪 (Progress)

| Task               | Status | Completed | Notes |
| ------------------ | ------ | --------- | ----- |
| 1 手续费口径            | 🔲     |           |       |
| 2 周期检查取真实成交        | 🔲     |           |       |
| 3 accountUpdate 探测 | 🔲     |           | 待授权 A |
| 4 accountUpdate 实现 | 🔲     |           | 待授权 B |
| 5 收尾               | 🔲     |           |       |

## 偏离与改进日志 (Deviations & Improvements)

| 类型        | 位置       | 描述                                                                                                              | 已批准              |
| --------- | -------- | --------------------------------------------------------------------------------------------------------------- | ---------------- |
| DEVIATION | 本计划的首次提交 | 首次提交 `fb80b799c5`（连同其后的表格规整提交）在推送之前被替换为 `7219189d08`：正文里有几处只适用于内部工作区的引用，不适合放进公开仓库，已改为中性说法。两次提交的计划内容相同，只有这几处措辞不同 | ✅（2026-10-19 确认） |

## 完成报告 (Close-out Report)

（执行完毕后填写）
