#!/usr/bin/env python3
"""Run native gates inside an existing, authenticated Tart macOS VM.

No provisioning, passwords, host UI automation, or implicit VM creation.
Artifacts and SSH host-key pins stay under ignored .build/vm/.
"""
import argparse
import hashlib
import ipaddress
import json
import os
from pathlib import Path
import re
import shlex
import subprocess
import tarfile
import uuid

ROOT = Path(__file__).resolve().parent.parent
SCRIPTS = ['ui-smoke.sh', 'test-dock-presence.py', 'test-app-instance.py',
           'measure-memory.py', 'memory-soak.py', 'test-native-motion.py', 'test-cli-native.py']


def vm_running(rows, name):
    return any(row.get('Name') == name and row.get('Source') == 'local'
               and row.get('Running') is True and row.get('State') == 'running'
               for row in rows)


def private_address(value):
    address = ipaddress.ip_address(value)
    if not address.is_private or address.is_loopback or address.is_unspecified:
        raise ValueError('VM address must be a private, non-loopback address')
    return str(address)


def resolve_address(vm):
    # DHCP leases can lag after restoring an existing VM; ARP is Tart's
    # supported fallback. SSH host-key pinning and guest proof still apply.
    for resolver in ('dhcp', 'arp'):
        result = subprocess.run(['tart', 'ip', vm, '--resolver', resolver, '--wait', '15'],
                                capture_output=True, text=True, timeout=25)
        if result.returncode == 0:
            return private_address(result.stdout.strip())
    raise ValueError('Running VM has no address from DHCP or ARP')


def guest_verified(meta):
    return (meta.get('platform') == 'Darwin' and meta.get('virtualized') == '1'
            and isinstance(meta.get('console_uid'), int) and meta['console_uid'] >= 500
            and meta.get('console_uid') == meta.get('uid')
            and meta.get('python_ready') is True and meta.get('console_unlocked') is True)


