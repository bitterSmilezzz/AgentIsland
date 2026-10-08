//! Pure, bounded scheduling. Temporary geometry never leaves verified work areas.
use crate::window_layout::{DisplayArea, Placement, Rect, WindowCandidate};
use crate::window_layout_execution::matches;

pub type Domain = (i32, u64);

#[derive(Clone, Copy, Debug)]
pub struct Step {
    pub index: usize,
    pub before: Rect,
    pub target: Rect,
    pub temporary: bool,
}

fn phases(before: Rect, target: Rect) -> [Rect; 3] {
    let shrunk = Rect {
        width: before.width.min(target.width),
        height: before.height.min(target.height),
        ..before
    };
    [
        shrunk,
        Rect {
            x: target.x,
            y: target.y,
            ..shrunk
        },
        target,
    ]
}

fn near(a: Rect, b: Rect) -> bool {
    // Keep temporary geometry distinct despite the existing one-point readback
    // allowance on both windows. This does not relax native identity matching.
    [a.x - b.x, a.y - b.y, a.width - b.width, a.height - b.height]
        .iter()
        .all(|v| v.abs() <= 3.0)
}

fn inside(area: Rect, rect: Rect) -> bool {
    area.valid()
        && rect.valid()
        && rect.x >= area.x
        && rect.y >= area.y
        && rect.x + rect.width <= area.x + area.width
        && rect.y + rect.height <= area.y + area.height
}

struct Scene<'a> {
    placements: &'a [Placement],
    candidates: &'a [WindowCandidate],
    domains: &'a [Option<Domain>],
    displays: &'a [DisplayArea],
    obstacles: &'a [(Domain, Rect)],
}

impl Scene<'_> {
    fn same(&self, i: usize, j: usize) -> bool {
        i != j && self.domains[i].is_some() && self.domains[i] == self.domains[j]
    }
    fn clear(&self, i: usize, target: Rect, current: &[Rect]) -> bool {
        let path = phases(current[i], target);
        !(0..current.len()).any(|j| self.same(i, j) && path.iter().any(|r| matches(*r, current[j])))
            && !self.obstacles.iter().any(|(domain, r)| {
                Some(*domain) == self.domains[i] && path.iter().any(|p| matches(*p, *r))
            })
    }
    fn ready(&self, i: usize, current: &[Rect], pending: &[usize]) -> bool {
        self.clear(i, self.placements[i].target, current)
            && !pending.iter().any(|&j| {
                self.same(i, j)
                    && phases(current[j], self.placements[j].target)
                        .iter()
                        .any(|r| matches(*r, self.placements[i].target))
            })
    }
    fn staging(&self, i: usize, current: &[Rect], pending: &[usize]) -> Option<Rect> {
        if !self.candidates[i].movable && !self.candidates[i].resizable {
            return None;
        }
        let before = current[i];
        let final_target = self.placements[i].target;
        let width = before.width.min(final_target.width);
        let height = before.height.min(final_target.height);
        let sizes = [
            (before.width, before.height),
            (width, height),
            (width - 4.0, height),
            (width, height - 4.0),
            (width - 4.0, height - 4.0),
        ];
        for (width, height) in sizes {
            if width <= 0.0 || height <= 0.0 {
                continue;
            }
            for display in self.displays {
                let a = display.rect;
                if !a.valid() || width > a.width || height > a.height {
                    continue;
                }
                for origin in [before, final_target] {
                    let x = origin.x.clamp(a.x, a.x + a.width - width);
                    let y = origin.y.clamp(a.y, a.y + a.height - height);
                    // At most 16 selected windows; finite candidates and at
                    // most one temporary step per window, never a retry loop.
                    for distance in 1..=2 * self.placements.len() + 1 {
                        let d = distance as f64 * 4.0;
                        for (dx, dy) in [(0.0, 0.0), (d, 0.0), (-d, 0.0), (0.0, d), (0.0, -d)] {
                            if dx == 0.0 && dy == 0.0 && distance > 1 {
                                continue;
                            }
                            let r = Rect {
                                x: x + dx,
                                y: y + dy,
                                width,
                                height,
                            };
                            if !inside(a, r)
                                || near(r, before)
                                || near(r, final_target)
                                || !self.clear(i, r, current)
                            {
                                continue;
                            }
                            if pending.iter().any(|&j| {
                                self.same(i, j)
                                    && phases(current[j], self.placements[j].target)
                                        .iter()
                                        .any(|p| near(r, *p))
                            }) {
                                continue;
                            }
                            if (0..current.len()).any(|j| self.same(i, j) && near(r, current[j])) {
                                continue;
                            }
                            let modified: Vec<_> = phases(before, r)
                                .into_iter()
                                .filter(|p| *p != before)
                                .collect();
                            if (0..current.len()).any(|j| {
                                self.same(i, j) && modified.iter().any(|p| near(*p, current[j]))
                            }) {
                                continue;
                            }
                            // Unselected snapshot rectangles are conservative
                            // avoidance hints. Native checks still see the live scene.
                            if self.obstacles.iter().any(|(domain, bounds)| {
                                Some(*domain) == self.domains[i]
                                    && modified.iter().any(|p| near(*p, *bounds))
                            }) {
                                continue;
                            }
                            // A fixed window will not vacate later in this plan.
                            // Staging must also make the final path possible.
                            if self.obstacles.iter().any(|(domain, bounds)| {
                                Some(*domain) == self.domains[i]
                                    && phases(r, final_target).iter().any(|p| matches(*p, *bounds))
                            }) {
                                continue;
                            }
                            let candidate = WindowCandidate {
                                rect: before,
                                ..self.candidates[i].clone()
                            };
                            if crate::window_layout_execution::verify(&candidate, before, r)
                                .is_err()
                            {
                                continue;
                            }
                            return Some(r);
                        }
                    }
                }
            }
        }
        None
    }
}

