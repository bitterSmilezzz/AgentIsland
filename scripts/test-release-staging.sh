#!/bin/bash
# 测试 source 生产暂存函数；所有 git add / commit 均限定在临时夹具。
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
RELEASE_SCRIPT="$SCRIPT_DIR/release.sh"
. "$RELEASE_SCRIPT"
TEST_ROOT=$(mktemp -d "${TMPDIR:-/tmp}/agentisland-release-staging.XXXXXX")
trap 'rm -rf "$TEST_ROOT"' EXIT
# 隔离调用者 Git 环境、全局忽略规则及 hooks。
unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE GIT_COMMON_DIR
export GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL=/dev/null
export GIT_AUTHOR_NAME='Release Test' GIT_AUTHOR_EMAIL='release@example.invalid'
export GIT_COMMITTER_NAME="$GIT_AUTHOR_NAME" GIT_COMMITTER_EMAIL="$GIT_AUTHOR_EMAIL"
COUNT=0
fail() { printf 'FAIL: %s\n' "$*" >&2; exit 1; }
pass() { COUNT=$((COUNT + 1)); printf 'ok %s - %s\n' "$COUNT" "$*"; }
fixture() {
    cd "$TEST_ROOT"
    mkdir "case-$COUNT"
    cd "case-$COUNT"
    git init -q
    git config core.hooksPath /dev/null
    git config core.excludesFile /dev/null
    mkdir owned-dir
    printf 'base\n' > owned.txt
    printf 'base\n' > user.txt
    printf 'base\n' > deleted.txt
    printf 'base\n' > owned-dir/child.txt
    printf 'ignored.txt\n' > .gitignore
    git add -- .gitignore owned.txt user.txt deleted.txt owned-dir/child.txt
    git commit -qm fixture
    printf 'owned change\n' > owned.txt
    printf 'user change\n' > user.txt
    printf 'user new\n' > user-new.txt
    printf 'new\n' > owned-new.txt
    printf 'ignored\n' > ignored.txt
    RELEASE_FILES_FILE="$TEST_ROOT/manifest"
    printf 'owned.txt\n' > "$RELEASE_FILES_FILE"
}
assert_staged() {
    local actual
    actual=$(git diff --cached --name-only --no-renames)
    [[ "$actual" == "$1" ]] || fail "staged mismatch: [$actual], expected [$1]"
}
assert_rejected() {
    local before
    cp .git/index "$TEST_ROOT/index-before"
    before=$(git status --porcelain=v1 -uall)
    if release_files_preflight > "$TEST_ROOT/output" 2>&1; then
        fail 'unexpected preflight success'
    fi
    cmp -s .git/index "$TEST_ROOT/index-before" || fail 'rejection modified index bytes'
    [[ "$(git status --porcelain=v1 -uall)" == "$before" ]] || fail 'rejection modified worktree'
    [[ -s "$TEST_ROOT/output" ]] || fail 'missing rejection reason'
}

fixture
printf 'owned.txt\nowned-new.txt\n' > "$RELEASE_FILES_FILE"
release_files_preflight
assert_staged ''
release_stage_files
assert_staged "$(printf 'owned-new.txt\nowned.txt')"
git diff --quiet -- user.txt && fail 'unrelated tracked change disappeared'
[[ "$(cat user-new.txt)" == 'user new' ]] || fail 'unrelated untracked file changed'
pass 'only explicit modified and new files staged; unrelated tracked/untracked remain'

fixture
rm deleted.txt
printf 'owned.txt\ndeleted.txt\n' > "$RELEASE_FILES_FILE"
release_files_preflight
release_stage_files
assert_staged "$(printf 'deleted.txt\nowned.txt')"
[[ "$(git diff --cached --diff-filter=D --name-only)" == deleted.txt ]] || fail 'deletion missing'
pass 'tracked deletion accepted'

fixture
git add -- user.txt
assert_rejected
assert_staged user.txt
pass 'unrelated existing staged path rejected without touching index'

