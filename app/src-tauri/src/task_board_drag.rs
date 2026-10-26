//! 任务看板**拖拽语义**的判据（TASK-01 进展）。
//!
//! 需求原文：CRUD、归属、筛选、**拖拽**、归档、依赖、跨项目搜索与状态版本冲突。
//!
//! 背景：看板的四个列（需处理 / 执行中 / 未开始 / 已结束）**全部由运行状态派生**
//! （前端 `taskGroup` 从 run status 算出来），不是用户可以随手拖进去的容器。
//! 这带来一个必须说清的问题：**拖到「已结束」列，到底意味着什么？**
//!
//! 本模块不替使用者拍板，而是把**每种可能语义的后果写清楚**并做成显式选项，
//! 让决策只需要选一个，而不是每次拖拽都重新想一遍：
//!
//! 1. **不改写历史（默认语义）**：拖拽只改**看板投影**（顺序/归属），不产生新的
//!    运行记录、不改运行状态——否则「拖一下卡片」就凭空多出一次执行，
//!    账目与审计全对不上。
//! 2. **状态变更请求**：拖到派生列（如「已结束」）**不直接改状态**，而是产出一条
//!    带 `expected_revision` 的状态变更请求，由既有的状态机执行——尊重
//!    revision 冲突（并发时拒绝，而不是覆盖别人的改动）。
//! 3. **归属变更**：拖到「项目分组」这类**用户可拖入**的容器 ⇒ 改归属
//!    （同样带 expected_revision）。
//! 4. **越权与未知目标一律拒绝**：目标不存在、拖动已归档任务、revision 不符 ⇒ 拒绝。

/// 看板列。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoardColumn {
    /// 需处理（有未决关口）。
    Attention,
    /// 执行中。
    Running,
    /// 未开始。
    Queued,
    /// 已结束。
    Finished,
    /// 项目分组（用户可拖入的容器，不是状态）。
    ProjectGroup,
}

impl BoardColumn {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Attention => "attention",
            Self::Running => "running",
            Self::Queued => "queued",
            Self::Finished => "finished",
            Self::ProjectGroup => "project_group",
        }
    }

    /// 该列是否由**运行状态派生**（拖进去意味着要改状态）。
    pub fn is_derived_from_status(self) -> bool {
        !matches!(self, Self::ProjectGroup)
    }
}

/// 拖拽产生的动作。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DragAction {
    /// 无事可做（拖回原列/同一容器）。
    Noop,
    /// 只改看板投影的顺序（不改状态、不改归属）。
    ReorderOnly { task_id: String },
    /// 改归属（拖到项目分组）。
    ChangeOwnership {
        task_id: String,
        project_id: String,
        expected_revision: u64,
    },
    /// 产出状态变更请求（拖到派生列）。
    RequestStatusChange {
        task_id: String,
        target_column: BoardColumn,
        expected_revision: u64,
    },
}

/// 拒绝原因。固定码。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DragRefusal {
    /// 目标任务不存在。
    TaskNotFound,
    /// 目标容器不存在。
    TargetNotFound,
    /// 任务已归档，不许拖。
    TaskArchived,
    /// revision 不匹配（并发改动）——拒绝而不是覆盖。
    RevisionMismatch,
}

impl DragRefusal {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TaskNotFound => "task_not_found",
            Self::TargetNotFound => "target_not_found",
            Self::TaskArchived => "task_archived",
            Self::RevisionMismatch => "revision_mismatch",
        }
    }
}

/// 被拖动的任务。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraggableTask {
    pub id: String,
    pub project_id: Option<String>,
    pub archived: bool,
    /// 当前所在列（派生自状态）。
    pub current_column: BoardColumn,
}

