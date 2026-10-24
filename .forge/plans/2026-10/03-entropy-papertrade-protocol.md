# Entropy 接入与 Papertrade 协议适配

状态：已完成原生 LiveNode 接线与本地恢复验收；真实资金交易未行使。

日期：2026-10-21。开始时 revision 为 `ac86ec3c5f`，工作树为空。
用户要求分析两个平台与 Hyperliquid 适配器的关系，并在本 fork 编写相应逻辑；用户随后明确要求继续网络搜索 Papertrade 的 API 资料。

## LiveNode 扩展实施（用户 2026-10-21 续做授权）

续做基线为 `d827cf79c8`，工作树为空。用户明确要求补齐 `LiveNode`、成交、仓位对账与重启恢复。
本节扩展原来的协议层范围，不能把原协议层交付误报为完整适配器。

- [x] 核验并实现市场配置、分页成交历史、账户 SSE 快照及连续增量；保存公开响应 fixture。
- [x] 使用已有 redb 依赖持久化账户命名空间、nonce、本地命令、签名意图与确认过的成交。
- [x] 实现原生 Rust 数据和执行客户端、factories 与 PyO3 注册。
- [x] 实现以意图 ID 绑定链上 batch 结果的成交对账，覆盖清算、外部操作及历史分页。
- [x] 验证断流重连、版本缺口、重复事件、提交结果未知和重启恢复；不重签替代未知订单。
- [x] 构建 Python 扩展并验证真实 `LiveNode` 接线与公开只读路径，更新说明与验收边界。

交易能力明确限制为协议实际支持的保证金开仓和整仓平仓。开仓使用 quote quantity 表示美元名义金额，
通过配置杠杆换算保证金；确认成交后把订单更新为按实际入场价计算的标的数量，再发送 fill。
平仓使用明确的 venue position ID 和匹配的整仓数量。拒绝限价、部分平仓及原生不支持的条件单。
行情只发布 BBO 中点参考报价，报价数量为零，不伪造撮合深度或把报价当作成交。
精确原始金额、付款队列与链上证据单独保留，不把 queued balance 当作可用现金。

已直接核验的公开接口包括 `GET /state/trading`、`GET /state/user/live?wallet=...`、
`GET /query/accounts/{wallet}/trades?filter=all&limit=75`。历史分页参数必须按 canonical URL 排序。
历史交易 `0xcb71cadd4fc05d01b91c5413993b3f2781476afbb0d352bfd38324f3df5fd1e1`
的三个 close calldata 均用独立 `eth-account` 重算，其意图 ID 与 BatchExecutor 日志对应字段一致。
`GET /intents/{id}` 探针返回 404，不能依赖该路径完成重启恢复。

## 交付范围与状态

- [x] 从官方文档、官方前端和公开 API 核实两者的实际交易路径。
- [x] Entropy 复用 Hyperliquid HIP-3 客户端，补充市场发现、行情和默认 dry-run 的执行示例。
- [x] Papertrade 实现独立 Rust 签名意图协议层和 relayer 单次提交，不借用 HyperCore 下单接口。
- [x] 增加独立签名向量、HTTP 异常结果测试、原生 node 构造和示例行为测试。
- [x] 文档说明已验证能力、未完成边界和后续补齐路线。
- [x] 后续授权已补齐原生 `LiveNode` 数据与执行客户端、对账与恢复；未进行真实资金下单验收。

原协议层交付仅包含签名与 relayer 回执；本轮按续做授权增加账户流、成交与仓位证据、清算历史
和持久化恢复。付款队列保留原始债权字段，不当作可用现金。未行使的真实交易与资金操作边界
见 `docs_zh/integrations/papertrade.md`。

## 实现与验证入口

分析说明：`docs_zh/integrations/entropy-papertrade.md`。
Papertrade 代码：`crates/adapters/papertrade/`。
Entropy 代码：`python/nautilus_trader/adapters/hyperliquid/entropy.py` 与 `examples/live/entropy/`。

Rust 协议测试使用 `cargo test -p nautilus-papertrade --locked`；静态检查使用
`cargo clippy -p nautilus-papertrade --all-targets --locked -- -D warnings`。
Python 测试在 `python/` 下运行 `uv run --no-sync pytest tests/unit/adapters/hyperliquid/test_entropy.py tests/unit/adapters/hyperliquid/test_hyperliquid_factories.py -q`。
新增 crate 同时加入 workspace test inventory，避免 CI 的 adapter 测试选择漏掉它。

