#!/usr/bin/env python3
"""Snapshot safety tests: synthetic temporary repos; no real model or shared index.

Run: python3.14 scripts/test-automation-snapshot.py
Sensitive-shaped fixture inputs are assembled at runtime, never printed. The
checker itself writes no files. Git writes are restricted to temporary repos.
"""
from __future__ import annotations

from contextlib import redirect_stderr, redirect_stdout
import io
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.dont_write_bytecode = True
SCRIPTS = Path(__file__).resolve().parent
sys.path.insert(0, str(SCRIPTS / "automation"))
import snapshot_safety as safety


def credential() -> bytes:
    return ("s" + "k" + "-").encode() + b"Z" * 24


class SnapshotTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="snapshot-safety-test-")
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.git("init", "-q")
        self.write("source.txt", b"ordinary legal text\n")
        self.git("add", "--", "source.txt")
        self.index = (self.root / ".git/index").read_bytes()
        self.objects = self.object_files()

    def git(self, *args, data=None):
        proc = subprocess.run(["git", *args], cwd=self.root, input=data, capture_output=True)
        if proc.returncode:
            self.fail("synthetic git command failed")
        return proc.stdout

    def write(self, name, data):
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
        return path

    def object_files(self):
        return {p.relative_to(self.root / ".git/objects"): p.read_bytes()
                for p in (self.root / ".git/objects").rglob("*") if p.is_file()}

    def install_scanner(self):
        self.write(safety.SCANNER, (SCRIPTS / "scan-secrets.sh").read_bytes())
        self.write(safety.BASELINE, b"# synthetic approved rows\n")

    def unchanged_git(self):
        self.assertTrue((self.root / ".git/index").read_bytes() == self.index,
                        "main index changed")
        self.assertTrue(self.object_files() == self.objects, "Git objects changed")

    def rejected(self, names, sensitive=b"", **kwargs):
        out, err = io.StringIO(), io.StringIO()
        with redirect_stdout(out), redirect_stderr(err):
            try:
                safety.prepare_snapshot(self.root, names, **kwargs)
            except safety.SnapshotSafetyError as failure:
                message = str(failure)
                self.assertTrue(message.startswith("snapshot_"), "unexpected error code")
                self.assertTrue(not sensitive or sensitive not in message.encode(),
                                "sensitive input escaped via exception")
                self.assertIsNone(failure.__cause__)
            else:
                self.fail("unsafe snapshot accepted")
        self.assertTrue(not out.getvalue() and not err.getvalue(), "checker emitted output")
        self.unchanged_git()

    def test_plain_text_and_missing_scanner_fixture_pass(self):
        entries = safety.prepare_snapshot(self.root, ["source.txt", "deleted.txt", "source.txt"])
        self.assertEqual(len(entries), 1)
        self.assertEqual(entries[0].data, b"ordinary legal text\n")
        self.unchanged_git()

    def test_ordinary_tracked_file_generated_credential_rejected(self):
        secret = credential()
        self.write("source.txt", b"ordinary prose " + secret + b"\n")
        self.rejected(["source.txt"], secret)

    def test_production_snapshot_rejects_before_any_git_object_write(self):
        import pipeline
        self.git('-c', 'user.name=Snapshot Test', '-c', 'user.email=test@example.invalid',
                 'commit', '-qm', 'fixture')
        parent = self.git('rev-parse', 'HEAD').decode().strip()
        self.index = (self.root / '.git/index').read_bytes()
        objects = self.object_files()
        self.write('notes.txt', b'private synthetic marker ' + credential())
        with self.assertRaises(pipeline.PipelineError) as failure:
            pipeline.snapshot_commit(self.root, parent)
        self.assertEqual(str(failure.exception), 'snapshot_content_rejected')
        self.assertTrue(self.object_files() == objects, 'Rejected bytes entered Git objects')
        self.assertEqual((self.root / '.git/index').read_bytes(), self.index)

    def test_all_files_scanned_before_any_hash_object(self):
        self.write("z-final.txt", credential())
        writes = []
        def integration_bridge():
            entries = safety.prepare_snapshot(self.root, ["source.txt", "z-final.txt"])
            for entry in entries:
                writes.append(entry.name)
                self.git("hash-object", "-w", "--no-filters", "--stdin", data=entry.data)
        with self.assertRaises(safety.SnapshotSafetyError):
            integration_bridge()
        self.assertEqual(writes, [])
        self.unchanged_git()

    def test_binary_payload_credential_rejected(self):
        self.write("image.dat", b"\0binary\xff" + credential() + b"\0")
        self.rejected(["image.dat"], credential())

    def test_hard_rule_cannot_use_inline_exemption(self):
        self.install_scanner()
        self.write("source.txt", credential() + b" # nosec: synthetic fixture\n")
        self.rejected(["source.txt"], credential())

    def test_private_key_header_rejected(self):
        marker = b"-" * 5 + b"BEGIN " + b"PRIVATE " + b"KEY" + b"-" * 5
        self.write("notes.md", marker)
        self.rejected(["notes.md"], marker)

    def test_minimum_assignment_guard_without_scanner(self):
        value = b"synthetic" + b"-value"
        line = b"pass" + b"word = '" + value + b"'\n"
        self.write("notes.md", line)
        self.rejected(["notes.md"], value)

    def test_minimum_url_guard_without_scanner(self):
        value = b"Z" * 16
        self.write("notes.md", b"https://example.invalid/?to" + b"ken=" + value)
        self.rejected(["notes.md"], value)

    def test_project_scanner_reads_content_of_exact_selection(self):
        self.install_scanner()
        value = b"fixture" + b"@" + b"example.com"
        self.write("not-selected.txt", value)
        entries = safety.prepare_snapshot(self.root, ["source.txt"])
        self.assertEqual(len(entries), 1)
        self.rejected(["source.txt", "not-selected.txt"], value)

    def test_project_valid_text_passes(self):
        self.install_scanner()
        result = safety.prepare_snapshot(self.root, ["source.txt"], require_project_scanner=True)
        self.assertEqual(result[0].git_mode, "100644")
        self.unchanged_git()

    def test_baseline_uses_real_posix_checksum(self):
        self.install_scanner()
        line = b"fixture" + b"@" + b"example.com"
        checksum = subprocess.run(["cksum"], input=line, capture_output=True, check=True).stdout.split()[0]
        self.assertEqual(safety._checksum(line), checksum.decode())
        self.write("source.txt", line + b"\n")
        self.write(safety.BASELINE, b"email|source.txt|" + checksum + b"\n")
        self.assertEqual(len(safety.prepare_snapshot(self.root, ["source.txt"])), 1)
        self.write("other.txt", line + b"\n")
        self.rejected(["other.txt"], line)

    def test_soft_assignment_baseline_retains_scanner_semantics(self):
        self.install_scanner()
        line = b"pass" + b"word = 'synthetic" + b"-value'"
        self.write("source.txt", line + b"\n")
        self.write(safety.BASELINE, ("secret_assign|source.txt|" + safety._checksum(line) + "\n").encode())
        self.assertEqual(len(safety.prepare_snapshot(self.root, ["source.txt"])), 1)

    def test_hard_baseline_is_invalid(self):
        self.install_scanner()
        self.write(safety.BASELINE, b"cred_prefix|source.txt|123\n")
        self.rejected(["source.txt"])

    def test_project_scanner_missing_baseline_fails_closed(self):
        self.install_scanner()
        (self.root / safety.BASELINE).unlink()
        self.rejected(["source.txt"])

    def test_required_scanner_missing_fails_closed(self):
        self.rejected(["source.txt"], require_project_scanner=True)

    def test_malformed_scanner_fails_closed_without_execution(self):
        self.install_scanner()
        self.write(safety.SCANNER, b"exit 0\n")
        self.rejected(["source.txt"])

    def test_io_error_is_fixed_and_does_not_echo_sensitive_value(self):
        secret = credential()
        with patch.object(safety.os, "read", side_effect=OSError(secret.decode())):
            self.rejected(["source.txt"], secret)

    def test_regex_error_is_fixed_and_does_not_echo_sensitive_value(self):
        self.install_scanner()
        secret = credential()
        with patch.dict(os.environ, USER="[" + secret.decode()):
            self.rejected(["source.txt"], secret)

    def test_symlink_target_bytes_are_scanned(self):
        target = credential().decode()
        (self.root / "ordinary-link").symlink_to(target)
        self.rejected(["ordinary-link"], credential())

    def test_symlinked_parent_and_external_target_rejected(self):
        (self.root / "sub").symlink_to(self.root, target_is_directory=True)
        self.rejected(["sub/source.txt"])
        (self.root / "link").symlink_to("../outside")
        self.rejected(["link"])

    def test_invalid_path_and_special_file_fail_closed(self):
        self.rejected(["../source.txt"])
        self.rejected([".git/config"])
        self.rejected(["./source.txt"])
        os.mkfifo(self.root / "pipe")
        self.rejected(["pipe"])

    def test_sensitive_filename_never_in_error_or_repr(self):
        name = credential().decode() + ".txt"
        self.write(name, b"legal\n")
        self.rejected([name], credential())
        entry = safety.SnapshotEntry(name, "100644", credential())
        self.assertTrue(credential().decode() not in repr(entry), "entry repr disclosed input")

    def test_frozen_bytes_survive_source_change_before_git_write(self):
        entries = safety.prepare_snapshot(self.root, ["source.txt"])
        self.write("source.txt", credential())
        # This is the integration contract: only vetted bytes go to Git stdin.
        blob = self.git("hash-object", "-w", "--no-filters", "--stdin", data=entries[0].data).strip().decode()
        stored = self.git("cat-file", "blob", blob)
        self.assertTrue(stored == b"ordinary legal text\n", "Git wrote unvetted bytes")
        self.assertTrue(credential() not in stored, "Git contains changed unsafe content")
        self.assertTrue((self.root / ".git/index").read_bytes() == self.index, "main index changed")
        with self.assertRaises(Exception):
            entries[0].data = b"mutate"

    def test_project_regex_matches_grep_ere_on_generated_corpus(self):
        source = (SCRIPTS / "scan-secrets.sh").read_bytes()
        identity = "fixture_identity"
        raw = safety._array(source.decode(), "RULE_PAT", True)
        with patch.dict(os.environ, USER=identity):
            rules = safety._project_rules(source)
        corpus = [
            b"ordinary legal text", credential(),
            ("g" + "hp_").encode() + b"A" * 24,
            ("github" + "_pat_").encode() + b"A_" * 16,
            ("gl" + "pat-").encode() + b"A" * 24,
            ("xo" + "xb-").encode() + b"A" * 16,
            ("AK" + "IA").encode() + b"A" * 16,
            ("AI" + "za").encode() + b"A" * 28,
            ("ya" + "29.").encode() + b"A" * 24,
            b"-" * 5 + b"BEGIN RSA " + b"PRIVATE " + b"KEY" + b"-" * 5,
            b"pass" + b"word\t= 'synthetic" + b"-value'",
            b"api" + b"_key\r: 'synthetic" + b"-value'",
            b"?to" + b"ken=" + b"A" * 16,
            ("/Users/" + identity).encode(), identity.encode(),
            b"fixture" + b"@" + b"example.com", b"1" + b"3" + b"8" * 9,
            b"binary\0" + credential(),
        ]
        for (rule, compiled), pattern in zip(rules, raw):
            pattern = pattern.replace("$WHOAMI", identity)
            for payload in corpus:
                proc = subprocess.run(["grep", "-a", "-q", "-E", "--", pattern],
                                      input=payload + b"\n", capture_output=True,
                                      env={**os.environ, "LC_ALL": "C"})
                self.assertTrue(proc.returncode in (0, 1), "ERE probe failed")
                self.assertTrue(bool(compiled.search(payload)) == (proc.returncode == 0),
                                "rule conversion differs: " + rule)

    def test_checker_performs_no_subprocess_or_file_writes(self):
        self.install_scanner()
        real_open = safety.os.open
        def readonly_open(path, flags, *args, **kwargs):
            if flags & (os.O_CREAT | os.O_WRONLY | os.O_RDWR | os.O_TRUNC):
                self.fail("checker requested a writable descriptor")
            return real_open(path, flags, *args, **kwargs)
        with patch.object(safety.os, "open", side_effect=readonly_open), \
                patch.object(subprocess, "run", side_effect=AssertionError("checker ran a subprocess")), \
                patch.object(Path, "write_bytes", side_effect=AssertionError("checker wrote bytes")), \
                patch.object(Path, "write_text", side_effect=AssertionError("checker wrote text")):
            self.assertEqual(len(safety.prepare_snapshot(self.root, ["source.txt"])), 1)
        self.unchanged_git()

    def test_selected_scanner_deleted_fails_closed(self):
        self.rejected(["source.txt", safety.SCANNER])

    def test_source_size_limit_fails_closed(self):
        with patch.object(safety, "MAX_FILE_BYTES", 4):
            self.rejected(["source.txt"])

    def test_executable_and_safe_symlink_modes(self):
        (self.root / "source.txt").chmod(0o755)
        (self.root / "link").symlink_to("source.txt")
        result = safety.prepare_snapshot(self.root, ["source.txt", "link"])
        self.assertEqual([entry.git_mode for entry in result], ["120000", "100755"])
        self.unchanged_git()


