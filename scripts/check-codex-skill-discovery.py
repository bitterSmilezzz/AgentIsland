#!/usr/bin/env python3
"""Verify a real client's local skill discovery inside a disposable macOS fixture.

Only initialize and skills/list are sent. No thread, turn, authentication, server
configuration or external API command is allowed. Raw client output is discarded.
"""
import argparse
import errno
import json
import os
from pathlib import Path
import selectors
import signal
import subprocess
import sys
import tempfile
import time


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--client', required=True)
    parser.add_argument('--fixture', required=True)
    parser.add_argument('--expected', required=True)
    args = parser.parse_args()
    if sys.platform != 'darwin':
        raise RuntimeError('this verifier requires macOS network isolation')
    fixture = Path(args.fixture).resolve(strict=True)
    expected = Path(args.expected).resolve(strict=True)
    client = Path(args.client).resolve(strict=True)
    # Never accept the actual user home or a project checkout as a client fixture.
    if (not fixture.name.startswith('agentisland-skill-client-')
            or fixture.parent != Path(tempfile.gettempdir()).resolve()
            or (fixture / '.agentisland-client-fixture').read_text() != 'fixture-only\n'
            or expected.parent != fixture or not client.is_file()
            or fixture == Path.home().resolve()):
        raise RuntimeError('disposable fixture identity could not be verified')
    wanted = json.loads(expected.read_text())
    def valid_expected(value):
        return (value is None or isinstance(value, str)
                or (isinstance(value, dict) and set(value) == {'description', 'enabled'}
                    and isinstance(value['description'], str)
                    and isinstance(value['enabled'], bool)))
    if not isinstance(wanted, dict) or not wanted or not all(
            isinstance(key, str) and valid_expected(value)
            for key, value in wanted.items()):
        raise RuntimeError('invalid expected fixture metadata')
    work = fixture / 'project'
    work.mkdir(exist_ok=True)
    (fixture / '.codex').mkdir(exist_ok=True)
    (fixture / 'tmp').mkdir(exist_ok=True)
    # Config paths have their genuine meanings in the child only. Never inherit
    # auth, proxies, MCP settings, telemetry credentials or the current project.
    environment = {'HOME': str(fixture), 'CODEX_HOME': str(fixture / '.codex'),
                   'XDG_CONFIG_HOME': str(fixture / '.config'),
                   'TMPDIR': str(fixture / 'tmp'), 'PATH': '/usr/bin:/bin:/usr/sbin:/sbin',
                   'LANG': 'en_US.UTF-8'}
    profile = ('(version 1) (allow default) (deny network*) '
               '(deny file-read* (subpath ' + json.dumps(str(Path.home().resolve())) + ')) '
               '(deny file-write* (require-not (subpath ' + json.dumps(str(fixture)) + ')))')
    isolation = ['/usr/bin/sandbox-exec', '-p', profile]
    # Verify the policy actually denies sockets; a text flag alone is not proof.
    check = subprocess.run(isolation + [sys.executable, '-c',
        'import socket, errno; s=socket.socket(); '
        'assert s.connect_ex(("127.0.0.1",9)) in (errno.EPERM,errno.EACCES)'],
        cwd=work, env=environment, stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL, timeout=5)
    if check.returncode != 0:
        raise RuntimeError('network denial could not be verified')
    process = subprocess.Popen(isolation + [str(client), 'app-server', '--listen', 'stdio://'],
        cwd=work, env=environment, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL, start_new_session=True)
    selector = selectors.DefaultSelector()
    selector.register(process.stdout, selectors.EVENT_READ)
    buffer = bytearray()
    received = 0

    def send(value):
        process.stdin.write(json.dumps(value).encode() + b'\n')
        process.stdin.flush()

    def response(request_id):
        nonlocal received
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline:
            while b'\n' in buffer:
                line, _, rest = buffer.partition(b'\n')
                buffer[:] = rest
                try:
                    value = json.loads(line)
                except (ValueError, UnicodeError):
                    raise RuntimeError('client output was not JSONL') from None
                if value.get('id') == request_id:
                    if 'error' in value or not isinstance(value.get('result'), dict):
                        raise RuntimeError('client rejected the discovery protocol')
                    return value['result']
                if 'id' in value and 'method' in value:
                    raise RuntimeError('unexpected client request; no side effects are authorized')
            if not selector.select(max(0, deadline - time.monotonic())):
                break
            part = os.read(process.stdout.fileno(), 65536)
            if not part:
                raise RuntimeError('client exited before discovery completed')
            received += len(part)
            if received > 2 * 1024 * 1024:
                raise RuntimeError('client output exceeded the bounded fixture limit')
            buffer.extend(part)
        raise RuntimeError('client discovery timed out')

    try:
        send({'id': 1, 'method': 'initialize', 'params': {
            'clientInfo': {'name': 'agentisland_acceptance', 'version': '1.0'},
            'capabilities': {'experimentalApi': False}}})
        response(1)
        send({'method': 'initialized', 'params': {}})
        send({'id': 2, 'method': 'skills/list', 'params': {
            'cwds': [str(work)], 'forceReload': True}})
        result = response(2)
        entries = result.get('data')
        if not isinstance(entries, list) or len(entries) != 1:
            raise RuntimeError('client did not return the requested fixture scope')
        entry = entries[0]
        if Path(entry.get('cwd', '')).resolve() != work.resolve():
            raise RuntimeError('client substituted another working directory')
        if entry.get('errors'):
            raise RuntimeError('client reported a skill parsing error')
        skills = entry.get('skills')
        if not isinstance(skills, list):
            raise RuntimeError('client returned malformed discovery metadata')
        for name, description in wanted.items():
            matches = [item for item in skills if item.get('name') == name]
            if description is None:
                if matches:
                    raise RuntimeError('a removed fixture skill is still discovered')
                continue
            if len(matches) != 1:
                raise RuntimeError('installed fixture skill was missing or duplicated')
            enabled = description.get('enabled') if isinstance(description, dict) else True
            description = description.get('description') if isinstance(description, dict) else description
            found = matches[0]
            expected_path = fixture / '.agents' / 'skills' / name / 'SKILL.md'
            if (Path(found.get('path', '')).resolve() != expected_path.resolve()
                    or found.get('description') != description
                    or found.get('enabled') is not enabled
                    or found.get('scope') != 'user'):
                raise RuntimeError('client skill identity, scope, content or enabled state differed')
        print('PASS: real Codex client discovered the expected fixture skill metadata; network denied')
    finally:
        selector.close()
        try:
            process.stdin.close()
        except BrokenPipeError:
            pass
        try:
            process.wait(timeout=2)
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGTERM)
            try:
                process.wait(timeout=2)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait(timeout=2)


if __name__ == '__main__':
    try:
        main()
    except Exception as error:
        # No raw server response, path, source content or credential output.
        print('FAIL: ' + str(error) if isinstance(error, RuntimeError)
              else 'FAIL: fixture discovery could not be completed', file=sys.stderr)
        sys.exit(1)
