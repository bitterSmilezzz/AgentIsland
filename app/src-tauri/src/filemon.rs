use crate::models::AgentProfile;
use std::collections::HashMap;
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
/// 最新写入与活跃计数共用 2s 元数据缓存，信号内容仍由解析器按采样读取。
pub struct FileMonitor {
    cache: HashMap<String, DirectoryScan>,
}

struct DirectoryScan {
    at: std::time::Instant,
    files: Vec<(SystemTime, String)>,
    modified: Vec<SystemTime>,
}

// Codex sessions/<year>/<month>/<day>/<rollout> 的文件深度为 4。
const MAX_DEPTH: usize = 4;
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

    pub fn diagnostic_cache(&self) -> crate::resource_diagnostics::FileCache {
        crate::resource_diagnostics::FileCache {
            roots: self.cache.len(),
            files: self.cache.values().map(|s| s.modified.len()).sum(),
            file_capacity: self.cache.values().map(|s| s.modified.capacity()).sum(),
            cached_candidates: self.cache.values().map(|s| s.files.len()).sum(),
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
            active_sessions += active_count(&self.cache[root].modified, active_window_secs);
        }
        FileActivityResult {
            latest_write: latest,
            latest_file,
            active_sessions,
        }
    }

    /// 同拍元数据里的最近候选（新→旧，最多 16 个）。较新的普通日志不能遮住会话。
    pub fn probe_files(&mut self, profile: &AgentProfile) -> Vec<String> {
        let roots: Vec<_> = profile
            .session_dirs
            .iter()
            .filter(|root| Path::new(root).is_dir())
            .collect();
        for root in &roots {
            self.probe_dir(root);
        }
        recent_candidates(roots.iter().flat_map(|root| self.cache[*root].files.iter()))
    }

    fn probe_dir(&mut self, dir: &str) -> (Option<SystemTime>, Option<String>) {
        if !self
            .cache
            .get(dir)
            .is_some_and(|scan| scan.at.elapsed().as_secs_f64() < 2.0)
        {
            self.cache.insert(dir.to_string(), scan_directory(dir));
        }
        self.cache[dir]
            .files
            .iter()
            .max_by_key(|(mtime, _)| *mtime)
            .map(|(mtime, path)| (Some(*mtime), Some(path.clone())))
            .unwrap_or((None, None))
    }
}

const CANDIDATE_LIMIT: usize = 16;

/// Compact bounded batches of borrowed metadata; clone only the final 16 paths.
/// Sorting and deduplication match full-sort → unique-path → take(16).
fn recent_candidates<'a>(files: impl Iterator<Item = &'a (SystemTime, String)>) -> Vec<String> {
    const BATCH_SIZE: usize = 256;
    fn compact<'a>(
        selected: &mut Vec<&'a (SystemTime, String)>,
        seen: &mut std::collections::HashSet<&'a str>,
    ) {
        selected.sort_unstable_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
        seen.clear();
        selected.retain(|entry| seen.len() < CANDIDATE_LIMIT && seen.insert(entry.1.as_str()));
    }
    let mut selected = Vec::with_capacity(BATCH_SIZE + CANDIDATE_LIMIT);
    let mut seen = std::collections::HashSet::with_capacity(CANDIDATE_LIMIT);
    for entry in files {
        selected.push(entry);
        if selected.len() == BATCH_SIZE + CANDIDATE_LIMIT {
            compact(&mut selected, &mut seen);
        }
    }
    compact(&mut selected, &mut seen);
    selected.into_iter().map(|entry| entry.1.clone()).collect()
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
fn active_count(files: &[SystemTime], window_secs: f64) -> usize {
    if window_secs <= 0.0 {
        return 0;
    }
    let now = SystemTime::now();
    let cutoff = now
        .checked_sub(std::time::Duration::from_secs_f64(window_secs.max(0.0)))
        .unwrap_or(now);
    files
        .iter()
        .filter(|modified| **modified >= cutoff && **modified <= now)
        .count()
}

