//! 持续资源增长与疑似卡死守护。内存按趋势判定，不按固定 RSS 判定。
//!
//! 与 [`crate::health`] 的分工：健康度回答「这一刻怎么样」（无状态纯函数），
//! 守护回答「这个坏状态已经持续多久了」（有状态，按阈值与冷却发告警）。

use crate::health::{MemoryGrowth, MEMORY_OBSERVATION_FLOOR_BYTES};
use crate::models::AgentSnapshot;
use std::collections::{HashMap, VecDeque};

/// 判定持续死锁的告警门槛（默认 180 秒 = 3 分钟）
pub const HUNG_THRESHOLD_MS: i64 = 180_000;
/// Each growth window lasts 5 minutes; two adjacent windows must both qualify.
pub const MEMORY_THRESHOLD_MS: i64 = 300_000;
/// 同一类型告警的冷却间隔（默认 600 秒 = 10 分钟）
pub const ALERT_COOLDOWN_MS: i64 = 600_000;

#[derive(Default)]
struct AgentState {
    hung_since: Option<i64>,
    last_hung_alert: Option<i64>,
    memory: MemoryTrend,
    last_mem_alert: Option<i64>,
}

// Reject isolated jumps, sparse samples, restarts and stable high plateaus.
const MEMORY_MIN_GROWTH_BYTES: u64 = 512 * 1024 * 1024;
const MEMORY_MAX_SAMPLE_GAP_MS: i64 = 30_000;
// The shortest supported engine interval is 500ms: retain a full 10min window.
const MEMORY_MAX_SAMPLES: usize = 1280;

#[derive(Default)]
struct MemoryTrend {
    pid: Option<u32>,
    samples: VecDeque<(i64, u64)>,
    growth: Option<MemoryGrowth>,
}

impl MemoryTrend {
    fn observe(&mut self, snap: &AgentSnapshot, now: i64, window: i64) {
        let interrupted = self.pid != snap.pid
            || self
                .samples
                .back()
                .is_some_and(|(t, _)| now < *t || now - *t > MEMORY_MAX_SAMPLE_GAP_MS);
        if interrupted || snap.memory_bytes == 0 {
            self.samples.clear();
            self.growth = None;
        }
        self.pid = snap.pid;
        if snap.memory_bytes == 0 || self.samples.back().is_some_and(|(t, _)| *t == now) {
            return;
        }
        self.samples.push_back((now, snap.memory_bytes));
        let window = window.max(1);
        let cutoff = now.saturating_sub(window.saturating_mul(2));
        // Preserve one observation at/before the start of the rolling window.
        while self.samples.len() > 1 && self.samples[1].0 <= cutoff {
            self.samples.pop_front();
        }
        while self.samples.len() > MEMORY_MAX_SAMPLES {
            self.samples.pop_front();
        }
        self.growth = None;
        let Some(&(start, baseline)) = self.samples.front() else {
            return;
        };
        if start > cutoff || snap.memory_bytes < MEMORY_OBSERVATION_FLOOR_BYTES {
            return;
        }
        let Some(&(middle_time, middle)) =
            self.samples.iter().rev().find(|(t, _)| *t <= now - window)
        else {
            return;
        };
        // Sampling does not land exactly on a 5min boundary. Require the full
        // 10min span, allowing one sampling interval around the middle.
        if middle_time - start < window.saturating_sub(MEMORY_MAX_SAMPLE_GAP_MS) {
            return;
        }
        let qualifies = |base: u64, current: u64| {
            current.saturating_sub(base) >= MEMORY_MIN_GROWTH_BYTES
                && u128::from(current.saturating_sub(base)) * 4 >= u128::from(base)
        };
        // A down/up fluctuation or child-process jump is insufficient evidence.
        let steady = self
            .samples
            .iter()
            .zip(self.samples.iter().skip(1))
            .all(|((_, a), (_, b))| u128::from(*b) * 100 >= u128::from(*a) * 95);
        if steady && qualifies(baseline, middle) && qualifies(middle, snap.memory_bytes) {
            self.growth = Some(MemoryGrowth {
                increase_bytes: snap.memory_bytes.saturating_sub(baseline),
                elapsed_ms: now - start,
            });
        }
    }
}

