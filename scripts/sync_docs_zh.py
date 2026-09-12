#!/usr/bin/env python3
# -------------------------------------------------------------------------------------------------
#  Copyright (C) 2015-2026 Nautech Systems Pty Ltd. All rights reserved.
#  https://nautechsystems.io
#
#  Licensed under the GNU Lesser General Public License Version 3.0 (the "License");
#  You may not use this file except in compliance with the License.
#  You may obtain a copy of the License at https://www.gnu.org/licenses/lgpl-3.0.en.html
#
#  Unless required by applicable law or agreed to in writing, software
#  distributed under the License is distributed on an "AS IS" BASIS,
#  WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
#  See the License for the specific language governing permissions and
#  limitations under the License.
# -------------------------------------------------------------------------------------------------
# ruff: noqa: RUF001, RUF002, RUF003, T201, D400, D415
# 说明：本工具面向中文用户，docstring/help/输出含有意的中文全角标点，
# 故在文件级忽略 RUF001/002/003（ambiguous-unicode）。同理忽略三条与中文命令行工具
# 冲突的规则：T201（print 正是本工具向人与 slash command 输出结果的手段，不是调试残留）、
# D400/D415（要求 docstring 以 ASCII 句点收尾，而中文 docstring 以「。」收尾）。
# 仓库惯例是在 python/pyproject.toml 的 per-file-ignores 里登记单个脚本，但本 fork 不改
# upstream 拥有的那份配置，故就近声明在本文件内。
"""
docs_zh 增量同步工具。

检测上游 ``docs/`` 变更、校验中文译文完整性、对照目录结构、推进同步状态。
零第三方依赖（仅标准库）。供 ``/sync-docs-zh`` slash command 编排调用，也可独立运行。

子命令：
    detect           检测上游 docs 变更，输出待翻译清单
    verify           校验中文译文完整性（行数/代码围栏/admonition/日韩文）
    structure-check  对照 docs/docs_zh 三目录文件集，报缺译/多余
    bump             推进 .sync-state 到给定 commit

"""

from __future__ import annotations

import argparse
import datetime
import json
import re
import shutil
import subprocess
import sys
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parent.parent
GIT = shutil.which("git") or "git"
DOCS_DIRS = ["docs/concepts", "docs/developer_guide", "docs/integrations"]
SYNC_STATE = "docs_zh/.sync-state"
SYNC_OVERRIDES = "docs_zh/.sync-overrides"


def _run_git(args: list[str]) -> str:
    """
    在仓库根运行 git，返回 stdout。
    """
    result = subprocess.run(  # noqa: S603 (safe - 命令固定为 git，无不可信输入)
        [GIT, *args],
        cwd=REPO_ROOT,
        capture_output=True,
        text=True,
        check=True,
    )
    return result.stdout


def _load_json(rel_path: str) -> dict:
    """
    读取仓库内 JSON 文件；不存在时返回空 dict。
    """
    path = REPO_ROOT / rel_path
    if not path.is_file():
        return {}
    return json.loads(path.read_text(encoding="utf-8"))


def _rename_map() -> dict[str, str]:
    """
    从 .sync-overrides 构造 rename 类型的 en_new -> zh_new 映射。
    """
    overrides = _load_json(SYNC_OVERRIDES).get("overrides", [])
    mapping: dict[str, str] = {}
    for ov in overrides:
        if ov.get("type") in ("rename", "map") and "en_new" in ov:
            mapping[ov["en_new"]] = ov.get("zh_new", _default_zh(ov["en_new"]))
    return mapping


def _default_zh(en_path: str) -> str:
    """
    默认映射 docs/X -> docs_zh/X。
    """
    return "docs_zh/" + en_path[len("docs/") :]


def _count_lines(rel_path: str) -> int:
    """
    统计文件行数（不存在返回 0）。
    """
    path = REPO_ROOT / rel_path
    if not path.is_file():
        return 0
    return path.read_text(encoding="utf-8").count("\n")


