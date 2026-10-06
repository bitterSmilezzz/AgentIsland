//! Local task organization. No task execution, approval forwarding or transcript storage.
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};

const MAX_BYTES: usize = 8 * 1024 * 1024;
const MAX_RECEIPTS: usize = 20_000;
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
    pub run_id: String,
    pub title: String,
    pub revision: u64,
    pub kind: AttentionKind,
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
            event_receipts: vec![],
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
            d.projects.push(Project { id: id(), name });
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
        if status == RunStatus::Accepted {
            return Err(error(
                "invalid_input",
                "来源验收需可信事件；本地处理不会批准工具操作",
            ));
        }
        let title = title.map(|t| text(&t)).transpose()?;
        if title.is_some() && kind.is_none() && status != RunStatus::Ready {
            return Err(error("invalid_input", "产出引用需关联结果就绪或人工关口"));
        }
        self.edit(revision, |d| {
            d.record(task, status, kind, title, now, false)
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
                            run_id: attention.run_id.clone(),
                            title: evidence.title().into(),
                            revision: 1,
                            kind: evidence.kind,
                        });
                        attention.artifact_id = Some(artifact_id);
                        let run = d
                            .runs
                            .iter_mut()
                            .find(|r| r.id == attention.run_id)
                            .unwrap();
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
    ) -> Result<(), Error> {
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
                run_id: run_id.clone(),
                title,
                revision: 1,
                kind: kind.unwrap_or(AttentionKind::ResultReview),
            });
            artifact_id
        });
        if let Some(kind) = kind {
            self.attentions.push(Attention {
                observed,
                id: id(),
                run_id: run_id.clone(),
                kind,
                state: AttentionState::Open,
                artifact_id,
                manual_resolution: false,
            });
        }
        let t = self.task_mut(task)?;
        t.current_run_id = Some(run_id);
        t.updated_ms = now;
        t.revision += 1;
        Ok(())
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
        for a in &self.artifacts {
            if a.event_ref.as_ref().is_some_and(|value| {
                value.len() != 64 || !value.bytes().all(|c| c.is_ascii_hexdigit())
            }) {
                return Err(invalid());
            }
            if !self.runs.iter().any(|r| r.id == a.run_id) {
                return Err(invalid());
            }
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
    fn stale_write_and_invalid_gate_leave_original_bytes_unchanged() {
        let f = Fixture::new();
        let d = f.task();
        let bytes = fs::read(&f.store.path).unwrap();
        assert_eq!(
            f.store
                .update(&d.tasks[0].id, "新标题", None, 0, 2)
                .unwrap_err()
                .code,
            "stale_revision"
        );
        assert!(f
            .store
            .record(
                &d.tasks[0].id,
                RunStatus::Running,
                Some(AttentionKind::Answer),
                None,
                d.revision,
                3
            )
            .is_err());
        assert_eq!(fs::read(&f.store.path).unwrap(), bytes);
    }
    #[test]
    fn corrupt_future_and_oversized_files_are_preserved() {
        let f = Fixture::new();
        fs::create_dir_all(&f.dir).unwrap();
        for bytes in [
            b"broken".to_vec(),
            serde_json::to_vec(&Data {
                schema_version: 99,
                ..Data::default()
            })
            .unwrap(),
            vec![b' '; MAX_BYTES + 1],
        ] {
            fs::write(&f.store.path, &bytes).unwrap();
            assert!(f.store.create("新任务", None, 0, 1).is_err());
            assert_eq!(fs::read(&f.store.path).unwrap(), bytes);
        }
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
    #[test]
    fn io_failure_does_not_create_an_empty_snapshot() {
        let f = Fixture::new();
        fs::write(&f.dir, b"parent is a file").unwrap();
        assert!(f.store.create("新任务", None, 0, 1).is_err());
        assert!(!f.store.path.exists());
        fs::remove_file(&f.dir).unwrap();
    }
}
