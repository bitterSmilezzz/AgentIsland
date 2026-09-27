//! 任务耗时与效率统计（Swift `TaskDurationTracker`）。
//!
//! 记的是**每次已完成任务**的时长，按 Agent 分开、每个 Agent 有上界，
//! 统计默认看过去 24 小时。它喂的是详情页那块「任务效能」。
//!
//! 三处口径照搬：
//! ① **`duration <= 0` 不记**（调用方可能拿到 0 或负的时长）；
//! ② **每个 Agent 只留最近 100 条**，超出丢**最早**的——是「最近的表现」而不是「全部历史」，
//!    所以上限到期时丢哪一头很关键；
//! ③ 窗口过滤按 `now - timestamp <= window`（含边界）✓ 与 Swift 的
//!    `now.timeIntervalSince(ts) <= window` 同义。
//!
//! 并发：Swift 用一个 `NSLock` 包住字典；Rust 侧这个追踪器由引擎持有，
//! 而引擎本身已经在 `Mutex` 后面（命令层每次锁住整个引擎），所以这里**不加锁**——
//! 多一层锁只会让人以为可以绕过引擎直接共享它。这一点写在类型注释里。

use std::collections::{HashMap, VecDeque};

/// 每个 Agent 保留的记录条数上限（Swift 默认 100）
pub const MAX_RECORDS_PER_AGENT: usize = 100;

/// 详情页那块统计默认看过去 24 小时
pub const DEFAULT_WINDOW_MS: i64 = 24 * 3600 * 1000;

/// 一次任务够格被记下来的最短时长（秒）。
///
/// Swift 在 `recordTaskCompleted` 里写死 3.5：「持续至少 3.5 秒的实质工作才视作完成一次任务
/// （过滤瞬时微抖动）」。Rust 侧此前**只有**保持 Working 的窗口（10 秒），没有这条——
/// 于是 0.2 秒的抖动也会记一笔并推一条「任务完成 (0秒)」。
pub const MIN_TASK_SECONDS: f64 = 3.5;

#[derive(Debug, Clone, Copy, PartialEq)]
struct Record {
    duration: f64,
    timestamp: i64,
}

/// 某个 Agent 在一个窗口内的效能统计
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    pub total_work_time: f64,
    pub task_count: usize,
    pub average_duration: f64,
    pub max_duration: f64,
    /// 已排版的两个数（`1小时5分` / `3分12秒/次`）。排版口径只有一处，
    /// 别让界面各写一份——这也是本仓对「句子在 Rust 侧拼好」的一贯做法
    pub formatted_total_time: String,
    pub formatted_average_duration: String,
}

impl Default for Stats {
    fn default() -> Self {
        Stats::empty()
    }
}

impl Stats {
    pub fn empty() -> Self {
        let mut stats = Stats {
            total_work_time: 0.0,
            task_count: 0,
            average_duration: 0.0,
            max_duration: 0.0,
            formatted_total_time: String::new(),
            formatted_average_duration: String::new(),
        };
        stats.refresh_text();
        stats
    }

    fn refresh_text(&mut self) {
        self.formatted_total_time = formatted_total(self.total_work_time);
        self.formatted_average_duration = formatted_average(self.average_duration);
    }
}

/// 总工作时长：`0秒` / `45秒` / `3分12秒` / `2分钟` / `1小时5分` / `2小时`
pub fn formatted_total(seconds: f64) -> String {
    if seconds <= 0.0 {
        return "0秒".to_string();
    }
    let total = seconds.round() as i64;
    if total < 60 {
        return format!("{total}秒");
    }
    if total < 3600 {
        let (m, s) = (total / 60, total % 60);
        // 整分钟写「4分钟」而不是「4分0秒」：后者读起来像精确到秒的测量值
        return if s > 0 {
            format!("{m}分{s}秒")
        } else {
            format!("{m}分钟")
        };
    }
    let (h, m) = (total / 3600, (total % 3600) / 60);
    if m > 0 {
        format!("{h}小时{m}分")
    } else {
        format!("{h}小时")
    }
}

