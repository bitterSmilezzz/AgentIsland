//! 测试沙箱目录：**只有这一处知道**「沙箱名必须带序号」。
//!
//! 为什么要收口：`SystemTime::as_nanos()` 在 macOS 上分辨率很粗，并行的两个用例
//! 完全可能取到同一个值；那样两个用例共用一个目录，先结束的那个 `Drop` 会把另一个的
//! 文件删掉——症状是「某次跑挂了几条、再跑全绿」，最难查也最容易被当成偶发放过。
//! 这件事在本仓已经咬过三次（selftest、provider/tokens、observability/settings/sqlite），
//! 所以命名规则只留一份，其余地方一律调它。

#![cfg(test)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// 进程内递增，保证同一纳秒内取的路径也不重名
static NEXT_SANDBOX: AtomicU64 = AtomicU64::new(0);

/// 造一个空的沙箱目录并返回路径。调用方负责清理（或用 [`Sandbox`]，它会自己清）。
pub fn temp_dir(tag: &str) -> PathBuf {
    let serial = NEXT_SANDBOX.fetch_add(1, Ordering::Relaxed);
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let path = std::env::temp_dir().join(format!(
        "agentisland-{tag}-{}-{stamp}-{serial}",
        std::process::id()
    ));
    let _ = std::fs::create_dir_all(&path);
    path
}

/// 用完自动删（`Drop` 在测试失败时也会跑）
pub struct Sandbox(PathBuf);

impl Sandbox {
    pub fn new(tag: &str) -> Self {
        Sandbox(temp_dir(tag))
    }
    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 这条用例直接守「并行取名不撞」：串行调用两次同名沙箱也必须不同——
    /// 这正是原先用 `as_nanos()` 裸取时会失败的地方（同纳秒就撞）。
    #[test]
    fn two_sandboxes_with_the_same_tag_never_collide() {
        let first = temp_dir("dup");
        let second = temp_dir("dup");
        assert_ne!(first, second, "同一标签也要给出不同目录");
        assert!(first.exists() && second.exists());
        std::fs::write(first.join("a"), "x").unwrap();
        std::fs::write(second.join("b"), "y").unwrap();
        // 删掉一个不该影响另一个——碰撞时正是这里出问题
        std::fs::remove_dir_all(&first).unwrap();
        assert!(second.join("b").exists(), "另一个沙箱被误删");
        let _ = std::fs::remove_dir_all(&second);
    }

    #[test]
    fn a_sandbox_cleans_up_after_itself() {
        let path = {
            let sandbox = Sandbox::new("cleanup");
            std::fs::write(sandbox.path().join("file"), "x").unwrap();
            sandbox.path().to_path_buf()
        };
        assert!(!path.exists(), "Drop 应当把沙箱删掉");
    }
}
