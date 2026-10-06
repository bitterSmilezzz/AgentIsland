//! Opt-in local transport. Only the verified installer lifecycle may start a receiver.
//! No decision JSON, network listener, auto-start, logs, or persistent plan bodies.
use std::time::{Duration, Instant};
pub(crate) const FLAG: &str = "--claude-plan-hook";
pub(crate) const MAX_FRAME: usize = 512 * 1024;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Error {
    Unsupported,
    Namespace,
    Busy,
    Unavailable,
    Input,
    Rejected,
}

#[cfg(target_os = "macos")]
#[path = "claude_plan_receiver_macos.rs"]
pub(crate) mod native;

/// Must run before settings, engine, UI, logger, or background thread initialization.
pub(crate) fn cli(args: &[String]) -> Option<i32> {
    if !args.iter().any(|arg| arg == FLAG) {
        return None;
    }
    #[cfg(target_os = "macos")]
    let result = if args.len() == 2 && args[1] == FLAG {
        native::collect_default()
    } else {
        Err(Error::Input)
    };
    #[cfg(not(target_os = "macos"))]
    let result: Result<(), Error> = Err(Error::Unsupported);
    match result {
        Ok(()) => Some(0),
        Err(_) => {
            eprintln!("方案采集未确认，请在 AgentIsland 中检查采集状态。");
            Some(1)
        }
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn default_root() -> std::path::PathBuf {
    std::path::PathBuf::from(format!("/private/tmp/agentisland-plans-{}", unsafe {
        libc::geteuid()
    }))
}
fn remaining(deadline: Instant) -> Result<Duration, Error> {
    let time = deadline
        .checked_duration_since(Instant::now())
        .ok_or(Error::Unavailable)?;
    if time.is_zero() {
        Err(Error::Unavailable)
    } else {
        Ok(time)
    }
}
