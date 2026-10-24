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
SCRATCH = ROOT / '.build'
# `.build/` is git-ignored, so it does not exist in a fresh checkout. Callers that
# stage temporary payloads there must not assume it is already present: on a clean
# tree `tempfile.TemporaryDirectory(dir=SCRATCH)` fails with FileNotFoundError before
# any test runs, which reads as a broken gate rather than a missing directory.
SCRATCH.mkdir(parents=True, exist_ok=True)
SCRIPTS = ['ui-smoke.sh', 'test-dock-presence.py', 'test-app-instance.py',
           'measure-memory.py', 'memory-soak.py', 'test-native-motion.py', 'test-cli-native.py',
           'resource-soak.py', 'analyze-memory-soak.py']


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



def resource_failure_hint(log):
    """Only surface known diagnostic codes; never echo arbitrary remote output."""
    hints = {
        'VM_CONSOLE_REQUIRED': '来宾需要已登录且解锁的匹配用户桌面',
        'VM_PROBE_FAILED': '来宾虚拟化或控制台探测失败',
        'BUNDLE_INVALID': '正式应用包不可读或不完整',
        'SIGNATURE_INVALID': '正式应用包签名验证失败',
        'PROCESS_ENUMERATION_FAILED': '来宾进程枚举失败',
        'EXISTING_APPLICATION': '来宾已有 AgentIsland 实例，请先退出该实例再跑资源长测',
        'OUTPUT_CREATE_FAILED': '资源证据目录无法创建（可能已存在或无权限）',
        'LEASE_EXISTS': '来宾存在验收独占锁，须核实其所有者后处理',
        'LEASE_CREATE_FAILED': '来宾验收独占锁无法创建',
        'PREFLIGHT_FAILED': '资源前置检查失败，尚无协议或清理通过证明',
    }
    if log.stat().st_size > 1024 * 1024:
        return 'Resource gate failed; evidence retained'
    for line in log.read_text().splitlines():
        try:
            record = json.loads(line)
        except ValueError:
            continue
        if (isinstance(record, dict) and record.get('stage') == 'preflight'
                and record.get('protocol_complete') is False
                and record.get('budget_assessed') is False
                and isinstance(record.get('error_code'), str)
                and record['error_code'] in hints):
            code = record['error_code']
            return 'Resource gate failed [' + code + ']: ' + hints[code]
    return 'Resource gate failed; evidence retained'