公开行情探针通过本机现有 Python 原生扩展读取到了 14 个 `io:` 市场；没有签名提交、真实成交或资金操作。
离线签名交叉验证使用 `eth-account==0.13.7`，不新增为产品依赖。

## 完成报告

本轮新增独立 Papertrade 原生客户端、Python registry 与生成 stubs，保留 venue 的 hedging 仓位身份。
请求先持久化再发送；不重签替代未知请求；重启结合完整快照、分页历史和逐意图链上结果恢复。
quote quantity 在真实成交数量可知后通过原生更新事件转换为 base quantity，避免标志与数量单位不一致。
公开账户流可直接读取；注册 session key 才能通过执行客户端的账户归属检查。

### 验证结果

- `cargo test -p nautilus-papertrade --locked`：68 项通过，含丢回执、版本缺口重连、数据库重开及重复命令恢复。
- Clippy 对该 crate 的全部 targets 和 Python feature 检查通过；Ruff、Rustfmt、workspace test inventory 与 diff 检查通过。
- `make build-debug SCCACHE=`：生成 stubs 并构建、安装 Python 开发扩展通过。
  默认 sccache 构建被系统拒绝，沙箱外重试仍然拒绝，故使用 Makefile 文档已有的禁用缓存开关。
  macOS linker 有大型 unwind table 提示，构建结果成功，不将提示当作测试通过证据。
- Papertrade、Entropy 与原有 Hyperliquid 工厂相关 Python 测试共 15 项通过。
  Papertrade 测试实际运行 `LiveNode`、执行引擎与缓存，恢复同标的的独立多空仓位。
- `make pytest-collect-fast SCCACHE=`：5,185 项成功收集。这是导入与收集检查，不是执行了全部测试。
- 公开 data-only 示例实际连接官方服务，连续收到 `BTC-USD-PERP.PAPERTRADE` 的零数量参考报价并正常退出。
  本地日志为 `target/papertrade-public-node-smoke.log`；日志属于本次运行证据，不进入源码固定或历史收据门禁。

### 功能验证（主路径）

1. 运行 `uv run --project python --no-sync python examples/live/papertrade/data_tester.py`，确认收到 `PAPERTRADE` 参考报价；按 Ctrl+C 后应正常退出。
2. 通过安全环境变量注入已注册 session key，按 `docs_zh/integrations/papertrade.md` 运行账户监控示例，确认钱包与独立仓位完成对账。示例没有下单策略。
3. 为该账户持续使用同一 journal。重启后应保留原交易身份；读源失败或证据不足时应保留未决并阻断新开仓，而非重新发送请求或声称空仓。

### 未行使的边界

没有持有或使用真实账户私钥，没有发送生产签名开平仓、注册密钥、存提或桥接请求。
真实资金与长期运行验收需要有权限的独立测试账户；本轮本地和公开只读证据不替代该验收。
适配器恢复不替代策略内部状态持久化；公开前端接口也没有被当作已发现的版本化兼容承诺。

## 深度审查修复验证（2026-10-22）

按只读审查发现的六项问题逐项修复并提交；本轮验证未发送任何真实订单。

| 问题            | 修复提交         | 行为与验证                                    |
| ------------- | ------------ | ---------------------------------------- |
| 账户流失效后轮询恢复旧状态 | `71431745ae` | 独立流健康门禁，读取截止时间跨轮询保持；持续 503 与 35 秒静默测试通过。 |
| 重启丢失覆盖基线      | `223423a044` | 读回最后验证检查点；拒绝旧快照，失败对账不推进基线。               |
| 本地平仓先发布为外部订单  | `19c8b57fd5` | receipt 暂不可读时保留未决；证据恢复后只发布已绑定本地成交。       |
| 链上成交与旧仓位覆盖混用  | `c43a345a4d` | 扫描受账户覆盖约束，恢复仓位须有当前或终结证据。                 |
| 关闭历史经济字段不核对   | `132b6a1810` | 写入前核对仓位不可变字段与市场尺度；冲突不污染持久化状态。            |
| 发送前崩溃永久阻断     | `f6c26f938c` | 发送尝试标记先落盘；未发送记录本地拒绝，未知尝试须有完整过期证据才结束。     |

