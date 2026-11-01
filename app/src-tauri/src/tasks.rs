//! Local task organization. No task execution, approval forwarding or transcript storage.
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};

const MAX_BYTES: usize = 8 * 1024 * 1024;
const MAX_RECEIPTS: usize = 20_000;
const MAX_CHECKLIST_ITEMS_PER_ARTIFACT: usize = 100;
const MAX_CHECKLIST_ITEMS_TOTAL: usize = 2_000;
const MAX_CHECKLIST_NOTE_BYTES: usize = 2 * 1024;
/// 产出条目的留存上限（TASK-05「超限说明」）。产出是**派生的展示元数据**
/// （来源观测的投影），不像任务/项目是用户手写的记录——没有这条上限时，
/// 长期运行攒下的产出行会先撑满 `MAX_BYTES`，把「任务记录达到容量上限」
/// 挡在**用户数据**的写入上：派生数据饿死用户数据。淘汰从最旧、且**没有**
/// 被任何 attention 引用的行开始（那条链接是用户回到来源内容的路）；
/// 本地版本链也整体保留，避免容量整理切断可比较/恢复的历史。全都被引用时
/// 不淘汰，仍由 `MAX_BYTES` 兜底拒绝。被淘汰的数量记在
/// `evicted_artifacts`，对界面可见——「淘汰发生了」不许静默。
const MAX_ARTIFACTS: usize = 2_000;
fn id() -> String {
    uuid::Uuid::new_v4().to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub agent_id: String,
    pub session_id: String,
    pub thread_id: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Project {
    pub id: String,
    pub name: String,
    /// 该项目开始前依赖的项目。旧 tasks.v1.json 缺少此字段时为空。
    #[serde(default)]
    pub dependencies: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Task {
    pub id: String,
    pub title: String,
    pub project_id: Option<String>,
    pub source: Option<Source>,
    pub current_run_id: Option<String>,
    pub created_ms: i64,
    pub updated_ms: i64,
    pub archived_ms: Option<i64>,
    pub revision: u64,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Queued,
    Running,
    Waiting,
    Ready,
    Failed,
    Cancelled,
    Accepted,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Run {
    #[serde(default)]
    pub source: Option<Source>,
    pub id: String,
    pub task_id: String,
    pub status: RunStatus,
    pub started_ms: i64,
    pub finished_ms: Option<i64>,
    pub revision: u64,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AttentionKind {
    Answer,
    PlanApproval,
    ResultReview,
    Confirmation,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AttentionState {
    Open,
    Resolved,
    Expired,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Attention {
    #[serde(default)]
    pub observed: bool,
    pub id: String,
    pub run_id: String,
    pub kind: AttentionKind,
    pub state: AttentionState,
    pub artifact_id: Option<String>,
    pub manual_resolution: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Artifact {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_ref: Option<String>,
    pub id: String,
    /// Stable identity shared by explicit local versions. None keeps legacy
    /// and source-event artifacts as independent entries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logical_id: Option<String>,
    pub run_id: String,
    pub title: String,
    /// Revision within this logical artifact chain; source entries remain 1.
    pub revision: u64,
    pub kind: AttentionKind,
    /// User-authored local output is stored in an immutable content-addressed blob.
    /// Source-event bodies remain transient and never use this reference.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_content: Option<LocalArtifactContentRef>,
    /// Per-version local verification data. Old task files default to empty.
    #[serde(
        default,
        skip_serializing_if = "crate::artifact_version::Checklist::is_empty"
    )]
    pub checklist: crate::artifact_version::Checklist,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalArtifactContentRef {
    pub sha256: String,
    pub bytes: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    pub source: Source,
    pub fingerprint: String,
    pub run_id: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Data {
    pub schema_version: u32,
    pub revision: u64,
    pub projects: Vec<Project>,
    pub tasks: Vec<Task>,
    pub runs: Vec<Run>,
    pub attentions: Vec<Attention>,
    pub artifacts: Vec<Artifact>,
    /// 留存上限累计淘汰的产出条数（见 `MAX_ARTIFACTS`）。0 = 从未淘汰。
    #[serde(default)]
    pub evicted_artifacts: u64,
    pub event_receipts: Vec<Receipt>,
}
impl Default for Data {
    fn default() -> Self {
        Self {
            schema_version: 1,
            revision: 0,
            projects: vec![],
            tasks: vec![],
            runs: vec![],
            attentions: vec![],
            artifacts: vec![],
            evicted_artifacts: 0,
            event_receipts: vec![],
        }
    }
}
impl Data {
    /// 产出留存的上限执行：淘汰只动**未被引用且不属于版本链**的最旧行，
    /// 引用行与完整版本历史永远保留；一个都动不了时由 `MAX_BYTES` 兜底拒写。
    fn enforce_artifact_cap(&mut self) {
        while self.artifacts.len() > MAX_ARTIFACTS {
            let Some(index) = self.artifacts.iter().position(|artifact| {
                artifact.logical_id.is_none()
                    && !self
                        .attentions
                        .iter()
                        .any(|attention| attention.artifact_id.as_deref() == Some(&artifact.id))
            }) else {
                break;
            };
            self.artifacts.remove(index);
            self.evicted_artifacts = self.evicted_artifacts.saturating_add(1);
        }
    }
}
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Error {
    pub code: String,
    pub message: String,
}
fn error(code: &str, message: &str) -> Error {
    Error {
        code: code.into(),
        message: message.into(),
    }
}
fn artifact_store_error(reason: crate::artifact_store::StoreRefusal) -> Error {
    let (code, message) = reason.error_details();
    error(code, message)
}
fn artifact_version_error(reason: crate::artifact_version::VersionRefusal) -> Error {
    use crate::artifact_version::VersionRefusal;
    match reason {
        VersionRefusal::ExternalReadOnlyCannotEvolve => error(
            "external_readonly_cannot_evolve",
            "来源事件产物不能创建本地版本",
        ),
        VersionRefusal::RevisionNotMonotonic => {
            error("read_only", "产出版本链无效，原任务记录已保留")
        }
        VersionRefusal::TargetVersionNotFound => error("not_found", "所选产出版本不存在"),
        VersionRefusal::DifferentLogicalArtifact => {
            error("source_mismatch", "所选版本不属于同一逻辑产出")
        }
    }
}
fn dependency_error(reason: crate::project_deps::DepRefusal) -> Error {
    use crate::project_deps::DepRefusal;
    match reason {
        DepRefusal::UnknownDependency => error("unknown_dependency", "依赖项目不存在"),
        DepRefusal::DuplicateDependency => error("duplicate_dependency", "依赖项目不能重复"),
        DepRefusal::DuplicateProject => error("duplicate_project", "项目记录重复"),
        DepRefusal::Cycle => error("dependency_cycle", "该依赖会形成循环"),
        DepRefusal::DepthExceeded => error("dependency_depth_exceeded", "依赖链最多 8 层"),
        DepRefusal::SelfDependency => error("self_dependency", "项目不能依赖自己"),
        DepRefusal::StillDependedOn => error("still_depended_on", "仍有其他项目依赖此项目"),
    }
}
fn task_drag_error(reason: crate::task_board_drag::DragRefusal) -> Error {
    use crate::task_board_drag::DragRefusal;
    match reason {
        DragRefusal::TaskNotFound => error("not_found", "任务不存在"),
        DragRefusal::TargetNotFound => error("target_not_found", "目标项目不存在"),
        DragRefusal::TaskArchived => error("task_archived", "已归档任务不能拖动"),
        DragRefusal::RevisionMismatch => error("stale_revision", "任务已更新，请刷新后重试"),
        DragRefusal::SourceColumnMismatch => {
            error("source_column_mismatch", "任务所在看板列已变化，拖放未应用")
        }
    }
}
fn text(value: &str) -> Result<String, Error> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > 500 {
        return Err(error("invalid_input", "名称需为 1–500 个字符"));
    }
    // Do not accept credential-shaped material as a task/project/artifact title.
    if value.contains("-----BEGIN")
        || value
            .split_whitespace()
            .any(|s| s.starts_with("sk-") && s.len() > 20)
    {
        return Err(error("invalid_input", "名称不能包含凭据"));
    }
    Ok(value.into())
}

pub struct Store {
    pub path: PathBuf,
}
impl Store {
    pub fn at_default() -> Self {
        Self {
            path: crate::settings::config_dir().join("tasks.v1.json"),
        }
    }
    pub fn artifact_blob_store(&self) -> Result<crate::artifact_store::FileBlobStore, Error> {
        let parent = self
            .path
            .parent()
            .ok_or_else(|| error("io_error", "任务目录不可用"))?;
        Ok(crate::artifact_store::FileBlobStore::new(
            parent.join("task-artifact-content"),
        ))
    }
    pub fn load(&self) -> Result<Data, Error> {
        let metadata = match fs::symlink_metadata(&self.path) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Data::default()),
            Err(_) => return Err(error("io_error", "任务文件不可读，原文件已保留")),
        };
        if !metadata.is_file()
            || metadata.file_type().is_symlink()
            || metadata.len() > MAX_BYTES as u64
        {
            return Err(error(
                "read_only",
                "任务文件类型或容量不受支持，原文件已保留",
            ));
        }
        let bytes =
            fs::read(&self.path).map_err(|_| error("io_error", "任务文件不可读，原文件已保留"))?;
        let data: Data = serde_json::from_slice(&bytes)
            .map_err(|_| error("read_only", "任务文件损坏或版本不兼容，原文件已保留"))?;
        data.validate()?;
        Ok(data)
    }
    fn write(&self, data: &Data) -> Result<(), Error> {
        data.validate()?;
        let bytes =
            serde_json::to_vec_pretty(data).map_err(|_| error("io_error", "任务编码失败"))?;
        if bytes.len() > MAX_BYTES {
            return Err(error("capacity", "任务记录达到容量上限；未删除旧记录"));
        }
        fs::create_dir_all(
            self.path
                .parent()
                .ok_or_else(|| error("io_error", "任务目录不可用"))?,
        )
        .map_err(|_| error("io_error", "任务目录不可写"))?;
        crate::atomicfile::atomic_replace_validated(&self.path, &bytes, |path| {
            let check: Data =
                serde_json::from_slice(&fs::read(path)?).map_err(std::io::Error::other)?;
            check
                .validate()
                .map_err(|_| std::io::Error::other("invalid task data"))
        })
        .map_err(|_| error("io_error", "任务保存失败，原记录已保留"))
    }
    fn edit(
        &self,
        revision: u64,
        f: impl FnOnce(&mut Data) -> Result<(), Error>,
    ) -> Result<Data, Error> {
        let mut data = self.load()?;
        if data.revision != revision {
            return Err(error("stale_revision", "任务已更新，请刷新后重试"));
        }
        f(&mut data)?;
        data.revision = data
            .revision
            .checked_add(1)
            .ok_or_else(|| error("capacity", "任务版本已达上限"))?;
        self.write(&data)?;
        Ok(data)
    }
    pub fn project_create(&self, name: &str, revision: u64) -> Result<Data, Error> {
        let name = text(name)?;
        self.edit(revision, |d| {
            d.projects.push(Project {
                id: id(),
                name,
                dependencies: Vec::new(),
            });
            Ok(())
        })
    }
    pub fn project_dependencies_update(
        &self,
        project: &str,
        dependencies: Vec<String>,
        revision: u64,
    ) -> Result<Data, Error> {
        self.edit(revision, |d| {
            let project_index = d
                .projects
                .iter()
                .position(|item| item.id == project)
                .ok_or_else(|| error("not_found", "项目不存在"))?;
            let mut graph = d.project_dependency_graph();
            graph[project_index].1 = dependencies.clone();
            crate::project_deps::validate_graph(&graph).map_err(dependency_error)?;
            d.projects[project_index].dependencies = dependencies;
            Ok(())
        })
    }
    pub fn project_delete(&self, project: &str, revision: u64) -> Result<Data, Error> {
        self.edit(revision, |d| {
            if !d.projects.iter().any(|item| item.id == project) {
                return Err(error("not_found", "项目不存在"));
            }
            crate::project_deps::check_removable(&d.project_dependency_graph(), project)
                .map_err(dependency_error)?;
            if d.tasks
                .iter()
                .any(|task| task.project_id.as_deref() == Some(project))
            {
                return Err(error(
                    "project_in_use",
                    "该项目仍有任务归属，请先调整任务项目",
                ));
            }
            d.projects.retain(|item| item.id != project);
            Ok(())
        })
    }
    pub fn create(
        &self,
        title: &str,
        project: Option<String>,
        revision: u64,
        now: i64,
    ) -> Result<Data, Error> {
        let title = text(title)?;
        self.edit(revision, |d| {
            d.check_project(project.as_deref())?;
            d.tasks.push(Task {
                id: id(),
                title,
                project_id: project,
                source: None,
                current_run_id: None,
                created_ms: now,
                updated_ms: now,
                archived_ms: None,
                revision: 1,
            });
            Ok(())
        })
    }
    pub fn update(
        &self,
        task: &str,
        title: &str,
        project: Option<String>,
        revision: u64,
        now: i64,
    ) -> Result<Data, Error> {
        let title = text(title)?;
        self.edit(revision, |d| {
            d.check_project(project.as_deref())?;
            let t = d.task_mut(task)?;
            t.title = title;
            t.project_id = project;
            t.updated_ms = now;
            t.revision += 1;
            Ok(())
        })
    }
    /// Move an active task between project groups without creating a run or
    /// changing the run-derived status. The source column and target project
    /// are checked against the same snapshot whose aggregate revision is
    /// advanced atomically with the ownership change.
    pub fn board_move_project(
        &self,
        task_id: &str,
        source_column: crate::task_board_drag::BoardColumn,
        project_id: Option<String>,
        expected_revision: u64,
        now: i64,
    ) -> Result<Data, Error> {
        use crate::task_board_drag::{resolve_drag, DragAction, DraggableTask};
        let mut data = self.load()?;
        let task = data
            .tasks
            .iter()
            .find(|task| task.id == task_id)
            .ok_or_else(|| error("not_found", "任务不存在"))?;
        let draggable = DraggableTask {
            id: task.id.clone(),
            project_id: task.project_id.clone(),
            archived: task.archived_ms.is_some(),
            current_column: board_column(&data, task),
        };
        let project_ids = data
            .projects
            .iter()
            .map(|project| project.id.clone())
            .collect::<Vec<_>>();
        let action = resolve_drag(
            Some(&draggable),
            source_column,
            crate::task_board_drag::BoardColumn::ProjectGroup,
            project_id.as_deref(),
            expected_revision,
            data.revision,
            &project_ids,
        )
        .map_err(task_drag_error)?;
        match action {
            DragAction::Noop => Ok(data),
            DragAction::ChangeOwnership {
                task_id,
                project_id,
                expected_revision: action_revision,
            } => {
                if action_revision != data.revision {
                    return Err(task_drag_error(
                        crate::task_board_drag::DragRefusal::RevisionMismatch,
                    ));
                }
                let task = data
                    .tasks
                    .iter_mut()
                    .find(|task| task.id == task_id)
                    .ok_or_else(|| error("not_found", "任务不存在"))?;
                task.project_id = project_id;
                task.updated_ms = now;
                task.revision = task
                    .revision
                    .checked_add(1)
                    .ok_or_else(|| error("capacity", "任务版本已达上限"))?;
                data.revision = data
                    .revision
                    .checked_add(1)
                    .ok_or_else(|| error("capacity", "任务版本已达上限"))?;
                self.write(&data)?;
                Ok(data)
            }
            DragAction::ReorderOnly { .. } | DragAction::RequestStatusChange { .. } => {
                Err(error("invalid_input", "项目拖放只能调整任务归属"))
            }
        }
    }
    /// Consume an explicit drag request between run-derived columns.
    /// The caller must provide the concrete local progress chosen in the
    /// confirmation dialog; the coarse target column alone is never written.
    pub fn board_change_status(
        &self,
        task_id: &str,
        source_column: crate::task_board_drag::BoardColumn,
        target_column: crate::task_board_drag::BoardColumn,
        status: RunStatus,
        kind: Option<AttentionKind>,
        expected_revision: u64,
        now: i64,
    ) -> Result<Data, Error> {
        use crate::task_board_drag::{resolve_drag, DragAction, DraggableTask};

        if status == RunStatus::Accepted {
            return Err(error(
                "invalid_input",
                "拖放只能记录本地进度，不能标记来源已验收",
            ));
        }
        let mut data = self.load()?;
        if data.revision != expected_revision {
            return Err(error("stale_revision", "任务已更新，请刷新后重试"));
        }
        let task = data
            .tasks
            .iter()
            .find(|task| task.id == task_id)
            .ok_or_else(|| error("not_found", "任务不存在"))?;
        let draggable = DraggableTask {
            id: task.id.clone(),
            project_id: task.project_id.clone(),
            archived: task.archived_ms.is_some(),
            current_column: board_column(&data, task),
        };
        let project_ids = data
            .projects
            .iter()
            .map(|project| project.id.clone())
            .collect::<Vec<_>>();
        let action = resolve_drag(
            Some(&draggable),
            source_column,
            target_column,
            None,
            expected_revision,
            data.revision,
            &project_ids,
        )
        .map_err(task_drag_error)?;

        match action {
            DragAction::Noop => Ok(data),
            DragAction::RequestStatusChange {
                task_id: action_task,
                target_column: action_target,
                expected_revision: action_revision,
            } => {
                if action_task != task_id
                    || action_target != target_column
                    || action_revision != data.revision
                    || !status_matches_board_column(status, kind, target_column)
                {
                    return Err(error("invalid_input", "所选本地进度与拖放目标不匹配"));
                }
                data.record(task_id, status, kind, None, now, false)?;
                data.revision = data
                    .revision
                    .checked_add(1)
                    .ok_or_else(|| error("capacity", "任务版本已达上限"))?;
                self.write(&data)?;
                Ok(data)
            }
            DragAction::ChangeOwnership { .. } | DragAction::ReorderOnly { .. } => {
                Err(error("invalid_input", "状态拖放目标必须是看板状态列"))
            }
        }
    }
    pub fn archive(
        &self,
        task: &str,
        archived: bool,
        revision: u64,
        now: i64,
    ) -> Result<Data, Error> {
        self.edit(revision, |d| {
            let t = d.task_mut(task)?;
            t.archived_ms = archived.then_some(now);
            t.updated_ms = now;
            t.revision += 1;
            Ok(())
        })
    }
    pub fn link(&self, task: &str, source: Source, revision: u64, now: i64) -> Result<Data, Error> {
        self.edit(revision, |d| {
            source.validate()?;
            if d.tasks
                .iter()
                .any(|t| t.id != task && t.source.as_ref() == Some(&source))
            {
                return Err(error("source_in_use", "此会话已关联其他任务"));
            }
            let t = d.task_mut(task)?;
            if t.current_run_id.is_some()
                && t.source.is_some()
                && t.source.as_ref() != Some(&source)
            {
                return Err(error(
                    "invalid_input",
                    "已有来源运行记录的任务不能更换会话；请新建任务",
                ));
            }
            if t.source.is_none() {
                let old = t.current_run_id.take();
                for a in &mut d.attentions {
                    if Some(&a.run_id) == old.as_ref() && a.state == AttentionState::Open {
                        a.state = AttentionState::Expired;
                    }
                }
            }
            let t = d.task_mut(task)?;
            t.source = Some(source);
            t.updated_ms = now;
            t.revision += 1;
            Ok(())
        })
    }
    // Explicit user-authored local progress, never a command to the source agent.
    pub fn record(
        &self,
        task: &str,
        status: RunStatus,
        kind: Option<AttentionKind>,
        title: Option<String>,
        revision: u64,
        now: i64,
    ) -> Result<Data, Error> {
        self.record_with_content(task, status, kind, title, None, revision, now)
    }

    /// Record local progress and optionally persist a user-authored output body.
    /// The body is published before the task JSON references it, so a failed
    /// metadata write can leave only an unreadable orphan blob.
    pub fn record_with_content(
        &self,
        task: &str,
        status: RunStatus,
        kind: Option<AttentionKind>,
        title: Option<String>,
        content: Option<String>,
        revision: u64,
        now: i64,
    ) -> Result<Data, Error> {
        if status == RunStatus::Accepted {
            return Err(error(
                "invalid_input",
                "来源验收需可信事件；本地处理不会批准工具操作",
            ));
        }
        let title = title.map(|t| text(&t)).transpose()?;
        let content = content.filter(|body| !body.trim().is_empty());
        if content.is_some() && title.is_none() {
            return Err(error("invalid_input", "保存产出正文前请先填写产出名称"));
        }
        if content
            .as_ref()
            .is_some_and(|body| body.len() > crate::task_artifacts::MAX_BODY)
        {
            return Err(error("capacity", "产出正文超过 64 KiB，未保存"));
        }
        if content
            .as_deref()
            .is_some_and(crate::private_text::known_private)
        {
            return Err(error(
                "invalid_input",
                "产出正文包含已识别的凭据形态，未保存",
            ));
        }
        if title.is_some() && kind.is_none() && status != RunStatus::Ready {
            return Err(error("invalid_input", "产出引用需关联结果就绪或人工关口"));
        }
        let blob_store = content
            .as_ref()
            .map(|_| self.artifact_blob_store())
            .transpose()?;
        self.edit(revision, |d| {
            let artifact_id = d.record(task, status, kind, title, now, false)?;
            if let Some(content) = content.as_deref() {
                let artifact_id =
                    artifact_id.ok_or_else(|| error("capacity", "产出留存已满，正文未保存"))?;
                let total = d.local_artifact_content_bytes();
                let stored = blob_store
                    .as_ref()
                    .ok_or_else(|| error("io_error", "产出存储不可用"))?
                    .put(content.as_bytes(), false, total)
                    .map_err(artifact_store_error)?;
                let artifact = d
                    .artifacts
                    .iter_mut()
                    .find(|artifact| artifact.id == artifact_id)
                    .ok_or_else(|| error("capacity", "产出留存已满，正文未保存"))?;
                artifact.local_content = Some(LocalArtifactContentRef {
                    sha256: stored.sha256,
                    bytes: stored.bytes,
                });
            }
            Ok(())
        })
    }

    /// Add an explicitly authored version to an existing local output chain.
    /// This changes task metadata only; it never starts, accepts, or forwards work.
    pub fn record_local_artifact_version(
        &self,
        task: &str,
        artifact_id: &str,
        title: String,
        content: String,
        revision: u64,
        now: i64,
    ) -> Result<Data, Error> {
        let title = text(&title)?;
        if content.trim().is_empty() {
            return Err(error("invalid_input", "新版本正文不能为空"));
        }
        if content.len() > crate::task_artifacts::MAX_BODY {
            return Err(error("capacity", "产出正文超过 64 KiB，未保存"));
        }
        if crate::private_text::known_private(&content) {
            return Err(error(
                "invalid_input",
                "产出正文包含已识别的凭据形态，未保存",
            ));
        }
        let blob_store = self.artifact_blob_store()?;
        self.edit(revision, |data| {
            let task_record = data
                .tasks
                .iter()
                .find(|item| item.id == task)
                .ok_or_else(|| error("not_found", "任务不存在"))?;
            if task_record.archived_ms.is_some() {
                return Err(error("invalid_input", "请先恢复任务"));
            }
            let current_run_id = task_record.current_run_id.clone();
            let (selected, _) = data.local_artifact_version_ref(task, artifact_id)?;
            let mut chain = data.local_artifact_chain(task, artifact_id)?;
            let logical_id = chain.logical_id.clone();

            // Once a user starts a version chain, retain every member. Make the
            // existing singleton explicit before enforcing the row limit.
            for artifact in &mut data.artifacts {
                if artifact.logical_id.as_deref().unwrap_or(&artifact.id) == logical_id {
                    artifact.logical_id = Some(logical_id.clone());
                }
            }
            let run_id = current_run_id.unwrap_or_else(|| selected.run_id.clone());
            if !data
                .runs
                .iter()
                .any(|run| run.id == run_id && run.task_id == task)
            {
                return Err(error("source_mismatch", "当前运行不属于所选任务"));
            }
            let new_id = id();
            let next_revision = chain
                .current_revision()
                .checked_add(1)
                .ok_or_else(|| error("capacity", "产出版本号已达上限"))?;
            let version = crate::artifact_version::ArtifactVersion {
                logical_id: logical_id.clone(),
                entry_id: new_id.clone(),
                run_id: run_id.clone(),
                title: title.clone(),
                revision: next_revision,
                event_ref: None,
            };
            chain.append(version).map_err(artifact_version_error)?;

            let kind = data
                .artifacts
                .iter()
                .find(|artifact| artifact.id == selected.entry_id)
                .map(|artifact| artifact.kind)
                .ok_or_else(|| error("not_found", "产出版本不存在"))?;
            data.artifacts.push(Artifact {
                event_ref: None,
                id: new_id.clone(),
                logical_id: Some(logical_id),
                run_id: run_id.clone(),
                title,
                revision: next_revision,
                kind,
                local_content: None,
                checklist: crate::artifact_version::Checklist::default(),
            });
            data.enforce_artifact_cap();
            if data.artifacts.len() > MAX_ARTIFACTS {
                return Err(error("capacity", "产出版本留存达到上限；历史未改动"));
            }
            let stored = blob_store
                .put(
                    content.as_bytes(),
                    false,
                    data.local_artifact_content_bytes(),
                )
                .map_err(artifact_store_error)?;
            let new_artifact = data
                .artifacts
                .iter_mut()
                .find(|artifact| artifact.id == new_id)
                .ok_or_else(|| error("capacity", "产出版本留存达到上限；正文未保存"))?;
            new_artifact.local_content = Some(LocalArtifactContentRef {
                sha256: stored.sha256,
                bytes: stored.bytes,
            });
            let run = data
                .runs
                .iter_mut()
                .find(|run| run.id == run_id && run.task_id == task)
                .ok_or_else(|| error("source_mismatch", "当前运行不属于所选任务"))?;
            run.revision = run
                .revision
                .checked_add(1)
                .ok_or_else(|| error("capacity", "运行版本已达上限"))?;
            let task_record = data
                .tasks
                .iter_mut()
                .find(|item| item.id == task)
                .ok_or_else(|| error("not_found", "任务不存在"))?;
            task_record.updated_ms = now;
            task_record.revision = task_record
                .revision
                .checked_add(1)
                .ok_or_else(|| error("capacity", "任务版本已达上限"))?;
            Ok(())
        })
    }

    /// Restore a historical local body by appending it as the next version.
    pub fn restore_local_artifact_version(
        &self,
        task: &str,
        artifact_id: &str,
        expected_revision: u64,
        now: i64,
    ) -> Result<Data, Error> {
        let snapshot = self.load()?;
        if snapshot.revision != expected_revision {
            return Err(error("stale_revision", "任务已更新，请刷新后重试"));
        }
        let (version, content_ref) = snapshot.local_artifact_version_ref(task, artifact_id)?;
        let bytes = self
            .artifact_blob_store()?
            .read(&content_ref.sha256, content_ref.bytes)
            .map_err(artifact_store_error)?;
        let content = String::from_utf8(bytes)
            .map_err(|_| error("content_corrupt", "本地产出正文格式无效，未恢复"))?;
        let readable = crate::task_artifacts::local_content(content.clone());
        if !readable.available {
            return Err(error("invalid_input", &readable.notice));
        }
        self.record_local_artifact_version(
            task,
            artifact_id,
            version.title,
            content,
            expected_revision,
            now,
        )
    }

    fn verify_checklist_artifact_body(&self, data: &Data, artifact_id: &str) -> Result<(), Error> {
        let content_ref = data
            .artifacts
            .iter()
            .find(|artifact| artifact.id == artifact_id)
            .and_then(|artifact| artifact.local_content.as_ref())
            .ok_or_else(|| error("content_missing", "此产出没有保存本地正文"))?;
        let bytes = self
            .artifact_blob_store()?
            .read(&content_ref.sha256, content_ref.bytes)
            .map_err(artifact_store_error)?;
        let content = String::from_utf8(bytes)
            .map_err(|_| error("content_corrupt", "本地产出正文格式无效，未保存验证清单"))?;
        let readable = crate::task_artifacts::local_content(content);
        if !readable.available {
            return Err(error("invalid_input", &readable.notice));
        }
        Ok(())
    }

    /// Add one validation item to an exact locally persisted artifact version.
    pub fn add_artifact_checklist_item(
        &self,
        task: &str,
        artifact_id: &str,
        title: &str,
        revision: u64,
        now: i64,
    ) -> Result<Data, Error> {
        let title = text(title)?;
        self.edit(revision, |data| {
            let task_record = data
                .tasks
                .iter()
                .find(|item| item.id == task)
                .ok_or_else(|| error("not_found", "任务不存在"))?;
            if task_record.archived_ms.is_some() {
                return Err(error("invalid_input", "请先恢复任务"));
            }
            let run_id = data.local_checklist_run(task, artifact_id)?;
            self.verify_checklist_artifact_body(data, artifact_id)?;
            let item_count = data.checklist_item_count();
            if item_count >= MAX_CHECKLIST_ITEMS_TOTAL {
                return Err(error("capacity", "验证清单达到本地条目上限"));
            }
            let artifact = data
                .artifacts
                .iter_mut()
                .find(|item| item.id == artifact_id)
                .ok_or_else(|| error("not_found", "产出版本不存在"))?;
            if artifact.checklist.items.len() >= MAX_CHECKLIST_ITEMS_PER_ARTIFACT {
                return Err(error("capacity", "此版本的验证项已达到上限"));
            }
            artifact
                .checklist
                .add(&id(), &title)
                .map_err(|_| error("invalid_input", "验证项名称无效或重复"))?;
            touch_checklist_owner(data, task, &run_id, now)
        })
    }

    /// Update the explicit result for one item on an exact artifact version.
    pub fn mark_artifact_checklist_item(
        &self,
        task: &str,
        artifact_id: &str,
        item_id: &str,
        status: crate::artifact_version::CheckStatus,
        revision: u64,
        now: i64,
    ) -> Result<Data, Error> {
        self.edit(revision, |data| {
            let task_record = data
                .tasks
                .iter()
                .find(|item| item.id == task)
                .ok_or_else(|| error("not_found", "任务不存在"))?;
            if task_record.archived_ms.is_some() {
                return Err(error("invalid_input", "请先恢复任务"));
            }
            let run_id = data.local_checklist_run(task, artifact_id)?;
            self.verify_checklist_artifact_body(data, artifact_id)?;
            let artifact = data
                .artifacts
                .iter_mut()
                .find(|item| item.id == artifact_id)
                .ok_or_else(|| error("not_found", "产出版本不存在"))?;
            if !artifact.checklist.mark(item_id, status, None) {
                return Err(error("not_found", "验证项不存在"));
            }
            touch_checklist_owner(data, task, &run_id, now)
        })
    }

    pub fn handled(
        &self,
        task: &str,
        run: &str,
        attention: &str,
        revision: u64,
        now: i64,
    ) -> Result<Data, Error> {
        self.edit(revision, |d| {
            let t = d
                .tasks
                .iter()
                .find(|t| t.id == task)
                .ok_or_else(|| error("not_found", "任务不存在"))?;
            if t.current_run_id.as_deref() != Some(run) {
                return Err(error("source_expired", "这次运行已过期"));
            }
            let a = d
                .attentions
                .iter_mut()
                .find(|a| a.id == attention && a.run_id == run)
                .ok_or_else(|| error("not_found", "需处理事项不存在"))?;
            if a.state != AttentionState::Open {
                return Err(error("source_expired", "这条事项已处理或过期"));
            }
            a.state = AttentionState::Resolved;
            a.manual_resolution = true;
            let r = d.runs.iter_mut().find(|r| r.id == run).unwrap();
            if r.status == RunStatus::Waiting {
                r.status = RunStatus::Queued;
            }
            r.revision += 1;
            let t = d.task_mut(task)?;
            t.updated_ms = now;
            t.revision += 1;
            Ok(())
        })
    }
    pub fn sync(
        &self,
        observations: &[crate::task_sources::Observation],
        now: i64,
    ) -> Result<Option<Data>, Error> {
        let data = self.load()?;
        let pending: Vec<_> = observations
            .iter()
            .filter(|o| {
                data.tasks
                    .iter()
                    .any(|t| t.source.as_ref() == Some(&o.source))
                    && (!data
                        .event_receipts
                        .iter()
                        .any(|r| r.source == o.source && r.fingerprint == o.fingerprint)
                        || data.late_plan_attention(o).is_some())
            })
            .collect();
        if pending.is_empty() {
            return Ok(None);
        }
        let new_receipts = pending
            .iter()
            .filter(|o| {
                !data
                    .event_receipts
                    .iter()
                    .any(|r| r.source == o.source && r.fingerprint == o.fingerprint)
            })
            .count();
        if data.event_receipts.len().saturating_add(new_receipts) > MAX_RECEIPTS {
            return Err(error("capacity", "事件记录达到容量上限；旧记录已保留"));
        }
        self.edit(data.revision, |d| {
            for o in pending {
                o.source.validate()?;
                if o.fingerprint.is_empty()
                    || o.fingerprint.len() > 128
                    || !o
                        .fingerprint
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || c == b'-')
                    || !matches!(
                        o.status,
                        RunStatus::Running
                            | RunStatus::Waiting
                            | RunStatus::Ready
                            | RunStatus::Failed
                            | RunStatus::Cancelled
                    )
                {
                    return Err(error("invalid_input", "来源事件无效"));
                }
                if d.event_receipts
                    .iter()
                    .any(|r| r.source == o.source && r.fingerprint == o.fingerprint)
                {
                    if let Some(attention_id) = d.late_plan_attention(o) {
                        let evidence = o.artifact.as_ref().unwrap();
                        let attention = d
                            .attentions
                            .iter_mut()
                            .find(|a| a.id == attention_id)
                            .unwrap();
                        let artifact_id = id();
                        d.artifacts.push(Artifact {
                            event_ref: Some(evidence.event_ref.clone()),
                            id: artifact_id.clone(),
                            logical_id: None,
                            run_id: attention.run_id.clone(),
                            title: evidence.title().into(),
                            revision: 1,
                            kind: evidence.kind,
                            local_content: None,
                            checklist: crate::artifact_version::Checklist::default(),
                        });
                        attention.artifact_id = Some(artifact_id);
                        let run_id = attention.run_id.clone();
                        d.enforce_artifact_cap();
                        let run = d.runs.iter_mut().find(|r| r.id == run_id).unwrap();
                        run.revision += 1;
                        let task_id = run.task_id.clone();
                        let task = d.task_mut(&task_id)?;
                        task.updated_ms = now;
                        task.revision += 1;
                    }
                    continue;
                }
                let task = d
                    .tasks
                    .iter()
                    .find(|t| t.source.as_ref() == Some(&o.source))
                    .unwrap()
                    .id
                    .clone();
                if let Some(evidence) = &o.artifact {
                    if !evidence.valid()
                        || !((matches!(
                            evidence.kind,
                            AttentionKind::Answer | AttentionKind::PlanApproval
                        ) && o.status == RunStatus::Waiting
                            && o.kind == Some(evidence.kind))
                            || (evidence.kind == AttentionKind::ResultReview
                                && o.status == RunStatus::Ready
                                && o.kind.is_none()))
                    {
                        return Err(error("invalid_input", "来源引用与事件不匹配"));
                    }
                }
                d.record(
                    &task,
                    o.status,
                    o.kind,
                    o.artifact.as_ref().map(|e| e.title().to_string()),
                    now,
                    true,
                )?;
                if let Some(evidence) = &o.artifact {
                    d.artifacts
                        .last_mut()
                        .ok_or_else(|| error("invalid_input", "来源引用未生成"))?
                        .event_ref = Some(evidence.event_ref.clone());
                }
                let run_id = d.task_mut(&task)?.current_run_id.clone().unwrap();
                d.event_receipts.push(Receipt {
                    source: o.source.clone(),
                    fingerprint: o.fingerprint.clone(),
                    run_id,
                });
            }
            Ok(())
        })
        .map(Some)
    }
    pub fn observe(
        &self,
        source: Source,
        fingerprint: String,
        status: RunStatus,
        kind: Option<AttentionKind>,
        now: i64,
    ) -> Result<Option<Data>, Error> {
        source.validate()?;
        if fingerprint.is_empty()
            || fingerprint.len() > 128
            || !fingerprint
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-')
        {
            return Err(error("invalid_input", "事件标识无效"));
        }
        if !matches!(
            status,
            RunStatus::Running
                | RunStatus::Waiting
                | RunStatus::Ready
                | RunStatus::Failed
                | RunStatus::Cancelled
                // **受理（Accepted）走来源路径**：它需要「可信事件」，而可信事件的
                // 载体正是本方法（来源绑定 + 指纹 + 幂等回执）。本地 `record` 刻意
                // 拒绝受理（「本地记录永不批准来源操作」）——两边的分工在这里合上：
                // 只有绑定了已知来源的事件才能把一次运行标为已受理。
                | RunStatus::Accepted
        ) {
            return Err(error("invalid_input", "不支持的来源状态"));
        }
        let data = self.load()?;
        // Sampling alone never invents tasks: a user must first bind a known source.
        let Some(task) = data
            .tasks
            .iter()
            .find(|t| t.source.as_ref() == Some(&source))
            .map(|t| t.id.clone())
        else {
            return Ok(None);
        };
        if data
            .event_receipts
            .iter()
            .any(|r| r.source == source && r.fingerprint == fingerprint)
        {
            return Ok(None);
        }
        if data.event_receipts.len() >= MAX_RECEIPTS {
            return Err(error("capacity", "事件记录达到容量上限；旧记录已保留"));
        }
        self.edit(data.revision, |d| {
            d.record(&task, status, kind, None, now, true)?;
            let run_id = d.task_mut(&task)?.current_run_id.clone().unwrap();
            d.event_receipts.push(Receipt {
                source,
                fingerprint,
                run_id,
            });
            Ok(())
        })
        .map(Some)
    }
}
impl Data {
    fn checklist_item_count(&self) -> usize {
        self.artifacts
            .iter()
            .map(|artifact| artifact.checklist.items.len())
            .fold(0usize, usize::saturating_add)
    }

    fn local_checklist_run(&self, task_id: &str, artifact_id: &str) -> Result<String, Error> {
        self.tasks
            .iter()
            .find(|task| task.id == task_id)
            .ok_or_else(|| error("not_found", "任务不存在"))?;
        let artifact = self
            .artifacts
            .iter()
            .find(|artifact| artifact.id == artifact_id)
            .ok_or_else(|| error("not_found", "产出版本不存在"))?;
        if artifact.event_ref.is_some() {
            return Err(error(
                "external_readonly_cannot_evolve",
                "来源事件产物不能保存本地验证清单",
            ));
        }
        if artifact.local_content.is_none() {
            return Err(error("content_missing", "此产出没有保存本地正文"));
        }
        self.runs
            .iter()
            .find(|run| run.id == artifact.run_id && run.task_id == task_id)
            .map(|run| run.id.clone())
            .ok_or_else(|| error("source_mismatch", "产出版本不属于所选任务"))
    }

    fn local_artifact_content_bytes(&self) -> usize {
        self.artifacts
            .iter()
            .filter_map(|artifact| artifact.local_content.as_ref().map(|content| content.bytes))
            .fold(0usize, usize::saturating_add)
    }

    /// Return an exact local body reference after checking task, run, artifact,
    /// and snapshot revision ownership. `None` means the artifact is a source
    /// event and must stay on the transient source-reader path.
    pub fn local_artifact_content_ref(
        &self,
        task_id: &str,
        run_id: &str,
        artifact_id: &str,
        revision: u64,
    ) -> Result<Option<LocalArtifactContentRef>, Error> {
        if self.revision != revision {
            return Err(error("conflict", "任务已更新，请刷新后重试"));
        }
        let task = self
            .tasks
            .iter()
            .find(|task| task.id == task_id)
            .ok_or_else(|| error("not_found", "任务不存在"))?;
        if !self
            .runs
            .iter()
            .any(|run| run.id == run_id && run.task_id == task.id)
        {
            return Err(error("source_mismatch", "运行记录不属于所选任务"));
        }
        let artifact = self
            .artifacts
            .iter()
            .find(|artifact| artifact.id == artifact_id)
            .ok_or_else(|| error("not_found", "产出引用不存在"))?;
        if artifact.run_id != run_id {
            return Err(error("source_mismatch", "产出引用不属于所选运行"));
        }
        if artifact.event_ref.is_some() {
            return Ok(None);
        }
        artifact
            .local_content
            .clone()
            .map(Some)
            .ok_or_else(|| error("content_missing", "这条本地记录没有保存正文"))
    }

    fn local_artifact_version_ref(
        &self,
        task_id: &str,
        artifact_id: &str,
    ) -> Result<
        (
            crate::artifact_version::ArtifactVersion,
            LocalArtifactContentRef,
        ),
        Error,
    > {
        self.tasks
            .iter()
            .find(|task| task.id == task_id)
            .ok_or_else(|| error("not_found", "任务不存在"))?;
        let artifact = self
            .artifacts
            .iter()
            .find(|artifact| artifact.id == artifact_id)
            .ok_or_else(|| error("not_found", "产出版本不存在"))?;
        let run = self
            .runs
            .iter()
            .find(|run| run.id == artifact.run_id && run.task_id == task_id)
            .ok_or_else(|| error("source_mismatch", "产出版本不属于所选任务"))?;
        if artifact.event_ref.is_some() {
            return Err(artifact_version_error(
                crate::artifact_version::VersionRefusal::ExternalReadOnlyCannotEvolve,
            ));
        }
        let local_content = artifact
            .local_content
            .clone()
            .ok_or_else(|| error("content_missing", "本地产出版本没有保存正文"))?;
        Ok((
            crate::artifact_version::ArtifactVersion {
                logical_id: artifact
                    .logical_id
                    .clone()
                    .unwrap_or_else(|| artifact.id.clone()),
                entry_id: artifact.id.clone(),
                run_id: run.id.clone(),
                title: artifact.title.clone(),
                revision: artifact.revision,
                event_ref: artifact.event_ref.clone(),
            },
            local_content,
        ))
    }

    fn local_artifact_chain(
        &self,
        task_id: &str,
        artifact_id: &str,
    ) -> Result<crate::artifact_version::ArtifactChain, Error> {
        let (target, _) = self.local_artifact_version_ref(task_id, artifact_id)?;
        let mut entries = self
            .artifacts
            .iter()
            .filter(|artifact| {
                artifact.logical_id.as_deref().unwrap_or(&artifact.id) == target.logical_id
            })
            .map(|artifact| self.local_artifact_version_ref(task_id, &artifact.id))
            .collect::<Result<Vec<_>, _>>()?;
        entries.sort_by_key(|(version, _)| version.revision);
        let mut chain = crate::artifact_version::ArtifactChain::new(&target.logical_id);
        for (version, _) in entries {
            chain.append(version).map_err(artifact_version_error)?;
        }
        Ok(chain)
    }

    /// Resolve two exact local versions after checking task ownership, chain
    /// identity, and the snapshot revision supplied by the UI.
    pub fn local_artifact_version_pair(
        &self,
        task_id: &str,
        from_artifact_id: &str,
        to_artifact_id: &str,
        expected_revision: u64,
    ) -> Result<
        (
            crate::artifact_version::ArtifactVersion,
            LocalArtifactContentRef,
            crate::artifact_version::ArtifactVersion,
            LocalArtifactContentRef,
        ),
        Error,
    > {
        if self.revision != expected_revision {
            return Err(error("conflict", "任务已更新，请刷新后重试"));
        }
        let (from, from_content) = self.local_artifact_version_ref(task_id, from_artifact_id)?;
        let (to, to_content) = self.local_artifact_version_ref(task_id, to_artifact_id)?;
        if from.logical_id != to.logical_id {
            return Err(artifact_version_error(
                crate::artifact_version::VersionRefusal::DifferentLogicalArtifact,
            ));
        }
        Ok((from, from_content, to, to_content))
    }

    fn project_dependency_graph(&self) -> crate::project_deps::DepGraph {
        self.projects
            .iter()
            .map(|project| (project.id.clone(), project.dependencies.clone()))
            .collect()
    }

    /// Late body availability enriches only the exact still-open observed plan gate.
    fn late_plan_attention(&self, o: &crate::task_sources::Observation) -> Option<String> {
        let evidence = o
            .artifact
            .as_ref()
            .filter(|e| e.valid() && e.kind == AttentionKind::PlanApproval)?;
        if o.source.agent_id != "claude"
            || o.status != RunStatus::Waiting
            || o.kind != Some(evidence.kind)
        {
            return None;
        }
        let receipt = self
            .event_receipts
            .iter()
            .find(|r| r.source == o.source && r.fingerprint == o.fingerprint)?;
        // Waiting steps may share a run: only its latest observed receipt owns the open gate.
        let latest = self
            .event_receipts
            .iter()
            .rev()
            .find(|r| r.run_id == receipt.run_id)?;
        if latest.source != o.source || latest.fingerprint != o.fingerprint {
            return None;
        }
        let task = self.tasks.iter().find(|t| {
            t.source.as_ref() == Some(&o.source)
                && t.current_run_id.as_deref() == Some(receipt.run_id.as_str())
        })?;
        self.runs.iter().find(|r| {
            r.id == receipt.run_id
                && r.task_id == task.id
                && r.source.as_ref() == Some(&o.source)
                && r.status == RunStatus::Waiting
        })?;
        self.attentions
            .iter()
            .rev()
            .find(|a| {
                a.run_id == receipt.run_id
                    && a.observed
                    && a.state == AttentionState::Open
                    && a.kind == evidence.kind
                    && a.artifact_id.is_none()
            })
            .map(|a| a.id.clone())
    }
    /// Resolve only stored ownership. Historical records must not inherit the task's newer source.
    pub fn navigation_source(
        &self,
        task_id: &str,
        run_id: Option<&str>,
        artifact_id: Option<&str>,
        revision: u64,
    ) -> Result<&Source, Error> {
        if self.revision != revision {
            return Err(error("conflict", "任务已更新，请刷新后重试"));
        }
        let task = self
            .tasks
            .iter()
            .find(|t| t.id == task_id)
            .ok_or_else(|| error("not_found", "任务不存在"))?;
        let artifact = artifact_id
            .map(|id| {
                self.artifacts
                    .iter()
                    .find(|a| a.id == id)
                    .ok_or_else(|| error("not_found", "产出引用不存在"))
            })
            .transpose()?;
        if let (Some(run), Some(artifact)) = (run_id, artifact) {
            if run != artifact.run_id {
                return Err(error("source_mismatch", "产出引用不属于所选运行"));
            }
        }
        let chosen_run = run_id.or(artifact.map(|a| a.run_id.as_str()));
        if let Some(id) = chosen_run {
            let run = self
                .runs
                .iter()
                .find(|r| r.id == id && r.task_id == task.id)
                .ok_or_else(|| error("source_mismatch", "运行记录不属于所选任务"))?;
            return run
                .source
                .as_ref()
                .ok_or_else(|| error("source_missing", "这次运行未保存来源会话"));
        }
        task.source
            .as_ref()
            .ok_or_else(|| error("source_missing", "任务尚未关联来源"))
    }
    fn check_project(&self, project: Option<&str>) -> Result<(), Error> {
        if project.is_some_and(|p| !self.projects.iter().any(|x| x.id == p)) {
            Err(error("not_found", "项目不存在"))
        } else {
            Ok(())
        }
    }
    fn task_mut(&mut self, task: &str) -> Result<&mut Task, Error> {
        self.tasks
            .iter_mut()
            .find(|t| t.id == task)
            .ok_or_else(|| error("not_found", "任务不存在"))
    }
    fn record(
        &mut self,
        task: &str,
        status: RunStatus,
        kind: Option<AttentionKind>,
        title: Option<String>,
        now: i64,
        observed: bool,
    ) -> Result<Option<String>, Error> {
        let t = self
            .tasks
            .iter()
            .find(|t| t.id == task)
            .ok_or_else(|| error("not_found", "任务不存在"))?;
        if t.archived_ms.is_some() && !observed {
            return Err(error("invalid_input", "请先恢复任务"));
        }
        let run_source = t.source.clone();
        let old_run = t.current_run_id.clone();
        let old = old_run
            .as_ref()
            .and_then(|id| self.runs.iter().find(|r| &r.id == id));
        let start_new = old.is_none()
            || (matches!(status, RunStatus::Running | RunStatus::Waiting)
                && old.is_some_and(|r| {
                    matches!(
                        r.status,
                        RunStatus::Ready
                            | RunStatus::Accepted
                            | RunStatus::Failed
                            | RunStatus::Cancelled
                    )
                }));
        let run_id = if start_new {
            if let Some(old) = &old_run {
                for a in &mut self.attentions {
                    if &a.run_id == old && a.state == AttentionState::Open {
                        a.state = AttentionState::Expired;
                    }
                }
            }
            let run_id = id();
            self.runs.push(Run {
                source: run_source,
                id: run_id.clone(),
                task_id: task.into(),
                status,
                started_ms: now,
                finished_ms: None,
                revision: 1,
            });
            run_id
        } else {
            // **已受理的运行不得被改写回未完成状态**（TASK-03「晚到结果不得复活
            // 已结束的运行」在 RunStatus 上的接线）。受理是终态：一条已经验收过的
            // 运行被后来的迟到观测改回「就绪/失败/取消」，等于把一段已结束的过程
            // 改写成另一种结局，账目与审计都对不上。重复受理（Accepted → Accepted）
            // 允许——那是同一条结论的幂等重报，不是改写。
            if old.is_some_and(|r| r.status == RunStatus::Accepted) && status != RunStatus::Accepted
            {
                return Err(error(
                    "run_already_accepted",
                    "这次运行已被受理，不能被后来的结果改回未完成状态",
                ));
            }
            old_run.unwrap()
        };
        // A new source step expires unresolved questions from the previous step.
        for a in &mut self.attentions {
            if a.run_id == run_id && a.state == AttentionState::Open {
                a.state = AttentionState::Expired;
            }
        }
        let run = self.runs.iter_mut().find(|r| r.id == run_id).unwrap();
        run.status = status;
        run.revision += 1;
        run.finished_ms = matches!(
            status,
            RunStatus::Ready | RunStatus::Failed | RunStatus::Cancelled | RunStatus::Accepted
        )
        .then_some(now);
        if kind.is_some() && !matches!(status, RunStatus::Waiting | RunStatus::Ready) {
            return Err(error("invalid_input", "人工关口需关联等待或结果就绪状态"));
        }
        let artifact_id = title.map(|title| {
            let artifact_id = id();
            self.artifacts.push(Artifact {
                event_ref: None,
                id: artifact_id.clone(),
                logical_id: None,
                run_id: run_id.clone(),
                title,
                revision: 1,
                kind: kind.unwrap_or(AttentionKind::ResultReview),
                local_content: None,
                checklist: crate::artifact_version::Checklist::default(),
            });
            self.enforce_artifact_cap();
            artifact_id
        });
        if let Some(kind) = kind {
            self.attentions.push(Attention {
                observed,
                id: id(),
                run_id: run_id.clone(),
                kind,
                state: AttentionState::Open,
                artifact_id: artifact_id.clone(),
                manual_resolution: false,
            });
        }
        let t = self.task_mut(task)?;
        t.current_run_id = Some(run_id);
        t.updated_ms = now;
        t.revision += 1;
        Ok(artifact_id)
    }
    pub fn validate(&self) -> Result<(), Error> {
        let invalid = || error("read_only", "任务记录版本或关联不完整，原文件已保留");
        if self.schema_version != 1 {
            return Err(invalid());
        }
        let mut ids = std::collections::HashSet::new();
        for id in self
            .projects
            .iter()
            .map(|x| &x.id)
            .chain(self.tasks.iter().map(|x| &x.id))
            .chain(self.runs.iter().map(|x| &x.id))
            .chain(self.attentions.iter().map(|x| &x.id))
            .chain(self.artifacts.iter().map(|x| &x.id))
        {
            if id.is_empty() || !ids.insert(id) {
                return Err(invalid());
            }
        }
        if self
            .projects
            .iter()
            .any(|project| text(&project.name).is_err())
            || crate::project_deps::validate_graph(&self.project_dependency_graph()).is_err()
        {
            return Err(invalid());
        }
        for t in &self.tasks {
            self.check_project(t.project_id.as_deref())
                .map_err(|_| invalid())?;
            if t.current_run_id
                .as_ref()
                .is_some_and(|id| !self.runs.iter().any(|r| &r.id == id && r.task_id == t.id))
            {
                return Err(invalid());
            }
            if text(&t.title).is_err() {
                return Err(invalid());
            }
            if let Some(s) = &t.source {
                s.validate().map_err(|_| invalid())?;
            }
        }
        for r in &self.runs {
            if let Some(source) = &r.source {
                source.validate().map_err(|_| invalid())?;
            }
            if !self.tasks.iter().any(|t| t.id == r.task_id) {
                return Err(invalid());
            }
        }
        for a in &self.attentions {
            if !self.runs.iter().any(|r| r.id == a.run_id)
                || a.artifact_id.as_ref().is_some_and(|id| {
                    !self
                        .artifacts
                        .iter()
                        .any(|x| &x.id == id && x.run_id == a.run_id)
                })
            {
                return Err(invalid());
            }
        }
        let mut version_chains: std::collections::HashMap<
            String,
            (String, std::collections::BTreeSet<u64>),
        > = std::collections::HashMap::new();
        let mut checklist_item_total = 0usize;
        for a in &self.artifacts {
            if a.revision == 0
                || a.logical_id
                    .as_ref()
                    .is_some_and(|logical| logical.is_empty())
            {
                return Err(invalid());
            }
            if a.event_ref.as_ref().is_some_and(|value| {
                value.len() != 64 || !value.bytes().all(|c| c.is_ascii_hexdigit())
            }) {
                return Err(invalid());
            }
            if !self.runs.iter().any(|r| r.id == a.run_id) {
                return Err(invalid());
            }
            if let Some(logical_id) = &a.logical_id {
                if a.event_ref.is_some() || a.local_content.is_none() {
                    return Err(invalid());
                }
                let owner = self
                    .runs
                    .iter()
                    .find(|run| run.id == a.run_id)
                    .map(|run| run.task_id.clone())
                    .ok_or_else(invalid)?;
                let chain = version_chains
                    .entry(logical_id.clone())
                    .or_insert_with(|| (owner.clone(), std::collections::BTreeSet::new()));
                if chain.0 != owner || !chain.1.insert(a.revision) {
                    return Err(invalid());
                }
            } else if a.revision != 1 {
                return Err(invalid());
            }
            match &a.local_content {
                None => {}
                Some(content)
                    if a.event_ref.is_none()
                        && content.sha256.len() == 64
                        && content
                            .sha256
                            .bytes()
                            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
                        && content.bytes <= crate::task_artifacts::MAX_BODY =>
                {
                    // The aggregate bound below is checked after row validation.
                }
                _ => return Err(invalid()),
            }
            if a.checklist.items.len() > MAX_CHECKLIST_ITEMS_PER_ARTIFACT
                || (!a.checklist.is_empty() && (a.event_ref.is_some() || a.local_content.is_none()))
            {
                return Err(invalid());
            }
            checklist_item_total = checklist_item_total
                .checked_add(a.checklist.items.len())
                .filter(|count| *count <= MAX_CHECKLIST_ITEMS_TOTAL)
                .ok_or_else(invalid)?;
            let mut checklist_ids = std::collections::HashSet::new();
            for item in &a.checklist.items {
                if uuid::Uuid::parse_str(&item.id).is_err()
                    || !checklist_ids.insert(&item.id)
                    || item.title.trim() != item.title
                    || crate::private_text::known_private(&item.title)
                    || text(&item.title).is_err()
                    || item.note.as_ref().is_some_and(|note| {
                        note.len() > MAX_CHECKLIST_NOTE_BYTES
                            || crate::private_text::known_private(note)
                    })
                {
                    return Err(invalid());
                }
            }
        }
        for (logical_id, (_, revisions)) in version_chains {
            if !self.artifacts.iter().any(|artifact| {
                artifact.id == logical_id
                    && artifact.logical_id.as_deref() == Some(logical_id.as_str())
                    && artifact.revision == 1
            }) || revisions
                .iter()
                .enumerate()
                .any(|(index, revision)| *revision != index as u64 + 1)
            {
                return Err(invalid());
            }
        }
        if self.local_artifact_content_bytes() > crate::artifact_store::MAX_TOTAL_BYTES {
            return Err(invalid());
        }
        let mut sources = std::collections::HashSet::new();
        for t in &self.tasks {
            if let Some(source) = &t.source {
                if !sources.insert((&source.agent_id, &source.session_id)) {
                    return Err(invalid());
                }
            }
        }
        for p in &self.projects {
            if text(&p.name).is_err() {
                return Err(invalid());
            }
        }
        if self.event_receipts.len() > MAX_RECEIPTS {
            return Err(invalid());
        }
        let mut receipts = std::collections::HashSet::new();
        for r in &self.event_receipts {
            r.source.validate().map_err(|_| invalid())?;
            if r.fingerprint.is_empty()
                || r.fingerprint.len() > 128
                || !receipts.insert((&r.source.agent_id, &r.source.session_id, &r.fingerprint))
            {
                return Err(invalid());
            }
            let Some(run) = self.runs.iter().find(|x| x.id == r.run_id) else {
                return Err(invalid());
            };
            if !self
                .tasks
                .iter()
                .any(|t| t.id == run.task_id && t.source.as_ref() == Some(&r.source))
            {
                return Err(invalid());
            }
        }
        Ok(())
    }
}

/// Keep drag source validation aligned with the task board's run-derived columns.
pub fn board_column(data: &Data, task: &Task) -> crate::task_board_drag::BoardColumn {
    use crate::task_board_drag::BoardColumn;
    let Some(run) = task
        .current_run_id
        .as_ref()
        .and_then(|id| data.runs.iter().find(|run| &run.id == id))
    else {
        return BoardColumn::Queued;
    };
    if data
        .attentions
        .iter()
        .any(|attention| attention.run_id == run.id && attention.state == AttentionState::Open)
        || matches!(run.status, RunStatus::Waiting | RunStatus::Failed)
    {
        return BoardColumn::Attention;
    }
    match run.status {
        RunStatus::Running => BoardColumn::Running,
        RunStatus::Queued => BoardColumn::Queued,
        RunStatus::Waiting
        | RunStatus::Ready
        | RunStatus::Failed
        | RunStatus::Cancelled
        | RunStatus::Accepted => BoardColumn::Finished,
    }
}

fn status_matches_board_column(
    status: RunStatus,
    kind: Option<AttentionKind>,
    target: crate::task_board_drag::BoardColumn,
) -> bool {
    use crate::task_board_drag::BoardColumn;
    match target {
        BoardColumn::Attention => {
            (matches!(
                (status, kind),
                (
                    RunStatus::Waiting,
                    Some(
                        AttentionKind::Answer
                            | AttentionKind::PlanApproval
                            | AttentionKind::Confirmation
                    )
                ) | (RunStatus::Ready, Some(AttentionKind::ResultReview))
            )) || (status == RunStatus::Failed && kind.is_none())
        }
        BoardColumn::Running => status == RunStatus::Running && kind.is_none(),
        BoardColumn::Queued => status == RunStatus::Queued && kind.is_none(),
        BoardColumn::Finished => {
            matches!(status, RunStatus::Ready | RunStatus::Cancelled) && kind.is_none()
        }
        BoardColumn::ProjectGroup => false,
    }
}

fn touch_checklist_owner(
    data: &mut Data,
    task_id: &str,
    run_id: &str,
    now: i64,
) -> Result<(), Error> {
    let run = data
        .runs
        .iter_mut()
        .find(|run| run.id == run_id && run.task_id == task_id)
        .ok_or_else(|| error("source_mismatch", "产出版本运行记录已不存在"))?;
    run.revision = run
        .revision
        .checked_add(1)
        .ok_or_else(|| error("capacity", "运行版本已达上限"))?;

    let task = data.task_mut(task_id)?;
    task.revision = task
        .revision
        .checked_add(1)
        .ok_or_else(|| error("capacity", "任务版本已达上限"))?;
    task.updated_ms = now;
    Ok(())
}

impl Source {
    fn validate(&self) -> Result<(), Error> {
        if self.agent_id.is_empty()
            || self.agent_id.len() > 80
            || !self
                .agent_id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
            || self.session_id.len() != 64
            || !self.session_id.bytes().all(|c| c.is_ascii_hexdigit())
            || self
                .thread_id
                .as_ref()
                .is_some_and(|t| uuid::Uuid::parse_str(t).is_err())
        {
            return Err(error("invalid_input", "来源标识无效"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture {
        store: Store,
        dir: PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!("agentisland-tasks-{}", id()));
            Self {
                store: Store {
                    path: dir.join("tasks.v1.json"),
                },
                dir,
            }
        }
        fn task(&self) -> Data {
            self.store.create("整理方案", None, 0, 1).unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }
    fn source(agent: &str) -> Source {
        Source {
            agent_id: agent.into(),
            session_id: "a".repeat(64),
            thread_id: None,
        }
    }

    // ── TASK-05：产出的有界留存（派生数据不得饿死用户数据）──────────────────

    /// 直接在 `Data` 上构造一批产出与引用，跑上限执行。
    fn artifact_hoard(count: usize, referenced_every: usize) -> Data {
        let mut data = Data::default();
        for index in 0..count {
            let artifact_id = format!("artifact-{index}");
            let run_id = format!("run-{index}");
            data.runs.push(Run {
                id: run_id.clone(),
                task_id: "task-1".into(),
                status: RunStatus::Ready,
                started_ms: index as i64,
                finished_ms: None,
                revision: 1,
                source: None,
            });
            data.artifacts.push(Artifact {
                event_ref: None,
                id: artifact_id.clone(),
                logical_id: None,
                run_id,
                title: format!("产出 {index}"),
                revision: 1,
                kind: AttentionKind::ResultReview,
                local_content: None,
                checklist: crate::artifact_version::Checklist::default(),
            });
            if index % referenced_every.max(1) == 0 {
                data.attentions.push(Attention {
                    observed: true,
                    id: format!("attention-{index}"),
                    run_id: format!("run-{index}"),
                    kind: AttentionKind::ResultReview,
                    state: AttentionState::Resolved,
                    artifact_id: Some(artifact_id),
                    manual_resolution: true,
                });
            }
        }
        data
    }

    /// 超限时从**最旧的未引用行**开始淘汰、计数可见；引用中的行永远保留。
    #[test]
    fn artifact_retention_evicts_oldest_unreferenced_and_counts() {
        let mut data = artifact_hoard(MAX_ARTIFACTS + 10, 500);
        // 夹具里 0/500/1000/1500/2000 被引用；超限 11 行要从未引用的最旧行淘汰。
        let referenced_before: Vec<String> = data
            .attentions
            .iter()
            .filter_map(|a| a.artifact_id.clone())
            .collect();
        data.enforce_artifact_cap();
        assert_eq!(data.artifacts.len(), MAX_ARTIFACTS, "容量应精确落回上限");
        assert_eq!(data.evicted_artifacts, 10, "淘汰数必须如实计数");
        // 引用中的行一个不少——链接是用户回到来源内容的路。
        for id in &referenced_before {
            assert!(
                data.artifacts.iter().any(|a| &a.id == id),
                "被引用的产出 {id} 不得被淘汰"
            );
        }
        // 被淘汰的是最旧的未引用行：artifact-1..=artifact-10 里未引用的先走。
        assert!(
            !data.artifacts.iter().any(|a| a.id == "artifact-2"),
            "最旧的未引用行应先被淘汰"
        );
        assert!(
            data.artifacts.iter().any(|a| a.id == "artifact-2009"),
            "最新的行必须保留"
        );
    }

    /// 全部被引用时**一个都不淘汰**：宁可继续增长（`MAX_BYTES` 仍兜底），
    /// 也不切断用户还指着的那条链接。计数不动。
    #[test]
    fn artifact_retention_never_evicts_a_referenced_row() {
        let mut data = artifact_hoard(MAX_ARTIFACTS + 5, 1);
        data.enforce_artifact_cap();
        assert_eq!(
            data.artifacts.len(),
            MAX_ARTIFACTS + 5,
            "全被引用时不得淘汰任何行"
        );
        assert_eq!(data.evicted_artifacts, 0, "计数不得虚增");
    }

    #[test]
    fn artifact_version_retention_never_splits_a_local_version_chain() {
        let mut data = artifact_hoard(MAX_ARTIFACTS + 10, 500);
        let first_version = data
            .artifacts
            .iter_mut()
            .find(|artifact| artifact.id == "artifact-2")
            .unwrap();
        first_version.logical_id = Some(first_version.id.clone());
        first_version.local_content = Some(LocalArtifactContentRef {
            sha256: "a".repeat(64),
            bytes: 1,
        });
        data.enforce_artifact_cap();
        assert!(
            data.artifacts
                .iter()
                .any(|artifact| artifact.id == "artifact-2"),
            "未被 attention 引用的版本链根也必须保留"
        );
        assert_eq!(data.artifacts.len(), MAX_ARTIFACTS);
        assert_eq!(data.evicted_artifacts, 10);
    }

    /// 旧文件（没有 `evicted_artifacts` 字段）必须还能读进来：
    /// `#[serde(default)]` 的存在理由就是这条兼容。
    #[test]
    fn old_task_files_without_the_eviction_counter_still_load() {
        let mut data = artifact_hoard(1, 1);
        data.revision = 7;
        let mut value = serde_json::to_value(&data).unwrap();
        value.as_object_mut().unwrap().remove("evicted_artifacts");
        value["artifacts"][0]
            .as_object_mut()
            .unwrap()
            .remove("checklist");
        let loaded: Data = serde_json::from_value(value).unwrap();
        assert_eq!(loaded.revision, 7);
        assert_eq!(loaded.evicted_artifacts, 0, "缺字段按 0 起步，不炸不猜");
        assert!(
            loaded.artifacts[0].checklist.is_empty(),
            "旧产物缺清单字段时按空清单加载"
        );
    }
    #[test]
    fn historical_navigation_uses_run_snapshot_and_never_falls_back() {
        let f = Fixture::new();
        let mut d = f.task();
        let task = d.tasks[0].id.clone();
        d.record(
            &task,
            RunStatus::Ready,
            None,
            Some("本地产出".into()),
            2,
            false,
        )
        .unwrap();
        let local_run = d.runs[0].id.clone();
        let local_artifact = d.artifacts[0].id.clone();
        let old = Source {
            thread_id: Some("019c6e27-e55b-73d1-87d8-4e01f1f75043".into()),
            ..source("codex")
        };
        d.tasks[0].source = Some(old.clone());
        d.record(&task, RunStatus::Running, None, None, 3, false)
            .unwrap();
        let historical = d.runs[1].id.clone();
        let newer = Source {
            session_id: "b".repeat(64),
            thread_id: Some("019c7714-3b77-74d1-9866-e1f484aae2ab".into()),
            ..source("codex")
        };
        d.tasks[0].source = Some(newer.clone());
        assert_eq!(
            d.navigation_source(&task, None, None, d.revision).unwrap(),
            &newer
        );
        assert_eq!(
            d.navigation_source(&task, Some(&historical), None, d.revision)
                .unwrap(),
            &old
        );
        assert_eq!(
            d.navigation_source(&task, Some(&local_run), None, d.revision)
                .unwrap_err()
                .code,
            "source_missing"
        );
        assert_eq!(
            d.navigation_source(&task, None, Some(&local_artifact), d.revision)
                .unwrap_err()
                .code,
            "source_missing"
        );
    }
    #[test]
    fn navigation_rejects_cross_task_run_artifact_and_stale_revision() {
        let f = Fixture::new();
        let mut d = f.task();
        let first = d.tasks[0].id.clone();
        d.record(
            &first,
            RunStatus::Ready,
            None,
            Some("产出".into()),
            2,
            false,
        )
        .unwrap();
        let run = d.runs[0].id.clone();
        let artifact = d.artifacts[0].id.clone();
        let second = id();
        let mut task = d.tasks[0].clone();
        task.id = second.clone();
        task.current_run_id = None;
        d.tasks.push(task);
        assert_eq!(
            d.navigation_source(&second, Some(&run), None, d.revision)
                .unwrap_err()
                .code,
            "source_mismatch"
        );
        assert_eq!(
            d.navigation_source(&second, None, Some(&artifact), d.revision)
                .unwrap_err()
                .code,
            "source_mismatch"
        );
        assert_eq!(
            d.navigation_source(&first, Some("another"), Some(&artifact), d.revision)
                .unwrap_err()
                .code,
            "source_mismatch"
        );
        assert_eq!(
            d.navigation_source(&first, None, Some("missing"), d.revision)
                .unwrap_err()
                .code,
            "not_found"
        );
        assert_eq!(
            d.navigation_source(&first, None, None, d.revision + 1)
                .unwrap_err()
                .code,
            "conflict"
        );
    }
    #[test]
    fn artifact_navigation_follows_its_own_run_without_mutating_records() {
        let f = Fixture::new();
        let mut d = f.task();
        let task = d.tasks[0].id.clone();
        let src = source("traework");
        d.tasks[0].source = Some(src.clone());
        d.record(
            &task,
            RunStatus::Ready,
            Some(AttentionKind::ResultReview),
            Some("结果引用".into()),
            2,
            false,
        )
        .unwrap();
        let before = serde_json::to_vec(&d).unwrap();
        assert_eq!(
            d.navigation_source(&task, None, Some(&d.artifacts[0].id), d.revision)
                .unwrap(),
            &src
        );
        assert_eq!(serde_json::to_vec(&d).unwrap(), before);
        assert_eq!(d.attentions[0].state, AttentionState::Open);
    }
    #[test]
    fn observations_require_a_binding_and_are_namespaced_and_idempotent() {
        let f = Fixture::new();
        assert!(f
            .store
            .observe(
                source("codex"),
                "event1".into(),
                RunStatus::Running,
                None,
                1
            )
            .unwrap()
            .is_none());
        assert!(!f.store.path.exists());
        let d = f.task();
        let tid = d.tasks[0].id.clone();
        let d = f.store.link(&tid, source("codex"), d.revision, 2).unwrap();
        let d = f.store.create("第二个来源", None, d.revision, 3).unwrap();
        f.store
            .link(&d.tasks[1].id, source("claude"), d.revision, 4)
            .unwrap();
        let d = f
            .store
            .observe(
                source("codex"),
                "event1".into(),
                RunStatus::Running,
                None,
                5,
            )
            .unwrap()
            .unwrap();
        assert!(f
            .store
            .observe(
                source("codex"),
                "event1".into(),
                RunStatus::Running,
                None,
                6
            )
            .unwrap()
            .is_none());
        assert_eq!(f.store.load().unwrap().revision, d.revision);
        let d = f
            .store
            .observe(
                source("claude"),
                "event1".into(),
                RunStatus::Running,
                None,
                7,
            )
            .unwrap()
            .unwrap();
        assert_eq!(d.runs.len(), 2);
        assert_ne!(d.runs[0].task_id, d.runs[1].task_id);
    }
    #[test]
    fn rerun_preserves_history_and_expires_old_gate() {
        let f = Fixture::new();
        let d = f.task();
        let tid = d.tasks[0].id.clone();
        let d = f
            .store
            .record(
                &tid,
                RunStatus::Ready,
                Some(AttentionKind::ResultReview),
                Some("结果引用".into()),
                d.revision,
                2,
            )
            .unwrap();
        let old = d.runs[0].id.clone();
        let gate = d.attentions[0].id.clone();
        let d = f
            .store
            .record(&tid, RunStatus::Running, None, None, d.revision, 3)
            .unwrap();
        assert_eq!(d.runs.len(), 2);
        assert_eq!(d.runs[0].status, RunStatus::Ready);
        assert_eq!(d.artifacts.len(), 1);
        assert_eq!(d.attentions[0].state, AttentionState::Expired);
        assert_eq!(
            f.store
                .handled(&tid, &old, &gate, d.revision, 4)
                .unwrap_err()
                .code,
            "source_expired"
        );
    }
    #[test]
    fn local_handling_never_approves_or_starts_the_agent() {
        let f = Fixture::new();
        let d = f.task();
        let tid = d.tasks[0].id.clone();
        let d = f
            .store
            .record(
                &tid,
                RunStatus::Waiting,
                Some(AttentionKind::PlanApproval),
                None,
                d.revision,
                2,
            )
            .unwrap();
        let d = f
            .store
            .handled(&tid, &d.runs[0].id, &d.attentions[0].id, d.revision, 3)
            .unwrap();
        assert_eq!(d.runs[0].status, RunStatus::Queued);
        assert!(d.attentions[0].manual_resolution);
        assert_eq!(
            f.store
                .record(&tid, RunStatus::Accepted, None, None, d.revision, 4)
                .unwrap_err()
                .code,
            "invalid_input"
        );
    }
    #[test]
    fn source_binding_is_unique_and_cannot_replace_run_history() {
        let f = Fixture::new();
        let d = f.task();
        let tid = d.tasks[0].id.clone();
        let d = f.store.link(&tid, source("codex"), d.revision, 2).unwrap();
        let d = f.store.create("另一个任务", None, d.revision, 3).unwrap();
        assert_eq!(
            f.store
                .link(&d.tasks[1].id, source("codex"), d.revision, 4)
                .unwrap_err()
                .code,
            "source_in_use"
        );
        let d = f
            .store
            .record(&tid, RunStatus::Running, None, None, d.revision, 5)
            .unwrap();
        assert!(f.store.link(&tid, source("claude"), d.revision, 6).is_err());
    }
    #[test]
    fn binding_after_local_history_keeps_that_history_local() {
        let f = Fixture::new();
        let d = f.task();
        let tid = d.tasks[0].id.clone();
        let d = f
            .store
            .record(
                &tid,
                RunStatus::Waiting,
                Some(AttentionKind::Answer),
                None,
                d.revision,
                2,
            )
            .unwrap();
        let d = f.store.link(&tid, source("codex"), d.revision, 3).unwrap();
        assert!(d.tasks[0].current_run_id.is_none());
        assert!(d.runs[0].source.is_none());
        assert_eq!(d.attentions[0].state, AttentionState::Expired);
        let o = crate::task_sources::Observation {
            source: source("codex"),
            fingerprint: "fresh".into(),
            status: RunStatus::Running,
            kind: None,
            artifact: None,
        };
        let d = f.store.sync(&[o.clone(), o.clone()], 4).unwrap().unwrap();
        assert_eq!(d.runs.len(), 2);
        assert_eq!(d.event_receipts.len(), 1);
        assert_eq!(d.runs[1].source, Some(source("codex")));
        assert!(f.store.sync(&[o], 5).unwrap().is_none());
    }
    #[test]
    fn new_observed_question_after_a_terminal_run_preserves_the_old_run() {
        let f = Fixture::new();
        let d = f.task();
        let task = d.tasks[0].id.clone();
        let d = f.store.link(&task, source("codex"), d.revision, 1).unwrap();
        let d = f
            .store
            .observe(
                source("codex"),
                "completed-one".into(),
                RunStatus::Ready,
                None,
                2,
            )
            .unwrap()
            .unwrap();
        let old = d.runs[0].id.clone();
        let d = f
            .store
            .observe(
                source("codex"),
                "question-two".into(),
                RunStatus::Waiting,
                Some(AttentionKind::Answer),
                3,
            )
            .unwrap()
            .unwrap();
        assert_eq!(d.runs.len(), 2);
        assert_eq!(d.runs[0].id, old);
        assert_eq!(d.runs[0].status, RunStatus::Ready);
        assert_eq!(d.runs[0].finished_ms, Some(2));
        assert_ne!(d.tasks[0].current_run_id.as_deref(), Some(old.as_str()));
        assert_eq!(d.attentions[0].run_id, d.runs[1].id);
        assert_eq!(d.attentions[0].state, AttentionState::Open);
        assert!(f
            .store
            .observe(
                source("codex"),
                "question-two".into(),
                RunStatus::Waiting,
                Some(AttentionKind::Answer),
                4
            )
            .unwrap()
            .is_none());
        assert_eq!(f.store.load().unwrap().runs.len(), 2);
        d.validate().unwrap();
    }
    #[test]
    fn ready_result_reference_does_not_invent_an_acceptance_gate() {
        let f = Fixture::new();
        let d = f.task();
        let tid = d.tasks[0].id.clone();
        let d = f
            .store
            .record(
                &tid,
                RunStatus::Ready,
                None,
                Some("本地结果引用".into()),
                d.revision,
                2,
            )
            .unwrap();
        assert_eq!(d.artifacts.len(), 1);
        assert!(d.attentions.is_empty());
        assert_eq!(d.runs[0].status, RunStatus::Ready);
        let bytes = fs::read(&f.store.path).unwrap();
        assert!(f
            .store
            .record(
                &tid,
                RunStatus::Running,
                None,
                Some("不可丢弃的引用".into()),
                d.revision,
                3
            )
            .is_err());
        assert_eq!(fs::read(&f.store.path).unwrap(), bytes);
    }
    /// 写侧夹具：沙箱目录 + 指向其中 tasks.v1.json 的 Store，只碰合成数据。
    fn store_in(tag: &str) -> (crate::testutil::Sandbox, Store) {
        let sandbox = crate::testutil::Sandbox::new(tag);
        let store = Store {
            path: sandbox.path().join("tasks.v1.json"),
        };
        (sandbox, store)
    }

    #[test]
    fn local_artifact_body_is_persisted_by_hash_and_read_after_store_reopen() {
        let (_sandbox, store) = store_in("tasks-local-artifact-content");
        let created = store.create("本地任务", None, 0, 1).unwrap();
        let task_id = created.tasks[0].id.clone();
        let body = "用户记录的产出正文\n第二行";
        let recorded = store
            .record_with_content(
                &task_id,
                RunStatus::Ready,
                None,
                Some("交付说明".into()),
                Some(body.into()),
                created.revision,
                2,
            )
            .unwrap();
        let artifact = recorded.artifacts.last().unwrap();
        let run_id = artifact.run_id.clone();
        let artifact_id = artifact.id.clone();
        let content_ref = artifact.local_content.as_ref().unwrap();
        assert_eq!(content_ref.bytes, body.len());
        let digest = content_ref.sha256.as_str();
        assert_eq!(digest.len(), 64);

        let task_bytes = fs::read(&store.path).unwrap();
        let task_json = String::from_utf8(task_bytes).unwrap();
        assert!(!task_json.contains(body), "正文不得进入 tasks JSON");
        assert!(task_json.contains(digest), "任务元数据要保存正文摘要");

        let second_task = store
            .create("另一条任务", None, recorded.revision, 3)
            .unwrap();
        let second_task_id = second_task.tasks.last().unwrap().id.clone();
        let second_recorded = store
            .record(
                &second_task_id,
                RunStatus::Ready,
                None,
                Some("另一条产出".into()),
                second_task.revision,
                4,
            )
            .unwrap();
        let second_run_id = second_recorded
            .runs
            .iter()
            .find(|run| run.task_id == second_task_id)
            .unwrap()
            .id
            .clone();

        let reopened = Store {
            path: store.path.clone(),
        };
        let loaded = reopened.load().unwrap();
        let body_ref = loaded
            .local_artifact_content_ref(&task_id, &run_id, &artifact_id, second_recorded.revision)
            .unwrap()
            .unwrap();
        let bytes = reopened
            .artifact_blob_store()
            .unwrap()
            .read(&body_ref.sha256, body_ref.bytes)
            .unwrap();
        assert_eq!(String::from_utf8(bytes).unwrap(), body);
        assert_eq!(
            loaded
                .local_artifact_content_ref(
                    &second_task_id,
                    &run_id,
                    &artifact_id,
                    second_recorded.revision,
                )
                .unwrap_err()
                .code,
            "source_mismatch"
        );
        assert_eq!(
            loaded
                .local_artifact_content_ref(
                    &task_id,
                    &second_run_id,
                    &artifact_id,
                    second_recorded.revision,
                )
                .unwrap_err()
                .code,
            "source_mismatch"
        );
        assert_eq!(
            loaded
                .local_artifact_content_ref(
                    &task_id,
                    &run_id,
                    "missing-artifact",
                    second_recorded.revision,
                )
                .unwrap_err()
                .code,
            "not_found"
        );
        assert_eq!(
            loaded
                .local_artifact_content_ref(
                    &task_id,
                    &run_id,
                    &artifact_id,
                    second_recorded.revision - 1,
                )
                .unwrap_err()
                .code,
            "conflict"
        );
    }

    #[test]
    fn local_artifact_versions_append_compare_and_restore_without_rewriting_history() {
        let (_sandbox, store) = store_in("tasks-local-artifact-versions");
        let created = store.create("版本任务", None, 0, 1).unwrap();
        let task_id = created.tasks[0].id.clone();
        let first_body = "保留\n旧方案\n结尾";
        let first = store
            .record_with_content(
                &task_id,
                RunStatus::Ready,
                None,
                Some("方案".into()),
                Some(first_body.into()),
                created.revision,
                2,
            )
            .unwrap();
        let first_artifact_id = first.artifacts.last().unwrap().id.clone();
        let next_run = store
            .record(&task_id, RunStatus::Running, None, None, first.revision, 3)
            .unwrap();
        let second_body = "保留\n新方案\n结尾";
        let second = store
            .record_local_artifact_version(
                &task_id,
                &first_artifact_id,
                "方案修订".into(),
                second_body.into(),
                next_run.revision,
                4,
            )
            .unwrap();
        let chain: Vec<_> = second
            .artifacts
            .iter()
            .filter(|artifact| artifact.logical_id.as_deref() == Some(first_artifact_id.as_str()))
            .collect();
        assert_eq!(chain.len(), 2, "两个物理版本都应保留");
        assert_eq!(chain[0].id, first_artifact_id);
        assert_eq!(chain[0].revision, 1);
        assert_eq!(chain[1].revision, 2);
        assert_ne!(chain[0].run_id, chain[1].run_id, "新版本绑定当前任务运行");

        let (from, from_ref, to, to_ref) = second
            .local_artifact_version_pair(&task_id, &chain[0].id, &chain[1].id, second.revision)
            .unwrap();
        let blobs = store.artifact_blob_store().unwrap();
        let from_body =
            String::from_utf8(blobs.read(&from_ref.sha256, from_ref.bytes).unwrap()).unwrap();
        let to_body = String::from_utf8(blobs.read(&to_ref.sha256, to_ref.bytes).unwrap()).unwrap();
        let diff =
            crate::artifact_version::compare_content(&from, &to, &from_body, &to_body).unwrap();
        assert_eq!(diff.from_revision, 1);
        assert_eq!(diff.to_revision, 2);
        assert!(diff.title_changed);
        assert!(diff.cross_run);
        assert!(diff.content_diff.contains("-旧方案\n"));
        assert!(diff.content_diff.contains("+新方案\n"));

        let restored = store
            .restore_local_artifact_version(&task_id, &first_artifact_id, second.revision, 5)
            .unwrap();
        let versions: Vec<_> = restored
            .artifacts
            .iter()
            .filter(|artifact| artifact.logical_id.as_deref() == Some(first_artifact_id.as_str()))
            .collect();
        assert_eq!(versions.len(), 3, "恢复产生新版本，不改写旧版本");
        assert_eq!(
            versions
                .iter()
                .map(|item| item.revision)
                .collect::<Vec<_>>(),
            [1, 2, 3]
        );
        assert_eq!(versions[0].title, "方案");
        assert_eq!(versions[1].title, "方案修订");
        assert_eq!(versions[2].title, "方案");
        let restored_ref = versions[2].local_content.as_ref().unwrap();
        let restored_body = blobs
            .read(&restored_ref.sha256, restored_ref.bytes)
            .unwrap();
        assert_eq!(String::from_utf8(restored_body).unwrap(), first_body);
        restored.validate().unwrap();
        let reopened = Store {
            path: store.path.clone(),
        };
        let persisted = reopened.load().unwrap();
        assert_eq!(
            persisted
                .artifacts
                .iter()
                .filter(|artifact| {
                    artifact.logical_id.as_deref() == Some(first_artifact_id.as_str())
                })
                .map(|artifact| artifact.revision)
                .collect::<Vec<_>>(),
            [1, 2, 3]
        );
    }

    #[test]
    fn local_artifact_checklists_are_scoped_to_each_persisted_version() {
        let (_sandbox, store) = store_in("tasks-local-artifact-checklists");
        let created = store.create("验证清单任务", None, 0, 1).unwrap();
        let task_id = created.tasks[0].id.clone();
        let first = store
            .record_with_content(
                &task_id,
                RunStatus::Ready,
                None,
                Some("初版".into()),
                Some("初版正文".into()),
                created.revision,
                2,
            )
            .unwrap();
        let first_artifact = first.artifacts.last().unwrap().id.clone();
        let first_run = first.artifacts.last().unwrap().run_id.clone();
        let first_run_revision = first
            .runs
            .iter()
            .find(|run| run.id == first_run)
            .unwrap()
            .revision;
        let first_task_revision = first.tasks[0].revision;
        assert!(!first.artifacts.last().unwrap().checklist.all_passed());

        let added = store
            .add_artifact_checklist_item(
                &task_id,
                &first_artifact,
                "  可读回正文  ",
                first.revision,
                3,
            )
            .unwrap();
        let original = added
            .artifacts
            .iter()
            .find(|item| item.id == first_artifact)
            .unwrap();
        assert_eq!(original.checklist.items.len(), 1);
        assert_eq!(original.checklist.items[0].title, "可读回正文");
        assert_eq!(
            original.checklist.items[0].status,
            crate::artifact_version::CheckStatus::Pending
        );
        assert!(!original.checklist.all_passed(), "待验项不能算通过");
        assert_eq!(
            added
                .runs
                .iter()
                .find(|run| run.id == first_run)
                .unwrap()
                .revision,
            first_run_revision + 1
        );
        assert_eq!(added.tasks[0].revision, first_task_revision + 1);
        let item_id = original.checklist.items[0].id.clone();
        let before_stale_status = fs::read(&store.path).unwrap();
        assert_eq!(
            store
                .mark_artifact_checklist_item(
                    &task_id,
                    &first_artifact,
                    &item_id,
                    crate::artifact_version::CheckStatus::Passed,
                    first.revision,
                    4,
                )
                .unwrap_err()
                .code,
            "stale_revision"
        );
        assert_eq!(fs::read(&store.path).unwrap(), before_stale_status);

        let passed = store
            .mark_artifact_checklist_item(
                &task_id,
                &first_artifact,
                &item_id,
                crate::artifact_version::CheckStatus::Passed,
                added.revision,
                4,
            )
            .unwrap();
        assert!(passed
            .artifacts
            .iter()
            .find(|item| item.id == first_artifact)
            .unwrap()
            .checklist
            .all_passed());

        let next = store
            .record_local_artifact_version(
                &task_id,
                &first_artifact,
                "第二版".into(),
                "第二版正文".into(),
                passed.revision,
                5,
            )
            .unwrap();
        let second_artifact = next.artifacts.last().unwrap();
        assert_eq!(second_artifact.revision, 2);
        assert!(
            second_artifact.checklist.is_empty(),
            "追加版本必须有全新空清单"
        );
        assert!(next
            .artifacts
            .iter()
            .find(|item| item.id == first_artifact)
            .unwrap()
            .checklist
            .all_passed());

        let failed = store
            .mark_artifact_checklist_item(
                &task_id,
                &first_artifact,
                &item_id,
                crate::artifact_version::CheckStatus::Failed,
                next.revision,
                6,
            )
            .unwrap();
        assert!(!failed
            .artifacts
            .iter()
            .find(|item| item.id == first_artifact)
            .unwrap()
            .checklist
            .all_passed());

        let restored = store
            .restore_local_artifact_version(&task_id, &first_artifact, failed.revision, 7)
            .unwrap();
        let third_artifact = restored.artifacts.last().unwrap();
        assert_eq!(third_artifact.revision, 3);
        assert!(
            third_artifact.checklist.is_empty(),
            "恢复也必须从空清单开始"
        );
        assert_eq!(
            restored
                .artifacts
                .iter()
                .find(|item| item.id == first_artifact)
                .unwrap()
                .checklist
                .items[0]
                .status,
            crate::artifact_version::CheckStatus::Failed,
            "旧版的检查结果仍绑定旧版"
        );
        let reopened = Store {
            path: store.path.clone(),
        };
        let persisted = reopened.load().unwrap();
        assert_eq!(
            persisted
                .artifacts
                .iter()
                .find(|item| item.id == first_artifact)
                .unwrap()
                .checklist
                .items[0]
                .status,
            crate::artifact_version::CheckStatus::Failed,
            "清单与版本状态应在重新打开后保持"
        );
        restored.validate().unwrap();
    }

    #[test]
    fn local_artifact_checklist_rejects_stale_archived_source_and_bodyless_targets() {
        let (_sandbox, store) = store_in("tasks-local-artifact-checklist-refusals");
        let created = store.create("验证边界", None, 0, 1).unwrap();
        let task_id = created.tasks[0].id.clone();
        let empty = store
            .record(
                &task_id,
                RunStatus::Ready,
                None,
                Some("只有标题".into()),
                created.revision,
                2,
            )
            .unwrap();
        let empty_artifact = empty.artifacts.last().unwrap().id.clone();
        assert_eq!(
            store
                .add_artifact_checklist_item(
                    &task_id,
                    &empty_artifact,
                    "正文已保存",
                    empty.revision,
                    3
                )
                .unwrap_err()
                .code,
            "content_missing"
        );

        let with_body = store
            .record_local_artifact_version(
                &task_id,
                &empty_artifact,
                "有正文".into(),
                "可校验正文".into(),
                empty.revision,
                4,
            )
            .unwrap_err();
        assert_eq!(
            with_body.code, "content_missing",
            "无正文的旧产物不能生成内容版本"
        );

        let body_record = store
            .record_with_content(
                &task_id,
                RunStatus::Ready,
                None,
                Some("本地正文".into()),
                Some("正文".into()),
                empty.revision,
                5,
            )
            .unwrap();
        let body_artifact = body_record.artifacts.last().unwrap().id.clone();
        assert_eq!(
            store
                .add_artifact_checklist_item(
                    &task_id,
                    &body_artifact,
                    "陈旧提交",
                    empty.revision,
                    6
                )
                .unwrap_err()
                .code,
            "stale_revision"
        );

        let other_task = store
            .create("其他任务", None, body_record.revision, 6)
            .unwrap();
        let other_task_id = other_task.tasks.last().unwrap().id.clone();
        assert_eq!(
            store
                .add_artifact_checklist_item(
                    &other_task_id,
                    &body_artifact,
                    "跨任务拒绝",
                    other_task.revision,
                    7
                )
                .unwrap_err()
                .code,
            "source_mismatch"
        );

        let before_private_title = fs::read(&store.path).unwrap();
        assert_eq!(
            store
                .add_artifact_checklist_item(
                    &task_id,
                    &body_artifact,
                    "Authorization: Bearer placeholder-token",
                    other_task.revision,
                    8,
                )
                .unwrap_err()
                .code,
            "invalid_input"
        );
        assert_eq!(fs::read(&store.path).unwrap(), before_private_title);

        let checklist_added = store
            .add_artifact_checklist_item(
                &task_id,
                &body_artifact,
                "正文必须保持可读",
                other_task.revision,
                9,
            )
            .unwrap();
        let checklist_item_id = checklist_added
            .artifacts
            .iter()
            .find(|item| item.id == body_artifact)
            .unwrap()
            .checklist
            .items[0]
            .id
            .clone();

        let body_store = store.path.parent().unwrap().join("task-artifact-content");
        fs::remove_dir_all(body_store).unwrap();
        let before_missing_body_write = fs::read(&store.path).unwrap();
        assert_eq!(
            store
                .add_artifact_checklist_item(
                    &task_id,
                    &body_artifact,
                    "正文缺失时拒绝",
                    checklist_added.revision,
                    10
                )
                .unwrap_err()
                .code,
            "content_missing"
        );
        assert_eq!(fs::read(&store.path).unwrap(), before_missing_body_write);
        assert_eq!(
            store
                .mark_artifact_checklist_item(
                    &task_id,
                    &body_artifact,
                    &checklist_item_id,
                    crate::artifact_version::CheckStatus::Passed,
                    checklist_added.revision,
                    11,
                )
                .unwrap_err()
                .code,
            "content_missing"
        );
        assert_eq!(fs::read(&store.path).unwrap(), before_missing_body_write);

        let archived = store
            .archive(&task_id, true, checklist_added.revision, 12)
            .unwrap();
        let archived_bytes = fs::read(&store.path).unwrap();
        assert_eq!(
            store
                .add_artifact_checklist_item(
                    &task_id,
                    &body_artifact,
                    "归档后拒绝",
                    archived.revision,
                    9
                )
                .unwrap_err()
                .code,
            "invalid_input"
        );
        assert_eq!(
            fs::read(&store.path).unwrap(),
            archived_bytes,
            "拒绝写入不得改动原记录"
        );

        let mut source = archived;
        source.tasks[0].archived_ms = None;
        let source_artifact = source
            .artifacts
            .iter_mut()
            .find(|item| item.id == body_artifact)
            .unwrap();
        source_artifact.event_ref = Some("b".repeat(64));
        source_artifact.local_content = None;
        source_artifact.checklist = crate::artifact_version::Checklist::default();
        store.write(&source).unwrap();
        let source_bytes = fs::read(&store.path).unwrap();
        assert_eq!(
            store
                .add_artifact_checklist_item(
                    &task_id,
                    &body_artifact,
                    "来源事件拒绝",
                    source.revision,
                    10
                )
                .unwrap_err()
                .code,
            "external_readonly_cannot_evolve"
        );
        assert_eq!(fs::read(&store.path).unwrap(), source_bytes);
    }

    #[test]
    fn local_artifact_version_writes_reject_stale_archived_and_source_entries() {
        let (_sandbox, store) = store_in("tasks-local-artifact-version-refusals");
        let created = store.create("版本边界", None, 0, 1).unwrap();
        let task_id = created.tasks[0].id.clone();
        let recorded = store
            .record_with_content(
                &task_id,
                RunStatus::Ready,
                None,
                Some("本地方案".into()),
                Some("正文".into()),
                created.revision,
                2,
            )
            .unwrap();
        let artifact_id = recorded.artifacts.last().unwrap().id.clone();
        let path_bytes = fs::read(&store.path).unwrap();
        for (body, expected_code) in [
            (["api", "_key = ", "present"].concat(), "invalid_input"),
            ("x".repeat(crate::task_artifacts::MAX_BODY + 1), "capacity"),
        ] {
            assert_eq!(
                store
                    .record_local_artifact_version(
                        &task_id,
                        &artifact_id,
                        "拒绝的版本".into(),
                        body,
                        recorded.revision,
                        3,
                    )
                    .unwrap_err()
                    .code,
                expected_code
            );
            assert_eq!(fs::read(&store.path).unwrap(), path_bytes);
        }
        assert_eq!(
            store
                .record_local_artifact_version(
                    &task_id,
                    &artifact_id,
                    "陈旧版本".into(),
                    "不应保存".into(),
                    recorded.revision - 1,
                    3,
                )
                .unwrap_err()
                .code,
            "stale_revision"
        );
        assert_eq!(fs::read(&store.path).unwrap(), path_bytes);

        let archived = store.archive(&task_id, true, recorded.revision, 4).unwrap();
        let archived_bytes = fs::read(&store.path).unwrap();
        assert_eq!(
            store
                .record_local_artifact_version(
                    &task_id,
                    &artifact_id,
                    "归档版本".into(),
                    "不应保存".into(),
                    archived.revision,
                    5,
                )
                .unwrap_err()
                .code,
            "invalid_input"
        );
        assert_eq!(fs::read(&store.path).unwrap(), archived_bytes);

        let mut source = archived;
        source.tasks[0].archived_ms = None;
        source.artifacts[0].event_ref = Some("a".repeat(64));
        source.artifacts[0].local_content = None;
        store.write(&source).unwrap();
        let source_bytes = fs::read(&store.path).unwrap();
        let source_snapshot = store.load().unwrap();
        assert_eq!(
            source_snapshot
                .local_artifact_version_pair(
                    &task_id,
                    &artifact_id,
                    &artifact_id,
                    source_snapshot.revision,
                )
                .unwrap_err()
                .code,
            "external_readonly_cannot_evolve"
        );
        assert_eq!(
            store
                .record_local_artifact_version(
                    &task_id,
                    &artifact_id,
                    "来源伪造版本".into(),
                    "不应保存".into(),
                    source.revision,
                    6,
                )
                .unwrap_err()
                .code,
            "external_readonly_cannot_evolve"
        );
        assert_eq!(fs::read(&store.path).unwrap(), source_bytes);
    }

    #[test]
    fn artifact_version_comparison_rejects_other_tasks_and_stale_snapshots() {
        let (_sandbox, store) = store_in("tasks-artifact-version-compare-ownership");
        let first_task = store.create("任务一", None, 0, 1).unwrap();
        let first_task_id = first_task.tasks[0].id.clone();
        let first = store
            .record_with_content(
                &first_task_id,
                RunStatus::Ready,
                None,
                Some("产出".into()),
                Some("第一份".into()),
                first_task.revision,
                2,
            )
            .unwrap();
        let first_artifact_id = first.artifacts.last().unwrap().id.clone();
        let second_task = store.create("任务二", None, first.revision, 3).unwrap();
        let second_task_id = second_task.tasks.last().unwrap().id.clone();
        let second = store
            .record_with_content(
                &second_task_id,
                RunStatus::Ready,
                None,
                Some("产出".into()),
                Some("第二份".into()),
                second_task.revision,
                4,
            )
            .unwrap();
        let second_artifact_id = second.artifacts.last().unwrap().id.clone();
        assert_eq!(
            second
                .local_artifact_version_pair(
                    &first_task_id,
                    &first_artifact_id,
                    &second_artifact_id,
                    second.revision,
                )
                .unwrap_err()
                .code,
            "source_mismatch"
        );
        assert_eq!(
            second
                .local_artifact_version_pair(
                    &first_task_id,
                    &first_artifact_id,
                    &first_artifact_id,
                    second.revision - 1,
                )
                .unwrap_err()
                .code,
            "conflict"
        );
    }

    #[test]
    fn local_artifact_content_refuses_secret_oversized_stale_and_unlabeled_bodies_without_writes() {
        let (_sandbox, store) = store_in("tasks-local-artifact-reject");
        let created = store.create("本地任务", None, 0, 1).unwrap();
        let task_id = created.tasks[0].id.clone();
        let before = fs::read(&store.path).unwrap();
        let secret = ["api", "_key = ", "present"].concat();
        let cases = [
            (
                Some(secret),
                Some("凭据".into()),
                created.revision,
                "invalid_input",
            ),
            (
                Some("x".repeat(crate::task_artifacts::MAX_BODY + 1)),
                Some("过长".into()),
                created.revision,
                "capacity",
            ),
            (
                Some("没有名称的正文".into()),
                None,
                created.revision,
                "invalid_input",
            ),
            (
                Some("陈旧草稿".into()),
                Some("陈旧".into()),
                created.revision - 1,
                "stale_revision",
            ),
        ];
        for (body, title, revision, expected) in cases {
            assert_eq!(
                store
                    .record_with_content(
                        &task_id,
                        RunStatus::Ready,
                        None,
                        title,
                        body,
                        revision,
                        2,
                    )
                    .unwrap_err()
                    .code,
                expected
            );
            assert_eq!(fs::read(&store.path).unwrap(), before);
            assert!(
                !store
                    .path
                    .parent()
                    .unwrap()
                    .join("task-artifact-content")
                    .exists(),
                "拒绝路径不得发布正文 blob"
            );
        }
    }

    #[test]
    fn legacy_local_artifacts_load_but_future_content_fields_remain_read_only() {
        let (_sandbox, store) = store_in("tasks-local-artifact-legacy");
        let created = store.create("旧任务", None, 0, 1).unwrap();
        let recorded = store
            .record(
                &created.tasks[0].id,
                RunStatus::Ready,
                None,
                Some("旧产出".into()),
                created.revision,
                2,
            )
            .unwrap();
        let mut legacy = serde_json::to_value(&recorded).unwrap();
        let artifact = legacy["artifacts"][0].as_object_mut().unwrap();
        artifact.remove("local_content");
        fs::write(&store.path, serde_json::to_vec(&legacy).unwrap()).unwrap();

        let loaded = store.load().unwrap();
        assert_eq!(loaded.artifacts[0].local_content, None);
        loaded.validate().unwrap();

        legacy["artifacts"][0]["local_content"] = serde_json::json!({
            "sha256": "a".repeat(64),
            "bytes": 1,
            "future": true
        });
        let future_bytes = serde_json::to_vec(&legacy).unwrap();
        fs::write(&store.path, &future_bytes).unwrap();
        assert_eq!(store.load().unwrap_err().code, "read_only");
        assert_eq!(fs::read(&store.path).unwrap(), future_bytes);
    }

    #[test]
    fn crud_roundtrip_survives_a_fresh_store_instance() {
        let (sandbox, store) = store_in("tasks-crud");
        let project = store.project_create("看板", 0).unwrap();
        let pid = project.projects[0].id.clone();
        let d = store
            .create("整理方案", None, project.revision, 1_000)
            .unwrap();
        assert_eq!(d.revision, 2, "每次写都把 revision 往前推一格");
        let task = d.tasks[0].id.clone();
        let d = store
            .update(&task, "整理方案 v2", Some(pid.clone()), d.revision, 2_000)
            .unwrap();
        assert_eq!(d.tasks[0].title, "整理方案 v2");
        assert_eq!(d.tasks[0].project_id.as_deref(), Some(pid.as_str()));
        assert_eq!(d.tasks[0].revision, 2);
        let d = store.archive(&task, true, d.revision, 3_000).unwrap();
        assert_eq!(d.tasks[0].archived_ms, Some(3_000));
        let d = store.archive(&task, false, d.revision, 4_000).unwrap();
        assert_eq!(d.tasks[0].archived_ms, None);
        // 换一个 Store 实例读回：落盘字节必须与返回值一致
        let loaded = Store {
            path: store.path.clone(),
        }
        .load()
        .unwrap();
        assert_eq!(
            serde_json::to_vec(&loaded).unwrap(),
            serde_json::to_vec(&d).unwrap()
        );
        assert_eq!(loaded.revision, 5);
        assert!(sandbox.path().join("tasks.v1.json").exists());
    }

    #[test]
    fn project_dependencies_persist_and_enforce_graph_and_delete_guards() {
        let (_sandbox, store) = store_in("tasks-project-dependencies");
        let a = store.project_create("前端", 0).unwrap();
        let aid = a.projects[0].id.clone();
        let b = store.project_create("接口", a.revision).unwrap();
        let bid = b.projects[1].id.clone();
        let c = store.project_create("文档", b.revision).unwrap();
        let cid = c.projects[2].id.clone();

        let linked = store
            .project_dependencies_update(&aid, vec![bid.clone()], c.revision)
            .unwrap();
        assert_eq!(linked.projects[0].dependencies, vec![bid.clone()]);
        let loaded = Store {
            path: store.path.clone(),
        }
        .load()
        .unwrap();
        assert_eq!(loaded.projects[0].dependencies, vec![bid.clone()]);

        let bytes = fs::read(&store.path).unwrap();
        for (project, dependencies, expected) in [
            (&bid, vec![aid.clone()], "dependency_cycle"),
            (&aid, vec!["missing-project".into()], "unknown_dependency"),
            (&aid, vec![aid.clone()], "self_dependency"),
            (&aid, vec![bid.clone(), bid.clone()], "duplicate_dependency"),
        ] {
            let refused = store
                .project_dependencies_update(project, dependencies, linked.revision)
                .unwrap_err();
            assert_eq!(refused.code, expected);
            assert_eq!(
                fs::read(&store.path).unwrap(),
                bytes,
                "拒绝的依赖写入不得改动原文件"
            );
        }
        assert_eq!(
            store
                .project_delete(&bid, linked.revision)
                .unwrap_err()
                .code,
            "still_depended_on"
        );
        assert_eq!(
            fs::read(&store.path).unwrap(),
            bytes,
            "依赖守卫拒绝后原文件不变"
        );

        let task = store
            .create("文档任务", Some(cid.clone()), linked.revision, 10)
            .unwrap();
        assert_eq!(
            store.project_delete(&cid, task.revision).unwrap_err().code,
            "project_in_use"
        );
        let unlinked = store
            .project_dependencies_update(&aid, vec![], task.revision)
            .unwrap();
        let without_b = store.project_delete(&bid, unlinked.revision).unwrap();
        let unassigned = store
            .update(&task.tasks[0].id, "文档任务", None, without_b.revision, 11)
            .unwrap();
        let without_c = store.project_delete(&cid, unassigned.revision).unwrap();
        assert_eq!(without_c.projects.len(), 1);
        without_c.validate().unwrap();
    }

    #[test]
    fn stale_project_dependency_revision_preserves_the_newer_snapshot() {
        let (_sandbox, store) = store_in("tasks-project-dependency-stale");
        let a = store.project_create("甲", 0).unwrap();
        let aid = a.projects[0].id.clone();
        let b = store.project_create("乙", a.revision).unwrap();
        let bid = b.projects[1].id.clone();
        let stale_revision = b.revision;
        let newer = store.project_create("丙", stale_revision).unwrap();
        let bytes = fs::read(&store.path).unwrap();

        assert_eq!(
            store
                .project_dependencies_update(&aid, vec![bid], stale_revision)
                .unwrap_err()
                .code,
            "stale_revision"
        );
        assert_eq!(
            fs::read(&store.path).unwrap(),
            bytes,
            "过期草稿不得覆盖新项目"
        );
        assert_eq!(store.load().unwrap().revision, newer.revision);
        assert!(store.load().unwrap().projects[0].dependencies.is_empty());
    }

    #[test]
    fn project_dependency_store_rejects_a_new_edge_that_makes_nine_levels() {
        let (_sandbox, store) = store_in("tasks-project-dependency-depth");
        let mut revision = 0;
        let mut projects = Vec::new();
        for index in 0..9 {
            let next = store
                .project_create(&format!("项目 {index}"), revision)
                .unwrap();
            revision = next.revision;
            projects.push(next.projects[index].id.clone());
        }
        for index in 0..7 {
            let next = store
                .project_dependencies_update(
                    &projects[index],
                    vec![projects[index + 1].clone()],
                    revision,
                )
                .unwrap();
            revision = next.revision;
        }
        let bytes = fs::read(&store.path).unwrap();
        assert_eq!(
            store
                .project_dependencies_update(&projects[8], vec![projects[0].clone()], revision)
                .unwrap_err()
                .code,
            "dependency_depth_exceeded"
        );
        assert_eq!(
            fs::read(&store.path).unwrap(),
            bytes,
            "深度拒绝不得改写原记录"
        );
    }

    #[test]
    fn legacy_project_dependencies_default_empty() {
        let (_sandbox, store) = store_in("tasks-legacy-project-dependencies");
        let mut legacy = serde_json::to_value(Data::default()).unwrap();
        legacy["projects"] = serde_json::json!([{"id":"legacy-project","name":"旧项目"}]);
        fs::write(&store.path, serde_json::to_vec(&legacy).unwrap()).unwrap();
        let loaded = store.load().unwrap();
        assert!(loaded.projects[0].dependencies.is_empty());
        loaded.validate().unwrap();
        let written = store.project_create("新项目", loaded.revision).unwrap();
        assert!(written.projects[0].dependencies.is_empty());
        assert!(store.load().unwrap().projects[0].dependencies.is_empty());
    }

    #[test]
    fn create_rejects_bad_titles_unknown_projects_and_credential_shaped_text() {
        let (_sandbox, store) = store_in("tasks-create-reject");
        assert!(!store.path.exists(), "夹具起始必须是空目录");
        let oversized = "x".repeat(501);
        let credential = "sk-".to_owned() + &"a".repeat(40);
        let cases: Vec<(String, Option<String>)> = vec![
            ("".into(), None),
            ("  \n\t ".into(), None),
            (oversized, None),
            ("-----BEGIN TEST-----".into(), None),
            (credential, None),
            ("合法标题".into(), Some("ghost-project".to_string())),
        ];
        for (title, project) in cases {
            let err = store.create(&title, project.clone(), 0, 1).unwrap_err();
            assert_eq!(
                err.code,
                if project.is_some() {
                    "not_found"
                } else {
                    "invalid_input"
                },
                "「{title}」必须被拒"
            );
            assert!(!store.path.exists(), "被拒的写入不得留下任何文件");
        }
    }

    #[test]
    fn update_archive_and_record_reject_unknown_ids_without_writing() {
        let (_sandbox, store) = store_in("tasks-unknown-id");
        let d = store.create("第一条", None, 0, 1).unwrap();
        let bytes = fs::read(&store.path).unwrap();
        assert_eq!(
            store
                .update("missing", "新标题", None, d.revision, 2)
                .unwrap_err()
                .code,
            "not_found"
        );
        assert_eq!(
            store
                .archive("missing", true, d.revision, 3)
                .unwrap_err()
                .code,
            "not_found"
        );
        assert_eq!(
            store
                .record("missing", RunStatus::Running, None, None, d.revision, 4)
                .unwrap_err()
                .code,
            "not_found"
        );
        assert_eq!(
            store
                .handled("missing", "run", "gate", d.revision, 5)
                .unwrap_err()
                .code,
            "not_found"
        );
        assert_eq!(fs::read(&store.path).unwrap(), bytes, "未知 ID 必须拒写");
        let loaded = store.load().unwrap();
        assert_eq!(loaded.revision, d.revision, "被拒的写不得推进 revision");
        assert_eq!(loaded.tasks.len(), 1);
    }

    #[test]
    fn stale_revision_is_refused_and_a_refreshed_retry_loses_nothing() {
        let (sandbox, store) = store_in("tasks-stale-revision");
        let other = Store {
            path: store.path.clone(),
        };
        let first = store.create("甲", None, 0, 1).unwrap();
        // 另一个 writer 先提交：revision 前进，第一个 writer 手上的快照就此过期
        let after_other = other.create("乙", None, first.revision, 2).unwrap();
        let bytes = fs::read(&store.path).unwrap();
        assert_eq!(
            store
                .update(&first.tasks[0].id, "甲改名", None, first.revision, 3)
                .unwrap_err()
                .code,
            "stale_revision",
            "过期 revision 必须被 compare-and-save 拒绝"
        );
        assert_eq!(fs::read(&store.path).unwrap(), bytes, "被拒后原字节不动");
        // 刷新后重试：两次提交都不能丢
        let after_retry = store
            .update(&first.tasks[0].id, "甲改名", None, after_other.revision, 4)
            .unwrap();
        assert_eq!(after_retry.revision, after_other.revision + 1);
        assert_eq!(after_retry.tasks.len(), 2);
        assert_eq!(after_retry.tasks[0].title, "甲改名");
        assert_eq!(after_retry.tasks[1].title, "乙");
        // 再拿更旧的快照写一次：仍然拒绝，且不冲掉已成功的提交
        let bytes = fs::read(&store.path).unwrap();
        assert_eq!(
            store
                .archive(&first.tasks[0].id, true, after_other.revision, 5)
                .unwrap_err()
                .code,
            "stale_revision"
        );
        assert_eq!(fs::read(&store.path).unwrap(), bytes);
        let loaded = other.load().unwrap();
        assert_eq!(
            serde_json::to_vec(&loaded).unwrap(),
            serde_json::to_vec(&after_retry).unwrap()
        );
        assert!(sandbox.path().exists());
    }

    #[test]
    fn ownership_isolation_rejects_cross_task_runs_and_unknown_projects() {
        let (sandbox, store) = store_in("tasks-ownership");
        let d = store.create("甲", None, 0, 1).unwrap();
        let a = d.tasks[0].id.clone();
        let d = store.create("乙", None, d.revision, 2).unwrap();
        let b = d.tasks[1].id.clone();
        let d = store
            .record(
                &a,
                RunStatus::Waiting,
                Some(AttentionKind::Answer),
                None,
                d.revision,
                3,
            )
            .unwrap();
        let run_a = d.runs[0].id.clone();
        let gate_a = d.attentions[0].id.clone();
        let bytes = fs::read(&store.path).unwrap();
        // 拿甲的 run 去处理乙：归属不符
        assert_eq!(
            store
                .handled(&b, &run_a, &gate_a, d.revision, 4)
                .unwrap_err()
                .code,
            "source_expired"
        );
        // 拿不属于这条 run 的 gate id：同样拒
        assert_eq!(
            store
                .handled(&a, &run_a, "gate-of-another-run", d.revision, 5)
                .unwrap_err()
                .code,
            "not_found"
        );
        assert_eq!(fs::read(&store.path).unwrap(), bytes, "跨任务访问不改字节");
        // 项目归属：未知 project id 一律拒绝，已存在的才挂得上
        let project = store.project_create("看板", d.revision).unwrap();
        let pid = project.projects[0].id.clone();
        let d = store
            .update(&b, "乙", Some(pid), project.revision, 7)
            .unwrap();
        assert_eq!(
            store
                .update(&b, "乙", Some("ghost".into()), d.revision, 8)
                .unwrap_err()
                .code,
            "not_found"
        );
        // 伪造跨任务关联：validate 与 write 都必须拒，且不留暂存文件
        let mut cross = store.load().unwrap();
        cross.tasks[1].current_run_id = Some(run_a);
        assert_eq!(cross.validate().unwrap_err().code, "read_only");
        let bytes = fs::read(&store.path).unwrap();
        assert!(store.write(&cross).is_err());
        assert_eq!(fs::read(&store.path).unwrap(), bytes);
        let names: Vec<_> = fs::read_dir(sandbox.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names,
            vec!["tasks.v1.json".to_string()],
            "拒写不得留下暂存或留档文件"
        );
    }

    #[test]
    fn archived_task_refuses_local_records_until_it_is_restored() {
        let (_sandbox, store) = store_in("tasks-archived-gate");
        let d = store.create("甲", None, 0, 1).unwrap();
        let tid = d.tasks[0].id.clone();
        let d = store.archive(&tid, true, d.revision, 2).unwrap();
        let bytes = fs::read(&store.path).unwrap();
        assert_eq!(
            store
                .record(&tid, RunStatus::Running, None, None, d.revision, 3)
                .unwrap_err()
                .code,
            "invalid_input",
            "归档任务不接受本地记录"
        );
        assert_eq!(
            store
                .record(&tid, RunStatus::Accepted, None, None, d.revision, 4)
                .unwrap_err()
                .code,
            "invalid_input",
            "本地记录永不批准来源操作"
        );
        assert_eq!(
            store
                .record(
                    &tid,
                    RunStatus::Running,
                    None,
                    Some("   ".into()),
                    d.revision,
                    5
                )
                .unwrap_err()
                .code,
            "invalid_input",
            "空标题必须拒"
        );
        assert_eq!(
            store
                .record(
                    &tid,
                    RunStatus::Running,
                    None,
                    Some("产出".into()),
                    d.revision,
                    6
                )
                .unwrap_err()
                .code,
            "invalid_input",
            "无归属的产出引用必须拒"
        );
        assert_eq!(fs::read(&store.path).unwrap(), bytes, "被拒记录不得落盘");
        let d = store.archive(&tid, false, d.revision, 7).unwrap();
        let d = store
            .record(&tid, RunStatus::Running, None, None, d.revision, 8)
            .unwrap();
        assert_eq!(d.runs.len(), 1);
        assert_eq!(d.tasks[0].archived_ms, None);
    }

    #[test]
    fn running_with_answer_on_an_active_task_preserves_bytes_and_revision() {
        let (_sandbox, store) = store_in("tasks-invalid-running-gate");
        let d = store.create("整理方案", None, 0, 1).unwrap();
        d.validate().unwrap();
        assert_eq!(d.tasks[0].archived_ms, None);
        let bytes = fs::read(&store.path).unwrap();
        assert_eq!(
            store
                .record(
                    &d.tasks[0].id,
                    RunStatus::Running,
                    Some(AttentionKind::Answer),
                    None,
                    d.revision,
                    2,
                )
                .unwrap_err()
                .code,
            "invalid_input"
        );
        assert_eq!(fs::read(&store.path).unwrap(), bytes);
        let loaded = store.load().unwrap();
        assert_eq!(loaded.revision, d.revision);
        assert_eq!(
            serde_json::to_vec(&loaded).unwrap(),
            serde_json::to_vec(&d).unwrap()
        );
    }

    #[test]
    fn max_bytes_boundary_loads_and_one_byte_over_refuses_writes_without_changes() {
        let (sandbox, store) = store_in("tasks-capacity-boundary");
        let d = store.create("保留原任务", None, 0, 1).unwrap();
        let mut bytes = serde_json::to_vec(&d).unwrap();
        assert!(bytes.len() < MAX_BYTES);
        // 合法 JSON 后仅填充合法空白：容量必须是唯一拒绝原因。
        bytes.resize(MAX_BYTES, b' ');
        let boundary: Data = serde_json::from_slice(&bytes).unwrap();
        boundary.validate().unwrap();
        fs::write(&store.path, &bytes).unwrap();
        assert_eq!(fs::metadata(&store.path).unwrap().len(), MAX_BYTES as u64);
        let loaded = store.load().unwrap();
        assert_eq!(
            serde_json::to_vec(&loaded).unwrap(),
            serde_json::to_vec(&d).unwrap()
        );
        assert_eq!(fs::read(&store.path).unwrap(), bytes);

        bytes.push(b'\n');
        assert_eq!(bytes.len(), MAX_BYTES + 1);
        let oversized: Data = serde_json::from_slice(&bytes).unwrap();
        oversized.validate().unwrap();
        fs::write(&store.path, &bytes).unwrap();
        let names_before: std::collections::BTreeSet<_> = fs::read_dir(sandbox.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(store.load().unwrap_err().code, "read_only");
        assert_eq!(
            store
                .create("不得覆盖原任务", None, d.revision, 2)
                .unwrap_err()
                .code,
            "read_only"
        );
        assert_eq!(fs::read(&store.path).unwrap(), bytes);
        let preserved: Data = serde_json::from_slice(&fs::read(&store.path).unwrap()).unwrap();
        assert_eq!(preserved.revision, d.revision);
        let names_after: std::collections::BTreeSet<_> = fs::read_dir(sandbox.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names_after, names_before, "拒写不得留下任何新文件");
    }

    #[test]
    fn corrupt_future_symlink_and_unknown_field_files_are_preserved() {
        let (sandbox, store) = store_in("tasks-corrupt-symlink");
        let variants: Vec<(&str, Vec<u8>)> = vec![
            ("broken", "{ 不是 JSON".as_bytes().to_vec()),
            ("not-an-object", b"[1,2,3]".to_vec()),
            (
                "unknown-fields",
                br#"{"schema_version":1,"revision":0,"projects":[],"tasks":[],"runs":[],"attentions":[],"artifacts":[],"event_receipts":[],"future":true}"#.to_vec(),
            ),
            (
                "future-schema",
                br#"{"schema_version":2,"revision":0,"projects":[],"tasks":[],"runs":[],"attentions":[],"artifacts":[],"event_receipts":[]}"#.to_vec(),
            ),
            (
                "unknown-project-dependency",
                r#"{"schema_version":1,"revision":0,"projects":[{"id":"project","name":"项目","dependencies":["missing"]}],"tasks":[],"runs":[],"attentions":[],"artifacts":[],"event_receipts":[]}"#.as_bytes().to_vec(),
            ),
        ];
        for (label, bytes) in variants {
            fs::write(&store.path, &bytes).unwrap();
            assert_eq!(
                store.create("新任务", None, 0, 1).unwrap_err().code,
                "read_only",
                "{label} 必须拒写"
            );
            assert_eq!(
                &fs::read(&store.path).unwrap(),
                &bytes,
                "{label} 原字节保留"
            );
            assert_eq!(
                fs::read_dir(sandbox.path()).unwrap().count(),
                1,
                "{label} 不得留档或留暂存文件"
            );
        }
        fs::remove_file(&store.path).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let real = sandbox.path().join("real.json");
            fs::write(&real, b"keep me exactly").unwrap();
            symlink(&real, &store.path).unwrap();
            assert_eq!(
                store.create("新任务", None, 0, 1).unwrap_err().code,
                "read_only"
            );
            assert_eq!(fs::read(&real).unwrap(), b"keep me exactly");
            assert!(
                fs::symlink_metadata(&store.path)
                    .unwrap()
                    .file_type()
                    .is_symlink(),
                "符号链接不得被替换成普通文件"
            );
            assert_eq!(fs::read_dir(sandbox.path()).unwrap().count(), 2);
        }
    }

    #[test]
    fn project_dependency_write_failure_leaves_no_staging_file_and_preserves_original() {
        let (sandbox, store) = store_in("tasks-write-failure");
        assert!(!store.path.exists());
        // 目录链路上挡着一个文件：连目录都建不起来 ⇒ 不得留下任何文件
        let blocker = sandbox.path().join("blocker");
        fs::write(&blocker, b"not a directory").unwrap();
        let nested = Store {
            path: blocker.join("sub").join("tasks.v1.json"),
        };
        let directory_names = || -> std::collections::BTreeSet<_> {
            fs::read_dir(sandbox.path())
                .unwrap()
                .map(|entry| entry.unwrap().file_name())
                .collect()
        };
        let names_before = directory_names();
        assert!(nested.create("新任务", None, 0, 1).is_err());
        assert!(!nested.path.exists(), "失败的创建不得留下空快照");
        assert!(!blocker.join("sub").exists());
        assert_eq!(directory_names(), names_before);
        assert_eq!(fs::read(&blocker).unwrap(), b"not a directory");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // Root 绕过目录权限，那种环境由上面的路径失败用例覆盖
            if unsafe { libc::geteuid() } != 0 {
                let d = store.create("甲", None, 0, 1).unwrap();
                let bytes = fs::read(&store.path).unwrap();
                let names_before = directory_names();
                fs::set_permissions(sandbox.path(), fs::Permissions::from_mode(0o500)).unwrap();
                let failed = store.create("乙", None, d.revision, 2);
                fs::set_permissions(sandbox.path(), fs::Permissions::from_mode(0o700)).unwrap();
                assert!(failed.is_err(), "目录不可写必须明确失败");
                assert_eq!(fs::read(&store.path).unwrap(), bytes, "写失败不动原字节");
                assert_eq!(store.load().unwrap().revision, d.revision);
                assert_eq!(
                    directory_names(),
                    names_before,
                    "写失败前后目录全集必须一致，包括原有 blocker"
                );
                assert_eq!(fs::read(&blocker).unwrap(), b"not a directory");

                let first = store.project_create("依赖方", d.revision).unwrap();
                let first_id = first.projects[0].id.clone();
                let second = store.project_create("前置项目", first.revision).unwrap();
                let second_id = second.projects[1].id.clone();
                let bytes = fs::read(&store.path).unwrap();
                let names_before = directory_names();
                fs::set_permissions(sandbox.path(), fs::Permissions::from_mode(0o500)).unwrap();
                let failed =
                    store.project_dependencies_update(&first_id, vec![second_id], second.revision);
                fs::set_permissions(sandbox.path(), fs::Permissions::from_mode(0o700)).unwrap();
                assert_eq!(
                    failed.unwrap_err().code,
                    "io_error",
                    "依赖写失败必须显式返回"
                );
                assert_eq!(
                    fs::read(&store.path).unwrap(),
                    bytes,
                    "依赖写失败不动原字节"
                );
                assert_eq!(store.load().unwrap().revision, second.revision);
                assert_eq!(
                    directory_names(),
                    names_before,
                    "依赖写失败不得留下暂存文件"
                );
            }
        }
    }

    /// TASK-03 接线：受理（Accepted）只走**来源路径**，且受理是终态。
    ///
    /// 此前 `Accepted` 状态在**所有路径上都不可达**（本地 record 刻意拒绝，
    /// observe 的白名单又没列它），于是「来源验收」这条需求没有落地路径。
    /// 本轮补上：可信来源事件（来源绑定 + 指纹 + 幂等回执）可以受理一次运行；
    /// 而受理之后，**迟到的结果不得把它改写回未完成**。
    #[test]
    fn acceptance_comes_from_a_trusted_source_event_and_is_final() {
        let f = Fixture::new();
        let d = f.task();
        let tid = d.tasks[0].id.clone();
        let d = f.store.link(&tid, source("codex"), d.revision, 2).unwrap();
        // ① 可信来源事件可以把运行标为已受理
        let d = f
            .store
            .observe(
                source("codex"),
                "accept-1".into(),
                RunStatus::Accepted,
                None,
                3,
            )
            .unwrap()
            .unwrap();
        assert_eq!(d.runs[0].status, RunStatus::Accepted, "受理必须落到运行上");
        assert!(d.runs[0].finished_ms.is_some(), "受理是终态 ⇒ 有完成时刻");
        // ② 受理后迟到的「就绪」结果**不得改写**它
        let refused = f.store.observe(
            source("codex"),
            "late-ready".into(),
            RunStatus::Ready,
            None,
            4,
        );
        let error = refused.expect_err("已受理的运行不得被晚到结果改写");
        assert_eq!(error.code, "run_already_accepted");
        let after = f.store.load().unwrap();
        assert_eq!(
            after.runs[0].status,
            RunStatus::Accepted,
            "状态必须原样保留"
        );
        // ③ 重复受理（同一条结论的幂等重报）允许
        let again = f
            .store
            .observe(
                source("codex"),
                "accept-2".into(),
                RunStatus::Accepted,
                None,
                5,
            )
            .unwrap()
            .unwrap();
        assert_eq!(again.runs[0].status, RunStatus::Accepted);
        // ④ 本地路径依然永不受理（既有边界不变）
        let local = f
            .store
            .record(&tid, RunStatus::Accepted, None, None, again.revision, 6)
            .unwrap_err();
        assert_eq!(local.code, "invalid_input");
    }
}
