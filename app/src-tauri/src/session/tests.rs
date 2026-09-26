use super::*;

const CLAUDE: &str = include_str!("../../tests/fixtures/session/claude-lifecycle.jsonl");
const CODEX: &str = include_str!("../../tests/fixtures/session/codex-lifecycle.jsonl");
const CODEX_QUESTION: &str = include_str!("../../tests/fixtures/session/codex-question-function.jsonl");
const CODEX_MESSAGES: &str = include_str!("../../tests/fixtures/session/codex-messages.jsonl");
const CODEX_REPEATED: &str = include_str!("../../tests/fixtures/session/codex-repeated-command.jsonl");
const CLINE: &str = include_str!("../../tests/fixtures/session/cline-lifecycle.json");
const FIXTURE_PATH: &str = "/synthetic/session-fixture";

fn prefix(fixture: &str, count: usize) -> Vec<String> {
    fixture.lines().take(count).map(str::to_owned).collect()
}

fn cline_prefix(count: usize) -> Vec<String> {
    let messages: Vec<Value> = serde_json::from_str(CLINE).expect("valid synthetic fixture");
    vec![serde_json::to_string(&messages[..count]).unwrap()]
}

fn fixture_path(name: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/session")
        .join(name)
        .to_str()
        .unwrap()
        .to_owned()
}

fn assert_no_signal(probe: SessionProbe) {
    assert!(probe.signal.is_none(), "expected no semantic signal: {:?}", probe.signal);
    assert_eq!(probe.subagent_count, 0);
}

fn active_fingerprint(probe: SessionProbe, action_contains: &str) -> String {
    match probe.signal {
        Some(Signal::Active(fingerprint, Some(action))) => {
            assert!(!fingerprint.is_empty());
            assert!(action.contains(action_contains), "unexpected action: {action}");
            fingerprint
        }
        signal => panic!("expected active work, got {signal:?}"),
    }
}

fn assert_attention(probe: SessionProbe) {
    match probe.signal {
        Some(Signal::Attention(fingerprint, message)) => {
            assert!(!fingerprint.is_empty());
            assert!(!message.is_empty());
        }
        signal => panic!("expected pending user input, got {signal:?}"),
    }
}

fn completed_fingerprint(probe: SessionProbe) -> String {
    match probe.signal {
        Some(Signal::Completed(fingerprint)) => {
            assert!(!fingerprint.is_empty());
            fingerprint
        }
        signal => panic!("expected explicit completion, got {signal:?}"),
    }
}

#[test]
fn claude_open_tool_is_active_until_its_matching_result() {
    active_fingerprint(probe_claude(&prefix(CLAUDE, 2), FIXTURE_PATH), "cargo check");
    active_fingerprint(probe_claude(&prefix(CLAUDE, 3), FIXTURE_PATH), "cargo check");
    assert_no_signal(probe_claude(&prefix(CLAUDE, 4), FIXTURE_PATH));
}

#[test]
fn claude_question_is_attention_until_its_result() {
    assert_attention(probe_claude(&prefix(CLAUDE, 5), FIXTURE_PATH));
    assert_no_signal(probe_claude(&prefix(CLAUDE, 6), FIXTURE_PATH));
}

#[test]
fn claude_final_response_completes_via_public_file_probe() {
    completed_fingerprint(probe("claude", &fixture_path("claude-lifecycle.jsonl")));
}

#[test]
fn codex_open_tool_is_active_until_its_matching_result() {
    active_fingerprint(probe_codex(&prefix(CODEX, 2), FIXTURE_PATH), "cargo check");
    active_fingerprint(probe_codex(&prefix(CODEX, 3), FIXTURE_PATH), "cargo check");
    assert_no_signal(probe_codex(&prefix(CODEX, 4), FIXTURE_PATH));
}

#[test]
fn codex_custom_question_stays_pending_through_accounting_and_closes_on_output() {
    assert_attention(probe_codex(&prefix(CODEX, 5), FIXTURE_PATH));
    assert_attention(probe_codex(&prefix(CODEX, 6), FIXTURE_PATH));
    assert_no_signal(probe_codex(&prefix(CODEX, 7), FIXTURE_PATH));
}

