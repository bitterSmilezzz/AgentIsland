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
                bundle_ids: vec![],
                cmdline_hints: vec![],
                path_excludes: vec![],
                path_contains: vec![],
                cpu_floor: None,
                session_dirs: vec![],
                token_roots: vec![],
                token_alert_floor: None,
                session_dialect: crate::models::SessionDialect::GenericTail,
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
                active_sessions: 0,
                latest_write,
                latest_file: None,
            },
            &session::SessionProbe {
                health: None,
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
            observability: crate::observability::evaluate(&crate::observability::Evidence {
                provenance: None,
                active_sessions: 0,
                level,
                process_running: running,
                installed: running.then_some(true),
                probe_health: None,
                probe_health_fresh: false,
                has_local_detail_source: false,
                has_token_usage: false,
            }),
            process_running: running,
            installed: None,
            work_stats: crate::duration::Stats::empty(),
            provenance: None,
            provenance_suffix: String::new(),
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
            session_probe_health: None,
            background_tasks: vec![],
            subagents: vec![],
            token_breakdown: None,
        }];
        level
    }
}

#[test]
fn agents_without_token_source_do_not_return_zero_usage_reports() {
    let (_, rx) = mpsc::channel();
    let mut engine = ActivityEngine::new(Settings::default(), rx);
    // 明确构造无用量源的档案，避免已安装的 OpenCode 数据库影响测试。
    for profile in &mut engine.profiles {
        profile.token_roots.clear();
        profile.session_database = None;
    }
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
fn repeated_attention_fingerprints_do_not_repeat_notifications() {
    let mut replay = Replay::new();
    let ask = Some(Signal::Attention("ask-once".into(), "Confirm?".into()));
    replay.sample(100_000, true, None, ask.clone());
    assert_eq!(
        replay.engine.latest_event.take().unwrap().event_type,
        "attention"
    );
    replay.sample(101_000, true, None, ask);
    assert!(replay.engine.latest_event.is_none());
}

#[test]
fn repeated_completion_fingerprints_do_not_repeat_notifications() {
    let mut replay = Replay::new();
    // 先真的工作一段（≥3.5 秒的门槛），否则完成事件按设计不推
    replay.sample(100_000, true, Some(20.0), None);
    assert_eq!(replay.engine.snapshots[0].level, ActivityLevel::Working);

    let write = Some(SystemTime::UNIX_EPOCH + Duration::from_millis(104_000));
    let done = Some(Signal::Completed("done-once".into()));
    replay.sample_with_write(104_000, true, None, done.clone(), write);
    let event = replay.engine.latest_event.take().expect("4 秒的实质工作应算一次完成");
    assert_eq!(event.event_type, "completed");
    assert!(
        (event.duration - 4.0).abs() < 1e-9,
        "事件要带上真实时长：{}",
        event.duration
    );
    // 同一份指纹再来一次：不再推
    replay.sample_with_write(105_000, true, None, done, write);
    assert!(replay.engine.latest_event.is_none());
}

#[test]
fn a_flash_of_work_shorter_than_the_threshold_is_not_a_task() {
    // Swift `recordTaskCompleted` 的门槛是 3.5 秒（「过滤瞬时微抖动」）。
    // 不够格时**既不推事件、也不记时长**——否则界面会冒出「任务完成 (0秒)」，
    // 而效能统计里会多出一次根本没发生的任务
    let mut replay = Replay::new();
    replay.sample(100_000, true, Some(20.0), None); // 起点 100_000
    let write = Some(SystemTime::UNIX_EPOCH + Duration::from_millis(102_000));
    replay.sample_with_write(102_000, true, None, Some(Signal::Completed("flash".into())), write);
    assert!(
        replay.engine.latest_event.is_none(),
        "2 秒的抖动不该算一次任务"
    );
    assert_eq!(
        replay
            .engine
            .durations
            .stats("fixture-agent", crate::duration::DEFAULT_WINDOW_MS, 102_000)
            .task_count,
        0,
        "不够格的任务不进效能统计"
    );
    // 够格的那次照常记
    replay.sample(103_000, true, Some(20.0), None);
    let write = Some(SystemTime::UNIX_EPOCH + Duration::from_millis(108_000));
    replay.sample_with_write(
        108_000,
        true,
        None,
        Some(Signal::Completed("real".into())),
        write,
    );
    let stats = replay.engine.durations.stats(
        "fixture-agent",
        crate::duration::DEFAULT_WINDOW_MS,
        108_000,
    );
    assert_eq!(stats.task_count, 1);
    assert!((stats.total_work_time - 5.0).abs() < 1e-9, "{}", stats.total_work_time);
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

// MARK: 外发闸门接线（事件 → 判定 → 记账）

/// 每个事件都要过一次外发闸门并留痕：否则界面只能显示「最近没发过」，
/// 看不出是被静默时段挡下、通道没配好，还是压根没接。
#[test]
fn every_event_records_an_outbound_decision() {
    let mut replay = Replay::new();
    replay.sample(
        100_000,
        true,
        Some(10.0),
        Some(Signal::Attention("fp-outbound".into(), "要不要继续".into())),
    );
    // 账本由**工作线程**回填（dispatch 不阻塞采样），所以这里轮询等它，
    // 而不是假设调用返回时就写好了——这条本身就证明了「不阻塞」是真的
    let mut recent = Vec::new();
    for _ in 0..200 {
        recent = replay.engine.state().recent_outbound;
        if !recent.is_empty() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(recent.len(), 1, "一条事件应留下一条外发判定");
    // 默认策略：总开关关 ⇒ 如实记「未发」，且不碰传输
    assert!(
        recent[0].text.starts_with("未发：总开关未开"),
        "{}",
        recent[0].text
    );
    assert!(!recent[0].delivered);
    assert_eq!(
        replay.engine.notifier.throttle_keys(),
        0,
        "被挡下不该占节流位"
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
        observability: crate::observability::evaluate(&crate::observability::Evidence {
            provenance: None,
            active_sessions: 0,
            level: ActivityLevel::Working,
            process_running: running,
            installed: Some(true),
            probe_health: None,
                probe_health_fresh: false,
            has_local_detail_source: false,
            has_token_usage: false,
        }),
        is_hung: Some(true),
        health: crate::health::Report::not_running(),
        process_running: running,
        installed: None,
        work_stats: crate::duration::Stats::empty(),
        provenance: None,
        provenance_suffix: String::new(),
        cpu_percent: Some(97.0),
        memory_bytes: crate::health::MEMORY_SEVERE_BYTES + 1,
        memory_text: "2.4 GB".into(),
        last_activity_text: "—".into(),
        token_usage: None,
        pid: Some(4242),
        current_action: None,
        subagent_count: 0,
        session_probe_health: None,
        background_tasks: vec![],
        subagents: vec![],
        token_breakdown: None,
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
            duration: 0.0,
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

// MARK: Token 预算接线（总量 → 状态机 → 事件）

/// 预算评估的接线：跨级推**一条**事件，同级不再推；关掉告警只停事件不停状态。
#[test]
fn the_budget_alerts_once_per_level_crossing() {
    let mut replay = Replay::new();
    replay.engine.settings.daily_token_budget = 1_000_000;
    replay.engine.settings.budget_alert_enabled = true;
    replay.engine.grand_total.tokens24h = 500_000;

    // 50%：正常，不推事件
    replay.engine.evaluate_budget(1_000);
    assert!(matches!(
        replay.engine.budget_status,
        crate::budget::BudgetStatus::Normal { .. }
    ));
    assert!(replay.engine.latest_event.is_none(), "没越线不该有事件");

    // 82%：推一条「预警」，数字落在 detail 里
    replay.engine.grand_total.tokens24h = 820_000;
    replay.engine.evaluate_budget(2_000);
    let event = replay.engine.latest_event.clone().expect("应推预警事件");
    assert_eq!(event.agent_id, "system", "预算不属于某个 Agent");
    assert_eq!(event.agent_name, "Token 预算");
    assert_eq!(event.event_type, "costSpike");
    assert_eq!(event.message.as_deref(), Some("⚠️ Token 预算预警"));
    assert_eq!(
        event.detail.as_deref(),
        Some("Token 消费已接近预算预警线：82% (820.0k / 1.00M)")
    );

    // 仍在这一档：不重复推（否则只要用量压在线上就每拍一条）
    replay.engine.latest_event = None;
    replay.engine.grand_total.tokens24h = 900_000;
    replay.engine.evaluate_budget(3_000);
    assert!(replay.engine.latest_event.is_none(), "同一档不该重复报");

    // 跨到超额：再推一条
    replay.engine.grand_total.tokens24h = 1_500_000;
    replay.engine.evaluate_budget(4_000);
    let event = replay.engine.latest_event.clone().expect("应推超额事件");
    assert_eq!(event.message.as_deref(), Some("🚨 Token 预算超额"));
    assert_eq!(
        event.detail.as_deref(),
        Some("Token 消费已达预算上限：150% (1.50M / 1.00M)")
    );
    assert!(replay.engine.budget_status.is_exceeded());
}

#[test]
fn turning_the_alert_off_keeps_the_status_but_stops_the_events() {
    let mut replay = Replay::new();
    replay.engine.settings.daily_token_budget = 1_000_000;
    replay.engine.settings.budget_alert_enabled = false;
    replay.engine.grand_total.tokens24h = 2_000_000;

    replay.engine.evaluate_budget(1_000);
    assert!(
        replay.engine.latest_event.is_none(),
        "关掉告警就不该推事件（采集与告警解耦）"
    );
    // 但状态照样给界面：用户关掉告警不代表不想看用量
    match replay.engine.budget_status {
        crate::budget::BudgetStatus::Normal { used, budget, .. } => {
            assert_eq!(used, 2_000_000);
            assert_eq!(budget, 1_000_000);
        }
        other => panic!("应给正常档的状态，实际 {other:?}"),
    }
}

#[test]
fn clearing_the_budget_rearms_the_tracker_so_the_next_budget_alert_is_honest() {
    let mut replay = Replay::new();
    replay.engine.settings.daily_token_budget = 1_000_000;
    replay.engine.grand_total.tokens24h = 2_000_000;
    replay.engine.evaluate_budget(1_000);
    assert!(replay.engine.latest_event.is_some());
    assert_eq!(replay.engine.budget.level_for_test(), 2);

    // 用户把预算清空：状态机必须复位，否则他再设一个预算时会**立刻**收到一条旧级别的告警
    replay.engine.settings.daily_token_budget = 0;
    replay.engine.evaluate_budget(2_000);
    assert!(matches!(
        replay.engine.budget_status,
        crate::budget::BudgetStatus::Disabled
    ));
    assert_eq!(replay.engine.budget.level_for_test(), 0, "清空预算要复位");
}

#[test]
fn a_negative_or_absurd_budget_from_a_hand_edited_file_is_normalized_away() {
    // 手改 settings.json 写负预算会让「超额」永远成立；越界值读到即钳制
    let mut settings = Settings::default();
    settings.daily_token_budget = -5;
    assert_eq!(settings.normalized().daily_token_budget, 0);
    settings.daily_token_budget = 9_999_999_999;
    assert_eq!(settings.normalized().daily_token_budget, 1_000_000_000);
}

#[test]
fn changed_cpu_threshold_controls_real_decision_path() {
    let mut replay = Replay::new();
    replay.engine.settings.cpu_threshold = 23.0;
    assert_eq!(replay.sample(100_000, true, Some(20.0), None), ActivityLevel::Idle);
    assert_eq!(replay.sample(101_000, true, Some(24.0), None), ActivityLevel::Working);
    assert_eq!(replay.sample(102_000, true, Some(20.0), None), ActivityLevel::Working);
    assert_eq!(replay.sample(112_000, true, Some(20.0), None), ActivityLevel::Idle);
}

#[test]
fn automatic_outbound_uses_presence_and_never_forwards_external_events() {
    struct OfflineTransport;
    impl crate::notifier::Transport for OfflineTransport {
        fn perform(&self, _: &crate::render::Request) -> crate::notifier::Outcome {
            panic!("在场或外部投递的事件不应外发");
        }
    }
    let mut replay = Replay::new();
    replay.engine.notifier = crate::notifier::Notifier::with_transport(Box::new(OfflineTransport));
    replay.engine.settings.remote_policy.master_enabled = true;
    replay.engine.settings.remote_policy.only_when_away = true;
    let mut event = AgentTaskEvent {
        id: "fixture-notify".into(), agent_id: "fixture-agent".into(), agent_name: "Fixture".into(),
        event_type: "attention".into(), timestamp: 100_000, message: None, detail: None,
        duration: 0.0, externally_delivered: true,
    };
    replay.engine.push_event(event.clone());
    std::thread::sleep(Duration::from_millis(20));
    assert!(replay.engine.notifier.recent().is_empty());
    event.externally_delivered = false;
    replay.engine.notify_outbound_with_presence(&event, crate::remote::PresenceSignals {
        screen_locked: false, display_asleep: false, idle_seconds: Some(1.0),
    });
    for _ in 0..100 {
        if !replay.engine.notifier.recent().is_empty() { break; }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(matches!(replay.engine.notifier.recent()[0].outcome, crate::notifier::Outcome::Suppressed { .. }));
}

#[test]
fn high_cpu_evidence_is_updated_even_during_attention() {
    let mut replay = Replay::new();
    replay.engine.settings.runaway_cpu_threshold = 80.0;
    replay.sample(100_000, true, Some(90.0), Some(Signal::Attention("fixture-attention".into(), "fixture".into())));
    assert_eq!(replay.engine.high_cpu_since.get(&replay.profile.id), Some(&100_000));
    replay.sample(101_000, true, Some(20.0), Some(Signal::Attention("fixture-attention".into(), "fixture".into())));
    assert!(!replay.engine.high_cpu_since.contains_key(&replay.profile.id));
}
