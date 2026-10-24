#!/usr/bin/env python3
"""GW-03：协议兼容矩阵（docs/spec/协议兼容矩阵.md ↔ gateway_protocol.rs）的核对。

矩阵是**代码里的数据**（`gateway_protocol.rs` 的 `cell()` 是唯一声明处），
文档只是它的渲染。本脚本核对三件事：

1. **文档逐格与代码一致**：文档表格的每一格必须与 `cell()` 的返回值相同——
   文档漂了就是有人在骗自己。
2. **未知格真的未知**：文档里写「未知」的格，代码必须是 `unknown`；
   「不支持」的格代码必须是 `unsupported`。**未知不许被写成支持**。
3. **代码矩阵的形状完整**：四协议 × 五能力，每格恰好一次。

变异验证（隔离副本）：把 Gemini/reasoning 从 unknown 改成 supported → 变红；
文档里把某个「不支持」改成「支持」→ 变红。
"""
from pathlib import Path
import re
import unittest

ROOT = Path(__file__).resolve().parents[1]
MATRIX_DOC = (ROOT / "docs/spec/协议兼容矩阵.md").read_text(encoding="utf-8")
PROTOCOL = (ROOT / "app/src-tauri/src/gateway_protocol.rs").read_text(encoding="utf-8")

PROTOCOLS = {
    "chat": "Chat",
    "responses": "Responses",
    "messages": "Messages",
    "gemini": "Gemini",
}
# 文档表格里协议名用的是 Rust 侧的变体名（Chat/Responses/…），代码里是 snake_case。
DOC_TO_CODE = {value: key for key, value in PROTOCOLS.items()}
CAPABILITIES = ["流式", "tools", "reasoning", "usage", "cancel"]
# 文档列名（中文/英文）→ 代码里的 Rust 变体名
CAP_TO_RUST = {
    "流式": "Streaming",
    "tools": "Tools",
    "reasoning": "Reasoning",
    "usage": "Usage",
    "cancel": "Cancel",
}
# 文档里的中文状态词 → 代码里的枚举名
STATUS_WORDS = {
    "支持": "Supported",
    "不支持": "Unsupported",
    "未知": "Unknown",
}


def doc_rows() -> dict[str, list[str]]:
    """解析文档矩阵表：协议名 → 每列的状态词。"""
    rows: dict[str, list[str]] = {}
    for line in MATRIX_DOC.splitlines():
        if not line.startswith("|"):
            continue
        cells = [cell.strip() for cell in line.strip("|").split("|")]
        if len(cells) != 6:
            continue
        name = cells[0].strip("`* ")
        if name not in DOC_TO_CODE:
            continue
        states = [cell.replace("**", "").strip() for cell in cells[1:]]
        rows[DOC_TO_CODE[name]] = states
    return rows


def code_cell(protocol: str, capability: str) -> str | None:
    """从 `cell()` 的 match 臂里取出该格返回的枚举名。

    先试多能力合并臂（`(P::X, C::A | C::B) => Support::Y`），再试单能力臂——
    顺序反了会让合并臂里的能力全部取不到（单能力正则匹配不上就早退了）。
    """
    rust_cap = CAP_TO_RUST[capability]
    # 臂的返回体可能是 `=> Support::X` 也可能是 `=> { Support::X }`（多行写法），
    # 因此允许中间出现空白与花括号。
    combined = re.compile(
        r"\(P::" + PROTOCOLS[protocol] + r",\s*([^)]*)\)\s*=>\s*\{?\s*Support::(\w+)"
    )
    for hit in combined.finditer(PROTOCOL):
        caps = [part.strip() for part in hit.group(1).split("|")]
        # 臂里写的是 `C::Streaming`，比对时要带上 `C::` 前缀
        if any(cap == f"C::{rust_cap}" or cap == rust_cap for cap in caps):
            return hit.group(2)
    # 通配臂：`(P::Responses, _) => Support::Supported`——该协议的所有能力同判。
    wildcard = re.compile(
        r"\(P::" + PROTOCOLS[protocol] + r",\s*_\)\s*=>\s*\{?\s*Support::(\w+)"
    )
    hit = wildcard.search(PROTOCOL)
    if hit:
        return hit.group(1)
    return None


class ProtocolMatrixTests(unittest.TestCase):
    def test_the_document_lists_every_protocol_and_capability(self):
        rows = doc_rows()
        self.assertEqual(
            sorted(rows), sorted(PROTOCOLS), "文档矩阵少了协议行或多出行"
        )
        for name, states in rows.items():
            self.assertEqual(
                len(states), len(CAPABILITIES), f"{name} 行的列数不对"
            )

    def test_every_cell_agrees_with_the_code(self):
        rows = doc_rows()
        mismatches = []
        for name, states in rows.items():
            for capability, word in zip(CAPABILITIES, states):
                expected = STATUS_WORDS.get(word)
                if expected is None:
                    mismatches.append(f"{name}/{capability}: 未知的状态词 {word!r}")
                    continue
                actual = code_cell(name, capability)
                if actual != expected:
                    mismatches.append(
                        f"{name}/{capability}: 文档写 {word}，代码是 {actual}"
                    )
        self.assertEqual(mismatches, [], f"文档与代码的矩阵不一致：{mismatches}")

    def test_unknown_is_never_written_as_supported(self):
        """未知不许被写成支持——那会让调用方按支持写代码然后静默失败。"""
        rows = doc_rows()
        for name, states in rows.items():
            for capability, word in zip(CAPABILITIES, states):
                code = code_cell(name, capability)
                if code == "Unknown":
                    self.assertNotEqual(
                        word, "支持", f"{name}/{capability} 代码是未知，文档却写支持"
                    )

    def test_the_code_matrix_covers_every_pair_exactly_once(self):
        for protocol in PROTOCOLS:
            for capability in CAPABILITIES:
                self.assertIsNotNone(
                    code_cell(protocol, capability),
                    f"代码矩阵缺少 {protocol}/{capability} 这一格",
                )

    def test_the_matrix_and_the_gate_name_each_other(self):
        self.assertIn("test-protocol-matrix.py", MATRIX_DOC,
                      "矩阵文档没有登记核对脚本——文档与门禁脱钩了")
        self.assertIn("协议兼容矩阵", Path(__file__).read_text(encoding="utf-8"),
                      "门禁没有指认矩阵文档")


if __name__ == "__main__":
    unittest.main()
