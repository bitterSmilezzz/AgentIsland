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

# ---------------------------------------------------------------------------
# 本轮未验项登记簿（RELEASE_UNVERIFIED）
#
# 发布链里有两类检查：
#   (a) 无条件跑，失败即中止（脱敏扫描、各测试、打包）；
#   (b) 需要外部环境的 opt-in 项——真机 VM、真实官方 CLI、独占来宾桌面。
# (b) 不能无条件跑（四边动效 4×75s 共约 5 分钟且独占桌面；官方客户端要 pinned CLI）。
#
# 但 (b) 也**不许静默跳过**。这两个脚本正是长期"不在门禁里"，断言改了没人发现
# （docs/测试映射盘点.md §3）。所以：未启用就登记一条未验项，并在推 tag 前汇总打印、
# 在最终成功消息里点名数量——宁可每轮都被提醒"这项没验"，也不许"为了全绿而跳过"。
#
# bash 3.2（macOS 自带 /bin/bash）在 `set -u` 下展开空数组会报 unbound variable，
# 所以所有展开都走 `${RELEASE_UNVERIFIED[@]+"${RELEASE_UNVERIFIED[@]}"}` 保护。
RELEASE_UNVERIFIED=()
unverified() {
    RELEASE_UNVERIFIED+=("$1")
    printf '⚠ 未验项：%s\n' "$1" >&2
}
report_unverified() {
    [[ "${#RELEASE_UNVERIFIED[@]}" -eq 0 ]] && return 0
    printf '\n\033[33m==> 本轮未验项 %d 项（本次发布不覆盖，不得据此声称全链路通过）\033[0m\n' \
        "${#RELEASE_UNVERIFIED[@]}"
    local item
    for item in ${RELEASE_UNVERIFIED[@]+"${RELEASE_UNVERIFIED[@]}"}; do
        printf '    - %s\n' "$item"
    done
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

step "源码门禁（逻辑、壳变体、浏览器、Rust 与框架自测）"
# 与 CI 共用 registry，避免新增测试只进一条执行链。缺运行时和未执行项均非零退出。
python3 scripts/automation/pipeline.py --profile ci --check-timeout 1800

step "Rust/Tauri 打包"
# Rust 全量回归已在上一阶段完成。
export SKIP_TESTS=1

scripts/build-app.sh "$VERSION"
# 构建成功即替换运行实例，后续验收/发布失败也不遗留旧实例。
scripts/restart-app.sh

step "构建身份与随包 CLI 验证（受控夹具，不打开窗口）"
# 先验构建收据（源码内容摘要 ↔ 产物哈希），再跑随包 CLI：旧 dist 不得冒充新构建。
# 这里不重复跑 ci——源码门禁已在打包前完成，重复跑只会拖长发版。
python3 scripts/automation/pipeline.py --profile package

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

# ---- 需外部环境的 opt-in 门禁（未启用即登记未验项，不许静默跳过）-----------
#
# 下面两项的共同点：**改了不会被发现**。docs/测试映射盘点.md §3 点名过——它们
# 一个只被 native-gates-vm.py 的 --motion-only 分支调用（release.sh 不传该旗标），
# 一个全仓零调用点。把断言改坏，现有门禁照样全绿。
#
# 所以接进来的方式不是"能跑就跑"，而是两条都必须满足：
#   1. 显式 opt-in 才跑（环境变量），因为两者都要真机资源且耗时/独占；
#   2. 没启用就**登记未验项**并在推 tag 前点名——不拿"跳过"冒充"通过"。
#
# 为什么不用"缺环境就自动失败"（像上面的原生验收那样 die）：那会让每一轮发版
# 在没有 VM 的机器上直接卡死，实操结果只会是所有人开始传旗标绕过，等于没有门禁。
# 诚实的折中：放行发布，但把缺口写进本轮记录，且**成功消息里带未验项数量**——
# 每次发版都看见"还有 N 项没验"，而不是安静地全绿。

if [[ "${NATIVE_MOTION:-0}" == "1" ]]; then
    step "原生动效逐边取证（四边，各约 75s，共约 5 分钟）"
    # 必须走 VM 来宾路径：native-gates-vm.py --motion-only 会把 test-native-motion.py
    # 投递到隔离 VM 内执行。**不能在此处宿主侧直调**——test-native-motion.py 自带
    # 真机前置校验（darwin + Aqua uid≥500 + /dev/console 属主一致 + kern.hv_vmm_present==1），
    # 在宿主桌面跑会被它自己拒掉并 exit 2，于是"启用了反而永远失败"，
    # 比不启用更糟（会逼人绕过旗标）。这里只做宿主的投递与前置检查。
    [[ -n "${NATIVE_TEST_VM:-}" ]] || die "NATIVE_MOTION=1 需要 NATIVE_TEST_VM（真机隔离 VM）"
    # 参数以 native-gates-vm.py 的 argparse 为准：--vm 必填、--user 可被 NATIVE_TEST_USER
    # 免填、--motion-only 为互斥模式（与 --check-only/--resource-only 不能同用）。
    # 该脚本不接受 --binary：二进制由它在来宾内自行定位。
    python3 scripts/native-gates-vm.py --motion-only --vm "$NATIVE_TEST_VM" --user "${NATIVE_TEST_USER:-}"
else
    unverified "原生动效逐边取证未运行（需 NATIVE_MOTION=1 + NATIVE_TEST_VM，经 native-gates-vm.py --motion-only 投递到 VM 来宾）"
fi

if [[ "${CLAUDE_PLAN_CLIENT:-0}" == "1" ]]; then
    step "官方 Claude 客户端取证（pinned CLI + sandbox 策略探测）"
    # 需要预先安装 pinned 官方 CLI（.scratch/claude-real-client/runtime/...），
    # 且脚本会校验版本恰为 2.1.291，不符即失败——不静默降级。
    python3 scripts/test-claude-plan-client.py
else
    unverified "官方 Claude 客户端取证未运行（需 CLAUDE_PLAN_CLIENT=1 与真实 pinned CLI）"
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
# 推出去之前最后看一眼本轮未验项：tag 一推就收不回来了。
report_unverified
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
if [[ "${#RELEASE_UNVERIFIED[@]}" -gt 0 ]]; then
    # 成功消息里也点名：不许"✓ 已发布"单独出现，让人以为全链路都验过了。
    printf '\033[33m⚠ 本轮有 %d 项未验（需外部环境）——本版本不覆盖它们，见上面的未验项清单\033[0m\n' \
        "${#RELEASE_UNVERIFIED[@]}"
fi
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