/// Own only a bounded batch of paths while all readable mtimes remain available.
struct Candidates {
    paths: Vec<(SystemTime, String)>,
    floor: Option<SystemTime>,
}
impl Candidates {
    const BATCH: usize = 256;
    fn new() -> Self {
        Self {
            paths: Vec::with_capacity(Self::BATCH + CANDIDATE_LIMIT),
            floor: None,
        }
    }
    fn consider(&mut self, modified: SystemTime, path: &Path) {
        // Older files cannot enter the unique top16. Equal times still need lexical ordering.
        if self.floor.is_some_and(|floor| modified < floor) {
            return;
        }
        self.paths
            .push((modified, path.to_string_lossy().into_owned()));
        if self.paths.len() == Self::BATCH + CANDIDATE_LIMIT {
            self.compact();
        }
    }
    fn compact(&mut self) {
        self.paths
            .sort_unstable_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
        let mut keep = [false; Self::BATCH + CANDIDATE_LIMIT];
        {
            let mut seen = std::collections::HashSet::with_capacity(CANDIDATE_LIMIT);
            for (i, (_, path)) in self.paths.iter().enumerate() {
                if seen.len() == CANDIDATE_LIMIT {
                    break;
                }
                keep[i] = seen.insert(path.as_str());
            }
        }
        let mut index = 0;
        self.paths.retain(|_| {
            let retain = keep[index];
            index += 1;
            retain
        });
        self.floor = (self.paths.len() == CANDIDATE_LIMIT).then(|| self.paths.last().unwrap().0);
    }
    fn finish(mut self) -> Vec<(SystemTime, String)> {
        self.compact();
        self.paths.into_boxed_slice().into_vec()
    }
}
fn scan_directory(root: &str) -> DirectoryScan {
    let mut modified = Vec::new();
    let mut candidates = Candidates::new();
    for entry in WalkDir::new(root)
        .max_depth(MAX_DEPTH)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| {
            entry.depth() == 0 || !SKIP_DIRS.contains(&entry.file_name().to_string_lossy().as_ref())
        })
    {
        let Ok(entry) = entry else { continue };
        if !entry.file_type().is_file() {
            continue;
        }
        let ext = entry
            .path()
            .extension()
            .map(|ext| ext.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if !matches!(ext.as_str(), "jsonl" | "json" | "db" | "sqlite" | "log") {
            continue;
        }
        if let Some(time) = entry
            .metadata()
            .ok()
            .and_then(|metadata| metadata.modified().ok())
        {
            modified.push(time);
            candidates.consider(time, entry.path());
        }
    }
    DirectoryScan {
        at: std::time::Instant::now(),
        files: candidates.finish(),
        modified,
    }
}

#[cfg(test)]
fn count_active_sessions(root: &str, window_secs: f64) -> usize {
    active_count(&scan_directory(root).modified, window_secs)
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

    #[test]
    fn unrelated_files_are_not_active_sessions() {
        let sandbox = crate::testutil::Sandbox::new("active-signal-only");
        std::fs::write(sandbox.path().join("session.jsonl"), b"{}").unwrap();
        std::fs::write(sandbox.path().join("image.png"), b"fixture").unwrap();
        std::fs::write(sandbox.path().join("settings.txt"), b"fixture").unwrap();
        assert_eq!(
            count_active_sessions(sandbox.path().to_str().unwrap(), 600.0),
            1
        );
    }

    #[test]
    fn latest_activity_and_counts_share_one_cached_scan() {
        let sandbox = crate::testutil::Sandbox::new("shared-session-scan");
        for index in 0..500 {
            std::fs::write(sandbox.path().join(format!("{index}.jsonl")), b"{}").unwrap();
        }
        let mut profile = crate::registry::builtin().remove(0);
        let root = sandbox.path().to_string_lossy().into_owned();
        profile.session_dirs = vec![root.clone()];
        let mut monitor = FileMonitor::new();
        let first = monitor.probe(&profile, 600.0);
        let scanned_at = monitor.cache[&root].at;
        assert_eq!(first.active_sessions, 500);
        assert_eq!(monitor.probe_files(&profile).len(), 16);
        assert_eq!(monitor.probe(&profile, 0.0).active_sessions, 0);
        assert_eq!(
            monitor.cache[&root].at, scanned_at,
            "读候选和计数不能重复遍历目录"
        );
    }

    #[test]
    fn codex_date_partitioned_rollout_is_detected() {
        let sandbox = crate::testutil::Sandbox::new("codex-partitioned-session");
        let dated = sandbox.path().join("2026/10/01");
        std::fs::create_dir_all(&dated).unwrap();
        let rollout = dated.join("rollout-fixture.jsonl");
        std::fs::write(&rollout, b"{}\n").unwrap();
        let mut profile = crate::registry::builtin()
            .into_iter()
            .find(|profile| profile.id == "codex")
            .unwrap();
        profile.session_dirs = vec![sandbox.path().to_string_lossy().into_owned()];
        let mut monitor = FileMonitor::new();
        let result = monitor.probe(&profile, 600.0);
        assert_eq!(result.active_sessions, 1, "Codex 的年/月/日目录必须可见");
        assert_eq!(result.latest_file.as_deref(), rollout.to_str());
        assert_eq!(
            monitor.probe_files(&profile),
            vec![rollout.to_string_lossy().into_owned()]
        );
    }

    fn filetime_set(path: &std::path::Path, when: std::time::SystemTime) {
        let file = std::fs::File::options()
            .write(true)
            .open(path)
            .expect("应当能打开来改 mtime");
        file.set_modified(when).expect("set_modified 应当成功");
    }
}

