//! 产物内容与附件的**细粒度预览与下载限额判据**（TASK-04 第二片）。
//!
//! 需求原文：过程消息、结构化提问、反馈修订、附件、预览/下载/文件浏览。
//! 场景 token：**超限下载/预览隔离**。
//!
//! 背景：现有 `task_artifacts.rs` 只有一个粗暴的全局硬截断 `MAX_BODY=512KB`，
//! 一旦超过就返回 `available: false`（用户只看到「超过显示范围」一行字，
//! 连正文前几行都看不到）。而且预览（界面 DOM 渲染）与下载（保存为本地文件）
//! 共用同一份限额——把界面承受力和文件保存混为一谈。
//!
//! 本模块只做**限额判据**（纯函数，不碰文件系统、不发请求）：
//!
//! 1. **预览 vs 下载隔离**：
//!    - 预览（`Preview`）：渲染进界面，限额严格（默认软上限 64KB、硬上限 256KB），
//!      防止大文本拖卡前端 WebView；
//!    - 下载（`Download`）：保存为本地文件，限额宽松（默认硬上限 10MB）。
//! 2. **软上限截断并说明**：在软硬上限之间时，**截取前 N 字节并如实说明**
//!    （`Truncated { kept_bytes, total_bytes }`），而不是整篇不可读——
//!    用户至少能看开头，并知道被截断了。
//! 3. **硬上限拒绝**：超过硬上限彻底不可读（`ExceededHardLimit`），
//!    如实引导「内容过大，请在来源工具查看」。
//! 4. **私密内容零容忍**：涉及私密输入/凭据时，无论大小一律拒（`RedactedSecret`），
//!    与 SEC-01 的凭据不外泄红线同口径。

/// 使用意图（需求：预览/下载隔离）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Intent {
    /// 界面内预览（直接渲染进 DOM）。
    Preview,
    /// 导出 / 保存为本地文件。
    Download,
}

/// 限额配置。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// 软上限：超过时截断并说明；低于时完整交付。
    pub soft_limit_bytes: usize,
    /// 硬上限：超过时彻底不可读。
    pub hard_limit_bytes: usize,
}

impl Limits {
    pub fn for_intent(intent: Intent) -> Self {
        match intent {
            // 预览：64KB 软上限（截断并说明），256KB 硬上限（彻底不读防卡死）
            Intent::Preview => Limits {
                soft_limit_bytes: 64 * 1024,
                hard_limit_bytes: 256 * 1024,
            },
            // 下载：无软上限（文件保存不需要截断），10MB 硬上限（防 OOM）
            Intent::Download => Limits {
                soft_limit_bytes: 10 * 1024 * 1024,
                hard_limit_bytes: 10 * 1024 * 1024,
            },
        }
    }
}

/// 限额裁决结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LimitVerdict<'a> {
    /// 完整交付。
    Full(&'a str),
    /// 软超限：截取前 `kept_bytes`，保留超限事实（原大小 `total_bytes`）。
    Truncated {
        snippet: &'a str,
        kept_bytes: usize,
        total_bytes: usize,
    },
    /// 硬超限：彻底不可交付。
    ExceededHardLimit { total_bytes: usize, hard_limit: usize },
    /// 包含凭据/私密输入：不可交付（无论大小）。
    RedactedSecret,
    /// 空内容。
    Empty,
}

impl<'a> LimitVerdict<'a> {
    pub fn is_available(&self) -> bool {
        matches!(self, Self::Full(_) | Self::Truncated { .. })
    }

    /// 给界面的提示文案。
    pub fn notice_text(&self) -> &'static str {
        match self {
            Self::Full(_) => "完整内容",
            Self::Truncated { .. } => "内容已截取前段预览，完整文件请打开来源查看",
            Self::ExceededHardLimit { .. } => "内容超过最大允许范围，请在来源工具中查看",
            Self::RedactedSecret => "内容涉及私密输入或凭据，请在来源工具中查看",
            Self::Empty => "此项没有可展示的内容",
        }
    }
}