def run_resource(ssh, python, quoted, evidence, manifest):
    """Keep failed evidence too; only the strict analyzer may mark this gate passed."""
    verify = "import hashlib,json;from pathlib import Path;root=Path.cwd();manifest=json.loads((root/'manifest.json').read_text());assert all(hashlib.sha256((root/name).read_bytes()).hexdigest()==value for name,value in manifest.items())"
    command = (f'cd {quoted} && {python} -c {shlex.quote(verify)} && {python} scripts/resource-soak.py '
               '--bundle dist/AgentIsland.app --output .build/resource')
    transport_error = None
    with (evidence / 'gates.log').open('w') as log:
        try:
            result = subprocess.run(ssh + [command], stdout=log, stderr=subprocess.STDOUT, timeout=6600)
        except (subprocess.TimeoutExpired, OSError) as error:
            transport_error = type(error).__name__
            result = subprocess.CompletedProcess(ssh, 2)
            (evidence / 'resource-transport-failure.json').write_text(json.dumps(dict(
                passed=False, error_type=transport_error, cleanup_verified=False)))
    capture = r"""import hashlib,importlib.util,json,os,sys
from pathlib import Path
root=Path.cwd();output=root/'.build/resource';files={}
limits={'manifest.json':32768,'baseline.json':32768,'running.json':4096,'target.json':4096,
        'samples.jsonl':1048576,'environment.jsonl':2097152,'receipt.json':4096,'analysis.json':32768}
for name,limit in limits.items():
 path=output/name
 if path.exists():
  if path.is_symlink() or not path.is_file() or path.stat().st_size>limit:raise ValueError('Invalid evidence file')
  body=path.read_bytes();files[name]={'body':body.decode(),'sha256':hashlib.sha256(body).hexdigest()}
postcheck={}
if all((output/name).is_file() for name in ('receipt.json','running.json','target.json','baseline.json')):
 spec=importlib.util.spec_from_file_location('resource',root/'scripts/resource-soak.py')
 resource=importlib.util.module_from_spec(spec);spec.loader.exec_module(resource)
 native=resource.soak.measure.Native();running=json.loads((output/'running.json').read_text());target=json.loads((output/'target.json').read_text())
 postcheck={'runner_absent':native.is_absent(running['pid']),'awake_absent':native.is_absent(running['awake_pid']),
            'target_absent':native.is_absent(target['pid']),'lock_absent':not (Path.home()/'.agentisland-native-gates.lock').exists(),
            'baseline_unchanged':resource.baseline_matches(output/'home',json.loads((output/'baseline.json').read_text()))}
print(json.dumps({'files':files,'postcheck':postcheck}))
"""
    failure_hint = resource_failure_hint(evidence / 'gates.log') if result.returncode != 0 else None
    allowed = {'manifest.json', 'baseline.json', 'running.json', 'target.json', 'samples.jsonl',
               'environment.jsonl', 'receipt.json', 'analysis.json'}
    try:
        captured = run(ssh + [f'cd {quoted} && {python} -c {shlex.quote(capture)}'],
                       capture_output=True, text=True)
        captured_data = json.loads(captured.stdout)
        if (not isinstance(captured_data, dict)
                or not isinstance(captured_data.get('files'), dict)
                or not isinstance(captured_data.get('postcheck'), dict)):
            raise ValueError('Invalid capture structure')
        files = captured_data['files']
        postcheck = captured_data['postcheck']
        for name, record in files.items():
            if (name not in allowed or not isinstance(record, dict)
                    or not isinstance(record.get('body'), str)
                    or not isinstance(record.get('sha256'), str)
                    or hashlib.sha256(record['body'].encode()).hexdigest() != record['sha256']):
                raise ValueError('Invalid capture record')
    except (subprocess.SubprocessError, OSError, ValueError):
        (evidence / 'resource-capture-failure.json').write_text(json.dumps(dict(
            passed=False, error_code='EVIDENCE_CAPTURE_FAILED', cleanup_verified=False)))
        raise ValueError((failure_hint or 'Resource evidence unavailable')
                         + '; EVIDENCE_CAPTURE_FAILED（证据回收失败，清理未核实）')
    (evidence / 'resource-postcheck.json').write_text(json.dumps(postcheck))
    hashes = {}
    for name, record in files.items():
        (evidence / ('resource-' + name)).write_text(record['body'])
        hashes[name] = record['sha256']
    (evidence / 'resource-hashes.json').write_text(json.dumps(hashes))
    if result.returncode != 0:
        raise ValueError(resource_failure_hint(evidence / 'gates.log'))
    if set(files) != allowed:
        raise ValueError('Resource gate evidence missing')
    checks = ('runner_absent', 'awake_absent', 'target_absent', 'lock_absent', 'baseline_unchanged')
    if any(postcheck.get(name) is not True for name in checks):
        raise ValueError('Resource independent cleanup check failed')
    resource_manifest = json.loads(files['manifest.json']['body'])
    if set(resource_manifest['files']) != {'resource-soak.py', 'memory-soak.py', 'measure-memory.py', 'analyze-memory-soak.py'}:
        raise ValueError('Resource input manifest is incomplete')
    if resource_manifest['binary_sha256'] != manifest['dist/AgentIsland.app/Contents/MacOS/agentisland'] or any(
            manifest['scripts/' + name] != value for name, value in resource_manifest['files'].items()):
        raise ValueError('Resource run differs from transferred input hashes')
    import importlib.util
    spec = importlib.util.spec_from_file_location('resource_analysis', ROOT / 'scripts/analyze-memory-soak.py')
    analyzer = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(analyzer)
    rows = [json.loads(line) for line in files['samples.jsonl']['body'].splitlines()]
    receipt = json.loads(files['receipt.json']['body'])
    analysis = analyzer.analyze(rows, receipt, require_cleanup=True)
    target = json.loads(files['target.json']['body'])
    if type(target.get('pid')) is not int or type(target.get('start')) is not int:
        raise ValueError('Resource target identity is invalid')
    for row in rows[:-1]:
        observation = row['observation']
        main = next(member for member in observation['members'] if member['role'] == 'main')
        if (main['pid'], observation['main_start']) != (target['pid'], target['start']):
            raise ValueError('Resource samples differ from recorded target identity')
    if receipt['run_id'] != resource_manifest['run_id']:
        raise ValueError('Resource receipt belongs to another run')
    if analysis != json.loads(files['analysis.json']['body']):
        raise ValueError('Resource analysis differs from independent host check')
    (evidence / 'receipt.json').write_text(json.dumps(dict(
        schema_version=1, gates=['resource-soak'], manifest=manifest, exit_code=0,
        passed=True, analysis=analysis, budget_assessed=False, resource_hashes=hashes), indent=2))
    print('Strict resource protocol and cleanup passed:', evidence.relative_to(ROOT), flush=True)

