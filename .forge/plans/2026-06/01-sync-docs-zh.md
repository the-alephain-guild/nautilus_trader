# 01 - docs_zh 增量同步流程固化

> **Status**: ✅ Completed
> **Created**: 2026-06-14
> **Completed**: 2026-06-14
> **Project**: nautilus_trader (fork) — docs_zh 中文文档层
> **For Claude**: Use `/forge:execute` to implement this plan.

## 上下文 (Context)

- 本仓库为 `nautechsystems/nautilus_trader` 的 fork，`docs/` 为英文上游源，`docs_zh/` 为下游维护的中文镜像层（upstream 无此目录）。
- 本计划固化"上游 docs 变动 → 下游 docs_zh 同步翻译"流程，源于 2026-06-14 一次全量同步（merge `upstream/develop @ 11eeb0019a`，翻译 99 文件）暴露的痛点：fetch 走 SSH、登录中断、大文件截断、需磁盘实证校验、结构重构迁移人工注释、日韩文质量门。
- 设计经 brainstorming 三轮确认。
- 前置事实（已实证）：fork 无 `.forge/`、`.claude/` 目录；`.claude/` 被 fork `.gitignore` 忽略（命令文件需 `git add -f`）；`.forge/`、`docs_zh/.sync-state`、`scripts/*.py` 可正常跟踪；Python 工具链有 ruff（format+lint+isort）；`scripts/` 现有脚本以 shell 为主，本脚本为首个 `.py`。
- Step 1.5 契约 gate：本计划纯新建，无已发布 MCP/struct 契约依赖，主要依赖 git CLI 与文件系统，已核对 docs/docs_zh 结构 99=99。

## 目标 (Goal)

提供一条可重复执行的 `/sync-docs-zh` 命令，让上游 docs 变动后，下游能增量、可校验、保护人工注释地把中文文档同步到位。

## 架构 (Architecture)

三部件分离：`scripts/sync_docs_zh.py`（确定性：检测增量 / 校验完整性 / 对照结构 / 推进状态）+ `.claude/commands/sync-docs-zh.md`（Claude 编排 5 阶段 + 派翻译子代理 + 把守 review gate）+ `docs_zh/.sync-state`（记录上次同步的 upstream commit 作为 diff 基准）。重构映射记于 `docs_zh/.sync-overrides`。

## 关键设计决策 (Key Design Decisions)

| 问题    | 决策                                                           | 理由                                                            |
| ----- | ------------------------------------------------------------ | ------------------------------------------------------------- |
| 固化形态  | slash command 编排 + Python 检测脚本                               | 翻译必须 Claude 做、流程多阶段含人工 gate，命令是唯一可承载形态；确定性部分脚本化保证可靠可测         |
| 增量检测  | `.sync-state` 记 commit + `git diff base..HEAD -- docs/{三目录}` | 最精确，能捕捉"内容改了但行数没变"；比哈希 manifest 简单                            |
| 自动化边界 | 翻译+校验全自动，停在 commit 前 review gate                             | 冲突/结构重构需人判断，自动暂停；commit 前人工 review diff                       |
| 脚本语言  | Python                                                       | 需解析 git diff / 生成 JSON / 正则校验，远优于 shell；项目重度 Python + 已配 ruff |
| 放置    | 全部放 fork 内                                                   | docs_zh 已确立 fork 内中文层模式，同性质下游附加，upstream 无对应路径不冲突             |
| 结构重构  | 不自动猜映射，暂停人工确认后记入 `.sync-overrides`                           | 改名/拆分/删除语义脚本无法可靠推断（本次 instruments→目录等）                        |

## 承载决策 (Capability Hosting Decision)

| 能力             | plan mode?    | hook?                      | CLAUDE.md?         | 现有 skill flag? | 新 skill/command? | 决策                                   |
| -------------- | ------------- | -------------------------- | ------------------ | -------------- | ---------------- | ------------------------------------ |
| docs_zh 增量同步编排 | 否（常态流程非一次性规划） | 否（需人工发起+多阶段+人工 gate，非事件触发） | 否（被动上下文，无法编排/派子代理） | 无              | 是                | slash command 编排 + Python 脚本承载确定性子任务 |

## 文件清单 (File Inventory)

| 文件路径                                      | 操作                   | 描述                                               |
| ----------------------------------------- | -------------------- | ------------------------------------------------ |
| `scripts/sync_docs_zh.py`                 | Create               | 检测/校验脚本：detect / verify / structure-check / bump |
| `.claude/commands/sync-docs-zh.md`        | Create（`git add -f`） | slash command 编排定义（`.claude` 被 gitignore）        |
| `docs_zh/.sync-state`                     | Create               | 同步状态，初值 `upstream_commit: 11eeb0019a`            |
| `docs_zh/.sync-overrides`                 | Create               | 重构映射表（JSON），种子记录本次 3 个重构                         |
| `.forge/README.md`                        | Create               | fork 内 forge 计划索引                                |
| `.forge/plans/2026-06/01-sync-docs-zh.md` | Create               | 本计划文件                                            |

