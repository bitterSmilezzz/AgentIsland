//! 产出**内容主体**按版本的存取判据（TASK-05 第三片）。
//!
//! 需求原文：自有执行产物版本留存、任意版本比较/恢复。
//! 前两片已做：产出**元数据**链（版本演进/比较/恢复，`artifact_version.rs`）与
//! 验证清单；本片补**内容主体**——元数据说「有这个版本」，内容存储说
//! 「这一版的具体字节在哪、有多大、是不是私有」。
//!
//! 判据（纯函数/内存实现，不碰文件系统）：
//!
//! 1. **内容与版本一一对应**：内容按 (逻辑 id, revision) 存放；读不到就**如实说
//!    没有**（`ContentMissing`），绝不返回别的版本的内容——那会让「对比两版」
//!    悄悄变成「同一版比两遍」。
//! 2. **内容不可变**：同一 (逻辑 id, revision) 二次写入**拒绝**
//!    （`ContentImmutable`）——内容与元数据不一致就是账目问题。
//! 3. **大小上限有界**：单版与总量各有上限，超限**拒绝**而不是截断
//!    （截断的内容看起来是完整的，比没有更危险）。
//! 4. **私密内容不进存储**：涉及凭据/私密输入的内容**当场拒收**，不留副本。
//! 5. **内容寻址**：存储按内容摘要（sha256）标识，便于核对「元数据说的那一份」
//!    与「实际存着的那一份」是同一份字节。

use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

/// 单版内容的上限（4 MiB，与技能包正文的量级一致）。
pub const MAX_CONTENT_BYTES: usize = 4 * 1024 * 1024;
/// 全部内容总量上限（64 MiB）。
pub const MAX_TOTAL_BYTES: usize = 64 * 1024 * 1024;

/// 内容存取拒绝原因。固定码。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreRefusal {
    /// 该版本没有内容。
    ContentMissing,
    /// 同一版本二次写入（内容不可变）。
    ContentImmutable,
    /// 单版超过上限。
    ContentTooLarge,
    /// 总量超过上限。
    StoreFull,
    /// 内容涉及凭据/私密输入，当场拒收。
    ContentRejected,
    /// 已发布内容缺失、长度不符或摘要校验失败。
    ContentCorrupt,
    /// 文件系统不可用或目标不是受支持的普通文件。
    StorageUnavailable,
}

impl StoreRefusal {
    pub fn error_details(self) -> (&'static str, &'static str) {
        match self {
            Self::ContentMissing => ("content_missing", "本地产出正文不存在"),
            Self::ContentImmutable => ("conflict", "本地产出正文版本不可覆盖"),
            Self::ContentTooLarge => ("capacity", "单份产出正文超过允许上限，未保存"),
            Self::StoreFull => ("capacity", "本地产出正文达到容量上限，未保存"),
            Self::ContentRejected => ("invalid_input", "正文包含已识别凭据，未保存"),
            Self::ContentCorrupt => ("content_corrupt", "本地产出正文完整性校验失败"),
            Self::StorageUnavailable => ("io_error", "本地产出正文存储不可用"),
        }
    }
}

/// 内容寻址文件的公开索引值。任务 JSON 只保存此摘要与长度，不保存正文。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredBlob {
    pub sha256: String,
    pub bytes: usize,
}

/// 按 SHA-256 摘要原子发布的本地正文文件。
///
/// 逻辑产物与 revision 的绑定由消费方的原子元数据文件保存；本模块只负责
/// 不可变 blob。若元数据提交失败，剩下的孤立 blob 不会被读取，且仍计入磁盘上限。
pub struct FileBlobStore {
    root: PathBuf,
}