def run(args, **options):
    return subprocess.run(args, check=True, timeout=options.pop('timeout', 30), **options)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--vm', required=True)
    parser.add_argument('--user', required=not bool(os.environ.get('NATIVE_TEST_USER')),
                        default=os.environ.get('NATIVE_TEST_USER'))
    parser.add_argument('--check-only', action='store_true')
    parser.add_argument('--motion-only', action='store_true',
                        help='Run the visible synthetic four-edge motion gate instead of the default release gates')
    parser.add_argument('--identity', default=os.environ.get('NATIVE_TEST_IDENTITY'),
                        help='Existing SSH identity path; never copied into the guest or evidence')
    args = parser.parse_args()
    if not re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9_.-]{0,63}', args.vm):
        parser.error('Invalid VM name')
    if not re.fullmatch(r'[A-Za-z_][A-Za-z0-9_.-]{0,31}', args.user):
        parser.error('Invalid guest username')
    rows = json.loads(run(['tart', 'list', '--format', 'json'], capture_output=True, text=True).stdout)
    if not vm_running(rows, args.vm):
        raise ValueError('Selected local VM is not running; start its existing instance first')
    ip = resolve_address(args.vm)
    base = ROOT / '.build/vm'; base.mkdir(parents=True, exist_ok=True)
    ssh = ['ssh', '-o', 'BatchMode=yes', '-o', 'ConnectTimeout=10',
           '-o', 'StrictHostKeyChecking=accept-new', '-o', f'UserKnownHostsFile={base / "known_hosts"}',
           f'{args.user}@{ip}']
    if args.identity:
        ssh[1:1] = ['-i', str(Path(args.identity).expanduser()), '-o', 'IdentitiesOnly=yes']
    probe = '''import json,os,subprocess,sys,plistlib
from pathlib import Path
v=subprocess.run(['sysctl','-n','kern.hv_vmm_present'],capture_output=True,text=True)
console=plistlib.loads(subprocess.check_output(['ioreg','-n','Root','-d1','-a']))
print(json.dumps({'platform':subprocess.check_output(['uname','-s'],text=True).strip(),
'virtualized':v.stdout.strip(),'console_uid':os.stat('/dev/console').st_uid,'uid':os.getuid(),
'python_ready':True,'console_unlocked':console.get('IOConsoleLocked') is False,'home':str(Path.home()),'python':sys.executable,
'os_build':subprocess.check_output(['sw_vers','-buildVersion'],text=True).strip()}))'''
    # Existing user runtimes only; system Python may trigger an interactive CLT install.
    probe_command = ('for p in "$HOME/miniconda3/bin/python3" "$HOME/miniconda/bin/python3"; do '
                     'if [ -x "$p" ]; then exec "$p" -c ' + shlex.quote(probe) + '; fi; done; exit 2')
    meta = json.loads(run(ssh + [probe_command],
                          capture_output=True, text=True).stdout)
    if not guest_verified(meta):
        raise ValueError('Guest virtualization, logged-in Aqua session, or Python proof failed')
    python = shlex.quote(meta['python'])
    host_build = run(['sw_vers', '-buildVersion'], capture_output=True, text=True).stdout.strip()
    if meta.get('os_build') != host_build:
        raise ValueError('Guest OS build differs from host; native acceptance requires matching builds')
    print('Verified macOS virtualized Aqua session and pinned SSH connection.', flush=True)
    if args.check_only:
        return
    app = ROOT / 'dist/AgentIsland.app'
    binary = app / 'Contents/MacOS/agentisland'
    if not binary.is_file():
        raise ValueError('Build the application before native acceptance')
    run(['codesign', '--verify', '--strict', str(app)], capture_output=True)
    evidence = base / ('acceptance-' + uuid.uuid4().hex)
    evidence.mkdir()
    source = evidence / 'policy.swift'; helper = evidence / 'policy-helper'
    source.write_text('''import AppKit
if let pid = Int32(CommandLine.arguments[1]), let app = NSRunningApplication(processIdentifier: pid) {
    print(app.activationPolicy.rawValue)
}
''')
    run(['swiftc', str(source), '-o', str(helper)], capture_output=True, timeout=120)
    files = {f'scripts/{name}': ROOT / 'scripts' / name for name in SCRIPTS}
    files['.build/policy-helper'] = helper
    files['dist/AgentIsland.app/Contents/MacOS/agentisland'] = binary
    manifest = {name: hashlib.sha256(path.read_bytes()).hexdigest() for name, path in files.items()}
    (evidence / 'manifest.json').write_text(json.dumps(manifest))
    archive = evidence / 'payload.tar'
    with tarfile.open(archive, 'w') as bundle:
        bundle.add(app, arcname='dist/AgentIsland.app')
        bundle.add(ROOT / 'app/ui', arcname='app/ui')
        for name, path in files.items():
            if name.startswith('scripts/') or name.startswith('.build/'):
                bundle.add(path, arcname=name)
        bundle.add(evidence / 'manifest.json', arcname='manifest.json')
    target = meta['home'] + '/agentisland-native-' + evidence.name
    quoted = shlex.quote(target)
    with archive.open('rb') as stream:
        run(ssh + [f'mkdir -m 700 {quoted} && tar -xf - -C {quoted}'], stdin=stream, timeout=180)
    guest = '''import hashlib,json,os,subprocess,sys,plistlib
from pathlib import Path
root=Path.cwd()
console=plistlib.loads(subprocess.check_output(['ioreg','-n','Root','-d1','-a']))
if console.get('IOConsoleLocked') is not False:
    raise SystemExit('Native acceptance requires an unlocked console')
lock=Path.home()/'.agentisland-native-gates.lock'
try:
    lock.mkdir(mode=0o700)
except FileExistsError:
    raise SystemExit('Native acceptance lock already exists; inspect the guest before retrying')
import atexit
(lock/'owner.json').write_text(json.dumps({'pid':os.getpid()}))
def unlock():
    (lock/'owner.json').unlink()
    lock.rmdir()
atexit.register(unlock)
awake=subprocess.Popen(['caffeinate','-diu','-t','600'],stdin=subprocess.DEVNULL,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
def release_awake():
    awake.terminate()
    awake.wait(timeout=5)
atexit.register(release_awake)
for name,expected in json.loads((root/'manifest.json').read_text()).items():
    assert hashlib.sha256((root/name).read_bytes()).hexdigest()==expected, 'Artifact identity mismatch'
subprocess.run(['codesign','--verify','--strict','dist/AgentIsland.app'],check=True)
env=dict(os.environ,UI_SMOKE_BIN=str(root/'dist/AgentIsland.app/Contents/MacOS/agentisland'),POLICY_HELPER=str(root/'.build/policy-helper'))
commands=[['bash','scripts/ui-smoke.sh','--isolated-session'],[sys.executable,'scripts/test-dock-presence.py','--isolated-session'],[sys.executable,'scripts/test-app-instance.py','--cold'],[sys.executable,'scripts/test-cli-native.py']]
if MOTION_ONLY:
    commands=[[sys.executable,'scripts/test-native-motion.py','--binary',str(root/'dist/AgentIsland.app/Contents/MacOS/agentisland')]]
for command in commands:
    print('GATE '+Path(command[1]).name,flush=True)
    subprocess.run(command,env=env,check=True,timeout=360 if MOTION_ONLY else 180)
print('ALL GATES PASS',flush=True)
'''.replace('MOTION_ONLY', repr(args.motion_only))
    with (evidence / 'gates.log').open('w') as log:
        result = subprocess.run(ssh + [f'cd {quoted} && {python} -c {shlex.quote(guest)}'],
                                stdout=log, stderr=subprocess.STDOUT, timeout=600)
    if args.motion_only:
        for edge in ('top', 'bottom', 'left', 'right'):
            artifact = quoted + '/.build/motion/' + edge + '.json'
            captured = subprocess.run(ssh + ['cat ' + artifact], capture_output=True, text=True, timeout=15)
            if captured.returncode == 0:
                data = json.loads(captured.stdout)
                (evidence / f'motion-{edge}.json').write_text(json.dumps(data))
    receipt = {'schema_version': 1, 'vm': args.vm, 'manifest': manifest,
               'gates': ['motion-four-edges'] if args.motion_only else ['ui', 'dock', 'single-instance', 'cli-native'],
               'guest_verified': True, 'console_unlocked': True, 'os_build': host_build, 'exit_code': result.returncode,
               'passed': result.returncode == 0 and 'ALL GATES PASS' in (evidence / 'gates.log').read_text()}
    (evidence / 'receipt.json').write_text(json.dumps(receipt, indent=2))
    print('Native gate evidence:', evidence.relative_to(ROOT), flush=True)
    if not receipt['passed']:
        raise ValueError('Native gates failed; inspect the ignored gate log')
    print('All native gates passed against transferred artifact hashes.')


if __name__ == '__main__':
    try:
        main()
    except (ValueError, OSError, subprocess.SubprocessError, json.JSONDecodeError) as error:
        print('Native VM acceptance failed:', str(error) if isinstance(error, ValueError) else type(error).__name__, file=__import__('sys').stderr)
        raise SystemExit(2)
