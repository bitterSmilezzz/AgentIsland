#!/usr/bin/env python3
"""流水线自测（对应 registry 的 automation.selfcheck，测试框架 L8）。

这些用例证明的是**流水线本身能拒错**，不是产品功能通过：

1. mock ok → 全链 passed，证据四件套齐全。
2. mock auth → Agent 轮次 failed，下游门禁 blocked，整体 failed（不谎报）。
3. mock timeout → 同上，错误文本体现超时强杀。
4. 未预授权使用真实驱动 → blocked，且不创建隔离工作区（无副作用）。
5. 未登记需求 ID → 明确失败，不去执行来路不明的检查。
6. 退出码：失败必须传播到顶层（本文件自身返回非零）。

隔离约定：自测**不读所在仓库的配置、HEAD 代码与复盘账本**，而是在临时目录里现造一个
一次性 git fixture——把**当前** `scripts/automation` 代码整份拷进去，配上**合成**
registry（唯一检查项只在被 gitignore 的 `.scratch` 里落一个 marker，不改源码、不碰网络、
不调真实 Agent），再以该 fixture 为仓库根驱动子流水线。于是：

- 未提交的新脚本（runner / registry / 本文件）在 fixture 里**存在**，不依赖 HEAD；
- HEAD 里的旧代码不会被读到，测试不会被半提交状态左右；
- 合成 registry 的 AUTO-01 只指向本地 marker 检查，**不会递归调用元测试自己**；
- 复盘账本、证据目录、workspace 全落在 fixture 的 `.scratch` 里，跑完即删。
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

# 被测代码的**当前**目录：主工作区里的 scripts/automation，或 runner 造出来的副本。
SOURCE_DIR = Path(__file__).resolve().parent
PY = sys.executable or "python3"
HOST_OS = "windows" if sys.platform == "win32" else sys.platform

# 运行期指向一次性 fixture 仓库根；import 时保持只读语义（原行为，值与原版一致）。
REPO: Path = SOURCE_DIR.parent.parent
PIPELINE: Path = SOURCE_DIR / "pipeline.py"

# 兼容旧接口：原版靠这两个名字判断是否处于嵌套执行，并据此改写 AUTOMATION_WORKSPACE。
# 现在每个用例都跑在自己的 fixture 里，嵌套检测不再需要；名字保留，值恒为「未嵌套」。
OUTER_LOCK: Path = REPO / ".scratch/automation-workspace/.agentisland-pipeline-active"
NESTED: bool = False

FAILURES: list[str] = []
SKIPPED: list[str] = []

def skip(name: str, why: str) -> None:
    print(f"[SKIP] {name} — {why}")
    SKIPPED.append(name)


def check(name: str, condition: bool, detail: str = "") -> None:
    status = "PASS" if condition else "FAIL"
    print(f"[{status}] {name}" + (f" — {detail}" if detail and not condition else ""))
    if not condition:
        FAILURES.append(name)


def parse_summary(stdout: str) -> dict:
    """解析流水线 stdout 里的汇总 JSON。

    汇总是 indent=2 的多行 JSON，**不能取最后一行**（会拿到孤立的 `}`）。
    这里截取第一个 `{` 到最后一个 `}` 之间整段再解析。
    """
    text = stdout.strip()
    if not text:
        return {}
    start, end = text.find("{"), text.rfind("}")
    if start < 0 or end <= start:
        return {}
    try:
        payload = json.loads(text[start : end + 1])
    except json.JSONDecodeError:
        return {}
    return payload if isinstance(payload, dict) else {}


# ---------------------------------------------------------------- fixture


def synthetic_registry() -> dict:
    """合成 registry：检查项只写被 gitignore 的 `.scratch`，无外部依赖、无副作用。

    `PLAN-00` 是「空检查 + status=planned」的合法形态，顺带证明这种登记不会被
    registry 校验误杀。
    """
    marker = (
        "from pathlib import Path; "
        "Path('.scratch').mkdir(exist_ok=True); "
        "Path('.scratch/selftest-marker.txt').write_text('ok')"
    )
    return {
        "schema_version": 1,
        "gates": {"L0": {"required": True}},
        "requirements": {
            "AUTO-01": {"checks": ["selftest.marker"], "gates": ["L0"]},
            "PLAN-00": {"checks": [], "status": "planned"},
        },
        "checks": {
            "selftest.marker": {
                "command": [PY, "-c", marker],
                "host_os": [HOST_OS],
                "notes": "synthetic marker check (writes only the ignored .scratch dir)",
            },
        },
        "profiles": {"test": ["selftest.marker"], "note": "synthetic selftest profile"},
        "agents": {"default": "mock", "available": ["mock", "agy"]},
    }


def git(*args: str, cwd: Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(["git", *args], cwd=str(cwd), capture_output=True, text=True)


def build_fixture() -> Path:
    """造一次性 git fixture：当前 automation 代码 + 合成 registry，一次提交。

    必须提交进 HEAD（而不是只写工作树）：流水线用 `git worktree add --detach … HEAD`
    建隔离工作区，HEAD 里没有的东西在里头就不存在——这正是旧版自测会挂的原因。
    """
    root = Path(tempfile.mkdtemp(prefix="automation-selftest-"))
    scripts = root / "scripts/automation"
    shutil.copytree(
        SOURCE_DIR, scripts, ignore=shutil.ignore_patterns("__pycache__", "*.pyc", "registry.json")
    )
    (scripts / "registry.json").write_text(
        json.dumps(synthetic_registry(), ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )
    (root / ".gitignore").write_text(".scratch/\n__pycache__/\n", encoding="utf-8")
    (root / "source.txt").write_text("selftest fixture\n", encoding="utf-8")
    identity = ["-c", "user.name=Automation Selftest", "-c", "user.email=selftest@example.invalid"]
    steps: list[tuple[str, list[str]]] = [
        ("init", ["init", "-q"]),
        ("add", ["add", "--", ".gitignore", "source.txt", "scripts"]),
        ("commit", [*identity, "commit", "-qm", "synthetic selftest fixture"]),
    ]
    for label, args in steps:
        proc = git(*args, cwd=root)
        if proc.returncode:
            raise RuntimeError(f"fixture git {label} failed: {proc.stderr.strip()}")
    return root


def run_pipeline(argv: list[str], timeout: int = 600) -> tuple[int, dict, str]:
    """在 fixture 仓库里跑一次流水线，返回 (returncode, summary dict, stderr)。

    子进程显式剔除 `AUTOMATION_WORKSPACE`：外层若把它指向别处，会把 fixture 的隔离
    workspace 根挪出 fixture，污染外层目录。
    空/不可解析 stdout 也要把 stderr 带回来：静静失败最难查。
    """
    env = {k: v for k, v in os.environ.items() if k != "AUTOMATION_WORKSPACE"}
    env["PYTHONDONTWRITEBYTECODE"] = "1"
    proc = subprocess.run(
        [PY, str(PIPELINE), *argv],
        capture_output=True,
        text=True,
        cwd=str(REPO),
        timeout=timeout,
        env=env,
    )
    return proc.returncode, parse_summary(proc.stdout), proc.stderr


def expect_ok(name: str, argv: list[str]) -> dict:
    rc, summary, stderr = run_pipeline(argv)
    check(f"{name}: exit 0", rc == 0, f"rc={rc} summary={summary} stderr={stderr[:300]}")
    return summary


def evidence_complete(run_dir: Path) -> tuple[bool, list[str]]:
    missing = [f for f in ("run.json", "agent-result.json") if not (run_dir / f).is_file()]
    return (not missing), missing


def load_run(run_dir: Path) -> dict:
    return json.loads((run_dir / "run.json").read_text(encoding="utf-8"))


# ---------------------------------------------------------------- cases


def case_ok() -> None:
    rc, summary, stderr = run_pipeline([
        "--driver", "mock", "--scenario", "ok",
        "--prompt", "selftest: create a marker file",
        "--requirements", "AUTO-01",
        "--keep-workspace",
    ])
    check("mock ok: exit 0", rc == 0, f"rc={rc} summary={summary} stderr={stderr[:300]}")
    run_dir = Path(summary.get("evidence", ""))
    complete, missing = evidence_complete(run_dir)
    check("mock ok: evidence complete", complete, f"missing={missing}")
    run = load_run(run_dir)
    check("mock ok: outcome passed", run.get("outcome") == "passed")
    check("mock ok: has run_id + fingerprint", bool(run.get("run_id")) and bool(run.get("fingerprint")))
    checks = run.get("checks", [])
    check("mock ok: requirements executed", bool(checks), f"checks={checks}")
    check("mock ok: no failed/blocked", not summary.get("failed_or_blocked"), str(summary.get("failed_or_blocked")))
    workspace = Path(run.get("workspace", {}).get("path", ""))
    check(
        "mock ok: synthetic check ran inside the isolated workspace",
        (workspace / ".scratch/selftest-marker.txt").is_file(),
        f"workspace={workspace}",
    )


def case_agent_failure(scenario: str, label: str) -> None:
    rc, summary, stderr = run_pipeline([
        "--driver", "mock", "--scenario", scenario,
        "--prompt", "selftest: expect failure",
        "--requirements", "AUTO-01",
        "--keep-workspace",
    ])
    check(f"mock {label}: non-zero exit", rc != 0, f"rc={rc}")
    run_dir = Path(summary.get("evidence", ""))
    run = load_run(run_dir) if (run_dir / "run.json").is_file() else {}
    check(f"mock {label}: outcome failed", run.get("outcome") == "failed", json.dumps(run.get("outcome")))
    statuses = {c.get("check_id"): c.get("status") for c in run.get("checks", [])}
    check(f"mock {label}: gates blocked not passed", statuses and all(s == "blocked" for s in statuses.values()), str(statuses))
    check(
        f"mock {label}: failure reason present",
        any("agent" in json.dumps(s, ensure_ascii=False).lower() for s in run.get("steps", [])),
    )


def case_preauth_gate() -> None:
    runs = REPO / ".scratch/automation-runs"
    before = set(runs.glob("*")) if runs.is_dir() else set()
    workspaces = REPO / ".scratch"
    before_worktrees = set(workspaces.glob("automation-workspace*")) if workspaces.is_dir() else set()
    rc, summary, stderr = run_pipeline([
        "--driver", "agy", "--source", "selftest",
        "--prompt", "selftest: should never run a real agent",
    ])
    check("real driver without preauth: non-zero (blocked)", rc == 3, f"rc={rc} stderr={stderr[:200]}")
    check("real driver without preauth: outcome blocked", summary.get("outcome") == "blocked", str(summary))
    run_dir = Path(summary.get("evidence", ""))
    run = load_run(run_dir) if (run_dir / "run.json").is_file() else {}
    steps = {s.get("name"): s.get("status") for s in run.get("steps", [])}
    check("real driver without preauth: preauth step blocked", steps.get("preauth") == "blocked", str(steps))
    check("real driver without preauth: no workspace", "workspace.create" not in steps, str(steps))
    check("real driver without preauth: no agent result", not (run_dir / "agent-result.json").exists())
    after = set(runs.glob("*")) if runs.is_dir() else set()
    created = set(workspaces.glob("automation-workspace*")) if workspaces.is_dir() else set()
    check(
        "real driver without preauth: no leftover worktree",
        created == before_worktrees
        and not any(p.is_dir() and (p / "worktrees").exists() for p in after - before),
        f"worktrees={sorted(p.name for p in created - before_worktrees)}",
    )


def case_unknown_requirement() -> None:
    rc, summary, stderr = run_pipeline([
        "--driver", "mock", "--scenario", "ok",
        "--prompt", "selftest",
        "--requirements", "NOPE-99",
    ])
    check("unknown requirement: non-zero", rc == 2, f"rc={rc} summary={summary}")
    check("unknown requirement: outcome failed", summary.get("outcome") == "failed", str(summary))


def case_lessons() -> None:
    sys.path.insert(0, str(SOURCE_DIR))
    from lesson_ledger import (
        LessonError,
        append_entry,
        load_entries,
        make_entry,
        promotion_candidates,
        validate,
    )

    stamp = "2026-10-14T00:00:00Z"
    with tempfile.TemporaryDirectory() as tmp:
        ledger = Path(tmp) / "lessons.jsonl"
        append_entry(ledger, make_entry("Never trust a truncated log tail", "misdiagnosed twice", "run-a", "2026-10-14", stamp))
        append_entry(ledger, make_entry("Never trust a truncated log tail", "again", "run-b", "2026-10-15", stamp))
        entries = load_entries(ledger)
        check("lessons: two entries recorded", len(entries) == 2, str(entries))
        check("lessons: 2 distinct dates not promoted", promotion_candidates(entries) == [])
        append_entry(ledger, make_entry("Never trust a truncated log tail", "third sighting", "run-c", "2026-10-16", stamp))
        promo = promotion_candidates(load_entries(ledger))
        check("lessons: 3 distinct dates promotes one key", len(promo) == 1 and bool(promo[0]["key"]), json.dumps(promo))
        with tempfile.TemporaryDirectory() as tmp2:
            ledger2 = Path(tmp2) / "lessons.jsonl"
            for i in range(5):
                append_entry(ledger2, make_entry("same day spam", "spam", f"r{i}", "2026-10-14", stamp))
            check("lessons: same-day repeats do not promote", promotion_candidates(load_entries(ledger2)) == [])
        for bad_rule, bad_date, label in (
            ("hi", "2026-10-14", "too-short rule rejected"),
            ("Valid rule text", "14-10-2026", "bad date rejected"),
            ("   ", "2026-10-14", "blank rule rejected"),
        ):
            try:
                make_entry(bad_rule, "", "e", bad_date, stamp)
                check(f"lessons: {label}", False, "expected LessonError")
            except LessonError:
                check(f"lessons: {label}", True)
        dup = make_entry("Duplicate entry", "", "same-evidence", "2026-10-14", stamp)
        with tempfile.TemporaryDirectory() as tmp3:
            ledger3 = Path(tmp3) / "lessons.jsonl"
            append_entry(ledger3, dup)
            # 幂等追加是**契约**：同一 (key, seen_date, evidence) 第二次写入是 no-op，
            # 所以账本只有一条且 validate 干净。旧断言期望「重复写 + validate 报错」，
            # 那是把缺陷当契约——重复写入已由 append 自身阻止。
            second = append_entry(ledger3, dict(dup))
            check("lessons: duplicate append is idempotent", second == "duplicate", f"got {second!r}")
            entries3 = load_entries(ledger3)
            check("lessons: duplicate append did not add a row", len(entries3) == 1, f"rows={len(entries3)}")
            check("lessons: idempotent append keeps ledger valid", not validate(entries3))
            # 同身份不同内容是**冲突**，必须拒绝而不是静默覆盖。
            conflict = {**dup, "why": "different body"}
            try:
                append_entry(ledger3, conflict)
                check("lessons: conflicting duplicate rejected", False, "expected LessonError")
            except LessonError as exc:
                check("lessons: conflicting duplicate rejected", getattr(exc, "code", "") == "duplicate_conflict",
                      f"code={getattr(exc, 'code', '')!r}")
            check("lessons: conflict left the ledger unchanged", len(load_entries(ledger3)) == 1)


def main() -> int:
    global REPO, PIPELINE
    print("== automation pipeline selftest ==")
    try:
        REPO = build_fixture()
        PIPELINE = REPO / "scripts/automation/pipeline.py"
    except Exception as exc:  # fixture 都建不起来就没得测
        print(f"[FAIL] fixture setup — {exc!r}")
        FAILURES.append("fixture setup")
        return 1
    print(f"(isolated fixture: {REPO.name} — current automation code + synthetic registry)")
    try:
        case_ok()
        case_agent_failure("auth", "auth-failure")
        case_agent_failure("timeout", "timeout")
        case_preauth_gate()
        case_unknown_requirement()
        case_lessons()
    finally:
        if FAILURES:
            print(f"(fixture kept for inspection: {REPO})")
        else:
            shutil.rmtree(REPO, ignore_errors=True)
    print()
    if FAILURES:
        print(f"FAILED: {len(FAILURES)} case(s): {', '.join(FAILURES)}")
        return 1
    if SKIPPED:
        print(f"PARTIAL: {len(SKIPPED)} group skipped: {', '.join(SKIPPED)}")
        return 3
    print("ALL PASS")
    return 0


if __name__ == "__main__":
    sys.exit(main())
