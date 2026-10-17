#!/usr/bin/env python3
"""Regression tests for isolation gates; never start a VM or native window."""
import importlib.util
import subprocess
import unittest
from pathlib import Path
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('native_vm', Path(__file__).with_name('native-gates-vm.py'))
vm = importlib.util.module_from_spec(spec)
spec.loader.exec_module(vm)


class IsolationTests(unittest.TestCase):
    def test_requires_exact_running_local_vm(self):
        row = dict(Name='test-vm', Source='local', Running=True, State='running')
        self.assertTrue(vm.vm_running([row], 'test-vm'))
        for changes in [dict(Name='other'), dict(Source='remote'), dict(Running=False),
                        dict(Running='true'), dict(State='stopped')]:
            self.assertFalse(vm.vm_running([dict(row, **changes)], 'test-vm'))
        self.assertFalse(vm.vm_running([], 'test-vm'))

    def test_rejects_host_public_and_unspecified_addresses(self):
        for address in ['127.0.0.1', '::1', '0.0.0.0', '::', '8.8.8.8', 'invalid']:
            with self.subTest(address=address), self.assertRaises(ValueError):
                vm.private_address(address)
        self.assertEqual(vm.private_address('192.168.64.3'), '192.168.64.3')

    def test_virtualized_logged_in_aqua_required(self):
        meta = dict(platform='Darwin', virtualized='1', console_uid=501, uid=501, python_ready=True, console_unlocked=True)
        self.assertTrue(vm.guest_verified(meta))
        for change in [dict(platform='Linux'), dict(virtualized='0'), dict(console_uid=0),
                       dict(console_uid='501'), dict(uid=502), dict(python_ready=False), dict(console_unlocked=False), dict(console_unlocked=None)]:
            self.assertFalse(vm.guest_verified(dict(meta, **change)))
        self.assertFalse(vm.guest_verified({}))

    @patch.object(vm.subprocess, 'run')
    def test_arp_fallback_after_missing_dhcp(self, run):
        run.side_effect = [subprocess.CompletedProcess([], 1, stdout=''),
                           subprocess.CompletedProcess([], 0, stdout='192.168.64.3\n')]
        self.assertEqual(vm.resolve_address('test-vm'), '192.168.64.3')
        self.assertEqual([call.args[0][4] for call in run.call_args_list], ['dhcp', 'arp'])

    @patch.object(vm.subprocess, 'run')
    def test_no_address_fails_closed(self, run):
        run.return_value = subprocess.CompletedProcess([], 1, stdout='')
        with self.assertRaisesRegex(ValueError, 'no address'):
            vm.resolve_address('test-vm')

    @patch.object(vm.subprocess, 'run')
    def test_resolver_cannot_target_host(self, run):
        run.return_value = subprocess.CompletedProcess([], 0, stdout='127.0.0.1')
        with self.assertRaises(ValueError):
            vm.resolve_address('test-vm')
        self.assertEqual(run.call_count, 1)


class PriorAcceptanceTests(unittest.TestCase):
    def test_only_verified_five_gate_receipt_for_same_vm_authorizes_cleanup(self):
        import json, tempfile
        name = 'acceptance-' + 'a' * 32
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            path = base / name
            path.mkdir()
            manifest = {'dist/AgentIsland.app/Contents/MacOS/agentisland': 'b' * 64}
            receipt = dict(passed=True, exit_code=0, vm='fixture-vm', guest_verified=True,
                           console_unlocked=True, manifest=manifest,
                           gates=['ui', 'dock', 'single-instance', 'cli-native', 'window-lifetime'])
            (path / 'manifest.json').write_text(json.dumps(manifest))
            (path / 'receipt.json').write_text(json.dumps(receipt))
            self.assertEqual(vm.verified_prior_acceptance(name, 'fixture-vm', base), 'b' * 64)
            for change in [dict(passed=False), dict(exit_code=True), dict(vm='other'),
                           dict(console_unlocked=False), dict(gates=['resource-soak']), dict(manifest={})]:
                (path / 'receipt.json').write_text(json.dumps(dict(receipt, **change)))
                with self.subTest(change=change), self.assertRaises(ValueError):
                    vm.verified_prior_acceptance(name, 'fixture-vm', base)
            with self.assertRaises(ValueError): vm.verified_prior_acceptance('../escape', 'fixture-vm', base)


