//! Bounded enqueue identities and recoverable notifications, separate from session dedup.
//! Only the resident GUI opts into disk IO. Publication runs on a dedicated worker;
//! enqueue/ack only replace a bounded mailbox snapshot, never wait for filesystem IO.
//! A crash before the worker publishes can lose recent enqueues or replay recent acks.
//! Graceful shutdown drains the mailbox; persisted state is the recovery boundary.

use crate::models::AgentTaskEvent;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, VecDeque};
use std::fs::{self, OpenOptions};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Condvar, Mutex};
use std::time::SystemTime;

pub(crate) const CAP: usize = 1024;
pub(crate) const PENDING_CAP: usize = 64;
const MAX_BYTES: u64 = 8 * 1024 * 1024;
const SCHEMA_VERSION: u32 = 1;
const READ_ERROR: &str = "notify ledger unreadable; persistence disabled";
const STALE_ERROR: &str = "notify ledger changed externally; persistence disabled";
const WRITE_ERROR: &str = "notify ledger could not be saved";

#[derive(Default)]
pub(crate) struct Ledger {
    identities: VecDeque<String>,
    /// In-memory payload fingerprint per identity. Conflicting payloads reuse one
    /// explicit id; the first payload wins and the conflict is counted, never
    /// enqueued again. Not persisted: after a restart the first payload wins again.
    fingerprints: HashMap<String, String>,
    pending: VecDeque<StoredEvent>,
    duplicates: u64,
    conflicts: u64,
    /// Lifetime count of enqueued events dropped from the retained queue by the
    /// `PENDING_CAP` bound. The queue is the recovery set, so a dropped event is
    /// neither shown nor restored; without this counter that loss had no readable
    /// exit at all. `identities` is deliberately wider than `pending`, so the
    /// dropped event's identity usually survives and a replay stays a duplicate —
    /// see `accept_hash` for why that is the intended reading of this window.
    evicted: u64,
    writer: Option<Writer>,
}

/// Recovery projection only: no client id, navigation path, process or credential data.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredEvent {
    id: String,
    agent_id: String,
    event_type: String,
    timestamp: i64,
    duration: f64,
    externally_delivered: bool,
}

impl From<&AgentTaskEvent> for StoredEvent {
    fn from(event: &AgentTaskEvent) -> Self {
        Self {
            // Every entry point can carry arbitrary IDs, names and private text.
            id: stored_internal_id(event),
            agent_id: safe_agent_id(&event.agent_id).into(),
            event_type: normalized_kind(&event.event_type).into(),
            timestamp: event.timestamp,
            duration: if event.duration.is_finite() { event.duration.max(0.0) } else { 0.0 },
            externally_delivered: event.externally_delivered,
        }
    }
}
impl StoredEvent {
    fn event(&self) -> AgentTaskEvent {
        AgentTaskEvent {
            id: self.id.clone(),
            agent_id: self.agent_id.clone(),
            agent_name: safe_agent_name(&self.agent_id),
            event_type: self.event_type.clone(),
            timestamp: self.timestamp,
            message: Some("重启恢复的提醒，原始正文未保存，请查看来源".into()),
            detail: None,
            duration: self.duration,
            externally_delivered: self.externally_delivered,
        }
    }
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Payload {
    schema_version: u32,
    identities: VecDeque<String>,
    pending: VecDeque<StoredEvent>,
}

pub(crate) fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| (b' '..=b'~').contains(&b) && !matches!(b, b'"' | b'\'' | b'/' | b'\\'))
}

/// Same length-prefixed field encoding as dedup::identity; explicit IDs have their
/// own domain. Fallback time is HTTP arrival seconds, never the consumer's clock.
pub(crate) fn identity(event: &AgentTaskEvent, client_id: Option<&str>) -> String {
    let seconds = event.timestamp.div_euclid(1000).to_string();
    let parts = match client_id {
        Some(id) => vec!["id", &event.agent_id, normalized_kind(&event.event_type), id],
        None => vec![
            event.agent_id.as_str(),
            normalized_kind(&event.event_type),
            event.message.as_deref().unwrap_or(""),
            event.detail.as_deref().unwrap_or(""),
            &seconds,
        ],
    };
    hash_parts(&parts)
}

