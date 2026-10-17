#!/bin/bash
# 合成进程验收；不调用 cargo，不读取真实 dist，不创建真实仓库锁。
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
. "$SCRIPT_DIR/common.sh"
[[ -n "$PYTHON_MIN" ]] || exit 2
exec "$PYTHON_MIN" - "$SCRIPT_DIR" <<'PY'
import json
import os
from pathlib import Path
import select
import signal
import subprocess
import sys
import tempfile
import unittest

HELPER = Path(sys.argv.pop()) / 'automation/heavy_build_lock.py'

class LockTests(unittest.TestCase):
    def setUp(self):
        sandbox = tempfile.TemporaryDirectory(prefix='heavy-lock-')
        self.addCleanup(sandbox.cleanup)
        self.root = Path(sandbox.name)

    def command(self, code):
        return [sys.executable, str(HELPER), '--root', str(self.root), '--',
                sys.executable, '-c', code]

    def run_code(self, code='pass'):
        return subprocess.run(self.command(code), text=True, capture_output=True, timeout=5)

    def holder(self, code="print('READY', flush=True); input()"):
        proc = subprocess.Popen(self.command(code), stdin=subprocess.PIPE,
                                stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        def cleanup():
            if proc.stdin and not proc.stdin.closed:
                proc.stdin.close()
            try:
                proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                proc.kill()
                proc.wait(timeout=5)
            proc.stdout.close()
            proc.stderr.close()
        self.addCleanup(cleanup)
        self.assertTrue(select.select([proc.stdout], [], [], 5)[0], 'holder did not start')
        self.assertEqual(proc.stdout.readline().strip(), 'READY')
        return proc

    def assert_busy(self):
        result = self.run_code("raise SystemExit('must not execute')")
        self.assertEqual(result.returncode, 2, result.stderr)
        self.assertEqual(json.loads(result.stderr)['code'], 'heavy_build_busy')

    def test_live_holder_rejects_second_process_and_preserves_inode(self):
        proc = self.holder()
        path = self.root / '.build/heavy-build.lock'
        before = path.stat().st_ino
        self.assert_busy()
        proc.stdin.write('\n'); proc.stdin.flush()
        self.assertEqual(proc.wait(timeout=5), 0)
        self.assertEqual(self.run_code().returncode, 0)
        self.assertEqual(path.stat().st_ino, before)
        self.assertEqual(path.stat().st_mode & 0o777, 0o600)

    def test_nonzero_status_propagates_and_unlocks(self):
        self.assertEqual(self.run_code('raise SystemExit(42)').returncode, 42)
        self.assertEqual(self.run_code().returncode, 0)

    def test_sigterm_releases_after_holder_exits(self):
        proc = self.holder()
        proc.terminate()
        self.assertEqual(proc.wait(timeout=5), -signal.SIGTERM)
        self.assertEqual(self.run_code().returncode, 0)

    def test_sigkill_releases_after_holder_exits(self):
        proc = self.holder()
        proc.kill()
        self.assertEqual(proc.wait(timeout=5), -signal.SIGKILL)
        self.assertEqual(self.run_code().returncode, 0)

    def test_parent_exit_does_not_unlock_surviving_child(self):
        proc = self.holder("""import os, sys
pid = os.fork()
if pid:
    os._exit(0)
print('READY', flush=True)
sys.stdin.readline()
""")
        self.assertEqual(proc.wait(timeout=5), 0)
        self.assert_busy()
        proc.stdin.write('\n'); proc.stdin.flush()
        self.assertTrue(select.select([proc.stdout], [], [], 5)[0])
        self.assertEqual(proc.stdout.read(), '')  # last inherited fd has closed
        self.assertEqual(self.run_code().returncode, 0)

    def test_symlink_lock_rejected_without_touching_target(self):
        directory = self.root / '.build'
        directory.mkdir()
        target = self.root / 'original'
        target.write_text('preserve')
        (directory / 'heavy-build.lock').symlink_to(target)
        self.assertEqual(self.run_code().returncode, 2)
        self.assertEqual(target.read_text(), 'preserve')

    def test_directory_and_fifo_lock_rejected_without_hanging(self):
        directory = self.root / '.build'
        directory.mkdir()
        path = directory / 'heavy-build.lock'
        path.mkdir()
        self.assertEqual(self.run_code().returncode, 2)
        path.rmdir()
        os.mkfifo(path)
        self.assertEqual(self.run_code().returncode, 2)

    def test_shell_exit_trap_preserved(self):
        result = subprocess.run(
            [sys.executable, str(HELPER), '--root', str(self.root), '--', 'bash', '-c',
             "trap 'echo CALLER_TRAP_RAN' EXIT; exit 17"],
            capture_output=True, text=True, timeout=5)
        self.assertEqual(result.returncode, 17)
        self.assertEqual(result.stdout.strip(), 'CALLER_TRAP_RAN')
        self.assertEqual(self.run_code().returncode, 0)

unittest.main()
PY