最终 Rust 测试为 86 项通过；Clippy（含 Python feature）、Rustfmt、Ruff 和 workspace test inventory 检查通过。
`make build-debug SCCACHE=` 重建并安装实际原生扩展成功；相关 Python 回归为 17 项通过。
新增两项 LiveNode 测试验证了 receipt 延迟时的本地成交归因、空缓存冷启动，以及节点进程被直接终止后的恢复。
`make pytest-collect-fast SCCACHE=` 成功收集 5,187 项；该数字仅代表收集成功，并非全量执行。
最终缓存具有两张订单、两个稳定 Trade ID 和已关闭仓位；独立预设的实现净盈亏为 18.25 USDC，
settlement adjustment 为 1.75 USDC，账户总额为 1018.25 USDC，冷恢复后保持一致。
完整测试入口及策略认领配置边界见 `docs_zh/integrations/papertrade.md` 的故障恢复验证一节。

## 第二轮复审修复验收（2026-10-22）

基线为 `3d324bbe2e`。复审发现的三个 P1、两个 P2 均先由测试复现，再修改实现；
代码验证截至 `9890630de6`，本段仅补充验收记录。

- `4e88082dcb`：持久化链上及账户流成功证据，成功平仓不因历史滞后或重启变成过期。
- `9ce74e7127`：失败对账保留的历史也必须经过当前账户覆盖检查。
- `66bbe0d4fe`：原生执行引擎对已记账成交只恢复订单投影，拒绝身份或经济字段冲突。
- `3f1f93ad39`：开仓与关闭证据双向校验，同轮证据验证后原子保存，随后才推进扫描位置。
- `5fe38b80dd`：用独立流活动期限约束连接、报告和下单，慢历史读取不能延长旧账户的可用期。
- `9890630de6`：旧未决日志缺少成功标记时重新扫描原区间，保留 nonce、意图 ID 和签名载荷。

验收命令与结果：

| 命令或检查                                                                                                  | 结果                              |
| ------------------------------------------------------------------------------------------------------ | ------------------------------- |
| `cargo test -p nautilus-papertrade --tests --locked`                                                   | 103 项通过，包含 6 个原生 LiveNode 恢复用例。 |
| `cargo test -p nautilus-execution --test integration exec_engine:: --features high-precision --locked` | 267 项通过。                        |
| `make build-debug SCCACHE=`                                                                            | 原生扩展重建和安装成功。                    |
| Papertrade、Entropy 与 Hyperliquid 工厂 Python 回归                                                          | 17 项通过。                         |
| Clippy、Rustfmt、Ruff、测试约定、workspace test inventory、diff 检查                                              | 通过。                             |

LiveNode 测试分别覆盖保留开仓/平仓与缺失/落后订单、停机期间完成开平仓、历史先到、
冲突隔离及纠正后恢复。Trade ID、佣金、仓位事件数和盈亏保持一致；已平仓样例的净盈亏
为 18.25 USDC，结算调整为 1.75 USDC。保留快照由公有 Cache 接口装入并重建索引，
未将这些结果扩大为 Redis/PostgreSQL 驱动的崩溃原子性或主网交易验收。
详见 `docs_zh/integrations/papertrade.md` 的第二轮复审修复验证一节。

## 第三轮复审修复（2026-10-23）

基线为 `a7bcb86594`，目标是修复旧版已误判过期的日志，以及关闭历史早于开仓扫描覆盖
跨轮次到达时的成交归属。全部交易来源均为本地故障服务，没有提交真实订单。

| 逻辑单元        | 提交           | 失败复现与修复                                                                                                         |
| ----------- | ------------ | --------------------------------------------------------------------------------------------------------------- |
| 旧日志过期纠正     | `9798f27ab0` | 旧格式 `Expired` 加成功 receipt 的测试先失败。迁移恢复待核实状态并重扫，不改变原签名身份、不重发；真正过期的订单仍按完整证据结束。                                     |
| 历史跨轮次归因     | `5117c1d04a` | 连续运行和等待期间重启两个 LiveNode 用例均先观察到错误外部成交。未排除本地开仓归属的历史延迟保存和发布；意图结束后主动重读历史。                                           |
| 保留过期订单的引擎恢复 | `d0a859bc98` | LiveNode 保留 `Expired` 缓存时先出现未关闭仓位；模型对账成交的完整、部分成交测试也先失败。仅 reconciliation 成交可纠正过期结果，部分成交保留过期状态，重复 Trade ID 不重复收费。 |
| 强制终止与流成功恢复  | `d7f3d7d47e` | 五个子进程在 journal 提交边界被 `SIGKILL` 后检查持久化状态。账户流 snapshot 与 patch 的 `succeeded` 各自跨无 outcome 的重连恢复。                  |

