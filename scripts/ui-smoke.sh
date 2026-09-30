#!/bin/bash
# UI 冒烟：把三个窗口里所有可点的元素点一遍，检查有没有跑出错误。
#
# 为什么要有它：v0.0.242 之前界面从来没渲染过，UI 这一层是完全没被验证过的。
# 而历史上真出过「导航项在、注水函数在、页面函数压根没定义」的白屏——
# 那一类只有真的点进去才会现形。静态检查扫不到，冒烟也扫不到（它只看有没有报错）。
#
# 判定标准（都在这里，不散落到驱动脚本里）：
#   1. 三个窗口各自跑完（state == done）
#   2. 全程零未捕获错误（errs 为空）
#   3. 没有「点不动」的步骤
#
# 用法：scripts/ui-smoke.sh [--keep-log]
set -uo pipefail
cd "$(dirname "$0")/.."

LOG="${TMPDIR:-/tmp}agentisland-tauri.log"
RUN_MS="${UI_SMOKE_RUN_MS:-15000}"

# 用哪个二进制：默认取**新一点**的那个。发布产物比源码旧时（刚改完还没重打包）
# 会自动退回 debug——否则会拿一个不带 --ui-smoke 的二进制去跑，冒烟静默不执行。
# 要指定就设 UI_SMOKE_BIN。
if [[ -n "${UI_SMOKE_BIN:-}" ]]; then
    BIN="$UI_SMOKE_BIN"
else
    REL="dist/AgentIsland.app/Contents/MacOS/agentisland"
    DBG="app/src-tauri/target/debug/agentisland"
    if [[ -x "$DBG" && ( ! -x "$REL" || "$DBG" -nt "$REL" ) ]]; then
        BIN="$DBG"
    elif [[ -x "$REL" ]]; then
        BIN="$REL"
    else
        BIN="$DBG"
    fi
    # ⚠️ **光比新旧不够**：普通 `cargo build` **不跟踪 `app/ui/`**——
    # 只改前端文件再 `cargo build`，二进制**不会重新嵌入资源**，
    # 冒烟于是拿旧资源去验，**验的东西不是当前代码**。
    # （`cargo tauri build` 会跟踪；`build-app.sh` 走的是那条。）
    # 所以这里额外要求：二进制必须比 `app/ui/` 下最新的文件新。
    if [[ -x "$BIN" ]]; then
        NEWEST_UI=$(find app/ui -type f -newer "$BIN" 2>/dev/null | head -1)
        if [[ -n "$NEWEST_UI" ]]; then
            echo "✗ $BIN 比前端文件旧（$NEWEST_UI 还没嵌进去）" >&2
            echo "  原因：普通 \`cargo build\` 不跟踪 app/ui/，只改前端不会重新嵌入资源。" >&2
            echo "  修法：用 \`cargo tauri build --bundles app\`（build-app.sh 走的就是那条）" >&2
            echo "       或本就跑 ./scripts/build-app.sh 让 dist/AgentIsland.app 变新。" >&2
            exit 1
        fi
    fi
fi
[[ -x "$BIN" ]] || { echo "✗ 找不到可执行文件（${BIN}）——先构建" >&2; exit 1; }

echo "==> UI 冒烟：$BIN --ui-smoke"
# **先把上一次的残留实例清掉**，再开跑。
# 不清会有两个实例同时往同一个累积日志里写，证据就搅浑了——
# 而且新实例会撞上「127.0.0.1:42000 绑定失败」，
# 于是 `/__probe__` 那些兜底请求记到了**别人**的日志里（实测踩过）。
# 端口被占这件事本身不影响冒烟（驱动与读回都走 eval，不走 HTTP），
# 但它会让日志里的证据不再可信。
pkill -x agentisland 2>/dev/null
sleep 1

# 日志是累积的（同一路径按运行叠加），先清空，否则会把历史遗留的
# `[smoke]` 行当成本次结果 —— 这个坑在 main.rs 的 [run] 注释里记过一次。
: > "$LOG"

"$BIN" --ui-smoke --expand --route=todo >/dev/null 2>&1 &
PID=$!
trap 'kill $PID 2>/dev/null' EXIT

# 等它跑完（冒烟 1.2s 开跑、约 5~6s 点完、8s 后读回，留 3s 余量）
sleep "$(( RUN_MS / 1000 ))"
kill $PID 2>/dev/null
wait $PID 2>/dev/null

# 让用户手边的岛还在
pkill -x agentisland 2>/dev/null
if [[ -d "dist/AgentIsland.app" ]]; then
    open dist/AgentIsland.app 2>/dev/null || true
fi

echo "==> 冒烟结果"
# 优先用**驱动自报**的结果（`SMOKE_RESULT`）——它没有定时读回那个竞态。
# `island/smoke` 那一行是固定时刻的 eval 读回，**只当诊断**，不作判据。
SMOKE=$(grep '\[webview\] SMOKE_RESULT' "$LOG" | sed 's/^\[webview\] //' || true)
[[ -n "$SMOKE" ]] || SMOKE=$(grep '\[smoke\]' "$LOG" || true)
if [[ -z "$SMOKE" ]]; then
    echo "✗ 一行 [smoke] 都没有——冒烟压根没跑起来"
    echo "  日志里前 20 行："
    head -20 "$LOG" | sed 's/^/    /'
    exit 1
fi

echo "$SMOKE" | sed 's/^/    /'

# `eval_with_callback` 的回调拿到的是**被 JSON 编码过的字符串**，
# 于是日志里长这样：`sidebar/smoke "{\"state\":\"done\"…}"`。
# 直接拿原文去匹配 `"state":"done"` 一条都中不了——先把反斜杠去掉再判。
FLAT=$(echo "$SMOKE" | tr -d '\\')