def verified_prior_acceptance(name, vm, base):
    if not re.fullmatch(r'acceptance-[0-9a-f]{32}', name):
        raise ValueError('Invalid prior acceptance name')
    directory = base / name
    receipt = json.loads((directory / 'receipt.json').read_text())
    manifest = json.loads((directory / 'manifest.json').read_text())
    if (receipt.get('passed') is not True or type(receipt.get('exit_code')) is not int
            or receipt['exit_code'] != 0 or receipt.get('vm') != vm
            or receipt.get('guest_verified') is not True or receipt.get('console_unlocked') is not True
            or receipt.get('gates') != ['ui', 'dock', 'single-instance', 'cli-native', 'window-lifetime']
            or receipt.get('manifest') != manifest):
        raise ValueError('Prior acceptance receipt is not verified for this VM')
    digest = manifest.get('dist/AgentIsland.app/Contents/MacOS/agentisland')
    if not isinstance(digest, str) or not re.fullmatch(r'[0-9a-f]{64}', digest):
        raise ValueError('Prior acceptance binary hash is invalid')
    return digest


def retire_prior_acceptance(ssh, python, quoted, evidence, home, name, expected_hash):
    code = '''import hashlib,importlib.util,json
from pathlib import Path
root=Path.cwd()
manifest=json.loads((root/'manifest.json').read_text())
if any(hashlib.sha256((root/name).read_bytes()).hexdigest()!=value for name,value in manifest.items()):
 raise ValueError('Transferred inputs changed')
spec=importlib.util.spec_from_file_location('resource',root/'scripts/resource-soak.py')
resource=importlib.util.module_from_spec(spec);spec.loader.exec_module(resource)
if not resource.console_ready():raise ValueError('Console unavailable')
lock=Path.home()/'.agentisland-native-gates.lock'
lock.mkdir(mode=0o700)
try:
 binary=Path(PRIOR)/'dist/AgentIsland.app/Contents/MacOS/agentisland'
 count=resource.retire_verified(resource.soak.measure.Native(),binary,EXPECTED)
 print(json.dumps({'retired':True,'count':count}))
finally:
 lock.rmdir()
'''.replace('PRIOR', repr(home + '/agentisland-native-' + name)).replace('EXPECTED', repr(expected_hash))
    with (evidence / 'prior-cleanup.log').open('w') as log:
        result = subprocess.run(ssh + [f'cd {quoted} && {python} -c {shlex.quote(code)}'],
                                stdout=log, stderr=subprocess.DEVNULL, timeout=30)
    if result.returncode != 0:
        raise ValueError('Prior acceptance cleanup refused; no resource run started')
    record = json.loads((evidence / 'prior-cleanup.log').read_text())
    if record.get('retired') is not True or type(record.get('count')) is not int or record['count'] not in (0, 1):
        raise ValueError('Prior acceptance cleanup proof is invalid')


