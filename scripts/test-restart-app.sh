#!/bin/bash
# restart-app.sh 的优雅退出路径验证。
#
# 为什么需要：这是本仓库**唯一规定**的重启动作，也是通知账本写盘收尾
# （钉在 Tauri 退出回调上）的唯一正常路径。此前它没有任何门禁覆盖，只靠「实测过一次」。
#
# 钉住两条容易失效的地方，且**不启动真实应用**：
#   ① 应用无响应时 Apple Event 会一直挂着 → 必须有 osascript 自身的超时，
#      否则那个 8 秒等待（从 osascript 返回后才开始计时）一次都轮不到。
#   ② 优雅退出失败后必须明说「本次写盘收尾可能未执行」，不能静默降级成信号终止。
#
# 全部用临时沙箱里的假命令驱动真实脚本，不接触用户的应用、配置或提醒。
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$SCRIPT_DIR/.." && pwd)"

SANDBOXES=$(mktemp -d "${TMPDIR:-/tmp}/restart-app-test.XXXXXX")
cleanup_all() { rm -rf "$SANDBOXES" 2>/dev/null || true; }
trap cleanup_all EXIT

FAILED=0
ok()   { printf '  ✓ %s\n' "$1"; }
fail() { printf '  ✗ %s\n' "$1"; FAILED=1; }

make_case() {
    local case_dir="$1"
    mkdir -p "$case_dir/scripts" "$case_dir/dist/AgentIsland.app/Contents/MacOS" \
             "$case_dir/shims" "$case_dir/state"
    cp "$REPO/scripts/restart-app.sh" "$case_dir/scripts/restart-app.sh"
    cp "$REPO/scripts/common.sh" "$case_dir/scripts/common.sh"
    # 写**真实**的 Info.plist：脚本里读的是硬编码的 /usr/libexec/PlistBuddy，
    # 用 PATH shim 拦不住，所以这里必须给一份真能让它读通的 plist。
    cat > "$case_dir/dist/AgentIsland.app/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleExecutable</key><string>agentisland</string>
</dict></plist>
PLIST

    # 假 pgrep：以 $CASE_STATE/pid 决定实例是否存活，并记录调用次数，
    # 让「等待循环是否真的跑过」可观测。
    cat > "$case_dir/shims/pgrep" <<'SHIM'
#!/bin/sh
count_file="$CASE_STATE/pgrep.calls"
n=$(cat "$count_file" 2>/dev/null || echo 0); echo $((n + 1)) > "$count_file"
[ -f "$CASE_STATE/pid" ] || exit 1
cat "$CASE_STATE/pid"
exit 0
SHIM
    cat > "$case_dir/shims/pkill" <<'SHIM'
#!/bin/sh
echo pkill >> "$CASE_STATE/actions"
rm -f "$CASE_STATE/pid"
exit 0
SHIM
    cat > "$case_dir/shims/open" <<'SHIM'
#!/bin/sh
echo "open $1" >> "$CASE_STATE/actions"
echo 42424 > "$CASE_STATE/pid"
exit 0
SHIM
    # 只在问 CFBundleExecutable 时回答；其余交回真 PlistBuddy（本用例用真实 plist，shim 只是保险）。
    cat > "$case_dir/shims/PlistBuddy" <<'SHIM'
#!/bin/sh
for arg in "$@"; do
    case "$arg" in
        *CFBundleExecutable*) printf 'agentisland\n'; exit 0 ;;
    esac
