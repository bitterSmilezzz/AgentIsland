//! 产出版本留存与比较的**判据**（TASK-05 第二片）。
//!
//! 需求原文：自有执行产物版本留存、任意版本比较/恢复、超限说明、验证清单；
//! 外部只读源不伪造历史。
//!
//! 上一片（v0.0.371）做了「有界留存与超限说明」（`MAX_ARTIFACTS=2000` +
//! `evicted_artifacts` 计数 + 引用行永不淘汰）。
//! 本片解决**版本演进与比较**（纯函数，不碰文件系统）：
//!
//! 1. **版本演进**：同一产物（同一个逻辑 id 或同一个 run）产出新版本时，
//!    `revision` 单调递增（1, 2, 3…）；**旧版本的记录原样保留**——
//!    版本不是覆盖，是向后追加。
//! 2. **外部只读源不伪造历史**：外部只读源来的产物（带 `event_ref`），其版本
//!    必须来自**外部实际观测**，不得由本仓自造 revision 推进。
//!    只有本地/自有执行产物才允许由本仓演进版本。
//! 3. **两版本比较（diff）**：任意两个 revision 的元数据比对——标题变化、
//!    种类变化、时间先后。版本相同时无 diff；版本不同时给出明确的前后变化。
//! 4. **任意版本恢复**：恢复到某一历史 revision 产出一个**新 revision**
//!    （内容等于历史版本，revision = 当前最高 + 1）——**历史不可改写**，
//!    恢复不是「删掉后面的版本」。

/// 产物的一个版本快照（包含版本演进所需的元数据）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactVersion {
    /// 逻辑产物 id（同一产物的多个版本共享此 id）。
    pub logical_id: String,
    /// 物理条目 id（每个版本的唯一条目标识）。
    pub entry_id: String,
    pub run_id: String,
    pub title: String,
    pub revision: u64,
    /// 外部事件引用。`Some` = 外部只读源，`None` = 自有执行。
    pub event_ref: Option<String>,
}

/// 版本演进拒绝原因。固定码。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionRefusal {
    /// 外部只读源不得由本仓自造版本推进（需求：外部只读源不伪造历史）。
    ExternalReadOnlyCannotEvolve,
    /// revision 倒退或重复（必须严格单调递增）。
    RevisionNotMonotonic,
    /// 要恢复的历史版本不存在。
    TargetVersionNotFound,
}

impl VersionRefusal {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ExternalReadOnlyCannotEvolve => "external_readonly_cannot_evolve",
            Self::RevisionNotMonotonic => "revision_not_monotonic",
            Self::TargetVersionNotFound => "target_version_not_found",
        }
    }
}

/// 两个版本的差异比较结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionDiff {
    pub from_revision: u64,
    pub to_revision: u64,
    /// 标题是否发生变化。
    pub title_changed: bool,
    pub title_from: String,
    pub title_to: String,
    /// 是否跨 run（新版本是否在不同的 run 产出）。
    pub cross_run: bool,
}

/// 一个逻辑产物的版本链。
#[derive(Debug, Clone, Default)]
pub struct ArtifactChain {
    pub logical_id: String,
    pub versions: Vec<ArtifactVersion>,
}

impl ArtifactChain {
    pub fn new(logical_id: &str) -> Self {
        ArtifactChain {
            logical_id: logical_id.into(),
            versions: Vec::new(),
        }
    }

    /// 当前最高 revision（无版本时为 0）。
    pub fn current_revision(&self) -> u64 {
        self.versions.last().map(|v| v.revision).unwrap_or(0)
    }

    /// 追加一个新版本。
    ///
    /// - 外部只读源（`event_ref.is_some()`）只允许初始版本（revision 1），
    ///   **不允许推进**——外部源的历史必须来自外部实际观测，本仓不替它演进；
    /// - revision 必须**严格单调递增**；
    /// - 旧版本的记录原样保留（向后追加，不覆盖）。
    pub fn append(&mut self, version: ArtifactVersion) -> Result<(), VersionRefusal> {
        if version.event_ref.is_some() && !self.versions.is_empty() {
            return Err(VersionRefusal::ExternalReadOnlyCannotEvolve);
        }
        if version.revision <= self.current_revision() {
            return Err(VersionRefusal::RevisionNotMonotonic);
        }
        self.versions.push(version);
        Ok(())
    }

    /// 任意两个版本之间的比较。
    pub fn diff(&self, from_rev: u64, to_rev: u64) -> Option<VersionDiff> {
        let from = self.versions.iter().find(|v| v.revision == from_rev)?;
        let to = self.versions.iter().find(|v| v.revision == to_rev)?;
        Some(VersionDiff {
            from_revision: from_rev,
            to_revision: to_rev,
            title_changed: from.title != to.title,
            title_from: from.title.clone(),
            title_to: to.title.clone(),
            cross_run: from.run_id != to.run_id,
        })
    }