def reset_guest_acceptance(ssh, python, home, evidence):
    """终止**只属于本工具**的残留验收实例，否则 open 会投递给旧 app。

    BUG-007：门禁只 launch 新 bundle，但 `open` 按 bundle id 投递；上一轮中断留下的
    实例仍在运行时，请求全部进了旧 app，新实例根本不接——同一产物重跑**稳定失败**于
    helper-hide-delivery，看起来像产品缺陷。

    判据只认本工具自己的路径前缀（`<home>/agentisland-native-acceptance-*`）：
    不按进程名杀、不碰用户自己跑的 AgentIsland，也不碰其他任何 app。杀完必须复核消失，
    否则拒绝继续——「以为清理了」比不清理更危险。
    """
    code = '''import json,os,signal,subprocess,time
from pathlib import Path
home=Path(HOME).resolve()
prefix=str(home)+'/agentisland-native-acceptance-'
def instances():
    # 必须用 args（完整命令行）：command/comm 会被截断成路径前缀，截断后既匹配不到验收实例，也可能误判。
    out=subprocess.run(['ps','-axo','pid=,args='],capture_output=True,text=True,timeout=5).stdout
    found=[]
    for line in out.splitlines():
        pid,_,command=line.strip().partition(' ')
        command=command.strip()
        # 判据是**路径前缀**：本工具把验收产物解到 <home>/agentisland-native-acceptance-<hex>/，
        # 只要命令行里出现这个前缀就归本工具所有；用户自己的 app 不在这个前缀下。
        if prefix not in command:continue
        if 'AgentIsland.app' not in command:continue
        if not pid.isdigit():continue
        found.append((int(pid),command))
    return found
stopped=[]
for pid,command in instances():
    os.kill(pid,signal.SIGTERM)
    stopped.append(pid)
deadline=time.time()+10
while time.time()<deadline and instances():
    time.sleep(.1)
remaining=[pid for pid,_ in instances()]
# 只杀进程不够：历轮解包目录会被 LaunchServices 记住，`open` 按 bundle id 解析时
# 可能落到一个**已不存在或未运行**的副本上，URL 投递于是落空——症状与 BUG-007 相同。
# 因此把这些只属于本工具的旧目录先注销再删除（判据仍是同一路径前缀）。
register='/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister'
stale=sorted(p for p in home.iterdir() if p.is_dir() and p.name.startswith('agentisland-native-acceptance-'))
unregistered=0
for path in stale:
    bundle=path/'dist/AgentIsland.app'
    if bundle.is_dir():
        subprocess.run([register,'-u',str(bundle)],capture_output=True,timeout=30)
        unregistered+=1
    subprocess.run(['rm','-rf','--',str(path)],capture_output=True,timeout=60)
removed=[p for p in stale if p.exists()]
print(json.dumps({'stopped':len(stopped),'remaining':len(remaining),
                  'unregistered':unregistered,'removed':len(stale)-len(removed),'not_removed':len(removed)}))
'''.replace('HOME', repr(home))
    with (evidence / 'guest-reset.log').open('w') as log:
        result = subprocess.run(ssh + [f'{python} -c {shlex.quote(code)}'],
                                stdout=log, stderr=subprocess.STDOUT, timeout=60)
    if result.returncode != 0:
        raise ValueError('Guest acceptance reset failed; refusing to run native gates')
    record = json.loads((evidence / 'guest-reset.log').read_text())
    if (type(record.get('stopped')) is not int or record.get('remaining') != 0
            or record.get('not_removed') != 0):
        raise ValueError('Guest acceptance reset did not prove every stale instance is gone')
    if record['stopped'] or record['removed']:
        print(f'Guest reset: stopped {record["stopped"]} stale instance(s), '
              f'unregistered {record["unregistered"]} and removed {record["removed"]} stale acceptance '
              f'director{"y" if record["removed"] == 1 else "ies"}.', flush=True)


