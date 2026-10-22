# Papertrade LiveNode 适配器

本适配器已提供原生 Rust 数据客户端、执行客户端、factories 和 Python 注册，
可以通过 `nautilus_trader.adapters.papertrade` 接入 `LiveNode`。
实现以官方前端接口和公开链上交易为依据；本次未找到官方发布的版本化 trade SDK。

## Papertrade 暴露的接口

| 接口                                                        | 用途                        | 本次证据                                       |
| --------------------------------------------------------- | ------------------------- | ------------------------------------------ |
| `GET /state/trading`                                      | 市场、精度、暂停状态、杠杆及交易限额        | 官方 origin 实际返回 JSON                        |
| `GET /state/user/live?wallet=0x...`                       | 账户 SSE 快照、连续增量和心跳         | 实测 HTTP 200 与 `text/event-stream`，包含非空多空仓位 |
| `GET /query/accounts/{wallet}/trades?filter=all&limit=75` | 分页平仓、清算、入场及退出价格、实现盈亏和费用   | 实际读取非空历史记录                                 |
| `POST /intents`                                           | 开仓、平仓及 session key 注册     | 官方前端的签名结构与请求格式；没有发送真实账户的写请求                |
| `POST /intents/{intentId}/cancel`                         | 取消 relayer 中的排队意图         | 官方前端格式；本地 HTTP harness 验证                  |
| HyperEVM JSON-RPC                                         | 逐意图 batch 结果与 Exchange 事件 | 实际读取开仓、平仓交易及日志                             |

查询参数必须排序生成 canonical URL。成交历史必须翻到 `nextCursor=null`；
重复 cursor、非成功 HTTP、损坏记录和超出配置页数均返回错误，不返回“空历史”。
`GET /intents/{id}` 探针返回 404，恢复流程不依赖这个路径。
来源、地址和远端版本记录在
[`sources.json`](../../crates/adapters/papertrade/test_data/sources.json)。

