//! 资源 soak 的**冷/热/稳态分段判据**（OPS-01 首片）。
//!
//! 需求原文：监控与网关资源分别计量，长期有界、断网/睡眠/崩溃/升级/回滚不丢任务与账目。
//! 场景 token：**冷/热/稳态+30分钟及扩展soak**。
//!
//! 现状：`test-resource-soak.py` 是**故障注入**式的（mock 掉 Native、patch 掉等待），
//! 它验证「出问题时会不会丢数据」，不验证「长时间跑会不会涨」。长时增长这件事
//! 只有把采样按阶段分开看才能判——否则冷启动的一次性开销（字体、LaunchServices
//! 注册、着色器编译）会被当成泄漏，稳态里的真泄漏也会被冷启动峰值淹没。
//!
//! 判据（纯函数，不读真实进程）：
//!
//! 1. **三阶段推进**：`Cold → Warm → Steady`。进入稳态的条件是**连续 N 次采样
//!    稳定**（见 `STEADY_SAMPLES`），不是「跑够多少分钟」——机器慢的时候
//!    按时间判定会把还没稳的阶段当成稳态。
//! 2. **冷启动峰值单独记**：它**不作为稳态基线**（一次性开销不是泄漏）。
//! 3. **稳态基线与漂移上限**：稳态基线取窗口内的最小值；超过基线 + 允许漂移
//!    ⇒ 越界（疑似泄漏），**报告事实而不是下结论**。
//! 4. **单调增长检测**：连续 M 次采样都涨 ⇒ 疑似泄漏（比越界更早报警）。
//! 5. **不规整的采样不参与判定**：间隔异常（时钟回拨、睡眠唤醒后的跳变）
//!    的样本被排除，而不是拿它当「涨了」。

/// 进入稳态所需的连续稳定采样数。
pub const STEADY_SAMPLES: usize = 3;
/// 冷启动窗口的采样数：进程刚起时的一次性开销期（字体、LaunchServices 注册、
/// 着色器编译）。这段时间的峰值**不是泄漏**，不能作稳态基线。
pub const COLD_SAMPLES: usize = 3;
/// 单调增长检测的连续增长次数。
pub const GROWTH_STREAK: usize = 3;
/// 稳态允许的漂移（相对基线的百分比）。
pub const STEADY_DRIFT_PERCENT: u64 = 10;

/// 采样阶段。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// 冷启动：第一次运行，一次性开销期。
    Cold,
    /// 预热中：还在跑，但尚未观察到稳定。
    Warm,
    /// 稳态。
    Steady,
}

impl Phase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cold => "cold",
            Self::Warm => "warm",
            Self::Steady => "steady",
        }
    }
}

/// 一次资源采样。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sample {
    pub at_ms: i64,
    pub rss_bytes: u64,
}

/// 判定结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoakVerdict {
    /// 采样被接受（阶段可能变化）。
    Accepted,
    /// 采样被排除：间隔异常（时钟回拨或睡眠唤醒后的跳变）——不参与判定。
    IrregularInterval,
    /// 稳态越界：超过基线 + 允许漂移。
    DriftExceeded { baseline: u64, limit: u64 },
    /// 疑似泄漏：连续多次单调增长。
    GrowthSuspected,
}

/// 冷/热/稳态判定器。
#[derive(Debug, Clone)]
pub struct SoakJudge {
    phase: Phase,
    /// 冷启动峰值（单列，不作稳态基线）。
    cold_peak: Option<u64>,
    /// 稳态基线（窗口内最小值）。
    steady_baseline: Option<u64>,
    /// 连续稳定采样计数。
    stable_streak: usize,
    /// 连续增长计数。
    growth_streak: usize,
    /// 上一次采样。
    last: Option<Sample>,
    /// 期望的采样间隔（毫秒）；超过它的间隔视为不规整。
    expected_interval_ms: i64,
    /// 已摄入的样本数（用于划分冷启动窗口）。
    samples_seen: usize,
}

