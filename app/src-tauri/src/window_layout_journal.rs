//! Durable window geometry receipts. Native handles and window titles never enter this file.
use crate::window_layout::Rect;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fs, io::Read, path::PathBuf};
const CAP: u64 = 512 * 1024;
const MAX_RECORDS: usize = 100;
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Pending,
    Finished,
    Failed,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Applied,
    Failed,
    Skipped,
    Restored,
    Conflict,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Slot {
    pub id: String,
    pub agent_id: String,
    pub before: Rect,
    pub target: Rect,
    pub actual: Option<Rect>,
    pub outcome: Option<Outcome>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceContext {
    pub id: String,
    pub expected_revision: u64,
    pub layout_id: String,
    pub expected_rules_revision: u64,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub id: String,
    pub created_ms: i64,
    pub phase: Phase,
    pub execution_id: Option<String>,
    pub recovery_of: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<WorkspaceContext>,
    pub slots: Vec<Slot>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Data {
    schema_version: u8,
    items: Vec<Record>,
}
#[derive(Clone, Serialize)]
pub struct List {
    pub revision: String,
    pub items: Vec<Record>,
}
/// Ephemeral IDs are used only to match the observed result to its requested slots.
pub struct Input {
    pub window_id: String,
    pub agent_id: String,
    pub before: Rect,
    pub target: Rect,
}
pub struct Ticket {
    record: Record,
    window_ids: Vec<String>,
    revision: String,
}
pub struct Store {
    path: PathBuf,
}
fn id_ok(id: &str) -> bool {
    uuid::Uuid::parse_str(id).is_ok_and(|v| v.to_string() == id)
}
fn validate(data: &Data) -> Result<(), String> {
    if data.schema_version != 1 || data.items.len() > MAX_RECORDS {
        return Err("窗口历史格式或容量不支持，原件保留".into());
    }
    let tools = crate::registry::builtin()
        .into_iter()
        .map(|p| p.id)
        .collect::<std::collections::HashSet<_>>();
    let mut ids = std::collections::HashSet::new();
    for record in &data.items {
        if !id_ok(&record.id)
            || !ids.insert(record.id.clone())
            || record.created_ms < 0
            || record.slots.is_empty()
            || record.slots.len() > 16
            || record.execution_id.as_ref().is_some_and(|id| !id_ok(id))
            || record
                .recovery_of
                .as_ref()
                .is_some_and(|id| !id_ok(id) || id == &record.id)
        {
            return Err("窗口历史记录身份或范围无法核实".into());
        }
        if record.workspace.as_ref().is_some_and(|c| {
            !id_ok(&c.id)
                || !id_ok(&c.layout_id)
                || c.expected_revision > 9_007_199_254_740_991
                || c.expected_rules_revision > 9_007_199_254_740_991
        }) {
            return Err("窗口记录的工作空间身份或版本无效".into());
        }
        let mut slots = std::collections::HashSet::new();
        for slot in &record.slots {
            if !id_ok(&slot.id)
                || !slots.insert(slot.id.clone())
                || !tools.contains(&slot.agent_id)
                || !slot.before.valid()
                || !slot.target.valid()
                || slot.actual.is_some_and(|r| !r.valid())
            {
                return Err("窗口历史中的工具或位置无法核实".into());
            }
            if record.phase == Phase::Pending && (slot.actual.is_some() || slot.outcome.is_some()) {
                return Err("待核对窗口记录不能声明执行结果".into());
            }
            if record.phase == Phase::Finished && slot.outcome.is_none() {
                return Err("窗口结果记录不完整".into());
            }
        }
        if record.phase == Phase::Finished && record.execution_id.is_none() {
            return Err("窗口结果缺少执行身份".into());
        }
        if record.phase != Phase::Finished && record.execution_id.is_some() {
            return Err("窗口历史阶段与执行身份不一致".into());
        }
    }
    Ok(())
}
impl Store {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
    pub fn at_default() -> Self {
        Self::new(crate::settings::config_dir().join("window-operations.v1.json"))
    }
    fn read(&self) -> Result<(Data, String), String> {
        let mut options = fs::OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        }
        let mut file = match options.open(&self.path) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                if fs::symlink_metadata(&self.path).is_ok() {
                    return Err("窗口历史为链接或不可核实，原件保留".into());
                }
                return Ok((
                    Data {
                        schema_version: 1,
                        items: vec![],
                    },
                    "absent".into(),
                ));
            }
            Err(_) => return Err("窗口历史不可读，原件保留".into()),
        };
        let before = file.metadata().map_err(|_| "窗口历史信息不可读")?;
        if !before.is_file() || before.len() > CAP {
            return Err("窗口历史类型或大小不受支持".into());
        }
        if fs::symlink_metadata(&self.path).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err("窗口历史为链接，原件保留".into());
        }
        let mut bytes = vec![];
        (&mut file)
            .take(CAP + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "窗口历史读取失败")?;
        let after = file.metadata().map_err(|_| "窗口历史信息不可读")?;
        if bytes.len() as u64 > CAP
            || before.len() != bytes.len() as u64
            || before.len() != after.len()
            || before.modified().ok() != after.modified().ok()
        {
            return Err("窗口历史在读取期间变化，请重试".into());
        }
        let data: Data =
            serde_json::from_slice(&bytes).map_err(|_| "窗口历史损坏或格式不支持，原件保留")?;
        validate(&data)?;
        Ok((data, format!("{:x}", Sha256::digest(&bytes))))
    }
    pub fn list(&self) -> Result<List, String> {
        let (data, revision) = self.read()?;
        Ok(List {
            revision,
            items: data.items,
        })
    }
    fn write(&self, data: &Data, expected: &str) -> Result<String, String> {
        validate(data)?;
        let bytes = serde_json::to_vec(data).map_err(|_| "窗口历史无法编码")?;
        if bytes.len() as u64 > CAP {
            return Err("窗口历史达到容量上限，原件保留".into());
        }
        if self.read()?.1 != expected {
            return Err("窗口历史已被修改，未覆盖原件".into());
        }
        let parent = self.path.parent().ok_or("窗口历史目录不可用")?;
        fs::create_dir_all(parent).map_err(|_| "窗口历史目录无法创建")?;
        crate::atomicfile::atomic_replace_validated(&self.path, &bytes, |staged| {
            let parsed: Data =
                serde_json::from_slice(&fs::read(staged)?).map_err(std::io::Error::other)?;
            validate(&parsed).map_err(std::io::Error::other)?;
            if self.read().map_err(std::io::Error::other)?.1 != expected {
                return Err(std::io::Error::other("窗口历史已经变化"));
            }
            Ok(())
        })
        .map_err(|_| "窗口历史保存失败，原件保留")?;
        let expected = format!("{:x}", Sha256::digest(&bytes));
        if self.read()?.1 != expected {
            return Err("窗口历史已发布但读回变化，请核对原件".into());
        }
        Ok(expected)
    }
    pub fn remove(&self, id: &str, expected: &str) -> Result<List, String> {
        let (mut data, revision) = self.read()?;
        if revision != expected {
            return Err("历史已变化，请刷新后核对；未删除记录".into());
        }
        let index = data
            .items
            .iter()
            .position(|r| r.id == id)
            .ok_or("此记录已不存在，请刷新")?;
        data.items.remove(index);
        let revision = self.write(&data, &revision)?;
        Ok(List {
            revision,
            items: data.items,
        })
    }
    pub fn begin(
        &self,
        input: &[Input],
        recovery_of: Option<&str>,
        now: i64,
    ) -> Result<Ticket, String> {
        self.begin_checked(input, recovery_of, now, None)
    }
    pub fn begin_checked(
        &self,
        input: &[Input],
        recovery_of: Option<&str>,
        now: i64,
        expected: Option<&str>,
    ) -> Result<Ticket, String> {
        self.begin_scoped(input, recovery_of, now, expected, None)
    }
    pub fn begin_scoped(
        &self,
        input: &[Input],
        recovery_of: Option<&str>,
        now: i64,
        expected: Option<&str>,
        workspace: Option<&WorkspaceContext>,
    ) -> Result<Ticket, String> {
        let (mut data, revision) = self.read()?;
        if expected.is_some_and(|expected| expected != revision) {
            return Err("历史已变化，请重新预览；未移动窗口".into());
        }
        if data.items.len() >= MAX_RECORDS {
            return Err("窗口历史达到100项，请先处理历史；未移动窗口".into());
        }
        if input.is_empty() || input.len() > 16 {
            return Err("请选择1至16个窗口".into());
        }
        if let Some(parent) = recovery_of {
            let record = data
                .items
                .iter()
                .find(|r| r.id == parent)
                .ok_or("原窗口记录已变化，请刷新")?;
            if let Some(context) = workspace {
                if record
                    .workspace
                    .as_ref()
                    .is_none_or(|c| c.id != context.id || c.layout_id != context.layout_id)
                {
                    return Err("历史窗口记录不属于此工作空间布局".into());
                }
            }
        }
        let mut seen = std::collections::HashSet::new();
        for row in input {
            if row.window_id.is_empty() || !seen.insert(&row.window_id) {
                return Err("所选窗口身份重复或缺失".into());
            }
        }
        let record = Record {
            id: uuid::Uuid::new_v4().to_string(),
            created_ms: now,
            phase: Phase::Pending,
            execution_id: None,
            recovery_of: recovery_of.map(str::to_owned),
            workspace: workspace.cloned(),
            slots: input
                .iter()
                .map(|row| Slot {
                    id: uuid::Uuid::new_v4().to_string(),
                    agent_id: row.agent_id.clone(),
                    before: row.before,
                    target: row.target,
                    actual: None,
                    outcome: None,
                })
                .collect(),
        };
        data.items.push(record.clone());
        let revision = self.write(&data, &revision)?;
        Ok(Ticket {
            record,
            window_ids: input.iter().map(|r| r.window_id.clone()).collect(),
            revision,
        })
    }
    pub fn finish(
        &self,
        ticket: &Ticket,
        result: Option<&crate::window_layout_execution::ResultDto>,
    ) -> Result<Record, String> {
        let (mut data, revision) = self.read()?;
        if revision != ticket.revision {
            return Err("窗口操作后历史发生变化，请核对窗口，不重复排列".into());
        }
        let record = data
            .items
            .iter_mut()
            .find(|r| r.id == ticket.record.id)
            .ok_or("窗口意图记录已不可核实")?;
        if record != &ticket.record {
            return Err("窗口意图记录已变化，原件保留".into());
        }
        if let Some(result) = result {
            if !id_ok(&result.operation_id) || result.windows.len() != ticket.window_ids.len() {
                return Err("窗口执行结果身份或范围无法核实".into());
            }
            let mut seen = std::collections::HashSet::new();
            for row in &result.windows {
                let index = ticket
                    .window_ids
                    .iter()
                    .position(|id| id == &row.window_id)
                    .ok_or("执行结果不属于此记录")?;
                if !seen.insert(index) || row.actual_rect.is_some_and(|r| !r.valid()) {
                    return Err("执行结果重复或位置无法核实".into());
                }
                record.slots[index].actual = row.actual_rect;
                record.slots[index].outcome = Some(match row.status {
                    crate::window_layout_execution::Status::Applied => Outcome::Applied,
                    crate::window_layout_execution::Status::Failed => Outcome::Failed,
                    crate::window_layout_execution::Status::Skipped => Outcome::Skipped,
                    crate::window_layout_execution::Status::Restored => Outcome::Restored,
                    crate::window_layout_execution::Status::Conflict => Outcome::Conflict,
                });
            }
            record.phase = Phase::Finished;
            record.execution_id = Some(result.operation_id.clone());
        } else {
            record.phase = Phase::Failed;
        }
        let result = record.clone();
        self.write(&data, &revision)?;
        Ok(result)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn input() -> Input {
        Input {
            window_id: "ephemeral-window".into(),
            agent_id: "codex".into(),
            before: Rect {
                x: 20.,
                y: 30.,
                width: 500.,
                height: 400.,
            },
            target: Rect {
                x: 0.,
                y: 0.,
                width: 800.,
                height: 600.,
            },
        }
    }
    fn result() -> crate::window_layout_execution::ResultDto {
        crate::window_layout_execution::ResultDto {
            operation_id: uuid::Uuid::new_v4().to_string(),
            windows: vec![crate::window_layout_execution::Row {
                window_id: input().window_id,
                status: crate::window_layout_execution::Status::Applied,
                reason: None,
                actual_rect: Some(input().target),
            }],
            undo_available: true,
            record_id: None,
            record_warning: None,
        }
    }
    #[test]
    fn intent_survives_restart_and_complete_result_contains_no_native_identity() {
        let s = crate::testutil::Sandbox::new("window-history");
        let path = s.path().join("history.json");
        let store = Store::new(path.clone());
        assert_eq!(store.list().unwrap().revision, "absent");
        assert!(!path.exists());
        let ticket = store.begin(&[input()], None, 1).unwrap();
        assert_eq!(
            Store::new(path.clone()).list().unwrap().items[0].phase,
            Phase::Pending
        );
        let record = store.finish(&ticket, Some(&result())).unwrap();
        assert_eq!(record.slots[0].actual, Some(input().target));
        assert_eq!(record.phase, Phase::Finished);
        let bytes = fs::read_to_string(&path).unwrap();
        assert!(!bytes.contains("ephemeral-window"));
        assert!(!bytes.contains("title"));
        assert!(!bytes.contains("pid"));
        assert_eq!(Store::new(path).list().unwrap().items[0], record);
    }
    #[test]
    fn fractional_grid_intent_settles_without_a_false_external_change() {
        let s = crate::testutil::Sandbox::new("window-history-fractional-grid");
        let store = Store::new(s.path().join("history.json"));
        let target = Rect {
            x: -1062.6666666666665,
            y: 30.,
            width: 525.3333333333334,
            height: 970.,
        };
        let mut row = input();
        row.target = target;
        let ticket = store.begin(&[row], None, 1).unwrap();
        let mut observed = result();
        observed.windows[0].actual_rect = Some(Rect {
            x: -1063.,
            width: 525.,
            ..target
        });
        let settled = store.finish(&ticket, Some(&observed)).unwrap();
        assert_eq!(settled.phase, Phase::Finished);
        assert_eq!(settled.slots[0].target, target);
        assert_eq!(store.list().unwrap().items[0], settled);
    }
    #[test]
    fn mixed_display_templates_preserve_intents_and_settle_across_store_reopen() {
        use crate::window_layout::{preview_geometry, DisplayArea, Template, WindowCandidate};
        let s = crate::testutil::Sandbox::new("window-history-display-templates");
        for (screen_name, rect, scale) in [
            (
                "left",
                Rect {
                    x: -1600.,
                    y: 30.,
                    width: 1600.,
                    height: 970.,
                },
                1.,
            ),
            (
                "top",
                Rect {
                    x: 0.,
                    y: -870.,
                    width: 1280.,
                    height: 870.,
                },
                2.,
            ),
        ] {
            for template in [Template::SideBySide, Template::MainAndTwo, Template::Grid] {
                let count = if template == Template::SideBySide {
                    2
                } else {
                    3
                };
                let windows = (0..count)
                    .map(|i| WindowCandidate {
                        window_id: format!("ephemeral-{i}"),
                        agent_id: "codex".into(),
                        application: "fixture".into(),
                        title: String::new(),
                        screen_id: None,
                        rect: Rect {
                            width: 500. + i as f64 * 13.,
                            ..input().before
                        },
                        movable: true,
                        resizable: true,
                        restriction: None,
                        minimum_size: None,
                    })
                    .collect::<Vec<_>>();
                let display = DisplayArea {
                    screen_id: screen_name.into(),
                    rect,
                    scale,
                };
                let geometry = preview_geometry(&display, &windows, template, 12.).unwrap();
                assert!(geometry.applicable);
                let intents = geometry
                    .placements
                    .iter()
                    .map(|p| Input {
                        window_id: p.window_id.clone(),
                        agent_id: "codex".into(),
                        before: p.before,
                        target: p.target,
                    })
                    .collect::<Vec<_>>();
                let path = s.path().join(format!("{screen_name}-{template:?}.json"));
                let ticket = Store::new(path.clone()).begin(&intents, None, 1).unwrap();
                let reopened = Store::new(path);
                assert_eq!(reopened.list().unwrap().items[0], ticket.record);
                let observed = crate::window_layout_execution::ResultDto {
                    windows: geometry
                        .placements
                        .iter()
                        .map(|p| crate::window_layout_execution::Row {
                            window_id: p.window_id.clone(),
                            status: crate::window_layout_execution::Status::Applied,
                            reason: None,
                            actual_rect: Some(Rect {
                                x: p.target.x.round(),
                                y: p.target.y.round(),
                                width: p.target.width.round(),
                                height: p.target.height.round(),
                            }),
                        })
                        .collect(),
                    ..result()
                };
                let settled = reopened.finish(&ticket, Some(&observed)).unwrap();
                assert_eq!(settled.phase, Phase::Finished);
                assert_eq!(reopened.list().unwrap().items[0], settled);
            }
        }
    }
    #[test]
    fn a_valid_subpoint_external_edit_still_refuses_settlement() {
        let s = crate::testutil::Sandbox::new("window-history-subpoint-conflict");
        let path = s.path().join("history.json");
        let store = Store::new(path.clone());
        let ticket = store.begin(&[input()], None, 1).unwrap();
        let (mut external, _) = store.read().unwrap();
        external.items[0].slots[0].target.x += 0.125;
        let bytes = serde_json::to_vec(&external).unwrap();
        fs::write(&path, &bytes).unwrap();
        assert!(store.finish(&ticket, Some(&result())).is_err());
        assert_eq!(fs::read(path).unwrap(), bytes);
        assert_eq!(store.list().unwrap().items[0].phase, Phase::Pending);
    }
    #[test]
    fn postwrite_external_changes_do_not_overwrite_or_reclassify_pending() {
        let s = crate::testutil::Sandbox::new("window-history-conflict");
        let path = s.path().join("history.json");
        let store = Store::new(path.clone());
        let ticket = store.begin(&[input()], None, 1).unwrap();
        fs::write(&path, b"external content").unwrap();
        assert!(store.finish(&ticket, Some(&result())).is_err());
        assert_eq!(fs::read(path).unwrap(), b"external content");
    }
    #[test]
    fn result_scope_and_failed_phase_are_checked_without_persisting_diagnostics() {
        let s = crate::testutil::Sandbox::new("window-history-results");
        let store = Store::new(s.path().join("history.json"));
        let ticket = store.begin(&[input()], None, 1).unwrap();
        let mut r = result();
        r.windows[0].window_id = "another-window".into();
        assert!(store.finish(&ticket, Some(&r)).is_err());
        assert_eq!(store.list().unwrap().items[0].phase, Phase::Pending);
        let record = store.finish(&ticket, None).unwrap();
        assert_eq!(record.phase, Phase::Failed);
        assert!(record.execution_id.is_none());
        let ticket = store.begin(&[input()], Some(&record.id), 2).unwrap();
        let mut r = result();
        r.windows[0].status = crate::window_layout_execution::Status::Failed;
        r.windows[0].actual_rect = Some(Rect {
            x: 50.,
            ..input().target
        });
        r.windows[0].reason = Some("diagnostic not persisted".into());
        let record = store.finish(&ticket, Some(&r)).unwrap();
        assert_eq!(
            record.recovery_of,
            Some(store.list().unwrap().items[0].id.clone())
        );
        assert_eq!(record.slots[0].outcome, Some(Outcome::Failed));
        assert!(!fs::read_to_string(&store.path)
            .unwrap()
            .contains("diagnostic"));
    }
    #[test]
    fn invalid_or_full_records_are_preserved_without_automatic_trimming() {
        let s = crate::testutil::Sandbox::new("window-history-capacity");
        let path = s.path().join("history.json");
        let store = Store::new(path.clone());
        assert!(store
            .begin(&[input()], Some(&uuid::Uuid::new_v4().to_string()), 1)
            .is_err());
        assert!(!path.exists());
        let ticket = store.begin(&[input()], None, 1).unwrap();
        let mut data = Data {
            schema_version: 1,
            items: vec![],
        };
        for _ in 0..MAX_RECORDS {
            let mut row = ticket.record.clone();
            row.id = uuid::Uuid::new_v4().to_string();
            data.items.push(row);
        }
        store.write(&data, &ticket.revision).unwrap();
        let before = fs::read(&path).unwrap();
        assert!(store.begin(&[input()], None, 2).is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        data.items[0].slots[0].agent_id = "unknown-tool".into();
        assert!(store.write(&data, &store.list().unwrap().revision).is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        fs::write(&path, b"{\"schema_version\":9,\"items\":[]}").unwrap();
        assert!(store.list().is_err());
    }
    #[test]
    fn explicit_record_removal_releases_capacity_and_stale_revision_never_deletes() {
        let sandbox = crate::testutil::Sandbox::new("window-history-removal");
        let store = Store::new(sandbox.path().join("history.json"));
        for _ in 0..100 {
            store.begin(&[input()], None, 1).unwrap();
        }
        let history = store.list().unwrap();
        assert!(store.begin(&[input()], None, 2).is_err());
        let next = store
            .remove(&history.items[0].id, &history.revision)
            .unwrap();
        assert_eq!(next.items.len(), 99);
        assert!(store
            .remove(&history.items[1].id, &history.revision)
            .is_err());
        assert_eq!(store.list().unwrap().items.len(), 99);
        store
            .begin_checked(&[input()], Some(&next.items[0].id), 2, Some(&next.revision))
            .unwrap();
        assert_eq!(store.list().unwrap().items.len(), 100);
        assert!(store
            .begin_checked(&[input()], None, 2, Some(&next.revision))
            .is_err());
    }
    #[cfg(unix)]
    #[test]
    fn linked_or_special_history_is_refused_without_reading_external_content() {
        let s = crate::testutil::Sandbox::new("window-history-link");
        let target = s.path().join("original");
        fs::write(&target, b"keep original").unwrap();
        let path = s.path().join("history.json");
        std::os::unix::fs::symlink(&target, &path).unwrap();
        assert!(Store::new(path.clone()).begin(&[input()], None, 1).is_err());
        assert_eq!(fs::read(&target).unwrap(), b"keep original");
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();
        assert!(Store::new(path).list().is_err());
    }
}
