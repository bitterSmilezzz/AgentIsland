use crate::models::AgentProfile;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::time::SystemTime;
use walkdir::WalkDir;

pub struct FileActivityResult {
    pub latest_write: Option<SystemTime>,
    pub latest_file: Option<String>,
    /// **活跃会话数**：最近 `active_session_window` 内有写入的会话文件个数。
    ///
    /// 这是 `activeSessions` 的真口径。此前 Rust 侧拿「十分钟内有没有写入」这个
    /// 布尔量当代理——**文件新鲜度与活跃会话不是同一个量**：
    /// 一次写入只证明「它动过」，不证明「有几场会话在跑」；
    /// 而 `noLocalData` 判定的输入正是这个数。
    pub active_sessions: usize,
}

/// 会话目录扫描：找最近写入的会话文件（JSONL/JSON/DB/LOG）。
/// 目录结果带节流缓存：刚活动过 2s，安静 6s。
pub struct FileMonitor {
    cache: HashMap<String, (std::time::Instant, Option<SystemTime>, Option<String>)>,
}

const MAX_DEPTH: usize = 3;
const SKIP_DIRS: &[&str] = &[
    "node_modules",
    ".git",
    "Cache",
    "CachedData",
    "GPUCache",
    "logs",
    "Code Cache",
];

impl FileMonitor {
    pub fn new() -> Self {
        FileMonitor {
            cache: HashMap::new(),
        }
    }

    pub fn probe(&mut self, profile: &AgentProfile, active_window_secs: f64) -> FileActivityResult {
        let mut latest: Option<SystemTime> = None;
        let mut latest_file: Option<String> = None;
        let mut active_sessions = 0usize;
        for root in &profile.session_dirs {
            if !Path::new(root).is_dir() {
                continue;
            }
            let (lw, lf) = self.probe_dir(root);
            if lw > latest {
                latest = lw;
                latest_file = lf;
            }
            // 活跃会话数**按目录各自数再相加**（Swift `activeSessionCounts` 逐目录产出一份）。
            // 不这么做的后果是跨目录时重复计：同一场会话在两个根下各被数一次。
            active_sessions += count_active_sessions(root, active_window_secs);
        }
        FileActivityResult {
            latest_write: latest,
            latest_file,
            active_sessions,
        }
    }

    /// 每个会话目录的最新文件（按修改时间新→旧）。动作/语义解析按此顺序候选：
    /// 最新的不一定是语义文件（如 CLI 日志比 rollout 更新），调用方逐个尝试。
    pub fn probe_files(&mut self, profile: &AgentProfile) -> Vec<String> {
        let mut files: Vec<(SystemTime, String)> = Vec::new();
        for root in &profile.session_dirs {
            if !Path::new(root).is_dir() {
                continue;
            }
            let (lw, lf) = self.probe_dir(root);
            if let (Some(t), Some(f)) = (lw, lf) {
                files.push((t, f));
            }
        }
        files.sort_by(|a, b| b.0.cmp(&a.0));
        files.into_iter().map(|(_, f)| f).collect()
    }

    fn probe_dir(&mut self, dir: &str) -> (Option<SystemTime>, Option<String>) {
        if let Some((at, lw, lf)) = self.cache.get(dir) {
            let ttl = if at.elapsed().as_secs() < 30 {
                2
            } else {
                6
            };
            if at.elapsed().as_secs() < ttl {
                return (*lw, lf.clone());
            }
        }
        let mut latest: Option<SystemTime> = None;
        let mut latest_file: Option<String> = None;
        let mut count = 0usize;
        for entry in WalkDir::new(dir)
            .max_depth(MAX_DEPTH)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                e.depth() == 0 || !SKIP_DIRS.contains(&name.as_str())
            })
        {
            let Ok(entry) = entry else { continue };
            if !entry.file_type().is_file() {
                continue;
            }
            let ext = entry
                .path()
                .extension()
                .map(|e| e.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            if !matches!(ext.as_str(), "jsonl" | "json" | "db" | "sqlite" | "log") {
                continue;
            }
            count += 1;
            if count > 4000 {
                break;
            }
            if let Ok(meta) = entry.metadata() {
                let mtime = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                if latest.is_none() || mtime > latest.unwrap() {
                    latest = Some(mtime);
                    latest_file = Some(entry.path().to_string_lossy().to_string());
                }
            }
        }
        self.cache
            .insert(dir.to_string(), (std::time::Instant::now(), latest, latest_file.clone()));
        (latest, latest_file)
    }
}

