#!/bin/bash
# 重启应用（AGENTS.md：「每次改动完成、构建出新产物后直接重启」）。
#
# ⚠️ **这里曾经是个假保证。** AGENTS.md 与 `release.sh` 都写的是
# `pkill -x AgentIsland`——但**进程名从来不是 `AgentIsland`**：
# 可执行文件叫 `agentisland`（小写），于是 `pkill -x` 精确匹配**一个都打不中**，
# 退出码 1。实测：`pkill -x AgentIsland` 退出码 = 1，而 `pgrep -x agentisland` 命中。
#
# 后果不是「偶尔没重启」——是**每次都没重启**，而脚本还打印了「重启应用」那一步。
# 应用之所以看起来更新了，是 `open` 撞上 bundle 被改过、Launch Services 自己重开的，
# 跟文档里写的那条命令没关系。**「本机跑过」与「看着在跑」都不证明它重启过。**
#
# 为什么用 `pkill -f` 匹配完整命令行、而不是 `pkill -x agentisland`：
# 后者按进程名匹配，而**用户自己跑的那个 CLI 也叫 `agentisland`**（`dist/agentisland`）。
# 只按名字杀会把用户正在跑的一次性命令一起带走。按应用内的完整路径匹配才只打应用。
#
# 两步都**校验结果**，任一步不符就非零退出：重启失败必须被看见，
# 不能像以前那样安静地什么都不做。
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
. "$SCRIPT_DIR/common.sh"
cd "$SCRIPT_DIR/.."

APP="dist/${APP_NAME}.app"
[[ -d "$APP" ]] || { echo "✗ 没有 ${APP}，先构建" >&2; exit 1; }

# ⚠️ 可执行文件名**不等于** bundle 名：bundle 是 `AgentIsland.app`，
# 而里面的二进制叫 **`agentisland`（小写）**。这正是 `pkill -x AgentIsland`
# 打不中任何进程的根因——`pkill -x` 匹配的是**进程名**，也就是这个文件名。
# 所以这里从 Info.plist 的 `CFBundleExecutable` 读权威值，不靠猜。
EXE=$(/usr/libexec/PlistBuddy -c "Print :CFBundleExecutable" "$APP/Contents/Info.plist" 2>/dev/null || true)
if [[ -z "$EXE" ]]; then
    # 读不到就退而求其次：MacOS 目录里只有一个可执行文件
    EXE=$(basename "$(find "$APP/Contents/MacOS" -maxdepth 1 -type f -perm -111 2>/dev/null | head -1)")
fi
[[ -n "$EXE" ]] || { echo "✗ 从 $APP/Contents/Info.plist 读不到 CFBundleExecutable" >&2; exit 1; }
CMDLINE="$APP/Contents/MacOS/$EXE"
[[ -x "$CMDLINE" ]] || { echo "✗ 包里没有可执行的 $CMDLINE" >&2; exit 1; }

# 主产物曾叫 AgentIsland-Rust.app；旧名必须一起清理，CLI 不在匹配范围。
APP_PATTERN='/AgentIsland(-Rust|-Swift)?[.]app/Contents/MacOS/(agentisland|AgentIsland)([[:space:]]|$)'
pkill -f "$APP_PATTERN" 2>/dev/null || true
for _ in 1 2 3 4 5 6 7 8 9 10; do
    pgrep -f "$APP_PATTERN" >/dev/null || break
    sleep 0.2
done
if pgrep -f "$APP_PATTERN" >/dev/null; then
    echo "✗ 旧实例仍在运行，重启没有发生：$(pgrep -f "$APP_PATTERN" | tr '\n' ' ')" >&2
    exit 1
fi

# 更新后在后台启动，避免验收与发版抢占用户正在操作的窗口。
open -g "$APP"

# 起：新实例必须在若干秒内出现，否则这仍是假保证
NEW=""
for _ in $(seq 1 40); do
    NEW=$(pgrep -f "$APP_PATTERN" | head -1)
    [[ -n "$NEW" ]] && break
    sleep 0.25
done
[[ -n "$NEW" ]] || { echo "✗ 打开后 10 秒内没看到新实例：${APP}" >&2; exit 1; }
echo "✓ 已重启：pid ${NEW}  ←  $(pwd)/${APP}"
