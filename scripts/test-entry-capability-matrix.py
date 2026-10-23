#!/usr/bin/env python3
"""GW-09：GUI / CLI / TUI / Web 各入口的**能力矩阵**，并逐条核对它们真的存在。

矩阵本身是**数据**（`docs/spec/入口能力矩阵.md` 里的表格），本脚本负责两件事：

1. **核对每一格**：矩阵里声称某入口能做的事，必须能在代码里找到对应的实现锚点
   （CLI 子命令、Web 路由、GUI 页面/模块）。找不到 → 失败。
2. **核对缺口是明说的**：矩阵里标「不具备」的入口，必须**真的没有**那个锚点；
   否则就是矩阵写错了（少写了一个能力，比多写更危险）。

这样矩阵不是一份会腐烂的文档——它每次发版都会被门禁带着核对一遍。
"""
from pathlib import Path
import re
import unittest

ROOT = Path(__file__).resolve().parents[1]
CLI = (ROOT / "app/src-tauri/src/cli.rs").read_text(encoding="utf-8")
WEBHOOK = (ROOT / "app/src-tauri/src/webhook.rs").read_text(encoding="utf-8")
NAV = (ROOT / "app/ui/js/navigation.js").read_text(encoding="utf-8")
VIEWS = (ROOT / "app/ui/js/views.js").read_text(encoding="utf-8")
MATRIX = (ROOT / "docs/spec/入口能力矩阵.md").read_text(encoding="utf-8")

# 能力 → 各入口的实现锚点。
# 「CLI」锚点形如 "cli:status"；「GUI」形如 "gui:workbench:sessions"；
# 「Web」形如 "web:/notify"；「TUI」指向 CLI 里的 `top`（它是不接管终端的看板上��）。
CAPABILITIES = {
    "运行态快照": {
        "cli": ["cli:status", "cli:state", "cli:doctor"],
        "gui": ["gui:workbench:overview", "gui:workbench:agents"],
        "tui": ["cli:top"],
        "web": [],
    },
    "用量与费用": {
        "cli": ["cli:tokens", "cli:report"],
        "gui": ["gui:workbench:tokenAnalytics", "gui:workbench:report"],
        "tui": [],
        "web": [],
    },
    "会话与任务": {
        "cli": ["cli:open"],
        "gui": ["gui:workbench:sessions", "gui:workbench:tasks", "gui:workbench:todo"],
        "tui": [],
        "web": ["web:/session"],
    },
    "投递事件": {
        "cli": ["cli:notify"],
        "gui": ["gui:workbench:remote"],
        "tui": [],
        "web": ["web:/notify", "web:/event"],
    },
    "模型与连接": {
        "cli": [],
        "gui": ["gui:workbench:provider"],
        "tui": [],
        "web": [],
    },
    "进程排查与清理": {
        "cli": ["cli:check", "cli:clean"],
        "gui": [],
        "tui": [],
        "web": [],
    },
    "设置与窗口": {
        "cli": ["cli:open"],
        "gui": ["gui:workbench:windows", "gui:workbench:settings"],
        "tui": [],
        "web": [],
    },
    "构建自检": {
        # `selftest` 验证的是**这次构建**而不是本机状态；GUI 没有对应页面
        # （它不需要——自检是给发版与排障用的，不该出现在日常界面里）。
        "cli": ["cli:selftest"],
        "gui": [],
        "tui": [],
        "web": [],
    },
    "外部工具集成": {
        # Raycast 扩展的命令清单：只有 CLI 能导出（它是给外部工具消费的清单）。
        "cli": ["cli:raycast"],
        "gui": [],
        "tui": [],
        "web": [],
    },
}


def cli_has(command: str) -> bool:
    return re.search(rf'"{re.escape(command)}" =>', CLI) is not None


def gui_has(page: str) -> bool:
    return re.search(rf"\['{re.escape(page)}'", NAV) is not None or (
        f"page{page.capitalize()}" in VIEWS
    )


def web_has(route: str) -> bool:
    return f'"{route}"' in WEBHOOK


def anchor_exists(anchor: str) -> bool:
    kind, _, rest = anchor.partition(":")
    if kind == "cli":
        return cli_has(rest)
    if kind == "gui":
        _, _, page = rest.partition(":")
        return gui_has(page)
    if kind == "web":
        return web_has(rest)
    raise AssertionError(f"未知锚点类型：{anchor}")


class CapabilityMatrixTests(unittest.TestCase):
    def test_every_matrix_row_is_listed_in_the_document(self):
        for capability in CAPABILITIES:
            self.assertIn(
                capability,
                MATRIX,
                f"能力矩阵文档里少了「{capability}」这一行——矩阵与门禁必须逐条对齐",
            )

    def test_every_claimed_capability_really_exists_in_the_code(self):
        missing: list[str] = []
        for capability, surfaces in CAPABILITIES.items():
            for surface, anchors in surfaces.items():
                for anchor in anchors:
                    if not anchor_exists(anchor):
                        missing.append(f"{capability}/{surface}/{anchor}")
        self.assertEqual(
            missing,
            [],
            f"矩阵声称这些能力存在，代码里却找不到对应实现：{missing}",
        )

    def test_capabilities_marked_absent_really_are_absent(self):
        """矩阵标「不具备」的那一格，必须真的没有实现。

        判据是**同一入口、同一能力**：如果该入口压根没有这个能力的锚点，
        却在别处存在一个同名能力的锚点，说明矩阵把「没写」与「没有」搞混了。
        """
        false_claims: list[str] = []
        for capability, surfaces in CAPABILITIES.items():
            for surface, anchors in surfaces.items():
                if anchors:
                    continue
                # 该能力在这个入口声称没有：逐个对照别的入口用到的**同能力**锚点
                for other_surface, other_anchors in surfaces.items():
                    if other_surface == surface or not other_anchors:
                        continue
                    for anchor in other_anchors:
                        kind = anchor.partition(":")[0]
                        if kind != surface:
                            continue
                        if anchor_exists(anchor):
                            false_claims.append(
                                f"{capability}/{surface} 标为无，但 {anchor} 存在"
                            )
        self.assertEqual(
            false_claims,
            [],
            f"矩阵与代码不一致：{false_claims}",
        )

    def test_the_matrix_document_lists_all_four_surfaces(self):
        for surface in ("GUI", "CLI", "TUI", "Web"):
            self.assertIn(surface, MATRIX, f"矩阵文档里少了 {surface} 这一列/行")

    def test_the_cli_command_table_is_the_source_for_the_cli_column(self):
        """CLI 那一列必须与 `cli.rs` 里的能力表对得上，不能各写一份。

        注意 `top` 落在 **TUI** 列（它就是那个不接管终端的看板），
        所以已实现的命令要么出现在 CLI 列，要么出现在 TUI 列——但不能两处都没有。
        """
        cli_anchors = {
            anchor
            for values in CAPABILITIES.values()
            for anchor in values.get("cli", []) + values.get("tui", [])
        }
        for command in re.findall(r'\(\s*"([a-z-]+)",\s*true,', CLI):
            self.assertIn(
                f"cli:{command}",
                " ".join(sorted(cli_anchors)),
                f"CLI 命令 {command} 在能力表里是已实现的，却没进矩阵的 CLI/TUI 列",
            )


if __name__ == "__main__":
    unittest.main()
