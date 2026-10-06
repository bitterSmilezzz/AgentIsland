//! Manual metadata discovery; no task state, prompt preview, or persistent path index.
use crate::{models::AgentProfile, session_navigation, tasks};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs::{File, Metadata, OpenOptions},
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};
const MAX_HEADER: u64 = 2 * 1024 * 1024;
const MAX_DEPTH: usize = 4;
#[derive(Clone, Serialize)]
pub struct Item {
    pub source: tasks::Source,
    pub name: String,
    pub modified_ms: Option<u64>,
    pub archived: bool,
    pub target: session_navigation::Target,
}
#[derive(Default, Serialize)]
pub struct Gaps {
    pub unreadable: usize,
    pub invalid: usize,
    pub compressed: usize,
    pub deep_directories: usize,
    pub missing_roots: usize,
}
#[derive(Serialize)]
pub struct Catalog {
    pub generation: String,
    pub items: Vec<Item>,
    pub gaps: Gaps,
}
#[derive(Deserialize)]
struct Header {
    #[serde(rename = "type")]
    kind: String,
    payload: Meta,
}
#[derive(Deserialize)]
struct Meta {
    id: String,
}
#[derive(PartialEq)]
struct FileIdentity {
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
    #[cfg(not(unix))]
    created: Option<std::time::SystemTime>,
}
fn file_identity(meta: &Metadata) -> FileIdentity {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        FileIdentity {
            device: meta.dev(),
            inode: meta.ino(),
        }
    }
    #[cfg(not(unix))]
    {
        FileIdentity {
            created: meta.created().ok(),
        }
    }
}
fn open(path: &Path) -> Result<File, ()> {
    if !std::fs::symlink_metadata(path).map_err(|_| ())?.is_file() {
        return Err(());
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path).map_err(|_| ())?;
    if !file.metadata().map_err(|_| ())?.is_file() {
        return Err(());
    }
    Ok(file)
}
enum HeaderError {
    Unreadable,
    Invalid,
}
fn header(file: &File) -> Result<String, HeaderError> {
    let mut bytes = Vec::new();
    BufReader::new(file.take(MAX_HEADER + 1))
        .read_until(b'\n', &mut bytes)
        .map_err(|_| HeaderError::Unreadable)?;
    if bytes.len() as u64 > MAX_HEADER {
        return Err(HeaderError::Invalid);
    }
    let parsed: Header = serde_json::from_slice(&bytes).map_err(|_| HeaderError::Invalid)?;
    if parsed.kind != "session_meta" {
        return Err(HeaderError::Invalid);
    }
    uuid::Uuid::parse_str(&parsed.payload.id)
        .map(|id| id.to_string())
        .map_err(|_| HeaderError::Invalid)
}
struct Entry {
    path: PathBuf,
    item: Item,
    identity: FileIdentity,
}
pub struct Store {
    profile: AgentProfile,
    roots: Vec<(PathBuf, bool)>,
    entries: HashMap<String, Entry>,
    generation: String,
}
impl Store {
    pub fn at_default() -> Self {
        let profile = crate::registry::builtin()
            .into_iter()
            .find(|profile| profile.id == "codex")
            .expect("builtin codex");
        let root = PathBuf::from(&profile.session_dirs[0]);
        let archive = root.parent().expect("codex home").join("archived_sessions");
        Self::new(profile, vec![(root, false), (archive, true)])
    }
    fn new(profile: AgentProfile, roots: Vec<(PathBuf, bool)>) -> Self {
        Self {
            profile,
            roots,
            entries: HashMap::new(),
            generation: String::new(),
        }
    }
    pub fn read(&mut self) -> Catalog {
        let mut entries = HashMap::new();
        let mut gaps = Gaps::default();
        for (root, archived) in &self.roots {
            match std::fs::symlink_metadata(root) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    gaps.missing_roots += 1;
                    continue;
                }
                Ok(meta) if meta.is_dir() => {}
                _ => {
                    gaps.unreadable += 1;
                    continue;
                }
            }
            for found in walkdir::WalkDir::new(root)
                .follow_links(false)
                .max_depth(MAX_DEPTH)
            {
                let found = match found {
                    Ok(found) => found,
                    Err(_) => {
                        gaps.unreadable += 1;
                        continue;
                    }
                };
                if found.file_type().is_dir() {
                    if found.depth() == MAX_DEPTH {
                        gaps.deep_directories += 1;
                    }
                    continue;
                }
                let path = found.path();
                if path.to_string_lossy().ends_with(".jsonl.zst") {
                    gaps.compressed += 1;
                    continue;
                }
                if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                    continue;
                }
                if !found.file_type().is_file() {
                    gaps.unreadable += 1;
                    continue;
                }
                let Some(path_text) = path.to_str() else {
                    gaps.invalid += 1;
                    continue;
                };
                let Some(target) = session_navigation::resolve(&self.profile, Some(path_text))
                    .filter(|target| target.exact_session)
                else {
                    gaps.invalid += 1;
                    continue;
                };
                let file = match open(path) {
                    Ok(file) => file,
                    Err(_) => {
                        gaps.unreadable += 1;
                        continue;
                    }
                };
                let id = match header(&file) {
                    Ok(id) => id,
                    Err(HeaderError::Unreadable) => {
                        gaps.unreadable += 1;
                        continue;
                    }
                    Err(HeaderError::Invalid) => {
                        gaps.invalid += 1;
                        continue;
                    }
                };
                if target.url.as_deref() != Some(format!("codex://threads/{id}").as_str()) {
                    gaps.invalid += 1;
                    continue;
                }
                let meta = match file.metadata() {
                    Ok(meta) => meta,
                    Err(_) => {
                        gaps.unreadable += 1;
                        continue;
                    }
                };
                let source = tasks::Source {
                    agent_id: self.profile.id.clone(),
                    session_id: format!("{:x}", Sha256::digest(path_text.as_bytes())),
                    thread_id: Some(id),
                };
                let modified_ms = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .and_then(|t| u64::try_from(t.as_millis()).ok());
                let item = Item {
                    source: source.clone(),
                    name: "Codex".into(),
                    modified_ms,
                    archived: *archived,
                    target,
                };
                entries.insert(
                    source.session_id,
                    Entry {
                        path: path.to_owned(),
                        item,
                        identity: file_identity(&meta),
                    },
                );
            }
        }
        let mut items: Vec<_> = entries.values().map(|entry| entry.item.clone()).collect();
        items.sort_by(|a, b| {
            b.modified_ms
                .cmp(&a.modified_ms)
                .then_with(|| a.source.session_id.cmp(&b.source.session_id))
        });
        self.generation = uuid::Uuid::new_v4().to_string();
        self.entries = entries;
        Catalog {
            generation: self.generation.clone(),
            items,
            gaps,
        }
    }
    pub fn target(
        &self,
        generation: &str,
        source: &tasks::Source,
    ) -> Result<session_navigation::Target, String> {
        if self.generation.is_empty() || generation != self.generation {
            return Err("目录已更新，请重新读取".into());
        }
        let entry = self
            .entries
            .get(&source.session_id)
            .filter(|entry| entry.item.source == *source)
            .ok_or("会话来源不在当前目录")?;
        let file = open(&entry.path).map_err(|_| "会话文件已不可读，请重新读取目录")?;
        if file_identity(&file.metadata().map_err(|_| "会话文件不可读")?) != entry.identity
            || header(&file).ok().as_ref() != source.thread_id.as_ref()
        {
            return Err("会话文件已变化，请重新读取目录".into());
        }
        Ok(entry.item.target.clone())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    struct Sandbox(PathBuf);
    impl Sandbox {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("agentisland-catalog-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn profile() -> AgentProfile {
        crate::registry::builtin()
            .into_iter()
            .find(|p| p.id == "codex")
            .unwrap()
    }
    fn rollout(root: &Path, id: &str) -> PathBuf {
        root.join(format!("rollout-2026-10-05-{id}.jsonl"))
    }
    fn write(path: &Path, id: &str) {
        std::fs::write(path,format!("{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"{id}\",\"cwd\":\"private-fixture-directory\",\"creator_account_id\":\"private-fixture-account\",\"base_instructions\":\"private-fixture-instruction\"}}}}\n{{\"type\":\"response_item\",\"payload\":{{\"message\":\"private-fixture-body\"}}}}\n")).unwrap();
    }
    #[test]
    fn directory_checks_protocol_and_reports_gaps_without_private_fields() {
        let root = Sandbox::new();
        let archive = Sandbox::new();
        let id = uuid::Uuid::new_v4().to_string();
        write(&rollout(root.path(), &id), &id);
        let archived = uuid::Uuid::new_v4().to_string();
        write(&rollout(archive.path(), &archived), &archived);
        write(
            &rollout(root.path(), &uuid::Uuid::new_v4().to_string()),
            &id,
        );
        std::fs::write(
            root.path().join("rollout-compressed.jsonl.zst"),
            b"not-read",
        )
        .unwrap();
        std::fs::write(root.path().join("random.jsonl"), b"not-read").unwrap();
        std::fs::create_dir_all(root.path().join("a/b/c/d/e")).unwrap();
        let mut store = Store::new(
            profile(),
            vec![
                (root.path().to_owned(), false),
                (archive.path().to_owned(), true),
                (root.path().join("missing"), false),
            ],
        );
        let result = store.read();
        assert_eq!(result.items.len(), 2);
        assert_eq!(result.gaps.invalid, 2);
        assert_eq!(result.gaps.compressed, 1);
        assert_eq!(result.gaps.deep_directories, 1);
        assert_eq!(result.gaps.missing_roots, 1);
        assert_eq!(result.items.iter().filter(|item| item.archived).count(), 1);
        let encoded = serde_json::to_string(&result).unwrap();
        assert!(!encoded.contains("private-fixture"));
        assert!(!encoded.contains(root.path().to_str().unwrap()));
    }
    #[test]
    fn catalog_navigation_accepts_append_but_rejects_changed_identity_and_generation() {
        let root = Sandbox::new();
        let id = uuid::Uuid::new_v4().to_string();
        let path = rollout(root.path(), &id);
        write(&path, &id);
        let mut store = Store::new(profile(), vec![(root.path().to_owned(), false)]);
        let result = store.read();
        let source = &result.items[0].source;
        assert!(
            store
                .target(&result.generation, source)
                .unwrap()
                .exact_session
        );
        OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"{}\n")
            .unwrap();
        assert!(store.target(&result.generation, source).is_ok());
        let mut changed = source.clone();
        changed.thread_id = None;
        assert!(store.target(&result.generation, &changed).is_err());
        write(&path, &uuid::Uuid::new_v4().to_string());
        assert!(store.target(&result.generation, source).is_err());
        write(&path, &id);
        let next = store.read();
        assert!(store.target(&result.generation, source).is_err());
        let replacement = root.path().join("replacement");
        write(&replacement, &id);
        std::fs::rename(replacement, &path).unwrap();
        assert!(store
            .target(&next.generation, &next.items[0].source)
            .is_err());
        std::fs::remove_file(&path).unwrap();
        assert!(store
            .target(&next.generation, &next.items[0].source)
            .is_err());
    }
    #[test]
    fn full_directory_does_not_silently_stop_at_four_thousand_files() {
        let root = Sandbox::new();
        for _ in 0..4101 {
            let id = uuid::Uuid::new_v4().to_string();
            write(&rollout(root.path(), &id), &id);
        }
        let mut store = Store::new(profile(), vec![(root.path().to_owned(), false)]);
        let result = store.read();
        assert_eq!(result.items.len(), 4101);
        assert_eq!(result.gaps.invalid, 0);
    }
    #[cfg(unix)]
    #[test]
    fn symlink_entries_and_roots_are_not_followed() {
        use std::os::unix::fs::symlink;
        let root = Sandbox::new();
        let external = Sandbox::new();
        let id = uuid::Uuid::new_v4().to_string();
        let path = rollout(external.path(), &id);
        write(&path, &id);
        symlink(&path, rollout(root.path(), &id)).unwrap();
        symlink(external.path(), root.path().join("linked-root")).unwrap();
        let mut store = Store::new(
            profile(),
            vec![
                (root.path().to_owned(), false),
                (root.path().join("linked-root"), false),
            ],
        );
        let result = store.read();
        assert!(result.items.is_empty());
        assert_eq!(result.gaps.unreadable, 2);
    }
}