/// 一条待发事件。字段只留引擎需要的：Swift 的 `AgentTaskEvent` 还带 `duration`/`pid`，
/// Rust 的事件模型没有这两列（见 `models.rs`），时长在这里给出、由调用方决定怎么用。
#[derive(Debug, Clone, PartialEq)]
pub struct Alert {
    pub agent_id: String,
    pub agent_name: String,
    /// `hung` | `memory`——冷却与去重按类型分开
    pub kind: &'static str,
    pub message: String,
    pub elapsed_ms: i64,
}

pub struct Guard {
    states: HashMap<String, AgentState>,
    pub hung_threshold_ms: i64,
    pub memory_threshold_ms: i64,
    pub alert_cooldown_ms: i64,
}

impl Default for Guard {
    fn default() -> Self {
        Guard {
            states: HashMap::new(),
            hung_threshold_ms: HUNG_THRESHOLD_MS,
            memory_threshold_ms: MEMORY_THRESHOLD_MS,
            alert_cooldown_ms: ALERT_COOLDOWN_MS,
        }
    }
}

impl Guard {
    /// Observation is independent of notification preferences.
    pub fn observe_memory(&mut self, snapshots: &[AgentSnapshot], now: i64) {
        for snap in snapshots {
            if !snap.process_running {
                self.states.remove(&snap.id);
                continue;
            }
            let st = self.states.entry(snap.id.clone()).or_default();
            let reset = st.memory.pid != snap.pid
                || st.memory.samples.back().is_some_and(|(t, _)| now < *t);
            st.memory.observe(snap, now, self.memory_threshold_ms);
            // A threshold flicker must not reset notification cooldown.
            if reset {
                st.last_mem_alert = None;
            }
        }
        if self.states.len() > 8000 {
            self.states.clear();
        }
    }

    pub fn memory_growth(&self, id: &str) -> Option<MemoryGrowth> {
        self.states.get(id).and_then(|s| s.memory.growth)
    }

