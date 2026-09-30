#!/bin/bash
# 两个脚本共用的常量。**这里必须是唯一一处**。
#
# 起因（v0.0.266）：`release.sh` 在自己体内写 `${APP_NAME}`，而 `APP_NAME` 是
# `build-app.sh` 的变量——**跨进程取不到**。两处都有 `set -u`，于是发版链在
# 「自动 SKIP_SWIFT」那段直接中止（`APP_NAME: unbound variable`）。
#
# 抽出来不是洁癖：那类 bug **已经犯过两次**了。上一回是 `$VAR` 后面紧跟中文标点
# （`build-app.sh` 的注释里写着「这个坑踩了两次」），修完没留门禁，
# 结果换个文件又出现 6 处，其中一处当场卡死发版。
# 一份文件 + 一条守护，才叫修完。
#
# 用法：`SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"` 之后 `. "$SCRIPT_DIR/common.sh"`。
# 不在 `cd` 之后取——脚本会先切到仓库根，那时相对路径已经变了。

APP_NAME="AgentIsland"

# ── Swift 工具链探测 ────────────────────────────────────────────────
#
# 同样**只此一份实现**。此前 `release.sh` 与 `build-app.sh` 各写了一遍
# `xcode-select -p` + 插件目录，于是 v0.0.266 只修好 release.sh 那一侧，
# build-app.sh 那边仍按旧规则要求两个 flag 同时为 1 —— 发版链**第三次**
# 死在同一段 `if` 块里。重复实现就是这种「改了一处忘了另一处」的来源。

swift_developer_dir() {
    xcode-select -p 2>/dev/null || echo /Library/Developer/CommandLineTools
}

# 插件目录必须**跟着 `xcode-select -p` 走**。写死 CommandLineTools 的话，
# 装好 Xcode 并 `xcode-select -s` 之后这里仍然找不到插件 → 误报成「没装 Xcode」。
#
# ⚠️ 这里用 `grep -c` 而不是 `grep -q`：`grep -q` 命中即退出，
# 上游 `ls` 会吃到 SIGPIPE，而脚本开着 `set -o pipefail`，
# 于是**明明有插件也会被算成没有**。这条在本机验不到（没装 Xcode），
# 是从 `pipefail` + SIGPIPE 的语义推出来的，改动时留意。
swiftui_macros_available() {
    local plugin_dir count
    plugin_dir="$(swift_developer_dir)/usr/lib/swift/host/plugins"
    [[ -d "$plugin_dir" ]] || return 1
    count=$(ls "$plugin_dir" 2>/dev/null | grep -c SwiftUIMacros) || true
    [[ "$count" -gt 0 ]]
}
