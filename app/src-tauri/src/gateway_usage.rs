//! 四类协议的 **reasoning 内容形态**与 **usage 字段映射**判据（GW-03 第四片）。
//!
//! 需求原文：Chat/Responses/Messages/Gemini，流式/tools/reasoning/usage/cancel。
//! 场景 token：**reasoning 内容形态、usage 字段映射**。
//!
//! 两个问题的形状：
//!
//! 1. **推理内容不是答案**：四类协议把「推理」放在不同字段（Responses 的
//!    `reasoning` item、Chat 的 `reasoning_content`、Anthropic 的 `thinking`、
//!    Gemini 的 `thoughts`），而且**加密/被删减**的推理**没有文本**。
//!    归一时必须与最终回答**分开存放**——把推理当答案交出去，用户看到的是
//!    一段自言自语而不是结果。
//! 2. **usage 字段名各不相同**：输入 token 在 Chat 是 `prompt_tokens`、
//!    在 Responses 是 `input_tokens`；输出在 Chat 是 `completion_tokens`、
//!    在 Responses 是 `output_tokens`；缓存 token 又藏在各自的细节里。
//!    而且**缺失就是缺失**——折算成 0 会让「不知道花了多少」变成「没花钱」。
//!
//! 判据（纯函数，不碰网络）：
//!
//! - 推理归一后**只承载推理**，与回答分开；加密 ⇒ 无文本、删减 ⇒ 标记；
//! - usage 逐字段映射，**缺失用 `None` 表达**（不折算 0）；
//! - **reasoning token 不重复计入 output**（各协议口径不一，统一取不重复的那个）；
//! - 缓存命中的 token **单独列出**，不与输入 token 相加（相加会重复计费）。

use crate::gateway_protocol::Protocol;

/// 推理片段的形态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReasoningShape {
    /// 纯文本推理。
    Text,
    /// 加密推理（服务端不给明文）。
    Encrypted,
    /// 被删减的推理（服务端给了摘要或占位）。
    Redacted,
}

/// 归一后的推理片段。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReasoningFragment {
    pub shape: ReasoningShape,
    /// 明文内容。加密/删减时为 `None`——**不编造内容**。
    pub text: Option<String>,
}

/// 推理归一的拒绝原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsageRefusal {
    /// 载荷不是 JSON 对象。
    MalformedPayload,
}

impl UsageRefusal {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MalformedPayload => "malformed_payload",
        }
    }
}

/// 归一后的用量。
///
/// 每个字段都是 `Option`：**缺失就是缺失**。把缺失折算成 0 会让
/// 「不知道花了多少」变成「没花钱」——与 GW-08 的未知不折叠同口径。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NormalizedUsage {
    /// 输入 token（含缓存命中的部分，取决于协议口径）。
    pub input_tokens: Option<u64>,
    /// 输出 token（**不含** reasoning token，见 `map_usage`）。
    pub output_tokens: Option<u64>,
    /// 缓存命中的输入 token（单独列出，不与输入相加）。
    pub cached_input_tokens: Option<u64>,
    /// 推理 token（不计入 output_tokens，避免重复计费）。
    pub reasoning_tokens: Option<u64>,
}

impl NormalizedUsage {
    /// 是否**完全**缺失（一个字段都没有）。
    pub fn is_absent(&self) -> bool {
        self.input_tokens.is_none()
            && self.output_tokens.is_none()
            && self.cached_input_tokens.is_none()
            && self.reasoning_tokens.is_none()
    }
}

