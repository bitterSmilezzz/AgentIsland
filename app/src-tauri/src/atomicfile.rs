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

    /// 断电残留：上一次崩溃留下的暂存文件既不能被当作新内容顶上去，
    /// 也不能让正常写入失败。写入路径要能「无视别人的残骸继续干活」。
    #[test]
    fn a_leftover_staging_file_from_a_crash_never_reaches_the_target() {
        let sandbox = Sandbox::new();
        let target = sandbox.0.join("fixture.config.toml");
        fs::write(&target, "model = 'before'\n").unwrap();
        // 模拟崩溃残留：名字符合本模块的暂存规则，内容是「半截垃圾」
        let residue = sandbox
            .0
            .join(format!(".fixture.config.toml.agentisland-{}-0.tmp", std::process::id()));
        fs::write(&residue, "model = 'half-written-by-a-dead-process'\n").unwrap();

        atomic_replace_validated(&target, b"model = 'after'\n", |stage| {
            Ok(fs::read_to_string(stage).map(|_| ())?)
        })
        .expect("残留暂存文件不该让正常写入失败");

        assert_eq!(fs::read_to_string(&target).unwrap(), "model = 'after'\n");
        assert_ne!(
            fs::read_to_string(&residue).unwrap(),
            "model = 'after'\n",
            "残留文件被当成了发布目标"
        );
    }

    /// 磁盘满 / 有界写入失败：`write_all` 中途 Err ⇒ **原件逐字节不动**、
    /// 暂存文件被守卫清掉、错误显式返回。这是「断网/睡眠/崩溃/升级不丢任务与
    /// 账目」在写入侧的兜底：设置与档位的落盘都走这一层，失败永远到不了 rename。
    ///
    /// **以 RLIMIT_FSIZE 注入同一条失败路径**（`write_all` 返回 EFBIG）——不是
    /// ENOSPC 本身：真实磁盘满需要挂载小容量镜像，而 `hdiutil` 在本机被系统拒绝
    /// （Operation not permitted，TCC 级环境限制，如实登记）。两条错误码走的是
    /// **完全相同**的处理路径：错误传播、暂存清理、原件保护，都不看 errno。
    ///
    /// RLIMIT 是**进程级**限制，会波及并行线程的其它写文件测试——所以本测试
    /// 标 `#[ignore]`，由 `ops01.diskfull` 门禁以 `--ignored --test-threads=1`
    /// 单独运行，测试退出前恢复限额与信号处理（守卫保证，失败路径也不例外）。
    #[test]
    #[cfg(unix)]
    #[ignore = "RLIMIT_FSIZE 是进程级限制，必须独占运行；由 ops01.diskfull 门禁显式运行"]
    fn a_write_that_hits_the_size_limit_preserves_the_original_and_leaves_no_residue() {
        struct ProcessGuard {
            saved_limit: libc::rlimit,
            saved_sigxfsz: usize,
        }
        impl Drop for ProcessGuard {
            fn drop(&mut self) {
                unsafe {
                    libc::setrlimit(libc::RLIMIT_FSIZE, &self.saved_limit);
                    libc::signal(libc::SIGXFSZ, self.saved_sigxfsz as libc::sighandler_t);
                }
            }
        }
        let sandbox = Sandbox::new();
        let target = sandbox.0.join("settings.json");
        let original = vec![b'a'; 4096];

        // 写入限额前的正常基线：先落一份原件。
        atomic_replace_validated(&target, &original, |stage| {
            Ok(fs::metadata(stage).map(|_| ())?)
        })
        .expect("限额内写入应当成功");

        let mut saved_limit = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        let rc = unsafe { libc::getrlimit(libc::RLIMIT_FSIZE, &mut saved_limit) };
        assert_eq!(rc, 0, "getrlimit 应当可用");
        // 只降软限：限额=原件大小+少量余量；硬限不动（降硬限不可逆）。
        let mut limited = saved_limit;
        limited.rlim_cur = (original.len() as u64 + 4096).min(limited.rlim_max);
        let saved_sigxfsz = unsafe { libc::signal(libc::SIGXFSZ, libc::SIG_IGN) };
        assert_ne!(saved_sigxfsz, libc::SIG_ERR, "signal 不应失败");
        let _guard = ProcessGuard {
            saved_limit,
            saved_sigxfsz: saved_sigxfsz as usize,
        };
        let rc = unsafe { libc::setrlimit(libc::RLIMIT_FSIZE, &limited) };
        assert_eq!(rc, 0, "setrlimit 应当可用");

        // 超限写入（1MB ≫ 8KB 限额）：write_all 必须失败，且失败是显式的。
        let too_big = vec![b'b'; 1024 * 1024];
        let result = atomic_replace_validated(&target, &too_big, |stage| {
            Ok(fs::metadata(stage).map(|_| ())?)
        });
        let error = result.expect_err("超限写入必须失败，而不是截断成功");
        assert_ne!(
            error.kind(),
            io::ErrorKind::Other,
            "失败要有真实原因，不是包装出来的空错误"
        );

        // 原件逐字节不动；暂存无残留（守卫在错误路径上也要清理）。
        assert_eq!(
            fs::read(&target).unwrap(),
            original,
            "失败的写入不得碰原件"
        );
        only_target_remains(&sandbox.0, "settings.json");

        // 限额内的写入照常成功：证明上面的失败归因于限额本身，不是环境坏了。
        let small = vec![b'c'; 1024];
        atomic_replace_validated(&target, &small, |stage| {
            Ok(fs::metadata(stage).map(|_| ())?)
        })
        .expect("限额内写入应当成功");
        assert_eq!(fs::read(&target).unwrap(), small);
    }

}
