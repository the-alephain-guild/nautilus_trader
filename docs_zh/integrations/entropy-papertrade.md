# Entropy 与 Papertrade 的接入分析

本次以 2026-10-21 的公开文档、官方前端和只读 API 为证据，对照仓库
`ac86ec3c5f` 的 Hyperliquid 适配器。结论是：Entropy 直接复用现有适配器；
Papertrade 使用独立的 HyperEVM 意图协议。后续实施已补齐 Papertrade 的原生数据、执行客户端、
Python factories、成交和仓位对账、持久化重启恢复，并以真实 `LiveNode` 验证本地接线。

## 两个平台不是同一类接口

| 对象      | Entropy                             | Papertrade                                 |
| ------- | ----------------------------------- | ------------------------------------------ |
| 交易发生在哪里 | HyperCore HIP-3 `io` 市场             | HyperEVM 合约与 LP 之间                         |
| 交易接口    | Hyperliquid `POST /exchange`        | 官方 origin 的 `POST /intents`，由授权 relayer 提交 |
| 行情      | 对应 `io:` 市场的 Hyperliquid 订单簿、成交和上下文 | Hyperliquid BBO 是合约定价源；没有自己的撮合订单簿          |
| 身份与签名   | Hyperliquid 主钱包或已批准 API agent       | Papertrade 主钱包和注册 session key，独立 EIP-712 域 |
| 数量含义    | 标的数量，精度取当前 metadata                 | `size` 是 18 位小数的保证金                        |
| 平仓与取消   | 原生数量订单、reduce-only 和订单撤销            | 按具体仓位 ID 平仓；取消的是 relayer 中的意图              |
| 盈亏到账    | 按 Hyperliquid 账户和成交模型处理             | 平仓盈利可能先形成付款队列债权                            |
| 本次接入结果  | 复用原生工厂，增加发现函数、行情和执行示例               | 独立原生客户端、Python factories 与持久化恢复            |

