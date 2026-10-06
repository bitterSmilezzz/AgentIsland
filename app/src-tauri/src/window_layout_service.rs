//! Server-owned, short-lived selection/preview broker; native objects never enter IPC.
use crate::window_layout::{self, DisplayArea, Geometry, Template, WindowCandidate};
use serde::Serialize;
use std::{collections::HashMap, time::Instant};

#[derive(Clone, Serialize)]
pub struct Snapshot {
    pub revision: u64,
    pub displays: Vec<DisplayArea>,
    pub windows: Vec<WindowCandidate>,
    pub warnings: Vec<String>,
}
#[derive(Clone, Serialize)]
pub struct Preview {
    pub preview_id: String,
    pub revision: u64,
    pub created_ms: u64,
    pub expires_ms: u64,
    pub geometry: Geometry,
}
pub struct Store<H> {
    pub handles: HashMap<String, H>,
    snapshot: Option<Snapshot>,
    read_at: Option<Instant>,
    preview: Option<Preview>,
    preview_at: Option<Instant>,
    operation: Option<crate::window_layout_execution::Operation<H>>,
    journal: Option<crate::window_layout_journal::Store>,
    recovery: Option<(String, String)>,
}
impl<H> Default for Store<H> {
    fn default() -> Self {
        Self {
            handles: HashMap::new(),
            snapshot: None,
            read_at: None,
            preview: None,
            preview_at: None,
            operation: None,
            journal: None,
            recovery: None,
        }
    }
}