fn hash_parts(parts: &[&str]) -> String {
    let mut digest = Sha256::new();
    for part in parts {
        digest.update((part.len() as u64).to_be_bytes());
        digest.update(part.as_bytes());
    }
    format!("{:x}", digest.finalize())
}

fn normalized_kind(kind: &str) -> &str {
    match kind {
        "attention" => "attention",
        "costSpike" | "cost_spike" | "alert" => "costSpike",
        // 持久化身份要保留 NOT-01 的细分类型：两类不同性质的提醒（例如「提问」与
        // 「失败」）必须在幂等分域上可区分，否则同 id 会被跨类型误判成重放。
        // 未知类型仍折叠为 `completed`（这是**有意有损**的投影，与
        // `webhook::normalize_event_kind` 的拒绝语义不同）。
        "question" => "question",
        "failure" => "failure",
        "review" => "review",
        "schedule" => "schedule",
        "chief" => "chief",
        _ => "completed",
    }
}

/// Only declared builtin profile ids (or the fixed `system`/`external` placeholders)
/// may reach disk. Anything else — arbitrary HTTP agent strings, ad-hoc client names,
/// custom or auto-discovered profiles — is stored as `external`. A raw client-supplied
/// value must never survive into persisted state, even as an opaque identifier.
fn safe_agent_id(id: &str) -> &str {
    if id == "system" {
        "system"
    } else {
        declared_agent(id).map(|profile| profile.id.as_str()).unwrap_or("external")
    }
}

fn declared_agent(id: &str) -> Option<&'static crate::models::AgentProfile> {
    static PROFILES: std::sync::OnceLock<Vec<crate::models::AgentProfile>> =
        std::sync::OnceLock::new();
    PROFILES
        .get_or_init(crate::registry::builtin)
        .iter()
        .find(|profile| profile.id == id)
}

/// Static display name for a stored id: registry names for declared profiles, the
/// external placeholder for `external`, and `system` as-is otherwise.
fn safe_agent_name(id: &str) -> String {
    declared_agent(id)
        .map(|profile| profile.name.clone())
        .unwrap_or_else(|| if id == "external" { "外部投递".into() } else { id.into() })
}

/// Payload identity for conflict detection, in the same length-prefixed SHA-256
/// encoding as `identity`. A 64-bit `DefaultHasher` was the previous choice: its
/// collision probability is small per comparison but the consequence of one hit is
/// silent — two genuinely different payloads under one explicit id would be read as
/// a replayed duplicate, so the second body would never be enqueued and the conflict
/// counter would stay at zero. The hash is only compared in memory and never stored.
fn payload_fingerprint(event: &AgentTaskEvent) -> String {
    let duration = event.duration.to_bits().to_string();
    let delivered = if event.externally_delivered { "1" } else { "0" };
    hash_parts(&[
        "payload",
        event.message.as_deref().unwrap_or(""),
        event.detail.as_deref().unwrap_or(""),
        event.agent_name.as_str(),
        &duration,
        delivered,
    ])
}

fn stored_internal_id(event: &AgentTaskEvent) -> String {
    hash_parts(&["stored", &event.agent_id, normalized_kind(&event.event_type), &event.id])
}

impl Ledger {
    /// Startup read is explicit and read-only. Malformed/future state is never repaired.
    pub(crate) fn open_at(path: impl AsRef<Path>) -> Self {
        let path = path.as_ref().to_path_buf();
        let loaded = read_current(&path).and_then(|current| match current {
            None => Ok((
                Payload {
                    schema_version: SCHEMA_VERSION,
                    identities: VecDeque::new(),
                    pending: VecDeque::new(),
                },
                None,
            )),
            Some((bytes, revision)) => Ok((decode(&bytes)?, Some(revision))),
        });
        let (payload, disk) = match loaded {
            Ok((payload, revision)) => (
                Some(payload),
                Disk {
                    path,
                    revision,
                    blocked: false,
                },
            ),
            Err(_) => {
                crate::log_line(READ_ERROR);
                (
                    None,
                    Disk {
                        path,
                        revision: None,
                        blocked: true,
                    },
                )
            }
        };
        let mut ledger = Self::default();
        if let Some(payload) = payload {
            ledger.identities = payload.identities;
            ledger.pending = payload.pending;
        }
        ledger.writer = Some(Writer::start(disk));
        ledger
    }