/// 「最近活动」这一列（Swift `ActivityEngine.formatAgo` 同口径）。
///
/// **这一轮修掉的又一处同表面不同口径**：Rust 曾经写「30秒前 / 2分钟前 / 3小时前」
/// 并把「刚刚」的阈值放在 10 秒；而参考实现是 `30s 前 / 2m 前`、阈值 **5** 秒、且没有「天」这一档。
/// 两个数都是「最近活动」这一列，用户在岛上看到 `2m 前`、在侧边栏看到 `2分钟前`——
/// 同一份数据两种说法。与 `compact` 那次同样的判断：审美偏好不足以支撑与参考实现分叉。
///
/// `None` → `—`：**「没有活动信号」与「刚刚活动过」是两件事**，
/// 把 nil 当 0 秒显示成「刚刚」会让读的人以为这个 Agent 很活跃。
pub fn time_ago_text(ago_secs: Option<f64>) -> String {
    let Some(ago) = ago_secs else {
        return "—".into();
    };
    let seconds = ago.max(0.0).round() as i64;
    if seconds < 5 {
        return "刚刚".into();
    }
    if seconds < 60 {
        return format!("{seconds}s 前");
    }
    if seconds < 3600 {
        return format!("{}m 前", seconds / 60);
    }
    format!("{}h 前", seconds / 3600)
}

/// 数「最近 `window_secs` 内有写入的会话文件」个数。
///
/// 窗口取自设置（`active_session_window`，默认 600s），与 Swift `FileMonitor.activeSessionWindow`
/// **同一个来源**——否则用户在设置里调了窗口，两边会按不同的数判「有没有活跃会话」。
///
/// 只数**文件**：目录的 mtime 会被创建/删除子项改动，而那不代表有会话在跑。
fn count_active_sessions(root: &str, window_secs: f64) -> usize {
    let now = SystemTime::now();
    let cutoff = now
        .checked_sub(std::time::Duration::from_secs_f64(window_secs.max(0.0)))
        .unwrap_or(now);
    let mut count = 0usize;
    let walker = walkdir::WalkDir::new(root)
        .max_depth(MAX_DEPTH)
        .into_iter()
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            !SKIP_DIRS.contains(&name.as_ref())
        });
    for entry in walker.flatten() {
        if !entry.file_type().is_file() {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        let Ok(modified) = meta.modified() else { continue };
        if modified >= cutoff {
            count += 1;
        }
    }
    count
}

#[cfg(test)]
mod active_session_tests {
    use super::*;

    #[test]
    fn a_file_written_inside_the_window_counts_and_one_outside_does_not() {
        let sandbox = crate::testutil::Sandbox::new("activesessions");
        let dir = sandbox.path().join("projects");
        std::fs::create_dir_all(&dir).unwrap();
        let fresh = dir.join("a.jsonl");
        let stale = dir.join("b.jsonl");
        std::fs::write(&fresh, b"{}").unwrap();
        std::fs::write(&stale, b"{}").unwrap();
        // 把 b 的 mtime 推到窗口之外
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(7200);
        filetime_set(&stale, old);

        let count = count_active_sessions(dir.to_str().unwrap(), 600.0);
        assert_eq!(count, 1, "窗口内的算、窗口外的不算");
    }

    /// 只数**文件**：目录 mtime 会因为增删子项而变，那不代表有会话在跑。
    #[test]
    fn directories_whose_mtime_moved_are_not_counted_as_sessions() {
        let sandbox = crate::testutil::Sandbox::new("activedirs");
        let dir = sandbox.path().join("projects");
        std::fs::create_dir_all(&dir).unwrap();
        // 只建目录、不建文件
        std::fs::create_dir_all(dir.join("empty-subdir")).unwrap();
        assert_eq!(count_active_sessions(dir.to_str().unwrap(), 600.0), 0);
    }

    /// 窗口取 0 ⇒ 什么都不算。**不是「全部都算」**——
    /// 一个 0 宽的窗口若被当成无限制，`noLocalData` 的判定就整个反了。
    #[test]
    fn a_zero_window_counts_nothing() {
        let sandbox = crate::testutil::Sandbox::new("activezero");
        let dir = sandbox.path().join("p");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.jsonl"), b"{}").unwrap();
        assert_eq!(count_active_sessions(dir.to_str().unwrap(), 0.0), 0);
    }

    fn filetime_set(path: &std::path::Path, when: std::time::SystemTime) {
        let file = std::fs::File::options()
            .write(true)
            .open(path)
            .expect("应当能打开来改 mtime");
        file.set_modified(when).expect("set_modified 应当成功");
    }
}
