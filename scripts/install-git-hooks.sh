#!/bin/bash
# 装上 pre-commit 脱敏门禁（幂等，可重复执行）。
#
# 为什么用钩子而不是把扫描塞进 build-app.sh：打包是为了看效果，一天可能跑十几遍，
# 每遍多等 4 秒会让人直接 SKIP 掉；而「提交前必须扫」这条是红线，正好是 pre-commit
# 的位置。发版链路里 scripts/release.sh 还会再扫一遍并连 git 历史一起扫。
#
# .git/hooks 不入库，所以新克隆/新工作区要先跑这一个命令。
set -euo pipefail
cd "$(dirname "$0")/.."

HOOK="$(git rev-parse --git-path hooks)/pre-commit"
MARKER="# agentisland:scan-secrets"

if [[ -f "$HOOK" ]] && ! grep -q "$MARKER" "$HOOK"; then
    echo "✗ $HOOK 已有别的钩子，本脚本不覆盖它。" >&2
    echo "  把下面这几行自己加到它末尾即可：" >&2
    echo '    scripts/scan-secrets.sh --staged || exit 1' >&2
    exit 1
fi

cat > "$HOOK" <<HOOK_EOF
#!/bin/bash
$MARKER
# 只扫已暂存内容；命中即拒绝提交。理由与豁免见 scripts/scan-secrets.sh 头部注释。
set -uo pipefail
cd "\$(git rev-parse --show-toplevel)"
scripts/scan-secrets.sh --staged || {
    echo "" >&2
    echo "提交被拒：上面是新增的密钥/个人信息命中。" >&2
    echo "确属假值再核对后执行 scripts/scan-secrets.sh --rebaseline，别改宽规则。" >&2
    exit 1
}
HOOK_EOF

chmod +x "$HOOK"
echo "✓ 已写入 $HOOK"
echo "  手动跑一遍：scripts/scan-secrets.sh --staged"