fail=0
for label in island sidebar workbench; do
    line=$(echo "$FLAT" | grep -F "SMOKE_RESULT ${label} " || true)
    if [[ -z "$line" ]]; then
        echo "✗ ${label}：没有结果"
        fail=1
        continue
    fi
    if ! echo "$line" | grep -q '"state":"done"'; then
        echo "✗ ${label}：没跑完 —— $(echo "$line" | grep -o '"state":"[^"]*"' | head -1)"
        fail=1
    fi
    if echo "$line" | grep -q '"errs":\[[^]]'; then
        echo "✗ ${label}：有未捕获错误 —— $(echo "$line" | grep -o '"errs":\[[^]]*\]' | head -1)"
        fail=1
    fi
    if echo "$line" | grep -q '"err":"'; then
        echo "✗ ${label}：有元素点不动 —— $(echo "$line" | grep -o '"label":"[^"]*点不动"' | head -1)"
        fail=1
    fi
    # 点到了几个可点元素。**一个都没点到**时上面三条照样会「通过」——
    # state 仍是 done、errs 仍是空、步骤里也没有点不动的记录。
    # 所以这条得单独判，否则「三扇门后面都空着」会被算成绿灯。
    #
    # 灵动岛**同样适用**：冒烟会先发一条 `tray://toggle`（点托盘图标那条真实路径）
    # 把岛展开，所以它应该有卡片里的可点元素。此前这里对灵动岛开了豁免，
    # 理由是「收起态本来就没有可点元素」——现在驱动会先展开，豁免也就没必要了。
    # 搜索这一步**只跑、不判**。曾经想判「过滤是否生效」，两次都失败：
    # 第一次判据写成 `indexOf('empty') >= 0`（会匹配任何含 empty 的子串，假阳性）；
    # 第二次收紧成只认「未找到匹配」——**去掉输入事件照样通过**，
    # 因为本机没有在跑的 Agent，空态与「过滤生效」**无法区分**。
    # 证不了失败的门禁比没有门禁更糟（它看起来像覆盖），所以只保留这一步本身：
    # 它仍能抓到崩溃（异常进 errs），但不假装验过什么。
    # 工作台监控栏在**没有 Agent 在跑**时必须给出空态文案。
    #
    # 这条是被拍图引出来的：整窗截图里左栏看着是空的，而代码写着
    # `还没有检测到运行中的智能体`。分不清「没进 DOM」（bug）与「太暗看不见」（观感）——
    # 直接问 DOM 就分清了：**元素在、52px 高、文案正确** ⇒ 不是缺陷。
    # 判据因此只钉「在不在」，不钉「看不看得清」——后者是观感，不该由门禁裁决。
    # **只有工作台有监控栏**——灵动岛与侧边栏压根没有 `[data-wb-monitor]`，
    # 它们报 `emptyEl:false` 是正确的。第一版没按形态收窄，于是三个窗口一起判失败。
    # **启动路由必须生效**：脚本带 `--route=todo` 起进程，
    # 所以高亮的导航项**不该**是第一项（`监控`）。逐个点击覆盖了页面，
    # 但「启动即落在那一页」是另一条分支，从没被单独验过。
    if [[ "$label" == "sidebar" ]] && echo "$line" | grep -q '"activeNav"'; then
        active=$(echo "$line" | grep -o '"activeNav":"[^"]*"' | head -1 | sed 's/.*:"//; s/"$//')
        first=$(echo "$line" | grep -o '"firstNav":"[^"]*"' | head -1 | sed 's/.*:"//; s/"$//')
        if [[ -z "$active" ]]; then
            echo "✗ ${label}：没有导航项被高亮 ⇒ 整页路由没生效"
            fail=1
        elif [[ "$active" == "$first" ]]; then
            echo "✗ ${label}：带了 --route=todo，落点却还是首项「${first}」⇒ 启动路由被吞了"
            fail=1
        else
            echo "    ${label}  启动路由生效：落点「${active}」（首项是「${first}」）"
        fi
    fi
    if [[ "$label" == "workbench" ]] && echo "$line" | grep -q '"emptyState"'; then
        if ! echo "$line" | grep -q '"emptyEl":true'; then
            echo "✗ ${label}：监控栏没有 Agent 在跑，却没有空态文案 —— 一片空白会被看成「界面没加载」"
            fail=1
        else
            echo "    ${label}  监控栏空态在场（$(echo "$line" | grep -o '"box":[0-9-]*' | head -1 | tr -d '\n')）"
        fi
    fi
    if echo "$line" | grep -q '"inputFound":true'; then
        echo "    ${label}  搜索已走到（输入框 → 派发输入事件 → 重画）｜**过滤是否生效未验**，见驱动注释"
    fi
    found=$(echo "$line" | grep -o '"found":{[^}]*}' | grep -o ':[0-9]\+' | tr -d ':' | paste -sd+ - | bc 2>/dev/null || echo 0)
    echo "    ${label}  点到 ${found} 个可点元素"
    if [[ "$found" == "0" ]]; then
        echo "✗ ${label}：一个可点元素都没有 ⇒ 界面没渲染出来，或冒烟的展开/驱动没生效"
        fail=1
    fi
done

if [[ "$fail" != "0" ]]; then
    echo
    echo "✗ UI 冒烟未通过"
    [[ "${1:-}" == "--keep-log" ]] || cp "$LOG" /tmp/ui-smoke-fail.log 2>/dev/null
    exit 1
fi

echo
echo "✓ UI 冒烟通过（完整日志留在 ${LOG}）"
