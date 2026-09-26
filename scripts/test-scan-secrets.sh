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

printf '结果: %s 通过, %s 失败\n' "$PASS" "$FAIL"
[[ "$FAIL" -eq 0 ]]