Rust 验证结果：116 项 Papertrade 测试通过，其中 9 项直接运行 Rust LiveNode；
订单模型全套 3,353 项通过，1 项原有的文档再生成测试按其声明忽略；267 项执行引擎回归通过。
模型与适配器的 all-targets Clippy、Rustfmt、测试约定和 47 个成员的 workspace inventory 检查通过。

LiveNode 逐项核对本地订单与 venue order 绑定、逐笔 Trade ID、订单佣金、仓位事件数和净盈亏。
跨轮次、中途重启和随后冷启动均保持两张订单、两个 Trade ID、已关闭仓位、18.25 USDC
净盈亏与 1.75 USDC 结算调整。五个强制终止用例定位公开 journal 操作的提交前后，
没有向生产代码增加测试钩子；这不覆盖写盘途中掉电或 Redis/PostgreSQL 驱动的原子性。

代码验证截至 `d7f3d7d47e`。`make build-debug SCCACHE=` 重建并安装 Python 原生扩展成功，
保留已有的 macOS unwind table 链接器警告；Papertrade、Entropy 和 Hyperliquid 工厂相关
Python 回归共 17 项通过，包含实际 LiveNode 进程强制终止、禁止重发及冷启动后的盈亏一致性。
详细运行日志保存在本地 `target/papertrade-round3-*.log`；它们是本次验证记录，不固定后续源码字节。

## 第四轮复审修复（2026-10-23）

本轮基线为 `1519cc19e3`，修复审查发现的两个 P1 和一个 P2。三个问题分别先由失败测试复现，
再修改实现；所有交易来源均为本地故障服务，没有下真实订单。

| 问题                     | 修复提交         | 失败复现与验收                                                                   |
| ---------------------- | ------------ | ------------------------------------------------------------------------- |
| succeeded 在扫描推进后丢失     | `df1cd31ac8` | 多订单 RPC 失败以及真实客户端对账途中被 SIGKILL 均先复现成功标记丢失；成功事实改为在后续 I/O 和扫描前批量保存。         |
| 空意图 ID 的已观察仓位被提前归为外部成交 | `e4a1f36456` | 两个 LiveNode 用例先复现错误外部归属；未排除本地开仓关联前，成交与仓位报告暂缓发布，原始事实继续交叉验证。                |
| 旧日志区间超过单轮预算            | `8ea683df52` | 一页预算下的 2,501 区块旧日志先报预算错误；改为每轮最多 1,000 区块，失败页重读，完成页跨重启保留。成功证据在最后一段时仍恢复成交。  |
| 保留缓存与旧过期订单的组合验收        | `ae2b7b43b0` | 实际 LiveNode 生成缓存后，分别保留未平仓或已平仓仓位，并加载旧 Expired 订单；验证延迟证明、分段重扫、等待期间重启及最终冷启动。 |

最终 127 项 Papertrade Rust 测试通过，其中 15 项直接运行 Rust LiveNode。保留仓位等待证明时，
数量、入场均价、Trade ID、事件数、佣金和实现盈亏不变；证明到齐后仅补齐缺少事件。
最终及冷启动后均为两张订单、两个稳定 Trade ID、已关闭仓位、18.25 USDC 净盈亏和
1.75 USDC 结算调整。成功事实与另一订单终态发生矛盾时，测试确认对账持续阻断，已观察成功不丢失。

强制终止测试通过本地 RPC 等待点定位真实 `synchronize` 的执行进度，再由父进程发送 SIGKILL；
生产代码没有测试钩子。缓存通过公有接口加载；本轮不声称验证 Redis/PostgreSQL 驱动自身原子性
或写盘途中掉电。详细日志保存在本地 `target/papertrade-round4-*.log`。

代码验证截至 `ae2b7b43b0`。`make build-debug SCCACHE=` 成功重建并安装原生扩展，保留已有的
macOS unwind table 链接器警告；Papertrade、Entropy 与 Hyperliquid 工厂相关 Python 回归
17 项通过。all-targets Clippy、Rustfmt、测试约定、47 成员 workspace test inventory 和 diff
检查通过。详细行为与边界见 `docs_zh/integrations/papertrade.md` 第四轮恢复修复一节。

