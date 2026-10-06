//! Read-only Claude hook protocol and ephemeral cache. No transport, config writer or decisions.
//! The receiver must authenticate its local peer before calling `ingest`; file/session correlation
//! here does not prove that an editable local transcript was authored by Claude.
use serde::{
    de::{self, MapAccess, SeqAccess, Visitor},
    Deserialize, Deserializer,
};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashSet, VecDeque},
    path::{Component, Path, PathBuf},
    time::{Duration, Instant, SystemTime},
};

pub(crate) const MAX_BODY: usize = 64 * 1024;
const MAX_INPUT: usize = 512 * 1024;
const MAX_ITEMS: usize = 32;
const MAX_BYTES: usize = 2 * 1024 * 1024;
const TTL: Duration = Duration::from_secs(15 * 60);
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn valid_id(id: &str) -> bool {
    !id.trim().is_empty() && id.len() <= 256 && !id.chars().any(char::is_control)
}
fn session_id(id: &str) -> Option<String> {
    let u = uuid::Uuid::parse_str(id).ok()?;
    (!u.is_nil() && u.to_string() == id).then(|| id.to_string())
}
fn private(input: &Value) -> bool {
    ["isSecret", "is_secret"].iter().any(|key| {
        input
            .get(key)
            .is_some_and(|value| value.as_bool() != Some(false))
    })
}
fn plain_input(input: &Value) -> Option<Value> {
    let mut object = input.as_object()?.clone();
    object.remove("plan");
    object.remove("planFilePath");
    Some(Value::Object(object))
}

// Reject duplicate keys at every depth; last-key-wins can otherwise change the event identity.
struct Strict(Value);
impl<'de> Deserialize<'de> for Strict {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Strict;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("unambiguous JSON")
            }
            fn visit_bool<E: de::Error>(self, v: bool) -> Result<Strict, E> {
                Ok(Strict(Value::Bool(v)))
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<Strict, E> {
                Ok(Strict(Value::Number(v.into())))
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Strict, E> {
                Ok(Strict(Value::Number(v.into())))
            }
            fn visit_f64<E: de::Error>(self, v: f64) -> Result<Strict, E> {
                serde_json::Number::from_f64(v)
                    .map(|n| Strict(Value::Number(n)))
                    .ok_or_else(|| E::custom("invalid number"))
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<Strict, E> {
                Ok(Strict(Value::String(v.into())))
            }
            fn visit_string<E: de::Error>(self, v: String) -> Result<Strict, E> {
                Ok(Strict(Value::String(v)))
            }
            fn visit_unit<E: de::Error>(self) -> Result<Strict, E> {
                Ok(Strict(Value::Null))
            }
            fn visit_none<E: de::Error>(self) -> Result<Strict, E> {
                Ok(Strict(Value::Null))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut a: A) -> Result<Strict, A::Error> {
                let mut rows = Vec::new();
                while let Some(v) = a.next_element::<Strict>()? {
                    rows.push(v.0);
                }
                Ok(Strict(Value::Array(rows)))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut a: A) -> Result<Strict, A::Error> {
                let mut object = Map::new();
                let mut keys = HashSet::new();
                while let Some(key) = a.next_key::<String>()? {
                    if !keys.insert(key.clone()) {
                        return Err(de::Error::custom("duplicate key"));
                    }
                    object.insert(key, a.next_value::<Strict>()?.0);
                }
                Ok(Strict(Value::Object(object)))
            }
        }
        d.deserialize_any(V)
    }
}

pub(crate) fn strict_json(bytes: &[u8]) -> Result<Value, ()> {
    if bytes.len() > MAX_INPUT {
        return Err(());
    }
    serde_json::from_slice::<Strict>(bytes)
        .map(|v| v.0)
        .map_err(|_| ())
}
// Neither the payload, cache nor body implements Debug/Serialize.
struct Payload {
    path: PathBuf,
    session: String,
    call: String,
    input_hash: String,
    body: Option<String>,
}
fn parse(bytes: &[u8]) -> Result<Payload, Error> {
    if bytes.len() > MAX_INPUT {
        return Err(Error::Invalid);
    }
    let doc = strict_json(bytes).map_err(|_| Error::Invalid)?;
    if doc.get("hook_event_name").and_then(Value::as_str) != Some("PreToolUse")
        || doc.get("tool_name").and_then(Value::as_str) != Some("ExitPlanMode")
    {
        return Err(Error::Invalid);
    }
    let session = session_id(
        doc.get("session_id")
            .and_then(Value::as_str)
            .ok_or(Error::Invalid)?,
    )
    .ok_or(Error::Invalid)?;
    let call = doc
        .get("tool_use_id")
        .and_then(Value::as_str)
        .filter(|s| valid_id(s))
        .ok_or(Error::Invalid)?;
    let path = doc
        .get("transcript_path")
        .and_then(Value::as_str)
        .ok_or(Error::Invalid)?;
    if path.len() > 4096 || path.chars().any(char::is_control) {
        return Err(Error::Invalid);
    }
    let input = doc.get("tool_input").ok_or(Error::Invalid)?;
    let body = input
        .get("plan")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty() && s.len() <= MAX_BODY)
        .ok_or(Error::Invalid)?;
    let permitted = !private(&doc) && !private(input) && !crate::private_text::known_private(body);
    let original = plain_input(input).ok_or(Error::Invalid)?;
    let input_hash = digest(&serde_json::to_vec(&original).map_err(|_| Error::Invalid)?);
    Ok(Payload {
        path: PathBuf::from(path),
        session,
        call: digest(call.as_bytes()),
        input_hash,
        body: permitted.then(|| body.to_string()),
    })
}

