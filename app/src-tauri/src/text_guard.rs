//! 不可信自由文本的**唯一清洗口**（SEC-01：第三方恶意输入）。
//!
//! 来源：webhook 的 `/notify`·`/event` 通知文本与 `/session` 自报的动作文本——
//! 本机任何一个进程都能投进来。去处：UI（另有 escapeHtml）、macOS 通知正文、
//! 审计/报表导出、终端打印。清洗在**入口一次**做，下游所有表面继承同一份结果。
//!
//! 防三类事：
//! 1. **ANSI/控制字符注入**：C0/C1 控制字符在终端里是命令（`report` 会把事件文本
//!    印到 stdout），在通知与 UI 里是隐形内容。保留 `\n` 与 `\t`（正当排版），其余剥掉。
//! 2. **双向覆写伪装**（Trojan Source，U+202A–U+202E / U+2066–U+2069）：
//!    能让「允许」在屏幕上看起来像「拒绝」。通知文本没有任何正当用途——
//!    **拒绝**而不是清洗，让调用方知道来的是敌意输入（400 固定错误体）。
//! 3. **无边界膨胀**：按**字符**数截断（中文按字算，与 `selfreport::clamp_text` 同口径）。
//!
//! 只剥不转义：HTML 转义是显示层的职责（UI 已有 escapeHtml），这里做的是
//! 「文本就是它声称的那些字符」，两层各管一件事。

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextGuardError {
    /// 双向覆写控制符。固定码，不携带原文。
    BidirectionalOverride,
}

/// `/notify`·`/event` 的 agent 字段（它同时是 agent_id 与显示名）。
pub const AGENT_LIMIT: usize = 120;
/// `/notify`·`/event` 的 message 字段。
pub const MESSAGE_LIMIT: usize = 2000;
/// `/notify`·`/event` 的 detail 字段。
pub const DETAIL_LIMIT: usize = 4000;

/// 清洗一段不可信文本：双向覆写**拒绝**，其余控制字符剥除，按字符截断。
pub fn sanitize_text(raw: &str, max_chars: usize) -> Result<String, TextGuardError> {
    if raw.chars().any(is_bidirectional_override) {
        return Err(TextGuardError::BidirectionalOverride);
    }
    Ok(raw.chars().filter(|c| !is_stripped(*c)).take(max_chars).collect())
}

fn is_bidirectional_override(c: char) -> bool {
    matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

fn is_stripped(c: char) -> bool {
    if c.is_ascii() {
        // C0：ESC（终端序列的头）、\r、\0……只留换行与制表。
        return c.is_control() && !matches!(c, '\n' | '\t');
    }
    // C1（U+0080–U+009F，含单字符 CSI 0x9B）与不可见的方向/零宽标记。
    // U+FEFF 在正文里没有任何可读用途，只会在比对与复制时制造「看起来一样」。
    matches!(
        c,
        '\u{0080}'..='\u{009F}'
            | '\u{200E}'
            | '\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2066}'..='\u{2069}'
            | '\u{FEFF}'
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_characters_are_stripped_but_layout_is_kept() {
        let out = sanitize_text("a\u{1b}[31mred\u{0}b\rc\nd\te\u{9b}1f", 100).unwrap();
        assert_eq!(out, "a[31mredbc\nd\te1f");
        // C1 与零宽/方向标记不可见：剥掉之后文本可见内容不变。
        assert_eq!(
            sanitize_text("可\u{85}见\u{200b}\u{feff}文本", 100).unwrap(),
            "可见\u{200b}文本",
            "ZWSP（U+200B）不剥：它有排版用途且不伪装方向；C1 与 BOM 剥"
        );
    }

    #[test]
    fn bidirectional_overrides_are_rejected_not_cleaned() {
        for hostile in [
            "\u{202E}dlrow olleh",   // RLO：经典反转伪装
            "允许\u{2066}x\u{2069}", // isolate 包夹
            "\u{202A}hidden",
        ] {
            assert_eq!(
                sanitize_text(hostile, 100),
                Err(TextGuardError::BidirectionalOverride),
                "{hostile:?} 必须整段拒绝，不能清洗后放行"
            );
        }
    }

    #[test]
    fn sanitize_truncation_counts_characters_not_bytes() {
        let text = "字".repeat(50);
        let out = sanitize_text(&text, 20).unwrap();
        assert_eq!(out.chars().count(), 20);
        assert_eq!(out, "字".repeat(20));
    }

    #[test]
    fn empty_and_ordinary_text_pass_through() {
        assert_eq!(sanitize_text("", 10).unwrap(), "");
        assert_eq!(
            sanitize_text("普通中文 text 123\n\t", 10).unwrap(),
            "普通中文 text "
        );
    }
}
