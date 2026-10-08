use super::*;

fn engine() -> ActivityEngine {
    let (_, rx) = std::sync::mpsc::channel();
    let mut engine = ActivityEngine::new(Settings::default(), rx);
    engine.apply_demo(); // Deterministic snapshots only; no OS monitor or network.
    while engine.latest_event.is_some() {
        engine.ack_latest_event();
    }
    engine
}

#[test]
fn missing_is_not_zero_and_disabled_is_not_a_budget_crossing() {
    let mut engine = engine();
    engine
        .settings
        .tool_token_budgets
        .insert("codex".into(), 1000);
    let snapshot = engine
        .snapshots
        .iter_mut()
        .find(|row| row.id == "codex")
        .unwrap();
    snapshot.token_usage = None;
    let row = engine
        .tool_budget_report()
        .rows
        .into_iter()
        .find(|row| row.agent_id == "codex")
        .unwrap();
    assert_eq!(
        (row.used, row.status, row.source),
        (None, "unavailable", "unavailable")
    );
    engine
        .snapshots
        .iter_mut()
        .find(|row| row.id == "codex")
        .unwrap()
        .token_usage = Some(TokenUsage::default());
    let row = engine
        .tool_budget_report()
        .rows
        .into_iter()
        .find(|row| row.agent_id == "codex")
        .unwrap();
    assert_eq!(
        (row.used, row.status, row.source),
        (Some(0), "normal", "local")
    );
    engine.settings.disabled_agents.push("codex".into());
    let row = engine
        .tool_budget_report()
        .rows
        .into_iter()
        .find(|row| row.agent_id == "codex")
        .unwrap();
    assert_eq!(
        (row.used, row.status, row.source),
        (None, "unavailable", "disabled")
    );
    engine.evaluate_tool_budgets(1);
    assert!(engine.latest_event.is_none());
}

#[test]
fn tool_crossings_are_isolated_and_missing_samples_do_not_rearm() {
    let mut trackers = ToolBudgetTrackers::default();
    assert!(trackers.evaluate("a", 1000, Some(850), true, 1).is_some());
    assert!(trackers.evaluate("b", 1000, Some(850), true, 2).is_some());
    assert!(trackers.evaluate("a", 1000, None, true, 3).is_none());
    assert!(trackers.evaluate("a", 1000, Some(850), true, 4).is_none());
    assert!(trackers.evaluate("a", 1000, Some(1050), true, 5).unwrap().0);
    assert!(trackers.evaluate("a", 1000, Some(740), true, 6).is_none());
    assert!(trackers.evaluate("a", 1000, Some(850), true, 7).is_some());
    assert!(trackers.evaluate("b", 1000, Some(900), true, 8).is_none());
    assert!(
        trackers.evaluate("b", 500, Some(900), true, 9).unwrap().0,
        "changed limit gets a fresh crossing"
    );
}

#[test]
fn muted_alerts_do_not_consume_or_rearm_crossings() {
    let mut trackers = ToolBudgetTrackers::default();
    assert!(trackers.evaluate("a", 1000, Some(850), false, 1).is_none());
    assert!(trackers.evaluate("a", 1000, Some(850), true, 2).is_some());
    assert!(trackers.evaluate("a", 1000, Some(0), false, 3).is_none());
    assert!(trackers.evaluate("a", 1000, Some(850), true, 4).is_none());
}

#[test]
fn save_roundtrip_conflicts_and_other_settings_are_preserved() {
    let sandbox = crate::testutil::Sandbox::new("tool-budget-persistence");
    let mut engine = engine();
    engine
        .set_tool_budget("codex", 1000, 0, Some(sandbox.path()))
        .unwrap();
    engine
        .set_tool_budget("claude", 2000, 0, Some(sandbox.path()))
        .unwrap();
    assert!(engine
        .set_tool_budget("codex", 3000, 0, Some(sandbox.path()))
        .is_err());
    let stored = Settings::load_from(sandbox.path());
    assert_eq!(stored.tool_token_budgets.get("codex"), Some(&1000));
    assert_eq!(stored.tool_token_budgets.get("claude"), Some(&2000));
    assert_eq!(stored.appearance, engine.settings.appearance);
    engine
        .set_tool_budget("codex", 0, 1000, Some(sandbox.path()))
        .unwrap();
    assert!(!Settings::load_from(sandbox.path())
        .tool_token_budgets
        .contains_key("codex"));
}

