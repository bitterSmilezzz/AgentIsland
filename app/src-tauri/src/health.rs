//! 智能体健康度评估 + 卡死/僵卡的三态判定。
//!
//! 对齐 Swift `AgentHealthEvaluator`（v0.0.73 起）与 `ActivityEngine` 里那段 `isHung`。
//! 两者必须一起搬：健康度报告的第一个扣分维度就是卡死，而「卡死」是**时间性**判定——
//! Rust 此前连 `isHung` 这个概念都没有（`grep hung` 零命中）。

use crate::models::AgentSnapshot;
use serde::Serialize;

/// 熔断 / 卡死判定的两个阈值。**全仓只在这一处定义**：
/// Swift 侧在 `Models.swift` 的 `EngineConfig` 里（`runawayCpuThreshold = 70.0`、
/// `runawayDurationThreshold = 300`）；Rust 此前把这两个数写成 `engine.rs` 里的
/// `70.0` 与 `300_000` 字面量，判定与告警各写一遍——改一处就会让「告警会响」与
/// 「健康度说卡死」对不上。
pub const RUNAWAY_CPU_THRESHOLD: f64 = 70.0;
pub const RUNAWAY_DURATION_MS: i64 = 300_000;

/// 健康等级。`serde` 出的是给界面做类名/图标用的 ASCII 码，
/// 中文等级在 [`Report::grade_label`] 里（与 Swift `HealthGrade.rawValue` 逐字相同）。
///
/// Swift 的 `HealthGrade.icon` 给的是五个 SF Symbol 名（`checkmark.shield.fill` 等），
/// 网页渲染不了，所以 Rust 侧**刻意不搬**那份映射：要么去猜等义码位（猜错没人发现），
/// 要么留一段永不被调用的代码。界面用分数 + 等级文案表达同一件事。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Grade {
    Healthy,
    Partial,
    Attention,
    Warning,
    Critical,
}

impl Grade {
    /// 与 Swift `HealthGrade.rawValue` 逐字相同
    pub fn label(self) -> &'static str {
        match self {
            Grade::Healthy => "健康",
            Grade::Partial => "观测不全",
            Grade::Attention => "需留意",
            Grade::Warning => "异常",
            Grade::Critical => "危急",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub score: i32,
    pub grade: Grade,
    pub grade_label: String,
    pub summary: String,
    pub issues: Vec<String>,
    pub suggestion: String,
}

impl Report {
    fn make(score: i32, grade: Grade, summary: String, issues: Vec<String>, suggestion: String) -> Report {
        Report {
            // Swift 在初始化器里钳制；照搬，免得上游改了扣分表就漏出 0..100
            score: score.clamp(0, 100),
            grade,
            grade_label: grade.label().to_string(),
            summary,
            issues,
            suggestion,
        }
    }

