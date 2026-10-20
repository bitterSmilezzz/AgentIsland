//! File logging is opt-in for the resident GUI. Headless startup never opens a log.
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

pub(crate) const LOG_CAP_BYTES: u64 = 2 * 1024 * 1024;

/// No destination until the GUI startup boundary is crossed. The destination cannot
/// be restored or changed by a probe returning or by a worker outliving its caller.
pub(crate) struct Logger {
    destination: OnceLock<PathBuf>,
}

impl Logger {
    pub(crate) const fn new() -> Self {
        Self { destination: OnceLock::new() }
    }

    fn enable_gui_in(&self, directory: &Path, isolated: bool, pid: u32) {
        self.destination.get_or_init(|| directory.join(if isolated {
            format!("agentisland-test-{pid}.log")
        } else {
            "agentisland-tauri.log".to_owned()
        }));
    }

    pub(crate) fn write(&self, message: &str) {
        // Check before metadata, truncation, file creation, or stderr fallback.
        let Some(path) = self.destination.get() else { return; };
        if let Some(before) = truncate_log_if_oversized(path) {
            let line = format!("[run] 上一份日志 {before} 字节，已按 {LOG_CAP_BYTES} 字节封顶清空");
            use std::io::Write;
            if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
                let _ = writeln!(file, "{line}");
            }
        }
        use std::io::Write;
        match std::fs::OpenOptions::new().create(true).append(true).open(path) {
            Ok(mut file) => { let _ = writeln!(file, "{message}"); }
            Err(error) => {
                eprintln!("[log] 写不进 {}：{error}", path.display());
                eprintln!("[log] {message}");
            }
        }
    }
}

static LOGGER: Logger = Logger::new();

/// Call once, after every headless handler declined argv, before GUI settings load.
pub(crate) fn enable_gui(isolated: bool) {
    LOGGER.enable_gui_in(&std::env::temp_dir(), isolated, std::process::id());
}

pub(crate) fn write(message: &str) {
    LOGGER.write(message);
}

pub(crate) fn truncate_log_if_oversized(path: &Path) -> Option<u64> {
    let size = std::fs::metadata(path).ok()?.len();
    if size <= LOG_CAP_BYTES { return None; }
    std::fs::write(path, b"").ok()?;
    Some(size)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::Sandbox;

    #[test]
    fn default_logger_neither_creates_appends_nor_truncates_files() {
        let dir = Sandbox::new("headless-logger");
        let logger = Logger::new();
        let path = dir.path().join("agentisland-tauri.log");
        logger.write("synthetic early diagnostic");
        assert!(!path.exists());
        for bytes in [b"synthetic user log\n".to_vec(), vec![b'x'; 2_097_153]] {
            std::fs::write(&path, &bytes).unwrap();
            logger.write("synthetic probe diagnostic");
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
        }
        assert!(logger.destination.get().is_none());
    }

    #[test]
    fn ordinary_gui_appends_and_keeps_the_log_cap() {
        let dir = Sandbox::new("gui-logger");
        let logger = Logger::new();
        logger.enable_gui_in(dir.path(), false, 42);
        let path = dir.path().join("agentisland-tauri.log");
        std::fs::write(&path, b"existing\n").unwrap();
        logger.write("first diagnostic");
        assert_eq!(std::fs::read(&path).unwrap(), b"existing\nfirst diagnostic\n");
        std::fs::write(&path, vec![b'x'; 2_097_153]).unwrap();
        logger.write("after cap");
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("[run] 上一份日志 2097153 字节"));
        assert!(text.ends_with("after cap\n"));
        assert!(text.len() < 1024);
    }

    #[test]
    fn isolated_gui_keeps_its_own_destination_for_its_lifetime() {
        let dir = Sandbox::new("isolated-logger");
        let logger = Logger::new();
        let user = dir.path().join("agentisland-tauri.log");
        std::fs::write(&user, b"user log\n").unwrap();
        logger.enable_gui_in(dir.path(), true, 42);
        logger.write("smoke diagnostic");
        logger.enable_gui_in(dir.path(), false, 43);
        logger.write("second diagnostic");
        assert_eq!(std::fs::read(&user).unwrap(), b"user log\n");
        assert_eq!(std::fs::read(dir.path().join("agentisland-test-42.log")).unwrap(),
                   b"smoke diagnostic\nsecond diagnostic\n");
    }

    #[test]
    fn a_probe_worker_does_not_need_its_own_suppression_flag() {
        let logger = std::sync::Arc::new(Logger::new());
        let worker_logger = logger.clone();
        std::thread::spawn(move || worker_logger.write("worker diagnostic")).join().unwrap();
        assert!(logger.destination.get().is_none());
    }
}
