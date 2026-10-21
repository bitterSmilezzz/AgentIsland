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
        Self::from_engine(ActivityEngine::new(Settings::default(), rx))
    }

    /// 用一个已配置好的 engine 构造（测试需要注入去重存储时用）。
    fn from_engine(engine: ActivityEngine) -> Self {
        Self {
            engine,
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
    let event = replay
        .engine
        .latest_event
        .take()
        .expect("4 秒的实质工作应算一次完成");
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
    replay.sample_with_write(
        102_000,
        true,
        None,
        Some(Signal::Completed("flash".into())),
        write,
    );
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
    let stats =
        replay
            .engine
            .durations
            .stats("fixture-agent", crate::duration::DEFAULT_WINDOW_MS, 108_000);
    assert_eq!(stats.task_count, 1);
    assert!(
        (stats.total_work_time - 5.0).abs() < 1e-9,
        "{}",
        stats.total_work_time
    );
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
    assert!(!recent[0].accepted);
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
        memory_bytes: crate::health::MEMORY_OBSERVATION_FLOOR_BYTES + 1,
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
    let mut snap = alarming_snapshot(true);
    let start = 1_000_000i64;

    // 第一拍：两个条件都刚成立，只记起点
    engine.publish_guard_alerts(&[snap.clone()], start);
    assert!(engine.latest_event.is_none(), "刚成立不告警");

    // Dense memory observations without publishing; then two alerts in one tick.
    for tick in 1..20 {
        snap.memory_bytes += 128 * 1024 * 1024;
        engine
            .resilience
            .observe_memory(&[snap.clone()], start + tick * 30_000);
    }
    snap.memory_bytes += 128 * 1024 * 1024;
    engine.publish_guard_alerts(
        &[snap.clone()],
        start + 2 * crate::resilience::MEMORY_THRESHOLD_MS,
    );
    let first = engine.latest_event.clone().expect("队首应有一条");
    assert_eq!(first.event_type, "attention");
    assert!(
        first.message.as_deref().unwrap().contains("死锁"),
        "先发卡死那条"
    );
    assert_eq!(engine.pending_count(), 1, "第二条在排队，不许被顶掉");

    engine.ack_latest_event();
    let second = engine.latest_event.clone().expect("确认后应推下一条");
    assert!(second.message.as_deref().unwrap().contains("内存连续增长"));
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
    engine.publish_guard_alerts(
        &[snap.clone()],
        start + crate::resilience::MEMORY_THRESHOLD_MS,
    );
    assert!(engine.latest_event.is_none());
    assert_eq!(engine.pending_count(), 0);

    // 打开后从这一刻起**重新计时**——开关关着时守护根本没被调用，所以没有可继承的窗口。
    // 这与 Swift 一致（`if autoAlert { resilienceGuard.evaluate(...) }`）。
    engine.settings.auto_anomalies_alert = true;
    let reopened = start + 2 * crate::resilience::MEMORY_THRESHOLD_MS;
    engine.publish_guard_alerts(&[snap.clone()], reopened);
    assert!(engine.latest_event.is_none(), "打开的那一拍才开始计时");
    engine.publish_guard_alerts(
        &[snap.clone()],
        reopened + crate::resilience::MEMORY_THRESHOLD_MS,
    );
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
    assert!(
        engine.pending_count() <= 64,
        "待发队列必须有界：{}",
        engine.pending_count()
    );
    // 与恢复账本共用总容量 64，队首也是最近窗口的最早一条。
    assert_eq!(engine.recent_events().len(), 64);
    assert_eq!(engine.latest_event.as_ref().unwrap().id, "ev-136");
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
    assert_eq!(
        replay.sample(100_000, true, Some(20.0), None),
        ActivityLevel::Idle
    );
    assert_eq!(
        replay.sample(101_000, true, Some(24.0), None),
        ActivityLevel::Working
    );
    assert_eq!(
        replay.sample(102_000, true, Some(20.0), None),
        ActivityLevel::Working
    );
    assert_eq!(
        replay.sample(112_000, true, Some(20.0), None),
        ActivityLevel::Idle
    );
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
        id: "fixture-notify".into(),
        agent_id: "fixture-agent".into(),
        agent_name: "Fixture".into(),
        event_type: "attention".into(),
        timestamp: 100_000,
        message: None,
        detail: None,
        duration: 0.0,
        externally_delivered: true,
    };
    replay.engine.push_event(event.clone());
    std::thread::sleep(Duration::from_millis(20));
    assert!(replay.engine.notifier.recent().is_empty());
    event.externally_delivered = false;
    replay.engine.notify_outbound_with_presence(
        &event,
        crate::remote::PresenceSignals {
            screen_locked: false,
            display_asleep: false,
            idle_seconds: Some(1.0),
        },
    );
    for _ in 0..100 {
        if !replay.engine.notifier.recent().is_empty() {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(matches!(
        replay.engine.notifier.recent()[0].outcome,
        crate::notifier::Outcome::Suppressed { .. }
    ));
}

#[test]
fn high_cpu_evidence_is_updated_even_during_attention() {
    let mut replay = Replay::new();
    replay.engine.settings.runaway_cpu_threshold = 80.0;
    replay.sample(
        100_000,
        true,
        Some(90.0),
        Some(Signal::Attention(
            "fixture-attention".into(),
            "fixture".into(),
        )),
    );
    assert_eq!(
        replay.engine.high_cpu_since.get(&replay.profile.id),
        Some(&100_000)
    );
    replay.sample(
        101_000,
        true,
        Some(20.0),
        Some(Signal::Attention(
            "fixture-attention".into(),
            "fixture".into(),
        )),
    );
    assert!(!replay
        .engine
        .high_cpu_since
        .contains_key(&replay.profile.id));
}

#[test]
fn memory_growth_health_observation_is_independent_of_notifications() {
    let (_, rx) = mpsc::channel();
    let mut engine = ActivityEngine::new(Settings::default(), rx);
    engine.settings.auto_anomalies_alert = false;
    let mut snap = alarming_snapshot(true);
    snap.is_hung = Some(false);
    snap.cpu_percent = Some(2.0);
    for tick in 0..=20 {
        snap.memory_bytes =
            crate::health::MEMORY_OBSERVATION_FLOOR_BYTES + tick * 128 * 1024 * 1024;
        engine.publish_guard_alerts(&[snap.clone()], tick as i64 * 30_000);
    }
    assert!(engine.latest_event.is_none());
    assert_eq!(engine.pending_count(), 0);
    let report = crate::health::evaluate_with_memory_growth(
        &snap,
        engine.resilience.memory_growth(&snap.id),
    );
    assert_eq!(report.grade, crate::health::Grade::Attention);
    assert!(report.issues[0].contains("连续增长"));
}

#[test]
fn reminder_navigation_freezes_its_session_and_rejects_stale_events() {
    let (_, rx) = mpsc::channel();
    let mut engine = ActivityEngine::new(Settings::default(), rx);
    let profile = engine.profiles.iter().find(|p| p.id == "codex").unwrap();
    let first = crate::session_navigation::resolve(
        profile,
        Some("/fixture/rollout-019c6e27-e55b-73d1-87d8-4e01f1f75043.jsonl"),
    )
    .unwrap();
    let second = crate::session_navigation::resolve(
        profile,
        Some("/fixture/rollout-019c6e27-e55b-73d1-87d8-4e01f1f75044.jsonl"),
    )
    .unwrap();
    engine
        .session_navigation
        .insert("codex".into(), first.clone());
    let event = |id: &str| AgentTaskEvent {
        id: id.into(),
        agent_id: "codex".into(),
        agent_name: "Codex".into(),
        event_type: "attention".into(),
        timestamp: 0,
        message: Some("确认测试".into()),
        detail: None,
        duration: 0.0,
        externally_delivered: false,
    };
    engine.push_session_event(event("observed"));
    engine
        .session_navigation
        .insert("codex".into(), second.clone());
    assert_eq!(
        engine
            .navigation_target("codex", Some("observed"))
            .unwrap()
            .url,
        first.url
    );
    assert_eq!(
        engine.navigation_target("codex", None).unwrap().url,
        second.url
    );
    assert!(engine
        .navigation_target("traework", Some("observed"))
        .is_err());
    engine.push_event(event("external"));
    assert!(
        !engine
            .navigation_target("codex", Some("external"))
            .unwrap()
            .exact_session
    );
    engine.ack_latest_event();
    assert!(engine.navigation_target("codex", Some("observed")).is_err());
    assert!(engine
        .event_navigation
        .values()
        .all(|targets| !targets.contains_key("observed")));
    for i in 0..100 {
        engine.push_session_event(event(&format!("queued-{i}")));
    }
    assert!(
        engine
            .event_navigation
            .values()
            .map(std::collections::HashMap::len)
            .sum::<usize>()
            <= 65
    );
    assert!(engine.navigation_target("codex", Some("queued-0")).is_err());
}

// Sampling gaps and restart deduplication use synthetic signals and real temporary storage.

#[test]
fn a_sampling_gap_does_not_lose_a_new_attention_event() {
    // 采样间隙：两次采样之间隔了很久（进程没重启，只是这一轮没采到）。
    // 新指纹必须仍然推得出来——不能因为"上次推过东西"就把后来的也吞了。
    let mut replay = Replay::new();
    replay
        .sample(100_000, true, None, Some(Signal::Attention("ask-a".into(), "A?".into())));
    assert_eq!(
        replay.engine.latest_event.take().unwrap().event_type,
        "attention",
        "第一个问题应送达"
    );

    // 间隙：模拟漏采/停采 10 分钟，然后出现一个**不同**的问题。
    replay.sample(
        700_000,
        true,
        None,
        Some(Signal::Attention("ask-b".into(), "B?".into())),
    );
    let event = replay
        .engine
        .latest_event
        .take()
        .expect("采样间隙后的新问题必须仍能送达，不得被上一次告警吞掉");
    assert_eq!(event.event_type, "attention");
    assert_eq!(event.id, "ask-b");
}

#[test]
fn an_identical_fingerprint_survives_many_sampling_gaps_without_repeating() {
    // 反方向：同指纹跨多次间隙不得重报。这条与 `:310` 合起来构成"不重报也不漏报"。
    let mut replay = Replay::new();
    let ask = Some(Signal::Attention("stuck-question".into(), "Confirm?".into()));
    replay.sample(100_000, true, None, ask.clone());
    assert!(replay.engine.latest_event.take().is_some());

    for now in [160_000, 400_000, 900_000, 1_800_000] {
        replay.sample(now, true, None, ask.clone());
        assert!(
            replay.engine.latest_event.is_none(),
            "同指纹跨间隙不得重报（第 {now} 毫秒）"
        );
    }
}

struct DedupSandbox(std::path::PathBuf);

impl DedupSandbox {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("engine-dedup-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        Self(root)
    }

    fn path(&self) -> std::path::PathBuf {
        self.0.join("state.json")
    }

    fn replay(&self) -> Replay {
        let mut replay = Replay::new();
        replay.engine.disable_outbound();
        replay.engine.enable_dedup_persistence(self.path());
        replay
    }
}

impl Drop for DedupSandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn completion(replay: &mut Replay, start: i64, raw: &str) {
    replay.sample(start, true, Some(20.0), None);
    replay.sample_with_write(
        start + 4_000, true, None, Some(Signal::Completed(raw.into())),
        Some(SystemTime::UNIX_EPOCH + Duration::from_millis((start + 4_000) as u64)),
    );
}

#[test]
fn real_disk_restart_silences_old_attention_and_completion_but_delivers_new_events() {
    let sandbox = DedupSandbox::new();
    let ask = || Some(Signal::Attention("ask-restart".into(), "Confirm?".into()));
    let mut first = sandbox.replay();
    first.sample(100_000, true, None, ask());
    assert_eq!(first.engine.latest_event.take().unwrap().event_type, "attention");
    completion(&mut first, 110_000, "done-restart");
    assert_eq!(first.engine.latest_event.take().unwrap().event_type, "completed");
    drop(first);

    let loaded = crate::dedup::Store::open_at(sandbox.path());
    assert!(loaded.attention_seen("fixture-agent", "ask-restart"));
    assert!(loaded.completion_seen("fixture-agent", "done-restart"));
    let mut second = sandbox.replay();
    second.sample(120_000, true, None, ask());
    completion(&mut second, 130_000, "done-restart");
    assert!(second.engine.latest_event.is_none());
    assert_eq!(second.engine.durations.stats("fixture-agent", crate::duration::DEFAULT_WINDOW_MS, 134_000).task_count, 0);
    assert!(!second.engine.work_started_at.contains_key("fixture-agent"));

    second.sample(140_000, true, None, Some(Signal::Attention("ask-new".into(), "New?".into())));
    assert_eq!(second.engine.latest_event.take().unwrap().id, "ask-new");
    completion(&mut second, 150_000, "done-new");
    assert_eq!(second.engine.latest_event.take().unwrap().id, "done-new");
}

#[test]
fn short_completion_is_not_persisted_and_can_be_announced_after_real_work() {
    let sandbox = DedupSandbox::new();
    let mut first = sandbox.replay();
    first.sample(100_000, true, Some(20.0), None);
    first.sample_with_write(101_000, true, None, Some(Signal::Completed("short".into())),
        Some(SystemTime::UNIX_EPOCH + Duration::from_millis(101_000)));
    assert!(first.engine.latest_event.is_none());
    assert!(!sandbox.path().exists());
    assert!(!first.engine.dedup.completion_seen("fixture-agent", "short"));
    // Persist a different real event so the restart must read a real file.
    first.sample(102_000, true, None, Some(Signal::Attention("ask".into(), "Confirm?".into())));
    first.engine.latest_event.take();
    let loaded = crate::dedup::Store::open_at(sandbox.path());
    assert!(!loaded.completion_seen("fixture-agent", "short"));
    let mut second = sandbox.replay();
    completion(&mut second, 110_000, "short");
    assert_eq!(second.engine.latest_event.take().unwrap().id, "short");
}

#[test]
fn same_raw_attention_and_completion_from_different_agents_are_independent() {
    let sandbox = DedupSandbox::new();
    let mut replay = sandbox.replay();
    for (index, agent) in ["agent-a", "agent-b"].iter().enumerate() {
        replay.profile.id = (*agent).into();
        let start = 100_000 + index as i64 * 20_000;
        replay.sample(start, true, None, Some(Signal::Attention("same".into(), "Confirm?".into())));
        assert_eq!(replay.engine.latest_event.take().unwrap().agent_id, *agent);
        completion(&mut replay, start + 1_000, "same");
        assert_eq!(replay.engine.latest_event.take().unwrap().agent_id, *agent);
    }
    let loaded = crate::dedup::Store::open_at(sandbox.path());
    for agent in ["agent-a", "agent-b"] {
        assert!(loaded.attention_seen(agent, "same"));
        assert!(loaded.completion_seen(agent, "same"));
    }
}

#[test]
fn a_persist_failure_does_not_interrupt_monitoring_or_lose_in_memory_dedup() {
    let sandbox = DedupSandbox::new();
    let mut replay = sandbox.replay();
    // Introduce a non-directory parent after successful open to force real persist failure.
    std::fs::remove_dir(&sandbox.0).unwrap();
    std::fs::write(&sandbox.0, b"blocker").unwrap();
    replay.sample(100_000, true, None, Some(Signal::Attention("ask".into(), "Confirm?".into())));
    assert_eq!(replay.engine.latest_event.take().unwrap().id, "ask");
    replay.sample(101_000, true, None, Some(Signal::Attention("ask".into(), "Confirm?".into())));
    assert!(replay.engine.latest_event.is_none());
    completion(&mut replay, 110_000, "done");
    assert_eq!(replay.engine.latest_event.take().unwrap().id, "done");
    replay.sample(120_000, true, None, Some(Signal::Attention("next".into(), "Next?".into())));
    assert_eq!(replay.engine.latest_event.take().unwrap().id, "next");
    assert_eq!(std::fs::read(&sandbox.0).unwrap(), b"blocker");
    std::fs::remove_file(&sandbox.0).unwrap();
}

#[test]
fn a_clock_rewind_does_not_re_report_a_fingerprint_already_announced() {
    // 时钟回拨（睡眠唤醒、NTP 校正）是 SUP-02 点名的风险：不能把"已经报过的事"
    // 当成新事再报一遍。指纹本身不含时间戳，所以回拨不该影响去重判定。
    let mut replay = Replay::new();
    let ask = Some(Signal::Attention("ask-rewind".into(), "Confirm?".into()));
    replay.sample(100_000, true, None, ask.clone());
    assert!(replay.engine.latest_event.take().is_some());

    // 时钟大幅回拨到更早的时刻，同指纹仍在
    replay.sample(20_000, true, None, ask.clone());
    assert!(
        replay.engine.latest_event.is_none(),
        "时钟回拨不得让已报过的指纹重报"
    );

    // 回拨之后出现的新指纹仍要能送达
    replay.sample(21_000, true, None, Some(Signal::Attention("ask-after-rewind".into(), "B?".into())));
    assert_eq!(
        replay.engine.latest_event.take().map(|e| e.id),
        Some("ask-after-rewind".to_string()),
        "时钟回拨后到来的新事件不得被吞"
    );
}

#[test]
fn a_second_distinct_question_of_the_same_length_is_not_swallowed() {
    // 与 session.rs:1428 的解析层守护呼应，这里钉 engine 层：
    // 两条**指纹不同**但消息长度相同的告警，必须都活下来（不得靠内容长度去重）。
    let mut replay = Replay::new();
    replay.sample(
        100_000,
        true,
        None,
        Some(Signal::Attention("fp-1".into(), "确认继续吗？".into())),
    );
    let first = replay.engine.latest_event.take().expect("第一条应送达");
    assert_eq!(first.id, "fp-1");

    // 同样字数、不同指纹 → 必须作为独立事件入队
    replay.sample(
        101_000,
        true,
        None,
        Some(Signal::Attention("fp-2".into(), "确认继续吗？".into())),
    );
    let second = replay
        .engine
        .latest_event
        .take()
        .expect("等长但不同指纹的第二个问题必须独立送达");
    assert_eq!(second.id, "fp-2");
}

#[test]
fn concurrent_attention_and_completion_in_one_tick_both_survive_the_queue() {
    // 一拍 attention、下一拍 completed：两条都得活。这条同时钉住
    // engine.rs:87 记录过的 bug 形状——`push_event` 曾是**覆盖** `latest_event`，
    // 同一拍两条告警只活一条。
    let mut replay = Replay::new();
    // 先工作够 3.5 秒门槛，否则 completed 按设计不推
    replay.sample(100_000, true, Some(20.0), None);
    assert_eq!(replay.engine.snapshots[0].level, ActivityLevel::Working);

    let write = Some(SystemTime::UNIX_EPOCH + Duration::from_millis(110_000));
    replay.sample(
        105_000,
        true,
        None,
        Some(Signal::Attention("fp-attn".into(), "确认？".into())),
    );
    let attn = replay.engine.latest_event.take().expect("attention 应入队");
    assert_eq!(attn.event_type, "attention");

    // completed 单独一拍。注意 attention 那一拍已经清掉了 `work_started_at`
    // （engine.rs:635），所以这里要重新攒够工作时长，否则 `seconds < 3.5`
    // 会让 completed 按设计不入队（engine.rs:671-677）。
    replay.sample(115_000, true, Some(20.0), None);
    let done_write = Some(SystemTime::UNIX_EPOCH + Duration::from_millis(121_000));
    replay.sample_with_write(
        121_000,
        true,
        None,
        Some(Signal::Completed("fp-done".into())),
        done_write,
    );
    let done = replay.engine.latest_event.take().expect("completed 应入队");
    assert_eq!(done.event_type, "completed");
    // 两条不是同一条：attention 没被 completed 顶掉
    assert_ne!(attn.id, done.id);
}

#[test]
fn an_attention_tick_clears_the_work_start_anchor_so_a_bare_completion_is_not_a_task() {
    // 钉一个不直观但已核实的实现语义：attention 分支会 `work_started_at.remove(&key)`
    // （engine.rs:635）。于是紧接着的 completed 即使判定成功，也因 `seconds = 0`
    // 低于 MIN_TASK_SECONDS 而不推事件——**不得**把"确认一下就完成"算成一次任务。
    // 若将来要保留 attention 期间的工作时长，请连同此断言一起改。
    let mut replay = Replay::new();
    replay.sample(100_000, true, Some(20.0), None);
    let write = Some(SystemTime::UNIX_EPOCH + Duration::from_millis(105_000));
    replay.sample(
        104_000,
        true,
        None,
        Some(Signal::Attention("fp-attn".into(), "确认？".into())),
    );
    assert!(replay.engine.latest_event.take().is_some());

    // 紧接 completed：判定层面成立，但时长不足门槛
    let level = replay.sample_with_write(
        105_000,
        true,
        None,
        Some(Signal::Completed("fp-done".into())),
        write,
    );
    assert_eq!(level, ActivityLevel::Completed, "状态层仍是完成");
    assert!(
        replay.engine.latest_event.is_none(),
        "但 attention 已清掉工作起点，0 秒不得算一次任务完成（engine.rs:671-677）"
    );
}

#[test]
fn a_repeated_completion_is_silent_even_with_enough_new_work_duration() {
    // Rebuild a qualifying duration so this assertion actually exercises event deduplication.
    let mut replay = Replay::new();
    replay.sample(100_000, true, Some(20.0), None);

    let done = Some(Signal::Completed("done-fp-map".into()));
    let write = || Some(SystemTime::UNIX_EPOCH + Duration::from_millis(104_000));

    // 第一次：4 秒实质工作，正常推出
    replay.sample_with_write(104_000, true, None, done.clone(), write());
    let first = replay.engine.latest_event.take().expect("首次完成应送达");
    assert_eq!(first.event_type, "completed");
    assert!((first.duration - 4.0).abs() < 1e-9);

    // The same completed identity remains silent after another qualifying work interval.
    replay.sample(105_000, true, Some(20.0), None);
    replay.sample_with_write(112_000, true, None, done.clone(), write());
    assert!(
        replay.engine.latest_event.is_none(),
        "同一完成指纹不得二次推送"
    );

    // 换一个指纹就该推得出来——证明上面的静默不是因为 completed 分支坏了
    replay.sample(113_000, true, Some(20.0), None);
    replay.sample_with_write(
        121_000,
        true,
        None,
        Some(Signal::Completed("done-fp-map-2".into())),
        Some(SystemTime::UNIX_EPOCH + Duration::from_millis(121_000)),
    );
    assert_eq!(
        replay.engine.latest_event.take().map(|e| e.event_type),
        Some("completed".to_string()),
        "新指纹必须能推出完成，否则上面的断言只是 completed 分支整体失效"
    );


}

    /// BUG-008：两个 agent 产出**同一原始 id** 时，导航绑定必须各自独立。
    ///
    /// 账本侧的内部分身份一直是 (agent, kind, raw id) 分域的，并写明「会话信号可以跨
    /// 这些作用域复用同一原始 id」；导航侧原先只用事件 id 单键，后写入的会覆盖前一个，
    /// 于是 `navigation_target(agent, event)` 可能返回**另一个 agent 的会话地址**。
    ///
    /// 断言用**冻结语义**而不是「等于当前 session 目标」：推送后再把两个 agent 的
    /// session 目标都改掉。若事件绑定真的按 agent 分域存住了，读回来应是推送那一刻的值；
    /// 若绑定被另一条覆盖/丢失，读取会退回到**已改动**的 session 目标——两者可区分。
    #[test]
    fn the_same_raw_event_id_from_two_agents_never_shares_one_navigation_target() {
        let (_, rx) = mpsc::channel();
        let mut engine = ActivityEngine::new(Settings::default(), rx);
        let target = |agent: &str, session: &str| crate::session_navigation::Target {
            agent_id: agent.into(),
            exact_session: true,
            label: agent.into(),
            hint: String::new(),
            bundle_id: None,
            url: Some(format!("{agent}://threads/{session}")),
        };
        for agent in ["codex", "claude"] {
            assert!(
                engine.profiles.iter().any(|p| p.id == agent),
                "夹具假设：{agent} 必须是内置档案"
            );
            engine
                .session_navigation
                .insert(agent.into(), target(agent, "at-push"));
        }
        let event = |agent: &str, id: &str| AgentTaskEvent {
            id: id.into(),
            agent_id: agent.into(),
            agent_name: agent.into(),
            event_type: "attention".into(),
            timestamp: 0,
            message: Some("确认测试".into()),
            detail: None,
            duration: 0.0,
            externally_delivered: false,
        };
        engine.push_session_event(event("codex", "same-id"));
        engine.push_session_event(event("claude", "same-id"));
        // 推送之后把 session 目标改掉：冻结的事件绑定必须仍指向 at-push
        for agent in ["codex", "claude"] {
            engine
                .session_navigation
                .insert(agent.into(), target(agent, "moved-after"));
        }
        assert_eq!(
            engine
                .navigation_target("codex", Some("same-id"))
                .unwrap()
                .url
                .as_deref(),
            Some("codex://threads/at-push"),
            "codex 的事件绑定被顶掉了（或根本没存住）"
        );
        assert_eq!(
            engine
                .navigation_target("claude", Some("same-id"))
                .unwrap()
                .url
                .as_deref(),
            Some("claude://threads/at-push"),
            "claude 的同一原始 id 事件读到了别的目标：这正是 BUG-008 的串号"
        );
        // 队首确认只摘掉自己那条：另一条绑定的导航必须还在
        engine.ack_latest_event();
        assert!(engine.navigation_target("codex", Some("same-id")).is_err());
        assert_eq!(
            engine
                .navigation_target("claude", Some("same-id"))
                .unwrap()
                .url
                .as_deref(),
            Some("claude://threads/at-push")
        );
    }