[官方架构说明](https://docs.papertrade.xyz/how/architecture/)写明：
“Papertrade's relayer submits every open and close.”
因此本适配器使用 HyperEVM 意图协议，不能把请求发送到 Hyperliquid 的 `/exchange`。
[官方 session key 说明](https://docs.papertrade.xyz/learn/session-keys/)定义了交易密钥权限和有效期。

## LiveNode 接线

公开行情不需要凭据，可运行：

```bash
uv run --project python --no-sync python examples/live/papertrade/data_tester.py
```

账户连接示例为
[`account_monitor.py`](../../examples/live/papertrade/account_monitor.py)。
它连接数据和执行客户端、执行账户对账，但没有安装下单策略：

```bash
uv run --project python --no-sync python examples/live/papertrade/account_monitor.py \
  --wallet YOUR_MASTER_WALLET_ADDRESS \
  --build-id CURRENT_OFFICIAL_FRONTEND_BUILD \
  --journal target/papertrade/account.redb
```

事先通过安全的环境变量渠道设置 `PAPERTRADE_SESSION_PRIVATE_KEY`。
这里需要已在对应钱包注册的 session key；适配器不自动注册、不读取主钱包私钥、不自动续期。
连接时会核对钱包快照中的注册记录，并要求密钥至少还有一小时有效期。
错误钱包即使返回空账户，也不能绕过这个检查。

配置类为 `PapertradeDataClientConfig` 和 `PapertradeExecutionClientConfig`；
对应工厂为 `PapertradeDataClientFactory` 和 `PapertradeExecutionClientFactory`。
数据、订单和仓位统一使用 `PAPERTRADE` venue，例如
`BTC-USD-PERP.PAPERTRADE`，OMS 为 `Hedging`。
该场所可以同时存在同一标的的独立多仓和空仓，不能压成净头寸。

## 支持的交易语义

- 开仓支持独立 `MARKET GTC` 意图。设置 `quote_quantity=True`，
  订单数量表示美元名义金额；适配器按配置杠杆换算保证金。
  比如名义金额 1000、杠杆 10，对应保证金 100。
- 开仓前核对市场状态、intake、当前杠杆范围、最小保证金、最小与最大名义金额、
  可用现金和本地未决请求占用。OI 计数使用场所原生单位，不把美元金额直接加到 OI；
  合约在实际执行时校验本次增量。
- `OpenPosition.size` 是 18 位小数的保证金。`margin_to_raw` 拒绝舍入和溢出，
  不把抵押币 ERC-20 的小数位当作这个字段的小数位。
- 开仓确认后，以实际入场价计算标的数量，再由真实成交报告把订单转为实际 base quantity。
  所有数量计算使用 Decimal 和原生定点类型。
- 平仓设置 `reduce_only=True`，并以原生 base quantity 提交完整仓位数量。
  使用 `papertrade_position_id` 参数或对应的 venue position ID 明确目标；
  方向、标的和数量必须与当前快照一致，拒绝部分平仓和反向目标。
- 限价、修改和条件订单不属于该协议能力。修改请求会收到明确拒绝；
  订单列表会被逐单拒绝，避免把依赖关系拆成独立交易。
- 单笔取消和批量取消针对排队意图，不是对已确认仓位进行反向交易。
  `cancel_all_orders` 只处理该策略、该标的及方向过滤后、本地 journal 中的未决意图。

协议层还保留主钱包注册 session key 的签名函数，供独立授权流程使用，
执行客户端本身不调用它。存款、提现、approve、桥接与 PAPER 质押不在本实现中。

## 行情、成交与结算的区别

数据客户端加载官方市场配置，发布用于合约定价的 BBO 中点参考报价。
参考报价的 bid/ask 数量都是零，没有伪造 Papertrade 撮合深度。
启动时加载参考报价，后续按配置轮询已订阅标的；暂停结算市场使用场所公布的冻结价格。
`reference_info_url` 可显式配置，默认为 Hyperliquid 官方 `/info`。

成交证据来自账户仓位、完整成交历史和必要的链上结果，绝不来自这条参考报价。
`queued`、`prepared`、`submitted` 或单独的 `confirmed` 回执均不足以生成 fill。
一笔 batch 的交易状态成功，也不证明其中每个用户意图都成功。

原生引擎以 USDC 的六位货币精度显示金额；源数据中的 18 位整数保存在 journal。
原生 base quantity 按当前构建的定点精度向零取整。
平仓 fill 的 settlement adjustment 同时包含非线性收益扣减、清算 payoff cap 和显式费用，
使原生已实现净盈亏与协议源记录在 USDC 显示精度内一致。
它不是按名义金额计算的常规手续费率；不要把该字段当作独立费率研究数据。
未实现盈亏仍按原生 instrument 模型计算，协议最终结算以源记录为准。

现金账户状态只包括可支配余额和以现金支付的锁定保证金。
`queued` 债权、PAPER 及债权支付的保证金不计为可用现金；
原始账户快照保留这些字段，供单独的资金与队列核算使用。

## 成交对账与重启恢复

journal 使用已有 redb，提交采用 immediate durability，并持有数据库的独占进程锁。
在 Unix 上，journal 文件设为 0600；它包含签名请求，不能当作普通公开日志。
每个 journal 固定绑定链、Exchange 代理、钱包和账户 ID。

发送请求前，nonce 与完整命令、原始签名载荷、意图 ID、扫描起点在同一事务中提交。
相同 client order ID 的相同命令在网络失败、重连及重启后均复用已有记录，不再次签名或发送；
同 ID 不同载荷明确报错。晚到的接收回执不能覆盖已确认的成交或终态。

账户 SSE 按 `baseAccountRevision` 连续应用增量，核对钱包、仓位唯一性、
锁定保证金与可用资金等不变量。同一 revision 不同载荷、版本缺口、非法帧或断流会撤销当前状态。
重连必须先得到完整快照，旧缓存不能作为新的权威状态。
账户流健康状态与对账成功独立维护；定时轮询和单独账户查询不能恢复已失效的流。
等待下一帧的读取超时跨轮询保持，静默断流同样会停止交易。

恢复同时读取原始 journal、新账户快照和完整分页历史。
最后通过验证的账户覆盖区块、时间和 revision 在重启后继续作为回退检查基线；
较旧快照不能覆盖该检查点，检查点也不会因为一轮失败对账而推进。
对未决意图，从持久化区块起点扫描 BatchExecutor 对应 ID 的结果日志。
只有实际扫描过的区间才推进扫描位置；排队状态不会推进尚未读取的链上区间。
扫描上界取市场与账户覆盖区块的较小值；本地平仓 receipt 也不能超过该区块。
恢复出的开仓必须能在当前仓位或终结历史中找到，历史关闭时间不得超过账户时间覆盖，
避免把较新的成交与较旧的余额、空仓位列表拼成完整报告。

链上开仓结果返回仓位 ID，再与 Exchange 开仓事件中的钱包、标的、方向、保证金、
杠杆和入场价核对。这样，即使进程丢失接收回执、仓位又在停机期间关闭，
仍能恢复原开仓身份，再使用真实终结记录恢复关闭。
本地平仓同样要求意图级成功结果与目标仓位的 Exchange 事件匹配，不能仅凭仓位消失归因。
存在本地平仓候选而 receipt 暂不可读时，该轮对账保持不可用，不抢先发布外部平仓；
证据恢复后只以已绑定的本地命令发布成交，避免同一 Trade ID 更换订单身份。
某一次链上尝试失败也不会被当作签名意图的最终拒绝：有效期内 relayer 仍可能再次执行同一个意图。
最终失败状态取自已绑定意图 ID 的 relayer 终态，而成功成交仍需实际执行证据。

Trade ID 由事件种类、仓位 ID 和关闭交易 hash 稳定生成，长度适配原生 36 字符约束；
重复源帧和重新对账不会创造新的经济成交。
发现已观察仓位消失却没有终结证据、历史冲突或读取失败时，不发布“空仓”。
平仓历史写入前必须与已观察仓位的标的、方向、入场价、保证金、杠杆和开仓时间一致，
历史标的与价格尺度也须匹配市场配置；冲突记录不会写入 journal。
无法证明的意图保留未决状态，并将 mass status 的 `reports_complete` 设为 false；
新开仓会被阻断。没有用超时、HTTP 409 或链上交易整体成功冒充逐意图结论。

journal 恢复的是适配器的交易身份和证据，不代替策略自身的内部状态持久化。
已有引擎缓存的部署应保留其正常持久化配置；新缓存通过 Native reconciliation 恢复账户和仓位。
如果删除 journal、切换钱包或改变账户归属，不能再声称是同一执行实例的无损恢复。

## 验证与尚未行使的边界

本次验证包括独立 `eth-account==0.13.7` 签名向量、实际公开非空账户和成交记录、
真实开平仓 receipt、SSE 增量与重连、持久化重开、丢回执和重复命令测试，
以及通过实际 Python `LiveNode`、原生执行引擎和缓存恢复多空仓位的本地集成测试。
公开数据节点还实际收到 `QuoteTick` 并正常停止。

这不等于真实账户下单验收：本次没有使用资金账户私钥、提交生产签名意图或操作资金。
主网签名接收、真实开平仓、控制权竞争和长期运行仍需由有权限的测试账户单独验收。
公开前端接口不是已发现的版本化兼容承诺；schema 或链上事件变化时实现会报错并停止相应对账，
而不会返回默认余额或猜测成交。

官方地址文档与读取的前端 deployment 对 collateral 地址存在差异，
本次没有核实两个地址当前的链上资金角色；资金操作仍未实现，后续必须先核验再接入。
