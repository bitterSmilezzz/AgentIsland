#!/bin/bash
# 装上 pre-commit 脱敏门禁（幂等，可重复执行）。
#
# 为什么用钩子而不是把扫描塞进 build-app.sh：打包是为了看效果，一天可能跑十几遍，
# 每遍多等 4 秒会让人直接 SKIP 掉；而「提交前必须扫」这条是红线，正好是 pre-commit
# 的位置。发版链路里 scripts/release.sh 还会再扫一遍并连 git 历史一起扫。
#
# .git/hooks 不入库，所以新克隆/新工作区要先跑这一个命令。
#
# ## 与别的钩子共存（这条不是洁癖，是会静默吞掉门禁的真 bug）
#
# 早先这个脚本的做法是「检测到已有 pre-commit 就 exit 1」。本机 .git/hooks 里已经有
# post-checkout / post-commit 两个别人的钩子（Qoder 的），说明「仓库里会有别人的钩子」
# 是常态。`set -e` 语义下，前一个钩子只要 `exit 0`，我们的扫描就被吞掉——提交照样过，
# 门禁看起来装着，其实一次都没响。
#
# 所以改成 graphify 的做法：**marker 环绕 + 置顶插入**。我们的整段包在子 shell `( … )` 里，
# 自带 set 与 cd，既不被前面的坑影响，也不把后面的坑住。重复执行只重写 marker 之间的
# 内容，别人的部分一字符不动、一行不删，只是排在我们后面。
#
# 装好的钩子长这样：
#
#     #!/bin/bash
#     # >>> agentisland:scan-secrets >>>
#     ( …我们的扫描，子 shell 隔离… )
#     # <<< agentisland:scan-secrets <<<
#     …（别的钩子的内容，原样保留，排在我们后面）…
set -euo pipefail
cd "$(dirname "$0")/.."

HOOK="$(git rev-parse --git-path hooks)/pre-commit"
BEGIN="# >>> agentisland:scan-secrets >>>"
END="# <<< agentisland:scan-secrets <<<"

# 我们的段单独放一个文件，避免把一坨 shell 塞进 heredoc 又被变量展开搅烂。
BLOCK_FILE="$(mktemp)"
trap 'rm -f "$BLOCK_FILE"' EXIT
cat > "$BLOCK_FILE" <<'HOOK_EOF'
# >>> agentisland:scan-secrets >>>
# 只扫已暂存内容；命中即拒绝提交。理由与豁免见 scripts/scan-secrets.sh 头部注释。
# 整段包在子 shell 里：自带 set/cd，不受前后其他钩子的 shell 选项影响，也不影响它们。
# 子 shell 的退出码**不会**自动让 git 拒提交——外层钩子未必有 set -e，
# 失败码会被静默丢掉（实测过：扫出了假凭据、打印了拒绝文案、提交却成功了）。
# 所以失败必须显式 `exit 1`，不能只靠子 shell 的返回码。
(
    set -uo pipefail
    cd "$(git rev-parse --show-toplevel)" || exit 1
    if ! scripts/scan-secrets.sh --staged; then
        echo "" >&2
        echo "提交被拒：上面是新增的密钥/个人信息命中。" >&2
        echo "确属假值再核对后执行 scripts/scan-secrets.sh --rebaseline，别改宽规则。" >&2
        exit 1
    fi
) || exit 1
# <<< agentisland:scan-secrets <<<
HOOK_EOF

# 合并逻辑交给 python3：要处理「新文件 / 旧格式（无 marker 的裸段）/ 已有我们的段 /
# 别人的段」四种情况，bash 字符串替换在这里只会越写越难读。
#
# ## 为什么我们的段必须置顶，而不是追加到末尾
#
# git 的 pre-commit **是单文件**：别人钩子里一句 `exit 0` 会把整个脚本结束掉，
# 追加在后面的我们那段连解释的机会都没有。这不是假设——我用一个
# 「`set -e` + `exit 0` 在前、我们那段在后」的混合钩子实测过，假的 `sk-proj-` 凭据
# 照样提交成功，门禁一次都没响。
#
# 置顶的代价我们乐意付：我们的扫描先跑，命中就 `exit 1`，别人的钩子不执行。
# 顺序反过来（别人先跑）代价是门禁可能被静默吞掉——那正是要消灭的东西。
# 别人的内容一字符不动，只是排在我们后面。
python3 - "$HOOK" "$BLOCK_FILE" "$BEGIN" "$END" <<'PY'
import os
import re
import sys

hook_path, block_path, begin, end = sys.argv[1:5]
block = open(block_path, encoding="utf-8").read()

# 旧格式：无 marker 的裸段（早期版本整文件覆写的那一版）
legacy = re.compile(r"^#\s*agentisland:scan-secrets\s*\n.*?^\}\s*$", re.M | re.S)
# 新格式：marker 环绕
marked = re.compile(re.escape(begin) + r".*?" + re.escape(end), re.S)

src = ""
if os.path.exists(hook_path):
    src = open(hook_path, encoding="utf-8").read()

replaced_legacy = replaced_marked = False
if legacy.search(src):
    src = legacy.sub("", src, count=1)
    replaced_legacy = True
if marked.search(src):
    # 先把旧位置的我们那段摘出来，稍后统一置顶
    src = marked.sub("", src, count=1).lstrip("\n")
    replaced_marked = True

shebang = ""
body = src
if body.startswith("#!"):
    shebang, _, body = body.partition("\n")

# 我们置顶，别人的原样跟在后面
new = (shebang + "\n" if shebang else "") + block
if body.strip():
    new = new + body.lstrip("\n")

new = new.rstrip("\n") + "\n"

# 段数自检：marker 只许出现一次，否则重复扫描且更难维护
if new.count(begin) != 1 or new.count(end) != 1:
    sys.exit(
        f"✗ {hook_path} 里标记段数量异常（begin={new.count(begin)}, end={new.count(end)}）。\n"
        f"  这通常是手工改过钩子。请把文件里所有 {begin}…{end} 段手工收敛成一段再跑。"
    )

open(hook_path, "w", encoding="utf-8").write(new)

if replaced_legacy:
    sys.stderr.write("· 检测到旧格式钩子，已迁移为 marker 环绕版\n")
if replaced_marked:
    sys.stderr.write("· 已将标记段移到最前（他人钩子的 exit 会让后置段不可达）\n")
else:
    sys.stderr.write("· 已在最前插入标记段，其他内容原样保留\n")
PY

chmod +x "$HOOK"
echo "✓ 已写入 $HOOK"
echo "  手动跑一遍：scripts/scan-secrets.sh --staged"
