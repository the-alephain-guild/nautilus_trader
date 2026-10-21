# Entropy 接入与 Papertrade 协议适配

日期：2026-10-21。开始时 revision 为 `ac86ec3c5f`，工作树为空。
用户要求分析两个平台与 Hyperliquid 适配器的关系，并在本 fork 编写相应逻辑；用户随后明确要求继续网络搜索 Papertrade 的 API 资料。

## LiveNode 扩展实施（用户 2026-10-21 续做授权）

续做基线为 `d827cf79c8`，工作树为空。用户明确要求补齐 `LiveNode`、成交、仓位对账与重启恢复。
本节扩展原来的协议层范围，不能把原协议层交付误报为完整适配器。

- [ ] 核验并实现市场配置、分页成交历史、账户 SSE 快照及连续增量；保存公开响应 fixture。
- [ ] 使用已有 redb 依赖持久化账户命名空间、nonce、本地命令、签名意图与确认过的成交。
- [ ] 实现原生 Rust 数据和执行客户端、factories 与 PyO3 注册。
- [ ] 实现以意图 ID 绑定链上 batch 结果的成交对账，覆盖清算、外部操作及历史分页。
- [ ] 验证断流重连、版本缺口、重复事件、提交结果未知和重启恢复；不重签替代未知订单。
- [ ] 构建 Python 扩展并验证真实 `LiveNode` 接线与公开只读路径，更新说明与验收边界。

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
- [ ] Papertrade 的完整 `LiveNode` 数据与执行客户端尚未实现，不属于已完成或已通过实盘验收的能力。

Papertrade 的最后一项需要定义数量订单到保证金仓位的语义，并实现账户流、链上成交证据、
清算、付款队列及持久化恢复。当前协议层只有签名与 relayer 回执，不足以生成引擎成交事件。
详细的缺项、原因、影响和下一步见 `docs_zh/integrations/papertrade.md`。

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
