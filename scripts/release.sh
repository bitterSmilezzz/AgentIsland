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
cd "$(dirname "$0")/.."

VERSION="${1:-}"
TITLE="${2:-}"
NOTES_FILE="${3:-}"

die() { echo "✗ $*" >&2; exit 1; }
step() { printf '\n\033[1m==> %s\033[0m\n' "$*"; }

[[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "用法: $0 <X.Y.Z> \"<标题>\" —— 版本号形如 0.0.119"
[[ -n "$TITLE" ]] || die "缺 release 标题（CHANGELOG 首条的那句话）"

step "版本三处一致性预检 v${VERSION}"
TOP=$(grep -m1 -oE '^## \[[0-9]+\.[0-9]+\.[0-9]+\]' CHANGELOG.md | grep -oE '[0-9]+\.[0-9]+\.[0-9]+' || true)
[[ "$TOP" == "$VERSION" ]] || die "CHANGELOG 首条是 [$TOP]，与要发的 v$VERSION 不一致"
[[ "$(grep -m1 -oE 'string = "[0-9.]+"' Sources/AgentIslandCore/AppVersion.swift | grep -oE '[0-9]+\.[0-9]+\.[0-9]+')" == "$VERSION" ]] \
    || die "AppVersion.string 还没改到 $VERSION（CLI 横幅与设置页都读它）"
[[ "$(grep -m1 -oE '本文档描述 \*\*v[0-9.]+' README.md | grep -oE '[0-9]+\.[0-9]+\.[0-9]+')" == "$VERSION" ]] \
    || die "README 的「本文档描述 vX.Y.Z」没跟着改（README 讲功能，逐版记录归 CHANGELOG）"
[[ -z "$(git tag -l "v$VERSION")" ]] || die "tag v$VERSION 已存在，别重复发版"
git rev-parse --verify --quiet "HEAD" >/dev/null || die "没有提交历史"
BRANCH=$(git rev-parse --abbrev-ref HEAD)
[[ "$BRANCH" == "main" ]] || die "当前在 $BRANCH，发版要求在 main"
MERGED=$(git diff --name-only --diff-filter=U | wc -l | tr -d ' ')
[[ "$MERGED" == "0" ]] || die "有 $MERGED 个未解决冲突，先解冲突"

if [[ "${SKIP_SCAN:-0}" == "1" ]]; then
    step "脱敏扫描被 SKIP_SCAN=1 跳过 —— 这个产物不得对外发布"
else
    step "密钥 / 个人信息扫描（含全部 git 对象）"
    scripts/scan-secrets.sh --release
fi

step "测试门禁与打包"
scripts/build-app.sh "$VERSION"

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
pkill -x AgentIsland || true
sleep 0.6
open dist/AgentIsland.app

printf '\n\033[32m✓ v%s 已发布：提交已推送、tag 已打、release 已建\033[0m\n' "$VERSION"
gh release view "v$VERSION" --json tagName,assets -q '"    " + .tagName + "  附件: " + ([.assets[].name] | join(", "))'
