use super::*;

struct Sandbox(PathBuf);

impl Sandbox {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("dedup-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        Self(root)
    }

    fn path(&self) -> PathBuf {
        self.0.join("state.json")
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn production_persist_roundtrips_raw_identities_without_plaintext() {
    let sandbox = Sandbox::new();
    let path = sandbox.path();
    let agent = "fixture-agent-sensitive-name";
    let raw = "/synthetic/session/location:confirm-private-action";
    let mut store = Store::open_at(&path);
    assert!(!path.exists(), "opening an absent path is read-only");
    store.note_attention(agent, raw);
    store.note_completion(agent, raw);
    store.persist().unwrap();
    let bytes = fs::read(&path).unwrap();
    let text = String::from_utf8(bytes).unwrap();
    assert!(!text.contains(raw));
    assert!(!text.contains(agent));
    let loaded = Store::open_at(&path);
    assert!(loaded.attention_seen(agent, raw));
    assert!(loaded.completion_seen(agent, raw));
    assert!(!loaded.attention_seen("another-agent", raw));
    assert!(!loaded.completion_seen(agent, "new-event"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    // Replacing an existing file uses the same private atomic path.
    store.note_attention(agent, "next");
    store.persist().unwrap();
    assert!(Store::open_at(&path).attention_seen(agent, "next"));
    assert_eq!(fs::read_dir(&sandbox.0).unwrap().count(), 1);
}

#[test]
fn corrupt_future_legacy_oversized_and_excess_history_are_preserved_after_persist() {
    let oversized_history = serde_json::to_vec(&Payload {
        schema_version: SCHEMA_VERSION,
        events: (0..=CAP)
            .map(|i| identity("agent", "attention", &i.to_string()))
            .collect(),
    })
    .unwrap();
    for original in [
        b"{broken".to_vec(),
        br#"{"schema_version":99,"events":[]}"#.to_vec(),
        br#"{"schema_version":1,"attention":["raw"],"completed":{}}"#.to_vec(),
        vec![b'x'; (MAX_BYTES + 1) as usize],
        oversized_history,
        br#"{"schema_version":2,"events":["plaintext"]}"#.to_vec(),
    ] {
        let sandbox = Sandbox::new();
        let path = sandbox.path();
        fs::write(&path, &original).unwrap();
        let mut store = Store::open_at(&path);
        store.note_attention("agent", "new");
        assert_eq!(store.persist(), Err(READ_ERROR));
        assert_eq!(fs::read(&path).unwrap(), original);
        assert!(
            store.attention_seen("agent", "new"),
            "memory remains usable"
        );
        assert_eq!(store.persist(), Err(READ_ERROR));
    }
}

#[test]
fn non_not_found_read_errors_do_not_enable_later_writes() {
    let sandbox = Sandbox::new();
    let blocker = sandbox.0.join("parent");
    fs::write(&blocker, b"not a directory").unwrap();
    let path = blocker.join("state.json");
    let mut store = Store::open_at(&path);
    fs::remove_file(&blocker).unwrap();
    fs::create_dir(&blocker).unwrap();
    store.note_attention("agent", "ask");
    assert_eq!(store.persist(), Err(READ_ERROR));
    assert!(
        !path.exists(),
        "repairing the path must not turn a failed read into an absent file"
    );
}

#[cfg(unix)]
#[test]
fn symlinks_and_non_regular_files_are_preserved() {
    use std::os::unix::fs::symlink;
    let sandbox = Sandbox::new();
    let victim = sandbox.0.join("victim");
    let original = br#"{"schema_version":2,"events":[]}"#;
    fs::write(&victim, original).unwrap();
    let path = sandbox.path();
    symlink(&victim, &path).unwrap();
    let mut store = Store::open_at(&path);
    store.note_completion("agent", "done");
    assert_eq!(store.persist(), Err(READ_ERROR));
    assert!(fs::symlink_metadata(&path)
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(fs::read(&victim).unwrap(), original);
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    let mut directory = Store::open_at(&path);
    assert_eq!(directory.persist(), Err(READ_ERROR));
    assert!(path.is_dir());
}

#[test]
fn a_stale_writer_never_overwrites_another_writer_or_external_edits() {
    for initially_present in [false, true] {
        let sandbox = Sandbox::new();
        let path = sandbox.path();
        if initially_present {
            Store::open_at(&path).persist().unwrap();
        }
        let mut first = Store::open_at(&path);
        let mut stale = Store::open_at(&path);
        first.note_attention("agent", "first");
        first.persist().unwrap();
        let winner = fs::read(&path).unwrap();
        stale.note_attention("agent", "stale");
        assert_eq!(stale.persist(), Err(STALE_ERROR));
        assert_eq!(fs::read(&path).unwrap(), winner);
        fs::write(&path, b"external edit").unwrap();
        first.note_completion("agent", "done");
        assert_eq!(first.persist(), Err(STALE_ERROR));
        assert_eq!(fs::read(&path).unwrap(), b"external edit");
    }
}

#[test]
fn an_active_writer_lock_refuses_a_second_writer_without_waiting() {
    let sandbox = Sandbox::new();
    let path = sandbox.path();
    let mut store = Store::open_at(&path);
    store.persist().unwrap();
    let original = fs::read(&path).unwrap();
    let lock = open_regular(&path).unwrap();
    lock.try_lock().unwrap();
    store.note_attention("agent", "new");
    assert_eq!(store.persist(), Err(WRITE_ERROR));
    assert_eq!(fs::read(&path).unwrap(), original);
    drop(lock);
    store.persist().unwrap();
    assert!(Store::open_at(&path).attention_seen("agent", "new"));
}

#[test]
fn bounded_history_evicts_one_oldest_entry_and_refreshes_duplicates() {
    let sandbox = Sandbox::new();
    let path = sandbox.path();
    let mut store = Store::open_at(&path);
    for i in 0..CAP {
        store.note_completion("agent", &i.to_string());
    }
    store.note_completion("agent", "0");
    store.note_attention("other-agent", "new");
    assert_eq!(store.events.len(), CAP);
    assert!(store.completion_seen("agent", "0"));
    assert!(!store.completion_seen("agent", "1"));
    assert!(store.completion_seen("agent", &(CAP - 1).to_string()));
    store.persist().unwrap();
    assert!(fs::metadata(&path).unwrap().len() <= MAX_BYTES);
    let loaded = Store::open_at(&path);
    assert!(loaded.completion_seen("agent", "0"));
    assert!(loaded.attention_seen("other-agent", "new"));
}

#[test]
fn identities_separate_agents_kinds_and_field_boundaries() {
    let mut store = Store::default();
    store.note_attention("a", "same");
    assert!(!store.attention_seen("b", "same"));
    assert!(!store.completion_seen("a", "same"));
    assert_ne!(identity("ab", "c", "d"), identity("a", "bc", "d"));
    store.persist().unwrap();
    assert!(store.disk.is_none());
}

#[cfg(unix)]
#[test]
fn a_real_atomic_write_failure_preserves_old_bytes_and_memory() {
    use std::os::unix::fs::PermissionsExt;
    // Root bypasses directory permissions; the portable engine failure test covers that case.
    if unsafe { libc::geteuid() } == 0 {
        return;
    }
    let sandbox = Sandbox::new();
    let path = sandbox.path();
    let mut store = Store::open_at(&path);
    store.note_attention("agent", "old");
    store.persist().unwrap();
    let original = fs::read(&path).unwrap();
    fs::set_permissions(&sandbox.0, fs::Permissions::from_mode(0o500)).unwrap();
    store.note_attention("agent", "new");
    let result = store.persist();
    fs::set_permissions(&sandbox.0, fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(result, Err(WRITE_ERROR));
    assert_eq!(fs::read(&path).unwrap(), original);
    assert!(store.attention_seen("agent", "new"));
    store.persist().unwrap();
    assert!(Store::open_at(&path).attention_seen("agent", "new"));
}
