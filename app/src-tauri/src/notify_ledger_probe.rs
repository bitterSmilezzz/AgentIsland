//! Explicit synthetic, headless process-restart acceptance. Never loads user config.
use crate::engine::{ActivityEngine, HookScheduler};
use crate::webhook::LocalEventServer;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

const MARKER: &str = ".agentisland-notify-ledger-probe";
const MARKER_BYTES: &[u8] = b"agentisland synthetic notification ledger probe v1\n";

pub(crate) fn cli(argv: &[String]) -> Option<i32> {
    if argv.get(1).map(String::as_str) != Some("--notify-ledger-probe") { return None; }
    let parsed = if argv.len() == 4 {
        argv[3].parse::<u8>().ok().filter(|phase| (1..=3).contains(phase))
            .ok_or("invalid probe phase").map(|phase| (argv[2].clone(), phase))
    } else { Err("usage: --notify-ledger-probe <empty-temp-fixture-dir> <1|2|3>") };
    let result = match parsed {
        Ok((dir, phase)) => run(Path::new(&dir), phase),
        Err(reason) => Err(reason),
    };
    match result {
        Ok(report) => { println!("{report}"); Some(0) }
        Err(reason) => { eprintln!("notify ledger probe failed: {reason}"); Some(2) }
    }
}

fn fixture(path: &Path, phase: u8) -> Result<PathBuf, &'static str> {
    if !path.is_absolute() { return Err("fixture must be an explicit absolute temp directory"); }
    let meta = fs::symlink_metadata(path).map_err(|_| "fixture directory missing")?;
    if !meta.is_dir() || meta.file_type().is_symlink() { return Err("fixture must be a real directory"); }
    let canonical = path.canonicalize().map_err(|_| "fixture unavailable")?;
    let temp = std::env::temp_dir().canonicalize().map_err(|_| "temp directory unavailable")?;
    if canonical == temp || !canonical.starts_with(&temp) { return Err("fixture must be beneath system temp"); }
    let entries: Vec<_> = fs::read_dir(&canonical).map_err(|_| "fixture unreadable")?
        .collect::<Result<_, _>>().map_err(|_| "fixture unreadable")?;
    if phase == 1 {
        if !entries.is_empty() { return Err("phase 1 requires an empty fixture"); }
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)] { use std::os::unix::fs::OpenOptionsExt; options.mode(0o600); }
        options.open(canonical.join(MARKER)).and_then(|mut file| file.write_all(MARKER_BYTES))
            .map_err(|_| "fixture marker creation failed")?;
    } else {
        for entry in entries {
            let name = entry.file_name();
            if name != MARKER && name != "notify-ledger.json" { return Err("fixture contains unrelated files"); }
            let metadata = fs::symlink_metadata(entry.path()).map_err(|_| "fixture entry unavailable")?;
            if !metadata.is_file() || metadata.file_type().is_symlink() { return Err("fixture entry must be regular"); }
        }
        if fs::read(canonical.join(MARKER)).map_err(|_| "fixture marker missing")? != MARKER_BYTES {
            return Err("fixture marker invalid");
        }
    }
    Ok(canonical.join("notify-ledger.json"))
}

fn post(addr: std::net::SocketAddr, agent: &str, id: &str) -> Result<(), &'static str> {
    let body = serde_json::json!({"agent":agent,"event":"attention","id":id,
        "message":"synthetic private body must remain in memory", "detail":"/synthetic/private/source"}).to_string();
    let mut stream = std::net::TcpStream::connect_timeout(&addr, Duration::from_secs(2))
        .map_err(|_| "probe HTTP connect failed")?;
    stream.set_read_timeout(Some(Duration::from_secs(2))).map_err(|_| "probe timeout setup failed")?;
    stream.set_write_timeout(Some(Duration::from_secs(2))).map_err(|_| "probe timeout setup failed")?;
    write!(stream, "POST /notify HTTP/1.1\r\nHost: {addr}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
        .map_err(|_| "probe HTTP write failed")?;
    let mut response = String::new();
    stream.take(4096).read_to_string(&mut response).map_err(|_| "probe HTTP read failed")?;
    if !response.starts_with("HTTP/1.1 200 ") || !response.contains("accepted") { return Err("probe HTTP rejected"); }
    Ok(())
}

