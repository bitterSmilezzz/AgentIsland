//! Wait for asynchronous app geometry without issuing another write.
use crate::window_layout::Rect;
use std::time::Duration;

pub struct Observation {
    pub actual: Option<Rect>,
    pub error: Option<String>,
    pub timed_out: bool,
}

/// Two matching samples prevent a single intermediate animation frame being accepted.
/// The budget bounds polling; an individual native read has its own AX timeout.
pub fn settle(
    mut read: impl FnMut() -> Result<Rect, String>,
    mut accepts: impl FnMut(Rect) -> bool,
    mut elapsed: impl FnMut() -> Duration,
    mut pause: impl FnMut(Duration),
) -> Observation {
    let budget = Duration::from_millis(600);
    let interval = Duration::from_millis(40);
    let mut actual = None;
    let mut matches = 0;
    loop {
        match read() {
            Ok(value) if value.valid() => {
                actual = Some(value);
                matches = if accepts(value) { matches + 1 } else { 0 };
                if matches >= 2 {
                    return Observation {
                        actual,
                        error: None,
                        timed_out: false,
                    };
                }
            }
            Ok(_) => {
                return Observation {
                    actual,
                    error: Some("窗口返回了无效尺寸".into()),
                    timed_out: false,
                }
            }
            Err(reason) => {
                return Observation {
                    actual,
                    error: Some(reason),
                    timed_out: false,
                }
            }
        }
        let remaining = budget.saturating_sub(elapsed());
        if remaining.is_zero() {
            return Observation {
                actual,
                error: Some("窗口尚未到达目标位置，请核对后重试".into()),
                timed_out: true,
            };
        }
        pause(interval.min(remaining));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    fn rect(x: f64) -> Rect {
        Rect {
            x,
            y: 0.0,
            width: 600.0,
            height: 400.0,
        }
    }
    #[test]
    fn delayed_geometry_requires_two_consecutive_samples() {
        let time = Cell::new(Duration::ZERO);
        let samples = Cell::new(0);
        let sequence = [0.0, 10.0, 5.0, 10.0, 10.0];
        let result = settle(
            || {
                let n = samples.get();
                samples.set(n + 1);
                Ok(rect(sequence[n]))
            },
            |r| r.x == 10.0,
            || time.get(),
            |d| time.set(time.get() + d),
        );
        assert!(result.error.is_none());
        assert_eq!(samples.get(), 5);
        assert_eq!(time.get(), Duration::from_millis(160));
    }
    #[test]
    fn constrained_window_stops_at_budget_and_preserves_actual() {
        let time = Cell::new(Duration::ZERO);
        let result = settle(
            || Ok(rect(3.0)),
            |r| r.x == 10.0,
            || time.get(),
            |d| time.set(time.get() + d),
        );
        assert!(result.error.is_some());
        assert!(result.timed_out);
        assert_eq!(result.actual.unwrap().x, 3.0);
        assert_eq!(time.get(), Duration::from_millis(600));
    }
    #[test]
    fn identity_loss_stops_without_further_polling() {
        let time = Cell::new(Duration::ZERO);
        let samples = Cell::new(0);
        let result = settle(
            || {
                samples.set(samples.get() + 1);
                if samples.get() == 1 {
                    Ok(rect(3.0))
                } else {
                    Err("工具已退出".into())
                }
            },
            |_| false,
            || time.get(),
            |d| time.set(time.get() + d),
        );
        assert_eq!(result.error.as_deref(), Some("工具已退出"));
        assert_eq!(result.actual.unwrap().x, 3.0);
        assert_eq!(samples.get(), 2);
        assert!(!result.timed_out);
    }
}