    pub(crate) fn accept(&mut self, event: &AgentTaskEvent, client_id: Option<&str>) -> bool {
        self.accept_hash(event, identity(event, client_id))
    }

    /// Source IDs for internal observations are scoped by agent and event kind.
    /// Session signals can deliberately reuse the same raw id across these scopes.
    pub(crate) fn accept_internal(&mut self, event: &AgentTaskEvent) -> bool {
        self.accept_hash(
            event,
            hash_parts(&["internal", &event.agent_id, normalized_kind(&event.event_type), &event.id]),
        )
    }

    fn accept_hash(&mut self, event: &AgentTaskEvent, hash: String) -> bool {
        let fingerprint = payload_fingerprint(event);
        if self.identities.contains(&hash) {
            // Only a recorded, differing payload is a conflict. Restored identities
            // carry no persisted payload fingerprint, so a replay after restart is
            // simply a duplicate — never reported as a conflict we cannot prove.
            //
            // An event whose *body* was evicted from `pending` is still a duplicate
            // here, and that is the intended reading of the two bounds: `identities`
            // (CAP 1024) is the idempotency window, `pending` (CAP 64) is only the
            // presentation/recovery window. Eviction may drop the body, never the
            // identity, because the identity is what makes a redelivery idempotent;
            // deleting it would accept the same event again with a body we no longer
            // have, and a producer retrying in a loop would keep re-enqueueing an
            // event the user already saw. The window is bounded, not permanent: once
            // the hash itself is evicted from `identities` (CAP 1024) a later
            // redelivery is a new event again. Both drops are visible — the body drop
            // via `evicted`, the identity drop by the documented window.
            if matches!(self.fingerprints.get(&hash), Some(seen) if seen != &fingerprint) {
                self.conflicts = self.conflicts.saturating_add(1);
            }
            self.duplicates = self.duplicates.saturating_add(1);
            return false;
        }
        self.identities.push_back(hash.clone());
        self.fingerprints.insert(hash, fingerprint);
        while self.identities.len() > CAP {
            if let Some(evicted) = self.identities.pop_front() {
                self.fingerprints.remove(&evicted);
            }
        }
        self.pending.push_back(StoredEvent::from(event));
        while self.pending.len() > PENDING_CAP {
            self.pending.pop_front();
            self.evicted = self.evicted.saturating_add(1);
        }
        self.publish();
        true
    }

    /// Confirm one pending notification.
    ///
    /// The match key is the stored identity plus agent / normalized kind / millisecond.
    /// Two *different* events can share agent, kind and millisecond, so the key cannot
    /// be those three alone: an acknowledgement must never remove a row that belongs to
    /// another event. Rows are in insertion order and `position()` scans front to back,
    /// so a tie among equally-keyed rows resolves to the earliest inserted — the queue
    /// head, which is the only row `ActivityEngine::ack_latest_event` ever acknowledges
    /// (it passes `latest_event`, alias the head). Taking the newest instead would be
    /// indistinguishable in today's call path; earliest is chosen because it is the
    /// conservative reading if a future caller ever acks by identity out of order.
    ///
    /// Exact identity is tried before the derived one. A restored row carries the stored
    /// hash *as* its id (see `StoredEvent::event`), while a live event's raw id has to be
    /// projected through `stored_internal_id`.
    pub(crate) fn ack(&mut self, acknowledged: &AgentTaskEvent) {
        let agent = safe_agent_id(&acknowledged.agent_id);
        let kind = normalized_kind(&acknowledged.event_type);
        let scope = |event: &StoredEvent| {
            event.agent_id == agent && event.event_type == kind && event.timestamp == acknowledged.timestamp
        };
        let derived = stored_internal_id(acknowledged);
        let index = self
            .pending
            .iter()
            .position(|event| scope(event) && event.id == acknowledged.id)
            .or_else(|| {
                self.pending
                    .iter()
                    .position(|event| scope(event) && event.id == derived)
            });
        if let Some(index) = index {
            self.pending.remove(index);
        }
        self.publish();
    }