impl FileBlobStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn valid_root(metadata: &std::fs::Metadata) -> bool {
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return false;
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            // Reject directory junctions and all other reparse points too.
            if metadata.file_attributes() & 0x400 != 0 {
                return false;
            }
        }
        true
    }

    fn validate_existing_root(&self) -> Result<(), StoreRefusal> {
        let metadata = std::fs::symlink_metadata(&self.root).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                StoreRefusal::ContentMissing
            } else {
                StoreRefusal::StorageUnavailable
            }
        })?;
        if !Self::valid_root(&metadata) {
            return Err(StoreRefusal::StorageUnavailable);
        }
        Ok(())
    }

    fn verify_blob(
        bytes: &[u8],
        expected_digest: &str,
        expected_bytes: usize,
        expected_content: Option<&[u8]>,
    ) -> Result<(), StoreRefusal> {
        if bytes.len() != expected_bytes
            || format!("{:x}", Sha256::digest(bytes)) != expected_digest
            || expected_content.is_some_and(|expected| expected != bytes)
        {
            return Err(StoreRefusal::ContentCorrupt);
        }
        Ok(())
    }

    fn ensure_root(&self) -> Result<(), StoreRefusal> {
        match std::fs::symlink_metadata(&self.root) {
            Ok(metadata) => {
                if !Self::valid_root(&metadata) {
                    return Err(StoreRefusal::StorageUnavailable);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::DirBuilderExt;
                    let mut builder = std::fs::DirBuilder::new();
                    builder.mode(0o700);
                    builder
                        .create(&self.root)
                        .map_err(|_| StoreRefusal::StorageUnavailable)?;
                }
                #[cfg(not(unix))]
                {
                    std::fs::create_dir(&self.root)
                        .map_err(|_| StoreRefusal::StorageUnavailable)?;
                }
            }
            Err(_) => return Err(StoreRefusal::StorageUnavailable),
        }
        Ok(())
    }

    fn path(&self, digest: &str) -> PathBuf {
        self.root.join(format!("{digest}.blob"))
    }

    fn physical_bytes(&self) -> Result<usize, StoreRefusal> {
        let mut total = 0usize;
        for entry in std::fs::read_dir(&self.root).map_err(|_| StoreRefusal::StorageUnavailable)? {
            let entry = entry.map_err(|_| StoreRefusal::StorageUnavailable)?;
            let metadata = std::fs::symlink_metadata(entry.path())
                .map_err(|_| StoreRefusal::StorageUnavailable)?;
            if !metadata.is_file() || metadata.file_type().is_symlink() {
                return Err(StoreRefusal::StorageUnavailable);
            }
            total = total
                .checked_add(
                    metadata
                        .len()
                        .try_into()
                        .map_err(|_| StoreRefusal::StoreFull)?,
                )
                .ok_or(StoreRefusal::StoreFull)?;
        }
        Ok(total)
    }

    fn read_file(path: &Path) -> Result<Vec<u8>, StoreRefusal> {
        let file = crate::persistence_file::open_regular(path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                StoreRefusal::ContentMissing
            } else {
                StoreRefusal::StorageUnavailable
            }
        })?;
        let length: usize = file
            .metadata()
            .map_err(|_| StoreRefusal::StorageUnavailable)?
            .len()
            .try_into()
            .map_err(|_| StoreRefusal::ContentCorrupt)?;
        if length > MAX_CONTENT_BYTES {
            return Err(StoreRefusal::ContentCorrupt);
        }
        let mut bytes = Vec::with_capacity(length);
        file.take((MAX_CONTENT_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|_| StoreRefusal::StorageUnavailable)?;
        if bytes.len() != length {
            return Err(StoreRefusal::ContentCorrupt);
        }
        Ok(bytes)
    }

    /// 发布正文。`referenced_bytes` 来自消费方当前元数据；物理扫描同时计入
    /// 未被引用的孤立文件，避免原子提交中断后绕过总容量上限。
    pub fn put(
        &self,
        bytes: &[u8],
        is_secret: bool,
        referenced_bytes: usize,
    ) -> Result<StoredBlob, StoreRefusal> {
        if is_secret {
            return Err(StoreRefusal::ContentRejected);
        }
        if bytes.len() > MAX_CONTENT_BYTES {
            return Err(StoreRefusal::ContentTooLarge);
        }
        if referenced_bytes
            .checked_add(bytes.len())
            .is_none_or(|total| total > MAX_TOTAL_BYTES)
        {
            return Err(StoreRefusal::StoreFull);
        }
        self.ensure_root()?;

        let digest = format!("{:x}", Sha256::digest(bytes));
        let target = self.path(&digest);
        let physical_bytes = self.physical_bytes()?;
        match std::fs::symlink_metadata(&target) {
            Ok(_) => {
                let existing = Self::read_file(&target)?;
                Self::verify_blob(&existing, &digest, bytes.len(), Some(bytes))?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if physical_bytes
                    .checked_add(bytes.len())
                    .is_none_or(|total| total > MAX_TOTAL_BYTES)
                {
                    return Err(StoreRefusal::StoreFull);
                }
                let publish =
                    crate::atomicfile::atomic_create_validated(&target, bytes, |staged| {
                        let stored = Self::read_file(staged)
                            .map_err(|_| std::io::Error::other("invalid content blob"))?;
                        Self::verify_blob(&stored, &digest, bytes.len(), Some(bytes))
                            .map_err(|_| std::io::Error::other("invalid content blob"))?;
                        Ok(())
                    });
                if let Err(error) = publish {
                    if error.kind() != std::io::ErrorKind::AlreadyExists {
                        return Err(StoreRefusal::StorageUnavailable);
                    }
                    let existing = Self::read_file(&target)?;
                    Self::verify_blob(&existing, &digest, bytes.len(), Some(bytes))?;
                }
            }
            Err(_) => return Err(StoreRefusal::StorageUnavailable),
        }

        Ok(StoredBlob {
            sha256: digest,
            bytes: bytes.len(),
        })
    }

    /// 读取并核对任务元数据中的摘要与长度。
    pub fn read(&self, digest: &str, expected_bytes: usize) -> Result<Vec<u8>, StoreRefusal> {
        if digest.len() != 64
            || !digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            || expected_bytes > MAX_CONTENT_BYTES
        {
            return Err(StoreRefusal::ContentCorrupt);
        }
        self.validate_existing_root()?;
        let bytes = Self::read_file(&self.path(digest))?;
        self.validate_existing_root()?;
        Self::verify_blob(&bytes, digest, expected_bytes, None)?;
        Ok(bytes)
    }
}

