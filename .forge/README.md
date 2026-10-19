# Forge 计划索引 — nautilus_trader (fork)

本目录管理 nautilus_trader fork 内下游附加层的 forge 实现计划：`docs_zh/` 中文文档，以及 Guild 自维护的 SoDEX 场所适配器。

> 注：本仓库为 `nautechsystems/nautilus_trader` 的 fork。forge 计划只覆盖下游自维护的内容：`docs_zh/`、`scripts/sync_docs_zh.py`，以及 Guild 自维护的 `crates/adapters/sodex/`（含其示例与测试）。上游已有的源码文件不在计划的修改范围内；某项工作若只能靠改上游已有文件完成，计划停下回报，不改。

## 计划列表

| 编号                                                                | 计划                | 状态                        | 创建日期       |
| ----------------------------------------------------------------- | ----------------- | ------------------------- | ---------- |
| [01](plans/2026-06/01-sync-docs-zh.md)                            | docs_zh 增量同步流程固化  | ✅ Completed               | 2026-06-14 |
| [02](plans/2026-10/02-sodex-inferred-fills-and-account-stream.md) | SoDEX 成交以真实回报进入引擎 | 🔲 Not started（第一次修订，待复核） | 2026-10-19 |

## 状态标记

| 标记  | 含义          |
| --- | ----------- |
| 🔲  | Not started |
| ⏳   | In Progress |
| ✅   | Completed   |
| ❌   | Blocked     |
