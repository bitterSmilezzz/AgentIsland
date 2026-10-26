//! 流式工具调用的**聚合判据**（GW-03 第三片）。
//!
//! 需求原文：Chat/Responses/Messages/Gemini，流式/tools/reasoning/usage/cancel；
//! 场景 token：**tools 分片/结果/并行**。
//!
//! 背景：`sse_parser.rs` 已把字节流解析成事件（SSE 任意切片安全），但事件本身
//! 仍是碎的——一次工具调用的参数会被拆成多个 delta 事件到达，一次对话里可能有
//! **多个并行**的工具调用，结果事件又要按 call id 归回各自。
//! 没有聚合层，转发层要么猜，要么把碎片当完整答案交出去。
//!
//! 判据（纯函数，不碰网络）：
//!
//! 1. **按 call id 归属**：每个调用有唯一 id；没有 id 的事件**无法归属** ⇒ 拒绝
//!    （`MissingCallId`），不并到「最近一个」调用上——那会把参数接到别的工具上。
//! 2. **参数分片按序拼接**：同一调用的参数片段依次追加；**乱序到达**判为
//!    协议违例（`OutOfOrderDelta`），不拼出一个错乱的参数串。
//! 3. **结果归属唯一**：结果按 call id 归回；**同一调用收到两个不同结果**
//!    ⇒ 冲突拒绝（`ConflictingResult`）——同一工具不可能有两个不同答案。
//! 4. **未闭合的调用如实标注**：流结束时仍有调用没有结果 ⇒ 标为
//!    `Incomplete`，**不**假装成功（半截参数被当成完整参数去执行是危险的）。
//! 5. **并行调用互不干扰**：不同 call id 各自独立累积，顺序按到达顺序。

use crate::sse_parser::SseEvent;
use std::collections::HashMap;

/// 一次工具调用的聚合状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCall {
    pub call_id: String,
    pub name: String,
    /// 累积的参数文本（分片拼接结果）。
    pub arguments: String,
    /// 收到的结果（`None` = 还没收到）。
    pub result: Option<String>,
    /// 参数是否已收完（协议发了 done 事件）。
    pub arguments_complete: bool,
}

/// 聚合后的一个调用摘要。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCallSummary {
    pub call_id: String,
    pub name: String,
    pub arguments: String,
    pub result: Option<String>,
    /// 参数与结果都齐了才算完成。
    pub complete: bool,
}

/// 聚合拒绝原因。固定码。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AggregateRefusal {
    /// 事件没有 call id，无法归属。
    MissingCallId,
    /// 同一调用的参数分片乱序到达。
    OutOfOrderDelta,
    /// 同一调用收到两个不同的结果。
    ConflictingResult,
    /// 事件类型不认识。
    UnknownEvent,
}

impl AggregateRefusal {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MissingCallId => "missing_call_id",
            Self::OutOfOrderDelta => "out_of_order_delta",
            Self::ConflictingResult => "conflicting_result",
            Self::UnknownEvent => "unknown_event",
        }
    }
}

/// 工具调用聚合器。
#[derive(Debug, Default)]
pub struct ToolCallAggregator {
    calls: Vec<ToolCall>,
    /// call id → 在 calls 里的下标。
    index: HashMap<String, usize>,
    /// 每个调用已收到的分片数（用于判定乱序：分片序号必须递增）。
    delta_counts: HashMap<String, u32>,
}

impl ToolCallAggregator {
    pub fn new() -> Self {
        Self::default()
    }

