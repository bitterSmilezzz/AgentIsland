#!/usr/bin/env python3
"""Three-process acceptance for the notification ingress ledger using the real binary.

Each phase is a separate process, so recovery is exercised across real exits, not by
rebuilding an in-memory store. The fixture is an explicit empty temp directory with an
ownership marker; the probe refuses anything else. No user config, token or notification
channel is touched, and no system or outbound notification is sent.
"""
import argparse
import json
import os
from pathlib import Path
import stat
import subprocess
import sys
import tempfile

# Private text the probe posts; it must never appear in persisted state.
SENTINELS = ('synthetic private body', '/synthetic/private/source')
LEDGER = 'notify-ledger.json'


def run_phase(binary, fixture, phase, timeout=30):
    result = subprocess.run([str(binary), '--notify-ledger-probe', str(fixture), str(phase)],
                            capture_output=True, timeout=timeout)
    assert result.returncode == 0, (
        f'phase {phase} failed (exit {result.returncode}): '
        f'{result.stderr.decode(errors="replace").strip()[:200]}')
    return json.loads(result.stdout.decode())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)

    with tempfile.TemporaryDirectory(prefix='agentisland-ledger-probe-') as temporary:
        fixture = Path(temporary).resolve()
        ledger = fixture / LEDGER

        first = run_phase(binary, fixture, 1)
        assert first['restoredPending'] == 0, 'a fresh process must start with no restored notifications'
        assert first['pendingBeforeAck'] == 3, 'two agents plus one new id must each enqueue once'
        assert first['duplicates'] == 1, 'a repeated id must be suppressed exactly once'
        # The probe never exceeds the 64-entry retained queue, so nothing may be
        # evicted: a nonzero count would mean a body was dropped while the receipt
        # still described it as pending.
        assert first['evicted'] == 0, 'no probe event may be evicted by the capacity bound'
        assert first['restoredSafe'] and first['flushSaved'] and first['failures'] == 0
        assert first['httpAccepted'] == 4

        # The ledger is the recovery boundary: it must exist, be owner-only, and carry
        # no private body, no source path and no raw client id.
        assert ledger.is_file(), 'the probe must publish its ledger'
        mode = stat.S_IMODE(ledger.stat().st_mode)
        assert mode == 0o600, f'ledger must be owner-only, got {oct(mode)}'
        raw = ledger.read_text(encoding='utf-8', errors='replace')
        for sentinel in SENTINELS:
            assert sentinel not in raw, f'private text leaked into persisted state: {sentinel!r}'
        for raw_id in ('same-id', 'new-id'):
            assert raw_id not in raw, f'raw client id leaked into persisted state: {raw_id!r}'
        payload = json.loads(raw)
        assert payload['schema_version'] == 1
        assert len(payload['pending']) == 3
        for stored in payload['pending']:
            assert set(stored) == {'id', 'agent_id', 'event_type', 'timestamp',
                                  'duration', 'externally_delivered'}, stored.keys()
            assert len(stored['id']) == 64
        assert all(len(h) == 64 for h in payload['identities'])

        # Phase 2 is a new process: persisted identities must still suppress replays,
        # restored notifications must reappear, and acknowledging them must empty the queue.
        second = run_phase(binary, fixture, 2)
        assert second['pid'] != first['pid'], 'each phase must be a separate process'
        assert second['restoredPending'] == 3, 'unacknowledged notifications must survive restart'
        assert second['restoredSafe'], 'restored entries must be safe projections'
        assert second['httpAccepted'] == 3
        assert second['duplicates'] == 3, 'replays after restart must be suppressed'
        assert second['pendingAfterAck'] == 0, 'acknowledging must empty the recovered queue'
        assert second['evicted'] == 0, 'restored replays must not evict a body'
        assert second['flushSaved'] and second['failures'] == 0

        # Phase 3 is another new process: acknowledged notifications must not revive.
        third = run_phase(binary, fixture, 3)
        assert third['pid'] not in (first['pid'], second['pid'])
        assert third['restoredPending'] == 0, 'acknowledged notifications must not revive'
        assert third['pendingBeforeAck'] == 0
        assert third['duplicates'] == 1, 'identity survives ack within the window'
        assert third['evicted'] == 0, 'a suppressed replay is not an eviction'
        assert third['flushSaved'] and third['failures'] == 0

        print('PASS: notification ledger survives three real processes — '
              f'idempotent ({first["duplicates"]} suppressed), restored {second["restoredPending"]}, '
              f'acknowledged without revival, 0600 ledger free of private text and raw ids')


if __name__ == '__main__':
    main()