def break_experiments() -> int:
    """Structured Break outlet: prove the guard rejects, and that removing it falsely passes.

    The second row is the ablation: with the content check bypassed the same bytes are
    accepted, which is what makes the first row evidence rather than a coincidence.
    """
    import json as _json
    import tempfile as _tempfile
    import pipeline
    results = []
    for name in ("baseline_rejects_credential_bytes", "mutant_accepts_when_check_removed"):
        with _tempfile.TemporaryDirectory(prefix="snapshot-break-") as directory:
            root = Path(directory).resolve()
            subprocess.run(["git", "init", "-q"], cwd=root, check=True, capture_output=True)
            (root / "source.txt").write_bytes(b"ordinary legal text\n")
            subprocess.run(["git", "add", "--", "source.txt"], cwd=root, check=True, capture_output=True)
            subprocess.run(["git", "-c", "user.name=t", "-c", "user.email=t@example.invalid",
                            "commit", "-qm", "fixture"], cwd=root, check=True, capture_output=True)
            parent = subprocess.run(["git", "rev-parse", "HEAD"], cwd=root, check=True,
                                    capture_output=True).stdout.decode().strip()
            (root / "notes.txt").write_bytes(b"private synthetic marker " + credential())
            bypass = name.startswith("mutant_")
            if bypass:
                with patch.object(safety, "prepare_snapshot",
                                  return_value=(safety.SnapshotEntry("source.txt", "100644",
                                                                     b"ordinary legal text\n"),)):
                    outcome = _snapshot_outcome(pipeline, root, parent)
            else:
                outcome = _snapshot_outcome(pipeline, root, parent)
            rejected = outcome == "snapshot_content_rejected"
            results.append({"id": name, "baseline_rejected": rejected,
                            "mutant_false_green": rejected})
    if not any(row["baseline_rejected"] for row in results):
        raise SystemExit("baseline did not reject")
    print(_json.dumps({"schema_version": 1, "experiments": [
        {"id": results[0]["id"], "baseline_rejected": results[0]["baseline_rejected"],
         "mutant_false_green": not results[1]["baseline_rejected"]}]}, sort_keys=True))
    return 0


def _snapshot_outcome(pipeline, root, parent) -> str:
    try:
        pipeline.snapshot_commit(root, parent)
    except pipeline.PipelineError as error:
        return str(error)
    return "accepted"


if __name__ == "__main__":
    if sys.argv[1:] == ["--break-experiments"]:
        sys.exit(break_experiments())
    unittest.main(verbosity=2)
