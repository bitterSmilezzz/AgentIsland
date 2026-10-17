#!/bin/bash
# Exercise the shipped scanner in disposable Git repositories. A failing case
# must identify its intended fault and must never print the final pass marker.
set -uo pipefail
cd "$(dirname "$0")/.." || exit 3

SCANNER="$(pwd)/scripts/scan-secrets.sh"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/test-scan-secrets.XXXXXXXX")" || exit 3
trap 'rm -rf "$WORK"' EXIT
PASS=0
FAIL=0
SKIP=0
mkdir -p "$WORK/run-tmp" "$WORK/legacy" || exit 3
# Keep even an old/vulnerable scanner inside this test's disposable namespace.
# Never touch existing /tmp/scan-secrets-* files or their symlink targets.
sandbox_scanner() {
    awk -v prefix="$WORK/legacy/scan-secrets-" \
        '{ gsub("/tmp/scan-secrets-", prefix); print }' "$SCANNER"
}
export TMPDIR="$WORK/run-tmp"

# Read-only guard for the real shared /tmp: the scanner and this test must not
# create, overwrite or delete /tmp/scan-secrets-* entries. Snapshot names only;
# never create or remove anything there (that is the user's shared namespace).
# 只读取签名（名称 + 目标 + 大小 + 首行校验和），不创建、不写、不删。
host_tmp_signature() {
    local entry sig=""
    for entry in /tmp/scan-secrets*; do
        [[ -e "$entry" || -L "$entry" ]] || continue
        sig+="$(basename "$entry")|$(readlink "$entry" 2>/dev/null)|"
        if [[ -f "$entry" ]]; then
            sig+="$(cksum < "$entry")"
        fi
        sig+=$'\n'
    done
    printf '%s' "$sig"
}
HOST_TMP_BEFORE="$(host_tmp_signature)"

