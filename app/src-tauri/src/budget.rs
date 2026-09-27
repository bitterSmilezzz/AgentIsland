//! Token 预算告警的状态机（Swift `TokenBudgetTracker`）。
//!
//! 口径：预算度量的是 `tokens24h`——**滚动 24 小时**，与卡片、汇总栏、分析页的 24h 同一个数。
//! 因此这里**不按自然日重置**告警级别。Swift 的注释记着这条曾经的错误：
//! 23:50 已经报过 90%，00:05 归零后同一段 24 小时窗口还没滚动掉任何用量，
//! 于是同一个越线再报一次，每天午夜重复——只要用量一直压在线上。
//! 重新武装只由回落给（<75% 的滞回），告警次数因此与「越线次数」而不是「日历翻页」对齐。
//!
//! 这一层只做「该不该报警」的判定，不含时间与价格：`now_ms` 由调用方给，
//! 于是它可以被离线逐条断言（Swift 侧同样是注入 `now`）。

use serde::Serialize;

/// 预算状态。`Disabled` = 没设预算（0 表示未设，与 Swift 同口径）
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum BudgetStatus {
    Disabled,
    #[serde(rename_all = "camelCase")]
    Normal {
        used: i64,
        budget: i64,
        ratio: f64,
    },
    #[serde(rename_all = "camelCase")]
    Warning {
        used: i64,
        budget: i64,
        ratio: f64,
    },
    #[serde(rename_all = "camelCase")]
    Exceeded {
        used: i64,
        budget: i64,
        ratio: f64,
    },
}

impl BudgetStatus {
    pub fn is_exceeded(&self) -> bool {
        matches!(self, BudgetStatus::Exceeded { .. })
    }

}

impl Default for BudgetStatus {
    fn default() -> Self {
        BudgetStatus::Disabled
    }
}

/// 预警线 80%、超额线 100%、回落重新武装线 75%
pub const WARNING_RATIO: f64 = 0.8;
pub const EXCEEDED_RATIO: f64 = 1.0;
pub const REARM_RATIO: f64 = 0.75;

#[derive(Default)]
pub struct BudgetTracker {
    /// 0 = normal、1 = warning、2 = exceeded
    last_notified_level: u8,
}

impl BudgetTracker {
    pub fn new() -> Self {
        BudgetTracker::default()
    }

    /// 复位（预算被取消时调用）。不清的话，用户设上预算的那一刻会莫名报一次旧级别。
    pub fn reset(&mut self) {
        self.last_notified_level = 0;
    }

    /// 引擎用例要观察「级别记忆」这一位：它是这个状态机里唯一有状态的东西，
    /// 只读访问器比让用例去点私有字段更稳（字段名一改用例就编译不过）。
    #[cfg(test)]
    pub fn level_for_test(&self) -> u8 {
        self.last_notified_level
    }

