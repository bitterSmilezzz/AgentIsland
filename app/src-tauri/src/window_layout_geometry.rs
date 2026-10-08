//! Bounded geometry sequencing; each phase has independent identity and readback checks.
use crate::window_layout::Rect;
use crate::window_layout_readback::Observation;

#[derive(Clone, Copy)]
pub enum Part {
    Size,
    Position,
    Complete,
}
impl Part {
    pub fn accepts(self, actual: Rect, target: Rect) -> bool {
        match self {
            Self::Size => {
                (actual.width - target.width).abs() <= 1.0
                    && (actual.height - target.height).abs() <= 1.0
            }
            Self::Position => {
                (actual.x - target.x).abs() <= 1.0 && (actual.y - target.y).abs() <= 1.0
            }
            Self::Complete => crate::window_layout_execution::matches(actual, target),
        }
    }
}

pub trait Driver {
    fn read(&self) -> Result<Rect, String>;
    fn resize(&self, target: Rect) -> Result<(), String>;
    fn move_to(&self, target: Rect) -> Result<(), String>;
    fn settle(&self, target: Rect, part: Part) -> Observation;
}

fn settle_size(driver: &impl Driver, target: Rect) -> Result<(), String> {
    let observation = driver.settle(target, Part::Size);
    if !observation.timed_out {
        if let Some(error) = observation.error {
            return Err(error);
        }
    }
    Ok(())
}

fn position(driver: &impl Driver, target: Rect) -> Result<(), String> {
    let actual = driver.read()?;
    if actual.x != target.x || actual.y != target.y {
        driver.move_to(target)?;
        // Growing before this move settles can hit the same edge constraint.
        if let Some(error) = driver.settle(target, Part::Position).error {
            return Err(error);
        }
    }
    Ok(())
}