fixture() {
    local dir="$WORK/$1"
    mkdir -p "$dir/scripts" || exit 3
    sandbox_scanner > "$dir/scripts/scan-secrets.sh" || exit 3
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
    elif [[ "$expected_rc" =~ ^[0-9]+$ ]]; then
        [[ "$actual_rc" -eq "$expected_rc" ]] || ok=0
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

# 红线门禁的判别力：一个"干净才通过"的扫描器，如果对真正的凭据形状视而不见，
# 就等于没有。此前这条只在本机一次性验证过，没有回归保护——每次改动扫描模式后
# 都不会有人知道它是否还能抓住凭据。四类形状各自注入一次，路径都避开 .gitignore
# 已经挡掉的名字（id_rsa / *.pem / *credentials* 等），因为那是由 git 负责的另一道防线，
# 混在一起会让"抓到了"无法归因到扫描器本身。
# 断言锁定到规则名：扫描器打印 `    [<rule>] <file> → <masked>`，其中 rule 是明文，
# 而命中内容被打码不回显（scan-secrets.sh:94）——这是红线，不能反过来要求它。
# 凭据字面量在运行时拼接：写死的完整形状会被编辑器的脱敏层改写成占位符，
# 落盘就不再是扫描器要匹配的东西（本轮第一版就是这么错的，4 条全 FAIL）。
fixture credential-aws
printf 'aws_access_key_id = AKIA%s\n' 'IOSFODNN7EXAMPLE' > "$FIXTURE/creds-aws.txt"
(cd "$FIXTURE" && git add -A -- creds-aws.txt) >/dev/null 2>&1
run credential-aws bash scripts/scan-secrets.sh
check 'AWS access key is caught' nonzero '[cred_prefix] creds-aws.txt' "$RC" "$OUT"

fixture credential-key
printf -- '-----BEGIN %sPRIVATE KEY-----\n' 'OPENSSH ' > "$FIXTURE/material.txt"
(cd "$FIXTURE" && git add -A -- material.txt) >/dev/null 2>&1
run credential-key bash scripts/scan-secrets.sh
check 'private key block is caught' nonzero '[private_key] material.txt' "$RC" "$OUT"

fixture credential-pat
printf 'token: ghp_%s\n' 'A1b2C3d4E5f6G7h8I9j0K1l2M3n4O5p6Q7r8' > "$FIXTURE/pat.txt"
(cd "$FIXTURE" && git add -A -- pat.txt) >/dev/null 2>&1
run credential-pat bash scripts/scan-secrets.sh
check 'GitHub personal access token is caught' nonzero '[cred_prefix] pat.txt' "$RC" "$OUT"

fixture credential-slack
printf 'SLACK_TOKEN=%s-%s\n' 'xox''b' 'A1b2C3d4E5f6G7h8I9j0' > "$FIXTURE/slack.txt"
(cd "$FIXTURE" && git add -A -- slack.txt) >/dev/null 2>&1
run credential-slack bash scripts/scan-secrets.sh
check 'Slack token is caught' nonzero '[cred_prefix] slack.txt' "$RC" "$OUT"

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
sandbox_scanner \
    | awk '{ print; if (index($0, "ln=\"${rest%%:*}\"")) print "KILLPROBE" }' \
    | awk -v probe='                kill -9 $$' '{ gsub(/KILLPROBE/, probe); print }' \
    > "$FIXTURE/scripts/scan-secrets.sh"
grep -Fq 'kill -9 $$' "$FIXTURE/scripts/scan-secrets.sh" || { echo 'probe injection failed' >&2; exit 3; }
run killed bash scripts/scan-secrets.sh
check 'interrupted scan is not success' 137 '' "$RC" "$OUT"

# 探测信号在当前进程环境里是否真的可捕获：POSIX 规定以后台方式启动的作业会忽略 SIGINT，
# 那时用例无法成立——如实记为「未验证」，绝不当成通过。
# 探测用独立 bash 进程（$$即自身），继承同一信号处置。
signal_catchable() {
    bash -c 'trap "exit 129" "$1"; kill -"$1" $$; exit 0' _ "$1" >/dev/null 2>&1
    [[ $? -eq 129 ]]
}

# --rebaseline 会在仓库侧创建 baseline 的发布临时文件（`${dest}.new.XXXXXX`）。
# 正常结束与中断都不能把它留在工作区；也不能把未完成的 baseline 当作成功。
fixture rebaseline-clean
run rebaseline-clean bash scripts/scan-secrets.sh --rebaseline
check 'rebaseline succeeds' zero '已重写' "$RC" "$OUT"
if [[ -z "$(find "$FIXTURE" -name 'secrets-baseline.txt.new*' -print -quit)" ]]; then
    printf '  OK   %s\n' 'rebaseline leaves no publication temp file behind'
    PASS=$((PASS+1))
else
    printf '  FAIL rebaseline left a temp file in the repository\n' >&2
    FAIL=$((FAIL+1))
fi

fixture rebaseline-interrupted
sandbox_scanner \
    | awk '{ print; if ($0 ~ /^ *\"\$@\" > \"\$tmpf\"/) print "    if [[ -n \"${REBASE_KILL:-}\" && \"$dest\" == scripts/secrets-baseline.txt ]]; then kill -TERM $$; fi" }' \
    > "$FIXTURE/scripts/scan-secrets.sh"
grep -Fq 'REBASE_KILL' "$FIXTURE/scripts/scan-secrets.sh" || { echo 'probe injection failed' >&2; exit 3; }
REBASE_BEFORE="$(cat "$FIXTURE/scripts/secrets-baseline.txt")"
run rebaseline-interrupted env REBASE_KILL=1 bash scripts/scan-secrets.sh --rebaseline
check 'interrupted rebaseline is not success' 143 '' "$RC" "$OUT"
if [[ -z "$(find "$FIXTURE" -name 'secrets-baseline.txt.new*' -print -quit)" ]] \
   && [[ "$(cat "$FIXTURE/scripts/secrets-baseline.txt")" == "$REBASE_BEFORE" ]]; then
    printf '  OK   %s\n' 'interrupted rebaseline removes its temp file and keeps the old baseline'
    PASS=$((PASS+1))
else
    printf '  FAIL interrupted rebaseline left a temp file or changed the baseline\n' >&2
    FAIL=$((FAIL+1))
fi

# INT/TERM 必须同样 fail closed，并由 EXIT trap 清掉本轮自己的目录。
# SIGKILL 做不到（见下方边界断言），所以这里只对可捕获信号下结论。
interrupt_case() {
    local sig="$1" expected="$2" label="$3"
    fixture "signal-$sig"
    printf '%s@%s\n' synthetic example.com > "$FIXTURE/fixture.txt"
    sandbox_scanner \
        | awk '{ print; if (index($0, "ln=\"${rest%%:*}\"")) print "KILLPROBE" }' \
        | awk -v sig="$sig" '$0 == "KILLPROBE" {
              print "                if [ -n \"${PERM_RECORD:-}\" ]; then { ls -ld \"${RUN_TMP:-}\" | cut -c1-10; printf \"%s\\n\" \"${RUN_TMP:-}\"; } > \"$PERM_RECORD\"; fi";
              print "                kill -" sig " $$"; next } { print }' \
        > "$FIXTURE/scripts/scan-secrets.sh"
    grep -Fq -- "kill -$sig " "$FIXTURE/scripts/scan-secrets.sh" || { echo 'probe injection failed' >&2; exit 3; }
    local before after perms
    before="$(find "$WORK/run-tmp" -maxdepth 1 -name 'scan-secrets.*' | sort)"
    export PERM_RECORD="$WORK/perm-$sig.txt"
    rm -f "$PERM_RECORD"
    run "signal-$sig" bash scripts/scan-secrets.sh
    after="$(find "$WORK/run-tmp" -maxdepth 1 -name 'scan-secrets.*' | sort)"
    check "$label" "$expected" '' "$RC" "$OUT"
    perms="$(sed -n 1p "$PERM_RECORD" 2>/dev/null || true)"
    run_dir="$(sed -n 2p "$PERM_RECORD" 2>/dev/null || true)"
    if [[ "$perms" == "drwx------" ]]; then
        printf '  OK   %s\n' "$sig run directory is private (0700)"
        PASS=$((PASS+1))
    else
        printf '  FAIL %s run directory permissions: %s\n' "$sig" "${perms:-<未记录>}" >&2
        FAIL=$((FAIL+1))
    fi
    # 可归因：私有目录必须建在本次 TMPDIR（测试沙箱）之下，而不是共享 /tmp。
    case "$run_dir" in
        "$WORK/run-tmp"/*)
            printf '  OK   %s\n' "$sig run directory lives under TMPDIR, not shared /tmp"
            PASS=$((PASS+1))
            ;;
        *)
            printf '  FAIL %s run directory is not under TMPDIR: %s\n' "$sig" "${run_dir:-<未记录>}" >&2
            FAIL=$((FAIL+1))
            ;;
    esac
    if [[ "$after" == "$before" ]]; then
        printf '  OK   %s\n' "$sig leaves no private directory (EXIT trap cleaned up)"
        PASS=$((PASS+1))
    else
        printf '  FAIL %s left a private directory behind\n' "$sig" >&2
        FAIL=$((FAIL+1))
    fi
}
for spec in 'TERM 143 SIGTERM' 'INT 130 SIGINT'; do
    set -- $spec
    if signal_catchable "$1"; then
        interrupt_case "$1" "$2" "$3 is fail-closed"
    else
        printf '  跳过 %s：当前进程环境忽略该信号（例如套件本身后台运行），未验证\n' "$3"
        SKIP=$((SKIP+1))
    fi
done

fixture redaction
printf '%s@%s\n' synthetic example.com > "$FIXTURE/fixture.txt"
run redaction bash scripts/scan-secrets.sh
check 'unreviewed hit is still reported' nonzero '新增' "$RC" "$OUT"
# 地址在运行时拼出来，仓库里不留可被扫描命中的邮箱字面量
if grep -q '«masked»' "$OUT" && ! grep -qF "$(printf '%s@%s' synthetic example.com)" "$OUT"; then
    printf '  OK   %s\n' 'reported hits stay masked (raw value never printed)'
    PASS=$((PASS+1))
else
    printf '  FAIL %s\n' 'hit reporting leaked the raw value or lost the mask' >&2
    FAIL=$((FAIL+1))
fi

fixture unwritable
sandbox_scanner | sed 's|^HITS=.*|HITS=/dev/null/no-file|' \
    > "$FIXTURE/scripts/scan-secrets.sh"
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

# 第三方依赖不许把发布拦下：浏览器 UI 回归需要 Playwright，而它自带的示例
# 邮箱/密码字面量会被扫描当成新命中（v0.0.323 真实发生过）。`.gitignore` 必须
# 忽略 node_modules，且 git 必须真的不把这些文件视为待扫文件。
fixture node-modules-ignored
mkdir -p "$FIXTURE/node_modules/playwright/types"
printf '# synthetic third-party sample\n%s@%s\n' fixture playwright.invalid \
    > "$FIXTURE/node_modules/playwright/types/test.d.ts"
printf 'node_modules/\n' >> "$FIXTURE/.gitignore"
run node-modules-ignored bash scripts/scan-secrets.sh
check 'third-party dependency text is out of scan scope' zero '✓ 密钥/个人信息扫描通过' "$RC" "$OUT"

# 历史扫描必须每轮重新枚举对象。曾经它是拿「/tmp 对象流文件还在不在」当记忆的，
# 于是同一个工作区里第二次运行会复用第一次的对象流——新提交的 blob 一个都扫不到，
# 而输出照样打 ✓。这条用「提交后 blob 计数必须变大」来抓，不依赖任何凭据形状。
fixture history-fresh
git -C "$FIXTURE" config user.email fixture@example.invalid
git -C "$FIXTURE" config user.name fixture
printf 'first\n' > "$FIXTURE/first.txt"
git -C "$FIXTURE" add -- first.txt >/dev/null 2>&1
git -C "$FIXTURE" commit -qm first >/dev/null 2>&1
# Leave a stale legacy stream inside the sandbox; never delete host /tmp files.
printf 'stale synthetic stream\n' > "$WORK/legacy/scan-secrets-blobs.txt"
run history-first bash scripts/scan-secrets.sh --history
FIRST=$(sed -nE 's/^==> git 对象：([0-9]+) 个 blob.*/\1/p' "$OUT" | head -1)
printf 'second\n' > "$FIXTURE/second.txt"
git -C "$FIXTURE" add -- second.txt >/dev/null 2>&1
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

# 结构棘轮：脚本不得依赖任何固定的 /tmp 中间文件。唯一允许出现的是 TMPDIR 兜底
# `${TMPDIR:-/tmp}`；注释先剥掉再查，所以这不是「文档里别写 /tmp」。
code_only=$(sed 's/#.*//' "$SCANNER")
leftovers=$(printf '%s\n' "$code_only" | grep -oE '/tmp[a-zA-Z0-9_./-]*' | sort -u)
if [[ "$leftovers" == '/tmp' ]]; then
    printf '  OK   %s\n' 'scanner has no fixed /tmp intermediate paths (only the TMPDIR fallback)'
    PASS=$((PASS+1))
else
    printf '  FAIL scanner still holds fixed /tmp paths:\n%s\n' "${leftovers:-（无）}" >&2
    FAIL=$((FAIL+1))
fi

# A run that cannot create its private directory must fail closed, not silently
# scan nothing (the old silent-pass class of failure).
fixture tmpdir-unusable
run tmpdir-unusable env TMPDIR="$FIXTURE/does-not-exist/nested" bash scripts/scan-secrets.sh
check 'unusable private temp root is fatal' nonzero '建不了扫描临时目录' "$RC" "$OUT"

# 每轮自己的目录必须随运行一起消失：正常结束后不许新增残留（被 SIGKILL 的那一轮
# 的残留是预期内的，所以比较的是前后差集，而不是「目录里为空」）。
fixture private-dir-cleanup
LEFTOVER_BEFORE="$(find "$WORK/run-tmp" -maxdepth 1 -name 'scan-secrets.*' | sort)"
run private-dir-cleanup bash scripts/scan-secrets.sh
check 'private run directory scan passes' zero '✓ 密钥/个人信息扫描通过' "$RC" "$OUT"
LEFTOVER_AFTER="$(find "$WORK/run-tmp" -maxdepth 1 -name 'scan-secrets.*' | sort)"
if [[ "$LEFTOVER_AFTER" == "$LEFTOVER_BEFORE" ]]; then
    printf '  OK   %s\n' 'private run directory is removed on normal exit'
    PASS=$((PASS+1))
else
    printf '  FAIL private run directory survived a normal exit: %s\n' \
        "$(comm -13 <(printf '%s\n' "$LEFTOVER_BEFORE") <(printf '%s\n' "$LEFTOVER_AFTER"))" >&2
    FAIL=$((FAIL+1))
fi

# SIGKILL 的准确边界：SIGKILL 不可捕获，EXIT/INT/TERM 的 trap 都不会执行，
# 所以被 SIGKILL 的那一轮**必然可能留下私有目录**。这里如实断言残留存在
# （它是上一段 `killed` 用例留下的），并明确它不作为成功证据、不会被下一轮复用作输入。
KILLED_DIR="$(find "$WORK/run-tmp" -maxdepth 1 -name 'scan-secrets.*' -print -quit)"
if [[ -n "$KILLED_DIR" ]]; then
    printf '  OK   %s\n' "SIGKILL leaves a private directory (trap cannot run): ${KILLED_DIR#$WORK/run-tmp/}"
    PASS=$((PASS+1))
    # 该目录属于本测试自己，收尾由测试删掉；它从未被任何一轮当作输入。
    rm -rf -- "$KILLED_DIR" || exit 3
else
    printf '  FAIL %s\n' 'SIGKILLed scan left no private directory: cleanup boundary unverified' >&2
    FAIL=$((FAIL+1))
fi

# 真实 git（无 stub）的两个 scanner 同时跑 --release：只声明「同时运行、结果各自
# 正确、互不掺入对方的命中」。重叠本身由下面带屏障的 fake-Git 用例证明，这里不
# 假装计时能保证重叠。
real_concurrent() {
    local clean hit dir
    fixture real-clean
    clean="$FIXTURE"
    fixture real-hit
    hit="$FIXTURE"
    printf '%s@%s\n' realprobe example.com > "$hit/realprobe.txt"
    for dir in "$clean" "$hit"; do
        (cd "$dir" && git add -- . >/dev/null 2>&1 && \
            git -c user.name=fixture -c user.email=fixture@example.invalid \
                commit -qm fixture >/dev/null 2>&1) || exit 3
        printf '# synthetic\n' > "$dir/.gitignore"
    done
    (cd "$clean" && bash scripts/scan-secrets.sh --release) > "$WORK/real-clean.out" 2>&1 &
    local pid_clean=$!
    (cd "$hit" && bash scripts/scan-secrets.sh --release) > "$WORK/real-hit.out" 2>&1 &
    local pid_hit=$!
    wait "$pid_clean"; local rc_clean=$?
    wait "$pid_hit"; local rc_hit=$?
    if [[ "$rc_clean" -eq 0 ]] &&
       grep -q '工作区：0 条命中全部已备案' "$WORK/real-clean.out" &&
       ! grep -q 'realprobe' "$WORK/real-clean.out" &&
       [[ "$rc_hit" -eq 1 ]] &&
       grep -q 'realprobe.txt' "$WORK/real-hit.out"; then
        printf '  OK   %s\n' 'two real-Git scanners run together without mixing results'
        PASS=$((PASS+1))
    else
        printf '  FAIL real-Git concurrent scans mixed or dropped results (clean=%s hit=%s)\n' \
            "$rc_clean" "$rc_hit" >&2
        FAIL=$((FAIL+1))
    fi
}
real_concurrent

# Two actual scanner processes overlap at a fake-Git barrier, after Git has
# written its output but before the caller consumes it. FIFOs fix the ordering
# without timing assumptions: B writes last, A reports first.
parallel_scans() {
    local mode="$1" a b side dir pid_a pid_b rc_a rc_b
    fixture "parallel-$mode-a"; a="$FIXTURE"
    fixture "parallel-$mode-b"; b="$FIXTURE"
    for side in a b; do
        if [[ "$side" == a ]]; then dir="$a"; else dir="$b"; fi
        printf '%s@%s\n' alpha example.com > "$dir/alpha.txt"
        printf '%s@%s\n' bravo example.org > "$dir/bravo.txt"
        if [[ "$mode" == history ]]; then
            printf '%s%s\n' scan_fixture_ identity >> "$dir/alpha.txt"
            printf '%s%s\n' scan_fixture_ identity >> "$dir/bravo.txt"
        fi
        # Both paths exist, but each Git manifest/history owns only one hit.
        if [[ "$side" == a ]]; then OWN=alpha.txt; else OWN=bravo.txt; fi
        git -C "$dir" add -- "$OWN" || exit 3
        git -C "$dir" -c user.name=fixture -c user.email=fixture@example.invalid \
            -c core.hooksPath=/dev/null commit -qm fixture || exit 3
        mkdir -p "$dir/stub"
        cat > "$dir/stub/git" <<'SH'
#!/bin/bash
if { [[ "$SCAN_MODE" == worktree && "$1" == ls-files ]]; } ||
   { [[ "$SCAN_MODE" == history && "$1" == cat-file && "${2:-}" == --batch ]]; }; then
    if [[ "$SCAN_MODE" == worktree ]]; then
        # `scan-secrets.sh` 用 `-z` 取路径（NUL 分隔），所以这个 stub 也必须输出 NUL，
        # 否则列表解析不出条目、扫描器报「什么都没扫」。换行分隔是旧约定，已随 -z 一并改。
        printf '%s\0' "$OWN"
    else
        "$REAL_GIT" "$@" || exit $?
    fi
    printf 'ready\n' > "$BARRIER.ready"
    read -r go < "$BARRIER.go"
else
    exec "$REAL_GIT" "$@"
fi
SH
        chmod +x "$dir/stub/git"
        mkfifo "$WORK/$mode-$side.ready" "$WORK/$mode-$side.go" || exit 3
    done
    (cd "$a" && env SCAN_MODE="$mode" OWN=alpha.txt REAL_GIT="$(command -v git)" \
        BARRIER="$WORK/$mode-a" PATH="$a/stub:$PATH" USER=scan_fixture_identity \
        bash scripts/scan-secrets.sh "${2:-worktree}") > "$WORK/$mode-a.out" 2>&1 &
    pid_a=$!
    read -r ready < "$WORK/$mode-a.ready"
    (cd "$b" && env SCAN_MODE="$mode" OWN=bravo.txt REAL_GIT="$(command -v git)" \
        BARRIER="$WORK/$mode-b" PATH="$b/stub:$PATH" USER=scan_fixture_identity \
        bash scripts/scan-secrets.sh "${2:-worktree}") > "$WORK/$mode-b.out" 2>&1 &
    pid_b=$!
    read -r ready < "$WORK/$mode-b.ready"
    printf 'go\n' > "$WORK/$mode-a.go"
    wait "$pid_a"; rc_a=$?
    printf 'go\n' > "$WORK/$mode-b.go"
    wait "$pid_b"; rc_b=$?
    # History uses an identity-shaped synthetic hit, because email is deliberately
    # outside history's rule scope. Worktree checks email and exact owned path.
    if [[ "$rc_a" -eq 1 && "$rc_b" -eq 1 ]] &&
       grep -q 'alpha.txt' "$WORK/$mode-a.out" &&
       ! grep -q 'bravo.txt' "$WORK/$mode-a.out" &&
       grep -q 'bravo.txt' "$WORK/$mode-b.out" &&
       ! grep -q 'alpha.txt' "$WORK/$mode-b.out" &&
       ! grep -q '✓ 密钥/个人信息扫描通过' "$WORK/$mode-a.out" "$WORK/$mode-b.out"; then
        printf '  OK   concurrent %s scanners keep distinct hits\n' "$mode"
        PASS=$((PASS+1))
    else
        printf '  FAIL concurrent %s scanners mix/drop hits (a=%s b=%s)\n' "$mode" "$rc_a" "$rc_b" >&2
        FAIL=$((FAIL+1))
    fi
}
parallel_scans worktree
parallel_scans history --history

# Seed legacy fixed names with symlinks only in the disposable namespace. The
# real scanner must leave both the links and their sentinel targets unchanged.
fixture legacy-symlinks
for name in hits.txt hits.txt.new blobs.txt blobs.txt.new files.raw files.txt files.txt.new matches.txt baseline.txt shas.txt shas.txt.new paths.txt paths.txt.new; do
    printf 'sentinel\n' > "$WORK/legacy/target-$name"
    ln -sf "target-$name" "$WORK/legacy/scan-secrets-$name"
done
run legacy-symlinks bash scripts/scan-secrets.sh --release
check 'legacy symlink scan completes' zero '✓ 密钥/个人信息扫描通过' "$RC" "$OUT"
SYMLINK_OK=1
for name in hits.txt hits.txt.new blobs.txt blobs.txt.new files.raw files.txt files.txt.new matches.txt baseline.txt shas.txt shas.txt.new paths.txt paths.txt.new; do
    [[ -L "$WORK/legacy/scan-secrets-$name" ]] || SYMLINK_OK=0
    [[ "$(cat "$WORK/legacy/target-$name")" == sentinel ]] || SYMLINK_OK=0
done
if [[ "$SYMLINK_OK" -eq 1 ]]; then
    printf '  OK   legacy fixed-path symlinks and targets are untouched\n'
    PASS=$((PASS+1))
else
    printf '  FAIL legacy fixed-path symlinks or targets changed\n' >&2
    FAIL=$((FAIL+1))
fi

# 共享 /tmp 的全局比对只作诊断、不判失败：同名前缀也会被本机其他会话或钩子
# 独立读写，那些写入无法归因到本测试。可归因的门禁在上面的信号用例里（私有目录
# 必须落在本次 TMPDIR 之下）与 legacy-symlinks 用例里（沙箱内的旧路径符号链接与
# 其目标必须原样）。这里如实打印证据，不制造会随外部并发变红的假门禁。
HOST_TMP_AFTER="$(host_tmp_signature)"
if [[ "$HOST_TMP_BEFORE" == "$HOST_TMP_AFTER" ]]; then
    printf '  信息 共享 /tmp/scan-secrets-* 快照未变（本测试未写它）\n'
else
    printf '  信息 共享 /tmp/scan-secrets-* 快照有变化——可能来自本机其他会话，本测试不作判据\n'
fi

printf '结果: %s 通过, %s 失败, %s 跳过\n' "$PASS" "$FAIL" "$SKIP"
[[ "$FAIL" -eq 0 ]]
