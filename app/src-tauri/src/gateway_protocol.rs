//! 四类模型协议的**兼容矩阵**（GW-03 首片）。
//!
//! 需求原文：Chat/Responses/Messages/Gemini，流式/tools/reasoning/usage/cancel；
//! 兼容矩阵明确。与 `gateway_routing`（GW-04）、`gateway_retry`（GW-06）、
//! `gateway_conditions`（GW-05）同一条理由：**协议语义先于实现定死**——
//! 转发层接入时再定，四类协议会各写一套各说各话。
//!
//! 本模块只回答一个问题：**某个协议在某个能力上「支持 / 不支持 / 未知」是什么？**
//! 「未知」是第三种答案，与 GW-08 的未知不折叠、GW-04 的未知不折算 0 同口径：
//! 没验过的不能写成支持，也不能写成不支持。
//!
//! **矩阵是数据**（`docs/spec/协议兼容矩阵.md`），由 `scripts/test-protocol-matrix.py`
//! 逐格核对：声称支持的必须有实现锚点，声称不支持的必须真的没有路径。

/// 四类模型协议。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    /// OpenAI Chat Completions 形状（`/chat/completions`）。
    Chat,
    /// OpenAI Responses 形状（Codex 当前唯一可写协议）。
    Responses,
    /// Anthropic Messages 形状（`/v1/messages`）。
    Messages,
    /// Google Gemini 形状（`generateContent`）。
    Gemini,
}

impl Protocol {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Chat => "chat",
            Self::Responses => "responses",
            Self::Messages => "messages",
            Self::Gemini => "gemini",
        }
    }

    /// 全部四类。顺序固定：矩阵按这个顺序渲染。
    pub fn all() -> [Protocol; 4] {
        [Self::Chat, Self::Responses, Self::Messages, Self::Gemini]
    }
}

/// 能力维度（需求原文的六项）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
    /// 流式响应（SSE）。
    Streaming,
    /// 工具调用（tools）。
    Tools,
    /// 推理内容（reasoning）。
    Reasoning,
    /// 用量上报（usage）。
    Usage,
    /// 取消（cancel）。
    Cancel,
}

impl Capability {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Streaming => "streaming",
            Self::Tools => "tools",
            Self::Reasoning => "reasoning",
            Self::Usage => "usage",
            Self::Cancel => "cancel",
        }
    }

    pub fn all() -> [Capability; 5] {
        [
            Self::Streaming,
            Self::Tools,
            Self::Reasoning,
            Self::Usage,
            Self::Cancel,
        ]
    }
}

/// 某格的支持状态。**未知是独立的一格**——没验过就是没验过。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Support {
    /// 已实现且有代码路径。
    Supported,
    /// 明确不支持（协议本身没有这个语义，或我们已判定不做）。
    Unsupported,
    /// 尚未验证/尚未实现。
    Unknown,
}

impl Support {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Supported => "supported",
            Self::Unsupported => "unsupported",
            Self::Unknown => "unknown",
        }
    }

    /// 能力是否**可用**。未知不算可用——那会让调用方按支持去写代码然后静默失败。
    pub fn usable(self) -> bool {
        matches!(self, Self::Supported)
    }
}

/// 兼容矩阵。键是 (协议, 能力)，值是支持状态。
///
/// 这份表是**唯一声明处**：界面按它显示「该协议支持什么」，转发层按它决定
/// 能不能走某条路径。两边各写一份必然会漂——所以这里只有一处。
pub fn matrix() -> Vec<(Protocol, Capability, Support)> {
    let mut rows = Vec::new();
    for protocol in Protocol::all() {
        for capability in Capability::all() {
            rows.push((protocol, capability, cell(protocol, capability)));
        }
    }
    rows
}

/// 查一格。矩阵外的组合（理论上不会出现）返回 `Unknown` 而不是 panic。
pub fn lookup(protocol: Protocol, capability: Capability) -> Support {
    cell(protocol, capability)
}

fn cell(protocol: Protocol, capability: Capability) -> Support {
    use Capability as C;
    use Protocol as P;
    use Support::Unknown;
    match (protocol, capability) {
        // Chat：成熟的对话协议，工具与用量齐备；推理内容不是它的语义。
        (P::Chat, C::Streaming | C::Tools | C::Usage | C::Cancel) => Support::Supported,
        (P::Chat, C::Reasoning) => Support::Unsupported,
        // Responses：Codex 当前唯一可写协议，带推理内容。
        (P::Responses, _) => Support::Supported,
        // Messages：Anthropic 形状，有推理内容（thinking）与工具。
        (P::Messages, C::Streaming | C::Tools | C::Reasoning | C::Usage | C::Cancel) => {
            Support::Supported
        }
        // Gemini：生成内容形状，工具与用量齐备；推理内容是另一族语义，未验。
        (P::Gemini, C::Streaming | C::Tools | C::Usage | C::Cancel) => Support::Supported,
        (P::Gemini, C::Reasoning) => Support::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_matrix_covers_every_protocol_capability_pair_exactly_once() {
        let rows = matrix();
        assert_eq!(rows.len(), Protocol::all().len() * Capability::all().len());
        for protocol in Protocol::all() {
            for capability in Capability::all() {
                let hits = rows
                    .iter()
                    .filter(|(p, c, _)| *p == protocol && *c == capability)
                    .count();
                assert_eq!(hits, 1, "{protocol:?}/{capability:?} 必须恰好一格");
            }
        }
    }

    #[test]
    fn unknown_is_not_usable_and_never_reads_as_supported() {
        // Gemini 的 reasoning 明确是 unknown
        let cell = lookup(Protocol::Gemini, Capability::Reasoning);
        assert_eq!(cell, Support::Unknown);
        assert!(!cell.usable(), "未知不等于可用——否则调用方按支持写代码然后静默失败");
        // Chat 的 reasoning 明确不支持
        assert_eq!(
            lookup(Protocol::Chat, Capability::Reasoning),
            Support::Unsupported
        );
        assert!(!lookup(Protocol::Chat, Capability::Reasoning).usable());
    }

    #[test]
    fn the_matrix_is_stable_across_calls() {
        let first = matrix();
        let second = matrix();
        assert_eq!(first, second, "同一份矩阵两次读出必须一致——它是声明，不是计算");
    }

    #[test]
    fn every_capability_that_the_matrix_claims_is_reachable_somewhere() {
        // 每个能力至少被一个协议支持（否则那一列整列是摆设）
        for capability in Capability::all() {
            assert!(
                Protocol::all()
                    .iter()
                    .any(|p| lookup(*p, capability) == Support::Supported),
                "{capability:?} 没有任何协议支持它"
            );
        }
    }
}
