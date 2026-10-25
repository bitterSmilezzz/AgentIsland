//! SSE（Server-Sent Events）**增量解析器**（GW-03 第二片）。
//!
//! 需求原文：四类模型协议网关，流式/tools/reasoning/usage/cancel；
//! 场景 token：**SSE 任意切片**；畸形/过深/超限；取消/流中断。
//!
//! 「任意切片」是流式解析的核心难点：网络分片**不按行边界切**，可能切在
//! `\r\n` 中间、切在字段名中间、切在一个 UTF-8 多字节字符中间。解析器必须
//! **增量**工作：喂进任意字节序列，产出的事件序列与「一次性喂入完整流」完全一致。
//!
//! 本模块只做**解析**（纯函数/无 IO，不发请求、不解析业务 JSON）：
//!
//! 1. **任意切片安全**：字节级增量缓冲，按 `\r\n` / `\n` / `\r` 三种行结束符
//!    切行；行结束符被切在中间时不误判。
//! 2. **SSE 字段语义**：`data:`（多行以 `\n` 拼接）、`event:`、`id:`、`retry:`；
//!    `:` 开头的注释行忽略；空行触发事件派发。
//! 3. **有界**：单行长度、单事件缓冲、事件条数都有上限，超限**明确拒绝**
//!    （`LineTooLong` / `EventTooLarge` / `TooManyEvents`），不静默截断。
//! 4. **取消与流中断**：取消后丢弃缓冲并拒绝后续字节（`Cancelled`）——
//!    取消后仍在解析，等于把用户以为已经停掉的流继续写进界面。

/// 单行最大字节。
pub const MAX_LINE_BYTES: usize = 64 * 1024;
/// 单个事件（data 累计）最大字节。
pub const MAX_EVENT_BYTES: usize = 256 * 1024;
/// 单次解析最多产出多少事件。
pub const MAX_EVENTS: usize = 10_000;

/// 解析错误。固定码。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SseError {
    /// 单行超过上限。
    LineTooLong,
    /// 单个事件的 data 累计超过上限。
    EventTooLarge,
    /// 事件条数超过上限。
    TooManyEvents,
    /// 已取消：取消后的字节不再被接受。
    Cancelled,
}

impl SseError {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LineTooLong => "line_too_long",
            Self::EventTooLarge => "event_too_large",
            Self::TooManyEvents => "too_many_events",
            Self::Cancelled => "cancelled",
        }
    }
}

/// 一个已派发的 SSE 事件。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SseEvent {
    /// 事件名（`event:` 字段；缺省为空）。
    pub event: String,
    /// 数据（多行 `data:` 以 `\n` 拼接；无 data 字段则为空串）。
    pub data: String,
    /// 最后一次 `id:` 字段的值。
    pub id: Option<String>,
    /// `retry:` 字段（毫秒）。
    pub retry_ms: Option<u64>,
}

/// 增量 SSE 解析器。
#[derive(Debug, Default)]
pub struct SseParser {
    /// 未成行的字节缓冲。
    line_buffer: Vec<u8>,
    /// 当前事件的累积 data 行。
    data_lines: Vec<String>,
    /// 当前事件名。
    event_name: String,
    /// 当前事件 id。
    event_id: Option<String>,
    /// 当前事件 retry。
    event_retry: Option<u64>,
    /// 累计事件数。
    events_seen: usize,
    /// 是否已取消。
    cancelled: bool,
    /// 上一个字节是否为 `\r`（用于识别被切开的 CRLF：紧随的 `\n` 须跳过）。
    skip_lf: bool,
}

impl SseParser {
    pub fn new() -> Self {
        Self::default()
    }

    /// 取消解析：丢弃缓冲，之后的字节一律拒绝。
    pub fn cancel(&mut self) {
        self.cancelled = true;
        self.line_buffer.clear();
        self.data_lines.clear();
        self.event_name.clear();
        self.event_id = None;
        self.event_retry = None;
    }