def main():

    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--vm', required=True)
    parser.add_argument('--user', required=not bool(os.environ.get('NATIVE_TEST_USER')),
                        default=os.environ.get('NATIVE_TEST_USER'))
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument('--check-only', action='store_true')
    mode.add_argument('--resource-only', action='store_true',
                      help='Run the 30-minute/20-cycle synthetic resource protocol with strict cleanup')
    mode.add_argument('--motion-only', action='store_true',
                        help='Run the visible synthetic four-edge motion gate instead of the default release gates')
    parser.add_argument('--retire-acceptance',
                        help='With --resource-only, retire the exact bundle of a verified prior five-gate receipt')
    parser.add_argument('--identity', default=os.environ.get('NATIVE_TEST_IDENTITY'),
                        help='Existing SSH identity path; never copied into the guest or evidence')
    args = parser.parse_args()
    if args.retire_acceptance and not args.resource_only:
        parser.error('--retire-acceptance requires --resource-only')
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
        # 四项任一不满足都落到同一句话，无从定位：把实际读数一并说出。
        # 不含路径、用户名或凭据；probe 失败（例如来宾没有 Miniconda）不会走到这里，
        # 它会在上面那次 ssh 就抛 CalledProcessError。
        raise ValueError('Guest virtualization, logged-in Aqua session, or Python proof failed: '
                         + json.dumps({key: meta.get(key) for key in (
                             'platform', 'virtualized', 'console_uid', 'uid',
                             'python_ready', 'console_unlocked')}, sort_keys=True)
                         + '（virtualized 需为 "1"；console_uid 需 ≥500 且等于 uid；'
                           'python_ready 与 console_unlocked 需为 true）')
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
    # 先复位来宾：残留实例会让 open 把请求投给旧 app（BUG-007）。
    reset_guest_acceptance(ssh, python, meta['home'], evidence)
    source = evidence / 'policy.swift'; helper = evidence / 'policy-helper'
    source.write_text('''import AppKit
if let pid = Int32(CommandLine.arguments[1]), let app = NSRunningApplication(processIdentifier: pid) {
    print(app.activationPolicy.rawValue)
}
''')
    run(['swiftc', str(source), '-o', str(helper)], capture_output=True, timeout=120)
    lifetime_target = ROOT / '.build/window-lifetime-target'
    # 探针 crate vendor 了 tao，**每次版本号 bump 都全量重编**（agentisland 版本进了
    # 它的依赖闭包）——BUG-013 两次 TimeoutExpired 的根因都是这里 180 秒不够。
    # 这是基础设施超时不是门禁判据：判据是后面的五道原生检查，构建慢不该误伤发布。
    run(['cargo', 'build', '--locked', '--quiet', '--manifest-path',
         str(ROOT / 'scripts/window-lifetime-probe/Cargo.toml')],
        env=dict(os.environ, CARGO_TARGET_DIR=str(lifetime_target)), timeout=900)
    lifetime_helper = evidence / 'window-lifetime-probe'
    lifetime_helper.write_bytes((lifetime_target / 'debug/agentisland-window-lifetime-probe').read_bytes())
    lifetime_helper.chmod(0o700)
    run(['codesign', '--force', '--sign', '-', str(lifetime_helper)], capture_output=True)
    files = {f'scripts/{name}': ROOT / 'scripts' / name for name in SCRIPTS}
    files['.build/policy-helper'] = helper
    files['.build/window-lifetime-probe'] = lifetime_helper
    for name in ['scripts/window-lifetime-probe/Cargo.toml',
                 'scripts/window-lifetime-probe/Cargo.lock',
                 'scripts/window-lifetime-probe/src/main.rs',
                 'vendor/tao/src/platform_impl/macos/window.rs',
                 'vendor/tao/src/platform_impl/macos/view.rs',
                 'vendor/tao/src/platform_impl/macos/window_delegate.rs',
                 'app/src-tauri/Cargo.toml', 'app/src-tauri/Cargo.lock']:
        files[name] = ROOT / name
    files['dist/AgentIsland.app/Contents/MacOS/agentisland'] = binary
    manifest = {name: hashlib.sha256(path.read_bytes()).hexdigest() for name, path in files.items()}
    (evidence / 'manifest.json').write_text(json.dumps(manifest))
    archive = evidence / 'payload.tar'
    with tarfile.open(archive, 'w') as bundle:
        bundle.add(app, arcname='dist/AgentIsland.app')
        bundle.add(ROOT / 'app/ui', arcname='app/ui')
        for name, path in files.items():
            if name != 'dist/AgentIsland.app/Contents/MacOS/agentisland':
                bundle.add(path, arcname=name)
        bundle.add(evidence / 'manifest.json', arcname='manifest.json')
    target = meta['home'] + '/agentisland-native-' + evidence.name
    quoted = shlex.quote(target)
    with archive.open('rb') as stream:
        run(ssh + [f'mkdir -m 700 {quoted} && tar -xf - -C {quoted}'], stdin=stream, timeout=180)
    if args.resource_only:
        if args.retire_acceptance:
            expected = verified_prior_acceptance(args.retire_acceptance, args.vm, base)
            retire_prior_acceptance(ssh, python, quoted, evidence, meta['home'], args.retire_acceptance, expected)
        run_resource(ssh, python, quoted, evidence, manifest)
        return
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
subprocess.run(['codesign','--verify','--strict','.build/window-lifetime-probe'],check=True)
env=dict(os.environ,UI_SMOKE_BIN=str(root/'dist/AgentIsland.app/Contents/MacOS/agentisland'),POLICY_HELPER=str(root/'.build/policy-helper'))
commands=[['bash','scripts/ui-smoke.sh','--isolated-session'],[sys.executable,'scripts/test-dock-presence.py','--isolated-session'],[sys.executable,'scripts/test-app-instance.py','--cold'],[sys.executable,'scripts/test-cli-native.py'],['.build/window-lifetime-probe']]
if MOTION_ONLY:
    commands=[[sys.executable,'scripts/test-native-motion.py','--binary',str(root/'dist/AgentIsland.app/Contents/MacOS/agentisland')]]
for command in commands:
    print('GATE '+Path(command[1] if len(command)>1 else command[0]).name,flush=True)
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
               'gates': ['motion-four-edges'] if args.motion_only else ['ui', 'dock', 'single-instance', 'cli-native', 'window-lifetime'],
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
