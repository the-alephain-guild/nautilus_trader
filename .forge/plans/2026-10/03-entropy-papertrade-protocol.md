# Entropy 接入与 Papertrade 协议适配

日期：2026-10-21。开始时 revision 为 `ac86ec3c5f`，工作树为空。
用户要求分析两个平台与 Hyperliquid 适配器的关系，并在本 fork 编写相应逻辑；用户随后明确要求继续网络搜索 Papertrade 的 API 资料。

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
