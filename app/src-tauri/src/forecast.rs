//! 月末预估（Swift `TokenForecastEvaluator`）。
//!
//! 以 **24 小时活跃度**作为当前基准日消耗率，线性外推到当月总天数。
//! 它是 CLI `tokens` 的招牌输出与分析页顶部那块，所以文案与数字口径都要照搬。
//!
//! 两个容易被忽略的钳制（Swift `SafeNumber` 里各有一段注释说明踩过的坑）：
//! ① `dailyBudget * 当月天数` 会溢出——Swift 的 `*` 溢出即 SIGTRAP（实测
//!    `tokens --budget 9e18`），Rust 的 debug 构建同样会 panic，所以这里用饱和乘；
//! ② 金额那条链路若不钳，脏 cost 会让月末预估跑到 `Inf`，界面上出现
//!    「累计 $1e9、预估月末 $3.1e10」这种自相矛盾的数（本该同量级差了 31 倍）。

use serde::Serialize;

/// 金额上限（与 Swift `SafeNumber.costCeiling` 同值）
pub const COST_CEILING: f64 = 1_000_000_000.0;

/// 饱和乘：溢出时按符号钳到两端，绝不 panic。
/// （Rust 在 debug 构建里对 i64 溢出的处理同样是 panic，与 Swift 的 trap 等价）
pub fn saturating_product(lhs: i64, rhs: i64) -> i64 {
    match lhs.checked_mul(rhs) {
        Some(value) => value,
        None => {
            if (lhs < 0) != (rhs < 0) {
                i64::MIN
            } else {
                i64::MAX
            }
        }
    }
}

/// 金额口径的饱和乘：Inf → 上限、NaN → 0、负数 → 0，最后钳在 `COST_CEILING`
pub fn cost_product(lhs: f64, days: i64) -> f64 {
    let days = days.max(0) as f64;
    if days <= 0.0 {
        return 0.0;
    }
    if lhs.is_infinite() {
        return if lhs > 0.0 { COST_CEILING } else { 0.0 };
    }
    if !lhs.is_finite() {
        return 0.0;
    }
    let value = lhs.max(0.0) * days;
    if !value.is_finite() {
        return COST_CEILING;
    }
    value.min(COST_CEILING)
}

/// 当月天数与今天几号（**本地时间**，与 Swift `Calendar.current` 同口径）
fn local_month_shape(now_ms: i64) -> (i64, i64) {
    let Some(tm) = crate::localclock::local_time(now_ms) else {
        // 取不到本地时间就退回 31 天的保守值：宁可预测偏大，也不要静默给 0
        return (31, 1);
    };
    let year = tm.tm_year + 1900;
    let month = tm.tm_mon + 1;
    (days_in_month(year, month), tm.tm_mday.max(1) as i64)
}

/// 某年某月的天数（闰年规则照抄日历：4 年一闰、100 年不闰、400 年再闰）
fn days_in_month(year: i32, month: i32) -> i64 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
            if leap {
                29
            } else {
                28
            }
        }
        _ => 30,
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForecastReport {
    pub projected_month_end_tokens: i64,
    pub projected_month_end_cost: f64,
    pub days_remaining_in_month: i64,
    pub total_days_in_month: i64,
    /// 按当前增速会在本月的第几天耗尽月度预算（只在「已超出日均预算且会在月内耗尽」时有值）
    pub budget_exhaustion_day: Option<i64>,
    /// 一句话结论（界面直接显示，不再自己拼）
    pub forecast_summary: String,
    /// 同上，已排版的两个数（`1.23M` / `$4.56`）——排版口径只有一处，别让界面各写一份
    pub formatted_monthly_tokens: String,
    pub formatted_monthly_cost: String,
}

