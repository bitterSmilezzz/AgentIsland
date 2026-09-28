//! 运维审计报告导出（Swift `AuditReportExporter`）。
//!
//! 两种格式：Markdown（贴进工单/群聊/Issue）与 CSV（进表格）。三条口径值得单独说：
//!
//! ① **表格单元必须转义**。`agentisland://notify?...message=x%0A%7C...` 里
//!    `queryItems` 会把 `%0A` 解成真换行，于是外部输入能往用户粘进工单的报告里
//!    **自造行与小节**（导出还会把这份报告写进剪贴板）。Swift 侧修过这条，
//!    注释里写明「此前只有 summaryText 过了 `|`」。
//! ② **`—` 与 `0` 是两件事**：`—` 是「这个源本轮没取到用量」，`0` 才是「查了、确实是零」。
//!    把 nil 印成 0，会让一份运维报告把「这个工具的 token 我监控不了」写成「这个工具没花钱」。
//! ③ **跨源总量与逐条相加不等时要把口径说出来**，而不是悄悄选一个——差额来自
//!    「离线但仍有用量记录」「与宿主合并的内嵌组件」这类在册范围差异，读报告的人需要知道。
//!
//! 未迁：`generateRaycastManifest`。它要写一个**版本号**进 JSON，而 Rust 侧的
//! `Cargo.toml` 版本是 `0.1.0`、与 App 版本（`AppVersion.string`）**并未同步**——
//! 直接用它会把一个错号发给用户。要迁先得解决「第四个版本位」的同步规则（见 review）。

use crate::models::{AgentSnapshot, AgentTaskEvent, Export, TokenUsage};
use crate::observability::Code;

/// 「yyyy-MM-dd HH:mm:ss」（本地时间）
pub fn timestamp_text(now_ms: i64) -> String {
    match crate::tokens::local_time_parts(now_ms) {
        Some((year, month, day, hour, minute, second)) => {
            format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}")
        }
        None => "—".to_string(),
    }
}

/// 「yyyyMMdd_HHmmss」（本地时间，做文件名）
pub fn file_timestamp_text(now_ms: i64) -> String {
    match crate::tokens::local_time_parts(now_ms) {
        Some((year, month, day, hour, minute, second)) => {
            format!("{year:04}{month:02}{day:02}_{hour:02}{minute:02}{second:02}")
        }
        None => "unknown".to_string(),
    }
}

pub fn default_filename(ext: &str, now_ms: i64) -> String {
    format!("AgentIsland_Audit_{}.{ext}", file_timestamp_text(now_ms))
}

pub fn markdown_export(
    snapshots: &[AgentSnapshot],
    history: &[AgentTaskEvent],
    grand_total: Option<&TokenUsage>,
    now_ms: i64,
) -> Export {
    Export {
        filename: default_filename("md", now_ms),
        content: generate_markdown(snapshots, history, grand_total, now_ms),
    }
}

pub fn csv_export(snapshots: &[AgentSnapshot], now_ms: i64) -> Export {
    Export {
        filename: default_filename("csv", now_ms),
        content: generate_csv(snapshots, now_ms),
    }
}

/// Markdown 表格单元转义。
///
/// 只处理**会破坏表格结构**的字符：`|` 分列、换行分行。**反斜杠不转义**——
/// Markdown 单元里的 `\` 不是列分隔符，转它只会让文本变样。
pub fn cell(text: &str) -> String {
    text.replace('|', "\\|").replace(['\r', '\n'], " ")
}

/// CSV 字段转义：含逗号、引号或换行时整体加引号，内部引号加倍
pub fn escape_csv(text: &str) -> String {
    if text.contains(',') || text.contains('"') || text.contains('\n') {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text.to_string()
    }
}

fn event_type_text(event_type: &str) -> &'static str {
    match event_type {
        "attention" => "待确认",
        "costSpike" => "熔断告警",
        _ => "任务完成",
    }
}

