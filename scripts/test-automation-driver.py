#!/usr/bin/env python3
"""Driver regression tests: real subprocesses on a temporary PATH, never real agents.

Run: python3 scripts/test-automation-driver.py
The Python fixture dispatches on a JSON scenario, so more cases can be added
without changing production drivers. Unknown fixture/mock scenarios fail closed.
"""
from __future__ import annotations

import importlib.util
import json
import os
from pathlib import Path
import signal
import stat
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import patch

sys.dont_write_bytecode = True
DRIVER = Path(__file__).resolve().parent / "automation/agent_driver.py"
spec = importlib.util.spec_from_file_location("automation_driver_meta", DRIVER)
driver = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = driver
spec.loader.exec_module(driver)
PY = sys.executable
PRIVATE = "SYNTHETIC_PRIVATE_DRIVER_SENTINEL"
FIXTURE = r'''
import json, os, stat, subprocess, sys, time
from pathlib import Path
cfg = json.loads(Path(os.environ['DRIVER_FIXTURE_CONFIG']).read_text())
mode = cfg.get('mode', 'output')
if mode not in ('output', 'tree', 'query'):
    sys.exit(93)
if mode == 'query':
    assert '--accept-hooks' not in sys.argv
    q = Path(sys.argv[sys.argv.index('--query-file') + 1])
    info = {'path': str(q), 'mode': stat.S_IMODE(q.stat().st_mode),
            'prompt_matches': q.read_text() == cfg['prompt']}
    Path(os.environ['DRIVER_FIXTURE_QUERY']).write_text(json.dumps(info))
if mode == 'tree':
    # The direct child ignores TERM; both child and grandchild must be reclaimed.
    child = "import os,signal,subprocess,sys,time; from pathlib import Path; signal.signal(signal.SIGTERM,signal.SIG_IGN); Path('child.pid').write_text(str(os.getpid())); subprocess.Popen([sys.executable,'-c',\"import os,time; from pathlib import Path; Path('grandchild.pid').write_text(str(os.getpid())); time.sleep(30)\"]); time.sleep(30)"
    subprocess.Popen([sys.executable, '-c', child])
    while not Path('grandchild.pid').exists():
        time.sleep(0.01)
if cfg.get('size'):
    os.write(2 if cfg.get('stderr_only') else 1, b'x' * cfg['size'])
if mode == 'tree' and cfg.get('exit_parent'):
    sys.exit(0)
if mode == 'tree' or cfg.get('wait'):
    time.sleep(30)
sys.stdout.buffer.write(cfg.get('stdout', '').encode())
sys.stderr.buffer.write(cfg.get('stderr', '').encode())
sys.exit(cfg.get('rc', 0))
'''


def agy_payload(**extra):
    return json.dumps({'status': 'OK', 'response': 'verified answer',
                       'conversation_id': 'fixture-conv', 'usage': {'input_tokens': 1}, **extra})


def hermes_result(**extra):
    return json.dumps({'type': 'result', 'text': 'verified answer', 'exit_code': 0,
                       'session_id': 'fixture-session', 'tokens': {'input': 2, 'output': 3},
                       'duration_ms': 1, **extra}) + '\n'


