use super::*;
use crate::filemon::FileActivityResult;
use std::sync::mpsc;
use std::time::Duration;

/// Drive the production decision path with synthetic samples, never tick the OS monitors.
struct Replay {
    engine: ActivityEngine,
    profile: AgentProfile,
}

impl Replay {
    fn new() -> Self {
        let (_, rx) = mpsc::channel();
        Self {
            engine: ActivityEngine::new(Settings::default(), rx),
            profile: AgentProfile {
                id: "fixture-agent".into(),
                name: "Fixture Agent".into(),
                glyph: String::new(),
                emoji: String::new(),
                process_names: vec![],
                cmdline_hints: vec![],
                path_excludes: vec![],
                cpu_floor: None,
                session_dirs: vec![],
                token_roots: vec![],
                session_database: None,
                category: "assistant".into(),
            },
        }
    }

    fn sample(
        &mut self,
        now: i64,
        running: bool,
        cpu: Option<f64>,
        signal: Option<Signal>,
    ) -> ActivityLevel {
        self.sample_with_write(now, running, cpu, signal, None)
    }

    fn sample_with_write(
        &mut self,
        now: i64,
        running: bool,
        cpu: Option<f64>,
        signal: Option<Signal>,
        latest_write: Option<SystemTime>,
    ) -> ActivityLevel {
        let level = self.engine.decide_level(
            &self.profile,
            now,
            running,
            cpu,
            self.engine.settings.cpu_threshold,
            60.0,
            10.0,
            &FileActivityResult {
                latest_write,
                latest_file: None,
            },
            &session::SessionProbe {
                signal,
                subagent_count: 0,
            },
            None,
        );
        // tick publishes each returned level before the next sample; preserve that seam.
        self.engine.snapshots = vec![AgentSnapshot {
            id: self.profile.id.clone(),
            name: self.profile.name.clone(),
            glyph: String::new(),
            emoji: String::new(),
            level,
            level_label: level.label().into(),
            observability: crate::observability::evaluate(crate::observability::Evidence {
                level,
                process_running: running,
                installed: running.then_some(true),
                source_unreadable: false,
                has_local_detail_source: false,
                recent_session_write: latest_write.is_some(),
                has_token_usage: false,
            }),
            process_running: running,
            is_hung: None,
            health: crate::health::Report::not_running(),
            cpu_percent: cpu,
            memory_bytes: 0,
            memory_text: "—".into(),
            last_activity_text: "—".into(),
            token_usage: None,
            pid: None,
            current_action: None,
            subagent_count: 0,
        }];
        level
    }
}

#[test]
fn agents_without_token_source_do_not_return_zero_usage_reports() {
    let (_, rx) = mpsc::channel();
    let mut engine = ActivityEngine::new(Settings::default(), rx);
    assert!(engine.get_report("opencode").is_none());
    assert!(engine.get_report("cline").is_none());
    assert!(engine.get_report("roo-code").is_none());
    assert!(engine.get_report("unknown-agent").is_none());
}

#[test]
fn working_hold_expires_despite_frequent_sampling() {
    let mut replay = Replay::new();
    assert_eq!(
        replay.sample(100_000, true, Some(10.0), None),
        ActivityLevel::Working
    );
    for now in [102_000, 104_000, 106_000, 108_000] {
        assert_eq!(
            replay.sample(now, true, Some(0.0), None),
            ActivityLevel::Working
        );
    }
    assert_eq!(
        replay.sample(110_000, true, Some(0.0), None),
        ActivityLevel::Idle,
        "ten seconds since the last real signal must expire even with two-second samples"
    );
    assert!(
        replay.engine.latest_event.is_none(),
        "CPU-only work must end silently"
    );
}

#[test]
fn real_work_renews_hold_even_after_a_long_task() {
    let mut replay = Replay::new();
    for now in [100_000, 200_000, 500_000] {
        assert_eq!(
            replay.sample(now, true, Some(10.0), None),
            ActivityLevel::Working
        );
    }
    assert_eq!(
        replay.sample(509_999, true, None, None),
        ActivityLevel::Working
    );
    assert_eq!(
        replay.sample(510_000, true, None, None),
        ActivityLevel::Idle
    );
}

#[test]
fn clock_rewind_reanchors_hold_without_perpetually_renewing_it() {
    let mut replay = Replay::new();
    replay.sample(100_000, true, Some(10.0), None);
    assert_eq!(
        replay.sample(50_000, true, None, None),
        ActivityLevel::Working
    );
    assert_eq!(
        replay.sample(59_999, true, None, None),
        ActivityLevel::Working
    );
    assert_eq!(replay.sample(60_000, true, None, None), ActivityLevel::Idle);
}

