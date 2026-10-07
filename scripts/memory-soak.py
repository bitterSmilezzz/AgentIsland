#!/usr/bin/env python3
"""Observe a pre-existing formal application in an exclusively used macOS VM.

Reports protocol completion, never an automatic memory-budget verdict.
Requires workbench initially open (as left by the native single-instance gate).
"""
import argparse
import atexit
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import time

spec = importlib.util.spec_from_file_location('measure', Path(__file__).with_name('measure-memory.py'))
measure = importlib.util.module_from_spec(spec)
spec.loader.exec_module(measure)


def wait_until(deadline, clock=time.monotonic, sleep=time.sleep):
    while clock() < deadline:
        sleep(min(1, max(0, deadline - clock())))


def identity_matches(native, pid, binary, start):
    usage = native.usage(pid)
    return (usage is not None and usage['start'] == start
            and any(p == pid and Path(command).resolve() == binary for p, command in native.processes()))


class Protocol:
    def __init__(self, pid, binary, log, native, emit, clock=time.monotonic, sleep=time.sleep):
        self.pid, self.binary, self.log, self.native = pid, binary, log, native
        self.emit, self.clock, self.sleep = emit, clock, sleep
        self.started = clock()
        usage = native.usage(pid)
        if usage is None:
            raise ValueError('Target unavailable')
        self.start = usage['start']
        self.check()

    def check(self):
        if not identity_matches(self.native, self.pid, self.binary, self.start):
            raise ValueError('Application exited, restarted, or binary identity changed')

    def sample(self, phase, cycle=0):
        self.check()
        result = measure.sample(self.pid, self.native, expected_start=self.start)
        if result.get('status') != 'observed':
            raise ValueError('Resource observation failed; protocol incomplete')
        self.emit(dict(schema_version=1, phase=phase, cycle=cycle,
                       protocol_elapsed=round(self.clock()-self.started, 3), observation=result))

    def transition(self, target, marker, timeout):
        self.check()
        offset = self.log.stat().st_size
        # Multiple acceptance bundles can have the same scheme registration.
        # Bind native dispatch to the measured bundle; the global CLI handler is
        # a separate delivery check, not a reliable selector for this protocol.
        subprocess.run(['open', '-a', str(self.binary.parents[2]), f'agentisland://{target}'], check=True, timeout=15,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        deadline = self.clock() + timeout
        while self.clock() < deadline:
            self.check()
            if self.log.stat().st_size < offset:
                raise ValueError('Application log rotated; transition proof unavailable')
            with self.log.open('rb') as stream:
                stream.seek(offset)
                # Only consume newly appended bytes; unrelated log content is never emitted.
                if marker in stream.read():
                    return
            self.sleep(1)
        raise ValueError('Expected native window transition was not observed')

    def execute(self):
        self.transition('workbench-hide', b'[window] released hidden workbench', 120)
        idle_start = self.clock()
        for seconds in range(0, 1801, 300):
            wait_until(idle_start + seconds, self.clock, self.sleep)
            self.sample('idle')
        if self.clock() - idle_start < 1800:
            raise ValueError('Idle observation shorter than 30 minutes')
        idle_elapsed = self.clock() - idle_start
        for cycle in range(1, 21):
            self.transition('workbench', b'[window] created workbench', 30)
            wait_until(self.clock() + 6, self.clock, self.sleep)
            self.sample('open', cycle)
            self.transition('workbench-hide', b'[window] released hidden workbench', 120)
            self.sample('released', cycle)
        self.emit(dict(schema_version=1, phase='complete', protocol_complete=True,
                       budget_assessed=False, idle_seconds=round(idle_elapsed, 3),
                       cycles=20, protocol_elapsed=round(self.clock()-self.started, 3)))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--pid', type=int, required=True)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--log', type=Path, required=True)
    args = parser.parse_args()
    if sys.platform != 'darwin' or args.pid <= 0:
        parser.error('Requires an existing macOS application PID')
    virtualized = subprocess.check_output(['sysctl', '-n', 'kern.hv_vmm_present'], text=True).strip()
    if virtualized != '1' or os.getuid() < 500 or os.stat('/dev/console').st_uid != os.getuid():
        parser.error('Requires the matching logged-in Aqua user inside a macOS VM')
    binary = args.binary.resolve(strict=True)
    if not args.log.is_file() or not str(binary).endswith('/AgentIsland.app/Contents/MacOS/agentisland'):
        parser.error('Requires a formal bundle binary and its existing application log')
    lock = Path.home() / '.agentisland-native-gates.lock'
    lock.mkdir(mode=0o700)
    atexit.register(lock.rmdir)
    emit = lambda record: print(json.dumps(record), flush=True)
    protocol = Protocol(args.pid, binary, args.log, measure.Native(), emit)
    protocol.execute()


if __name__ == '__main__':
    try:
        main()
    except (OSError, ValueError, subprocess.SubprocessError):
        print(json.dumps(dict(schema_version=1, phase='failed', protocol_complete=False,
                              budget_assessed=False)), flush=True)
        raise SystemExit(2)
