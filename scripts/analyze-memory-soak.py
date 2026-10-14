#!/usr/bin/env python3
"""Validate a completed resource protocol; never infer a memory budget verdict."""
import argparse
import json
import math
from pathlib import Path
import sys


def number(value):
    return type(value) in (int, float) and math.isfinite(value)


CLEANUP_FIELDS = ('owned_target_stopped', 'awake_stopped', 'lock_absent', 'baseline_unchanged')


def cleanup_verified(receipt, required=False):
    if 'cleanup' not in receipt:
        if required:
            raise ValueError('Cleanup evidence is missing')
        return False
    cleanup = receipt['cleanup']
    if not isinstance(cleanup, dict) or any(cleanup.get(key) is not True for key in CLEANUP_FIELDS):
        raise ValueError('Cleanup evidence is incomplete or failed')
    return True


def analyze(rows, receipt, require_cleanup=False):
    cleaned = cleanup_verified(receipt, require_cleanup)
    expected = [('idle', 0)] * 7
    expected += [(phase, cycle) for cycle in range(1, 21) for phase in ('open', 'released')]
    expected += [('complete', 0)]
    if len(rows) != len(expected):
        raise ValueError('Incomplete or duplicated sample sequence')
    elapsed, identities, complete_samples = [], set(), 0
    footprints = {phase: [] for phase in ('idle', 'open', 'released')}
    for row, (phase, cycle) in zip(rows, expected):
        if row.get('schema_version') != 1 or row.get('phase') != phase or row.get('cycle', 0) != cycle:
            raise ValueError('Unexpected protocol phase or cycle')
        timestamp = row.get('protocol_elapsed')
        if not number(timestamp) or timestamp < 0 or (elapsed and timestamp < elapsed[-1]):
            raise ValueError('Invalid or regressing protocol time')
        elapsed.append(timestamp)
        if phase == 'complete':
            continue
        observation = row.get('observation', {})
        start = observation.get('main_start')
        main = [member for member in observation.get('members', []) if member.get('role') == 'main']
        if (observation.get('schema_version') != 2 or observation.get('status') != 'observed' or type(start) is not int or start <= 0
                or len(main) != 1 or type(main[0].get('pid')) is not int or main[0]['pid'] <= 0):
            raise ValueError('Missing stable target identity')
        identities.add((main[0]['pid'], start))
        footprint = main[0].get('footprint_mib')
        if number(footprint) and footprint >= 0:
            footprints[phase].append(footprint)
        total = observation.get('total_footprint_mib')
        if (observation.get('complete') is True and observation.get('membership_stable') is True
                and observation.get('attribution') == 'observed-resource-coalition'
                and observation.get('unresolved_membership') == 0
                and number(total) and total >= 0
                and all(number(member.get('footprint_mib')) and member['footprint_mib'] >= 0
                        for member in observation['members'])
                and abs(total - sum(member['footprint_mib'] for member in observation['members'])) < .011):
            complete_samples += 1
    final = rows[-1]
    if (len(identities) != 1 or final.get('protocol_complete') is not True
            or final.get('budget_assessed') is not False or final.get('cycles') != 20
            or not number(final.get('idle_seconds')) or final['idle_seconds'] < 1800):
        raise ValueError('Completion or identity proof is missing')
    # Printed timestamps have millisecond resolution and sampling itself takes
    # time. Allow 100ms of read-duration variation, never a shorter idle phase.
    if any(elapsed[index] - elapsed[0] < index * 300 - .1 for index in range(1, 7)):
        raise ValueError('Idle sampling did not cover thirty minutes')
    started, finished = receipt.get('started'), receipt.get('finished')
    if (receipt.get('schema') != 1 or type(receipt.get('exit_code')) is not int or receipt['exit_code'] != 0
            or receipt.get('budget_assessed') is not False
            or not number(started) or not number(finished)
            or finished - started < elapsed[-1] - .1):
        raise ValueError('Runner receipt does not prove successful completion')
    return {'protocol_complete': True, 'cleanup_verified': cleaned, 'idle_samples': 7, 'cycles': 20,
            'same_target_identity': True, 'complete_samples': complete_samples,
            'partial_samples': 47 - complete_samples, 'budget_assessed': False,
            'main_footprint_mib': {phase: {'readable_samples': len(values),
                'min': min(values) if values else None, 'max': max(values) if values else None}
                for phase, values in footprints.items()}}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--stream', type=Path, required=True)
    parser.add_argument('--receipt', type=Path, required=True)
    parser.add_argument('--require-cleanup', action='store_true')
    args = parser.parse_args()
    if args.stream.stat().st_size > 1024 * 1024 or args.receipt.stat().st_size > 4096:
        raise ValueError('Evidence exceeds the protocol bounds')
    rows = [json.loads(line) for line in args.stream.read_text().splitlines()]
    print(json.dumps(analyze(rows, json.loads(args.receipt.read_text()), args.require_cleanup)))


if __name__ == '__main__':
    try:
        main()
    except (OSError, ValueError, TypeError, KeyError, AttributeError):
        print('Resource protocol evidence incomplete or invalid; no budget verdict.', file=sys.stderr)
        sys.exit(2)