    /// 未运行（待机休眠）时的报告。`evaluate` 的第一条分支就用它，
    /// 测试要造快照时也用它——不必为了填字段去跑一遍评估。
    pub fn not_running() -> Report {
        Report::make(
            100,
            Grade::Healthy,
            "未运行（待机休眠）".into(),
            Vec::new(),
            "智能体未启动或在会话间隙休眠，无系统资源占用".into(),
        )
    }
}

/// 死锁 / 僵卡是**时间性**判定：CPU 连续超阈值达 [`RUNAWAY_DURATION_MS`] 才算数，
/// 而它的前提是这段窗口**确实被观测过**——一次性 CLI 进程活不到 5 分钟，
/// 它的每一拍都凑不出窗口。此时 `false` 的含义是「没测」而不是「没有」，
/// 所以返回 `None`。
///
/// 资格由 `observed_running_since` 给出，**不靠调用方自报**：Swift 侧上一版让调用方传
/// `sustainedObservation`，漏传一处的症状是静默谎报（v0.0.119 的异常扫描就是这个坑）。
pub fn is_hung(
    observed_running_since: Option<i64>,
    high_cpu_since: Option<i64>,
    now_ms: i64,
) -> Option<bool> {
    let since = observed_running_since?;
    if now_ms - since < RUNAWAY_DURATION_MS {
        return None;
    }
    Some(
        high_cpu_since
            .map(|high| now_ms - high >= RUNAWAY_DURATION_MS)
            .unwrap_or(false),
    )
}

/// 100 分制纯函数，零副作用（与 Swift 同名函数同口径）。
pub fn evaluate(snapshot: &AgentSnapshot) -> Report {
    if !snapshot.process_running {
        return Report::not_running();
    }

    let mut deduction = 0;
    let mut issues: Vec<String> = Vec::new();

    // 1. 卡死 / 死锁（最严重）。三态里只有 `Some(true)` 扣分：
    //    `None` 是「本轮判不出」——给它扣分等于凭空造一个病症，给它满分又等于宣布清白。
    //    所以扣 0，但在下面作废「健康」（降级为「观测不全」）。
    if snapshot.is_hung == Some(true) {
        deduction += 50;
        issues.push("疑似线程死锁或主循环卡死（长时间高负荷无响应）".into());
    }

    // 2. CPU 激增与死循环倾向。`None` = 本拍没有差分窗口，不扣分、也不许当成「CPU 不高」
    if let Some(cpu) = snapshot.cpu_percent {
        if cpu >= 80.0 {
            deduction += 25;
            issues.push(format!("CPU 持续占用高达 {cpu:.1}%"));
        } else if cpu >= 50.0 {
            deduction += 10;
            issues.push(format!("CPU 负荷较高 ({cpu:.1}%)"));
        }
    }

    // 3. 内存驻留集（RSS）溢出与泄露倾向
    const TWO_GB: u64 = 2 * 1024 * 1024 * 1024;
    const ONE_AND_HALF_GB: u64 = 1536 * 1024 * 1024;
    if snapshot.memory_bytes >= TWO_GB {
        deduction += 30;
        issues.push(format!("物理内存严重过高 ({} ≥ 2.0GB)", snapshot.memory_text));
    } else if snapshot.memory_bytes >= ONE_AND_HALF_GB {
        deduction += 15;
        issues.push(format!("物理内存占用偏大 ({} ≥ 1.5GB)", snapshot.memory_text));
    }

    let score = (100 - deduction).max(0);
    let mut grade = if score >= 85 {
        Grade::Healthy
    } else if score >= 70 {
        Grade::Attention
    } else if score >= 50 {
        Grade::Warning
    } else {
        Grade::Critical
    };

    // 本轮判不出的维度。扣分只针对测到的东西，但「健康」是一条全清结论——
    // 有维度没测就不许宣布全清。只在其余维度都干净时降级：内存/CPU 已经扣分时
    // 那两级正在喊话，换成「观测不全」反而盖掉了真实的严重度。
    let mut unevaluated: Vec<&str> = Vec::new();
    if snapshot.is_hung.is_none() {
        unevaluated.push("死锁/僵卡");
    }
    if snapshot.cpu_percent.is_none() {
        unevaluated.push("CPU 持续负荷");
    }
    if !unevaluated.is_empty() && grade == Grade::Healthy {
        grade = Grade::Partial;
    }
    let unevaluated_text = unevaluated.join(" 与 ");

    let summary = match grade {
        Grade::Healthy => "运行平稳正常".to_string(),
        Grade::Partial => format!("已测维度无异常，{unevaluated_text}本轮未评估"),
        Grade::Attention => "资源占用略高".to_string(),
        Grade::Warning => "负载异常，建议关注".to_string(),
        Grade::Critical => "严重异常，需紧急介入".to_string(),
    };

    let suggestion = if snapshot.is_hung == Some(true) {
        "检测到死锁，建议点击「终止逃生舱」重置智能体进程".to_string()
    } else if snapshot.memory_bytes >= TWO_GB {
        "长会话存在内存泄露隐患，建议在新会话中重新开始".to_string()
    } else if snapshot.cpu_percent.is_some_and(|cpu| cpu >= 80.0) {
        "任务可能陷入重度计算或死循环，请检查终端日志".to_string()
    } else if !unevaluated.is_empty() {
        // 措辞刻意不带阈值数字：那段时长只有一个来源（Swift `AgentCleaner.hungNotEvaluatedNote`
        // 从配置读），这里再写一遍就是同一结论两份措辞，漂移只是时间问题
        format!(
            "{unevaluated_text}本轮未评估，这一档分数不等于全清；\
             持续观测请用灵动岛工作台或 `agentisland top`"
        )
    } else if grade == Grade::Attention {
        "进程资源使用正常，可继续观测执行进展".to_string()
    } else {
        "会话心跳活跃，内存与 CPU 分布均衡".to_string()
    };

    Report::make(score, grade, summary, issues, suggestion)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(running: bool, is_hung: Option<bool>, cpu: Option<f64>, memory_text: &str) -> AgentSnapshot {
        snapshot_with_memory(running, is_hung, cpu, 0, memory_text)
    }

    fn snapshot_with_memory(
        running: bool,
        is_hung: Option<bool>,
        cpu: Option<f64>,
        memory_bytes: u64,
        memory_text: &str,
    ) -> AgentSnapshot {
        AgentSnapshot {
            id: "fixture".into(),
            name: "Fixture".into(),
            glyph: String::new(),
            emoji: String::new(),
            level: crate::models::ActivityLevel::Idle,
            level_label: "空闲".into(),
            observability: crate::observability::Verdict {
                code: crate::observability::Code::Observed,
                summary: "",
                evidence: Vec::new(),
            },
            process_running: running,
            is_hung,
            health: Report::not_running(),
            cpu_percent: cpu,
            memory_bytes,
            memory_text: memory_text.into(),
            last_activity_text: "—".into(),
            token_usage: None,
            pid: None,
            current_action: None,
            subagent_count: 0,
        }
    }

    #[test]
    fn a_stopped_agent_is_healthy_by_definition_not_by_measurement() {
        // 没跑就谈不上资源占用：这一档不打分，也不去列表里找「没测的维度」
        let report = evaluate(&snapshot(false, None, None, "—"));
        assert_eq!(report.score, 100);
        assert_eq!(report.grade, Grade::Healthy);
        assert!(report.issues.is_empty());
        assert_eq!(report.summary, "未运行（待机休眠）");
    }

    #[test]
    fn hung_is_the_heaviest_deduction_and_drives_the_suggestion() {
        let report = evaluate(&snapshot(true, Some(true), Some(10.0), "120 MB"));
        assert_eq!(report.score, 50, "卡死扣 50");
        assert_eq!(report.grade, Grade::Warning, "50 分落「异常」");
        assert!(report.issues[0].contains("死锁"));
        assert!(report.suggestion.contains("终止逃生舱"));
    }

    #[test]
    fn unevaluated_dimensions_block_a_clean_bill_but_never_mask_real_trouble() {
        // 判不出卡死 + CPU 干净 → 不许宣布「健康」，降级为「观测不全」
        let clean_but_blind = evaluate(&snapshot(true, None, Some(12.0), "180 MB"));
        assert_eq!(clean_but_blind.score, 100, "没测的维度不扣分");
        assert_eq!(clean_but_blind.grade, Grade::Partial);
        assert_eq!(clean_but_blind.summary, "已测维度无异常，死锁/僵卡本轮未评估");
        assert!(clean_but_blind.suggestion.contains("不等于全清"));

        // 内存已经扣分时不许被「观测不全」盖掉：那一档正在喊话
        let troubled_and_blind = evaluate(&snapshot_with_memory(
            true,
            None,
            Some(12.0),
            2 * 1024 * 1024 * 1024,
            "2.1 GB",
        ));
        assert_eq!(troubled_and_blind.grade, Grade::Attention, "★ 70 分档，不是「观测不全」");
        assert_eq!(troubled_and_blind.summary, "资源占用略高");
    }

    #[test]
    fn cpu_and_memory_deductions_use_the_same_thresholds_as_swift() {
        assert_eq!(evaluate(&snapshot(true, Some(false), Some(80.0), "1.0 GB")).score, 75);
        assert_eq!(evaluate(&snapshot(true, Some(false), Some(79.9), "1.0 GB")).score, 90);
        assert_eq!(evaluate(&snapshot(true, Some(false), Some(50.0), "1.0 GB")).score, 90);
        assert_eq!(evaluate(&snapshot(true, Some(false), Some(49.9), "1.0 GB")).score, 100);
        let big = evaluate(&snapshot_with_memory(true, Some(false), Some(0.0), 2 * 1024 * 1024 * 1024, "2.0 GB"));
        assert_eq!(big.score, 70, "2GB 扣 30");
        let half = evaluate(&snapshot_with_memory(
            true,
            Some(false),
            Some(0.0),
            1536 * 1024 * 1024,
            "1.5 GB",
        ));
        assert_eq!(half.score, 85, "1.5GB 扣 15，恰好还在「健康」线上");
        assert_eq!(half.grade, Grade::Healthy);
    }

    #[test]
    fn grade_boundaries_follow_the_same_ladder_as_swift() {
        // 85 / 70 / 50 三条线：分数刚好落在线上时取**更严重**的那一档
        let healthy = evaluate(&snapshot_with_memory(
            true,
            Some(false),
            Some(0.0),
            1536 * 1024 * 1024,
            "1.5 GB",
        ));
        assert_eq!((healthy.score, healthy.grade), (85, Grade::Healthy));
        let attention = evaluate(&snapshot_with_memory(
            true,
            Some(false),
            Some(0.0),
            2 * 1024 * 1024 * 1024,
            "2.0 GB",
        ));
        assert_eq!((attention.score, attention.grade), (70, Grade::Attention));
        let warning = evaluate(&snapshot(true, Some(true), Some(0.0), "1.0 GB"));
        assert_eq!((warning.score, warning.grade), (50, Grade::Warning));
        let critical = evaluate(&snapshot(true, Some(true), Some(50.0), "1.0 GB"));
        assert_eq!(
            (critical.score, critical.grade),
            (40, Grade::Critical),
            "卡死 50 + CPU≥50 的 10 = 60 分扣分"
        );
    }

    #[test]
    fn score_is_clamped_to_the_hundred_point_scale() {
        // 扣分表叠加后可以是负数，Swift 在初始化器里钳制；照搬
        let worst = evaluate(&snapshot_with_memory(
            true,
            Some(true),
            Some(95.0),
            3 * 1024 * 1024 * 1024,
            "3.0 GB",
        ));
        assert_eq!(worst.score, 0, "50 + 25 + 30 = 105 分扣分 → 钳到 0");
        assert_eq!(worst.grade, Grade::Critical);
        assert_eq!(Report::not_running().score, 100);
    }

    #[test]
    fn is_hung_needs_a_sustained_observation_window_not_just_high_cpu() {
        let now = 10_000_000i64;
        // 观测窗口还没凑够：哪怕 CPU 一直高，也只能说「没测」
        assert_eq!(is_hung(Some(now - 60_000), Some(now - 60_000), now), None);
        // 观测够久了，但高 CPU 是刚起来的 → 明确 false（不是 None）
        assert_eq!(
            is_hung(Some(now - RUNAWAY_DURATION_MS), Some(now - 1_000), now),
            Some(false)
        );
        // 两个窗口都够 → true
        assert_eq!(
            is_hung(Some(now - RUNAWAY_DURATION_MS), Some(now - RUNAWAY_DURATION_MS), now),
            Some(true)
        );
        // 观测够久但从来没高过 CPU → false
        assert_eq!(is_hung(Some(now - RUNAWAY_DURATION_MS), None, now), Some(false));
        // 从来没观测过 → None（不是 false）
        assert_eq!(is_hung(None, Some(now - RUNAWAY_DURATION_MS), now), None);
        // 边界取等号：恰好 5 分钟算「够」
        assert_eq!(
            is_hung(Some(now - RUNAWAY_DURATION_MS + 1), None, now),
            None,
            "差 1ms 不算够"
        );
    }
}