/// 一份存入的内容（只保留摘要与长度，不保留原文——调用方自己拿字节）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredContent {
    pub logical_id: String,
    pub revision: u64,
    pub sha256: String,
    pub bytes: usize,
}

/// 内容存储（内存实现；判据可复用到真实文件后端）。
#[derive(Debug, Default)]
pub struct ContentStore {
    entries: HashMap<(String, u64), (String, Vec<u8>)>,
    total_bytes: usize,
}

impl ContentStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// 存入一版内容。
    ///
    /// `is_secret`：调用方已识别的私密标记（或通过 `known_private` 检查）。
    pub fn put(
        &mut self,
        logical_id: &str,
        revision: u64,
        bytes: Vec<u8>,
        is_secret: bool,
    ) -> Result<StoredContent, StoreRefusal> {
        if is_secret {
            return Err(StoreRefusal::ContentRejected);
        }
        if bytes.len() > MAX_CONTENT_BYTES {
            return Err(StoreRefusal::ContentTooLarge);
        }
        let key = (logical_id.to_string(), revision);
        if self.entries.contains_key(&key) {
            return Err(StoreRefusal::ContentImmutable);
        }
        if self.total_bytes + bytes.len() > MAX_TOTAL_BYTES {
            return Err(StoreRefusal::StoreFull);
        }
        let digest = format!("{:x}", Sha256::digest(&bytes));
        self.total_bytes += bytes.len();
        let length = bytes.len();
        self.entries.insert(key, (digest.clone(), bytes));
        Ok(StoredContent {
            logical_id: logical_id.to_string(),
            revision,
            sha256: digest,
            bytes: length,
        })
    }

    /// 读取某版内容的字节。读不到**如实说没有**，不返回别的版本。
    pub fn read(&self, logical_id: &str, revision: u64) -> Result<&[u8], StoreRefusal> {
        self.entries
            .get(&(logical_id.to_string(), revision))
            .map(|(_, bytes)| bytes.as_slice())
            .ok_or(StoreRefusal::ContentMissing)
    }

    /// 读某版内容的摘要（用于核对「元数据说的那一份」与「实际存着的」）。
    pub fn digest_of(&self, logical_id: &str, revision: u64) -> Result<&str, StoreRefusal> {
        self.entries
            .get(&(logical_id.to_string(), revision))
            .map(|(digest, _)| digest.as_str())
            .ok_or(StoreRefusal::ContentMissing)
    }

    /// 核对元数据记录的摘要与实际存储是否一致（内容被换过 ⇒ 报 `ContentMissing`
    /// 之外的语义：内容与元数据对不上，**不能**当作「没有」来糊弄）。
    pub fn verify_digest(
        &self,
        logical_id: &str,
        revision: u64,
        expected_sha256: &str,
    ) -> Result<bool, StoreRefusal> {
        let actual = self.digest_of(logical_id, revision)?;
        Ok(actual == expected_sha256)
    }

    /// 当前存储的字节总量。
    pub fn total_bytes(&self) -> usize {
        self.total_bytes
    }

    /// 存储的版本条数。
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_is_stored_and_read_back_by_version() {
        let mut store = ContentStore::new();
        let stored = store
            .put("art-1", 1, "第一版内容".as_bytes().to_vec(), false)
            .unwrap();
        assert_eq!(stored.revision, 1);
        assert_eq!(stored.bytes, "第一版内容".len());
        assert_eq!(store.read("art-1", 1).unwrap(), "第一版内容".as_bytes());

        // 第二版独立存放
        store
            .put("art-1", 2, "第二版内容".as_bytes().to_vec(), false)
            .unwrap();
        assert_eq!(store.read("art-1", 2).unwrap(), "第二版内容".as_bytes());
        assert_eq!(store.read("art-1", 1).unwrap(), "第一版内容".as_bytes());
    }

    #[test]
    fn a_missing_version_is_reported_as_missing_not_substituted() {
        let mut store = ContentStore::new();
        store
            .put("art-1", 1, "第一版".as_bytes().to_vec(), false)
            .unwrap();
        // 读第二版（不存在）⇒ 明确说没有，**不是**返回第一版
        assert_eq!(store.read("art-1", 2), Err(StoreRefusal::ContentMissing));
        assert_eq!(
            store.digest_of("art-1", 2),
            Err(StoreRefusal::ContentMissing)
        );
        // 读另一个逻辑 id 的同一版本号 ⇒ 同样没有
        assert_eq!(store.read("other", 1), Err(StoreRefusal::ContentMissing));
    }

    #[test]
    fn the_same_version_cannot_be_rewritten() {
        let mut store = ContentStore::new();
        store
            .put("art-1", 1, "原内容".as_bytes().to_vec(), false)
            .unwrap();
        // 同一版本二次写入 ⇒ 拒绝（内容与元数据不一致就是账目问题）
        assert_eq!(
            store.put("art-1", 1, "改过的内容".as_bytes().to_vec(), false),
            Err(StoreRefusal::ContentImmutable)
        );
        // 原内容未被改动
        assert_eq!(store.read("art-1", 1).unwrap(), "原内容".as_bytes());
    }

    #[test]
    fn oversized_and_secret_content_is_refused_without_being_stored() {
        let mut store = ContentStore::new();
        // 超单版上限
        assert_eq!(
            store.put("art-1", 1, vec![b'x'; MAX_CONTENT_BYTES + 1], false),
            Err(StoreRefusal::ContentTooLarge)
        );
        // 私密内容当场拒收（不留副本）
        assert_eq!(
            store.put("art-1", 2, "sk-live-secret".as_bytes().to_vec(), true),
            Err(StoreRefusal::ContentRejected)
        );
        assert!(store.is_empty(), "被拒的内容不得留下任何副本");
    }

    #[test]
    fn the_digest_matches_the_metadata_and_detects_a_swap() {
        let mut store = ContentStore::new();
        let stored = store
            .put("art-1", 1, "内容".as_bytes().to_vec(), false)
            .unwrap();
        // 摘要与元数据记录的一致
        assert!(store.verify_digest("art-1", 1, &stored.sha256).unwrap());
        // 元数据写的摘要对不上 ⇒ 报 false（内容被换过，不能当作「没有」糊弄）
        assert!(!store.verify_digest("art-1", 1, &"0".repeat(64)).unwrap());
        // 读不存在的版本 ⇒ 明确说没有
        assert_eq!(
            store.verify_digest("art-1", 9, &stored.sha256),
            Err(StoreRefusal::ContentMissing)
        );
    }

    #[test]
    fn file_blobs_survive_a_fresh_store_and_publish_as_private_content_addressed_files() {
        let sandbox = crate::testutil::Sandbox::new("artifact-file-store");
        let root = sandbox.path().join("blobs");
        let first = FileBlobStore::new(&root)
            .put("本地产出".as_bytes(), false, 0)
            .unwrap();
        assert_eq!(first.bytes, "本地产出".len());
        assert_eq!(
            std::fs::read_dir(&root).unwrap().count(),
            1,
            "相同正文只发布一个摘要文件"
        );
        assert_eq!(
            FileBlobStore::new(&root)
                .read(&first.sha256, first.bytes)
                .unwrap(),
            "本地产出".as_bytes()
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&root).unwrap().permissions().mode() & 0o077,
                0,
                "正文目录创建时就不能对组或其他用户开放"
            );
            assert_eq!(
                std::fs::metadata(root.join(format!("{}.blob", first.sha256)))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn file_blob_refuses_secret_capacity_symlink_missing_and_digest_mismatch() {
        let sandbox = crate::testutil::Sandbox::new("artifact-file-store-guards");
        let root = sandbox.path().join("blobs");
        let store = FileBlobStore::new(&root);
        assert_eq!(
            store.put(b"secret", true, 0),
            Err(StoreRefusal::ContentRejected)
        );
        assert!(!root.exists(), "拒绝私密内容前不得创建存储目录");
        assert_eq!(
            store.put(b"x", false, MAX_TOTAL_BYTES),
            Err(StoreRefusal::StoreFull)
        );
        assert!(!root.exists(), "容量拒绝不得留下文件");

        let stored = store.put(b"expected", false, 0).unwrap();
        assert_eq!(
            store.read(&stored.sha256, stored.bytes + 1),
            Err(StoreRefusal::ContentCorrupt)
        );
        std::fs::write(root.join(format!("{}.blob", stored.sha256)), b"swapped").unwrap();
        assert_eq!(
            store.read(&stored.sha256, stored.bytes),
            Err(StoreRefusal::ContentCorrupt)
        );
        assert_eq!(
            store.read(&"0".repeat(64), 1),
            Err(StoreRefusal::ContentMissing)
        );
    }

    #[cfg(unix)]
    #[test]
    fn file_blob_never_reads_through_a_symlink() {
        use std::os::unix::fs::symlink;
        let sandbox = crate::testutil::Sandbox::new("artifact-file-store-symlink");
        let root = sandbox.path().join("blobs");
        std::fs::create_dir(&root).unwrap();
        let body = b"outside";
        let digest = format!("{:x}", Sha256::digest(body));
        let outside = sandbox.path().join("outside");
        std::fs::write(&outside, body).unwrap();
        symlink(&outside, root.join(format!("{digest}.blob"))).unwrap();
        assert_eq!(
            FileBlobStore::new(root).read(&digest, body.len()),
            Err(StoreRefusal::StorageUnavailable)
        );
        assert_eq!(std::fs::read(outside).unwrap(), body);

        let outside_dir = sandbox.path().join("outside-directory");
        std::fs::create_dir(&outside_dir).unwrap();
        std::fs::write(outside_dir.join(format!("{digest}.blob")), body).unwrap();
        let linked_root = sandbox.path().join("linked-root");
        symlink(&outside_dir, &linked_root).unwrap();
        assert_eq!(
            FileBlobStore::new(linked_root).read(&digest, body.len()),
            Err(StoreRefusal::StorageUnavailable),
            "软链接目录不能把读请求导向其他位置"
        );
        assert_eq!(
            std::fs::read(outside_dir.join(format!("{digest}.blob"))).unwrap(),
            body
        );
    }

    #[test]
    fn file_blob_capacity_counts_orphan_files_on_disk() {
        let sandbox = crate::testutil::Sandbox::new("artifact-file-store-orphan-cap");
        let root = sandbox.path().join("blobs");
        std::fs::create_dir(&root).unwrap();
        let orphan = std::fs::File::create(root.join("orphan.blob")).unwrap();
        orphan.set_len(MAX_TOTAL_BYTES as u64).unwrap();

        let store = FileBlobStore::new(&root);
        assert_eq!(
            store.put(b"next", false, 0),
            Err(StoreRefusal::StoreFull),
            "元数据提交中断留下的孤立正文也占用磁盘预算"
        );
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
    }
}
