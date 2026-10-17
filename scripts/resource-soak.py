#!/usr/bin/env python3
"""Run the canonical long resource protocol in an exclusive macOS VM.

Creates a unique non-demo synthetic HOME and owns only its copied formal bundle.
No real sessions, credentials, external services, or automatic budget verdicts.
"""
import argparse
from contextlib import contextmanager
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import plistlib
import signal
import subprocess
import sys
import tempfile
import time
import uuid


# 前置拒绝发生在 receipt 事务之前，没有协议回执可留。但它们必须**可辨认**：
# 否则「来宾里已有实例」「上一个租约没清」「签名不符」「控制台没登录」全都退化成
# 同一句「Resource run failed」，人就只能猜（v0.0.323 实际发生过）。
# 只发固定码与白名单计数，不带异常正文、路径、环境或凭据。
PREFLIGHT_CODES = (
    'VM_CONSOLE_REQUIRED', 'VM_PROBE_FAILED', 'BUNDLE_INVALID', 'SIGNATURE_INVALID',
    'EXISTING_APPLICATION', 'PROCESS_ENUMERATION_FAILED', 'OUTPUT_CREATE_FAILED',
    'LEASE_EXISTS', 'LEASE_CREATE_FAILED',
)
PREFLIGHT_FIELDS = ('existing_count',)


class Preflight(Exception):
    def __init__(self, code, **fields):
        if code not in PREFLIGHT_CODES or not set(fields) <= set(PREFLIGHT_FIELDS):
            raise ValueError('Invalid preflight diagnostic')
        if 'existing_count' in fields and (type(fields['existing_count']) is not int
                                           or fields['existing_count'] <= 0):
            raise ValueError('Invalid preflight count')
        self.code = code
        self.diagnostic = {name: value for name, value in fields.items()}
        super().__init__(code)


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


soak = module('soak', Path(__file__).with_name('memory-soak.py'))
analyzer = module('analyzer', Path(__file__).with_name('analyze-memory-soak.py'))


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def baseline_matches(home, baseline):
    home = home.resolve(strict=True)
    return all(not (home / name).is_symlink() and (home / name).is_file()
               and (home / name).resolve().is_relative_to(home)
               and digest(home / name) == value for name, value in baseline.items())


def finish_code(protocol_complete, cleanup):
    # Fail closed even when sampling completed before a cleanup failure.
    return 0 if protocol_complete and all(cleanup.get(key) is True for key in analyzer.CLEANUP_FIELDS) else 2


CANCEL_SIGNALS = (signal.SIGINT, signal.SIGTERM, signal.SIGHUP)


@contextmanager
def cleanup_signals():
    # No cancellation signal may interrupt the lock/receipt transaction.
    previous = {sig: signal.getsignal(sig) for sig in CANCEL_SIGNALS}
    try:
        for sig in CANCEL_SIGNALS:
            signal.signal(sig, signal.SIG_IGN)
        yield
    finally:
        for sig, handler in previous.items():
            signal.signal(sig, handler)


def stop_owned(native, pid, binary, start, sleep=time.sleep):
    if native.is_absent(pid):
        return
    if not soak.identity_matches(native, pid, binary, start):
        raise ValueError('Owned application identity changed; refusing to signal')
    os.kill(pid, signal.SIGTERM)
    for _ in range(100):
        if native.is_absent(pid):
            return
        sleep(.1)
    raise ValueError('Owned application did not stop')


def retire_verified(native, binary, expected_hash):
    """Stop only a verified prior acceptance bundle, never other running apps."""
    if any(path.is_symlink() for path in (binary, *binary.parents)):
        raise ValueError('Prior acceptance path contains a link')
    binary = binary.resolve(strict=True)
    if digest(binary) != expected_hash:
        raise ValueError('Prior acceptance binary changed')
    applications = [(pid, Path(command)) for pid, command in native.processes()
                    if command.endswith('/AgentIsland.app/Contents/MacOS/agentisland')]
    if any(path != binary for _, path in applications):
        raise ValueError('Unrelated application present; refusing cleanup')
    if len(applications) > 1:
        raise ValueError('Ambiguous prior acceptance instances')
    for pid, _ in applications:
        usage = native.usage(pid)
        if usage is None:
            raise ValueError('Prior acceptance identity unavailable')
        stop_owned(native, pid, binary, usage['start'])
    if any(Path(command) == binary for _, command in native.processes()):
        raise ValueError('Prior acceptance instance remains')
    return len(applications)


