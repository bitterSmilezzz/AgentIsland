#!/bin/bash
# 测试 source 生产暂存函数；所有 git add / commit 均限定在临时夹具。
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
RELEASE_SCRIPT="$SCRIPT_DIR/release.sh"
. "$RELEASE_SCRIPT"
TEST_ROOT=$(mktemp -d "${TMPDIR:-/tmp}/agentisland-release-staging.XXXXXX")
trap 'rm -rf "$TEST_ROOT"' EXIT
# 隔离调用者 Git 环境、全局忽略规则及 hooks。
unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE GIT_COMMON_DIR RELEASE_FILES_FILE RELEASE_COMMITTED RELEASE_RESUME
export GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL=/dev/null
export GIT_AUTHOR_NAME='Release Test' GIT_AUTHOR_EMAIL='release@example.invalid'
export GIT_COMMITTER_NAME="$GIT_AUTHOR_NAME" GIT_COMMITTER_EMAIL="$GIT_AUTHOR_EMAIL"
COUNT=0
fail() { printf 'FAIL: %s\n' "$*" >&2; exit 1; }
pass() { COUNT=$((COUNT + 1)); printf 'ok %s - %s\n' "$COUNT" "$*"; }
# --- 断点续跑前提（BUG-009：--resume 把 tag 打到上一个版本的提交上） -------------
assert_resume_guard() {
    local subject="$1" expected="$2" dirty="$3" expect="$4" label="$5"
    local actual
    actual="$(resume_precondition_error "$subject" "$expected" "$dirty")"
    if [[ "$expect" == "empty" ]]; then
        [[ -z "$actual" ]] || fail "${label}：应放行，实际拒绝：${actual}"
    else
        [[ "$actual" == *"$expect"* ]] || fail "${label}：拒绝理由应含「${expect}」，实际：${actual}"
    fi
    pass "$label"
}

assert_resume_guard "release: v0.0.9 - x" "release: v0.0.9 - x" "" empty "断点续跑放行：HEAD 就是本次发布提交且树干净"
assert_resume_guard "release: v0.0.8 - y" "release: v0.0.9 - x" "" "要求当前 HEAD 就是本次发布的提交" "断点续跑拒绝：HEAD 不是本次发布的提交"
assert_resume_guard "release: v0.0.9 - x" "release: v0.0.9 - x" "M a.txt" "要求工作区干净" "断点续跑拒绝：工作树有未提交改动"

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

# BUG-005 回归：在已提交树模式（--committed 或 RELEASE_COMMITTED=1）下，干净工作树无需 RELEASE_FILES_FILE
fixture
git reset --hard -q HEAD
git clean -fdq
mkdir -p scripts
cp "$RELEASE_SCRIPT" scripts/release.sh
printf 'APP_NAME=AgentIsland\n' > scripts/common.sh
git add scripts
git commit -qm 'scripts'
# 干净工作区带 --committed，不应报错「必须设置 RELEASE_FILES_FILE」，而应越过清单检查进入版本检查
if env -u RELEASE_FILES_FILE bash scripts/release.sh --committed 0.0.999 'fixture' > "$TEST_ROOT/output" 2>&1; then
    fail 'expected stop at version check'
fi
grep -q '已提交树模式' "$TEST_ROOT/output" || fail 'missing committed tree mode prompt'
grep -q '版本三处一致性预检' "$TEST_ROOT/output" || fail 'did not proceed to version check in committed mode'
pass 'committed tree mode allows release on clean working tree without RELEASE_FILES_FILE'

# BUG-005 负例：如果工作区有未提交改动，--committed 必须明确拒绝
fixture
mkdir -p scripts
cp "$RELEASE_SCRIPT" scripts/release.sh
printf 'APP_NAME=AgentIsland\n' > scripts/common.sh
if env -u RELEASE_FILES_FILE bash scripts/release.sh --committed 0.0.999 'fixture' > "$TEST_ROOT/output" 2>&1; then
    fail 'dirty worktree unexpectedly accepted in committed mode'
fi
grep -q '要求工作区完全干净' "$TEST_ROOT/output" || fail 'missing dirty rejection in committed mode'
pass 'committed tree mode strictly rejects dirty worktree'

# BUG-002 回归：在断点续跑模式下，HEAD 恰为当前版本的发布提交时，自动断点续跑且复用已有提交
fixture
git reset --hard -q HEAD
git clean -fdq
mkdir -p scripts
cp "$RELEASE_SCRIPT" scripts/release.sh
printf 'APP_NAME=AgentIsland\n' > scripts/common.sh
git add scripts
git commit -qm 'release: v0.0.999 - fixture'
if env -u RELEASE_FILES_FILE bash scripts/release.sh 0.0.999 'fixture' > "$TEST_ROOT/output" 2>&1; then
    fail 'expected stop at version check'
fi
grep -q '断点续跑模式' "$TEST_ROOT/output" || fail 'missing resume mode detection'
grep -q '版本三处一致性预检' "$TEST_ROOT/output" || fail 'did not proceed to version check in resume mode'
pass 'resume mode automatically detects matching release commit and resumes without RELEASE_FILES_FILE'

# BUG-002 回归：断点续跑下若 tag 已存在且恰指向当前 HEAD，允许继续复用；若指向外部提交则严格拒绝
fixture
git reset --hard -q HEAD
git clean -fdq
mkdir -p scripts
cp "$RELEASE_SCRIPT" scripts/release.sh
printf 'APP_NAME=AgentIsland\n' > scripts/common.sh
printf 'import sys; sys.exit(0)\n' > scripts/check-version.py
git add scripts
git commit -qm 'release: v0.0.999 - fixture'
git tag -a 'v0.0.999' -m 'existing tag'
if env -u RELEASE_FILES_FILE bash scripts/release.sh 0.0.999 'fixture' > "$TEST_ROOT/output" 2>&1; then
    fail 'expected stop after version/tag check'
fi
grep -q '复用当前 HEAD 已有的 tag' "$TEST_ROOT/output" || fail 'missing tag reuse message in resume mode'
pass 'resume mode reuses existing tag when it points to current release commit'

# 负例：tag 若指向历史旧提交，断点续跑仍必须拒绝
fixture
git reset --hard -q HEAD
git clean -fdq
git tag -a 'v0.0.999' -m 'old tag on base'
mkdir -p scripts
cp "$RELEASE_SCRIPT" scripts/release.sh
printf 'APP_NAME=AgentIsland\n' > scripts/common.sh
printf 'import sys; sys.exit(0)\n' > scripts/check-version.py
git add scripts
git commit -qm 'release: v0.0.999 - fixture'
if env -u RELEASE_FILES_FILE bash scripts/release.sh 0.0.999 'fixture' > "$TEST_ROOT/output" 2>&1; then
    fail 'expected rejection when tag points to different commit'
fi
grep -q 'tag v0.0.999 已存在，别重复发版' "$TEST_ROOT/output" || fail 'missing tag collision rejection'
pass 'resume mode rejects existing tag pointing to a different commit'

printf 'PASS: %s release staging regressions\n' "$COUNT"
