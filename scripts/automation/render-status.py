#!/usr/bin/env python3
"""Regenerate the requirement status overview from the registry.

Why this exists: `docs/需求状态.md` used to be something a human had to remember to
update. Remembering does not scale and does not survive handoffs — a new agent (or a
different agent) could not tell what was done. This generator makes the **status table**
a pure function of `scripts/automation/registry.json`, and a gate (`--check`) fails when
the committed document drifts from the registry.

Split of responsibility:

- **Machine-written**: the status table and the open-bug table (from the registry).
- **Human-written**: the prose in the marked blocks below (notes, rationale).

Hand edits to the generated blocks are overwritten on `--write`.

Status vocabulary (registry `requirements[].status`):

- `planned`  — no executable checks yet. Must have an empty `checks` list.
- `partial`  — has checks, but `coverage_note` still names a gap (i.e. does not start
               with a "done" marker).
- `done`     — has checks **and** the requirement's acceptance scenarios are all covered.
               Recorded by prefixing `coverage_note` with `done — `.

The generator does not decide `done`; it reports what the registry says and fails on
combinations that cannot be true (see `validate`).
"""
from __future__ import annotations

import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
REGISTRY = ROOT / "scripts/automation/registry.json"
STATUS_DOC = ROOT / "docs/需求状态.md"
BEGIN = "<!-- generated:requirement-status:begin -->"
END = "<!-- generated:requirement-status:end -->"

# Coverage note prefixes that mean "no known gap left". Kept explicit and short so the
# vocabulary cannot drift into synonyms that the validator would silently accept.
DONE_MARKERS = ("done —", "done -")
PARTIAL_MARKERS = ("partial —", "partial -")
PLANNED_MARKERS = ("planned —", "planned -")

STATUS_LABEL = {
    "done": "已完成",
    "partial": "部分完成",
    "planned": "未开始",
}


def declared_status(entry: dict) -> str:
    """The requirement's own claim, derived from `status` + `coverage_note`.

    `status` alone is not enough: `partial` is also used as the historical bucket for
    "has checks". The note prefix is what distinguishes `partial` from `done`, because
    only the note says whether a gap remains.
    """
    checks = entry.get("checks", [])
    note = (entry.get("coverage_note") or "").strip()
    if not checks:
        return "planned"
    if note.startswith(DONE_MARKERS):
        return "done"
    return "partial"


def validate(registry: dict) -> list[str]:
    """Reject combinations that cannot both be true."""
    errors: list[str] = []
    for rid, entry in sorted(registry["requirements"].items()):
        checks = entry.get("checks", [])
        status = entry.get("status")
        note = (entry.get("coverage_note") or "").strip()
        if not checks and status != "planned":
            errors.append(f"{rid}: 无检查项却标 {status}（无检查项只能是 planned）")
        if checks and status == "planned":
            errors.append(f"{rid}: 有检查项却标 planned（有检查项应为 partial 或 done）")
        if status == "done" and not note.startswith(DONE_MARKERS):
            errors.append(f"{rid}: status=done 但 coverage_note 未以 'done — ' 开头")
        if status == "partial" and note.startswith(DONE_MARKERS):
            errors.append(f"{rid}: status=partial 但 coverage_note 声称 done（自相矛盾）")
        if checks and not note:
            errors.append(f"{rid}: 有检查项但缺 coverage_note（无法判断是否还有缺口）")
        allowed = DONE_MARKERS + PARTIAL_MARKERS + PLANNED_MARKERS
        if note and not note.startswith(allowed):
            errors.append(
                f"{rid}: coverage_note 必须以 'done — ' / 'partial — ' / 'planned — ' 开头，"
                f"否则状态无法判定：{note[:40]!r}"
            )
        if not checks and note and not note.startswith(PLANNED_MARKERS):
            errors.append(
                f"{rid}: 无检查项时 coverage_note 应以 'planned — ' 开头：{note[:40]!r}"
            )
    for bug in registry.get("open_bugs", []):
        for field in ("id", "title", "status", "evidence"):
            if not bug.get(field):
                errors.append(f"open_bugs 条目缺 {field}: {bug}")
    return errors