def cmd_detect(args: argparse.Namespace) -> int:
    """
    检测 base..HEAD 间 docs/ 的变更，分为待翻译清单与结构异常。
    """
    base = args.base or _load_json(SYNC_STATE).get("upstream_commit")
    if not base:
        print(
            "错误: 无法确定 diff 基准（.sync-state 缺 upstream_commit 且未给 --base）",
            file=sys.stderr,
        )
        return 2

    diff = _run_git(["diff", "--name-status", f"{base}..HEAD", "--", *DOCS_DIRS])
    renames = _rename_map()
    pending: list[dict] = []
    anomalies: list[dict] = []

    for line in diff.splitlines():
        if not line.strip():
            continue
        parts = line.split("\t")
        code = parts[0][0]
        if code in ("A", "M"):
            en = parts[1]
            zh = renames.get(en, _default_zh(en))
            pending.append(
                {
                    "en": en,
                    "zh": zh,
                    "exists": (REPO_ROOT / zh).is_file(),
                    "en_lines": _count_lines(en),
                    "status": "added" if code == "A" else "modified",
                },
            )
        else:
            # D（删除）/ R（改名）/ C（复制）等无法可靠自动映射 → 需人工确认
            anomalies.append({"status_code": parts[0], "paths": parts[1:]})

    if args.json:
        payload = {"base": base, "pending": pending, "anomalies": anomalies}
        print(json.dumps(payload, ensure_ascii=False, indent=2))
        return 0

    print(f"基准 commit: {base}")
    print(f"待翻译/更新: {len(pending)} 个文件")
    for p in pending:
        mode = "新建" if not p["exists"] else "更新"
        print(f"  [{mode}] {p['en']} -> {p['zh']} ({p['en_lines']} 行)")
    if anomalies:
        print(f"\n⚠️ 结构异常（需人工确认映射，记入 .sync-overrides）: {len(anomalies)} 项")
        for a in anomalies:
            print(f"  {a['status_code']}: {' '.join(a['paths'])}")
    return 0


# 禁止出现在中文译文里的字符：日文假名（U+3040–30FF）+ 韩文谚文（U+AC00–D7AF）
# 用 \u 转义而非字面字符，避免源码内字面歧义 Unicode（参见 historical-lessons #2）
CJK_FORBIDDEN = re.compile("[\\u3040-\\u30ff\\uac00-\\ud7af]")


def _iter_zh_files() -> list[str]:
    """
    列出 docs_zh 三目录下全部 .md（相对仓库根路径）。
    """
    files: list[str] = []
    for d in ("concepts", "developer_guide", "integrations"):
        base = REPO_ROOT / "docs_zh" / d
        if base.is_dir():
            files.extend(str(p.relative_to(REPO_ROOT)) for p in sorted(base.rglob("*.md")))
    return files


def _verify_one(rel_path: str) -> list[str]:
    """
    校验单个中文文件，返回问题列表（空列表表示通过）。
    """
    path = REPO_ROOT / rel_path
    if not path.is_file():
        return ["文件不存在"]

    text = path.read_text(encoding="utf-8")
    problems: list[str] = []

    fences = len(re.findall(r"^```", text, re.MULTILINE))
    if fences % 2 != 0:
        problems.append(f"代码围栏 ``` 不配对（{fences} 个）")

    colons = len(re.findall(r"^:::", text, re.MULTILINE))
    if colons % 2 != 0:
        problems.append(f"admonition ::: 不配对（{colons} 个）")

    found = CJK_FORBIDDEN.search(text)
    if found:
        problems.append(f"含日文假名/韩文谚文字符 U+{ord(found.group()):04X}")

    # 行数下限：仅当能推导出对应英文源时校验
    if rel_path.startswith("docs_zh/"):
        en = "docs/" + rel_path[len("docs_zh/") :]
        en_lines = _count_lines(en)
        if en_lines:
            zh_lines = text.count("\n")
            floor = max(15, int(en_lines * 0.45))
            if zh_lines < floor:
                problems.append(f"行数过低 {zh_lines} < {floor}（英文 {en_lines}）")

    return problems


