#!/usr/bin/env python3
"""GW-01: the connections form actually wires the token-refresh declaration.

Shape assertions on the shipped page source, not browser behaviour: prove the
refresh inputs exist, the submit maps them into a single refresh object (never
a partial one), edit backfills them, and the fields stay hidden until an auth
scheme is chosen. The negative control removes the submit wiring and must fail.
"""
from pathlib import Path
import re
import unittest

ROOT = Path(__file__).resolve().parents[1]
PAGE = ROOT / "app/ui/js/connections-page.js"

FIELDS = ("refresh_kind", "refresh_token_url", "refresh_client_id", "refresh_env")
# refresh_kind 是下拉框；后端模型字段名与表单名不完全同名，分开钉。
PAYLOAD = ("token_url", "client_id", "refresh_token_env")


class RefreshUiWiringTests(unittest.TestCase):
    def setUp(self):
        self.source = PAGE.read_text(encoding="utf-8")

    def test_every_refresh_input_is_declared_with_a_label(self):
        for name in FIELDS:
            with self.subTest(field=name):
                self.assertRegex(
                    self.source,
                    rf'<(?:input|select) name="{name}"[^>]*aria-label="[^"]+"',
                    f"{name} must be reachable by assistive tech",
                )

    def test_submit_maps_the_whole_triple_into_one_refresh_object(self):
        submit = self.source[self.source.index("form.addEventListener('submit'"):]
        submit = submit[:submit.index("invoke('connection_save'")]
        self.assertIn("refresh:", submit)
        for name in PAYLOAD:
            self.assertIn(name, submit, f"submit must send refresh.{name}")
        # 刷新只在显式认证方案下可选：不发认证头就没有「401 后重试」的前提。
        self.assertIn("field('auth_kind').value !== 'none'", submit)
        self.assertIn("toUpperCase()", submit, "环境变量名按约定转大写")

    def test_edit_backfills_every_refresh_field(self):
        edit = self.source[self.source.index("if (edit) {"):]
        edit = edit[:edit.index("editor.querySelector('summary')")]
        self.assertIn("item.refresh ? 'refresh_token' : 'none'", edit)
        for name in PAYLOAD:
            self.assertIn(f"item.refresh?.{name}", edit)

    def test_refresh_fields_stay_hidden_until_an_auth_scheme_is_chosen(self):
        self.assertIn("field('refresh_kind').closest('label').hidden", self.source)
        # 三个明细字段在同一个循环里显隐：漏掉任何一个都会留一个空白标签。
        for name in FIELDS[1:]:
            self.assertIn(f"'{name}'", self.source)
        self.assertIn("field(name).closest('label').hidden = !on", self.source)
        self.assertIn("field('refresh_kind').onchange = syncRefreshFields", self.source)
        # 认证方案切换必须即时联动（v0.0.335 前的回归：只在初始化时同步，运行中切换不生效）。
        self.assertIn("field('auth_kind').onchange = syncAuthFields", self.source)

    def test_row_summary_says_the_refresh_endpoint(self):
        self.assertIn("401 刷新", self.source)

    def test_quota_switch_is_visible_without_any_prior_interaction(self):
        # 「按声明查询」开关与认证无关、必须一开始就可见：它若被静态 hidden 掉，
        # 三个明细字段的联动就永远没有触发入口，整个余额协议在界面上不可达。
        self.assertRegex(
            self.source,
            r'<label class="sb-field"><span>订阅 / 余额查询</span><select name="quota_kind"',
        )
        self.assertNotRegex(
            self.source,
            r'<label class="sb-field" hidden><span>订阅 / 余额查询</span>',
            "开关不得静态隐藏：没有解除它的代码路径",
        )

    def test_quota_declaration_is_wired_end_to_end(self):
        for name in ("quota_kind", "quota_endpoint", "quota_path", "quota_unit"):
            with self.subTest(field=name):
                self.assertRegex(
                    self.source,
                    rf'<(?:input|select) name="{name}"[^>]*aria-label="[^"]+"')
        submit = self.source[self.source.index("form.addEventListener('submit'"):]
        submit = submit[:submit.index("invoke('connection_save'")]
        for name in ("endpoint", "balance_path", "unit"):
            self.assertIn(name, submit, f"submit must send quota.{name}")
        edit = self.source[self.source.index("if (edit) {"):]
        edit = edit[:edit.index("editor.querySelector('summary')")]
        self.assertIn("item.quota?.endpoint", edit)
        self.assertIn("item.quota?.balance_path", edit)
        self.assertIn("field('quota_kind').onchange = syncQuotaFields", self.source)

    def test_row_shows_a_verified_balance_only_when_the_probe_read_one(self):
        # 余额只有在真正提取到数值时才渲染：未声明或提取失败时不许显示 0/占位值。
        self.assertIn("probe?.balance ?", self.source)
        self.assertIn("probe.balance.raw_text", self.source)

    def test_negative_control_removing_the_submit_wiring_fails(self):
        stripped = self.source.replace(
            "refresh: field('refresh_kind').value === 'refresh_token'", "/* removed */ x:")
        submit = stripped[stripped.index("form.addEventListener('submit'"):]
        submit = submit[:submit.index("invoke('connection_save'")]
        self.assertNotIn("refresh:", submit,
                         "负例：接线被删掉时，上一条用例必须变红")


if __name__ == "__main__":
    unittest.main()
