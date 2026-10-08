#!/usr/bin/env python3
"""Check synthetic user guidance in real clients with an isolated loopback model.

Run only from an explicit production-Store acceptance fixture. All raw client
output and model requests remain in memory; ordinary tests never start clients.
"""
import argparse
import hashlib
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


def responses_events(model):
    text = 'Controlled fixture completed.'
    item = {'id': 'msg_fixture', 'type': 'message', 'role': 'assistant', 'status': 'completed',
            'content': [{'type': 'output_text', 'text': text, 'annotations': []}]}
    response = {'id': 'resp_fixture', 'object': 'response', 'created_at': 1,
                'model': model, 'status': 'completed', 'output': [item],
                'usage': {'input_tokens': 100, 'output_tokens': 10, 'total_tokens': 110,
                          'input_tokens_details': {'cached_tokens': 0},
                          'output_tokens_details': {'reasoning_tokens': 0}}}
    return [
        ('response.created', {'response': {**response, 'status': 'in_progress', 'output': [], 'usage': None}}),
        ('response.output_item.added', {'output_index': 0, 'item': {**item, 'status': 'in_progress', 'content': []}}),
        ('response.content_part.added', {'item_id': item['id'], 'output_index': 0, 'content_index': 0,
                                         'part': {'type': 'output_text', 'text': '', 'annotations': []}}),
        ('response.output_text.delta', {'item_id': item['id'], 'output_index': 0, 'content_index': 0, 'delta': text}),
        ('response.output_text.done', {'item_id': item['id'], 'output_index': 0, 'content_index': 0, 'text': text}),
        ('response.content_part.done', {'item_id': item['id'], 'output_index': 0, 'content_index': 0, 'part': item['content'][0]}),
        ('response.output_item.done', {'output_index': 0, 'item': item}),
        ('response.completed', {'response': response}),
    ]


