#!/usr/bin/env python3
"""Exercise the real version gate against independently changed release inputs."""
import json
import shutil
import subprocess
import tempfile
from pathlib import Path

with tempfile.TemporaryDirectory(prefix='agentisland-version-') as name:
    root = Path(name)
    (root / 'scripts').mkdir()
    (root / 'app/src-tauri').mkdir(parents=True)
    verifier = root / 'scripts/check-version.py'
    shutil.copyfile(Path(__file__).with_name('check-version.py'), verifier)
    files = {
        'app/src-tauri/Cargo.toml': '[package]\nname = "agentisland"\nversion = "1.2.3"\n',
        'app/src-tauri/Cargo.lock': '[[package]]\nname = "agentisland"\nversion = "1.2.3"\n',
        'app/src-tauri/tauri.conf.json': json.dumps({'version': '1.2.3'}),
        'CHANGELOG.md': '## [1.2.3]\n',
        'README.md': '本文档描述 **v1.2.3**\n',
    }
    for file, content in files.items():
        (root / file).write_text(content)
    def run(version='1.2.3'):
        return subprocess.run(['python3', str(verifier), version], capture_output=True, text=True)
    assert run().returncode == 0
    for file, content in files.items():
        (root / file).write_text(content.replace('1.2.3', '1.2.4'))
        assert run().returncode != 0, f'{file} drift was accepted'
        (root / file).write_text(content)
    assert run('invalid').returncode != 0
print('✓ 版本门禁：一致通过，五处独立漂移与非法版本均拒绝')
