"""Versioned, content-free provenance and honest requirement execution coverage."""
from __future__ import annotations

import hashlib
import platform
import re
import shutil
import subprocess
import sys
from pathlib import Path


BASELINES = {
    'requirements': 'docs/需求.md',
    'matrix': 'docs/需求测试矩阵.md',
    'registry': 'scripts/automation/registry.json',
}


def file_digest(path: Path) -> str | None:
    if path.is_symlink() or not path.is_file():
        return None
    digest = hashlib.sha256()
    with path.open('rb') as stream:
        for chunk in iter(lambda: stream.read(65536), b''):
            digest.update(chunk)
    return digest.hexdigest()


def baseline_digests(root: Path) -> dict:
    return {key: file_digest(root / name) for key, name in BASELINES.items()}


def input_binding(trigger: str, source: str, prompt: str) -> dict:
    """Bind external material without persisting source notes or prompt text."""
    digest = lambda value: hashlib.sha256(value.encode('utf-8')).hexdigest()
    binding = {
        'trigger': trigger if trigger in ('manual-github', 'manual', 'x-mention') else 'unknown',
        'source_sha256': digest(source),
        'prompt_sha256': digest(prompt),
    }
    match = re.fullmatch(
        r'https://[A-Za-z0-9.-]+(?::[0-9]+)?/[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+/issues/[1-9][0-9]*#sha256=([0-9a-f]{64})',
        source,
    )
    if match:
        binding['snapshot_sha256'] = match.group(1)
    return binding


def provenance(root: Path) -> dict:
    """No host username, file content, prompts or environment values in the record."""
    versions = {'python': platform.python_version()}
    for name, arguments in [('node', ['--version']), ('cargo', ['--version']), ('git', ['--version'])]:
        executable = shutil.which(name)
        if executable is None:
            versions[name] = None
            continue
        try:
            result = subprocess.run([executable, *arguments], cwd=root, stdin=subprocess.DEVNULL,
                                    stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
                                    timeout=5, text=True)
            # Retain only a version number, never arbitrary program output.
            match = re.search(r'(?<!\d)\d+\.\d+(?:\.\d+)?(?:[-+][A-Za-z0-9.-]+)?', result.stdout[:256])
            versions[name] = match.group(0) if result.returncode == 0 and match else None
        except (OSError, subprocess.TimeoutExpired):
            versions[name] = None
    return {
        'schema_version': 1,
        'host_os': 'windows' if sys.platform == 'win32' else sys.platform,
        'architecture': platform.machine(),
        'tools': versions,
        'baselines': baseline_digests(root),
    }


def requirement_results(registry: dict, checks: list[dict]) -> dict:
    """Execution mapping is never a claim that all requirement scenarios are covered."""
    results = {check['check_id']: check['status'] for check in checks}
    coverage = {}
    for rid, entry in registry['requirements'].items():
        mapped = list(dict.fromkeys(entry.get('checks', [])))
        statuses = {cid: results.get(cid, 'not_selected') for cid in mapped}
        values = set(statuses.values())
        if not mapped:
            status = 'unimplemented'
        elif 'failed' in values:
            status = 'failed'
        elif 'blocked' in values or 'not_run' in values:
            status = 'blocked'
        elif values == {'not_selected'}:
            status = 'not_selected'
        elif 'not_selected' in values:
            status = 'partial_execution'
        else:
            status = 'mapped_checks_passed'
        coverage[rid] = {
            'execution_status': status,
            'coverage_status': entry.get('status', 'partial'),
            'checks': statuses,
            'complete_requirement_verified': False,
        }
    return coverage
