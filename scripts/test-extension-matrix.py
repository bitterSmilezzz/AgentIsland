#!/usr/bin/env python3
"""GW-10：扩展适配矩阵（docs/spec/扩展适配矩阵.md）的形状门禁。

矩阵是数据，本脚本核对两件事（与入口能力矩阵同一套思路）：

1. **声称有的必须有锚点**：每个「能配」的格子都能在代码里找到实现锚点
   （模块、目标路径、写入门禁）。
2. **声称没有的必须真的没有**：「不具备」的格子若长出了管理锚点
   （例如 Claude 的 MCP 管理），矩阵必须更新——否则矩阵在撒谎。

变异验证（隔离副本）：摘掉 mcp_config 的 Codex 目标判定 → 变红；
在矩阵里给 Claude MCP 格子凭空写上锚点 → 变红。
"""
from pathlib import Path
import re
import unittest

ROOT = Path(__file__).resolve().parents[1]
MATRIX = (ROOT / "docs/spec/扩展适配矩阵.md").read_text(encoding="utf-8")
PROMPTS = (ROOT / "app/src-tauri/src/prompts.rs").read_text(encoding="utf-8")
MCP = (ROOT / "app/src-tauri/src/mcp_config.rs").read_text(encoding="utf-8")
SKILLS = (ROOT / "app/src-tauri/src/skills_config.rs").read_text(encoding="utf-8")
SKILLS_PACKAGE = (ROOT / "app/src-tauri/src/skills_package.rs").read_text(encoding="utf-8")


def module_has_no_anchor(module: str, patterns: tuple[str, ...]) -> bool:
    return not any(pattern in module for pattern in patterns)


class ExtensionMatrixTests(unittest.TestCase):
    def test_instructions_anchors_exist_for_both_tools(self):
        self.assertIn('"AGENTS.md"', PROMPTS, "Codex AGENTS.md 锚点不见了——矩阵在撒谎")
        self.assertIn('"CLAUDE.md"', PROMPTS, "Claude CLAUDE.md 锚点不见了——矩阵在撒谎")
        # 矩阵的两格都必须指名这两个文件
        self.assertIn("AGENTS.md", MATRIX)
        self.assertIn("CLAUDE.md", MATRIX)

    def test_mcp_anchor_is_codex_only_and_the_matrix_says_so(self):
        self.assertIn("is_codex_config_target", MCP,
                      "Codex config 目标判定不见了——MCP 管理的锚点漂了")
        # Claude 侧确实没有管理锚点：mcp_config 里不允许出现 claude 配置目标
        self.assertNotIn(".claude", MCP,
                          "mcp_config 出现了 .claude 目标——矩阵的「Claude 不具备」格过时了，更新矩阵")
        self.assertIn("无管理锚点", MATRIX, "矩阵没有如实标注 Claude MCP 的边界")

    def test_skills_anchor_is_the_shared_root(self):
        self.assertIn(".agents/skills", SKILLS, "共享技能根锚点不见了——矩阵在撒谎")
        # Claude 目录的禁写名字仍在（synced/anthropic-skills 对 Claude 禁写）
        self.assertIn("anthropic-skills", SKILLS_PACKAGE,
                      "Claude 禁写名不见了——Skills 的工具差异锚点漂了")
        self.assertIn("共享根", MATRIX)

    def test_media_and_prewarm_really_have_no_anchors(self):
        """媒体/预热格写「没有」：仓库里不许长出对应适配器而矩阵不说。"""
        combined = PROMPTS + MCP + SKILLS + SKILLS_PACKAGE
        for keyword in ("prewarm", "warmup", "预热"):
            self.assertNotIn(
                keyword, combined,
                f"扩展模块里出现了 {keyword}——预热适配器已存在，矩阵的「没有」格过时了，更新矩阵",
            )
        for keyword in ("image_search", "video_adapter", "媒体适配"):
            self.assertNotIn(
                keyword, combined,
                f"扩展模块里出现了 {keyword}——媒体适配器已存在，矩阵的「没有」格过时了，更新矩阵",
            )
        self.assertIn("媒体（搜索/图像/视频适配器）", MATRIX)
        self.assertIn("预热", MATRIX)

    def test_every_matrix_anchor_named_in_the_doc_exists_in_code(self):
        """doc→code 绑定：矩阵指名的代码锚点（反引号内）必须真实存在。"""
        named = set(re.findall(r"`([a-z_][a-z_0-9]{3,}\.rs|is_codex_config_target)`", MATRIX))
        self.assertTrue(named, "矩阵里没有可核对的锚点——矩阵被改空了？")
        code = PROMPTS + MCP + SKILLS + SKILLS_PACKAGE
        missing = sorted(name for name in named if name not in code and name not in SKILLS_PACKAGE)
        # .rs 文件名核对的是「文件存在」而不是内容包含
        missing = [name for name in missing
                   if not (name.endswith(".rs") and (ROOT / "app/src-tauri/src" / name).is_file())]
        self.assertEqual(missing, [], f"矩阵指名的锚点不存在：{missing}")

    def test_the_matrix_and_the_gate_name_each_other(self):
        self.assertIn("test-extension-matrix.py", MATRIX,
                      "矩阵没有登记形状门禁——文档与门禁脱钩了")
        self.assertIn("扩展适配矩阵", Path(__file__).read_text(encoding="utf-8"),
                      "门禁没有指认矩阵")


if __name__ == "__main__":
    unittest.main()
