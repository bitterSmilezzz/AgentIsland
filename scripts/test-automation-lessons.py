#!/usr/bin/env python3
"""AUTO-02 复盘账本元测试（合成夹具，不读真实账本）。

Run: python3 scripts/test-automation-lessons.py

隔离约定：
- 每个用例都在自己的 `tempfile.TemporaryDirectory()` 里造账本，绝不读/写
  `.scratch/automation-lessons.jsonl`；
- 并发用例起**两个真实子进程**同时追加同一账本，验证互斥不丢更新；
- 不联网、不调真实 Agent、不改仓库任何文件（只读本目录源码做静态兼容检查）。
"""

from __future__ import annotations

import json
import os
import re
import stat
import subprocess
import sys
import tempfile
import textwrap
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE / "automation"))

import lesson_ledger as ledger  # noqa: E402
from lesson_ledger import (  # noqa: E402
    LessonError,
    append_entry,
    load_entries,
    make_entry,
    normalize_key,
    promotion_candidates,
    revoke_key,
    validate,
)

STAMP = "2026-10-15T00:00:00Z"
PY = sys.executable or "python3"


class LedgerTestCase(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory(prefix="auto02-lessons-")
        self.addCleanup(self._tmp.cleanup)
        self.tmp = Path(self._tmp.name)
        self.ledger = self.tmp / "lessons.jsonl"

    def entry(self, rule="Never trust a truncated log tail", date="2026-10-14",
              evidence="run-a", why="", recorded_at=STAMP):
        return make_entry(rule, why, evidence, date, recorded_at)

    def raw_lines(self):
        return self.ledger.read_text(encoding="utf-8").splitlines()


# ---------------------------------------------------------------- 键


class KeyTests(LedgerTestCase):
    def test_ascii_rules_keep_their_historical_key_bytes(self):
        self.assertEqual(normalize_key("Never trust a truncated log tail"), "never-trust-a-truncated-log-tail")
        self.assertEqual(normalize_key("Mock OK  -> upstream auth"), "mock-ok-upstream-auth")

    def test_chinese_rules_get_usable_distinct_keys(self):
        first = normalize_key("不要相信截断的日志尾部")
        second = normalize_key("不要相信日志被截断")
        self.assertTrue(first and second)
        self.assertNotEqual(first, second)
        self.assertTrue(ledger.KEY_RE.match(first), first)

    def test_same_rule_text_is_stable_across_calls_and_normalization_forms(self):
        fullwidth = "Ｃａｔｃｈ 假绿：空账本"
        halfwidth = "Catch 假绿:空账本"
        self.assertEqual(normalize_key(fullwidth), normalize_key(fullwidth))
        self.assertEqual(normalize_key(fullwidth), normalize_key(halfwidth))
        self.assertEqual(normalize_key("Catch  假绿—空账本"), normalize_key(halfwidth))

    def test_truncation_collision_is_disambiguated(self):
        base = "a" * 300
        left = normalize_key(base + " x")
        right = normalize_key(base + " y")
        self.assertNotEqual(left, right)
        self.assertLessEqual(len(left), ledger.KEY_MAX)
        # 同文本仍然恒定（跨日期聚合依赖这一点）
        self.assertEqual(left, normalize_key(base + " x"))

    def test_long_ascii_rule_and_unicode_rule_never_collide_with_legacy_keys(self):
        legacy = normalize_key("a" * 300)
        self.assertNotEqual(normalize_key("a" * 300 + " x"), legacy)

    def test_rule_without_alphanumerics_is_rejected_not_silently_emptied(self):
        self.assertEqual(normalize_key("   "), "")
        for rule in ("...", "!!!", "。、，！？"):
            with self.assertRaises(LessonError) as caught:
                self.entry(rule=rule)
            self.assertEqual(caught.exception.code, "rule_not_keyable")
        for blank in ("   ", "\u3000", ""):
            with self.assertRaises(LessonError) as caught:
                self.entry(rule=blank)
            self.assertEqual(caught.exception.code, "empty_rule")

    def test_control_characters_are_rejected(self):
        for rule in ("line\nbreak rule", "tab\there", "nul\x00byte"):
            with self.subTest(rule=rule):
                with self.assertRaises(LessonError) as caught:
                    self.entry(rule=rule)
                self.assertEqual(caught.exception.code, "invalid_rule")
        with self.assertRaises(LessonError) as caught:
            self.entry(why="why\nwith newline")
        self.assertEqual(caught.exception.code, "invalid_why")


# ---------------------------------------------------------------- 日期/时间


class DateTests(LedgerTestCase):
    def test_impossible_calendar_dates_are_rejected(self):
        for bad in ("2026-99-99", "2026-02-30", "2026-13-01", "2026-00-10", "2026-1-1", "14-10-2026"):
            with self.assertRaises(LessonError) as caught:
                self.entry(date=bad)
            self.assertEqual(caught.exception.code, "invalid_seen_date")

    def test_real_calendar_dates_accepted_including_leap_day(self):
        leap = self.entry(date="2024-02-29")
        self.assertEqual(leap["seen_date"], "2024-02-29")
        self.entry(date="2026-02-28")

    def test_recorded_at_must_be_an_utc_instant(self):
        for bad in ("2026-99-99T00:00:00Z", "not-a-timestamp", "2026-10-15T00:00:00+08:00", "2026-10-15"):
            with self.assertRaises(LessonError) as caught:
                self.entry(recorded_at=bad)
            self.assertEqual(caught.exception.code, "invalid_recorded_at")
        self.entry(recorded_at="2026-10-15T08:00:00.500+00:00")

    def test_bounded_fields_reject_oversized_input(self):
        with self.assertRaises(LessonError) as caught:
            self.entry(rule="bounded " + "r" * (ledger.RULE_MAX + 5))
        self.assertEqual(caught.exception.code, "invalid_rule")
        with self.assertRaises(LessonError) as caught:
            self.entry(evidence="e" * (ledger.EVIDENCE_MAX + 1))
        self.assertEqual(caught.exception.code, "invalid_evidence")


# ---------------------------------------------------------------- 追加/幂等


class AppendTests(LedgerTestCase):
    def test_append_publishes_new_file_with_0600_permissions(self):
        status = append_entry(self.ledger, self.entry())
        self.assertEqual(status, "appended")
        mode = stat.S_IMODE(self.ledger.stat().st_mode)
        self.assertEqual(mode, 0o600, oct(mode))
        self.assertEqual(stat.S_IMODE(self.ledger.parent.stat().st_mode) & stat.S_IWOTH, 0)

    def test_same_identity_and_content_is_idempotent(self):
        first = self.entry(why="misdiagnosed twice")
        self.assertEqual(append_entry(self.ledger, first), "appended")
        self.assertEqual(append_entry(self.ledger, dict(first)), "duplicate")
        self.assertEqual(append_entry(self.ledger, dict(first)), "duplicate")
        self.assertEqual(len(self.raw_lines()), 1)

    def test_same_identity_different_content_is_refused_not_overwritten(self):
        append_entry(self.ledger, self.entry(why="first wording"))
        with self.assertRaises(LessonError) as caught:
            append_entry(self.ledger, self.entry(why="different wording"))
        self.assertEqual(caught.exception.code, "duplicate_conflict")
        self.assertEqual(len(self.raw_lines()), 1)

    def test_different_evidence_same_day_is_a_new_entry_not_a_duplicate(self):
        append_entry(self.ledger, self.entry(evidence="run-a"))
        append_entry(self.ledger, self.entry(evidence="run-b"))
        self.assertEqual(len(load_entries(self.ledger)), 2)

    def test_append_never_rewrites_bytes_it_did_not_add(self):
        append_entry(self.ledger, self.entry(evidence="run-a"))
        before = self.ledger.read_bytes()
        first_line = before.splitlines()[0]
        append_entry(self.ledger, self.entry(evidence="run-b"))
        after = self.ledger.read_bytes()
        self.assertTrue(after.startswith(first_line + b"\n"))
        self.assertEqual(after[: len(before)], before)

    def test_ledger_size_cap_is_enforced_with_a_named_error(self):
        filler = "x" * ledger.WHY_MAX
        with self.assertRaises(LessonError) as caught:
            for i in range(1000):
                append_entry(self.ledger, self.entry(why=filler, evidence=f"run-{i}"))
        self.assertEqual(caught.exception.code, "ledger_full")
        self.assertLessEqual(self.ledger.stat().st_size, ledger.MAX_LEDGER_BYTES)


# ---------------------------------------------------------------- 并发


class ConcurrencyTests(LedgerTestCase):
    def test_two_processes_appending_concurrently_lose_no_update(self):
        script = textwrap.dedent(
            """
            import sys
            sys.path.insert(0, sys.argv[1])
            from pathlib import Path
            from lesson_ledger import append_entry, make_entry
            ledger = Path(sys.argv[2])
            tag = sys.argv[3]
            for i in range(15):
                append_entry(ledger, make_entry(f"concurrent rule {tag}", "", f"run-{tag}-{i}",
                                                "2026-10-15", "2026-10-15T00:00:00Z"))
            """
        )
        procs = [
            subprocess.Popen([PY, "-c", script, str(HERE / "automation"), str(self.ledger), tag],
                             stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
            for tag in ("alpha", "beta")
        ]
        for proc in procs:
            out, err = proc.communicate(timeout=120)
            self.assertEqual(proc.returncode, 0, out + err)
        entries = load_entries(self.ledger)
        self.assertEqual(len(entries), 30, [e.get("evidence") for e in entries])
        self.assertEqual(len({e["evidence"] for e in entries}), 30)
        self.assertEqual(self.ledger.read_bytes().count(b"\n"), 30)
        self.assertEqual(validate(entries), [])

    def test_lock_does_not_leave_stray_temp_files(self):
        append_entry(self.ledger, self.entry())
        leftovers = [p.name for p in self.tmp.iterdir() if p.suffix == ".tmp"]
        self.assertEqual(leftovers, [])


# ---------------------------------------------------------------- 坏文件


class CorruptLedgerTests(LedgerTestCase):
    def test_bad_json_line_reports_a_fixed_code_with_line_number(self):
        self.ledger.write_bytes(b'{"v":1,"key":"ok-key","rule":"ok rule","seen_date":"2026-10-14",'
                                b'"recorded_at":"2026-10-15T00:00:00Z"}\n{oops not json\n')
        with self.assertRaises(LessonError) as caught:
            load_entries(self.ledger)
        self.assertEqual(caught.exception.code, "ledger_json_invalid")
        self.assertEqual(caught.exception.line, 2)

    def test_non_object_and_blank_lines_are_rejected_not_skipped(self):
        valid = json.dumps(self.entry(), ensure_ascii=False).encode("utf-8")
        for payload, code, line in (
            (b'[]\n', "ledger_not_object", 1),
            (b'"text"\n', "ledger_not_object", 1),
            (b'42\n', "ledger_not_object", 1),
            (b'\n', "ledger_blank_line", 1),
            (b'   \n', "ledger_blank_line", 1),
            (valid + b'\n\n', "ledger_blank_line", 2),
            (valid + b'\n   \n', "ledger_blank_line", 2),
            (b'{}\n', "ledger_missing_field", 1),
        ):
            with self.subTest(payload=payload):
                self.ledger.write_bytes(payload)
                with self.assertRaises(LessonError) as caught:
                    load_entries(self.ledger)
                self.assertEqual(caught.exception.code, code)
                self.assertEqual(caught.exception.line, line)

    def test_corrupt_ledger_is_never_reported_as_a_valid_empty_ledger(self):
        self.ledger.write_bytes(b'{broken\n[]\n')
        with self.assertRaises(LessonError) as caught:
            load_entries(self.ledger)
        self.assertEqual(caught.exception.code, "ledger_json_invalid")
        # 旧版行为对照：坏 JSON 与非 object 行被静默丢弃，于是 load→[] → validate([]) 通过。
        self.assertEqual(validate([]), [])

    def test_append_refuses_to_touch_a_corrupt_ledger_and_preserves_bytes(self):
        payload = b'{broken json}\n'
        self.ledger.write_bytes(payload)
        with self.assertRaises(LessonError) as caught:
            append_entry(self.ledger, self.entry())
        self.assertEqual(caught.exception.code, "ledger_json_invalid")
        self.assertEqual(self.ledger.read_bytes(), payload)

    def test_future_schema_is_refused_and_bytes_kept(self):
        payload = b'{"v":2,"key":"future-key","rule":"future rule","seen_date":"2026-10-14","recorded_at":"2026-10-15T00:00:00Z"}\n'
        self.ledger.write_bytes(payload)
        with self.assertRaises(LessonError) as caught:
            load_entries(self.ledger)
        self.assertEqual(caught.exception.code, "ledger_schema_unsupported")
        with self.assertRaises(LessonError):
            append_entry(self.ledger, self.entry())
        self.assertEqual(self.ledger.read_bytes(), payload)

    def test_missing_or_unknown_schema_fields_are_refused(self):
        self.ledger.write_bytes(b'{"v":1,"key":"legacy-key","rule":"legacy rule","seen_date":"2026-10-14"}\n')
        with self.assertRaises(LessonError) as caught:
            load_entries(self.ledger)
        self.assertEqual(caught.exception.code, "ledger_missing_field")
        self.ledger.write_bytes(b'{"v":1,"key":"k-1","rule":"ok rule","seen_date":"2026-10-14",'
                                b'"recorded_at":"2026-10-15T00:00:00Z","credential":"x"}\n')
        with self.assertRaises(LessonError) as caught:
            load_entries(self.ledger)
        self.assertEqual(caught.exception.code, "ledger_unknown_field")

    def test_oversized_ledger_and_line_are_refused(self):
        self.ledger.write_bytes(b"x" * (ledger.MAX_LEDGER_BYTES + 1))
        with self.assertRaises(LessonError) as caught:
            load_entries(self.ledger)
        self.assertEqual(caught.exception.code, "ledger_too_large")
        self.ledger.write_bytes(b"y" * (ledger.MAX_LINE_BYTES + 1) + b"\n")
        with self.assertRaises(LessonError) as caught:
            load_entries(self.ledger)
        self.assertEqual(caught.exception.code, "ledger_line_too_long")

    def test_symlinked_ledger_is_refused_for_read_and_write(self):
        real = self.tmp / "real.jsonl"
        real.write_bytes(b"")
        link = self.tmp / "link.jsonl"
        link.symlink_to(real)
        for call in (lambda: load_entries(link), lambda: append_entry(link, self.entry())):
            with self.assertRaises(LessonError) as caught:
                call()
            self.assertEqual(caught.exception.code, "ledger_symlink_refused")
        self.assertEqual(real.read_bytes(), b"")

    def test_non_utf8_ledger_is_refused(self):
        self.ledger.write_bytes(b'\xff\xfe{"v":1}\n')
        with self.assertRaises(LessonError) as caught:
            load_entries(self.ledger)
        self.assertEqual(caught.exception.code, "ledger_not_utf8")


# ---------------------------------------------------------------- 晋升


class PromotionTests(LedgerTestCase):
    def test_same_day_repeats_never_promote_even_with_distinct_evidence(self):
        for i in range(6):
            append_entry(self.ledger, self.entry(evidence=f"run-{i}", date="2026-10-14"))
        self.assertEqual(promotion_candidates(load_entries(self.ledger)), [])

    def test_promotion_needs_distinct_dates_that_each_carry_evidence(self):
        for date in ("2026-10-14", "2026-10-15"):
            append_entry(self.ledger, self.entry(date=date, evidence=f"run-{date}"))
        self.assertEqual(promotion_candidates(load_entries(self.ledger)), [])
        append_entry(self.ledger, self.entry(date="2026-10-16", evidence="run-2026-10-16"))
        promo = promotion_candidates(load_entries(self.ledger))
        self.assertEqual(len(promo), 1)
        self.assertEqual(promo[0]["distinct_dates"], ["2026-10-14", "2026-10-15", "2026-10-16"])
        self.assertEqual(promo[0]["distinct_dates_count"], 3)
        self.assertEqual(promo[0]["evidences"]["2026-10-16"], ["run-2026-10-16"])

    def test_dates_without_evidence_do_not_count_towards_promotion(self):
        entries = [
            make_entry("evidence bound promotion rule", "", f"run-{d}", d, STAMP)
            for d in ("2026-10-14", "2026-10-15", "2026-10-16")
        ]
        entries[1]["evidence"] = ""
        self.assertEqual(promotion_candidates(entries), [])
        self.assertEqual(promotion_candidates(entries, threshold=2)[0]["distinct_dates"],
                         ["2026-10-14", "2026-10-16"])

    def test_threshold_below_two_cannot_be_used_to_make_same_day_pass(self):
        for i in range(4):
            append_entry(self.ledger, self.entry(evidence=f"run-{i}", date="2026-10-14"))
        self.assertEqual(promotion_candidates(load_entries(self.ledger), threshold=1), [])

    def test_unicode_rule_promotes_under_one_stable_key(self):
        for date in ("2026-10-14", "2026-10-15", "2026-10-16"):
            append_entry(self.ledger, self.entry(rule="不要相信截断的日志尾部", date=date, evidence=f"run-{date}"))
        promo = promotion_candidates(load_entries(self.ledger))
        self.assertEqual(len(promo), 1)
        self.assertTrue(promo[0]["key"].startswith("不要相信截断的日志尾部"))


# ---------------------------------------------------------------- 校验与脱敏


class ValidateTests(LedgerTestCase):
    def test_validate_reports_fixed_codes_without_echoing_rule_or_credentials(self):
        # 运行时拼接：保持「validate 不回显凭据」的断言，但源码里不留赋值 + 引号字面量的形状。
        credential = "sk-" + "probe" + "-" + "0123456789abcdef"
        entries = [
            {**self.entry(rule="Baseline good rule", evidence="run-a")},
            {**self.entry(rule="Second rule text", evidence="run-b"), "seen_date": "2026-99-99"},
            {**self.entry(rule=f"Rule carrying {credential}", evidence="run-c"), "key": "bad key with spaces"},
        ]
        problems = validate(entries)
        joined = "\n".join(problems)
        self.assertIn("invalid_seen_date", joined)
        self.assertIn("invalid_key", joined)
        self.assertIn("key_rule_mismatch", joined)
        self.assertNotIn(credential, joined)
        self.assertNotIn("Second rule text", joined)
        self.assertNotIn("Baseline good rule", joined)

    def test_validate_flags_duplicate_identity_and_conflicting_repeat(self):
        first = self.entry(why="one")
        problems = validate([first, dict(first)])
        self.assertTrue(any("duplicate_entry" in p for p in problems), problems)
        problems = validate([first, {**first, "why": "two"}])
        self.assertTrue(any("conflicting_entry" in p for p in problems), problems)

    def test_validate_accepts_a_clean_ledger(self):
        for i, date in enumerate(("2026-10-14", "2026-10-15")):
            append_entry(self.ledger, self.entry(date=date, evidence=f"run-{i}"))
        self.assertEqual(validate(load_entries(self.ledger)), [])


# ---------------------------------------------------------------- 撤销


class RevokeTests(LedgerTestCase):
    def test_revoke_removes_only_matching_key_and_keeps_others(self):
        append_entry(self.ledger, self.entry(rule="Rule to revoke", evidence="run-a"))
        append_entry(self.ledger, self.entry(rule="Rule to keep", evidence="run-b", date="2026-10-15"))
        removed = revoke_key(self.ledger, normalize_key("Rule to revoke"))
        self.assertEqual(removed, 1)
        remaining = load_entries(self.ledger)
        self.assertEqual([e["rule"] for e in remaining], ["Rule to keep"])
        self.assertEqual(stat.S_IMODE(self.ledger.stat().st_mode), 0o600)

    def test_revoke_of_unknown_key_is_refused(self):
        append_entry(self.ledger, self.entry())
        with self.assertRaises(LessonError) as caught:
            revoke_key(self.ledger, "no-such-key")
        self.assertEqual(caught.exception.code, "key_not_found")

    def test_revoke_does_not_change_permissions_of_anything(self):
        append_entry(self.ledger, self.entry(rule="Rule to revoke"))
        before = {p.name: stat.S_IMODE(p.stat().st_mode) for p in self.tmp.iterdir()}
        revoke_key(self.ledger, normalize_key("Rule to revoke"))
        after = {p.name: stat.S_IMODE(p.stat().st_mode) for p in self.tmp.iterdir()}
        self.assertEqual(before, after)
        self.assertTrue(all(mode & stat.S_IWOTH == 0 for mode in after.values()))


# ---------------------------------------------------------------- CLI


class CliTests(LedgerTestCase):
    def cli(self, *args, ledger_path=None):
        script = HERE / "automation/lessons.py"
        env = {**os.environ, "PYTHONDONTWRITEBYTECODE": "1"}
        code = textwrap.dedent(
            """
            import runpy, sys
            from pathlib import Path
            sys.path.insert(0, sys.argv[1])
            import lessons
            lessons.LEDGER = Path(sys.argv[2])
            sys.argv = ["lessons.py", *sys.argv[3:]]
            raise SystemExit(lessons.main())
            """
        )
        return subprocess.run(
            [PY, "-c", code, str(HERE / "automation"), str(ledger_path or self.ledger), *args],
            capture_output=True, text=True, env=env, timeout=60,
        )

    def test_cli_add_is_idempotent_and_validate_reports_promotion_with_evidence(self):
        for date in ("2026-10-14", "2026-10-15", "2026-10-16"):
            proc = self.cli("add", "--rule", "不要相信截断的日志尾部", "--why", "w",
                            "--evidence", f"run-{date}", "--date", date)
            self.assertEqual(proc.returncode, 0, proc.stderr)
        again = self.cli("add", "--rule", "不要相信截断的日志尾部", "--why", "w",
                         "--evidence", "run-2026-10-16", "--date", "2026-10-16")
        self.assertEqual(again.returncode, 0, again.stderr)
        self.assertIn("already recorded", again.stdout)
        self.assertEqual(len(self.raw_lines()), 3)
        proc = self.cli("validate")
        self.assertEqual(proc.returncode, 0, proc.stderr)
        self.assertIn("promotion candidates", proc.stdout)
        self.assertIn("- 不要相信截断的日志尾部", proc.stdout)
        self.assertNotIn("2026-10-14", proc.stdout.split("promotion candidates")[0])
        self.assertIn('dates=[\'2026-10-14\', \'2026-10-15\', \'2026-10-16\']', proc.stdout)

    def test_cli_error_output_carries_only_fixed_codes(self):
        credential = "sk-" + "probe" + "-" + "fedcba9876543210"
        proc = self.cli("add", "--rule", "Valid rule text", "--evidence", "run-a", "--date", "2026-99-99")
        self.assertEqual(proc.returncode, 2)
        self.assertIn("invalid_seen_date", proc.stderr)
        self.assertNotIn("Valid rule text", proc.stderr + proc.stdout)
        # 失败路径不能把原规则/凭据反射出来（旧版把 rule 原文塞进了错误文本）。
        # 注：argv 不能带 NUL（OS 限制），NUL 由单元用例覆盖。
        for bad_rule in (credential + "\t", credential + "\n"):
            with self.subTest(rule=bad_rule):
                proc = self.cli("add", "--rule", bad_rule, "--evidence", "run-a", "--date", "2026-10-14")
                self.assertEqual(proc.returncode, 2)
                self.assertNotIn(credential, proc.stderr + proc.stdout)
                self.assertIn("invalid_rule", proc.stderr)
        proc = self.cli("list")
        self.assertEqual(proc.returncode, 0, proc.stderr)
        self.assertNotIn(credential, proc.stdout)

    def test_cli_revoke_is_an_explicit_file_operation(self):
        proc = self.cli("add", "--rule", "CLI revoke rule", "--evidence", "run-a")
        self.assertEqual(proc.returncode, 0, proc.stderr)
        proc = self.cli("revoke", "--key", normalize_key("CLI revoke rule"))
        self.assertEqual(proc.returncode, 0, proc.stderr)
        self.assertEqual(load_entries(self.ledger), [])

    def test_cli_never_creates_a_global_permission_side_effect(self):
        before = stat.S_IMODE(self.tmp.stat().st_mode)
        self.cli("add", "--rule", "Permission neutral rule", "--evidence", "run-a")
        self.assertEqual(stat.S_IMODE(self.tmp.stat().st_mode), before)
        self.assertEqual(stat.S_IMODE(self.ledger.stat().st_mode), 0o600)

    def test_cli_reports_a_corrupt_ledger_instead_of_listing_nothing(self):
        self.ledger.write_bytes(b'{broken\n')
        proc = self.cli("list")
        self.assertEqual(proc.returncode, 2)
        self.assertIn("ledger_json_invalid", proc.stderr)
        proc = self.cli("validate")
        self.assertEqual(proc.returncode, 2)
        self.assertIn("ledger_json_invalid", proc.stderr)


# ---------------------------------------------------------------- 兼容性（静态）


class CompatibilityTests(unittest.TestCase):
    def read(self, relative):
        return (HERE / relative).read_text(encoding="utf-8")

    def test_pipeline_imports_are_still_satisfied(self):
        pipeline_source = self.read("automation/pipeline.py")
        for name in ("LessonError", "append_entry", "load_entries", "make_entry",
                     "promotion_candidates"):
            self.assertIn(name, pipeline_source, name)
        match = re.search(r"from lesson_ledger import \(([^)]*)\)", pipeline_source)
        self.assertIsNotNone(match)
        self.assertNotIn("normalize_key", match.group(1))

    def test_runner_meta_tests_only_override_the_ledger_path(self):
        runner_source = self.read("test-automation-runner.py")
        self.assertIn("pipeline.LESSON_LEDGER = root / '.scratch/lessons.jsonl'", runner_source)

    def test_public_interface_names_are_stable(self):
        for name in ("normalize_key", "make_entry", "load_entries", "append_entry",
                     "promotion_candidates", "validate", "revoke_key", "LessonError"):
            self.assertTrue(callable(getattr(ledger, name, None)), name)
        self.assertIsInstance(ledger.MAX_LEDGER_BYTES, int)

    def test_selftest_and_ledger_agree_on_the_idempotent_append_contract(self):
        """自测与账本必须对「重复追加」给出同一套契约。

        这里断言的是**一致性**而不是某个字符串还在：`append_entry` 返回 `duplicate`
        且账本不增行，自测里也必须按这个契约断言（旧的「重复写 + validate 报错」
        写法已随契约变更移除，不能重新出现）。
        """
        selftest_source = self.read("automation/selftest.py")
        self.assertNotIn("lessons: validate flags exact duplicate", selftest_source)
        self.assertIn("lessons: duplicate append is idempotent", selftest_source)
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "lessons.jsonl"
            dup = make_entry("Duplicate entry", "", "same-evidence", "2026-10-14", STAMP)
            self.assertEqual(append_entry(path, dup), "appended")
            self.assertEqual(append_entry(path, dict(dup)), "duplicate")
            self.assertEqual(validate(load_entries(path)), [])
            self.assertEqual(len(path.read_text(encoding="utf-8").splitlines()), 1)


if __name__ == "__main__":
    unittest.main(verbosity=2)
