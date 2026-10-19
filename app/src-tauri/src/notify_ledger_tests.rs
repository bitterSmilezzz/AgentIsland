use super::*;
use crate::engine::{ActivityEngine, HookScheduler};
use crate::hook_event::{HookArrival, HookEvent};
use crate::settings::Settings;
use std::sync::mpsc;
use std::time::{Duration, Instant};

pub(super) struct Sandbox(pub(super) PathBuf);
impl Sandbox {
    pub(super) fn new() -> Self {
        let path = std::env::temp_dir().join(format!("notify-ledger-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    pub(super) fn path(&self) -> PathBuf {
        self.0.join("ledger.json")
    }
}
impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub(super) fn event(n: usize) -> AgentTaskEvent {
    AgentTaskEvent {
        id: format!("server-{n}"),
        agent_id: format!("agent-{n}"),
        agent_name: format!("Agent {n}"),
        event_type: "attention".into(),
        timestamp: 100_123,
        message: Some(format!("message-{n}")),
        detail: Some(format!("detail-{n}")),
        duration: 0.0,
        externally_delivered: true,
    }
}
fn engine() -> ActivityEngine {
    let (_, rx) = mpsc::channel();
    let mut engine = ActivityEngine::new(Settings::default(), rx);
    engine.disable_outbound();
    engine
}
fn hook(engine: &mut ActivityEngine, event: AgentTaskEvent, id: Option<&str>) {
    engine.push_hook_event(HookEvent {
        client_id: id.map(str::to_string),
        event,
        received_at: Instant::now(),
    });
}

#[test]
fn same_id_suppresses_enqueue_and_sample_but_different_id_preserves_content() {
    let mut engine = engine();
    hook(&mut engine, event(1), Some("client-a"));
    let mut replay = event(1);
    replay.id = "another-server-id".into();
    hook(&mut engine, replay.clone(), Some("client-a"));
    assert_eq!(engine.recent_events().len(), 1);
    assert_eq!(engine.state().notify_duplicates, 1);
    assert_eq!(engine.notify_latency.samples(), 1);
    hook(&mut engine, replay, Some("client-b"));
    assert_eq!(engine.recent_events().len(), 2);
    assert_eq!(engine.notify_latency.samples(), 2);
    assert_eq!(engine.state().notify_duplicates, 1);
    assert!(engine.notifier.recent().is_empty());
}

#[test]
fn fallback_is_same_second_only_and_uses_length_prefixed_fields() {
    let mut engine = engine();
    let first = event(1);
    hook(&mut engine, first.clone(), None);
    let mut replay = first.clone();
    replay.id = "second-server-id".into();
    replay.timestamp = 100_999;
    hook(&mut engine, replay.clone(), None);
    assert_eq!(engine.recent_events().len(), 1);
    replay.timestamp = 101_000;
    hook(&mut engine, replay, None);
    assert_eq!(engine.recent_events().len(), 2);
    assert_eq!(engine.state().notify_duplicates, 1);
    assert_eq!(engine.notify_latency.samples(), 2);
    let mut a = event(2);
    a.message = Some("a|b".into());
    a.detail = Some("c".into());
    let mut b = a.clone();
    b.message = Some("a".into());
    b.detail = Some("b|c".into());
    assert_ne!(identity(&a, None), identity(&b, None));
    // Exact encoding contract, independent of the production identity function.
    let mut digest = Sha256::new();
    for part in ["id", "agent-2", "attention", "client-a"] {
        digest.update((part.len() as u64).to_be_bytes());
        digest.update(part.as_bytes());
    }
    assert_eq!(
        identity(&a, Some("client-a")),
        format!("{:x}", digest.finalize())
    );
}

#[test]
fn internal_push_event_also_deduplicates_without_hook_samples() {
    let mut engine = engine();
    engine.push_event(event(1));
    engine.push_event(event(1));
    assert_eq!(engine.recent_events().len(), 1);
    assert_eq!(engine.state().notify_duplicates, 1);
    assert_eq!(engine.notify_latency.samples(), 0);
}

#[test]
fn disk_rebuild_retains_identity_and_unacknowledged_queue_without_resending() {
    let sandbox = Sandbox::new();
    let path = sandbox.path();
    let mut first = engine();
    first.enable_notify_ledger_persistence(&path);
    assert!(!path.exists(), "opening is read-only");
    hook(&mut first, event(1), Some("private-client-identity"));
    hook(&mut first, event(2), Some("client-b"));
    first.notify_ledger.flush();
    let raw = fs::read(&path).unwrap();
    assert!(!String::from_utf8_lossy(&raw).contains("private-client-identity"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    drop(first);
    let mut second = engine();
    second.enable_notify_ledger_persistence(&path);
    assert_eq!(
        second
            .recent_events()
            .iter()
            .map(|e| e.id.as_str())
            .collect::<Vec<_>>(),
        [stored_internal_id(&event(1)), stored_internal_id(&event(2))].iter().map(String::as_str).collect::<Vec<_>>()
    );
    assert_eq!(second.notify_latency.samples(), 0);
    assert!(second.notifier.recent().is_empty());
    // 恢复条数是界面「本次启动恢复」那一格的唯一来源，且必须是**恢复到的条数**
    // 而不是「当前队列长度」：两者在恢复后立刻又收到新事件时就会分叉。
    assert_eq!(second.state().notify_restored, 2, "两条未确认提醒应在重启后恢复");
    hook(&mut second, event(1), Some("private-client-identity"));
    assert_eq!(second.recent_events().len(), 2);
    assert_eq!(second.state().notify_duplicates, 1);
    assert_eq!(
        second.state().notify_restored,
        2,
        "恢复条数是启动那一刻的快照，之后进来的事件不得改写它"
    );
    assert_eq!(second.state().notify_unacked, 2);
    assert_eq!(second.notify_latency.samples(), 0);
}

#[test]
fn acknowledged_events_do_not_revive_and_idempotency_survives_ack() {
    let sandbox = Sandbox::new();
    let path = sandbox.path();
    let mut first = engine();
    first.enable_notify_ledger_persistence(&path);
    hook(&mut first, event(1), Some("client-a"));
    hook(&mut first, event(2), Some("client-b"));
    first.ack_latest_event();
    first.notify_ledger.flush();
    drop(first);
    let mut second = engine();
    second.enable_notify_ledger_persistence(&path);
    assert_eq!(second.recent_events().len(), 1);
    assert_eq!(second.latest_event.as_ref().unwrap().id, stored_internal_id(&event(2)));
    hook(&mut second, event(1), Some("client-a"));
    assert_eq!(second.recent_events().len(), 1);
    second.ack_latest_event();
    drop(second); // graceful destruction drains latest ack
    let mut third = engine();
    third.enable_notify_ledger_persistence(&path);
    assert!(third.recent_events().is_empty());
}

#[test]
fn corrupt_future_oversized_and_invalid_payload_preserve_bytes_and_block_forever() {
    for original in [
        b"{broken".to_vec(),
        br#"{"schema_version":999,"identities":[],"pending":[]}"#.to_vec(),
        vec![b'x'; MAX_BYTES as usize + 1],
        br#"{"schema_version":1,"identities":["plain"],"pending":[]}"#.to_vec(),
    ] {
        let sandbox = Sandbox::new();
        let path = sandbox.path();
        fs::write(&path, &original).unwrap();
        let mut ledger = Ledger::open_at(&path);
        assert!(ledger.accept(&event(1), Some("one")));
        ledger.flush();
        assert_eq!(fs::read(&path).unwrap(), original);
        fs::remove_file(&path).unwrap(); // repairing the external path must not re-enable writes
        assert!(ledger.accept(&event(2), Some("two")));
        ledger.flush();
        assert!(!path.exists());
        assert_eq!(ledger.pending.len(), 2);
    }
}

#[test]
fn external_edits_and_stale_absent_writer_never_overwrite() {
    for present in [false, true] {
        let sandbox = Sandbox::new();
        let path = sandbox.path();
        if present {
            let mut seed = Ledger::open_at(&path);
            seed.accept(&event(0), Some("seed"));
            seed.flush();
        }
        let mut winner = Ledger::open_at(&path);
        let mut stale = Ledger::open_at(&path);
        winner.accept(&event(1), Some("winner"));
        winner.flush();
        let original = fs::read(&path).unwrap();
        stale.accept(&event(2), Some("stale"));
        stale.flush();
        assert_eq!(fs::read(&path).unwrap(), original);
        fs::write(&path, b"external broken bytes").unwrap();
        winner.accept(&event(3), Some("next"));
        winner.flush();
        assert_eq!(fs::read(&path).unwrap(), b"external broken bytes");
        fs::write(&path, &original).unwrap();
        winner.accept(&event(4), Some("later"));
        winner.flush();
        assert_eq!(
            fs::read(&path).unwrap(),
            original,
            "blocked remains permanent"
        );
    }
}

#[cfg(unix)]
#[test]
fn symlinks_and_non_regular_files_remain_untouched() {
    use std::os::unix::fs::symlink;
    let sandbox = Sandbox::new();
    let path = sandbox.path();
    let victim = sandbox.0.join("victim");
    fs::write(&victim, b"private original").unwrap();
    symlink(&victim, &path).unwrap();
    let mut ledger = Ledger::open_at(&path);
    ledger.accept(&event(1), None);
    ledger.flush();
    assert!(fs::symlink_metadata(&path)
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(fs::read(&victim).unwrap(), b"private original");
    drop(ledger);
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    let mut ledger = Ledger::open_at(&path);
    ledger.accept(&event(2), None);
    ledger.flush();
    assert!(path.is_dir());
}

#[test]
fn invalid_http_ids_return_400_without_send_wake_or_disk_write() {
    let sandbox = Sandbox::new();
    let path = sandbox.path();
    let (tx, rx) = mpsc::channel();
    let (wake, wake_rx) = mpsc::sync_channel(1);
    let mut engine = ActivityEngine::new(Settings::default(), rx);
    engine.disable_outbound();
    engine.enable_notify_ledger_persistence(&path);
    for id in [
        serde_json::json!(""),
        serde_json::json!("x".repeat(129)),
        serde_json::json!("control\n"),
        serde_json::json!("a/b"),
        serde_json::json!("a\\b"),
        serde_json::json!("a\"b"),
        serde_json::json!("非ASCII"),
        serde_json::Value::Null,
        serde_json::json!(123),
    ] {
        for untrusted in [false, true] {
            let body = serde_json::json!({"agent":"fixture", "id":id}).to_string();
            assert_eq!(
                crate::webhook::notify_route(&body, untrusted, &tx, &wake, HookArrival::before(0))
                    .0,
                400
            );
        }
    }
    assert_eq!(engine.consume_hook_events(), 0);
    assert!(wake_rx.try_recv().is_err());
    assert!(engine.recent_events().is_empty());
    engine.notify_ledger.flush();
    assert!(!path.exists());
    assert!(valid_id(&"x".repeat(128)));
    assert!(valid_id("safe-id: 1|2"));
}

#[test]
fn one_200_request_burst_is_accepted_once_then_capacity_retains_latest() {
    let sandbox = Sandbox::new();
    let path = sandbox.path();
    let (tx, rx) = mpsc::channel();
    let (wake, _wake_rx) = mpsc::sync_channel(1);
    let mut engine = ActivityEngine::new(Settings::default(), rx);
    engine.disable_outbound();
    engine.enable_notify_ledger_persistence(&path);
    for _ in 0..2 {
        for n in 0..200 {
            let body = serde_json::json!({"agent":format!("agent-{n}"),"message":format!("message-{n}"),"id":format!("id-{n}")}).to_string();
            assert_eq!(
                crate::webhook::notify_route(&body, false, &tx, &wake, HookArrival::before(0)).0,
                200
            );
        }
    }
    assert_eq!(engine.consume_hook_events(), 400);
    assert_eq!(engine.notify_latency.samples(), 200);
    assert_eq!(engine.state().notify_duplicates, 200);
    assert_eq!(engine.notify_ledger.identities.len(), 200);
    let retained = engine.recent_events();
    assert_eq!(retained.len(), 64);
    for (n, event) in (136..200).zip(retained) {
        assert_eq!(event.agent_id, format!("agent-{n}"));
        assert_eq!(event.message, Some(format!("message-{n}")));
    }
    engine.notify_ledger.flush();
    assert_eq!(Ledger::open_at(&path).pending.len(), 64);
    eprintln!(
        "one 200-request burst plus replay: {:?}",
        engine.notify_latency.summary()
    );
    assert_eq!(engine.notify_latency.summary().meets_target, Some(true));
}

#[test]
fn same_raw_internal_id_across_agent_or_kind_survives_ack_and_restart() {
    let sandbox = Sandbox::new();
    let path = sandbox.path();
    let mut first = engine();
    first.enable_notify_ledger_persistence(&path);
    let mut a = event(1);
    a.externally_delivered = false;
    a.id = "/fixture/private/session-fingerprint".into();
    let mut b = a.clone();
    b.event_type = "completed".into();
    let mut c = a.clone();
    c.agent_id = "other-agent".into();
    first.push_event(a);
    first.push_event(b);
    first.push_event(c);
    first.ack_latest_event();
    first.notify_ledger.flush();
    drop(first);
    let restored = Ledger::open_at(&path).restored_events();
    assert_eq!(restored.len(), 2);
    assert_eq!(restored[0].event_type, "completed");
    assert_eq!(restored[1].agent_id, "external");
    assert!(!String::from_utf8_lossy(&fs::read(&path).unwrap())
        .contains("/fixture/private/session-fingerprint"));
    let mut second = engine();
    second.enable_notify_ledger_persistence(&path);
    second.ack_latest_event();
    second.ack_latest_event();
    second.notify_ledger.flush();
    assert!(Ledger::open_at(&path).pending.is_empty());
}

#[test]
fn real_http_unknown_agent_lands_as_external_and_never_as_arbitrary_name() {
    use std::io::Write;
    let sandbox = Sandbox::new();
    let path = sandbox.path();
    let http = crate::webhook::LocalEventServer::bind_http("127.0.0.1:0").unwrap();
    let addr = http.server_addr().to_ip().unwrap();
    let (tx, rx) = mpsc::channel();
    let (wake, wake_rx) = mpsc::sync_channel(1);
    let mut activity = ActivityEngine::new(Settings::default(), rx);
    activity.disable_outbound();
    activity.enable_notify_ledger_persistence(&path);
    let shared = Arc::new(Mutex::new(activity));
    let server = crate::webhook::LocalEventServer::serve_bound(http, tx, shared.clone(), wake);
    // 设计文档把「未知 HTTP agent 归为 external」当作对外承诺；此前只有内部入口有正例。
    let arbitrary = "totally-made-up-agent-name";
    let body = serde_json::json!({
        "agent": arbitrary,
        "event": "attention",
        "message": "private sentinel 私密正文",
        "id": "unknown-agent-id",
    })
    .to_string();
    let mut stream = std::net::TcpStream::connect_timeout(&addr, Duration::from_secs(1)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    write!(stream, "POST /notify HTTP/1.1\r\nHost: {addr}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    assert!(response.starts_with("HTTP/1.1 200 "), "{response}");
    let state = HookScheduler::new(wake_rx)
        .wait_and_consume(&shared, Instant::now() + Duration::from_secs(2), || {})
        .unwrap();
    assert_eq!(state.notify_unacked, 1, "单条事件应计入未确认（含队首）");
    assert!(state.latest_event.is_some());
    shared.lock().unwrap().notify_ledger.flush();
    drop(server);
    let restored = Ledger::open_at(&path).restored_events();
    assert_eq!(restored.len(), 1);
    assert_eq!(
        restored[0].agent_id, "external",
        "未知 HTTP agent 必须归一为 external"
    );
    let raw = String::from_utf8_lossy(&fs::read(&path).unwrap()).to_string();
    assert!(!raw.contains(arbitrary), "任意 agent 名称不得落盘");
    assert!(!raw.contains("unknown-agent-id"), "原始 id 不得落盘");
    assert!(!raw.contains("private sentinel"), "私密正文不得落盘");
}

#[test]
fn real_http_id_validation_and_replay_use_the_production_consumer() {
    use std::io::Write;
    let sandbox = Sandbox::new();
    let path = sandbox.path();
    let http = crate::webhook::LocalEventServer::bind_http("127.0.0.1:0").unwrap();
    let addr = http.server_addr().to_ip().unwrap();
    let (tx, rx) = mpsc::channel();
    let (wake, wake_rx) = mpsc::sync_channel(1);
    let mut activity = ActivityEngine::new(Settings::default(), rx);
    activity.disable_outbound();
    activity.enable_notify_ledger_persistence(&path);
    let shared = Arc::new(Mutex::new(activity));
    let server = crate::webhook::LocalEventServer::serve_bound(http, tx, shared.clone(), wake);
    for (route, id, expected) in [
        ("/notify", "bad/path", 400),
        ("/event", "bad\\path", 400),
        ("/notify", "same-id", 200),
        ("/event", "same-id", 200),
    ] {
        let body = serde_json::json!({"agent":"fixture", "message":"same", "id":id}).to_string();
        let mut stream =
            std::net::TcpStream::connect_timeout(&addr, Duration::from_secs(1)).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        write!(stream, "POST {route} HTTP/1.1\r\nHost: {addr}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        assert!(
            response.starts_with(&format!("HTTP/1.1 {expected} ")),
            "{response}"
        );
        if expected == 400 {
            assert!(!path.exists());
        }
    }
    let state = HookScheduler::new(wake_rx)
        .wait_and_consume(&shared, Instant::now() + Duration::from_secs(2), || {})
        .unwrap();
    assert_eq!(state.notify_duplicates, 1);
    assert_eq!(state.notify_latency.samples, 1);
    assert_eq!(state.pending_events, 0);
    assert!(state.latest_event.is_some());
    shared.lock().unwrap().notify_ledger.flush();
    drop(server);
}

/// Same explicit id, two different agents, over a real socket and the production
/// consumer. The four requests above all use agent `fixture`, so they only prove
/// "same agent + same id is a duplicate"; that identity is scoped by agent (and by
/// normalized kind) was so far proved only at the `Ledger` unit level. Here both
/// requests must be accepted with 200 and **both** must enqueue, because a same-id
/// retry from another agent is a different event, not a replay.
#[test]
fn real_http_same_id_across_agents_both_enqueue() {
    use std::io::Write;
    let sandbox = Sandbox::new();
    let path = sandbox.path();
    let http = crate::webhook::LocalEventServer::bind_http("127.0.0.1:0").unwrap();
    let addr = http.server_addr().to_ip().unwrap();
    let (tx, rx) = mpsc::channel();
    let (wake, wake_rx) = mpsc::sync_channel(1);
    let mut activity = ActivityEngine::new(Settings::default(), rx);
    activity.disable_outbound();
    activity.enable_notify_ledger_persistence(&path);
    let shared = Arc::new(Mutex::new(activity));
    let server = crate::webhook::LocalEventServer::serve_bound(http, tx, shared.clone(), wake);
    for agent in ["alpha", "beta"] {
        let body =
            serde_json::json!({"agent":agent, "id":"shared-id", "message":"same body"}).to_string();
        let mut stream =
            std::net::TcpStream::connect_timeout(&addr, Duration::from_secs(1)).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        write!(stream, "POST /notify HTTP/1.1\r\nHost: {addr}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        assert!(response.starts_with("HTTP/1.1 200 "), "{response}");
        assert!(
            response.contains("accepted"),
            "受理语义不得宣称已投递: {response}"
        );
    }
    let scheduler = HookScheduler::new(wake_rx);
    let deadline = Instant::now() + Duration::from_secs(3);
    let state = loop {
        let state = scheduler
            .wait_and_consume(&shared, deadline, || {})
            .expect("production consumer timed out");
        if state.notify_latency.samples >= 2 {
            break state;
        }
    };
    assert_eq!(state.notify_duplicates, 0, "跨 agent 同 id 不是重复");
    assert_eq!(state.notify_latency.samples, 2);
    assert_eq!(state.pending_events, 1);
    let engine = shared.lock().unwrap();
    let events = engine.recent_events();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].agent_id, "alpha");
    assert_eq!(events[1].agent_id, "beta");
    drop(engine);
    shared.lock().unwrap().notify_ledger.flush();
    drop(server);
}

/// NOT-01 的细分类型（提问/失败/审查/调度/Chief）必须在**幂等分域**上彼此独立：
/// 同一个 id 换个类型就是一个新事件，不是重放。这条走真实 HTTP + 生产消费链，
/// 因为「同 id 同内容却是另一类事件」正是分域最容易退化的地方
/// （把类型折叠掉、或只按 id 判重，都会让第二条被静默吞掉）。
#[test]
fn real_http_same_id_different_not01_kinds_all_enqueue() {
    use std::io::Write;
    let sandbox = Sandbox::new();
    let path = sandbox.path();
    let http = crate::webhook::LocalEventServer::bind_http("127.0.0.1:0").unwrap();
    let addr = http.server_addr().to_ip().unwrap();
    let (tx, rx) = mpsc::channel();
    let (wake, wake_rx) = mpsc::sync_channel(1);
    let mut activity = ActivityEngine::new(Settings::default(), rx);
    activity.disable_outbound();
    activity.enable_notify_ledger_persistence(&path);
    let shared = Arc::new(Mutex::new(activity));
    let server = crate::webhook::LocalEventServer::serve_bound(http, tx, shared.clone(), wake);
    let kinds = ["question", "failure", "review", "schedule", "chief"];
    for kind in kinds {
        let body = serde_json::json!({
            "agent": "opencode",
            "id": "shared-id",
            "event": kind,
            "message": "same body",
        })
        .to_string();
        let mut stream =
            std::net::TcpStream::connect_timeout(&addr, Duration::from_secs(1)).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        write!(stream, "POST /notify HTTP/1.1\r\nHost: {addr}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        assert!(
            response.starts_with("HTTP/1.1 200 "),
            "{kind} 必须被受理，实际响应：{response}"
        );
    }
    let scheduler = HookScheduler::new(wake_rx);
    let deadline = Instant::now() + Duration::from_secs(5);
    let state = loop {
        let state = scheduler
            .wait_and_consume(&shared, deadline, || {})
            .expect("production consumer timed out");
        if state.notify_latency.samples >= kinds.len() {
            break state;
        }
    };
    assert_eq!(
        state.notify_duplicates, 0,
        "同 id 不同类型不是重复——类型必须参与身份分域"
    );
    assert_eq!(state.notify_latency.samples, kinds.len());
    let engine = shared.lock().unwrap();
    let events = engine.recent_events();
    let mut observed: Vec<&str> = events.iter().map(|e| e.event_type.as_str()).collect();
    observed.sort_unstable();
    let mut expected = kinds.to_vec();
    expected.sort_unstable();
    assert_eq!(observed, expected, "五类必须各占一条且类型不被折叠");
    drop(engine);
    // 先 `flush()` 投递快照，再 `shutdown()` 等它落盘——两者是不同步骤：
    // `shutdown` 只等「已发布的 generation 被保存」，不会替你发布。
    // `server` 仍持 `shared` 引用，`Drop` 那一刻才会收尾，所以这里显式做。
    assert!(
        shared.lock().unwrap().notify_ledger.flush(),
        "快照未能落盘"
    );
    drop(server);
    let restored: Vec<String> = Ledger::open_at(&path)
        .restored_events()
        .into_iter()
        .map(|e| e.event_type)
        .collect();
    let mut restored_sorted = restored.clone();
    restored_sorted.sort();
    let mut expected_restored: Vec<String> = kinds.iter().map(|k| k.to_string()).collect();
    expected_restored.sort();
    assert_eq!(restored_sorted, expected_restored, "恢复后类型仍是细分值");
}

#[test]
fn over_capacity_keeps_latest_in_memory_disk_and_recovered_engine() {
    let sandbox = Sandbox::new();
    let path = sandbox.path();
    let mut engine = engine();
    engine.enable_notify_ledger_persistence(&path);
    for n in 0..CAP + 200 {
        hook(&mut engine, event(n), Some(&format!("id-{n}")));
    }
    engine.notify_ledger.flush();
    assert_eq!(engine.notify_ledger.identities.len(), CAP);
    assert_eq!(engine.notify_ledger.pending.len(), PENDING_CAP);
    assert_eq!(engine.recent_events().len(), PENDING_CAP);
    // Every enqueue past the retained queue bound dropped one body, and that loss is
    // now readable instead of invisible.
    assert_eq!(
        engine.state().notify_evicted,
        (CAP + 200 - PENDING_CAP) as u64,
        "淘汰计数必须与超出保留容量的入队数一致"
    );
    assert_eq!(
        engine.latest_event.as_ref().unwrap().id,
        format!("server-{}", CAP + 200 - PENDING_CAP)
    );
    assert!(!engine
        .notify_ledger
        .identities
        .contains(&identity(&event(0), Some("id-0"))));
    assert!(fs::metadata(&path).unwrap().len() <= MAX_BYTES);
    let recovered = Ledger::open_at(&path);
    assert_eq!(
        recovered
            .restored_events()
            .iter()
            .map(|e| e.id.clone())
            .collect::<Vec<_>>(),
        engine
            .recent_events()
            .iter()
            .map(stored_internal_id)
            .collect::<Vec<_>>()
    );
}

/// The two bounds with different widths are intentional, and so is what happens in
/// the gap between them. `pending` (64) is the visible/recoverable queue; `identities`
/// (1024) is the idempotency window. Between 64 and 1024 enqueues the body is gone
/// while the identity remains, so a redelivery is still suppressed as a duplicate —
/// the body is dropped, the identity is not. That is the chosen semantics: an event
/// the user already had delivered must not reappear just because the retention bound
/// reclaimed its copy. The loss itself is counted (`evicted`), never silent.
#[test]
fn an_evicted_body_stays_a_duplicate_until_its_identity_leaves_the_window() {
    let mut engine = engine();
    let path = Sandbox::new().path();
    engine.enable_notify_ledger_persistence(&path);
    // Fill past the retained queue: `event(0)`'s body is dropped, its identity kept.
    for n in 0..PENDING_CAP + 5 {
        hook(&mut engine, event(n), Some(&format!("id-{n}")));
    }
    assert_eq!(engine.notify_ledger.pending.len(), PENDING_CAP);
    assert!(!engine
        .notify_ledger
        .pending
        .iter()
        .any(|stored| stored.id == stored_internal_id(&event(0))));
    assert_eq!(engine.state().notify_evicted, 5);
    let duplicates_before = engine.state().notify_duplicates;
    hook(&mut engine, event(0), Some("id-0"));
    assert_eq!(
        engine.state().notify_duplicates,
        duplicates_before + 1,
        "正文被淘汰不等于身份被遗忘：重放仍须判重"
    );
    assert_eq!(engine.recent_events().len(), PENDING_CAP, "不得因重放而多出一条");
    assert_eq!(engine.state().notify_evicted, 5, "判重不是淘汰，计数器不动");
    // `notify_unacked` 是界面「未确认」那一格的唯一来源：它必须**含队首那条**。
    // 若有人把它改写成 `pending_events.len()`（不含队首），界面会永远少报一条。
    assert_eq!(
        engine.state().notify_unacked,
        engine.recent_events().len(),
        "未确认条数必须覆盖队首，等于实际未确认事件总数"
    );
    engine.ack_latest_event();
    assert_eq!(
        engine.state().notify_unacked,
        PENDING_CAP - 1,
        "确认一条后未确认数必须减一"
    );
    // Past the identity window the same payload is a new event again: the window is
    // bounded, not a permanent exactly-once claim.
    for n in PENDING_CAP + 5..CAP + PENDING_CAP + 5 {
        hook(&mut engine, event(n), Some(&format!("id-{n}")));
    }
    assert_eq!(engine.notify_ledger.identities.len(), CAP);
    assert!(!engine
        .notify_ledger
        .identities
        .contains(&identity(&event(0), Some("id-0"))));
    let duplicates = engine.state().notify_duplicates;
    hook(&mut engine, event(0), Some("id-0"));
    assert_eq!(
        engine.state().notify_duplicates,
        duplicates,
        "身份一旦滑出 1024 窗口，同样的投递就是新事件（窗口有界，不做永久承诺）"
    );
}

/// Two events can share agent / kind / millisecond and still be different events (their
/// bodies and ids differ, so they get different identities and both enqueue). The
/// acknowledgement key includes the stored identity, so acknowledging the *later* row
/// must leave the earlier one in place — with a key of agent/kind/millisecond alone this
/// same call would have matched and dropped the earlier row.
///
/// Not asserted here: "earliest vs newest among equally-keyed rows". `ack_latest_event`
/// only ever acks the queue head, so in the production call path the two orderings are
/// indistinguishable — a `rposition` mutation still passes this test (verified). The
/// tie-break is documented at `Ledger::ack` as a conservative reading, not proved by a
/// test; claiming otherwise would be a test that cannot fail.
#[test]
fn ack_removes_only_the_matching_identity_and_not_the_earlier_same_key_row() {
    let mut ledger = Ledger::default();
    let observed = event(1); // agent-1 / attention / 100_123
    let mut later_same_key = observed.clone();
    later_same_key.id = "second-server-id".into();
    later_same_key.message = Some("a different body for the same key".into());
    assert!(ledger.accept(&observed, Some("first")));
    assert!(ledger.accept(&later_same_key, Some("second")));
    assert_eq!(ledger.pending.len(), 2);
    // Acknowledge the *later* row: the earlier one must survive, which only holds
    // because the key carries the identity.
    ledger.ack(&later_same_key);
    assert_eq!(ledger.pending.len(), 1);
    assert_eq!(
        ledger.pending[0].id,
        stored_internal_id(&observed),
        "确认后插那条不得顺手删掉先插那条"
    );
    ledger.ack(&observed);
    assert!(ledger.pending.is_empty());
}


/// Concurrency regression for two **different** ingress paths, deliberately in one test:
/// - 32 threads send **real TCP** requests to the bound server, so framing, the accept
///   loop and the production consumer are exercised end to end.
/// - The remaining 200 events are **in-process `notify_route` calls**, not HTTP: they
///   drive the same route function and the same production consumer, but they skip the
///   32-thread socket layer on purpose (a 200-way real-TCP burst is a different test —
///   `one_200_request_burst_is_accepted_once_then_capacity_retains_latest` covers the
///   capacity side in-process too). Read this as "32 real sockets + 200 direct route
///   calls", never as "232 real HTTP requests".
///
/// Both halves must enqueue each event exactly once with no cross-binding: every
/// observed event carries a unique server id and its own agent/message pair.
#[test]
fn concurrent_32_real_tcp_and_200_route_burst_each_enqueue_once_without_cross_binding() {
    use std::io::Write;
    let http = crate::webhook::LocalEventServer::bind_http("127.0.0.1:0").unwrap();
    let addr = http.server_addr().to_ip().unwrap();
    let sandbox = Sandbox::new();
    let path = sandbox.path();
    let (tx, rx) = mpsc::channel();
    let (wake, wake_rx) = mpsc::sync_channel(1);
    let mut activity = ActivityEngine::new(Settings::default(), rx);
    activity.disable_outbound();
    activity.enable_notify_ledger_persistence(&path);
    let shared = Arc::new(Mutex::new(activity));
    let consumer_shared = shared.clone();
    let (done_tx, done_rx) = mpsc::channel();
    let consumer = std::thread::spawn(move || {
        let scheduler = HookScheduler::new(wake_rx);
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut observed = Vec::new();
        while observed.len() < 232 {
            assert!(
                scheduler
                    .wait_and_consume(&consumer_shared, deadline, || {})
                    .is_some(),
                "consumer timed out"
            );
            let mut engine = consumer_shared.lock().unwrap();
            while let Some(event) = engine.latest_event.clone() {
                observed.push(event);
                engine.ack_latest_event();
            }
            done_tx.send(observed.len()).unwrap();
        }
        observed
    });
    let server = crate::webhook::LocalEventServer::serve_bound(
        http,
        tx.clone(),
        shared.clone(),
        wake.clone(),
    );
    let barrier = Arc::new(std::sync::Barrier::new(32));
    let mut senders = Vec::new();
    for n in 0..32 {
        let barrier = barrier.clone();
        senders.push(std::thread::spawn(move || {
            barrier.wait();
            let body = serde_json::json!({"agent":format!("agent-{n}"),"message":format!("message-{n}"),"id":format!("id-{n}")}).to_string();
            let mut stream = std::net::TcpStream::connect_timeout(&addr, Duration::from_secs(1)).unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
            write!(stream, "POST /notify HTTP/1.1\r\nHost: {addr}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            let mut response = String::new(); stream.read_to_string(&mut response).unwrap();
            assert!(response.starts_with("HTTP/1.1 200 ")); assert!(response.contains("accepted")); assert!(!response.contains("delivered"));
        }));
    }
    for sender in senders {
        sender.join().unwrap();
    }
    loop {
        if done_rx.recv_timeout(Duration::from_secs(3)).unwrap() == 32 {
            break;
        }
    }
    // Four 50-event bursts through the route function directly (no sockets), with
    // acknowledgements between them. A single >64 unacknowledged burst intentionally
    // evicts oldest (separately tested above).
    for start in (32..232).step_by(50) {
        for n in start..start + 50 {
            let body = serde_json::json!({"agent":format!("agent-{n}"),"message":format!("message-{n}"),"id":format!("id-{n}")}).to_string();
            assert_eq!(
                crate::webhook::notify_route(&body, false, &tx, &wake, HookArrival::before(0)).0,
                200
            );
        }
        let target = start + 50;
        loop {
            if done_rx.recv_timeout(Duration::from_secs(3)).unwrap() >= target {
                break;
            }
        }
    }
    let events = consumer.join().unwrap();
    drop(server);
    assert_eq!(events.len(), 232);
    let mut ids = std::collections::HashSet::new();
    let mut agents = std::collections::HashSet::new();
    for event in events {
        assert!(ids.insert(event.id));
        assert!(agents.insert(event.agent_id.clone()));
        assert_eq!(
            event.message.as_deref(),
            Some(event.agent_id.replace("agent-", "message-").as_str())
        );
    }
    let engine = shared.lock().unwrap();
    engine.notify_ledger.flush();
    assert_eq!(engine.notify_latency.samples(), 232);
    assert_eq!(engine.state().notify_duplicates, 0);
    let summary = engine.notify_latency.summary();
    eprintln!("ledger burst 32+200: {summary:?}");
    assert_eq!(summary.meets_target, Some(true));
    assert!(Ledger::open_at(&path).pending.is_empty());
}

#[test]
fn active_writer_lock_is_nonblocking_and_retryable_for_the_worker() {
    let sandbox = Sandbox::new();
    let path = sandbox.path();
    let mut ledger = Ledger::open_at(&path);
    assert!(ledger.accept(&event(1), Some("first")));
    assert!(ledger.flush());
    let original = fs::read(&path).unwrap();
    let lock = open_regular(&path).unwrap();
    lock.try_lock().unwrap();
    assert!(ledger.accept(&event(2), Some("second")));
    let start = Instant::now();
    assert!(!ledger.flush());
    assert!(start.elapsed() < Duration::from_secs(1));
    let status = ledger.status();
    assert_eq!(status.error, Some(WRITE_ERROR));
    assert!(!status.blocked, "lock contention must not permanently disable the worker");
    assert_eq!(status.saved_generation, 1);
    assert_eq!(status.attempted_generation, 2);
    assert_eq!(
        crate::persistence_file::read_current(&path, MAX_BYTES, Some(&lock)).unwrap().unwrap().0,
        original,
    );
    drop(lock);
    ledger.ack(&event(1)); // A new snapshot retries the failed publication.
    assert!(ledger.flush());
    assert!(ledger.status().saved);
    let restored = Ledger::open_at(&path).restored_events();
    assert_eq!(restored.len(), 1);
    assert_eq!(restored[0].id, stored_internal_id(&event(2)));
}

#[test]
fn identical_replacement_with_preserved_mtime_blocks_the_ledger_worker() {
    let sandbox = Sandbox::new();
    let path = sandbox.path();
    let mut ledger = Ledger::open_at(&path);
    assert!(ledger.accept(&event(1), Some("first")));
    assert!(ledger.flush());
    let original = fs::read(&path).unwrap();
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    let replacement = sandbox.0.join("replacement.json");
    fs::write(&replacement, &original).unwrap();
    fs::OpenOptions::new().write(true).open(&replacement).unwrap()
        .set_times(fs::FileTimes::new().set_modified(modified)).unwrap();
    fs::rename(&replacement, &path).unwrap();
    assert!(ledger.accept(&event(2), Some("second")));
    assert!(!ledger.flush());
    assert!(ledger.status().blocked);
    assert_eq!(ledger.status().error, Some(STALE_ERROR));
    assert_eq!(fs::read(&path).unwrap(), original);
}

#[test]
fn removing_a_loaded_ledger_permanently_blocks_future_publications() {
    let sandbox = Sandbox::new();
    let path = sandbox.path();
    let mut ledger = Ledger::open_at(&path);
    ledger.accept(&event(1), Some("first"));
    assert!(ledger.flush());
    let original = fs::read(&path).unwrap();
    fs::remove_file(&path).unwrap();
    ledger.accept(&event(2), Some("second"));
    assert!(!ledger.flush());
    assert!(ledger.status().blocked);
    assert_eq!(ledger.status().error, Some(STALE_ERROR));
    fs::write(&path, &original).unwrap();
    ledger.accept(&event(3), Some("third"));
    assert!(!ledger.flush());
    assert_eq!(fs::read(&path).unwrap(), original);
}
