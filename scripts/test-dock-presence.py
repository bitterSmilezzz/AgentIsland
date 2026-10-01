#!/usr/bin/env python3
"""检查 macOS 原生 activationPolicy，依次覆盖默认、打开、收起、再打开、关闭。"""
import pathlib
import subprocess
import sys
import tempfile
import time

if sys.platform != 'darwin':
    print('Dock 策略检查仅适用于 macOS')
    sys.exit(0)
root = pathlib.Path(__file__).resolve().parents[1]
binary = root / 'dist/AgentIsland.app/Contents/MacOS/agentisland'
with tempfile.TemporaryDirectory(prefix='agentisland-dock-') as directory:
    helper = pathlib.Path(directory) / 'policy'
    source = helper.with_suffix('.swift')
    source.write_text('''import AppKit
if let pid = Int32(CommandLine.arguments[1]), let app = NSRunningApplication(processIdentifier: pid) {
    print(app.activationPolicy.rawValue)
}
''')
    subprocess.run(['swiftc', str(source), '-o', str(helper)], check=True)
    # 按 bundle 完整路径结束应用；不匹配同名 CLI。
    subprocess.run(['pkill', '-f', '/AgentIsland[.]app/Contents/MacOS/agentisland([[:space:]]|$)'], check=False)
    time.sleep(0.5)
    app = subprocess.Popen([str(binary), '--dock-smoke'], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    observed = []
    try:
        deadline = time.monotonic() + 20
        while time.monotonic() < deadline and app.poll() is None:
            result = subprocess.run([str(helper), str(app.pid)], capture_output=True, text=True, check=True).stdout.strip()
            if result in ('0', '1'):
                value = int(result)
                if not observed and value == 0:
                    continue
                if not observed or observed[-1] != value:
                    observed.append(value)
            time.sleep(0.1)
        assert app.wait(timeout=5) == 0, '原生 Dock 回归应用未正常结束'
        assert observed == [1, 0, 1, 0, 1], f'Dock 显隐顺序错误: {observed}'
        print('✓ Dock 原生回归通过：默认隐藏 → 打开显示 → 收起隐藏 → 再打开显示 → 关闭隐藏')
    finally:
        if app.poll() is None:
            app.terminate()
            app.wait(timeout=5)