pub fn build(
    placements: &[Placement],
    candidates: &[WindowCandidate],
    domains: &[Option<Domain>],
    displays: &[DisplayArea],
    obstacles: &[(Domain, Rect)],
) -> Result<Vec<Step>, String> {
    if !(1..=16).contains(&placements.len())
        || candidates.len() != placements.len()
        || domains.len() != placements.len()
        || placements
            .iter()
            .any(|p| !p.before.valid() || !p.target.valid())
        || placements
            .iter()
            .zip(candidates)
            .any(|(p, c)| crate::window_layout_execution::verify(c, p.before, p.target).is_err())
    {
        return Err("窗口执行计划无效，请重新预览".into());
    }
    let scene = Scene {
        placements,
        candidates,
        domains,
        displays,
        obstacles,
    };
    let mut current: Vec<_> = candidates.iter().map(|c| c.rect).collect();
    let mut pending: Vec<_> = (0..placements.len()).collect();
    let mut staged = vec![false; placements.len()];
    let mut steps = Vec::new();
    while !pending.is_empty() {
        if let Some(position) = pending
            .iter()
            .position(|&i| scene.ready(i, &current, &pending))
        {
            let i = pending.remove(position);
            steps.push(Step {
                index: i,
                before: current[i],
                target: placements[i].target,
                temporary: false,
            });
            current[i] = placements[i].target;
        } else if let Some((i, target)) = pending
            .iter()
            .filter(|&&i| !staged[i])
            .find_map(|&i| scene.staging(i, &current, &pending).map(|r| (i, r)))
        {
            staged[i] = true;
            steps.push(Step {
                index: i,
                before: current[i],
                target,
                temporary: true,
            });
            current[i] = target;
        } else {
            return Err("当前窗口无法安全交换位置，请调整窗口或屏幕后重新预览".into());
        }
    }
    Ok(steps)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn pair(offset: f64) -> (Vec<Placement>, Vec<WindowCandidate>, Vec<DisplayArea>) {
        let rect = |x| Rect {
            x: x + offset,
            y: 30.0,
            width: 506.0,
            height: 680.0,
        };
        let placements = (0..2)
            .map(|i| Placement {
                window_id: i.to_string(),
                before: rect(i as f64 * 518.0),
                target: rect((1 - i) as f64 * 518.0),
                restriction: None,
            })
            .collect();
        let candidates = (0..2)
            .map(|i| WindowCandidate {
                window_id: i.to_string(),
                agent_id: "code".into(),
                application: String::new(),
                title: String::new(),
                screen_id: None,
                rect: rect(i as f64 * 518.0),
                movable: true,
                resizable: true,
                restriction: None,
                minimum_size: None,
            })
            .collect();
        let displays = vec![DisplayArea {
            screen_id: "d".into(),
            scale: 2.0,
            rect: Rect {
                x: offset,
                y: 30.0,
                width: 1024.0,
                height: 681.0,
            },
        }];
        (placements, candidates, displays)
    }
    #[test]
    fn swap_has_one_small_stage_and_preserves_target_slots() {
        let (p, c, d) = pair(0.0);
        let steps = build(&p, &c, &[Some((42, 1)); 2], &d, &[]).unwrap();
        assert_eq!(steps.len(), 3);
        assert!(steps[0].temporary);
        assert_eq!(steps[0].target.x, 4.0);
        assert_eq!(steps[0].target.width, 506.0);
        assert_eq!(
            steps.iter().map(|s| s.index).collect::<Vec<_>>(),
            vec![0, 1, 0]
        );
        assert!(steps
            .iter()
            .filter(|s| !s.temporary)
            .all(|s| s.target == p[s.index].target));
    }
    #[test]
    fn negative_origin_and_dpi_stages_remain_inside_available_area() {
        let (p, c, d) = pair(-1024.0);
        let steps = build(&p, &c, &[Some((42, 1)); 2], &d, &[]).unwrap();
        assert!(steps.iter().all(|s| inside(d[0].rect, s.target)));
        assert_eq!(steps[0].target.x, -1020.0);
    }
    #[test]
    fn missing_area_or_immovable_cycle_fails_before_execution() {
        let (p, mut c, d) = pair(0.0);
        assert!(build(&p, &c, &[Some((42, 1)); 2], &[], &[]).is_err());
        c.iter_mut().for_each(|w| w.movable = false);
        assert!(build(&p, &c, &[Some((42, 1)); 2], &d, &[]).is_err());
    }
    #[test]
    fn distinct_process_launch_domains_need_no_staging() {
        let (p, c, d) = pair(0.0);
        let steps = build(&p, &c, &[Some((42, 1)), Some((42, 2))], &d, &[]).unwrap();
        assert_eq!(steps.len(), 2);
        assert!(steps.iter().all(|s| !s.temporary));
    }
    #[test]
    fn known_unselected_window_avoids_first_temporary_position() {
        let (p, c, d) = pair(0.0);
        let obstacle = Rect {
            x: 4.0,
            ..p[0].before
        };
        let steps = build(&p, &c, &[Some((42, 1)); 2], &d, &[((42, 1), obstacle)]).unwrap();
        assert_eq!(steps[0].target.x, 8.0);
    }

    #[test]
    fn sixteen_window_rotation_is_bounded_and_never_enters_occupied_geometry() {
        let (_, mut c, mut d) = pair(0.0);
        let rect = |i: usize| Rect {
            x: i as f64 * 64.0,
            y: 30.0,
            width: 60.0,
            height: 100.0,
        };
        let p: Vec<_> = (0..16)
            .map(|i| Placement {
                window_id: i.to_string(),
                before: rect(i),
                target: rect((i + 1) % 16),
                restriction: None,
            })
            .collect();
        c = (0..16)
            .map(|i| WindowCandidate {
                window_id: i.to_string(),
                rect: rect(i),
                ..c[0].clone()
            })
            .collect();
        d[0].rect.height = 100.0;
        let steps = build(&p, &c, &[Some((42, 1)); 16], &d, &[]).unwrap();
        assert_eq!(steps.len(), 17);
        let mut current: Vec<_> = p.iter().map(|p| p.before).collect();
        for step in steps {
            assert!(inside(d[0].rect, step.target));
            for r in phases(step.before, step.target) {
                assert!((0..16).all(|j| j == step.index || !matches(r, current[j])));
            }
            current[step.index] = step.target;
        }
        assert_eq!(current, p.iter().map(|p| p.target).collect::<Vec<_>>());
    }

    #[test]
    fn impossible_duplicate_destinations_cannot_issue_a_partial_plan() {
        let (mut p, c, d) = pair(0.0);
        p[1].target = p[0].target;
        assert!(build(&p, &c, &[Some((42, 1)); 2], &d, &[]).is_err());
    }

    #[test]
    fn invalid_geometry_is_rejected_before_generating_setters() {
        let (mut p, c, d) = pair(0.0);
        p[0].target.x = f64::NAN;
        assert!(build(&p, &c, &[Some((42, 1)); 2], &d, &[]).is_err());
    }
}