    /// 评估快照列表，返回本拍需要发出的持久性严重告警。
    ///
    /// 两条规则是**载荷性**的，删掉就会静默改变行为：
    /// ① 进程不在 ⇒ 该 agent 的状态整份作废（只清一部分会让「下一次刚出现的异常」被旧的冷却挡掉）；
    /// ② 卡死条件消失清计时；内存趋势临界波动保留冷却，避免反复提醒。
    pub fn evaluate(&mut self, snapshots: &[AgentSnapshot], now_ms: i64) -> Vec<Alert> {
        self.observe_memory(snapshots, now_ms);
        let mut alerts: Vec<Alert> = Vec::new();

        for snap in snapshots {
            if !snap.process_running {
                self.states.remove(&snap.id);
                continue;
            }
            let st = self.states.entry(snap.id.clone()).or_default();

            // 1. 持续死锁。只有 `Some(true)` 计时——`None` 是「本轮判不出」，
            //    把它当成卡死会凭空告警，把它当成正常又会清掉正在计时的窗口。
            //    Swift 的口径是后者（`else` 分支一律清零），这里照搬。
            if snap.is_hung == Some(true) {
                let since = *st.hung_since.get_or_insert(now_ms);
                let elapsed = now_ms - since;
                let cooled = st
                    .last_hung_alert
                    .map(|last| now_ms - last >= self.alert_cooldown_ms)
                    .unwrap_or(true);
                if elapsed >= self.hung_threshold_ms && cooled {
                    st.last_hung_alert = Some(now_ms);
                    alerts.push(Alert {
                        agent_id: snap.id.clone(),
                        agent_name: snap.name.clone(),
                        kind: "hung",
                        message: format!(
                            "{} 疑似死锁已达 {} 分钟，建议点击逃生舱重置",
                            snap.name,
                            elapsed / 60_000
                        ),
                        elapsed_ms: elapsed,
                    });
                }
            } else {
                st.hung_since = None;
                st.last_hung_alert = None;
            }

            // 2. Confirmed sustained growth, never absolute resident size.
            if let Some(growth) = st.memory.growth {
                let cooled = st
                    .last_mem_alert
                    .map(|last| now_ms - last >= self.alert_cooldown_ms)
                    .unwrap_or(true);
                if cooled {
                    st.last_mem_alert = Some(now_ms);
                    alerts.push(Alert {
                        agent_id: snap.id.clone(), agent_name: snap.name.clone(), kind: "memory",
                        message: format!("{} 内存连续增长：{} 分钟增加 {}，当前 {}；请检查任务进展与系统内存压力",
                            snap.name, growth.elapsed_ms / 60_000, crate::procmon::memory_text(growth.increase_bytes), snap.memory_text),
                        elapsed_ms: growth.elapsed_ms,
                    });
                }
            }
        }

        // 状态表有界：与引擎其它 per-agent 表同一量级
        if self.states.len() > 8000 {
            self.states.clear();
        }
        alerts
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ActivityLevel;

    fn snapshot(
        id: &str,
        running: bool,
        is_hung: Option<bool>,
        memory: u64,
        memory_text: &str,
    ) -> AgentSnapshot {
        AgentSnapshot {
            id: id.into(),
            name: id.to_uppercase(),
            glyph: String::new(),
            emoji: String::new(),
            level: ActivityLevel::Idle,
            level_label: "空闲".into(),
            observability: crate::observability::Verdict {
                code: crate::observability::Code::Observed,
                summary: "",
                evidence: Vec::new(),
            },
            is_hung,
            health: crate::health::Report::not_running(),
            process_running: running,
            installed: None,
            work_stats: crate::duration::Stats::empty(),
            provenance: None,
            provenance_suffix: String::new(),
            cpu_percent: Some(1.0),
            memory_bytes: memory,
            memory_text: memory_text.into(),
            last_activity_text: "—".into(),
            token_usage: None,
            pid: None,
            current_action: None,
            subagent_count: 0,
            session_probe_health: None,
            background_tasks: vec![],
            subagents: vec![],
            token_breakdown: None,
        }
    }

    #[test]
    fn stable_high_memory_never_alerts() {
        let mut guard = Guard::default();
        let big = snapshot("codex", true, Some(false), 4 * 1024 * 1024 * 1024, "4 GB");
        for tick in 0..=80 {
            assert!(
                guard.evaluate(&[big.clone()], tick * 30_000).is_empty(),
                "稳定占用不是异常，tick={tick}"
            );
        }
    }

    #[test]
    fn a_condition_must_persist_before_it_alerts() {
        let mut guard = Guard::default();
        let hung = snapshot("a", true, Some(true), 0, "—");
        let start = 1_000_000i64;
        assert!(
            guard.evaluate(&[hung.clone()], start).is_empty(),
            "刚卡上不告警"
        );
        assert!(
            guard
                .evaluate(&[hung.clone()], start + HUNG_THRESHOLD_MS - 1)
                .is_empty(),
            "差 1ms 还不够"
        );
        let alerts = guard.evaluate(&[hung.clone()], start + HUNG_THRESHOLD_MS);
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].kind, "hung");
        assert_eq!(
            alerts[0].message,
            "A 疑似死锁已达 3 分钟，建议点击逃生舱重置"
        );
    }

    #[test]
    fn a_second_alert_waits_for_the_cooldown() {
        let mut guard = Guard::default();
        let hung = snapshot("a", true, Some(true), 0, "—");
        let start = 0i64;
        assert!(guard.evaluate(&[hung.clone()], start).is_empty());
        assert_eq!(
            guard.evaluate(&[hung.clone()], HUNG_THRESHOLD_MS).len(),
            1,
            "首次告警"
        );
        assert!(
            guard
                .evaluate(&[hung.clone()], HUNG_THRESHOLD_MS + ALERT_COOLDOWN_MS - 1)
                .is_empty(),
            "冷却内不重复告警"
        );
        assert_eq!(
            guard
                .evaluate(&[hung.clone()], HUNG_THRESHOLD_MS + ALERT_COOLDOWN_MS)
                .len(),
            1,
            "冷却过后再提醒一次（还在卡，用户需要知道）"
        );
    }

    #[test]
    fn recovery_clears_the_cooldown_so_a_new_episode_alerts_again() {
        // 「好了又坏」被十分钟冷却吃掉，是这条状态机最容易出的错
        let mut guard = Guard::default();
        let hung = snapshot("a", true, Some(true), 0, "—");
        let healthy = snapshot("a", true, Some(false), 0, "—");
        assert!(guard.evaluate(&[hung.clone()], 0).is_empty());
        assert_eq!(guard.evaluate(&[hung.clone()], HUNG_THRESHOLD_MS).len(), 1);

        // 恢复正常 → 状态整组作废
        assert!(guard
            .evaluate(&[healthy.clone()], HUNG_THRESHOLD_MS + 1_000)
            .is_empty());

        // 立刻又卡：起点从此刻算，凑满 3 分钟就应再报
        let again = HUNG_THRESHOLD_MS + 2_000;
        assert!(guard.evaluate(&[hung.clone()], again).is_empty());
        assert_eq!(
            guard
                .evaluate(&[hung.clone()], again + HUNG_THRESHOLD_MS)
                .len(),
            1,
            "新一轮不该被上一轮的冷却挡住"
        );
    }

    #[test]
    fn not_evaluated_never_starts_a_timer_but_does_clear_one() {
        // `None` 是「本轮判不出」。它不该开始计时；按 Swift 口径它也清掉正在计时的窗口
        let mut guard = Guard::default();
        let unknown = snapshot("a", true, None, 0, "—");
        let hung = snapshot("a", true, Some(true), 0, "—");
        assert!(guard.evaluate(&[hung.clone()], 0).is_empty());
        assert!(guard
            .evaluate(&[unknown.clone()], HUNG_THRESHOLD_MS / 2)
            .is_empty());
        // 窗口已被清掉：即便后面一直卡，也要重新凑满 3 分钟
        assert!(guard
            .evaluate(&[hung.clone()], HUNG_THRESHOLD_MS)
            .is_empty());
        assert_eq!(
            guard.evaluate(&[hung.clone()], HUNG_THRESHOLD_MS * 2).len(),
            1,
            "重新计时：从 180s 起算，到 360s 才够"
        );
    }

    #[test]
    fn a_stopped_agent_drops_its_state_and_its_cooldown() {
        // 行为断言（而不是去数内部状态表）：进程消失后再卡，**不该**被上一轮的冷却挡住。
        // 上一轮告警发生在 180s，冷却到 780s；若状态没被丢掉，这里就会一次都不报。
        let mut guard = Guard::default();
        let hung = snapshot("a", true, Some(true), 0, "—");
        let stopped = snapshot("a", false, None, 0, "—");
        assert!(guard.evaluate(&[hung.clone()], 0).is_empty());
        assert_eq!(
            guard.evaluate(&[hung.clone()], HUNG_THRESHOLD_MS).len(),
            1,
            "第一次告警"
        );

        guard.evaluate(&[stopped.clone()], HUNG_THRESHOLD_MS + 1_000);
        let restart = HUNG_THRESHOLD_MS + 2_000;
        assert!(
            guard.evaluate(&[hung.clone()], restart).is_empty(),
            "重启后重新计时"
        );
        assert_eq!(
            guard
                .evaluate(&[hung.clone()], restart + HUNG_THRESHOLD_MS)
                .len(),
            1,
            "进程离开又回来 ⇒ 状态与冷却都应作废"
        );
    }

    fn rising(tick: i64, hung: Option<bool>) -> AgentSnapshot {
        let bytes = MEMORY_OBSERVATION_FLOOR_BYTES + tick as u64 * 128 * 1024 * 1024;
        snapshot("a", true, hung, bytes, &crate::procmon::memory_text(bytes))
    }

    #[test]
    fn two_continuous_growth_windows_alert_with_measured_facts() {
        let mut guard = Guard::default();
        for tick in 0..20 {
            assert!(guard
                .evaluate(&[rising(tick, Some(false))], tick * 30_000)
                .is_empty());
        }
        let alerts = guard.evaluate(&[rising(20, Some(false))], 600_000);
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].kind, "memory");
        assert!(alerts[0].message.contains("10 分钟增加"));
        assert!(!alerts[0].message.contains("防爆"));
        assert!(!alerts[0].message.contains("重置"));
        assert!(guard.memory_growth("a").is_some());
        assert!(
            guard
                .evaluate(&[rising(21, Some(false))], 630_000)
                .is_empty(),
            "cooldown prevents repeated growth alerts"
        );
    }

    #[test]
    fn loading_then_plateau_and_small_fluctuations_do_not_alert() {
        let mut guard = Guard::default();
        for tick in 0..=40 {
            // Real growth for the first 5min, then a stable working set with noise.
            let mut snap = rising(tick.min(10), Some(false));
            if tick > 10 {
                snap.memory_bytes += (tick % 2) as u64 * 32 * 1024 * 1024;
            }
            assert!(guard.evaluate(&[snap], tick * 30_000).is_empty());
        }
        assert!(guard.memory_growth("a").is_none());
    }

    #[test]
    fn a_single_child_process_jump_is_not_continuous_growth() {
        let mut guard = Guard::default();
        for tick in 0..=40 {
            let mut snap = rising(0, Some(false));
            if tick >= 10 {
                snap.memory_bytes *= 2;
            }
            assert!(guard.evaluate(&[snap], tick * 30_000).is_empty());
        }
    }

    #[test]
    fn restart_stop_missing_readings_and_observation_gaps_discard_the_baseline() {
        for reset in 0..5 {
            let mut guard = Guard::default();
            for tick in 0..=19 {
                guard.evaluate(&[rising(tick, Some(false))], tick * 30_000);
            }
            let mut snap = rising(20, Some(false));
            let now = match reset {
                0 => {
                    snap.pid = Some(123);
                    600_000
                }
                1 => {
                    snap.process_running = false;
                    600_000
                }
                2 => {
                    snap.memory_bytes = 0;
                    600_000
                }
                3 => 900_000,
                _ => 100_000, // clock rollback / sleep discontinuity
            };
            assert!(guard.evaluate(&[snap], now).is_empty());
            assert!(guard.memory_growth("a").is_none(), "reset={reset}");
        }
    }

    #[test]
    fn sparse_samples_cannot_confirm_continuous_growth() {
        let mut guard = Guard::default();
        for tick in [0, 10, 20] {
            assert!(guard
                .evaluate(&[rising(tick, Some(false))], tick * 30_000)
                .is_empty());
        }
        assert!(guard.memory_growth("a").is_none());
    }

    #[test]
    fn irregular_sampling_and_threshold_flicker_cannot_bypass_memory_cooldown() {
        let mut guard = Guard::default();
        let mut last_alert = None;
        let mut now = 0;
        let mut count = 0;
        for tick in 0..80 {
            let snap = rising(now / 30_000, Some(false));
            for alert in guard.evaluate(&[snap], now) {
                if alert.kind == "memory" {
                    if let Some(previous) = last_alert {
                        assert!(now - previous >= ALERT_COOLDOWN_MS);
                    }
                    last_alert = Some(now);
                    count += 1;
                }
            }
            now += if tick % 2 == 0 { 19_000 } else { 21_000 };
        }
        assert!(
            count > 0,
            "irregular valid sampling must still detect sustained growth"
        );
    }

    #[test]
    fn hung_and_growth_alerts_are_independent() {
        let mut guard = Guard::default();
        let mut kinds = Vec::new();
        for tick in 0..=20 {
            kinds.extend(
                guard
                    .evaluate(&[rising(tick, Some(true))], tick * 30_000)
                    .into_iter()
                    .map(|a| a.kind),
            );
        }
        assert_eq!(kinds, ["hung", "memory"]);
    }
}
