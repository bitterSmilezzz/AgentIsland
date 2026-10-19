//! Bounded persistence reads that can reuse a writer's locking handle.
//!
//! Win32 byte-range locks are mandatory: reading the locked range through a second
//! handle fails, even inside the locking process. Revision reads therefore go through
//! the handle that owns the lock, and freshness is judged from path metadata instead
//! of reopening the name. No sidecar file and no second handle are needed.

use sha2::{Digest, Sha256};
use std::fs::{self, File, Metadata, OpenOptions};
use std::io::{self, Read, Seek};
use std::path::Path;
use std::time::SystemTime;

#[derive(PartialEq, Eq)]
pub(crate) struct Revision {
    digest: Vec<u8>,
    modified: Option<SystemTime>,
    identity: Identity,
}

/// Which file object a name currently refers to. A replacement that preserves the
/// bytes and the modification time is still a different file, and a writer that
/// already recorded the old one must refuse to publish over it.
#[derive(PartialEq, Eq)]
struct Identity {
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
    #[cfg(windows)]
    created: u64,
    #[cfg(windows)]
    attributes: u32,
}

fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "invalid persistence file")
}

fn regular(meta: &Metadata) -> bool {
    if !meta.is_file() || meta.file_type().is_symlink() {
        return false;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // Reject every reparse point, including links the std file type does not
        // report as symlinks.
        if meta.file_attributes() & 0x400 != 0 {
            return false;
        }
    }
    true
}

fn identity(meta: &Metadata) -> Identity {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Identity {
            device: meta.dev(),
            inode: meta.ino(),
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // Creation time changes when a name is republished from a fresh staging file
        // and cannot be backdated through `FileTimes`, so it survives the
        // bytes-and-mtime-preserved replacement above.
        Identity {
            created: meta.creation_time(),
            attributes: meta.file_attributes(),
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = meta;
        Identity {}
    }
}

/// The name still refers to one unchanged regular file, judged without reading bytes.
fn unchanged(before: &Metadata, after: &Metadata) -> bool {
    regular(before)
        && regular(after)
        && before.len() == after.len()
        && before.modified().ok() == after.modified().ok()
        && identity(before) == identity(after)
}

pub(crate) fn open_regular(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // Inspect the named entry itself instead of following a link, and keep the
        // default sharing (read/write/delete) so atomic publication can replace the
        // name while this handle stays open.
        options.custom_flags(0x0020_0000); // FILE_FLAG_OPEN_REPARSE_POINT
    }
    let file = options.open(path)?;
    if !regular(&file.metadata()?) {
        return Err(invalid());
    }
    Ok(file)
}

/// Read the current name, or reuse the exact handle owning the exclusive lock.
/// Rewind on every invocation: validators may check a revision multiple times.
pub(crate) fn read_current(
    path: &Path,
    max_bytes: u64,
    locked: Option<&File>,
) -> io::Result<Option<(Vec<u8>, Revision)>> {
    let before = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            // Windows maps a non-directory parent to NotFound too. It is a read
            // failure, not permission to replace a state file that was never read.
            if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                if fs::symlink_metadata(parent).is_ok_and(|meta| !meta.is_dir()) {
                    return Err(invalid());
                }
            }
            return if locked.is_some() {
                Err(invalid())
            } else {
                Ok(None)
            };
        }
        Err(error) => return Err(error),
    };
    if !regular(&before) || before.len() > max_bytes {
        return Err(invalid());
    }
    // An absent name is published by atomic create; with a lock held the name must
    // still be there, and `open_regular` alone would follow a replacement.
    let opened;
    let mut file = match locked {
        Some(file) => file,
        None => {
            opened = open_regular(path)?;
            &opened
        }
    };
    let meta = file.metadata()?;
    if !unchanged(&before, &meta) {
        return Err(invalid());
    }
    file.rewind()?;
    let mut bytes = Vec::new();
    file.take(max_bytes + 1).read_to_end(&mut bytes)?;
    let after = fs::symlink_metadata(path)?;
    if bytes.len() as u64 > max_bytes || !unchanged(&meta, &after) {
        return Err(invalid());
    }
    let revision = Revision {
        digest: Sha256::digest(&bytes).to_vec(),
        modified: after.modified().ok(),
        identity: identity(&after),
    };
    Ok(Some((bytes, revision)))
}

pub(crate) fn revision_matches(
    path: &Path,
    expected: &Option<Revision>,
    max_bytes: u64,
    locked: Option<&File>,
) -> bool {
    read_current(path, max_bytes, locked)
        .is_ok_and(|current| current.map(|(_, revision)| revision).as_ref() == expected.as_ref())
}