fn run(path: &Path, phase: u8) -> Result<serde_json::Value, &'static str> {
    let path = fixture(path, phase)?;
    let (tx, rx) = mpsc::channel();
    let (wake, wake_rx) = mpsc::sync_channel(1);
    let mut engine = ActivityEngine::notification_probe(rx);
    engine.enable_notify_ledger_persistence(&path);
    let before = engine.recent_events().len();
    if engine.notify_ledger.status().blocked { return Err("probe ledger blocked"); }
    if before != if phase == 2 { 3 } else { 0 } { return Err("unexpected restored pending count"); }
    let restored_safe = engine.recent_events().iter().all(|event| {
        event.id.len() == 64 && event.detail.is_none()
            && event.message.as_deref() == Some("重启恢复的提醒，原始正文未保存，请查看来源")
    });
    let shared = Arc::new(Mutex::new(engine));
    let http = LocalEventServer::bind_http("127.0.0.1:0").map_err(|_| "probe bind failed")?;
    let addr = http.server_addr().to_ip().ok_or("probe address unavailable")?;
    let server = LocalEventServer::serve_bound(http, tx, shared.clone(), wake);
    let requests: &[(&str, &str)] = match phase {
        1 => &[("claude", "same-id"), ("claude", "same-id"), ("codex", "same-id"), ("claude", "new-id")],
        2 => &[("claude", "same-id"), ("codex", "same-id"), ("claude", "new-id")],
        _ => &[("claude", "same-id")],
    };
    let scheduler = HookScheduler::new(wake_rx);
    for &(agent, id) in requests {
        post(addr, agent, id)?;
        scheduler.wait_and_consume(&shared, Instant::now() + Duration::from_secs(2), || {})
            .ok_or("probe production consumer timed out")?;
    }
    drop(server);
    let (duplicates, conflicts, evicted, samples, after, hashes, handle) = {
        let mut engine = shared.lock().map_err(|_| "probe engine unavailable")?;
        let state = engine.state();
        let after = engine.recent_events().len();
        let hashes: Vec<_> = engine.notify_ledger.restored_events().iter().map(|event| event.id.clone()).collect();
        let expected_duplicates = match phase { 1 => 1, 2 => 3, _ => 1 };
        if state.notify_duplicates != expected_duplicates || after != if phase == 3 { 0 } else { 3 } {
            return Err("probe idempotency mismatch");
        }
        if state.notify_conflicts != 0 { return Err("probe reported an unprovable conflict"); }
        // The probe never exceeds the retained queue, so it must also report that
        // nothing was evicted: a nonzero counter here would mean the bounded queue
        // dropped a body the receipt still claims is pending.
        if state.notify_evicted != 0 { return Err("probe unexpectedly evicted a pending body"); }
        if phase == 2 { while engine.latest_event.is_some() { engine.ack_latest_event(); } }
        (state.notify_duplicates, state.notify_conflicts, state.notify_evicted, state.notify_latency.samples, after, hashes, engine.notify_ledger.flush_handle().ok_or("probe flush unavailable")?)
    };
    if !handle.shutdown(Duration::from_secs(2)) { return Err("probe flush failed"); }
    let status = handle.status();
    let bytes = fs::read(&path).map_err(|_| "probe ledger unreadable")?;
    let text = String::from_utf8_lossy(&bytes);
    for forbidden in ["synthetic private body", "/synthetic/private", "agent_name", "\"message\"", "\"detail\""] {
        if text.contains(forbidden) { return Err("probe persisted private content"); }
    }
    let unsigned: serde_json::Value = serde_json::from_slice(&bytes).map_err(|_| "probe ledger invalid")?;
    if unsigned["pending"].as_array().map(Vec::len).unwrap_or(1) != shared.lock().map_err(|_| "probe engine unavailable")?.recent_events().len() {
        return Err("probe disk and memory disagree");
    }
    Ok(serde_json::json!({"schemaVersion":1,"phase":phase,"pid":std::process::id(),
        "restoredPending":before,"restoredSafe":restored_safe,"httpAccepted":requests.len(),
        "pendingBeforeAck":after,"duplicates":duplicates,"conflicts":conflicts,
        "evicted":evicted,"samples":samples,
        "pendingAfterAck":shared.lock().map_err(|_| "probe engine unavailable")?.recent_events().len(),
        "hashes":hashes,"hashesAreSafe":hashes.iter().all(|hash| crate::notify_ledger::is_hash(hash)),
        "diskHasPrivateText":false,"flushSaved":true,"generation":status.generation,
        "savedGeneration":status.saved_generation,"failures":status.failures}))
}