    /// 喂入一段字节，返回本次派发的全部事件。
    ///
    /// **任意切片安全**：同一段字节流无论怎么切分，多次 `feed` 的产出
    /// 事件序列必须与一次性喂入完全一致（由用例逐位切分验证）。
    pub fn feed(&mut self, chunk: &[u8]) -> Result<Vec<SseEvent>, SseError> {
        if self.cancelled {
            return Err(SseError::Cancelled);
        }
        let mut events = Vec::new();
        for &byte in chunk {
            // CRLF 被切在中间时：`\r` 已结束行，紧随的 `\n` 必须被跳过，
            // 否则同一个行结束会被算两次（多出一个空行 → 多派发一个空事件）。
            if self.skip_lf {
                self.skip_lf = false;
                if byte == b'\n' {
                    continue;
                }
            }
            match byte {
                b'\r' => {
                    self.skip_lf = true;
                    events.extend(self.finish_line()?);
                }
                b'\n' => {
                    events.extend(self.finish_line()?);
                }
                _ => {
                    self.line_buffer.push(byte);
                    if self.line_buffer.len() > MAX_LINE_BYTES {
                        return Err(SseError::LineTooLong);
                    }
                }
            }
            if self.events_seen > MAX_EVENTS {
                return Err(SseError::TooManyEvents);
            }
        }
        Ok(events)
    }

    /// 处理一条完整行。
    fn finish_line(&mut self) -> Result<Vec<SseEvent>, SseError> {
        let line = std::mem::take(&mut self.line_buffer);
        // 非 UTF-8 的行按替换字符处理，不让一个坏字节毁掉整个流
        let text = String::from_utf8_lossy(&line).into_owned();
        let text = text.trim_end_matches('\r');

        // 空行 ⇒ 派发事件
        if text.is_empty() {
            return Ok(self.dispatch());
        }
        // 注释行忽略
        if text.starts_with(':') {
            return Ok(Vec::new());
        }

        // 字段解析：name:value（value 前导空格去掉）
        let (name, value) = match text.split_once(':') {
            Some((name, value)) => (name, value.strip_prefix(' ').unwrap_or(value)),
            // 没有冒号 ⇒ 整行是字段名，值为空串
            None => (&text[..], ""),
        };
        match name {
            "data" => {
                self.data_lines.push(value.to_string());
                let total: usize = self.data_lines.iter().map(|l| l.len() + 1).sum();
                if total > MAX_EVENT_BYTES {
                    return Err(SseError::EventTooLarge);
                }
            }
            "event" => self.event_name = value.to_string(),
            "id" => self.event_id = Some(value.to_string()),
            "retry" => {
                if let Ok(parsed) = value.parse::<u64>() {
                    self.event_retry = Some(parsed);
                }
                // 非法 retry 按 SSE 规范忽略
            }
            _ => {} // 未知字段忽略
        }
        Ok(Vec::new())
    }

    /// 派发当前累积的事件（空行触发）。
    fn dispatch(&mut self) -> Vec<SseEvent> {
        if self.data_lines.is_empty() && self.event_name.is_empty() && self.event_id.is_none() {
            // 空事件（只有注释行）不派发
            self.event_retry = None;
            return Vec::new();
        }
        self.events_seen += 1;
        let event = SseEvent {
            event: std::mem::take(&mut self.event_name),
            data: self.data_lines.join("\n"),
            id: self.event_id.take(),
            retry_ms: self.event_retry.take(),
        };
        self.data_lines.clear();
        vec![event]
    }