@unittest.skipUnless(os.name == 'posix', 'POSIX executable fixtures and process group guarantees')
class DriverTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix='automation-driver-meta-')
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.bin = self.root / 'bin'
        self.bin.mkdir()
        # macOS shebang cannot contain the host runtime's spaces.
        (self.bin / 'python').symlink_to(PY)
        for name in ('agy', 'hermes'):
            p = self.bin / name
            p.write_text('#!' + str(self.bin / 'python') + '\n' + FIXTURE)
            p.chmod(0o700)
        self.config = self.root / 'fixture.json'
        self.query = self.root / 'query-info.json'
        self.env = {'PATH': str(self.bin), 'DRIVER_FIXTURE_CONFIG': str(self.config),
                    'DRIVER_FIXTURE_QUERY': str(self.query)}
        self.env_patch = patch.dict(os.environ, self.env)
        self.env_patch.start()
        self.addCleanup(self.env_patch.stop)

    def configure(self, **data):
        self.config.write_text(json.dumps(data))

    # 数据路径用例（1 MiB 输出上限等）不应该和 2 秒墙钟赛跑：把 1 MiB 经管道
    # 搬到 driver，再叠加夹具解释器启动，在负载高时可能超过 2 秒，于是
    # 'timeout' 抢在 'output_limit' 前面——那是测量装置的抖动，不是被测行为。
    # 断言真正关于超时的用例仍显式传 timeout=3，语义不变。
    def run_driver(self, name='agy', timeout=30, **data):
        self.configure(**data)
        return driver.run_agent(PRIVATE, self.root, driver=name, timeout=timeout)

    def assert_error(self, result, error=None):
        self.assertEqual(result.status, 'ERROR')
        self.assertEqual(result.response, '')
        self.assertEqual(result.usage, {})
        self.assertTrue(result.error)
        if error:
            self.assertEqual(result.error, error)
        self.assertNotIn(PRIVATE, result.to_json())

    def cli_command(self, name='agy', timeout=30):
        return [PY, str(DRIVER), '--driver', name, '--cwd', str(self.root),
                '--prompt', PRIVATE, '--timeout', str(timeout)]

    def test_agy_nonzero_ok_is_error(self):
        result = self.run_driver(stdout=agy_payload(response=PRIVATE), stderr=PRIVATE, rc=9)
        self.assert_error(result)
        self.assertEqual(result.returncode, 9)

    def test_agy_success_redacts_command_and_discards_stderr(self):
        result = self.run_driver(stdout=agy_payload(), stderr=PRIVATE)
        self.assertEqual(result.status, 'OK')
        self.assertEqual(result.response, 'verified answer')
        self.assertEqual(result.returncode, 0)
        self.assertEqual(result.command, ['agy', '<redacted>'])
        self.assertNotIn(PRIVATE, result.to_json())

    def test_agy_empty_malformed_and_schema(self):
        cases = ['', PRIVATE, '[]', 'null', '1', 'true', '{}',
                 '{"status":"OK","response":12}', agy_payload(response=' '),
                 agy_payload(status='ok'), agy_payload(error=PRIVATE),
                 agy_payload(conversation_id=[]),
                 '{"status":"ERROR","error":"' + PRIVATE + '"}',
                 '{"status":"OK","status":"OK","response":"a"}',
                 agy_payload().replace('1}', 'NaN}'),
                 agy_payload().replace('1}', '1e999}'),
                 '[' * 2000 + ']' * 2000, '{"number":' + '1' * 5000 + '}']
        for text in cases:
            with self.subTest(text=text[:30]):
                self.assert_error(self.run_driver(stdout=text, stderr=PRIVATE))

    def test_usage_requires_nonnegative_integer(self):
        for name in ('agy', 'hermes'):
            for bad in (-1, True, False, 1.0, '2', None, [], {'nested': 1}):
                with self.subTest(name=name, bad=bad):
                    output = agy_payload(usage={'input': bad}) if name == 'agy' else hermes_result(tokens={'input': bad})
                    self.assert_error(self.run_driver(name, stdout=output))
            for bad in (None, [], 'tokens'):
                with self.subTest(name=name, bad=bad):
                    output = agy_payload(usage=bad) if name == 'agy' else hermes_result(tokens=bad)
                    self.assert_error(self.run_driver(name, stdout=output))

    def test_hermes_requires_one_verified_terminal_result(self):
        cases = ['', ' \n', PRIVATE, '[]\n', '{}\n', 'null\n',
                 '{bad\n' + hermes_result(),
                 json.dumps({'type': 'text', 'text': 'delta'}) + '\n',
                 json.dumps({'text': 'untyped answer'}) + '\n',
                 hermes_result(text=''), hermes_result(text=12),
                 hermes_result(exit_code=True), hermes_result(exit_code=2),
                 hermes_result(error=PRIVATE), hermes_result(session_id=[]),
                 hermes_result(duration_ms=1.0),
                 hermes_result() + hermes_result(),
                 hermes_result() + '{"type":"text","text":"after final"}\n',
                 hermes_result(type='unknown')]
        for text in cases:
            with self.subTest(text=text[:40]):
                self.assert_error(self.run_driver('hermes', stdout=text, stderr=PRIVATE))
        self.assert_error(self.run_driver('hermes', stdout=hermes_result(), rc=3))

    def test_hermes_success_protocol_and_private_file(self):
        old = self.root / '.agentisland-pipeline-query.txt'
        old.write_text('preserve original')
        out = '\n'.join([json.dumps({'type': 'system', 'subtype': 'init', 'model': 'fixture', 'session_id': 's'}),
                         json.dumps({'type': 'text', 'text': 'delta'}),
                         json.dumps({'type': 'tool_use', 'name': 'fixture', 'input': {}}),
                         json.dumps({'type': 'tool_result', 'name': 'fixture', 'output': 'done', 'duration_ms': 0, 'is_error': False}),
                         hermes_result()])
        result = self.run_driver('hermes', mode='query', prompt=PRIVATE, stdout=out)
        self.assertEqual(result.status, 'OK')
        self.assertEqual(result.response, 'verified answer')
        self.assertEqual(result.usage, {'input': 2, 'output': 3})
        info = json.loads(self.query.read_text())
        self.assertEqual(info['mode'], 0o600)
        self.assertTrue(info['prompt_matches'])
        self.assertFalse(Path(info['path']).exists())
        self.assertEqual(old.read_text(), 'preserve original')
        self.assertEqual(result.command, ['hermes', '<redacted>'])

    def test_hermes_file_cleanup_failure_paths(self):
        for data in ({'stdout': 'bad'}, {'stdout': hermes_result(), 'rc': 7}, {'wait': True}):
            with self.subTest(data=list(data)):
                self.assert_error(self.run_driver('hermes', timeout=3, mode='query', prompt=PRIVATE, **data))
                self.assertFalse(Path(json.loads(self.query.read_text())['path']).exists())
        with patch.object(driver, '_run', side_effect=OSError(PRIVATE)):
            self.assert_error(driver.run_agent(PRIVATE, self.root, driver='hermes'))
        self.assertEqual(list(self.root.glob('.agentisland-query-*')), [])

    def test_missing_executable_and_invalid_cwd_have_no_prompt_artifacts(self):
        for name in ('agy', 'hermes'):
            with patch.dict(os.environ, {'PATH': str(self.root / 'missing')}):
                self.assert_error(driver.run_agent(PRIVATE, self.root, driver=name))
            self.assertEqual(list(self.root.glob('.agentisland-query-*')), [])
            self.assert_error(driver.run_agent(PRIVATE, self.root / 'missing', driver=name))

    def test_output_cap_includes_stderr(self):
        self.assertLessEqual(driver.MAX_OUTPUT_BYTES, 1024 * 1024)
        for name in ('agy', 'hermes'):
            for stderr_only in (False, True):
                with self.subTest(name=name, stderr=stderr_only):
                    result = self.run_driver(name, size=driver.MAX_OUTPUT_BYTES + 1, stderr_only=stderr_only)
                    self.assert_error(result, 'output_limit')
                    self.assertEqual(result.returncode, 125)
        # Combined stdout/stderr size, and exact cap accepted.
        result = self.run_driver(stdout=agy_payload(), stderr='x' * driver.MAX_OUTPUT_BYTES)
        self.assert_error(result, 'output_limit')
        payload = agy_payload()
        result = self.run_driver(stdout=payload + ' ' * (driver.MAX_OUTPUT_BYTES - len(payload.encode())))
        self.assertEqual(result.status, 'OK')

    def wait_file(self, path, proc):
        deadline = time.monotonic() + 5
        while not path.exists() and time.monotonic() < deadline:
            if proc.poll() is not None:
                self.fail('fixture exited before readiness')
            time.sleep(0.01)
        self.assertTrue(path.exists())

    def assert_pid_gone(self, pid):
        deadline = time.monotonic() + 3
        while time.monotonic() < deadline:
            try:
                os.kill(pid, 0)
            except ProcessLookupError:
                return
            ps = subprocess.run(['/bin/ps', '-o', 'stat=', '-p', str(pid)], capture_output=True, text=True)
            if ps.stdout.lstrip().startswith('Z'):
                return
            time.sleep(0.02)
        self.fail('descendant survived driver cleanup')

    def assert_tree_gone(self):
        for name in ('child.pid', 'grandchild.pid'):
            self.assert_pid_gone(int((self.root / name).read_text()))

    def test_timeout_kills_descendants_even_when_parent_exits(self):
        result = self.run_driver(mode='tree', timeout=3)
        self.assert_error(result, 'timeout')
        self.assertEqual(result.returncode, 124)
        self.assert_tree_gone()

    def test_timeout_reclaims_tree_after_leader_exits(self):
        self.assert_error(self.run_driver(mode='tree', timeout=3, exit_parent=True), 'timeout')
        self.assert_tree_gone()

    def test_query_write_failure_cleanup_and_preflight(self):
        self.configure(stdout=hermes_result())
        # Invalid Unicode fails while writing the already-created exclusive file.
        self.assert_error(driver.run_agent('\ud800', self.root, driver='hermes'), 'query_failed')
        self.assertEqual(list(self.root.glob('.agentisland-query-*')), [])
        with patch.object(driver.tempfile, 'mkstemp', side_effect=OSError(PRIVATE)):
            self.assert_error(driver.run_agent(PRIVATE, self.root, driver='hermes'), 'query_failed')
        # A fixed legacy name, even a symlink, must never be opened or removed.
        original = self.root / 'original.txt'
        original.write_text('original')
        legacy = self.root / '.agentisland-pipeline-query.txt'
        legacy.symlink_to(original)
        self.assertEqual(self.run_driver('hermes', stdout=hermes_result()).status, 'OK')
        self.assertEqual(original.read_text(), 'original')
        self.assertTrue(legacy.is_symlink())

    def test_oversize_kills_descendants(self):
        self.assert_error(self.run_driver(mode='tree', size=driver.MAX_OUTPUT_BYTES + 1), 'output_limit')
        self.assert_tree_gone()

    def test_sigint_kills_descendants_and_cleans_query(self):
        self.configure(mode='tree')
        proc = subprocess.Popen(self.cli_command('hermes', 20), stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        self.addCleanup(lambda: proc.kill() if proc.poll() is None else None)
        self.wait_file(self.root / 'grandchild.pid', proc)
        proc.send_signal(signal.SIGINT)
        out, err = proc.communicate(timeout=5)
        self.assertEqual(proc.returncode, 1)
        self.assertEqual(json.loads(out)['error'], 'interrupted')
        self.assertEqual(json.loads(out)['returncode'], 130)
        self.assertNotIn(PRIVATE, out + err)
        self.assert_tree_gone()
        self.assertEqual(list(self.root.glob('.agentisland-query-*')), [])

    def test_cli_json_has_no_body_command_prompt_or_tokens(self):
        for name in ('agy', 'hermes'):
            output = agy_payload(response=PRIVATE) if name == 'agy' else hermes_result(text=PRIVATE)
            self.configure(stdout=output, stderr=PRIVATE)
            proc = subprocess.run(self.cli_command(name), capture_output=True, text=True, timeout=60)
            self.assertEqual(proc.returncode, 0)
            data = json.loads(proc.stdout)
            self.assertEqual(data['status'], 'OK')
            self.assertNotIn(PRIVATE, proc.stdout + proc.stderr)
            for key in ('response', 'command', 'prompt', 'usage', 'tokens', 'conversation_id'):
                self.assertNotIn(key, data)

    def test_mock_scenarios_and_unknown_fail_closed(self):
        for scenario, status in (('ok', 'OK'), ('auth', 'ERROR'), ('timeout', 'ERROR')):
            self.assertEqual(driver.run_agent(PRIVATE, self.root, scenario=scenario).status, status)
        with self.assertRaises(driver.DriverError):
            driver.run_agent(PRIVATE, self.root, scenario=PRIVATE)
        for kwargs in ({'driver': PRIVATE}, {'argv_extra': [PRIVATE]}, {'timeout': True}, {'timeout': 0}):
            with self.assertRaises(driver.DriverError):
                driver.run_agent(PRIVATE, self.root, **kwargs)
        proc = subprocess.run([PY, str(DRIVER), '--driver', 'mock', '--prompt', PRIVATE, '--scenario', PRIVATE],
                              capture_output=True, text=True, timeout=5)
        self.assertEqual(proc.returncode, 2)
        self.assertNotIn(PRIVATE, proc.stdout + proc.stderr)


if __name__ == '__main__':
    unittest.main(verbosity=2)
