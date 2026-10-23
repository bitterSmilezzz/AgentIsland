#!/usr/bin/env python3
"""GW-09：`status --json` 与 GUI 概览的**字段对拍**（逐项，不是「大概都有」）。

入口能力矩阵（`docs/spec/入口能力矩阵.md`）回答「哪个入口能做什么」；
本门禁回答更细的一层：「同一个事实，两个入口读的**是不是同一个字段、同一处措辞**」。

数据是同源的（CLI 单拍与 GUI 共用 `ActivityEngine`，都会产出 `AgentSnapshot`），
会分叉的是**各自挑字段、各自拼文案**：

- CLI 的 JSON 行是手工挑字段拼 camelCase（`cli.rs status_json_row`）；
- GUI 概览的行模型按 snake_case 直读快照（`views.js agentRowModel`，
  外加 `agent-actions.js agentNextStep`/`healthReason` 与 `isVisible`）。

任何一侧漂了就变红：GUI 新读一个 JSON 没暴露的字段、JSON 删一个 GUI 在读的键、
「 · 自报」这样的措辞出现第二份定义。

**登记在案的有意差异**（核对它们仍然是差异，不是漏对拍）：

- `cpuPercent` 恒为 null：单拍没有差分窗口，GUI 概览行也不显示 CPU
  （持续观测入口才有读数）。真实读数属于 `top` / 详情页，另有口径。
- GUI 的待机不确定文案（`UNCERTAIN_LABELS`）是按 `observability.code` 挑的**显示短标签**；
  JSON 给的是同一个 `code` + Rust 侧 summary 原文。身份（code）同源，措辞各自排版——
  与 `agentRowModel` 注释里「只共用内容不共用排版」同一纪律。
- 健康度：JSON 给整份 `health` 报告（GUI healthChip / agentNextStep 读的就是这份），
  不再造「只有分数」的形状。
- `provenanceSuffix` 在 CLI 进程恒为空串：CLI 是另一个进程，看不到 app 的自报记录
  （`state` 命令的同一条诚实边界）。字段在，形状不因「碰巧为空」而漂。

变异验证（在隔离副本执行，两个方向都要能拒）：
①从 `status_json_row` 摘掉 `currentAction` → 变红；
②在 `agentRowModel` 里新读一个 `snap.memory_text` → 变红（GUI 新事实没进对拍表）。
"""
from pathlib import Path
import re
import unittest

ROOT = Path(__file__).resolve().parents[1]
CLI = (ROOT / "app/src-tauri/src/cli.rs").read_text(encoding="utf-8")
VIEWS = (ROOT / "app/ui/js/views.js").read_text(encoding="utf-8")
ACTIONS = (ROOT / "app/ui/js/agent-actions.js").read_text(encoding="utf-8")
SELFREPORT = (ROOT / "app/src-tauri/src/selfreport.rs").read_text(encoding="utf-8")
MATRIX = (ROOT / "docs/spec/入口能力矩阵.md").read_text(encoding="utf-8")


def fn_span(source: str, header: str) -> str:
    """取一个函数/块的源码片段：从 header 起，到下一个同层级 `}` 前。

    够用即可——这里只需要函数体内不会出现 `}` 独占一行的嵌套块以外的内容，
    而 JS 函数体里的嵌套块都以缩进 `}` 收尾；Rust 侧用显式终点更稳。
    """
    start = source.index(header)
    end = source.index("\n}", start)
    return source[start:end]


def rust_json_row() -> dict[str, str]:
    """解析 `status_json_row` 的 json! 块：键 → 该键的取值表达式。"""
    body = fn_span(CLI, "fn status_json_row")
    start = body.index("serde_json::json!({")
    end = body.index("\n    })", start)
    block = body[start:end]
    pairs = re.findall(r'"([A-Za-z0-9]+)": (.+),', block)
    assert pairs, "status_json_row 的 json! 块解析不到任何键——函数被改写了？"
    return dict(pairs)


