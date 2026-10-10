#!/usr/bin/env python3
"""Check compiled CLI usage against an isolated, synthetic conversation ledger.

Creates no windows and never reads personal conversation/configuration directories.
Status keeps its explicit usage opt-in; tokens and reports must read real usage.
"""
import argparse
import csv
from datetime import datetime, timedelta, timezone
import hashlib
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    args = parser.parse_args()
    if sys.platform != 'darwin':
        parser.error('macOS CLI usage acceptance')
    binary = args.binary.resolve(strict=True)
    with tempfile.TemporaryDirectory(prefix='agentisland-cli-usage-') as temporary:
        home = Path(temporary)
        config = home/'Library/Application Support/AgentIsland'
        config.mkdir(parents=True)
        settings = config/'settings.json'
        settings.write_text(json.dumps({'remote_policy': {'master_enabled': False},
            'token_alert_enabled': False, 'budget_alert_enabled': False,
            'auto_anomalies_alert': False, 'play_completion_sound': False}))
        sessions = home/'.codex/sessions'; sessions.mkdir(parents=True)
        ledger = sessions/'fixture.jsonl'
        now = datetime.now(timezone.utc)

        def record(identifier, timestamp, input_tokens, cached_tokens, output_tokens):
            return json.dumps({'type': 'token_usage_record',
                'timestamp': timestamp.isoformat(timespec='milliseconds').replace('+00:00', 'Z'),
                'payload': {'response_id': identifier, 'usage': {'input_tokens': input_tokens,
                    'cached_input_tokens': cached_tokens, 'output_tokens': output_tokens}}})

        ledger.write_text(record('fixture-recent', now-timedelta(hours=1), 140, 30, 40)+'\n'+
                          record('fixture-old', now-timedelta(days=2), 100, 40, 15)+'\n')
        baseline = {p: hashlib.sha256(p.read_bytes()).hexdigest() for p in [ledger, settings]}
        env = dict(os.environ, HOME=str(home), XDG_CONFIG_HOME=str(home/'config'),
                   XDG_DATA_HOME=str(home/'data'), XDG_STATE_HOME=str(home/'state'))

        def run(*arguments, expected_stderr=None):
            result = subprocess.run([str(binary), *arguments], env=env,
                capture_output=True, text=True, timeout=45)
            if expected_stderr is None:
                expected_stderr = '\n（建议写到文件：-o <路径>）\n' if arguments[0] == 'report' else ''
            assert result.returncode == 0 and result.stderr == expected_stderr, \
                f'Compiled CLI command failed: {arguments[0]}, exit={result.returncode}, stderr_bytes={len(result.stderr.encode())}'
            return result.stdout

        total = json.loads(run('tokens', '--json'))
        assert (total['tokens24h'], total['tokensTotal']) == (150, 225), \
            'tokens must fetch the ledger instead of returning unfetched zero totals'
        status = json.loads(run('status', '--all', '--json'))
        assert status and all(row['tokens24h'] is None for row in status), \
            'default status must continue to describe usage as not fetched'
        fetched = json.loads(run('status', '--all', '--json', '--usage'))
        assert next(row for row in fetched if row['id'] == 'codex')['tokens24h'] == 150
        rows = list(csv.DictReader(io.StringIO(run('report', '--format=csv'))))
        codex = next(row for row in rows if row['AgentID'] == 'codex')
        assert (codex['Tokens_24h'], codex['Tokens_Total']) == ('150', '225'), \
            'CSV report must fetch the same usage as tokens and opted-in status'
        separated = list(csv.DictReader(io.StringIO(run('report', '--format', 'csv'))))
        assert next(row for row in separated if row['AgentID'] == 'codex')['Tokens_24h'] == '150', \
            'The documented report --format csv form must produce CSV'
        for option in ['-o', '--out']:
            destination = home/('fixture '+option.lstrip('-')+'.csv')
            run('report', '--format=csv', option, str(destination), expected_stderr='')
            written = list(csv.DictReader(io.StringIO(destination.read_text())))
            assert next(row for row in written if row['AgentID'] == 'codex')['Tokens_Total'] == '225', \
                'The report destination option must write the requested file'
            inline_destination = home/('fixture inline '+option.lstrip('-')+'.csv')
            run('report', '--format', 'csv', option+'='+str(inline_destination), expected_stderr='')
            inline_rows = list(csv.DictReader(io.StringIO(inline_destination.read_text())))
            assert next(row for row in inline_rows if row['AgentID'] == 'codex')['Tokens_24h'] == '150'
        forbidden = home/'invalid-report.csv'
        for options in [('--format=pdf',), ('--format',), ('-o',),
                        ('--format=csv', '--format=md', '-o', str(forbidden))]:
            invalid = subprocess.run([str(binary), 'report', *options], env=env,
                capture_output=True, text=True, timeout=45)
            assert invalid.returncode == 2 and not invalid.stdout and invalid.stderr.startswith('✗ '), \
                'Invalid report arguments must fail as usage errors without report output'
            assert not forbidden.exists(), 'Invalid report arguments wrote an output file'
        markdown = run('report', '--format=md')
        cells = [[cell.strip() for cell in line.split('|')] for line in markdown.splitlines()]
        assert any(len(row) == 7 and row[1:3] == [codex['AgentName'], '150'] and row[4] == '225'
                   for row in cells), 'Markdown report omitted fixture usage'
        assert all(hashlib.sha256(p.read_bytes()).hexdigest() == digest for p, digest in baseline.items()), \
            'Read-only CLI commands changed the fixture ledger or settings'
        ledger.write_text('')
        empty = json.loads(run('tokens', '--json'))
        assert (empty['tokens24h'], empty['tokensTotal']) == (0, 0), 'An empty ledger is a real zero'
    print('PASS: compiled usage, report formats/destinations, invalid arguments, status opt-in, real zero and read-only inputs')


if __name__ == '__main__':
    main()
