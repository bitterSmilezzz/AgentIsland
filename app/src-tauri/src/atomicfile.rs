//! 原子替换一个文件：写同目录暂存文件 → 调用方校验暂存内容 → rename 覆盖。
//!
//! 两个消费方共用：`provider.rs` 的档位配置切换、`settings.rs` 的设置落盘。
//! 抽出来之前它是 `provider.rs` 里的一个函数，而 settings 那边是裸 `fs::write`——
//! 于是「写一半崩掉」在档位上被挡住、在设置上却会留下截断的 JSON：
//! 下次启动 `load()` 解析失败、**静默回落出厂值**，用户的设置整份消失且没有任何提示。
//! 同一件事只能有一处实现，所以这层不该属于任一消费方。

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

/// 暂存文件的守卫：无论成功还是失败路径，离开作用域时都把自己的临时文件删掉。
struct StagedFile(PathBuf);

impl Drop for StagedFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// Replace `target` only after its staged bytes pass the caller's validator.
/// The target must already have a parent directory. A failed write or
/// validation leaves the old file byte-for-byte unchanged. The staged
/// file lives beside the target, so rename is on the same filesystem.
///
/// Symlinks are rejected: renaming over one would silently replace the link
/// instead of the file it points at. The staged file is created 0600 (unix),
/// which is the stricter of what either consumer needs.
pub(crate) fn atomic_replace_validated<F>(
    target: &Path,
    bytes: &[u8],
    validate: F,
) -> io::Result<()>
where
    F: FnOnce(&Path) -> io::Result<()>,
{
    atomic_write_validated(target, bytes, validate, true)
}

/// Publish a complete validated file without replacing any existing directory entry.
/// A hard link from a synced same-directory staging file atomically claims the name;
/// even a concurrent creator or dangling symlink is never overwritten. Filesystems
/// without hard-link support fail closed, before a consumer changes its config.
pub(crate) fn atomic_create_validated<F>(target: &Path, bytes: &[u8], validate: F) -> io::Result<()>
where
    F: FnOnce(&Path) -> io::Result<()>,
{
    atomic_write_validated(target, bytes, validate, false)
}

fn atomic_write_validated<F>(
    target: &Path,
    bytes: &[u8],
    validate: F,
    replace: bool,
) -> io::Result<()>
where
    F: FnOnce(&Path) -> io::Result<()>,
{
    let parent = target
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "target needs a parent"))?;
    let name = target
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "target needs a filename"))?;
    if !replace && fs::symlink_metadata(target).is_ok() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "target already exists",
        ));
    }
    if replace
        && fs::symlink_metadata(target).is_ok_and(|m| m.file_type().is_symlink() || !m.is_file())
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "target must be a regular file",
        ));
    }

    let mut staged = None;
    for _ in 0..32 {
        let serial = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let path = parent.join(format!(
            ".{}.agentisland-{}-{serial}.tmp",
            name.to_string_lossy(),
            std::process::id()
        ));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&path) {
            Ok(file) => {
                staged = Some((StagedFile(path), file));
                break;
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    let (guard, mut file): (StagedFile, File) = staged
        .ok_or_else(|| io::Error::new(io::ErrorKind::AlreadyExists, "staging name exhausted"))?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    validate(&guard.0)?;
    if replace {
        fs::rename(&guard.0, target)?;
    } else {
        fs::hard_link(&guard.0, target)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct Sandbox(PathBuf);

    impl Sandbox {
        fn new() -> Self {
            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let serial = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "agentisland-atomicfile-{}-{stamp}-{serial}",
                std::process::id()
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn only_target_remains(root: &Path, target_name: &str) {
        let entries: Vec<_> = fs::read_dir(root)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(
            entries,
            vec![target_name],
            "staging file leaked: {entries:?}"
        );
    }

    #[test]
    fn new_file_publication_is_complete_private_and_never_replaces_an_existing_name() {
        let sandbox = Sandbox::new();
        let target = sandbox.0.join("config-1.toml");
        atomic_create_validated(&target, b"model = 'first'\n", |stage| {
            assert!(!target.exists());
            assert_eq!(fs::read(stage)?, b"model = 'first'\n");
            Ok(())
        })
        .unwrap();
        let error = atomic_create_validated(&target, b"second", |_| Ok(())).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read(&target).unwrap(), b"model = 'first'\n");
        only_target_remains(&sandbox.0, "config-1.toml");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(target).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn a_concurrent_creator_after_validation_is_never_overwritten() {
        let sandbox = Sandbox::new();
        let target = sandbox.0.join("config-2.toml");
        let error = atomic_create_validated(&target, b"ours", |stage| {
            assert_eq!(fs::read(stage)?, b"ours");
            fs::write(&target, b"concurrent winner")?;
            Ok(())
        })
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read(&target).unwrap(), b"concurrent winner");
        only_target_remains(&sandbox.0, "config-2.toml");
    }

    #[test]
    fn rejected_new_backup_is_never_published_and_leaves_no_staging_file() {
        let sandbox = Sandbox::new();
        let target = sandbox.0.join("config-3.toml");
        let error = atomic_create_validated(&target, b"incomplete", |_| {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "fixture rejection",
            ))
        })
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(!target.exists());
        assert_eq!(fs::read_dir(&sandbox.0).unwrap().count(), 0);
    }

    #[test]
    fn successful_replacement_validates_staged_bytes_then_replaces_atomically() {
        let sandbox = Sandbox::new();
        let target = sandbox.0.join("fixture.config.toml");
        fs::write(&target, "model = 'before'\n").unwrap();
        atomic_replace_validated(&target, b"model = 'after'\n", |stage| {
            assert_eq!(fs::read_to_string(&target)?, "model = 'before'\n");
            assert_eq!(fs::read_to_string(stage)?, "model = 'after'\n");
            Ok(())
        })
        .unwrap();
        assert_eq!(fs::read_to_string(&target).unwrap(), "model = 'after'\n");
        only_target_remains(&sandbox.0, "fixture.config.toml");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&target).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn invalid_staged_config_preserves_original_and_cleans_up() {
        let sandbox = Sandbox::new();
        let target = sandbox.0.join("fixture.config.toml");
        fs::write(&target, "model = 'before'\n").unwrap();
        let result = atomic_replace_validated(&target, b"not valid TOML = [", |stage| {
            assert_eq!(fs::read_to_string(stage)?, "not valid TOML = [");
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "synthetic parse rejection",
            ))
        });
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidData);
        assert_eq!(fs::read_to_string(&target).unwrap(), "model = 'before'\n");
        only_target_remains(&sandbox.0, "fixture.config.toml");
    }

    #[test]
    fn invalid_new_config_does_not_create_the_target() {
        let sandbox = Sandbox::new();
        let target = sandbox.0.join("fixture.config.toml");
        let result = atomic_replace_validated(&target, b"broken", |_| {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "synthetic parse rejection",
            ))
        });
        assert!(result.is_err());
        assert!(!target.exists());
        assert_eq!(fs::read_dir(&sandbox.0).unwrap().count(), 0);
    }

    #[test]
    fn symlink_target_is_rejected_without_touching_its_destination() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let sandbox = Sandbox::new();
            let destination = sandbox.0.join("real.config.toml");
            fs::write(&destination, "keep").unwrap();
            let link = sandbox.0.join("fixture.config.toml");
            symlink(&destination, &link).unwrap();
            let result = atomic_replace_validated(&link, b"replace", |_| Ok(()));
            assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidInput);
            assert_eq!(fs::read_to_string(destination).unwrap(), "keep");
        }
    }
}