/// 生成结构化 Markdown 审计报告。
///
/// `grand_total` 是面板汇总栏那份跨源总量：**给了就必须用它做头条数字**——
/// `snapshots` 只覆盖当前在册条目（离线但仍有用量的工具、被宿主去重掉的档案都不在
/// 那份列表里），逐条相加会比面板少一大截。用户拿着报告对不上岛，怀疑的是面板。
pub fn generate_markdown(
    snapshots: &[AgentSnapshot],
    history: &[AgentTaskEvent],
    grand_total: Option<&TokenUsage>,
    now_ms: i64,
) -> String {
    let mut md = String::new();
    md.push_str("# AgentIsland 智能体运维与 Token 消耗审计报告\n\n");
    md.push_str(&format!(
        "- **生成时间**：{}\n",
        timestamp_text(now_ms)
    ));
    md.push_str(&format!("- **监控智能体总数**：{} 个\n", snapshots.len()));

    let running_count = snapshots.iter().filter(|s| s.process_running).count();
    let working_count = snapshots
        .iter()
        .filter(|s| s.level == crate::models::ActivityLevel::Working)
        .count();
    let listed_tokens24h: i64 = snapshots
        .iter()
        .map(|s| s.token_usage.as_ref().map(|u| u.tokens24h).unwrap_or(0))
        .sum();
    let listed_cost24h: f64 = snapshots
        .iter()
        .map(|s| s.token_usage.as_ref().map(|u| u.cost24h).unwrap_or(0.0))
        .sum();
    let total_tokens24h = grand_total.map(|g| g.tokens24h).unwrap_or(listed_tokens24h);
    let total_cost24h = grand_total.map(|g| g.cost24h).unwrap_or(listed_cost24h);
    // 合计只覆盖「取到的那些源」——没取到的必须当场说出来，否则一个总量读起来像全机真相
    let unmeasured: Vec<&AgentSnapshot> = snapshots
        .iter()
        .filter(|s| s.token_usage.is_none())
        .collect();

    md.push_str(&format!(
        "- **在线运行中**：{running_count} 个 (其中 {working_count} 个工作中)\n"
    ));
    md.push_str(&format!(
        "- **24h Token 消耗**：{} tokens",
        crate::tokens::compact(total_tokens24h)
    ));
    if total_cost24h > 0.0 {
        md.push_str(&format!(" ({})", crate::cost::format_cost(total_cost24h)));
    }
    md.push('\n');
    if !unmeasured.is_empty() {
        md.push_str(&format!(
            "  ·  其中 {} 个智能体**本轮没取到用量**（不计入上面的合计，表里标 `—`；\
             `—` 是「没查」，`0` 才是「查了、确实是零」）\n",
            unmeasured.len()
        ));
    }
    // 两份口径都摆出来，而不是悄悄选一个：差额是「离线但有历史」「被宿主去重」这类
    // 在册范围差异，读报告的人需要知道
    if let Some(grand) = grand_total {
        if grand.tokens24h != listed_tokens24h {
            md.push_str(&format!(
                "  ·  口径：全部数据源总量（与面板汇总栏同源）；下表逐条相加为 {} tokens，\
                 差额来自离线但仍有用量记录、以及与宿主合并的内嵌组件\n",
                crate::tokens::compact(listed_tokens24h)
            ));
        }
    }
    md.push_str("\n---\n\n");

    // 1. 智能体健康与资源状态
    md.push_str("## 1. 智能体健康与系统负载\n\n");
    md.push_str("| 智能体 | 状态 | 进程 PID | CPU | 物理内存 | 健康评分 | 评级 | 诊断建议 |\n");
    md.push_str("| :--- | :--- | :--- | :--- | :--- | :--- | :--- | :--- |\n");
    for snap in snapshots {
        let report = snap.health.clone();
        let pid = snap
            .pid
            .map(|pid| pid.to_string())
            .unwrap_or_else(|| "—".to_string());
        let cpu = snap
            .cpu_percent
            .map(|cpu| format!("{cpu:.1}%"))
            .unwrap_or_else(|| "—（本拍无差分窗口）".to_string());
        let memory = if snap.memory_bytes > 0 {
            snap.memory_text.clone()
        } else {
            "—".to_string()
        };
        // 评级直接用 health 的措辞：曾经这里自己 switch 出「需关注 / 预警」，
        // 而 `get_report` 与详情卡说「需留意 / 异常」——同一个评级两份话
        md.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} | {} |\n",
            cell(&snap.name),
            status_with_provenance(snap),
            pid,
            cpu,
            memory,
            report.score,
            report.grade.label(),
            cell(&report.suggestion)
        ));
    }
    // 会话源不可读的 Agent 单独列出：健康评分只看进程/CPU/内存，读不到会话库时
    // 报告里只会是一片「待机」，等于把「解析器坏了」伪装成「智能体闲着」。
    // Swift 用的是 `sessionProbeHealth.diagnosticText`；Rust 的可观测性判定
    // （`observability::Verdict`）里已经有同一个信息（`BlindSessionSource` + 依据），
    // 所以这里用它而不是再引入一个字段。
    let blind: Vec<String> = snapshots
        .iter()
        .filter(|s| s.observability.code == Code::BlindSessionSource)
        .map(|s| {
            let why = s
                .observability
                .evidence
                .first()
                .map(String::as_str)
                .unwrap_or(s.observability.summary);
            format!("- **{}**：{why}", cell(&s.name))
        })
        .collect();
    if !blind.is_empty() {
        md.push_str("\n### 会话源不可读（下述智能体的「待机」只代表没有信号）\n\n");
        md.push_str(&blind.join("\n"));
        md.push('\n');
    }
    md.push('\n');

    // 2. Token 消耗统计
    md.push_str("## 2. Token 与成本消耗总览\n\n");
    md.push_str("| 智能体 | 24h Token | 24h 成本 | 累计 Token | 累计成本 |\n");
    md.push_str("| :--- | :--- | :--- | :--- | :--- |\n");
    for snap in snapshots {
        // 整行 `—` = 这个源本轮没取到用量；`0` = 取到了、确实是零。
        // 把 nil 印成 0：一份运维报告会写成「这个工具没花钱」，而同一个快照在
        // `get_report --json` 里是 null——同一份数据两份结论，读报告的人无从核对。
        let Some(usage) = &snap.token_usage else {
            md.push_str(&format!("| {} | — | — | — | — |\n", cell(&snap.name)));
            continue;
        };
        md.push_str(&format!(
            "| {} | {} | {} | {} | {} |\n",
            cell(&snap.name),
            crate::tokens::compact(usage.tokens24h),
            cost_text(usage.cost24h),
            crate::tokens::compact(usage.tokens_total),
            cost_text(usage.cost_total)
        ));
    }
    md.push('\n');

    // 3. 近期任务与告警事件流水
    if !history.is_empty() {
        md.push_str("## 3. 近期生命周期与告警事件流水\n\n");
        md.push_str("| 时间 | 智能体 | 类型 | 耗时 | 摘要说明 |\n");
        md.push_str("| :--- | :--- | :--- | :--- | :--- |\n");
        for event in history.iter().take(20) {
            let duration = if event.duration > 0.0 {
                AgentTaskEvent::duration_text(event.duration)
            } else {
                "—".to_string()
            };
            md.push_str(&format!(
                "| {} | {} | {} | {} | {} |\n",
                timestamp_text(event.timestamp),
                cell(&event.agent_name),
                event_type_text(&event.event_type),
                duration,
                cell(&event.summary())
            ));
        }
        md.push('\n');
    }

    md.push_str("> 本报告由 AgentIsland 自动生成并导出。\n");
    md
}