def gui_overview_fields() -> dict[str, list[str]]:
    """GUI 概览显示模型读的快照字段 → 读它的位置。

    「概览」的口径：工作台概览页渲染一个 Agent 行时真正会读到的字段——
    行模型（`agentRowModel`）、下一步建议（`agentNextStep`/`healthReason`）、
    可见性判据（`isVisible`）。详情页、侧边栏排版不算（它们有自己的入口与门禁）。
    """
    row_model = fn_span(VIEWS, "export function agentRowModel(snap)")
    next_step = fn_span(ACTIONS, "export function agentNextStep(snapshot")
    health_reason = fn_span(ACTIONS, "export function healthReason(snapshot")
    reads: dict[str, list[str]] = {}
    for label, body, needle in (
        ("views.agentRowModel", row_model, "snap."),
        ("agent-actions.agentNextStep", next_step, "snapshot."),
        ("agent-actions.healthReason", health_reason, "snapshot."),
    ):
        # 只取**根字段**：`snapshot.health?.issues` 读的是 health 这份报告，
        # JSON 侧整份转发（`health` 键）；子路径不属于单独的字段面
        for field in re.findall(r"[sS]nap(?:shot)?\.([a-z_]+)", body):
            reads.setdefault(field, []).append(label)
    # 可见性判据单独登记：它是「默认看到哪些行」的口径，不是一格显示。
    # isVisible 不在行模型函数里读快照，所以这里以它自己的锚点为准。
    if "snap.process_running === true" not in VIEWS:
        raise AssertionError("views.js 的 isVisible 读不到 process_running——可见性口径漂了")
    reads.setdefault("process_running", []).append("views.isVisible")
    return reads


# GUI 概览读的每个快照字段 → status --json 的键 → 该键必须引用的 Rust 字段。
# 这是**对拍表本体**：GUI 加读一个字段而这里没有映射，门禁红；
# JSON 键的表达式不再引用同一个 Rust 字段，门禁也红。
FIELD_MAP: dict[str, tuple[str, str]] = {
    "id": ("id", "s.id"),
    "name": ("name", "s.name"),
    "level": ("level", "s.level"),
    "level_label": ("levelLabel", "s.level_label"),
    "provenance_suffix": ("provenanceSuffix", "s.provenance_suffix"),
    "process_running": ("processRunning", "s.process_running"),
    "is_hung": ("isHung", "s.is_hung"),
    "health": ("health", "&s.health"),
    "current_action": ("currentAction", "s.current_action"),
    "token_usage": ("tokens24h", "s.token_usage"),
    "last_activity_text": ("lastActivity", "s.last_activity_text"),
    "observability": ("observability", "s.observability.code"),
    # observabilitySummary 是 JSON 对同一 verdict 的补充（GUI 概览只用 code），
    # 它不是 GUI 读的字段，所以不在 FIELD_MAP 的 GUI 侧，键集测试里单独钉。
}

# JSON 键集里「GUI 概览不读、但登记在案」的键。每一条都要写明为什么在：
# 它们不是漏对拍，是 JSON 对脚本侧的超集。GUI 哪天开始读其中之一，
# 就把它挪进 FIELD_MAP（那时「GUI 读的都在」才需要它）。
PINNED_EXTRA_KEYS = {
    # 这句话凭什么（身份码）：与 provenanceSuffix 成对——码给机器、措辞给人
    "provenance",
    # 脚本侧排进程的常用锚点；GUI 概览不显示，详情页才用
    "pid",
    # 用量的费用半边：与 tokens24h 同一份 token_usage，报表与预算读它
    "cost24h",
    # 装机探测的结论：概览只显示 process_running 的行（那时 installed 无关紧要），
    # `--all` / 智能体管理页才需要它——JSON 对脚本侧保留这个字段
    "installed",
    # Rust 侧 verdict 原文（GUI 概览只用 code 挑短标签，见模块 docstring）
    "observabilitySummary",
    # 有意差异：单拍没有差分窗口，恒为 null（有专门测试盯着它别变成真读数）
    "cpuPercent",
}


