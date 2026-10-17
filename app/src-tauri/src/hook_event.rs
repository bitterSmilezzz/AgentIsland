//! HTTP-only channel envelope.
//!
//! Two clocks, deliberately separated. `instant` is the monotonic arrival mark and
//! exists **only** to measure enqueue latency — it is never persisted and never shown.
//! `timestamp_ms` is the arrival **wall clock**, and it does become the event's
//! display time: it is written into `AgentTaskEvent.timestamp`, hence into the UI and
//! into the persisted `StoredEvent.timestamp`. So say it precisely: no clock from this
//! envelope reaches persisted/UI data *as a latency measurement*; the wall clock is
//! exactly the timestamp that does.
use crate::models::AgentTaskEvent;
#[cfg(test)]
use std::time::SystemTime;
use std::time::{Instant, UNIX_EPOCH};

#[derive(Debug, Clone, Copy)]
pub(crate) struct HookArrival {
    pub instant: Instant,
    pub timestamp_ms: i64,
}

impl From<tiny_http::RequestArrival> for HookArrival {
    fn from(arrival: tiny_http::RequestArrival) -> Self {
        Self {
            instant: arrival.instant,
            timestamp_ms: arrival
                .wall_time
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
                .unwrap_or(0),
        }
    }
}

#[cfg(test)]
impl HookArrival {
    pub(crate) fn before(milliseconds: u64) -> Self {
        let now = Self::from(tiny_http::RequestArrival {
            instant: Instant::now(),
            wall_time: SystemTime::now(),
        });
        Self {
            instant: now.instant - std::time::Duration::from_millis(milliseconds),
            timestamp_ms: now.timestamp_ms - milliseconds as i64,
        }
    }
}

pub(crate) struct HookEvent {
    /// Validated HTTP id; never used as a UI id or persisted in plaintext.
    pub client_id: Option<String>,
    pub event: AgentTaskEvent,
    pub received_at: Instant,
}