#[derive(Clone, PartialEq, Eq)]
struct FileIdentity {
    device: u64,
    inode: u64,
    created: SystemTime,
}
#[derive(Clone, PartialEq, Eq)]
struct SourceIdentity {
    path_hash: String,
    file: FileIdentity,
    namespace: Vec<FileIdentity>,
}
fn source_ref(source: &SourceIdentity) -> String {
    let mut hash = Sha256::new();
    hash.update(b"claude-hook-source-v1");
    hash.update(source.path_hash.as_bytes());
    hash.update((source.namespace.len() as u64).to_le_bytes());
    for file in source.namespace.iter().chain(std::iter::once(&source.file)) {
        hash.update(file.device.to_le_bytes());
        hash.update(file.inode.to_le_bytes());
        let (sign, age) = match file.created.duration_since(SystemTime::UNIX_EPOCH) {
            Ok(age) => (0u8, age),
            Err(error) => (1u8, error.duration()),
        };
        hash.update([sign]);
        hash.update(age.as_secs().to_le_bytes());
        hash.update(age.subsec_nanos().to_le_bytes());
    }
    format!("{:x}", hash.finalize())
}
fn absolute_plain(path: &Path) -> bool {
    path.is_absolute()
        && path
            .components()
            .all(|c| matches!(c, Component::RootDir | Component::Normal(_)))
}
#[cfg(unix)]
fn source(path: &Path, roots: &[PathBuf], session: &str) -> Result<SourceIdentity, Error> {
    use std::{
        ffi::CString,
        os::{
            fd::{AsRawFd, FromRawFd},
            unix::{ffi::OsStrExt, fs::MetadataExt},
        },
    };
    fn identity(m: &std::fs::Metadata) -> Result<FileIdentity, Error> {
        Ok(FileIdentity {
            device: m.dev(),
            inode: m.ino(),
            created: m.created().map_err(|_| Error::Source)?,
        })
    }
    if !absolute_plain(path)
        || path.extension().and_then(|s| s.to_str()) != Some("jsonl")
        || path.file_stem().and_then(|s| s.to_str()) != Some(session)
    {
        return Err(Error::Source);
    }
    let root = roots
        .iter()
        .find(|root| {
            path.strip_prefix(root)
                .is_ok_and(|p| p.components().count() <= 5 && p.components().count() > 0)
        })
        .ok_or(Error::Source)?;
    let mut directory = std::fs::File::open("/").map_err(|_| Error::Source)?;
    let mut current = PathBuf::from("/");
    let mut opened = Vec::new();
    let mut namespace = Vec::new();
    for part in path
        .components()
        .filter(|p| matches!(p, Component::Normal(_)))
    {
        current.push(part.as_os_str());
        let leaf = current == path;
        let name = CString::new(part.as_os_str().as_bytes()).map_err(|_| Error::Source)?;
        let flags = libc::O_RDONLY
            | libc::O_CLOEXEC
            | libc::O_NOFOLLOW
            | libc::O_NONBLOCK
            | if leaf { 0 } else { libc::O_DIRECTORY };
        // Each component is opened relative to the checked parent descriptor, never followed by name.
        let fd = unsafe { libc::openat(directory.as_raw_fd(), name.as_ptr(), flags) };
        if fd < 0 {
            return Err(Error::Source);
        }
        let next = unsafe { std::fs::File::from_raw_fd(fd) };
        let meta = next.metadata().map_err(|_| Error::Source)?;
        if (leaf && !meta.is_file()) || (!leaf && !meta.is_dir()) {
            return Err(Error::Source);
        }
        if current.starts_with(root)
            && (meta.uid() != unsafe { libc::geteuid() } || meta.mode() & 0o022 != 0)
        {
            return Err(Error::Source);
        }
        let id = identity(&meta)?;
        if current.starts_with(root) && !leaf {
            namespace.push(id.clone());
        }
        opened.push((current.clone(), id));
        directory = next;
    }
    // Names must still point to the opened chain; a replaced root containing a hard link to the
    // old file also fails because the cache retains directory identities, not just the leaf inode.
    for (name, id) in &opened {
        let meta = std::fs::symlink_metadata(name).map_err(|_| Error::Source)?;
        if meta.file_type().is_symlink() || identity(&meta)? != *id {
            return Err(Error::Source);
        }
    }
    let file = identity(&directory.metadata().map_err(|_| Error::Source)?)?;
    Ok(SourceIdentity {
        path_hash: digest(path.as_os_str().as_encoded_bytes()),
        file,
        namespace,
    })
}
#[cfg(not(unix))]
fn source(_: &Path, _: &[PathBuf], _: &str) -> Result<SourceIdentity, Error> {
    Err(Error::Source)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Error {
    Disabled,
    Invalid,
    Private,
    Source,
    Conflict,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Receipt {
    Captured,
    Duplicate,
}
struct Entry {
    source: SourceIdentity,
    session: String,
    call: String,
    input_hash: String,
    body: Option<String>,
    body_hash: String,
    created: Instant,
    binding: Option<String>,
}
/// Opt-in memory only. Transport/config layers are separate and are not implemented by this module.
pub(crate) struct Store {
    roots: Vec<PathBuf>,
    enabled: bool,
    entries: VecDeque<Entry>,
}
impl Default for Store {
    fn default() -> Self {
        Self {
            roots: vec![],
            enabled: false,
            entries: VecDeque::new(),
        }
    }
}
impl Store {
    /// Only the installer/receiver lifecycle may enable this, after configuration readback.
    pub(crate) fn enable(&mut self, roots: Vec<PathBuf>) -> Result<(), Error> {
        self.disable();
        if roots.is_empty() || roots.len() > 2 || roots.iter().any(|p| !absolute_plain(p)) {
            return Err(Error::Source);
        }
        self.roots = roots;
        self.enabled = true;
        Ok(())
    }
    pub(crate) fn disable(&mut self) {
        self.entries.clear();
        self.roots.clear();
        self.enabled = false;
    }
    pub(crate) fn purge_expired(&mut self, now: Instant) {
        self.entries.retain(|e| {
            now.checked_duration_since(e.created)
                .is_some_and(|elapsed| elapsed < TTL)
        });
    }
    /// Requires an authenticated local transport; this method only enforces payload/source policy.
    pub(crate) fn ingest(&mut self, bytes: &[u8], now: Instant) -> Result<Receipt, Error> {
        if !self.enabled {
            return Err(Error::Disabled);
        }
        self.purge_expired(now);
        let p = parse(bytes)?;
        let identity = source(&p.path, &self.roots, &p.session)?;
        let body_hash = p
            .body
            .as_ref()
            .map(|body| digest(body.as_bytes()))
            .unwrap_or_default();
        let rejected = p.body.is_none();
        if let Some(old) = self
            .entries
            .iter_mut()
            .find(|e| e.source == identity && e.session == p.session && e.call == p.call)
        {
            if !rejected
                && old.body.is_some()
                && old.body_hash == body_hash
                && old.input_hash == p.input_hash
            {
                return Ok(Receipt::Duplicate);
            }
            // Ambiguity poisons this call for the remainder of its original TTL, including old refs.
            old.body = None;
            old.binding = None;
            return Err(if rejected {
                Error::Private
            } else {
                Error::Conflict
            });
        }
        while self.entries.len() >= MAX_ITEMS
            || self
                .entries
                .iter()
                .filter_map(|e| e.body.as_ref())
                .map(String::len)
                .sum::<usize>()
                + p.body.as_ref().map_or(0, String::len)
                > MAX_BYTES
        {
            self.entries.pop_front();
        }
        self.entries.push_back(Entry {
            source: identity,
            session: p.session,
            call: p.call,
            input_hash: p.input_hash,
            body_hash,
            body: p.body,
            created: now,
            binding: None,
        });
        if rejected {
            Err(Error::Private)
        } else {
            Ok(Receipt::Captured)
        }
    }
    fn event(
        &self,
        path: &Path,
        doc: &Value,
        tool: &Value,
    ) -> Option<(SourceIdentity, String, String, String, String)> {
        if !self.enabled {
            return None;
        }
        verified_event(path, doc, tool, &self.roots)
    }
    /// Call ONLY after the semantic probe has selected this pending main call. No task is created here.
    pub(crate) fn bind_selected(
        &mut self,
        path: &Path,
        doc: &Value,
        tool: &Value,
        now: Instant,
    ) -> Option<String> {
        self.purge_expired(now);
        let (source, session, call, input_hash, version) = self.event(path, doc, tool)?;
        let e = self.entries.iter_mut().find(|e| {
            e.source == source
                && e.session == session
                && e.call == call
                && e.input_hash == input_hash
                && e.body.is_some()
        })?;
        if e.binding.as_ref().is_some_and(|bound| bound != &version) {
            return None;
        }
        e.binding = Some(version.clone());
        Some(digest(
            &serde_json::to_vec(&serde_json::json!([
                "claude-hook-plan-v1",
                source_ref(&source),
                version,
                e.body_hash
            ]))
            .ok()?,
        ))
    }
    /// Reads only an already-bound event. An unobserved/changed transcript never adopts a capture.
    pub(crate) fn read_bound(
        &mut self,
        path: &Path,
        doc: &Value,
        tool: &Value,
        event_ref: &str,
        now: Instant,
    ) -> Option<String> {
        self.read_bound_with_ttl(path, doc, tool, event_ref, now)
            .map(|(body, _)| body)
    }
    pub(crate) fn read_bound_with_ttl(
        &mut self,
        path: &Path,
        doc: &Value,
        tool: &Value,
        event_ref: &str,
        now: Instant,
    ) -> Option<(String, u64)> {
        self.purge_expired(now);
        let (source, session, call, input_hash, version) = self.event(path, doc, tool)?;
        let e = self.entries.iter().find(|e| {
            e.source == source
                && e.session == session
                && e.call == call
                && e.input_hash == input_hash
                && e.binding.as_ref() == Some(&version)
        })?;
        let expected = digest(
            &serde_json::to_vec(&serde_json::json!([
                "claude-hook-plan-v1",
                source_ref(&source),
                version,
                e.body_hash
            ]))
            .ok()?,
        );
        if expected != event_ref {
            return None;
        }
        let remaining = TTL.checked_sub(now.checked_duration_since(e.created)?)?;
        let millis = u64::try_from(remaining.as_millis()).ok()?;
        if millis == 0 {
            return None;
        }
        Some((e.body.clone()?, millis))
    }
}

/// Immutable pending event identity does not depend on capture availability or enablement.
fn verified_event(
    path: &Path,
    doc: &Value,
    tool: &Value,
    roots: &[PathBuf],
) -> Option<(SourceIdentity, String, String, String, String)> {
    if private(doc)
        || doc.get("type")?.as_str()? != "assistant"
        || doc
            .get("isSidechain")
            .is_some_and(|value| value.as_bool() != Some(false))
        || tool.get("type")?.as_str()? != "tool_use"
        || tool.get("name")?.as_str()? != "ExitPlanMode"
    {
        return None;
    }
    // The tool must actually belong to this exact main assistant message.
    if !doc
        .get("message")?
        .get("content")?
        .as_array()?
        .iter()
        .any(|item| item == tool)
    {
        return None;
    }
    let session = session_id(doc.get("sessionId")?.as_str()?)?;
    let call = tool.get("id")?.as_str()?.to_string();
    if !valid_id(&call) {
        return None;
    }
    let input = tool.get("input")?;
    if private(input) {
        return None;
    }
    if input
        .get("plan")
        .is_some_and(|value| value.as_str().is_none_or(|text| !text.trim().is_empty()))
    {
        return None;
    }
    let identity = source(path, roots, &session).ok()?;
    let input_hash = digest(&serde_json::to_vec(&plain_input(input)?).ok()?);
    // The event binding includes the main-message identity, not just a reused tool ID.
    let message = doc.get("uuid")?.as_str()?;
    session_id(message)?;
    let version = digest(&serde_json::to_vec(&serde_json::json!([message, tool])).ok()?);
    Some((
        identity,
        session,
        digest(call.as_bytes()),
        input_hash,
        version,
    ))
}
pub(crate) fn event_version(
    path: &Path,
    doc: &Value,
    tool: &Value,
    roots: &[PathBuf],
) -> Option<String> {
    let (source, _, _, _, version) = verified_event(path, doc, tool, roots)?;
    Some(digest(
        &serde_json::to_vec(&serde_json::json!([
            "claude-plan-event-v1",
            source_ref(&source),
            version
        ]))
        .ok()?,
    ))
}

#[cfg(all(test, unix))]
#[path = "claude_plan_capture_tests.rs"]
mod tests;
