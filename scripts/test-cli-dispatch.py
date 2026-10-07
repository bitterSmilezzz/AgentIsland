#!/usr/bin/env python3
"""Exercise compiled macOS CLI dispatch with a recording system-command fixture.

No real open command, application launch, window or user settings are touched.
This verifies CLI selection/argv, not LaunchServices delivery or native UI.
"""
import argparse
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import sys
import tempfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    args = parser.parse_args()
    if sys.platform != 'darwin':
        parser.error('macOS CLI selection test')
    binary = args.binary.resolve(strict=True)
    with tempfile.TemporaryDirectory(prefix='agentisland-cli-dispatch-') as temporary:
        root = Path(temporary)
        shim = root/'bin'; shim.mkdir()
        capture = root/'captured.json'
        recorder = root/'record.py'
        recorder.write_text('import json,os,sys\nfrom pathlib import Path\n'
                            'Path(os.environ["AGENTISLAND_DISPATCH_CAPTURE"]).write_text(json.dumps(sys.argv[1:]))\n')
        opener = shim/'open'
        opener.write_text('#!/bin/sh\nexec '+shlex.quote(sys.executable)+' '+shlex.quote(str(recorder))+' "$@"\n')
        opener.chmod(0o755)
        env = dict(os.environ, PATH=str(shim)+os.pathsep+os.environ.get('PATH',os.defpath),
                   AGENTISLAND_DISPATCH_CAPTURE=str(capture))
        distribution = root/"Release '中文' folder"; distribution.mkdir()
        bundle = distribution/'AgentIsland.app'
        (bundle/'Contents/MacOS').mkdir(parents=True)
        (bundle/'Contents/Helpers').mkdir()
        (bundle/'Contents/Info.plist').write_text('fixture')
        paths = [distribution/'agentisland', bundle/'Contents/MacOS/agentisland',
                 bundle/'Contents/Helpers/agentisland']
        for path in paths:
            shutil.copy2(binary,path)

        def check(cli, command, expected, code=0):
            capture.unlink(missing_ok=True)
            result = subprocess.run([str(cli),'open',*command],env=env,capture_output=True,timeout=15)
            assert result.returncode == code, 'CLI exit status does not match dispatch outcome'
            actual = json.loads(capture.read_text()) if capture.exists() else None
            assert actual == expected, 'CLI passed an unexpected system command argument vector'

        for cli in paths:
            check(cli,['workbench-hide'],['-a',str(bundle.resolve()),'agentisland://workbench-hide'])
        link = root/'linked-cli'; link.symlink_to(paths[0])
        check(link,['agent','a&b=c'],['-a',str(bundle.resolve()),'agentisland://agent?id=a%26b%3Dc'])
        isolated = root/'standalone'; isolated.mkdir()
        cli = isolated/'agentisland'; shutil.copy2(binary,cli)
        check(cli,['workbench'],['agentisland://workbench'])
        broken = isolated/'AgentIsland.app'; broken.mkdir()
        check(cli,['workbench'],None,1)
        broken.rmdir(); broken.symlink_to(isolated/'missing.app')
        check(cli,['workbench'],None,1)
        check(cli,['agent'],None,2)
        print('PASS: compiled CLI selects paired apps, follows symlinks, preserves argv and standalone fallback, and refuses damaged packages')


if __name__ == '__main__':
    main()
