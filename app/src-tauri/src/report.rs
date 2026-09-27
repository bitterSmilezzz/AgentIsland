//! Token 消费报表导出（Swift `TokenReportExporter`）。
//!
//! Markdown（贴进 Issue/群聊）与 CSV（进表格）。口径上有三处必须照搬，每处都有具体后果：
//!
//! ① **每行的费用格永远写钱数、不写 `—`**：右边那一列（已连接/未发现）已经在说
//!    「这个源到底查没查到」。费用格再兼一个意思，同一份导出的 Markdown 与 CSV 就会
//!    对同一个零各说各话（CSV 那侧本来就是 `$0.00`）。
//! ② **累计历史那一行用的是 `cost()` 而不是 `costText()`**：所以累计成本为零时那里是
//!    **空**，而不是 `$0.00`。这是参考实现的行为，照搬——报告里的「零怎么写」只能有一条线，
//!    而这条线在两处出口本来就不一样（表里要占位、汇总行省略）。
//! ③ **CSV 以 UTF-8 BOM 开头**：否则 Excel 打开是乱码。
//!
//! 与 Swift 的一处结构差异（有意）：Rust 的 [`Timeline`] **不带 `points`**。
//! 那一份是给折线图用的，而 Rust 的图表数据来自 `get_report` 的 `hourly30d`
//! （单一来源）；把同一份桶序列塞两个类型里，迟早会有一处先过期。

use crate::models::{Export, TokenUsage};

/// 统计周期。档位与文案与 Swift `TokenTimeRange` 同值
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TokenTimeRange {
    Day,
    Week,
    Month,
}

impl TokenTimeRange {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "day" => Some(TokenTimeRange::Day),
            "week" => Some(TokenTimeRange::Week),
            "month" => Some(TokenTimeRange::Month),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            TokenTimeRange::Day => "day",
            TokenTimeRange::Week => "week",
            TokenTimeRange::Month => "month",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            TokenTimeRange::Day => "最近 24 小时 (24h)",
            TokenTimeRange::Week => "最近 7 天 (7d)",
            TokenTimeRange::Month => "最近 30 天 (30d)",
        }
    }

    pub fn duration_ms(self) -> i64 {
        match self {
            TokenTimeRange::Day => 24 * 3_600_000,
            TokenTimeRange::Week => 7 * 24 * 3_600_000,
            TokenTimeRange::Month => 30 * 24 * 3_600_000,
        }
    }

    /// 桶数（Swift 同值：日 24 个每小时、周 28 个每 6 小时、月 30 个每天）
    pub fn bucket_count(self) -> usize {
        match self {
            TokenTimeRange::Day => 24,
            TokenTimeRange::Week => 28,
            TokenTimeRange::Month => 30,
        }
    }
}

/// 一条数据源（一个 Agent）在**这个周期内**的用量。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceUsage {
    pub agent_id: String,
    pub tokens: i64,
    pub cost: f64,
    /// 已发现该工具的本地统计源；false = 这台机器上没有可读的明细，
    /// **不是「用量为 0」**（报告里那一格写「未发现」）
    pub is_available: bool,
}

/// 一份周期报表：周期内的合计 + 逐来源拆解
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Timeline {
    pub range: TokenTimeRange,
    pub tokens: i64,
    pub cost: f64,
    pub sources: Vec<SourceUsage>,
}

