#!/usr/bin/env python3
"""Regenerate the machine-readable mapping section of the requirement test matrix.

The scenario table stays human-authored; this section is generated from registry.json
so the two can never drift: a check added to the registry appears here, and a check
removed from it disappears. Hand edits to the generated block are overwritten.
"""
from __future__ import annotations

import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
REGISTRY = ROOT / "scripts/automation/registry.json"
MATRIX = ROOT / "docs/需求测试矩阵.md"
BEGIN = "<!-- generated:requirement-check-mapping:begin -->"
END = "<!-- generated:requirement-check-mapping:end -->"


def render(registry: dict) -> str:
    lines = [
        BEGIN,
        "",
        "## 需求 → 检查项映射（由 registry.json 生成，勿手改）",
        "",
        "执行状态来自 registry：`checks` 非空 = 有可执行检查；空 = `planned`（未实现，选中即拒绝）。",
        "覆盖程度见每条 `coverage_note`；本表只回答「有哪些检查项、跑在哪个 profile」。",
        "",
        "| 需求 | 执行状态 | 检查项 | profile |",
        "| --- | --- | --- | --- |",
    ]
    profiles = registry["profiles"]
    where = {cid: sorted(name for name, ids in profiles.items()
                         if isinstance(ids, list) and cid in ids)
             for cid in registry["checks"]}
    for rid, entry in sorted(registry["requirements"].items()):
        checks = entry.get("checks", [])
        if not checks:
            lines.append(f"| {rid} | planned（未实现） | — | — |")
            continue
        for cid in checks:
            lines.append(f"| {rid} | 有检查项 | `{cid}` | {', '.join(where.get(cid, [])) or '—'} |")
    # Checks not attached to any requirement still execute; listing only requirement-owned
    # checks would leave them invisible and make the table disagree with the registry.
    attached = {cid for entry in registry["requirements"].values() for cid in entry.get("checks", [])}
    for cid in sorted(set(registry["checks"]) - attached):
        lines.append(f"| （未挂需求） | 有检查项 | `{cid}` | {', '.join(where.get(cid, [])) or '—'} |")
    lines += ["", END]
    return "\n".join(lines) + "\n"


def main() -> int:
    registry = json.loads(REGISTRY.read_text(encoding="utf-8"))
    text = MATRIX.read_text(encoding="utf-8")
    expected = render(registry)
    # A gate must not modify the source it is gating. Report drift and fail; the
    # developer regenerates deliberately with --write.
    if "--write" in sys.argv[1:]:
        if BEGIN in text and END in text:
            head = text[: text.index(BEGIN)]
            tail = text[text.index(END) + len(END):]
        else:
            head, tail = text.rstrip() + "\n\n", ""
        MATRIX.write_text(head + expected + tail, encoding="utf-8")
        print("matrix mapping regenerated")
        return 0
    if BEGIN not in text or END not in text:
        print("matrix mapping block missing; run with --write", file=sys.stderr)
        return 1
    # Compare the same span: the stored block ends right after END (the newline that
    # follows belongs to the document, not to the generated block).
    current = text[text.index(BEGIN):text.index(END) + len(END)]
    if current != expected.rstrip("\n"):
        print("matrix mapping drifted from registry; run render-matrix.py --write", file=sys.stderr)
        return 1
    mapped = sum(1 for e in registry["requirements"].values() if e.get("checks"))
    print(f"matrix mapping matches registry: {mapped} with checks, "
          f"{len(registry['requirements']) - mapped} planned")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