[Entropy 官方架构文档](https://docs.entropy.io/about-entropy/readme)写明：
“Entropy (ticker: io) is one such HIP-3 market deployer.”
因此它使用 Hyperliquid 的交易协议。用户给出的
[`/funding` 页面](https://entropy.io/funding)是产品页面，不能作为另一套 trade API 的证据。
资金费率机制应另读[官方 Funding Rate 文档](https://docs.entropy.io/equity-perp-mechanics/funding-rate)。

[Papertrade 官方架构文档](https://docs.papertrade.xyz/how/architecture/)写明：
“Papertrade's relayer submits every open and close.”
因此，即使两个平台都使用 Hyperliquid 生态的基础设施，也不能把 Papertrade 的订单交给
Hyperliquid `ExecutionClient`。

## Entropy 实现方式

复用的客户端和工厂是：

- `HyperliquidDataClientConfig` 与 `HyperliquidDataClientFactory`。
- `HyperliquidExecutionClientConfig` 与 `HyperliquidExecutionClientFactory`。

所有行情和订单仍路由到 `HYPERLIQUID`，不能把 venue 改为 `ENTROPY`。
例如，Entropy 的原始市场名 `io:SNDK` 对应：
`io:SNDK-USD-PERP.HYPERLIQUID`。

新增的
[`load_entropy_instruments`](../../python/nautilus_trader/adapters/hyperliquid/entropy.py)
显式启用 `include_perps_hip3=True`，从原生客户端返回的对象中选择 `io:` 永续市场。
它保留原生对象、资产映射、精度和结算币种；没有固定 dex 序号或重新推算资产 ID。
它只过滤返回列表，不承诺限制整个 `LiveNode` 的市场集合。
市场不存在时明确报错，不回退到同名的 `xyz:` 或默认 perp。

本次通过本机 `python/.venv` 中已安装的原生扩展与新增发现函数，实际读取公开 API，
得到 14 个 Entropy 市场，包含 `io:SNDK-USD-PERP.HYPERLIQUID`。
抽样合约结算币为 USDC，数量精度按各自 metadata 保留。
这次探针没有加载私钥、建立执行账户或发送订单，也没有重新构建整个 Python 原生扩展。

Nautilus 的现有加载逻辑在
[`python/http.rs`](../../crates/adapters/hyperliquid/src/python/http.rs)中区分
`is_hip3`，并按 `include_perps_hip3` 保留相应合约。
现有 HIP-3 订单转换、签名、撤单、账户和恢复逻辑继续由 Hyperliquid 适配器承担。
无需再造一套 Entropy 交易客户端。

运行公开行情示例：

```bash
uv run --project python --no-sync python examples/live/entropy/data_tester.py
```

执行接线示例在
[`exec_tester.py`](../../examples/live/entropy/exec_tester.py)，默认 `DRY_RUN=True`，
并关闭退出时撤单和平仓，也没有绕过 Nautilus 风控。
示例不会创建或划转抵押资金，不会自动批准 agent，不会设置杠杆。
若手动启用执行，必须使用当前市场 metadata 和已配置的账户；本次没有启用或运行生产下单。

Entropy 的不同产品可以有不同资金费率机制、市场开放状态、费用、最大杠杆和 margin mode。
[官方 Margin Modes 文档](https://docs.entropy.io/risk-and-margining/margin-modes)介绍了不同模式；
实际请求仍应以当前 API metadata 为准，不能把某个历史杠杆值或保证金模式写死。

## Entropy LiveNode 执行与恢复验证（2026-10-23）

本轮以 `d0df0c2a01` 为基线，新增
`crates/adapters/hyperliquid/tests/entropy_live_node.rs` 与本地 HTTP/WebSocket 测试服务。
测试使用原生 Rust LiveNode、Hyperliquid 工厂、执行引擎和缓存。节点进入 Running 后，执行
引擎提交合成订单；服务检查实际签名请求，再分别发送提交响应、`orderUpdates` 与成交。
所有测试请求均发往 `127.0.0.1` 临时端口，执行端的 info、exchange 与 WebSocket 地址均覆盖，
签名使用固定测试密钥。

八个用例覆盖：

- 空账户缓存启动，依靠账户事件注册，节点必须进入 Running。
- 交换 `io` 和另一个 deployer 的 metadata 顺序；两个 deployer 同时提供 `SNDK`，实际请求
  的 asset ID 分别为 110000、120000，客户端、合约、CLOID、方向、数量和 reduce-only 均正确。
  预期编号取自[官方 Asset IDs 规则](https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/asset-ids)，
  没有调用待测的资产映射函数计算测试期望值。
- WebSocket 在同一帧重复成交；重启后 REST 成交重复、历史反向到达，经济数据不重复。
- 保留实际成交产生的仓位，分别移除订单、将订单恢复到 Accepted 且未记录成交；节点必须
  找回原客户端订单 ID、venue order ID 和策略归属。恢复后再次重启仍保持相同 Trade ID。
- 历史 CLOID 冲突、同名但属于 `xyz` 的合约、方向相反时，启动必须失败且给出身份冲突错误；
  缓存没有新订单，保留仓位的事件、Trade ID、费用和盈亏保持不变。

外部订单认领配置与执行示例的 `external_order_instrument_ids` 一致，认领策略为
`ENTROPY-001`。缓存样本来自前一次 LiveNode 的真实本地事件处理结果，再通过公开缓存接口
加载到新节点；较旧订单样本取自成交前保存的 Accepted 订单。成功恢复用例必须完成启动对账；
冲突用例必须在启动期间返回指定身份冲突错误，不以请求计数或固定等待时间替代恢复完成。

测试先复现了保留仓位但订单缺失时的故障：已成交限价单报告没有原始限价，节点虽然进入
Running，却未恢复任何订单。日志包含 `` `price` is required for `LimitOrder` initialization ``。
补回历史限价后，同一测试继续失败：订单 ID 变成线上 CLOID 哈希，原本地订单 ID 未恢复。
最终修复保留历史限价，并通过已有 venue order ID 和保留成交事件恢复本地身份；账户、合约、
方向或 CLOID 冲突会使该轮报告生成失败。生产代码没有测试开关。修复提交为 `37834f30bc`，
冲突验证提交为 `3365606ab2`。

经济断言使用独立常数：买入 2 单位，价格 100，卖出价格 110；原生仓位记录 `20 USD`
价差盈亏，另行累计 `0.50 USDC` 佣金，两张订单佣金分别为 `0.20` 和 `0.30 USDC`。
Hyperliquid 合约保留 USD 报价和 USDC 结算标识；Nautilus 此处按其原生 cost currency 规则
分别累计这两个币种，测试不会把 `20 USD` 标成扣费后的 USDC 净盈亏，也不假设 USD/USDC
汇率恒为 1。断言同时检查一个已关闭仓位、两张 Filled 订单、两笔不同 Trade ID、逐订单数量
和费用，并在重启前后比较身份。没有仅凭账户报告余额来推断没有重复记账。

复现日志包括 `target/entropy-livenode-retained-red.log`（缺少限价），
`target/entropy-livenode-retained-price.log`（仅补限价后身份仍错误），以及
`target/entropy-livenode-retained-green.log`（完整修复通过）。首个空账户缓存测试直接通过，
没有修改账户注册逻辑。一次默认并行全套中的原有 WebSocket 取消测试出现共享额度等待超时；
该测试单独复跑通过，串行全套也通过，未修改或忽略它。最终验证结果如下。

验证代码截至 `3365606ab2`：

- `cargo test -p nautilus-hyperliquid --tests --locked -- --test-threads=1`：1136 项通过、零失败，
  两个原有忽略项未变；其中八项为本轮实际 LiveNode 用例。最终日志为
  `target/entropy-livenode-suite-final.log`。
- all-targets Clippy、新增测试目标 Clippy、Rustfmt、测试约定、47 成员 workspace inventory、
  generated-file drift、Markdown lint 和 diff 检查通过。
- `make build-debug SCCACHE=` 成功重新生成类型文件并重建、安装 Python 扩展；生成文件无漂移。
  保留既有 macOS unwind table 链接器警告，未修改链接配置。
- 重建后在 `python/` 运行
  `uv run --no-sync pytest tests/unit/adapters/hyperliquid/test_entropy.py tests/unit/adapters/hyperliquid/test_hyperliquid_factories.py -q`：
  13 项通过。构建与 Python 日志为 `target/entropy-livenode-python-build.log` 和
  `target/entropy-livenode-python-tests.log`。

本轮没有提交真实交易所订单或操作资金。实际运行的节点连接本地模拟服务；主网账户的
订单接收、真实成交和资金结算仍需另行授权的账户验收。

## Entropy 第二轮恢复修复（2026-10-24）

本轮基线为 `16ec38e4db`，修复复审指出的两个 P1 与两个 P2。仍使用原生 Rust LiveNode、
实际 Hyperliquid 工厂和执行引擎，订单请求仅发往本地 HTTP/WebSocket 故障服务。
`d4abf94dbc` 先提取共享场景，原有八个 LiveNode 用例继续通过。

| 问题                       | 修复提交         | 失败复现与最终行为                                                                                |
| ------------------------ | ------------ | ---------------------------------------------------------------------------------------- |
| P1：Submitted 没有 venue ID | `5f2f443f65` | 服务已收到请求，但扣住 Accepted 通知；原实现跨重启产生第三张影子订单。修复按原 CLOID 找回本地订单，分别验证仍挂单与已成交的平仓，并再次重启。          |
| P1：成交窗口截断                | `e0a576e41a` | 2,000 行窗口缺少第一笔开仓成交，原实现使用限价生成推断成交。修复只合并已核验的缓存成交证明；证据不足时启动失败，订单、仓位和佣金不增加。补齐历史后可以恢复。        |
| P2：LimitIfTouched 历史恢复   | `8bce89e7a9` | 只有最终 Take Profit Limit 历史行时，原实现丢失限价，订单无法重建。恢复报告保留限价 109.50、触发价 110.00，实际成交价仍为 110.00。    |
| P2：跨编码 Trade ID          | `4bb745a8a5` | 等值小数的不同位数导致原实现把已成交量从 2 重放到 4。新成交使用规范化指纹，旧缓存通过旧指纹验证保留原 ID；同时修正部分成交挂单仍被报告为 Accepted 的恢复状态。 |

窗口测试使用两笔开仓 `1 @ 90`、`1 @ 95` 和平仓 `2 @ 115`，原始限价分别为 100 和 110。
恢复与再次重启均必须得到三个原始 Trade ID，开仓均价 92.5，平仓均价 115，价差盈亏
45 USD，订单佣金分别为 0.20 与 0.30 USDC。窗口中的其他行来自当前 metadata 无法解析的
退市市场，不能用它们证明 Entropy 成交完整。保留仓位证明可以补齐本单缺少的成交，
但不会把达到端点上限的全局扫描标记改成完整。没有证明时不按限价估算缺少的成交。

跨编码测试分为新旧缓存与保留订单、只保留仓位的四种组合。买单数量为 4，实际只成交
两笔数量为 1 的成交；两笔成交的交易哈希、时间、价格和数量相同，`startPosition` 分别为
0 和 1。REST 首次恢复、恢复后的 WebSocket 回放以及再次重启的 REST 历史分别独立改变
小数位数，且保持金额数值不变。WebSocket 同一帧还携带一笔新的平仓成交，测试等待该成交
完成后才断言结果，防止旧成交尚未处理就提前通过。

最终买单必须仍为 PartiallyFilled，成交量为 2，保留原客户端 ID、venue ID、策略和两个
Trade ID；平仓成交仅增加第三个 Trade ID。两次重启后仓位关闭，价差盈亏仍为 20 USD，
累计佣金仍为 0.50 USDC。买单仍有未成交余量，因此重复回放确实能改变仓位，测试不会只靠
超量成交保护拒绝重复事件而通过。所有恢复均不重新提交订单。

新 Trade ID 格式为 `HL-{指纹16位十六进制}-{订单ID16位十六进制}`，长度为 36。指纹将
价格、数量和 `startPosition` 的 Decimal 尾零规范化，并分隔变长字段。旧缓存原有的
33 字符 Trade ID 不改写：先由账户、合约、订单和毫秒时间缩小候选，再验证旧 FNV 指纹
是否可由同一组成交事实的合法 Decimal 编码生成，最后核对方向、价格、数量、费用和流动性。
没有旧证明时使用新格式，不能凭价量时间猜测旧 ID。两个旧 ID 同时匹配同一成交或费用冲突时，
恢复返回错误，客户端拒绝后续提交和改单；不会自动合并已重复记账的缓存。

额外故障测试确认，WebSocket 一帧中的后一行校验失败时，前面的新成交也不会提前写入
已处理标记，新的恢复会话仍能收到这笔成交。旧版浮点纳秒时间只按场所提供的毫秒精度比较；
内部仓位 ID 不作为场所仓位 ID 发送。推断成交生成的 UUID 不作为缓存成交证明。

验证代码截至 `4bb745a8a5`：Hyperliquid Rust 串行全套 1,151 项通过，零失败，两个原有
测试忽略项不变；另有六个原有文档样例保持忽略。实际运行 LiveNode 的用例共 17 项，较上一轮
增加九项。all-targets Clippy、Rustfmt、控制流格式约定、测试约定、47 成员 workspace
inventory 与生成文件漂移检查通过。

`make build-debug SCCACHE=` 成功重建并安装 Python 原生扩展，保留既有的 macOS
unwind table 链接器警告。重建后的 Entropy 与 Hyperliquid 工厂 Python 回归 13 项通过；
生成文件未产生漂移，Markdown lint、表格格式与差异检查通过。

复现与回归命令示例：

```bash
cargo test -p nautilus-hyperliquid --test entropy_pending --locked
cargo test -p nautilus-hyperliquid --test entropy_window --locked
cargo test -p nautilus-hyperliquid --test entropy_conditional --locked
cargo test -p nautilus-hyperliquid --test entropy_encoding --locked
cargo test -p nautilus-hyperliquid --locked -- --test-threads=1
cargo clippy -p nautilus-hyperliquid --all-targets --locked -- -D warnings
```

详细日志保存在本地 `target/entropy-round2-*.log`。其中 `submitted-red`、`window-red`、
`lit-red` 与 `encoding-original-red` 记录对应失败；`rust-full` 和 `clippy` 记录最终全套结果。
缓存样本来自实际 LiveNode 事件处理，通过公开缓存接口加载到新节点；旧格式样本仅将这些
真实缓存事件的 Trade ID 替换为冻结旧算法生成的 ID，保留其他经济字段。此次验证不包含
Redis/PostgreSQL 驱动自身掉电原子性，也没有向真实场所下单或操作资金。

## Papertrade 实现方式

新增 crate 位于
[`crates/adapters/papertrade`](../../crates/adapters/papertrade/src/lib.rs)，详细协议、接线与验收边界见
[Papertrade 协议适配层](papertrade.md)。

该实现从官方前端核对协议结构，独立编写 Rust 逻辑，没有引入搜索到的第三方交易机器人作为依赖。
它实现四类签名、原生整数串编码、意图 ID、HTTP 提交、回执绑定和结果未知状态。
签名层使用本仓库已有的 Alloy，HTTP 使用本仓库的共享网络组件。
没有新增外部依赖版本，没有读取或写入真实凭据。

公开文档没有提供覆盖这些接口的版本化 SDK 契约。当前观察到的前端 build 是
`169e8a6905e3`，客户端要求调用者显式传入 build，而不把它当作长期有效值。
观察的来源和摘要仅记录此次远端证据，见
[`sources.json`](../../crates/adapters/papertrade/test_data/sources.json)。

官方地址文档与本次读取的前端 deployment 对 collateral 地址给出了不同值：
文档为 `0xb88339CB7199b77E23DB6E890353E22632Ba630f`，
前端为 `0x6b9e773128f453f5c2c60935ee2de2cbc5390a24`。
本次没有验证这两个地址当前的链上角色，所以没有实现存款、提现、approve 或桥接，
也不推断其中哪一个可安全使用。后续资金接入必须先用当前代理存储与链上调用独立核验。

Papertrade 的账户流、链上意图证据、订单与仓位映射、动态交易校验和持久化恢复已补充实现，
具体能力、接线方式和未行使的主网交易边界见[适配器说明](papertrade.md)。
新增执行路径支持保证金开仓、整仓平仓和排队取消；不把本地集成测试当作真实资金下单验收。

## 验证范围

Rust 签名测试与独立 `eth-account==0.13.7` 向量比较。
HTTP 测试覆盖回执身份不匹配、损坏响应、限流、冲突、失败状态、过期和连接失败，
全部访问本地 mock server。Python 测试验证 Entropy 的市场筛选、路由、示例默认行为及原生 node 构造。
测试命令为：

```bash
cargo test -p nautilus-papertrade --locked
cargo clippy -p nautilus-papertrade --all-targets --locked -- -D warnings
cd python
uv run --no-sync pytest tests/unit/adapters/hyperliquid/test_entropy.py tests/unit/adapters/hyperliquid/test_hyperliquid_factories.py -q
```

这些测试不使用真实资金，不构成真实签名提交、成交、清算、资金队列或跨链到账验收。