/// 扫一遍所有带 token 根的档案，拼出某个周期的报表。
///
/// **成本随区间缩放**：`range_totals` 复用 24h 那一档的解析（同一份明细按不同 cutoff 聚合），
/// 所以三档之间不会因为两处各写一份口径而对不上。
pub fn build_timeline(range: TokenTimeRange, now_ms: i64) -> Timeline {
    let mut monitor = crate::tokens::TokenUsageMonitor::new();
    let mut sources = Vec::new();
    let mut tokens = 0i64;
    let mut cost = 0f64;
    for profile in crate::registry::builtin() {
        if profile.token_roots.is_empty() {
            continue;
        }
        let (range_tokens, range_cost) =
            monitor.range_totals(&profile, range.duration_ms(), now_ms);
        let available = profile
            .token_roots
            .iter()
            .any(|root| std::path::Path::new(root).is_dir());
        // 既没读到用量、也没找到本地统计源的档案不进报告：一行全是 0 的「未发现」
        // 会把「这台机器没装这个工具」混进用量分布里
        if range_tokens == 0 && !available {
            continue;
        }
        tokens += range_tokens;
        cost += range_cost;
        sources.push(SourceUsage {
            agent_id: profile.id.clone(),
            tokens: range_tokens,
            cost: range_cost,
            is_available: available,
        });
    }
    Timeline {
        range,
        tokens,
        cost,
        sources,
    }
}

fn timestamp_text(now_ms: i64) -> String {
    crate::audit::timestamp_text(now_ms)
}

/// 按用量降序。**稳定排序**：用量相同时保持输入顺序（Swift 的 `sorted(by:)` 不保证稳定，
/// 于是同一份数据两次导出可能换序；这里更好，而且更好是免费的）
fn sorted_sources(sources: &[SourceUsage]) -> Vec<&SourceUsage> {
    let mut sorted: Vec<&SourceUsage> = sources.iter().collect();
    sorted.sort_by(|a, b| b.tokens.cmp(&a.tokens));
    sorted
}

/// 生成 Markdown 报表。`grand_total` 是面板汇总栏那份跨源累计
pub fn generate_markdown(timeline: &Timeline, grand_total: &TokenUsage, now_ms: i64) -> String {
    let total_cost = crate::cost::resolve_cost(timeline.cost, None, Some(timeline.tokens));
    // 用 `text` 而不是按数值重排：估出来的成本带 `~`，重排会把那个标记弄丢
    let cost_display = if total_cost.1.is_empty() {
        "$0.00".to_string()
    } else {
        total_cost.1.clone()
    };
    let estimated = if total_cost.2 { " (参考估算)" } else { "" };

    let mut md = String::new();
    md.push_str("# AgentIsland Token 消费与用量分析报表\n");
    md.push_str(&format!("- 导出时间：{}\n", timestamp_text(now_ms)));
    md.push_str(&format!("- 统计周期：{}\n", timeline.range.label()));
    md.push_str(&format!(
        "- 周期内 Token 消耗：**{}** ({} tokens)\n",
        crate::tokens::compact(timeline.tokens),
        timeline.tokens
    ));
    md.push_str(&format!(
        "- 周期内费用估算：**{cost_display}**{estimated}\n"
    ));
    // 注意：这里用的是 `cost()` 而不是 `costText()` ⇒ 累计成本为零时是**空**，不是 $0.00
    md.push_str(&format!(
        "- 累计历史总用量：{} ({})\n",
        crate::tokens::compact(grand_total.tokens_total),
        crate::cost::format_cost(grand_total.cost_total)
    ));
    md.push_str("\n\n---\n\n");
    md.push_str("### 数据源与智能体分布\n\n");
    md.push_str("| 智能体 (Agent) | Token 用量 | 预估/实际费用 | 状态 |\n");
    md.push_str("| :--- | :--- | :--- | :--- |\n\n");
    for source in sorted_sources(&timeline.sources) {
        // 按来源 id 解析费率（与 Swift 传 `modelId: s.agentId` 一致）
        let resolved = crate::cost::resolve_cost(source.cost, Some(&source.agent_id), Some(source.tokens));
        let fee = if resolved.1.is_empty() {
            "$0.00".to_string()
        } else {
            resolved.1.clone()
        };
        let status = if source.is_available { "已连接" } else { "未发现" };
        md.push_str(&format!(
            "| `{}` | {} | {} | {} |\n",
            source.agent_id,
            crate::tokens::compact(source.tokens),
            fee,
            status
        ));
    }
    md.push_str("\n> 生成自 AgentIsland (macOS AI 智能体监控灵动岛)\n");
    md
}