#[test]
fn five_state_replay_honors_semantics_before_cpu_and_hold() {
    let mut replay = Replay::new();
    assert_eq!(
        replay.sample(100_000, false, None, None),
        ActivityLevel::Offline
    );
    assert_eq!(
        replay.sample(101_000, true, None, None),
        ActivityLevel::Idle
    );
    assert_eq!(
        replay.sample(
            102_000,
            true,
            None,
            Some(Signal::Active("call".into(), None))
        ),
        ActivityLevel::Working
    );
    assert_eq!(
        replay.sample(
            103_000,
            true,
            Some(30.0),
            Some(Signal::Attention("ask".into(), "Confirm?".into()))
        ),
        ActivityLevel::Attention
    );
    assert_eq!(
        replay.sample(104_000, true, None, None),
        ActivityLevel::Idle
    );
    assert_eq!(
        replay.sample_with_write(
            105_000,
            true,
            Some(30.0),
            Some(Signal::Completed("done".into())),
            Some(SystemTime::UNIX_EPOCH + Duration::from_millis(105_000))
        ),
        ActivityLevel::Completed
    );
    assert_eq!(
        replay.sample(106_000, true, None, None),
        ActivityLevel::Idle
    );
    assert_eq!(
        replay.sample(
            107_000,
            false,
            Some(30.0),
            Some(Signal::Attention("old-ask".into(), "Old".into()))
        ),
        ActivityLevel::Offline
    );
}

#[test]
fn offline_ends_work_without_completion_and_restart_has_no_hold() {
    let mut replay = Replay::new();
    replay.sample(100_000, true, Some(75.0), None);
    assert_eq!(
        replay.sample(101_000, false, None, None),
        ActivityLevel::Offline
    );
    assert!(replay.engine.latest_event.is_none());
    assert_eq!(
        replay.sample(102_000, true, None, None),
        ActivityLevel::Idle
    );
    // A disconnected CPU run cannot contribute to the next run's five-minute warning.
    replay.sample(401_000, true, Some(75.0), None);
    assert!(replay.engine.latest_event.is_none());
}

#[test]
fn cpu_floor_and_user_threshold_both_apply_and_unknown_is_not_activity() {
    for (floor, user, threshold) in [
        (Some(20.0), 6.0, 20.0),
        (Some(20.0), 30.0, 30.0),
        (None, 6.0, 6.0),
    ] {
        let mut replay = Replay::new();
        replay.profile.cpu_floor = floor;
        replay.engine.settings.cpu_threshold = user;
        assert_eq!(
            replay.sample(100_000, true, None, None),
            ActivityLevel::Idle
        );
        assert_eq!(
            replay.sample(100_000, true, Some(threshold - 0.01), None),
            ActivityLevel::Idle
        );
        assert_eq!(
            replay.sample(100_000, true, Some(threshold), None),
            ActivityLevel::Working
        );
    }
}

#[test]
fn file_freshness_uses_the_sampling_clock_and_includes_the_window_boundary() {
    let write = SystemTime::UNIX_EPOCH + Duration::from_millis(100_000);
    for (now, expected) in [
        (90_000, ActivityLevel::Working),
        (160_000, ActivityLevel::Working),
        (160_001, ActivityLevel::Idle),
    ] {
        let mut replay = Replay::new();
        assert_eq!(
            replay.sample_with_write(now, true, None, None, Some(write)),
            expected
        );
    }
}

#[test]
fn repeated_semantic_fingerprints_do_not_repeat_notifications() {
    let mut replay = Replay::new();
    let ask = Some(Signal::Attention("ask-once".into(), "Confirm?".into()));
    replay.sample(100_000, true, None, ask.clone());
    assert_eq!(
        replay.engine.latest_event.take().unwrap().event_type,
        "attention"
    );
    replay.sample(101_000, true, None, ask);
    assert!(replay.engine.latest_event.is_none());

    let write = Some(SystemTime::UNIX_EPOCH + Duration::from_millis(102_000));
    let done = Some(Signal::Completed("done-once".into()));
    replay.sample_with_write(102_000, true, None, done.clone(), write);
    assert_eq!(
        replay.engine.latest_event.take().unwrap().event_type,
        "completed"
    );
    replay.sample_with_write(103_000, true, None, done, write);
    assert!(replay.engine.latest_event.is_none());
}

#[test]
fn work_hold_is_isolated_per_agent() {
    let mut replay = Replay::new();
    replay.sample(100_000, true, Some(10.0), None);
    replay.profile.id = "another-fixture-agent".into();
    assert_eq!(
        replay.sample(101_000, true, None, None),
        ActivityLevel::Idle
    );
}

// MARK: 告警事件的发布路径（守护 → 事件队列 → 确认）