/// 归一某个协议的推理载荷。
///
/// 各协议把推理放在不同字段；找不到 ⇒ `Ok(None)`（**这个协议/这条消息没有推理**），
/// 而不是 `Err`——没有推理不是错误。
pub fn normalize_reasoning(
    protocol: Protocol,
    payload: &serde_json::Value,
) -> Result<Option<ReasoningFragment>, UsageRefusal> {
    let object = payload.as_object().ok_or(UsageRefusal::MalformedPayload)?;
    // 各协议的推理字段名
    let candidates: &[&str] = match protocol {
        Protocol::Chat => &["reasoning_content", "reasoning"],
        Protocol::Responses => &["reasoning", "reasoning_summary"],
        Protocol::Messages => &["thinking"],
        Protocol::Gemini => &["thoughts", "thought"],
    };
    for key in candidates {
        let Some(value) = object.get(*key) else { continue };
        // 加密推理：明文字段为空但带密文
        if let Some(encrypted) = value.get("encrypted_content").and_then(|v| v.as_str()) {
            if !encrypted.is_empty() {
                return Ok(Some(ReasoningFragment {
                    shape: ReasoningShape::Encrypted,
                    text: None,
                }));
            }
        }
        if let Some(text) = value.as_str() {
            if !text.trim().is_empty() {
                return Ok(Some(ReasoningFragment {
                    shape: ReasoningShape::Text,
                    text: Some(text.to_string()),
                }));
            }
            // 空的文本字段：内容被删减（服务端只留了空壳）
            return Ok(Some(ReasoningFragment {
                shape: ReasoningShape::Redacted,
                text: None,
            }));
        }
        // 对象形态里的 text 字段（如 `{"text": ""}` 的空壳推理）
        if let Some(text) = value.get("text").and_then(|v| v.as_str()) {
            if !text.trim().is_empty() {
                return Ok(Some(ReasoningFragment {
                    shape: ReasoningShape::Text,
                    text: Some(text.to_string()),
                }));
            }
            // 空壳：**推理被删减**，不是「没有推理」——两者对用户的含义完全不同
            return Ok(Some(ReasoningFragment {
                shape: ReasoningShape::Redacted,
                text: None,
            }));
        }
        // 对象形态：可能内含 summary 数组
        if let Some(summary) = value.get("summary").and_then(|v| v.as_array()) {
            let joined: Vec<String> = summary
                .iter()
                .filter_map(|item| item.get("text").and_then(|v| v.as_str()))
                .map(str::to_string)
                .collect();
            if !joined.is_empty() {
                return Ok(Some(ReasoningFragment {
                    shape: ReasoningShape::Text,
                    text: Some(joined.join("\n")),
                }));
            }
            return Ok(Some(ReasoningFragment {
                shape: ReasoningShape::Redacted,
                text: None,
            }));
        }
    }
    Ok(None)
}

