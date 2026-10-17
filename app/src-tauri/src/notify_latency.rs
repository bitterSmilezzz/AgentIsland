//! 已接受通知的服务入口到引擎入队延迟，保留最近 1000 条非负毫秒样本。
//! 负样本累计计入 discarded；测量不完整时不作达标判定。

use std::collections::VecDeque;

pub const MAX_SAMPLES: usize = 1000;
pub const TARGET_P95_MS: i64 = 1_000;
pub const MIN_SAMPLES_FOR_VERDICT: usize = 20;

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LatencySummary {
    pub samples: usize,
    pub p95_ms: Option<i64>,
    /// 记录器生命周期内累计丢弃的负样本数。
    pub discarded: u64,
    pub verdict_available: bool,
    /// 至少 20 条有效样本且没有丢弃记录时判定，否则为 None。
    pub meets_target: Option<bool>,
}

/// 由引擎持有，调用方负责同步。
#[derive(Debug, Default)]
pub struct NotifyLatency {
    samples_ms: VecDeque<i64>,
    discarded: u64,
}

impl NotifyLatency {
    pub fn new() -> Self {
        Self::default()
    }

    /// 记录一次「进入服务 → 入队」的耗时。
    ///
    /// 所有非负 i64 值均接受；负值被丢弃并计入 `discarded`。
    pub fn record(&mut self, elapsed_ms: i64) -> bool {
        if elapsed_ms < 0 {
            self.discarded = self.discarded.saturating_add(1);
            return false;
        }
        if self.samples_ms.len() >= MAX_SAMPLES {
            self.samples_ms.pop_front();
        }
        self.samples_ms.push_back(elapsed_ms);
        true
    }

    pub fn samples(&self) -> usize {
        self.samples_ms.len()
    }

    pub fn discarded(&self) -> u64 {
        self.discarded
    }

    /// 最近窗口的 nearest-rank p95：排序后取 `ceil(0.95 * n) - 1` 下标。
    pub fn p95_ms(&self) -> Option<i64> {
        if self.samples_ms.is_empty() {
            return None;
        }
        let mut sorted: Vec<i64> = self.samples_ms.iter().copied().collect();
        sorted.sort_unstable();
        let n = sorted.len();
        // ceil(0.95 * n) 用整数算术：(95 * n + 99) / 100 即向上取整。
        let rank = ((95 * n) + 99) / 100;
        let index = rank.saturating_sub(1).min(n - 1);
        Some(sorted[index])
    }

    pub fn summary(&self) -> LatencySummary {
        let samples = self.samples_ms.len();
        let verdict_available = samples >= MIN_SAMPLES_FOR_VERDICT && self.discarded == 0;
        let p95_ms = self.p95_ms();
        LatencySummary {
            samples,
            p95_ms,
            discarded: self.discarded,
            verdict_available,
            meets_target: if verdict_available {
                p95_ms.map(|value| value <= TARGET_P95_MS)
            } else {
                None
            },
        }
    }

