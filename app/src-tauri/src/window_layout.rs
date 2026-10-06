//! External-tool layout geometry in logical desktop points. No native handles or mutations.
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

const MAX_WINDOWS: usize = 16;
const MAX_GAP: f64 = 64.0;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}
impl Rect {
    pub(super) fn valid(self) -> bool {
        [
            self.x,
            self.y,
            self.width,
            self.height,
            self.x + self.width,
            self.y + self.height,
        ]
        .into_iter()
        .all(f64::is_finite)
            && self.width > 0.0
            && self.height > 0.0
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DisplayArea {
    pub screen_id: String,
    /// Available area already excludes the menu bar, Dock or taskbar.
    pub rect: Rect,
    pub scale: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WindowCandidate {
    /// Ephemeral server-owned identity; never a title, PID or frontend AX pointer.
    pub window_id: String,
    pub agent_id: String,
    pub application: String,
    pub title: String,
    pub screen_id: Option<String>,
    pub rect: Rect,
    pub movable: bool,
    pub resizable: bool,
    pub restriction: Option<String>,
    /// Only a verified native constraint is supplied, never a guessed app minimum.
    pub minimum_size: Option<[f64; 2]>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Template {
    SideBySide,
    MainAndTwo,
    Grid,
}

#[derive(Clone, Debug, Serialize)]
pub struct Placement {
    pub window_id: String,
    pub before: Rect,
    pub target: Rect,
    pub restriction: Option<String>,
}
#[derive(Clone, Debug, Serialize)]
pub struct Geometry {
    pub screen_id: String,
    pub template: Template,
    pub gap: f64,
    pub placements: Vec<Placement>,
    pub applicable: bool,
}

/// Preserves explicit selection order, including the primary window at index zero.
pub fn preview_geometry(
    display: &DisplayArea,
    windows: &[WindowCandidate],
    template: Template,
    gap: f64,
) -> Result<Geometry, String> {
    if display.screen_id.is_empty()
        || !display.rect.valid()
        || !display.scale.is_finite()
        || display.scale <= 0.0
    {
        return Err("屏幕工作区无效，请重新读取屏幕".into());
    }
    if !gap.is_finite() || !(0.0..=MAX_GAP).contains(&gap) {
        return Err("窗口间距应在 0–64 点之间".into());
    }
    if windows.is_empty() || windows.len() > MAX_WINDOWS {
        return Err("请选择 1–16 个窗口".into());
    }
    match template {
        Template::SideBySide if windows.len() != 2 => return Err("左右并排需要 2 个窗口".into()),
        Template::MainAndTwo if windows.len() != 3 => return Err("主窗与辅窗需要 3 个窗口".into()),
        _ => {}
    }
    let mut ids = HashSet::new();
    for w in windows {
        if w.window_id.is_empty()
            || !ids.insert(&w.window_id)
            || !w.rect.valid()
            || w.minimum_size
                .is_some_and(|size| size.into_iter().any(|v| !v.is_finite() || v <= 0.0))
        {
            return Err("窗口信息已失效，请重新选择".into());
        }
    }
    let area = display.rect;
    let targets = match template {
        Template::SideBySide => {
            let spans = split(area.x, area.width, 2, gap)?;
            spans
                .into_iter()
                .map(|(x, width)| Rect { x, width, ..area })
                .collect()
        }
        Template::MainAndTwo => {
            let width = area.width - gap;
            let height = area.height - gap;
            if width <= 0.0 || height <= 0.0 {
                return Err("屏幕空间不足，请减小间距".into());
            }
            let main_width = width * 0.6;
            let side_x = area.x + main_width + gap;
            let side_width = area.x + area.width - side_x;
            let spans = split(area.y, area.height, 2, gap)?;
            vec![
                Rect {
                    width: main_width,
                    ..area
                },
                Rect {
                    x: side_x,
                    y: spans[0].0,
                    width: side_width,
                    height: spans[0].1,
                },
                Rect {
                    x: side_x,
                    y: spans[1].0,
                    width: side_width,
                    height: spans[1].1,
                },
            ]
        }
        Template::Grid => {
            let columns = ((windows.len() as f64 * area.width / area.height)
                .sqrt()
                .ceil() as usize)
                .clamp(1, windows.len());
            let rows = windows.len().div_ceil(columns);
            let ys = split(area.y, area.height, rows, gap)?;
            let mut cells = Vec::with_capacity(windows.len());
            for (row, (y, height)) in ys.into_iter().enumerate() {
                // The last row uses all available width; never leave a selected window out.
                let count = columns.min(windows.len() - row * columns);
                for (x, width) in split(area.x, area.width, count, gap)? {
                    cells.push(Rect {
                        x,
                        y,
                        width,
                        height,
                    });
                }
            }
            cells
        }
    };
    if targets.len() != windows.len() || targets.iter().any(|r: &Rect| !r.valid()) {
        return Err("屏幕坐标无法计算布局".into());
    }
    let placements: Vec<_> = windows
        .iter()
        .zip(targets)
        .map(|(w, target)| {
            let restriction = w.restriction.clone().filter(|v| !v.is_empty()).or_else(|| {
                if !w.movable && (w.rect.x != target.x || w.rect.y != target.y) {
                    Some("此窗口不能移动".into())
                } else if !w.resizable
                    && (w.rect.width != target.width || w.rect.height != target.height)
                {
                    Some("此窗口不能调整大小".into())
                } else if w
                    .minimum_size
                    .is_some_and(|size| target.width < size[0] || target.height < size[1])
                {
                    Some("目标区域小于窗口最小尺寸".into())
                } else {
                    None
                }
            });
            Placement {
                window_id: w.window_id.clone(),
                before: w.rect,
                target,
                restriction,
            }
        })
        .collect();
    // Native adapters can impose further restrictions; geometry never clears those.
    let applicable = placements.iter().all(|p| p.restriction.is_none());
    Ok(Geometry {
        screen_id: display.screen_id.clone(),
        template,
        gap,
        placements,
        applicable,
    })
}

fn split(origin: f64, extent: f64, count: usize, gap: f64) -> Result<Vec<(f64, f64)>, String> {
    let available = extent - gap * (count - 1) as f64;
    if available <= 0.0 {
        return Err("屏幕空间不足，请减小间距".into());
    }
    let width = available / count as f64;
    let mut spans = Vec::with_capacity(count);
    for i in 0..count {
        let start = origin + (width + gap) * i as f64;
        let end = if i + 1 == count {
            origin + extent
        } else {
            start + width
        };
        if !start.is_finite() || !end.is_finite() || end <= start {
            return Err("屏幕坐标无法计算布局".into());
        }
        spans.push((start, end - start));
    }
    Ok(spans)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn display() -> DisplayArea {
        DisplayArea {
            screen_id: "left".into(),
            rect: Rect {
                x: -1512.0,
                y: 25.0,
                width: 1512.0,
                height: 895.0,
            },
            scale: 2.0,
        }
    }
    fn windows(n: usize) -> Vec<WindowCandidate> {
        (0..n)
            .map(|i| WindowCandidate {
                window_id: format!("opaque-{i}"),
                agent_id: "fixture".into(),
                application: "Tool".into(),
                title: "same title".into(),
                screen_id: None,
                rect: Rect {
                    x: 20.0,
                    y: 50.0,
                    width: 600.0,
                    height: 400.0,
                },
                movable: true,
                resizable: true,
                restriction: None,
                minimum_size: None,
            })
            .collect()
    }
    fn inside(a: Rect, b: Rect) -> bool {
        b.x >= a.x
            && b.y >= a.y
            && b.x + b.width <= a.x + a.width + 1e-8
            && b.y + b.height <= a.y + a.height + 1e-8
    }
    #[test]
    fn negative_display_origin_work_area_and_scale_are_preserved() {
        let d = display();
        let p = preview_geometry(&d, &windows(2), Template::SideBySide, 12.0).unwrap();
        assert!(p.applicable);
        assert_eq!(p.placements[0].target.x, -1512.0);
        assert_eq!(p.placements[0].target.width, 750.0);
        assert_eq!(p.placements[1].target.x, -750.0);
        assert_eq!(p.placements[1].target.x + p.placements[1].target.width, 0.0);
        assert_eq!(p.placements[1].target.y, 25.0);
        let mut other = d.clone();
        other.scale = 1.0;
        assert_eq!(
            preview_geometry(&other, &windows(2), Template::SideBySide, 12.0)
                .unwrap()
                .placements[0]
                .target,
            p.placements[0].target
        );
    }
    #[test]
    fn primary_selection_is_first_and_auxiliaries_use_remaining_space() {
        let d = display();
        let p = preview_geometry(&d, &windows(3), Template::MainAndTwo, 11.0).unwrap();
        assert_eq!(p.placements[0].window_id, "opaque-0");
        assert_eq!(p.placements[0].target.width, (1512.0 - 11.0) * 0.6);
        assert_eq!(
            p.placements[2].target.y + p.placements[2].target.height,
            920.0
        );
        assert!(p.placements.iter().all(|p| inside(d.rect, p.target)));
    }
    #[test]
    fn every_grid_selection_gets_one_nonoverlapping_in_bounds_cell() {
        for n in 1..=MAX_WINDOWS {
            for shape in [(1511.7, 893.3), (500.0, 1700.0)] {
                let mut d = display();
                d.rect.width = shape.0;
                d.rect.height = shape.1;
                let p = preview_geometry(&d, &windows(n), Template::Grid, 9.5).unwrap();
                assert_eq!(p.placements.len(), n);
                for (i, a) in p.placements.iter().enumerate() {
                    assert!(inside(d.rect, a.target));
                    for b in &p.placements[i + 1..] {
                        assert!(
                            a.target.x + a.target.width <= b.target.x + 1e-8
                                || b.target.x + b.target.width <= a.target.x + 1e-8
                                || a.target.y + a.target.height <= b.target.y + 1e-8
                                || b.target.y + b.target.height <= a.target.y + 1e-8
                        );
                    }
                }
            }
        }
    }
    #[test]
    fn constraints_are_explicit_and_never_drop_a_window() {
        let mut ws = windows(2);
        ws[0].resizable = false;
        ws[1].minimum_size = Some([900.0, 500.0]);
        let p = preview_geometry(&display(), &ws, Template::SideBySide, 12.0).unwrap();
        assert!(!p.applicable);
        assert_eq!(p.placements.len(), 2);
        assert_eq!(
            p.placements[0].restriction.as_deref(),
            Some("此窗口不能调整大小")
        );
        assert_eq!(
            p.placements[1].restriction.as_deref(),
            Some("目标区域小于窗口最小尺寸")
        );
        ws[0].restriction = Some("全屏窗口".into());
        assert_eq!(
            preview_geometry(&display(), &ws, Template::SideBySide, 12.0)
                .unwrap()
                .placements[0]
                .restriction
                .as_deref(),
            Some("全屏窗口")
        );
    }
    #[test]
    fn invalid_input_and_wrong_template_count_are_rejected() {
        let d = display();
        for gap in [-1.0, 65.0, f64::NAN, f64::INFINITY] {
            assert!(preview_geometry(&d, &windows(2), Template::Grid, gap).is_err());
        }
        assert!(preview_geometry(&d, &windows(3), Template::SideBySide, 0.0).is_err());
        assert!(preview_geometry(&d, &windows(2), Template::MainAndTwo, 0.0).is_err());
        assert!(preview_geometry(&d, &windows(17), Template::Grid, 0.0).is_err());
        let mut ws = windows(2);
        ws[1].window_id = ws[0].window_id.clone();
        assert!(preview_geometry(&d, &ws, Template::Grid, 0.0).is_err());
        let mut tiny = d.clone();
        tiny.rect.width = 1.0;
        assert!(preview_geometry(&tiny, &windows(2), Template::SideBySide, 2.0).is_err());
        tiny.rect.width = f64::INFINITY;
        assert!(preview_geometry(&tiny, &windows(1), Template::Grid, 0.0).is_err());
    }
    #[test]
    fn fixed_window_already_at_target_is_not_a_false_restriction() {
        let d = display();
        let mut ws = windows(1);
        ws[0].rect = d.rect;
        ws[0].movable = false;
        ws[0].resizable = false;
        assert!(
            preview_geometry(&d, &ws, Template::Grid, 0.0)
                .unwrap()
                .applicable
        );
    }
}
