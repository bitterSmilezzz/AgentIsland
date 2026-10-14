#!/usr/bin/env python3
"""Fault-inject the real runner transaction without starting a VM or native UI."""
import importlib.util
import json
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

    def transaction(self, mutate=False, fail_launch=False):
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

            def execute():
                emit = protocol.call_args.args[4]
                for _ in range(47): emit({'phase': 'idle'})
                emit({'phase': 'complete', 'protocol_complete': True})
                if mutate:
                    (output / 'home/.codex/config.toml').write_text('external mutation')

            with patch.object(runner.sys, 'platform', 'darwin'), \
                    patch.object(runner.Path, 'home', return_value=root), \
                    patch.object(runner, 'console_ready', return_value=True), \
                    patch.object(runner.subprocess, 'run', side_effect=command), \
                    patch.object(runner.subprocess, 'check_output', side_effect=lambda args, **kw:
                        '1' if args[0]=='sysctl' else '--shell=workbench HOME=' + str(output / 'home')), \
                    patch.object(runner.subprocess, 'Popen', return_value=awake), \
                    patch.object(runner.soak.measure, 'Native', return_value=native), \
                    patch.object(runner.os, 'kill', side_effect=lambda *args: state.update(alive=False)), \
                    patch.object(runner.soak, 'wait_until'), \
                    patch.object(runner.soak, 'Protocol') as protocol, \
                    patch.object(runner.analyzer, 'analyze', return_value={'protocol_complete': True}):
                protocol.return_value.execute.side_effect = execute
                code = runner.run(bundle, output)
            receipt = json.loads((output / 'receipt.json').read_text())
            self.assertFalse(state['alive'])
            self.assertFalse((root / '.agentisland-native-gates.lock').exists())
            self.assertTrue(receipt['cleanup']['awake_stopped'])
            self.assertEqual(receipt['exit_code'], code)
            return code, receipt

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