impl SoakJudge {
    pub fn new(expected_interval_ms: i64) -> Self {
        SoakJudge {
            phase: Phase::Cold,
            cold_peak: None,
            steady_baseline: None,
            stable_streak: 0,
            growth_streak: 0,
            last: None,
            expected_interval_ms: expected_interval_ms.max(1),
            samples_seen: 0,
        }
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// 冷启动峰值（只读）。
    pub fn cold_peak(&self) -> Option<u64> {
        self.cold_peak
    }

    /// 稳态基线（只读）。
    pub fn steady_baseline(&self) -> Option<u64> {
        self.steady_baseline
    }

    /// 摄入一次采样并给出判定。
    ///
    /// 三段口径：**冷启动窗口**（前 `COLD_SAMPLES` 个样本，一次性开销期，
    /// 峰值单独记、不参与稳态判定）→ **预热**（窗口结束但尚未连续稳定）→
    /// **稳态**（连续 `STEADY_SAMPLES` 次采样达到基线）。
    pub fn observe(&mut self, sample: Sample) -> SoakVerdict {
        // ① 间隔不规整的样本**排除**：时钟回拨（时刻变小）或间隔远超预期
        //    （睡眠唤醒）都会造出假「增长」。
        if let Some(last) = self.last {
            let delta = sample.at_ms - last.at_ms;
            if delta <= 0 || delta > self.expected_interval_ms * 3 {
                // **仍然更新 last**：判定基准必须是**最近一次观测**。
                // 若不更新，下一次采样就会相对更早的有效样本比较——
                // 时钟回拨会被算成一个「正常间隔」，把回拨变成假增长。
                self.last = Some(sample);
                return SoakVerdict::IrregularInterval;
            }
        }

        // ② 冷启动窗口：峰值单列，且**不参与**稳态漂移与增长判定
        //    （进程刚起时的一次性开销不是泄漏）。
        if self.samples_seen < COLD_SAMPLES {
            self.samples_seen += 1;
            self.cold_peak =
                Some(self.cold_peak.map_or(sample.rss_bytes, |p| p.max(sample.rss_bytes)));
            self.phase = Phase::Cold;
            self.last = Some(sample);
            return SoakVerdict::Accepted;
        }
        self.samples_seen += 1;
        if self.phase == Phase::Cold {
            self.phase = Phase::Warm;
        }

        // ③ 稳态基线与漂移
        let Some(baseline) = self.steady_baseline else {
            // 窗口结束后的第一个样本**建立基线**（冷启动峰值不作基线）
            self.steady_baseline = Some(sample.rss_bytes);
            self.stable_streak = 1;
            self.last = Some(sample);
            return SoakVerdict::Accepted;
        };
        // limit 是**绝对上限**（基线 + 允许漂移），比较的是采样值本身。
        let limit =
            baseline.saturating_add(baseline.saturating_mul(STEADY_DRIFT_PERCENT) / 100);
        if sample.rss_bytes > limit {
            // 越界：报告事实（基线与上限），不下「泄漏」的结论
            self.last = Some(sample);
            return SoakVerdict::DriftExceeded { baseline, limit };
        }
        // 基线取窗口内最小值（回落不算漂移）
        let new_baseline = baseline.min(sample.rss_bytes);
        self.steady_baseline = Some(new_baseline);
        if sample.rss_bytes == new_baseline {
            self.stable_streak += 1;
        } else {
            self.stable_streak = 1;
        }
        if self.phase != Phase::Steady && self.stable_streak >= STEADY_SAMPLES {
            self.phase = Phase::Steady;
        }

        // ④ 单调增长检测（相对上一次采样）
        let grew = self.last.is_some_and(|last| sample.rss_bytes > last.rss_bytes);
        if grew {
            self.growth_streak += 1;
        } else {
            self.growth_streak = 0;
        }
        self.last = Some(sample);
        if self.growth_streak >= GROWTH_STREAK {
            return SoakVerdict::GrowthSuspected;
        }
        SoakVerdict::Accepted
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn judge() -> SoakJudge {
        SoakJudge::new(1_000)
    }

    #[test]
    fn the_cold_peak_is_recorded_but_never_becomes_the_steady_baseline() {
        let mut j = judge();
        // 冷启动窗口：前 COLD_SAMPLES 个样本（一次性开销期）
        j.observe(Sample { at_ms: 0, rss_bytes: 100_000 });
        j.observe(Sample { at_ms: 1_000, rss_bytes: 180_000 });
        j.observe(Sample { at_ms: 2_000, rss_bytes: 100_000 });
        assert_eq!(j.cold_peak(), Some(180_000), "冷启动峰值单列");
        assert_eq!(j.phase(), Phase::Cold, "窗口内属冷启动期");

        // 窗口结束后的稳定水平 ⇒ 稳态基线是回落后的水平，**不是**冷启动峰值
        for i in 3..6 {
            j.observe(Sample { at_ms: i * 1_000, rss_bytes: 100_000 });
        }
        assert_eq!(j.phase(), Phase::Steady);
        assert_eq!(
            j.steady_baseline(),
            Some(100_000),
            "冷启动峰值不得作稳态基线"
        );
    }

    #[test]
    fn the_phase_only_reaches_steady_after_consecutive_stable_samples() {
        let mut j = judge();
        assert_eq!(j.phase(), Phase::Cold);
        // 冷启动窗口内
        j.observe(Sample { at_ms: 0, rss_bytes: 100 });
        assert_eq!(j.phase(), Phase::Cold, "冷启动窗口内仍属 Cold");
        // 窗口结束 ⇒ 预热（基线刚建立，尚未连续稳定）
        j.observe(Sample { at_ms: 1_000, rss_bytes: 100 });
        j.observe(Sample { at_ms: 2_000, rss_bytes: 100 });
        j.observe(Sample { at_ms: 3_000, rss_bytes: 100 });
        assert_eq!(j.phase(), Phase::Warm, "窗口结束但未连续稳定 ⇒ 预热");
        // 再连续两次稳定 ⇒ 稳态
        j.observe(Sample { at_ms: 4_000, rss_bytes: 100 });
        j.observe(Sample { at_ms: 5_000, rss_bytes: 100 });
        assert_eq!(j.phase(), Phase::Steady);
    }

    #[test]
    fn a_steady_drift_beyond_the_allowance_is_reported_with_its_baseline() {
        let mut j = judge();
        // 冷启动窗口 + 预热：建立基线 100_000
        for i in 0..4 {
            j.observe(Sample { at_ms: i * 1_000, rss_bytes: 100_000 });
        }
        // 涨到 120% ⇒ 超过 110% 的允许漂移
        let verdict = j.observe(Sample { at_ms: 4_000, rss_bytes: 120_000 });
        match verdict {
            SoakVerdict::DriftExceeded { baseline, limit } => {
                assert_eq!(baseline, 100_000);
                assert_eq!(limit, 110_000);
            }
            other => panic!("应当报越界，实际 {other:?}"),
        }
    }

    #[test]
    fn a_monotonic_growth_run_is_flagged_before_it_drifts_too_far() {
        let mut j = judge();
        // 冷启动窗口 + 预热：建立基线
        for i in 0..4 {
            j.observe(Sample { at_ms: i * 1_000, rss_bytes: 100_000 });
        }
        // 连续三次小幅增长（还没到 110% 的漂移上限）
        let v1 = j.observe(Sample { at_ms: 4_000, rss_bytes: 101_000 });
        let v2 = j.observe(Sample { at_ms: 5_000, rss_bytes: 102_000 });
        let v3 = j.observe(Sample { at_ms: 6_000, rss_bytes: 103_000 });
        assert_ne!(v1, SoakVerdict::GrowthSuspected, "第一次增长不算");
        assert_ne!(v2, SoakVerdict::GrowthSuspected, "第二次增长不算");
        assert_eq!(v3, SoakVerdict::GrowthSuspected, "第三次连续增长才判定");
    }

    #[test]
    fn irregular_intervals_are_excluded_instead_of_being_read_as_growth() {
        let mut j = judge();
        j.observe(Sample { at_ms: 0, rss_bytes: 100_000 });
        // 睡眠唤醒：间隔远超预期
        assert_eq!(
            j.observe(Sample { at_ms: 60_000, rss_bytes: 500_000 }),
            SoakVerdict::IrregularInterval,
            "睡眠后的跳变不是泄漏"
        );
        // 时钟回拨：时刻变小（相对**最近观测**才算回拨）
        assert_eq!(
            j.observe(Sample { at_ms: 500, rss_bytes: 600_000 }),
            SoakVerdict::IrregularInterval
        );
    }
}

