#!/usr/bin/env python3
"""复盘账本 CLI（AUTO-02）。逻辑见 lesson_ledger.py，CLI 只做参数与输出。

用法：
  lessons.py add --rule "..." --why "..." --evidence <run_id> [--date YYYY-MM-DD]
  lessons.py validate [--threshold 3]
  lessons.py list
  lessons.py revoke --key <key>            # 人工撤销：只改该账本文件内容

约定：
- `add` 是幂等的：同 (key, seen_date, evidence) 且内容一致 → 不重复写入；
  同身份不同内容 → 拒绝（duplicate_conflict），不覆盖已有记录。
- 错误只打印**固定 code**（含行号），不回显规则文本或凭据。
- 本 CLI 不改变任何权限模型：不 chmod/chown 账本之外的任何文件，
  也不因为一条撤销而放宽或升级权限。
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from lesson_ledger import (  # noqa: E402
    LessonError,
    append_entry,
    load_entries,
    make_entry,
    normalize_key,  # 重导出：外部脚本用它由 rule 计算 --key
    promotion_candidates,
    revoke_key,
    validate,
)

LEDGER = Path(__file__).resolve().parent.parent.parent / ".scratch/automation-lessons.jsonl"


def _utcnow() -> str:
    return dt.datetime.now(dt.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def _today() -> str:
    return dt.datetime.now(dt.timezone.utc).strftime("%Y-%m-%d")


def _fail(exc: LessonError) -> int:
    location = f" (line {exc.line})" if exc.line else ""
    print(f"✗ {exc.code}{location}", file=sys.stderr)
    return 2


def main() -> int:
    parser = argparse.ArgumentParser(description="AgentIsland lessons ledger")
    sub = parser.add_subparsers(dest="command", required=True)

    add = sub.add_parser("add")
    add.add_argument("--rule", required=True)
    add.add_argument("--why", default="")
    add.add_argument("--evidence", default="")
    add.add_argument("--date", default=_today())

    validate_cmd = sub.add_parser("validate")
    validate_cmd.add_argument("--threshold", type=int, default=3)

    sub.add_parser("list")

    revoke = sub.add_parser("revoke")
    revoke.add_argument("--key", required=True)

    args = parser.parse_args()

    if args.command == "add":
        try:
            entry = make_entry(
                rule=args.rule,
                why=args.why,
                evidence=args.evidence,
                seen_date=args.date,
                recorded_at=_utcnow(),
            )
            status = append_entry(LEDGER, entry)
        except LessonError as exc:
            return _fail(exc)
        if status == "duplicate":
            print(f"= already recorded: {entry['key']} ({entry['seen_date']})")
        else:
            print(f"✓ recorded: {entry['key']} ({entry['seen_date']})")
        return 0

    if args.command == "revoke":
        try:
            removed = revoke_key(LEDGER, args.key)
        except LessonError as exc:
            return _fail(exc)
        print(f"✓ revoked {removed} entr(ies) for key {args.key}")
        return 0

    try:
        entries = load_entries(LEDGER)
    except LessonError as exc:
        return _fail(exc)

    if args.command == "list":
        for entry in entries:
            print(json.dumps(entry, ensure_ascii=False))
        print(f"total: {len(entries)}", file=sys.stderr)
        return 0

    problems = validate(entries)
    if problems:
        print(f"✗ ledger invalid: {len(problems)} problem(s)", file=sys.stderr)
        for problem in problems[:20]:
            print(f"  - {problem}", file=sys.stderr)
        return 1
    promo = promotion_candidates(entries, args.threshold)
    print(
        f"✓ ledger valid: {len(entries)} entr(ies); "
        f"promotion candidates (>= {args.threshold} distinct dates with evidence): {len(promo)}"
    )
    for item in promo:
        print(f"  - {item['key']}  dates={item['distinct_dates']}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