done
exec /usr/libexec/PlistBuddy "$@"
SHIM
    # 可执行文件必须在 plist 写好后创建
    printf '#!/bin/sh\nexit 0\n' > "$case_dir/dist/AgentIsland.app/Contents/MacOS/agentisland"
    chmod +x "$case_dir/dist/AgentIsland.app/Contents/MacOS/agentisland"
    chmod +x "$case_dir"/shims/*
}

# 用沙箱里的真实脚本 + 假命令跑一次；注意 restart-app.sh 读 OSASCRIPT 覆盖点，
# 否则它写死的 /usr/bin/osascript 会绕过沙箱、真的去操作用户的应用。
run_case() {
    local case_dir="$1"
    ( cd "$case_dir" && env CASE_STATE="$case_dir/state" \
        OSASCRIPT="$case_dir/shims/osascript" \
        PATH="$case_dir/shims:$PATH" bash scripts/restart-app.sh )
}

echo "== ① osascript 挂住时必须被超时打断，并明说收尾可能未执行 =="
CASE=$(mktemp -d "$SANDBOXES/hang.XXXXXX")
make_case "$CASE"
echo 111 > "$CASE/state/pid"
cat > "$CASE/shims/osascript" <<'SHIM'
#!/bin/sh
echo osascript-called >> "$CASE_STATE/actions"
sleep 600
SHIM
chmod +x "$CASE/shims/osascript"
START=$(date +%s)
OUT=$(run_case "$CASE" 2>&1)
ELAPSED=$(( $(date +%s) - START ))
ACTIONS=$(cat "$CASE/state/actions" 2>/dev/null || true)

printf '%s' "$ACTIONS" | grep -q osascript-called \
    && ok "优雅退出确实先被尝试（osascript 被调用）" \
    || fail "osascript 没有被调用（优雅退出路径未走）"
printf '%s' "$OUT" | grep -q '优雅退出未在 8 秒内完成' \
    && ok "明说本次写盘收尾可能未执行" \
    || fail "未说明写盘收尾可能未执行"
[[ "$ELAPSED" -lt 60 ]] \
    && ok "未被无限挂住（${ELAPSED}s，含 10s 超时上限）" \
    || fail "被 osascript 挂住了（${ELAPSED}s）——超时未生效"
printf '%s' "$ACTIONS" | grep -q pkill \
    && ok "超时后仍以信号兜底，流程没卡死" \
    || fail "超时后没有兜底，流程卡死"
rm -rf "$CASE"

echo "== ② 优雅退出生效时：不告警、等待真的跑过、能报出新实例 =="
CASE=$(mktemp -d "$SANDBOXES/graceful.XXXXXX")
make_case "$CASE"
echo 222 > "$CASE/state/pid"
cat > "$CASE/shims/osascript" <<'SHIM'
#!/bin/sh
echo osascript-called >> "$CASE_STATE/actions"
rm -f "$CASE_STATE/pid"     # 模拟应用自己走完退出链
exit 0
SHIM
chmod +x "$CASE/shims/osascript"
OUT=$(run_case "$CASE" 2>&1); RC=$?
ACTIONS=$(cat "$CASE/state/actions" 2>/dev/null || true)
CALLS=$(cat "$CASE/state/pgrep.calls" 2>/dev/null || echo 0)

printf '%s' "$ACTIONS" | grep -q osascript-called \
    && ok "优雅退出被尝试" \
    || fail "优雅退出未被尝试"
printf '%s' "$OUT" | grep -q '优雅退出未在 8 秒内完成' \
    && fail "优雅退出已成功却仍告警" \
    || ok "优雅退出成功时不告警（未静默降级为信号终止）"
[[ "$CALLS" -ge 2 ]] \
    && ok "等待循环真的跑过（pgrep 调用 ${CALLS} 次）" \
    || fail "等待循环没有跑（pgrep 仅 ${CALLS} 次）"
[[ "$RC" == "0" ]] \
    && ok "重启成功（退出码 0）" \
    || fail "重启失败（退出码 ${RC}）：${OUT}"
printf '%s' "$OUT" | grep -q '已重启：pid' \
    && ok "报出新实例 pid" \
    || fail "没有报出新实例 pid：${OUT}"
rm -rf "$CASE"

echo "== ③ 旧实例杀不掉时必须明确失败，而不是假称重启成功 =="
CASE=$(mktemp -d "$SANDBOXES/stuck.XXXXXX")
make_case "$CASE"
echo 333 > "$CASE/state/pid"
cat > "$CASE/shims/osascript" <<'SHIM'
#!/bin/sh
exit 0
SHIM
cat > "$CASE/shims/pkill" <<'SHIM'
#!/bin/sh
exit 0        # 信号发出但进程不消失（模拟卡住）
SHIM
chmod +x "$CASE/shims/osascript" "$CASE/shims/pkill"
OUT=$(run_case "$CASE" 2>&1); RC=$?
[[ "$RC" != "0" ]] \
    && ok "旧实例未消失时非零退出（码 ${RC}）" \
    || fail "旧实例仍在却返回 0——又是假保证"
printf '%s' "$OUT" | grep -q '旧实例仍在运行' \
    && ok "明确指出重启没有发生" \
    || fail "没有给出失败原因：${OUT}"
rm -rf "$CASE"

echo "== ④ 脚本语法 =="
bash -n "$REPO/scripts/restart-app.sh" \
    && ok "restart-app.sh 语法通过" \
    || fail "restart-app.sh 语法错误"

printf '\n'
if [[ "$FAILED" == "0" ]]; then
    echo "restart-app: 全部断言通过"
else
    echo "restart-app: 有断言失败"
fi
exit "$FAILED"
