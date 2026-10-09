#!/usr/bin/env python3
"""Read an existing macOS process, without creating or manipulating windows.

Footprint is a per-process ledger; a member sum is not exclusive machine memory.
Unknown reads remain null. Coalition enumeration is a non-atomic observation;
complete does not prove every future process or shared allocation is covered.
"""
import argparse
import ctypes
import errno
import json
import math
import os
import subprocess
import sys
import time


class RusageV2(ctypes.Structure):
    # Installed macOS SDK sys/resource.h: RUSAGE_INFO_V2, public libproc API.
    _fields_ = [('uuid', ctypes.c_uint8 * 16)] + [(name, ctypes.c_uint64) for name in (
        'user_time', 'system_time', 'idle_wakeups', 'interrupt_wakeups', 'pageins',
        'wired_size', 'resident_size', 'phys_footprint', 'start', 'exit',
        'child_user_time', 'child_system_time', 'child_idle_wakeups',
        'child_interrupt_wakeups', 'child_pageins', 'child_elapsed',
        'disk_read', 'disk_write')]


class Native:
    def __init__(self):
        self.lib = ctypes.CDLL('/usr/lib/libproc.dylib', use_errno=True)
        self.lib.proc_pidinfo.argtypes = [ctypes.c_int, ctypes.c_int, ctypes.c_uint64, ctypes.c_void_p, ctypes.c_int]
        self.lib.proc_pidinfo.restype = ctypes.c_int
        self.lib.proc_pid_rusage.argtypes = [ctypes.c_int, ctypes.c_int, ctypes.c_void_p]
        self.lib.proc_pid_rusage.restype = ctypes.c_int

    def coalition(self, pid):
        # XNU private PROC_PIDCOALITIONINFO=20; may fail or change by OS version.
        buf = (ctypes.c_uint64 * 5)()
        if self.lib.proc_pidinfo(pid, 20, 0, ctypes.byref(buf), ctypes.sizeof(buf)) != ctypes.sizeof(buf):
            return None
        return int(buf[0]) or None

    def usage(self, pid):
        buf = RusageV2()
        if self.lib.proc_pid_rusage(pid, 2, ctypes.byref(buf)) != 0 or not buf.start or buf.exit:
            return None
        return {name: int(getattr(buf, name)) for name in (
            'start', 'resident_size', 'phys_footprint', 'user_time', 'system_time',
            'idle_wakeups', 'interrupt_wakeups', 'pageins', 'disk_read', 'disk_write')}

    def is_absent(self, pid):
        # Signal zero only checks existence/permission; no signal is delivered.
        # EPERM and other failures cannot prove absence or coalition ownership.
        if pid <= 0:
            return False
        try:
            os.kill(pid, 0)
        except OSError as error:
            return error.errno == errno.ESRCH
        return False

    def processes(self):
        command = ['ps', '-axo', 'pid=,state=,comm=']
        with subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                              text=True, errors='replace') as observer:
            try:
                output, _ = observer.communicate(timeout=5)
            except subprocess.TimeoutExpired:
                observer.kill()
                observer.communicate()
                raise
            if observer.returncode:
                raise subprocess.CalledProcessError(observer.returncode, command)
            observer_pid = observer.pid
        rows = []
        for row in output.splitlines():
            if not row.strip():
                continue
            values = row.strip().split(None, 2)
            if len(values) != 3 or not values[0].isdigit():
                raise ValueError('invalid process enumeration')
            # The snapshot contains this exact child, which has exited before
            # coalition reads. Exclude our observer, never other processes by name.
            # An explicit zombie has no live task/resource membership to read.
            # Unknown states and processes that disappear later remain unresolved.
            if int(values[0]) != observer_pid and not values[1].startswith('Z'):
                rows.append((int(values[0]), values[2]))
        return rows


def role(pid, main, command):
    if pid == main:
        return 'main'
    for suffix, name in (('com.apple.WebKit.WebContent', 'web-content'),
                         ('com.apple.WebKit.GPU', 'gpu'), ('com.apple.WebKit.Networking', 'network')):
        if command.endswith('/' + suffix) or command == suffix:
            return name
    return 'other'