/// 月末预估。`daily_budget` = 0 表示没设预算（与 Swift 同口径）
pub fn evaluate(tokens_24h: i64, cost_24h: f64, daily_budget: i64, now_ms: i64) -> ForecastReport {
    let (total_days, current_day) = local_month_shape(now_ms);
    let current_day = current_day.clamp(1, total_days);
    let days_remaining = (total_days - current_day).max(0);

    // 以 24h 活跃度作为当前基准日消耗率
    let daily_tokens = tokens_24h.max(0);
    let daily_cost = cost_24h.max(0.0);

    let projected_tokens = saturating_product(daily_tokens, total_days);
    let projected_cost = cost_product(daily_cost, total_days);

    let mut exhaustion_day = None;
    let summary;

    if daily_tokens <= 0 {
        summary = "近期暂无活跃消耗，月末预估平稳".to_string();
    } else if daily_budget > 0 {
        let monthly_budget = saturating_product(daily_budget, total_days);
        if daily_tokens > daily_budget {
            // 超出日均预算，推算何时耗尽月度总池
            let days = 1.max(monthly_budget / 1.max(daily_tokens));
            if days < total_days {
                exhaustion_day = Some(days);
                summary = format!("按当前增速，预估本月第 {days} 天将耗尽月度预算配额");
            } else {
                summary = format!(
                    "按当前增速，预计月末消耗 {} tokens",
                    crate::tokens::compact(projected_tokens)
                );
            }
        } else {
            // Swift：`Int(projected / max(1, monthlyBudget) * 100)`——先算比例再取整
            let usage_ratio =
                (projected_tokens as f64 / 1.max(monthly_budget) as f64 * 100.0) as i64;
            summary = format!("预算健康，预计月末使用率约为 {usage_ratio}%");
        }
    } else {
        let cost = crate::cost::format_cost(projected_cost);
        let cost_str = if cost.is_empty() {
            String::new()
        } else {
            format!(" ({cost})")
        };
        summary = format!(
            "按当前增速，预计月末总消耗 {} tokens{cost_str}",
            crate::tokens::compact(projected_tokens)
        );
    }

    ForecastReport {
        projected_month_end_tokens: projected_tokens,
        projected_month_end_cost: projected_cost,
        days_remaining_in_month: days_remaining,
        total_days_in_month: total_days,
        budget_exhaustion_day: exhaustion_day,
        forecast_summary: summary,
        formatted_monthly_tokens: crate::tokens::compact(projected_tokens),
        // 与 Swift `costText(_, zero: "$0.00")` 同口径：没有成本可报时写 `$0.00`
        formatted_monthly_cost: {
            let text = crate::cost::format_cost(projected_cost);
            if text.is_empty() {
                "$0.00".to_string()
            } else {
                text
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 造一个本地时间戳：用 `libc::mktime` 反推，避免写死「某年某月某日对应多少毫秒」
    /// （那会因为跑测试的机器时区不同而飘）
    fn local_ms(year: i32, month: i32, day: i32, hour: i32) -> i64 {
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        tm.tm_year = year - 1900;
        tm.tm_mon = month - 1;
        tm.tm_mday = day;
        tm.tm_hour = hour;
        tm.tm_isdst = -1;
        let seconds = unsafe { libc::mktime(&mut tm) };
        (seconds as i64) * 1000
    }

    #[test]
    fn days_in_month_follows_the_leap_rules() {
        assert_eq!(days_in_month(2026, 1), 31);
        assert_eq!(days_in_month(2026, 2), 28);
        assert_eq!(days_in_month(2024, 2), 29, "4 年一闰");
        assert_eq!(days_in_month(1900, 2), 28, "100 年不闰");
        assert_eq!(days_in_month(2000, 2), 29, "400 年再闰");
        assert_eq!(days_in_month(2026, 4), 30);
    }

    #[test]
    fn no_recent_activity_reports_a_flat_month() {
        let report = evaluate(0, 0.0, 0, local_ms(2026, 9, 27, 12));
        assert_eq!(report.projected_month_end_tokens, 0);
        assert_eq!(report.forecast_summary, "近期暂无活跃消耗，月末预估平稳");
        assert_eq!(report.total_days_in_month, 30);
        assert_eq!(report.days_remaining_in_month, 3, "9 月 27 日还剩 3 天");
        assert_eq!(report.formatted_monthly_cost, "$0.00", "零成本按出口约定写 $0.00");
        assert_eq!(report.formatted_monthly_tokens, "0");
    }

    #[test]
    fn without_a_budget_it_extrapolates_flat_to_month_end() {
        // 9 月 27 日、当天 1.2M ⇒ 月末 1.2M × 30 = 36M
        let report = evaluate(1_200_000, 0.42, 0, local_ms(2026, 9, 27, 12));
        assert_eq!(report.projected_month_end_tokens, 36_000_000);
        assert_eq!(report.projected_month_end_cost, 12.6);
        assert_eq!(report.budget_exhaustion_day, None);
        assert_eq!(
            report.forecast_summary,
            "按当前增速，预计月末总消耗 36.00M tokens ($12.60)"
        );
        assert_eq!(report.formatted_monthly_tokens, "36.00M");
        assert_eq!(report.formatted_monthly_cost, "$12.60");
    }

    #[test]
    fn with_no_cost_the_summary_omits_the_cost_clause_instead_of_writing_zero() {
        let report = evaluate(1_200_000, 0.0, 0, local_ms(2026, 9, 27, 12));
        assert_eq!(report.forecast_summary, "按当前增速，预计月末总消耗 36.00M tokens");
        assert!(!report.forecast_summary.contains("$"));
    }

    #[test]
    fn exceeding_the_daily_budget_reports_the_exhaustion_day() {
        // 日均预算 1M、当天 2M、9 月 30 天 ⇒ 月度池 30M，2M/天 ⇒ 第 15 天耗尽
        let report = evaluate(2_000_000, 0.0, 1_000_000, local_ms(2026, 9, 27, 12));
        assert_eq!(report.budget_exhaustion_day, Some(15));
        assert_eq!(
            report.forecast_summary,
            "按当前增速，预估本月第 15 天将耗尽月度预算配额"
        );
    }

    #[test]
    fn a_daily_overrun_always_reports_an_exhaustion_day_within_the_month() {
        // 8 月 31 天、日均预算 1M、当天 1.05M ⇒ 月度池 31M、1.05M/天 ⇒ 第 29 天耗尽
        let report = evaluate(1_050_000, 0.0, 1_000_000, local_ms(2026, 8, 31, 12));
        assert_eq!(report.budget_exhaustion_day, Some(29));

        // 不变量：只要「当天用量 > 日均预算」，就一定能给出月内的耗尽日。
        // 这条其实把 Swift 里那个 `else`（"预计月末消耗 …"）分支证成了**不可达**：
        // 设 T=当月天数、B=日均预算、D=当天用量，D > B ⇒
        //   floor(B·T / D) ≤ floor((D−1)·T / D) = T − ceil(T/D) < T
        // 即使 B·T 饱和到 i64::MAX 也不成立（饱和要求 B·T > i64::MAX ⇒ B > i64::MAX/T ≥ D，
        // 与 D > B 矛盾）。Rust 侧保留同样的分支结构以对齐 Swift，但这条不变量说明
        // 它今天永远不会走到——真走到就说明算术被改坏了。
        let mut checked = 0;
        for total_days in [28i64, 29, 30, 31] {
            // 用 2 月的闰年与平年各造一个时间戳
            let now = if total_days == 29 {
                local_ms(2024, 2, 10, 12)
            } else if total_days == 28 {
                local_ms(2026, 2, 10, 12)
            } else if total_days == 30 {
                local_ms(2026, 9, 10, 12)
            } else {
                local_ms(2026, 8, 10, 12)
            };
            for budget in [1i64, 7, 1_000, 999_999, 1_000_000] {
                for used in [budget + 1, budget + 2, budget * 2, budget * 3 + 1] {
                    let report = evaluate(used, 0.0, budget, now);
                    assert_eq!(
                        report.total_days_in_month, total_days,
                        "当月天数应取本地日历"
                    );
                    assert!(
                        report.budget_exhaustion_day.is_some(),
                        "用量({used}) > 日均预算({budget}) ⇒ 必须给出耗尽日，实际 {:?}",
                        report
                    );
                    let day = report.budget_exhaustion_day.unwrap();
                    assert!(day >= 1 && day < total_days, "耗尽日应在月内：{day}");
                    checked += 1;
                }
            }
        }
        assert!(checked >= 60, "不变量应覆盖足够多的组合，实际 {checked}");
    }

    #[test]
    fn a_healthy_budget_reports_a_month_end_usage_ratio() {
        // 日均预算 2M、当天 1M、9 月 30 天 ⇒ 月末 30M / 60M = 50%
        let report = evaluate(1_000_000, 0.0, 2_000_000, local_ms(2026, 9, 27, 12));
        assert_eq!(report.budget_exhaustion_day, None);
        assert_eq!(report.forecast_summary, "预算健康，预计月末使用率约为 50%");
    }

    #[test]
    fn the_projection_cannot_overflow_or_run_to_infinity() {
        // Swift 的 `*` 在这里会 SIGTRAP（它的注释记着 `tokens --budget 9e18` 实测崩过）
        let report = evaluate(i64::MAX, f64::MAX, i64::MAX, local_ms(2026, 9, 27, 12));
        assert_eq!(report.projected_month_end_tokens, i64::MAX, "饱和而不是 panic");
        assert_eq!(report.projected_month_end_cost, COST_CEILING, "金额钳在上限");
        assert!(report.projected_month_end_cost.is_finite());
        // 摘要也不该出现 Inf / NaN 字样
        assert!(!report.forecast_summary.contains("inf"), "{}", report.forecast_summary);
        assert!(!report.forecast_summary.contains("NaN"), "{}", report.forecast_summary);
    }

    #[test]
    fn a_negative_reading_is_clamped_to_zero() {
        let report = evaluate(-5_000, -3.0, 0, local_ms(2026, 9, 27, 12));
        assert_eq!(report.projected_month_end_tokens, 0);
        assert_eq!(report.projected_month_end_cost, 0.0);
        assert_eq!(report.forecast_summary, "近期暂无活跃消耗，月末预估平稳");
    }

    #[test]
    fn the_last_day_of_the_month_reports_zero_days_remaining() {
        let report = evaluate(1_000, 0.0, 0, local_ms(2026, 9, 30, 23));
        assert_eq!(report.days_remaining_in_month, 0);
        assert_eq!(report.total_days_in_month, 30);
    }
}
