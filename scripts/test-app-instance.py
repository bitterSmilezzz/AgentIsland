#!/usr/bin/env python3
"""对正式应用验证：旧名称已退出、重复启动退出且保留原实例。"""
import re
import os
import signal
import sys
import subprocess
import time
from pathlib import Path


def app_pids():
    rows = subprocess.check_output(["ps", "-axo", "pid=,comm="], text=True)
    pattern = re.compile(r"/AgentIsland(?:-Rust|-Swift)?\.app/Contents/MacOS/(?:agentisland|AgentIsland)$")
    return [int(pid) for row in rows.splitlines()
            for pid, command in [row.strip().split(None, 1)] if pattern.search(command)]


root = Path(__file__).resolve().parent.parent
binary = root / "dist/AgentIsland.app/Contents/MacOS/agentisland"
if "--cold" in sys.argv:
    # 与 restart-app.sh 相同，只清理 bundle 内实例，不触碰同名 CLI。
    for pid in app_pids():
        os.kill(pid, signal.SIGTERM)
    for _ in range(40):
        if not app_pids(): break
        time.sleep(0.1)
    assert not app_pids(), "冷启动验证前旧应用仍在运行"
    launches = [subprocess.Popen([str(binary), "--shell=workbench"],
                stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL) for _ in range(2)]
    for _ in range(150):
        codes = [process.poll() for process in launches]
        if codes.count(None) == 1 and codes.count(0) == 1: break
        time.sleep(0.1)
    else:
        for process in launches:
            if process.poll() is None: process.terminate()
        raise AssertionError("两次同时冷启动未收敛为一个正常应用")
    assert len(app_pids()) == 1, "同时冷启动留下了多个应用实例"
    print("✓ 同时冷启动验证通过：两次打开仅保留一个实例")

for _ in range(40):
    before = app_pids()
    if before:
        break
    time.sleep(0.25)
assert len(before) == 1, f"预期一个应用实例，实际 {len(before)}"
second = subprocess.Popen([str(binary)], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
try:
    code = second.wait(timeout=10)
except subprocess.TimeoutExpired:
    second.terminate()
    second.wait(timeout=5)
    raise AssertionError("第二次启动未退出，单实例保护失效")
assert code == 0, f"第二次启动异常退出：{code}"
assert app_pids() == before, "重复启动改变了原实例或留下了第二个应用"
print("✓ 单实例验证通过：重复打开复用原进程，旧名称无残留")