    /// 供诊断输出用的一行文本。
    pub fn report_line(&self) -> String {
        let summary = self.summary();
        let measurement = summary
            .p95_ms
            .map(|p95| format!("p95 {p95}ms"))
            .unwrap_or_else(|| "尚无样本".to_string());
        let verdict = match summary.meets_target {
            Some(true) => format!("≤ {TARGET_P95_MS}ms，达标"),
            Some(false) => format!("> {TARGET_P95_MS}ms，未达标"),
            None if summary.discarded > 0 => "测量不完整，未判定".to_string(),
            None => format!("样本不足 {MIN_SAMPLES_FOR_VERDICT}，未判定"),
        };
        format!(
            "入队延迟 {measurement}（样本 {}，丢弃 {}，{verdict}）",
            summary.samples, summary.discarded
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filled(values: &[i64]) -> NotifyLatency {
        let mut tracker = NotifyLatency::new();
        for value in values {
            tracker.record(*value);
        }
        tracker
    }

    #[test]
    fn empty_tracker_has_no_p95_and_no_verdict() {
        let tracker = NotifyLatency::new();
        assert_eq!(tracker.p95_ms(), None);
        let summary = tracker.summary();
        assert_eq!(summary.samples, 0);
        assert!(!summary.verdict_available);
        assert_eq!(summary.meets_target, None);
        assert!(tracker.report_line().contains("尚无样本"));
    }

    #[test]
    fn p95_is_nearest_rank_not_interpolated() {
        // 1..=100，最近邻口径下 ceil(0.95*100)=95 → 下标 94 → 值 95。
        // 线性插值会给出 95.05；这里钉住最近邻，避免换算法就换结论。
        let tracker = filled(&(1..=100).collect::<Vec<_>>());
        assert_eq!(tracker.p95_ms(), Some(95));
    }

    #[test]
    fn nearest_rank_rounds_the_rank_up() {
        // n=21 → ceil(0.95*21)=ceil(19.95)=20 → 下标 19 → 值 20。
        // 若误用向下取整会得到下标 18（值 19），这正是要钉住的边界。
        let tracker = filled(&(1..=21).collect::<Vec<_>>());
        assert_eq!(tracker.p95_ms(), Some(20));
    }

    #[test]
    fn single_sample_is_its_own_p95() {
        let tracker = filled(&[7]);
        assert_eq!(tracker.p95_ms(), Some(7));
    }

    #[test]
    fn only_negative_samples_are_discarded() {
        let mut tracker = NotifyLatency::new();
        assert!(!tracker.record(-1));
        assert!(!tracker.record(i64::MIN));
        assert!(tracker.record(120));
        assert_eq!(tracker.samples(), 1);
        assert_eq!(tracker.discarded(), 2);
        assert_eq!(tracker.p95_ms(), Some(120));
        assert_eq!(tracker.summary().meets_target, None);
    }

    #[test]
    fn boundaries_are_inclusive() {
        let mut tracker = NotifyLatency::new();
        for value in [0, 60_000, 60_001, i64::MAX] {
            assert!(tracker.record(value));
        }
        assert_eq!(tracker.samples(), 4);
        assert_eq!(tracker.discarded(), 0);
        assert_eq!(tracker.p95_ms(), Some(i64::MAX));
        assert_eq!(
            filled(&[0; MIN_SAMPLES_FOR_VERDICT]).summary().meets_target,
            Some(true)
        );
    }

    #[test]
    fn samples_are_bounded_and_drop_the_oldest() {
        let mut tracker = NotifyLatency::new();
        for value in 0..(MAX_SAMPLES as i64 + 250) {
            tracker.record(value);
        }
        assert_eq!(tracker.samples(), MAX_SAMPLES);
        // 最早的 250 个（0..250）已被丢弃，所以最小样本是 250。
        let mut sorted: Vec<i64> = tracker.samples_ms.iter().copied().collect();
        sorted.sort_unstable();
        assert_eq!(sorted[0], 250);
        assert_eq!(tracker.p95_ms(), Some(1_199));
        assert_eq!(tracker.discarded(), 0);
    }

    #[test]
    fn verdict_requires_enough_samples() {
        let tracker = filled(&[10; MIN_SAMPLES_FOR_VERDICT - 1]);
        let summary = tracker.summary();
        assert!(!summary.verdict_available);
        assert_eq!(summary.meets_target, None, "样本不足不得给出达标结论");
        assert!(tracker.report_line().contains("不足"));
    }

    #[test]
    fn target_is_met_at_the_boundary_and_missed_above_it() {
        let met = filled(&[TARGET_P95_MS; MIN_SAMPLES_FOR_VERDICT]);
        assert_eq!(
            met.summary().meets_target,
            Some(true),
            "恰好等于目标视为达标"
        );
        assert!(met.summary().verdict_available);
        assert!(met.report_line().contains("，达标"));

        let missed = filled(&[TARGET_P95_MS + 1; MIN_SAMPLES_FOR_VERDICT]);
        assert_eq!(missed.summary().meets_target, Some(false));
        assert!(missed.report_line().contains("未达标"));
    }

    #[test]
    fn one_slow_sample_does_not_move_p95_but_many_do() {
        // 99 快 + 1 慢：n=100 → 下标 94 → 仍是快值。单点抖动不该报警。
        let mut values = vec![5; 99];
        values.push(9_000);
        assert_eq!(filled(&values).p95_ms(), Some(5));

        // 慢样本占到 6% 时 p95 必须翻上去——这才是「p95 在管的事」。
        let mut values = vec![5; 94];
        values.extend([61_000; 6]);
        let tracker = filled(&values);
        let summary = tracker.summary();
        assert_eq!(summary.samples, 100);
        assert_eq!(summary.discarded, 0);
        assert_eq!(summary.p95_ms, Some(61_000));
        assert!(summary.verdict_available);
        assert_eq!(summary.meets_target, Some(false));
    }

    #[test]
    fn negative_samples_prevent_a_verdict_even_after_the_window_rolls_over() {
        let mut tracker = filled(&[5; MIN_SAMPLES_FOR_VERDICT]);
        assert_eq!(tracker.summary().meets_target, Some(true));
        assert!(!tracker.record(-5));
        for _ in 0..=MAX_SAMPLES {
            let summary = tracker.summary();
            assert!(!summary.verdict_available);
            assert_eq!(summary.meets_target, None);
            assert!(tracker.record(5));
        }
        assert_eq!(tracker.discarded(), 1);
        let line = tracker.report_line();
        assert!(line.contains("丢弃 1"), "丢弃必须可见：{line}");
        assert!(line.contains("测量不完整，未判定"));
        assert!(!line.contains("达标"));
    }

    #[test]
    fn summary_serializes_camel_case_fields_and_unknown_verdict() {
        let mut tracker = filled(&[5; MIN_SAMPLES_FOR_VERDICT]);
        assert!(!tracker.record(-1));
        assert_eq!(
            serde_json::to_value(tracker.summary()).unwrap(),
            serde_json::json!({
                "samples": 20,
                "p95Ms": 5,
                "discarded": 1,
                "verdictAvailable": false,
                "meetsTarget": null
            })
        );
    }
}