## 第五轮恢复覆盖补充（2026-10-23）

基线为 `b64b636929`，补齐复审指出的三个测试缺口。现有实现通过新增场景，本轮没有修改生产源码。

- `6d408d3831`：将日志可见性和 receipt 可用性分开控制；实际 LiveNode 在 receipt 不可读时
  不推进扫描位置、不发布经济事件，恢复后重读并验证保留缓存重启。
- `56d4cade51`：三个不同扫描游标，首个订单提交后强制终止子进程，再通过实际 LiveNode
  验证缺口补读、较快游标不回退、扫描途中上界增长、后段成功与最终过期。
- `158f181d63`：两个无意图关联仓位、多空两张本地开仓、两种证明先后顺序；验证分批归因，
  中途保留缓存重启和最终冷启动不改变 Trade ID、佣金或盈亏。

验证截至 `158f181d63`：适配器 Rust 全套 131 项通过，其中 19 项包含实际 LiveNode；
all-targets Clippy、Rustfmt、测试约定、workspace test inventory 和 diff 检查通过。
双仓位样例共四笔唯一成交，净盈亏 45.25 USDC、结算调整 4.75 USDC、最终余额 1045.25 USDC。
逐订单中断发生在子进程调用生产 API 和 Journal 逐条写入的边界；恢复使用完整 LiveNode，
未向生产代码添加测试钩子。所有服务均为本地故障服务，没有真实下单或推送。
详细记录见 `docs_zh/integrations/papertrade.md` 第五轮恢复覆盖补充一节及本地
`target/papertrade-round5-*.log`。

## 第六轮连续扫描覆盖补强（2026-10-23）

基线 `604e7a94d8` 的混合游标测试存在一个 P2：页大小、恢复起点和最终上界正确，不能排除
中间漏扫。本轮先临时注入游标跳过 2000 个区块的故障，原实际 LiveNode 用例通过；加入独立
连续覆盖断言后，同一故障被 `Unscanned gap` 拒绝。撤销故障后测试通过，生产源码未改变。

- `b86e785b1f`：保留跨重启 RPC 轨迹，按 intent 集合和成功读取区间重建覆盖，核对每张订单
  的游标、上界及过期条件。新增六个校验器测试，包含漏页、其他订单的页面、日志或 receipt
  失败，以及合法重叠补读。
- `6d4a76d140`：在相对区块 3200 加入成功订单哨兵。先在该页注入 RPC 失败并停机，重启必须
  从固定的 2500..3499 补读；覆盖上界增长后核对中间和后段成功订单归属，以及无成交订单过期。
  保留缓存和冷启动均检查 Trade ID、费用、数量及盈亏不重复。

最终在 `6d4a76d140` 上运行适配器 Rust 全套，137 项通过，零失败、零忽略，其中 19 项包含
实际 LiveNode。Clippy、Rustfmt、测试约定、47 成员 workspace inventory、Markdown lint 与
差异检查通过。混合游标场景为三个已关闭仓位、六笔唯一成交和一张无成交过期订单；净盈亏
63.50 USDC，结算调整 6.50 USDC，账户余额 1063.50 USDC。账户余额是报告值，仓位经济数据
由独立常数另行核对。

初始游标是构造的持久化前缀，逐订单 SIGKILL 仍定位于生产 API 与 Journal 接口边界，随后
执行完整 LiveNode 恢复；没有把此项扩大为真实 synchronize 内部中断或存储驱动掉电验收。
未下真实订单，未操作资金，未推送；未修改 Python 绑定、未重建扩展或重跑 Python 测试。
故障步骤、测试结果和边界详见 `docs_zh/integrations/papertrade.md` 第六轮一节及本地
`target/papertrade-round6-*.log`。

## Entropy LiveNode 执行与恢复补充（2026-10-23）

基线 `d0df0c2a01`。新增实际原生 LiveNode 测试，使用本地 HTTP/WebSocket 服务和固定测试
密钥，覆盖动态 HIP-3 asset ID、同名但不同 deployer 的市场、本地成交归因、重复 WebSocket
成交、反向及重复 REST 历史，以及保留仓位加缺失或 Accepted 订单的跨节点恢复。