## 实现任务 (Tasks)

### Task 1: forge 基础设施 + 状态种子文件

**Files**: Create `.forge/README.md`、`docs_zh/.sync-state`、`docs_zh/.sync-overrides`

- **Step 1 证伪**: `test ! -f docs_zh/.sync-state` 成立（尚未创建）。
- **Step 2 实现**:
  - `.sync-state`（JSON）写 `upstream_commit: 11eeb0019a` + `synced_at: 2026-06-14` + `upstream_ref: upstream/develop`。
  - `.sync-overrides`（JSON）记录 detect 默认映射的例外：本次 3 个重构（`instruments.md`→`instruments/` 拆分、`orders.md`→`orders/` 拆分、`coinbase_intx.md`→`coinbase.md` 改名），标注 `status: done`，作为格式样例与历史。
  - `.forge/README.md` 建索引，列本计划（已在 planning Step 5 随计划 commit 创建）。
- **Step 3 证实**: `python3 -c "import json; d=json.load(open('docs_zh/.sync-state')); assert d['upstream_commit']=='11eeb0019a'"` 通过；`.sync-overrides` 可被 json 解析。
- **Step 4 提交**: `git add .forge/README.md docs_zh/.sync-state docs_zh/.sync-overrides`。

### Task 2: detect 子命令

**Files**: Create `scripts/sync_docs_zh.py`（detect）

- 契约：`python3 scripts/sync_docs_zh.py detect [--base <commit>] [--json]`
  - 读 `.sync-state` 基准（或 `--base`），跑 `git diff --name-status base..HEAD -- docs/concepts docs/developer_guide docs/integrations`。
  - 默认映射 `docs/X`→`docs_zh/X`，套用 `.sync-overrides` 例外。
  - 输出待翻译清单 JSON（每项 `en/zh/exists/en_lines/status∈{added,modified}`）。
  - 单独区段列"结构异常"（deleted/renamed/copied，即 D/R/C），需人工确认，不进自动翻译清单。新增文件（A，含新目录里的）自动进 pending。
- **Step 1 证伪**: `python3 scripts/sync_docs_zh.py detect` 报错（未实现）。
- **Step 4 证实**: 当前 `.sync-state=11eeb0019a` 且 HEAD 已同步完 → detect 输出空待翻译清单（diff 为空，天然回归场景）；`--base HEAD~N` 跨越某次 docs 改动 → 输出对应文件。
- 提交。

### Task 3: verify 子命令

**Files**: Modify `scripts/sync_docs_zh.py`（verify）

- 契约：`python3 scripts/sync_docs_zh.py verify [files...] [--all]`，每文件校验：行数下限（中文 ≥ 英文×0.45 或绝对下限）、代码围栏 ``` 偶数、admonition `:::` 偶数、无日文假名/韩文谚文（U+3040–30FF / U+AC00–D7AF）。全通过 exit 0，有问题列清单 exit 1。
- **Step 1 证伪**: 对构造的坏文件（奇数 ``` ）verify 尚不存在/不报错。
- **Step 4 证实**: `verify --all` 对当前 99 文件 exit 0；构造截断文件 → 报"代码围栏不配对" exit 1；构造含「あ」文件 → 报日韩文 exit 1。
- 提交。

### Task 4: structure-check + bump 子命令

**Files**: Modify `scripts/sync_docs_zh.py`（structure-check, bump）

- structure-check：对照 docs/docs_zh 三目录文件集，报缺译/多余，99=99 时 exit 0。
- bump：`bump [--commit <c>]` 把 `.sync-state` 更新为给定（默认 `git rev-parse upstream/develop`）commit + 当日日期。
- **Step 1 证伪**: 两子命令未实现报错。
- **Step 4 证实**: structure-check 当前输出 99=99 exit 0；`bump --commit deadbeef` 后 `.sync-state` 含 deadbeef，再还原为 11eeb0019a。
- 提交。

### Task 5: slash command 编排定义

**Files**: Create `.claude/commands/sync-docs-zh.md`（`git add -f`）

- 内容：5 阶段编排——① 预检（确认 upstream 为 SSH）+ fetch 重试 + 后台跑；② merge + 冲突暂停；③ `detect` + 结构异常暂停等人工；④ 翻译子代理并行 + `verify` 自愈（最多 3 轮）+ 破损文件先 `git restore` 再重译；⑤ review gate 停 commit 前 + 确认后 `bump`。内置翻译子代理 prompt 模板（简体中文 / 保留结构 / 代码块 / admonition / 链接锚点 / 技术术语英文 / H1 用「中文 (English)」/ 大文件分段写+自检 / update 保留人工提示框 / prior 迁移）。人工注释保护：重构暂停点用"提取带中文标题 admonition 对照新结构"核查法。
- **Step 1 证伪**: `test ! -f .claude/commands/sync-docs-zh.md`。
- **Step 4 证实**: 文件存在且含 5 阶段标题、翻译规范、`git add -f` 提示、调用 `sync_docs_zh.py` 各子命令的步骤（grep 锚点断言）。
- 提交（`git add -f`）。