fixture
release_files_preflight
git add -- user.txt
cp .git/index "$TEST_ROOT/index-before"
if release_stage_files > "$TEST_ROOT/output" 2>&1; then fail 'late unrelated stage accepted'; fi
cmp -s .git/index "$TEST_ROOT/index-before" || fail 'late rejection modified index'
assert_staged user.txt
pass 'stage-time recheck rejects unrelated index changes'

fixture
git add -- owned.txt
release_files_preflight
release_stage_files
assert_staged owned.txt
pass 'already staged explicit file accepted'

fixture
unset RELEASE_FILES_FILE
assert_rejected
pass 'missing environment variable rejected'

fixture
RELEASE_FILES_FILE="$TEST_ROOT/missing-manifest"
assert_rejected
pass 'missing manifest rejected'

fixture
: > "$RELEASE_FILES_FILE"
assert_rejected
pass 'empty manifest rejected'

fixture
printf '\n' > "$RELEASE_FILES_FILE"
assert_rejected
pass 'blank line rejected'

fixture
printf 'owned.txt\nowned.txt\n' > "$RELEASE_FILES_FILE"
assert_rejected
pass 'duplicate rejected'

for invalid in /tmp/outside.txt ../outside.txt owned-dir/../owned.txt .git/config nested/.git/config .GIT/config owned-dir owned-dir/ ./owned.txt owned-dir//child.txt missing.txt ignored.txt 'C:/outside.txt'; do
    fixture
    printf 'owned.txt\n%s\n' "$invalid" > "$RELEASE_FILES_FILE"
    assert_rejected
    assert_staged ''
    pass "invalid path rejected before staging: $invalid"
done

fixture
rm -r owned-dir
printf 'owned-dir\n' > "$RELEASE_FILES_FILE"
assert_rejected
pass 'deleted directory cannot implicitly stage its children'

fixture
printf 'owned.txt\000user.txt\n' > "$RELEASE_FILES_FILE"
assert_rejected
pass 'NUL cannot silently merge paths'

fixture
printf 'owned.txt\r\n' > "$RELEASE_FILES_FILE"
assert_rejected
pass 'CRLF rejected with explicit error'

fixture
printf 'literal\n' > 'literal*.txt'
printf 'unrelated\n' > literal-extra.txt
printf 'literal\n' > ':(glob)*.txt'
printf 'spaced\n' > 'with space.txt'
printf 'dash\n' > -dash.txt
printf '%s\n' 'literal*.txt' ':(glob)*.txt' 'with space.txt' '-dash.txt' > "$RELEASE_FILES_FILE"
release_files_preflight
release_stage_files
[[ "$(git diff --cached --name-only | wc -l | tr -d ' ')" == 4 ]] || fail 'literal paths expanded'
[[ -z "$(git ls-files -- literal-extra.txt)" ]] || fail 'wildcard staged unrelated file'
pass 'wildcards, pathspec magic, spaces and leading dash treated literally'

fixture
printf 'owned.txt' > "$RELEASE_FILES_FILE"
release_files_preflight
# 清单预检后冻结内存路径，构建期间改写清单不改变本轮范围。
printf 'user.txt\n' > "$RELEASE_FILES_FILE"
release_stage_files
assert_staged owned.txt
pass 'no-final-newline accepted; manifest cannot alter frozen selection'

fixture
mkdir -p scripts
cp "$RELEASE_SCRIPT" scripts/release.sh
printf 'APP_NAME=AgentIsland\n' > scripts/common.sh
# 真实脚本缺清单须早于版本检查（夹具不提供 check-version.py）。
if env -u RELEASE_FILES_FILE bash scripts/release.sh 0.0.999 'fixture' > "$TEST_ROOT/output" 2>&1; then
    fail 'release without manifest unexpectedly succeeded'
fi
grep -q '必须设置 RELEASE_FILES_FILE' "$TEST_ROOT/output" || fail 'missing early manifest failure'
if grep -qE 'check-version|版本三处' "$TEST_ROOT/output"; then fail 'release proceeded past missing manifest'; fi
assert_staged ''
pass 'real release entrypoint fails before version checks or mutations'

printf 'PASS: %s release staging regressions\n' "$COUNT"
