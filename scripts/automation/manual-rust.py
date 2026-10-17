#!/usr/bin/env python3
"""Run one authorized compiled ignored probe; suppress all private probe output."""
import math
import os
from pathlib import Path
import re
import signal
import subprocess
import sys
import threading

ROOT = Path(__file__).resolve().parents[2]
LIST_LIMIT = 4 * 1024 * 1024


class CommandError(Exception):
    def __init__(self, code, reason):
        self.code = code
        self.reason = reason


def run_command(command, timeout, *, listing=False):
    # The session owns the entire subprocess group, including descendants that
    # outlive cargo. Never inherit probe output: it can contain real prompts.
    try:
        proc = subprocess.Popen(command, cwd=ROOT, stdin=subprocess.DEVNULL,
                                stdout=subprocess.PIPE if listing else subprocess.DEVNULL,
                                stderr=subprocess.DEVNULL, start_new_session=True)
    except OSError:
        raise CommandError(3, 'Manual probe command unavailable.') from None
    output = bytearray()
    incomplete = []

    def drain():
        try:
            while chunk := proc.stdout.read(65536):
                room = LIST_LIMIT - len(output)
                output.extend(chunk[:room])
                if len(chunk) > room:
                    incomplete.append(True)
        except OSError:
            incomplete.append(True)

    reader = threading.Thread(target=drain, daemon=True) if listing else None
    try:
        if reader:
            reader.start()
        try:
            proc.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            raise CommandError(1, 'Manual probe command timed out.') from None
    finally:
        # Always reclaim descendants, including on SIGINT/SIGTERM and success.
        try:
            os.killpg(proc.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        proc.wait(timeout=5)
        if reader and reader.ident is not None:
            reader.join(timeout=1)
            if reader.is_alive():
                incomplete.append(True)
    if proc.returncode:
        raise CommandError(1, 'Manual probe command failed (output suppressed).')
    if incomplete:
        raise CommandError(1, 'Compiled ignored-test listing was incomplete.')
    try:
        return output.decode('utf-8', errors='strict')
    except UnicodeDecodeError:
        raise CommandError(1, 'Compiled ignored-test listing was invalid.') from None


def main():
    if os.environ.get('ALLOW_PRIVATE_TEST_DATA') != '1':
        print('Manual probes require ALLOW_PRIVATE_TEST_DATA=1.', file=sys.stderr)
        return 3
    selected = os.environ.get('MANUAL_RUST_TEST', '')
    if len(sys.argv) != 1 or not re.fullmatch(r'[A-Za-z_][A-Za-z0-9_]*', selected):
        print('Select exactly one ignored Rust test by function name.', file=sys.stderr)
        return 2
    try:
        timeout = float(os.environ.get('MANUAL_RUST_TIMEOUT_SECONDS', '900'))
        if not math.isfinite(timeout) or not 0 < timeout <= 900:
            raise ValueError
    except ValueError:
        print('MANUAL_RUST_TIMEOUT_SECONDS must be finite and in (0, 900].', file=sys.stderr)
        return 2
    if os.name != 'posix':
        print('Manual probes require POSIX process-group cleanup support.', file=sys.stderr)
        return 3
    # The compiled binary alone determines membership; never inspect private data
    # or maintain a source-regex list. Duplicate full names also remain ambiguous.
    command = ['cargo', 'test', '--locked', '--manifest-path', str(ROOT / 'app/src-tauri/Cargo.toml')]
    try:
        listed = run_command(command + ['--', '--list', '--ignored'], timeout, listing=True)
        matches = [line.removesuffix(': test') for line in listed.splitlines()
                   if line.endswith(': test') and line.removesuffix(': test').split('::')[-1] == selected]
        if len(matches) != 1 or not re.fullmatch(r'[A-Za-z_][A-Za-z0-9_]*(?:::[A-Za-z_][A-Za-z0-9_]*)*', matches[0]):
            print('Selection must resolve to exactly one compiled ignored test.', file=sys.stderr)
            return 2
        run_command(command + [matches[0], '--', '--ignored', '--exact'], timeout)
    except CommandError as error:
        print(error.reason, file=sys.stderr)
        return error.code
    print('Selected manual probe passed (output suppressed).')
    return 0


def interrupted(_signum, _frame):
    raise KeyboardInterrupt


if __name__ == '__main__':
    signal.signal(signal.SIGTERM, interrupted)
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        print('Manual probe interrupted; subprocess group reclaimed.', file=sys.stderr)
        sys.exit(1)
