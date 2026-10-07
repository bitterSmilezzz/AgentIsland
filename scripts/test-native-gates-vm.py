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
        meta = dict(platform='Darwin', virtualized='1', console_uid=501, uid=501, python_ready=True)
        self.assertTrue(vm.guest_verified(meta))
        for change in [dict(platform='Linux'), dict(virtualized='0'), dict(console_uid=0),
                       dict(console_uid='501'), dict(uid=502), dict(python_ready=False)]:
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


if __name__ == '__main__':
    unittest.main()