#[test]
fn failed_write_and_invalid_inputs_do_not_publish() {
    let sandbox = crate::testutil::Sandbox::new("tool-budget-failed-save");
    let blocked = sandbox.path().join("not-a-directory");
    std::fs::write(&blocked, "fixture").unwrap();
    let mut engine = engine();
    assert!(engine
        .set_tool_budget("codex", 1000, 0, Some(&blocked))
        .is_err());
    assert!(engine.settings.tool_token_budgets.is_empty());
    for (id, value) in [
        ("unknown-tool", 1000),
        ("codex", -1),
        ("codex", 1_000_000_001),
    ] {
        assert!(engine.set_tool_budget(id, value, 0, None).is_err());
    }
    assert!(engine.settings.tool_token_budgets.is_empty());
}

#[test]
fn tool_events_use_the_tool_identity_and_removal_clears_tracking() {
    let mut engine = engine();
    engine.set_tool_budget("codex", 1000, 0, None).unwrap();
    engine
        .snapshots
        .iter_mut()
        .find(|row| row.id == "codex")
        .unwrap()
        .token_usage = Some(TokenUsage {
        tokens24h: 850,
        ..Default::default()
    });
    engine.evaluate_tool_budgets(1);
    let event = engine.latest_event.as_ref().unwrap();
    assert_eq!(event.agent_id, "codex");
    assert_eq!(event.message.as_deref(), Some("工具预算预警"));
    assert!(event.detail.as_ref().unwrap().contains("本机滚动 24h"));
    while engine.latest_event.is_some() {
        engine.ack_latest_event();
    }
    engine.evaluate_tool_budgets(2);
    assert!(engine.latest_event.is_none());
    engine.set_tool_budget("codex", 0, 1000, None).unwrap();
    engine.evaluate_tool_budgets(3);
    assert!(engine.tool_budgets.0.is_empty());
    engine.set_tool_budget("codex", 1000, 0, None).unwrap();
    engine.evaluate_tool_budgets(4);
    assert!(engine.latest_event.is_some());
    engine.settings.budget_alert_enabled = false;
    let row = engine
        .tool_budget_report()
        .rows
        .into_iter()
        .find(|row| row.agent_id == "codex")
        .unwrap();
    assert_eq!(
        row.status, "warning",
        "muting must not hide the actual threshold state"
    );
}

#[test]
fn settings_bound_the_map_and_generic_patches_cannot_overwrite_budgets() {
    let old: Settings = serde_json::from_str("{}").unwrap();
    assert!(old.tool_token_budgets.is_empty());
    let mut settings = old;
    for i in 0..150 {
        settings
            .tool_token_budgets
            .insert(format!("fixture-{i:03}"), 2_000_000_000);
    }
    settings.tool_token_budgets.insert("../invalid".into(), 50);
    settings.tool_token_budgets.insert("negative".into(), -10);
    let settings = settings.normalized();
    assert_eq!(settings.tool_token_budgets.len(), 128);
    assert!(settings
        .tool_token_budgets
        .values()
        .all(|value| *value == 1_000_000_000));
    assert!(settings
        .patched(serde_json::json!({"tool_token_budgets": {}}))
        .is_err());
    assert!(settings
        .patched(serde_json::json!({"appearance": "dark"}))
        .is_ok());
}

#[test]
fn adding_a_budget_never_exceeds_the_persistent_capacity() {
    let mut engine = engine();
    for i in 0..128 {
        engine
            .settings
            .tool_token_budgets
            .insert(format!("fixture-{i}"), 1000);
    }
    assert!(engine.set_tool_budget("codex", 1000, 0, None).is_err());
    assert_eq!(engine.settings.tool_token_budgets.len(), 128);
}