#[cfg(test)]
mod optimization_regressions {
    use super::*;
    #[test]
    fn a_newer_log_cannot_hide_the_session_candidate() {
        let sandbox = crate::testutil::Sandbox::new("log-shadows-session");
        let session = sandbox.path().join("rollout.jsonl");
        let log = sandbox.path().join("runtime.log");
        std::fs::write(&session, b"{}\n").unwrap();
        std::fs::write(&log, b"runtime heartbeat\n").unwrap();
        let older = SystemTime::now() - std::time::Duration::from_secs(10);
        std::fs::File::options()
            .write(true)
            .open(&session)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(older))
            .unwrap();
        let mut profile = crate::registry::builtin().remove(0);
        profile.session_dirs = vec![sandbox.path().to_string_lossy().into_owned()];
        let files = FileMonitor::new().probe_files(&profile);
        assert_eq!(files.first(), Some(&log.to_string_lossy().into_owned()));
        assert!(
            files.contains(&session.to_string_lossy().into_owned()),
            "fall through to actual session when the newer log has no signal"
        );
    }
}

#[cfg(test)]
mod candidate_selection_tests {
    use super::*;
    fn reference(files: &[(SystemTime, String)]) -> Vec<String> {
        let mut files = files.to_vec();
        files.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
        let mut seen = std::collections::HashSet::new();
        files
            .into_iter()
            .filter_map(|(_, path)| seen.insert(path.clone()).then_some(path))
            .take(16)
            .collect()
    }
    #[test]
    fn bounded_selection_matches_full_sort_across_duplicates_ties_and_orders() {
        let base = SystemTime::UNIX_EPOCH;
        let mut files = vec![];
        for i in 0..2000 {
            files.push((
                base + std::time::Duration::from_secs((i * 37 % 113) as u64),
                format!("fixture/session-{}", i % 97),
            ));
        }
        for rotate in [0, 1, 16, 97, 1999] {
            files.rotate_left(rotate);
            assert_eq!(recent_candidates(files.iter()), reference(&files));
            files.reverse();
            assert_eq!(recent_candidates(files.iter()), reference(&files));
        }
        for len in [0, 1, 15, 16, 17, 255, 256, 271, 272, 273, 528, 2000] {
            assert_eq!(
                recent_candidates(files[..len].iter()),
                reference(&files[..len])
            );
        }
    }
    #[test]
    fn same_time_candidates_keep_lexical_order_and_unique_paths() {
        let files = (0..100)
            .rev()
            .flat_map(|i| vec![(SystemTime::UNIX_EPOCH, format!("fixture/{i:03}")); 3])
            .collect::<Vec<_>>();
        let actual = recent_candidates(files.iter());
        assert_eq!(
            actual,
            (0..16)
                .map(|i| format!("fixture/{i:03}"))
                .collect::<Vec<_>>()
        );
    }
    #[test]
    #[ignore = "explicit synthetic performance comparison, no native application acceptance"]
    fn synthetic_candidate_selection_measurement() {
        let mut files = (0..100_000)
            .map(|i| {
                (
                    SystemTime::UNIX_EPOCH
                        + std::time::Duration::from_secs((i * 37 % 100_003) as u64),
                    format!("fixture/{i:06}/{}", "x".repeat(128)),
                )
            })
            .collect::<Vec<_>>();
        for order in ["mixed", "oldest_first", "newest_first"] {
            if order == "oldest_first" {
                files.sort_by(|a, b| a.0.cmp(&b.0));
            }
            if order == "newest_first" {
                files.reverse();
            }
            let start = std::time::Instant::now();
            let old = reference(&files);
            let old_time = start.elapsed();
            let start = std::time::Instant::now();
            let new = recent_candidates(files.iter());
            let new_time = start.elapsed();
            assert_eq!(old, new);
            println!("synthetic selection: order={} entries={} old_us={} bounded_us={} old_cloned_path_payload={} bounded_output_path_payload={}",order,files.len(),old_time.as_micros(),new_time.as_micros(),files.iter().map(|(_,p)|p.len()).sum::<usize>(),new.iter().map(String::len).sum::<usize>());
        }
    }
}