    /// 恢复到某历史版本：**产出新 revision**（当前最高 + 1），
    /// 内容取自被恢复的历史版本。**历史不删不改**。
    pub fn restore_to(
        &mut self,
        target_revision: u64,
        new_entry_id: &str,
        new_run_id: &str,
    ) -> Result<ArtifactVersion, VersionRefusal> {
        let target = self
            .versions
            .iter()
            .find(|v| v.revision == target_revision)
            .cloned()
            .ok_or(VersionRefusal::TargetVersionNotFound)?;

        if target.event_ref.is_some() {
            return Err(VersionRefusal::ExternalReadOnlyCannotEvolve);
        }

        let new_rev = self.current_revision() + 1;
        let restored = ArtifactVersion {
            logical_id: self.logical_id.clone(),
            entry_id: new_entry_id.into(),
            run_id: new_run_id.into(),
            title: target.title,
            revision: new_rev,
            event_ref: None,
        };
        self.versions.push(restored.clone());
        Ok(restored)
    }
}

// ── TASK-05 第三片：验证清单（Verification Checklist）──────────────────
//
// 需求原文：验证清单；产出伴随明确的验收项列表。
//
// 判据：
// 1. **空清单不得声称通过**：空清单 = 还没写清单 = 不能代表验证完成；
// 2. **全通过才算通过**：所有条目必须明确为 Passed（Pending/Failed 都不算）；
// 3. **条目状态互斥**：Pending（待验）、Passed（通过）、Failed（未过）；
// 4. **条目不可匿名**：标题去空白后不能为空（未命名的条目是摆设）。

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckStatus {
    Pending,
    Passed,
    Failed,
}

impl CheckStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Passed => "passed",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckItem {
    pub id: String,
    pub title: String,
    pub status: CheckStatus,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Checklist {
    pub items: Vec<CheckItem>,
}

impl Checklist {
    pub fn new() -> Self {
        Checklist::default()
    }

    /// 添加一条验证项。标题去空白为空时拒绝。
    pub fn add(&mut self, id: &str, title: &str) -> Result<(), &'static str> {
        let clean = title.trim();
        if clean.is_empty() {
            return Err("title_empty");
        }
        if self.items.iter().any(|i| i.id == id) {
            return Err("duplicate_id");
        }
        self.items.push(CheckItem {
            id: id.to_string(),
            title: clean.to_string(),
            status: CheckStatus::Pending,
            note: None,
        });
        Ok(())
    }

    /// 记录某项的验证结果。
    pub fn mark(&mut self, id: &str, status: CheckStatus, note: Option<&str>) -> bool {
        if let Some(item) = self.items.iter_mut().find(|i| i.id == id) {
            item.status = status;
            item.note = note.map(|n| n.trim().to_string());
            true
        } else {
            false
        }
    }

    /// 整体是否**全部通过**：条目非空，且每一条都必须是 Passed。
    /// 空清单返回 false（没写清单不能声称通过）。
    pub fn all_passed(&self) -> bool {
        !self.items.is_empty() && self.items.iter().all(|i| i.status == CheckStatus::Passed)
    }

    /// 是否存在失败项。
    pub fn has_failures(&self) -> bool {
        self.items.iter().any(|i| i.status == CheckStatus::Failed)
    }

