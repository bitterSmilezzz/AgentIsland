#!/bin/bash
# 提交 / 发版前的密钥与个人信息扫描（红线见 docs/agent/desensitization.md）。
#
# 为什么不是「零命中才算过」：本仓的测试夹具与调研文档合法地写着假口令与占位收件地址，
# 硬要清零只会让人把规则调瞎。改成**棘轮**：命中的 (规则, 文件, 行内容) 三元组要在
# scripts/secrets-baseline.txt 里逐条备过案，新增即失败——真凭据必然是新命中，
# 而为凑 baseline 把某条真东西备进案里，diff 看得见。
# 「凭据前缀 / 私钥块」两类是例外：它们绝对零命中，不许进 baseline。
#
# 用法：
#   scripts/scan-secrets.sh              # 扫工作区中 git 认得的全部文件
#   scripts/scan-secrets.sh --staged     # 只扫已暂存内容（提交前）
#   scripts/scan-secrets.sh --release    # 工作区 + 全部 git 对象（发版前）
#   scripts/scan-secrets.sh --history    # 只扫 git 对象
#   scripts/scan-secrets.sh --rebaseline # 人工核对后重写 baseline
#
# ── 铁律：这个脚本只能「真的扫完了」才说通过 ────────────────────────────────
# 曾经有两处失败是静默的，症状都是「什么都没扫，却打印 ✓」：
#   1. `cd` 与 `git` 失败后 file_list 输出空列表，下游把空列表当「无待扫文件」放行；
#   2. 中间文件（HITS / 文件列表）的写入失败没人接，空文件被读成「零命中」。
# 两者都由 put / load_file_list 兜住：任何一步失败就 exit 3，绝不放行。
set -uo pipefail
cd "$(dirname "$0")/.." || { echo "✗ 进不了仓库根目录" >&2; exit 3; }

MODE="${1:-worktree}"
case "$MODE" in
    --staged) MODE=staged ;;
    --release) MODE=release ;;
    --history) MODE=history ;;
    --rebaseline) MODE=rebaseline ;;
    worktree) ;;
    *) echo "用法: $0 [--staged|--release|--history|--rebaseline]" >&2; exit 2 ;;
esac

BASELINE="scripts/secrets-baseline.txt"
HISTORY_ALLOW="scripts/secrets-history-allow.txt"
HITS=/tmp/scan-secrets-hits.txt
BLOBSTREAM=/tmp/scan-secrets-blobs.txt

# 本机用户名走环境变量注入，不写死在脚本里——写死等于把开发者代号放进被扫描的仓库
WHOAMI="${USER:-$(id -un)}"
[[ -n "$WHOAMI" ]] || { echo "取不到本机用户名，身份类规则无法生效" >&2; exit 2; }

RULE_IDS=(cred_prefix private_key secret_assign url_token_param local_user_path username email phone)
RULE_PAT=(
  '(sk|pk|rk)-[A-Za-z0-9]{16,}|ghp_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{20,}|glpat-[A-Za-z0-9_-]{20,}|xox[baprs]-[A-Za-z0-9-]{10,}|AKIA[0-9A-Z]{16}|AIza[0-9A-Za-z_-]{25,}|ya29\.[0-9A-Za-z_-]{20,}'
  '-----BEGIN [A-Z ]*PRIVATE KEY-----'
  '(password|passwd|api[_-]?key|access[_-]?token|auth[_-]?token|secret)["'"'"']?[[:space:]]*[:=][[:space:]]*["'"'"'][^"'"'"']{4,}'
  '[?&](access[_-]?token|api[_-]?key|auth[_-]?token|token|key|secret)=[A-Za-z0-9_-]{12,}'
  "/Users/$WHOAMI"
  "$WHOAMI"
  '[A-Za-z0-9._%+-]+@[A-Za-z][A-Za-z0-9-]*\.(com|cn|net|org|io|dev|ai|co|me|app|xyz)'
  '(^|[^0-9])1[3-9][0-9]{9}([^0-9]|$)'
)
# 绝对零命中的规则（真凭据的形状，没有「假值」这一说）
HARD=(cred_prefix private_key)
is_hard() { local r; for r in "${HARD[@]}"; do [[ "$r" == "$1" ]] && return 0; done; return 1; }

