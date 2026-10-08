//! Native-operation seam: verify all selected objects before writing any, report each result.
use crate::window_layout::{Rect, WindowCandidate};
use serde::Serialize;

pub trait Handle: Clone {
    /// Server-private process and launch identity; never persisted or sent to IPC.
    fn collision_domain(&self) -> Option<crate::window_layout_plan::Domain> {
        None
    }
    /// Rechecks process launch identity, object lifetime, permissions and restrictions.
    fn inspect(&self) -> Result<WindowCandidate, InspectError>;
    /// May partially succeed. Always attempts to read the resulting rectangle.
    fn write(&self, target: Rect) -> WriteResult;
}

pub enum InspectError {
    Gone(String),
    Unavailable(String),
}
impl InspectError {
    pub fn reason(self) -> String {
        match self {
            Self::Gone(s) | Self::Unavailable(s) => s,
        }
    }
}
pub struct WriteResult {
    pub actual: Option<Rect>,
    pub error: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Applied,
    Failed,
    Skipped,
    Restored,
    Conflict,
}
#[derive(Clone, Debug, Serialize)]
pub struct Row {
    pub window_id: String,
    pub status: Status,
    pub reason: Option<String>,
    pub actual_rect: Option<Rect>,
}
#[derive(Serialize)]
pub struct ResultDto {
    pub operation_id: String,
    pub windows: Vec<Row>,
    pub undo_available: bool,
    pub record_id: Option<String>,
    pub record_warning: Option<String>,
}
#[derive(Clone)]
pub struct Change<H> {
    pub id: String,
    pub handle: H,
    pub before: Rect,
    pub actual: Rect,
}
pub struct Operation<H> {
    pub id: String,
    pub changes: Vec<Change<H>>,
    pub conflicts: std::collections::HashSet<String>,
    // Bounded server-private handles also retain unchanged selected windows,
    // which can occupy an undo path without needing restoration themselves.
    participants: Vec<H>,
}
pub fn matches(a: Rect, b: Rect) -> bool {
    [a.x - b.x, a.y - b.y, a.width - b.width, a.height - b.height]
        .into_iter()
        .all(|v| v.is_finite() && v.abs() <= 1.0)
}
pub fn verify(candidate: &WindowCandidate, before: Rect, target: Rect) -> Result<(), String> {
    if !matches(candidate.rect, before) {
        return Err("窗口位置或尺寸已变化，请重新预览".into());
    }
    if let Some(reason) = &candidate.restriction {
        return Err(reason.clone());
    }
    if !candidate.movable && (target.x != before.x || target.y != before.y) {
        return Err("窗口不能移动".into());
    }
    if !candidate.resizable && (target.width != before.width || target.height != before.height) {
        return Err("窗口不能调整大小".into());
    }
    if candidate
        .minimum_size
        .is_some_and(|s| target.width < s[0] || target.height < s[1])
    {
        return Err("目标小于窗口最小尺寸".into());
    }
    Ok(())
}

fn prepare<H: Handle>(
    selected: &[(H, crate::window_layout::Placement)],
) -> Result<Vec<WindowCandidate>, String> {
    selected
        .iter()
        .map(|(h, p)| {
            let c = h.inspect().map_err(InspectError::reason)?;
            if c.window_id != p.window_id {
                return Err("窗口身份已变化，请重新读取".into());
            }
            verify(&c, p.before, p.target)?;
            Ok(c)
        })
        .collect()
}

#[cfg(test)]
pub fn apply<H: Handle>(
    selected: &[(H, crate::window_layout::Placement)],
) -> Result<(ResultDto, Operation<H>), String> {
    apply_on_displays(selected, &[], &[])
}

pub fn apply_on_displays<H: Handle>(
    selected: &[(H, crate::window_layout::Placement)],
    displays: &[crate::window_layout::DisplayArea],
    obstacles: &[(crate::window_layout_plan::Domain, Rect)],
) -> Result<(ResultDto, Operation<H>), String> {
    // Identity, constraints and the complete bounded plan precede every write.
    let candidates = prepare(selected)?;
    let placements: Vec<_> = selected.iter().map(|(_, p)| p.clone()).collect();
    let domains: Vec<_> = selected.iter().map(|(h, _)| h.collision_domain()).collect();
    let plan =
        crate::window_layout_plan::build(&placements, &candidates, &domains, displays, obstacles)?;
    Ok(perform(selected, &plan))
}

fn perform<H: Handle>(
    selected: &[(H, crate::window_layout::Placement)],
    plan: &[crate::window_layout_plan::Step],
) -> (ResultDto, Operation<H>) {
    let id = uuid::Uuid::new_v4().to_string();
    let mut changes: Vec<Option<Change<H>>> = vec![None; selected.len()];
    let mut rows: Vec<Option<Row>> = vec![None; selected.len()];
    let has_staging = plan.iter().any(|step| step.temporary);
    for step in plan {
        let (handle, placement) = &selected[step.index];
        let checked = handle
            .inspect()
            .map_err(InspectError::reason)
            .and_then(|c| {
                if c.window_id != placement.window_id {
                    return Err("窗口身份已变化".into());
                }
                verify(&c, step.before, step.target)?;
                Ok(c)
            });
        let candidate = match checked {
            Ok(c) => c,
            Err(reason) => {
                rows[step.index] = Some(Row {
                    window_id: placement.window_id.clone(),
                    status: Status::Skipped,
                    reason: Some(reason),
                    actual_rect: None,
                });
                if has_staging {
                    break;
                } else {
                    continue;
                }
            }
        };
        if matches(candidate.rect, step.target) {
            if !step.temporary {
                rows[step.index] = Some(Row {
                    window_id: placement.window_id.clone(),
                    status: Status::Skipped,
                    reason: Some("窗口已在目标位置".into()),
                    actual_rect: Some(candidate.rect),
                });
            }
            continue;
        }
        let mut written = handle.write(step.target);
        written.actual = written.actual.filter(|r| r.valid());
        if let Some(actual) = written.actual {
            changes[step.index] = (!matches(actual, placement.before)).then(|| Change {
                id: placement.window_id.clone(),
                handle: handle.clone(),
                before: placement.before,
                actual,
            });
        }
        let applied =
            written.error.is_none() && written.actual.is_some_and(|r| matches(r, step.target));
        if !applied || !step.temporary {
            rows[step.index] = Some(Row {
                window_id: placement.window_id.clone(),
                status: if applied {
                    Status::Applied
                } else {
                    Status::Failed
                },
                reason: if applied {
                    None
                } else {
                    written.error.or_else(|| {
                        Some(
                            if written.actual.is_none() {
                                "调整后无法读取位置，请在工具中核对"
                            } else {
                                "工具未采用目标尺寸，请在工具中核对"
                            }
                            .into(),
                        )
                    })
                },
                actual_rect: written.actual,
            });
        }
        if !applied && has_staging {
            break;
        }
    }
    let windows = rows
        .into_iter()
        .enumerate()
        .map(|(i, row)| {
            row.unwrap_or_else(|| Row {
                window_id: selected[i].1.window_id.clone(),
                status: if changes[i].is_some() {
                    Status::Failed
                } else {
                    Status::Skipped
                },
                reason: Some(
                    if changes[i].is_some() {
                        "窗口交换未完成，保留本次撤销；请核对实际位置"
                    } else {
                        "其他窗口调整未完成，已停止此次交换"
                    }
                    .into(),
                ),
                actual_rect: changes[i].as_ref().map(|c| c.actual),
            })
        })
        .collect();
    let changes: Vec<_> = changes.into_iter().flatten().collect();
    let dto = ResultDto {
        operation_id: id.clone(),
        windows,
        undo_available: !changes.is_empty(),
        record_id: None,
        record_warning: None,
    };
    (
        dto,
        Operation {
            id,
            changes,
            conflicts: std::collections::HashSet::new(),
            participants: selected.iter().map(|(h, _)| h.clone()).collect(),
        },
    )
}

