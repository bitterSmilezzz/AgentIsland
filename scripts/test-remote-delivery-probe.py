#!/usr/bin/env python3
"""Real-binary end-to-end acceptance for the **outbound** delivery path (NOT-05).

Why this exists: the outbound path previously had only `#[cfg(test)]` unit tests with a
`ScriptedTransport`. Nothing exercised the *real* `HttpTransport` inside the *shipped
binary* against a real peer that answers 429 / 500 / 200. The ingress ledger had
`not01.ledgerrestart`; outbound had no counterpart. This closes that gap.

Safety: the probe is a headless argv mode. It never reads user config or the keychain,
never touches a real channel, and only talks to a loopback peer the probe itself starts.
No system notification and no outbound notification is sent.
"""
import argparse
import json
from pathlib import Path
import subprocess

PROBE = '--remote-delivery-probe'
# 四态文案的**唯一来源**是契约夹具（Rust 的 `DeliveryState::label()` 也对着它断言）。
# 这里不抄一份字面量：抄了就把「唯一来源」变成多处，改夹具而漏改这里时维护面变大。
# 路径相对于本脚本所在目录，脚本从仓库根跑或别处跑都一样。
CONTRACT = Path(__file__).resolve().parent.parent / 'app/src-tauri/tests/fixtures/contracts/notify-keys.json'
REQUIRED_KEYS = {'schemaVersion', 'pid', 'throttled', 'escalation', 'accepted',
                 'throttledPeerRequests'}


def run_probe(binary, timeout=90):
    result = subprocess.run([str(binary), PROBE], capture_output=True, timeout=timeout)
    assert result.returncode == 0, (
        f'probe failed (exit {result.returncode}): '
        f'{result.stderr.decode(errors="replace").strip()[:300]}')
    stdout = result.stdout.decode()
    # The probe must print exactly one JSON object and nothing else, so a stray log line
    # can never be mistaken for the receipt.
    report = json.loads(stdout)
    assert isinstance(report, dict), 'probe must emit a JSON object'
    return report


with CONTRACT.open(encoding='utf-8') as handle:
    LABELS = json.load(handle)['delivery_labels']


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    assert binary.is_file(), f'binary not found: {binary}'

    report = run_probe(binary)
    missing = REQUIRED_KEYS - report.keys()
    assert not missing, f'probe receipt is missing keys: {sorted(missing)}'
    assert report['schemaVersion'] == 1, 'receipt schema must be version 1'
    assert isinstance(report['pid'], int) and report['pid'] > 0, 'receipt must name its pid'

    # ① 429 + Retry-After: the peer's own wait time must reach the UI text, and a
    # throttled send must not claim any delivery segment.
    throttled = report['throttled']
    assert throttled['outcome'] == 'throttled', 'a 429 must be classified as throttled'
    assert '约 12 秒后可重试' in throttled['text'], (
        f'the peer-supplied wait time must appear in the text, got {throttled["text"]!r}')
    assert throttled['deliveryLabel'] == LABELS['unknown'], (
        'a throttled send must not claim accepted/received/read')
    assert report['throttledPeerRequests'] == 1, (
        'a throttled send must not be retried immediately (Retry-After says when to come back)')

    # ② Three consecutive transient failures escalate the channel; escalation then stops
    # retrying without swallowing the result.
    escalation = report['escalation']
    assert escalation['escalated'] == ['customHTTP'], (
        f'the failing channel must escalate, got {escalation["escalated"]}')
    assert escalation['requestsAfterThree'] == 6, (
        'three transient failures = 3 x (first attempt + one retry) = 6 requests, '
        f'got {escalation["requestsAfterThree"]}')
    assert escalation['requestsAfterFourth'] == 7, (
        'after escalation the send must still happen exactly once and not retry, '
        f'got {escalation["requestsAfterFourth"]}')

    # ③ A 200 is 受理 only: it must never be reported as 已送达.
    accepted = report['accepted']
    assert accepted['accepted'] is True, 'a 200 must be recorded as accepted'
    assert accepted['delivery'] == 'accepted', (
        f'delivery segment must be accepted, got {accepted["delivery"]}')
    assert accepted['deliveryLabel'] == LABELS['accepted'], (
        f'acceptance must not be called delivered, got {accepted["deliveryLabel"]!r}')
    assert accepted['deliveryLabel'] != LABELS['received'], 'acceptance must never read as delivered'

    print(f'PASS: outbound delivery end-to-end — throttled(retry-after)/escalation(no loop)'
          f'/accepted(not delivered), pid {report["pid"]}')


if __name__ == '__main__':
    main()
