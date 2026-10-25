//! Git 仓库与并行隔离的**工作区判据**（TASK-07 首片）。
//!
//! 需求原文：独立 worktree/branch、PR/其他 Git 服务/自托管仓库、合并冲突、
//! 基线漂移与回退。场景 token：并行 worktree 不串文件；base 漂移/冲突重新 review；
//! 脏工作树保护。
//!
//! 自动化管线（`pipeline.py`）已经在临时 worktree 机制上跑过数十轮，
//! 但这些口径散在 Python 脚本里，缺少本仓原生的**结构化判据**。
//!
//! 本模块只做**判据**（纯函数，不直接调 git 命令，不碰磁盘）：
//!
//! 1. **并行分支命名与隔离**：每个任务分配独立的 `branch_for_task`；
//!    两个活跃任务**不许绑定同一分支**——串分支会让 A 任务的提交直接
//!    污染 B 任务的上下文。
//! 2. **基线漂移检测（Base Drift）**：合并前，目标分支的当前 HEAD
//!    必须与任务开始时的 `base_commit` 严格一致；**一旦漂移必须拒绝**
//!    （`BaseDrifted`），要求重新 review 后才能合并。
//! 3. **脏工作树保护**：工作树存在未提交改动时**拒绝派发新 worktree**——
//!    未提交的修改既不属于任何 commit、又可能被新 worktree 误认作基线。
//! 4. **路径重叠与冲突预警**：两个并行分支如果修改了同一个文件路径，
//!    标记为 `PathOverlap`（潜在冲突），提示需串行合并或人工介入。

use std::collections::{HashMap, HashSet};

/// 工作区隔离状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeSpec {
    pub task_id: String,
    pub branch: String,
    pub base_commit: String,
    pub modified_paths: HashSet<String>,
}

/// 拒绝原因。固定码。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitRefusal {
    /// 两个任务试图绑定同一个分支。
    BranchCollision,
    /// 基线已漂移：目标 HEAD 不等于任务开始时的 base_commit。
    BaseDrifted,
    /// 存在未提交修改（工作树脏）。
    WorktreeDirty,
    /// 任务不存在。
    TaskNotFound,
}

impl GitRefusal {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::BranchCollision => "branch_collision",
            Self::BaseDrifted => "base_drifted",
            Self::WorktreeDirty => "worktree_dirty",
            Self::TaskNotFound => "task_not_found",
        }
    }
}

/// 并行工作区管理器（纯判据状态）。
#[derive(Debug, Default)]
pub struct ParallelWorktreeManager {
    /// task_id -> WorktreeSpec
    worktrees: HashMap<String, WorktreeSpec>,
}