/// 平均单任务时长：`—`（没有样本）/ `45秒/次` / `3分12秒/次` / `2分/次`
pub fn formatted_average(seconds: f64) -> String {
    if seconds <= 0.0 {
        return "—".to_string();
    }
    let total = seconds.round() as i64;
    if total < 60 {
        return format!("{total}秒/次");
    }
    let (m, s) = (total / 60, total % 60);
    if s > 0 {
        format!("{m}分{s}秒/次")
    } else {
        format!("{m}分/次")
    }
}

#[derive(Debug, Clone)]
pub struct TaskDurationTracker {
    records: HashMap<String, VecDeque<Record>>,
    max_records_per_agent: usize,
}

impl Default for TaskDurationTracker {
    fn default() -> Self {
        TaskDurationTracker::new()
    }
}

impl TaskDurationTracker {
    pub fn new() -> Self {
        TaskDurationTracker {
            records: HashMap::new(),
            max_records_per_agent: MAX_RECORDS_PER_AGENT,
        }
    }

    /// 记录一次已完成任务。`duration <= 0` 直接丢弃（拿不到时长时记 0 会让平均值失真）
    pub fn record(&mut self, agent_id: &str, duration: f64, timestamp: i64) {
        if duration <= 0.0 {
            return;
        }
        let list = self.records.entry(agent_id.to_string()).or_default();
        list.push_back(Record {
            duration,
            timestamp,
        });
        // 只留最近 N 条：丢**最早**的（这是「最近表现」，不是全部历史）
        while list.len() > self.max_records_per_agent {
            list.pop_front();
        }
    }

    /// 过去 `window_ms` 内的统计。没有样本时返回全零（排版文本也随之是 `0秒` / `—`）
    pub fn stats(&self, agent_id: &str, window_ms: i64, now: i64) -> Stats {
        let Some(list) = self.records.get(agent_id) else {
            return Stats::empty();
        };
        let mut stats = Stats::empty();
        for record in list {
            // 未来时间戳（时钟回拨）也算在窗口内：把它排除会让刚记下的那次凭空消失
            if now - record.timestamp <= window_ms {
                stats.total_work_time += record.duration;
                stats.task_count += 1;
                if record.duration > stats.max_duration {
                    stats.max_duration = record.duration;
                }
            }
        }
        if stats.task_count > 0 {
            stats.average_duration = stats.total_work_time / stats.task_count as f64;
        }
        stats.refresh_text();
        stats
    }

}

