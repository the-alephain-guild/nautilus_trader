# Forge 计划索引 — nautilus_trader (fork)

本目录管理 nautilus_trader fork 内下游附加层的 forge 实现计划：`docs_zh/` 中文文档，以及 Guild 自维护的场所接入。

> 注：本仓库为 `nautechsystems/nautilus_trader` 的 fork。forge 计划只覆盖下游自维护的内容：`docs_zh/`、`scripts/sync_docs_zh.py`，以及 Guild 自维护的 `crates/adapters/sodex/`（含其示例与测试）。上游已有的源码文件不在计划的修改范围内；某项工作若只能靠改上游已有文件完成，计划停下回报，不改。

计划 03 按用户 2026-10-21 的明确接入要求，另增加 `crates/adapters/papertrade/`、
Hyperliquid 下的 Entropy 发现函数及对应示例和测试，并调整根 `Cargo.toml`、`Cargo.lock`、
`Makefile` 的 workspace 与测试登记。这是该任务的限定范围，不扩大其它计划的修改权限。

用户续做授权另覆盖原生 `LiveNode` 接线所需的 `crates/pyo3/`、生成的 Python stubs、
`scripts/strip-adapter-env.bash` 和对应集成测试；不包含真实资金交易与外部发布。

## 计划列表

| 编号                                                                | 计划                               | 状态                    | 创建日期       |
| ----------------------------------------------------------------- | -------------------------------- | --------------------- | ---------- |
| [01](plans/2026-06/01-sync-docs-zh.md)                            | docs_zh 增量同步流程固化                 | ✅ Completed           | 2026-06-14 |
| [02](plans/2026-10/02-sodex-inferred-fills-and-account-stream.md) | SoDEX 成交以真实回报进入引擎                | 定稿，实施待授权              | 2026-10-19 |
| [03](plans/2026-10/03-entropy-papertrade-protocol.md)             | Entropy 与 Papertrade LiveNode 接入 | 实现与本地恢复验收完成；真实资金交易未行使 | 2026-10-21 |

## 状态标记

| 标记  | 含义          |
| --- | ----------- |
| 🔲  | Not started |
| ⏳   | In Progress |
| ✅   | Completed   |
| ❌   | Blocked     |
