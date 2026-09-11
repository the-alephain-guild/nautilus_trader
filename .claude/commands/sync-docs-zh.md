---
description: 同步上游 docs 变更到 docs_zh 中文文档（增量检测 → 翻译 → 校验 → 提交）
argument-hint: "[--base <commit>] [--full]"
---

# /sync-docs-zh — 上游 docs → 下游 docs_zh 增量同步

把上游 `docs/`（英文）的变更增量同步到 `docs_zh/`（简体中文镜像层）。
确定性工作交给 `scripts/sync_docs_zh.py`，翻译由你（Claude）派并行子代理完成，
最后停在 commit 前等人工 review。

**参数**：

- `--base <commit>`：覆盖 diff 基准（默认读 `docs_zh/.sync-state` 的 `upstream_commit`）。
- `--full`：忽略增量，全量对照 `structure-check` 的缺译清单重译（仅在 `.sync-state` 丢失或大重构后用）。

**关键事实**（执行前须知）：

- 本仓库是 `nautechsystems/nautilus_trader` 的 fork；`docs_zh/` 是下游附加层，upstream 无此目录。
- `.claude/` 被 fork `.gitignore` 忽略——提交本命令或其它 `.claude/` 文件须用 `git add -f`。
- `scripts/sync_docs_zh.py` 零第三方依赖（标准库）。校验脚本本身改动用 ruff（`miniforge ruff` 或项目 `make ruff`）。

---

## 阶段 1：预检 & 取数

1. 确认 `upstream` 走 SSH：`git remote -v | grep upstream`。若为 HTTPS，提示用户——HTTPS 传 develop 大 pack 会反复 `early EOF`；建议 `git remote set-url upstream git@github.com:nautechsystems/nautilus_trader.git`。
2. fetch：`git fetch upstream develop`。**大 pack 耗时**，用后台任务跑（`run_in_background`）+ Monitor/轮询等待，失败重试（最多 3 次）。
3. 记录 fetch 后的 `upstream/develop` HEAD 备用（阶段 5 bump 的目标）。

## 阶段 2：合并

1. `git merge upstream/develop --no-edit`。
2. **若有冲突 → 停止**，列出冲突文件报告用户，等人工解决后再继续。docs_zh 是下游独有目录，正常不与 upstream 冲突；冲突通常在 docs/ 或代码，需人判断。

## 阶段 3：检测增量

1. 跑 `python3 scripts/sync_docs_zh.py detect --json`（`--full` 时改用 `structure-check` 的缺译清单）。
2. 解析 `pending`（待翻译/更新清单）与 `anomalies`（结构异常）。
3. **若 `anomalies` 非空 → 停止**，把删除/改名/复制（D/R/C）列给用户确认映射关系（新增文件——含新目录里的——是 A 状态，已自动进 `pending`，无需人工）：
   - 删除（D）：上游删了某英文文件 → 对应中文该删还是已被新结构取代？
   - 改名/拆分（R，或表现为 D 旧 + A 新）：旧文件 → 新文件/目录的映射是什么？
   - 用户确认后写入 `docs_zh/.sync-overrides`（`type: rename|split|delete` + 路径 + `note`），再重跑 detect。
4. `pending` 为空且无异常 → 已是最新，跳到阶段 5 收尾（无改动则直接结束）。

## 阶段 4：翻译 + 自愈

对 `pending` 清单派**并行子代理**翻译（推荐用 Workflow 编排，每文件一个子代理）。每个子代理遵循下方**翻译规范**。

翻译完成后：

1. 跑 `python3 scripts/sync_docs_zh.py verify <本批文件...>`（或 `--all`）。
2. **对 verify 失败/截断的文件自愈重做**，最多 3 轮：
   - 被部分写入破坏的文件（行数远低于英文、围栏不配对）：先 `git restore <file>` 回干净基线（若该文件是 update 模式且 git 有旧版），再重译。
   - 仍失败的进入下一轮，直到全绿或达 3 轮（剩余失败列给用户）。

### 翻译规范（子代理 prompt 必含）

- 一律**简体中文（zh-CN）**。禁止日语假名、韩文谚文（verify 会拦截）。
- **保留 Markdown 结构**：标题层级、表格、代码围栏、admonition（`:::info`/`:::note`/`:::warning` 等）、frontmatter、HTML、锚点一字不动。
- **代码块内容原样**，不翻译代码本身；代码内英文注释可保留或译。
- **技术术语、类名、API 名、配置键、枚举值保持英文**；关键术语首次出现用「中文 (English)」形式。
- **H1 用「中文 (English)」格式**，如 `# 订单 (Orders)`。
- 链接：翻译链接**文字**，**URL 与锚点不变**。
- 行文流畅、完整句子，不要电报式短语堆叠。
- **大文件（>400 行）分段写 + 写后自检**：分段 Write/追加，写完用 Read 复核结尾与英文对应、行数接近、无截断或重复。
- **update 模式（目标中文已存在）**：先读现有中文，**保留维护者加入的提示框/注释**（`:::info`/`:::note`/`> 引用` 等带中文标题的人工注释），只补齐英文新增/扩展内容。
- **prior 迁移（重构场景）**：若 `.sync-overrides` 标了 prior 旧文件，读它并把仍适用的人工注释迁入新文件。

## 阶段 4.5：人工注释保护核查（重构时）

发生结构重构（split/rename）时，旧扁平文件含的维护者人工注释可能漏迁。核查法：

1. 从旧文件提取**带中文标题的 admonition**（`:::xxx 中文标题` 是人工注释的标志，纯英文/无标题的多是英文原文翻译）。
2. 用精确短语 `grep` 新结构，确认每条是否已迁入。
3. 未迁入的：判断对应英文章节在新结构哪个文件，补迁过去；若注释与上游最新事实矛盾，列给用户决断（按最新英文为准）。

## 阶段 5：Review gate（停在 commit 前）

1. 全量校验：`python3 scripts/sync_docs_zh.py verify --all` 与 `python3 scripts/sync_docs_zh.py structure-check`（应 99=99 类对应、无缺译多余）。
2. 扫描 `git status --short docs_zh/` 与 `git diff --stat`，把改动概览（新增/修改/删除/重命名计数）报告用户。
3. **停下等用户决定是否 commit**（这是命令的人工边界，不自动入库）。
4. 用户确认后：
   - `git add docs_zh/`（删除用 `git rm`；重构删旧文件一并纳入）。
   - commit（描述性英文消息，遵循本仓库 commit 风格）。
   - 推进状态：`python3 scripts/sync_docs_zh.py bump --commit <阶段1记录的 upstream/develop HEAD>`，再 `git add docs_zh/.sync-state && git commit`。

---

## 失败处理速查（本命令固化的实战教训）

| 症状                         | 处理                                                  |
| -------------------------- | --------------------------------------------------- |
| HTTPS fetch 反复 `early EOF` | upstream 改 SSH（阶段 1）                                |
| 子代理批量翻译中途 403 / 登录过期       | 用户 `/login` 后，对 verify 未通过的文件重跑（阶段 4 自愈）            |
| 大文件被写到一半截断（围栏不配对）          | `git restore` 回旧版再重译（阶段 4）                          |
| 译文混入日韩文                    | verify 拦截，重译该文件                                     |
| 结构重构（改名/拆分/删除）             | detect 标 anomalies，停下人工确认 → `.sync-overrides`（阶段 3） |
| 旧文件人工注释漏迁                  | 阶段 4.5 核查法                                          |