/// 生成标准 CSV 报表（与 Swift 同表头、同列序）
/// 报表里那一格状态 = 状态标签 + 出处后缀（` · 自报` / ` · 自报冲突` / 空）。
///
/// 为什么必须带上：不带的话，报告里那份「工作中」可能来自**带令牌的自报**，
/// 而读者无从分辨——这正是对照表里记着的那条缺口（Swift 的 `AgentProvenance` 后缀）。
/// 后缀由 `selfreport` 拼好，报表不自己拼。
fn status_with_provenance(snap: &AgentSnapshot) -> String {
    format!("{}{}", snap.level_label, snap.provenance_suffix)
}

pub fn generate_csv(snapshots: &[AgentSnapshot], now_ms: i64) -> String {
    let mut csv = String::from(
        "Timestamp,AgentID,AgentName,Level,PID,CPU_Percent,Memory_Bytes,HealthScore,Grade,\
         Tokens_24h,Cost_24h,Tokens_Total,Cost_Total,Observability,ObservationEvidence\n",
    );
    let time_str = timestamp_text(now_ms);
    for snap in snapshots {
        let report = &snap.health;
        let verdict = &snap.observability;
        let pid = snap.pid.map(|pid| pid.to_string()).unwrap_or_default();
        let usage = snap.token_usage.as_ref();
        let row = [
            escape_csv(&time_str),
            escape_csv(&snap.id),
            escape_csv(&snap.name),
            escape_csv(snap.level.as_str()),
            pid,
            snap.cpu_percent.map(|cpu| format!("{cpu:.1}")).unwrap_or_default(),
            snap.memory_bytes.to_string(),
            report.score.to_string(),
            escape_csv(report.grade.label()),
            // 空 = 没取到（与同一行的 PID/CPU 列同口径），0 = 取到了确实是零。
            // 折成 0 会让电子表格把「监控不了」求和成「没花钱」。
            usage.map(|u| u.tokens24h.to_string()).unwrap_or_default(),
            usage.map(|u| format!("{:.4}", u.cost24h)).unwrap_or_default(),
            usage.map(|u| u.tokens_total.to_string()).unwrap_or_default(),
            usage.map(|u| format!("{:.4}", u.cost_total)).unwrap_or_default(),
            escape_csv(verdict.code.as_str()),
            escape_csv(verdict.evidence.first().map_or("", String::as_str)),
        ];
        csv.push_str(&row.join(","));
        csv.push('\n');
    }
    csv
}

