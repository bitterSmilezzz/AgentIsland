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
    volume: u32,
    #[cfg(windows)]
    index: u64,
}

fn file_identity(file: &File) -> io::Result<Identity> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let meta = file.metadata()?;
        Ok(Identity {
            device: meta.dev(),
            inode: meta.ino(),
        })
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        #[repr(C)]
        struct BY_HANDLE_FILE_INFORMATION {
            dwFileAttributes: u32,
            ftCreationTime: [u32; 2],
            ftLastAccessTime: [u32; 2],
            ftLastWriteTime: [u32; 2],
            dwVolumeSerialNumber: u32,
            nFileSizeHigh: u32,
            nFileSizeLow: u32,
            nNumberOfLinks: u32,
            nFileIndexHigh: u32,
            nFileIndexLow: u32,
        }
        extern "system" {
            fn GetFileInformationByHandle(
                hFile: *mut std::ffi::c_void,
                lpFileInformation: *mut BY_HANDLE_FILE_INFORMATION,
            ) -> i32;
        }
        let mut info = std::mem::MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::uninit();
        let ok = unsafe {
            GetFileInformationByHandle(file.as_raw_handle() as _, info.as_mut_ptr())
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        let info = unsafe { info.assume_init() };
        let index = ((info.nFileIndexHigh as u64) << 32) | (info.nFileIndexLow as u64);
        Ok(Identity {
            volume: info.dwVolumeSerialNumber,
            index,
        })
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = file;
        Ok(Identity {})
    }
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

/// The name still refers to one unchanged regular file, judged without reading bytes.
fn unchanged(before: &Metadata, after: &Metadata) -> bool {
    regular(before)
        && regular(after)
        && before.len() == after.len()
        && before.modified().ok() == after.modified().ok()
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
    let file_id = file_identity(file)?;
    if !unchanged(&before, &meta) || file_identity(&open_regular(path)?)? != file_id {
        return Err(invalid());
    }
    file.rewind()?;
    let mut bytes = Vec::new();
    file.take(max_bytes + 1).read_to_end(&mut bytes)?;
    let after = fs::symlink_metadata(path)?;
    if bytes.len() as u64 > max_bytes
        || !unchanged(&meta, &after)
        || file_identity(&open_regular(path)?)? != file_id
    {
        return Err(invalid());
    }
    let revision = Revision {
        digest: Sha256::digest(&bytes).to_vec(),
        modified: after.modified().ok(),
        identity: file_id,
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
