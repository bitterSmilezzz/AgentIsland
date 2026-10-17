#!/usr/bin/env python3
"""Fault-inject the real runner transaction without starting a VM or native UI."""
import contextlib
import io
import importlib.util
import json
import signal
import os
import subprocess
import sys
from pathlib import Path
import plistlib
import tempfile
import unittest
from unittest.mock import patch, Mock

spec = importlib.util.spec_from_file_location('runner', Path(__file__).with_name('resource-soak.py'))
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)


class Tests(unittest.TestCase):
    def test_baseline_rejects_change_missing_and_link(self):
        with tempfile.TemporaryDirectory() as root:
            home = Path(root) / 'home'
            baseline = runner.seed(home)
            self.assertTrue(runner.baseline_matches(home, baseline))
            path = home / '.codex/config.toml'
            original = path.read_bytes()
            path.write_text('different')
            self.assertFalse(runner.baseline_matches(home, baseline))
            path.unlink()
            self.assertFalse(runner.baseline_matches(home, baseline))
            other = Path(root) / 'original'
            other.write_bytes(original)
            path.symlink_to(other)
            self.assertFalse(runner.baseline_matches(home, baseline))

    def test_pid_replacement_and_other_binary_never_signaled(self):
        native = Mock()
        native.is_absent.return_value = False
        native.usage.return_value = {'start': 123}
        native.processes.return_value = [(10, '/fixture/agentisland')]
        with patch.object(runner.os, 'kill') as kill:
            for binary, start in [(Path('/fixture/agentisland'), 124), (Path('/other/agentisland'), 123)]:
                with self.assertRaises(ValueError): runner.stop_owned(native, 10, binary, start)
            kill.assert_not_called()

    def test_completion_does_not_mask_failed_cleanup(self):
        cleanup = dict.fromkeys(runner.analyzer.CLEANUP_FIELDS, True)
        self.assertEqual(runner.finish_code(True, cleanup), 0)
        self.assertEqual(runner.finish_code(False, cleanup), 2)
        for field in cleanup:
            self.assertEqual(runner.finish_code(True, dict(cleanup, **{field: False})), 2)

    def transaction(self, mutate=False, fail_launch=False, *, via_main=False,
                    ps_timeout=None, cancel=None, cleanup_signal=None):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            bundle = root / 'source/AgentIsland.app'
            binary = bundle / 'Contents/MacOS/agentisland'
            binary.parent.mkdir(parents=True)
            binary.write_bytes(b'formal-fixture')
            (bundle / 'Contents/Info.plist').write_bytes(plistlib.dumps({'CFBundleShortVersionString': '0.0.0'}))
            output = root / 'run'
            target = output / 'AgentIsland.app/Contents/MacOS/agentisland'
            state = {'alive': False}
            native = Mock()
            native.processes.side_effect = lambda: [(10, str(target))] if state['alive'] else []
            native.usage.return_value = {'start': 123}
            native.is_absent.side_effect = lambda pid: not state['alive']
            awake = Mock()
            awake.poll.return_value = None
            awake.pid = 20
            awake.terminate.side_effect = lambda: setattr(awake.poll, 'return_value', 0)

            def command(args, **kwargs):
                if args[0] == 'ditto':
                    import shutil
                    shutil.copytree(args[1], args[2])
                if args[0] == 'open':
                    state['alive'] = True
                    if fail_launch: raise OSError('failure after launch')
                return Mock(returncode=0)

            def check_output(args, **kwargs):
                if args[0] == 'sysctl':
                    return '1'
                if ps_timeout is not None and ps_timeout in args:
                    timeout = kwargs.get('timeout')
                    self.assertIsInstance(timeout, (int, float))
                    self.assertGreater(timeout, 0)
                    self.assertLessEqual(timeout, 5)
                    raise subprocess.TimeoutExpired(args, timeout,
                        output='private-environment-must-not-appear',
                        stderr='private-environment-must-not-appear')
                return '--shell=workbench HOME=' + str(output / 'home')

            def stop_target(pid, signum):
                self.assertEqual((pid, signum), (10, signal.SIGTERM))
                if cleanup_signal is not None:
                    # Deliver to this test process; the owned target remains synthetic.
                    signal.raise_signal(cleanup_signal)
                state['alive'] = False

            def execute():
                if cancel is not None:
                    signal.raise_signal(cancel)
                    self.fail('Cancellation did not interrupt the protocol')
                emit = protocol.call_args.args[4]
                for _ in range(47): emit({'phase': 'idle'})
                emit({'phase': 'complete', 'protocol_complete': True})
                if mutate:
                    (output / 'home/.codex/config.toml').write_text('external mutation')

            with patch.object(runner.sys, 'platform', 'darwin'), \
                    patch.object(runner.Path, 'home', return_value=root), \
                    patch.object(runner, 'console_ready', return_value=True), \
                    patch.object(runner.subprocess, 'run', side_effect=command), \
                    patch.object(runner.subprocess, 'check_output', side_effect=check_output), \
                    patch.object(runner.subprocess, 'Popen', return_value=awake), \
                    patch.object(runner.soak.measure, 'Native', return_value=native), \
                    patch.object(runner.os, 'kill', side_effect=stop_target), \
                    patch.object(runner.soak, 'wait_until'), \
                    patch.object(runner.soak, 'Protocol') as protocol, \
                    patch.object(runner.analyzer, 'analyze', return_value={'protocol_complete': True}):
                protocol.return_value.execute.side_effect = execute
                with contextlib.redirect_stdout(io.StringIO()) as stdout, \
                        contextlib.redirect_stderr(io.StringIO()) as stderr:
                    if via_main:
                        signals = (signal.SIGINT, signal.SIGTERM, signal.SIGHUP)
                        previous = {sig: signal.getsignal(sig) for sig in signals}
                        try:
                            with patch.object(runner.sys, 'argv', ['resource-soak.py',
                                    '--bundle', str(bundle), '--output', str(output)]):
                                code = runner.main()
                            for sig, handler in previous.items():
                                self.assertEqual(signal.getsignal(sig), handler)
                        finally:
                            for sig, handler in previous.items():
                                signal.signal(sig, handler)
                    else:
                        code = runner.run(bundle, output)
            self.assertNotIn('private-environment', stdout.getvalue() + stderr.getvalue())
            receipt_text = (output / 'receipt.json').read_text()
            self.assertNotIn('private-environment', receipt_text)
            receipt = json.loads(receipt_text)
            self.assertFalse(state['alive'])
            self.assertFalse((root / '.agentisland-native-gates.lock').exists())
            self.assertTrue(receipt['cleanup']['awake_stopped'])
            self.assertEqual(receipt['exit_code'], code)
            return code, receipt

    def test_main_initialization_failure_does_not_leave_an_unowned_lock(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            bundle = root / 'AgentIsland.app'
            bundle.mkdir()
            native = Mock()
            native.processes.return_value = []
            signals = (signal.SIGINT, signal.SIGTERM, signal.SIGHUP)
            previous = {sig: signal.getsignal(sig) for sig in signals}
            try:
                with patch.object(runner.sys, 'platform', 'darwin'), \
                        patch.object(runner.Path, 'home', return_value=root), \
                        patch.object(runner, 'console_ready', return_value=True), \
                        patch.object(runner.subprocess, 'check_output', return_value='1'), \
                        patch.object(runner.subprocess, 'run'), \
                        patch.object(runner.soak.measure, 'Native', return_value=native), \
                        patch.object(runner.uuid, 'uuid4', side_effect=OSError('fixture-init-failure')), \
                        patch.object(runner.sys, 'argv', ['resource-soak.py', '--bundle', str(bundle),
                                                         '--output', str(root / 'output')]):
                    with self.assertRaises(OSError):
                        runner.main()
                self.assertFalse((root / '.agentisland-native-gates.lock').exists())
            finally:
                for sig, handler in previous.items():
                    signal.signal(sig, handler)

    def test_main_ps_timeouts_are_bounded_and_cleaned(self):
        for column in ('args=', 'command='):
            with self.subTest(column=column):
                code, receipt = self.transaction(via_main=True, ps_timeout=column)
                self.assertEqual(code, 2)
                self.assertEqual(receipt['error_type'], 'TimeoutExpired')
                self.assertEqual(receipt['stage'], 'launch')
                self.assertFalse(receipt['protocol_complete'])
                self.assertTrue(all(receipt['cleanup'].values()))

    def signal_child(self):
        # Missing SIGHUP handlers or repeated KeyboardInterrupt must not kill unittest.
        if os.environ.get('RESOURCE_SIGNAL_CHILD') == '1':
            return False
        child = subprocess.run([sys.executable, '-B', str(Path(__file__).resolve()),
                                self.id().split('.', 1)[-1]],
                               env={'PATH': os.defpath, 'RESOURCE_SIGNAL_CHILD': '1'},
                               capture_output=True, text=True, timeout=10)
        self.assertEqual(child.returncode, 0, child.stderr)
        return True

    def test_main_second_sigterm_during_cleanup_keeps_receipt_and_lock_cleanup(self):
        if self.signal_child():
            return
        code, receipt = self.transaction(via_main=True, cancel=signal.SIGTERM,
                                         cleanup_signal=signal.SIGTERM)
        self.assertEqual(code, 2)
        self.assertEqual(receipt['error_type'], 'KeyboardInterrupt')
        self.assertFalse(receipt['protocol_complete'])
        self.assertTrue(all(receipt['cleanup'].values()))

    def test_main_sighup_cancels_with_complete_cleanup(self):
        if self.signal_child():
            return
        code, receipt = self.transaction(via_main=True, cancel=signal.SIGHUP)
        self.assertEqual(code, 2)
        self.assertEqual(receipt['error_type'], 'KeyboardInterrupt')
        self.assertFalse(receipt['protocol_complete'])
        self.assertTrue(all(receipt['cleanup'].values()))

    def test_main_sighup_during_cleanup_keeps_receipt_and_lock_cleanup(self):
        if self.signal_child():
            return
        code, receipt = self.transaction(via_main=True, cancel=signal.SIGTERM,
                                         cleanup_signal=signal.SIGHUP)
        self.assertEqual(code, 2)
        self.assertEqual(receipt['error_type'], 'KeyboardInterrupt')
        self.assertTrue(all(receipt['cleanup'].values()))

    def test_main_existing_lease_and_handler_state_are_preserved(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            bundle = root / 'AgentIsland.app'
            bundle.mkdir()
            lock = root / '.agentisland-native-gates.lock'
            lock.mkdir()
            owner = lock / 'owner.json'
            owner.write_text('{"pid": 99, "run_id": "other-fixture"}')
            original = owner.read_bytes()
            native = Mock()
            native.processes.return_value = []
            signals = (signal.SIGINT, signal.SIGTERM, signal.SIGHUP)
            previous = {sig: signal.getsignal(sig) for sig in signals}
            try:
                with patch.object(runner.sys, 'platform', 'darwin'), \
                        patch.object(runner.Path, 'home', return_value=root), \
                        patch.object(runner, 'console_ready', return_value=True), \
                        patch.object(runner.subprocess, 'check_output', return_value='1'), \
                        patch.object(runner.subprocess, 'run'), \
                        patch.object(runner.soak.measure, 'Native', return_value=native), \
                        patch.object(runner.os, 'kill') as kill, \
                        patch.object(runner.sys, 'argv', ['resource-soak.py', '--bundle', str(bundle),
                                                         '--output', str(root / 'output')]):
                    with self.assertRaises(runner.Preflight) as caught:
                        runner.main()
                    self.assertEqual(caught.exception.code, 'LEASE_EXISTS')
                    kill.assert_not_called()
                self.assertEqual(owner.read_bytes(), original)
                for sig, handler in previous.items():
                    self.assertEqual(signal.getsignal(sig), handler)
            finally:
                for sig, handler in previous.items():
                    signal.signal(sig, handler)

    def test_main_sigint_during_cleanup_keeps_receipt_and_lock_cleanup(self):
        if self.signal_child():
            return
        code, receipt = self.transaction(via_main=True, cancel=signal.SIGTERM,
                                         cleanup_signal=signal.SIGINT)
        self.assertEqual(code, 2)
        self.assertEqual(receipt['error_type'], 'KeyboardInterrupt')
        self.assertTrue(all(receipt['cleanup'].values()))

    def test_real_runner_baseline_mutation_fails_after_protocol_completes(self):
        code, receipt = self.transaction(mutate=True)
        self.assertEqual(code, 2)
        self.assertTrue(receipt['protocol_complete'])
        self.assertFalse(receipt['cleanup']['baseline_unchanged'])

    def test_real_runner_partial_launch_is_cleaned(self):
        code, receipt = self.transaction(fail_launch=True)
        self.assertEqual(code, 2)
        self.assertFalse(receipt['protocol_complete'])
        self.assertTrue(all(receipt['cleanup'].values()))

    def test_preflight_diagnostics_are_fixed_and_do_not_echo_exceptions(self):
        import contextlib, io
        failures = [runner.Preflight('EXISTING_APPLICATION', existing_count=2),
                    runner.Preflight('LEASE_EXISTS'),
                    ValueError('private-path-and-environment-must-not-appear'),
                    OSError('private-path-and-environment-must-not-appear')]
        for error in failures:
            with self.subTest(error=type(error).__name__), \
                    patch.object(runner, 'main', side_effect=error), \
                    contextlib.redirect_stderr(io.StringIO()) as output:
                self.assertEqual(runner.cli(), 2)
            record = json.loads(output.getvalue())
            self.assertFalse(record['protocol_complete'])
            self.assertFalse(record['budget_assessed'])
            self.assertNotIn('private-path', output.getvalue())
            self.assertEqual(record['error_code'], error.code if isinstance(error, runner.Preflight)
                             else 'PREFLIGHT_FAILED')

    def test_console_probe_failure_has_specific_code_without_exception_body(self):
        with patch.object(runner.sys, 'platform', 'darwin'), \
                patch.object(runner.subprocess, 'check_output', return_value='1'), \
                patch.object(runner, 'console_ready', side_effect=OSError('private-content')):
            with self.assertRaises(runner.Preflight) as caught:
                runner.run(Path('/unused'), Path('/unused'))
        self.assertEqual(caught.exception.code, 'VM_PROBE_FAILED')
        self.assertNotIn('private-content', str(caught.exception))

    def test_non_dictionary_console_probe_is_a_safe_probe_failure(self):
        import contextlib, io
        def probe(args, **kwargs):
            return '1' if args[0] == 'sysctl' else plistlib.dumps([])
        with patch.object(runner.sys, 'platform', 'darwin'), \
                patch.object(runner.subprocess, 'check_output', side_effect=probe), \
                patch.object(runner.sys, 'argv', ['resource-soak.py', '--bundle', '/unused', '--output', '/unused']), \
                patch.object(runner.signal, 'signal'), \
                contextlib.redirect_stderr(io.StringIO()) as output:
            self.assertEqual(runner.cli(), 2)
        self.assertEqual(json.loads(output.getvalue())['error_code'], 'VM_PROBE_FAILED')
        self.assertNotIn('Traceback', output.getvalue())

    def test_existing_application_is_rejected_before_evidence_or_lock_creation(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            bundle = root / 'AgentIsland.app'
            bundle.mkdir()
            native = Mock()
            native.processes.return_value = [(10, '/private/AgentIsland.app/Contents/MacOS/agentisland')]
            with patch.object(runner.sys, 'platform', 'darwin'), \
                    patch.object(runner, 'console_ready', return_value=True), \
                    patch.object(runner.subprocess, 'check_output', return_value='1'), \
                    patch.object(runner.subprocess, 'run'), \
                    patch.object(runner.soak.measure, 'Native', return_value=native):
                with self.assertRaises(runner.Preflight) as caught:
                    runner.run(bundle, root / 'output')
            self.assertEqual(caught.exception.code, 'EXISTING_APPLICATION')
            self.assertEqual(caught.exception.diagnostic, {'existing_count': 1})
            self.assertFalse((root / 'output').exists())

    def test_retire_verified_refuses_other_bundle_or_changed_identity(self):
        with tempfile.TemporaryDirectory() as directory:
            binary = Path(directory).resolve() / 'AgentIsland.app/Contents/MacOS/agentisland'
            binary.parent.mkdir(parents=True)
            binary.write_bytes(b'formal')
            native = Mock()
            native.processes.return_value = [(10, str(binary)),
                (11, '/other/AgentIsland.app/Contents/MacOS/agentisland')]
            with patch.object(runner.os, 'kill') as kill:
                with self.assertRaises(ValueError): runner.retire_verified(native, binary, runner.digest(binary))
                native.processes.return_value = [(10, str(binary))]
                with self.assertRaises(ValueError): runner.retire_verified(native, binary, 'changed')
                native.usage.side_effect = [{'start': 123}, {'start': 124}]
                native.is_absent.return_value = False
                with self.assertRaises(ValueError): runner.retire_verified(native, binary, runner.digest(binary))
                kill.assert_not_called()

    def test_retire_verified_only_stops_exact_prior_bundle(self):
        with tempfile.TemporaryDirectory() as directory:
            binary = Path(directory).resolve() / 'AgentIsland.app/Contents/MacOS/agentisland'
            binary.parent.mkdir(parents=True)
            binary.write_bytes(b'formal')
            native = Mock()
            native.processes.side_effect = [[(10, str(binary))], [(10, str(binary))], []]
            native.usage.return_value = {'start': 123}
            native.is_absent.side_effect = [False, True]
            with patch.object(runner.os, 'kill') as kill:
                self.assertEqual(runner.retire_verified(native, binary, runner.digest(binary)), 1)
                kill.assert_called_once_with(10, runner.signal.SIGTERM)

    def test_real_runner_success(self):
        code, receipt = self.transaction()
        self.assertEqual(code, 0)
        self.assertTrue(all(receipt['cleanup'].values()))


if __name__ == '__main__': unittest.main()
