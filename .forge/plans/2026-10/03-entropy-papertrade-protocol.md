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
