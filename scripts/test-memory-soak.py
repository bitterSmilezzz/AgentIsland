#!/usr/bin/env python3
"""Exercise protocol duration, identity and failure outcomes without native UI."""
import importlib.util
from pathlib import Path
import unittest
import tempfile
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('soak', Path(__file__).with_name('memory-soak.py'))
soak = importlib.util.module_from_spec(spec)
spec.loader.exec_module(soak)
BINARY = Path('/fixture/AgentIsland.app/Contents/MacOS/agentisland')


class Clock:
    def __init__(self): self.now = 0
    def read(self): return self.now
    def sleep(self, duration): self.now += duration


class Native:
    def usage(self, pid): return {'start': 123}
    def processes(self): return [(10, str(BINARY))]


class Tests(unittest.TestCase):
    def test_wrong_pid_path_and_reused_pid_are_rejected(self):
        native = Native()
        self.assertTrue(soak.identity_matches(native, 10, BINARY, 123))
        self.assertFalse(soak.identity_matches(native, 11, BINARY, 123))
        self.assertFalse(soak.identity_matches(native, 10, Path('/other'), 123))
        self.assertFalse(soak.identity_matches(native, 10, BINARY, 124))

    def fixture(self):
        clock, output = Clock(), []
        protocol = soak.Protocol(10, BINARY, Path('/unused-log'), Native(), output.append,
                                 clock.read, clock.sleep)
        transitions = []
        def transition(target, marker, timeout):
            transitions.append(target)
            clock.sleep(90 if target == 'workbench-hide' else 1)
        protocol.transition = transition
        return clock, output, protocol, transitions

    @patch.object(soak.measure, 'sample', return_value={'status':'observed','complete':False})
    def test_full_thirty_minutes_and_twenty_verified_cycles(self, sample):
        clock, output, protocol, transitions = self.fixture()
        protocol.execute()
        idle = [row for row in output if row['phase'] == 'idle']
        self.assertEqual(len(idle), 7)
        self.assertEqual(idle[-1]['protocol_elapsed']-idle[0]['protocol_elapsed'], 1800)
        self.assertEqual(transitions.count('workbench'), 20)
        self.assertEqual(transitions.count('workbench-hide'), 21)
        self.assertEqual([row['cycle'] for row in output if row['phase']=='released'], list(range(1,21)))
        self.assertEqual(output[-1]['idle_seconds'], 1800)
        self.assertTrue(output[-1]['protocol_complete'])
        self.assertFalse(output[-1]['budget_assessed'], 'partial attribution cannot become a budget pass')

    @patch.object(soak.measure, 'sample', return_value={'status':'target_replaced'})
    def test_failed_sampling_never_emits_complete(self, sample):
        _, output, protocol, _ = self.fixture()
        with self.assertRaises(ValueError): protocol.execute()
        self.assertFalse(any(row.get('protocol_complete') for row in output))

    @patch.object(soak.subprocess, 'run')
    def test_old_log_marker_cannot_prove_new_transition(self, run):
        clock = Clock()
        with tempfile.TemporaryDirectory() as directory:
            log = Path(directory)/'app.log'
            log.write_bytes(b'[window] created workbench\n')
            protocol = soak.Protocol(10, BINARY, log, Native(), lambda _:None,
                                     clock.read, clock.sleep)
            with self.assertRaisesRegex(ValueError, 'not observed'):
                protocol.transition('workbench', b'[window] created workbench', 3)
            self.assertEqual(clock.now, 3)

    @patch.object(soak.subprocess, 'run')
    def test_new_native_marker_proves_transition(self, run):
        clock = Clock()
        with tempfile.TemporaryDirectory() as directory:
            log = Path(directory)/'app.log'
            log.write_bytes(b'')
            def append(*args, **kwargs):
                with log.open('ab') as stream:stream.write(b'[window] created workbench\n')
            run.side_effect = append
            protocol = soak.Protocol(10, BINARY, log, Native(), lambda _:None,
                                     clock.read, clock.sleep)
            protocol.transition('workbench', b'[window] created workbench', 3)
            self.assertEqual(clock.now, 0)
            self.assertEqual(run.call_args.args[0],
                             ['open', '-a', str(BINARY.parents[2]), 'agentisland://workbench'],
                             'resource protocol must target the measured bundle, not another registered copy')

    def test_sleep_rechecks_deadline(self):
        clock = Clock()
        soak.wait_until(30, clock.read, clock.sleep)
        self.assertEqual(clock.now, 30)
        soak.wait_until(20, clock.read, clock.sleep)
        self.assertEqual(clock.now, 30, 'expired slots must not delay or repeat')


if __name__ == '__main__': unittest.main()