    pub(crate) fn restored_events(&self) -> VecDeque<AgentTaskEvent> {
        self.pending.iter().map(StoredEvent::event).collect()
    }

    pub(crate) fn duplicates(&self) -> u64 {
        self.duplicates
    }

    /// Explicit id reused with a different payload: first payload wins, counted here.
    pub(crate) fn conflicts(&self) -> u64 {
        self.conflicts
    }

    /// Lifetime enqueued events dropped from the retained queue by `PENDING_CAP`.
    /// In-memory only, like `duplicates`/`conflicts`: it counts what this process
    /// bound did, and the ledger never claims a cumulative-across-restarts figure.
    pub(crate) fn evicted(&self) -> u64 {
        self.evicted
    }

    fn publish(&self) {
        if let Some(writer) = &self.writer {
            writer.publish(Payload {
                schema_version: SCHEMA_VERSION,
                identities: self.identities.clone(),
                pending: self.pending.clone(),
            });
        }
    }

    pub(crate) fn flush_handle(&self) -> Option<FlushHandle> {
        self.writer.as_ref().map(|writer| FlushHandle(writer.mailbox.clone()))
    }

    pub(crate) fn status(&self) -> PersistenceStatus {
        self.flush_handle().map(|handle| handle.status()).unwrap_or_default()
    }

    #[cfg(test)]
    fn flush(&self) -> bool {
        self.flush_handle().is_none_or(|handle| handle.flush(std::time::Duration::from_secs(5)))
    }
}

#[derive(Clone, Debug, Default, serde::Serialize)]
pub(crate) struct PersistenceStatus {
    pub enabled: bool,
    pub healthy: bool,
    pub pending: bool,
    pub saved: bool,
    pub blocked: bool,
    pub error: Option<&'static str>,
    pub generation: u64,
    pub attempted_generation: u64,
    pub saved_generation: u64,
    pub failures: u64,
}

/// A read-only worker capability. No engine lock, UI dispatch or thread join.
#[derive(Clone)]
pub(crate) struct FlushHandle(Arc<(Mutex<Mailbox>, Condvar)>);
impl FlushHandle {
    pub(crate) fn status(&self) -> PersistenceStatus {
        self.0.0.lock().unwrap().status.clone()
    }
    pub(crate) fn shutdown(&self, timeout: std::time::Duration) -> bool {
        self.0.0.lock().unwrap().closed = true;
        self.flush(timeout)
    }
    pub(crate) fn flush(&self, timeout: std::time::Duration) -> bool {
        let (lock, ready) = &*self.0;
        let state = lock.lock().unwrap();
        let target = state.status.generation;
        let (state, _) = ready.wait_timeout_while(state, timeout, |state| {
            state.status.attempted_generation < target && !state.status.blocked
        }).unwrap();
        !state.status.blocked && state.status.error.is_none() && state.status.saved_generation >= target
    }
}

