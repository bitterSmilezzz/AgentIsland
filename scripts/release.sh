#!/bin/bash
# 一条命令走完发版：脱敏门禁 → 测试与打包 → 提交 → 打 tag → 推送 → GitHub Release。
#
# 为什么要有这个脚本：AGENTS.md 写的是「测试通过就自动提交并发版」，但这条链路此前靠
# 手工执行，结果 v0.0.81..v0.0.118 共 38 个版本提交推上去了、tag 与 release 却没建。
# 脚本把顺序钉死，且任何一步失败即停 —— 尤其是不许在扫描之前 commit。
#
# 用法：scripts/release.sh <X.Y.Z> "<一句话标题>" [notes 文件]
#   notes 缺省时取 CHANGELOG 里该版本那一节。
# 环境变量：RELEASE_FILES_FILE 必填，UTF-8 文本，每行一个仓库根目录相对文件路径。
#   例：RELEASE_FILES_FILE=.scratch/release-files.txt scripts/release.sh ...
#   清单必须显式列出本轮文件（含删除）；不接受空行、目录或重复路径。
#   SKIP_SCAN=1 跳过脱敏扫描（只用于本地试跑，等于自废门禁，输出里会标出来）

release_staging_error() { printf '✗ 发布文件清单：%s\n' "$*" >&2; return 1; }