    /// 待验证的条目数。
    pub fn pending_count(&self) -> usize {
        self.items
            .iter()
            .filter(|i| i.status == CheckStatus::Pending)
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn version(entry: &str, run: &str, title: &str, rev: u64, ext: bool) -> ArtifactVersion {
        ArtifactVersion {
            logical_id: "art-1".into(),
            entry_id: entry.into(),
            run_id: run.into(),
            title: title.into(),
            revision: rev,
            event_ref: if ext { Some("evt-1".into()) } else { None },
        }
    }

    #[test]
    fn versions_evolve_monotonically_and_preserve_history() {
        let mut chain = ArtifactChain::new("art-1");
        assert_eq!(chain.current_revision(), 0);
        chain.append(version("e1", "run-1", "初版方案", 1, false)).unwrap();
        chain.append(version("e2", "run-1", "修订方案", 2, false)).unwrap();
        chain.append(version("e3", "run-2", "终版方案", 3, false)).unwrap();

        assert_eq!(chain.current_revision(), 3);
        assert_eq!(chain.versions.len(), 3, "历史记录全部保留，不覆盖");
        assert_eq!(chain.versions[0].title, "初版方案");
        assert_eq!(chain.versions[2].title, "终版方案");

        // revision 倒退或重复 ⇒ 拒绝
        assert_eq!(
            chain.append(version("e4", "run-2", "倒退", 2, false)),
            Err(VersionRefusal::RevisionNotMonotonic)
        );
        assert_eq!(
            chain.append(version("e4", "run-2", "重复", 3, false)),
            Err(VersionRefusal::RevisionNotMonotonic)
        );
    }

    #[test]
    fn external_readonly_sources_cannot_evolve_versions_in_agentisland() {
        let mut chain = ArtifactChain::new("art-ext");
        // 外部源的初始版本允许进链（记录当时观测到的状态）
        assert_eq!(
            chain.append(version("e1", "run-1", "外部方案", 1, true)),
            Ok(())
        );
        // 试图由本仓推进外部源的版本 ⇒ 明确拒绝（不伪造外部历史）
        assert_eq!(
            chain.append(version("e2", "run-1", "伪造的第二版", 2, true)),
            Err(VersionRefusal::ExternalReadOnlyCannotEvolve)
        );
        assert_eq!(chain.versions.len(), 1, "被拒的版本不得进链");
    }

    #[test]
    fn arbitrary_versions_can_be_compared() {
        let mut chain = ArtifactChain::new("art-1");
        chain.append(version("e1", "run-1", "初版方案", 1, false)).unwrap();
        chain.append(version("e2", "run-1", "初版方案", 2, false)).unwrap();
        chain.append(version("e3", "run-2", "大改方案", 3, false)).unwrap();

        // 1 vs 2：同 run，同标题
        let diff12 = chain.diff(1, 2).unwrap();
        assert_eq!(diff12.from_revision, 1);
        assert_eq!(diff12.to_revision, 2);
        assert!(!diff12.title_changed);
        assert!(!diff12.cross_run);

        // 1 vs 3：跨 run，标题改变
        let diff13 = chain.diff(1, 3).unwrap();
        assert!(diff13.title_changed);
        assert_eq!(diff13.title_from, "初版方案");
        assert_eq!(diff13.title_to, "大改方案");
        assert!(diff13.cross_run);

        // 不存在的 revision 返回 None
        assert!(chain.diff(1, 99).is_none());
    }

    #[test]
    fn restoring_a_historical_version_produces_a_new_forward_revision() {
        let mut chain = ArtifactChain::new("art-1");
        chain.append(version("e1", "run-1", "稳定版", 1, false)).unwrap();
        chain.append(version("e2", "run-2", "尝试改坏了", 2, false)).unwrap();

        // 恢复到 v1：产出 v3，内容=稳定版，历史 1 与 2 原样保留
        let restored = chain.restore_to(1, "e3", "run-3").unwrap();
        assert_eq!(restored.revision, 3, "恢复是向前追加新版本，revision=最高+1");
        assert_eq!(restored.title, "稳定版");
        assert_eq!(chain.versions.len(), 3, "历史不可改写：v1/v2/v3 都在");
        assert_eq!(chain.current_revision(), 3);

        // 恢复不存在的版本 ⇒ 拒绝
        assert_eq!(
            chain.restore_to(99, "e4", "run-4"),
            Err(VersionRefusal::TargetVersionNotFound)
        );
    }

    #[test]
    fn checklist_all_passed_requires_non_empty_and_every_item_passed() {
        let mut list = Checklist::new();
        // 空清单不得声称通过——没写清单不是通过
        assert!(!list.all_passed(), "空清单不得通过");

        list.add("c1", "单测全绿").unwrap();
        list.add("c2", "脱敏扫描 0 命中").unwrap();
        assert_eq!(list.pending_count(), 2);
        assert!(!list.all_passed(), "全部处于 Pending 时不得通过");

        list.mark("c1", CheckStatus::Passed, None);
        assert!(!list.all_passed(), "部分通过不算全部通过");

        list.mark("c2", CheckStatus::Passed, None);
        assert!(list.all_passed(), "全部条目 Passed 才算通过");
        assert!(!list.has_failures());
        assert_eq!(list.pending_count(), 0);

        // 出现一项 Failed ⇒ 不再通过且 has_failures 为真
        list.mark("c2", CheckStatus::Failed, Some("发现 1 条命中"));
        assert!(!list.all_passed());
        assert!(list.has_failures());
    }

    #[test]
    fn checklist_rejects_empty_titles_and_duplicate_ids() {
        let mut list = Checklist::new();
        assert_eq!(list.add("c1", ""), Err("title_empty"));
        assert_eq!(list.add("c1", "   "), Err("title_empty"));
        assert_eq!(list.add("c1", "合法标题"), Ok(()));
        assert_eq!(list.add("c1", "重复 id"), Err("duplicate_id"));
    }
}
