#!/bin/bash
# 一条命令走完发版：脱敏门禁 → 测试与打包 → 提交 → 打 tag → 推送 → GitHub Release。
#
# 为什么要有这个脚本：AGENTS.md 写的是「测试通过就自动提交并发版」，但这条链路此前靠
# 手工执行，结果 v0.0.81..v0.0.118 共 38 个版本提交推上去了、tag 与 release 却没建。
# 脚本把顺序钉死，且任何一步失败即停 —— 尤其是不许在扫描之前 commit。
#
# 用法：scripts/release.sh <X.Y.Z> "<一句话标题>" [notes 文件]
#   notes 缺省时取 CHANGELOG 里该版本那一节。
# 环境变量：SKIP_SCAN=1 跳过脱敏扫描（只用于本地试跑，等于自废门禁，输出里会标出来）
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
scripts/test-scan-secrets.sh
python3 scripts/test-version.py
cargo test --locked --manifest-path app/src-tauri/Cargo.toml

step "Rust/Tauri 打包"
# Rust 全量回归已在上一阶段完成。
export SKIP_TESTS=1

scripts/build-app.sh "$VERSION"

# 界面这一层在 v0.0.242 之前**完全没被验证过**（界面从来没渲染过）。
# 侧边栏的 Provider 页就出过「导航项在、注水函数在、页面函数压根没定义」的白屏，
# 而那一类 bug 静态符号检查扫不到、也没有报错可听——只有真的点进去才会现形。
# 所以打包完立刻点一遍：三个窗口里所有可点元素各点一次，看有没有跑出错误。
#
# 原生 UI / Dock 测试会抢占桌面焦点，脚本要求 --isolated-session（隔离用户会话或 VM）。
# 本机没有隔离环境时，操作者可以显式设 ALLOW_DESKTOP_UI=1 接受焦点干扰、在当前桌面跑：
# 默认不带这个变量时门禁原样生效（测试退出 2，发版停止），这里只是转发授权，不是绕过。
ISOLATED=""
if [[ "${ALLOW_DESKTOP_UI:-0}" == "1" ]]; then
    echo "⚠ ALLOW_DESKTOP_UI=1：操作者已确认接受桌面焦点干扰，原生 UI 测试将在当前桌面运行"
    ISOLATED="--isolated-session"
fi
step "UI 冒烟（点遍三个窗口的可点元素）"
scripts/ui-smoke.sh $ISOLATED

step "macOS Dock 原生显隐回归"
python3 scripts/test-dock-presence.py $ISOLATED

step "正式应用单实例回归（重复打开仍保留原进程）"
python3 scripts/test-app-instance.py --cold

step "发布包"
ZIP="dist/AgentIsland-$VERSION.zip"
rm -f "$ZIP"
( cd dist && zip -qry "AgentIsland-$VERSION.zip" AgentIsland.app agentisland )
ls -lh "$ZIP" | awk '{print "    " $9 "  " $5}'

step "提交"
# 不用 `git add -A`：它会连未被 .gitignore 排除的临时文件一起收进来。
# 这里只收「已跟踪的改动」+「git 认得且未忽略的新文件」，并逐条打印出来过目
git add -u
git ls-files --cached --others --exclude-standard | while IFS= read -r f; do
    [[ -f "$f" ]] && git add -- "$f"
done
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