# 只读预检；也在暂存前调用，避免构建期间新增的无关暂存内容被提交。
release_validate_files() {
    local path tracked prior staged found
    [[ "${RELEASE_FILE_COUNT:-0}" -gt 0 ]] || { release_staging_error "清单为空"; return 1; }
    git rev-parse --show-toplevel >/dev/null 2>&1 || { release_staging_error "不在 Git 仓库中"; return 1; }
    for path in "${RELEASE_FILES[@]}"; do
        case "$path" in
            ''|/*|[A-Za-z]:*|*\\*|*$'\r'*|*$'\n'*|*$'\t'*)
                release_staging_error "不是有效的仓库相对文件路径：$path"; return 1 ;;
        esac
        case "/$path/" in
            *'/../'*|*'/./'*|*'//'*|*'/.'[gG][iI][tT]'/'*)
                release_staging_error "禁止 ..、.、空路径段与 .git 路径：$path"; return 1 ;;
        esac
        [[ ! -d "$path" ]] || { release_staging_error "不能列出目录：$path"; return 1; }
        # literal pathspec 防止 *、?、[ 或 :(...) 扩大到清单外的文件。
        tracked=0
        while IFS= read -r -d '' prior; do
            # ls-files 对已删除目录也会返回子文件；只能接受精确的 tracked 文件。
            [[ "$prior" == "$path" ]] || { release_staging_error "不是单个文件：$path"; return 1; }
            tracked=1
        done < <(git --literal-pathspecs ls-files -z -- "$path")
        [[ "$tracked" == 1 || -f "$path" || -L "$path" ]] || {
            release_staging_error "文件不存在且未被跟踪：$path"; return 1;
        }
        # 不用 check-ignore（它不接受 --literal-pathspecs）；被忽略且未跟踪的文件
        # 会出现在 --others --ignored 列表里。
        if [[ -n "$(git --literal-pathspecs ls-files -z --others --ignored --exclude-standard -- "$path")" ]]; then
            release_staging_error "文件被 gitignore 忽略：$path"; return 1
        fi
    done
    # 禁用 rename 合并，确保改名涉及的旧路径和新路径都在清单里。
    while IFS= read -r -d '' staged; do
        found=0
        for prior in "${RELEASE_FILES[@]}"; do
            [[ "$staged" != "$prior" ]] || { found=1; break; }
        done
        [[ "$found" == 1 ]] || { release_staging_error "索引已有清单外路径（索引未改动）：$staged"; return 1; }
    done < <(git diff --cached --name-only --no-renames -z)
}

release_files_preflight() {
    local manifest="${RELEASE_FILES_FILE:-}" path prior
    [[ -n "$manifest" ]] || { release_staging_error "必须设置 RELEASE_FILES_FILE，禁止自动收取工作区改动"; return 1; }
    [[ -f "$manifest" && -r "$manifest" ]] || { release_staging_error "清单不是可读文件：$manifest"; return 1; }
    if IFS= read -r -d '' path < "$manifest"; then
        release_staging_error "清单须逐行列出路径，不能含 NUL"; return 1
    fi
    RELEASE_FILES=()
    RELEASE_FILE_COUNT=0
    while IFS= read -r path || [[ -n "$path" ]]; do
        [[ -n "$path" ]] || { release_staging_error "不能有空行"; return 1; }
        if [[ "$RELEASE_FILE_COUNT" -gt 0 ]]; then
            for prior in "${RELEASE_FILES[@]}"; do
                [[ "$prior" != "$path" ]] || { release_staging_error "重复路径：$path"; return 1; }
            done
        fi
        RELEASE_FILES[RELEASE_FILE_COUNT]="$path"
        RELEASE_FILE_COUNT=$((RELEASE_FILE_COUNT + 1))
    done < "$manifest"
    release_validate_files
}

release_stage_files() {
    release_validate_files || return 1
    git --literal-pathspecs add -- "${RELEASE_FILES[@]}"
}

# 测试直接 source 生产函数；不启动发布，也不改变调用者的目录或 shell 选项。
if [[ "${BASH_SOURCE[0]}" != "$0" ]]; then
    return 0
fi

set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
. "$SCRIPT_DIR/common.sh"
cd "$SCRIPT_DIR/.."

VERSION="${1:-}"
TITLE="${2:-}"
NOTES_FILE="${3:-}"

die() { echo "✗ $*" >&2; exit 1; }
step() { printf '\n\033[1m==> %s\033[0m\n' "$*"; }

[[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "用法: $0 <X.Y.Z> \"<标题>\" —— 版本号形如 0.0.119"
[[ -n "$TITLE" ]] || die "缺 release 标题（CHANGELOG 首条的那句话）"

step "显式发布文件清单预检"
release_files_preflight || exit 1

step "版本三处一致性预检 v${VERSION}"
python3 scripts/check-version.py "$VERSION"
[[ -z "$(git tag -l "v$VERSION")" ]] || die "tag v$VERSION 已存在，别重复发版"
git rev-parse --verify --quiet "HEAD" >/dev/null || die "没有提交历史"
BRANCH=$(git rev-parse --abbrev-ref HEAD)
[[ "$BRANCH" == "main" ]] || die "当前在 ${BRANCH}，发版要求在 main"
MERGED=$(git diff --name-only --diff-filter=U | wc -l | tr -d ' ')
[[ "$MERGED" == "0" ]] || die "有 $MERGED 个未解决冲突，先解冲突"

if [[ "${SKIP_SCAN:-0}" == "1" ]]; then
    step "脱敏扫描被 SKIP_SCAN=1 跳过 —— 这个产物不得对外发布"
else
    step "密钥 / 个人信息扫描（含全部 git 对象）"
    scripts/scan-secrets.sh --release
fi

step "脱敏守护与 Rust 回归测试"
bash scripts/test-release-staging.sh
scripts/test-scan-secrets.sh
python3 scripts/test-version.py
python3 scripts/test-native-gates-vm.py
python3 scripts/test-memory-soak.py
python3 scripts/test-memory-soak-result.py
python3 scripts/test-resource-soak.py
python3 scripts/test-measure-memory.py
python3 scripts/test-native-motion-result.py
node scripts/test-native-motion.mjs
node scripts/test-first-snapshot.mjs
node scripts/test-tool-budgets.mjs
node scripts/test-usage-context.mjs
node scripts/test-interface-models.mjs
node scripts/test-model-directory.mjs
node scripts/test-window-lifecycle.mjs
node scripts/test-page-motion.mjs
node scripts/test-task-focus.mjs
node scripts/test-tasks.mjs

step "纯数据 UI 回归（无浏览器 / 无 VM / 无外部客户端）"
# 这 18 条只 import app/ui/js 下的模块并对返回值断言：DOM 与 IPC 都是脚本内的桩，
# 不连网络、不写用户目录、不需要真实客户端或 VM，任意机器上裸跑 `node` 即可。
# 它们此前不在任何门禁里（其中 8 条连文档都没提过），也就没人发现断言是否还成立。
if ! command -v node >/dev/null 2>&1; then
    die "纯数据 UI 回归需要 node（只用 Node 内置模块，无需额外依赖；仓库实测 Node 24）"
fi
for ui_test in test-agent-actions test-agent-identities test-functional-ui test-island-click \
                test-island-content-motion test-island-deeplink test-island-motion test-island-return \
                test-prompts test-provider-feedback test-quick-navigation test-report-panel \
                test-sessions test-skills-package test-task-attention test-usage-trend \
                test-window-layout test-workspaces; do
    node "scripts/$ui_test.mjs"
done

# 同一批桩测试的**壳变体**：workbench 是默认桌面形态，main.js 在那里走另一条启动分支
# （renderWorkbench + 快捷导航对话框 + 草稿守卫），island 变体整条都碰不到；冷启动变体
# 则覆盖「启动瞬间深链批次到达」的消费顺序。三条都只多花 0.1~0.2 秒，也不需要浏览器。
# 此前它们只被手工跑过——`test-functional-ui` 的 workbench 变体因此一直没人发现它的桩
# 缺了 dialog 结构（`quick-navigation.js` 在壳里崩，产品代码没有错）。
TEST_NAVIGATION_SHELL=workbench node scripts/test-functional-ui.mjs
TEST_NAVIGATION_SHELL=workbench node scripts/test-island-deeplink.mjs
TEST_COLD_NAVIGATION=1 node scripts/test-island-deeplink.mjs

step "浏览器 UI 回归（会话刷新 / 任务详情双栏 / 正文离页）"
# 这三条要真实浏览器（Playwright），不能裸跑 `node`。缺运行时时明确失败，不许
# 静默跳过——过去它们不在任何门禁里，v0.0.321 起 committed 测试对 committed
# 产品变红（窄窗口隐藏列表让旧断言超时）也没人发现。
if [[ -z "${TEST_BROWSER_MODULE:-}" ]] && ! node -e "require.resolve('playwright')" >/dev/null 2>&1; then
    die "浏览器 UI 回归需要 Playwright：先 npm i playwright，或设 TEST_BROWSER_MODULE 指向 playwright/index.mjs 的绝对路径"
fi
for browser_test in test-session-refresh-focus test-task-detail-navigation test-task-reader-leave; do
    node "scripts/$browser_test.mjs"
done

cargo test --locked --manifest-path app/src-tauri/Cargo.toml

step "Rust/Tauri 打包"
# Rust 全量回归已在上一阶段完成。
export SKIP_TESTS=1

scripts/build-app.sh "$VERSION"
# 构建成功即替换运行实例，后续验收/发布失败也不遗留旧实例。
scripts/restart-app.sh

step "随包 CLI 派发参数验证（受控系统命令，不打开窗口）"
python3 scripts/test-cli-dispatch.py --binary dist/agentisland
python3 scripts/test-cli-usage.py --binary dist/agentisland

# VM 路径核实虚拟化、Aqua、系统 build 与产物身份，不以一个旗标冒充隔离。
if [[ -n "${NATIVE_TEST_VM:-}" ]]; then
    step "隔离 macOS VM 原生验收（UI / Dock / 单实例 / CLI / 窗口释放）"
    # On macOS the system Python can read Tart's local ARP metadata; a separately
    # installed interpreter may be denied by Local Network privacy (empty ARP).
    # 来宾账号必须显式给出：native-gates-vm.py 的 --user 没有默认值，缺了它只会在
    # 整条门禁跑完之后才失败，白跑一遍全量测试与打包。
    [[ -n "${NATIVE_TEST_USER:-}" ]] || die "原生 VM 验收需要 NATIVE_TEST_USER（来宾短用户名，如 admin）"
    /usr/bin/python3 scripts/native-gates-vm.py --vm "$NATIVE_TEST_VM" --user "$NATIVE_TEST_USER"
elif [[ "${ALLOW_DESKTOP_UI:-0}" == "1" ]]; then
    echo "⚠ ALLOW_DESKTOP_UI=1：操作者已明确接受当前桌面的焦点干扰"
    step "当前桌面原生验收（显式授权）"
    scripts/ui-smoke.sh --isolated-session
    python3 scripts/test-dock-presence.py --isolated-session
    python3 scripts/test-app-instance.py --cold
    python3 scripts/test-cli-native.py --allow-desktop
    CARGO_TARGET_DIR="$PWD/.build/window-lifetime-target" cargo build --locked --quiet --manifest-path scripts/window-lifetime-probe/Cargo.toml
    .build/window-lifetime-target/debug/agentisland-window-lifetime-probe --allow-desktop
else
    die "原生验收需要 NATIVE_TEST_VM 指定真实隔离 VM；未运行原生门禁，不发布"
fi

step "发布包"
ZIP="dist/AgentIsland-$VERSION.zip"
rm -f "$ZIP"
( cd dist && zip -qry "AgentIsland-$VERSION.zip" AgentIsland.app agentisland )
ls -lh "$ZIP" | awk '{print "    " $9 "  " $5}'

step "提交"
# 清单已在任何发布副作用之前预检；此处复核后仅暂存显式文件（含 tracked 删除）。
release_stage_files || exit 1
STAGED=$(git diff --cached --name-only)
[[ -n "$STAGED" ]] || die "没有可提交的改动（版本一致性预检都过了，说明文档没同步）"
echo "$STAGED" | sed 's/^/    /'
git commit -q -m "release: v$VERSION - $TITLE"

# 提交**之后**再扫一次历史：上面那次 `--release` 跑在提交之前，本次新增的 blob
# 还没进历史，于是「工作区靠 nosec 放行、历史这一关却要按对象备案」的差异
# 要到下一轮才暴露（v0.0.174 真发生过：发版成功，下一次冷扫描才发现 1 个
# url_token_param blob 未核准）。放在 tag/push 之前，失败就还没推出去。
step "提交后复扫 git 历史（新 blob 也要过这一关）"
scripts/scan-secrets.sh --history

step "打 tag 并推送"
git tag -a "v$VERSION" -m "v$VERSION — $TITLE"
git push -q origin main
git push -q origin "v$VERSION"

step "GitHub Release"
if [[ -z "$NOTES_FILE" ]]; then
    NOTES_FILE=$(mktemp)
    # 取 CHANGELOG 里 `## [版本]` 到下一条 `## [` 之间的正文。版本号里有方括号，
    # 所以走前缀比对而不是把版本号拼进正则
    HEAD="## [$VERSION]"
    awk -v head="$HEAD" '
        BEGIN { p = 0 }
        p == 0 && substr($0, 1, length(head)) == head { p = 1; next }
        p == 1 && /^## \[[0-9]/ { p = 0 }
        p == 1 { print }
    ' CHANGELOG.md > "$NOTES_FILE"
    [[ -s "$NOTES_FILE" ]] || die "CHANGELOG 里取不出 v$VERSION 那一节的正文"
    printf '\n下载地址：`AgentIsland-%s.zip`（未公证，首次打开需右键 → 打开）。\n' "$VERSION" >> "$NOTES_FILE"
fi
if gh release view "v$VERSION" >/dev/null 2>&1; then
    gh release edit "v$VERSION" --title "v$VERSION: $TITLE" --notes-file "$NOTES_FILE"
else
    gh release create "v$VERSION" "$ZIP" --title "v$VERSION: $TITLE" \
        --notes-file "$NOTES_FILE" --latest
fi

step "重启应用（AGENTS.md：出包后无感替换旧实例）"
# 此前这里是 `pkill -x AgentIsland; sleep 0.6; open dist/AgentIsland.app`，
# 而 `pkill -x AgentIsland` **打不中任何进程**（进程名是小写 `agentisland`）——
# 也就是说发版末尾的「重启」从来没发生过，脚本却照常打印这一步。收进共用脚本，
# 由它校验「旧实例确实没了、新实例确实起来了」。
scripts/restart-app.sh

printf '\n\033[32m✓ v%s 已发布：提交已推送、tag 已打、release 已建\033[0m\n' "$VERSION"
# 附件复核。注意 `releases/tags/…` 这个端点会短暂返回空 assets（实测刚传完读到 0，
# 按 id 读是 1），所以只重试着读、不做「读不到就重传」——重传会撞 already exists，
# 反而把一次正常发版报成失败。读不到只打警告，人去看一眼比脚本猜更靠得住
for _ in 1 2 3 4 5; do
    ASSETS=$(gh release view "v$VERSION" --json assets -q '[.assets[].name] | join(", ")' 2>/dev/null || true)
    [[ -n "$ASSETS" ]] && break
    sleep 2
done
if [[ -n "$ASSETS" ]]; then
    echo "    v$VERSION  附件: $ASSETS"
else
    echo "⚠ 附件复核没读到（GitHub 的 tag 端点有短暂一致性问题）。手工确认：" >&2
    echo "  gh api repos/\$(gh repo view --json owner -q .owner.login)/AgentIsland/releases/tags/v$VERSION -q '.assets[].name'" >&2
fi