/// 生成 CSV（UTF-8 **带 BOM**，否则 Excel 打开是乱码）
pub fn generate_csv(timeline: &Timeline, now_ms: i64) -> String {
    let mut csv = String::from("\u{FEFF}");
    csv.push_str("报表时间,周期,Agent,Tokens,费用,状态\n");
    for source in sorted_sources(&timeline.sources) {
        let resolved = crate::cost::resolve_cost(source.cost, Some(&source.agent_id), Some(source.tokens));
        let fee = if resolved.1.is_empty() {
            "$0.00".to_string()
        } else {
            resolved.1.clone()
        };
        let status = if source.is_available { "已连接" } else { "未发现" };
        csv.push_str(&format!(
            "\"{}\",\"{}\",\"{}\",{},\"{}\",\"{}\"\n",
            timestamp_text(now_ms),
            timeline.range.label(),
            source.agent_id,
            source.tokens,
            fee,
            status
        ));
    }
    csv
}

pub fn markdown_export(timeline: &Timeline, grand_total: &TokenUsage, now_ms: i64) -> Export {
    Export {
        filename: crate::audit::default_filename("md", now_ms).replace(
            "AgentIsland_Audit_",
            "AgentIsland_TokenReport_",
        ),
        content: generate_markdown(timeline, grand_total, now_ms),
    }
}

