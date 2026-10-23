#!/usr/bin/env python3
"""OPS-01：磁盘满（有界写入失败）路径的显式门禁。

原子写失败路径的回归测试（`atomicfile::tests::a_write_that_hits_the_size_limit_…`）
以 RLIMIT_FSIZE 注入 `write_all` 中途失败——RLIMIT 是**进程级**限制，波及并行
线程，所以那条测试标 `#[ignore]`，由本脚本以 `--ignored --test-threads=1`
单独运行：只跑它一个，进程里没有别的测试可被误伤。

errno 边界（如实登记）：注入的是 EFBIG 而非 ENOSPC——真实磁盘满需要挂载小容量
镜像，hdiutil 在本机被系统拒绝（Operation not permitted）。两条错误码在原子写
里的处理路径完全相同（错误传播、暂存清理、原件保护），失败处理不看 errno。

Windows 不跑：RLIMIT_FSIZE 与本模块的 POSIX 权限语义都是 unix 的。
"""
from __future__ import annotations

import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
TEST_PATH = "atomicfile::tests::a_write_that_hits_the_size_limit_preserves_the_original_and_leaves_no_residue"


def main() -> int:
    if sys.platform == "win32":
        print("SKIP: RLIMIT_FSIZE is unix-only; the POSIX write path is not built on windows")
        return 0
    command = [
        "cargo", "test", "--locked",
        "--manifest-path", str(ROOT / "app/src-tauri/Cargo.toml"),
        TEST_PATH, "--", "--ignored", "--exact", "--test-threads=1",
    ]
    result = subprocess.run(command, cwd=ROOT)
    if result.returncode != 0:
        print("FAILED: 磁盘满（有界写入失败）路径的原子写回归变红", file=sys.stderr)
        return result.returncode
    print("PASS: a bounded-write failure preserves the original and leaves no residue")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
