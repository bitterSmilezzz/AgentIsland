//! 异常驻留与死锁持续守护状态机（对齐 Swift `AgentResilienceGuard`，v0.0.75 起）。
//!
//! 与 [`crate::health`] 的分工：健康度回答「这一刻怎么样」（无状态纯函数），
//! 守护回答「这个坏状态已经持续多久了」（有状态，按阈值与冷却发告警）。

use crate::health::MEMORY_SEVERE_BYTES;
use crate::models::AgentSnapshot;
use std::collections::HashMap;

/// 判定持续死锁的告警门槛（默认 180 秒 = 3 分钟）
pub const HUNG_THRESHOLD_MS: i64 = 180_000;
/// 判定内存严重泄漏的告警门槛（默认 300 秒 = 5 分钟）
pub const MEMORY_THRESHOLD_MS: i64 = 300_000;
/// 同一类型告警的冷却间隔（默认 600 秒 = 10 分钟）
pub const ALERT_COOLDOWN_MS: i64 = 600_000;

#[derive(Default)]
struct AgentState {
    hung_since: Option<i64>,
    last_hung_alert: Option<i64>,
    high_mem_since: Option<i64>,
    last_mem_alert: Option<i64>,
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
    /// 评估快照列表，返回本拍需要发出的持久性严重告警。
    ///
    /// 两条规则是**载荷性**的，删掉就会静默改变行为：
    /// ① 进程不在 ⇒ 该 agent 的状态整份作废（只清一部分会让「下一次刚出现的异常」被旧的冷却挡掉）；
    /// ② 条件消失 ⇒ 起点与「上次告警时刻」一起清零（否则「好了又坏」会被十分钟冷却吃掉，
    ///    而用户看到的将是「明明又卡了三分钟却没提醒」）。
    pub fn evaluate(&mut self, snapshots: &[AgentSnapshot], now_ms: i64) -> Vec<Alert> {
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

            // 2. 持续物理内存超高
            if snap.memory_bytes >= MEMORY_SEVERE_BYTES {
                let since = *st.high_mem_since.get_or_insert(now_ms);
                let elapsed = now_ms - since;
                let cooled = st
                    .last_mem_alert
                    .map(|last| now_ms - last >= self.alert_cooldown_ms)
                    .unwrap_or(true);
                if elapsed >= self.memory_threshold_ms && cooled {
                    st.last_mem_alert = Some(now_ms);
                    alerts.push(Alert {
                        agent_id: snap.id.clone(),
                        agent_name: snap.name.clone(),
                        kind: "memory",
                        message: format!(
                            "{} 内存长期占用过高 ({})，防爆保护建议重置",
                            snap.name, snap.memory_text
                        ),
                        elapsed_ms: elapsed,
                    });
                }
            } else {
                st.high_mem_since = None;
                st.last_mem_alert = None;
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

    fn snapshot(id: &str, running: bool, is_hung: Option<bool>, memory: u64, memory_text: &str) -> AgentSnapshot {
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
        }
    }

    #[test]
    fn a_condition_must_persist_before_it_alerts() {
        let mut guard = Guard::default();
        let hung = snapshot("a", true, Some(true), 0, "—");
        let start = 1_000_000i64;
        assert!(guard.evaluate(&[hung.clone()], start).is_empty(), "刚卡上不告警");
        assert!(guard
            .evaluate(&[hung.clone()], start + HUNG_THRESHOLD_MS - 1)
            .is_empty(), "差 1ms 还不够");
        let alerts = guard.evaluate(&[hung.clone()], start + HUNG_THRESHOLD_MS);
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].kind, "hung");
        assert_eq!(alerts[0].message, "A 疑似死锁已达 3 分钟，建议点击逃生舱重置");
    }

    #[test]
    fn a_second_alert_waits_for_the_cooldown() {
        let mut guard = Guard::default();
        let hung = snapshot("a", true, Some(true), 0, "—");
        let start = 0i64;
        assert!(guard.evaluate(&[hung.clone()], start).is_empty());
        assert_eq!(guard.evaluate(&[hung.clone()], HUNG_THRESHOLD_MS).len(), 1, "首次告警");
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
        assert!(guard.evaluate(&[healthy.clone()], HUNG_THRESHOLD_MS + 1_000).is_empty());

        // 立刻又卡：起点从此刻算，凑满 3 分钟就应再报
        let again = HUNG_THRESHOLD_MS + 2_000;
        assert!(guard.evaluate(&[hung.clone()], again).is_empty());
        assert_eq!(
            guard.evaluate(&[hung.clone()], again + HUNG_THRESHOLD_MS).len(),
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
        assert!(guard.evaluate(&[unknown.clone()], HUNG_THRESHOLD_MS / 2).is_empty());
        // 窗口已被清掉：即便后面一直卡，也要重新凑满 3 分钟
        assert!(guard.evaluate(&[hung.clone()], HUNG_THRESHOLD_MS).is_empty());
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
        assert_eq!(guard.evaluate(&[hung.clone()], HUNG_THRESHOLD_MS).len(), 1, "第一次告警");

        guard.evaluate(&[stopped.clone()], HUNG_THRESHOLD_MS + 1_000);
        let restart = HUNG_THRESHOLD_MS + 2_000;
        assert!(guard.evaluate(&[hung.clone()], restart).is_empty(), "重启后重新计时");
        assert_eq!(
            guard.evaluate(&[hung.clone()], restart + HUNG_THRESHOLD_MS).len(),
            1,
            "进程离开又回来 ⇒ 状态与冷却都应作废"
        );
    }

    #[test]
    fn memory_alerts_on_its_own_threshold_and_text() {
        let mut guard = Guard::default();
        let big = snapshot("a", true, Some(false), MEMORY_SEVERE_BYTES, "2.4 GB");
        let normal = snapshot("a", true, Some(false), MEMORY_SEVERE_BYTES - 1, "1.9 GB");
        assert!(guard.evaluate(&[big.clone()], 0).is_empty());
        assert!(guard.evaluate(&[normal.clone()], MEMORY_THRESHOLD_MS - 1).is_empty(), "掉下门槛即清零");
        assert!(
            guard.evaluate(&[normal.clone()], MEMORY_THRESHOLD_MS).is_empty(),
            "内存不高时不管等多久都不报"
        );
        assert!(guard.evaluate(&[big.clone()], MEMORY_THRESHOLD_MS + 1).is_empty(), "重新计时");
        let alerts = guard.evaluate(&[big.clone()], MEMORY_THRESHOLD_MS * 2 + 2);
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].kind, "memory");
        assert!(alerts[0].message.contains("2.4 GB"), "文案要带上内存读数");
    }

    #[test]
    fn hung_and_memory_cooldowns_are_independent() {
        let mut guard = Guard::default();
        let both = snapshot("a", true, Some(true), MEMORY_SEVERE_BYTES, "2.4 GB");
        assert!(guard.evaluate(&[both.clone()], 0).is_empty());
        let alerts = guard.evaluate(&[both.clone()], MEMORY_THRESHOLD_MS);
        assert_eq!(alerts.len(), 2, "两个条件都到点了：卡死 3 分钟、内存 5 分钟");
        assert_eq!(alerts[0].kind, "hung");
        assert_eq!(alerts[1].kind, "memory");
    }
}