#[cfg(test)]
mod complete_metadata_tests {
    use super::*;
    #[test]
    fn more_than_four_thousand_files_keep_complete_activity_and_latest_candidate() {
        let sandbox = crate::testutil::Sandbox::new("complete-metadata");
        for i in 0..4101 {
            std::fs::write(sandbox.path().join(format!("{i:05}.jsonl")), b"{}\n").unwrap();
        }
        // Choose a file beyond the old traversal prefix, regardless of filesystem enumeration order.
        let entries = WalkDir::new(sandbox.path())
            .max_depth(MAX_DEPTH)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|e| e.file_type().is_file())
            .collect::<Vec<_>>();
        let latest = entries[4100].path();
        let now = SystemTime::now();
        for e in &entries {
            std::fs::File::options()
                .write(true)
                .open(e.path())
                .unwrap()
                .set_modified(now - std::time::Duration::from_secs(30))
                .unwrap();
        }
        std::fs::File::options()
            .write(true)
            .open(latest)
            .unwrap()
            .set_modified(now)
            .unwrap();
        let root = sandbox.path().to_string_lossy().into_owned();
        let scan = scan_directory(&root);
        assert_eq!(scan.modified.len(), 4101);
        assert_eq!(active_count(&scan.modified, 600.0), 4101);
        assert_eq!(scan.files.len(), 16);
        assert_eq!(scan.files.first().unwrap().1, latest.to_string_lossy());
        assert!(scan.files.capacity() <= 16);
        let mut profile = crate::registry::builtin().remove(0);
        profile.session_dirs = vec![root.clone()];
        let mut monitor = FileMonitor::new();
        let result = monitor.probe(&profile, 600.0);
        assert_eq!(result.active_sessions, 4101);
        assert_eq!(result.latest_file.as_deref(), latest.to_str());
        assert_eq!(
            monitor.probe_files(&profile).first().map(String::as_str),
            latest.to_str()
        );
        let counts = monitor.diagnostic_cache();
        assert_eq!(counts.files, 4101);
        assert_eq!(counts.cached_candidates, 16);
    }
    #[test]
    fn streamed_paths_match_full_sort_at_batch_boundaries_and_with_duplicate_times() {
        let mut input = (0..2000)
            .map(|i| {
                (
                    SystemTime::UNIX_EPOCH + std::time::Duration::from_secs((i * 37 % 113) as u64),
                    format!("fixture/{}", i % 97),
                )
            })
            .collect::<Vec<_>>();
        for _ in 0..4 {
            for len in [0, 1, 16, 255, 256, 271, 272, 273, 528, 2000] {
                let mut candidates = Candidates::new();
                for (time, path) in &input[..len] {
                    candidates.consider(*time, Path::new(path));
                    assert!(candidates.paths.len() <= 272);
                }
                let actual = candidates
                    .finish()
                    .into_iter()
                    .map(|(_, path)| path)
                    .collect::<Vec<_>>();
                assert_eq!(actual, recent_candidates(input[..len].iter()));
            }
            input.rotate_left(73);
            input.reverse();
        }
    }
}