def cmd_verify(args: argparse.Namespace) -> int:
    """
    校验中文译文完整性：行数/代码围栏/admonition/日韩文。
    """
    targets = _iter_zh_files() if args.all else list(args.files)
    if not targets:
        print("错误: 未指定文件，且未用 --all", file=sys.stderr)
        return 2

    failed = 0
    for t in targets:
        problems = _verify_one(t)
        if problems:
            failed += 1
            print(f"✗ {t}")
            for p in problems:
                print(f"    - {p}")

    total = len(targets)
    if failed:
        print(f"\n校验失败: {failed}/{total} 个文件有问题")
        return 1
    print(f"✓ 全部通过: {total} 个文件")
    return 0


def _docs_md_set(base_dir: str) -> set[str]:
    """
    返回 base_dir（docs 或 docs_zh）三目录下 .md 的相对路径集合。
    """
    out: set[str] = set()
    root = REPO_ROOT / base_dir
    for d in ("concepts", "developer_guide", "integrations"):
        sub = root / d
        if sub.is_dir():
            out.update(str(p.relative_to(root)) for p in sub.rglob("*.md"))
    return out


def cmd_structure_check(_args: argparse.Namespace) -> int:
    """
    对照 docs/docs_zh 三目录文件集，报缺译/多余。
    """
    en = _docs_md_set("docs")
    zh = _docs_md_set("docs_zh")
    missing = sorted(en - zh)
    extra = sorted(zh - en)

    print(f"docs(英): {len(en)} 个  docs_zh(中): {len(zh)} 个")
    if missing:
        print(f"\n缺译（英有中无）: {len(missing)}")
        for m in missing:
            print(f"  - {m}")
    if extra:
        print(f"\n多余（中有英无）: {len(extra)}")
        for e in extra:
            print(f"  - {e}")
    if missing or extra:
        return 1
    print("✓ 结构完全对应")
    return 0


def _today_utc() -> str:
    """
    当前 UTC 日期（ISO 格式），timezone-aware 以满足 flake8-datetimez。
    """
    return datetime.datetime.now(datetime.UTC).date().isoformat()


def cmd_bump(args: argparse.Namespace) -> int:
    """
    推进 .sync-state 到给定 commit（默认 upstream/develop 当前 HEAD）。
    """
    commit = args.commit or _run_git(["rev-parse", "upstream/develop"]).strip()
    short = commit[:10]
    state = _load_json(SYNC_STATE)
    state["upstream_commit"] = short
    state.setdefault("upstream_ref", "upstream/develop")
    state["synced_at"] = args.date or _today_utc()
    path = REPO_ROOT / SYNC_STATE
    path.write_text(json.dumps(state, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"已更新 .sync-state: upstream_commit={short} synced_at={state['synced_at']}")
    return 0


def main() -> int:
    """
    解析命令行参数并分发到子命令，返回进程退出码。
    """
    parser = argparse.ArgumentParser(description="docs_zh 增量同步工具")
    sub = parser.add_subparsers(dest="command", required=True)

    p_detect = sub.add_parser("detect", help="检测上游 docs 变更，输出待翻译清单")
    p_detect.add_argument("--base", help="diff 基准 commit（默认取 .sync-state）")
    p_detect.add_argument("--json", action="store_true", help="输出 JSON")
    p_detect.set_defaults(func=cmd_detect)

    p_verify = sub.add_parser("verify", help="校验中文译文完整性")
    p_verify.add_argument("files", nargs="*", help="待校验文件（默认配合 --all）")
    p_verify.add_argument("--all", action="store_true", help="校验所有 docs_zh 文件")
    p_verify.set_defaults(func=cmd_verify)

    p_struct = sub.add_parser("structure-check", help="对照 docs/docs_zh 三目录文件集")
    p_struct.set_defaults(func=cmd_structure_check)

    p_bump = sub.add_parser("bump", help="推进 .sync-state 到给定 commit")
    p_bump.add_argument("--commit", help="目标 commit（默认 upstream/develop）")
    p_bump.add_argument("--date", help="同步日期（默认今天）")
    p_bump.set_defaults(func=cmd_bump)

    args = parser.parse_args()
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main())