impl ParallelWorktreeManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// 为任务分配独立的工作区分支。
    ///
    /// 判据：
    /// - `is_clean` 必须为真（脏工作树保护：未提交修改不派新 worktree）；
    /// - 分支名不得与当前已有的其他活跃任务冲突。
    pub fn allocate(
        &mut self,
        task_id: &str,
        branch: &str,
        base_commit: &str,
        is_clean: bool,
    ) -> Result<WorktreeSpec, GitRefusal> {
        if !is_clean {
            return Err(GitRefusal::WorktreeDirty);
        }
        if self
            .worktrees
            .values()
            .any(|w| w.branch == branch && w.task_id != task_id)
        {
            return Err(GitRefusal::BranchCollision);
        }
        let spec = WorktreeSpec {
            task_id: task_id.to_string(),
            branch: branch.to_string(),
            base_commit: base_commit.to_string(),
            modified_paths: HashSet::new(),
        };
        self.worktrees.insert(task_id.to_string(), spec.clone());
        Ok(spec)
    }

    /// 登记该任务改动的文件路径列表（用于冲突检测）。
    pub fn record_paths(&mut self, task_id: &str, paths: &[&str]) -> Result<(), GitRefusal> {
        let spec = self
            .worktrees
            .get_mut(task_id)
            .ok_or(GitRefusal::TaskNotFound)?;
        for &p in paths {
            spec.modified_paths.insert(p.trim().to_string());
        }
        Ok(())
    }

    /// 合并前基线校验。
    ///
    /// 目标主干的当前 HEAD 必须仍等于任务开始时的 `base_commit`。
    /// 一旦主干被其他任务抢先推进，当前任务的基线即已过时，必须拒绝以 BaseDrifted。
    pub fn pre_merge_check(&self, task_id: &str, current_target_head: &str) -> Result<(), GitRefusal> {
        let spec = self
            .worktrees
            .get(task_id)
            .ok_or(GitRefusal::TaskNotFound)?;
        if spec.base_commit != current_target_head {
            return Err(GitRefusal::BaseDrifted);
        }
        Ok(())
    }

    /// 检测两个并行任务是否存在修改路径重叠（潜在冲突）。
    ///
    /// 返回重叠的文件列表。重叠为空表示两分支改动完全正交，可以安全并行。
    pub fn find_path_overlaps(&self, task_a: &str, task_b: &str) -> Result<Vec<String>, GitRefusal> {
        let spec_a = self.worktrees.get(task_a).ok_or(GitRefusal::TaskNotFound)?;
        let spec_b = self.worktrees.get(task_b).ok_or(GitRefusal::TaskNotFound)?;
        let mut overlaps: Vec<String> = spec_a
            .modified_paths
            .intersection(&spec_b.modified_paths)
            .cloned()
            .collect();
        overlaps.sort();
        Ok(overlaps)
    }

    /// 任务合并完成或取消，释放工作区登记。
    pub fn release(&mut self, task_id: &str) -> bool {
        self.worktrees.remove(task_id).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_worktree_allocates_and_dirty_tree_is_refused() {
        let mut mgr = ParallelWorktreeManager::new();
        // 干净工作树 ⇒ 分配成功
        let spec = mgr.allocate("task-1", "branch-task-1", "commit-base-1", true);
        assert!(spec.is_ok());

        // 脏工作树 ⇒ 明确拒绝
        let dirty = mgr.allocate("task-2", "branch-task-2", "commit-base-1", false);
        assert_eq!(dirty, Err(GitRefusal::WorktreeDirty));
    }

    #[test]
    fn two_parallel_tasks_cannot_collide_on_the_same_branch() {
        let mut mgr = ParallelWorktreeManager::new();
        mgr.allocate("task-1", "feature/login", "c-1", true).unwrap();

        // task-2 试图用同一分支名 ⇒ 碰撞拒绝
        let coll = mgr.allocate("task-2", "feature/login", "c-1", true);
        assert_eq!(coll, Err(GitRefusal::BranchCollision));

        // 用不同分支名 ⇒ 放行
        assert!(mgr.allocate("task-2", "feature/signup", "c-1", true).is_ok());
    }

    #[test]
    fn pre_merge_detects_base_drift_and_requires_re_review() {
        let mut mgr = ParallelWorktreeManager::new();
        mgr.allocate("task-1", "branch-1", "c-base", true).unwrap();

        // 目标 HEAD 没动 ⇒ 放行
        assert_eq!(mgr.pre_merge_check("task-1", "c-base"), Ok(()));

        // 目标 HEAD 被别人推进到了 c-drifted ⇒ 基线漂移，必须拒绝
        assert_eq!(
            mgr.pre_merge_check("task-1", "c-drifted"),
            Err(GitRefusal::BaseDrifted)
        );
    }

    #[test]
    fn path_overlap_identifies_shared_files_between_parallel_tasks() {
        let mut mgr = ParallelWorktreeManager::new();
        mgr.allocate("task-1", "b-1", "c", true).unwrap();
        mgr.allocate("task-2", "b-2", "c", true).unwrap();

        mgr.record_paths("task-1", &["src/auth.rs", "src/models.rs"]).unwrap();
        mgr.record_paths("task-2", &["src/views.js", "src/models.rs"]).unwrap();

        // models.rs 两边都改了 ⇒ 标记重叠
        let overlaps = mgr.find_path_overlaps("task-1", "task-2").unwrap();
        assert_eq!(overlaps, vec!["src/models.rs".to_string()]);

        // 正交改动无重叠
        mgr.allocate("task-3", "b-3", "c", true).unwrap();
        mgr.record_paths("task-3", &["docs/README.md"]).unwrap();
        let disjoint = mgr.find_path_overlaps("task-1", "task-3").unwrap();
        assert!(disjoint.is_empty());
    }

    #[test]
    fn releasing_a_task_allows_branch_reuse_safely() {
        let mut mgr = ParallelWorktreeManager::new();
        mgr.allocate("task-1", "branch-a", "c", true).unwrap();
        assert_eq!(
            mgr.allocate("task-2", "branch-a", "c", true),
            Err(GitRefusal::BranchCollision)
        );

        // task-1 完成并释放
        assert!(mgr.release("task-1"));
        // 释放后同名分支可以被新任务复用
        assert!(mgr.allocate("task-2", "branch-a", "c", true).is_ok());
    }
}