def claude_events(model):
    block = {'type': 'text', 'text': 'Controlled fixture completed.'}
    return [
        ('message_start', {'message': {'id': 'msg_fixture', 'type': 'message', 'role': 'assistant',
                                       'model': model, 'content': [], 'stop_reason': None,
                                       'usage': {'input_tokens': 100, 'output_tokens': 0}}}),
        ('content_block_start', {'index': 0, 'content_block': block}),
        ('content_block_stop', {'index': 0}),
        ('message_delta', {'delta': {'stop_reason': 'end_turn', 'stop_sequence': None}, 'usage': {'output_tokens': 10}}),
        ('message_stop', {}),
    ]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--client', type=Path, required=True)
    parser.add_argument('--kind', choices=['codex', 'claude'], required=True)
    parser.add_argument('--version', required=True)
    parser.add_argument('--fixture', type=Path, required=True)
    parser.add_argument('--expected', type=Path, required=True)
    args = parser.parse_args()
    if sys.platform != 'darwin':
        raise RuntimeError('requires macOS system isolation')
    fixture, client, expected = (p.resolve(strict=True) for p in (args.fixture, args.client, args.expected))
    if (fixture.parent != Path(tempfile.gettempdir()).resolve()
            or not fixture.name.startswith('agentisland-instructions-client-')
            or (fixture / '.agentisland-client-fixture').read_text() != 'fixture-only\n'
            or expected.parent != fixture or not client.is_file() or fixture == Path.home().resolve()):
        raise RuntimeError('disposable fixture identity could not be verified')
    wanted = json.loads(expected.read_text())
    if (set(wanted) != {'marker', 'absent'} or not isinstance(wanted['marker'], str)
            or not wanted['marker'].startswith('AGENTISLAND_FIXTURE_GUIDANCE_')
            or not isinstance(wanted['absent'], list)
            or not all(isinstance(s, str) and s.startswith('AGENTISLAND_FIXTURE_GUIDANCE_') for s in wanted['absent'])):
        raise RuntimeError('invalid expected fixture metadata')
    work, temporary = fixture / 'project', fixture / 'tmp'
    work.mkdir(exist_ok=True); temporary.mkdir(exist_ok=True)
    requests, failures = [], []

    class API(BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass

        def do_POST(self):
            try:
                size = int(self.headers.get('Content-Length', '0'))
                if size <= 0 or size > 2 * 1024 * 1024 or len(requests) >= 8:
                    raise ValueError()
                data = json.loads(self.rfile.read(size))
                path = urlsplit(self.path).path
                if args.kind == 'claude' and path == '/v1/messages/count_tokens':
                    self.send_response(200); self.end_headers()
                    self.wfile.write(b'{"input_tokens":100}'); return
                if path != ('/v1/responses' if args.kind == 'codex' else '/v1/messages'):
                    raise ValueError()
                if args.kind == 'claude' and data.get('tools'):
                    raise ValueError()
                if data.get('stream') is not True:
                    raise ValueError()
                requests.append(data)
                events = responses_events(data.get('model', 'fixture')) if args.kind == 'codex' else claude_events(data.get('model', 'fixture'))
                self.send_response(200); self.send_header('Content-Type', 'text/event-stream'); self.end_headers()
                for index, (name, event) in enumerate(events):
                    value = dict(type=name, **event)
                    if args.kind == 'codex':
                        value['sequence_number'] = index
                    self.wfile.write(('event: ' + name + '\ndata: ' + json.dumps(value) + '\n\n').encode())
                self.wfile.flush()
            except (ValueError, TypeError, OSError):
                failures.append('invalid or failed controlled request')
                self.send_error(400)

    server = ThreadingHTTPServer(('127.0.0.1', 0), API)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    environment = {'HOME': str(fixture), 'CODEX_HOME': str(fixture / '.codex'),
                   'XDG_CONFIG_HOME': str(fixture / '.config'), 'TMPDIR': str(temporary),
                   'PATH': '/usr/bin:/bin:/usr/sbin:/sbin', 'LANG': 'en_US.UTF-8',
                   'NO_PROXY': '127.0.0.1,localhost', 'no_proxy': '127.0.0.1,localhost'}
    if args.kind == 'claude':
        environment.update({'CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC': '1', 'DISABLE_TELEMETRY': '1',
                            'DISABLE_ERROR_REPORTING': '1', 'DISABLE_UPDATES': '1',
                            'ANTHROPIC_BASE_URL': f'http://127.0.0.1:{server.server_port}',
                            'ANTHROPIC_API_KEY': 'fixture-only'})  # nosec: synthetic loopback credential only
    else:
        config = fixture / '.codex' / 'config.toml'; config.parent.mkdir(exist_ok=True)
        config.write_text('model = "fixture-model"\nmodel_provider = "fixture"\n'
                          'approval_policy = "never"\nsandbox_mode = "read-only"\n'
                          '[model_providers.fixture]\nname = "Controlled loopback"\n'
                          f'base_url = "http://127.0.0.1:{server.server_port}/v1"\n'
                          'wire_api = "responses"\nrequires_openai_auth = false\n'
                          'request_max_retries = 0\nstream_max_retries = 0\n'
                          'supports_websockets = false\n')
    quote = lambda value: json.dumps(str(value), ensure_ascii=False)
    profile = ('(version 1) (allow default) (deny network*) '
               f'(allow network-outbound (remote ip "localhost:{server.server_port}")) '
               f'(deny file-read-data (require-all (subpath {quote(Path.home().resolve())}) '
               f'(require-not (literal {quote(client)})))) '
               f'(deny file-write* (require-not (subpath {quote(fixture)})))')
    isolated = ['/usr/bin/sandbox-exec', '-p', profile]
    probe = ('import socket,errno,os; s=socket.socket(); '
             'assert s.connect_ex(("127.0.0.1",9)) in (errno.EPERM,errno.EACCES); s.close(); '
             's=socket.socket(); '
             f'assert s.connect_ex(("127.0.0.1",{server.server_port})) == 0; s.close(); '
             '\ntry: os.listdir(' + repr(str(Path.home().resolve())) + ')'
             '\nexcept PermissionError: pass'
             '\nelse: raise AssertionError("daily home readable")')
    process = None
    selector = selectors.DefaultSelector()
    try:
        subprocess.run(['/usr/bin/codesign', '--verify', '--strict', str(client)],
                       check=True, capture_output=True, timeout=10)
        subprocess.run(isolated + ['/usr/bin/python3', '-B', '-c', probe], env=environment, cwd=work,
                       check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=5)
        version = subprocess.run(isolated + [str(client), '--version'], env=environment, cwd=work,
                                 check=True, capture_output=True, text=True, timeout=10).stdout.strip()
        if version != args.version:
            raise RuntimeError('client version differs from the explicit acceptance target')
        prompt = 'Controlled instruction loading check. Return without using any tools.'
        if args.kind == 'codex':
            command = [str(client), 'exec', '--json', '--ephemeral', '--skip-git-repo-check',
                       '--ignore-rules', '--sandbox', 'read-only', '--color', 'never', prompt]
        else:
            command = [str(client), '--print', '--no-session-persistence', '--output-format', 'stream-json',
                       '--verbose', '--setting-sources', 'user', '--strict-mcp-config', '--mcp-config', '{"mcpServers":{}}',
                       '--settings', '{"disableBundledSkills":true,"disableSkillShellExecution":true}',
                       '--tools', '', '--permission-mode', 'dontAsk', '--model', 'claude-sonnet-4-6', prompt]
        process = subprocess.Popen(isolated + command, env=environment, cwd=work, stdin=subprocess.DEVNULL,
                                   stdout=subprocess.PIPE, stderr=subprocess.PIPE, start_new_session=True)
        selector.register(process.stdout, selectors.EVENT_READ, 'stdout')
        selector.register(process.stderr, selectors.EVENT_READ, 'stderr')
        output, diagnostics, deadline = bytearray(), bytearray(), time.monotonic() + 40
        while True:
            if time.monotonic() >= deadline:
                known_types = {'thread.started', 'turn.started', 'turn.completed', 'turn.failed', 'error',
                               'item.started', 'item.updated', 'item.completed'}
                kinds = []
                error_signals = set()
                for line in output.splitlines():
                    try:
                        record = json.loads(line)
                        kind = record.get('type')
                        kinds.append(kind if kind in known_types else 'other')
                        if kind == 'error':
                            message = str(record.get('message', '')).lower()
                            for signal_text in ('http://127.0.0.1:', 'https://', 'ws://', 'wss://',
                                                'permission denied', 'operation not permitted',
                                                'connection', 'stream', 'status', 'reconnecting'):
                                if signal_text in message:
                                    error_signals.add(signal_text)
                    except (ValueError, AttributeError):
                        kinds.append('non-event')
                raise RuntimeError('client instruction loading timed out: ' + json.dumps({
                    'requests': len(requests), 'failures': len(failures), 'event_types': kinds,
                    'error_signals': sorted(error_signals),
                    'stderr_signals': [s for s in ('permission denied', 'operation not permitted', 'proxy',
                                                  'websocket', 'connection refused', 'connection reset',
                                                  'error sending request', 'dns', 'certificate', 'panic')
                                       if s in diagnostics.decode(errors='replace').lower()]}))
            events = selector.select(min(1, deadline - time.monotonic()))
            if not events:
                continue
            for key, _ in events:
                chunk = os.read(key.fileobj.fileno(), 65536)
                if not chunk:
                    selector.unregister(key.fileobj)
                    continue
                buffer = output if key.data == 'stdout' else diagnostics
                buffer.extend(chunk)
                if len(buffer) > 2 * 1024 * 1024:
                    raise RuntimeError('client output exceeded the fixture limit')
            if not selector.get_map():
                break
        code = process.wait(timeout=5)
        records = [json.loads(line) for line in output.splitlines()]
        if args.kind == 'codex':
            finished = [r for r in records if r.get('type') == 'turn.completed']
            unexpected = [r for r in records if r.get('type') in ['error', 'turn.failed'] or
                          r.get('item', {}).get('type') in ['command_execution', 'mcp_tool_call', 'file_change']]
            success = len(finished) == 1 and not unexpected
        else:
            finished = [r for r in records if r.get('type') == 'result']
            success = len(finished) == 1 and finished[0].get('is_error') is False and finished[0].get('subtype') == 'success'
        if code != 0 or not success or failures or not requests:
            raise RuntimeError('controlled client did not finish: ' + json.dumps({
                'exit': code, 'requests': len(requests), 'finished': len(finished), 'failures': len(failures)}))
        raw = json.dumps(requests, ensure_ascii=False)
        if wanted['marker'] not in raw or any(marker in raw for marker in wanted['absent']):
            raise RuntimeError('client guidance differs from the applied version')
        print(json.dumps({'passed': True, 'kind': args.kind, 'version': version,
                          'client_sha256': hashlib.sha256(client.read_bytes()).hexdigest(),
                          'requests': len(requests), 'isolation_verified': True, 'raw_output_saved': False}))
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
              else 'FAIL: controlled instruction loading could not be completed', file=sys.stderr)
        sys.exit(1)
