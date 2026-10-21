# Entropy 与 Papertrade 的接入分析

本次以 2026-10-21 的公开文档、官方前端和只读 API 为证据，对照仓库
`ac86ec3c5f` 的 Hyperliquid 适配器。结论是：Entropy 直接复用现有适配器；
Papertrade 需要独立协议适配，当前只实现了底层签名和 relayer 提交逻辑，还不是完整的实盘场所客户端。

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
| 本次接入结果  | 复用原生工厂，增加发现函数、行情和执行示例               | 新增独立 Rust 协议 crate，不注册到 `LiveNode`         |

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

## Papertrade 实现方式

新增 crate 位于
[`crates/adapters/papertrade`](../../crates/adapters/papertrade/src/lib.rs)，详细协议与未完成边界见
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

Papertrade 仍欠账户流、链上成交证据、订单与仓位映射、动态风险校验和持久化恢复；
原因、影响和补齐路线逐项写在[协议说明](papertrade.md)中。
这些缺失意味着协议层测试通过仍不足以宣称可以通过 `LiveNode` 实盘交易。

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