# 命中内容不回显原文：截断 + 打码，密钥不会因此进到终端或 CI 日志里
redact() { LC_ALL=C sed -E 's/[A-Za-z0-9_./+=~-]{9,}/«masked»/g' | cut -c1-100; }

# 所有中间文件的写入出口：先写临时文件再改名，且每一步失败都必须让脚本失败。
# 空文件在这里是「扫描没跑完」的信号，不是「零命中」的信号——下游分不清这两者。
put() {
    local dest="$1"; shift
    local tmpf="${dest}.new"
    : > "$tmpf" 2>/dev/null || { echo "✗ 建不了临时文件 $tmpf" >&2; exit 3; }
    "$@" > "$tmpf"   || { echo "✗ 生成 $tmpf 失败（$*）" >&2; rm -f "$tmpf"; exit 3; }
    mv -f "$tmpf" "$dest" || { echo "✗ 替换 $dest 失败" >&2; rm -f "$tmpf"; exit 3; }
}

file_list() {
    if [[ "$1" == staged ]]; then
        git diff --cached --name-only --diff-filter=ACM
    else
        # 只扫 git 认得的文件。.gitignore 排除的 .build/、dist/、artifacts/、.scratch/
        # 本就不入库不交付，扫它们等于把门禁淹死在构建缓存里
        git ls-files --cached --others --exclude-standard
    fi
}

# 取待扫文件列表。git 自己失败或列表为空都必须失败——「没扫到文件」不是「没有敏感内容」。
load_file_list() {
    local mode="$1" raw rc
    raw=/tmp/scan-secrets-files.raw
    file_list "$mode" > "$raw"
    rc=$?
    if [[ $rc -ne 0 ]]; then
        echo "✗ 取文件列表失败（git 退出码 ${rc}）。目录不对还是仓库坏了？门禁不放行。" >&2
        exit 3
    fi
    # 只保留还存在于工作区的条目：git 跟踪但文件已删的（submodule 未初始化等）不是扫描失败
    put /tmp/scan-secrets-files.txt awk 'NF && (system("test -e \"" $0 "\"") == 0)' "$raw"
    if [[ ! -s /tmp/scan-secrets-files.txt ]]; then
        echo "✗ 待扫文件列表为空：这个仓库里一个存在且被 git 认得的文件都没有。" >&2
        echo "  这不是「没有敏感内容」，而是「什么都没扫」——门禁不放行。" >&2
        exit 3
    fi
}