测试先复现了两处问题：已成交限价单缺少重建所需的原始限价，节点进入 Running 但没有恢复
订单；仅补回限价后，恢复订单 ID 仍变成 CLOID 哈希，原本地订单没有找回。
`37834f30bc` 保留历史限价，并在报告发布前利用缓存 venue ID 和保留成交事件恢复本地身份，
校验账户、合约、方向和 CLOID。实际空账户缓存启动直接通过，未修改账户注册逻辑。
`3365606ab2` 验证 CLOID、deployer 和方向冲突均拒绝启动，且不修改保留仓位的经济数据。

八个实际 LiveNode 用例通过；正常恢复必须得到两张原本地 Filled 订单、一个关闭仓位和
两笔不同 Trade ID，重复重启保持不变。仓位按原生口径分别记录 20 USD 价差盈亏与 0.50 USDC
佣金，订单佣金为 0.20 和 0.30 USDC，不把不同币种直接相减或称为 USDC 净收益。
策略认领配置对应执行示例的 external_order_instrument_ids，缓存通过公开接口加载。

完整复现过程、验证命令与边界见 `docs_zh/integrations/entropy-papertrade.md` 的 Entropy
LiveNode 一节。本地日志位于 `target/entropy-livenode-*.log`。全部订单请求仅到本地模拟端点，
未下真实订单，未操作资金，未推送。

验证代码截至 `3365606ab2`：Hyperliquid Rust 串行全套 1136 项通过，零失败，两个原有忽略项
未变，含本轮八项实际 LiveNode 用例。all-targets Clippy、测试目标 Clippy、Rustfmt、测试
约定、47 成员 inventory、生成文件漂移检查、Markdown lint 与 diff 检查通过。
一次默认并行全套中的旧 WebSocket 取消测试超时，单独复跑及串行全套通过，未修改该测试。
`make build-debug SCCACHE=` 成功重建并安装原生扩展，保留既有 macOS unwind table 警告；
重建后的 Entropy 与 Hyperliquid 工厂 Python 接线回归 13 项通过。

## Entropy 第二轮恢复修复（2026-10-24）

基线 `16ec38e4db`。按测试失败、实现修复、实际 LiveNode 复验的顺序处理两个 P1 与两个 P2，
所有交易请求仅发往本地模拟端点，没有真实下单、资金操作或推送。

- `d4abf94dbc`：提取原生 LiveNode 共享场景，原有八项行为保持。
- `5f2f443f65`：修复 Submitted 无 venue ID 时的 CLOID 归属；分别覆盖挂单和已成交平仓，
  真实请求已到服务但 Accepted 未返回，重启后不能生成影子订单。
- `e0a576e41a`：成交窗口截断时只复用已验证的缓存成交证明；没有证明则阻断启动。
  实际开仓均价 92.5、平仓均价 115，与原限价 100、110 不同；验证三个稳定 Trade ID、
  45 USD 价差盈亏与 0.50 USDC 费用，并覆盖补齐证据和再次重启。
- `8bce89e7a9`：保留 LimitIfTouched 的历史限价与触发价，只有最终历史行时也可恢复。
- `4bb745a8a5`：新成交使用规范化 Trade ID；验证旧指纹后保留旧缓存 ID，防止跨编码重放。
  新旧缓存分别覆盖保留订单与仅保留仓位，部分成交订单仍有可成交余量。修复 Hyperliquid
  open 行中部分成交数量被 Accepted 状态忽略的问题；另补歧义、费用冲突及帧内失败测试。

验证截至 `4bb745a8a5`：Hyperliquid Rust 串行全套 1,151 项通过，两个原有测试忽略项与六个
原有文档样例忽略项不变。包含 17 项实际 LiveNode 用例，比上轮增加九项；最终及再次重启后，
订单归属、Trade ID、费用和盈亏保持一致。all-targets Clippy、Rustfmt、控制流格式约定、
测试约定、47 成员 inventory 和生成文件漂移检查通过。详细路径、失败日志、旧缓存升级规则
与本地验证边界见 `docs_zh/integrations/entropy-papertrade.md` 第二轮一节及本地
`target/entropy-round2-*.log`。

`make build-debug SCCACHE=` 成功重建并安装原生扩展，保留既有 macOS unwind table 警告。
重建后的 Entropy 与 Hyperliquid 工厂 Python 回归 13 项通过；生成文件没有漂移，
Markdown lint、表格格式和差异检查通过。本轮未改 Python 绑定或生成文件。
