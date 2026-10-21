#!/usr/bin/env python3
"""Offline adapter regressions: fake packages, mock browser runner, fake cargo only."""
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import unittest

SOURCE = Path(__file__).resolve().parent / 'automation'
NODE = shutil.which('node')
TEST = 'test-session-refresh-focus.mjs'
SENSITIVE = 'SYNTHETIC_PRIVATE_OUTPUT_MUST_NOT_BE_REPORTED'


class Adapters(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='adapter-fixture-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.scripts = self.root / 'scripts'
        (self.scripts / 'automation').mkdir(parents=True)
        for name in ('browser-check.mjs', 'manual-rust.py'):
            shutil.copy2(SOURCE / name, self.scripts / 'automation' / name)
        self.lock = {'packages': {'node_modules/playwright': {'version': '1.64.0'}}}
        (self.root / 'package-lock.json').write_text(json.dumps(self.lock))
        self.marker = self.root / 'runner.json'
        (self.scripts / TEST).write_text("""
import fs from 'node:fs';
fs.writeFileSync(process.env.RUN_MARKER, JSON.stringify({module: process.env.TEST_BROWSER_MODULE, chromium: process.env.TEST_CHROMIUM_PATH}));
process.exit(Number(process.env.RUN_RC || '0'));
""")
        self.env = {k: v for k, v in os.environ.items() if k not in (
            'TEST_BROWSER_MODULE', 'TEST_CHROMIUM_PATH', 'ALLOW_PRIVATE_TEST_DATA',
            'MANUAL_RUST_TEST', 'MANUAL_RUST_TIMEOUT_SECONDS', 'NODE_OPTIONS', 'NODE_PATH')}
        self.env.update(RUN_MARKER=str(self.marker), HOME=str(self.root),
                        CARGO_HOME=str(self.root / 'cargo-home'))
        self.browser = self.root / 'chromium'
        self.browser.write_text('fake executable; never launched\n')
        self.browser.chmod(0o700)
        self.package = self.make_package(self.root / 'node_modules/playwright')
        self.bin = self.root / 'bin'
        self.bin.mkdir()
        self.calls = self.root / 'calls.jsonl'
        python = self.bin / 'python'
        python.symlink_to(sys.executable)
        cargo = self.bin / 'cargo'
        cargo.write_text('#!' + str(python) + '\n' + '''
import json, os, signal, subprocess, sys, time
from pathlib import Path
args = sys.argv[1:]
with open(os.environ['CARGO_CALLS'], 'a') as out:
    out.write(json.dumps(args) + '\\n')
listing = '--list' in args
phase = 'LIST' if listing else 'RUN'
print('SYNTHETIC_PRIVATE_OUTPUT_MUST_NOT_BE_REPORTED', file=sys.stderr, flush=True)
if os.environ.get('HANG_PHASE') == phase or os.environ.get('SPAWN_PHASE') == phase:
    child = subprocess.Popen([sys.executable, '-c', "import signal,time; signal.signal(signal.SIGTERM,signal.SIG_IGN); time.sleep(60)"])
    Path(os.environ['CHILD_PID']).write_text(str(child.pid))
    if os.environ.get('HANG_PHASE') == phase:
        time.sleep(60)
if listing:
    print(os.environ.get('COMPILED_LIST', 'suite::probe: test'))
else:
    print('SYNTHETIC_PRIVATE_OUTPUT_MUST_NOT_BE_REPORTED', flush=True)
sys.exit(int(os.environ.get(phase + '_RC', '0')))
''')
        cargo.chmod(0o700)
        self.env.update(PATH=str(self.bin), CARGO_CALLS=str(self.calls),
                        CHILD_PID=str(self.root / 'child.pid'),
                        ALLOW_PRIVATE_TEST_DATA='1', MANUAL_RUST_TEST='probe',
                        MANUAL_RUST_TIMEOUT_SECONDS='3')
        self.addCleanup(self.cleanup_child)

    def cleanup_child(self):
        pidfile = self.root / 'child.pid'
        if pidfile.exists():
            try:
                os.kill(int(pidfile.read_text()), signal.SIGKILL)
            except ProcessLookupError:
                pass

    def make_package(self, directory, version='1.64.0', name='playwright'):
        directory.mkdir(parents=True, exist_ok=True)
        (directory / 'package.json').write_text(json.dumps({'name': name, 'version': version, 'type': 'module'}))
        (directory / 'index.mjs').write_text("""
import fs from 'node:fs';
if (process.env.IMPORT_MARKER) fs.writeFileSync(process.env.IMPORT_MARKER, 'imported');
export const chromium = { executablePath() { return %s; } };
""" % json.dumps(str(self.browser)))
        return directory / 'index.mjs'

    def browser_run(self, rc=0, args=None, **env):
        result = subprocess.run([NODE, str(self.scripts / 'automation/browser-check.mjs'),
                                 *(args if args is not None else [TEST])],
                                env={**self.env, **env}, capture_output=True, text=True, timeout=120)
        self.assertEqual(result.returncode, rc, result.stdout + result.stderr)
        self.assertNotIn('Traceback', result.stderr)
        return result

    def manual_run(self, rc=0, **env):
        result = subprocess.run([sys.executable, str(self.scripts / 'automation/manual-rust.py')],
                                env={**self.env, **env}, capture_output=True, text=True, timeout=120)
        self.assertEqual(result.returncode, rc, result.stdout + result.stderr)
        self.assertNotIn('Traceback', result.stderr)
        self.assertNotIn(SENSITIVE, result.stdout + result.stderr)
        return result

    def get_calls(self):
        return [json.loads(line) for line in self.calls.read_text().splitlines()] if self.calls.exists() else []

    def assert_child_stopped(self):
        pid = int((self.root / 'child.pid').read_text())
        # A killed orphan can remain a zombie briefly; it cannot continue work.
        result = subprocess.run(['/bin/ps', '-o', 'stat=', '-p', str(pid)], capture_output=True, text=True)
        self.assertTrue(result.returncode != 0 or result.stdout.strip().startswith('Z'),
                        'descendant survived adapter cleanup')

    def test_browser_default_success(self):
        self.browser_run()
        self.assertTrue(self.marker.exists())

    def test_browser_external_override_without_local_package(self):
        external = self.make_package(self.root / 'external playwright')
        shutil.rmtree(self.root / 'node_modules')
        self.browser_run(TEST_BROWSER_MODULE=str(external))
        self.assertTrue(self.marker.exists())

    def test_browser_override_mismatch_before_import(self):
        external = self.make_package(self.root / 'external', version='0.0.1')
        imported = self.root / 'imported'
        self.browser_run(3, TEST_BROWSER_MODULE=str(external), IMPORT_MARKER=str(imported))
        self.assertFalse(imported.exists())
        self.assertFalse(self.marker.exists())

    def test_browser_default_mismatch_before_import(self):
        self.make_package(self.package.parent, version='0.0.1')
        imported = self.root / 'imported'
        self.browser_run(3, IMPORT_MARKER=str(imported))
        self.assertFalse(imported.exists())
        self.assertFalse(self.marker.exists())

    def test_browser_wrong_name(self):
        self.make_package(self.package.parent, name='unrelated')
        self.browser_run(3)
        self.assertFalse(self.marker.exists())

    def test_browser_missing_chromium(self):
        self.browser.unlink()
        self.browser_run(3)
        self.assertFalse(self.marker.exists())

    def test_browser_unexecutable_chromium(self):
        self.browser.chmod(0o600)
        self.browser_run(3)
        self.assertFalse(self.marker.exists())

    def test_browser_chromium_override_reports_exception(self):
        override = self.root / 'external-chromium'
        override.write_text('never executed')
        override.chmod(0o700)
        self.browser.unlink()
        result = self.browser_run(TEST_CHROMIUM_PATH=str(override))
        self.assertIn('not version-pinned', result.stderr)
        self.assertEqual(json.loads(self.marker.read_text())['chromium'], str(override.resolve()))

    def test_browser_missing_chromium_override(self):
        self.browser_run(3, TEST_CHROMIUM_PATH=str(self.root / 'absent'))
        self.assertFalse(self.marker.exists())

    def test_browser_malformed_package(self):
        (self.package.parent / 'package.json').write_text('{')
        self.browser_run(3)
        self.assertFalse(self.marker.exists())

    def test_browser_missing_lock(self):
        (self.root / 'package-lock.json').unlink()
        self.browser_run(3)

    def test_browser_invalid_module_import(self):
        self.package.write_text('throw new Error("fixture import error");')
        self.browser_run(3)
        self.assertFalse(self.marker.exists())

    def test_browser_symlink_uses_target_package(self):
        external = self.make_package(self.root / 'external', version='0.0.1')
        self.package.unlink()
        self.package.symlink_to(external)
        self.browser_run(3)
        self.assertFalse(self.marker.exists())

    def test_browser_valid_directory_symlink(self):
        external = self.make_package(self.root / 'external')
        shutil.rmtree(self.package.parent)
        self.package.parent.symlink_to(external.parent, target_is_directory=True)
        self.browser_run()

    def test_browser_metadata_symlink_rejected(self):
        metadata = self.package.parent / 'package.json'
        moved = self.root / 'elsewhere.json'
        metadata.rename(moved)
        metadata.symlink_to(moved)
        self.browser_run(3)

    def test_browser_usage(self):
        self.browser_run(2, args=['unregistered.mjs'])
        self.browser_run(2, TEST_BROWSER_MODULE='relative/index.mjs')

    def test_browser_runner_failure(self):
        self.browser_run(1, RUN_RC='7')
        self.assertTrue(self.marker.exists())

    def test_manual_exact_single(self):
        self.manual_run()
        calls = self.get_calls()
        self.assertEqual(len(calls), 2)
        self.assertEqual(calls[0][-3:], ['--', '--list', '--ignored'])
        self.assertEqual(calls[1][-4:], ['suite::probe', '--', '--ignored', '--exact'])
        self.assertIn('--locked', calls[1])

    def test_manual_duplicate_names(self):
        self.manual_run(2, COMPILED_LIST='a::probe: test\nb::probe: test')
        self.assertEqual(len(self.get_calls()), 1)

    def test_manual_duplicate_identical_entries(self):
        self.manual_run(2, COMPILED_LIST='a::probe: test\na::probe: test')
        self.assertEqual(len(self.get_calls()), 1)

    def test_manual_nonexistent(self):
        self.manual_run(2, COMPILED_LIST='suite::other: test')
        self.assertEqual(len(self.get_calls()), 1)

    def test_manual_unauthorized(self):
        self.manual_run(3, ALLOW_PRIVATE_TEST_DATA='0')
        self.assertEqual(self.get_calls(), [])

    def test_manual_invalid_selection(self):
        self.manual_run(2, MANUAL_RUST_TEST='probe --ignored')
        self.assertEqual(self.get_calls(), [])

    def test_manual_missing_cargo(self):
        (self.bin / 'cargo').unlink()
        self.manual_run(3)

    def test_manual_list_nonzero(self):
        self.manual_run(1, LIST_RC='7')
        self.assertEqual(len(self.get_calls()), 1)

    def test_manual_command_nonzero(self):
        self.manual_run(1, RUN_RC='7')
        self.assertEqual(len(self.get_calls()), 2)

    def test_manual_bad_timeout(self):
        for value in ('0', '-1', 'nan', 'inf', '901', 'garbage'):
            self.manual_run(2, MANUAL_RUST_TIMEOUT_SECONDS=value)
        self.assertEqual(self.get_calls(), [])

    def test_manual_timeout_child_cleanup(self):
        for phase in ('LIST', 'RUN'):
            with self.subTest(phase=phase):
                self.manual_run(1, HANG_PHASE=phase)
                self.assert_child_stopped()

    def test_manual_success_child_cleanup(self):
        self.manual_run(SPAWN_PHASE='RUN')
        self.assert_child_stopped()

    def test_manual_interrupt_child_cleanup(self):
        proc = subprocess.Popen([sys.executable, str(self.scripts / 'automation/manual-rust.py')],
                                env={**self.env, 'HANG_PHASE': 'RUN', 'MANUAL_RUST_TIMEOUT_SECONDS': '5'},
                                stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        self.addCleanup(lambda: proc.kill() if proc.poll() is None else None)
        deadline = time.monotonic() + 4
        pidfile = self.root / 'child.pid'
        while not pidfile.exists() and time.monotonic() < deadline:
            time.sleep(0.02)
        self.assertTrue(pidfile.exists())
        proc.send_signal(signal.SIGTERM)
        out, err = proc.communicate(timeout=60)
        self.assertEqual(proc.returncode, 1, err)
        self.assertNotIn(SENSITIVE, out + err)
        self.assertNotIn('Traceback', err)
        self.assert_child_stopped()


if __name__ == '__main__':
    if not NODE:
        sys.exit('node is required for these offline fixture tests')
    unittest.main(verbosity=2)
