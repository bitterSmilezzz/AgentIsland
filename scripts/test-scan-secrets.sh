#!/bin/bash
# Exercise the shipped scanner in disposable Git repositories. A failing case
# must identify its intended fault and must never print the final pass marker.
set -uo pipefail
cd "$(dirname "$0")/.." || exit 3

SCANNER="$(pwd)/scripts/scan-secrets.sh"
WORK="$(mktemp -d)" || exit 3
trap 'rm -rf "$WORK"' EXIT
PASS=0
FAIL=0

fixture() {
    local dir="$WORK/$1"
    mkdir -p "$dir/scripts" || exit 3
    cp "$SCANNER" "$dir/scripts/scan-secrets.sh" || exit 3
    printf '# synthetic baseline\nsynthetic|fixture.txt|0\n' > "$dir/scripts/secrets-baseline.txt"
    printf 'clean fixture\n' > "$dir/fixture.txt"
    (cd "$dir" && git init -q) || exit 3
    FIXTURE="$dir"
}

check() {
    local label="$1" expected_rc="$2" expected_text="$3" actual_rc="$4" output="$5"
    local ok=1
    if [[ "$expected_rc" == zero ]]; then
        [[ "$actual_rc" -eq 0 ]] || ok=0
    elif [[ "$expected_rc" == 137 ]]; then
        [[ "$actual_rc" -eq 137 ]] || ok=0
        ! grep -q '✓ 密钥/个人信息扫描通过' "$output" || ok=0
    else
        [[ "$actual_rc" -ne 0 ]] || ok=0
        ! grep -q '✓ 密钥/个人信息扫描通过' "$output" || ok=0
    fi
    if [[ -n "$expected_text" ]]; then
        grep -Fq -- "$expected_text" "$output" || ok=0
    fi
    if [[ "$ok" -eq 1 ]]; then
        printf '  OK   %s\n' "$label"
        PASS=$((PASS+1))
    else
        printf '  FAIL %s (rc=%s, expected %s and %s)\n' "$label" "$actual_rc" "$expected_rc" "$expected_text" >&2
        sed -n '1,4p' "$output" >&2
        FAIL=$((FAIL+1))
    fi
}

run() {
    local name="$1"; shift
    local output="$WORK/$name.out"
    (cd "$FIXTURE" && "$@") > "$output" 2>&1
    RC=$?
    OUT="$output"
}

fixture pass
run pass bash scripts/scan-secrets.sh
check 'clean repository passes' zero '✓ 密钥/个人信息扫描通过' "$RC" "$OUT"

fixture git-fails
mkdir -p "$FIXTURE/stub"
printf '#!/bin/sh\nexit 128\n' > "$FIXTURE/stub/git"
chmod +x "$FIXTURE/stub/git"
run git-fails env PATH="$FIXTURE/stub:$PATH" bash scripts/scan-secrets.sh
check 'Git failure is fatal' nonzero 'git 退出码 128' "$RC" "$OUT"

fixture killed
# Generate an email-shaped synthetic hit at run time; no address literal is
# committed to the repository.
printf '%s@%s\n' synthetic example.com > "$FIXTURE/fixture.txt"
awk '{ print; if (index($0, "ln=\"${rest%%:*}\"")) print "                kill -9 $$" }' \
    "$SCANNER" > "$FIXTURE/scripts/scan-secrets.sh"
grep -Fq 'kill -9 $$' "$FIXTURE/scripts/scan-secrets.sh" || { echo 'probe injection failed' >&2; exit 3; }
run killed bash scripts/scan-secrets.sh
check 'interrupted scan is not success' 137 '' "$RC" "$OUT"

fixture unwritable
sed 's|^HITS=/tmp/scan-secrets-hits.txt|HITS=/dev/null/no-file|' \
    "$SCANNER" > "$FIXTURE/scripts/scan-secrets.sh"
run unwritable bash scripts/scan-secrets.sh
check 'unwritable result is fatal' nonzero '初始化' "$RC" "$OUT"

fixture empty-list
mkdir -p "$FIXTURE/stub"
printf '#!/bin/sh\nexit 0\n' > "$FIXTURE/stub/git"
chmod +x "$FIXTURE/stub/git"
run empty-list env PATH="$FIXTURE/stub:$PATH" bash scripts/scan-secrets.sh
check 'empty Git file list is fatal' nonzero '待扫文件列表为空' "$RC" "$OUT"

fixture no-baseline
rm "$FIXTURE/scripts/secrets-baseline.txt"
run no-baseline bash scripts/scan-secrets.sh
check 'missing baseline is fatal' nonzero '找不到基线' "$RC" "$OUT"

fixture new-hit
printf '%s@%s\n' synthetic example.com > "$FIXTURE/fixture.txt"
run new-hit bash scripts/scan-secrets.sh
check 'unreviewed hit is rejected' nonzero '新增' "$RC" "$OUT"

# 历史扫描必须每轮重新枚举对象。曾经它是拿「/tmp 对象流文件还在不在」当记忆的，
# 于是同一个工作区里第二次运行会复用第一次的对象流——新提交的 blob 一个都扫不到，
# 而输出照样打 ✓。这条用「提交后 blob 计数必须变大」来抓，不依赖任何凭据形状。
fixture history-fresh
git -C "$FIXTURE" config user.email fixture@example.invalid
git -C "$FIXTURE" config user.name fixture
printf 'first\n' > "$FIXTURE/first.txt"
git -C "$FIXTURE" add -A >/dev/null 2>&1
git -C "$FIXTURE" commit -qm first >/dev/null 2>&1
# 故意先把上一轮留下的对象流放在那儿：修好之后它不该被采信
rm -f /tmp/scan-secrets-blobs.txt /tmp/scan-secrets-shas.txt /tmp/scan-secrets-paths.txt
run history-first bash scripts/scan-secrets.sh --history
FIRST=$(sed -nE 's/^==> git 对象：([0-9]+) 个 blob.*/\1/p' "$OUT" | head -1)
printf 'second\n' > "$FIXTURE/second.txt"
git -C "$FIXTURE" add -A >/dev/null 2>&1
git -C "$FIXTURE" commit -qm second >/dev/null 2>&1
run history-second bash scripts/scan-secrets.sh --history
SECOND=$(sed -nE 's/^==> git 对象：([0-9]+) 个 blob.*/\1/p' "$OUT" | head -1)
if [[ -n "$FIRST" && -n "$SECOND" && "$SECOND" -gt "$FIRST" ]]; then
    printf '  OK   %s\n' 'history scan re-enumerates objects instead of reusing a stale cache'
    PASS=$((PASS+1))
else
    printf '  FAIL history scan re-enumerates objects instead of reusing a stale cache (first=%s second=%s)\n' \
        "${FIRST:-无}" "${SECOND:-无}" >&2
    FAIL=$((FAIL+1))
fi

printf '结果: %s 通过, %s 失败\n' "$PASS" "$FAIL"
[[ "$FAIL" -eq 0 ]]
