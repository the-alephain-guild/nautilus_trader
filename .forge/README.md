# Forge 计划索引 — nautilus_trader (fork)

本目录管理 nautilus_trader fork 内下游附加层（主要是 `docs_zh/` 中文文档）的 forge 实现计划。

> 注：本仓库为 `nautechsystems/nautilus_trader` 的 fork。forge 计划仅覆盖下游自维护的内容（`docs_zh/`、`scripts/sync_docs_zh.py`、Guild 自维护的 `crates/adapters/sodex/` 等）；需要触及上游代码时，在计划里单独写明理由与改动面。

## 计划列表

| 编号                                                                | 计划                | 状态             | 创建日期       |
| ----------------------------------------------------------------- | ----------------- | -------------- | ---------- |
| [01](plans/2026-06/01-sync-docs-zh.md)                            | docs_zh 增量同步流程固化  | ✅ Completed    | 2026-06-14 |
| [02](plans/2026-10/02-sodex-inferred-fills-and-account-stream.md) | SoDEX 成交以真实回报进入引擎 | 🔲 Not started | 2026-10-19 |

## 状态标记

| 标记  | 含义          |
| --- | ----------- |
| 🔲  | Not started |
| ⏳   | In Progress |
| ✅   | Completed   |
| ❌   | Blocked     |