def console_ready():
    console = plistlib.loads(subprocess.check_output(['ioreg', '-n', 'Root', '-d1', '-a'], timeout=8))
    if not isinstance(console, dict):
        raise ValueError('Invalid console probe structure')
    return console.get('IOConsoleLocked') is False and os.stat('/dev/console').st_uid == os.getuid() >= 500


def seed(home):
    config = home / 'Library/Application Support/AgentIsland'
    config.mkdir(parents=True)
    (home / '.codex').mkdir()
    (home / '.claude').mkdir()
    (home / '.codex/config.toml').write_text('model="fixture-model"\n')
    (home / '.claude/settings.json').write_text('{}\n')
    (config / 'settings.json').write_text(json.dumps(dict(
        appearance='light', budget_alert_enabled=False, play_completion_sound=False,
        token_alert_enabled=False, auto_anomalies_alert=False)))
    now = int(time.time() * 1000)
    tasks = [dict(id=str(uuid.uuid4()), title=f'Resource fixture {index + 1:02d}',
                  project_id=None, source=None, current_run_id=None, created_ms=now + index,
                  updated_ms=now + index, archived_ms=None, revision=1) for index in range(16)]
    (config / 'tasks.v1.json').write_text(json.dumps(dict(
        schema_version=1, revision=16, projects=[], tasks=tasks, runs=[],
        attentions=[], artifacts=[], event_receipts=[])))
    return {str(path.relative_to(home)): digest(path) for path in (
        home / '.codex/config.toml', home / '.claude/settings.json', config / 'tasks.v1.json')}