// 参考实现里还有一个 `prune(olderThan:)`，**但它在 Swift 侧也从未被调用**——
// 而每个 Agent 的条数上限（①）已经管住了内存：超过 100 条时丢最早的，
// 于是「很久以前的记录占着位置」这件事本身不成立。照搬一个没人调的 API
// 只会让下一个人以为它有用（而且会为它写一条永远不会失败的用例）。
// 所以这里**不搬**，理由写在此处。

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_non_positive_duration_is_never_recorded() {
        let mut tracker = TaskDurationTracker::new();
        tracker.record("claude", 0.0, 1_000);
        tracker.record("claude", -5.0, 1_000);
        assert_eq!(
            tracker.stats("claude", DEFAULT_WINDOW_MS, 1_000).task_count,
            0,
            "0 与负数都不该记"
        );
        assert_eq!(tracker.stats("claude", DEFAULT_WINDOW_MS, 1_000), Stats::empty());
    }

    #[test]
    fn only_the_newest_hundred_records_per_agent_survive() {
        let mut tracker = TaskDurationTracker::new();
        for i in 0..150 {
            tracker.record("claude", 10.0 + i as f64, i as i64);
        }
        let stats = tracker.stats("claude", DEFAULT_WINDOW_MS, 1_000);
        assert_eq!(stats.task_count, 100, "每个 Agent 只留最近 100 条");
        // 丢的是**最早**的：最长的那条（10+149）必须还在
        assert_eq!(stats.max_duration, 159.0, "上限到期时丢最早、留最新");
        // 另一个 Agent 不受影响
        assert_eq!(tracker.stats("gemini", DEFAULT_WINDOW_MS, 1_000).task_count, 0);
    }

    #[test]
    fn the_window_filter_is_inclusive_and_uses_the_callers_clock() {
        let mut tracker = TaskDurationTracker::new();
        let now = 1_000_000i64;
        tracker.record("claude", 10.0, now - DEFAULT_WINDOW_MS); // 正好在边界上
        tracker.record("claude", 20.0, now - DEFAULT_WINDOW_MS - 1); // 窗外
        tracker.record("claude", 30.0, now);
        let stats = tracker.stats("claude", DEFAULT_WINDOW_MS, now);
        assert_eq!(stats.task_count, 2, "边界上那条算窗口内，窗外那条不算");
        assert_eq!(stats.total_work_time, 40.0);
        assert_eq!(stats.average_duration, 20.0);
        assert_eq!(stats.max_duration, 30.0);

        // 窗口收窄到 1 秒：只剩最后那条
        let stats = tracker.stats("claude", 1_000, now);
        assert_eq!(stats.task_count, 1);
        assert_eq!(stats.total_work_time, 30.0);
    }

    #[test]
    fn a_record_from_the_future_still_counts_so_clock_rewinds_do_not_erase_it() {
        // 系统时钟回拨后，刚记下的那条时间戳会「在未来」；把它排除会让它凭空消失
        let mut tracker = TaskDurationTracker::new();
        tracker.record("claude", 12.0, 2_000_000);
        let stats = tracker.stats("claude", DEFAULT_WINDOW_MS, 1_000_000);
        assert_eq!(stats.task_count, 1);
        assert_eq!(stats.total_work_time, 12.0);
    }

    #[test]
    fn unknown_agent_gets_zeroed_stats_with_readable_text() {
        let tracker = TaskDurationTracker::new();
        let stats = tracker.stats("never-seen", DEFAULT_WINDOW_MS, 1_000);
        assert_eq!(stats.task_count, 0);
        assert_eq!(stats.total_work_time, 0.0);
        assert_eq!(stats.formatted_total_time, "0秒");
        assert_eq!(
            stats.formatted_average_duration, "—",
            "平均时长没有样本时写 —，不是 0秒/次（那会被读成「每次都是瞬间」）"
        );
    }

    #[test]
    fn the_total_text_switches_units_at_the_right_boundaries() {
        assert_eq!(formatted_total(0.0), "0秒");
        assert_eq!(formatted_total(-3.0), "0秒");
        assert_eq!(formatted_total(45.0), "45秒");
        assert_eq!(formatted_total(59.4), "59秒");
        assert_eq!(formatted_total(192.0), "3分12秒");
        assert_eq!(formatted_total(120.0), "2分钟", "整分钟不写 2分0秒");
        assert_eq!(formatted_total(3_900.0), "1小时5分");
        assert_eq!(formatted_total(7_200.0), "2小时", "整小时不写 2小时0分");
    }

    #[test]
    fn the_average_text_keeps_the_per_task_suffix() {
        assert_eq!(formatted_average(0.0), "—");
        assert_eq!(formatted_average(45.0), "45秒/次");
        assert_eq!(formatted_average(192.0), "3分12秒/次");
        assert_eq!(formatted_average(120.0), "2分/次");
    }

    #[test]
    fn stats_are_per_agent_not_global() {
        let mut tracker = TaskDurationTracker::new();
        tracker.record("claude", 10.0, 1_000);
        tracker.record("gemini", 90.0, 1_000);
        assert_eq!(tracker.stats("claude", DEFAULT_WINDOW_MS, 1_000).total_work_time, 10.0);
        assert_eq!(tracker.stats("gemini", DEFAULT_WINDOW_MS, 1_000).total_work_time, 90.0);
    }
}