#[test]
fn codex_function_question_matches_output_by_call_id() {
    assert_attention(probe_codex(&prefix(CODEX_QUESTION, 1), FIXTURE_PATH));
    assert_attention(probe_codex(&prefix(CODEX_QUESTION, 2), FIXTURE_PATH));
    assert_no_signal(probe_codex(&prefix(CODEX_QUESTION, 3), FIXTURE_PATH));
}

#[test]
fn codex_completion_survives_accounting_but_new_user_turn_clears_it() {
    let completed = completed_fingerprint(probe_codex(&prefix(CODEX, 8), FIXTURE_PATH));
    assert_eq!(completed_fingerprint(probe_codex(&prefix(CODEX, 9), FIXTURE_PATH)), completed);
    assert_no_signal(probe_codex(&prefix(CODEX, 10), FIXTURE_PATH));
    assert_no_signal(probe("codex", &fixture_path("codex-lifecycle.jsonl")));
}

#[test]
fn codex_reasoning_invalidates_old_completion_without_a_user_record() {
    let lines: Vec<String> = CODEX.lines().skip(7).take(2).chain(CODEX.lines().skip(10)).map(str::to_owned).collect();
    assert_no_signal(probe_codex(&lines, FIXTURE_PATH));
}

#[test]
fn codex_ordinary_user_and_assistant_messages_never_claim_completion() {
    for line in CODEX_MESSAGES.lines() {
        assert_no_signal(probe_codex(&[line.to_owned()], FIXTURE_PATH));
    }
    assert_no_signal(probe("codex", &fixture_path("codex-messages.jsonl")));
}

#[test]
fn codex_unresolved_command_survives_commentary_and_premature_completion() {
    let mut lines = prefix(CODEX, 2);
    lines.push(CODEX_MESSAGES.lines().nth(1).unwrap().to_owned());
    lines.push(CODEX.lines().nth(7).unwrap().to_owned());
    active_fingerprint(probe_codex(&lines, FIXTURE_PATH), "cargo check");
}

#[test]
fn codex_call_identity_is_stable_and_distinguishes_repeated_commands() {
    let first = active_fingerprint(probe_codex(&prefix(CODEX_REPEATED, 1), FIXTURE_PATH), "cargo check");
    assert_eq!(active_fingerprint(probe_codex(&prefix(CODEX_REPEATED, 1), FIXTURE_PATH), "cargo check"), first);
    let second = active_fingerprint(probe_codex(&prefix(CODEX_REPEATED, 3), FIXTURE_PATH), "cargo check");
    assert_ne!(first, second, "different calls must not share their notification identity");
}

#[test]
fn cline_command_is_active_and_approval_replaces_it() {
    active_fingerprint(probe_cline(&cline_prefix(2), FIXTURE_PATH), "cargo check");
    assert_attention(probe_cline(&cline_prefix(3), FIXTURE_PATH));
}

#[test]
fn cline_output_clears_old_approval_and_completion_ends_work() {
    active_fingerprint(probe_cline(&cline_prefix(4), FIXTURE_PATH), "Synthetic build output.");
    completed_fingerprint(probe("cline", &fixture_path("cline-lifecycle.json")));
}

#[test]
fn broken_jsonl_tail_does_not_hide_last_complete_record() {
    active_fingerprint(probe("codex", &fixture_path("codex-broken-tail.jsonl")), "cargo check");
}

#[test]
fn empty_malformed_missing_and_unknown_sessions_have_no_signal() {
    for profile in ["claude", "codex", "cline"] {
        for name in ["empty.jsonl", "invalid.jsonl", "fixture-does-not-exist.jsonl"] {
            assert_no_signal(probe(profile, &fixture_path(name)));
        }
    }
    assert_no_signal(probe("cline", &fixture_path("cline-malformed.json")));
    assert_no_signal(probe("unknown-fixture-dialect", &fixture_path("claude-lifecycle.jsonl")));
}

#[test]
fn output_records_without_requests_do_not_recreate_work_or_attention() {
    for line in CLAUDE.lines().skip(2).step_by(3) {
        assert_no_signal(probe_claude(&[line.to_owned()], FIXTURE_PATH));
    }
    for index in [3, 6] {
        assert_no_signal(probe_codex(&[CODEX.lines().nth(index).unwrap().to_owned()], FIXTURE_PATH));
    }
}
