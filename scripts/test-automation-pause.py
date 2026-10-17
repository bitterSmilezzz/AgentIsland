#!/usr/bin/env python3
"""Pause records and duplicate-input detection.

Run: python3 scripts/test-automation-pause.py
Temporary directories only. Notification channels are replaced by fakes, so no
system notification is ever emitted and no private input is written to disk.
"""
from __future__ import annotations

import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.dont_write_bytecode = True
SCRIPTS = Path(__file__).resolve().parent
sys.path.insert(0, str(SCRIPTS / "automation"))
import pause


def binding(digest: str = "a" * 64, prompt: str = "b" * 64, snapshot: str | None = None) -> dict:
    value = {"trigger": "manual-github", "source_sha256": digest, "prompt_sha256": prompt}
    if snapshot:
        value["snapshot_sha256"] = snapshot
    return value


def run_record(run_id: str, record_binding: dict | None, *, outcome: str = "passed",
               returncode: int = 0) -> dict:
    record = {"run_id": run_id, "outcome": outcome, "returncode": returncode,
              "failed_or_blocked": [] if outcome == "passed" else ["rust.default"],
              "steps": [], "checks": []}
    if record_binding is not None:
        record["input_binding"] = record_binding
    return record


class PauseTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="pause-test-")
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.runs = self.root / "runs"
        self.runs.mkdir()
        self.log = self.root / "notify.log"

    def write_run(self, name: str, record: dict) -> Path:
        directory = self.runs / name
        directory.mkdir()
        (directory / "run.json").write_text(json.dumps(record), encoding="utf-8")
        return directory

    def test_binding_identity_ignores_nothing_and_is_stable(self):
        self.assertEqual(pause.binding_key(binding()), pause.binding_key(binding()))
        self.assertNotEqual(pause.binding_key(binding()), pause.binding_key(binding(prompt="c" * 64)))
        with self.assertRaises(pause.PauseError):
            pause.binding_key({"trigger": "evil", "source_sha256": "a" * 64,
                               "prompt_sha256": "b" * 64})

    def settled_record(self, name: str) -> dict:
        record = run_record(name, binding())
        record["candidate"] = {"kind": "local-only-candidate", "run_id": name}
        return record

    def test_duplicate_input_detected_and_new_run_exempt(self):
        self.write_run("old", self.settled_record("old"))
        self.assertEqual(pause.duplicate_input(self.runs, binding(), "new"), "old")
        self.assertIsNone(pause.duplicate_input(self.runs, binding(), "old"))

    def test_unfinished_run_does_not_block_a_legitimate_retry(self):
        for index, (outcome, returncode) in enumerate(
                (("failed", 1), ("blocked", 3), ("failed", 2))):
            with self.subTest(outcome=outcome, returncode=returncode):
                self.write_run(f"old{index}", run_record(f"old{index}", binding(),
                                                        outcome=outcome, returncode=returncode))
                self.assertIsNone(pause.duplicate_input(self.runs, binding(), "new"))
        self.write_run("settled", self.settled_record("settled"))
        self.assertEqual(pause.duplicate_input(self.runs, binding(), "new"), "settled")

    def test_a_passing_run_without_a_candidate_is_not_settled(self):
        # Profile-mode and configuration runs record passed/0 yet deliver nothing;
        # they must not block a later real development request.
        record = run_record("profile", binding())
        self.assertEqual((record["outcome"], record["returncode"]), ("passed", 0))
        self.write_run("profile", record)
        self.assertIsNone(pause.duplicate_input(self.runs, binding(), "new"))

    def test_different_snapshot_is_not_a_duplicate(self):
        self.write_run("old", run_record("old", binding(snapshot="d" * 64)))
        self.assertIsNone(pause.duplicate_input(self.runs, binding(), "new"))

    def test_same_input_on_changed_source_is_a_legitimate_retry(self):
        record = self.settled_record("old")
        record["origin"] = {"input_digest": "1" * 64}
        self.write_run("old", record)
        self.assertEqual(pause.duplicate_input(self.runs, binding(), "new", "1" * 64), "old")
        self.assertIsNone(pause.duplicate_input(self.runs, binding(), "new", "2" * 64))
        with self.assertRaises(pause.PauseError):
            pause.duplicate_input(self.runs, binding(), "new", "not-a-digest")

    def test_unreadable_history_is_skipped_but_identities_still_match(self):
        # A damaged or foreign record carries no identity: skipping it must not
        # blind the detector, or a corrupt file would hide a real duplicate.
        broken = self.runs / "broken"
        broken.mkdir()
        (broken / "run.json").write_text("{not json", encoding="utf-8")
        link = self.runs / "link"
        link.mkdir()
        (link / "run.json").symlink_to(broken / "run.json")
        empty = self.runs / "empty"
        empty.mkdir()
        (empty / "run.json").write_text(json.dumps({"run_id": "empty"}), encoding="utf-8")
        self.assertIsNone(pause.duplicate_input(self.runs, binding(), "new"))
        self.write_run("old", self.settled_record("old"))
        self.assertEqual(pause.duplicate_input(self.runs, binding(), "new"), "old")

    def test_missing_evidence_root_fails_closed(self):
        with self.assertRaises(pause.PauseError):
            pause.duplicate_input(self.root / "missing", binding(), "new")

    def test_pause_record_has_no_private_text_and_is_private(self):
        # Assembled at runtime: no credential-shaped literal is stored in the repo.
        private_note = "private " + "fixture " + "material"
        record = {"run_id": "run-1", "outcome": "failed", "returncode": 1,
                  "failed_or_blocked": ["rust.default"], "input_binding": binding()}
        run_dir = self.runs / "run-1"
        summary = pause.pause_record(run_dir, record, stage="development", log_path=self.log)
        self.assertEqual(summary["stage"], "development")
        self.assertEqual(summary["notification"]["delivered"], False)
        self.assertEqual(summary["notification"]["detail"], "not_requested")
        self.assertNotIn(private_note, json.dumps(summary))
        self.assertEqual((run_dir / "pause.json").stat().st_mode & 0o777, 0o600)

    def test_unanchored_host_still_writes_a_record_and_declares_its_weakness(self):
        # A host without anchored descriptors must still leave a durable pause
        # record; losing the record entirely is worse than a weaker guarantee.
        record = {"run_id": "run-plain", "outcome": "failed", "returncode": 1,
                  "failed_or_blocked": ["x"]}
        run_dir = self.runs / "run-plain"
        with patch.object(pause, "_anchored", return_value=False):
            summary = pause.pause_record(run_dir, record, stage="run",
                                         log_path=self.log, notify=True)
        written = json.loads((run_dir / "pause.json").read_text())
        self.assertEqual(written["write_guarantee"], "unanchored_no_symlink_refusal")
        self.assertEqual(written["run_id"], "run-plain")
        self.assertEqual(summary["write_guarantee"], "unanchored_no_symlink_refusal")
        self.assertTrue((run_dir / "pause.json").exists(),
                        "a failure record must survive on every supported host")

    def test_anchored_host_declares_the_stronger_guarantee(self):
        summary = pause.pause_record(self.runs / "run-anchored",
                                     {"run_id": "run-anchored", "outcome": "failed",
                                      "returncode": 1, "failed_or_blocked": ["x"]},
                                     stage="run", log_path=self.log)
        self.assertEqual(summary["write_guarantee"], "anchored_dir_fd")

    def test_a_long_failure_list_still_writes_the_pause_record(self):
        # A blocked run with many items must keep its pause record; a notification
        # body limit may not turn it into a failed run or drop the record.
        for count in (1, 57, 200):
            with self.subTest(count=count):
                record = {"run_id": "run-many", "outcome": "blocked", "returncode": 3,
                          "failed_or_blocked": [f"check.item.{index}" for index in range(count)]}
                run_dir = self.runs / f"run-many-{count}"
                with patch.object(pause.subprocess, "run") as runner:
                    runner.side_effect = FileNotFoundError
                    summary = pause.pause_record(run_dir, record, stage="profile",
                                                 log_path=self.log, notify=True)
                written = json.loads((run_dir / "pause.json").read_text())
                self.assertEqual(len(written["failed_or_blocked"]), count)
                self.assertFalse(written["notification"]["delivered"])
                self.assertLessEqual(len(summary["notification"].get("detail", "")), 64)

    def test_delivery_failure_never_hides_the_record(self):
        # deliver() raising is a notification outcome, not a reason to lose pause.json.
        record = {"run_id": "run-raise", "outcome": "failed", "returncode": 1,
                  "failed_or_blocked": ["x"]}
        run_dir = self.runs / "run-raise"
        with patch.object(pause, "deliver", side_effect=pause.PauseError("notify_text_too_long")):
            summary = pause.pause_record(run_dir, record, stage="run",
                                         log_path=self.log, notify=True)
        self.assertTrue((run_dir / "pause.json").is_file())
        self.assertEqual(summary["notification"]["detail"], "notify_notify_text_too_long")

    def test_notification_is_never_sent_unless_requested(self):
        record = {"run_id": "run-2", "outcome": "failed", "returncode": 1, "failed_or_blocked": []}
        with patch.object(pause.subprocess, "run") as runner:
            pause.pause_record(self.runs / "run-2", record, stage="run", log_path=self.log)
        runner.assert_not_called()
        self.assertFalse(self.log.exists())

    def test_channel_failure_degrades_to_log_and_flag(self):
        record = {"run_id": "run-3", "outcome": "failed", "returncode": 1,
                  "failed_or_blocked": ["rust.default"]}
        with patch.object(pause.subprocess, "run", side_effect=FileNotFoundError):
            summary = pause.pause_record(self.runs / "run-3", record, stage="run",
                                         log_path=self.log, notify=True)
        self.assertFalse(summary["notification"]["delivered"])
        self.assertEqual(summary["notification"]["detail"], "channel_unavailable")
        self.assertTrue(self.log.is_file(), "degradation log must be written")
        self.assertIn("channel unavailable", self.log.read_text())

    def test_log_symlink_is_rejected_and_receipt_records_both_failures(self):
        target = self.root / "untouched"
        target.write_bytes(b"original")
        self.log.symlink_to(target)
        with patch.object(pause.subprocess, "run", side_effect=FileNotFoundError):
            summary = pause.pause_record(self.runs / "unsafe-log", run_record("unsafe-log", None),
                                         stage="run", log_path=self.log, notify=True)
        self.assertEqual(target.read_bytes(), b"original")
        self.assertEqual(summary["notification"]["detail"], "channel_unavailable")
        self.assertFalse(summary["notification"]["log"]["written"])
        persisted = json.loads((self.runs / "unsafe-log/pause.json").read_text())
        self.assertEqual(persisted, summary)

    def test_no_test_delivers_a_real_notification(self):
        # This suite must never emit a system notification. Every assertion that
        # could deliver must replace the channel first; deriving that rule from
        # the source (not from a list kept here) is what makes it hold as the
        # suite grows.
        source = Path(__file__).read_text(encoding="utf-8")
        lines = source.splitlines()
        for index, line in enumerate(lines):
            stripped = line.strip()
            # Skip this guard's own source: its literal would otherwise match.
            if "notify=True" not in stripped or stripped.startswith(("if", "#")):
                continue
            window = "\n".join(lines[max(0, index - 8):index + 2])
            with self.subTest(line=index + 1):
                self.assertIn("patch.object", window,
                              "a notify=True call must replace the notification channel")

    def test_log_parent_symlink_and_nonregular_are_rejected(self):
        actual = self.root / "actual"
        actual.mkdir()
        alias = self.root / "alias"
        alias.symlink_to(actual, target_is_directory=True)
        for path in (alias / "notify.log", actual):
            with self.subTest(path=path.name):
                status = pause._append_log(path, "channel_failed")
                self.assertIsInstance(status, dict)
                self.assertFalse(status["written"])
        self.assertFalse((actual / "notify.log").exists())

    def test_log_unwritable_keeps_pause_receipt_readable(self):
        blocked = self.root / "not-a-directory"
        blocked.write_text("original")
        with patch.object(pause.subprocess, "run", side_effect=FileNotFoundError):
            summary = pause.pause_record(self.runs / "log-failed", run_record("log-failed", None),
                                         stage="run", log_path=blocked / "notify.log", notify=True)
        self.assertFalse(summary["notification"]["log"]["written"])
        self.assertEqual(json.loads((self.runs / "log-failed/pause.json").read_text()), summary)

    def test_log_fd_is_private_and_size_limit_is_reported(self):
        status = pause._append_log(self.log, "channel_failed")
        self.assertTrue(status["written"])
        self.assertEqual(self.log.stat().st_mode & 0o777, 0o600)
        self.log.write_bytes(b"x" * (pause.MAX_LOG_BYTES + 1))
        status = pause._append_log(self.log, "channel_failed")
        self.assertFalse(status["written"])
        self.assertEqual(status["detail"], "log_size_limit")

    def test_pause_temporary_symlink_cannot_overwrite_target(self):
        import os
        directory = self.runs / "temp-link"
        directory.mkdir()
        target = self.root / "temp-target"
        target.write_text("original")
        (directory / f"pause.json.new.{os.getpid()}").symlink_to(target)
        pause.pause_record(directory, run_record("temp-link", None), stage="run", log_path=self.log)
        self.assertEqual(target.read_text(), "original")
        self.assertEqual((directory / "pause.json").stat().st_mode & 0o777, 0o600)

    def test_pause_parent_symlink_is_rejected(self):
        alias = self.root / "runs-alias"
        alias.symlink_to(self.runs, target_is_directory=True)
        with self.assertRaises(pause.PauseError):
            pause.pause_record(alias / "unsafe", run_record("unsafe", None), stage="run", log_path=self.log)
        self.assertFalse((self.runs / "unsafe").exists())

    def test_delivered_notification_is_recorded(self):
        class Completed:
            returncode = 0
        with patch.object(pause.subprocess, "run", return_value=Completed()):
            summary = pause.pause_record(self.runs / "run-4",
                                         {"run_id": "run-4", "outcome": "failed",
                                          "returncode": 1, "failed_or_blocked": ["x"]},
                                         stage="run", log_path=self.log, notify=True)
        self.assertTrue(summary["notification"]["delivered"])


if __name__ == "__main__":
    unittest.main()