def render_status_table(registry: dict) -> str:
    lines = [
        BEGIN,
        "",
        "## 需求状态（由 `scripts/automation/registry.json` 生成，勿手改）",
        "",
        "| 需求 | 状态 | 检查项 | 说明 |",
        "| --- | --- | --- | --- |",
    ]
    counts = {"done": 0, "partial": 0, "planned": 0}
    for rid, entry in sorted(registry["requirements"].items()):
        status = declared_status(entry)
        counts[status] += 1
        checks = entry.get("checks", [])
        shown = "、".join(f"`{c}`" for c in checks) if checks else "—"
        note = (entry.get("coverage_note") or "").replace("|", "\\|").strip()
        lines.append(f"| {rid} | {STATUS_LABEL[status]} | {shown} | {note} |")
    total = len(registry["requirements"])
    lines += [
        "",
        f"**合计**：{total} 项 — 已完成 {counts['done']}、部分完成 {counts['partial']}、"
        f"未开始 {counts['planned']}。",
        "",
        "状态判定规则（`scripts/automation/render-status.py` 实现，`--check` 在门禁里跑）：",
        "",
        "- **未开始**：`checks` 为空。选中即拒绝执行。",
        "- **部分完成**：有检查项，但 `coverage_note` 仍列出缺口（以 `partial — ` 开头）。",
        "- **已完成**：有检查项，且 `coverage_note` 以 `done — ` 开头（表示该需求的验收场景已全部覆盖）。",
        "",
        END,
    ]
    return "\n".join(lines) + "\n"


def render_bug_table(registry: dict) -> str:
    bugs = registry.get("open_bugs", [])
    lines = [
        "## 未修 bug 登记（由 registry 生成，勿手改）",
        "",
        "**发现 bug 时写进 `scripts/automation/registry.json` 的 `open_bugs` 数组**，"
        "本节由生成器渲染。字段：`id` / `title` / `status` / `evidence`（必填），"
        "`requirement`（可选，关联需求 ID）。",
        "",
    ]
    if not bugs:
        lines += ["当前无未修 bug。", ""]
        return "\n".join(lines) + "\n"
    lines += [
        "| ID | 标题 | 状态 | 关联需求 | 证据 |",
        "| --- | --- | --- | --- | --- |",
    ]
    for bug in bugs:
        lines.append(
            f"| {bug['id']} | {bug['title']} | {bug['status']} | "
            f"{bug.get('requirement') or '—'} | {bug['evidence']} |"
        )
    lines.append("")
    return "\n".join(lines) + "\n"


def render(registry: dict) -> str:
    """Both tables live **inside** the generated block.

    Keeping the bug table outside it was a real defect caught while writing this: every
    `--write` appended a second copy, because head/tail are split on the markers. Inside
    the block, regeneration replaces the whole thing.
    """
    table = render_status_table(registry)
    assert table.rstrip().endswith(END), "状态表必须以 END 标记收尾"
    head = table.rstrip()[: -len(END)].rstrip("\n")
    return head + "\n\n" + render_bug_table(registry).rstrip("\n") + "\n\n" + END + "\n"


def main() -> int:
    registry = json.loads(REGISTRY.read_text(encoding="utf-8"))
    errors = validate(registry)
    if errors:
        for error in errors:
            print(f"registry 状态不一致：{error}", file=sys.stderr)
        return 1

    expected = render(registry)
    if "--write" in sys.argv[1:]:
        if STATUS_DOC.exists():
            text = STATUS_DOC.read_text(encoding="utf-8")
        else:
            text = "# 需求状态总览\n"
        if BEGIN in text and END in text:
            head = text[: text.index(BEGIN)]
            # Skip the newline that belongs to the document, not to the block — otherwise
            # every regeneration appends one more blank line (caught by a two-run
            # idempotency check while writing this).
            after = text[text.index(END) + len(END):]
            tail = after[1:] if after.startswith("\n") else after
        else:
            head, tail = text.rstrip() + "\n\n", ""
        STATUS_DOC.write_text(head + expected + tail, encoding="utf-8")
        print("requirement status regenerated")
        return 0

    if not STATUS_DOC.exists():
        print("docs/需求状态.md 不存在；用 --write 生成", file=sys.stderr)
        return 1
    text = STATUS_DOC.read_text(encoding="utf-8")
    if BEGIN not in text or END not in text:
        print("状态块缺失；用 --write 生成", file=sys.stderr)
        return 1
    current = text[text.index(BEGIN): text.index(END) + len(END)]
    if current != expected.rstrip("\n"):
        print("docs/需求状态.md 与 registry 漂移；跑 render-status.py --write", file=sys.stderr)
        return 1
    counts = {"done": 0, "partial": 0, "planned": 0}
    for entry in registry["requirements"].values():
        counts[declared_status(entry)] += 1
    print(
        f"requirement status matches registry: done={counts['done']}, "
        f"partial={counts['partial']}, planned={counts['planned']}, "
        f"open_bugs={len(registry.get('open_bugs', []))}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
