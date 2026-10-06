//! Native-operation seam: verify all selected objects before writing any, report each result.
use crate::window_layout::{Rect, WindowCandidate};
use serde::Serialize;

pub trait Handle: Clone {
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

pub fn apply<H: Handle>(
    selected: &[(H, crate::window_layout::Placement)],
) -> Result<(ResultDto, Operation<H>), String> {
    // A single preflight failure prevents all writes; never mix stale and current selection.
    for (h, p) in selected {
        let c = h.inspect().map_err(InspectError::reason)?;
        if c.window_id != p.window_id {
            return Err("窗口身份已变化，请重新读取".into());
        }
        verify(&c, p.before, p.target)?;
    }
    let mut op = Operation {
        id: uuid::Uuid::new_v4().to_string(),
        changes: Vec::new(),
        conflicts: std::collections::HashSet::new(),
    };
    let mut rows = Vec::new();
    for (h, p) in selected {
        let check = h.inspect().map_err(InspectError::reason).and_then(|c| {
            if c.window_id != p.window_id {
                Err("窗口身份已变化".into())
            } else {
                verify(&c, p.before, p.target)
            }
        });
        if let Err(reason) = check {
            rows.push(Row {
                window_id: p.window_id.clone(),
                status: Status::Skipped,
                reason: Some(reason),
                actual_rect: None,
            });
            continue;
        }
        if matches(p.before, p.target) {
            rows.push(Row {
                window_id: p.window_id.clone(),
                status: Status::Skipped,
                reason: Some("窗口已在目标位置".into()),
                actual_rect: Some(p.before),
            });
            continue;
        }
        let mut result = h.write(p.target);
        result.actual = result.actual.filter(|r| r.valid());
        if let Some(actual) = result.actual.filter(|r| r.valid()) {
            if !matches(actual, p.before) {
                op.changes.push(Change {
                    id: p.window_id.clone(),
                    handle: h.clone(),
                    before: p.before,
                    actual,
                });
            }
        }
        let applied = result.error.is_none() && result.actual.is_some_and(|a| matches(a, p.target));
        let reason = if applied {
            None
        } else {
            result.error.or_else(|| {
                Some(
                    if result.actual.is_none() {
                        "调整后无法读取位置，请在工具中核对"
                    } else {
                        "工具未采用目标尺寸，请在工具中核对"
                    }
                    .into(),
                )
            })
        };
        rows.push(Row {
            window_id: p.window_id.clone(),
            status: if applied {
                Status::Applied
            } else {
                Status::Failed
            },
            reason,
            actual_rect: result.actual,
        });
    }
    let dto = ResultDto {
        operation_id: op.id.clone(),
        windows: rows,
        undo_available: !op.changes.is_empty(),
        record_id: None,
        record_warning: None,
    };
    Ok((dto, op))
}

pub fn undo<H: Handle>(op: &mut Operation<H>, force_ids: &[String]) -> Result<ResultDto, String> {
    if force_ids.iter().any(|id| !op.conflicts.contains(id)) {
        return Err("请先查看撤销冲突，再选择要恢复的窗口".into());
    }
    let mut rows = Vec::new();
    let mut remaining = Vec::new();
    for change in op.changes.drain(..) {
        let current = match change.handle.inspect() {
            Ok(c) => c,
            Err(InspectError::Gone(reason)) => {
                op.conflicts.remove(&change.id);
                rows.push(Row {
                    window_id: change.id,
                    status: Status::Skipped,
                    reason: Some(reason),
                    actual_rect: None,
                });
                continue;
            }
            Err(InspectError::Unavailable(reason)) => {
                rows.push(Row {
                    window_id: change.id.clone(),
                    status: Status::Failed,
                    reason: Some(reason),
                    actual_rect: None,
                });
                remaining.push(change);
                continue;
            }
        };
        if current.window_id != change.id {
            op.conflicts.remove(&change.id);
            rows.push(Row {
                window_id: change.id,
                status: Status::Skipped,
                reason: Some("窗口身份已变化".into()),
                actual_rect: None,
            });
            continue;
        }
        if matches(current.rect, change.before) {
            op.conflicts.remove(&change.id);
            rows.push(Row {
                window_id: change.id,
                status: Status::Restored,
                reason: Some("窗口已恢复".into()),
                actual_rect: Some(current.rect),
            });
            continue;
        }
        if !matches(current.rect, change.actual) && !force_ids.contains(&change.id) {
            op.conflicts.insert(change.id.clone());
            rows.push(Row {
                window_id: change.id.clone(),
                status: Status::Conflict,
                reason: Some("窗口之后被调整，确认后才能覆盖恢复".into()),
                actual_rect: Some(current.rect),
            });
            remaining.push(change);
            continue;
        }
        if let Err(reason) = verify(&current, current.rect, change.before) {
            rows.push(Row {
                window_id: change.id.clone(),
                status: Status::Failed,
                reason: Some(reason),
                actual_rect: Some(current.rect),
            });
            remaining.push(change);
            continue;
        }
        let mut result = change.handle.write(change.before);
        result.actual = result.actual.filter(|r| r.valid());
        let restored =
            result.error.is_none() && result.actual.is_some_and(|r| matches(r, change.before));
        rows.push(Row {
            window_id: change.id.clone(),
            status: if restored {
                Status::Restored
            } else {
                Status::Failed
            },
            reason: if restored {
                None
            } else {
                result
                    .error
                    .or_else(|| Some("未能恢复原位置，请核对窗口".into()))
            },
            actual_rect: result.actual,
        });
        if restored {
            op.conflicts.remove(&change.id);
        } else {
            // Preserve the last observable result of a partial restore, never the original target.
            let actual = result.actual.unwrap_or(change.actual);
            remaining.push(Change { actual, ..change });
        }
    }
    op.changes = remaining;
    Ok(ResultDto {
        operation_id: op.id.clone(),
        windows: rows,
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
