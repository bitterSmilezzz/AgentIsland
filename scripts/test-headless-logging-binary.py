#!/usr/bin/env python3
"""BUG-003: verify the shipped application binary leaves user logs untouched.

Wraps ShippedBinaryTests from test-headless-logging.py with an explicit --binary path.
No user configuration or live application instance is touched.
"""
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
TEST = ROOT / 'scripts/test-headless-logging.py'


def main():
    binary = sys.argv[1] if len(sys.argv) > 1 else str(ROOT / 'dist/agentisland')
    cmd = [sys.executable, '-B', str(TEST), '--binary', binary, 'ShippedBinaryTests']
    result = subprocess.run(cmd)
    sys.exit(result.returncode)


if __name__ == '__main__':
    main()
