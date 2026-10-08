#!/usr/bin/env python3
"""Visible four-edge motion regression; requires a real logged-in macOS VM."""
import argparse
import json
import math
import os
from pathlib import Path
import subprocess
import sys
import tempfile

LABELS = ['settings-enter', 'settings-return', 'usage-enter', 'usage-return',
          'agent-enter', 'agent-return', 'rapid-settings-return', 'late-usage', 'late-usage-return']


def valid_frame(frame):
    if not isinstance(frame, dict):
        return False
    def number(value):
        return type(value) in (int, float) and math.isfinite(value)
    for name in ('card', 'native', 'viewport'):
        rect = frame.get(name, {})
        keys = ('width', 'height') if name == 'viewport' else ('x', 'y', 'width', 'height')
        if not isinstance(rect, dict) or not all(number(rect.get(key)) for key in keys):
            return False
        if rect['width'] <= 0 or rect['height'] <= 0:
            return False
    return (number(frame.get('t')) and frame['t'] >= 0
            and frame['native'].get('visible') is True and type(frame.get('navigating')) is bool)


def valid_result(result, edge):
    traces = result.get('traces', [])
    return (result.get('schema') == 1 and result.get('passed') is True
            and result.get('edge') == edge and [trace.get('label') for trace in traces] == LABELS
            and all(trace.get('edge') == edge and len(trace.get('frames', [])) >= 12
                    and all(valid_frame(frame) for frame in trace['frames'])
                    and trace['frames'][-1]['navigating'] is False
                    and trace.get('assessment', {}).get('passed') is True
                    and not trace.get('assessment', {}).get('failures')
                    for trace in traces))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    args = parser.parse_args()
    if sys.platform != 'darwin' or os.getuid() < 500 or os.stat('/dev/console').st_uid != os.getuid():
        parser.error('Requires the logged-in Aqua user inside a macOS VM')
    if subprocess.check_output(['sysctl', '-n', 'kern.hv_vmm_present'], text=True).strip() != '1':
        parser.error('Requires actual virtualization; host desktop is not allowed')
    binary = args.binary.resolve(strict=True)
    if not str(binary).endswith('/AgentIsland.app/Contents/MacOS/agentisland'):
        parser.error('Requires an explicit formal application bundle')
    lock = Path.home()/'.agentisland-native-gates.lock'
    owned = False
    try:
        try:
            lock.mkdir(mode=0o700)
            owned = True
        except FileExistsError:
            owner = json.loads((lock/'owner.json').read_text())
            if owner.get('pid') != os.getppid():
                raise ValueError('Native acceptance is already running')
        evidence = Path('.build/motion'); evidence.mkdir(parents=True, exist_ok=True)
        for edge in ['top', 'bottom', 'left', 'right']:
            app = subprocess.Popen([str(binary), '--motion-smoke', f'--motion-edge={edge}'],
                                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            log = Path(tempfile.gettempdir())/f'agentisland-test-{app.pid}.log'
            try:
                code = app.wait(timeout=75)
                lines = [line.split('MOTION_RESULT ', 1)[1] for line in log.read_text().splitlines()
                         if 'MOTION_RESULT ' in line]
                if len(lines) != 1:
                    raise ValueError('Missing or ambiguous motion result')
                result = json.loads(lines[0])
                (evidence/f'{edge}.json').write_text(json.dumps(result))
                if code != 0 or not valid_result(result, edge):
                    failures = {trace.get('label'):trace.get('assessment', {}).get('failures')
                                for trace in result.get('traces', []) if not trace.get('assessment', {}).get('passed')}
                    print(json.dumps(dict(edge=edge, passed=False, failures=failures)), flush=True)
                    raise ValueError('Native motion invariants failed')
                print(json.dumps(dict(edge=edge, passed=True, cases=len(result['traces']))), flush=True)
            finally:
                if app.poll() is None:
                    app.terminate(); app.wait(timeout=5)
    finally:
        if owned:
            lock.rmdir()


if __name__ == '__main__':
    try:
        main()
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        if isinstance(error, subprocess.TimeoutExpired):
            reason = f'Native process exceeded {error.timeout} seconds'
        elif isinstance(error, ValueError):
            reason = str(error)
        else:
            reason = type(error).__name__
        print(json.dumps({'native_motion_error': reason}), file=sys.stderr)
        print('Native motion acceptance incomplete; no quality verdict.', file=sys.stderr)
        sys.exit(2)
