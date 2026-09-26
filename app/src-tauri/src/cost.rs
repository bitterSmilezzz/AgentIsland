//! Token 成本估价：与 Swift `TokenCostEstimator` 同表、同算法、同边界。
//!
//! 迁移自 `Sources/AgentIslandCore/TokenCostEstimator.swift`（ADR 0010 / M3，
//! 该模块是 ADR 点名的 12 个缺失模块之一）。迁移的**唯一理由**是 23 号对照表
//! 记着的那条违规：两边都有成本概念，费率表与算法却不同，同日同量会算出不同钱。
//!
//! 铁律（与 Swift 侧一致）：**估不出来就是 `None`，不许凭空给一个费率。**
//! 旧实现有一条 `else { (1.25, 10.0) }` 的兜底，等于给任何没见过的模型编一个价钱，
//! 而界面上分不出它是记录值还是编的。宁可没有成本，也不许印一个假的。
//!
//! 表与顺序都被 `swift_table_parity` 那条测试逐值盯着——Swift 侧改一个数，
//! 这里就红，而不是等两边算出两个答案。

/// 美元 / 百万 tokens
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rate {
    pub input_per_million: f64,
    pub output_per_million: f64,
}

impl Rate {
    /// 按典型 3:1 输入/输出比例混合加权后的单价（美元 / token）。
    pub fn blended_per_token(&self) -> f64 {
        let blended_million = (self.input_per_million * 3.0 + self.output_per_million) / 4.0;
        blended_million / 1_000_000.0
    }
}

const fn r(input: f64, output: f64) -> Rate {
    Rate { input_per_million: input, output_per_million: output }
}

/// 主流模型官方基准费率表。**顺序即语义**：`rate_for` 取第一条命中，
/// 所以「更具体的写在更笼统的前面」（`gpt-4o-mini` 必须在 `gpt-4o` 之前，
/// `o1-mini` 必须在 `o1` 之前）。重排这张表会静默改变价格。
pub const KNOWN_RATES: &[(&str, Rate)] = &[
    // Claude 系列
    ("claude-3-7-sonnet", r(3.0, 15.0)),
    ("claude-3-5-sonnet", r(3.0, 15.0)),
    ("claude-3-5-haiku", r(0.8, 4.0)),
    ("claude-3-haiku", r(0.25, 1.25)),
    ("claude-3-opus", r(15.0, 75.0)),
    ("claude-opus", r(15.0, 75.0)),
    ("claude-sonnet", r(3.0, 15.0)),
    ("claude-haiku", r(0.8, 4.0)),
    // OpenAI 系列
    ("gpt-4o-mini", r(0.15, 0.60)),
    ("gpt-4o", r(2.50, 10.00)),
    ("gpt-4-turbo", r(10.0, 30.0)),
    ("gpt-4", r(30.0, 60.0)),
    ("gpt-3.5-turbo", r(0.5, 1.5)),
    ("o1-mini", r(1.10, 4.40)),
    ("o1-preview", r(15.0, 60.0)),
    ("o1", r(15.0, 60.0)),
    ("o3-mini", r(1.10, 4.40)),
    ("o3", r(2.50, 10.0)),
    // DeepSeek 系列
    ("deepseek-reasoner", r(0.55, 2.19)),
    ("deepseek-r1", r(0.55, 2.19)),
    ("deepseek-chat", r(0.14, 0.28)),
    ("deepseek-v3", r(0.14, 0.28)),
    ("deepseek", r(0.14, 0.28)),
    // Google Gemini 系列
    ("gemini-2.0-flash", r(0.10, 0.40)),
    ("gemini-2.5-flash", r(0.10, 0.40)),
    ("gemini-1.5-flash", r(0.075, 0.30)),
    ("gemini-2.0-pro", r(1.25, 5.00)),
    ("gemini-2.5-pro", r(1.25, 5.00)),
    ("gemini-1.5-pro", r(1.25, 5.00)),
    ("gemini-flash", r(0.10, 0.40)),
    ("gemini-pro", r(1.25, 5.00)),
    // Qwen 系列
    ("qwen-max", r(2.4, 9.6)),
    ("qwen-plus", r(0.4, 1.2)),
    ("qwen-turbo", r(0.1, 0.2)),
    // Mistral / Codestral 系列
    ("codestral", r(0.3, 0.9)),
    ("mistral-large", r(2.0, 6.0)),
    ("mistral-small", r(0.2, 0.6)),
];

/// 按模型 ID 找费率。语义与 Swift 一致：trim + 小写，`==` 或 `contains` 匹配，
/// **取第一条命中**（因此表序有意义），没有命中就是 `None`。
pub fn rate_for(model_id: &str) -> Option<Rate> {
    let lower = model_id.trim().to_lowercase();
    if lower.is_empty() {
        return None;
    }
    KNOWN_RATES
        .iter()
        .find(|(pattern, _)| lower == *pattern || lower.contains(pattern))
        .map(|(_, rate)| *rate)
}