/// 判定一次拖拽。
///
/// `current_revision` 是存储的当前 revision；调用方传 `expected_revision`
/// 表示「我基于哪个版本看的界面」——不匹配即拒绝（并发改动不覆盖）。
pub fn resolve_drag(
    task: Option<&DraggableTask>,
    source_column: BoardColumn,
    target_column: BoardColumn,
    target_project_id: Option<&str>,
    expected_revision: u64,
    current_revision: u64,
) -> Result<DragAction, DragRefusal> {
    let Some(task) = task else {
        return Err(DragRefusal::TaskNotFound);
    };
    if task.archived {
        return Err(DragRefusal::TaskArchived);
    }
    if expected_revision != current_revision {
        return Err(DragRefusal::RevisionMismatch);
    }
    // 拖回原处 ⇒ 无事可做
    if source_column == target_column && target_column != BoardColumn::ProjectGroup {
        return Ok(DragAction::Noop);
    }
    // 拖到项目分组 ⇒ 改归属（需要目标项目存在）
    if target_column == BoardColumn::ProjectGroup {
        let Some(project_id) = target_project_id else {
            return Err(DragRefusal::TargetNotFound);
        };
        return Ok(DragAction::ChangeOwnership {
            task_id: task.id.clone(),
            project_id: project_id.to_string(),
            expected_revision,
        });
    }
    // 拖到派生列 ⇒ **不直接改状态**，产出状态变更请求（走既有的状态机与 revision 校验）
    Ok(DragAction::RequestStatusChange {
        task_id: task.id.clone(),
        target_column,
        expected_revision,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task() -> DraggableTask {
        DraggableTask {
            id: "task-1".into(),
            project_id: Some("p-web".into()),
            archived: false,
            current_column: BoardColumn::Running,
        }
    }

    #[test]
    fn dragging_back_to_the_same_column_is_a_noop() {
        let action = resolve_drag(
            Some(&task()),
            BoardColumn::Running,
            BoardColumn::Running,
            None,
            7,
            7,
        );
        assert_eq!(action, Ok(DragAction::Noop), "拖回原处不应产生任何变更");
    }

    #[test]
    fn dragging_into_a_derived_column_produces_a_status_change_request_not_a_mutation() {
        // 拖到「已结束」：**不直接改状态**，产出带 revision 的请求
        let action = resolve_drag(
            Some(&task()),
            BoardColumn::Running,
            BoardColumn::Finished,
            None,
            7,
            7,
        );
        match action {
            Ok(DragAction::RequestStatusChange {
                task_id,
                target_column,
                expected_revision,
            }) => {
                assert_eq!(task_id, "task-1");
                assert_eq!(target_column, BoardColumn::Finished);
                assert_eq!(expected_revision, 7);
            }
            other => panic!("应当产出状态变更请求，实际 {other:?}"),
        }
    }

    #[test]
    fn dragging_into_a_project_group_changes_ownership_with_revision() {
        let action = resolve_drag(
            Some(&task()),
            BoardColumn::Running,
            BoardColumn::ProjectGroup,
            Some("p-db"),
            9,
            9,
        );
        assert_eq!(
            action,
            Ok(DragAction::ChangeOwnership {
                task_id: "task-1".into(),
                project_id: "p-db".into(),
                expected_revision: 9,
            })
        );
    }

    #[test]
    fn a_stale_revision_is_refused_rather_than_overwriting() {
        let action = resolve_drag(
            Some(&task()),
            BoardColumn::Running,
            BoardColumn::Finished,
            None,
            7,
            9, // 存储已经前进（别人改过）
        );
        assert_eq!(action, Err(DragRefusal::RevisionMismatch));
    }

    #[test]
    fn archived_and_missing_tasks_are_refused() {
        let mut archived = task();
        archived.archived = true;
        assert_eq!(
            resolve_drag(
                Some(&archived),
                BoardColumn::Running,
                BoardColumn::Finished,
                None,
                1,
                1
            ),
            Err(DragRefusal::TaskArchived)
        );
        assert_eq!(
            resolve_drag(None, BoardColumn::Running, BoardColumn::Finished, None, 1, 1),
            Err(DragRefusal::TaskNotFound)
        );
        // 拖到项目分组但没给目标项目 ⇒ 拒绝
        assert_eq!(
            resolve_drag(
                Some(&task()),
                BoardColumn::Running,
                BoardColumn::ProjectGroup,
                None,
                1,
                1
            ),
            Err(DragRefusal::TargetNotFound)
        );
    }
}
