#![cfg(target_os = "macos")]
use super::*;
use std::{
    fs,
    os::unix::fs::{symlink, PermissionsExt},
};
fn fixture() -> (crate::testutil::Sandbox, PathBuf, PathBuf) {
    let sandbox = crate::testutil::Sandbox::new("skills-package");
    let home = sandbox.path().join("home");
    let source = sandbox.path().join("source");
    fs::create_dir(&home).unwrap();
    fs::create_dir(&source).unwrap();
    fs::write(source.join("SKILL.md"),"---\nname: fixture-skill\ndescription: Test a bounded local installation.\n---\nFixture instructions.\n").unwrap();
    (sandbox, home, source)
}
#[test]
fn install_persists_complete_resources_and_restore_survives_store_restart() {
    let (_sandbox, home, source) = fixture();
    fs::create_dir(source.join("scripts")).unwrap();
    fs::create_dir(source.join("empty")).unwrap();
    fs::write(source.join("scripts/run.sh"), "exit 0\n").unwrap();
    fs::set_permissions(
        source.join("scripts/run.sh"),
        fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    let mut store = Store::new(home.clone());
    let preview = store.preview(&source, 1).unwrap();
    assert!(!home.join(".agents").exists());
    assert_eq!(preview.action, "install");
    assert_eq!(preview.files, 2);
    let installed = store.apply(&preview.plan_id, 2).unwrap();
    let target = home.join(".agents/skills/fixture-skill");
    assert_eq!(
        fs::read(target.join("scripts/run.sh")).unwrap(),
        b"exit 0\n"
    );
    assert!(target.join("empty").is_dir());
    assert_eq!(
        fs::metadata(target.join("scripts/run.sh"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    let store = Store::new(home.clone());
    let rows = store.recoveries().unwrap();
    assert_eq!(rows[0].state, "applied");
    let undo = store
        .restore_preview(&installed.id, &rows[0].revision)
        .unwrap();
    assert_eq!(undo.action, "remove");
    assert_eq!(undo.files, 0);
    assert!(undo
        .changes
        .iter()
        .any(|change| change.path == "SKILL.md" && change.kind == "removed"));
    store.restore(&installed.id, &rows[0].revision).unwrap();
    assert!(!target.exists());
    assert_eq!(store.recoveries().unwrap()[0].state, "inactive");
    assert!(source.join("scripts/run.sh").exists());
    assert!(home
        .join(".agents/.agentisland-skill-installs")
        .join(installed.id)
        .join("package/SKILL.md")
        .exists());
}
#[test]
fn update_diff_and_restore_keep_both_complete_versions() {
    let (_sandbox, home, source) = fixture();
    fs::write(source.join("old.txt"), "previous").unwrap();
    let mut store = Store::new(home.clone());
    let first = store.preview(&source, 1).unwrap();
    store.apply(&first.plan_id, 2).unwrap();
    fs::remove_file(source.join("old.txt")).unwrap();
    fs::write(source.join("new.txt"), "current").unwrap();
    let preview = store.preview(&source, 3).unwrap();
    assert_eq!(preview.action, "update");
    assert!(preview
        .changes
        .iter()
        .any(|c| c.path == "old.txt" && c.kind == "removed"));
    assert!(preview
        .changes
        .iter()
        .any(|c| c.path == "new.txt" && c.kind == "added"));
    let updated = store.apply(&preview.plan_id, 4).unwrap();
    let target = home.join(".agents/skills/fixture-skill");
    assert!(!target.join("old.txt").exists());
    assert_eq!(fs::read(target.join("new.txt")).unwrap(), b"current");
    let rows = Store::new(home.clone()).recoveries().unwrap();
    assert_eq!(rows[0].id, updated.id);
    assert_eq!(rows[0].state, "applied");
    assert_eq!(rows[1].state, "conflict");
    let undo = store
        .restore_preview(&updated.id, &rows[0].revision)
        .unwrap();
    assert_eq!(undo.action, "restore");
    assert!(undo
        .changes
        .iter()
        .any(|change| change.path == "old.txt" && change.kind == "added"));
    assert!(undo
        .changes
        .iter()
        .any(|change| change.path == "new.txt" && change.kind == "removed"));
    store.restore(&updated.id, &rows[0].revision).unwrap();
    assert_eq!(fs::read(target.join("old.txt")).unwrap(), b"previous");
    assert!(!target.join("new.txt").exists());
    assert!(home
        .join(".agents/.agentisland-skill-installs")
        .join(updated.id)
        .join("package/new.txt")
        .exists());
    assert!(store
        .recoveries()
        .unwrap()
        .iter()
        .all(|row| row.state != "conflict"));
}
#[test]
fn source_and_target_changes_expiry_and_cancellation_prevent_publication() {
    let (_sandbox, home, source) = fixture();
    let mut store = Store::new(home.clone());
    let preview = store.preview(&source, 10).unwrap();
    fs::write(source.join("new.txt"), "changed").unwrap();
    assert!(store.apply(&preview.plan_id, 11).is_err());
    assert!(!home.join(".agents").exists());
    let preview = store.preview(&source, 12).unwrap();
    assert!(store.apply(&preview.plan_id, 12 + PLAN_TTL + 1).is_err());
    assert!(store.apply(&preview.plan_id, 11).is_err());
    store.cancel(&preview.plan_id);
    assert!(store.apply(&preview.plan_id, 13).is_err());
    assert!(!home.join(".agents").exists());
    let preview = store.preview(&source, 14).unwrap();
    fs::create_dir_all(home.join(".agents/skills/fixture-skill")).unwrap();
    fs::write(
        home.join(".agents/skills/fixture-skill/keep.txt"),
        "external",
    )
    .unwrap();
    assert!(store.apply(&preview.plan_id, 15).is_err());
    assert_eq!(
        fs::read(home.join(".agents/skills/fixture-skill/keep.txt")).unwrap(),
        b"external"
    );
}
#[test]
fn drift_of_live_or_retained_package_blocks_restore_without_overwrite() {
    let (_sandbox, home, source) = fixture();
    let mut store = Store::new(home.clone());
    let preview = store.preview(&source, 1).unwrap();
    let installed = store.apply(&preview.plan_id, 2).unwrap();
    let row = store.recoveries().unwrap().remove(0);
    let target = home.join(".agents/skills/fixture-skill");
    fs::write(target.join("user.txt"), "later edit").unwrap();
    assert!(store.restore(&installed.id, &row.revision).is_err());
    assert_eq!(store.recoveries().unwrap()[0].state, "conflict");
    assert_eq!(fs::read(target.join("user.txt")).unwrap(), b"later edit");
    fs::remove_file(target.join("user.txt")).unwrap();
    fs::write(source.join("next.txt"), "next").unwrap();
    let preview = store.preview(&source, 3).unwrap();
    let updated = store.apply(&preview.plan_id, 4).unwrap();
    let row = store.recoveries().unwrap().remove(0);
    fs::write(
        home.join(".agents/.agentisland-skill-installs")
            .join(&updated.id)
            .join("package/user.txt"),
        "backup edit",
    )
    .unwrap();
    assert!(store.restore(&updated.id, &row.revision).is_err());
    assert!(target.join("next.txt").exists());
}
#[test]
fn symlinks_fifo_and_hidden_credentials_are_rejected_before_any_install() {
    let (_sandbox, home, source) = fixture();
    let mut store = Store::new(home.clone());
    let outside = home.join("outside.txt");
    fs::write(&outside, "outside").unwrap();
    symlink(&outside, source.join("link.txt")).unwrap();
    assert!(store.preview(&source, 1).is_err());
    fs::remove_file(source.join("link.txt")).unwrap();
    let fifo = std::ffi::CString::new(source.join("pipe").to_str().unwrap()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
    assert!(store.preview(&source, 1).is_err());
    fs::remove_file(source.join("pipe")).unwrap();
    fs::write(source.join(".env"), "fixture").unwrap();
    assert!(store.preview(&source, 1).is_err());
    fs::remove_file(source.join(".env")).unwrap();
    let private = format!("{}{}", "sk-", "a".repeat(48));
    fs::write(source.join("private.txt"), private).unwrap();
    let error = store.preview(&source, 1).unwrap_err();
    assert!(!error.contains(&"a".repeat(48)));
    assert!(!home.join(".agents").exists());
    assert_eq!(fs::read(outside).unwrap(), b"outside");
}
#[test]
fn linked_target_and_linked_storage_are_not_followed() {
    let (_sandbox, home, source) = fixture();
    let mut store = Store::new(home.clone());
    let preview = store.preview(&source, 1).unwrap();
    fs::create_dir(home.join("other")).unwrap();
    symlink(home.join("other"), home.join(".agents")).unwrap();
    assert!(store.apply(&preview.plan_id, 2).is_err());
    assert_eq!(fs::read_dir(home.join("other")).unwrap().count(), 0);
    fs::remove_file(home.join(".agents")).unwrap();
    fs::create_dir_all(home.join(".agents/skills")).unwrap();
    symlink(&source, home.join(".agents/skills/fixture-skill")).unwrap();
    assert!(store.preview(&source, 3).is_err());
    assert_eq!(fs::read_dir(&source).unwrap().count(), 1);
}
#[test]
fn publish_exclusive_target_and_staging_failure_keep_prior_entries() {
    let (_sandbox, home, source) = fixture();
    let home_dir = files::open_dir(&home).unwrap();
    let source_dir = files::open_dir(&source).unwrap();
    fs::create_dir(home.join("existing")).unwrap();
    fs::write(home.join("existing/keep.txt"), "keep").unwrap();
    assert!(files::publish(&source_dir, "SKILL.md", &home_dir, "existing", false).is_err());
    assert!(source.join("SKILL.md").exists());
    assert_eq!(fs::read(home.join("existing/keep.txt")).unwrap(), b"keep");
    let tree = files::scan(&source_dir).unwrap();
    let stage = files::create_dir(&home_dir, "stage").unwrap();
    files::write_new(&stage, "SKILL.md", b"existing", false).unwrap();
    assert!(files::stage(&stage, &tree).is_err());
    assert_eq!(fs::read(home.join("stage/SKILL.md")).unwrap(), b"existing");
}
#[test]
fn unreadable_partial_record_does_not_hide_healthy_recovery() {
    let (_sandbox, home, source) = fixture();
    let mut store = Store::new(home.clone());
    let preview = store.preview(&source, 1).unwrap();
    store.apply(&preview.plan_id, 2).unwrap();
    let id = uuid::Uuid::new_v4().to_string();
    fs::create_dir(home.join(".agents/.agentisland-skill-installs").join(id)).unwrap();
    let rows = store.recoveries().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].state, "applied");
    assert_eq!(rows[1].state, "unreadable");
    let json = serde_json::to_string(&rows).unwrap();
    assert!(!json.contains(home.to_str().unwrap()));
    assert!(!json.contains(source.to_str().unwrap()));
    assert!(!json.contains("Fixture instructions"));
}
#[test]
fn limits_and_metadata_fail_without_silent_truncation() {
    let (_sandbox, home, source) = fixture();
    let mut store = Store::new(home.clone());
    fs::write(source.join("large.bin"), vec![0; 4 * 1024 * 1024 + 1]).unwrap();
    assert!(store.preview(&source, 1).is_err());
    fs::remove_file(source.join("large.bin")).unwrap();
    for i in 0..256 {
        fs::write(source.join(format!("file-{i}.txt")), "x").unwrap();
    }
    assert!(store.preview(&source, 1).is_err());
    for i in 0..256 {
        fs::remove_file(source.join(format!("file-{i}.txt"))).unwrap();
    }
    fs::write(
        source.join("SKILL.md"),
        "---\nname: fixture-skill\nname: duplicate\ndescription: fixture\n---\n",
    )
    .unwrap();
    assert!(store.preview(&source, 1).is_err());
    fs::write(
        source.join("SKILL.md"),
        "---\nname: ../escape\ndescription: fixture\n---\n",
    )
    .unwrap();
    assert!(store.preview(&source, 1).is_err());
    assert!(!home.join(".agents").exists());
}
#[test]
fn total_bytes_depth_and_diff_display_limit_preserve_full_operation() {
    let (_sandbox, home, source) = fixture();
    let mut store = Store::new(home.clone());
    for i in 0..8 {
        fs::write(
            source.join(format!("asset-{i}.bin")),
            vec![0; 4 * 1024 * 1024],
        )
        .unwrap();
    }
    assert!(store.preview(&source, 1).is_err());
    for i in 0..8 {
        fs::remove_file(source.join(format!("asset-{i}.bin"))).unwrap();
    }
    let mut nested = source.clone();
    for _ in 0..13 {
        nested = nested.join("nested");
        fs::create_dir(&nested).unwrap();
    }
    assert!(store.preview(&source, 1).is_err());
    fs::remove_dir_all(source.join("nested")).unwrap();
    for i in 0..105 {
        fs::write(source.join(format!("asset-{i}.txt")), "fixture").unwrap();
    }
    let preview = store.preview(&source, 1).unwrap();
    assert_eq!(preview.change_count, 106);
    assert_eq!(preview.changes.len(), 100);
    store.apply(&preview.plan_id, 2).unwrap();
    assert_eq!(
        fs::read_dir(home.join(".agents/skills/fixture-skill"))
            .unwrap()
            .count(),
        106
    );
}

#[test]
fn yaml_metadata_supports_multiline_quotes_comments_aliases_and_merges_without_rewriting() {
    let (_sandbox, home, source) = fixture();
    let cases=[
        "name: fixture-skill\ndescription: |\n  A developer's workflow.\n  Run when the user says: check it.\n",
        "name: 'fixture-skill' # target\ndescription: >-\n  Use a folded\n  description.\nmetadata: {category: coding, examples: [one, two]}\n",
        "name: fixture-skill\ndescription: \"A \\\"quoted\\\" example with \\u4f60\\u597d.\"\n",
        "name: fixture-skill\ndescription: 'A developer''s workflow'\n",
        "description: &text A shared description\nname: fixture-skill\nmetadata: {summary: *text}\n",
        "defaults: &defaults {name: fixture-skill, description: A merged description}\n<<: *defaults\n",
        "{name: fixture-skill, description: 'Flow metadata with: punctuation'}\n",
    ];
    for (i, metadata) in cases.iter().enumerate() {
        let content =
            format!("---\n{metadata}---\nInstructions remain unchanged.\n---\nBody rule.\n");
        fs::write(source.join("SKILL.md"), &content).unwrap();
        let mut store = Store::new(home.clone());
        let preview = store.preview(&source, 10 + i as i64).unwrap();
        let applied = store.apply(&preview.plan_id, 20 + i as i64).unwrap();
        assert_eq!(
            fs::read(home.join(".agents/skills/fixture-skill/SKILL.md")).unwrap(),
            content.as_bytes()
        );
        let row = store
            .recoveries()
            .unwrap()
            .into_iter()
            .find(|row| row.id == applied.id)
            .unwrap();
        store.restore(&row.id, &row.revision).unwrap();
    }
    let content =
        "\u{feff}---\r\nname: fixture-skill\r\ndescription: A CRLF document\r\n...\r\nBody\r\n";
    fs::write(source.join("SKILL.md"), content).unwrap();
    assert_eq!(
        Store::new(home).preview(&source, 100).unwrap().name,
        "fixture-skill"
    );
}
#[test]
fn yaml_metadata_rejects_ambiguous_types_duplicates_tags_and_invalid_documents_without_excerpts() {
    let (_sandbox, home, source) = fixture();
    let bad = [
        "name: fixture-skill\ndescription: [not, text]\n",
        "name: fixture-skill\ndescription: 42\n",
        "name: fixture-skill\ndescription: null\n",
        "name: fixture-skill\ndescription: false\n",
        "name: fixture-skill\ndescription: '  '\n",
        "name: 123\ndescription: Text\n",
        "name: fixture-skill\ndescription: One\ndescription: Two\n",
        "name: fixture-skill\ndescription: One\nmetadata: {category: one, category: two}\n",
        "name: fixture-skill\ndescription: !include outside.txt\n",
        "name: fixture-skill\ndescription: !custom Redacted fixture value\n",
        "name: fixture-skill\ndescription: *unknown\n",
        "name: fixture-skill\ndescription: [broken\n",
        "- name: fixture-skill\n- description: Text\n",
    ];
    for metadata in bad {
        fs::write(
            source.join("SKILL.md"),
            format!("---\n{metadata}---\nBody\n"),
        )
        .unwrap();
        let error = Store::new(home.clone()).preview(&source, 1).unwrap_err();
        assert!(!error.contains("Redacted fixture value"));
        assert!(!error.contains(source.to_str().unwrap()));
        assert!(!home.join(".agents").exists());
    }
    fs::write(
        source.join("SKILL.md"),
        "---\nname: fixture-skill\ndescription: Text\n",
    )
    .unwrap();
    assert!(Store::new(home).preview(&source, 1).is_err());
}
#[test]
fn yaml_budget_limits_metadata_not_instruction_body_and_blocks_alias_amplification() {
    let (_sandbox, home, source) = fixture();
    let mut store = Store::new(home.clone());
    fs::write(
        source.join("SKILL.md"),
        format!(
            "---\nname: fixture-skill\ndescription: Text\n---\n{}",
            "body\n".repeat(20000)
        ),
    )
    .unwrap();
    assert!(store.preview(&source, 1).is_ok());
    fs::write(
        source.join("SKILL.md"),
        format!(
            "---\nname: fixture-skill\ndescription: {}\n---\n",
            "x".repeat(MAX_METADATA)
        ),
    )
    .unwrap();
    assert!(store.preview(&source, 1).unwrap_err().contains("64 KiB"));
    fs::write(
        source.join("SKILL.md"),
        format!(
            "---\nname: fixture-skill\ndescription: Text\nmetadata: {}0{}\n---\n",
            "[".repeat(20),
            "]".repeat(20)
        ),
    )
    .unwrap();
    assert!(store.preview(&source, 1).is_err());
    fs::write(source.join("SKILL.md"),format!("---\nname: fixture-skill\ndescription: Text\nseed: &seed [one, two]\nmetadata: [{}]\n---\n",vec!["*seed";100].join(","))).unwrap();
    assert!(store.preview(&source, 1).is_err());
    assert!(!home.join(".agents").exists());
}
#[test]
fn update_requires_existing_skill_metadata_to_match_directory_identity() {
    let (_sandbox, home, source) = fixture();
    let target = home.join(".agents/skills/fixture-skill");
    fs::create_dir_all(&target).unwrap();
    let originals = [
        "---\nname: another-skill\ndescription: Existing\n---\n",
        "---\nname: fixture-skill\ndescription: [invalid]\n---\n",
        "Not a skill document",
    ];
    for original in originals {
        fs::write(target.join("SKILL.md"), original).unwrap();
        assert!(Store::new(home.clone())
            .preview(&source, 1)
            .unwrap_err()
            .contains("身份"));
        assert_eq!(
            fs::read_to_string(target.join("SKILL.md")).unwrap(),
            original
        );
        assert_eq!(
            fs::read_to_string(source.join("SKILL.md"))
                .unwrap()
                .lines()
                .nth(1),
            Some("name: fixture-skill")
        );
    }
}

#[test]
fn trash_moves_complete_record_and_keeps_installed_skill_and_config_unchanged() {
    let (_sandbox, home, source) = fixture();
    let mut store = Store::new(home.clone());
    let p = store.preview(&source, 1).unwrap();
    store.apply(&p.plan_id, 2).unwrap();
    fs::write(source.join("before.txt"), "new version").unwrap();
    let p = store.preview(&source, 3).unwrap();
    let applied = store.apply(&p.plan_id, 4).unwrap();
    fs::create_dir(home.join(".codex")).unwrap();
    fs::write(home.join(".codex/config.toml"), "model='fixture'\n").unwrap();
    let record = home
        .join(".agents/.agentisland-skill-installs")
        .join(&applied.id);
    let before = files::scan_record(&files::open_dir(&record).unwrap())
        .unwrap()
        .revision;
    let target = home.join(".agents/skills/fixture-skill");
    let target_revision = files::scan(&files::open_dir(&target).unwrap())
        .unwrap()
        .revision;
    let p = store.trash_preview(&applied.id, 5).unwrap();
    assert_eq!(p.action, "trash");
    assert!(p.changes.iter().any(|c| c.path == "receipt.json"));
    assert!(p.changes.iter().any(|c| c.path == "package/SKILL.md"));
    assert!(!home.join(".Trash").exists());
    assert!(serde_json::to_string(&p)
        .unwrap()
        .find(home.to_str().unwrap())
        .is_none());
    store.trash(&p.plan_id, 6).unwrap();
    assert!(!record.exists());
    let moved = fs::read_dir(home.join(".Trash"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    assert_eq!(
        files::scan_record(&files::open_dir(&moved).unwrap())
            .unwrap()
            .revision,
        before
    );
    assert_eq!(
        files::scan(&files::open_dir(&target).unwrap())
            .unwrap()
            .revision,
        target_revision
    );
    assert_eq!(
        fs::read_to_string(home.join(".codex/config.toml")).unwrap(),
        "model='fixture'\n"
    );
    assert!(store
        .recoveries()
        .unwrap()
        .iter()
        .all(|row| row.id != applied.id));
    assert!(store.trash(&p.plan_id, 7).is_err());
    let rows = store.recoveries().unwrap();
    assert!(rows.iter().all(|row| row.state == "conflict"));
}
#[test]
fn trash_partial_records_frees_install_capacity_and_cancellation_has_no_filesystem_effect() {
    let (_sandbox, home, source) = fixture();
    let root = home.join(".agents/.agentisland-skill-installs");
    fs::create_dir_all(home.join(".agents/skills")).unwrap();
    fs::create_dir(&root).unwrap();
    let ids = (0..100)
        .map(|_| uuid::Uuid::new_v4().to_string())
        .collect::<Vec<_>>();
    for id in &ids {
        fs::create_dir(root.join(id)).unwrap();
    }
    let mut store = Store::new(home.clone());
    let install = store.preview(&source, 1).unwrap();
    assert!(store
        .apply(&install.plan_id, 2)
        .unwrap_err()
        .contains("100"));
    assert_eq!(store.recoveries().unwrap().len(), 100);
    let clear = store.trash_preview(&ids[0], 3).unwrap();
    assert_eq!(clear.files, 0);
    assert_eq!(clear.change_count, 0);
    store.cancel(&clear.plan_id);
    assert!(store.trash(&clear.plan_id, 4).is_err());
    assert!(root.join(&ids[0]).exists());
    assert!(!home.join(".Trash").exists());
    let extra = uuid::Uuid::new_v4().to_string();
    fs::create_dir(root.join(&extra)).unwrap();
    assert_eq!(store.recoveries().unwrap().len(), 101);
    let clear = store.trash_preview(&extra, 5).unwrap();
    store.trash(&clear.plan_id, 6).unwrap();
    let clear = store.trash_preview(&ids[0], 7).unwrap();
    store.trash(&clear.plan_id, 8).unwrap();
    let install = store.preview(&source, 9).unwrap();
    store.apply(&install.plan_id, 10).unwrap();
    assert_eq!(store.recoveries().unwrap().len(), 100);
}
#[test]
fn trash_expiry_content_drift_identity_replacement_and_symlinked_trash_keep_records() {
    let (_sandbox, home, source) = fixture();
    let mut store = Store::new(home.clone());
    let p = store.preview(&source, 1).unwrap();
    let applied = store.apply(&p.plan_id, 2).unwrap();
    let record = home
        .join(".agents/.agentisland-skill-installs")
        .join(&applied.id);
    let p = store.trash_preview(&applied.id, 3).unwrap();
    assert!(store.trash(&p.plan_id, 3 + PLAN_TTL + 1).is_err());
    assert!(store.trash(&p.plan_id, 2).is_err());
    let p = store.trash_preview(&applied.id, 4).unwrap();
    fs::write(record.join("receipt.json"), "{\"changed\":true}").unwrap();
    assert!(store.trash(&p.plan_id, 5).is_err());
    assert!(record.exists());
    assert!(!home.join(".Trash").exists());
    let p = store.trash_preview(&applied.id, 6).unwrap();
    let elsewhere = home.join("original-record");
    fs::rename(&record, &elsewhere).unwrap();
    fs::create_dir(&record).unwrap();
    fs::write(record.join("receipt.json"), "{\"changed\":true}").unwrap();
    assert!(store.trash(&p.plan_id, 7).is_err());
    assert!(elsewhere.exists());
    let outside = home.join("outside");
    fs::create_dir(&outside).unwrap();
    symlink(&outside, home.join(".Trash")).unwrap();
    let p = store.trash_preview(&applied.id, 8).unwrap();
    assert!(store
        .trash(&p.plan_id, 9)
        .unwrap_err()
        .contains("废纸篓不可用"));
    assert!(record.exists());
    assert_eq!(fs::read_dir(outside).unwrap().count(), 0);
    fs::remove_file(home.join(".Trash")).unwrap();
    store.trash(&p.plan_id, 10).unwrap();
    assert!(!record.exists());
    assert!(home.join(".agents/skills/fixture-skill/SKILL.md").exists());
}
#[test]
fn trash_rejects_unknown_entries_linked_packages_and_noncanonical_record_ids() {
    let (_sandbox, home, source) = fixture();
    let mut store = Store::new(home.clone());
    let p = store.preview(&source, 1).unwrap();
    let applied = store.apply(&p.plan_id, 2).unwrap();
    let root = home.join(".agents/.agentisland-skill-installs");
    let record = root.join(&applied.id);
    assert!(store.trash_preview("../outside", 3).is_err());
    assert!(store.trash_preview(&applied.id.to_uppercase(), 3).is_err());
    fs::write(record.join("unrelated.txt"), "keep").unwrap();
    assert!(store.trash_preview(&applied.id, 3).is_err());
    fs::remove_file(record.join("unrelated.txt")).unwrap();
    symlink(&source, record.join("package")).unwrap();
    assert!(store.trash_preview(&applied.id, 3).is_err());
    fs::remove_file(record.join("package")).unwrap();
    let other = uuid::Uuid::new_v4().to_string();
    symlink(&record, root.join(&other)).unwrap();
    assert!(store.trash_preview(&other, 3).is_err());
    assert!(!home.join(".Trash").exists());
    assert!(record.exists());
}
#[test]
fn trash_handles_maximum_package_tree_and_preserves_all_files_after_update() {
    let (_sandbox, home, source) = fixture();
    for i in 0..511 {
        fs::create_dir(source.join(format!("dir-{i:03}"))).unwrap();
    }
    let mut store = Store::new(home.clone());
    let p = store.preview(&source, 1).unwrap();
    store.apply(&p.plan_id, 2).unwrap();
    fs::write(
        source.join("SKILL.md"),
        "---\nname: fixture-skill\ndescription: Updated\n---\n",
    )
    .unwrap();
    let p = store.preview(&source, 3).unwrap();
    let applied = store.apply(&p.plan_id, 4).unwrap();
    let clear = store.trash_preview(&applied.id, 5).unwrap();
    assert_eq!(clear.change_count, 514);
    assert_eq!(clear.changes.len(), 100);
    assert_eq!(clear.files, 2);
    store.trash(&clear.plan_id, 6).unwrap();
    let moved = fs::read_dir(home.join(".Trash"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    assert_eq!(
        files::scan_record(&files::open_dir(&moved).unwrap())
            .unwrap()
            .nodes
            .len(),
        514
    );
    assert_eq!(
        files::scan(&files::open_dir(&home.join(".agents/skills/fixture-skill")).unwrap())
            .unwrap()
            .nodes
            .len(),
        512
    );
}

#[test]
fn sync_between_codex_and_claude_preserves_source_resources_and_target_aware_recovery() {
    let (_sandbox, home, source) = fixture();
    fs::create_dir(source.join("scripts")).unwrap();
    fs::write(source.join("scripts/run.sh"), "exit 0\n").unwrap();
    fs::set_permissions(
        source.join("scripts/run.sh"),
        fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    let mut store = Store::new(home.clone());
    let p = store.preview(&source, 1).unwrap();
    let first = store.apply(&p.plan_id, 2).unwrap();
    let catalog = store.inventory().unwrap();
    let src = catalog
        .entries
        .iter()
        .find(|r| r.tool == Tool::Codex)
        .unwrap();
    let serialized = serde_json::to_string(&catalog).unwrap();
    assert!(!serialized.contains(home.to_str().unwrap()));
    assert!(!serialized.contains("Fixture instructions"));
    let p = store
        .sync_preview(&src.id, &catalog.generation, Tool::Claude, 3)
        .unwrap();
    assert_eq!(p.target, Some(Tool::Claude));
    assert_eq!(p.source, Some(Tool::Codex));
    assert!(!home.join(".claude").exists());
    let synced = store.apply(&p.plan_id, 4).unwrap();
    let codex = home.join(".agents/skills/fixture-skill");
    let claude = home.join(".claude/skills/fixture-skill");
    assert_eq!(
        files::scan(&files::open_dir(&codex).unwrap())
            .unwrap()
            .revision,
        files::scan(&files::open_dir(&claude).unwrap())
            .unwrap()
            .revision
    );
    assert_eq!(
        fs::metadata(claude.join("scripts/run.sh"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    let store = Store::new(home.clone());
    let rows = store.recoveries().unwrap();
    let row = rows.iter().find(|r| r.id == first.id).unwrap();
    store.restore(&row.id, &row.revision).unwrap();
    assert!(!codex.exists());
    assert!(claude.exists());
    let mut store = Store::new(home.clone());
    let catalog = store.inventory().unwrap();
    let src = catalog
        .entries
        .iter()
        .find(|r| r.tool == Tool::Claude)
        .unwrap();
    let p = store
        .sync_preview(&src.id, &catalog.generation, Tool::Codex, 5)
        .unwrap();
    let returned = store.apply(&p.plan_id, 6).unwrap();
    assert!(codex.join("scripts/run.sh").exists());
    let rows = store.recoveries().unwrap();
    assert!(rows
        .iter()
        .any(|r| r.id == returned.id && r.target == Some(Tool::Codex)));
    let row = rows.iter().find(|r| r.id == synced.id).unwrap();
    assert_eq!(row.target, Some(Tool::Claude));
    let undo = store.restore_preview(&row.id, &row.revision).unwrap();
    assert_eq!(undo.target, Some(Tool::Claude));
    store.restore(&row.id, &row.revision).unwrap();
    assert!(!claude.exists());
    assert!(codex.exists());
}
#[test]
fn sync_same_name_update_restores_only_selected_tool_and_identical_target_is_noop() {
    let (_sandbox, home, source) = fixture();
    let mut store = Store::new(home.clone());
    let p = store.preview(&source, 1).unwrap();
    store.apply(&p.plan_id, 2).unwrap();
    let p = store.preview_for(&source, Tool::Claude, 3).unwrap();
    store.apply(&p.plan_id, 4).unwrap();
    let catalog = store.inventory().unwrap();
    let src = catalog
        .entries
        .iter()
        .find(|r| r.tool == Tool::Codex)
        .unwrap();
    assert!(store
        .sync_preview(&src.id, &catalog.generation, Tool::Claude, 5)
        .unwrap_err()
        .contains("一致"));
    let claude = home.join(".claude/skills/fixture-skill");
    fs::write(claude.join("extra.txt"), "old target resource").unwrap();
    let p = store
        .sync_preview(&src.id, &catalog.generation, Tool::Claude, 6)
        .unwrap();
    assert_eq!(p.action, "update");
    assert!(p
        .changes
        .iter()
        .any(|c| c.path == "extra.txt" && c.kind == "removed"));
    let applied = store.apply(&p.plan_id, 7).unwrap();
    assert!(!claude.join("extra.txt").exists());
    let rows = Store::new(home.clone()).recoveries().unwrap();
    let row = rows.iter().find(|r| r.id == applied.id).unwrap();
    store.restore(&row.id, &row.revision).unwrap();
    assert_eq!(
        fs::read_to_string(claude.join("extra.txt")).unwrap(),
        "old target resource"
    );
    assert!(!home.join(".agents/skills/fixture-skill/extra.txt").exists());
}
#[test]
fn legacy_receipts_default_to_codex_and_target_tampering_invalidates_old_recovery() {
    let (_sandbox, home, source) = fixture();
    let mut store = Store::new(home.clone());
    let p = store.preview(&source, 1).unwrap();
    let applied = store.apply(&p.plan_id, 2).unwrap();
    let receipt = home
        .join(".agents/.agentisland-skill-installs")
        .join(&applied.id)
        .join("receipt.json");
    let mut value: serde_json::Value =
        serde_json::from_slice(&fs::read(&receipt).unwrap()).unwrap();
    value.as_object_mut().unwrap().remove("target");
    fs::write(&receipt, serde_json::to_vec(&value).unwrap()).unwrap();
    let rows = Store::new(home.clone()).recoveries().unwrap();
    let row = rows.iter().find(|r| r.id == applied.id).unwrap();
    assert_eq!(row.target, Some(Tool::Codex));
    assert_eq!(row.state, "applied");
    let p = store.preview_for(&source, Tool::Claude, 3).unwrap();
    store.apply(&p.plan_id, 4).unwrap();
    value["target"] = "claude".into();
    fs::write(&receipt, serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(store.restore(&row.id, &row.revision).is_err());
    assert!(home.join(".agents/skills/fixture-skill").exists());
    assert!(home.join(".claude/skills/fixture-skill").exists());
    value["target"] = "unknown".into();
    fs::write(&receipt, serde_json::to_vec(&value).unwrap()).unwrap();
    assert_eq!(
        store
            .recoveries()
            .unwrap()
            .iter()
            .find(|r| r.id == applied.id)
            .unwrap()
            .state,
        "unreadable"
    );
    value.as_object_mut().unwrap().remove("target");
    fs::write(&receipt, serde_json::to_vec(&value).unwrap()).unwrap();
    store.restore(&row.id, &row.revision).unwrap();
    assert!(!home.join(".agents/skills/fixture-skill").exists());
    assert!(home.join(".claude/skills/fixture-skill").exists());
}
#[test]
fn sync_catalog_generation_metadata_resource_drift_and_linked_tool_root_reject_old_actions() {
    let (_sandbox, home, source) = fixture();
    let mut store = Store::new(home.clone());
    let p = store.preview(&source, 1).unwrap();
    store.apply(&p.plan_id, 2).unwrap();
    let catalog = store.inventory().unwrap();
    let src = catalog.entries.iter().find(|r| r.available).unwrap();
    assert!(store
        .sync_preview(&src.id, &catalog.generation, Tool::Codex, 3)
        .unwrap_err()
        .contains("相同"));
    store.inventory().unwrap();
    assert!(store
        .sync_preview(&src.id, &catalog.generation, Tool::Claude, 3)
        .is_err());
    let catalog = store.inventory().unwrap();
    let src = catalog.entries.iter().find(|r| r.available).unwrap();
    let codex = home.join(".agents/skills/fixture-skill");
    fs::write(
        codex.join("SKILL.md"),
        "---\nname: fixture-skill\ndescription: Changed\n---\n",
    )
    .unwrap();
    assert!(store
        .sync_preview(&src.id, &catalog.generation, Tool::Claude, 3)
        .is_err());
    let catalog = store.inventory().unwrap();
    let src = catalog.entries.iter().find(|r| r.available).unwrap();
    let p = store
        .sync_preview(&src.id, &catalog.generation, Tool::Claude, 4)
        .unwrap();
    fs::write(codex.join("new.txt"), "later resource").unwrap();
    assert!(store.apply(&p.plan_id, 5).is_err());
    assert!(!home.join(".claude").exists());
    let catalog = store.inventory().unwrap();
    let src = catalog.entries.iter().find(|r| r.available).unwrap();
    let p = store
        .sync_preview(&src.id, &catalog.generation, Tool::Claude, 6)
        .unwrap();
    let agents = home.join(".agents");
    let moved = home.join("outside-agents");
    fs::rename(&agents, &moved).unwrap();
    symlink(&moved, &agents).unwrap();
    assert!(store.apply(&p.plan_id, 7).is_err());
    assert!(!home.join(".claude").exists());
}
#[test]
fn catalog_is_metadata_only_empty_read_is_pure_and_reserved_claude_names_are_protected() {
    let (_sandbox, home, source) = fixture();
    let mut store = Store::new(home.clone());
    assert!(store.inventory().unwrap().entries.is_empty());
    assert!(!home.join(".agents").exists());
    assert!(!home.join(".claude").exists());
    for name in ["synced", "anthropic-skills"] {
        fs::write(
            source.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: Fixture\n---\n"),
        )
        .unwrap();
        assert!(store.preview_for(&source, Tool::Claude, 1).is_err());
    }
    let root = home.join(".claude/skills");
    fs::create_dir_all(root.join("synced")).unwrap();
    fs::create_dir(root.join("candidate")).unwrap();
    fs::write(root.join("candidate/SKILL.md"), "UNPARSED BODY").unwrap();
    symlink(&source, root.join("linked")).unwrap();
    let catalog = store.inventory().unwrap();
    assert!(catalog
        .entries
        .iter()
        .any(|r| r.name == "candidate" && r.available));
    assert!(catalog
        .entries
        .iter()
        .any(|r| r.name == "linked" && !r.available));
    assert!(!catalog.entries.iter().any(|r| r.name == "synced"));
    assert!(!catalog.notices.is_empty());
    let serialized = serde_json::to_string(&catalog).unwrap();
    assert!(!serialized.contains("UNPARSED BODY"));
    assert!(!serialized.contains(home.to_str().unwrap()));
    let source_row = catalog.entries.iter().find(|r| r.available).unwrap();
    assert!(store
        .sync_preview(&source_row.id, &catalog.generation, Tool::Codex, 2)
        .is_err());
    for i in 0..200 {
        fs::create_dir(root.join(format!("empty-{i}"))).unwrap();
    }
    let catalog = store.inventory().unwrap();
    assert!(catalog.entries.is_empty());
    assert!(catalog.notices.iter().any(|notice| notice.contains("200")));
}

fn edit_open(store: &mut Store, tool: Tool, now: i64) -> EditDocument {
    let list = store.inventory().unwrap();
    let row = list
        .entries
        .iter()
        .find(|row| row.tool == tool && row.available)
        .unwrap();
    store.edit_read(&row.id, &list.generation, now).unwrap()
}
#[test]
fn body_edit_keeps_metadata_resources_and_restore_is_bound_to_each_target() {
    for tool in [Tool::Codex, Tool::Claude] {
        let (_sandbox, home, source) = fixture();
        let header="\u{feff}---\r\nname: fixture-skill\r\ndescription: |\r\n  Preserve the original metadata.\r\n---\r\n";
        fs::write(source.join("SKILL.md"), format!("{header}Old body.\r\n")).unwrap();
        fs::create_dir(source.join("empty")).unwrap();
        fs::write(source.join("run.sh"), "exit 0\n").unwrap();
        fs::set_permissions(source.join("run.sh"), fs::Permissions::from_mode(0o700)).unwrap();
        let mut store = Store::new(home.clone());
        let plan = store.preview_for(&source, tool, 1).unwrap();
        store.apply(&plan.plan_id, 2).unwrap();
        let target = home.join(tool.directory()).join("skills/fixture-skill");
        let initial = files::scan(&files::open_dir(&target).unwrap())
            .unwrap()
            .revision;
        let doc = edit_open(&mut store, tool, 3);
        assert_eq!(doc.body, "Old body.\r\n");
        let result = store
            .edit_preview(
                &doc.ticket,
                "New body.\n<script>plain text</script>\n".into(),
                4,
            )
            .unwrap();
        assert_eq!(result.preview.action, "edit");
        assert_eq!(result.preview.target, Some(tool));
        assert_eq!(result.preview.change_count, 1);
        assert_eq!(result.preview.changes[0].path, "SKILL.md");
        assert_eq!(
            files::scan(&files::open_dir(&target).unwrap())
                .unwrap()
                .revision,
            initial
        );
        let applied = store.apply(&result.preview.plan_id, 5).unwrap();
        assert!(applied.notice.contains("保存"));
        assert_eq!(
            fs::read_to_string(target.join("SKILL.md")).unwrap(),
            format!("{header}New body.\n<script>plain text</script>\n")
        );
        assert_eq!(fs::read(target.join("run.sh")).unwrap(), b"exit 0\n");
        assert!(target.join("empty").is_dir());
        assert_eq!(
            fs::metadata(target.join("run.sh"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        let store = Store::new(home.clone());
        let row = store
            .recoveries()
            .unwrap()
            .into_iter()
            .find(|row| row.id == applied.id)
            .unwrap();
        assert_eq!(row.target, Some(tool));
        store.restore(&row.id, &row.revision).unwrap();
        assert_eq!(
            files::scan(&files::open_dir(&target).unwrap())
                .unwrap()
                .revision,
            initial
        );
        assert_eq!(
            fs::read_to_string(source.join("SKILL.md")).unwrap(),
            format!("{header}Old body.\r\n")
        );
    }
}
#[test]
fn body_edit_cancellation_expiry_and_resource_drift_preserve_files_and_draft() {
    let (_sandbox, home, source) = fixture();
    let mut store = Store::new(home.clone());
    let plan = store.preview(&source, 1).unwrap();
    store.apply(&plan.plan_id, 2).unwrap();
    let target = home.join(".agents/skills/fixture-skill");
    let doc = edit_open(&mut store, Tool::Codex, 3);
    assert!(store
        .edit_preview(&doc.ticket, doc.body.clone(), 4)
        .is_err());
    let preview = store
        .edit_preview(&doc.ticket, "Changed body.".into(), 5)
        .unwrap();
    store.cancel(&preview.preview.plan_id);
    assert!(store.apply(&preview.preview.plan_id, 6).is_err());
    assert_eq!(
        fs::read(target.join("SKILL.md")).unwrap(),
        fs::read(source.join("SKILL.md")).unwrap()
    );
    let preview = store
        .edit_preview(&doc.ticket, "Changed body.".into(), 7)
        .unwrap();
    fs::write(target.join("extra.txt"), "External content.").unwrap();
    assert!(store
        .edit_preview(&doc.ticket, "Changed body.".into(), 8)
        .is_err());
    assert!(store.apply(&preview.preview.plan_id, 8).is_err());
    assert_eq!(
        fs::read_to_string(target.join("extra.txt")).unwrap(),
        "External content."
    );
    fs::remove_file(target.join("extra.txt")).unwrap();
    assert!(store
        .edit_preview(&doc.ticket, "Changed body.".into(), doc.expires_ms + 1)
        .is_err());
    assert!(store
        .edit_preview(&doc.ticket, "Changed body.".into(), 2)
        .is_err());
    store.edit_close(&doc.ticket);
    assert!(store
        .edit_preview(&doc.ticket, "Changed body.".into(), 9)
        .is_err());
    let doc = edit_open(&mut store, Tool::Codex, 10);
    let preview = store
        .edit_preview(&doc.ticket, "Changed body.".into(), 11)
        .unwrap();
    assert!(store
        .apply(&preview.preview.plan_id, 11 + PLAN_TTL + 1)
        .is_err());
}
#[test]
fn body_edit_rejects_identity_replacement_private_content_and_size_overflow() {
    let (_sandbox, home, source) = fixture();
    let mut store = Store::new(home.clone());
    let plan = store.preview(&source, 1).unwrap();
    store.apply(&plan.plan_id, 2).unwrap();
    let target = home.join(".agents/skills/fixture-skill");
    let original = fs::read(target.join("SKILL.md")).unwrap();
    let doc = edit_open(&mut store, Tool::Codex, 3);
    let private = ["password", ": fixture-value"].concat();
    let error = store
        .edit_preview(&doc.ticket, private.clone(), 4)
        .unwrap_err();
    assert!(!error.contains(&private));
    assert!(store
        .edit_preview(&doc.ticket, "X".repeat(4 * 1024 * 1024), 4)
        .is_err()); // header would exceed the per-file budget
    assert!(store
        .edit_preview(&doc.ticket, "中".repeat(2 * 1024 * 1024), 4)
        .is_err()); // byte budget, not code points
    let preview = store
        .edit_preview(&doc.ticket, "Changed body.".into(), 5)
        .unwrap();
    let kept = home.join("replaced-directory");
    fs::rename(&target, &kept).unwrap();
    fs::create_dir(&target).unwrap();
    fs::write(target.join("SKILL.md"), &original).unwrap();
    assert!(store.apply(&preview.preview.plan_id, 6).is_err());
    assert!(store
        .edit_preview(&doc.ticket, "Changed body.".into(), 6)
        .is_err());
    assert_eq!(fs::read(target.join("SKILL.md")).unwrap(), original);
    fs::remove_dir_all(&target).unwrap();
    symlink(&kept, &target).unwrap();
    assert!(store
        .edit_preview(&doc.ticket, "Changed body.".into(), 7)
        .is_err());
}
#[test]
fn body_edit_ticket_capacity_read_failures_and_preview_preserve_receipt_privacy() {
    let (_sandbox, home, source) = fixture();
    let mut store = Store::new(home.clone());
    let plan = store.preview(&source, 1).unwrap();
    store.apply(&plan.plan_id, 2).unwrap();
    let target = home.join(".agents/skills/fixture-skill");
    let mut tickets = vec![];
    for _ in 0..16 {
        tickets.push(edit_open(&mut store, Tool::Codex, 3));
    }
    let list = store.inventory().unwrap();
    let row = &list.entries[0];
    assert!(store.edit_read(&row.id, &list.generation, 4).is_err());
    store.edit_close(&tickets[0].ticket);
    let doc = store.edit_read(&row.id, &list.generation, 4).unwrap();
    let preview = store
        .edit_preview(&doc.ticket, "Body text stays out of receipts.".into(), 5)
        .unwrap();
    let saved = store.apply(&preview.preview.plan_id, 6).unwrap();
    let receipt = fs::read_to_string(
        home.join(".agents/.agentisland-skill-installs")
            .join(saved.id)
            .join("receipt.json"),
    )
    .unwrap();
    assert!(!receipt.contains("Body text"));
    assert!(!receipt.contains(&home.to_string_lossy().to_string()));
    let list = store.inventory().unwrap();
    let row = &list.entries[0];
    fs::write(
        target.join("SKILL.md"),
        "---\nname: other-name\ndescription: mismatched\n---\nBody.\n",
    )
    .unwrap();
    assert!(store.edit_read(&row.id, &list.generation, 7).is_err());
    let list = store.inventory().unwrap();
    let row = &list.entries[0];
    assert!(store
        .edit_read(&row.id, &list.generation, 30 * 60 * 1000 + 10)
        .is_err());
}

#[test]
fn body_edit_accepts_empty_body_and_adds_boundary_for_metadata_without_final_newline() {
    let (_sandbox, home, source) = fixture();
    fs::write(
        source.join("SKILL.md"),
        "---\nname: fixture-skill\ndescription: Test.\n---",
    )
    .unwrap();
    let mut store = Store::new(home.clone());
    let plan = store.preview(&source, 1).unwrap();
    store.apply(&plan.plan_id, 2).unwrap();
    let doc = edit_open(&mut store, Tool::Codex, 3);
    assert!(doc.body.is_empty());
    let preview = store
        .edit_preview(&doc.ticket, "New body.".into(), 4)
        .unwrap();
    store.apply(&preview.preview.plan_id, 5).unwrap();
    let doc = edit_open(&mut store, Tool::Codex, 6);
    assert_eq!(doc.body, "New body.");
    let preview = store.edit_preview(&doc.ticket, String::new(), 7).unwrap();
    store.apply(&preview.preview.plan_id, 8).unwrap();
    assert!(edit_open(&mut store, Tool::Codex, 9).body.is_empty());
}

#[test]
fn namespace_checks_reject_root_replacement_and_links_even_with_identical_live_content() {
    for tool in [Tool::Codex, Tool::Claude] {
        let (_sandbox, home, source) = fixture();
        let mut store = Store::new(home.clone());
        let preview = store.preview_for(&source, tool, 1).unwrap();
        store.apply(&preview.plan_id, 2).unwrap();
        let skills = store.skills_root(tool, false).unwrap().unwrap();
        let roots = store.roots(false).unwrap();
        store.verify_skills_root(tool, &skills).unwrap();
        store.verify_records_root(&roots).unwrap();
        let base = home.join(tool.directory());
        let old = home.join("moved-skills");
        fs::rename(base.join("skills"), &old).unwrap();
        fs::create_dir_all(base.join("skills/fixture-skill")).unwrap();
        fs::copy(
            source.join("SKILL.md"),
            base.join("skills/fixture-skill/SKILL.md"),
        )
        .unwrap();
        assert!(store.verify_skills_root(tool, &skills).is_err());
        assert_eq!(
            fingerprint(&skills, "fixture-skill").unwrap(),
            store
                .existing(tool, "fixture-skill")
                .unwrap()
                .map(|tree| tree.revision)
        );
        fs::remove_dir_all(base.join("skills")).unwrap();
        symlink(&old, base.join("skills")).unwrap();
        assert!(store.verify_skills_root(tool, &skills).is_err());
        fs::remove_file(base.join("skills")).unwrap();
        fs::rename(&old, base.join("skills")).unwrap();
        store.verify_skills_root(tool, &skills).unwrap();
        let record_path = home.join(".agents/.agentisland-skill-installs");
        let record_old = home.join("moved-records");
        fs::rename(&record_path, &record_old).unwrap();
        fs::create_dir(&record_path).unwrap();
        assert!(store.verify_records_root(&roots).is_err());
        fs::remove_dir(&record_path).unwrap();
        symlink(&record_old, &record_path).unwrap();
        assert!(store.verify_records_root(&roots).is_err());
        fs::remove_file(&record_path).unwrap();
        fs::rename(&record_old, &record_path).unwrap();
        store.verify_records_root(&roots).unwrap();
        let parent_old = home.join("moved-tool-base");
        fs::rename(&base, &parent_old).unwrap();
        symlink(&parent_old, &base).unwrap();
        assert!(store.verify_skills_root(tool, &skills).is_err());
        assert_eq!(
            fs::read(parent_old.join("skills/fixture-skill/SKILL.md")).unwrap(),
            fs::read(source.join("SKILL.md")).unwrap()
        );
    }
}

/// Explicit acceptance only: real client discovery, no model turn or user configuration.
#[test]
#[ignore = "requires AGENTISLAND_CODEX_CLIENT and macOS verified network denial"]
fn real_codex_client_discovers_production_install_update_toggle_and_restore() {
    use serde_json::json;
    use std::process::Command;
    let client =
        std::env::var_os("AGENTISLAND_CODEX_CLIENT").expect("set an explicit Codex client binary");
    let sandbox = crate::testutil::Sandbox::new("skill-client");
    let home = sandbox.path();
    fs::write(home.join(".agentisland-client-fixture"), "fixture-only\n").unwrap();
    let source = home.join("source");
    fs::create_dir(&source).unwrap();
    let skill = source.join("SKILL.md");
    fs::write(&skill,"---\nname: client-check\ndescription: Fixture initial description\n---\nFixture instructions, do not execute.\n").unwrap();
    fs::create_dir(source.join("scripts")).unwrap();
    fs::write(
        source.join("scripts/not-run.sh"),
        "#!/bin/sh\ntouch never-execute\n",
    )
    .unwrap();
    fs::set_permissions(
        source.join("scripts/not-run.sh"),
        fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    let verify = |stage: &str, description: Option<&str>, enabled: bool| {
        let expected = home.join("expected.json");
        fs::write(&expected,serde_json::to_vec(&json!({"client-check":description.map(|description|json!({"description":description,"enabled":enabled}))})).unwrap()).unwrap();
        let script = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../scripts/check-codex-skill-discovery.py");
        let output = Command::new("/usr/bin/python3")
            .arg(script)
            .arg("--client")
            .arg(&client)
            .arg("--fixture")
            .arg(home)
            .arg("--expected")
            .arg(expected)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{stage}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        println!(
            "{stage}: {}",
            String::from_utf8_lossy(&output.stdout).trim()
        );
        assert!(!home.join("project/never-execute").exists());
        assert!(!home
            .join(".agents/skills/client-check/never-execute")
            .exists());
    };
    verify("before installation", None, true);
    let mut store = Store::new(home.to_path_buf());
    let preview = store.preview(&source, 1).unwrap();
    let first = store.apply(&preview.plan_id, 2).unwrap();
    verify("installed", Some("Fixture initial description"), true);
    let config = home.join(".codex/config.toml");
    let backups = home.join("toggle-backups");
    let snapshot = crate::skills_config::inspect(&config, home).unwrap();
    let operation = crate::skills_config::Operation {
        id: snapshot
            .entries
            .iter()
            .find(|entry| entry.name == "client-check")
            .unwrap()
            .id
            .clone(),
        enabled: false,
    };
    let preview =
        crate::skills_config::preview(&config, home, &operation, &snapshot.revision).unwrap();
    crate::skills_config::apply(
        &config,
        home,
        &backups,
        &operation,
        &preview.revision,
        &preview.plan_id,
        3,
    )
    .unwrap();
    verify("disabled", Some("Fixture initial description"), false);
    fs::write(&skill,"---\nname: &skill_name client-check\ndescription: >-\n  Fixture updated\n  description\nmetadata:\n  sample: *skill_name\n---\nUpdated fixture instructions.\n").unwrap();
    let preview = store.preview(&source, 4).unwrap();
    let update = store.apply(&preview.plan_id, 5).unwrap();
    verify(
        "updated while disabled",
        Some("Fixture updated description"),
        false,
    );
    let store = Store::new(home.to_path_buf());
    let rows = store.recoveries().unwrap();
    let row = rows.iter().find(|row| row.id == update.id).unwrap();
    let preview = store.restore_preview(&row.id, &row.revision).unwrap();
    store.restore(&row.id, &preview.plan_id).unwrap();
    verify(
        "restored earlier package",
        Some("Fixture initial description"),
        false,
    );
    let snapshot = crate::skills_config::inspect(&config, home).unwrap();
    let operation = crate::skills_config::Operation {
        enabled: true,
        ..operation
    };
    let preview =
        crate::skills_config::preview(&config, home, &operation, &snapshot.revision).unwrap();
    crate::skills_config::apply(
        &config,
        home,
        &backups,
        &operation,
        &preview.revision,
        &preview.plan_id,
        6,
    )
    .unwrap();
    verify("enabled again", Some("Fixture initial description"), true);
    let rows = store.recoveries().unwrap();
    let row = rows.iter().find(|row| row.id == first.id).unwrap();
    let preview = store.restore_preview(&row.id, &row.revision).unwrap();
    store.restore(&row.id, &preview.plan_id).unwrap();
    verify("first installation undone", None, true);
}

/// Real user-directory discovery and slash expansion; the model is loopback-only.
#[test]
#[ignore = "requires pinned AGENTISLAND_CLAUDE_CLIENT and macOS verified system isolation"]
fn real_claude_client_loads_production_install_update_and_restore() {
    use serde_json::json;
    use std::process::Command;
    let client = std::env::var_os("AGENTISLAND_CLAUDE_CLIENT")
        .expect("set an explicit pinned Claude Code client binary");
    let sandbox = crate::testutil::Sandbox::new("claude-skill-client");
    let home = sandbox.path();
    fs::write(home.join(".agentisland-client-fixture"), "fixture-only\n").unwrap();
    let source = home.join("source");
    fs::create_dir(&source).unwrap();
    let skill = source.join("SKILL.md");
    let write_skill = |marker: &str| {
        fs::write(&skill, format!("---\nname: agentisland-client-fixture\ndescription: Synthetic client loading verification.\n---\n{marker}\nReturn without running any tools.\n")).unwrap();
    };
    fs::create_dir(source.join("scripts")).unwrap();
    fs::write(
        source.join("scripts/not-run.sh"),
        "#!/bin/sh\ntouch never-execute\n",
    )
    .unwrap();
    fs::set_permissions(
        source.join("scripts/not-run.sh"),
        fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    let first_body = "AGENTISLAND_FIXTURE_FIRST_BODY";
    let second_body = "AGENTISLAND_FIXTURE_UPDATED_BODY";
    let verify = |stage: &str, present: bool, marker: &str, absent_marker: &str| {
        let expected = home.join("expected.json");
        fs::write(
            &expected,
            serde_json::to_vec(
                &json!({"present":present,"marker":marker,"absent_marker":absent_marker}),
            )
            .unwrap(),
        )
        .unwrap();
        let script = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../scripts/check-claude-skill-discovery.py");
        let output = Command::new("/usr/bin/python3")
            .arg(script)
            .arg("--client")
            .arg(&client)
            .arg("--fixture")
            .arg(home)
            .arg("--expected")
            .arg(expected)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{stage}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        println!(
            "{stage}: {}",
            String::from_utf8_lossy(&output.stdout).trim()
        );
        for directory in [
            home.join("project"),
            home.join(".claude/skills/agentisland-client-fixture"),
        ] {
            assert!(!directory.join("never-execute").exists());
        }
    };
    write_skill(first_body);
    verify("before installation", false, "", first_body);
    let mut store = Store::new(home.to_path_buf());
    let preview = store.preview_for(&source, Tool::Claude, 1).unwrap();
    let first = store.apply(&preview.plan_id, 2).unwrap();
    verify("installed user skill", true, first_body, second_body);
    write_skill(second_body);
    let preview = store.preview_for(&source, Tool::Claude, 3).unwrap();
    let update = store.apply(&preview.plan_id, 4).unwrap();
    verify("updated user skill", true, second_body, first_body);
    let store = Store::new(home.to_path_buf());
    let rows = store.recoveries().unwrap();
    let row = rows.iter().find(|row| row.id == update.id).unwrap();
    let preview = store.restore_preview(&row.id, &row.revision).unwrap();
    store.restore(&row.id, &preview.plan_id).unwrap();
    verify(
        "restored after Store restart",
        true,
        first_body,
        second_body,
    );
    let rows = store.recoveries().unwrap();
    let row = rows.iter().find(|row| row.id == first.id).unwrap();
    let preview = store.restore_preview(&row.id, &row.revision).unwrap();
    store.restore(&row.id, &preview.plan_id).unwrap();
    verify("first installation undone", false, "", first_body);
}