/// 按模型与总 token 数估价。`tokens <= 0`、没匹配到费率、或结果非有限值一律 `None`。
pub fn estimate_cost(model_id: &str, tokens: i64) -> Option<f64> {
    if tokens <= 0 {
        return None;
    }
    let rate = rate_for(model_id)?;
    let cost = tokens as f64 * rate.blended_per_token();
    cost.is_finite().then_some(cost)
}

/// 记录成本的排版（`$1.23` / `<$0.01` / 0 或负 → 空串）。与 `TokenUsageMonitor.cost` 同口径。
pub fn format_cost(cost: f64) -> String {
    if cost <= 0.0 {
        return String::new();
    }
    if cost < 0.01 {
        return "<$0.01".to_string();
    }
    format!("${cost:.2}")
}

/// 估算成本的排版，带 `~` 标记：`~$0.42` / `~<$0.01` / 0 或负 → 空串。
/// `~` 是「这个数是估的」唯一的痕迹，调用方**不许**按数字重排把它弄丢。
pub fn format_estimate(cost: f64) -> String {
    if cost <= 0.0 {
        return String::new();
    }
    if cost < 0.01 {
        return "~<$0.01".to_string();
    }
    format!("~${cost:.2}")
}

/// 与 Swift `TokenCostEstimator.resolveCost` 同语义：
/// 有记录成本就用记录值（无标记）；否则能估就估（带 `~`）；再否则 `(0, "", false)`。
pub fn resolve_cost(
    actual: f64,
    model_id: Option<&str>,
    tokens: Option<i64>,
) -> (f64, String, bool) {
    if actual > 0.0 {
        return (actual, format_cost(actual), false);
    }
    if let (Some(model), Some(tokens)) = (model_id, tokens) {
        if let Some(est) = estimate_cost(model, tokens) {
            return (est, format_estimate(est), true);
        }
    }
    (0.0, String::new(), false)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 跨语言漂移哨兵：直接读 Swift 源表，逐条比对 pattern 与两个价。
    /// 不放进来的话，两边改一个数就只有「用户发现算出来的钱不一样」能发现。
    fn swift_table() -> Vec<(String, f64, f64)> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../Sources/AgentIslandCore/TokenCostEstimator.swift");
        let src = std::fs::read_to_string(&path).unwrap_or_else(|e| {
            panic!("读不到 Swift 费率表 {}：{e}——哨兵读不到源就等于没有哨兵", path.display())
        });
        let mut rows = Vec::new();
        for raw in src.lines() {
            let line = raw.trim();
            let Some(rest) = line.strip_prefix("(\"") else { continue };
            let Some((pattern, tail)) = rest.split_once("\",") else { continue };
            let Some((_, after_input)) = tail.split_once("input:") else { continue };
            let Some((input_str, after_in)) = after_input.split_once(',') else { continue };
            let Some((_, after_output)) = after_in.split_once("output:") else { continue };
            let output_str = after_output
                .trim_end_matches(|c: char| c == ')' || c == ',')
                .trim();
            let (Ok(input), Ok(output)) =
                (input_str.trim().parse::<f64>(), output_str.parse::<f64>())
            else {
                continue;
            };
            rows.push((pattern.to_string(), input, output));
        }
        rows
    }

    #[test]
    fn swift_table_parity_is_value_by_value_and_order_by_order() {
        let swift = swift_table();
        assert!(
            !swift.is_empty(),
            "从 Swift 源里一条费率都没解析出来——解析口径换了，哨兵自己失效了"
        );
        assert_eq!(
            swift.len(),
            KNOWN_RATES.len(),
            "费率条数不一致：Swift {} 条 vs Rust {} 条",
            swift.len(),
            KNOWN_RATES.len()
        );
        for (idx, ((sp, si, so), (rp, rate))) in
            swift.iter().zip(KNOWN_RATES.iter()).enumerate()
        {
            assert_eq!(sp, rp, "第 {} 条 pattern 不一致（顺序也是契约）", idx + 1);
            assert_eq!(
                (*si, *so),
                (rate.input_per_million, rate.output_per_million),
                "pattern {sp} 的费率不一致：Swift ({si}, {so}) vs Rust ({}, {})",
                rate.input_per_million,
                rate.output_per_million
            );
        }
    }

    #[test]
    fn blended_rate_is_three_to_one() {
        // claude-3-7-sonnet: (3*3 + 15) / 4 = 6.0 美元 / 百万 → 1M tokens = $6.00
        let rate = rate_for("claude-3-7-sonnet").expect("应命中");
        assert!((rate.blended_per_token() * 1_000_000.0 - 6.0).abs() < 1e-9);
        assert_eq!(estimate_cost("claude-3-7-sonnet", 1_000_000), Some(6.0));
        // 与「按分量分别计价」不是一回事：同一份 3:1 的总量下，混合价就是上式
        assert_eq!(estimate_cost("claude-3-7-sonnet", 500_000), Some(3.0));
    }

    #[test]
    fn specific_patterns_win_over_generic_ones() {
        // 表序即语义：更具体的必须排在更笼统的前面，否则价格会静默改变。
        // 只挑**价不一样**的配对来断言——像 `claude-3-5-haiku` 与 `claude-haiku`
        // 在 Swift 表里刻意同价 (0.8,4.0)，`claude-3-opus` 与 `claude-opus` 同为 (15,75)，
        // `o1-preview` 与 `o1` 同为 (15,60)：那几对的顺序从价格上观察不到，
        // 靠下面的 parity 哨兵守「顺序与 Swift 一致」，不靠这里的取值断言。
        assert_eq!(rate_for("gpt-4o-mini"), rate_for("gpt-4o-mini-2024-08-06"));
        assert_ne!(rate_for("gpt-4o-mini"), rate_for("gpt-4o")); // (0.15,0.60) vs (2.5,10)
        // "gpt-4o" 本身含 "gpt-4"：靠 gpt-4o 排在 gpt-4 前面才没掉到 (30,60)
        assert_ne!(rate_for("gpt-4o"), rate_for("gpt-4"));
        assert_ne!(rate_for("gpt-4-turbo"), rate_for("gpt-4"));
        assert_eq!(rate_for("o1-mini"), rate_for("o1-mini-2024-09-12"));
        assert_ne!(rate_for("o1-mini"), rate_for("o1"));
        assert_eq!(rate_for("o3-mini"), rate_for("o3-mini-2025-01-31"));
        assert_ne!(rate_for("o3-mini"), rate_for("o3"));
        assert_ne!(rate_for("deepseek-reasoner"), rate_for("deepseek"));
        // 版本化 id 必须落到版本化条目上（同价，但语义上不该落成通用条目）
        assert_eq!(rate_for("claude-3-5-haiku-20241022"), rate_for("claude-3-5-haiku"));
        assert_eq!(rate_for("claude-3-opus-20240229"), rate_for("claude-3-opus"));
    }

    #[test]
    fn known_rates_are_the_ones_the_matrix_called_out() {
        // 23 号对照表点名的三处：haiku 旧实现给 (1.0,5.0) 而 Swift 是 (0.8,4.0)
        assert_eq!(rate_for("claude-3-5-haiku"), Some(Rate { input_per_million: 0.8, output_per_million: 4.0 }));
        // Swift 对 claude-3-7-sonnet 是 (3,15)
        assert_eq!(rate_for("claude-3-7-sonnet"), Some(Rate { input_per_million: 3.0, output_per_million: 15.0 }));
        // glm 在 Swift 表里没有对应：旧 Rust 实现编了 (0.55,2.0)，现在必须估不出来
        assert_eq!(rate_for("glm-4"), None);
    }

    #[test]
    fn unknown_models_are_not_given_an_invented_rate() {
        assert_eq!(rate_for(""), None);
        assert_eq!(rate_for("   "), None);
        assert_eq!(rate_for("some-local-finetune"), None);
        assert_eq!(estimate_cost("some-local-finetune", 1_000_000), None);
        assert_eq!(estimate_cost("claude-3-7-sonnet", 0), None);
        assert_eq!(estimate_cost("claude-3-7-sonnet", -5), None);
    }

    #[test]
    fn matching_is_case_insensitive_and_trims() {
        assert_eq!(rate_for("  CLAUDE-3-7-SONNET  "), rate_for("claude-3-7-sonnet"));
        assert_eq!(
            rate_for("anthropic/claude-3-5-sonnet-20241022"),
            rate_for("claude-3-5-sonnet")
        );
    }

    #[test]
    fn formatting_keeps_the_estimate_marker() {
        assert_eq!(format_cost(1.234), "$1.23");
        assert_eq!(format_cost(0.001), "<$0.01");
        assert_eq!(format_cost(0.0), "");
        assert_eq!(format_cost(-1.0), "");
        assert_eq!(format_estimate(0.42), "~$0.42");
        assert_eq!(format_estimate(0.001), "~<$0.01");
        assert_eq!(format_estimate(0.0), "");
    }

    #[test]
    fn resolve_prefers_recorded_cost_over_an_estimate() {
        let (cost, text, estimated) = resolve_cost(2.5, Some("claude-3-7-sonnet"), Some(1_000_000));
        assert_eq!((cost, estimated), (2.5, false));
        assert_eq!(text, "$2.50");

        let (cost, text, estimated) = resolve_cost(0.0, Some("claude-3-7-sonnet"), Some(1_000_000));
        assert_eq!((cost, estimated), (6.0, true));
        assert_eq!(text, "~$6.00");

        // 估不出来就什么都不报，不许回落成一个编的数
        let (cost, text, estimated) = resolve_cost(0.0, Some("some-local-finetune"), Some(1_000_000));
        assert_eq!((cost, text, estimated), (0.0, String::new(), false));

        let (cost, text, estimated) = resolve_cost(0.0, None, None);
        assert_eq!((cost, text, estimated), (0.0, String::new(), false));
    }
}
