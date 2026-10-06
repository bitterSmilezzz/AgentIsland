use super::*;
#[cfg(unix)]
use std::os::unix::fs::{symlink, PermissionsExt};
struct Fixture {
    _sandbox: crate::testutil::Sandbox,
    config: PathBuf,
    parent: PathBuf,
    binary: PathBuf,
    store: Store,
}
impl Fixture {
    fn new() -> Self {
        let sandbox = crate::testutil::Sandbox::new("claude-hook-config");
        let root = sandbox.path().canonicalize().unwrap();
        let config = root.join("claude");
        let parent = root.join("app");
        std::fs::create_dir(&config).unwrap();
        std::fs::create_dir(&parent).unwrap();
        let binary = root.join("agentisland");
        std::fs::write(&binary, b"fixture-only").unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
        let store = Store::new(
            config.clone(),
            parent.clone(),
            binary.clone(),
            root.join(".Trash"),
        );
        Self {
            _sandbox: sandbox,
            config,
            parent,
            binary,
            store,
        }
    }
    fn put(&self, value: Value) {
        std::fs::write(
            self.config.join("settings.json"),
            serde_json::to_vec(&value).unwrap(),
        )
        .unwrap();
    }
    fn value(&self) -> Value {
        serde_json::from_slice(&std::fs::read(self.config.join("settings.json")).unwrap()).unwrap()
    }
    fn preview(&mut self, action: Action) -> Preview {
        let inspect = self.store.inspect().unwrap();
        self.store.preview(action, &inspect.revision, None).unwrap()
    }
}
#[test]
fn install_cancel_disable_and_restart_restore_preserve_unknown_settings() {
    let mut f = Fixture::new();
    let original = serde_json::json!({"theme":"dark","permissions":{"allow":["Read"]},"hooks":{"PostToolUse":[{"matcher":"Read","hooks":[{"type":"command","command":"/bin/true"}]}]}});
    f.put(original.clone());
    let before = std::fs::read(f.config.join("settings.json")).unwrap();
    let p = f.preview(Action::Enable);
    f.store.cancel();
    assert!(f.store.apply(&p.plan_id).is_err());
    assert_eq!(
        std::fs::read(f.config.join("settings.json")).unwrap(),
        before
    );
    assert!(!f.parent.join("claude-plan-hooks").exists());
    let p = f.preview(Action::Enable);
    let enabled = f.store.apply(&p.plan_id).unwrap();
    assert!(enabled.verified && enabled.installed);
    assert_eq!(f.value()["permissions"], original["permissions"]);
    assert_eq!(
        f.value()["hooks"]["PostToolUse"],
        original["hooks"]["PostToolUse"]
    );
    assert!(f.store.apply(&p.plan_id).is_err());
    let p = f.preview(Action::Disable);
    let disabled = f.store.apply(&p.plan_id).unwrap();
    assert!(!disabled.installed);
    assert_eq!(f.value()["theme"], original["theme"]);
    f.store = Store::new(
        f.config.clone(),
        f.parent.clone(),
        f.binary.clone(),
        f.parent.parent().unwrap().join(".Trash"),
    );
    let inspect = f.store.inspect().unwrap();
    assert!(
        inspect
            .backup_records
            .iter()
            .find(|record| record.id == disabled.backup_id)
            .unwrap()
            .restore_available
    );
    assert!(
        !inspect
            .backup_records
            .iter()
            .find(|record| record.id == enabled.backup_id)
            .unwrap()
            .restore_available
    );
    let p = f
        .store
        .preview(
            Action::Restore,
            &inspect.revision,
            Some(&disabled.backup_id),
        )
        .unwrap();
    let restored = f.store.apply(&p.plan_id).unwrap();
    assert!(restored.installed);
    assert!(!enabled.backup_id.is_empty());
}
#[test]
fn missing_settings_file_is_restored_to_absence_not_an_empty_placeholder() {
    let mut f = Fixture::new();
    let p = f.preview(Action::Enable);
    let applied = f.store.apply(&p.plan_id).unwrap();
    let state = f.store.inspect().unwrap();
    let p = f
        .store
        .preview(Action::Restore, &state.revision, Some(&applied.backup_id))
        .unwrap();
    f.store.apply(&p.plan_id).unwrap();
    assert!(!f.config.join("settings.json").exists());
    assert!(!f.store.inspect().unwrap().installed);
}
#[test]
fn external_config_change_and_root_replacement_reject_before_write() {
    let mut f = Fixture::new();
    f.put(serde_json::json!({}));
    let p = f.preview(Action::Enable);
    f.put(serde_json::json!({"theme":"external"}));
    assert_eq!(f.store.apply(&p.plan_id).err().unwrap().code, "changed");
    assert_eq!(f.value()["theme"], "external");
    let p = f.preview(Action::Enable);
    let moved = f.config.with_extension("moved");
    std::fs::rename(&f.config, &moved).unwrap();
    std::fs::create_dir(&f.config).unwrap();
    f.put(serde_json::json!({}));
    assert_eq!(f.store.apply(&p.plan_id).err().unwrap().code, "changed");
    assert_eq!(f.value(), serde_json::json!({}));
    assert!(!f.parent.join("claude-plan-hooks").exists());
}
#[test]
fn auth_fields_and_unclassified_environment_values_never_create_backups() {
    for value in [
        serde_json::json!({"env":{"ANTHROPIC_AUTH_TOKEN":"opaque-fixture-value"}}),
        serde_json::json!({"env":{"UNCLASSIFIED":"opaque-fixture-value"}}),
        serde_json::json!({"apiKey":"opaque-fixture-value"}),
    ] {
        let mut f = Fixture::new();
        f.put(value);
        let state = f.store.inspect().unwrap();
        assert!(!state.writable);
        assert_eq!(
            f.store
                .preview(Action::Enable, &state.revision, None)
                .err()
                .unwrap()
                .code,
            "private"
        );
        assert!(!f.parent.join("claude-plan-hooks").exists());
    }
    let mut f = Fixture::new();
    f.put(serde_json::json!({"env":{"AUTH_REFERENCE":"env:FIXTURE_AUTH"}}));
    let p = f.preview(Action::Enable);
    assert!(f.store.apply(&p.plan_id).unwrap().verified);
}
#[test]
fn unowned_or_duplicated_hook_is_never_adopted_or_removed() {
    let mut f = Fixture::new();
    let command = f.store.command().unwrap();
    f.put(edit(serde_json::json!({}), true, &command).unwrap());
    assert_eq!(f.store.inspect().err().unwrap().code, "ownership");
    f.put(serde_json::json!({}));
    let p = f.preview(Action::Enable);
    f.store.apply(&p.plan_id).unwrap();
    let current = f.value();
    f.put(edit(current, true, &command).unwrap());
    assert_eq!(f.store.inspect().err().unwrap().code, "ownership");
}
#[test]
fn adding_unrelated_hook_to_owned_group_is_preserved_when_ours_is_removed() {
    let mut f = Fixture::new();
    let p = f.preview(Action::Enable);
    f.store.apply(&p.plan_id).unwrap();
    let mut value = f.value();
    value["hooks"]["PreToolUse"][0]["hooks"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({"type":"command","command":"/bin/true"}));
    value["hooks"]["PreToolUse"][0]["extra"] = "keep".into();
    f.put(value);
    let p = f.preview(Action::Disable);
    f.store.apply(&p.plan_id).unwrap();
    assert_eq!(f.value()["hooks"]["PreToolUse"][0]["extra"], "keep");
    assert_eq!(
        f.value()["hooks"]["PreToolUse"][0]["hooks"][0]["command"],
        "/bin/true"
    );
}
#[test]
fn duplicate_json_keys_and_symlink_targets_are_refused() {
    let mut f = Fixture::new();
    std::fs::write(
        f.config.join("settings.json"),
        br#"{"hooks":{},"hooks":{}}"#,
    )
    .unwrap();
    assert_eq!(f.store.inspect().err().unwrap().code, "invalid");
    std::fs::remove_file(f.config.join("settings.json")).unwrap();
    symlink(&f.binary, f.config.join("settings.json")).unwrap();
    assert!(f.store.inspect().is_err());
    assert_eq!(std::fs::read(&f.binary).unwrap(), b"fixture-only");
}
#[test]
fn owned_hook_can_be_removed_or_migrated_after_application_path_changes() {
    let mut f = Fixture::new();
    let p = f.preview(Action::Enable);
    f.store.apply(&p.plan_id).unwrap();
    let moved_binary = f.binary.with_extension("moved");
    std::fs::rename(&f.binary, &moved_binary).unwrap();
    f.store = Store::new(
        f.config.clone(),
        f.parent.clone(),
        moved_binary,
        f.parent.parent().unwrap().join(".Trash"),
    );
    assert!(f.store.inspect().unwrap().installed);
    let p = f.preview(Action::Enable);
    f.store.apply(&p.plan_id).unwrap();
    let command = f.store.command().unwrap();
    assert_eq!(
        f.value()["hooks"]["PreToolUse"][0]["hooks"][0]["command"],
        command
    );
    let p = f.preview(Action::Disable);
    assert!(!f.store.apply(&p.plan_id).unwrap().installed);
}
#[test]
fn equal_bytes_at_replaced_file_identity_invalidate_inspection_revision() {
    let mut f = Fixture::new();
    f.put(serde_json::json!({}));
    let revision = f.store.inspect().unwrap().revision;
    let old = f.config.join("settings.json");
    std::fs::rename(&old, f.config.join("old-settings")).unwrap();
    f.put(serde_json::json!({}));
    assert_eq!(
        f.store
            .preview(Action::Enable, &revision, None)
            .err()
            .unwrap()
            .code,
        "changed"
    );
    assert!(!f.parent.join("claude-plan-hooks").exists());
}
#[test]
fn malformed_env_and_forged_backup_owner_never_get_written() {
    let mut f = Fixture::new();
    f.put(serde_json::json!({"env":"opaque-fixture-value"}));
    assert!(!f.store.inspect().unwrap().writable);
    f.put(serde_json::json!({}));
    let p = f.preview(Action::Enable);
    let applied = f.store.apply(&p.plan_id).unwrap();
    let receipt = f
        .parent
        .join("claude-plan-hooks")
        .join(&applied.backup_id)
        .join("receipt.json");
    let mut value: Value = serde_json::from_slice(&std::fs::read(&receipt).unwrap()).unwrap();
    value["owner_before"] = serde_json::json!({"schema_version":2,"command":"unverified-owner"});
    std::fs::write(receipt, serde_json::to_vec(&value).unwrap()).unwrap();
    let before = std::fs::read(f.config.join("settings.json")).unwrap();
    assert_eq!(f.store.inspect().err().unwrap().code, "invalid");
    assert_eq!(
        std::fs::read(f.config.join("settings.json")).unwrap(),
        before
    );
}
#[test]
fn expiration_and_wrong_confirmation_consume_preview_without_writing() {
    let mut f = Fixture::new();
    let p = f.preview(Action::Enable);
    f.store.pending.as_mut().unwrap().created = Instant::now() - TTL;
    assert_eq!(f.store.apply(&p.plan_id).err().unwrap().code, "expired");
    let p = f.preview(Action::Enable);
    assert_eq!(
        f.store.apply("wrong-confirmation").err().unwrap().code,
        "expired"
    );
    assert_eq!(f.store.apply(&p.plan_id).err().unwrap().code, "expired");
    assert!(!f.config.join("settings.json").exists());
    assert!(!f.parent.join("claude-plan-hooks").exists());
}
#[test]
fn application_binary_replacement_invalidates_confirmation() {
    let mut f = Fixture::new();
    let p = f.preview(Action::Enable);
    std::fs::rename(&f.binary, f.binary.with_extension("old")).unwrap();
    std::fs::write(&f.binary, b"fixture-only").unwrap();
    std::fs::set_permissions(&f.binary, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(f.store.apply(&p.plan_id).err().unwrap().code, "changed");
    assert!(!f.config.join("settings.json").exists());
}
#[test]
fn cleanup_preview_cancel_and_apply_preserve_config_and_move_exact_record() {
    let mut f = Fixture::new();
    let p = f.preview(Action::Enable);
    let applied = f.store.apply(&p.plan_id).unwrap();
    let record = f.parent.join("claude-plan-hooks").join(&applied.backup_id);
    let before = std::fs::read(record.join("receipt.json")).unwrap();
    let config = std::fs::read(f.config.join("settings.json")).unwrap();
    let owner = std::fs::read(f.parent.join("claude-plan-hooks/owner.json")).unwrap();
    let state = f.store.inspect().unwrap();
    let p = f
        .store
        .preview(
            Action::TrashBackup,
            &state.revision,
            Some(&applied.backup_id),
        )
        .unwrap();
    f.store.cancel();
    assert!(f.store.apply(&p.plan_id).is_err());
    assert!(record.exists());
    assert!(!f.store.trash.exists());
    let state = f.store.inspect().unwrap();
    let p = f
        .store
        .preview(
            Action::TrashBackup,
            &state.revision,
            Some(&applied.backup_id),
        )
        .unwrap();
    assert!(f.store.apply(&p.plan_id).unwrap().verified);
    assert!(!record.exists());
    assert!(f.store.inspect().unwrap().backups.is_empty());
    assert_eq!(
        std::fs::read(f.config.join("settings.json")).unwrap(),
        config
    );
    assert_eq!(
        std::fs::read(f.parent.join("claude-plan-hooks/owner.json")).unwrap(),
        owner
    );
    let moved = std::fs::read_dir(&f.store.trash)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    assert_eq!(std::fs::read(moved.join("receipt.json")).unwrap(), before);
    assert!(f.store.apply(&p.plan_id).is_err());
}
#[test]
fn full_capacity_can_be_released_without_creating_another_backup() {
    let mut f = Fixture::new();
    for i in 0..LIMIT {
        let p = f.preview(if i % 2 == 0 {
            Action::Enable
        } else {
            Action::Disable
        });
        f.store.apply(&p.plan_id).unwrap();
    }
    let state = f.store.inspect().unwrap();
    assert_eq!(state.backups.len(), LIMIT);
    assert_eq!(
        f.store
            .preview(Action::Enable, &state.revision, None)
            .err()
            .unwrap()
            .code,
        "capacity"
    );
    let p = f
        .store
        .preview(
            Action::TrashBackup,
            &state.revision,
            Some(&state.backups[0]),
        )
        .unwrap();
    f.store.apply(&p.plan_id).unwrap();
    assert_eq!(f.store.inspect().unwrap().backups.len(), LIMIT - 1);
    let p = f.preview(Action::Enable);
    assert!(f.store.apply(&p.plan_id).unwrap().verified);
}
#[test]
fn changed_record_unknown_contents_or_symlink_trash_are_never_moved() {
    for case in 0..3 {
        let mut f = Fixture::new();
        let p = f.preview(Action::Enable);
        let applied = f.store.apply(&p.plan_id).unwrap();
        let record = f.parent.join("claude-plan-hooks").join(&applied.backup_id);
        let state = f.store.inspect().unwrap();
        let p = f
            .store
            .preview(
                Action::TrashBackup,
                &state.revision,
                Some(&applied.backup_id),
            )
            .unwrap();
        match case {
            0 => {
                std::fs::write(record.join("receipt.json"), b"{}").unwrap();
            }
            1 => {
                std::fs::write(record.join("other"), b"fixture-only").unwrap();
            }
            _ => {
                symlink(&f.config, &f.store.trash).unwrap();
            }
        }
        let config = std::fs::read(f.config.join("settings.json")).unwrap();
        assert!(f.store.apply(&p.plan_id).is_err());
        assert!(record.exists());
        assert_eq!(
            std::fs::read(f.config.join("settings.json")).unwrap(),
            config
        );
    }
}
#[test]
fn cleanup_does_not_backup_credentials_added_to_current_configuration() {
    let mut f = Fixture::new();
    let p = f.preview(Action::Enable);
    let applied = f.store.apply(&p.plan_id).unwrap();
    let mut value = f.value();
    value["env"] = serde_json::json!({"AUTH_VALUE":"opaque-fixture-only"});
    f.put(value);
    let before = std::fs::read(f.config.join("settings.json")).unwrap();
    let state = f.store.inspect().unwrap();
    assert!(!state.writable);
    let p = f
        .store
        .preview(
            Action::TrashBackup,
            &state.revision,
            Some(&applied.backup_id),
        )
        .unwrap();
    f.store.apply(&p.plan_id).unwrap();
    assert_eq!(
        std::fs::read(f.config.join("settings.json")).unwrap(),
        before
    );
    let moved = std::fs::read_dir(&f.store.trash)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    assert!(!std::fs::read_to_string(moved.join("receipt.json"))
        .unwrap()
        .contains("opaque-fixture-only"));
    assert!(f.store.inspect().unwrap().backups.is_empty());
}
#[test]
fn replaced_trash_or_newly_created_destination_invalidates_cleanup_preview() {
    for existed in [false, true] {
        let mut f = Fixture::new();
        if existed {
            std::fs::create_dir(&f.store.trash).unwrap();
        }
        let p = f.preview(Action::Enable);
        let applied = f.store.apply(&p.plan_id).unwrap();
        let state = f.store.inspect().unwrap();
        let p = f
            .store
            .preview(
                Action::TrashBackup,
                &state.revision,
                Some(&applied.backup_id),
            )
            .unwrap();
        if existed {
            std::fs::rename(&f.store.trash, f.store.trash.with_extension("old")).unwrap();
        }
        std::fs::create_dir(&f.store.trash).unwrap();
        assert_eq!(f.store.apply(&p.plan_id).err().unwrap().code, "changed");
        assert!(f
            .parent
            .join("claude-plan-hooks")
            .join(&applied.backup_id)
            .exists());
        assert!(std::fs::read_dir(&f.store.trash).unwrap().next().is_none());
    }
}
#[test]
fn background_status_read_preserves_live_confirmation_without_writing() {
    let mut f = Fixture::new();
    let p = f.preview(Action::Enable);
    assert!(!f.store.observe().unwrap().installed);
    assert!(!f.config.join("settings.json").exists());
    assert!(!f.parent.join("claude-plan-hooks").exists());
    assert!(f.store.apply(&p.plan_id).unwrap().installed);
    let p = f.preview(Action::Disable);
    f.put(serde_json::json!({"theme":"external"}));
    assert!(!f.store.observe().unwrap().installed);
    assert_eq!(f.store.apply(&p.plan_id).err().unwrap().code, "changed");
    assert_eq!(f.value()["theme"], "external");
}