/// 造一份「卡死且内存超高」的快照，用来驱动守护（不碰真实进程与真实时钟）
fn alarming_snapshot(running: bool) -> AgentSnapshot {
    AgentSnapshot {
        id: "fixture-agent".into(),
        name: "Fixture Agent".into(),
        glyph: String::new(),
        emoji: String::new(),
        level: ActivityLevel::Working,
        level_label: "工作中".into(),
        observability: crate::observability::evaluate(crate::observability::Evidence {
            level: ActivityLevel::Working,
            process_running: running,
            installed: Some(true),
            source_unreadable: false,
            has_local_detail_source: false,
            recent_session_write: true,
            has_token_usage: false,
        }),
        is_hung: Some(true),
        health: crate::health::Report::not_running(),
        process_running: running,
        cpu_percent: Some(97.0),
        memory_bytes: crate::health::MEMORY_SEVERE_BYTES + 1,
        memory_text: "2.4 GB".into(),
        last_activity_text: "—".into(),
        token_usage: None,
        pid: Some(4242),
        current_action: None,
        subagent_count: 0,
    }
}

/// 同一拍里两条告警**都要活下来**：这正是一直以来被覆盖式 `push_event` 丢掉的那条。
/// 走的是 `refresh` 真正调用的那条路径（`publish_guard_alerts`），不是另写一段测试专用代码。
#[test]
fn two_alerts_in_one_tick_both_reach_the_user() {
    let (_, rx) = mpsc::channel();
    let mut engine = ActivityEngine::new(Settings::default(), rx);
    let snap = alarming_snapshot(true);
    let start = 1_000_000i64;

    // 第一拍：两个条件都刚成立，只记起点
    engine.publish_guard_alerts(&[snap.clone()], start);
    assert!(engine.latest_event.is_none(), "刚成立不告警");

    // 第二拍：卡死 3 分钟 + 内存 5 分钟同时到点 ⇒ 两条
    engine.publish_guard_alerts(&[snap.clone()], start + crate::resilience::MEMORY_THRESHOLD_MS);
    let first = engine.latest_event.clone().expect("队首应有一条");
    assert_eq!(first.event_type, "attention");
    assert!(first.message.as_deref().unwrap().contains("死锁"), "先发卡死那条");
    assert_eq!(engine.pending_count(), 1, "第二条在排队，不许被顶掉");

    engine.ack_latest_event();
    let second = engine.latest_event.clone().expect("确认后应推下一条");
    assert!(second.message.as_deref().unwrap().contains("内存长期占用过高"));
    assert_ne!(second.id, first.id, "两条是各自的事件，不是同一条重复上屏");

    engine.ack_latest_event();
    assert!(engine.latest_event.is_none(), "都确认完就清空");
}

/// 开关真的关得住：`auto_anomalies_alert = false` 时一条都不发。
/// 这条不测，那个设置项就是装饰品。
#[test]
fn the_anomaly_alert_switch_actually_gates_the_publication() {
    let (_, rx) = mpsc::channel();
    let mut engine = ActivityEngine::new(Settings::default(), rx);
    engine.settings.auto_anomalies_alert = false;
    let snap = alarming_snapshot(true);
    let start = 1_000_000i64;
    engine.publish_guard_alerts(&[snap.clone()], start);
    engine.publish_guard_alerts(&[snap.clone()], start + crate::resilience::MEMORY_THRESHOLD_MS);
    assert!(engine.latest_event.is_none());
    assert_eq!(engine.pending_count(), 0);

    // 打开后从这一刻起**重新计时**——开关关着时守护根本没被调用，所以没有可继承的窗口。
    // 这与 Swift 一致（`if autoAlert { resilienceGuard.evaluate(...) }`）。
    engine.settings.auto_anomalies_alert = true;
    let reopened = start + 2 * crate::resilience::MEMORY_THRESHOLD_MS;
    engine.publish_guard_alerts(&[snap.clone()], reopened);
    assert!(engine.latest_event.is_none(), "打开的那一拍才开始计时");
    engine.publish_guard_alerts(&[snap.clone()], reopened + crate::resilience::MEMORY_THRESHOLD_MS);
    assert!(engine.latest_event.is_some(), "打开后照旧发得出来");
}

/// 队列有界：用户一直不确认也不会无限增长，且保留的是**最近**的那些。
#[test]
fn the_pending_queue_is_bounded_and_keeps_the_newest() {
    let (_, rx) = mpsc::channel();
    let mut engine = ActivityEngine::new(Settings::default(), rx);
    for i in 0..200 {
        engine.push_event(AgentTaskEvent {
            id: format!("ev-{i}"),
            agent_id: "a".into(),
            agent_name: "A".into(),
            event_type: "completed".into(),
            timestamp: i,
            message: Some(format!("第 {i} 条")),
            detail: None,
            externally_delivered: false,
        });
    }
    assert!(engine.pending_count() <= 64, "待发队列必须有界：{}", engine.pending_count());
    // 队首仍是最早那条（用户先看到它），队尾是最后进来的
    assert_eq!(engine.latest_event.as_ref().unwrap().id, "ev-0");
    let mut last = None;
    while let Some(ev) = engine.latest_event.clone() {
        last = Some(ev.id.clone());
        engine.ack_latest_event();
    }
    assert_eq!(last.as_deref(), Some("ev-199"), "最新的那条不能被丢掉");
}
