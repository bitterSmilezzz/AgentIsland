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
            process_running: running,
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