def summarize(members, attribution, unresolved, stable):
    readable = [member['footprint_mib'] for member in members if member['footprint_mib'] is not None]
    complete = (attribution == 'observed-resource-coalition' and bool(members)
                and len(readable) == len(members) and unresolved == 0 and stable)
    return {'attribution': attribution, 'members': members,
            'known_member_footprint_mib': round(sum(readable), 2),
            'total_footprint_mib': round(sum(readable), 2) if complete else None,
            'unresolved_membership': unresolved, 'membership_stable': stable,
            'complete': complete}


def sample(pid, native, expected_start=None, collector_pid=None):
    before = native.usage(pid)
    if before is None:
        return {'schema_version': 2, 'status': 'target_unavailable'}
    if expected_start is not None and before['start'] != expected_start:
        return {'schema_version': 2, 'status': 'target_replaced'}
    group = native.coalition(pid)
    collector_group = native.coalition(collector_pid if collector_pid is not None else os.getpid())
    isolated = group is not None and collector_group is not None and group != collector_group
    unresolved, exited, stable, members = 0, 0, True, []
    processes = native.processes() if isolated else [(pid, '')]
    seen = set()
    if not any(member == pid for member, _ in processes):
        return {'schema_version': 2, 'status': 'target_unavailable'}
    for member, command in processes:
        if member in seen:
            continue
        seen.add(member)
        member_group = native.coalition(member) if isolated else None
        if isolated and member_group is None:
            if native.is_absent(member):
                if member == pid:
                    return {'schema_version': 2, 'status': 'target_unavailable'}
                exited += 1
                continue
            unresolved += 1
        if member != pid and (not isolated or member_group != group):
            continue
        usage = native.usage(member)
        after = native.usage(member) if usage is not None else None
        if usage is not None and (after is None or after['start'] != usage['start']):
            usage = None
            stable = False
        if isolated and native.coalition(member) != group:
            usage = None
            stable = False
        metrics = {'pid': member, 'role': role(member, pid, command),
                   'resident_mib': round(usage['resident_size'] / 1048576, 2) if usage else None,
                   'footprint_mib': round(usage['phys_footprint'] / 1048576, 2) if usage else None}
        # CPU raw counters are deliberately not converted to percent without verified units.
        for field in ('user_time', 'system_time', 'idle_wakeups', 'interrupt_wakeups', 'pageins', 'disk_read', 'disk_write'):
            metrics[field + '_raw' if field.endswith('_time') else field] = usage[field] if usage else None
        members.append(metrics)
    after = native.usage(pid)
    if after is None:
        return {'schema_version': 2, 'status': 'target_unavailable'}
    if after['start'] != before['start']:
        return {'schema_version': 2, 'status': 'target_replaced'}
    stable = stable and native.coalition(pid) == group
    return {'schema_version': 2, 'status': 'observed', 'main_start': before['start'],
            'exited_during_enumeration': exited,
            **summarize(members, 'observed-resource-coalition' if isolated else 'main-only', unresolved, stable)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--pid', type=int, required=True)
    parser.add_argument('--seconds', type=int, default=30)
    parser.add_argument('--interval', type=int, default=10)
    args = parser.parse_args()
    if sys.platform != 'darwin':
        parser.error('libproc sampling requires macOS')
    if not 1 <= args.pid <= 2147483647 or not 1 <= args.seconds <= 3600 or not 1 <= args.interval <= 60:
        parser.error('pid: 1..2147483647; seconds: 1..3600; interval: 1..60')
    native = Native()
    started, deadline, expected_start, next_sample = time.monotonic(), time.monotonic() + args.seconds, None, 0
    while True:
        target = started + next_sample * args.interval
        if target > deadline:
            break
        time.sleep(max(0, target - time.monotonic()))
        try:
            result = sample(args.pid, native, expected_start)
        except (OSError, ValueError, subprocess.SubprocessError):
            result = {'schema_version': 2, 'status': 'sampling_failed'}
        print(json.dumps({'elapsed': round(time.monotonic() - started, 3), **result}), flush=True)
        if result['status'] != 'observed':
            return 2
        expected_start = result['main_start']
        # Skip missed slots rather than issuing a burst of overdue samples.
        next_sample = max(next_sample + 1, math.ceil((time.monotonic() - started) / args.interval))
    return 0


if __name__ == '__main__':
    sys.exit(main())