    /// 聚合一个流式事件。
    ///
    /// 事件形状（Responses 风格）：
    /// - `response.output_item.added`  → 新建调用（带 name）
    /// - `response.function_call_arguments.delta` → 参数分片
    /// - `response.function_call_arguments.done` → 参数收完
    /// - `response.output_item.done` → 结果
    ///
    /// 其余事件（文本 delta、usage 等）**不归本层管**，返回 `Ok(false)` 表示
    /// 「不是工具调用事件，已忽略」——它们由别的层消费。
    pub fn on_event(&mut self, event: &SseEvent) -> Result<bool, AggregateRefusal> {
        let payload = match event.event.as_str() {
            "response.output_item.added"
            | "response.function_call_arguments.delta"
            | "response.function_call_arguments.done"
            | "response.output_item.done" => match parse_payload(&event.data) {
                Some(payload) => payload,
                None => return Err(AggregateRefusal::MissingCallId),
            },
            _ => return Ok(false),
        };
        let call_id = payload
            .get("call_id")
            .and_then(|v| v.as_str())
            .filter(|id| !id.is_empty())
            .ok_or(AggregateRefusal::MissingCallId)?
            .to_string();

        match event.event.as_str() {
            "response.output_item.added" => {
                let name = payload
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                if let Some(&at) = self.index.get(&call_id) {
                    // 同一 id 再次「新建」⇒ 参数/名称不一致的重复声明
                    let existing = &self.calls[at];
                    if existing.name != name {
                        return Err(AggregateRefusal::ConflictingResult);
                    }
                    return Ok(true);
                }
                self.index.insert(call_id.clone(), self.calls.len());
                self.delta_counts.insert(call_id.clone(), 0);
                self.calls.push(ToolCall {
                    call_id,
                    name,
                    arguments: String::new(),
                    result: None,
                    arguments_complete: false,
                });
                Ok(true)
            }
            "response.function_call_arguments.delta" => {
                let at = *self
                    .index
                    .get(&call_id)
                    .ok_or(AggregateRefusal::MissingCallId)?;
                let delta = payload.get("delta").and_then(|v| v.as_str()).unwrap_or("");
                let count = self.delta_counts.entry(call_id.clone()).or_insert(0);
                // 分片序号必须**递增**：协议用序号标注顺序，乱序即协议违例。
                let sequence = payload.get("seq").and_then(|v| v.as_u64());
                if let Some(sequence) = sequence {
                    if sequence < u64::from(*count) {
                        return Err(AggregateRefusal::OutOfOrderDelta);
                    }
                    *count = sequence as u32 + 1;
                } else {
                    *count += 1;
                }
                self.calls[at].arguments.push_str(delta);
                Ok(true)
            }
            "response.function_call_arguments.done" => {
                let at = *self
                    .index
                    .get(&call_id)
                    .ok_or(AggregateRefusal::MissingCallId)?;
                self.calls[at].arguments_complete = true;
                Ok(true)
            }
            "response.output_item.done" => {
                let at = *self
                    .index
                    .get(&call_id)
                    .ok_or(AggregateRefusal::MissingCallId)?;
                let result = payload.get("result").and_then(|v| v.as_str()).map(str::to_string);
                if let (Some(existing), Some(incoming)) = (&self.calls[at].result, &result) {
                    if existing != incoming {
                        return Err(AggregateRefusal::ConflictingResult);
                    }
                } else if result.is_some() {
                    self.calls[at].result = result;
                }
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    /// 流结束时的摘要。**未闭合的调用标为不完整**，不假装成功。
    pub fn finalize(self) -> Vec<ToolCallSummary> {
        self.calls
            .into_iter()
            .map(|call| {
                let complete = call.arguments_complete && call.result.is_some();
                ToolCallSummary {
                    call_id: call.call_id,
                    name: call.name,
                    arguments: call.arguments,
                    result: call.result,
                    complete,
                }
            })
            .collect()
    }

    /// 当前已见到的调用数。
    pub fn call_count(&self) -> usize {
        self.calls.len()
    }
}

/// 解析事件数据（必须是 JSON 对象）。
fn parse_payload(data: &str) -> Option<serde_json::Value> {
    let value: serde_json::Value = serde_json::from_str(data).ok()?;
    if value.is_object() {
        Some(value)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(name: &str, data: &str) -> SseEvent {
        SseEvent {
            event: name.into(),
            data: data.into(),
            id: None,
            retry_ms: None,
        }
    }

    #[test]
    fn argument_fragments_are_joined_in_order() {
        let mut agg = ToolCallAggregator::new();
        agg.on_event(&event(
            "response.output_item.added",
            r#"{"call_id":"c1","name":"shell"}"#,
        ))
        .unwrap();
        agg.on_event(&event(
            "response.function_call_arguments.delta",
            r#"{"call_id":"c1","delta":"{\"cmd\":"}"#,
        ))
        .unwrap();
        agg.on_event(&event(
            "response.function_call_arguments.delta",
            r#"{"call_id":"c1","delta":"\"ls\"}"}"#,
        ))
        .unwrap();
        agg.on_event(&event(
            "response.function_call_arguments.done",
            r#"{"call_id":"c1"}"#,
        ))
        .unwrap();
        agg.on_event(&event(
            "response.output_item.done",
            r#"{"call_id":"c1","result":"文件列表"}"#,
        ))
        .unwrap();

        let summaries = agg.finalize();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].arguments, r#"{"cmd":"ls"}"#);
        assert_eq!(summaries[0].result.as_deref(), Some("文件列表"));
        assert!(summaries[0].complete, "参数与结果都齐才算完成");
    }

    #[test]
    fn parallel_calls_stay_independent() {
        let mut agg = ToolCallAggregator::new();
        agg.on_event(&event(
            "response.output_item.added",
            r#"{"call_id":"c1","name":"read"}"#,
        ))
        .unwrap();
        agg.on_event(&event(
            "response.output_item.added",
            r#"{"call_id":"c2","name":"grep"}"#,
        ))
        .unwrap();
        agg.on_event(&event(
            "response.function_call_arguments.delta",
            r#"{"call_id":"c2","delta":"--all"}"#,
        ))
        .unwrap();
        agg.on_event(&event(
            "response.function_call_arguments.delta",
            r#"{"call_id":"c1","delta":"file.rs"}"#,
        ))
        .unwrap();

        let summaries = agg.finalize();
        assert_eq!(summaries.len(), 2, "并行调用各自独立");
        let by_id = |id: &str| {
            summaries
                .iter()
                .find(|s| s.call_id == id)
                .cloned()
                .unwrap()
        };
        assert_eq!(by_id("c1").arguments, "file.rs");
        assert_eq!(by_id("c2").arguments, "--all");
    }

    #[test]
    fn an_event_without_a_call_id_cannot_be_attributed() {
        let mut agg = ToolCallAggregator::new();
        // 没有 call id ⇒ 拒绝（不并到「最近一个」调用上——那会把参数接到别的工具上）
        assert_eq!(
            agg.on_event(&event(
                "response.function_call_arguments.delta",
                r#"{"delta":"片段"}"#,
            )),
            Err(AggregateRefusal::MissingCallId)
        );
        // 未知调用 id 的分片同样拒绝（不能凭空创建）
        let mut agg2 = ToolCallAggregator::new();
        assert_eq!(
            agg2.on_event(&event(
                "response.function_call_arguments.delta",
                r#"{"call_id":"不存在","delta":"x"}"#,
            )),
            Err(AggregateRefusal::MissingCallId)
        );
    }

    #[test]
    fn conflicting_results_for_the_same_call_are_refused() {
        let mut agg = ToolCallAggregator::new();
        agg.on_event(&event(
            "response.output_item.added",
            r#"{"call_id":"c1","name":"shell"}"#,
        ))
        .unwrap();
        agg.on_event(&event(
            "response.output_item.done",
            r#"{"call_id":"c1","result":"结果 A"}"#,
        ))
        .unwrap();
        // 同一调用收到**不同**结果 ⇒ 冲突
        assert_eq!(
            agg.on_event(&event(
                "response.output_item.done",
                r#"{"call_id":"c1","result":"结果 B"}"#,
            )),
            Err(AggregateRefusal::ConflictingResult),
            "同一工具不可能有两个不同答案"
        );
        // 相同结果的重报是幂等，允许
        assert_eq!(
            agg.on_event(&event(
                "response.output_item.done",
                r#"{"call_id":"c1","result":"结果 A"}"#,
            )),
            Ok(true)
        );
    }

    #[test]
    fn an_unfinished_call_is_reported_as_incomplete_not_as_success() {
        let mut agg = ToolCallAggregator::new();
        agg.on_event(&event(
            "response.output_item.added",
            r#"{"call_id":"c1","name":"shell"}"#,
        ))
        .unwrap();
        agg.on_event(&event(
            "response.function_call_arguments.delta",
            r#"{"call_id":"c1","delta":"半截"}"#,
        ))
        .unwrap();
        // 流在此结束：c1 没有参数 done、也没有结果
        let summaries = agg.finalize();
        assert_eq!(summaries.len(), 1);
        assert!(
            !summaries[0].complete,
            "未闭合的调用必须标为不完整——半截参数被当成完整参数去执行是危险的"
        );
        assert_eq!(summaries[0].arguments, "半截");
        assert_eq!(summaries[0].result, None);
    }

    #[test]
    fn out_of_order_fragments_are_refused() {
        let mut agg = ToolCallAggregator::new();
        agg.on_event(&event(
            "response.output_item.added",
            r#"{"call_id":"c1","name":"shell"}"#,
        ))
        .unwrap();
        // 带 seq 的分片：先到 seq=1，再来 seq=0 ⇒ 乱序
        agg.on_event(&event(
            "response.function_call_arguments.delta",
            r#"{"call_id":"c1","seq":1,"delta":"后"}"#,
        ))
        .unwrap();
        assert_eq!(
            agg.on_event(&event(
                "response.function_call_arguments.delta",
                r#"{"call_id":"c1","seq":0,"delta":"前"}"#,
            )),
            Err(AggregateRefusal::OutOfOrderDelta),
            "乱序的分片不能拼成一个错乱的参数串"
        );
    }

    #[test]
    fn non_tool_events_are_ignored_by_this_layer() {
        let mut agg = ToolCallAggregator::new();
        // 文本 delta 与 usage 不归本层管 ⇒ 返回 Ok(false)（不是错误）
        assert_eq!(
            agg.on_event(&event("response.output_text.delta", r#"{"text":"你好"}"#)),
            Ok(false)
        );
        assert_eq!(agg.call_count(), 0);
    }
}
