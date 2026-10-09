use super::*;
use serde_json::json;
use std::io::Write;

fn profile(dir: &Path) -> crate::models::AgentProfile {
    let mut p = crate::registry::builtin()
        .into_iter()
        .find(|p| p.id == "codex")
        .unwrap();
    p.token_roots = vec![dir.to_string_lossy().into_owned()];
    p.session_database = None;
    p
}
fn meta(provider: &str) -> String {
    json!({"type":"session_meta","payload":{"id":"thread-fixture","session_id":"session-fixture","model_provider":provider}}).to_string()
}
fn turn(id: &str, model: &str) -> String {
    json!({"type":"turn_context","payload":{"turn_id":id,"model":model}}).to_string()
}
fn usage(id: &str, turn: &str, ts: i64) -> String {
    json!({"type":"token_usage_record","timestamp":iso_utc_from_ms(ts),"payload":{"thread_id":"thread-fixture","session_id":"session-fixture","turn_id":turn,"response_id":id,"usage":{"input_tokens":1000,"cached_input_tokens":900,"output_tokens":50}}}).to_string()
}
fn write(path: &Path, lines: &[String]) {
    fs::write(path, format!("{}\n", lines.join("\n"))).unwrap();
}
fn append(path: &Path, lines: &[String]) {
    writeln!(
        fs::OpenOptions::new().append(true).open(path).unwrap(),
        "{}",
        lines.join("\n")
    )
    .unwrap();
}
fn coverage(r: &TokenReport, matched: i64, unknown: i64) {
    let c = &r.context24h;
    assert_eq!((c.matched_tokens, c.unmatched_tokens), (matched, unknown));
    assert_eq!(matched + unknown, r.usage.tokens24h);
    assert_eq!(
        c.rows.iter().map(|r| r.tokens).sum::<i64>() + c.other_matched_tokens,
        matched
    );
}
#[test]
fn exact_context_is_separate_from_response_model_cost_and_interface() {
    let s = crate::testutil::Sandbox::new("context-exact");
    let path = s.path().join("a.jsonl");
    write(
        &path,
        &[
            meta("provider-alpha"),
            turn("turn-one", "gpt-4o"),
            usage("response-one", "turn-one", now_ms()),
            usage("response-two", "turn-one", now_ms()),
        ],
    );
    let mut m = TokenUsageMonitor::new();
    let r = m.monitor(&profile(s.path()));
    coverage(&r, 300, 0);
    assert_eq!(
        r.context24h.rows[0].provider.as_deref(),
        Some("provider-alpha")
    );
    assert_eq!(
        r.context24h.rows[0].requested_model.as_deref(),
        Some("gpt-4o")
    );
    assert_eq!(r.models24h[0].model, "unknown");
    assert_eq!(r.usage.cost_total, 0.);
    let entries = &m.states.values().next().unwrap().entries;
    assert!(Arc::ptr_eq(
        entries[0].context.as_ref().unwrap(),
        entries[1].context.as_ref().unwrap()
    ));
    let text = serde_json::to_string(&r.context24h).unwrap();
    for private in [
        "thread-fixture",
        "session-fixture",
        "turn-one",
        "response-one",
        "endpoint",
        "auth",
    ] {
        assert!(!text.contains(private));
    }
    coverage(&m.monitor(&profile(s.path())), 300, 0);
}
#[test]
fn incremental_baselines_do_not_relabel_previous_responses() {
    let s = crate::testutil::Sandbox::new("context-append");
    let path = s.path().join("a.jsonl");
    write(
        &path,
        &[
            meta("provider-alpha"),
            turn("turn-one", "model-one"),
            usage("response-one", "turn-one", now_ms()),
        ],
    );
    let mut m = TokenUsageMonitor::new();
    coverage(&m.monitor(&profile(s.path())), 150, 0);
    append(
        &path,
        &[
            meta("provider-bravo"),
            turn("turn-two", "model-two"),
            usage("response-two", "turn-two", now_ms()),
        ],
    );
    let r = m.monitor(&profile(s.path()));
    coverage(&r, 300, 0);
    assert_eq!(r.context24h.rows.len(), 2);
    assert_eq!(
        r.context24h
            .rows
            .iter()
            .map(|r| r.provider.as_deref().unwrap())
            .collect::<Vec<_>>(),
        ["provider-alpha", "provider-bravo"]
    );
}
#[test]
fn copied_other_session_thread_or_turn_remains_unmatched() {
    let s = crate::testutil::Sandbox::new("context-copied");
    let path = s.path().join("a.jsonl");
    let mut lines = vec![meta("provider-alpha"), turn("turn-one", "model-one")];
    for (i, field) in ["session_id", "thread_id", "turn_id"]
        .into_iter()
        .enumerate()
    {
        let mut u: Value =
            serde_json::from_str(&usage(&format!("copy-{i}"), "turn-one", now_ms())).unwrap();
        u["payload"][field] = json!("other");
        lines.push(u.to_string());
    }
    let mut old: Value = serde_json::from_str(&usage("older", "turn-one", now_ms())).unwrap();
    old["payload"].as_object_mut().unwrap().remove("session_id");
    lines.push(old.to_string());
    lines.push(usage("valid", "turn-one", now_ms()));
    write(&path, &lines);
    coverage(
        &TokenUsageMonitor::new().monitor(&profile(s.path())),
        150,
        600,
    );
}
#[test]
fn duplicate_context_conflict_invalidates_the_fact_without_recounting() {
    let s = crate::testutil::Sandbox::new("context-conflict");
    let path = s.path().join("a.jsonl");
    let a = [
        meta("provider-alpha"),
        turn("turn-one", "model-one"),
        usage("same-response", "turn-one", now_ms()),
    ];
    write(&path, &a);
    let mut m = TokenUsageMonitor::new();
    coverage(&m.monitor(&profile(s.path())), 150, 0);
    let b = [
        meta("provider-bravo"),
        turn("turn-one", "model-one"),
        usage("same-response", "turn-one", now_ms()),
        usage("new-response", "turn-one", now_ms()),
    ];
    append(&path, &b);
    let r = m.monitor(&profile(s.path()));
    coverage(&r, 150, 150);
    assert_eq!(r.usage.tokens_total, 300);
    // The same conflict must survive a whole-file read and the next cache hit.
    let mut fresh = TokenUsageMonitor::new();
    coverage(&fresh.monitor(&profile(s.path())), 150, 150);
    coverage(&fresh.monitor(&profile(s.path())), 150, 150);
}
#[test]
fn incomplete_context_append_is_not_admitted_before_newline() {
    let s = crate::testutil::Sandbox::new("context-partial");
    let path = s.path().join("a.jsonl");
    write(
        &path,
        &[
            meta("provider-alpha"),
            turn("turn-one", "model-one"),
            usage("response-one", "turn-one", now_ms()),
        ],
    );
    let mut m = TokenUsageMonitor::new();
    coverage(&m.monitor(&profile(s.path())), 150, 0);
    let next = turn("turn-two", "model-two");
    let split = next.len() / 2;
    fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(next[..split].as_bytes())
        .unwrap();
    coverage(&m.monitor(&profile(s.path())), 150, 0);
    let mut f = fs::OpenOptions::new().append(true).open(&path).unwrap();
    writeln!(
        f,
        "{}\n{}",
        &next[split..],
        usage("response-two", "turn-two", now_ms())
    )
    .unwrap();
    drop(f);
    let r = m.monitor(&profile(s.path()));
    coverage(&r, 300, 0);
    assert_eq!(r.context24h.rows.len(), 2);
}
#[test]
fn equal_length_rewrite_rebuilds_context_and_usage_together() {
    let s = crate::testutil::Sandbox::new("context-rewrite");
    let path = s.path().join("a.jsonl");
    let lines = [
        meta("provider-alpha"),
        turn("turn-one", "model-one"),
        usage("response-one", "turn-one", now_ms()),
    ];
    write(&path, &lines);
    let mut m = TokenUsageMonitor::new();
    coverage(&m.monitor(&profile(s.path())), 150, 0);
    let size = fs::metadata(&path).unwrap().len();
    std::thread::sleep(std::time::Duration::from_millis(10));
    write(
        &path,
        &lines.map(|s| s.replace("provider-alpha", "provider-bravo")),
    );
    assert_eq!(fs::metadata(&path).unwrap().len(), size);
    let r = m.monitor(&profile(s.path()));
    coverage(&r, 150, 0);
    assert_eq!(
        r.context24h.rows[0].provider.as_deref(),
        Some("provider-bravo")
    );
}
#[test]
fn folded_history_preserves_totals_and_does_not_enter_24h_coverage() {
    let s = crate::testutil::Sandbox::new("context-fold");
    let path = s.path().join("a.jsonl");
    write(
        &path,
        &[
            meta("provider-alpha"),
            turn("turn-one", "model-one"),
            usage("ancient", "turn-one", now_ms() - 80 * 86_400_000),
            usage("recent", "turn-one", now_ms()),
        ],
    );
    let mut m = TokenUsageMonitor::new();
    let r = m.monitor(&profile(s.path()));
    coverage(&r, 150, 0);
    assert_eq!(r.usage.tokens_total, 300);
    assert_eq!(m.states.values().next().unwrap().rolled_count, 1);
    coverage(&m.monitor(&profile(s.path())), 150, 0);
}
#[test]
fn capped_details_share_context_and_keep_coverage_on_the_same_ledger() {
    let s = crate::testutil::Sandbox::new("context-cap");
    let path = s.path().join("a.jsonl");
    let mut lines = vec![meta("provider-alpha"), turn("turn-one", "model-one")];
    lines.extend((0..20_001).map(|i| usage(&format!("response-{i}"), "turn-one", now_ms() + i)));
    write(&path, &lines);
    let mut m = TokenUsageMonitor::new();
    let r = m.monitor(&profile(s.path()));
    coverage(&r, 3_000_000, 0);
    assert_eq!(r.usage.tokens_total, 3_000_150);
    let state = m.states.values().next().unwrap();
    assert_eq!(state.entries.len(), 20_000);
    assert!(state.seen_ids.is_empty());
    assert!(state.entries.iter().all(|e| Arc::ptr_eq(
        e.context.as_ref().unwrap(),
        state.entries[0].context.as_ref().unwrap()
    )));
    coverage(&m.monitor(&profile(s.path())), 3_000_000, 0);
}
#[test]
fn removed_files_and_other_tools_do_not_contribute_context() {
    let a = crate::testutil::Sandbox::new("context-tool-a");
    let b = crate::testutil::Sandbox::new("context-tool-b");
    let path = a.path().join("a.jsonl");
    write(
        &path,
        &[
            meta("provider-alpha"),
            turn("turn-one", "model-one"),
            usage("response-one", "turn-one", now_ms()),
        ],
    );
    let mut m = TokenUsageMonitor::new();
    coverage(&m.monitor(&profile(a.path())), 150, 0);
    coverage(&m.monitor(&profile(b.path())), 0, 0);
    fs::remove_file(path).unwrap();
    coverage(&m.monitor(&profile(a.path())), 0, 0);
}
#[test]
fn unsupported_sources_are_unmatched_in_the_same_report() {
    let s = crate::testutil::Sandbox::new("context-other");
    write(&s.path().join("a.jsonl"),&[json!({"timestamp":iso_utc_from_ms(now_ms()),"message":{"id":"claude-message","model":"claude-model","usage":{"input_tokens":100,"output_tokens":50}}}).to_string()]);
    coverage(
        &TokenUsageMonitor::new().monitor(&profile(s.path())),
        0,
        150,
    );
}
