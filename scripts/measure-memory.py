#!/usr/bin/env python3
"""macOS memory samples; only aggregate counts, never arguments or session contents.

Measure an existing application with --pid PID. For lifecycle regression, first
in an isolated user session or VM, launch the packaged app with --shell=island --memory-smoke, then sample its PID
for 150 seconds. Attribution uses XNU resource coalitions, not PPID=launchd.
"""
import argparse
import ctypes
import json
import os
import pathlib
import re
import subprocess
import sys
import time


def coalition(pid):
    # XNU proc_info_private.h: PROC_PIDCOALITIONINFO=20, two IDs + three reserved u64.
    buf = (ctypes.c_uint64 * 5)()
    lib = ctypes.CDLL('/usr/lib/libproc.dylib')
    lib.proc_pidinfo.argtypes = [ctypes.c_int, ctypes.c_int, ctypes.c_uint64, ctypes.c_void_p, ctypes.c_int]
    lib.proc_pidinfo.restype = ctypes.c_int
    if lib.proc_pidinfo(pid, 20, 0, ctypes.byref(buf), ctypes.sizeof(buf)) != ctypes.sizeof(buf):
        return None
    return buf[0]


def footprint(pid):
    result = subprocess.run(['vmmap', '-summary', str(pid)], capture_output=True, text=True)
    match = re.search(r'^Physical footprint:\s+([\d.]+)([KMG])', result.stdout, re.M)
    if not match:
        return None
    return round(float(match[1]) * {'K':1/1024, 'M':1, 'G':1024}[match[2]], 2)


def sample(pid):
    group = coalition(pid)
    # A CLI launched from Codex shares its parent's coalition. Do not count that group.
    isolated = group is not None and group != coalition(os.getpid())
    rows = subprocess.run(['ps', '-axo', 'pid=,rss=,comm='], capture_output=True, text=True, errors="replace", check=True).stdout
    members = []
    for row in rows.splitlines():
        values = row.strip().split(None, 2)
        if len(values) != 3:
            continue
        member = int(values[0])
        if member != pid and not (isolated and coalition(member) == group):
            continue
        members.append({'pid':member, 'role':pathlib.Path(values[2]).name,
                        'rss_mib':round(int(values[1])/1024, 2), 'footprint_mib':footprint(member)})
    return {'attribution':'resource-coalition' if isolated else 'main-only', 'members':members,
            'total_footprint_mib':round(sum(m['footprint_mib'] or 0 for m in members), 2),
            'complete':all(m['footprint_mib'] is not None for m in members)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--pid', type=int, required=True)
    parser.add_argument('--seconds', type=int, default=30)
    parser.add_argument('--interval', type=int, default=10)
    args = parser.parse_args()
    if sys.platform != 'darwin':
        parser.error('vmmap and resource coalitions require macOS')
    if args.seconds < 1 or args.interval < 1:
        parser.error('seconds and interval must be positive')
    started = time.monotonic()
    while time.monotonic()-started <= args.seconds:
        result = sample(args.pid)
        if not result['members']:
            break
        print(json.dumps({'elapsed':round(time.monotonic()-started,1), **result}), flush=True)
        time.sleep(args.interval)


if __name__ == '__main__':
    main()
