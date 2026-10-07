#!/usr/bin/env python3
"""检查 macOS 原生 activationPolicy，依次覆盖默认、打开、收起、再打开、关闭。"""
import os
import pathlib
import select
import shutil
import subprocess
import sys
import tempfile
import time

if sys.platform != 'darwin':
    print('Dock 策略检查仅适用于 macOS')
    sys.exit(0)
if '--isolated-session' not in sys.argv[1:]:
    print('原生 Dock 测试会影响桌面焦点；仅在隔离用户会话或虚拟机内使用 --isolated-session。', file=sys.stderr)
    sys.exit(2)
root = pathlib.Path(__file__).resolve().parents[1]
binary = root / 'dist/AgentIsland.app/Contents/MacOS/agentisland'
with tempfile.TemporaryDirectory(prefix='agentisland-dock-') as directory:
    helper = pathlib.Path(directory) / 'policy'
    prebuilt = os.environ.get('POLICY_HELPER', '')
    # 隔离 VM 里没有 Xcode CLT；POLICY_HELPER 指向主机预编译的同一 helper，
    # 省去在 guest 里装整套工具链。默认行为不变：本机仍现场编译。
    if prebuilt and os.access(prebuilt, os.X_OK):
        shutil.copyfile(prebuilt, helper)
        os.chmod(helper, 0o755)
    else:
        source = helper.with_suffix('.swift')
        source.write_text('''import AppKit
if let pid = Int32(CommandLine.arguments[1]), let app = NSRunningApplication(processIdentifier: pid) {
    print(app.activationPolicy.rawValue)
}
''')
        subprocess.run(['swiftc', str(source), '-o', str(helper)], check=True)
    # 不结束用户运行中的应用；测试应在独立会话执行。
    app = subprocess.Popen([str(binary), '--dock-smoke'], stdin=subprocess.PIPE,
                           stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
    observed = []
    try:
        pending = b''
        for stage, expected in [('default', 1), ('open', 0), ('hide', 1), ('reopen', 0), ('close', 1)]:
            deadline = time.monotonic() + 30
            marker = f'DOCK_SMOKE_READY {stage}'.encode()
            ready = False
            while time.monotonic() < deadline and app.poll() is None:
                if select.select([app.stdout], [], [], 0.1)[0]:
                    chunk = os.read(app.stdout.fileno(), 4096)
                    assert chunk, f'Dock 回归在 {stage} 阶段提前结束'
                    pending += chunk
                while b'\n' in pending:
                    line, pending = pending.split(b'\n', 1)
                    if line.startswith(b'DOCK_SMOKE_READY '):
                        assert line == marker, f'Dock 阶段顺序错误: {line!r}'
                        ready = True
                if ready:
                    break
            assert ready, f'Dock 回归等待 {stage} 超时'
            # Read NSRunningApplication, not the driver's claimed state. No next
            # action can start before this native observation has succeeded.
            matched = False
            while time.monotonic() < deadline and app.poll() is None:
                result = subprocess.run([str(helper), str(app.pid)], capture_output=True,
                                        text=True, check=True, timeout=5).stdout.strip()
                if result == str(expected):
                    matched = True
                    break
                time.sleep(0.1)
            assert matched, f'Dock {stage} 的原生策略未成为 {expected}'
            observed.append(expected)
            app.stdin.write(b'continue\n')
            app.stdin.flush()
        assert app.wait(timeout=5) == 0, '原生 Dock 回归应用未正常结束'
        assert observed == [1, 0, 1, 0, 1], f'Dock 显隐顺序错误: {observed}'
        print('✓ Dock 原生回归通过：默认隐藏 → 打开显示 → 收起隐藏 → 再打开显示 → 关闭隐藏')
    finally:
        if app.poll() is None:
            app.terminate()
            app.wait(timeout=5)