    /// 解析完整流（便捷入口：一次性喂入并派发末尾事件）。
    pub fn parse_all(&mut self, bytes: &[u8]) -> Result<Vec<SseEvent>, SseError> {
        let mut events = self.feed(bytes)?;
        // 流末尾若没有以空行结束，最后一个事件也要派发
        events.extend(self.dispatch());
        Ok(events)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Vec<SseEvent> {
        SseParser::new().parse_all(text.as_bytes()).unwrap()
    }

    #[test]
    fn basic_events_are_parsed_with_fields() {
        let events = parse("event: delta\ndata: {\"a\":1}\nid: 7\nretry: 1500\n\n");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event, "delta");
        assert_eq!(events[0].data, "{\"a\":1}");
        assert_eq!(events[0].id.as_deref(), Some("7"));
        assert_eq!(events[0].retry_ms, Some(1500));
    }

    #[test]
    fn multi_line_data_is_joined_with_newlines_and_comments_ignored() {
        let events = parse(": keep-alive\ndata: line1\ndata: line2\n\n");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, "line1\nline2", "多行 data 以换行拼接");
    }

    #[test]
    fn arbitrary_chunking_yields_the_same_events_as_one_shot() {
        // 混合 CRLF、LF、单独 CR 的流
        let stream = "event: a\r\ndata: one\r\n\r\nevent: b\ndata: two\n\nevent: c\rdata: three\r\r";
        let one_shot = parse(stream);

        // 逐字节喂入（最坏的切片：每个字节一次，包括 CRLF 被拆两半）
        let mut parser = SseParser::new();
        let mut collected = Vec::new();
        for byte in stream.as_bytes() {
            collected.extend(parser.feed(&[*byte]).unwrap());
        }
        collected.extend(parser.dispatch());
        assert_eq!(collected, one_shot, "逐字节切片必须与一次性解析完全一致");

        // 两字节一组的切片（含 CRLF 被切开的情况）
        let mut parser = SseParser::new();
        let mut collected = Vec::new();
        let bytes = stream.as_bytes();
        for pair in bytes.chunks(2) {
            collected.extend(parser.feed(pair).unwrap());
        }
        collected.extend(parser.dispatch());
        assert_eq!(collected, one_shot, "CRLF 被切成两半也必须正确");
    }

    #[test]
    fn crlf_and_lf_line_endings_both_work() {
        let lf = parse("data: x\n\n");
        let crlf = parse("data: x\r\n\r\n");
        assert_eq!(lf, crlf);
        let cr = parse("data: x\r\r");
        assert_eq!(cr, lf, "CR 单独也是行结束符");
    }

    #[test]
    fn oversized_line_event_and_count_are_refused_explicitly() {
        let mut parser = SseParser::new();
        let huge = vec![b'x'; MAX_LINE_BYTES + 10];
        assert_eq!(parser.feed(&huge).unwrap_err(), SseError::LineTooLong);

        let mut parser = SseParser::new();
        // 单个事件超限：多行累加（每行在 64KB 内，累计超过 256KB）
        let piece = "data: ".to_string() + &"y".repeat(60 * 1024) + "\n";
        let mut big = piece.repeat(5); // 5 * 60KB = 300KB > 256KB
        big.push('\n');
        assert_eq!(parser.feed(big.as_bytes()).unwrap_err(), SseError::EventTooLarge);

        let mut parser = SseParser::new();
        let mut stream = String::new();
        for i in 0..(MAX_EVENTS + 10) {
            stream.push_str(&format!("data: {i}\n\n"));
        }
        assert_eq!(parser.feed(stream.as_bytes()).unwrap_err(), SseError::TooManyEvents);
    }

    #[test]
    fn cancel_discards_buffer_and_refuses_further_bytes() {
        let mut parser = SseParser::new();
        parser.feed(b"data: partial").unwrap();
        parser.cancel();
        assert_eq!(parser.feed(b"\n\n").unwrap_err(), SseError::Cancelled);
        assert_eq!(
            parser.parse_all(b"data: more\n\n").unwrap_err(),
            SseError::Cancelled,
            "取消后仍在解析，等于把用户以为已经停掉的流继续写进界面"
        );
    }
}
