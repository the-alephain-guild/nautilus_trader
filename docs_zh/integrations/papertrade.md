# Papertrade 协议适配层

本 crate 实现 Papertrade 在 HyperEVM 上的签名意图协议，包括开仓、按仓位 ID 批量平仓、
主钱包注册 session key、取消排队意图，以及显式调用官方 relayer 的 HTTP 客户端。
它不是已接入 `LiveNode` 的 Nautilus `ExecutionClient`，也没有 Python factory。

## 与 Hyperliquid 的区别

[官方架构说明](https://docs.papertrade.xyz/how/architecture/)写明：
“Papertrade's relayer submits every open and close.”
因此，Hyperliquid 的 `/exchange` 下单、`cloid`、订单簿撮合和撤单不能代替这里的意图协议。
`Exchange.openPositionBySig` 也不能由普通用户绕过授权 batch executor 直接调用。

[官方交易说明](https://docs.papertrade.xyz/)描述的是以 HyperCore BBO 中点为价格来源、
与 LP 对手方结算的合成交易。它没有自己的撮合订单簿。Hyperliquid 的 BBO 可以作参考行情，
不能作为 Papertrade 的成交证明，更不能把 Hyperliquid 的深度解释成 Papertrade 的可成交深度。

## 已实现的协议

签名结构与 HTTP 格式来自 2026-10-21 读取的官方前端，来源定位见
[`sources.json`](../../crates/adapters/papertrade/test_data/sources.json)。这是前端实现观察，不是官方对长期兼容性的承诺。
实现没有引入第三方机器人代码。

| 操作             | EIP-712 primary type | HTTP 路径                           | JSON action          |
| -------------- | -------------------- | --------------------------------- | -------------------- |
| 开仓             | `OpenPosition`       | `POST /intents`                   | `openPosition`       |
| 平仓             | `Close`              | `POST /intents`                   | `closePositions`     |
| 注册 session key | `RegisterSessionKey` | `POST /intents`                   | `registerSessionKey` |
| 取消排队意图         | `CancelIntent`       | `POST /intents/{intentId}/cancel` | 无，直接发送签名字段           |

交易和注册使用 chain ID `999`、Exchange 代理
`0x6cd5661646289fb6e65ea5c032310fded797d0a2`，domain name 原样为 `String`，
version 为 `1`。取消使用相同链和代理，但 domain name 为 `PaperTrade Intent Cancel`，
version 为 `2`。domain 名字虽然不常见，也是签名字节的一部分，不能改成品牌名。

`OpenPosition.size` 是 **18 位小数的抵押保证金**，不是 BTC/ETH 数量，也不是 ERC-20 的原始单位。
例如，保证金 `12.345` 对应 `12345000000000000000`。
`margin_to_raw` 只做精确换算，拒绝需要舍入的输入和溢出。
`instrumentId`、`leverage`、`nonce`、`deadline` 等整数在线上 JSON 中是十进制字符串，
布尔值保留为 JSON boolean。

意图 ID 使用：
`keccak256(keccak256("papertrade:intent-id:v2") || EIP712_digest || signer_address)`。
这里使用的是实际签名者地址；session key 和主钱包签同一个消息时不能混用 ID。
取消请求沿用被取消意图的 ID，不用取消签名的 digest 生成新交易 ID。
一次平仓允许 1 至 12 个不重复的仓位 ID。取消回执的 action 仍绑定原交易 action。

`PapertradeSigner` 不读取环境变量，调用者显式传入已有的 Alloy `PrivateKeySigner`。
注册时要求签名者就是 `user`，防止 session key 自授权。session key 的链上有效性、
scope 和是否被撤销，仍由协议校验；本地签名成功不表示已获得交易权限。

## 使用边界

调用者必须持久化并协调 nonce，然后保存原始消息、签名请求和意图 ID，再提交。
nonce 不是毫秒时间戳，也不是 Hyperliquid L1 action nonce。
本 crate 不分配 nonce、不保存钱包、不负责多进程锁，也不在结果未知时生成替代意图。

`PapertradeRelayerClient::new(build_id)` 使用官方 origin，要求调用者显式提供当前前端
`X-Papertrade-Build` 标识。客户端拒绝重定向，使用共享 HTTP transport 的 secret-body 路径，
按每秒一个请求进行本地节流，不添加应用层自动重试。这个节流值不是平台公布的限额。
本地测试通过 `with_origin` 指向 loopback mock server。

`submit` 的结果保留证据级别：

- `Acknowledged` 表示收到了与本地意图 ID 和 action 相符的 relayer 回执，同时保留服务器 state。
  `queued`、`prepared`、`submitted` 均不表示成交；`failed` 等失败状态也原样保留。
- `Unknown` 表示发送失败、HTTP 非成功状态、响应损坏或回执无法绑定。
  调用者必须用原意图 ID 对账。即使收到 HTTP 409，也不能推断之前的请求没有被接收。
- `confirmed` 仅表示 relayer 报告确认，不能单凭它生成 `OrderFilled` 或增加可提现余额。
  一笔 batch 交易包含多个用户意图，交易收据成功也不等于每个意图成功。

## 尚未完成的 Nautilus 接入

以下工作不在当前协议层中；因此当前 crate 不满足实盘适配器验收条件：

| 欠缺内容                                                 | 当前没有直接接入的原因                                                  | 缺失的后果                     | 后续补齐方式                                           |
| ---------------------------------------------------- | ------------------------------------------------------------ | ------------------------- | ------------------------------------------------ |
| Rust `DataClient`、`ExecutionClient` 和 PyO3 factories | Papertrade 原生操作是保证金开仓和按 ID 平仓，尚未定义为 Nautilus 数量订单的完整契约       | 不能在 `LiveNode` 中配置此场所执行策略 | 明确独立 venue、hedging 仓位标识、数量换算、订单能力和拒绝语义后实现 traits |
| 实时账户与成交对账                                            | 前端使用 `/state/user/live?wallet=...` 的流式状态，尚未实现快照、增量、重连和链上事件关联 | 无法证明数量、成交价、佣金和仓位的完整性      | 采集官方流与链上日志，覆盖部分失败、清算、重启和重复事件                     |
| 动态风控与资金状态                                            | 市场开放状态、杠杆和 OI 限额、可用保证金、queued balance 均会变化                   | 签名层只验证字段范围，不能保证请求满足平台交易条件 | 下单前加载当前配置及账户状态，并在控制面执行实时校验                       |
| 持久化恢复和 session 生命周期                                  | 本 crate 只负责签名与单次提交，不拥有持久化存储                                  | 重启后不能自行完成未知意图恢复或密钥续期      | 使用事务日志协调 nonce、请求和 ID，再以事件证据关闭在途状态               |
| 提现与付款队列结算                                            | 盈利可能先形成排队债权，不能直接当作现金                                         | 完成平仓不等于资金可支配              | 分开记录实现盈亏、可用余额、排队债权与实际到账                          |

没有测试账号、已注册 session key 和资金授权，本次不发送任何生产签名意图，也不操作存提资金。
没有实际运行账户流或链上成交验收，因此离线测试不能证明 relayer 会接收请求或真实资金路径可用。

## 验证

运行 `cargo test -p nautilus-papertrade --locked` 和
`cargo clippy -p nautilus-papertrade --all-targets --locked -- -D warnings`。

签名 fixture 使用独立 Python `eth-account==0.13.7` 生成，与 Rust Alloy 的输出逐字节比较，
覆盖开仓、含大整数数组的平仓、注册和取消。测试密钥由 32 个 `0x11` 字节组成，
只用于公开离线向量，不含真实账户凭据。所有 HTTP 测试只访问 loopback mock server。
