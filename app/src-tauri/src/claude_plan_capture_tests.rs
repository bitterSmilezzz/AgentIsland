use super::*;
const SESSION: &str = "019c6e27-e55b-73d1-87d8-4e01f1f75043";
const MESSAGE: &str = "019c7714-3b77-74d1-9866-e1f484aae2ab";
struct Fixture {
    _sandbox: crate::testutil::Sandbox,
    root: PathBuf,
    path: PathBuf,
    store: Store,
    now: Instant,
}
impl Fixture {
    fn new() -> Self {
        let sandbox = crate::testutil::Sandbox::new("claude-plan-cache");
        let root = sandbox.path().canonicalize().unwrap().join("projects");
        std::fs::create_dir(&root).unwrap();
        let path = root.join(format!("{SESSION}.jsonl"));
        std::fs::write(&path, "").unwrap();
        let mut store = Store::default();
        store.enable(vec![root.clone()]).unwrap();
        Self {
            _sandbox: sandbox,
            root,
            path,
            store,
            now: Instant::now(),
        }
    }
    fn payload(&self, id: &str, plan: &str) -> Value {
        serde_json::json!({"hook_event_name":"PreToolUse","tool_name":"ExitPlanMode","session_id":SESSION,"tool_use_id":id,"transcript_path":self.path,"tool_input":{"plan":plan,"planFilePath":"/never/read/this.md"},"cwd":"/ignored"})
    }
    fn ingest(&mut self, id: &str, body: &str) -> Result<Receipt, Error> {
        self.store.ingest(
            &serde_json::to_vec(&self.payload(id, body)).unwrap(),
            self.now,
        )
    }
    fn doc(&self, id: &str) -> Value {
        serde_json::json!({"type":"assistant","uuid":MESSAGE,"sessionId":SESSION,"isSidechain":false,"message":{"content":[{"type":"tool_use","id":id,"name":"ExitPlanMode","input":{}}]}})
    }
    fn bind(&mut self, doc: &Value) -> Option<String> {
        self.store
            .bind_selected(&self.path, doc, &doc["message"]["content"][0], self.now)
    }
    fn read(&mut self, doc: &Value, r: &str) -> Option<String> {
        self.store
            .read_bound(&self.path, doc, &doc["message"]["content"][0], r, self.now)
    }
}
#[test]
fn default_closed_and_memory_only_capture_requires_selected_exact_event() {
    let mut f = Fixture::new();
    let p = serde_json::to_vec(&f.payload("p", "对应方案")).unwrap();
    let mut closed = Store::default();
    assert_eq!(closed.ingest(&p, f.now), Err(Error::Disabled));
    assert_eq!(f.ingest("p", "对应方案"), Ok(Receipt::Captured));
    let doc = f.doc("p");
    assert!(f.read(&doc, &"0".repeat(64)).is_none());
    let r = f.bind(&doc).unwrap();
    assert_eq!(r.len(), 64);
    assert_eq!(f.read(&doc, &r).as_deref(), Some("对应方案"));
    assert_eq!(std::fs::read_to_string(&f.path).unwrap(), "");
    f.store.disable();
    assert!(f.read(&doc, &r).is_none());
    assert!(f.store.entries.is_empty());
    f.store.enable(vec![f.root.clone()]).unwrap();
    assert!(f.bind(&doc).is_none());
}
#[test]
fn strict_protocol_rejects_other_events_duplicate_keys_and_unbounded_input() {
    let f = Fixture::new();
    let mut p = f.payload("p", "方案");
    p["hook_event_name"] = "PostToolUse".into();
    assert!(matches!(
        parse(&serde_json::to_vec(&p).unwrap()),
        Err(Error::Invalid)
    ));
    p = f.payload("p", "方案");
    p["tool_name"] = "AskUserQuestion".into();
    assert!(parse(&serde_json::to_vec(&p).unwrap()).is_err());
    for bytes in [
        br#"{"tool_name":"ExitPlanMode","tool_name":"AskUserQuestion"}"#.as_slice(),
        br#"{"tool_input":{"plan":"a","plan":"b"}}"#,
    ] {
        assert!(parse(bytes).is_err());
    }
    assert!(parse(&vec![b' '; MAX_INPUT + 1]).is_err());
    p = f.payload("p", &"中".repeat(MAX_BODY / 3 + 1));
    assert!(parse(&serde_json::to_vec(&p).unwrap()).is_err());
    for id in ["", " \n", &"a".repeat(257)] {
        assert!(parse(&serde_json::to_vec(&f.payload(id, "方案")).unwrap()).is_err());
    }
    p = f.payload("p", "方案");
    p["session_id"] = "00000000-0000-0000-0000-000000000000".into();
    assert!(parse(&serde_json::to_vec(&p).unwrap()).is_err());
    p = f.payload("p", "方案");
    p["tool_input"] = serde_json::json!({"planFilePath":"/only/path"});
    assert!(parse(&serde_json::to_vec(&p).unwrap()).is_err());
}
#[test]
fn replay_never_extends_ttl_and_conflicting_body_revokes_existing_reference() {
    let mut f = Fixture::new();
    f.ingest("p", "首版").unwrap();
    let doc = f.doc("p");
    let r = f.bind(&doc).unwrap();
    f.now += TTL - Duration::from_secs(1);
    assert_eq!(f.ingest("p", "首版"), Ok(Receipt::Duplicate));
    f.now += Duration::from_secs(1);
    assert!(f.read(&doc, &r).is_none());
    assert!(f.store.entries.is_empty());
    f.ingest("p", "首版").unwrap();
    let r = f.bind(&doc).unwrap();
    assert_eq!(f.ingest("p", "冲突版"), Err(Error::Conflict));
    assert!(f.read(&doc, &r).is_none());
    assert_eq!(f.ingest("p", "首版"), Err(Error::Conflict));
    assert!(f.bind(&doc).is_none());
    f.now += TTL;
    f.ingest("p", "冲突版").unwrap();
    let next = f.bind(&doc).unwrap();
    assert_ne!(r, next);
    assert!(f.read(&doc, &r).is_none());
}
#[test]
fn private_capture_is_not_stored_and_revokes_old_body_without_logging_it() {
    let mut f = Fixture::new();
    f.ingest("p", "公开方案").unwrap();
    let doc = f.doc("p");
    let r = f.bind(&doc).unwrap();
    let mut private = f.payload("p", "临时私密正文");
    private["tool_input"]["isSecret"] = true.into();
    assert_eq!(
        f.store
            .ingest(&serde_json::to_vec(&private).unwrap(), f.now),
        Err(Error::Private)
    );
    assert!(f.read(&doc, &r).is_none());
    assert!(f.store.entries.iter().all(|e| e.body.is_none()));
    assert_eq!(
        f.ingest("secret", "password=fixture-only"),
        Err(Error::Private)
    );
    assert!(f.store.entries.iter().all(|e| e.body.is_none()));
}
#[test]
fn main_message_input_and_file_identity_are_required_not_tool_id_alone() {
    let mut f = Fixture::new();
    f.ingest("p", "方案").unwrap();
    let doc = f.doc("p");
    let mut child = doc.clone();
    child["isSidechain"] = true.into();
    assert!(f.bind(&child).is_none());
    let mut foreign = doc.clone();
    foreign["sessionId"] = MESSAGE.into();
    assert!(f.bind(&foreign).is_none());
    let mut altered = doc.clone();
    altered["message"]["content"][0]["input"] = serde_json::json!({"allowedPrompts":[]});
    assert!(f.bind(&altered).is_none());
    let r = f.bind(&doc).unwrap();
    let mut reused = doc.clone();
    reused["uuid"] = "019c6e27-e55b-73d1-87d8-4e01f1f75044".into();
    assert!(f.bind(&reused).is_none());
    assert!(f.read(&reused, &r).is_none());
    assert!(f.store.bind_selected(&f.path,&doc,&serde_json::json!({"type":"tool_use","id":"p","name":"ExitPlanMode","input":{"different":1}}),f.now).is_none());
    std::fs::rename(&f.path, f.root.join("old.jsonl")).unwrap();
    std::fs::write(&f.path, "").unwrap();
    assert!(f.read(&doc, &r).is_none());
    assert!(f.bind(&doc).is_none());
}
#[cfg(unix)]
#[test]
fn sources_outside_root_symlinks_wrong_names_and_writable_files_are_rejected() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let mut f = Fixture::new();
    let original = f.path.clone();
    let external = crate::testutil::Sandbox::new("claude-outside");
    let external_path = external
        .path()
        .canonicalize()
        .unwrap()
        .join(format!("{SESSION}.jsonl"));
    std::fs::write(&external_path, "").unwrap();
    f.path = external_path;
    assert_eq!(f.ingest("p", "方案"), Err(Error::Source));
    f.path = original.clone();
    std::fs::rename(&f.path, f.root.join("original.jsonl")).unwrap();
    symlink(f.root.join("original.jsonl"), &f.path).unwrap();
    assert_eq!(f.ingest("p", "方案"), Err(Error::Source));
    std::fs::remove_file(&f.path).unwrap();
    std::fs::write(&f.path, "").unwrap();
    std::fs::set_permissions(&f.path, std::fs::Permissions::from_mode(0o666)).unwrap();
    assert_eq!(f.ingest("p", "方案"), Err(Error::Source));
    std::fs::set_permissions(&f.path, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(f.ingest("p", "方案"), Ok(Receipt::Captured));
    f.path = f.root.join("different.jsonl");
    std::fs::write(&f.path, "").unwrap();
    assert_eq!(f.ingest("other", "方案"), Err(Error::Source));
    f.path = f.root.join("link").join(format!("{SESSION}.jsonl"));
    symlink(&f.root, f.root.join("link")).unwrap();
    assert_eq!(f.ingest("other", "方案"), Err(Error::Source));
}
#[test]
fn capacity_evicts_oldest_without_extending_replayed_entries() {
    let mut f = Fixture::new();
    let first = f.doc("p0");
    f.ingest("p0", &"a".repeat(MAX_BODY)).unwrap();
    let r = f.bind(&first).unwrap();
    for i in 1..MAX_ITEMS {
        f.ingest(&format!("p{i}"), &"a".repeat(MAX_BODY)).unwrap();
    }
    assert_eq!(f.store.entries.len(), MAX_ITEMS);
    assert_eq!(
        f.store
            .entries
            .iter()
            .filter_map(|e| e.body.as_ref())
            .map(String::len)
            .sum::<usize>(),
        MAX_BYTES
    );
    assert_eq!(
        f.ingest("p0", &"a".repeat(MAX_BODY)),
        Ok(Receipt::Duplicate)
    );
    f.ingest("overflow", "后续方案").unwrap();
    assert_eq!(f.store.entries.len(), MAX_ITEMS);
    assert!(f.read(&first, &r).is_none());
}
#[test]
fn decreasing_clock_is_not_treated_as_infinite_lifetime() {
    let mut f = Fixture::new();
    f.ingest("p", "方案").unwrap();
    let doc = f.doc("p");
    let r = f.bind(&doc).unwrap();
    f.now -= Duration::from_secs(1);
    assert!(f.read(&doc, &r).is_none());
}
#[cfg(unix)]
#[test]
fn replaced_namespace_cannot_reuse_old_leaf_hardlink_or_rebind_old_reference() {
    let mut f = Fixture::new();
    f.ingest("p", "原目录方案").unwrap();
    let doc = f.doc("p");
    let r = f.bind(&doc).unwrap();
    let old = f.root.with_extension("moved");
    std::fs::rename(&f.root, &old).unwrap();
    std::fs::create_dir(&f.root).unwrap();
    std::fs::hard_link(old.join(f.path.file_name().unwrap()), &f.path).unwrap();
    assert!(f.read(&doc, &r).is_none());
    assert!(f.bind(&doc).is_none());
    f.ingest("p", "原目录方案").unwrap();
    let changed = f.bind(&doc).unwrap();
    assert_ne!(r, changed);
    assert!(f.read(&doc, &r).is_none());
    assert_eq!(f.read(&doc, &changed).as_deref(), Some("原目录方案"));
    std::fs::remove_dir_all(&old).unwrap();
}
#[test]
fn reader_validity_uses_original_capture_deadline_and_duplicate_does_not_extend_it() {
    let mut f = Fixture::new();
    f.ingest("p", "临时正文").unwrap();
    let doc = f.doc("p");
    let reference = f.bind(&doc).unwrap();
    f.now += Duration::from_secs(60);
    f.ingest("p", "临时正文").unwrap();
    let (_, remaining) = f
        .store
        .read_bound_with_ttl(
            &f.path,
            &doc,
            &doc["message"]["content"][0],
            &reference,
            f.now,
        )
        .unwrap();
    assert_eq!(remaining, 14 * 60 * 1000);
    f.now += Duration::from_secs(14 * 60);
    assert!(f
        .store
        .read_bound_with_ttl(
            &f.path,
            &doc,
            &doc["message"]["content"][0],
            &reference,
            f.now
        )
        .is_none());
}
#[cfg(target_os = "macos")]
#[test]
#[ignore = "requires controlled real-client fixture under .scratch/claude-real-client"]
fn real_client_hook_matches_main_transcript_and_remains_read_only() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join(".scratch/claude-real-client")
        .canonicalize()
        .unwrap();
    let bytes = std::fs::read(root.join("hook-input.json")).unwrap();
    let hook: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(hook["hook_event_name"], "PreToolUse");
    assert_eq!(hook["tool_name"], "ExitPlanMode");
    let path = PathBuf::from(hook["transcript_path"].as_str().unwrap())
        .canonicalize()
        .unwrap();
    let trusted = root.join("session-config/projects").canonicalize().unwrap();
    assert!(path.starts_with(&trusted));
    let docs: Vec<Value> = std::fs::read_to_string(&path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let matching: Vec<(&Value, &Value)> = docs
        .iter()
        .filter(|doc| doc["type"] == "assistant")
        .flat_map(|doc| {
            doc["message"]["content"]
                .as_array()
                .into_iter()
                .flatten()
                .map(move |tool| (doc, tool))
        })
        .filter(|(_, tool)| tool["type"] == "tool_use" && tool["id"] == hook["tool_use_id"])
        .collect();
    assert_eq!(matching.len(), 1);
    let (doc, tool) = matching[0];
    assert_eq!(doc["sessionId"], hook["session_id"]);
    let mut store = Store::default();
    store.enable(vec![trusted]).unwrap();
    let now = Instant::now();
    assert_eq!(store.ingest(&bytes, now), Ok(Receipt::Captured));
    // This client persists injected plan text itself. The transient overlay must not take it over.
    assert!(tool["input"]["plan"]
        .as_str()
        .is_some_and(|text| !text.is_empty()));
    assert!(store.bind_selected(&path, doc, tool, now).is_none());
    let output: Vec<Value> =
        serde_json::from_slice(&std::fs::read(root.join("client-output.json")).unwrap()).unwrap();
    assert!(output
        .iter()
        .any(|r| r["type"] == "control_request" && r["request"]["subtype"] == "can_use_tool"));
    assert!(docs.iter().any(|doc| doc["message"]["content"]
        .as_array()
        .is_some_and(
            |items| items.iter().any(|item| item["type"] == "tool_result"
                && item["tool_use_id"] == hook["tool_use_id"]
                && item["is_error"] == true)
        )));
    store.disable();
    assert_eq!(store.ingest(&bytes, now), Err(Error::Disabled));
}