/// 审查一段文本并给出限额裁决。
///
/// `is_secret`：调用方已识别的私密标记（或通过 `known_private` 检查）。
pub fn evaluate_limits<'a>(text: &'a str, intent: Intent, is_secret: bool) -> LimitVerdict<'a> {
    if is_secret {
        return LimitVerdict::RedactedSecret;
    }
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return LimitVerdict::Empty;
    }
    let total = text.len();
    let limits = Limits::for_intent(intent);

    if total > limits.hard_limit_bytes {
        return LimitVerdict::ExceededHardLimit {
            total_bytes: total,
            hard_limit: limits.hard_limit_bytes,
        };
    }

    if total > limits.soft_limit_bytes {
        // 在字符边界处安全截断，不切半个 UTF-8 字符
        let boundary = floor_char_boundary(text, limits.soft_limit_bytes);
        return LimitVerdict::Truncated {
            snippet: &text[..boundary],
            kept_bytes: boundary,
            total_bytes: total,
        };
    }

    LimitVerdict::Full(text)
}

/// 找到不大于 `max_bytes` 的最大有效 UTF-8 字符边界。
fn floor_char_boundary(text: &str, max_bytes: usize) -> usize {
    if max_bytes >= text.len() {
        return text.len();
    }
    let mut i = max_bytes;
    while !text.is_char_boundary(i) {
        i -= 1;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_content_is_delivered_full() {
        let text = "普通短文本";
        let v = evaluate_limits(text, Intent::Preview, false);
        assert_eq!(v, LimitVerdict::Full(text));
        assert!(v.is_available());
        assert_eq!(v.notice_text(), "完整内容");
    }

    #[test]
    fn preview_soft_limit_truncates_safely_and_explains() {
        let soft = Limits::for_intent(Intent::Preview).soft_limit_bytes;
        // 构造一个在软硬上限之间的长文本（中文字符避免切出乱码）
        let big = "测试文本行\n".repeat((soft / 16) + 10);
        assert!(big.len() > soft);
        assert!(big.len() < Limits::for_intent(Intent::Preview).hard_limit_bytes);

        let v = evaluate_limits(&big, Intent::Preview, false);
        match v {
            LimitVerdict::Truncated {
                snippet,
                kept_bytes,
                total_bytes,
            } => {
                assert!(kept_bytes <= soft);
                assert_eq!(total_bytes, big.len());
                assert_eq!(snippet.len(), kept_bytes);
                assert!(snippet.is_char_boundary(kept_bytes));
            }
            other => panic!("应当被安全截断，实际 {other:?}"),
        }
        assert!(v.is_available(), "截断内容依然对用户可见（不是整篇变空）");
        assert!(v.notice_text().contains("已截取"));
    }

    #[test]
    fn preview_and_download_have_isolated_limits() {
        // 500KB 的文本：超过预览硬上限 (256KB)，但在下载上限 (10MB) 内
        let medium = "x".repeat(500 * 1024);
        let preview_verdict = evaluate_limits(&medium, Intent::Preview, false);
        assert!(!preview_verdict.is_available(), "500KB 超过预览硬上限必须拒绝");
        assert!(matches!(
            preview_verdict,
            LimitVerdict::ExceededHardLimit { .. }
        ));

        // 下载意图下完整交付
        let download_verdict = evaluate_limits(&medium, Intent::Download, false);
        assert!(download_verdict.is_available(), "500KB 在下载限额内应完整交付");
        assert_eq!(download_verdict, LimitVerdict::Full(&medium));
    }

    #[test]
    fn secrets_are_redacted_regardless_of_size() {
        // 哪怕只有 5 个字符，标记为私密一律拒绝
        let v = evaluate_limits("token", Intent::Download, true);
        assert_eq!(v, LimitVerdict::RedactedSecret);
        assert!(!v.is_available());
    }

    #[test]
    fn empty_content_is_distinguished() {
        assert_eq!(evaluate_limits("", Intent::Preview, false), LimitVerdict::Empty);
        assert_eq!(evaluate_limits("   \n\t", Intent::Preview, false), LimitVerdict::Empty);
    }
}
