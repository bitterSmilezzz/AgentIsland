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
set -uo pipefail
cd "$(dirname "$0")/.."

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

file_list() {
    if [[ "$1" == staged ]]; then
        git diff --cached --name-only --diff-filter=ACM
    else
        # 只扫 git 认得的文件。.gitignore 排除的 .build/、dist/、artifacts/、.scratch/
        # 本就不入库不交付，扫它们等于把门禁淹死在构建缓存里
        git ls-files --cached --others --exclude-standard
    fi
}

scan_worktree() {
    : > "$HITS"
    local idx rule pat f content ln
    for idx in "${!RULE_IDS[@]}"; do
        rule="${RULE_IDS[$idx]}"; pat="${RULE_PAT[$idx]}"
        while IFS= read -r f; do
            [[ -f "$f" ]] || continue
            while IFS= read -r match; do
                ln="${match%%:*}"; content="${match#*:}"
                [[ "$content" == *"nosec:"* ]] && continue   # 就地豁免，理由必须写在同一行
                printf '%s|%s|%s|%s\n' "$rule" "$f" \
                    "$(printf '%s' "$content" | cksum | cut -d' ' -f1)" \
                    "$(printf '%s' "$content" | redact)" >> "$HITS"
            done < <(LC_ALL=C grep -InE -- "$pat" "$f" 2>/dev/null)
        done < /tmp/scan-secrets-files.txt
        sort -u "$HITS" -o "$HITS"
    done
}

report_worktree() {
    local new hard_new baseline
    baseline=/tmp/scan-secrets-baseline.txt
    grep -v '^#' "$BASELINE" 2>/dev/null | cut -d'|' -f1-3 | sort -u > "$baseline" || : > "$baseline"
    new=$(comm -23 <(cut -d'|' -f1-3 "$HITS" | sort -u) "$baseline")
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
    git rev-list --objects --all | awk '{print $1}' | sort -u \
        | git cat-file --batch-check 2>/dev/null | awk '$2=="blob"{print $1}' > /tmp/scan-secrets-shas.txt
    # sha → 路径（重命名会让同一 blob 对应多个路径，全部保留）
    git rev-list --objects --all | awk 'NF>=2{print $1"\t"$2}' | sort -u > /tmp/scan-secrets-paths.txt
    # 一次性流式导出全部 blob，命中行前缀所属 blob 的 sha
    git cat-file --batch < /tmp/scan-secrets-shas.txt 2>/dev/null \
        | awk '/^[0-9a-f]{40} blob [0-9]+$/{sha=$1; next} {print sha"\t"$0}' > "$BLOBSTREAM"
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
        file_list worktree > /tmp/scan-secrets-files.txt
        scan_worktree
        { echo "# 由 scripts/scan-secrets.sh --rebaseline 生成：rule|path|行内容校验和"
          echo "# 只为「人核对过的假值」存在。加一行之前先问：这条要是真的，谁负责？"
          cut -d'|' -f1-3 "$HITS"; } > "$BASELINE"
        echo "✓ 已重写 $BASELINE（$(wc -l < "$HITS" | tr -d ' ') 条）"
        exit 0
        ;;
    worktree|staged)
        file_list "$MODE" > /tmp/scan-secrets-files.txt
        [[ -s /tmp/scan-secrets-files.txt ]] || { echo "✓ 无待扫文件"; exit 0; }
        scan_worktree
        report_worktree || rc=1
        ;;
    history)
        report_history || rc=1
        ;;
    release)
        file_list worktree > /tmp/scan-secrets-files.txt
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