    /// 评估用量。**只有跨级时**才给 `alert_message`（同一级别不重复报）。
    ///
    /// 返回 `(状态, 可选的一句话说给用户听)`
    pub fn evaluate(&mut self, used_24h: i64, budget: i64, _now_ms: i64) -> (BudgetStatus, Option<String>) {
        // `now_ms` 目前不参与判定（滚动口径下不需要日历日）。保留参数是为了让调用方
        // 不必在跨日那一刻改接口，也为了让「曾经按自然日重置」这条历史在签名上留个位置。
        if budget <= 0 {
            self.last_notified_level = 0;
            return (BudgetStatus::Disabled, None);
        }
        let used = used_24h.max(0);
        let ratio = used as f64 / budget as f64;
        let mut alert = None;

        let status = if ratio >= EXCEEDED_RATIO {
            let status = BudgetStatus::Exceeded {
                used,
                budget,
                ratio,
            };
            if self.last_notified_level < 2 {
                self.last_notified_level = 2;
                alert = Some(format!(
                    "Token 消费已达预算上限：{}% ({} / {})",
                    (ratio * 100.0) as i64,
                    crate::tokens::compact(used),
                    crate::tokens::compact(budget)
                ));
            }
            status
        } else if ratio >= WARNING_RATIO {
            let status = BudgetStatus::Warning {
                used,
                budget,
                ratio,
            };
            if self.last_notified_level < 1 {
                self.last_notified_level = 1;
                alert = Some(format!(
                    "Token 消费已接近预算预警线：{}% ({} / {})",
                    (ratio * 100.0) as i64,
                    crate::tokens::compact(used),
                    crate::tokens::compact(budget)
                ));
            }
            status
        } else {
            if ratio < REARM_RATIO {
                // 回落才重新武装：跨自然日不重置（见模块头注释）
                self.last_notified_level = 0;
            }
            BudgetStatus::Normal {
                used,
                budget,
                ratio,
            }
        };
        (status, alert)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_budget_means_disabled_and_never_alerts() {
        let mut tracker = BudgetTracker::new();
        let (status, alert) = tracker.evaluate(9_999_999, 0, 1_000);
        assert_eq!(status, BudgetStatus::Disabled);
        assert_eq!(alert, None);
        // 没预算时不该留下级别记忆（否则用户设上预算的那一刻会莫名报警）
        assert_eq!(tracker.last_notified_level, 0);
        let (_, alert) = tracker.evaluate(9_999_999, 0, 1_000);
        assert_eq!(alert, None);
    }

    #[test]
    fn crossing_the_warning_line_alerts_exactly_once() {
        let mut tracker = BudgetTracker::new();
        // 79% 不报
        let (status, alert) = tracker.evaluate(790, 1_000, 0);
        assert!(matches!(status, BudgetStatus::Normal { .. }));
        assert_eq!(alert, None);
        // 80% 报一次
        let (status, alert) = tracker.evaluate(800, 1_000, 0);
        assert!(matches!(status, BudgetStatus::Warning { .. }));
        assert_eq!(
            alert.as_deref(),
            Some("Token 消费已接近预算预警线：80% (800 / 1000)")
        );
        // 同级不重复报（这是这个状态机存在的全部理由）
        let (_, alert) = tracker.evaluate(850, 1_000, 0);
        assert_eq!(alert, None);
    }

    #[test]
    fn crossing_the_exceeded_line_alerts_once_on_top_of_the_warning() {
        let mut tracker = BudgetTracker::new();
        let (_, alert) = tracker.evaluate(810, 1_000, 0);
        assert!(alert.is_some(), "预警线该报");
        let (status, alert) = tracker.evaluate(1_050, 1_000, 0);
        assert!(matches!(status, BudgetStatus::Exceeded { .. }));
        assert_eq!(
            alert.as_deref(),
            Some("Token 消费已达预算上限：105% (1050 / 1000)")
        );
        let (_, alert) = tracker.evaluate(2_000, 1_000, 0);
        assert_eq!(alert, None, "同级不重复报");
    }

    #[test]
    fn a_direct_jump_from_normal_to_exceeded_reports_the_exceeded_alert_only() {
        // 小预算 + 一次采样跳过大半 ⇒ 只该收到「超额」那一条，不该补发预警
        let mut tracker = BudgetTracker::new();
        let (status, alert) = tracker.evaluate(1_200, 1_000, 0);
        assert!(matches!(status, BudgetStatus::Exceeded { .. }));
        assert_eq!(
            alert.as_deref(),
            Some("Token 消费已达预算上限：120% (1200 / 1000)")
        );
        assert_eq!(tracker.last_notified_level, 2);
    }

    #[test]
    fn only_a_fall_below_seventy_five_percent_rearms_the_alert() {
        let mut tracker = BudgetTracker::new();
        tracker.evaluate(900, 1_000, 0); // 报 warning
        // 回到 80%（仍然 warning 级）：不重新武装，也不重复报
        let (_, alert) = tracker.evaluate(800, 1_000, 0);
        assert_eq!(alert, None);
        // 落到 75% 以上、80% 以下：级别仍是 normal，但**不**重新武装
        let (status, alert) = tracker.evaluate(760, 1_000, 0);
        assert!(matches!(status, BudgetStatus::Normal { .. }));
        assert_eq!(alert, None);
        assert_eq!(tracker.last_notified_level, 1, "76% 不重新武装（滞回在下一次越线时才起作用）");
        // 再冲上 80%：因为没重新武装，所以**不报**
        let (_, alert) = tracker.evaluate(820, 1_000, 0);
        assert_eq!(alert, None);
        // 真正回落（<75%）之后才会再报
        tracker.evaluate(700, 1_000, 0);
        assert_eq!(tracker.last_notified_level, 0, "回落 <75% 才重新武装");
        let (_, alert) = tracker.evaluate(820, 1_000, 0);
        assert_eq!(
            alert.as_deref(),
            Some("Token 消费已接近预算预警线：82% (820 / 1000)")
        );
    }

    #[test]
    fn a_negative_reading_is_clamped_instead_of_producing_a_negative_ratio() {
        // 数据源写入异常时不该让比例变成负数（那会让「用量为负」看起来像健康）
        let mut tracker = BudgetTracker::new();
        let (status, _) = tracker.evaluate(-500, 1_000, 0);
        match status {
            BudgetStatus::Normal { used, ratio, .. } => {
                assert_eq!(used, 0);
                assert_eq!(ratio, 0.0);
            }
            other => panic!("应报正常，实际 {other:?}"),
        }
    }

    #[test]
    fn the_warning_and_exceeded_messages_use_the_same_compaction_as_the_cards() {
        // 卡片上写 1.2M、告警里写 1200000 会让人以为是两件事
        let mut tracker = BudgetTracker::new();
        let (_, alert) = tracker.evaluate(12_500_000, 12_000_000, 0);
        let message = alert.expect("应报超额");
        assert!(message.contains("12.50M / 12.00M"), "{message}");
    }

    #[test]
    fn the_status_carries_the_numbers_the_ui_needs() {
        // 界面要的是「哪一档 + 用了多少 + 预算多少 + 比例」，不是一句「超了」
        let mut tracker = BudgetTracker::new();
        let (status, _) = tracker.evaluate(2_000, 1_000, 0);
        assert_eq!(
            status,
            BudgetStatus::Exceeded {
                used: 2_000,
                budget: 1_000,
                ratio: 2.0
            }
        );
        let (status, _) = tracker.evaluate(850, 1_000, 0);
        assert_eq!(
            status,
            BudgetStatus::Warning {
                used: 850,
                budget: 1_000,
                ratio: 0.85
            }
        );
        let (status, _) = tracker.evaluate(500, 1_000, 0);
        assert_eq!(
            status,
            BudgetStatus::Normal {
                used: 500,
                budget: 1_000,
                ratio: 0.5
            }
        );
    }
}
