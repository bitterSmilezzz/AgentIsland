#!/usr/bin/env python3
"""对正式应用验证：旧名称已退出、重复启动退出且保留原实例。"""
import re
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