#[derive(Default)]
struct Mailbox {
    latest: Option<(u64, Payload)>,
    status: PersistenceStatus,
    closed: bool,
}
struct Writer {
    mailbox: Arc<(Mutex<Mailbox>, Condvar)>,
    wake: Option<mpsc::SyncSender<()>>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl Writer {
    fn start(mut disk: Disk) -> Self {
        let mailbox = Arc::new((Mutex::new(Mailbox {
            latest: None,
            closed: false,
            status: PersistenceStatus {
                enabled: true,
                healthy: !disk.blocked,
                blocked: disk.blocked,
                error: disk.blocked.then_some(READ_ERROR),
                ..Default::default()
            },
        }), Condvar::new()));
        let worker_mailbox = mailbox.clone();
        let (wake, rx) = mpsc::sync_channel(1);
        let worker = std::thread::spawn(move || {
            // A closed channel still drains its last wake; no filesystem IO holds
            // either the engine mutex or the mailbox mutex.
            while rx.recv().is_ok() {
                let snapshot = worker_mailbox.0.lock().unwrap().latest.take();
                if let Some((generation, payload)) = snapshot {
                    let result = disk.persist(&payload);
                    if let Err(reason) = result { crate::log_line(reason); }
                    let mut state = worker_mailbox.0.lock().unwrap();
                    state.status.attempted_generation = generation;
                    state.status.blocked = disk.blocked;
                    state.status.error = result.err();
                    state.status.healthy = result.is_ok();
                    if result.is_ok() {
                        state.status.saved_generation = generation;
                    } else {
                        state.status.failures = state.status.failures.saturating_add(1);
                    }
                    state.status.pending = state.status.saved_generation < state.status.generation;
                    state.status.saved = result.is_ok() && !state.status.pending;
                    worker_mailbox.1.notify_all();
                }
            }
        });
        Self {
            mailbox,
            wake: Some(wake),
            worker: Some(worker),
        }
    }
    fn publish(&self, payload: Payload) {
        let mut state = self.mailbox.0.lock().unwrap();
        if state.closed { return; }
        state.status.generation += 1;
        state.status.pending = true;
        state.status.saved = false;
        state.latest = Some((state.status.generation, payload));
        drop(state);
        // `closed` is checked above; take the sender defensively anyway so a racing
        // shutdown can never turn a publish into a panic.
        if let Some(wake) = self.wake.as_ref() {
            let _ = wake.try_send(());
        }
    }
}
impl Drop for Writer {
    fn drop(&mut self) {
        // Production also owns an independent shutdown handle: the engine can
        // outlive RunEvent::Exit in detached sampling threads. Never join forever.
        let _ = FlushHandle(self.mailbox.clone()).shutdown(std::time::Duration::from_secs(2));
        self.wake.take();
        self.worker.take();
    }
}

struct Disk {
    path: PathBuf,
    revision: Option<Revision>,
    blocked: bool,
}
#[derive(PartialEq, Eq)]
struct Revision {
    digest: Vec<u8>,
    modified: Option<SystemTime>,
    #[cfg(unix)]
    identity: (u64, u64),
}
impl Disk {
    fn persist(&mut self, payload: &Payload) -> Result<(), &'static str> {
        if self.blocked {
            return Err(READ_ERROR);
        }
        if !revision_matches(&self.path, &self.revision) {
            self.blocked = true;
            return Err(STALE_ERROR);
        }
        #[cfg(unix)]
        let _writer_lock = if self.revision.is_some() {
            let file = open_regular(&self.path).map_err(|_| WRITE_ERROR)?;
            file.try_lock().map_err(|_| WRITE_ERROR)?;
            Some(file)
        } else {
            None
        };
        if !revision_matches(&self.path, &self.revision) {
            self.blocked = true;
            return Err(STALE_ERROR);
        }
        let bytes = serde_json::to_vec(payload).map_err(|_| WRITE_ERROR)?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err(WRITE_ERROR);
        }
        let parent = self
            .path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .ok_or(WRITE_ERROR)?;
        fs::create_dir_all(parent).map_err(|_| WRITE_ERROR)?;
        let validate = |staged: &Path| {
            let (staged_bytes, _) = read_current(staged)?.ok_or_else(invalid)?;
            decode(&staged_bytes)?;
            if !revision_matches(&self.path, &self.revision) {
                return Err(invalid());
            }
            Ok(())
        };
        let result = if self.revision.is_none() {
            crate::atomicfile::atomic_create_validated(&self.path, &bytes, validate)
        } else {
            crate::atomicfile::atomic_replace_validated(&self.path, &bytes, validate)
        };
        if result.is_err() {
            if !revision_matches(&self.path, &self.revision) {
                self.blocked = true;
                return Err(STALE_ERROR);
            }
            return Err(WRITE_ERROR);
        }
        match read_current(&self.path) {
            Ok(Some((published, revision))) if published == bytes => {
                self.revision = Some(revision);
                Ok(())
            }
            _ => {
                self.blocked = true;
                Err(STALE_ERROR)
            }
        }
    }
}
fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "invalid notify ledger")
}
pub(crate) fn is_hash(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn decode(bytes: &[u8]) -> io::Result<Payload> {
    let payload: Payload = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    if payload.schema_version != SCHEMA_VERSION
        || payload.identities.len() > CAP
        || payload.pending.len() > PENDING_CAP
        || payload.identities.iter().any(|hash| {
            hash.len() != 64
                || !hash
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        })
        || payload.pending.iter().any(|e| {
            !is_hash(&e.id)
                || safe_agent_id(&e.agent_id) != e.agent_id
                || !e.duration.is_finite()
                // 与 `normalized_kind` 的**已识别**集合一致：漏一个就会让含该类型的
                // 账本被判无效文件、进而**永久停写**（本轮实测：新增五类后
                // 落盘被 `decode` 拒绝，`flush` 一直返回 false）。
                // 未知类型仍由 `normalized_kind` 折叠成 `completed`，到不了这里。
                || !matches!(
                    e.event_type.as_str(),
                    "attention"
                        | "completed"
                        | "costSpike"
                        | "question"
                        | "failure"
                        | "review"
                        | "schedule"
                        | "chief"
                )
        })
    {
        return Err(invalid());
    }
    Ok(payload)
}
fn revision(meta: &fs::Metadata, bytes: &[u8]) -> Revision {
    Revision {
        digest: Sha256::digest(bytes).to_vec(),
        modified: meta.modified().ok(),
        #[cfg(unix)]
        identity: {
            use std::os::unix::fs::MetadataExt;
            (meta.dev(), meta.ino())
        },
    }
}
fn open_regular(path: &Path) -> io::Result<fs::File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path)?;
    if !file.metadata()?.is_file() {
        return Err(invalid());
    }
    Ok(file)
}
fn read_current(path: &Path) -> io::Result<Option<(Vec<u8>, Revision)>> {
    let before = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            // 跨平台（Windows 将父路径非目录也映射为 NotFound）：
            // 若父路径存在且不是目录，必须返回错误，不可将中间路径失效视作目标文件不存在。
            if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                if let Ok(meta) = fs::symlink_metadata(parent) {
                    if !meta.is_dir() {
                        return Err(invalid());
                    }
                }
            }
            return Ok(None);
        }
        Err(error) => return Err(error),
    };
    if !before.is_file() || before.file_type().is_symlink() || before.len() > MAX_BYTES {
        return Err(invalid());
    }
    let file = open_regular(path)?;
    let opened = file.metadata()?;
    if !opened.is_file() || revision(&before, &[]) != revision(&opened, &[]) {
        return Err(invalid());
    }
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
    let after = fs::symlink_metadata(path)?;
    if bytes.len() as u64 > MAX_BYTES
        || bytes.len() as u64 != after.len()
        || after.file_type().is_symlink()
        || revision(&opened, &[]) != revision(&after, &[])
    {
        return Err(invalid());
    }
    let stamp = revision(&after, &bytes);
    Ok(Some((bytes, stamp)))
}
fn revision_matches(path: &Path, expected: &Option<Revision>) -> bool {
    read_current(path)
        .is_ok_and(|current| current.map(|(_, revision)| revision).as_ref() == expected.as_ref())
}

#[cfg(test)]
#[path = "notify_ledger_tests.rs"]
mod tests;
#[cfg(test)]
#[path = "notify_ledger_privacy_tests.rs"]
mod privacy_tests;
