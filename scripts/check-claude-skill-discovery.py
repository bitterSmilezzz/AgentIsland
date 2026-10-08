#!/usr/bin/env python3
"""Check a production-installed skill with a pinned, isolated Claude Code client.

Only synthetic content and a loopback model are used. Raw client/model records
stay in memory; all tools and skill shell injection are disabled.
"""
import argparse
import json
import os
from pathlib import Path
import selectors
import signal
import subprocess
import sys
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import urlsplit

NAME = 'agentisland-client-fixture'
VERSION = '2.1.291 (Claude Code)'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--client', type=Path, required=True)
    parser.add_argument('--fixture', type=Path, required=True)
    parser.add_argument('--expected', type=Path, required=True)
    args = parser.parse_args()
    if sys.platform != 'darwin':
        raise RuntimeError('requires macOS system isolation')
    fixture, client, expected = (p.resolve(strict=True) for p in (args.fixture, args.client, args.expected))
    if (fixture.parent != Path(tempfile.gettempdir()).resolve()
            or not fixture.name.startswith('agentisland-claude-skill-client-')
            or (fixture / '.agentisland-client-fixture').read_text() != 'fixture-only\n'
            or expected.parent != fixture or not client.is_file()
            or fixture == Path.home().resolve()):
        raise RuntimeError('disposable fixture identity could not be verified')
    wanted = json.loads(expected.read_text())
    if (set(wanted) != {'present', 'marker', 'absent_marker'}
            or type(wanted['present']) is not bool
            or not all(isinstance(wanted[k], str) and len(wanted[k]) < 128
                       for k in ('marker', 'absent_marker'))):
        raise RuntimeError('invalid expected fixture metadata')
    work, temporary = fixture / 'project', fixture / 'tmp'
    work.mkdir(exist_ok=True); temporary.mkdir(exist_ok=True)
    requests = []
    failures = []

    class API(BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass

        def do_POST(self):
            size = int(self.headers.get('Content-Length', '0'))
            if size <= 0 or size > 2 * 1024 * 1024 or len(requests) >= 8:
                failures.append('request limit'); self.send_error(400); return
            data = json.loads(self.rfile.read(size))
            endpoint = urlsplit(self.path).path
            if endpoint == '/v1/messages/count_tokens':
                self.send_response(200); self.end_headers()
                self.wfile.write(b'{"input_tokens":100}'); return
            if endpoint != '/v1/messages':
                failures.append('unexpected endpoint'); self.send_error(404); return
            if data.get('tools'):
                failures.append('unexpected tools'); self.send_error(400); return
            requests.append(data)
            block = {'type': 'text', 'text': 'Controlled fixture completed.'}
            message = {'id': 'msg_fixture', 'type': 'message', 'role': 'assistant',
                       'model': data.get('model', 'fixture'), 'content': [block],
                       'stop_reason': 'end_turn', 'stop_sequence': None,
                       'usage': {'input_tokens': 100, 'output_tokens': 10}}
            self.send_response(200)
            self.send_header('Content-Type', 'text/event-stream' if data.get('stream') else 'application/json')
            self.end_headers()
            if not data.get('stream'):
                self.wfile.write(json.dumps(message).encode()); return
            events = [
                ('message_start', {'type': 'message_start', 'message': {
                    **message, 'content': [], 'stop_reason': None,
                    'usage': {'input_tokens': 100, 'output_tokens': 0}}}),
                ('content_block_start', {'type': 'content_block_start', 'index': 0, 'content_block': block}),
                ('content_block_stop', {'type': 'content_block_stop', 'index': 0}),
                ('message_delta', {'type': 'message_delta', 'delta': {
                    'stop_reason': 'end_turn', 'stop_sequence': None}, 'usage': {'output_tokens': 10}}),
                ('message_stop', {'type': 'message_stop'}),
            ]
            for event, value in events:
                self.wfile.write(('event: ' + event + '\ndata: ' + json.dumps(value) + '\n\n').encode())

    server = ThreadingHTTPServer(('127.0.0.1', 0), API)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    environment = {'HOME': str(fixture), 'XDG_CONFIG_HOME': str(fixture / '.config'),
                   'TMPDIR': str(temporary), 'PATH': '/usr/bin:/bin:/usr/sbin:/sbin',
                   'LANG': 'en_US.UTF-8', 'CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC': '1',
                   'DISABLE_TELEMETRY': '1', 'DISABLE_ERROR_REPORTING': '1', 'DISABLE_UPDATES': '1',
                   'ANTHROPIC_BASE_URL': f'http://127.0.0.1:{server.server_port}',
                   'ANTHROPIC_API_KEY': 'fixture-only'}  # nosec: synthetic loopback credential only
    quote = lambda value: json.dumps(str(value), ensure_ascii=False)
    profile = ('(version 1) (allow default) (deny network*) '
               f'(allow network-outbound (remote ip "localhost:{server.server_port}")) '
               f'(deny file-read-data (require-all (subpath {quote(Path.home().resolve())}) '
               f'(require-not (literal {quote(client)})))) '
               f'(deny file-write* (require-not (subpath {quote(fixture)})))')
    isolated = ['/usr/bin/sandbox-exec', '-p', profile]
    probe = ('import socket,errno,os; s=socket.socket(); '
             'assert s.connect_ex(("127.0.0.1",9)) in (errno.EPERM,errno.EACCES); s.close(); '
             '\ntry: os.listdir(' + repr(str(Path.home().resolve())) + ')'
             '\nexcept PermissionError: pass'
             '\nelse: raise AssertionError("daily home readable")')
    process = None
    selector = selectors.DefaultSelector()
    try:
        subprocess.run(['/usr/bin/codesign', '--verify', '--strict', str(client)],
                       check=True, capture_output=True, timeout=10)
        subprocess.run(isolated + ['/usr/bin/python3', '-B', '-c', probe], env=environment,
                       cwd=work, check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=5)
        version = subprocess.run(isolated + [str(client), '--version'], env=environment, cwd=work,
                                 check=True, capture_output=True, text=True, timeout=10).stdout.strip()
        if version != VERSION:
            raise RuntimeError('client version differs from the verified fixture')
        prompt = '/' + NAME if wanted['present'] else 'Controlled discovery only; return without using tools.'
        command = [str(client), '--print', '--no-session-persistence', '--output-format', 'stream-json', '--verbose',
                   '--setting-sources', 'user', '--strict-mcp-config', '--mcp-config', '{"mcpServers":{}}',
                   '--settings', '{"disableBundledSkills":true,"disableSkillShellExecution":true}',
                   '--tools', '', '--permission-mode', 'dontAsk', '--model', 'claude-sonnet-4-6', prompt]
        process = subprocess.Popen(isolated + command, env=environment, cwd=work,
                                   stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                                   stderr=subprocess.DEVNULL, start_new_session=True)
        selector.register(process.stdout, selectors.EVENT_READ)
        output, deadline = bytearray(), time.monotonic() + 40
        while True:
            if time.monotonic() >= deadline:
                raise RuntimeError('client discovery timed out')
            if not selector.select(min(1, deadline - time.monotonic())):
                continue
            chunk = os.read(process.stdout.fileno(), 65536)
            if not chunk:
                break
            output.extend(chunk)
            if len(output) > 2 * 1024 * 1024:
                raise RuntimeError('client output exceeded the fixture limit')
        code = process.wait(timeout=5)
        records = [json.loads(line) for line in output.splitlines()]
        initial = [r for r in records if r.get('type') == 'system' and r.get('subtype') == 'init']
        finished = [r for r in records if r.get('type') == 'result']
        if (code != 0 or len(initial) != 1 or len(finished) != 1
                or finished[0].get('is_error') is not False
                or finished[0].get('subtype') != 'success'):
            raise RuntimeError('client did not complete the controlled discovery: ' + json.dumps({
                'exit': code, 'init_count': len(initial), 'result_count': len(finished),
                'requests': len(requests), 'endpoint_or_tools_failures': failures,
                'result_subtypes': [r.get('subtype') for r in finished]}))
        listed = initial[0].get('slash_commands', []).count(NAME)
        if listed != int(wanted['present']) or failures:
            raise RuntimeError('client skill discovery or tool isolation differed')
        raw = json.dumps(requests, ensure_ascii=False)
        if not requests or (wanted['marker'] and wanted['marker'] not in raw):
            raise RuntimeError('the installed skill body was not loaded')
        if wanted['absent_marker'] and wanted['absent_marker'] in raw:
            raise RuntimeError('an earlier skill body remained active')
        if wanted['present'] and str(fixture / '.claude' / 'skills' / NAME) not in raw:
            raise RuntimeError('client did not bind the expected user skill directory')
        print('PASS: real Claude Code user skill discovery and expected body; isolated loopback model, no tools')
    finally:
        selector.close()
        if process is not None and process.poll() is None:
            os.killpg(process.pid, signal.SIGTERM)
            try:
                process.wait(timeout=3)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL); process.wait(timeout=3)
        server.shutdown(); server.server_close()


if __name__ == '__main__':
    try:
        main()
    except Exception as error:
        print('FAIL: ' + str(error) if isinstance(error, RuntimeError)
              else 'FAIL: controlled client discovery could not be completed', file=sys.stderr)
        sys.exit(1)
