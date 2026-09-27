use crate::models::AgentProfile;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::time::SystemTime;
use walkdir::WalkDir;

pub struct FileActivityResult {
    pub latest_write: Option<SystemTime>,
    pub latest_file: Option<String>,
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

    pub fn probe(&mut self, profile: &AgentProfile) -> FileActivityResult {
        let mut latest: Option<SystemTime> = None;
        let mut latest_file: Option<String> = None;
        for root in &profile.session_dirs {
            if !Path::new(root).is_dir() {
                continue;
            }
            let (lw, lf) = self.probe_dir(root);
            if lw > latest {
                latest = lw;
                latest_file = lf;
            }
        }
        FileActivityResult {
            latest_write: latest,
            latest_file,
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