class ResourceGateTests(unittest.TestCase):
    def evidence(self):
        import hashlib, json
        result_spec = importlib.util.spec_from_file_location('fixtures', Path(__file__).with_name('test-memory-soak-result.py'))
        fixtures = importlib.util.module_from_spec(result_spec)
        result_spec.loader.exec_module(fixtures)
        rows, receipt = fixtures.fixture()
        receipt.update(run_id='run-1', cleanup=dict.fromkeys(fixtures.result.CLEANUP_FIELDS, True))
        analysis = fixtures.result.analyze(rows, receipt, require_cleanup=True)
        manifest = {'scripts/' + name: 'digest' for name in ('resource-soak.py', 'memory-soak.py', 'measure-memory.py', 'analyze-memory-soak.py')}
        manifest['dist/AgentIsland.app/Contents/MacOS/agentisland'] = 'binary'
        values = dict.fromkeys(('baseline.json', 'running.json', 'environment.jsonl'), '{}')
        values['target.json'] = json.dumps(dict(pid=10, start=123))
        values.update({'manifest.json': json.dumps({'files': {name.removeprefix('scripts/'): value for name, value in manifest.items() if name.startswith('scripts/')}, 'binary_sha256': 'binary', 'run_id': 'run-1'}),
                       'samples.jsonl': '\n'.join(json.dumps(row) for row in rows),
                       'receipt.json': json.dumps(receipt), 'analysis.json': json.dumps(analysis)})
        data = {'files': {name: {'body': value, 'sha256': hashlib.sha256(value.encode()).hexdigest()} for name, value in values.items()},
                'postcheck': dict.fromkeys(('runner_absent','awake_absent','target_absent','lock_absent','baseline_unchanged'), True)}
        return data, manifest

    def check(self, data, manifest, exit_code=0):
        import json, tempfile
        with tempfile.TemporaryDirectory(dir=vm.SCRATCH) as directory:
            root = Path(directory)
            with patch.object(vm.subprocess, 'run', return_value=subprocess.CompletedProcess([], exit_code)), \
                    patch.object(vm, 'run', return_value=subprocess.CompletedProcess([], 0, stdout=json.dumps(data))):
                vm.run_resource(['ssh'], 'python', 'guest', root, manifest)
            return json.loads((root / 'receipt.json').read_text())

    def test_scratch_directory_exists_on_a_clean_checkout(self):
        # `.build/` is git-ignored, so it is absent from a fresh checkout. The gate
        # stages temporary payloads there, so it must create the directory rather than
        # let TemporaryDirectory fail with FileNotFoundError -- which on a clean tree
        # reported 18 errors before a single assertion ran.
        import shutil
        import sys
        shutil.rmtree(vm.SCRATCH, ignore_errors=True)
        self.assertFalse(vm.SCRATCH.exists())
        # Other tests in this module stage their payloads in the same shared
        # directory, so it must be restored whatever this assertion concludes.
        self.addCleanup(vm.SCRATCH.mkdir, parents=True, exist_ok=True)
        # Re-importing must be enough to make the gate usable again.
        spec = importlib.util.spec_from_file_location('native_gates_vm_reload', vm.__file__)
        reloaded = importlib.util.module_from_spec(spec)
        sys.modules[spec.name] = reloaded
        try:
            spec.loader.exec_module(reloaded)
        finally:
            sys.modules.pop(spec.name, None)
        self.assertTrue(reloaded.SCRATCH.is_dir())
        self.assertEqual(reloaded.SCRATCH, vm.SCRATCH)

    def test_preflight_hint_ignores_arbitrary_remote_details(self):
        import json, tempfile
        with tempfile.TemporaryDirectory() as directory:
            log = Path(directory) / 'gates.log'
            record = dict(stage='preflight', error_code='EXISTING_APPLICATION',
                          protocol_complete=False, budget_assessed=False,
                          exception='private-environment-must-not-appear')
            log.write_text('private-environment-must-not-appear\n' + json.dumps(record))
            hint = vm.resource_failure_hint(log)
            self.assertIn('[EXISTING_APPLICATION]', hint)
            self.assertNotIn('private-environment', hint)
            for invalid in ('private-environment-must-not-appear', [], {}):
                record['error_code'] = invalid
                log.write_text(json.dumps(record))
                self.assertEqual(vm.resource_failure_hint(log), 'Resource gate failed; evidence retained')

    def test_capture_failure_preserves_preflight_diagnostic(self):
        import json, tempfile
        with tempfile.TemporaryDirectory(dir=vm.SCRATCH) as directory:
            root = Path(directory)
            def failed_run(*args, **kwargs):
                kwargs['stdout'].write(json.dumps(dict(stage='preflight', error_code='LEASE_EXISTS',
                    protocol_complete=False, budget_assessed=False)))
                return subprocess.CompletedProcess([], 2)
            with patch.object(vm.subprocess, 'run', side_effect=failed_run), \
                    patch.object(vm, 'run', side_effect=subprocess.CalledProcessError(255, ['ssh'])):
                with self.assertRaisesRegex(ValueError, 'LEASE_EXISTS.*EVIDENCE_CAPTURE_FAILED'):
                    vm.run_resource(['ssh'], 'python', 'guest', root, {})
            self.assertFalse((root / 'receipt.json').exists())
            self.assertFalse(json.loads((root / 'resource-capture-failure.json').read_text())['cleanup_verified'])

    def test_malformed_capture_preserves_preflight_diagnostic(self):
        import json, tempfile
        malformed = [{}, [], {'files': [], 'postcheck': {}},
                     {'files': {}, 'postcheck': []},
                     {'files': {'receipt.json': {'body': [], 'sha256': 'bad'}}, 'postcheck': {}},
                     {'files': {'../escape': {'body': 'x', 'sha256': 'bad'}}, 'postcheck': {}}]
        for payload in malformed:
            with self.subTest(payload=payload), tempfile.TemporaryDirectory(dir=vm.SCRATCH) as directory:
                root = Path(directory)
                def failed_run(*args, **kwargs):
                    kwargs['stdout'].write(json.dumps(dict(stage='preflight', error_code='LEASE_EXISTS',
                        protocol_complete=False, budget_assessed=False)))
                    return subprocess.CompletedProcess([], 2)
                with patch.object(vm.subprocess, 'run', side_effect=failed_run), \
                        patch.object(vm, 'run', return_value=subprocess.CompletedProcess([], 0, stdout=json.dumps(payload))):
                    with self.assertRaisesRegex(ValueError, 'LEASE_EXISTS.*EVIDENCE_CAPTURE_FAILED'):
                        vm.run_resource(['ssh'], 'python', 'guest', root, {})
                self.assertFalse((root / 'receipt.json').exists())
                self.assertTrue((root / 'resource-capture-failure.json').is_file())

    def test_verified_protocol_and_cleanup_pass(self):
        data, manifest = self.evidence()
        self.assertTrue(self.check(data, manifest)['passed'])

    def test_timeout_retains_failure_and_attempts_evidence_capture(self):
        import json, tempfile
        data, manifest = self.evidence()
        with tempfile.TemporaryDirectory(dir=vm.SCRATCH) as directory:
            root = Path(directory)
            with patch.object(vm.subprocess, 'run', side_effect=subprocess.TimeoutExpired('ssh', 6600)), \
                    patch.object(vm, 'run', return_value=subprocess.CompletedProcess([], 0, stdout=json.dumps(data))) as capture:
                with self.assertRaises(ValueError):
                    vm.run_resource(['ssh'], 'python', 'guest', root, manifest)
            self.assertEqual(capture.call_count, 1)
            self.assertTrue((root / 'resource-receipt.json').is_file())
            failure = json.loads((root / 'resource-transport-failure.json').read_text())
            self.assertFalse(failure['passed'])
            self.assertFalse(failure['cleanup_verified'])
            self.assertFalse((root / 'receipt.json').exists())

    def test_sample_identity_must_match_recorded_target(self):
        import json, hashlib
        data, manifest = self.evidence()
        for target in (dict(pid=999999, start=987654321), dict(pid=10, start=124), {}):
            body = json.dumps(target)
            data['files']['target.json'] = dict(body=body, sha256=hashlib.sha256(body.encode()).hexdigest())
            with self.subTest(target=target), self.assertRaises(ValueError):
                self.check(data, manifest)

    def test_missing_failed_or_mixed_run_evidence_rejected(self):
        import copy, json, hashlib
        data, manifest = self.evidence()
        variants=[]
        changed=copy.deepcopy(data);changed['postcheck']['baseline_unchanged']=False;variants.append(changed)
        changed=copy.deepcopy(data);del changed['files']['target.json'];variants.append(changed)
        changed=copy.deepcopy(data);changed['files']['receipt.json']['sha256']='different';variants.append(changed)
        changed=copy.deepcopy(data);receipt=json.loads(changed['files']['receipt.json']['body']);receipt['run_id']='run-2'
        body=json.dumps(receipt);changed['files']['receipt.json']={'body':body,'sha256':hashlib.sha256(body.encode()).hexdigest()};variants.append(changed)
        changed=copy.deepcopy(data);m=json.loads(changed['files']['manifest.json']['body']);m['files']={}
        body=json.dumps(m);changed['files']['manifest.json']={'body':body,'sha256':hashlib.sha256(body.encode()).hexdigest()};variants.append(changed)
        for changed in variants:
            with self.subTest(), self.assertRaises(ValueError): self.check(changed, manifest)
        with self.assertRaises(ValueError): self.check(data, manifest, exit_code=2)


if __name__ == '__main__':
    unittest.main()
