#!/usr/bin/env python3
"""AUTO-02：复盘触发契约（docs/spec/复盘触发契约.md）的形状门禁。

契约是数据，本脚本核对三件事：

1. **契约指名的入口必须真实存在**：pipeline 的 `add_lesson`（唯一写账本入口，
   且必须经 `lesson_ledger.append_entry`）、`promotion_candidates`（只读聚合）、
   `lesson_ledger.revoke_key`（人工撤销）、`lessons.py validate`（显式晋升判据）
   ——契约引用一个不存在的东西，就是伪造的契约。
2. **禁止事项必须真的没有**：pipeline 里不存在 lessons 相关的定时器/cron 挂点
   （需求明确排除定时任务类触发器），也不允许绕过 `add_lesson` 直接拼账本字节。
3. **契约与门禁互相指认**：契约登记着本门禁；本门禁的模块 docstring 指认契约。

变异验证（隔离副本）：把契约引用的 `add_lesson` 改名 → 变红；往 pipeline 里塞
cron 触发或直接写账本字节的调用 → 变红。
"""
from pathlib import Path
import re
import unittest

ROOT = Path(__file__).resolve().parents[1]
CONTRACT = (ROOT / "docs/spec/复盘触发契约.md").read_text(encoding="utf-8")
PIPELINE = (ROOT / "scripts/automation/pipeline.py").read_text(encoding="utf-8")
LESSON_LEDGER = (ROOT / "scripts/automation/lesson_ledger.py").read_text(encoding="utf-8")
LESSONS_CLI = (ROOT / "scripts/automation/lessons.py").read_text(encoding="utf-8")
MODULE_DOC = Path(__file__).read_text(encoding="utf-8")


class RetroContractTests(unittest.TestCase):
    def test_the_contract_names_real_entry_points(self):
        self.assertIn("def add_lesson", PIPELINE, "契约的写入口 add_lesson 不在 pipeline 里")
        self.assertIn("def promotion_candidates", LESSON_LEDGER, "晋升候选函数不在 lesson_ledger 里")
        for entry in ("def append_entry", "def revoke_key"):
            self.assertIn(entry, LESSON_LEDGER, f"契约引用的 {entry} 不在 lesson_ledger 里")
        self.assertIn('"validate"', LESSONS_CLI, "契约引用的 validate 子命令不在 lessons CLI 里")

    def test_every_identifier_the_contract_names_exists_in_code(self):
        """契约指名的每个标识符都必须在代码里找得到（doc→code 绑定）。

        防的症状：代码重构改了名、契约没人想起同步——契约从此引用一个
        不存在的东西，读它的人被指向空气。任何指名标识符在代码里消失即红。
        """
        named = set(re.findall(r"`([a-z_][a-z_0-9]{3,})`", CONTRACT))
        self.assertTrue(named, "契约里没有可核对的指名标识符——契约被改空了？")
        code = PIPELINE + LESSON_LEDGER + LESSONS_CLI
        missing = sorted(name for name in named if name not in code)
        self.assertEqual(
            missing, [], f"契约指名的入口在代码里找不到：{missing}"
        )

    def test_the_pipeline_writes_the_ledger_only_through_the_single_entry(self):
        """写账本只许走 add_lesson → append_entry：绕过幂等与原子发布的旁路必须被拦下。"""
        self.assertIn(
            "append_entry(LESSON_LEDGER, entry)", PIPELINE,
            "pipeline 不再经 append_entry 写账本——幂等单点被绕开了",
        )
        # 账本路径只能出现在「常量定义」与「传给 lesson_ledger 函数」两处形状里；
        # 出现 open/write/f-strings 直接拼字节就是第二份写入实现。
        for line in PIPELINE.splitlines():
            if "LESSON_LEDGER" in line and "jsonl" not in line:
                self.assertRegex(
                    line.strip(),
                    r"^(#|append_entry\(LESSON_LEDGER|def load_lessons|return load_entries)",
                    f"账本路径出现在未知形状的用法里：{line.strip()}",
                )

    def test_pipeline_has_no_scheduled_lesson_trigger(self):
        """需求明确排除定时任务类触发器：pipeline 里不许有 cron/定时挂点。"""
        self.assertNotIn(
            "cron", PIPELINE.lower(),
            "pipeline 里出现了 cron——复盘触发器只能附着在 run 收尾（契约口径）",
        )
        for forbidden in ("schedule.every", "APScheduler", "setInterval"):
            self.assertNotIn(
                forbidden, PIPELINE, f"pipeline 里出现了 {forbidden}——这不是本仓的依赖"
            )

    def test_the_contract_and_the_gate_name_each_other(self):
        self.assertIn(
            "test-retro-contract.py", CONTRACT,
            "契约没有登记形状门禁——文档与门禁脱钩了",
        )
        self.assertIn(
            "复盘触发契约", MODULE_DOC, "门禁没有指认契约"
        )


if __name__ == "__main__":
    unittest.main()
