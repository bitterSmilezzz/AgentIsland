#!/usr/bin/env bash
# Full automation/testing framework verification. Run from anywhere.
# Commands must succeed by exit status. Profile JSON must agree with that status;
# missing external prerequisites (blocked + rc 3) remain explicitly unverified.
# This script never grants authorization or enables external-test opt-ins.
set -uo pipefail
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO" || exit 2
source "$REPO/scripts/common.sh" || exit 2
if [ -z "$PYTHON_MIN" ]; then
  printf 'FAILED: Python >= 3.11 is required\n' >&2
  exit 2
fi
PY="$PYTHON_MIN"
fail=0
blocked=0
note() { printf '\n=== %s ===\n' "$1"; }
bad() { fail=1; printf 'FAILED: %s\n' "$1"; }

# Capture status before any output formatting; continue independent suites after failure.
run_suite() {
  local label="$1" out rc
  shift
  if out=$("$@" 2>&1); then rc=0; else rc=$?; fi
  if [ "$rc" -eq 0 ]; then
    printf '%-34s passed\n' "$label"
    printf '%s\n' "$out" | tail -6
  else
    printf '%s\n' "$out"
    bad "$label (exit $rc)"
  fi
}

run_profile() {
  local profile="$1" allow_blocked="$2" out rc state
  shift 2
  if out=$("$PY" scripts/automation/pipeline.py --profile "$profile" "$@" 2>&1); then
    rc=0
  else
    rc=$?
  fi
  # pipeline prints a single JSON summary. Reject malformed output and text lookalikes.
  if state=$(printf '%s' "$out" | "$PY" -c '
import json, sys
try:
    summary = json.load(sys.stdin)
    state = summary.get("outcome") if isinstance(summary, dict) else None
    if state not in ("passed", "failed", "blocked"):
        raise ValueError("invalid outcome")
except (ValueError, TypeError):
    raise SystemExit(1)
print(state)
'); then
    printf '%-10s %s (exit %s)\n' "$profile" "$state" "$rc"
    case "$state:$rc:$allow_blocked" in
      passed:0:*) return ;;
      blocked:3:yes)
        blocked=$((blocked + 1))
        printf 'UNVERIFIED: %s prerequisites unavailable; see pipeline evidence\n' "$profile"
        printf '%s\n' "$out"
        return ;;
    esac
  else
    state=invalid-json
  fi
  printf '%s\n' "$out"
  bad "$profile profile ($state, exit $rc)"
}

note "Rust full suite"
run_suite "Rust suite" cargo test --locked --manifest-path app/src-tauri/Cargo.toml

note "framework meta suites"
for t in test-automation-runner test-test-registry test-automation-artifact \
         test-automation-adapters test-automation-driver test-automation-trigger \
         test-automation-lessons test-automation-evidence test-automation-development \
         test-framework-verifier test-automation-snapshot test-automation-candidate \
         test-automation-pause test-automation-break; do
  run_suite "$t" "$PY" "scripts/$t.py"
done

note "pipeline selftest"
run_suite "selftest" "$PY" scripts/automation/selftest.py

note "scanner suite + workspace scan"
run_suite "scanner suite" bash scripts/test-scan-secrets.sh
run_suite "workspace scan" bash scripts/scan-secrets.sh

note "windows fragments (cfg(windows) code never compiles on this host)"
# macOS 上 #[cfg(windows)] 那一支零覆盖：改错了本机看不出来。
# 这个脚本把片段提取到独立 crate 交叉编译，专抓「凭记忆写」类错误
# （实测抓到过 unsafe 块遗漏与 windows-sys 模块路径猜错）。
# 它与下面的整 crate 检查**互补，不要删**：那个关掉了资源分支，这一段只有这里碰得到。
run_suite "windows fragments" bash scripts/check-windows-fragment.sh

note "windows crate type check (cross-module and test code)"
# 片段脚本只看被登记的少数函数。这一条对**整个 crate**（含测试）做 Windows 目标
# 类型检查，实测能抓到没人登记的新 Windows 代码，也一次找出过片段门禁看不见的
# 9 个错误（Unix 权限 API、libc::mktime）。与片段门禁有重叠但机制不同——
# 同题双抓已实测（smtp::hostname 被 cfg(any()) 抹掉时，两边都报 E0425，
# 但本脚本靠的是 bin 目标的真实调用方，--lib-only 抓不到）。
# 代价：首次分钟级（依赖编译），之后约 7 秒；缓存在 .build/，约 2.0G。
# 它在临时快照里关闭 tauri-build 的 Windows 资源分支（本机没有 llvm-rc），
# 因此**不是**标准 Windows 构建：资源编译、链接、打包、实机运行仍未验。
run_suite "windows crate" bash scripts/check-windows-crate.sh

note "unified gate (ci)"
run_profile ci no --check-timeout 1800

note "external-prerequisite profiles (passed or explicitly unverified)"
# soak is not a ci subset: its resource protocol is the only check the unified
# gate never reaches, so it must be attempted or reported unverified explicitly.
for p in native package manual client soak; do
  run_profile "$p" yes
done

printf '\n'
if [ "$fail" -eq 0 ]; then
  if [ "$blocked" -gt 0 ]; then
    printf 'ALL VERIFIED (available checks only; %s profiles blocked / unverified)\n' "$blocked"
  else
    printf 'ALL VERIFIED\n'
  fi
else
  printf 'SOMETHING FAILED\n'
fi
exit "$fail"
