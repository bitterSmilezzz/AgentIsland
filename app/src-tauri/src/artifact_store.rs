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
}

impl StoreRefusal {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ContentMissing => "content_missing",
            Self::ContentImmutable => "content_immutable",
            Self::ContentTooLarge => "content_too_large",
            Self::StoreFull => "store_full",
            Self::ContentRejected => "content_rejected",
        }
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
        store.put("art-1", 2, "第二版内容".as_bytes().to_vec(), false).unwrap();
        assert_eq!(store.read("art-1", 2).unwrap(), "第二版内容".as_bytes());
        assert_eq!(store.read("art-1", 1).unwrap(), "第一版内容".as_bytes());
    }

    #[test]
    fn a_missing_version_is_reported_as_missing_not_substituted() {
        let mut store = ContentStore::new();
        store.put("art-1", 1, "第一版".as_bytes().to_vec(), false).unwrap();
        // 读第二版（不存在）⇒ 明确说没有，**不是**返回第一版
        assert_eq!(store.read("art-1", 2), Err(StoreRefusal::ContentMissing));
        assert_eq!(store.digest_of("art-1", 2), Err(StoreRefusal::ContentMissing));
        // 读另一个逻辑 id 的同一版本号 ⇒ 同样没有
        assert_eq!(store.read("other", 1), Err(StoreRefusal::ContentMissing));
    }

    #[test]
    fn the_same_version_cannot_be_rewritten() {
        let mut store = ContentStore::new();
        store.put("art-1", 1, "原内容".as_bytes().to_vec(), false).unwrap();
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
        let stored = store.put("art-1", 1, "内容".as_bytes().to_vec(), false).unwrap();
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
}