class FieldParityTests(unittest.TestCase):
    def test_every_overview_field_is_mapped(self):
        """GUI 概览读的每个快照字段都必须在对拍表里。

        防的症状：概览新增一格显示（读一个新快照字段），CLI JSON 没人想起同步——
        于是「界面上看得见的事实，脚本拿不到」。缺映射即红。
        """
        unmapped = sorted(set(gui_overview_fields()) - set(FIELD_MAP))
        self.assertEqual(
            unmapped,
            [],
            f"GUI 概览读了这些快照字段，对拍表里却没有映射：{unmapped}",
        )

    def test_every_mapped_key_exists_in_the_json_row(self):
        row = rust_json_row()
        missing = {
            field: key for field, (key, _) in FIELD_MAP.items() if key not in row
        }
        self.assertEqual(
            missing,
            {},
            f"对拍表声称这些键存在，status_json_row 里却没有：{missing}",
        )

    def test_every_mapping_sources_the_same_snapshot_field(self):
        """键不只是「名字对上了」，取值表达式必须引用**同一个 Rust 字段**。

        防的症状：JSON 键还在、但被人改成了别的来源（拼一个近似值）——
        两入口从此各说各话，而键集测试毫无感觉。
        """
        row = rust_json_row()
        drifted = {
            field: (key, row.get(key), expected)
            for field, (key, expected) in FIELD_MAP.items()
            if expected not in row.get(key, "")
        }
        self.assertEqual(
            drifted,
            {},
            f"这些 JSON 键的取值不再来自对拍表登记的同一字段：{drifted}",
        )

    def test_the_json_key_set_is_exactly_the_pinned_contract(self):
        """键集钉死：多出的键必须先补进对拍/差异登记，少键直接红。

        对拍表管「GUI 读的都在」，这条管「没有没人认领的私货」——
        两个方向合起来，键集的任何变动都要路过这张表。
        """
        pinned = {key for key, _ in FIELD_MAP.values()} | PINNED_EXTRA_KEYS
        actual = set(rust_json_row())
        self.assertEqual(
            actual,
            pinned,
            "status_json_row 的键集与契约不符"
            f"（多：{sorted(actual - pinned)}；少：{sorted(pinned - actual)}）",
        )

    def test_the_status_wording_is_single_sourced(self):
        """「 · 自报」只能在 selfreport::badge_suffix 有一份**定义**。

        GUI 概览读 Rust 拼好的 `provenance_suffix`；JSON 原样转发同一字段。
        测试里钉住措辞的断言不算定义（契约测试本来就该逐字）；要拦的是
        **显示侧与拼装侧**自己再拼一遍——那才是「改一处漏一处」的形状，
        models.rs 注释里「卡片、侧边栏、报表各自拼一遍就会漂」的教训。
        """
        self.assertIn(
            '" · 自报', SELFREPORT, "selfreport.rs 里找不到出处后缀的定义"
        )
        # 显示侧：views.js 去掉注释后不许再出现这截措辞
        display = "\n".join(
            line for line in VIEWS.splitlines()
            if not line.lstrip().startswith(("//", "*", "/*"))
        )
        self.assertNotIn(
            " · 自报",
            display,
            "views.js 的显示代码里自己拼了出处后缀——措辞必须来自 selfreport::badge_suffix 一处",
        )
        # 拼装侧：JSON 行构建器必须原样转发字段，不许在表达式里写死措辞
        builder = fn_span(CLI, "fn status_json_row")
        self.assertNotIn(
            " · 自报",
            builder,
            "status_json_row 自己拼了出处后缀——应当转发 s.provenance_suffix",
        )
        # GUI 侧必须是「直读字段」而不是「自己映射 code → 文案」的旁路
        row_model = fn_span(VIEWS, "export function agentRowModel(snap)")
        self.assertIn(
            "snap.provenance_suffix",
            row_model,
            "agentRowModel 不再直读 provenance_suffix——后缀的接法变了，对拍表要跟着改",
        )

    def test_the_visibility_clause_matches(self):
        """默认可见口径两侧一致：都只认 process_running，别无隐藏判据。"""
        self.assertIn(
            "filter(|s| want_all || s.process_running)",
            CLI,
            "status 默认可见口径变了——与 GUI isVisible 的对拍要重新核对",
        )
        self.assertIn(
            "snap.process_running === true",
            VIEWS,
            "isVisible 判据变了——与 status 默认口径的对拍要重新核对",
        )

    def test_the_registered_differences_are_still_the_registered_ones(self):
        """有意差异必须是**仍然成立**的差异，而不是碰巧没被改掉。"""
        row = rust_json_row()
        self.assertEqual(
            row.get("cpuPercent"),
            "serde_json::Value::Null",
            "cpuPercent 不再恒为 null——单拍开始报 CPU 了？"
            "这是登记过的有意差异，改动必须连同登记与 GUI 口径一起复核",
        )
        self.assertNotIn(
            "healthScore",
            rust_json_row(),
            "healthScore 又回来了——健康度现在是整份 health 报告，别再造只有分数的形状",
        )

    def test_the_matrix_document_carries_the_parity_gate(self):
        """矩阵文档登记着本门禁：文档与门禁互相指认，不会各改各的。"""
        self.assertIn(
            "test-entry-field-parity.py",
            MATRIX,
            "入口能力矩阵文档没有登记字段对拍门禁——文档与门禁脱钩了",
        )


if __name__ == "__main__":
    unittest.main()