### Task 6: 文档收尾 (close-out)

**Files**: Modify 本 plan、`.forge/README.md`

- 动作：plan Status ⏳→✅ + Completed 日期；`.forge/README.md` 索引状态更新；完成报告章节；`git commit -m "docs(nautilus-fork): mark plan 01 completed"`。
- 无版本号文件，跳过版本升级；fork 无 `verification.md`，跳过该项。

## 验证清单 (Verification)

- [ ] `python3 scripts/sync_docs_zh.py verify --all`: PASS（当前 99 文件）
- [ ] `python3 scripts/sync_docs_zh.py structure-check`: 99=99 PASS
- [ ] `python3 scripts/sync_docs_zh.py detect`: 已同步态输出空清单
- [ ] `ruff check scripts/sync_docs_zh.py && ruff format --check scripts/sync_docs_zh.py`: PASS
- [ ] slash command 含 5 阶段 + 翻译规范 + 人工 gate（grep 锚点）
- [ ] 无死代码/未使用配置字段
- [ ] 所有引用的前置原语契约均有 file:line 证据锚（Step 1.5：本计划纯新建，无外部契约依赖）

## 进度追踪 (Progress)

| Task                   | Status | Completed  | Notes                    |
| ---------------------- | ------ | ---------- | ------------------------ |
| 1 基础设施+状态              | ✅      | 2026-06-14 | 2672c7e1d6；YAML→JSON 改进  |
| 2 detect               | ✅      | 2026-06-14 | 296218447d               |
| 3 verify               | ✅      | 2026-06-14 | d0eccd9889               |
| 4 structure-check+bump | ✅      | 2026-06-14 | 7826721ab8               |
| 5 slash command        | ✅      | 2026-06-14 | 0a69632fdf（`git add -f`） |
| 6 close-out            | ✅      | 2026-06-14 | 本 commit                 |

## 偏离与改进日志 (Deviations & Improvements)

| 类型          | 位置                       | 描述                                                                                                                                                                  | 已批准       |
| ----------- | ------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------- |
| IMPROVEMENT | Task 1 / sync_docs_zh.py | `.sync-state`/`.sync-overrides` 由 YAML 改为 JSON：实测环境无 `pyyaml`，独立工具脚本应零第三方依赖，JSON 用标准库即可解析。设计本质不变（仅序列化格式）。                                                           | ✅ (执行时确认) |
| IMPROVEMENT | Task 2 / sync_docs_zh.py | 用 `shutil.which("git")` 取绝对路径消除 S607、per-line `# noqa: S603` 处理 subprocess，沿用项目惯例；file-level `# ruff: noqa: RUF001/002/003` 声明有意中文标点。均不改 upstream 的 pyproject.toml。 | ✅         |
| IMPROVEMENT | Task 4 / sync_docs_zh.py | `bump` 日期用 `_today_utc()`（timezone-aware UTC）替代 `date.today()`，满足项目 flake8-datetimez (DTZ011)。                                                                      | ✅         |
| IMPROVEMENT | 自省 R1 / 命令+计划            | 修正"新目录"文档-实现不一致：detect 把新增文件（A，含新目录里的）归 pending 自动翻译，仅 D/R/C 为 anomalies。文档措辞同步纠正（commit 64680a0b0f）。                                                               | ✅         |

## 完成报告 (Close-out Report)

- **完成日期**: 2026-06-14
- **总 Task 数**: 6（全部 ✅）
- **偏离数**: 4（全部 IMPROVEMENT，见上表）
- **验证结果**: 全部通过
  - `verify --all` → 99 文件全 PASS
  - `structure-check` → docs/docs_zh 99=99 完全对应
  - `detect`（已同步态）→ 0 待翻译
  - `ruff check` / `format --check` → All checks passed
- **实际产出**:
  - `scripts/sync_docs_zh.py`（285 行，零第三方依赖）：detect / verify / structure-check / bump 四子命令
  - `.claude/commands/sync-docs-zh.md`：5 阶段编排命令（`git add -f` 入库）
  - `docs_zh/.sync-state`（基准 11eeb0019a）+ `docs_zh/.sync-overrides`（本次 3 个重构映射）
  - `.forge/`：fork 内首个 forge 计划目录
- **跨仓库同步**: 无（全部落在 nautilus_trader fork 内）
- **遗留项**: 无
  - 工具链备注：fork 的 uv 版本 pin 与本机不匹配，ruff 经 `miniforge ruff` 调用；执行者亦可用项目 `make ruff` / `make format`。