impl<H: crate::window_layout_execution::Handle> Store<H> {
    pub fn with_journal(journal: crate::window_layout_journal::Store) -> Self {
        Self {
            journal: Some(journal),
            ..Self::default()
        }
    }
    pub fn apply_preview(
        &mut self,
        preview_id: &str,
        expected_revision: u64,
        current_displays: &[DisplayArea],
        now_ms: u64,
    ) -> Result<crate::window_layout_execution::ResultDto, String> {
        self.apply_scoped(
            preview_id,
            expected_revision,
            current_displays,
            now_ms,
            None,
            None,
        )
    }
    pub fn apply_scoped(
        &mut self,
        preview_id: &str,
        expected_revision: u64,
        current_displays: &[DisplayArea],
        now_ms: u64,
        context: Option<&crate::window_layout_journal::WorkspaceContext>,
        rule: Option<&crate::window_layout_rules::Rule>,
    ) -> Result<crate::window_layout_execution::ResultDto, String> {
        let p = self.preview.as_ref().ok_or("请先预览窗口布局")?;
        if let Some(context) = context {
            let rule = rule
                .filter(|r| r.id == context.layout_id)
                .ok_or("组合布局已失效")?;
            if self.recovery.is_none() {
                let snapshot = self.snapshot.as_ref().ok_or("窗口列表已失效")?;
                let tools = p
                    .geometry
                    .placements
                    .iter()
                    .map(|p| {
                        snapshot
                            .windows
                            .iter()
                            .find(|w| w.window_id == p.window_id)
                            .map(|w| w.agent_id.clone())
                            .ok_or("所选工具已变化")
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                if matches!(
                    rule.screen_preference,
                    crate::window_layout_rules::ScreenPreference::Primary
                ) && snapshot
                    .displays
                    .first()
                    .is_none_or(|d| d.screen_id != p.geometry.screen_id)
                {
                    return Err("预览屏幕与组合的主屏偏好不一致".into());
                }
                if p.geometry.template != rule.template
                    || p.geometry.gap != rule.gap
                    || tools != rule.tools
                {
                    return Err("当前预览与组合布局不一致，请重新载入".into());
                }
            }
        }
        if p.preview_id != preview_id || p.revision != expected_revision {
            return Err("预览已变化，请重新预览".into());
        }
        let p = self.preview.take().unwrap();
        let recovery = self.recovery.take();
        if let Some((_, revision)) = &recovery {
            if self.journal.as_ref().ok_or("历史不可用")?.list()?.revision != *revision {
                return Err("历史已变化，请刷新后重新预览".into());
            }
        }
        let at = self.preview_at.take();
        if now_ms < p.created_ms
            || now_ms >= p.expires_ms
            || at.is_none_or(|t| t.elapsed().as_secs() >= 15)
        {
            return Err("预览已过期，请重新预览".into());
        }
        let snapshot = self.snapshot.as_ref().ok_or("窗口列表已失效")?;
        if snapshot.revision != expected_revision || !p.geometry.applicable {
            return Err("预览不可应用，请检查窗口限制".into());
        }
        let previous = snapshot
            .displays
            .iter()
            .find(|d| d.screen_id == p.geometry.screen_id)
            .ok_or("目标屏幕已失效")?;
        let current = current_displays
            .iter()
            .find(|d| d.screen_id == p.geometry.screen_id)
            .ok_or("屏幕已变化，请重新读取并预览")?;
        if previous.scale != current.scale || previous.rect != current.rect {
            return Err("屏幕工作区已变化，请重新预览".into());
        }
        let selected: Result<Vec<_>, String> = p
            .geometry
            .placements
            .into_iter()
            .map(|placement| {
                self.handles
                    .get(&placement.window_id)
                    .cloned()
                    .map(|h| (h, placement))
                    .ok_or_else(|| "窗口身份已失效，请重新读取".into())
            })
            .collect();
        let selected = selected?;
        let intents = selected
            .iter()
            .map(|(_, placement)| {
                let window = snapshot
                    .windows
                    .iter()
                    .find(|w| w.window_id == placement.window_id)
                    .ok_or("排列来源窗口已变化")?;
                Ok(crate::window_layout_journal::Input {
                    window_id: placement.window_id.clone(),
                    agent_id: window.agent_id.clone(),
                    before: placement.before,
                    target: placement.target,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let ticket = self
            .journal
            .as_ref()
            .map(|journal| {
                journal.begin_scoped(
                    &intents,
                    recovery.as_ref().map(|(id, _)| id.as_str()),
                    i64::try_from(now_ms).map_err(|_| "记录时间无效")?,
                    recovery.as_ref().map(|(_, rev)| rev.as_str()),
                    context,
                )
            })
            .transpose()?;
        let applied = crate::window_layout_execution::apply(&selected);
        let (mut result, operation) = match applied {
            Ok(value) => value,
            Err(error) => {
                if let (Some(journal), Some(ticket)) = (&self.journal, &ticket) {
                    if journal.finish(ticket, None).is_err() {
                        return Err(format!("{error}；意图记录未完成结算，请核对历史"));
                    }
                }
                return Err(error);
            }
        };
        self.operation = Some(operation);
        if let (Some(journal), Some(ticket)) = (&self.journal, &ticket) {
            match journal.finish(ticket,Some(&result)){
            Ok(record)=>result.record_id=Some(record.id),
            Err(_)=>result.record_warning=Some("窗口已执行，历史记录未完整结算；保留本次撤销入口，请核对窗口和待核对记录，不重复排列。".into()),
        }
        }
        Ok(result)
    }
    pub fn undo_operation(
        &mut self,
        operation_id: &str,
        force_ids: &[String],
    ) -> Result<crate::window_layout_execution::ResultDto, String> {
        let operation = self
            .operation
            .as_mut()
            .filter(|o| o.id == operation_id)
            .ok_or("此操作已失效，仅可撤销最近一次排列")?;
        crate::window_layout_execution::undo(operation, force_ids)
    }
    pub fn undo_on_displays(
        &mut self,
        operation_id: &str,
        force_ids: &[String],
        displays: &[DisplayArea],
    ) -> Result<crate::window_layout_execution::ResultDto, String> {
        use crate::window_layout_execution::{Row, Status};
        let operation = self
            .operation
            .as_mut()
            .filter(|o| o.id == operation_id)
            .ok_or("此操作已失效，仅可撤销最近一次排列")?;
        if force_ids.iter().any(|id| !operation.conflicts.contains(id)) {
            return Err("请先查看撤销冲突，再选择要恢复的窗口".into());
        }
        let (allowed, blocked): (Vec<_>, Vec<_>) = operation.changes.drain(..).partition(|c| {
            displays.iter().any(|d| {
                let a = d.rect;
                let b = c.before;
                (a.x + a.width).min(b.x + b.width) > a.x.max(b.x)
                    && (a.y + a.height).min(b.y + b.height) > a.y.max(b.y)
            })
        });
        operation.changes = allowed;
        let filtered: Vec<_> = force_ids
            .iter()
            .filter(|id| operation.changes.iter().any(|c| &c.id == *id))
            .cloned()
            .collect();
        let mut result = crate::window_layout_execution::undo(operation, &filtered)?;
        for c in blocked {
            result.windows.push(Row {
                window_id: c.id.clone(),
                status: Status::Failed,
                reason: Some("原位置所在屏幕不可用，重新连接后可重试".into()),
                actual_rect: None,
            });
            operation.changes.push(c);
        }
        result.undo_available = !operation.changes.is_empty();
        Ok(result)
    }
}
impl<H> Store<H> {
    pub fn rule_selection(
        &self,
        selection: &[String],
        screen_id: &str,
        expected_revision: u64,
    ) -> Result<(Vec<String>, crate::window_layout_rules::ScreenPreference), String> {
        let snapshot = self.snapshot.as_ref().ok_or("请先读取工具窗口")?;
        if snapshot.revision != expected_revision
            || self.read_at.is_none_or(|t| t.elapsed().as_secs() >= 60)
        {
            return Err("窗口列表已变化或过期，请重新读取".into());
        }
        let screen = snapshot
            .displays
            .iter()
            .position(|d| d.screen_id == screen_id)
            .ok_or("请选择目标屏幕")?;
        let mut seen = std::collections::HashSet::new();
        let tools: Result<Vec<_>, String> = selection
            .iter()
            .map(|id| {
                if !seen.insert(id) {
                    return Err("不能重复选择窗口".into());
                }
                snapshot
                    .windows
                    .iter()
                    .find(|w| &w.window_id == id)
                    .map(|w| w.agent_id.clone())
                    .ok_or_else(|| "所选窗口已失效".into())
            })
            .collect();
        Ok((
            tools?,
            if screen == 0 {
                crate::window_layout_rules::ScreenPreference::Primary
            } else {
                crate::window_layout_rules::ScreenPreference::Choose
            },
        ))
    }
    pub fn resolve_rule(
        &self,
        rule: crate::window_layout_rules::Rule,
        expected_revision: u64,
    ) -> Result<crate::window_layout_rules::Resolved, String> {
        let snapshot = self.snapshot.as_ref().ok_or("请先读取工具窗口")?;
        if snapshot.revision != expected_revision
            || self.read_at.is_none_or(|t| t.elapsed().as_secs() >= 60)
        {
            return Err("窗口列表已变化或过期，请重新读取".into());
        }
        if !snapshot.warnings.is_empty() {
            return Ok(crate::window_layout_rules::Resolved {
                rule,
                selection: Vec::new(),
                needs_selection: true,
                reason: Some("窗口读取不完整，请重新选择并核对".into()),
            });
        }
        Ok(crate::window_layout_rules::resolve(rule, &snapshot.windows))
    }
    pub fn replace(
        &mut self,
        displays: Vec<DisplayArea>,
        windows: Vec<(WindowCandidate, H)>,
        warnings: Vec<String>,
    ) -> Result<Snapshot, String> {
        let revision = self
            .snapshot
            .as_ref()
            .map_or(0, |s| s.revision)
            .checked_add(1)
            .ok_or("窗口读取版本已耗尽，请重启应用")?;
        let mut handles = HashMap::new();
        let mut candidates = Vec::new();
        for (mut candidate, handle) in windows {
            if candidate.window_id.is_empty() || handles.contains_key(&candidate.window_id) {
                return Err("窗口读取身份重复，请重新读取".into());
            }
            candidate.screen_id = displays
                .iter()
                .filter_map(|d| {
                    let r = candidate.rect;
                    let a = d.rect;
                    let overlap = ((r.x + r.width).min(a.x + a.width) - r.x.max(a.x)).max(0.0)
                        * ((r.y + r.height).min(a.y + a.height) - r.y.max(a.y)).max(0.0);
                    (overlap > 0.0).then_some((d, overlap))
                })
                .max_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(d, _)| d.screen_id.clone());
            handles.insert(candidate.window_id.clone(), handle);
            candidates.push(candidate);
        }
        let snapshot = Snapshot {
            revision,
            displays,
            windows: candidates,
            warnings,
        };
        self.handles = handles;
        self.snapshot = Some(snapshot.clone());
        self.read_at = Some(Instant::now());
        self.preview = None;
        self.preview_at = None;
        self.recovery = None;
        Ok(snapshot)
    }
    /// A historical slot is explicitly rebound to a fresh window of the same tool.
    pub fn preview_recovery(
        &mut self,
        record_id: &str,
        slot_id: &str,
        window_id: &str,
        history_revision: &str,
        expected_revision: u64,
        now_ms: u64,
    ) -> Result<Preview, String> {
        self.preview = None;
        self.preview_at = None;
        self.recovery = None;
        let history = self.journal.as_ref().ok_or("历史不可用")?.list()?;
        if history.revision != history_revision {
            return Err("历史已变化，请刷新后核对".into());
        }
        let record = history
            .items
            .iter()
            .find(|r| r.id == record_id)
            .ok_or("历史记录已不存在")?;
        let slot = record
            .slots
            .iter()
            .find(|s| s.id == slot_id)
            .ok_or("历史位置已不存在")?;
        let snapshot = self.snapshot.as_ref().ok_or("请先读取当前窗口")?;
        if snapshot.revision != expected_revision
            || self.read_at.is_none_or(|t| t.elapsed().as_secs() >= 60)
        {
            return Err("窗口列表已变化或过期，请重新读取".into());
        }
        let window = snapshot
            .windows
            .iter()
            .find(|w| w.window_id == window_id)
            .ok_or("请重新选择当前窗口")?;
        if window.agent_id != slot.agent_id {
            return Err("所选窗口与历史工具不一致".into());
        }
        let target = slot.before;
        let display = snapshot
            .displays
            .iter()
            .find(|d| {
                let a = d.rect;
                a.valid()
                    && d.scale.is_finite()
                    && d.scale > 0.0
                    && target.x >= a.x
                    && target.y >= a.y
                    && target.x + target.width <= a.x + a.width
                    && target.y + target.height <= a.y + a.height
            })
            .ok_or("历史位置超出当前屏幕工作区，请连接原屏幕或重新排列")?;
        let restriction = crate::window_layout_execution::verify(window, window.rect, target).err();
        let geometry = Geometry {
            screen_id: display.screen_id.clone(),
            template: Template::Grid,
            gap: 0.0,
            applicable: restriction.is_none(),
            placements: vec![window_layout::Placement {
                window_id: window.window_id.clone(),
                before: window.rect,
                target,
                restriction,
            }],
        };
        let preview = Preview {
            preview_id: uuid::Uuid::new_v4().to_string(),
            revision: snapshot.revision,
            created_ms: now_ms,
            expires_ms: now_ms.checked_add(15_000).ok_or("预览时间无效")?,
            geometry,
        };
        self.recovery = Some((record.id.clone(), history.revision));
        self.preview = Some(preview.clone());
        self.preview_at = Some(Instant::now());
        Ok(preview)
    }
    pub fn preview(
        &mut self,
        selection: &[String],
        screen_id: &str,
        template: Template,
        gap: f64,
        expected_revision: u64,
        now_ms: u64,
    ) -> Result<Preview, String> {
        self.preview = None;
        self.preview_at = None;
        self.recovery = None;
        let snapshot = self.snapshot.as_ref().ok_or("请先读取工具窗口")?;
        if snapshot.revision != expected_revision
            || self.read_at.is_none_or(|t| t.elapsed().as_secs() >= 60)
        {
            return Err("窗口列表已更新或过期，请重新读取".into());
        }
        let display = snapshot
            .displays
            .iter()
            .find(|d| d.screen_id == screen_id)
            .ok_or("目标屏幕已失效，请重新读取")?;
        let windows: Result<Vec<_>, _> = selection
            .iter()
            .map(|id| {
                snapshot
                    .windows
                    .iter()
                    .find(|w| &w.window_id == id)
                    .cloned()
                    .ok_or("所选窗口已失效，请重新读取")
            })
            .collect();
        let geometry = window_layout::preview_geometry(display, &windows?, template, gap)?;
        let expires_ms = now_ms.checked_add(15_000).ok_or("预览时间无效")?;
        let preview = Preview {
            preview_id: uuid::Uuid::new_v4().to_string(),
            revision: snapshot.revision,
            created_ms: now_ms,
            expires_ms,
            geometry,
        };
        self.preview = Some(preview.clone());
        self.preview_at = Some(Instant::now());
        Ok(preview)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::window_layout::Rect;
    fn fixture() -> (Vec<DisplayArea>, Vec<(WindowCandidate, ())>) {
        let r = Rect {
            x: -1200.0,
            y: 24.0,
            width: 1200.0,
            height: 800.0,
        };
        (
            vec![DisplayArea {
                screen_id: "screen".into(),
                rect: r,
                scale: 2.0,
            }],
            (0..2)
                .map(|i| {
                    (
                        WindowCandidate {
                            window_id: format!("id{i}"),
                            agent_id: "tool".into(),
                            application: "Tool".into(),
                            title: "duplicate title".into(),
                            screen_id: None,
                            rect: r,
                            movable: true,
                            resizable: true,
                            restriction: None,
                            minimum_size: None,
                        },
                        (),
                    )
                })
                .collect(),
        )
    }
    #[test]
    fn only_server_snapshot_ids_can_be_previewed_and_order_is_explicit() {
        let mut s = Store::default();
        let (d, w) = fixture();
        let snapshot = s.replace(d, w, vec![]).unwrap();
        assert_eq!(snapshot.windows[0].screen_id.as_deref(), Some("screen"));
        let p = s
            .preview(
                &["id1".into(), "id0".into()],
                "screen",
                Template::SideBySide,
                10.0,
                snapshot.revision,
                500,
            )
            .unwrap();
        assert_eq!(p.geometry.placements[0].window_id, "id1");
        assert_eq!(p.expires_ms, 15_500);
        assert!(s
            .preview(
                &["invented".into()],
                "screen",
                Template::Grid,
                0.0,
                snapshot.revision,
                500
            )
            .is_err());
        assert!(s
            .preview(
                &["id0".into()],
                "other",
                Template::Grid,
                0.0,
                snapshot.revision,
                500
            )
            .is_err());
    }
    #[test]
    fn refresh_invalidates_old_selection_revision_and_preview() {
        let mut s = Store::default();
        let (d, w) = fixture();
        s.replace(d, w, vec![]).unwrap();
        s.preview(&["id0".into()], "screen", Template::Grid, 0.0, 1, 10)
            .unwrap();
        let (d, w) = fixture();
        s.replace(d, w, vec![]).unwrap();
        assert!(s.preview.is_none());
        assert!(s
            .preview(&["id0".into()], "screen", Template::Grid, 0.0, 1, 10)
            .is_err());
    }
    #[test]
    fn stale_snapshot_and_time_overflow_do_not_issue_previews() {
        let mut s = Store::default();
        let (d, w) = fixture();
        s.replace(d, w, vec![]).unwrap();
        assert!(s
            .preview(&["id0".into()], "screen", Template::Grid, 0.0, 1, u64::MAX)
            .is_err());
        s.read_at = Instant::now().checked_sub(std::time::Duration::from_secs(61));
        assert!(s
            .preview(&["id0".into()], "screen", Template::Grid, 0.0, 1, 10)
            .is_err());
    }
    #[test]
    fn incomplete_enumeration_never_autoselects_a_preset_window() {
        let mut s = Store::default();
        let (d, mut w) = fixture();
        w.truncate(1);
        w[0].0.agent_id = "codex".into();
        s.replace(d, w, vec!["部分窗口未读取".into()]).unwrap();
        let rule = crate::window_layout_rules::Rule {
            id: uuid::Uuid::new_v4().to_string(),
            name: "layout".into(),
            tools: vec!["codex".into()],
            template: Template::Grid,
            gap: 12.0,
            screen_preference: crate::window_layout_rules::ScreenPreference::Primary,
        };
        let resolved = s.resolve_rule(rule, 1).unwrap();
        assert!(resolved.needs_selection);
        assert!(resolved.selection.is_empty());
        assert!(s.rule_selection(&["invented".into()], "screen", 1).is_err());
        assert!(s
            .rule_selection(&["id0".into(), "id0".into()], "screen", 1)
            .is_err());
    }
}