/// 成本文本：零写 `$0.00`（与 Swift `TokenUsage.costText(_, zero: "$0.00")` 同口径）
fn cost_text(cost: f64) -> String {
    let text = crate::cost::format_cost(cost);
    if text.is_empty() {
        "$0.00".to_string()
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ActivityLevel, TokenUsage};

    fn usage(tokens24h: i64, cost24h: f64) -> TokenUsage {
        TokenUsage {
            tokens24h,
            tokens_total: tokens24h * 3,
            cost24h,
            cost_total: cost24h * 3.0,
            cost_estimated: false,
        }
    }

    fn snapshot(name: &str, level: ActivityLevel) -> AgentSnapshot {
        AgentSnapshot {
            installed: None,
            id: name.to_lowercase(),
            name: name.to_string(),
            glyph: String::new(),
            emoji: String::new(),
            level,
            level_label: level.as_str().to_string(),
            observability: crate::observability::Verdict {
                code: Code::Observed,
                summary: "有本机证据",
                evidence: vec!["采样到了进程".into()],
            },
            is_hung: Some(false),
            health: crate::health::Report {
                score: 100,
                grade: crate::health::Grade::Healthy,
                grade_label: "健康".into(),
                summary: String::new(),
                issues: vec![],
                suggestion: "一切正常".into(),
            },
            process_running: true,
            work_stats: crate::duration::Stats::empty(),
            provenance: None,
            provenance_suffix: String::new(),
            cpu_percent: Some(3.5),
            memory_bytes: 1024 * 1024 * 512,
            memory_text: "512 MB".into(),
            last_activity_text: "刚刚".into(),
            token_usage: Some(usage(1_200_000, 0.42)),
            pid: Some(4242),
            current_action: None,
            subagent_count: 0,
            session_probe_health: None,
        }
    }

    fn event(kind: &str, duration: f64) -> AgentTaskEvent {
        AgentTaskEvent {
            id: "e1".into(),
            agent_id: "claude".into(),
            agent_name: "Claude".into(),
            event_type: kind.into(),
            timestamp: 1_700_000_000_000,
            message: None,
            detail: None,
            duration,
            externally_delivered: false,
        }
    }

    /// 报表的状态列必须**带上出处后缀**：不带的话，报告里那份「工作中」可能来自
    /// 带令牌的自报，而读者无从分辨（这正是对照表里记着的那条缺口）。
    #[test]
    fn the_status_column_carries_the_provenance_suffix() {
        use crate::selfreport::Provenance;
        let mut self_reported = snapshot("claude", ActivityLevel::Working);
        self_reported.provenance = Some(Provenance::SelfReported);
        self_reported.provenance_suffix = Provenance::badge_suffix(self_reported.provenance);
        let observed = snapshot("claude", ActivityLevel::Working);

        // 夹具的 `level_label` 用的是机器串（`as_str`），生产侧用的是显示标签
        // （`level.label()`）——后缀拼接与这两者无关，所以这里按夹具的口径断言。
        assert_eq!(status_with_provenance(&self_reported), "working · 自报");
        assert_eq!(
            status_with_provenance(&observed),
            "working",
            "观测是常态：不该给它挂标签"
        );
        // 冲突那一拍也一样要能看出来
        let mut conflicted = snapshot("claude", ActivityLevel::Working);
        conflicted.provenance = Some(Provenance::Conflict);
        conflicted.provenance_suffix = Provenance::badge_suffix(conflicted.provenance);
        assert_eq!(status_with_provenance(&conflicted), "working · 自报冲突");
    }

    #[test]
    fn a_table_cell_cannot_inject_rows_or_sections() {
        // 攻击路径：`agentisland://notify?...message=x%0A%7C...` ⇒ 真换行 + 竖线
        let injected = format!("正常{}| 注入列{}## 注入小节", '\n', '\r');
        let out = cell(&injected);
        assert!(!out.contains('\n') && !out.contains('\r'), "换行必须被折成空格：{out:?}");
        assert!(out.contains("\\|"), "竖线必须转义：{out:?}");
        assert!(!out.contains(" | "), "转义后不该再出现未转义的列分隔符：{out:?}");
        // 反斜杠不转义（它不是列分隔符，转它只会让文本变样）
        assert_eq!(cell("C:\\path"), "C:\\path");

        // 端到端：注入者造不出新的一行。最强也最直接的判据是**行数不变**——
        // 与一个正常名字生成的报告逐行对比，注入只应改变单元格**内部**的文字。
        let benign = generate_markdown(
            &[snapshot("Evil", ActivityLevel::Idle)],
            &[],
            None,
            1_700_000_000_000,
        );
        let mut snap = snapshot("Evil", ActivityLevel::Idle);
        snap.name = injected;
        let malicious = generate_markdown(&[snap], &[], None, 1_700_000_000_000);
        assert_eq!(
            malicious.lines().count(),
            benign.lines().count(),
            "换行被折成空格后，注入不该多出任何一行"
        );
        // 每一行的列分隔符数量也必须与该节表头一致（健康表 8 列 ⇒ 9 根）
        for line in malicious.lines() {
            if !line.starts_with('|') {
                continue;
            }
            let unescaped = line
                .char_indices()
                .filter(|(i, c)| *c == '|' && (*i == 0 || !line[..*i].ends_with('\\')))
                .count();
            assert!(
                unescaped == 9 || unescaped == 6,
                "表格行的列分隔符数量异常（应为 9 或 6）：{line}"
            );
        }
        assert!(malicious.contains("\\|"), "注入的竖线应被转义：{malicious}");
    }

    #[test]
    fn a_missing_usage_reads_as_not_measured_instead_of_zero() {
        let mut with = snapshot("Claude", ActivityLevel::Working);
        with.token_usage = Some(usage(1_200_000, 0.42));
        let mut without = snapshot("Gemini", ActivityLevel::Working);
        without.token_usage = None;

        let md = generate_markdown(&[with.clone(), without.clone()], &[], None, 1_700_000_000_000);
        assert!(md.contains("其中 1 个智能体**本轮没取到用量**"), "{md}");
        assert!(md.contains("| Gemini | — | — | — | — |"), "没取到要印整行 —：{md}");
        assert!(md.contains("| Claude | 1.20M | $0.42 | 3.60M | $1.26 |"), "{md}");

        // CSV 同口径：空列而不是 0
        let csv = generate_csv(&[with, without], 1_700_000_000_000);
        let gemini_row = csv.lines().find(|l| l.contains("Gemini")).unwrap();
        let columns: Vec<&str> = gemini_row.split(',').collect();
        assert_eq!(columns[9], "", "Tokens_24h 该是空列：{gemini_row}");
        assert_eq!(columns[10], "", "Cost_24h 该是空列：{gemini_row}");
        let claude_row = csv.lines().find(|l| l.contains("Claude")).unwrap();
        assert!(claude_row.contains(",1200000,0.4200,3600000,1.2600,"), "{claude_row}");
    }

    #[test]
    fn the_cross_source_total_and_the_listed_sum_are_both_stated_when_they_differ() {
        let snaps = vec![snapshot("Claude", ActivityLevel::Working)];
        let listed = usage(1_200_000, 0.42);
        // 面板那份总量更大（离线但有用量的源不在这份列表里）
        let grand = usage(9_600_000, 3.36);
        let md = generate_markdown(&snaps, &[], Some(&grand), 1_700_000_000_000);
        assert!(md.contains("**24h Token 消耗**：9.60M tokens"), "头条要用面板口径：{md}");
        assert!(md.contains("下表逐条相加为 1.20M tokens"), "差额必须写出来：{md}");
        assert!(md.contains("差额来自离线但仍有用量记录"), "{md}");

        // 两边一致时不该多嘴
        let same = generate_markdown(&snaps, &[], Some(&listed), 1_700_000_000_000);
        assert!(!same.contains("差额来自"), "一致时不该提差额：{same}");
    }

    #[test]
    fn the_report_carries_the_real_duration_and_marks_not_applicable_with_a_dash() {
        let history = vec![event("completed", 192.0), event("attention", 0.0)];
        let md = generate_markdown(&[], &history, None, 1_700_000_000_000);
        assert!(md.contains("| 3分12秒 |"), "完成事件要带精确时长：{md}");
        assert!(md.contains("Task" ) == false);
        // 不适用（0）印 `—`，不是「0秒」——「没记」与「零秒」是两件事
        assert!(md.contains("| — | Claude 等待确认操作 |"), "{md}");
        assert!(!md.contains("0秒"), "不适用不该印 0 秒：{md}");
        assert!(md.contains("Claude 任务完成 (3分12秒)"), "摘要要带时长（Swift summaryText 同口径）：{md}");
    }

    #[test]
    fn blind_session_sources_are_called_out_because_idle_can_mean_no_signal() {
        let mut blind = snapshot("Qoder", ActivityLevel::Idle);
        blind.observability = crate::observability::Verdict {
            code: Code::BlindSessionSource,
            summary: "会话源读不到",
            evidence: vec!["session_dirs 存在但当前用户不可读".into()],
        };
        let md = generate_markdown(&[blind], &[], None, 1_700_000_000_000);
        assert!(md.contains("### 会话源不可读"), "{md}");
        assert!(md.contains("**Qoder**：session_dirs 存在但当前用户不可读"), "{md}");
    }

    #[test]
    fn csv_escapes_quotes_and_commas_so_one_agent_cannot_shift_columns() {
        let mut snap = snapshot("a,b\"c", ActivityLevel::Working);
        snap.id = "id,with,commas".into();
        let csv = generate_csv(&[snap], 1_700_000_000_000);
        let row = csv.lines().nth(1).unwrap();
        assert!(row.contains("\"id,with,commas\""), "{row}");
        assert!(row.contains("\"a,b\"\"c\""), "{row}");
        // 表头列数 = 每行列数（转义正确时分割不会多出列）
        assert_eq!(csv.lines().next().unwrap().split(',').count(), 15);
    }

    #[test]
    fn the_filename_shape_is_stable_because_people_script_around_it() {
        let name = default_filename("md", 1_700_000_000_000);
        assert!(name.starts_with("AgentIsland_Audit_"), "{name}");
        assert!(name.ends_with(".md"), "{name}");
        let stamp = &name["AgentIsland_Audit_".len()..name.len() - 3];
        assert_eq!(stamp.len(), 15, "{name}");
        assert_eq!(&stamp[8..9], "_", "{name}");
        assert!(stamp.chars().filter(|c| *c == '_').count() == 1, "{name}");
    }

    #[test]
    fn the_observability_code_in_csv_matches_the_json_spelling() {
        // CSV 里写 `blindSessionSource`、JSON 里写别的，会让「用脚本核对报告」的人对不上
        for code in [
            Code::Observed,
            Code::BlindSessionSource,
            Code::NoLocalData,
            Code::SourceNotWired,
            Code::NotInstalled,
        ] {
            let json = serde_json::to_string(&code).unwrap();
            let json = json.trim_matches('"');
            assert_eq!(code.as_str(), json, "{code:?} 的两种写法不一致");
        }
    }

    #[test]
    fn an_export_carries_a_filename_from_the_same_instant_as_its_content() {
        let export = markdown_export(&[], &[], None, 1_700_000_000_000);
        assert!(export.filename.starts_with("AgentIsland_Audit_"), "{export:?}");
        assert!(export.filename.ends_with(".md"), "{export:?}");
        // 文件名里那串时间戳必须与报告正文里的「生成时间」是同一拍
        let stamp = &export.filename["AgentIsland_Audit_".len()..export.filename.len() - 3];
        let compact: String = stamp.chars().filter(|c| c.is_ascii_digit()).collect();
        let body: String = timestamp_text(1_700_000_000_000)
            .chars()
            .filter(|c| c.is_ascii_digit())
            .collect();
        assert_eq!(compact, body, "文件名与正文的时间戳应同源：{export:?}");

        let csv = csv_export(&[], 1_700_000_000_000);
        assert!(csv.filename.ends_with(".csv"), "{csv:?}");
    }

    #[test]
    fn an_empty_snapshot_set_still_produces_a_well_formed_report() {
        let md = generate_markdown(&[], &[], None, 1_700_000_000_000);
        assert!(md.starts_with("# AgentIsland"), "{md}");
        assert!(md.contains("**监控智能体总数**：0 个"), "{md}");
        assert!(md.contains("本报告由 AgentIsland 自动生成并导出"), "{md}");
        assert!(!md.contains("## 3."), "没有事件就不该有第 3 节：{md}");
        let csv = generate_csv(&[], 1_700_000_000_000);
        assert_eq!(csv.lines().count(), 1, "只有表头：{csv}");
    }
}
