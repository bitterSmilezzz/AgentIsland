use super::*;
use super::tests::{event, Sandbox};
use std::time::Duration;

#[test]
fn arbitrary_private_fields_never_reach_disk_from_any_entry_point() {
    let sandbox = Sandbox::new();
    let path = sandbox.path();
    let mut ledger = Ledger::open_at(&path);
    for external in [true, false] {
        let mut input = event(external as usize);
        input.id = format!("/synthetic/private/id/{external}");
        input.agent_id = "arbitrary-private-agent".into();
        input.agent_name = "arbitrary-private-name".into();
        input.message = Some("arbitrary-private-message".into());
        input.detail = Some("arbitrary-private-detail".into());
        input.externally_delivered = external;
        assert!(ledger.accept_internal(&input));
    }
    assert!(ledger.flush());
    let raw = fs::read(&path).unwrap();
    let text = String::from_utf8_lossy(&raw);
    for forbidden in ["arbitrary-private", "/synthetic/private", "agent_name", "message", "detail"] {
        assert!(!text.contains(forbidden), "private projection escaped");
    }
    let restored = Ledger::open_at(&path).restored_events();
    assert_eq!(restored.len(), 2);
    assert!(restored.iter().all(|e| e.agent_id == "external" && is_hash(&e.id) && e.detail.is_none()));
    assert!(restored.iter().all(|e| e.agent_name == "外部投递"));
    let mut doc: serde_json::Value = serde_json::from_slice(&raw).unwrap();
    doc["pending"][0]["message"] = serde_json::json!("must reject legacy text");
    assert!(decode(&serde_json::to_vec(&doc).unwrap()).is_err());
    doc["pending"][0].as_object_mut().unwrap().remove("message");
    doc["pending"][0]["agent_id"] = serde_json::json!("arbitrary-private-agent");
    assert!(decode(&serde_json::to_vec(&doc).unwrap()).is_err());
}

#[test]
fn client_identity_is_scoped_by_agent_and_normalized_kind_and_first_payload_wins() {
    let mut ledger = Ledger::default();
    let first = event(1);
    assert!(ledger.accept(&first, Some("shared-id")));
    let mut other_agent = first.clone();
    other_agent.agent_id = "codex".into();
    assert!(ledger.accept(&other_agent, Some("shared-id")));
    let mut other_kind = first.clone();
    other_kind.event_type = "cost_spike".into();
    assert!(ledger.accept(&other_kind, Some("shared-id")));
    other_kind.event_type = "costSpike".into();
    assert!(!ledger.accept(&other_kind, Some("shared-id")));
    let mut changed_payload = first.clone();
    changed_payload.message = Some("different payload".into());
    assert!(!ledger.accept(&changed_payload, Some("shared-id")));
    assert_eq!(ledger.pending.len(), 3);
    assert_eq!(ledger.duplicates(), 2);
    assert_eq!(ledger.conflicts(), 1);
    // After a restart only identities survive: a differing-payload replay is a plain
    // duplicate, never a conflict the old process cannot prove.
    let sandbox = Sandbox::new();
    let path = sandbox.path();
    let mut first_process = Ledger::open_at(&path);
    assert!(first_process.accept(&first, Some("shared-id")));
    assert!(first_process.flush());
    let mut second_process = Ledger::open_at(&path);
    assert!(!second_process.accept(&changed_payload, Some("shared-id")));
    assert_eq!(second_process.duplicates(), 1);
    assert_eq!(second_process.conflicts(), 0);
}

#[test]
fn ack_maps_raw_and_recovered_hashed_ids_through_the_stored_identity() {
    let mut ledger = Ledger::default();
    let mut external = event(1);
    external.externally_delivered = true;
    external.agent_id = "unknown-http-agent".into();
    assert!(ledger.accept(&external, Some("client-a")));
    assert_eq!(ledger.pending.len(), 1);
    // The live acknowledgement carries the raw id and unknown agent; the stored row
    // carries only the hash and the `external` placeholder.
    ledger.ack(&external);
    assert!(ledger.pending.is_empty());
    let mut internal = event(2);
    internal.externally_delivered = false;
    internal.id = "/synthetic/private/session-fingerprint".into();
    assert!(ledger.accept_internal(&internal));
    ledger.ack(&internal);
    assert!(ledger.pending.is_empty());
}

#[test]
fn flush_reports_blocked_and_io_error_without_claiming_saved() {
    let sandbox = Sandbox::new();
    let path = sandbox.path();
    fs::write(&path, b"invalid").unwrap();
    let mut blocked = Ledger::open_at(&path);
    assert!(blocked.status().blocked);
    blocked.accept(&event(1), None);
    assert!(!blocked.flush());
    assert_eq!(blocked.status().saved_generation, 0);
    assert!(!blocked.status().healthy);
    let parent_file = sandbox.0.join("not-directory");
    fs::write(&parent_file, b"fixture").unwrap();
    let mut error = Ledger::open_at(sandbox.0.join("new-directory").join("ledger.json"));
    fs::write(sandbox.0.join("new-directory"), b"fixture").unwrap();
    error.accept(&event(2), None);
    assert!(!error.flush());
    let state = error.status();
    assert!(!state.saved);
    assert!(state.error.is_some());
    assert_eq!(state.attempted_generation, 1);
    assert_eq!(state.saved_generation, 0);
    assert_eq!(state.failures, 1);
}

#[test]
fn persistence_status_names_are_the_documented_snake_case_contract() {
    let status = PersistenceStatus {
        enabled: true,
        healthy: false,
        pending: true,
        saved: false,
        blocked: false,
        error: Some(WRITE_ERROR),
        generation: 3,
        attempted_generation: 2,
        saved_generation: 1,
        failures: 1,
    };
    let value = serde_json::to_value(&status).unwrap();
    let keys: Vec<&str> = value.as_object().unwrap().keys().map(String::as_str).collect();
    for expected in [
        "enabled",
        "healthy",
        "pending",
        "saved",
        "blocked",
        "error",
        "generation",
        "attempted_generation",
        "saved_generation",
        "failures",
    ] {
        assert!(keys.contains(&expected), "missing {expected}: {keys:?}");
    }
    assert_eq!(keys.len(), 10, "unexpected extra fields: {keys:?}");
}

#[test]
fn detached_engine_owner_does_not_prevent_independent_shutdown_flush() {
    let sandbox = Sandbox::new();
    let path = sandbox.path();
    let mut ledger = Ledger::open_at(&path);
    ledger.accept(&event(1), Some("one"));
    let handle = ledger.flush_handle().unwrap();
    let retained = Arc::new(Mutex::new(ledger));
    let detached_owner = retained.clone();
    assert!(handle.shutdown(Duration::from_secs(2)));
    assert!(handle.status().saved);
    assert_eq!(Ledger::open_at(&path).restored_events().len(), 1);
    assert_eq!(Arc::strong_count(&detached_owner), 2);
}