scan_worktree() {
    : > "$HITS" 2>/dev/null || { echo "✗ 初始化 $HITS 失败" >&2; exit 3; }
    local idx rule pat f content ln row match rest grep_rc
    local files=()
    while IFS= read -r f; do
        [[ -f "$f" ]] && files+=("$f")
    done < /tmp/scan-secrets-files.txt
    [[ ${#files[@]} -gt 0 ]] || { echo "✗ 没有可扫描的常规文件" >&2; exit 3; }
    for idx in "${!RULE_IDS[@]}"; do
        rule="${RULE_IDS[$idx]}"; pat="${RULE_PAT[$idx]}"
        # 一条规则只启动一次 grep。macOS 27 的系统 Bash 在逐文件密集 fork 时
        # 会触发 SIGTRAP；批量扫描也让 grep 的失败码不再被 process substitution 吞掉。
        LC_ALL=C grep -InHE -- "$pat" "${files[@]}" > /tmp/scan-secrets-matches.txt 2>/dev/null
        grep_rc=$?
        [[ $grep_rc -le 1 ]] || { echo "✗ [$rule] 文件扫描失败（grep 退出码 $grep_rc）" >&2; exit 3; }
        while IFS= read -r match; do
            f="${match%%:*}"; rest="${match#*:}"
            ln="${rest%%:*}"; content="${rest#*:}"
            [[ "$content" == *"nosec:"* ]] && continue   # 就地豁免，理由必须写在同一行
            # 掩码放在写盘前：HITS 里永远不出现密钥原文
            row=$(printf '%s|%s|%s|%s\n' "$rule" "$f" \
                "$(printf '%s' "$content" | cksum | cut -d' ' -f1)" \
                "$(printf '%s' "$content" | redact)" | tr '\n' ' ') || exit 3
            printf '%s\n' "${row% }" >> "$HITS" || { echo "✗ 追加 $f:$ln 命中失败" >&2; exit 3; }
        done < /tmp/scan-secrets-matches.txt
        # 去重走临时文件再改名：对同一文件 in-place sort 是曾经静默产出空文件的那一步
        put "$HITS" sort -u "$HITS"
    done
}

report_worktree() {
    local new hard_new baseline
    baseline=/tmp/scan-secrets-baseline.txt
    # A missing baseline is a distinct failure, not an empty approved set.
    if [[ ! -e "$BASELINE" ]]; then
        echo "✗ 找不到基线 ${BASELINE}——没有基线就无法判断「新增」，门禁不放行。" >&2
        return 1
    fi
    grep -v '^#' "$BASELINE" | cut -d'|' -f1-3 | sort -u > "$baseline" || {
        echo "✗ 读取基线失败，门禁不放行" >&2
        return 1
    }
    [[ -f "$baseline" ]] || return 1
    new=$(comm -23 <(cut -d'|' -f1-3 "$HITS" | sort -u) "$baseline") || {
        echo "✗ 基线比较失败，门禁不放行" >&2
        return 1
    }
    if [[ -z "$new" ]]; then
        echo "✓ 工作区：$(wc -l < "$HITS" | tr -d ' ') 条命中全部已备案，无新增"
        return 0
    fi
    echo "✗ 工作区：新增 $(printf '%s\n' "$new" | wc -l | tr -d ' ') 条未备案命中" >&2
    printf '%s\n' "$new" | while IFS='|' read -r rule f h; do
        printf '    [%s] %s → %s\n' "$rule" "$f" "$(grep -m1 "^$rule|$f|$h|" "$HITS" | cut -d'|' -f4-)" >&2
    done
    hard_new=$(printf '%s\n' "$new" | while IFS='|' read -r rule rest; do
        is_hard "$rule" && printf '%s|%s\n' "$rule" "$rest"; done)
    if [[ -n "$hard_new" ]]; then
        echo "✗ 其中属于「绝对零命中」规则：不许进 baseline，删掉它并改走环境变量/钥匙串" >&2
        printf '%s\n' "$hard_new" | sed 's/^/    /' >&2
        return 1
    fi
    echo "  若确认都是假值：核对后运行 scripts/scan-secrets.sh --rebaseline" >&2
    return 1
}

blob_stream() {
    [[ -s "$BLOBSTREAM" && -s /tmp/scan-secrets-shas.txt && -s /tmp/scan-secrets-paths.txt ]] && return 0
    rm -f "$BLOBSTREAM"
    put /tmp/scan-secrets-shas.txt bash -c 'git rev-list --objects --all | awk "{print \$1}" | sort -u | git cat-file --batch-check 2>/dev/null | awk "\$2==\"blob\"{print \$1}"'
    # sha → 路径（重命名会让同一 blob 对应多个路径，全部保留）
    put /tmp/scan-secrets-paths.txt bash -c 'git rev-list --objects --all | awk "NF>=2{print \$1\"\t\"\$2}" | sort -u'
    # 一次性流式导出全部 blob，命中行前缀所属 blob 的 sha
    put "$BLOBSTREAM" bash -c 'git cat-file --batch < /tmp/scan-secrets-shas.txt 2>/dev/null | LC_ALL=C awk "/^[0-9a-f]{40} blob [0-9]+\$/{sha=\$1; next} {print sha\"\t\"\$0}"'
}

report_history() {
    blob_stream
    echo "==> git 对象：$(wc -l < /tmp/scan-secrets-shas.txt | tr -d ' ') 个 blob（含已删除的旧版本）"
    local idx rule pat fail=0 hits joined unreviewed
    local allow_shas allow_paths
    allow_shas=$(grep -v '^#' "$HISTORY_ALLOW" 2>/dev/null | sed -nE 's/^sha:([0-9a-f]+)\|.*/\1/p' | sort -u)
    allow_paths=$(grep -v '^#' "$HISTORY_ALLOW" 2>/dev/null | sed -nE 's/^path:([^|]+)\|.*/\1/p' | sort -u)
    for idx in "${!RULE_IDS[@]}"; do
        rule="${RULE_IDS[$idx]}"; pat="${RULE_PAT[$idx]}"
        # 历史里只追「会真泄漏」的五类：凭据形状、私钥、URL 里的 token、本机身份。
        # 测试假值（假邮箱/假口令）在历史里成百上千条，逐条备案没有意义
        case "$rule" in
            cred_prefix|private_key|url_token_param|local_user_path|username) ;;
            *) continue ;;
        esac
        hits=$(LC_ALL=C grep -a -E "$pat" "$BLOBSTREAM" 2>/dev/null | cut -f1 | sort -u)
        [[ -z "$hits" ]] && { echo "✓ [$rule] 历史零命中"; continue; }
        joined=$(join -t$'\t' <(printf '%s\n' "$hits" | sort -u) /tmp/scan-secrets-paths.txt)
        unreviewed=""
        while IFS=$'\t' read -r s p; do
            [[ -z "$s" ]] && continue
            printf '%s\n' "$allow_shas" | grep -qxF "$s" && continue
            local skip=0 pre
            while IFS= read -r pre; do
                [[ -n "$pre" && "$p" == "$pre"* ]] && { skip=1; break; }
            done <<< "$allow_paths"
            [[ $skip -eq 1 ]] && continue
            unreviewed+="$s  $p"$'\n'
        done <<< "$joined"
        if [[ -n "$unreviewed" ]]; then
            echo "✗ [$rule] $(printf '%s\n' "$unreviewed" | grep -c . ) 个未核准的 blob 命中（前 8 条）：" >&2
            printf '%s\n' "$unreviewed" | grep . | head -8 | sed 's/^/    blob /' >&2
            fail=1
        else
            echo "✓ [$rule] 历史命中 $(printf '%s\n' "$hits" | wc -l | tr -d ' ') 个 blob，全部在已核准豁免内"
        fi
    done
    return $fail
}

rc=0
case "$MODE" in
    rebaseline)
        load_file_list worktree
        scan_worktree
        put "$BASELINE" cat <<EOF
# 由 scripts/scan-secrets.sh --rebaseline 生成：rule|path|行内容校验和
# 只为「人核对过的假值」存在。加一行之前先问：这条要是真的，谁负责？
$(cut -d'|' -f1-3 "$HITS")
EOF
        [[ $? -eq 0 ]] || { echo "✗ 重写 $BASELINE 失败" >&2; exit 3; }
        echo "✓ 已重写 $BASELINE（$(wc -l < "$HITS" | tr -d ' ') 条）"
        exit 0
        ;;
    worktree|staged)
        load_file_list "$MODE"
        scan_worktree
        report_worktree || rc=1
        ;;
    history)
        report_history || rc=1
        ;;
    release)
        load_file_list worktree
        scan_worktree
        report_worktree || rc=1
        report_history || rc=1
        ;;
esac

if [[ $rc -eq 0 ]]; then
    echo "✓ 密钥/个人信息扫描通过"
else
    echo "✗ 扫描未通过：先脱敏再发版，别绕这道闸（AGENTS.md 数据脱敏红线）" >&2
fi
exit $rc