def run(bundle, output):
    if sys.platform != 'darwin':
        raise Preflight('VM_CONSOLE_REQUIRED')
    try:
        ready = (subprocess.check_output(
            ['sysctl', '-n', 'kern.hv_vmm_present'], text=True).strip() == '1'
            and console_ready())
    except (OSError, ValueError, subprocess.SubprocessError):
        raise Preflight('VM_PROBE_FAILED')
    if not ready:
        raise Preflight('VM_CONSOLE_REQUIRED')
    try:
        bundle = bundle.resolve(strict=True)
        source_binary = bundle / 'Contents/MacOS/agentisland'
    except OSError:
        raise Preflight('BUNDLE_INVALID')
    try:
        subprocess.run(['codesign', '--verify', '--deep', '--strict', str(bundle)],
                       check=True, capture_output=True, timeout=30)
    except (subprocess.SubprocessError, OSError):
        raise Preflight('SIGNATURE_INVALID')
    native = soak.measure.Native()
    # 单实例派发要求来宾里没有别的 AgentIsland。最常见来源是刚跑完的五门禁：
    # test-app-instance.py --cold 故意留下一个实例来证明「双启动收敛为一」。
    try:
        processes = native.processes()
    except Exception:
        raise Preflight('PROCESS_ENUMERATION_FAILED')
    occupied = [pid for pid, command in processes
                if command.endswith('/AgentIsland.app/Contents/MacOS/agentisland')]
    if occupied:
        raise Preflight('EXISTING_APPLICATION', existing_count=len(occupied))
    try:
        output.mkdir(mode=0o700)  # A run never overwrites existing evidence.
        output = output.resolve(strict=True)
    except OSError:
        raise Preflight('OUTPUT_CREATE_FAILED')
    lock = Path.home() / '.agentisland-native-gates.lock'
    # Prepare every value needed by finally before acquiring the lease.
    owner = dict(pid=os.getpid(), run_id=uuid.uuid4().hex)
    home = output / 'home'
    binary = output / 'AgentIsland.app/Contents/MacOS/agentisland'
    awake, identity, baseline = None, None, None
    complete, error_type, stage, rows = False, '', 'setup', []
    started = time.monotonic()
    manifest = {}
    cleanup = dict.fromkeys(analyzer.CLEANUP_FIELDS, False)
    try:
        lock.mkdir(mode=0o700)  # Never steal an existing lease.
    except FileExistsError:
        raise Preflight('LEASE_EXISTS')
    except OSError:
        raise Preflight('LEASE_CREATE_FAILED')
    try:
        (lock / 'owner.json').write_text(json.dumps(owner))
        scripts = Path(__file__).resolve().parent
        manifest = dict(schema_version=1, run_id=owner['run_id'],
                        version=plistlib.loads((bundle / 'Contents/Info.plist').read_bytes())['CFBundleShortVersionString'],
                        files={name: digest(scripts / name) for name in (
                            'resource-soak.py', 'memory-soak.py', 'measure-memory.py', 'analyze-memory-soak.py')},
                        binary_sha256=digest(source_binary), workload='16 synthetic local tasks; no live AI execution',
                        demo=False, budget_assessed=False)
        (output / 'manifest.json').write_text(json.dumps(manifest))
        baseline = seed(home)
        (output / 'baseline.json').write_text(json.dumps(baseline))
        subprocess.run(['ditto', str(bundle), str(binary.parents[2])], check=True, timeout=60,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        if digest(binary) != manifest['binary_sha256']:
            raise ValueError('Copied formal binary differs')
        native = soak.measure.Native()
        awake = subprocess.Popen(['caffeinate', '-diu', '-t', '6600'],
                                 stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        (output / 'running.json').write_text(json.dumps(dict(owner, awake_pid=awake.pid)))
        last_env, last_heartbeat = -10, -40
        with (output / 'environment.jsonl').open('x', buffering=1) as environment, \
                (output / 'samples.jsonl').open('x', buffering=1) as stream:
            def guard():
                nonlocal last_env, last_heartbeat
                now = time.monotonic() - started
                if now >= 6300 or awake.poll() is not None or json.loads((lock / 'owner.json').read_text()) != owner:
                    raise ValueError('Resource lease ended or changed')
                if now - last_env >= 2:
                    ready = console_ready()
                    last_env = time.monotonic() - started
                    environment.write(json.dumps(dict(elapsed=last_env, unlocked=ready)) + '\n')
                    if not ready:
                        raise ValueError('Aqua session no longer available')
                if now - last_heartbeat >= 30:
                    progress = dict(elapsed=round(now, 1), samples=len(rows))
                    print(json.dumps(progress), flush=True)
                    last_heartbeat = now

            def sleep(seconds):
                guard()
                time.sleep(min(seconds, 1))

            def emit(row):
                guard()
                rows.append(row)
                stream.write(json.dumps(row) + '\n')

            guard()
            stage = 'launch'
            subprocess.run(['open', '-n', '-a', str(binary.parents[2]), '--env', 'HOME=' + str(home),
                            '--args', '--shell=workbench', '--route=tasks'], check=True,
                           capture_output=True, timeout=15)
            for _ in range(100):
                pids = [pid for pid, command in native.processes() if Path(command) == binary]
                if len(pids) == 1:
                    usage = native.usage(pids[0])
                    if usage is not None:
                        identity = (pids[0], usage['start'])
                        break
                sleep(.1)
            if identity is None:
                raise ValueError('Unique resource target did not start')
            pid, start = identity
            (output / 'target.json').write_text(json.dumps(dict(pid=pid, start=start)))
            args = subprocess.check_output(['ps', '-p', str(pid), '-o', 'args='], text=True, timeout=5)
            env = subprocess.check_output(['ps', 'eww', '-p', str(pid), '-o', 'command='], text=True, timeout=5)
            if '--demo' in args or '--shell=workbench' not in args or 'HOME=' + str(home) not in env:
                raise ValueError('Target does not match isolated workload')
            env = None  # Environment is inspected in memory only, never logged.
            log = Path(tempfile.gettempdir()) / 'agentisland-tauri.log'
            # Require initialization of this task view before hiding it.
            soak.wait_until(time.monotonic() + 6, sleep=sleep)
            stage = 'protocol'
            protocol = soak.Protocol(pid, binary, log, native, emit, sleep=sleep)
            protocol.execute()
            guard()
            complete = rows[-1].get('protocol_complete') is True and len(rows) == 48
            if not soak.identity_matches(native, pid, binary, start):
                raise ValueError('Target replaced after sampling')
            if digest(binary) != manifest['binary_sha256'] or any(digest(scripts / name) != value for name, value in manifest['files'].items()):
                raise ValueError('Measurement inputs changed')
    except (Exception, KeyboardInterrupt) as error:
        complete = False
        error_type = type(error).__name__  # Never emit arbitrary paths, environment, or exception text.
    finally:
        with cleanup_signals():
            # Each cleanup runs even if an earlier cleanup fails. No broad name-based kills.
            try:
                native = soak.measure.Native()
                targets = [(pid, native.usage(pid)) for pid, command in native.processes() if Path(command) == binary]
                for pid, usage in targets:
                    if usage is None or identity is not None and (pid, usage['start']) != identity:
                        raise ValueError('Cleanup target identity changed')
                    stop_owned(native, pid, binary, usage['start'])
                cleanup['owned_target_stopped'] = not any(Path(command) == binary for _, command in native.processes())
            except Exception:
                pass
            try:
                if awake is not None:
                    awake.terminate()
                    awake.wait(timeout=5)
                cleanup['awake_stopped'] = awake is None or awake.poll() is not None
            except Exception:
                pass
            try:
                cleanup['baseline_unchanged'] = baseline is not None and baseline_matches(home, baseline)
            except Exception:
                pass
            try:
                if json.loads((lock / 'owner.json').read_text()) == owner:
                    (lock / 'owner.json').unlink()
                    lock.rmdir()
                cleanup['lock_absent'] = not lock.exists()
            except Exception:
                pass
            code = finish_code(complete, cleanup)
            receipt = dict(schema=1, started=started, finished=time.monotonic(), exit_code=code,
                           protocol_complete=complete, budget_assessed=False, cleanup=cleanup,
                           error_type=error_type, stage=stage, run_id=owner['run_id'])
            if code == 0:
                try:
                    report = analyzer.analyze(rows, receipt, require_cleanup=True)
                    (output / 'analysis.json').write_text(json.dumps(report))
                except Exception as error:
                    code = receipt['exit_code'] = 2
                    receipt['error_type'] = type(error).__name__
            (output / 'receipt.json').write_text(json.dumps(receipt))
            print(json.dumps(receipt), flush=True)
    return code


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--bundle', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    args = parser.parse_args()
    cancelled = False
    def interrupted(signum, frame):
        nonlocal cancelled
        if not cancelled:
            cancelled = True
            raise KeyboardInterrupt()
    previous = {sig: signal.getsignal(sig) for sig in CANCEL_SIGNALS}
    try:
        for sig in CANCEL_SIGNALS:
            signal.signal(sig, interrupted)
        return run(args.bundle, args.output)
    finally:
        for sig, handler in previous.items():
            signal.signal(sig, handler)


def cli():
    try:
        return main()
    except Preflight as error:
        print(json.dumps(dict(stage='preflight', error_code=error.code,
                              protocol_complete=False, budget_assessed=False,
                              **error.diagnostic)), file=sys.stderr, flush=True)
        return 2
    except (OSError, ValueError, subprocess.SubprocessError):
        print(json.dumps(dict(stage='preflight', error_code='PREFLIGHT_FAILED',
                              protocol_complete=False, budget_assessed=False)),
              file=sys.stderr, flush=True)
        return 2


if __name__ == '__main__':
    sys.exit(cli())