pub fn adjust(driver: &impl Driver, target: Rect) -> Result<(), String> {
    if !target.valid() {
        return Err("目标窗口尺寸无效".into());
    }
    let current = driver.read()?;
    // First shrink only the dimensions that need it. The intermediate size fits
    // at the destination even when another dimension needs to grow.
    let intermediate = Rect {
        width: current.width.min(target.width),
        height: current.height.min(target.height),
        ..current
    };
    if intermediate.width != current.width || intermediate.height != current.height {
        driver.resize(intermediate)?;
        settle_size(driver, intermediate)?;
    }
    position(driver, target)?;
    if target.width > intermediate.width || target.height > intermediate.height {
        driver.read()?; // Revalidate identity immediately before the next write.
        driver.resize(target)?;
        settle_size(driver, target)?;
        // Some apps move the origin while resizing. Correct it once, with a
        // fresh identity check; never replay setters during readback polling.
        position(driver, target)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    struct ConstrainedWindow(RefCell<Rect>, std::cell::Cell<usize>);
    impl Driver for ConstrainedWindow {
        fn read(&self) -> Result<Rect, String> {
            Ok(*self.0.borrow())
        }
        fn resize(&self, target: Rect) -> Result<(), String> {
            self.1.set(self.1.get() + 1);
            let mut r = self.0.borrow_mut();
            // Real VS Code behavior observed in the isolated VM: enlarging at
            // the right edge is constrained before a later move can free space.
            r.width = target.width.min(1024.0 - r.x);
            r.height = target.height.min(714.0 - r.y);
            Ok(())
        }
        fn move_to(&self, target: Rect) -> Result<(), String> {
            self.1.set(self.1.get() + 1);
            let mut r = self.0.borrow_mut();
            r.x = target.x;
            r.y = target.y;
            Ok(())
        }
        fn settle(&self, target: Rect, part: Part) -> Observation {
            let actual = *self.0.borrow();
            let matched = part.accepts(actual, target);
            Observation {
                actual: Some(actual),
                error: (!matched).then(|| "constrained".into()),
                timed_out: !matched,
            }
        }
    }
    #[test]
    fn undo_right_half_restores_wide_original_inside_screen() {
        let window = ConstrainedWindow(
            RefCell::new(Rect {
                x: 518.0,
                y: 30.0,
                width: 506.0,
                height: 684.0,
            }),
            std::cell::Cell::new(0),
        );
        let target = Rect {
            x: 30.0,
            y: 30.0,
            width: 994.0,
            height: 684.0,
        };
        adjust(&window, target).unwrap();
        assert!(
            Part::Complete.accepts(window.read().unwrap(), target),
            "right-edge resize lost original width"
        );
    }
    #[test]
    fn mixed_shrink_and_growth_fits_destination_before_expansion() {
        let window = ConstrainedWindow(
            RefCell::new(Rect {
                x: 518.0,
                y: 30.0,
                width: 506.0,
                height: 684.0,
            }),
            std::cell::Cell::new(0),
        );
        let target = Rect {
            x: 0.0,
            y: 100.0,
            width: 900.0,
            height: 500.0,
        };
        adjust(&window, target).unwrap();
        assert!(Part::Complete.accepts(window.read().unwrap(), target));
        assert_eq!(window.1.get(), 3); // shrink, move, grow
    }
    #[test]
    fn asynchronous_move_must_settle_before_growth() {
        struct Delayed {
            inner: ConstrainedWindow,
            pending: RefCell<Option<Rect>>,
        }
        impl Driver for Delayed {
            fn read(&self) -> Result<Rect, String> {
                self.inner.read()
            }
            fn resize(&self, r: Rect) -> Result<(), String> {
                self.inner.resize(r)
            }
            fn move_to(&self, r: Rect) -> Result<(), String> {
                *self.pending.borrow_mut() = Some(r);
                Ok(())
            }
            fn settle(&self, r: Rect, part: Part) -> Observation {
                if matches!(part, Part::Position) {
                    if let Some(pending) = self.pending.borrow_mut().take() {
                        self.inner.move_to(pending).unwrap();
                    }
                }
                self.inner.settle(r, part)
            }
        }
        let window = Delayed {
            inner: ConstrainedWindow(
                RefCell::new(Rect {
                    x: 518.0,
                    y: 30.0,
                    width: 506.0,
                    height: 684.0,
                }),
                std::cell::Cell::new(0),
            ),
            pending: RefCell::new(None),
        };
        let target = Rect {
            x: 30.0,
            y: 30.0,
            width: 994.0,
            height: 684.0,
        };
        adjust(&window, target).unwrap();
        assert!(Part::Complete.accepts(window.read().unwrap(), target));
    }
    #[test]
    fn minimum_size_rejection_keeps_actual_without_replaying_resize() {
        struct Minimum(ConstrainedWindow);
        impl Driver for Minimum {
            fn read(&self) -> Result<Rect, String> {
                self.0.read()
            }
            fn resize(&self, r: Rect) -> Result<(), String> {
                self.0.resize(Rect {
                    width: r.width.max(800.0),
                    ..r
                })
            }
            fn move_to(&self, r: Rect) -> Result<(), String> {
                self.0.move_to(r)
            }
            fn settle(&self, r: Rect, p: Part) -> Observation {
                self.0.settle(r, p)
            }
        }
        let window = Minimum(ConstrainedWindow(
            RefCell::new(Rect {
                x: 0.0,
                y: 30.0,
                width: 1024.0,
                height: 684.0,
            }),
            std::cell::Cell::new(0),
        ));
        let target = Rect {
            x: 100.0,
            y: 30.0,
            width: 506.0,
            height: 684.0,
        };
        adjust(&window, target).unwrap();
        let final_read = window.settle(target, Part::Complete);
        assert!(final_read.error.is_some());
        assert_eq!(final_read.actual.unwrap().width, 800.0);
        assert_eq!(window.0 .1.get(), 2); // one rejected shrink and one move
    }
    #[test]
    fn identity_loss_after_shrink_prevents_further_writes() {
        struct Lost(ConstrainedWindow);
        impl Driver for Lost {
            fn read(&self) -> Result<Rect, String> {
                if self.0 .1.get() > 0 {
                    Err("identity lost".into())
                } else {
                    self.0.read()
                }
            }
            fn resize(&self, r: Rect) -> Result<(), String> {
                self.0.resize(r)
            }
            fn move_to(&self, r: Rect) -> Result<(), String> {
                self.0.move_to(r)
            }
            fn settle(&self, r: Rect, p: Part) -> Observation {
                self.0.settle(r, p)
            }
        }
        let window = Lost(ConstrainedWindow(
            RefCell::new(Rect {
                x: 518.0,
                y: 30.0,
                width: 506.0,
                height: 684.0,
            }),
            std::cell::Cell::new(0),
        ));
        assert_eq!(
            adjust(
                &window,
                Rect {
                    x: 0.0,
                    y: 100.0,
                    width: 900.0,
                    height: 500.0
                }
            )
            .unwrap_err(),
            "identity lost"
        );
        assert_eq!(window.0 .1.get(), 1);
    }
}
