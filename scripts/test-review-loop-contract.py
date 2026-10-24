#!/usr/bin/env python3
"""TASK-06：审查闭环口径（docs/spec/审查闭环口径.md）的形状门禁。

契约是数据，本脚本核对三件事：

1. **四个 token 的锚点逐一存在**（doc→code 绑定）：只读审查与指纹对拍、
   身份分离、修改后重审的轮次回路、审查前门禁先绿。
2. **禁止事项真的没有**：publish 的 merge 路径永远先过 candidate.verify
   ——不存在「只凭审查 verdict 就合并」的旁路；只读审查必须带指纹对拍
   ——「宣称只读」不算只读。
3. **契约与门禁互相指认**。

变异验证（隔离副本）：删掉审查指纹对拍 → 变红；让 publish.merge 跳过
candidate.verify → 变红。
"""
from pathlib import Path
import re
import unittest

ROOT = Path(__file__).resolve().parents[1]
CONTRACT = (ROOT / "docs/spec/审查闭环口径.md").read_text(encoding="utf-8")
PIPELINE = (ROOT / "scripts/automation/pipeline.py").read_text(encoding="utf-8")
PUBLISH = (ROOT / "scripts/automation/publish.py").read_text(encoding="utf-8")
MODULE_DOC = Path(__file__).read_text(encoding="utf-8")


class ReviewLoopContractTests(unittest.TestCase):
    def test_readonly_review_is_enforced_by_fingerprint_not_by_claim(self):
        """只读审查靠前后指纹对拍兜底：权限位可以被绕，字节对不上赖不掉。"""
        self.assertIn("readonly_source(review_wt, True)", PIPELINE,
                      "审查工作区的只读收紧不见了——口径引用的锚点漂了")
        self.assertIn("review_source_modified", PIPELINE,
                      "审查指纹对拍的拒绝码不见了——「宣称只读」不能代替对拍")
        # 对拍必须真的比 before/after 两份指纹，而不是只记录不比较
        self.assertRegex(
            PIPELINE,
            r"review_fingerprint_before[\s\S]{0,400}require_same_digest\(digest, after",
            "审查后的指纹必须与审查前对拍——记录两份指纹却不比较等于没钉",
        )

    def test_author_and_reviewer_identities_are_separated(self):
        self.assertIn("review_identity_invalid", PIPELINE,
                      "作者=审查者的拒绝码不见了——身份分离的锚点漂了")
        # 拒绝必须真的基于两个身份的比较，而不是只有一个错误码字符串
        self.assertRegex(
            PIPELINE,
            r"def validate_review[\s\S]{0,600}author[\s\S]{0,200}reviewer",
            "validate_review 必须比对 author 与 reviewer——只有一个错误码不算分离",
        )

    def test_revise_reenters_the_full_review_loop_with_a_bound(self):
        """修改后重审：revise 回到下一轮完整审查，且有轮次上限防无限循环。"""
        self.assertIn('if verdict == "revise":', PIPELINE,
                      "revise 分支不见了——修改后重审的回路断了")
        self.assertIn("revision_limit_exhausted", PIPELINE,
                      "轮次上限不见了——重审回路必须有界")
        self.assertIn("blocked_by_checks", PIPELINE,
                      "门禁先绿的前置不见了——审查不许替红门禁背书")

    def test_publish_never_merges_on_review_verdict_alone(self):
        """结果不越权批准：merge 路径永远先过 candidate.verify 的全链校验。"""
        merge_body = PUBLISH[PUBLISH.index("def merge("):PUBLISH.index("def rollback(")]
        self.assertIn(
            "candidate.verify(", merge_body[: merge_body.index("proc = _git(target, \"merge\"")],
            "merge 必须在真正 git merge 之前先过 candidate.verify——审查 verdict 不是合并凭证",
        )
        # 候选清单里审查是「报告字段」而不是「授权字段」
        self.assertIn('"review_kind"', PUBLISH)
        self.assertIn("independent_review", PUBLISH)
        self.assertNotIn(
            'if verified["independent_review"]:', PUBLISH,
            "审查种类不得成为合并开关——严格模式是显式旗标，不是隐式旁路",
        )

    def test_every_identifier_the_contract_names_exists_in_code(self):
        """doc→code 绑定：契约指名的每个标识符都必须在代码里找得到。"""
        named = set(re.findall(r"`([a-z_][a-z_0-9]{3,})`", CONTRACT))
        self.assertTrue(named, "契约里没有可核对的指名标识符——契约被改空了？")
        code = PIPELINE + PUBLISH
        missing = sorted(name for name in named if name not in code)
        self.assertEqual(missing, [], f"契约指名的锚点在代码里找不到：{missing}")

    def test_the_contract_and_the_gate_name_each_other(self):
        self.assertIn("test-review-loop-contract.py", CONTRACT,
                      "口径文档没有登记形状门禁——文档与门禁脱钩了")
        self.assertIn("审查闭环口径", MODULE_DOC, "门禁没有指认口径文档")


if __name__ == "__main__":
    unittest.main()
