#!/bin/bash
# 打开 Rust/Tauri 端并把它的启动痕迹留在屏幕上，等着看。
#
# 为什么需要这个脚本：Rust 端的界面只能靠**眼睛**确认有没有渲染，而我这边的
# 证据链到「窗口建好了、URL 对、前端已嵌」为止，再往后（页面加载完成没有、
# JS 有没有回调）那两条日志**一直是空的**——而那两条恰恰是 webview 自己写的，
# 所以它们空着的时候无法自证。
#
# 用法：
#   ./scripts/trace-rust-app.sh          # 打开应用并把痕迹打到终端
#   ./scripts/trace-rust-app.sh --quiet  # 只打开，痕迹写进 /tmp/agentisland-rust-trace.log
set -u
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
APP="$ROOT/app/src-tauri/target/release/bundle/macos/AgentIsland.app"
# 日志文件名是**应用写死的**（`std::env::temp_dir()/agentisland-tauri.log`），
# 不是一个可以自定义的路径。第一版这里写了自己的文件名，于是永远 grep 空的——
# 而「没有 [page] 行」这个结论看起来还挺像真的，差点把自己骗过去。
LOG="/tmp/agentisland-tauri.log"

if [[ ! -x "$APP/Contents/MacOS/agentisland" ]]; then
    echo "Rust 端还没构建。先跑："
    echo "  (cd app/src-tauri && cargo tauri build --bundles app)"
    exit 1
fi

# 两个 app 的 bundle id 相同，同时开着会互相抢——先退掉 Swift 那个
pkill -x AgentIsland 2>/dev/null && echo "已退出 Swift 版（bundle id 相同，不能同时开）"
sleep 1
: > "$LOG"   # 截断而不是改名

TMPDIR=/tmp nohup "$APP/Contents/MacOS/agentisland" >>/tmp/agentisland-rust-console.log 2>&1 &
echo "已启动 Rust 端。窗口出现后请看一眼这三处："
echo "  ① 灵动岛（顶部）      ② 侧边栏（托盘菜单 → 切换形态）  ③ 托盘 → 打开工作台"
echo
echo "启动痕迹（[boot] = 窗口建好了；[page] = 页面加载完成；[webview] = JS 回调过 Rust）："
SEEN=0
for _ in $(seq 1 20); do
    sleep 2
    # 只打**新出现的**行：第一版每 2 秒把同一批 `[boot]` 重打一遍，
    # 20 轮下来 90 行里 80 行是重复，���真正的新信息只有 6 行。
    TOTAL=$(grep -cE '^\[boot\]|^\[page\]|^\[webview\]' "$LOG" 2>/dev/null || echo 0)
    if [[ "$TOTAL" -gt "$SEEN" ]]; then
        grep -E '^\[boot\]|^\[page\]|^\[webview\]' "$LOG" 2>/dev/null | tail -n $((TOTAL - SEEN))
        SEEN=$TOTAL
    fi
    grep -qE '^\[page\]' "$LOG" 2>/dev/null && break
done
echo
echo "完整痕迹：$LOG"
if ! grep -qE '^\[page\]' "$LOG" 2>/dev/null; then
    echo "⚠️  没有 [page] 行 ⇒ 页面始终没有加载完成。这与「窗口有没有显示」是两件事，"
    echo "    请告诉我是「窗口根本没出现」还是「出现了但空白」——两者的修法完全不同。"
fi