/// 映射某协议的 usage 载荷到统一形态。
///
/// 缺失字段一律 `None`（不折算 0）。reasoning token **不**加进 output——
/// 有的协议把它含在 output 里、有的不含，统一取「不含」的那个，避免重复计费。
pub fn map_usage(
    protocol: Protocol,
    payload: &serde_json::Value,
) -> Result<NormalizedUsage, UsageRefusal> {
    let object = payload.as_object().ok_or(UsageRefusal::MalformedPayload)?;
    // 各协议的用量容器名不同（Responses/Chat 是 `usage`，Gemini 是 `usageMetadata`）
    let usage = ["usage", "usageMetadata"]
        .iter()
        .find_map(|key| object.get(*key))
        .unwrap_or(payload);
    let usage = usage.as_object().ok_or(UsageRefusal::MalformedPayload)?;

    // 键可能带点号表示嵌套路径（如 `input_tokens_details.cached_tokens`）——
    // `get` 不解析点号，必须自己逐段走。
    fn at_path<'a>(
        value: &'a serde_json::Map<String, serde_json::Value>,
        path: &str,
    ) -> Option<&'a serde_json::Value> {
        let mut segments = path.split('.');
        let mut current = value.get(segments.next()?)?;
        for segment in segments {
            current = current.as_object()?.get(segment)?;
        }
        Some(current)
    }

    let number = |keys: &[&str]| -> Option<u64> {
        for key in keys {
            if let Some(value) = at_path(usage, key).and_then(|v| v.as_u64()) {
                return Some(value);
            }
        }
        None
    };

    let (input_keys, output_keys, cached_keys, reasoning_keys): (
        &[&str],
        &[&str],
        &[&str],
        &[&str],
    ) = match protocol {
        Protocol::Chat => (
            &["prompt_tokens"],
            &["completion_tokens"],
            &["prompt_tokens_details.cached_tokens", "cached_tokens"],
            &["completion_tokens_details.reasoning_tokens", "reasoning_tokens"],
        ),
        Protocol::Responses => (
            &["input_tokens"],
            &["output_tokens"],
            &["input_tokens_details.cached_tokens", "cached_tokens"],
            &["output_tokens_details.reasoning_tokens", "reasoning_tokens"],
        ),
        Protocol::Messages => (
            &["input_tokens"],
            &["output_tokens"],
            &["cache_read_input_tokens"],
            &["output_tokens_details.reasoning_tokens", "reasoning_tokens"],
        ),
        Protocol::Gemini => (
            &["promptTokenCount", "prompt_token_count"],
            &["candidatesTokenCount", "candidates_token_count"],
            &["cachedContentTokenCount", "cached_content_token_count"],
            &["thoughtsTokenCount", "thoughts_token_count"],
        ),
    };

    let reasoning_tokens = number(reasoning_keys);
    // 输出 token 若**已含**推理 token，则减去推理部分（避免重复计费）；
    // 减到负数说明口径不同 ⇒ 不做减法（如实取原值）。
    let output_tokens = number(output_keys).map(|output| match reasoning_tokens {
        Some(reasoning) if output >= reasoning => output - reasoning,
        _ => output,
    });

    Ok(NormalizedUsage {
        input_tokens: number(input_keys),
        output_tokens,
        cached_input_tokens: number(cached_keys),
        reasoning_tokens,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reasoning_is_normalized_per_protocol_and_never_invented() {
        // Chat 的 reasoning_content
        let chat = json!({"reasoning_content": "先看看再决定"});
        let fragment = normalize_reasoning(Protocol::Chat, &chat).unwrap().unwrap();
        assert_eq!(fragment.shape, ReasoningShape::Text);
        assert_eq!(fragment.text.as_deref(), Some("先看看再决定"));

        // Responses 的 reasoning 摘要数组
        let responses = json!({"reasoning": {"summary": [{"text": "第一段"}, {"text": "第二段"}]}});
        let fragment = normalize_reasoning(Protocol::Responses, &responses).unwrap().unwrap();
        assert_eq!(fragment.text.as_deref(), Some("第一段\n第二段"));

        // Anthropic 的加密推理 ⇒ **无文本**，不编造
        let messages = json!({"thinking": {"encrypted_content": "密文"}});
        let fragment = normalize_reasoning(Protocol::Messages, &messages).unwrap().unwrap();
        assert_eq!(fragment.shape, ReasoningShape::Encrypted);
        assert_eq!(fragment.text, None, "加密推理不得编造明文");

        // 被删减的推理（空壳）⇒ 标记 Redacted
        let redacted = json!({"thinking": {"text": ""}});
        let fragment = normalize_reasoning(Protocol::Messages, &redacted).unwrap().unwrap();
        assert_eq!(fragment.shape, ReasoningShape::Redacted);
        assert_eq!(fragment.text, None);

        // 没有推理 ⇒ None（不是错误）
        assert_eq!(
            normalize_reasoning(Protocol::Chat, &json!({"content": "回答"}))
                .unwrap(),
            None
        );
    }

    #[test]
    fn usage_fields_are_mapped_per_protocol_and_missing_stays_missing() {
        // Chat：prompt_tokens / completion_tokens
        let chat = json!({"usage": {"prompt_tokens": 100, "completion_tokens": 50}});
        let usage = map_usage(Protocol::Chat, &chat).unwrap();
        assert_eq!(usage.input_tokens, Some(100));
        assert_eq!(usage.output_tokens, Some(50));
        assert_eq!(usage.cached_input_tokens, None, "缺失就是缺失，不折算 0");

        // Responses：input_tokens / output_tokens
        let responses = json!({"usage": {"input_tokens": 7, "output_tokens": 3}});
        let usage = map_usage(Protocol::Responses, &responses).unwrap();
        assert_eq!(usage.input_tokens, Some(7));
        assert_eq!(usage.output_tokens, Some(3));

        // Gemini：驼峰字段名
        let gemini = json!({"usageMetadata": {"promptTokenCount": 11, "candidatesTokenCount": 4}});
        let usage = map_usage(Protocol::Gemini, &gemini).unwrap();
        assert_eq!(usage.input_tokens, Some(11));
        assert_eq!(usage.output_tokens, Some(4));

        // 完全没有 usage ⇒ 全部缺失，且 is_absent 为真
        let usage = map_usage(Protocol::Chat, &json!({})).unwrap();
        assert!(usage.is_absent());
        assert_eq!(usage.input_tokens, None);
    }

    #[test]
    fn reasoning_tokens_are_not_counted_twice() {
        // output 已含 reasoning ⇒ 减去，避免重复计费
        let payload = json!({"usage": {
            "output_tokens": 100,
            "output_tokens_details": {"reasoning_tokens": 40}
        }});
        let usage = map_usage(Protocol::Responses, &payload).unwrap();
        assert_eq!(usage.reasoning_tokens, Some(40));
        assert_eq!(
            usage.output_tokens,
            Some(60),
            "输出 token 不含推理部分，避免与 reasoning_tokens 重复计"
        );
    }

    #[test]
    fn cached_tokens_are_listed_separately_not_added_to_input() {
        let payload = json!({"usage": {
            "input_tokens": 1000,
            "input_tokens_details": {"cached_tokens": 800}
        }});
        let usage = map_usage(Protocol::Responses, &payload).unwrap();
        assert_eq!(usage.input_tokens, Some(1000));
        assert_eq!(
            usage.cached_input_tokens,
            Some(800),
            "缓存命中单独列出，不与输入相加（相加会重复计费）"
        );
    }
}

