#!/usr/bin/env python3
"""GW-02: the provider form exposes the reasoning tier and the catalog-id mapping.

Shape assertions on the shipped page source, not browser behaviour: prove both
controls exist with accessible labels, that the reasoning select offers exactly
the four values Codex understands plus "unset", and that the form still fills and
submits generically by ``data-field`` (so a new control cannot silently fall out
of the payload). The negative control removes a control and must fail.
"""
from pathlib import Path
import re
import unittest

ROOT = Path(__file__).resolve().parents[1]
PAGE = ROOT / "app/ui/js/views.js"


class ProviderProfileUiTests(unittest.TestCase):
    def setUp(self):
        self.source = PAGE.read_text(encoding="utf-8")

    def test_reasoning_tier_offers_unset_plus_the_four_codex_values(self):
        match = re.search(r'<select aria-label="推理档位" data-field="reasoning_effort">(.*?)</select>',
                          self.source, re.S)
        self.assertIsNotNone(match, "推理档位控件缺失或没有 aria-label")
        options = re.findall(r'<option value="([^"]*)"', match.group(1))
        self.assertEqual(options, ["", "minimal", "low", "medium", "high"],
                         "档位取值必须与后端白名单逐字一致，且包含「不设置」")

    def test_catalog_mapping_is_labelled_and_display_only(self):
        self.assertRegex(
            self.source,
            r'<input aria-label="目录模型标识" data-field="catalog_id"',
            "目录模型标识控件缺失")
        # 文案必须说清「只用于显示、写入用上游模型 ID」，否则用户会以为它改的是上游
        self.assertIn("仅用于显示", self.source)

    def test_editor_fills_and_submits_generically_by_data_field(self):
        # 表单按 data-field 统一回填/收集：新增控件自动进 payload，不需要再改两处。
        self.assertIn("form.querySelectorAll('[data-field]')", self.source)
        backfill = self.source[self.source.index("const fillForm"):]
        backfill = backfill[:backfill.index("editor.open = true")]
        self.assertIn("choice[control.dataset.field]", backfill)
        submit = self.source[self.source.index("form.onsubmit"):]
        submit = submit[:submit.index("provider_save_profile")]
        self.assertIn("querySelectorAll('[data-field]')", submit)

    def test_negative_control_without_the_reasoning_control_fails(self):
        stripped = self.source.replace('aria-label="推理档位" data-field="reasoning_effort"',
                                       'data-field="reasoning_effort"')
        match = re.search(r'<select aria-label="推理档位" data-field="reasoning_effort">(.*?)</select>',
                          stripped, re.S)
        self.assertIsNone(match, "负例：控件被改名时，上面第一条用例必须变红")

    def test_the_capability_boundary_is_shown_after_applying(self):
        # 后端随结果返回 `limitations` 是「第二道防线」；界面漏显示就只剩「配置已写入」，
        # 用户会把「写成功了」读成「模型能用了」。
        self.assertIn("data-limitations", self.source)
        self.assertIn("status.limitations", self.source)
        apply_flow = self.source[self.source.index("const apply = async"):]
        apply_flow = apply_flow[:apply_flow.index("const preview = (choice, callback)")]
        self.assertIn("applied.limitations", apply_flow,
                      "应用结果的能力边界必须上屏，不能只留在后端返回值里")

    def test_negative_control_without_the_boundary_line_fails(self):
        stripped = self.source.replace("${escapeHtml(applied.limitations ?? '')}", "")
        apply_flow = stripped[stripped.index("const apply = async"):]
        apply_flow = apply_flow[:apply_flow.index("const preview = (choice, callback)")]
        self.assertNotIn("applied.limitations", apply_flow,
                         "负例：这一行被删掉时，上面那条用例必须变红")


if __name__ == "__main__":
    unittest.main()
