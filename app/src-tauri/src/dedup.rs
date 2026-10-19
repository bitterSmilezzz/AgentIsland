//! Bounded notification memory. Disk persistence is opt-in and stores only SHA-256 identities.

use sha2::{Digest, Sha256};
use std::collections::VecDeque;
use std::fs::{self, OpenOptions};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

const CAP: usize = 512;
const MAX_BYTES: u64 = 64 * 1024;
// Version 1 stored unverified raw strings; never reinterpret those as hashed identities.
const SCHEMA_VERSION: u32 = 2;
const READ_ERROR: &str = "dedup state unreadable; persistence disabled";
const STALE_ERROR: &str = "dedup state changed externally; persistence disabled";
const WRITE_ERROR: &str = "dedup state could not be saved";

#[derive(Default)]
pub(crate) struct Store {
    events: VecDeque<String>,
    disk: Option<Disk>,
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

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Payload {
    schema_version: u32,
    events: VecDeque<String>,
}

impl Store {
    /// Read an explicit path without creating anything. Failed reads preserve the original
    /// and permanently disable writes for this instance; monitoring can still use memory.
    pub(crate) fn open_at(path: impl AsRef<Path>) -> Self {
        let path = path.as_ref().to_path_buf();
        let loaded = read_current(&path).and_then(|current| match current {
            None => Ok((VecDeque::new(), None)),
            Some((bytes, revision)) => Ok((decode(&bytes)?.events, Some(revision))),
        });
        let (events, revision, blocked) = match loaded {
            Ok((events, revision)) => (events, revision, false),
            Err(_) => {
                // GUI 由 LaunchServices 启动，stderr 不进 agentisland-tauri.log；
                // 持久化被永久禁用这个退化必须走日志才在取证链上可见。
                crate::log_line(READ_ERROR);
                (VecDeque::new(), None, true)
            }
        };
        Self {
            events,
            disk: Some(Disk {
                path,
                revision,
                blocked,
            }),
        }
    }

    pub(crate) fn attention_seen(&self, agent: &str, raw: &str) -> bool {
        self.seen(agent, "attention", raw)
    }

    pub(crate) fn note_attention(&mut self, agent: &str, raw: &str) {
        self.note(agent, "attention", raw);
    }

    pub(crate) fn completion_seen(&self, agent: &str, raw: &str) -> bool {
        self.seen(agent, "completed", raw)
    }

    pub(crate) fn note_completion(&mut self, agent: &str, raw: &str) {
        self.note(agent, "completed", raw);
    }

    fn seen(&self, agent: &str, kind: &str, raw: &str) -> bool {
        self.events.contains(&identity(agent, kind, raw))
    }

    fn note(&mut self, agent: &str, kind: &str, raw: &str) {
        let hash = identity(agent, kind, raw);
        if let Some(index) = self.events.iter().position(|known| *known == hash) {
            self.events.remove(index);
        }
        self.events.push_back(hash);
        while self.events.len() > CAP {
            self.events.pop_front();
        }
    }

    /// Memory-only stores perform no IO. IO errors never invalidate the in-memory history.
    pub(crate) fn persist(&mut self) -> Result<(), &'static str> {
        let Some(disk) = self.disk.as_mut() else {
            return Ok(());
        };
        if disk.blocked {
            return Err(READ_ERROR);
        }
        // Compare both content and file identity, including an absent-to-present change.
        if !revision_matches(&disk.path, &disk.revision) {
            disk.blocked = true;
            return Err(STALE_ERROR);
        }
        // Cooperating writers lock the existing inode through publication. An absent name
        // is claimed exclusively by atomic_create_validated instead.
        #[cfg(unix)]
        let _writer_lock = if disk.revision.is_some() {
            let file = open_regular(&disk.path).map_err(|_| WRITE_ERROR)?;
            file.try_lock().map_err(|_| WRITE_ERROR)?;
            Some(file)
        } else {
            None
        };
        if !revision_matches(&disk.path, &disk.revision) {
            disk.blocked = true;
            return Err(STALE_ERROR);
        }
        let bytes = serde_json::to_vec(&Payload {
            schema_version: SCHEMA_VERSION,
            events: self.events.clone(),
        })
        .map_err(|_| WRITE_ERROR)?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err(WRITE_ERROR);
        }
        let parent = disk
            .path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .ok_or(WRITE_ERROR)?;
        fs::create_dir_all(parent).map_err(|_| WRITE_ERROR)?;
        let validate = |staged: &Path| {
            let (staged_bytes, _) = read_current(staged)?.ok_or_else(invalid)?;
            decode(&staged_bytes)?;
            if !revision_matches(&disk.path, &disk.revision) {
                return Err(invalid());
            }
            Ok(())
        };
        let result = if disk.revision.is_none() {
            crate::atomicfile::atomic_create_validated(&disk.path, &bytes, validate)
        } else {
            crate::atomicfile::atomic_replace_validated(&disk.path, &bytes, validate)
        };
        if result.is_err() {
            if !revision_matches(&disk.path, &disk.revision) {
                disk.blocked = true;
                return Err(STALE_ERROR);
            }
            return Err(WRITE_ERROR);
        }
        match read_current(&disk.path) {
            Ok(Some((published, revision))) if published == bytes => {
                disk.revision = Some(revision);
                Ok(())
            }
            _ => {
                disk.blocked = true;
                Err(STALE_ERROR)
            }
        }
    }
}

fn identity(agent: &str, kind: &str, raw: &str) -> String {
    let mut digest = Sha256::new();
    for part in [agent, kind, raw] {
        digest.update((part.len() as u64).to_be_bytes());
        digest.update(part.as_bytes());
    }
    format!("{:x}", digest.finalize())
}

fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "invalid dedup state")
}

fn decode(bytes: &[u8]) -> io::Result<Payload> {
    let payload: Payload = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    if payload.schema_version != SCHEMA_VERSION
        || payload.events.len() > CAP
        || payload.events.iter().any(|hash| {
            hash.len() != 64
                || !hash
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
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

/// Reject links/non-files and read at most MAX_BYTES+1, including changes during a read.
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
#[path = "dedup_tests.rs"]
mod tests;