pub fn csv_export(timeline: &Timeline, now_ms: i64) -> Export {
    Export {
        filename: crate::audit::default_filename("csv", now_ms).replace(
            "AgentIsland_Audit_",
            "AgentIsland_TokenReport_",
        ),
        content: generate_csv(timeline, now_ms),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timeline(range: TokenTimeRange, sources: Vec<SourceUsage>) -> Timeline {
        let tokens = sources.iter().map(|s| s.tokens).sum();
        let cost = sources.iter().map(|s| s.cost).sum();
        Timeline {
            range,
            tokens,
            cost,
            sources,
        }
    }

    fn source(id: &str, tokens: i64, cost: f64, available: bool) -> SourceUsage {
        SourceUsage {
            agent_id: id.into(),
            tokens,
            cost,
            is_available: available,
        }
    }

    fn grand(tokens_total: i64, cost_total: f64) -> TokenUsage {
        TokenUsage {
            tokens24h: 0,
            tokens_total,
            cost24h: 0.0,
            cost_total,
            cost_estimated: false,
        }
    }

    #[test]
    fn the_range_labels_and_buckets_match_the_reference() {
        assert_eq!(TokenTimeRange::Day.label(), "最近 24 小时 (24h)");
        assert_eq!(TokenTimeRange::Week.label(), "最近 7 天 (7d)");
        assert_eq!(TokenTimeRange::Month.label(), "最近 30 天 (30d)");
        assert_eq!(TokenTimeRange::Day.duration_ms(), 86_400_000);
        assert_eq!(TokenTimeRange::Week.duration_ms(), 604_800_000);
        assert_eq!(TokenTimeRange::Month.duration_ms(), 2_592_000_000);
        assert_eq!(TokenTimeRange::Day.bucket_count(), 24);
        assert_eq!(TokenTimeRange::Week.bucket_count(), 28);
        assert_eq!(TokenTimeRange::Month.bucket_count(), 30);
        assert_eq!(TokenTimeRange::parse("week"), Some(TokenTimeRange::Week));
        assert_eq!(TokenTimeRange::parse("nope"), None);
        assert_eq!(TokenTimeRange::parse("day").unwrap().as_str(), "day");
    }

    #[test]
    fn the_markdown_header_carries_the_period_and_both_totals() {
        let report = timeline(
            TokenTimeRange::Month,
            vec![source("claude", 1_200_000, 0.42, true)],
        );
        let md = generate_markdown(&report, &grand(280_000_000, 19.37), 1_700_000_000_000);
        assert!(md.starts_with("# AgentIsland Token 消费与用量分析报表\n"), "{md}");
        assert!(md.contains("- 统计周期：最近 30 天 (30d)\n"), "{md}");
        assert!(md.contains("- 周期内 Token 消耗：**1.20M** (1200000 tokens)\n"), "{md}");
        assert!(md.contains("- 周期内费用估算：**$0.42**\n"), "{md}");
        assert!(md.contains("- 累计历史总用量：280.00M ($19.37)\n"), "{md}");
        assert!(md.contains("| `claude` | 1.20M | $0.42 | 已连接 |\n"), "{md}");
        assert!(
            md.contains("> 生成自 AgentIsland (macOS AI 智能体监控灵动岛)\n"),
            "{md}"
        );
    }

    #[test]
    fn the_grand_total_cost_line_is_empty_at_zero_because_it_uses_cost_not_cost_text() {
        // 参考实现里这一行用 `cost()`：零 ⇒ 空串。表里那一格是 `$0.00`。
        // 「零怎么写」只能有一条线，而这条线在两处出口本来就不一样——照搬，别自作主张统一
        let report = timeline(TokenTimeRange::Day, vec![]);
        let md = generate_markdown(&report, &grand(0, 0.0), 1_700_000_000_000);
        assert!(md.contains("- 累计历史总用量：0 ()\n"), "{md}");
        assert!(!md.contains("$0.00 (参考估算)"), "{md}");
    }

    #[test]
    fn the_fee_column_always_shows_money_never_a_dash() {
        // 未发现的来源：费用格仍是 `$0.00`，由「状态」列去说「未发现」
        let report = timeline(
            TokenTimeRange::Day,
            vec![source("gemini", 0, 0.0, false), source("claude", 1000, 0.002, true)],
        );
        let md = generate_markdown(&report, &grand(0, 0.0), 1_700_000_000_000);
        assert!(md.contains("| `gemini` | 0 | $0.00 | 未发现 |\n"), "{md}");
        assert!(!md.contains("| — |"), "费用格不许出现 —：{md}");
        // 小于一分钱写 `<$0.01`（`format_cost` 的口径），而且**不带 `~`**：
        // 这是记录值，不是估的
        assert!(md.contains("| `claude` | 1000 | <$0.01 | 已连接 |\n"), "{md}");
    }

    /// 估价标记：**行内的 `~`** 才是用户真正看得见的那个信号。
    ///
    /// 顺带记一个发现：汇总行那句「(参考估算)」在两边**都是死分支**——
    /// 那一行按参考实现 `resolveCost(actual:tokens:)` 解析（不带 modelId），
    /// 而 `isEstimated` 只在「没有记录值 + 有 modelId + 估得出」时才为真。
    /// 也就是说 `actual > 0` 时它恒为 false、`actual == 0` 时又没有 modelId。
    /// Rust 照搬这个结构（对齐优先），并用这条用例把「实际行为」钉住。
    #[test]
    fn the_estimate_marker_is_the_tilde_in_the_row_not_the_dead_marker_in_the_header() {
        let estimated = timeline(
            TokenTimeRange::Day,
            vec![source("claude-3-7-sonnet", 4_000, 0.0, true)],
        );
        let md = generate_markdown(&estimated, &grand(0, 0.0), 1_700_000_000_000);
        assert!(
            md.contains("| `claude-3-7-sonnet` | 4000 | ~$0.02 | 已连接 |"),
            "估价的行内费用要带 ~ 标记：{md}"
        );
        assert!(
            !md.contains(" (参考估算)"),
            "汇总行不带 modelId ⇒ isEstimated 恒假（两边一致，见用例注释）：{md}"
        );

        // 来源 id 不在费率表里 ⇒ 不估、也不标（不许凭空编一个成本）
        let unpriced = timeline(TokenTimeRange::Day, vec![source("gemini", 1_000, 0.0, true)]);
        let md = generate_markdown(&unpriced, &grand(0, 0.0), 1_700_000_000_000);
        assert!(md.contains("| `gemini` | 1000 | $0.00 | 已连接 |"), "{md}");
        assert!(!md.contains('~'), "费率表里没有的 id 不许估价：{md}");

        // 有记录成本就照实写，不估
        let recorded = timeline(TokenTimeRange::Day, vec![source("claude", 1_000, 0.5, true)]);
        let md = generate_markdown(&recorded, &grand(0, 0.0), 1_700_000_000_000);
        assert!(md.contains("| `claude` | 1000 | $0.50 | 已连接 |"), "{md}");
        assert!(!md.contains('~'), "{md}");
    }

    #[test]
    fn sources_are_sorted_by_usage_descending_and_ties_keep_input_order() {
        let report = timeline(
            TokenTimeRange::Week,
            vec![
                source("small", 10, 0.0, true),
                source("big", 9_000, 0.0, true),
                source("tie-a", 500, 0.0, true),
                source("tie-b", 500, 0.0, true),
            ],
        );
        let md = generate_markdown(&report, &grand(0, 0.0), 1_700_000_000_000);
        let order: Vec<usize> = ["`big`", "`tie-a`", "`tie-b`", "`small`"]
            .iter()
            .map(|name| md.find(name).unwrap_or_else(|| panic!("缺 {name}：{md}")))
            .collect();
        assert!(order.windows(2).all(|w| w[0] < w[1]), "应按用量降序：{md}");
    }

    #[test]
    fn the_csv_has_a_bom_and_quotes_the_text_fields() {
        let report = timeline(
            TokenTimeRange::Day,
            vec![source("claude", 1234, 0.5, true), source("gemini", 0, 0.0, false)],
        );
        let csv = generate_csv(&report, 1_700_000_000_000);
        assert!(csv.starts_with('\u{FEFF}'), "Excel 需要 BOM，否则中文乱码");
        assert_eq!(
            csv.lines().next().unwrap().trim_start_matches('\u{FEFF}'),
            "报表时间,周期,Agent,Tokens,费用,状态"
        );
        // 按用量降序 ⇒ claude（1234）在前、gemini（0）在后
        let claude_row = csv.lines().nth(1).unwrap();
        let gemini_row = csv.lines().nth(2).unwrap();
        assert!(claude_row.contains("\"claude\",1234,\"$0.50\",\"已连接\""), "{claude_row}");
        assert!(gemini_row.contains("\"gemini\",0,\"$0.00\",\"未发现\""), "{gemini_row}");
        // 每行列数固定（Tokens 不加引号，其余加），脚本按逗号切才稳
        for line in csv.lines().skip(1) {
            assert_eq!(line.split(',').count(), 6, "{line}");
        }
    }

    #[test]
    fn an_export_carries_the_report_filename_shape_and_the_same_instant_as_its_body() {
        let report = timeline(TokenTimeRange::Day, vec![]);
        let export = markdown_export(&report, &grand(0, 0.0), 1_700_000_000_000);
        assert!(export.filename.starts_with("AgentIsland_TokenReport_"), "{export:?}");
        assert!(export.filename.ends_with(".md"), "{export:?}");
        let stamp = &export.filename["AgentIsland_TokenReport_".len()..export.filename.len() - 3];
        let compact: String = stamp.chars().filter(|c| c.is_ascii_digit()).collect();
        let body: String = timestamp_text(1_700_000_000_000)
            .chars()
            .filter(|c| c.is_ascii_digit())
            .collect();
        assert_eq!(compact, body, "文件名与正文时间戳必须同源：{export:?}");

        let csv = csv_export(&report, 1_700_000_000_000);
        assert!(csv.filename.ends_with(".csv"), "{csv:?}");
    }
}
