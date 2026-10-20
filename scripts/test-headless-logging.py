#!/usr/bin/env python3
"""BUG-003: execute the real logger/startup boundary with newly registered handlers.

The small rustc harness uses production runtime_log.rs, headless.rs, and main's
pre-GUI startup fragment. Only unrelated CLI bodies are stubbed. Two fresh handlers
(one at table head, one at table tail) are injected into the dispatch table,
without any logger flag or registration. All file checks use a private temp root.
--binary additionally checks the shipped entrypoints against an isolated
HOME/config/TMPDIR, never the running user's app.
"""
import argparse
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
SRC = ROOT / 'app/src-tauri/src'
MODE = argparse.ArgumentParser(description=__doc__)
MODE.add_argument('--binary', type=Path)
ARGS, REMAINING = MODE.parse_known_args()


def environment(root):
    home = root / 'home'
    temporary = root / 'tmp'
    config = root / 'config'
    for path in (home, temporary, config):
        path.mkdir(exist_ok=True)
    return {**os.environ, 'HOME': str(home), 'USERPROFILE': str(home),
            'XDG_CONFIG_HOME': str(config), 'APPDATA': str(config), 'LOCALAPPDATA': str(config),
            'TMPDIR': str(temporary), 'TMP': str(temporary), 'TEMP': str(temporary)}


def build_harness(root, mutant=False):
    logger = SRC / 'runtime_log.rs'
    registry = (SRC / 'headless.rs').read_text(encoding='utf-8')
    head_anchor = 'dispatch_with(argv, &[\n'
    tail_anchor = '        crate::cli::try_run,\n'
    if registry.count(head_anchor) != 1 or registry.count(tail_anchor) != 1:
        raise AssertionError('dispatch table anchors changed; inspect the production entrypoint')
    # 首尾各注入一个未登记的新探针：防首位漏判，也防有人在中间的 handler 提前开启日志
    registry = registry.replace(head_anchor, head_anchor + '        crate::fresh_probe,\n', 1)
    registry = registry.replace(tail_anchor, tail_anchor + '        crate::fresh_probe_tail,\n', 1)
    dispatch = root / 'headless.rs'
    dispatch.write_text(registry, encoding='utf-8')
    source_main = (SRC / 'main.rs').read_text(encoding='utf-8')
    log_start = source_main.index('pub(crate) fn log_line(')
    log_end = source_main.index('\nfn engine_loop(', log_start)
    log_entry = source_main[log_start:log_end]
    main = source_main.split('fn main() {', 1)[1]
    start = main.index('    let argv: Vec<String> = std::env::args().collect();')
    end = main.index('    let (tx, rx) = mpsc::channel::<hook_event::HookEvent>();')
    startup = main[start:end]
    # Compileable mutation: authorize user-file logging before any handler runs.
    if mutant:
        startup = '    runtime_log::enable_gui(false);\n' + startup
    harness = '''
#[path = LOGGER] mod runtime_log;
#[path = DISPATCH] mod headless;
LOG_ENTRY
fn isolated_instance_requested() -> bool { std::env::args().any(|arg| arg == "--ui-smoke") }
macro_rules! declined { ($name:ident) => { mod $name { pub fn cli(_: &[String]) -> Option<i32> { None } } }; }
declined!(notify_ledger_probe);
declined!(gateway_auth_probe);
declined!(remote_delivery_probe);
declined!(claude_plan_receiver);
declined!(resource_diagnostics);
mod cli { pub fn try_run(_: &[String]) -> Option<i32> { None } }
fn fresh_probe(argv: &[String]) -> Option<i32> {
    if argv.get(1).map(String::as_str) != Some("--fresh-probe") { return None; }
    log_line("synthetic fresh handler diagnostic");
    std::thread::spawn(|| log_line("synthetic fresh handler worker")).join().unwrap();
    Some(if argv.len() == 2 { 0 } else { 2 })
}
fn fresh_probe_tail(argv: &[String]) -> Option<i32> {
    if argv.get(1).map(String::as_str) != Some("--fresh-probe-tail") { return None; }
    log_line("synthetic tail handler diagnostic");
    std::thread::spawn(|| log_line("synthetic tail handler worker")).join().unwrap();
    Some(if argv.len() == 2 { 0 } else { 2 })
}
fn main() {
STARTUP
    log_line("synthetic GUI diagnostic");
}
'''.replace('LOGGER', '"' + logger.as_posix() + '"').replace('DISPATCH', '"' + dispatch.as_posix() + '"').replace('STARTUP', startup).replace('LOG_ENTRY', log_entry)
    source = root / 'harness.rs'
    source.write_text(harness, encoding='utf-8')
    binary = root / ('harness.exe' if os.name == 'nt' else 'harness')
    compiled = subprocess.run(['rustc', '--edition=2021', str(source), '-o', str(binary)],
                              capture_output=True, timeout=60)
    if compiled.returncode:
        raise AssertionError('production logging harness did not compile; inspect source integration')
    return binary


class LoggingBoundaryTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.directory = tempfile.TemporaryDirectory(prefix='agentisland-log-harness-')
        cls.addClassCleanup(cls.directory.cleanup)
        cls.root = Path(cls.directory.name)
        cls.binary = build_harness(cls.root)
        mutant = cls.root / 'mutant'
        mutant.mkdir()
        cls.mutant = build_harness(mutant, mutant=True)

    def run_case(self, args, contents, binary=None):
        with tempfile.TemporaryDirectory(prefix='agentisland-log-case-') as directory:
            root = Path(directory)
            env = environment(root)
            log = root / 'tmp/agentisland-tauri.log'
            if contents is not None:
                log.write_bytes(contents)
            result = subprocess.run([str(binary or self.binary), *args], env=env, capture_output=True, timeout=15)
            after = log.read_bytes() if log.exists() else None
            isolated = list((root / 'tmp').glob('agentisland-test-*.log'))
            return result.returncode, after, [path.read_bytes() for path in isolated], result.stdout, result.stderr

    def test_new_handler_and_worker_never_create_append_or_truncate_user_logs(self):
        for flag in ('--fresh-probe', '--fresh-probe-tail'):
            for args, code in [([flag], 0), ([flag, 'invalid'], 2)]:
                for original in (None, b'synthetic existing log\n', b'x' * 2_097_153):
                    with self.subTest(flag=flag, args=args, size=None if original is None else len(original)):
                        rc, after, isolated, stdout, stderr = self.run_case(args, original)
                        self.assertEqual(rc, code)
                        self.assertEqual(after, original, 'a new headless handler modified the user log')
                        self.assertEqual(isolated, [])
                        self.assertEqual(stdout, b'')
                        self.assertEqual(stderr, b'')

    def test_gui_still_appends_and_truncates_at_the_real_cap(self):
        rc, after, isolated, _, _ = self.run_case([], b'existing\n')
        self.assertEqual(rc, 0)
        self.assertEqual(after, b'existing\nsynthetic GUI diagnostic\n')
        self.assertEqual(isolated, [])
        rc, after, _, _, _ = self.run_case([], b'x' * 2_097_153)
        self.assertEqual(rc, 0)
        self.assertTrue(after.startswith(b'[run] '))
        self.assertTrue(after.endswith(b'synthetic GUI diagnostic\n'))
        self.assertLess(len(after), 1024)

    def test_isolated_gui_keeps_its_pid_specific_log(self):
        rc, after, isolated, _, _ = self.run_case(['--ui-smoke'], b'user log\n')
        self.assertEqual(rc, 0)
        self.assertEqual(after, b'user log\n')
        self.assertEqual(isolated, [b'synthetic GUI diagnostic\n'])

    def test_early_gui_enable_mutation_reproduces_the_bug(self):
        for original in (None, b'old log\n', b'x' * 2_097_153):
            rc, after, _, _, _ = self.run_case(['--fresh-probe'], original, binary=self.mutant)
            self.assertEqual(rc, 0, 'mutant must compile and execute normally')
            self.assertNotEqual(after, original, 'the unchanged-file assertion must reject early logging')


class ShippedBinaryTests(unittest.TestCase):
    @unittest.skipUnless(ARGS.binary, 'shipped-binary mode requires --binary')
    def test_existing_probe_and_cli_entrypoints_leave_fixture_logs_unchanged(self):
        binary = ARGS.binary.resolve(strict=True)
        with tempfile.TemporaryDirectory(prefix='agentisland-log-binary-') as directory:
            root = Path(directory)
            env = environment(root)
            log = root / 'tmp/agentisland-tauri.log'
            original = b'synthetic user log\n'
            log.write_bytes(original)
            cases = [
                (['--notify-ledger-probe', 'invalid'], 2),
                (['--remote-delivery-probe', 'invalid'], 2),
                (['--gateway-auth-probe', 'invalid'], 2),
                (['--claude-plan-hook', 'invalid'], 1),
                (['--memory-core-probe', '--probe-samples', '0'], 2),
                (['--memory-core-probe', '--process-only', '--probe-samples', '1'], 0),
                (['unknown-command'], 2),
                (['--selftest'], 0),
                (['--gateway-auth-probe'], 0),
            ]
            for argv, expected in cases:
                with self.subTest(command=argv[0]):
                    result = subprocess.run([str(binary), *argv], env=env, capture_output=True, timeout=90)
                    self.assertEqual(result.returncode, expected, 'unexpected headless exit code')
                    self.assertEqual(log.read_bytes(), original, 'headless binary modified the user log')
                    self.assertEqual(list((root / 'tmp').glob('agentisland-test-*.log')), [])


if __name__ == '__main__':
    unittest.main(argv=[sys.argv[0], *REMAINING], verbosity=2)
