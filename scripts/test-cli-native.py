#!/usr/bin/env python3
"""Verify packaged CLI delivery to the existing formal app in an isolated macOS VM.

Run after the single-instance gate, while no resource protocol is active.
Registers an unmodified temporary decoy bundle; it never starts that copy.
"""
import os
import atexit
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time

REGISTER = '/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister'


def main():
    if sys.platform != 'darwin' or os.getuid() < 500 or os.stat('/dev/console').st_uid != os.getuid():
        raise ValueError('Requires the matching logged-in Aqua user')
    desktop_authorized = '--allow-desktop' in sys.argv and os.environ.get('ALLOW_DESKTOP_UI') == '1'
    if subprocess.check_output(['sysctl','-n','kern.hv_vmm_present'],text=True).strip() != '1' and not desktop_authorized:
        raise ValueError('Requires actual virtualization')
    lock = Path.home()/'.agentisland-native-gates.lock'
    try:
        lock.mkdir(mode=0o700)
        atexit.register(lock.rmdir)
    except FileExistsError:
        owner = json.loads((lock/'owner.json').read_text())
        if owner.get('pid') != os.getppid():
            raise ValueError('Another native observation is active')
    root = Path(__file__).resolve().parent.parent
    bundle = root/'dist/AgentIsland.app'
    binary = bundle/'Contents/MacOS/agentisland'
    helper = Path(os.environ['POLICY_HELPER']) if os.environ.get('POLICY_HELPER') else None

    def app_pid():
        rows = subprocess.check_output(['ps','-axo','pid=,comm='],text=True).splitlines()
        pids = [int(parts[0]) for row in rows if len(parts := row.strip().split(None,1)) == 2
                and parts[1] == str(binary)]
        if len(pids) != 1:
            raise ValueError('Expected the existing formal application')
        return pids[0]

    pid = app_pid()
    def policy():
        if app_pid() != pid:
            raise ValueError('CLI replaced the target instance')
        return subprocess.check_output([str(helper),str(pid)],text=True,timeout=5).strip()

    def wait_policy(expected):
        deadline = time.monotonic()+15
        while time.monotonic() < deadline:
            if policy() == expected:
                return
            time.sleep(.1)
        raise ValueError('CLI did not reach the existing native app')

    with tempfile.TemporaryDirectory(prefix='agentisland-cli-decoy-') as temporary:
        if helper is None:
            source = Path(temporary)/'policy.swift'
            source.write_text('import AppKit\nif let pid = Int32(CommandLine.arguments[1]), let app = NSRunningApplication(processIdentifier: pid) { print(app.activationPolicy.rawValue) }\n')
            helper = Path(temporary)/'policy-helper'
            subprocess.run(['swiftc',str(source),'-o',str(helper)],check=True,capture_output=True,timeout=60)
        wait_policy('0')
        decoy = Path(temporary)/'AgentIsland.app'
        shutil.copytree(bundle,decoy,symlinks=True)
        subprocess.run(['codesign','--verify','--strict',str(decoy)],check=True,capture_output=True,timeout=15)
        try:
            subprocess.run([REGISTER,'-f',str(decoy)],check=True,capture_output=True,timeout=15)
            for cli in (binary,bundle/'Contents/Helpers/agentisland'):
                subprocess.run([str(cli),'open','workbench-hide'],check=True,capture_output=True,timeout=15)
                wait_policy('1')
                subprocess.run([str(cli),'open','workbench'],check=True,capture_output=True,timeout=15)
                wait_policy('0')
        finally:
            try:
                subprocess.run([REGISTER,'-u',str(decoy)],check=True,capture_output=True,timeout=15)
            finally:
                subprocess.run([REGISTER,'-f',str(bundle)],check=True,capture_output=True,timeout=15)
    print('PASS: packaged CLI reaches the same native instance with another registered bundle present')


if __name__ == '__main__':
    try:
        main()
    except (OSError, ValueError, subprocess.SubprocessError, KeyError):
        print('Native CLI delivery incomplete; no delivery verdict.',file=sys.stderr)
        sys.exit(2)