#[cfg(test)]
pub fn undo<H: Handle>(op: &mut Operation<H>, force_ids: &[String]) -> Result<ResultDto, String> {
    undo_on_displays(op, force_ids, &[])
}

pub fn undo_on_displays<H: Handle>(
    op: &mut Operation<H>,
    force_ids: &[String],
    displays: &[crate::window_layout::DisplayArea],
) -> Result<ResultDto, String> {
    if force_ids.iter().any(|id| !op.conflicts.contains(id)) {
        return Err("请先查看撤销冲突，再选择要恢复的窗口".into());
    }
    let mut rows: Vec<Option<Row>> = vec![None; op.changes.len()];
    let mut eligible = Vec::new();
    let mut indices = Vec::new();
    let mut fixed = Vec::new();
    let mut removable = std::collections::HashSet::new();
    for (index, change) in op.changes.iter().enumerate() {
        let mut row = Row {
            window_id: change.id.clone(),
            status: Status::Failed,
            reason: None,
            actual_rect: None,
        };
        match change.handle.inspect() {
            Err(InspectError::Gone(reason)) => {
                row.status = Status::Skipped;
                row.reason = Some(reason);
                removable.insert(index);
            }
            Err(InspectError::Unavailable(reason)) => row.reason = Some(reason),
            Ok(current) => {
                row.actual_rect = Some(current.rect);
                if current.window_id != change.id {
                    row.status = Status::Skipped;
                    row.reason = Some("窗口身份已变化".into());
                    removable.insert(index);
                } else if matches(current.rect, change.before) {
                    row.status = Status::Restored;
                    row.reason = Some("窗口已恢复".into());
                    removable.insert(index);
                } else if !matches(current.rect, change.actual) && !force_ids.contains(&change.id) {
                    op.conflicts.insert(change.id.clone());
                    row.status = Status::Conflict;
                    row.reason = Some("窗口之后被调整，确认后才能覆盖恢复".into());
                } else if let Err(reason) = verify(&current, current.rect, change.before) {
                    row.reason = Some(reason);
                } else {
                    indices.push(index);
                    eligible.push((
                        change.handle.clone(),
                        crate::window_layout::Placement {
                            window_id: change.id.clone(),
                            before: current.rect,
                            target: change.before,
                            restriction: None,
                        },
                    ));
                    continue;
                }
                if let Some(domain) = change.handle.collision_domain() {
                    fixed.push((domain, current.rect));
                }
            }
        }
        rows[index] = Some(row);
    }
    let mut observed = std::collections::HashMap::new();
    for handle in &op.participants {
        if let Ok(current) = handle.inspect() {
            if !eligible
                .iter()
                .any(|(_, p)| p.window_id == current.window_id)
            {
                if let Some(domain) = handle.collision_domain() {
                    fixed.push((domain, current.rect));
                }
            }
        }
    }
    if !eligible.is_empty() {
        match apply_on_displays(&eligible, displays, &fixed) {
            Err(reason) => {
                for (local, &index) in indices.iter().enumerate() {
                    rows[index] = Some(Row {
                        window_id: op.changes[index].id.clone(),
                        status: Status::Failed,
                        reason: Some(reason.clone()),
                        actual_rect: Some(eligible[local].1.before),
                    });
                }
            }
            Ok((result, partial)) => {
                for change in partial.changes {
                    observed.insert(change.id, change.actual);
                }
                for (mut row, &index) in result.windows.into_iter().zip(&indices) {
                    if row.status == Status::Applied
                        || (row.status == Status::Skipped
                            && row
                                .actual_rect
                                .is_some_and(|r| matches(r, op.changes[index].before)))
                    {
                        row.status = Status::Restored;
                        removable.insert(index);
                    } else if row.status == Status::Skipped {
                        match op.changes[index].handle.inspect() {
                            Err(InspectError::Gone(_)) => {
                                removable.insert(index);
                            }
                            Ok(current) if current.window_id != op.changes[index].id => {
                                removable.insert(index);
                            }
                            _ => row.status = Status::Failed,
                        }
                    }
                    if let Some(actual) = row.actual_rect {
                        observed.insert(row.window_id.clone(), actual);
                    }
                    rows[index] = Some(row);
                }
            }
        }
    }
    let mut remaining = Vec::new();
    for (index, mut change) in op.changes.drain(..).enumerate() {
        if removable.contains(&index) {
            op.conflicts.remove(&change.id);
        } else {
            if let Some(actual) = observed.get(&change.id) {
                change.actual = *actual;
            }
            remaining.push(change);
        }
    }
    op.changes = remaining;
    if op.changes.is_empty() {
        op.participants.clear();
    }
    Ok(ResultDto {
        operation_id: op.id.clone(),
        windows: rows.into_iter().map(Option::unwrap).collect(),
        undo_available: !op.changes.is_empty(),
        record_id: None,
        record_warning: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, rc::Rc};
    #[derive(Clone)]
    struct Fake(Rc<RefCell<State>>);
    struct State {
        candidate: WindowCandidate,
        writes: usize,
        partial: bool,
        dead: bool,
        unavailable: bool,
        mutate_history: Option<std::path::PathBuf>,
    }
    impl Handle for Fake {
        fn inspect(&self) -> Result<WindowCandidate, InspectError> {
            let s = self.0.borrow();
            if s.dead {
                Err(InspectError::Gone("窗口已关闭".into()))
            } else if s.unavailable {
                Err(InspectError::Unavailable("工具未响应".into()))
            } else {
                Ok(s.candidate.clone())
            }
        }
        fn write(&self, target: Rect) -> WriteResult {
            let mut s = self.0.borrow_mut();
            s.writes += 1;
            if let Some(path) = s.mutate_history.take() {
                std::fs::write(path, b"external modification").unwrap();
            }
            if s.partial {
                s.candidate.rect.x = target.x;
            } else {
                s.candidate.rect = target;
            }
            WriteResult {
                actual: Some(s.candidate.rect),
                error: s.partial.then(|| "尺寸被拒绝".into()),
            }
        }
    }
    fn item(id: &str) -> (Fake, crate::window_layout::Placement) {
        let before = Rect {
            x: 20.0,
            y: 30.0,
            width: 600.0,
            height: 400.0,
        };
        let target = Rect {
            x: 0.0,
            y: 25.0,
            width: 750.0,
            height: 850.0,
        };
        let c = WindowCandidate {
            window_id: id.into(),
            agent_id: "fixture".into(),
            application: "Tool".into(),
            title: "same".into(),
            screen_id: None,
            rect: before,
            movable: true,
            resizable: true,
            restriction: None,
            minimum_size: None,
        };
        (
            Fake(Rc::new(RefCell::new(State {
                candidate: c,
                writes: 0,
                partial: false,
                dead: false,
                unavailable: false,
                mutate_history: None,
            }))),
            crate::window_layout::Placement {
                window_id: id.into(),
                before,
                target,
                restriction: None,
            },
        )
    }
    #[test]
    fn any_preflight_drift_prevents_every_write() {
        let a = item("a");
        let b = item("b");
        b.0 .0.borrow_mut().candidate.rect.x += 20.0;
        assert!(apply(&[a.clone(), b]).is_err());
        assert_eq!(a.0 .0.borrow().writes, 0);
    }
    #[test]
    fn partial_failure_is_reported_and_its_actual_change_is_undoable() {
        let a = item("a");
        let b = item("b");
        b.0 .0.borrow_mut().partial = true;
        let (r, mut op) = apply(&[a.clone(), b.clone()]).unwrap();
        assert_eq!(r.windows[0].status, Status::Applied);
        assert_eq!(r.windows[1].status, Status::Failed);
        assert_eq!(op.changes.len(), 2);
        b.0 .0.borrow_mut().partial = false;
        let r = undo(&mut op, &[]).unwrap();
        assert!(r.windows.iter().all(|r| r.status == Status::Restored));
        assert!(!r.undo_available);
    }
    #[test]
    fn user_drift_requires_observed_conflict_then_explicit_window_confirmation() {
        let a = item("a");
        let (_, mut op) = apply(&[a.clone()]).unwrap();
        a.0 .0.borrow_mut().candidate.rect.x = 50.0;
        assert!(undo(&mut op, &["a".into()]).is_err());
        assert_eq!(
            undo(&mut op, &[]).unwrap().windows[0].status,
            Status::Conflict
        );
        assert_eq!(a.0 .0.borrow().writes, 1);
        assert_eq!(
            undo(&mut op, &["a".into()]).unwrap().windows[0].status,
            Status::Restored
        );
    }
    #[test]
    fn a_closed_object_is_skipped_without_mutating_a_replacement() {
        let a = item("a");
        let (_, mut op) = apply(&[a.clone()]).unwrap();
        a.0 .0.borrow_mut().dead = true;
        let r = undo(&mut op, &[]).unwrap();
        assert_eq!(r.windows[0].status, Status::Skipped);
        assert_eq!(a.0 .0.borrow().writes, 1);
        assert!(!r.undo_available);
    }
    #[test]
    fn transient_unavailability_keeps_the_original_undo_for_retry() {
        let a = item("a");
        let (_, mut op) = apply(&[a.clone()]).unwrap();
        a.0 .0.borrow_mut().unavailable = true;
        let r = undo(&mut op, &[]).unwrap();
        assert_eq!(r.windows[0].status, Status::Failed);
        assert!(r.undo_available);
        a.0 .0.borrow_mut().unavailable = false;
        assert_eq!(
            undo(&mut op, &[]).unwrap().windows[0].status,
            Status::Restored
        );
    }
    #[test]
    fn a_different_object_identity_cannot_pass_geometry_checks() {
        let a = item("a");
        a.0 .0.borrow_mut().candidate.window_id = "replacement".into();
        assert!(apply(&[a.clone()]).is_err());
        assert_eq!(a.0 .0.borrow().writes, 0);
    }
    #[test]
    fn broker_consumes_preview_once_and_rechecks_display_and_lifetime() {
        let a = item("a");
        let displays = vec![crate::window_layout::DisplayArea {
            screen_id: "d".into(),
            rect: a.1.target,
            scale: 2.0,
        }];
        let mut store = crate::window_layout_service::Store::default();
        store
            .replace(
                displays.clone(),
                vec![(a.0.inspect().ok().unwrap(), a.0.clone())],
                vec![],
            )
            .unwrap();
        let p = store
            .preview(
                &["a".into()],
                "d",
                crate::window_layout::Template::Grid,
                0.0,
                1,
                100,
            )
            .unwrap();
        assert!(store
            .apply_preview(&p.preview_id, 1, &displays, 15_100)
            .is_err());
        assert_eq!(a.0 .0.borrow().writes, 0);
        let p = store
            .preview(
                &["a".into()],
                "d",
                crate::window_layout::Template::Grid,
                0.0,
                1,
                200,
            )
            .unwrap();
        let mut moved = displays.clone();
        moved[0].rect.y += 1.0;
        assert!(store.apply_preview(&p.preview_id, 1, &moved, 201).is_err());
        assert_eq!(a.0 .0.borrow().writes, 0);
        let p = store
            .preview(
                &["a".into()],
                "d",
                crate::window_layout::Template::Grid,
                0.0,
                1,
                300,
            )
            .unwrap();
        let r = store
            .apply_preview(&p.preview_id, 1, &displays, 301)
            .unwrap();
        assert!(r.undo_available);
        assert!(store
            .apply_preview(&p.preview_id, 1, &displays, 302)
            .is_err());
        assert_eq!(a.0 .0.borrow().writes, 1);
        assert!(store.undo_operation("other", &[]).is_err());
        assert_eq!(
            store.undo_operation(&r.operation_id, &[]).unwrap().windows[0].status,
            Status::Restored
        );
    }
    #[test]
    fn disconnected_original_display_keeps_undo_for_reconnect() {
        let a = item("a");
        let displays = vec![crate::window_layout::DisplayArea {
            screen_id: "d".into(),
            rect: a.1.target,
            scale: 2.0,
        }];
        let mut store = crate::window_layout_service::Store::default();
        store
            .replace(
                displays.clone(),
                vec![(a.0.inspect().ok().unwrap(), a.0.clone())],
                vec![],
            )
            .unwrap();
        let p = store
            .preview(
                &["a".into()],
                "d",
                crate::window_layout::Template::Grid,
                0.0,
                1,
                100,
            )
            .unwrap();
        let r = store
            .apply_preview(&p.preview_id, 1, &displays, 101)
            .unwrap();
        let failed = store.undo_on_displays(&r.operation_id, &[], &[]).unwrap();
        assert_eq!(failed.windows[0].status, Status::Failed);
        assert!(failed.undo_available);
        assert_eq!(a.0 .0.borrow().writes, 1);
        assert_eq!(
            store
                .undo_on_displays(&r.operation_id, &[], &displays)
                .unwrap()
                .windows[0]
                .status,
            Status::Restored
        );
    }
    #[test]
    fn journal_prewrite_failure_prevents_moves_and_preflight_failure_records_no_execution() {
        let sandbox = crate::testutil::Sandbox::new("window-journal-execution");
        let path = sandbox.path().join("history.json");
        let a = item("a");
        a.0 .0.borrow_mut().candidate.agent_id = "codex".into();
        let displays = vec![crate::window_layout::DisplayArea {
            screen_id: "d".into(),
            rect: a.1.target,
            scale: 2.0,
        }];
        let mut store = crate::window_layout_service::Store::with_journal(
            crate::window_layout_journal::Store::new(path.clone()),
        );
        store
            .replace(
                displays.clone(),
                vec![(a.0.inspect().ok().unwrap(), a.0.clone())],
                vec![],
            )
            .unwrap();
        std::fs::write(&path, b"corrupt history").unwrap();
        let p = store
            .preview(
                &["a".into()],
                "d",
                crate::window_layout::Template::Grid,
                0.0,
                1,
                100,
            )
            .unwrap();
        assert!(store
            .apply_preview(&p.preview_id, 1, &displays, 101)
            .is_err());
        assert_eq!(a.0 .0.borrow().writes, 0);
        assert_eq!(std::fs::read(&path).unwrap(), b"corrupt history");
        std::fs::remove_file(&path).unwrap();
        let p = store
            .preview(
                &["a".into()],
                "d",
                crate::window_layout::Template::Grid,
                0.0,
                1,
                200,
            )
            .unwrap();
        a.0 .0.borrow_mut().dead = true;
        assert!(store
            .apply_preview(&p.preview_id, 1, &displays, 201)
            .is_err());
        assert_eq!(a.0 .0.borrow().writes, 0);
        let records = crate::window_layout_journal::Store::new(path)
            .list()
            .unwrap();
        assert_eq!(
            records.items[0].phase,
            crate::window_layout_journal::Phase::Failed
        );
        assert!(records.items[0].slots[0].actual.is_none());
    }
    #[test]
    fn journal_postwrite_failure_preserves_actual_result_and_live_undo() {
        let sandbox = crate::testutil::Sandbox::new("window-journal-after-write");
        let path = sandbox.path().join("history.json");
        let a = item("a");
        a.0 .0.borrow_mut().candidate.agent_id = "codex".into();
        a.0 .0.borrow_mut().mutate_history = Some(path.clone());
        let displays = vec![crate::window_layout::DisplayArea {
            screen_id: "d".into(),
            rect: a.1.target,
            scale: 2.0,
        }];
        let mut store = crate::window_layout_service::Store::with_journal(
            crate::window_layout_journal::Store::new(path.clone()),
        );
        store
            .replace(
                displays.clone(),
                vec![(a.0.inspect().ok().unwrap(), a.0.clone())],
                vec![],
            )
            .unwrap();
        let p = store
            .preview(
                &["a".into()],
                "d",
                crate::window_layout::Template::Grid,
                0.0,
                1,
                100,
            )
            .unwrap();
        let r = store
            .apply_preview(&p.preview_id, 1, &displays, 101)
            .unwrap();
        assert!(r.record_warning.is_some());
        assert!(r.record_id.is_none());
        assert!(r.undo_available);
        assert_eq!(r.windows[0].status, Status::Applied);
        assert_eq!(a.0 .0.borrow().writes, 1);
        assert_eq!(std::fs::read(&path).unwrap(), b"external modification");
        let restored = store.undo_operation(&r.operation_id, &[]).unwrap();
        assert_eq!(restored.windows[0].status, Status::Restored);
        assert_eq!(a.0 .0.borrow().candidate.rect, a.1.before);
    }
    #[test]
    fn normal_journal_result_retains_geometry_across_service_restart_without_window_title() {
        let sandbox = crate::testutil::Sandbox::new("window-journal-readback");
        let path = sandbox.path().join("history.json");
        let a = item("a");
        a.0 .0.borrow_mut().candidate.agent_id = "codex".into();
        a.0 .0.borrow_mut().candidate.title = "private fixture window title".into();
        let displays = vec![crate::window_layout::DisplayArea {
            screen_id: "d".into(),
            rect: a.1.target,
            scale: 2.0,
        }];
        let mut store = crate::window_layout_service::Store::with_journal(
            crate::window_layout_journal::Store::new(path.clone()),
        );
        store
            .replace(
                displays.clone(),
                vec![(a.0.inspect().ok().unwrap(), a.0.clone())],
                vec![],
            )
            .unwrap();
        let p = store
            .preview(
                &["a".into()],
                "d",
                crate::window_layout::Template::Grid,
                0.0,
                1,
                100,
            )
            .unwrap();
        let r = store
            .apply_preview(&p.preview_id, 1, &displays, 101)
            .unwrap();
        assert!(r.record_warning.is_none());
        assert!(r.record_id.is_some());
        drop(store);
        let records = crate::window_layout_journal::Store::new(path.clone())
            .list()
            .unwrap();
        assert_eq!(records.items[0].id, r.record_id.unwrap());
        assert_eq!(records.items[0].execution_id, Some(r.operation_id));
        assert_eq!(records.items[0].slots[0].before, a.1.before);
        assert_eq!(records.items[0].slots[0].actual, Some(a.1.target));
        assert!(!std::fs::read_to_string(path)
            .unwrap()
            .contains("private fixture window title"));
    }

    #[test]
    fn restart_recovery_explicitly_rebinds_same_tool_and_retains_undo_and_parent() {
        let sandbox = crate::testutil::Sandbox::new("window-recovery-restart");
        let path = sandbox.path().join("history.json");
        let a = item("old-id");
        a.0 .0.borrow_mut().candidate.agent_id = "codex".into();
        let displays = vec![crate::window_layout::DisplayArea {
            screen_id: "d".into(),
            rect: Rect {
                x: 0.,
                y: 0.,
                width: 1200.,
                height: 1000.,
            },
            scale: 2.,
        }];
        let mut store = crate::window_layout_service::Store::with_journal(
            crate::window_layout_journal::Store::new(path.clone()),
        );
        store
            .replace(
                displays.clone(),
                vec![(a.0.inspect().ok().unwrap(), a.0.clone())],
                vec![],
            )
            .unwrap();
        let p = store
            .preview(
                &["old-id".into()],
                "d",
                crate::window_layout::Template::Grid,
                0.,
                1,
                100,
            )
            .unwrap();
        store
            .apply_preview(&p.preview_id, 1, &displays, 101)
            .unwrap();
        drop(store);
        let history = crate::window_layout_journal::Store::new(path.clone())
            .list()
            .unwrap();
        let record = &history.items[0];
        let slot = &record.slots[0];
        a.0 .0.borrow_mut().candidate.window_id = "fresh-id".into();
        let mut store = crate::window_layout_service::Store::with_journal(
            crate::window_layout_journal::Store::new(path.clone()),
        );
        store
            .replace(
                displays.clone(),
                vec![(a.0.inspect().ok().unwrap(), a.0.clone())],
                vec![],
            )
            .unwrap();
        assert!(store
            .preview_recovery(&record.id, &slot.id, "old-id", &history.revision, 1, 200)
            .is_err());
        a.0 .0.borrow_mut().candidate.agent_id = "claude".into();
        store
            .replace(
                displays.clone(),
                vec![(a.0.inspect().ok().unwrap(), a.0.clone())],
                vec![],
            )
            .unwrap();
        assert!(store
            .preview_recovery(&record.id, &slot.id, "fresh-id", &history.revision, 2, 200)
            .is_err());
        a.0 .0.borrow_mut().candidate.agent_id = "codex".into();
        store
            .replace(
                displays.clone(),
                vec![(a.0.inspect().ok().unwrap(), a.0.clone())],
                vec![],
            )
            .unwrap();
        let p = store
            .preview_recovery(&record.id, &slot.id, "fresh-id", &history.revision, 3, 200)
            .unwrap();
        assert_eq!(a.0 .0.borrow().writes, 1);
        assert_eq!(p.geometry.placements[0].target, slot.before);
        let r = store
            .apply_preview(&p.preview_id, 3, &displays, 201)
            .unwrap();
        assert!(r.undo_available);
        assert_eq!(a.0 .0.borrow().candidate.rect, slot.before);
        let recovered = crate::window_layout_journal::Store::new(path)
            .list()
            .unwrap();
        assert_eq!(recovered.items[1].recovery_of, Some(record.id.clone()));
        assert_eq!(recovered.items[1].slots[0].before, displays[0].rect);
        assert_eq!(
            store.undo_operation(&r.operation_id, &[]).unwrap().windows[0].status,
            Status::Restored
        );
        assert_eq!(a.0 .0.borrow().candidate.rect, displays[0].rect);
    }
    #[test]
    fn historical_recovery_rejects_changed_records_screens_and_native_geometry() {
        let sandbox = crate::testutil::Sandbox::new("window-recovery-conflicts");
        let path = sandbox.path().join("history.json");
        let a = item("a");
        a.0 .0.borrow_mut().candidate.agent_id = "codex".into();
        let journal = crate::window_layout_journal::Store::new(path.clone());
        let ticket = journal
            .begin(
                &[crate::window_layout_journal::Input {
                    window_id: "a".into(),
                    agent_id: "codex".into(),
                    before: a.1.before,
                    target: a.1.target,
                }],
                None,
                100,
            )
            .unwrap();
        journal.finish(&ticket, None).unwrap();
        let history = journal.list().unwrap();
        let r = &history.items[0];
        let slot = &r.slots[0];
        let displays = vec![crate::window_layout::DisplayArea {
            screen_id: "d".into(),
            rect: Rect {
                x: 0.,
                y: 0.,
                width: 1200.,
                height: 1000.,
            },
            scale: 2.,
        }];
        let mut store = crate::window_layout_service::Store::with_journal(journal);
        let mut small = displays.clone();
        small[0].rect.width = 100.;
        store
            .replace(
                small,
                vec![(a.0.inspect().ok().unwrap(), a.0.clone())],
                vec![],
            )
            .unwrap();
        assert!(store
            .preview_recovery(&r.id, &slot.id, "a", &history.revision, 1, 200)
            .is_err());
        store
            .replace(
                displays.clone(),
                vec![(a.0.inspect().ok().unwrap(), a.0.clone())],
                vec![],
            )
            .unwrap();
        let p = store
            .preview_recovery(&r.id, &slot.id, "a", &history.revision, 2, 200)
            .unwrap();
        a.0 .0.borrow_mut().candidate.rect.x += 20.;
        assert!(store
            .apply_preview(&p.preview_id, 2, &displays, 201)
            .is_err());
        assert_eq!(a.0 .0.borrow().writes, 0);
        store
            .replace(
                displays.clone(),
                vec![(a.0.inspect().ok().unwrap(), a.0.clone())],
                vec![],
            )
            .unwrap();
        let h = crate::window_layout_journal::Store::new(path.clone())
            .list()
            .unwrap();
        let p = store
            .preview_recovery(&r.id, &slot.id, "a", &h.revision, 3, 300)
            .unwrap();
        crate::window_layout_journal::Store::new(path.clone())
            .remove(&r.id, &h.revision)
            .unwrap();
        assert!(store
            .apply_preview(&p.preview_id, 3, &displays, 301)
            .is_err());
        assert_eq!(a.0 .0.borrow().writes, 0);
    }

    #[test]
    fn workspace_window_scope_rejects_unrelated_previews_and_survives_restart_recovery() {
        let sandbox = crate::testutil::Sandbox::new("workspace-window-scope");
        let path = sandbox.path().join("history.json");
        let a = item("a");
        a.0 .0.borrow_mut().candidate.agent_id = "codex".into();
        let context = crate::window_layout_journal::WorkspaceContext {
            id: uuid::Uuid::new_v4().to_string(),
            expected_revision: 4,
            layout_id: uuid::Uuid::new_v4().to_string(),
            expected_rules_revision: 7,
        };
        let rule = crate::window_layout_rules::Rule {
            id: context.layout_id.clone(),
            name: "fixture".into(),
            tools: vec!["codex".into()],
            template: crate::window_layout::Template::Grid,
            gap: 0.,
            screen_preference: crate::window_layout_rules::ScreenPreference::Primary,
        };
        let mut displays = vec![crate::window_layout::DisplayArea {
            screen_id: "d".into(),
            rect: Rect {
                x: 0.,
                y: 0.,
                width: 1200.,
                height: 1000.,
            },
            scale: 2.,
        }];
        let mut secondary = displays[0].clone();
        secondary.screen_id = "secondary".into();
        secondary.rect.x = 1200.;
        displays.push(secondary);
        let mut store = crate::window_layout_service::Store::with_journal(
            crate::window_layout_journal::Store::new(path.clone()),
        );
        store
            .replace(
                displays.clone(),
                vec![(a.0.inspect().ok().unwrap(), a.0.clone())],
                vec![],
            )
            .unwrap();
        let p = store
            .preview(&["a".into()], "d", rule.template, 12., 1, 100)
            .unwrap();
        assert!(store
            .apply_scoped(
                &p.preview_id,
                1,
                &displays,
                101,
                Some(&context),
                Some(&rule)
            )
            .is_err());
        assert_eq!(a.0 .0.borrow().writes, 0);
        assert!(!path.exists());
        let secondary = store
            .preview(&["a".into()], "secondary", rule.template, 0., 1, 150)
            .unwrap();
        assert!(store
            .apply_scoped(
                &secondary.preview_id,
                1,
                &displays,
                151,
                Some(&context),
                Some(&rule)
            )
            .is_err());
        assert_eq!(a.0 .0.borrow().writes, 0);
        let p = store
            .preview(&["a".into()], "d", rule.template, 0., 1, 200)
            .unwrap();
        let result = store
            .apply_scoped(
                &p.preview_id,
                1,
                &displays,
                201,
                Some(&context),
                Some(&rule),
            )
            .unwrap();
        let history = crate::window_layout_journal::Store::new(path.clone())
            .list()
            .unwrap();
        assert_eq!(history.items[0].workspace, Some(context.clone()));
        assert_eq!(history.items[0].id, result.record_id.unwrap());
        drop(store);
        let mut store = crate::window_layout_service::Store::with_journal(
            crate::window_layout_journal::Store::new(path.clone()),
        );
        a.0 .0.borrow_mut().candidate.window_id = "new".into();
        store
            .replace(
                displays.clone(),
                vec![(a.0.inspect().ok().unwrap(), a.0.clone())],
                vec![],
            )
            .unwrap();
        let record = &history.items[0];
        let p = store
            .preview_recovery(
                &record.id,
                &record.slots[0].id,
                "new",
                &history.revision,
                1,
                300,
            )
            .unwrap();
        let mut wrong = context.clone();
        wrong.id = uuid::Uuid::new_v4().to_string();
        assert!(store
            .apply_scoped(&p.preview_id, 1, &displays, 301, Some(&wrong), Some(&rule))
            .is_err());
        assert_eq!(a.0 .0.borrow().writes, 1);
        let p = store
            .preview_recovery(
                &record.id,
                &record.slots[0].id,
                "new",
                &history.revision,
                1,
                400,
            )
            .unwrap();
        let recovered = store
            .apply_scoped(
                &p.preview_id,
                1,
                &displays,
                401,
                Some(&context),
                Some(&rule),
            )
            .unwrap();
        assert!(recovered.undo_available);
        let records = crate::window_layout_journal::Store::new(path)
            .list()
            .unwrap();
        assert_eq!(records.items[1].workspace, Some(context));
        assert_eq!(records.items[1].recovery_of, Some(record.id.clone()));
        assert_eq!(a.0 .0.borrow().candidate.rect, a.1.before);
    }
}

#[cfg(test)]
mod sequencing_tests {
    use super::*;
    use crate::window_layout_geometry::{self, Driver, Part};
    use crate::window_layout_readback::Observation;
    use std::{cell::RefCell, rc::Rc};

    #[derive(Clone)]
    struct Window {
        fleet: Rc<RefCell<Vec<WindowCandidate>>>,
        index: usize,
        writes: Rc<RefCell<Vec<usize>>>,
        fail_write: Option<Rc<std::cell::Cell<bool>>>,
        block_after_stage: Option<Rc<std::cell::Cell<bool>>>,
    }
    impl Handle for Window {
        fn collision_domain(&self) -> Option<crate::window_layout_plan::Domain> {
            Some((42, 1))
        }
        fn inspect(&self) -> Result<WindowCandidate, InspectError> {
            if !self.writes.borrow().is_empty()
                && self.block_after_stage.as_ref().is_some_and(|f| f.get())
            {
                return Err(InspectError::Unavailable("窗口权限在交换中失效".into()));
            }
            let fleet = self.fleet.borrow();
            let candidate = fleet[self.index].clone();
            let same = fleet
                .iter()
                .filter(|w| matches(w.rect, candidate.rect))
                .count();
            let visible: Vec<_> = fleet
                .iter()
                .enumerate()
                .map(|(i, w)| (42, i as u32 + 1, w.rect))
                .collect();
            crate::window_visibility::match_id(42, candidate.rect, true, same, &visible)
                .map_err(InspectError::Unavailable)?;
            Ok(candidate)
        }
        fn write(&self, target: Rect) -> WriteResult {
            self.writes.borrow_mut().push(self.index);
            let error = window_layout_geometry::adjust(self, target).err();
            let error = error.or_else(|| self.inspect().err().map(InspectError::reason));
            let error = error.or_else(|| {
                self.fail_write
                    .as_ref()
                    .filter(|f| f.get())
                    .map(|_| "写入后未通过核验".into())
            });
            WriteResult {
                actual: Some(self.fleet.borrow()[self.index].rect),
                error,
            }
        }
    }
    impl Driver for Window {
        fn read(&self) -> Result<Rect, String> {
            self.inspect().map(|c| c.rect).map_err(InspectError::reason)
        }
        fn resize(&self, target: Rect) -> Result<(), String> {
            let mut fleet = self.fleet.borrow_mut();
            let rect = &mut fleet[self.index].rect;
            rect.width = target.width.min(1024.0 - rect.x);
            // The actual isolated Code run rounded a 681 pt height to 680 pt.
            rect.height = (target.height / 2.0).floor() * 2.0;
            Ok(())
        }
        fn move_to(&self, target: Rect) -> Result<(), String> {
            let mut fleet = self.fleet.borrow_mut();
            let rect = &mut fleet[self.index].rect;
            rect.x = target.x.min(1024.0 - rect.width);
            rect.y = target.y;
            Ok(())
        }
        fn settle(&self, target: Rect, part: Part) -> Observation {
            let actual = self.fleet.borrow()[self.index].rect;
            Observation {
                actual: Some(actual),
                error: None,
                timed_out: !part.accepts(actual, target),
            }
        }
    }
    fn swap_fixture() -> (
        Vec<(Window, crate::window_layout::Placement)>,
        Vec<crate::window_layout::DisplayArea>,
    ) {
        let rect = |x| Rect {
            x,
            y: 30.0,
            width: 506.0,
            height: 680.0,
        };
        let fleet = Rc::new(RefCell::new(
            (0..2)
                .map(|i| WindowCandidate {
                    window_id: i.to_string(),
                    agent_id: "vscode".into(),
                    application: "Code".into(),
                    title: String::new(),
                    screen_id: None,
                    rect: rect(i as f64 * 518.0),
                    movable: true,
                    resizable: true,
                    restriction: None,
                    minimum_size: None,
                })
                .collect(),
        ));
        let writes = Rc::new(RefCell::new(Vec::new()));
        let selected = (0..2)
            .map(|i| {
                (
                    Window {
                        fleet: fleet.clone(),
                        index: i,
                        writes: writes.clone(),
                        fail_write: None,
                        block_after_stage: None,
                    },
                    crate::window_layout::Placement {
                        window_id: i.to_string(),
                        before: rect(i as f64 * 518.0),
                        target: rect((1 - i) as f64 * 518.0),
                        restriction: None,
                    },
                )
            })
            .collect();
        (
            selected,
            vec![crate::window_layout::DisplayArea {
                screen_id: "d".into(),
                scale: 1.0,
                rect: Rect {
                    x: 0.0,
                    y: 30.0,
                    width: 1024.0,
                    height: 681.0,
                },
            }],
        )
    }
    #[test]
    fn failed_temporary_write_stops_and_undo_includes_its_actual_position() {
        let (mut selected, displays) = swap_fixture();
        let fault = Rc::new(std::cell::Cell::new(true));
        selected[0].0.fail_write = Some(fault.clone());
        let (result, mut op) = apply_on_displays(&selected, &displays, &[]).unwrap();
        assert_eq!(*selected[0].0.writes.borrow(), vec![0]);
        assert_eq!(result.windows[0].status, Status::Failed);
        assert_eq!(result.windows[1].status, Status::Skipped);
        assert_eq!(op.changes.len(), 1);
        assert_eq!(op.changes[0].before.x, 0.0);
        assert_eq!(op.changes[0].actual.x, 4.0);
        fault.set(false);
        let undone = undo_on_displays(&mut op, &[], &displays).unwrap();
        assert_eq!(undone.windows[0].status, Status::Restored);
        assert!(!undone.undo_available);
        assert_eq!(selected[0].0.fleet.borrow()[0].rect.x, 0.0);
    }
    #[test]
    fn identity_unavailable_after_stage_preserves_temporary_change_for_undo() {
        let (mut selected, displays) = swap_fixture();
        let blocked = Rc::new(std::cell::Cell::new(true));
        selected[1].0.block_after_stage = Some(blocked.clone());
        let (result, mut op) = apply_on_displays(&selected, &displays, &[]).unwrap();
        assert_eq!(*selected[0].0.writes.borrow(), vec![0]);
        assert_eq!(result.windows[0].status, Status::Failed);
        assert_eq!(result.windows[0].actual_rect.unwrap().x, 4.0);
        assert_eq!(result.windows[1].status, Status::Skipped);
        assert_eq!(op.changes.len(), 1);
        blocked.set(false);
        assert!(
            !undo_on_displays(&mut op, &[], &displays)
                .unwrap()
                .undo_available
        );
    }
    #[test]
    fn unplannable_swap_writes_nothing() {
        let (selected, _) = swap_fixture();
        assert!(apply_on_displays(&selected, &[], &[]).is_err());
        assert!(selected[0].0.writes.borrow().is_empty());
    }
    #[test]
    fn swap_undo_keeps_manual_drift_conflicted_until_explicit_confirmation() {
        let (selected, displays) = swap_fixture();
        let (_, mut op) = apply_on_displays(&selected, &displays, &[]).unwrap();
        selected[0].0.fleet.borrow_mut()[0].rect.x = 500.0;
        let result = undo_on_displays(&mut op, &[], &displays).unwrap();
        assert_eq!(result.windows[0].status, Status::Conflict);
        assert_eq!(result.windows[1].status, Status::Restored);
        assert_eq!(selected[0].0.fleet.borrow()[0].rect.x, 500.0);
        let again = undo_on_displays(&mut op, &[], &displays).unwrap();
        assert_eq!(again.windows[0].status, Status::Conflict);
        let forced = undo_on_displays(&mut op, &["0".into()], &displays).unwrap();
        assert_eq!(forced.windows[0].status, Status::Restored);
        assert!(!forced.undo_available);
    }
    #[test]
    fn failed_undo_planning_keeps_all_changes_and_writes_nothing() {
        let (selected, displays) = swap_fixture();
        let (_, mut op) = apply_on_displays(&selected, &displays, &[]).unwrap();
        let writes = selected[0].0.writes.borrow().len();
        let result = undo_on_displays(&mut op, &[], &[]).unwrap();
        assert!(result.windows.iter().all(|r| r.status == Status::Failed));
        assert_eq!(selected[0].0.writes.borrow().len(), writes);
        assert_eq!(op.changes.len(), 2);
        assert!(
            !undo_on_displays(&mut op, &[], &displays)
                .unwrap()
                .undo_available
        );
    }
    #[test]
    fn unchanged_selected_window_still_protects_the_full_size_undo_path() {
        let (mut selected, displays) = swap_fixture();
        selected[0].1.before.width = 1024.0;
        selected[0].0.fleet.borrow_mut()[0].rect = selected[0].1.before;
        selected[0].1.target.height = 681.0;
        selected[1].1.target.height = 681.0;
        selected[1].1.before.x = 0.0;
        selected[1].0.fleet.borrow_mut()[1].rect = selected[1].1.before;
        let (_, mut op) = apply_on_displays(&selected, &displays, &[]).unwrap();
        assert_eq!(op.changes.len(), 1);
        assert_eq!(op.changes[0].id, "0");
        let undone = undo_on_displays(&mut op, &[], &displays).unwrap();
        assert_eq!(undone.windows[0].status, Status::Restored);
        assert!(!undone.undo_available);
        assert_eq!(selected[0].0.fleet.borrow()[0].rect, selected[0].1.before);
        assert_eq!(selected[1].0.fleet.borrow()[1].rect, selected[1].1.before);
    }
    #[test]
    fn failed_stage_is_persisted_as_actual_geometry_and_remains_live_undoable() {
        let (mut selected, displays) = swap_fixture();
        let fault = Rc::new(std::cell::Cell::new(true));
        selected[1].0.fail_write = Some(fault.clone());
        let sandbox = crate::testutil::Sandbox::new("window-swap-stage-history");
        let path = sandbox.path().join("window-history.json");
        let mut store = crate::window_layout_service::Store::with_journal(
            crate::window_layout_journal::Store::new(path.clone()),
        );
        store
            .replace(
                displays.clone(),
                selected
                    .iter()
                    .map(|(h, _)| (h.inspect().ok().unwrap(), h.clone()))
                    .collect(),
                vec![],
            )
            .unwrap();
        let preview = store
            .preview(
                &["1".into(), "0".into()],
                "d",
                crate::window_layout::Template::SideBySide,
                12.0,
                1,
                100,
            )
            .unwrap();
        let result = store
            .apply_preview(&preview.preview_id, 1, &displays, 101)
            .unwrap();
        assert!(result.undo_available);
        assert_eq!(result.windows[0].status, Status::Failed);
        let history = crate::window_layout_journal::Store::new(path)
            .list()
            .unwrap();
        let record = &history.items[0];
        assert_eq!(record.slots[0].before.x, 518.0);
        assert_eq!(record.slots[0].target.x, 0.0);
        assert_eq!(record.slots[0].actual.unwrap().x, 514.0);
        assert_eq!(
            record.slots[0].outcome,
            Some(crate::window_layout_journal::Outcome::Failed)
        );
        assert_eq!(
            record.slots[1].outcome,
            Some(crate::window_layout_journal::Outcome::Skipped)
        );
        fault.set(false);
        assert!(
            !store
                .undo_on_displays(&result.operation_id, &[], &displays)
                .unwrap()
                .undo_available
        );
    }
    #[test]
    fn failed_undo_that_reaches_raw_target_still_requires_verified_success() {
        let (selected, displays) = swap_fixture();
        let (_, mut op) = apply_on_displays(&selected, &displays, &[]).unwrap();
        let fault = Rc::new(std::cell::Cell::new(true));
        // Undo stages window 0, then the final write of window 1 fails after
        // physically reaching its original rectangle.
        op.changes[1].handle.fail_write = Some(fault.clone());
        let result = undo_on_displays(&mut op, &[], &displays).unwrap();
        assert!(result.windows.iter().all(|r| r.status == Status::Failed));
        assert!(result.undo_available);
        assert_eq!(result.windows[1].actual_rect.unwrap(), selected[1].1.before);
        fault.set(false);
        let retry = undo_on_displays(&mut op, &[], &displays).unwrap();
        assert!(retry.windows.iter().all(|r| r.status == Status::Restored));
        assert!(!retry.undo_available);
    }

    #[test]
    fn near_overlapping_windows_arrange_without_blocking_the_second_identity() {
        let before = [
            Rect {
                x: 0.0,
                y: 30.0,
                width: 1024.0,
                height: 678.0,
            },
            Rect {
                x: 0.0,
                y: 30.0,
                width: 1024.0,
                height: 680.0,
            },
        ];
        let fleet = Rc::new(RefCell::new(
            before
                .iter()
                .enumerate()
                .map(|(i, rect)| WindowCandidate {
                    window_id: i.to_string(),
                    agent_id: "vscode".into(),
                    application: "VS Code".into(),
                    title: format!("Window-{i}"),
                    screen_id: None,
                    rect: *rect,
                    movable: true,
                    resizable: true,
                    restriction: None,
                    minimum_size: None,
                })
                .collect::<Vec<_>>(),
        ));
        let writes = Rc::new(RefCell::new(Vec::new()));
        let selected: Vec<_> = before
            .iter()
            .enumerate()
            .map(|(index, before)| {
                (
                    Window {
                        fleet: fleet.clone(),
                        index,
                        writes: writes.clone(),
                        fail_write: None,
                        block_after_stage: None,
                    },
                    crate::window_layout::Placement {
                        window_id: index.to_string(),
                        before: *before,
                        target: Rect {
                            x: index as f64 * 518.0,
                            y: 30.0,
                            width: 506.0,
                            height: 681.0,
                        },
                        restriction: None,
                    },
                )
            })
            .collect();
        let (result, mut operation) = apply(&selected).unwrap();
        assert_eq!(
            result.windows.iter().map(|r| &r.status).collect::<Vec<_>>(),
            vec![&Status::Applied, &Status::Applied]
        );
        assert_eq!(*writes.borrow(), vec![1, 0]);
        assert_eq!(
            result
                .windows
                .iter()
                .map(|r| r.window_id.as_str())
                .collect::<Vec<_>>(),
            vec!["0", "1"]
        );
        assert_eq!(
            operation
                .changes
                .iter()
                .map(|c| c.id.as_str())
                .collect::<Vec<_>>(),
            vec!["0", "1"]
        );
        let undone = undo(&mut operation, &[]).unwrap();
        assert!(undone.windows.iter().all(|r| r.status == Status::Restored));
        assert!(!undone.undo_available);
        assert_eq!(
            fleet.borrow().iter().map(|w| w.rect).collect::<Vec<_>>(),
            before
        );
    }

    #[test]
    fn cyclic_swap_and_undo_restore_exact_geometry_without_losing_identity() {
        let rect = |x| Rect {
            x,
            y: 30.0,
            width: 506.0,
            height: 680.0,
        };
        let fleet = Rc::new(RefCell::new(
            [0.0, 518.0]
                .iter()
                .enumerate()
                .map(|(i, x)| WindowCandidate {
                    window_id: i.to_string(),
                    agent_id: "vscode".into(),
                    application: "VS Code".into(),
                    title: String::new(),
                    screen_id: None,
                    rect: rect(*x),
                    movable: true,
                    resizable: true,
                    restriction: None,
                    minimum_size: None,
                })
                .collect::<Vec<_>>(),
        ));
        let writes = Rc::new(RefCell::new(Vec::new()));
        let selected: Vec<_> = (0..2)
            .map(|i| {
                (
                    Window {
                        fleet: fleet.clone(),
                        index: i,
                        writes: writes.clone(),
                        fail_write: None,
                        block_after_stage: None,
                    },
                    crate::window_layout::Placement {
                        window_id: i.to_string(),
                        before: rect(i as f64 * 518.0),
                        target: rect((1 - i) as f64 * 518.0),
                        restriction: None,
                    },
                )
            })
            .collect();
        let displays = vec![crate::window_layout::DisplayArea {
            screen_id: "d".into(),
            scale: 1.0,
            rect: Rect {
                x: 0.0,
                y: 30.0,
                width: 1024.0,
                height: 681.0,
            },
        }];
        let (result, mut operation) = apply_on_displays(&selected, &displays, &[]).unwrap();
        assert_eq!(
            result.windows.iter().map(|r| &r.status).collect::<Vec<_>>(),
            vec![&Status::Applied, &Status::Applied]
        );
        assert!(result.undo_available);
        assert_eq!(operation.changes.len(), 2);
        assert_eq!(fleet.borrow()[0].rect, rect(518.0));
        assert_eq!(fleet.borrow()[1].rect, rect(0.0));
        let undone = undo_on_displays(&mut operation, &[], &displays).unwrap();
        assert!(undone.windows.iter().all(|r| r.status == Status::Restored));
        assert!(!undone.undo_available);
        assert_eq!(fleet.borrow()[0].rect, rect(0.0));
        assert_eq!(fleet.borrow()[1].rect, rect(518.0));
    }
}
