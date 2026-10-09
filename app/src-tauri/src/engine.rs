use crate::filemon::{time_ago_text, FileMonitor};
use crate::health;
use crate::installed::{self, InstalledApps};
use crate::models::*;
use crate::notifier;
use crate::observability::{self, Evidence};
use crate::procmon::{memory_text, ProcessMonitor};
use crate::remote;
use crate::render;
use crate::resilience;
use crate::session::{self, Signal};
use crate::settings::Settings;
use crate::tokens::now_ms;
use crate::tokens::TokenUsageMonitor;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::mpsc::Receiver;
use std::time::SystemTime;

/// 五态状态机引擎（与 macOS 端 ActivityEngine 同规则）：
/// · 未解决的确认/授权请求            → attention
/// · 本轮明确结束（有写入证据）        → completed
/// · 进程在 + 写入 60s 内 或 CPU≥阈值 → working
/// · 进程在但静默                      → idle
/// · 进程不在                          → offline（可见口径隐藏）
/// 「读不到」这条故障的保质期。
///
/// 挂了 10 分钟：源恢复之后若没有新写入（探测被跳过），旧故障会一直挂着，
/// 于是「读不到」反过来伪装成「坏了」——同样是 CONTEXT.md 反对的谎报。
const HEALTH_TTL_MS: i64 = 10 * 60 * 1000;

/// 连续多少档超阈值才发 token 暴涨告警。与 Swift `ActivityEngine.tokenSpikeConfirmations` 同值。
pub const TOKEN_SPIKE_CONFIRMATIONS: u32 = 3;

mod tool_budgets;
pub(crate) use tool_budgets::ToolBudgetReport;

pub struct ActivityEngine {
    pub settings: Settings,
    pub demo_mode: bool,
    /// **这一轮要不要真去取 token 用量。**
    ///
    /// 常驻引擎是 `true`（界面上要显示用量）；而 CLI 的**单拍**入口
    /// （`status` / `report`）默认 `false`——同步解析全部会话索引本机实测多花 5 秒，
    /// 对 Raycast / 脚本调用不划算。不取时那一列印「—」**明说没取**，
    /// 而不是印 0（0 是「查了确实是零」，那是两件事）。
    pub refresh_usage: bool,
    diagnostics: Option<crate::resource_diagnostics::Tick>,
    profiles: Vec<AgentProfile>,
    procmon: ProcessMonitor,
    filemon: FileMonitor,
    /// 装机探测（Swift `InstalledAppsCache`）：TTL 缓存，未热时一律「未核实」。
    /// 「没核实」不能降级成「没装」——那会给用户一批凭空来的结论。
    installed: InstalledApps,
    tokens: TokenUsageMonitor,
    /// Last real work evidence, never refreshed by a hysteresis-only sample.
    last_work_signal_at: HashMap<String, i64>,
    work_started_at: HashMap<String, i64>,
    alerted_fingerprints: HashSet<String>,
    last_completed_fp: HashMap<String, String>,
    high_cpu_since: HashMap<String, i64>,
    /// 连续观测起点：进程在跑的每一拍续上，进程消失即作废。
    /// 它是「卡死」判定的**资格**——死锁是「CPU 连续超阈值达 5 分钟」的时间性判定，
    /// 而前提是这段窗口确实被观测过。资格写在数据里、不靠调用方自报，见 [`crate::health::is_hung`]。
    observed_running_since: HashMap<String, i64>,
    /// 异常驻留 / 死锁持续守护（[`crate::resilience::Guard`]）
    resilience: resilience::Guard,
    /// 外发调度与记账（[`crate::notifier::Notifier`]）
    pub notifier: notifier::Notifier,
    last_cost_spike: HashMap<String, i64>,
    token_rate: HashMap<String, (i64, i64)>,
    /// 连续多少档超阈值才发 token 暴涨告警（Swift `tokenSpikeConfirmations`）。
    /// 没有它，一次性账本补写就会误报成「消耗突增」。
    token_spike_streak: HashMap<String, u32>,
    token_cache: HashMap<String, (i64, TokenReport)>,
    pub event_rx: Option<Receiver<AgentTaskEvent>>,

    pub task_sources: HashMap<String, crate::task_sources::Choice>,
    #[cfg(target_os = "macos")]
    pub claude_plan_runtime:
        Option<std::sync::Arc<std::sync::Mutex<crate::claude_plan_runtime::Runtime>>>,
    session_navigation: HashMap<String, crate::session_navigation::Target>,
    event_navigation: HashMap<String, crate::session_navigation::Target>,
    pub latest_event: Option<AgentTaskEvent>,
    /// 已发出但用户还没确认的事件队列，`latest_event` 是它的队首。
    ///
    /// 此前 `push_event` 是**覆盖** `latest_event`：同一拍里两条告警只活一条，
    /// 用户永远看不到先发的那条（卡死与内存同时到点就是这个形状）。
    /// 现在一条一条地展示、确认一条推下一条，**不丢**。
    pending_events: VecDeque<AgentTaskEvent>,
    pub grand_total: TokenUsage,
    /// 可信自报（Swift `ActivityEngine.selfReports`）：令牌放行、TTL 内的那些话
    pub self_reports: crate::selfreport::Registry,
    /// 任务耗时与效率统计（Swift `ActivityEngine.durationTracker`）
    pub durations: crate::duration::TaskDurationTracker,
    /// Token 预算告警状态机（Swift `ActivityEngine.budgetTracker`）
    pub budget: crate::budget::BudgetTracker,
    /// 当前预算状态（界面绑定；Swift 侧是 `@Published budgetStatus`）
    pub budget_status: crate::budget::BudgetStatus,
    tool_budgets: tool_budgets::ToolBudgetTrackers,
    pub snapshots: Vec<AgentSnapshot>,
}

fn has_usage_location(profile: &AgentProfile) -> bool {
    profile.token_roots.iter().any(|root| {
        std::path::Path::new(root).is_dir()
            || (std::path::Path::new(root).is_file() && root.ends_with(".sqlite"))
    }) || profile.session_database.as_ref().is_some_and(|database| {
        matches!(
            database.schema,
            SessionSchema::OpenCode | SessionSchema::DimTasks | SessionSchema::MiniMaxRuntime
        ) && std::path::Path::new(&database.path).is_file()
    })
}

impl ActivityEngine {
    pub fn new(settings: Settings, event_rx: Receiver<AgentTaskEvent>) -> Self {
        let profiles = crate::registry::builtin();
        let installed = InstalledApps::new(InstalledApps::cli_names_for(&profiles));
        ActivityEngine {
            settings,
            demo_mode: false,
            // 默认开；CLI 单拍入口按需关掉
            refresh_usage: true,
            diagnostics: None,
            profiles,
            procmon: ProcessMonitor::new(),
            filemon: FileMonitor::new(),
            installed,
            tokens: TokenUsageMonitor::new(),
            last_work_signal_at: HashMap::new(),
            work_started_at: HashMap::new(),
            alerted_fingerprints: HashSet::new(),
            last_completed_fp: HashMap::new(),
            high_cpu_since: HashMap::new(),
            observed_running_since: HashMap::new(),
            resilience: resilience::Guard::default(),
            notifier: notifier::Notifier::new(),
            last_cost_spike: HashMap::new(),
            token_rate: HashMap::new(),
            token_spike_streak: HashMap::new(),
            token_cache: HashMap::new(),
            event_rx: Some(event_rx),
            task_sources: HashMap::new(),
            #[cfg(target_os = "macos")]
            claude_plan_runtime: None,
            session_navigation: HashMap::new(),
            event_navigation: HashMap::new(),
            latest_event: None,
            pending_events: VecDeque::new(),
            grand_total: TokenUsage::default(),
            self_reports: crate::selfreport::Registry::new(),
            durations: crate::duration::TaskDurationTracker::new(),
            budget: crate::budget::BudgetTracker::new(),
            budget_status: crate::budget::BudgetStatus::Disabled,
            tool_budgets: tool_budgets::ToolBudgetTrackers::default(),
            snapshots: vec![],
        }
    }

    /// Explicit observer mode; also prevents all outbound scheduling and secret reads.
    pub fn enable_resource_diagnostics(&mut self) {
        self.diagnostics = Some(Default::default());
    }
    pub fn resource_diagnostics(&self) -> Option<&crate::resource_diagnostics::Tick> {
        self.diagnostics.as_ref()
    }

    fn enabled_profiles(&self) -> Vec<AgentProfile> {
        self.profiles
            .iter()
            .filter(|p| !self.settings.disabled_agents.contains(&p.id))
            .cloned()
            .collect()
    }

    /// 采样一拍并返回完整状态（在引擎线程执行）
    pub fn tick(&mut self) {
        use crate::resource_diagnostics::{elapsed, Tick};
        let tracking = self.diagnostics.is_some();
        let start = tracking.then(std::time::Instant::now);
        let mut measured = Tick::default();
        self.task_sources.clear();
        if self.demo_mode {
            self.apply_demo();
            if tracking {
                measured.demo = true;
                measured.total_us = elapsed(start);
                measured.other_us = measured.total_us;
                self.diagnostics = Some(measured);
            }
            return;
        }

        let cpu_threshold = self.settings.cpu_threshold;
        // 这两个此前是硬编码字面量，Swift 侧同名字段是可在设置页调的
        let working_window = self.settings.working_window;
        let min_working_hold = self.settings.min_working_hold;

        let phase = tracking.then(std::time::Instant::now);
        self.procmon.refresh();
        measured.process_us = elapsed(phase);
        let now = now_ms();

        // 装机探测按 TTL 刷（默认 5 分钟）：`/Applications` 枚举 + 逐个读 Info.plist
        // 在应用多时要几十毫秒，不该每拍都做。未热时全部档案的 `installed` 是 `None`，
        // 也就是「未核实」——这正是我们要的默认：宁可先不给结论。
        let phase = tracking.then(std::time::Instant::now);
        self.installed.refresh_if_needed(installed::DEFAULT_MAX_AGE);
        measured.installed_us = elapsed(phase);

        // 自报到期：**只盖戳，不删记录**，且放在采样里而不是另开定时器——
        // 到期与否只取决于墙钟，而采样每拍本来就在读同一个 `now`（Swift 同一条理由）
        self.self_reports.sweep_expired(now);
        self.session_navigation.clear();
        let mut list: Vec<AgentSnapshot> = Vec::new();
        let mut total24 = 0i64;
        let mut total_all = 0i64;
        let mut cost24 = 0f64;
        let mut cost_all = 0f64;
        // 汇总位也要带估算标记：只要有一个档案的成本是估的，汇总就不是记录值
        let mut cost_estimated = false;

        for profile in self.enabled_profiles() {
            measured.profiles += 1;
            let hits = self.procmon.match_profile(&profile);
            let process_running = !hits.is_empty();
            let pid = hits.iter().max_by_key(|h| h.memory).map(|h| h.pid);
            let memory: u64 = hits.iter().map(|h| h.memory).sum();
            // 多进程档案（Electron）取最大 CPU 分量作代表
            let cpu: Option<f64> =
                hits.iter()
                    .filter_map(|h| h.cpu)
                    .fold(None, |acc: Option<f64>, c| {
                        Some(match acc {
                            Some(a) => a.max(c),
                            None => c,
                        })
                    });

            let phase = tracking.then(std::time::Instant::now);
            let file_result = self
                .filemon
                .probe(&profile, self.settings.active_session_window);
            let candidates = self.filemon.probe_files(&profile);
            measured.files_us += elapsed(phase);
            measured.candidates += candidates.len();
            let phase = tracking.then(std::time::Instant::now);
            let (probe, mut context) = self.probe_cached_multi(
                &profile.id,
                profile.session_dialect,
                &candidates,
                profile.session_database.as_ref(),
                now,
            );
            measured.sessions_us += elapsed(phase);
            #[cfg(target_os = "macos")]
            if profile.id == "claude" {
                if let Some(runtime) = &self.claude_plan_runtime {
                    if let Ok(mut runtime) = runtime.lock() {
                        let roots = profile
                            .session_dirs
                            .iter()
                            .map(std::path::PathBuf::from)
                            .collect::<Vec<_>>();
                        runtime.with_cache(|capture| {
                            crate::task_artifacts::attach_capture(
                                &mut context,
                                probe.signal.as_ref(),
                                &roots,
                                capture,
                            )
                        });
                    }
                }
            }
            if let Some(target) =
                crate::session_navigation::resolve(&profile, context.source_path.as_deref())
            {
                self.session_navigation.insert(profile.id.clone(), target);
            } else {
                self.session_navigation.remove(&profile.id);
            }
            let source_age = crate::task_sources::source_age(&context, now);
            if crate::task_sources::active_is_eligible(
                probe.signal.as_ref(),
                process_running,
                source_age,
                working_window,
            ) {
                if let Some(choice) =
                    crate::task_sources::from_context(&profile, &context, probe.signal.as_ref())
                {
                    self.task_sources.insert(profile.id.clone(), choice);
                }
            }
            let level = self.decide_level(
                &profile,
                now,
                process_running,
                cpu,
                cpu_threshold,
                working_window,
                min_working_hold,
                &file_result,
                &probe,
                pid,
            );

            // 对外那一拍：有可信自报就采信自报；与**强语义观测**对不上时标冲突并仍按观测走。
            // `has_session_signal` 直接从这一拍的 probe 拿，不需要 decide_level 回传
            // （它本来就把 probe 交给了调用方）。
            let report = self.self_reports.believable(&profile.id, now).cloned();
            // 探测健康**由引擎按采样时钟盖章**（不是构造点取当前时间）：
            // 合成时间的测试才能稳定判定保质期
            let mut probe = probe;
            if let Some(mut health) = probe.health.take() {
                health.observed_at = now;
                probe.health = Some(health);
            }
            let has_session_signal = probe.signal.is_some();
            let (level, provenance) =
                crate::selfreport::resolve(level, has_session_signal, report.as_ref());
            if matches!(
                provenance,
                Some(
                    crate::selfreport::Provenance::SelfReported
                        | crate::selfreport::Provenance::Conflict
                )
            ) {
                // Do not attach an observed file's conversation to a different self-report.
                if let Some(target) = crate::session_navigation::resolve(&profile, None) {
                    self.session_navigation.insert(profile.id.clone(), target);
                }
            }

            let phase = tracking.then(std::time::Instant::now);
            let token_usage = self.token_usage_cached(&profile);
            measured.tokens_us += elapsed(phase);
            let installed_state = self.installed.is_installed(&profile);
            let observability =
                observability::evaluate(&Evidence {
                    level,
                    process_running,
                    installed: installed_state,
                    provenance,
                    // 原因链：**原文**带上，并按保质期判断它还算不算数。
                    // 目录层那道粗判（元数据/列举失败）作为兜底——它覆盖不到深层读取失败，
                    // 但那也不是「一切正常」的证据
                    probe_health: probe.health.as_ref().map(|h| h.diagnostic_text()).or_else(
                        || {
                            (process_running
                                && level == ActivityLevel::Idle
                                && observability::has_unreadable_source(&profile))
                            .then(|| "已登记的本地会话源无法枚举；待机不代表真的空闲".to_string())
                        },
                    ),
                    probe_health_fresh: probe
                        .health
                        .as_ref()
                        .map(|h| h.is_fresh(now, HEALTH_TTL_MS))
                        .unwrap_or(true),
                    has_local_detail_source: observability::has_local_detail_source(&profile),
                    active_sessions: file_result.active_sessions,
                    has_token_usage: token_usage.as_ref().is_some_and(|u| u.tokens_total > 0),
                });
            if let Some(u) = &token_usage {
                total24 += u.tokens24h;
                total_all += u.tokens_total;
                cost24 += u.cost24h;
                cost_all += u.cost_total;
                cost_estimated = cost_estimated || u.cost_estimated;
            }

            let last_ago = file_result
                .latest_write
                .and_then(|t| t.elapsed().ok())
                .map(|d| d.as_secs_f64());

            // 卡死三态：资格（连续观测够不够久）与事实（高 CPU 持续够不够久）都来自本引擎的
            // 跨拍状态，所以判定放在这里、由 `health::is_hung` 一处实现
            // 持续时长与告警侧**同一个数**：可配之后两处各读各的，
            // 就会出现「告警响了、健康度还说不卡死」而两边都不报错
            let is_hung = health::is_hung(
                self.observed_running_since.get(&profile.id).copied(),
                self.high_cpu_since.get(&profile.id).copied(),
                now,
                (self.settings.runaway_duration_threshold * 1000.0) as i64,
            );

            let work_stats =
                self.durations
                    .stats(&profile.id, crate::duration::DEFAULT_WINDOW_MS, now);
            let mut snapshot = AgentSnapshot {
                id: profile.id.clone(),
                name: profile.name.clone(),
                glyph: profile.glyph.clone(),
                emoji: profile.emoji.clone(),
                level,
                level_label: level.label().to_string(),
                observability,
                is_hung,
                // 先占位再就地求值：健康度是**由这份快照自身**推出来的，
                // 用 not_running() 占位只是为了满足结构体字面量，紧接着就被覆盖
                health: health::Report::not_running(),
                process_running,
                installed: installed_state,
                cpu_percent: cpu,
                work_stats,
                memory_bytes: memory,
                memory_text: memory_text(memory),
                // None 的文案在这一处决定（`—`），不在调用点各写一遍
                last_activity_text: time_ago_text(last_ago),
                token_usage,
                pid,
                // 冲突那一拍不许拿自报的句子去填动作行：那等于在两处（副标题与动作条）
                // 各替用户挑了一次，而这一维存在的理由就是「两条都给」
                current_action: if provenance == Some(crate::selfreport::Provenance::Conflict) {
                    None
                } else {
                    match &probe.signal {
                        Some(Signal::Active(_, action)) => action.clone(),
                        Some(Signal::Attention(_, msg)) => Some(msg.clone()),
                        _ => report
                            .as_ref()
                            .and_then(|r| r.ask.clone().or_else(|| r.detail.clone())),
                    }
                },
                provenance,
                // 后缀由 Rust 拼好（" · 自报" / " · 自报冲突" / 空）：界面各自拼一遍就会漂
                provenance_suffix: crate::selfreport::Provenance::badge_suffix(provenance),
                subagent_count: probe.subagent_count,
                session_probe_health: None,
                // 上下文由本轮探测带出（`SessionProbe.context`），不跨拍存表
                background_tasks: context.background_tasks,
                subagents: context.subagents,
                token_breakdown: context.token_breakdown,
            };
            snapshot.health = health::evaluate(&snapshot);
            list.push(snapshot);
        }

        list.sort_by(|a, b| {
            b.level
                .cmp(&a.level)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });

        // 异常驻留与死锁持续守护（Swift 侧由 `autoAnomaliesAlertEnabled` 控制，默认开）。
        // 放在排序之后、赋值之前：守护吃的是**这一拍的快照**，而 `list` 此刻还是局部量，
        // 于是不会和 `self.resilience` 的可变借用打架
        self.publish_guard_alerts(&list, now);
        for snapshot in &mut list {
            snapshot.health = health::evaluate_with_memory_growth(
                snapshot,
                self.resilience.memory_growth(&snapshot.id),
            );
        }

        self.snapshots = list;
        self.grand_total = TokenUsage {
            tokens24h: total24,
            tokens_total: total_all,
            cost24h: cost24,
            cost_total: cost_all,
            cost_estimated,
        };

        // 预算评估放在总量算完、快照落地之后（Swift 侧同位置：`grandTotal` 一更新就评估）。
        // 单独抽成方法是为了让这段接线的口径能被用例直接观察。
        self.evaluate_budget(now);
        self.evaluate_tool_budgets(now);
        if tracking {
            let phase = std::time::Instant::now();
            measured.file_cache = self.filemon.diagnostic_cache();
            measured.token_cache = self.tokens.diagnostic_cache();
            measured.cache_stats_us = elapsed(Some(phase));
            measured.total_us = elapsed(start);
            measured.other_us = measured.total_us.saturating_sub(
                measured.process_us
                    + measured.installed_us
                    + measured.files_us
                    + measured.sessions_us
                    + measured.tokens_us
                    + measured.cache_stats_us,
            );
            self.diagnostics = Some(measured);
        }
    }

    /// Token 预算预警与超额告警。
    ///
    /// 口径照搬 Swift `ActivityEngine`：
    /// ① 由 `budgetAlertEnabled` 决定「要不要评估」（关掉只关告警，状态照样给界面）；
    /// ② 只有**跨级**才推事件（状态机里带滞回，否则只要用量压在线上就每拍推一条）；
    /// ③ 事件写成 `system` / 「Token 预算」+ `costSpike`，消息按「超额 / 预警」分开，
    ///    具体数字落在 `detail` 里——界面上标题要短，数字要能查。
    fn evaluate_budget(&mut self, now: i64) {
        let settings = self.settings.normalized();
        let used = self.grand_total.tokens24h;
        let budget = settings.daily_token_budget;
        if budget > 0 && !settings.budget_alert_enabled {
            // 关掉告警时仍要让界面看得到用量与预算（Swift 同分支）
            self.budget_status = crate::budget::BudgetStatus::Normal {
                used,
                budget,
                ratio: used as f64 / budget as f64,
            };
            return;
        }
        if budget <= 0 {
            // 没设预算：把状态机复位（否则用户设上预算的那一刻会莫名报一次）
            self.budget.reset();
            self.budget_status = crate::budget::BudgetStatus::Disabled;
            return;
        }
        let (status, alert) = self.budget.evaluate(used, budget, now);
        let exceeded = status.is_exceeded();
        self.budget_status = status;
        if let Some(detail) = alert {
            self.push_event(AgentTaskEvent {
                id: crate::webhook::webhook_uuid(),
                agent_id: "system".into(),
                agent_name: "Token 预算".into(),
                event_type: "costSpike".into(),
                timestamp: now,
                message: Some(if exceeded {
                    "🚨 Token 预算超额".into()
                } else {
                    "⚠️ Token 预算预警".into()
                }),
                detail: Some(detail),
                duration: 0.0,
                externally_delivered: false,
            });
        }
    }

    #[allow(clippy::too_many_arguments)]
    /// `pub(crate)`：自检（`selftest.rs`）要用它复现判定路径——自检与单元测试跑同一段决策代码，
    /// 而不是各写一份「差不多」的判据。
    pub(crate) fn decide_level(
        &mut self,
        profile: &AgentProfile,
        now: i64,
        process_running: bool,
        cpu: Option<f64>,
        cpu_threshold: f64,
        working_window: f64,
        min_working_hold: f64,
        file: &crate::filemon::FileActivityResult,
        probe: &session::SessionProbe,
        pid: Option<u32>,
    ) -> ActivityLevel {
        let key = profile.id.clone();
        // Match Swift's clock-rewind protection: future anchors must not extend a hold forever.
        for anchors in [
            &mut self.last_work_signal_at,
            &mut self.work_started_at,
            &mut self.high_cpu_since,
            &mut self.observed_running_since,
        ] {
            if let Some(since) = anchors.get_mut(&key) {
                if *since > now {
                    *since = now;
                }
            }
        }
        if !process_running {
            self.last_work_signal_at.remove(&key);
            self.work_started_at.remove(&key);
            self.high_cpu_since.remove(&key);
            self.observed_running_since.remove(&key);
            self.token_rate.remove(&key);
            return ActivityLevel::Offline;
        }
        // 进程在跑 ⇒ 续上连续观测窗口（首次插入即起点）
        self.observed_running_since
            .entry(key.clone())
            .or_insert(now);

        // 资源证据独立于活动等级；确认/完成的提前返回也不能跳过它。
        // 熔断：CPU 连续 70% 以上达 5 分钟。阈值只从 health 那一个来源取——
        // 此前这里和健康度判定各写一遍 70.0 / 300_000，改一处就会让
        // 「告警会响」与「健康度说卡死」对不上
        if let Some(c) = cpu {
            // 开关关掉只关**告警**，不清 `high_cpu_since`：持续高负载的证据要留着，
            // 否则「健康度说卡死」与「告警不响」会变成两个互相矛盾的结论。
            if c >= self.settings.runaway_cpu_threshold {
                let since = *self.high_cpu_since.entry(key.clone()).or_insert(now);
                if now - since >= (self.settings.runaway_duration_threshold * 1000.0) as i64
                    && self.settings.runaway_cpu_alert
                {
                    self.raise_cost_spike(
                        profile,
                        pid,
                        &format!("CPU 持续 {:.0}% 已超过 {} 分钟", c, (now - since) / 60_000),
                        now,
                        "cpu",
                    );
                }
            } else {
                self.high_cpu_since.remove(&key);
            }
        }

        // 强语义：attention 优先（同一指纹只提醒一次）
        if let Some(Signal::Attention(fp, message)) = &probe.signal {
            let fp = fp.clone();
            let message = message.clone();
            if self.alerted_fingerprints.insert(fp.clone()) {
                self.push_session_event(AgentTaskEvent {
                    id: fp.clone(),
                    agent_id: key.clone(),
                    agent_name: profile.name.clone(),
                    event_type: "attention".into(),
                    timestamp: now,
                    message: Some(message),
                    detail: None,
                    duration: 0.0,
                    externally_delivered: false,
                });
                if self.alerted_fingerprints.len() > 800 {
                    self.alerted_fingerprints.clear();
                }
            }
            self.last_work_signal_at.remove(&key);
            self.work_started_at.remove(&key);
            return ActivityLevel::Attention;
        }

        // 强语义：本轮明确结束（有写入证据才宣布完成）
        let write_evidence = file
            .latest_write
            .map(|t| {
                let written_ms = match t.duration_since(SystemTime::UNIX_EPOCH) {
                    Ok(d) => d.as_secs_f64() * 1000.0,
                    Err(e) => -e.duration().as_secs_f64() * 1000.0,
                };
                // One sampling clock, including synthetic replay and future-mtime clamping.
                (now as f64 - written_ms).max(0.0) / 1000.0 <= working_window
            })
            .unwrap_or(false);
        if let Some(Signal::Completed(fp)) = &probe.signal {
            if write_evidence {
                let fp = fp.clone();
                let is_new = match self.last_completed_fp.get(&key) {
                    Some(prev) => *prev != fp,
                    None => true,
                };
                if is_new {
                    self.last_completed_fp.insert(key.clone(), fp.clone());
                    // 本次任务用时：此刻 `work_started_at` 还没被清（下面几行才 remove），
                    // 与 `notify_outbound` 取的是同一个起点——两处若各算一次就会打架
                    let seconds = self
                        .work_started_at
                        .get(&key)
                        .map(|started| ((now - *started).max(0) as f64) / 1000.0)
                        .unwrap_or(0.0);
                    // **够格才算一次任务**：Swift `recordTaskCompleted` 的门槛是 3.5 秒
                    // （「过滤瞬时微抖动」）。Rust 侧此前只有「保持 Working 的窗口」，
                    // 于是 0.2 秒的抖动也会记一笔并推一条「任务完成 (0秒)」。
                    // 不够格时**既不记录也不推事件**，但下面照旧清掉起点（这一轮确实结束了）
                    if seconds >= crate::duration::MIN_TASK_SECONDS {
                        self.durations.record(&key, seconds, now);
                    } else {
                        self.last_work_signal_at.remove(&key);
                        self.work_started_at.remove(&key);
                        return ActivityLevel::Completed;
                    }
                    self.push_session_event(AgentTaskEvent {
                        id: fp,
                        agent_id: key.clone(),
                        agent_name: profile.name.clone(),
                        event_type: "completed".into(),
                        timestamp: now,
                        message: None,
                        detail: None,
                        duration: seconds,
                        externally_delivered: false,
                    });
                }
                self.last_work_signal_at.remove(&key);
                self.work_started_at.remove(&key);
                return ActivityLevel::Completed;
            }
        }

        let effective_cpu_threshold = profile.cpu_floor.unwrap_or(0.0).max(cpu_threshold);
        let cpu_hot = cpu.map(|c| c >= effective_cpu_threshold).unwrap_or(false);
        let in_flight = matches!(probe.signal, Some(Signal::Active(_, _)));

        // 在途命令全周期拦截 + 滞回
        let was_working = self
            .snapshots
            .iter()
            .any(|s| s.id == key && s.level == ActivityLevel::Working);
        let holding = was_working
            && self
                .last_work_signal_at
                .get(&key)
                .map(|s| (now - *s) as f64 / 1000.0 < min_working_hold)
                .unwrap_or(false);

        let level = if in_flight || write_evidence || cpu_hot {
            self.work_started_at.entry(key.clone()).or_insert(now);
            self.last_work_signal_at.insert(key.clone(), now);
            ActivityLevel::Working
        } else if holding {
            ActivityLevel::Working
        } else {
            self.work_started_at.remove(&key);
            self.last_work_signal_at.remove(&key);
            ActivityLevel::Idle
        };

        // Token 暴涨告警：按「每分钟净增量」判定（与 macOS 端口径一致）；
        // 24h 累计值会长期越过阈值，不能作为触发条件
        if self.settings.token_alert_enabled {
            let t24 = self
                .snapshots
                .iter()
                .find(|s| s.id == key)
                .and_then(|s| s.token_usage.as_ref())
                .map(|u| u.tokens24h);
            if let Some(t) = t24 {
                let entry = self.token_rate.entry(key.clone()).or_insert((now, t));
                let elapsed_min = (now - entry.0) as f64 / 60_000.0;
                if elapsed_min >= 1.0 {
                    let rate = (t - entry.1) as f64 / elapsed_min;
                    *entry = (now, t);
                    // 实际阈值 = max(档案专属下限, 全局设置)。WorkBuddy 日常 3-5 专家并行
                    // 的高消耗不该误报，而超大规模死循环或用户自设更高档位仍能熔断。
                    let floor = profile.token_alert_floor.unwrap_or(0);
                    let effective = floor.max(self.settings.token_alert_threshold) as f64;
                    let streak = self.token_spike_streak.entry(key.clone()).or_insert(0);
                    if rate >= effective {
                        *streak += 1;
                        // **需连续多档超阈值才告警**：滤掉单次账本补写
                        // （长任务结束时一次性落盘会让那一拍的速率虚高）
                        if *streak >= TOKEN_SPIKE_CONFIRMATIONS {
                            self.raise_cost_spike(
                                profile,
                                pid,
                                &format!(
                                    "Token 消耗突增（近 {:.0} 分钟约 {} tokens/分钟）{}",
                                    elapsed_min,
                                    crate::tokens::compact(rate as i64),
                                    if floor > self.settings.token_alert_threshold {
                                        format!(
                                            "，含 {} 专家团保护下限 {}",
                                            profile.name,
                                            crate::tokens::compact(floor)
                                        )
                                    } else {
                                        String::new()
                                    }
                                ),
                                now,
                                "token-rate",
                            );
                        }
                    } else {
                        *streak = 0;
                    }
                }
            }
        }

        level
    }

    fn raise_cost_spike(
        &mut self,
        profile: &AgentProfile,
        pid: Option<u32>,
        message: &str,
        now: i64,
        once: &str,
    ) {
        let dedupe_key = format!("{}:{}", profile.id, once);
        if let Some(last) = self.last_cost_spike.get(&dedupe_key) {
            if now - *last < 600_000 {
                return;
            }
        }
        self.last_cost_spike.insert(dedupe_key, now);
        if self.last_cost_spike.len() > 200 {
            self.last_cost_spike.clear();
        }
        self.push_session_event(AgentTaskEvent {
            id: crate::webhook::webhook_uuid(),
            agent_id: profile.id.clone(),
            agent_name: profile.name.clone(),
            event_type: "costSpike".into(),
            timestamp: now,
            message: Some(message.to_string()),
            detail: None,
            duration: 0.0,
            externally_delivered: false,
        });
    }

    /// 最近的事件流水（队首在前，随后是待确认队列）。
    /// 审计报告要用它做「近期生命周期与告警事件流水」那一节；
    /// 之所以走访问器而不是把字段开成 `pub`：队列的**顺序语义**（队首=正在上屏的那条）
    /// 只有这一层知道，让调用方直接读裸 Vec 迟早会有人反着拼。
    pub fn recent_events(&self) -> Vec<AgentTaskEvent> {
        let mut events = Vec::new();
        if let Some(event) = &self.latest_event {
            events.push(event.clone());
        }
        events.extend(self.pending_events.iter().cloned());
        events
    }

    /// 入队一条事件。队首直接上屏，其余排队等确认——见 `pending_events` 的注释。
    ///
    /// 顺带把这条事件过一遍外发闸门并记账：**发不出去也要留痕**，
    /// 否则界面只能显示「最近没发过」，看不出是被静默时段挡下还是通道没配好。
    fn push_session_event(&mut self, event: AgentTaskEvent) {
        let target = self.session_navigation.get(&event.agent_id).cloned();
        let id = event.id.clone();
        self.push_event(event);
        if let Some(target) = target {
            self.event_navigation.insert(id, target);
        }
        let live: HashSet<String> = self.recent_events().iter().map(|e| e.id.clone()).collect();
        self.event_navigation.retain(|id, _| live.contains(id));
    }

    pub fn navigation_target(
        &self,
        agent: &str,
        event: Option<&str>,
    ) -> Result<crate::session_navigation::Target, String> {
        let profile = self
            .profiles
            .iter()
            .find(|p| p.id == agent)
            .ok_or("未找到智能体")?;
        let target = if let Some(event) = event {
            if !self
                .recent_events()
                .iter()
                .any(|e| e.id == event && e.agent_id == agent)
            {
                return Err("这条提醒已更新，请重试".into());
            }
            self.event_navigation.get(event).cloned()
        } else {
            self.session_navigation.get(agent).cloned()
        };
        target
            .or_else(|| crate::session_navigation::resolve(profile, None))
            .ok_or("此智能体尚未接入桌面工具跳转".into())
    }

    pub fn push_event(&mut self, event: AgentTaskEvent) {
        self.notify_outbound(&event);
        if self.latest_event.is_none() {
            self.latest_event = Some(event);
            return;
        }
        // 有界：用户长时间不确认时丢**最早**的（当前状态比历史告警更值得看）。
        // 64 条的余量远大于真实告警频率（同类告警有 10 分钟冷却），所以这是纯保险。
        const MAX_PENDING_EVENTS: usize = 64;
        if self.pending_events.len() >= MAX_PENDING_EVENTS {
            if let Some(discarded) = self.pending_events.pop_front() {
                self.event_navigation.remove(&discarded.id);
            }
        }
        self.pending_events.push_back(event);
    }

    /// 每个事件过一次外发闸门并记账。
    ///
    /// `completed` 的 `seconds` 是「本次任务用时」：此刻 `work_started_at` 还没被清
    /// （`decide_level` 里先 `push_event` 再 `remove`），正好拿得到；其余类型按「刚刚」。
    fn notify_outbound(&mut self, event: &AgentTaskEvent) {
        if self.diagnostics.is_some() || event.externally_delivered {
            return;
        }
        self.notify_outbound_with_presence(event, crate::power::presence_signals());
    }

    fn notify_outbound_with_presence(
        &mut self,
        event: &AgentTaskEvent,
        presence: remote::PresenceSignals,
    ) {
        if self.diagnostics.is_some() {
            return;
        }
        let Some(kind) = remote::EventKind::parse(&event.event_type) else {
            return;
        };
        let seconds = if kind == remote::EventKind::Completed {
            self.work_started_at
                .get(&event.agent_id)
                .map(|since| (event.timestamp - *since).max(0) as f64 / 1000.0)
                .unwrap_or(0.0)
        } else {
            0.0
        };
        let mut inputs = render::Inputs::new(event.agent_name.clone(), kind, seconds);
        inputs.agent_id = event.agent_id.clone();
        inputs.message = event.message.clone();
        inputs.action_detail = event.detail.clone();

        let (channel, _) = remote::resolve_kind(Some(self.settings.remote_kind.as_str()));
        let config = self
            .settings
            .remote_channels
            .get(channel.as_str())
            .cloned()
            .unwrap_or_default();
        let policy = self.settings.remote_policy.clone();
        // 密钥不在这里读：`Notifier` 自己在**策略闸门放行之后**去钥匙串取
        // （总开关关着时不碰它），并只在渲染请求时使用。
        // **走 dispatch（工作线程）**：传输最长 10 秒，且 https 接进来之后「会真去连」
        // 是常态路径——直接在采样线程里跑会让界面卡住。结果由工作线程写进账本。
        self.notifier.dispatch(
            inputs,
            policy,
            channel,
            config,
            remote::Now::at(event.timestamp),
            presence,
            false,
        );
    }

    /// 确认当前这条，推下一条（前端关掉横幅时调用）
    pub fn ack_latest_event(&mut self) {
        if let Some(event) = &self.latest_event {
            self.event_navigation.remove(&event.id);
        }
        self.latest_event = self.pending_events.pop_front();
    }

    /// 还没确认的事件条数（不含队首那条）
    pub fn pending_count(&self) -> usize {
        self.pending_events.len()
    }

    /// 把守护判出的告警发成事件。**抽出来是为了让 refresh 的那条接线可测**：
    /// 引擎真跑一拍要真进程真 CPU，用例里造不出来；而这段映射（含设置开关）
    /// 恰恰是「判定接上了没有」最容易出错的地方。
    fn publish_guard_alerts(&mut self, snapshots: &[AgentSnapshot], now: i64) {
        if !self.settings.auto_anomalies_alert {
            self.resilience.observe_memory(snapshots, now);
            return;
        }
        let alerts = self.resilience.evaluate(snapshots, now);
        for alert in alerts {
            self.push_session_event(AgentTaskEvent {
                id: crate::webhook::webhook_uuid(),
                agent_id: alert.agent_id,
                agent_name: alert.agent_name,
                event_type: "attention".into(),
                timestamp: now,
                message: Some(alert.message),
                detail: Some(format!(
                    "kind={} elapsed_ms={}",
                    alert.kind, alert.elapsed_ms
                )),
                duration: 0.0,
                externally_delivered: false,
            });
        }
    }

    /// 按候选顺序（新→旧）逐个探测，返回第一个有信号的；全无信号返回空探测。
    /// 最新的文件不一定是语义文件（如 CLI 日志比 rollout 更新）。
    #[allow(clippy::type_complexity)]
    fn probe_cached_multi(
        &mut self,
        profile_id: &str,
        dialect: crate::models::SessionDialect,
        paths: &[String],
        database: Option<&crate::models::SessionDatabase>,
        // 采样时钟：由引擎盖章，不由探测层自己取当前时间
        now: i64,
    ) -> (session::SessionProbe, crate::models::SessionActiveContext) {
        let mut candidate_failure = None;
        for path in paths {
            if path.is_empty() {
                continue;
            }
            // Binary session stores are handled by the declared database adapter,
            // not the UTF-8 tail reader; an idle healthy DB must not look unreadable.
            if std::path::Path::new(path)
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| {
                    ext.eq_ignore_ascii_case("db") || ext.eq_ignore_ascii_case("sqlite")
                })
            {
                continue;
            }
            // Cache raw input in the parser, never its state result: dialects
            // depend on file age and must re-evaluate even when metadata is stable.
            let (probe, mut context) = session::probe_dialect(profile_id, dialect, path);
            if probe.signal.is_some() {
                context.source_path = Some(path.clone());
                return (probe, context);
            }
            if candidate_failure.is_none() && probe.health.is_some() {
                candidate_failure = probe.health;
            }
        }
        // 文件这条路全落空之后才轮到**状态索引库**（Swift 同顺序：`probe` 的最后一步
        // 才是 `inspectKnownDatabase`）。它排在后面不是随便定的——部分桌面 Agent 的会话
        // 只写进 SQLite，FileMonitor 定位到的最新文件就是那个二进制库本身，
        // 对它做尾窗解析必然读不出东西。反过来把它放前面，则会让库里的旧状态
        // 盖过文件里刚发生的活动。
        let Some(database) = database.filter(|db| {
            matches!(
                db.schema,
                crate::models::SessionSchema::StatusIndex
                    | crate::models::SessionSchema::DimTasks
                    | crate::models::SessionSchema::OpenCode
                    | crate::models::SessionSchema::MiniMaxRuntime
            )
        }) else {
            return (
                session::SessionProbe {
                    signal: None,
                    subagent_count: 0,
                    health: candidate_failure,
                },
                crate::models::SessionActiveContext::default(),
            );
        };
        let file_age = std::fs::metadata(&database.path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .map(|d| d.as_secs_f64())
            .unwrap_or(f64::INFINITY);
        // **不是所有 schema 都走同一个函数**，走错的话那个 Agent 就永远没有信号——
        // 而界面上看不出异样：
        // · `DimTasks`：会话表不带终态 status，「封没封口」看最新一条 assistant
        //   最后一个 part 的 `endTime`；
        // · `OpenCode`：看消息行 `time.completed` / `time.created`，且表名要现查
        //   （这一族跨版本改过名）。
        let mut source_keys = None;
        let (probe, failure) = match database.schema {
            crate::models::SessionSchema::DimTasks => {
                session::probe_dim_source(database, file_age, &mut source_keys)
            }
            crate::models::SessionSchema::OpenCode => {
                session::probe_opencode_source(database, file_age, now, &mut source_keys)
            }
            crate::models::SessionSchema::MiniMaxRuntime => {
                crate::minimax::probe_source(database, now, &mut source_keys)
            }
            _ => session::probe_status_index_source(database, file_age, now, &mut source_keys),
        };

        (
            session::SessionProbe {
                signal: probe.signal,
                subagent_count: 0,
                health: failure
                    .map(|failure| session::SessionProbeHealth {
                        failure,
                        path: database.path.clone(),
                        observed_at: now,
                    })
                    .or(candidate_failure),
            },
            crate::models::SessionActiveContext {
                source_path: source_keys.as_ref().map(|_| database.path.clone()),
                source_keys,
                ..Default::default()
            },
        )
    }

    fn token_usage_cached(&mut self, profile: &AgentProfile) -> Option<TokenUsage> {
        if !self.refresh_usage || !has_usage_location(profile) {
            return None;
        }
        if let Some((fetched, report)) = self.token_cache.get(&profile.id) {
            if now_ms() - fetched < 20_000 {
                return Some(report.usage.clone());
            }
        }
        let report = self.tokens.monitor(profile);
        let usage = report.usage.clone();
        self.token_cache
            .insert(profile.id.clone(), (now_ms(), report));
        Some(usage)
    }

    pub fn get_report(&mut self, agent_id: &str) -> Option<TokenReport> {
        if self.demo_mode {
            return Some(demo_report());
        }
        let profile = self.profiles.iter().find(|p| p.id == agent_id)?.clone();
        // `get_report` 是**按需**取（分析页点进来才要），所以它**不受 `refresh_usage` 约束**——
        // 调用它的人已经明确要这一份数据了，再加一道开关只会让分析页永远是空的
        if !has_usage_location(&profile) {
            return None;
        }
        if let Some((_, report)) = self.token_cache.get(agent_id) {
            return Some(report.clone());
        }
        let report = self.tokens.monitor(&profile);
        self.token_cache
            .insert(agent_id.to_string(), (now_ms(), report.clone()));
        Some(report)
    }

    // MARK: 演示数据（对齐 macOS 端 site 截图）

    fn apply_demo(&mut self) {
        let mk = |id: &str, name: &str, glyph: &str, emoji: &str| AgentProfile {
            id: id.into(),
            name: name.into(),
            glyph: glyph.into(),
            emoji: emoji.into(),
            process_names: vec!["demo".into()],
            bundle_ids: vec![],
            cmdline_hints: vec![],
            path_excludes: vec![],
            path_contains: vec![],
            cpu_floor: None,
            session_dirs: vec![],
            token_roots: vec![],
            token_alert_floor: None,
            session_dialect: crate::models::SessionDialect::GenericTail,
            session_database: None,
            category: "assistant".into(),
        };
        let demo: Vec<(AgentProfile, ActivityLevel, u64, Option<String>, i64)> = vec![
            (
                mk("claude", "Claude", "\u{E8BD}", "🧠"),
                ActivityLevel::Attention,
                430 << 20,
                Some("需要确认: …ift test".into()),
                12_080_000,
            ),
            (
                mk("antigravity", "Antigravity", "\u{E72C}", "⚛️"),
                ActivityLevel::Working,
                310 << 20,
                Some("正在修改: IslandView.swift".into()),
                1_600_000_000,
            ),
            (
                mk("qoder", "Qoder", "\u{E943}", "🖥️"),
                ActivityLevel::Working,
                877 << 20,
                Some("运行: chmod +x .scr.te-shots/shoot4.sh …".into()),
                1_200_000_000,
            ),
            (
                mk("dim", "DimAgent", "\u{E945}", "✨"),
                ActivityLevel::Idle,
                277 << 20,
                None,
                2_070_000,
            ),
            (
                mk("workbuddy", "WorkBuddy", "\u{E756}", "💼"),
                ActivityLevel::Idle,
                322 << 20,
                None,
                2_760_000,
            ),
            (
                mk("workbuddyai", "WorkBuddy AI", "\u{E774}", "🌐"),
                ActivityLevel::Idle,
                362 << 20,
                None,
                1_860_000,
            ),
            (
                mk("codex", "ChatGPT / Codex", "\u{E99A}", "🤖"),
                ActivityLevel::Idle,
                131 << 20,
                None,
                0,
            ),
        ];
        self.snapshots = demo
            .into_iter()
            .map(|(p, level, mem, action, t24)| AgentSnapshot {
                id: p.id.clone(),
                name: p.name.clone(),
                glyph: p.glyph.clone(),
                emoji: p.emoji.clone(),
                level,
                level_label: level.label().to_string(),
                observability: observability::evaluate(&Evidence {
                    level,
                    process_running: true,
                    installed: Some(true),
                    provenance: None,
                    probe_health: None,
                    probe_health_fresh: false,
                    has_local_detail_source: true,
                    active_sessions: 0,
                    has_token_usage: t24 > 0,
                }),
                // 演示数据里的进程在跑，但引擎没为它们采过样：观测窗口凑不够 ⇒ 卡死是「没测」
                is_hung: None,
                health: health::Report::not_running(),
                process_running: true,
                installed: Some(true),
                work_stats: crate::duration::Stats::empty(),
                provenance: None,
                provenance_suffix: String::new(),
                cpu_percent: Some(if level == ActivityLevel::Working {
                    34.0
                } else {
                    1.2
                }),
                memory_bytes: mem,
                memory_text: memory_text(mem),
                last_activity_text: if level == ActivityLevel::Working {
                    "刚刚".into()
                } else {
                    "—".into()
                },
                token_usage: (t24 > 0).then(|| TokenUsage {
                    tokens24h: t24,
                    tokens_total: t24 * 23,
                    cost24h: t24 as f64 / 14_000_000.0,
                    cost_total: t24 as f64 / 14_000.0,
                    cost_estimated: true,
                }),
                pid: Some(0),
                current_action: action,
                subagent_count: 0,
                session_probe_health: None,
                background_tasks: vec![],
                subagents: vec![],
                token_breakdown: None,
            })
            .map(|mut snapshot| {
                snapshot.health = health::evaluate(&snapshot);
                snapshot
            })
            .collect();
        self.grand_total = TokenUsage {
            tokens24h: 12_080_000,
            tokens_total: 280_000_000,
            cost24h: 0.84,
            cost_total: 19.37,
            cost_estimated: true,
        };
        if self.latest_event.is_none() {
            self.latest_event = Some(AgentTaskEvent {
                id: "demo-attention".into(),
                agent_id: "claude".into(),
                agent_name: "dim".into(),
                event_type: "attention".into(),
                timestamp: now_ms(),
                message: Some("需要确认: 是否允许执行 swift test".into()),
                detail: Some(
                    "会话请求执行 shell 命令 swift test，等待用户批准。可在岛内直达终端或忽略。"
                        .into(),
                ),
                duration: 0.0,
                externally_delivered: false,
            });
        }
    }

    pub fn state(&self) -> EngineState {
        EngineState {
            session_navigation: self.session_navigation.clone(),
            event_navigation: self.event_navigation.clone(),
            snapshots: self.snapshots.clone(),
            latest_event: self.latest_event.clone(),
            pending_events: self.pending_count(),
            recent_outbound: self.notifier.recent_view(),
            grand_total: self.grand_total.clone(),
            dock_edge: DockEdge::parse(&self.settings.dock_edge),
            shell_mode: self.settings.shell_mode.clone(),
            sidebar_edge: self.settings.sidebar_edge.clone(),
            sidebar_width: self.settings.sidebar_width,
            appearance: self.settings.appearance.clone(),
            any_working: self
                .snapshots
                .iter()
                .any(|s| s.level == ActivityLevel::Working),
            has_attention: self
                .snapshots
                .iter()
                .any(|s| s.level == ActivityLevel::Attention),
            demo: self.demo_mode,
        }
    }
}

#[cfg(test)]
#[path = "engine/state_tests.rs"]
mod state_tests;
fn demo_report() -> TokenReport {
    let mut hourly = Vec::new();
    let now = now_ms();
    for i in (0..24 * 30).rev() {
        let mut ts = now - i as i64 * 3_600_000;
        ts -= ts % 3_600_000;
        let v = ((ts as f64 / 3_600_000.0 / 5.0).sin() * 0.5 + 0.5).max(0.05);
        let mut tokens = (400_000.0 * v) as i64;
        if (ts / 1_000_000) % 3 == 0 {
            tokens = 0;
        }
        hourly.push((ts, tokens));
    }
    let peak_idx = hourly.len() - 3;
    hourly[peak_idx].1 = 2_830_000;
    let models = vec![
        ModelUsage {
            model: "codex-5".into(),
            tokens: 5_340_000,
            cost: 6.10,
            cost_estimated: true,
        },
        ModelUsage {
            model: "claude-sonnet-4-5".into(),
            tokens: 2_760_000,
            cost: 3.22,
            cost_estimated: true,
        },
        ModelUsage {
            model: "claude-opus-4".into(),
            tokens: 2_070_000,
            cost: 9.41,
            cost_estimated: true,
        },
        ModelUsage {
            model: "gpt-4o".into(),
            tokens: 1_860_000,
            cost: 2.05,
            cost_estimated: true,
        },
        ModelUsage {
            model: "claude-haiku-4".into(),
            tokens: 489_000,
            cost: 0.31,
            cost_estimated: true,
        },
    ];
    TokenReport {
        context24h: crate::usage_context::Report::unknown(12_080_000),
        usage: TokenUsage {
            tokens24h: 12_080_000,
            tokens_total: 280_000_000,
            cost24h: 0.84,
            cost_total: 19.37,
            cost_estimated: true,
        },
        models24h: models.clone(),
        models_total: models,
        hourly30d: hourly,
    }
}

/// token 暴涨告警的两条规则：档案专属下限 + 连续多档确认。
///
/// 这两条 Swift 侧都有、Rust 侧此前都没有，而它们各自防一种误报：
/// · **下限**（WorkBuddy 1M）：日常 3-5 专家并行的高消耗不该红。
/// · **连续三档**：长任务结束时一次性补写账本，那一拍的速率会虚高到爆表。
#[cfg(test)]
mod token_spike_rules {
    use super::*;

    fn profile_with_floor(floor: Option<i64>) -> AgentProfile {
        let mut p = crate::registry::builtin()
            .into_iter()
            .find(|p| p.id == "workbuddy")
            .expect("注册表里应当有 workbuddy");
        p.token_alert_floor = floor;
        p
    }

    /// 实际阈值 = **max**(档案下限, 全局设置)。
    ///
    /// 方向不能反：档案下限只能**抬高**门槛，不能把用户自设的高档位拉低——
    /// 否则「我把阈值调到 100 万防误报」会被档案的 50 万下限反过来架空。
    #[test]
    fn the_effective_threshold_is_the_max_of_profile_floor_and_global_setting() {
        let cases: [(i64, Option<i64>, i64); 4] = [
            // (全局设置, 档案下限, 实际阈值)
            (200_000, Some(1_000_000), 1_000_000), // 档案下限更高 ⇒ 取它
            (200_000, None, 200_000),              // 无下限 ⇒ 取全局
            (2_000_000, Some(1_000_000), 2_000_000), // 全局更高 ⇒ 取全局
            (200_000, Some(100_000), 200_000),     // 下限更低 ⇒ 取全局
        ];
        for (global, floor, want) in cases {
            let got = floor.unwrap_or(0).max(global);
            assert_eq!(got, want, "global={global} floor={floor:?}");
        }
    }

    /// 注册表里**只有 WorkBuddy 两家**该有下限。
    ///
    /// 给别的档案加下限等于替用户决定「这个 Agent 不值得告警」——
    /// 那是个不该由我们做的判断。
    #[test]
    fn only_the_multi_agent_architectures_carry_a_token_floor() {
        let with_floor: Vec<String> = crate::registry::builtin()
            .into_iter()
            .filter(|p| p.token_alert_floor.is_some())
            .map(|p| p.id)
            .collect();
        assert_eq!(
            with_floor,
            vec!["workbuddy".to_string(), "workbuddy-ai".to_string()],
            "带专家团保护下限的应当只有 WorkBuddy 两家"
        );
        assert_eq!(profile_with_floor(None).token_alert_floor, None);
    }

    /// 连续确认的档数与 Swift 同值。改成 1 的话，一次性账本补写就会误报。
    #[test]
    fn the_spike_needs_three_consecutive_samples_and_not_one() {
        assert_eq!(TOKEN_SPIKE_CONFIRMATIONS, 3);
        assert!(
            TOKEN_SPIKE_CONFIRMATIONS > 1,
            "单档就告警 ⇒ 长任务结束时一次性落盘会直接误报成消耗突增"
        );
    }
}

/// **真的走一遍引擎的库路分派**，而不是在别处把条件复述一遍。
///
/// 为什么必须有这条：`registry::session_coverage_sentinel` 里的
/// `covered_by_rust` 是**照着引擎的 `match database.schema` 重写的一份**。
/// 两处编码同一个条件，**可以一起漂**——把引擎的路由改错了，守护照样绿。
///
/// 这条用例直接把**真 DDL 的库**喂给 `probe_cached_multi`，由引擎自己选函数。
/// 改了 `match` 的某一支，这里精确变红。
#[cfg(test)]
mod database_dispatch_tests {
    use super::ActivityEngine;
    use crate::models::{SessionDialect as D, SessionSchema as S};
    use std::sync::mpsc::channel;

    /// 抄自本机真库 `~/.dimcode/v2/dimcode.sqlite`（只抄结构，不含任何数据）。
    const DIM_DDL: &str = "CREATE TABLE messages (
  messageId TEXT PRIMARY KEY, sessionId TEXT NOT NULL, role TEXT NOT NULL,
  parts TEXT NOT NULL, attachments TEXT, toolMetadata TEXT, metadata TEXT,
  orderKey TEXT NOT NULL, createdAt TEXT NOT NULL, updatedAt TEXT NOT NULL)";
    /// 同表结构的 fork（小米 MiMo Code）用的是**老表名**，表名必须现查。
    const CLINE_FORK_DDL: &str = "CREATE TABLE session (
  id TEXT PRIMARY KEY, project_id TEXT, parent_id TEXT, slug TEXT, directory TEXT,
  title TEXT, version INTEGER, time_created INTEGER, time_updated INTEGER);\
  CREATE TABLE message (
  id TEXT PRIMARY KEY, session_id TEXT NOT NULL, agent_id TEXT,
  time_created INTEGER, time_updated INTEGER, data TEXT)";

    fn engine() -> ActivityEngine {
        ActivityEngine::new(crate::settings::Settings::default(), channel().1)
    }

    /// `DimTasks` 必须走到 `probe_dim`：assistant 末个 part 已封口 ⇒ 完成。
    #[test]
    fn dim_tasks_reaches_the_dim_probe_through_the_engine() {
        let dir = crate::testutil::Sandbox::new("engine-dispatch-dim");
        let path = dir.path().join("dimcode.sqlite");
        {
            let conn = rusqlite::Connection::open(&path).unwrap();
            conn.execute_batch(DIM_DDL).unwrap();
            conn.execute(
                "INSERT INTO messages (messageId, sessionId, role, parts, orderKey, createdAt, updatedAt)
                 VALUES ('m1','s1','assistant',
                         '[{\"type\":\"text\",\"text\":\"好了\",\"endTime\":1730000000000}]',
                         '1','2026-09-29T00:00:00Z','2026-09-29T00:00:00Z')",
                [],
            )
            .unwrap();
        }
        let db = crate::models::SessionDatabase {
            path: path.to_string_lossy().to_string(),
            schema: S::DimTasks,
            status_sql: None,
        };
        let (probe, _) =
            engine().probe_cached_multi("dim", D::GenericTail, &[], Some(&db), 1_000_000);
        assert!(
            matches!(probe.signal, Some(crate::session::Signal::Completed(_))),
            "DimTasks 库必须由引擎路由到 probe_dim 并报完成，实际 {:?}",
            probe.signal
        );
    }

    /// `OpenCode` 必须走到 `probe_opencode`：走的是**老表名**那一支。
    #[test]
    fn open_code_reaches_its_probe_through_the_engine() {
        let dir = crate::testutil::Sandbox::new("engine-dispatch-opencode");
        let path = dir.path().join("mimocode.db");
        {
            let conn = rusqlite::Connection::open(&path).unwrap();
            conn.execute_batch(CLINE_FORK_DDL).unwrap();
            let now_ms = 1_757_000_000_000i64;
            // `session` 表是必需的：查询靠它定位「最新一场会话」，
            // 缺了它这一族就被判为「不是这一族的库」而拒绝——**这是对的**，
            // 所以夹具必须两张表都建。
            conn.execute(
                "INSERT INTO session (id, slug, time_created, time_updated) VALUES ('s1','s1',?1,?1)",
                rusqlite::params![now_ms],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO message (id, session_id, agent_id, time_created, time_updated, data)
                 VALUES ('msg_1','s1','agent',?1,?1,?2)",
                rusqlite::params![
                    now_ms,
                    format!(r#"{{"role":"assistant","time":{{"created":{now_ms},"completed":{now_ms}}}}}"#)
                ],
            )
            .unwrap();
        }
        let db = crate::models::SessionDatabase {
            path: path.to_string_lossy().to_string(),
            schema: S::OpenCode,
            status_sql: None,
        };
        let (probe, _) = engine().probe_cached_multi(
            "mimocode",
            D::GenericTail,
            &[],
            Some(&db),
            1_757_000_000_000,
        );
        assert!(
            matches!(probe.signal, Some(crate::session::Signal::Completed(_))),
            "OpenCode 库必须由引擎路由到 probe_opencode 并报完成，实际 {:?}",
            probe.signal
        );
    }

    /// **反向**：`DimTasks` 的库**不能**被当成状态索引去查。
    ///
    /// 做法不是去看内部（分派不返回失败原因），而是**造一个两边结果不同的库**：
    /// 没有 `messages` 表（`probe_dim` 必然无信号），却有一张 `sessions` 表
    /// 配一条**能跑通**的 `status_sql`（`probe_status_index` 必然有信号）。
    /// 于是「无信号」本身就证明了走的是 dim 那一支——
    /// 路由错了，这个库会冒出信号，而且**界面上看不出异样**。
    #[test]
    fn a_dim_library_is_not_read_as_a_status_index() {
        let dir = crate::testutil::Sandbox::new("engine-dispatch-mixup");
        let path = dir.path().join("mixup.sqlite");
        {
            let conn = rusqlite::Connection::open(&path).unwrap();
            // 故意**不建** `messages` 表
            conn.execute_batch(
                "CREATE TABLE sessions (id TEXT PRIMARY KEY, state TEXT);                 INSERT INTO sessions (id, state) VALUES ('s1', 'running');",
            )
            .unwrap();
        }
        let db = crate::models::SessionDatabase {
            path: path.to_string_lossy().to_string(),
            schema: S::DimTasks,
            status_sql: Some("SELECT id, state FROM sessions LIMIT 1".into()),
        };
        let (probe, _) =
            engine().probe_cached_multi("dim", D::GenericTail, &[], Some(&db), 1_000_000);
        assert!(
            probe.signal.is_none(),
            "DimTasks 的库被当成状态索引查了 —— 查出来的东西根本不是这一族要的：{:?}",
            probe.signal
        );
    }
}

#[cfg(test)]
mod optimization_regressions {
    use super::*;
    #[test]
    fn missing_usage_location_is_none_but_an_empty_readable_source_is_zero() {
        let sandbox = crate::testutil::Sandbox::new("missing-usage-source");
        let mut profile = crate::registry::builtin()
            .into_iter()
            .find(|profile| profile.id == "codex")
            .unwrap();
        profile.token_roots = vec![sandbox
            .path()
            .join("missing")
            .to_string_lossy()
            .into_owned()];
        profile.session_database = None;
        let (_, rx) = std::sync::mpsc::channel();
        let mut engine = ActivityEngine::new(Settings::default(), rx);
        assert!(
            engine.token_usage_cached(&profile).is_none(),
            "missing source is not measured zero"
        );
        std::fs::create_dir_all(&profile.token_roots[0]).unwrap();
        assert_eq!(engine.token_usage_cached(&profile).unwrap().tokens24h, 0);
    }

    #[test]
    fn unreadable_candidate_health_reaches_the_engine_and_recovers() {
        let sandbox = crate::testutil::Sandbox::new("candidate-health");
        let file = sandbox.path().join("session.jsonl");
        std::fs::write(&file, [0xff]).unwrap();
        let path = file.to_string_lossy().into_owned();
        let (_, rx) = std::sync::mpsc::channel();
        let mut engine = ActivityEngine::new(Settings::default(), rx);
        let (probe, _) = engine.probe_cached_multi(
            "qoder",
            SessionDialect::QoderTranscript,
            &[path.clone()],
            None,
            now_ms(),
        );
        assert!(
            probe.health.is_some(),
            "candidate failures must not become silent no-signal results"
        );
        std::fs::write(&file, "{}\n").unwrap();
        let (probe, _) = engine.probe_cached_multi(
            "qoder",
            SessionDialect::QoderTranscript,
            &[path],
            None,
            now_ms(),
        );
        assert!(
            probe.health.is_none(),
            "a repaired source must recover on the next sample"
        );
    }
}
#[cfg(test)]
mod database_task_source_tests {
    use super::ActivityEngine;
    use crate::{
        models::{SessionDatabase, SessionDialect, SessionSchema},
        task_sources, tasks,
    };
    const NOW: i64 = 1_757_000_000_000;
    fn profile(agent: &str, db: &SessionDatabase) -> crate::models::AgentProfile {
        let mut p = crate::registry::builtin()
            .into_iter()
            .find(|p| p.id == agent)
            .unwrap();
        p.session_database = Some(db.clone());
        p
    }
    fn choice(db: &SessionDatabase, agent: &str) -> task_sources::Choice {
        let mut engine = ActivityEngine::new(Default::default(), std::sync::mpsc::channel().1);
        let (probe, context) =
            engine.probe_cached_multi(agent, SessionDialect::GenericTail, &[], Some(db), NOW);
        assert!(
            probe.health.is_none(),
            "database probe must really query the fixture"
        );
        let encoded = serde_json::to_string(&context).unwrap();
        assert!(!encoded.contains(&db.path));
        assert!(!encoded.contains("fixture-session"));
        assert!(!encoded.contains("sourceKeys"));
        if matches!(probe.signal, Some(crate::session::Signal::Active(..))) {
            assert_eq!(task_sources::source_age(&context, NOW), Some(0.0));
            assert!(task_sources::active_is_eligible(
                probe.signal.as_ref(),
                true,
                task_sources::source_age(&context, NOW),
                60.0
            ));
        }
        let choice =
            task_sources::from_context(&profile(agent, db), &context, probe.signal.as_ref())
                .unwrap();
        let output = serde_json::to_string(&choice).unwrap();
        assert!(!output.contains(&db.path));
        assert!(!output.contains("fixture-session"));
        assert!(choice.source.thread_id.is_none());
        assert!(!choice
            .target
            .as_ref()
            .is_some_and(|target| target.exact_session));
        choice
    }
    #[test]
    fn database_sessions_are_distinct_stable_and_flow_into_the_task_store() {
        for (case, agent, schema) in [
            ("dim", "dim", SessionSchema::DimTasks),
            ("old-open", "opencode", SessionSchema::OpenCode),
            ("new-open", "mimocode", SessionSchema::OpenCode),
            ("minimax", "minimaxcode", SessionSchema::MiniMaxRuntime),
            ("index", "workbuddy", SessionSchema::StatusIndex),
        ] {
            let sandbox = crate::testutil::Sandbox::new(case);
            let path = sandbox.path().join("source.sqlite");
            let conn = rusqlite::Connection::open(&path).unwrap();
            let db = SessionDatabase {
                path: path.to_string_lossy().into(),
                schema,
                status_sql: (schema == SessionSchema::StatusIndex).then(|| {
                    "SELECT id,status,updated_at FROM sessions ORDER BY updated_at DESC LIMIT 1"
                        .into()
                }),
            };
            match schema {
                SessionSchema::DimTasks => conn.execute_batch("CREATE TABLE messages(messageId TEXT PRIMARY KEY,sessionId TEXT NOT NULL,role TEXT NOT NULL,parts TEXT NOT NULL,orderKey TEXT,createdAt TEXT,updatedAt TEXT); INSERT INTO messages VALUES('m1','fixture-session-a','assistant','[{\"type\":\"text\",\"endTime\":1}]','1','','');").unwrap(),
                SessionSchema::OpenCode => {
                    let (sessions,messages) = if case == "old-open" {("session","message")} else {("session_v2","session_message")};
                    conn.execute_batch(&format!("CREATE TABLE {sessions}(id TEXT PRIMARY KEY,time_updated INTEGER);CREATE TABLE {messages}(id TEXT PRIMARY KEY,session_id TEXT,data TEXT);")).unwrap();
                    conn.execute(&format!("INSERT INTO {sessions} VALUES('fixture-session-a',?1)"),[NOW]).unwrap();
                    conn.execute(&format!("INSERT INTO {messages} VALUES('m1','fixture-session-a',?1)"),[format!(r#"{{"role":"user","time":{{"created":{NOW}}}}}"#)]).unwrap();
                },
                SessionSchema::MiniMaxRuntime => {
                    conn.execute_batch("CREATE TABLE local_runtime_sessions(session_id TEXT,status TEXT,updated_at_ms INTEGER,archived INTEGER);CREATE TABLE local_runtime_token_usage(session_id TEXT,ts INTEGER);").unwrap();
                    conn.execute("INSERT INTO local_runtime_sessions VALUES('fixture-session-a','started',?1,0)",[NOW]).unwrap();
                },
                SessionSchema::StatusIndex => {
                    conn.execute_batch("CREATE TABLE sessions(id TEXT,status TEXT,updated_at INTEGER);").unwrap();
                    conn.execute("INSERT INTO sessions VALUES('fixture-session-a','awaiting_approval',?1)",[NOW]).unwrap();
                },
            }
            let a = choice(&db, agent);
            let repeat = choice(&db, agent);
            assert_eq!(a.source, repeat.source);
            assert_eq!(a.observation.fingerprint, repeat.observation.fingerprint);
            let store = tasks::Store {
                path: sandbox.path().join("tasks.json"),
            };
            let data = store.create("第一个会话", None, 0, NOW).unwrap();
            let first = data.tasks[0].id.clone();
            let data = store
                .link(&first, a.source.clone(), data.revision, NOW)
                .unwrap();
            let data = store.sync(&[a.observation.clone()], NOW).unwrap().unwrap();
            let first_run = data.tasks[0].current_run_id.clone().unwrap();
            assert!(store.sync(&[repeat.observation], NOW).unwrap().is_none());
            match schema {
                SessionSchema::DimTasks => {
                    conn.execute(
                        "UPDATE messages SET sessionId='fixture-session-b',messageId='m2'",
                        [],
                    )
                    .unwrap();
                }
                SessionSchema::OpenCode => {
                    let (sessions, messages) = if case == "old-open" {
                        ("session", "message")
                    } else {
                        ("session_v2", "session_message")
                    };
                    conn.execute(&format!("UPDATE {sessions} SET id='fixture-session-b'"), [])
                        .unwrap();
                    conn.execute(
                        &format!("UPDATE {messages} SET id='m2',session_id='fixture-session-b'"),
                        [],
                    )
                    .unwrap();
                }
                SessionSchema::MiniMaxRuntime => {
                    conn.execute(
                        "UPDATE local_runtime_sessions SET session_id='fixture-session-b'",
                        [],
                    )
                    .unwrap();
                }
                SessionSchema::StatusIndex => {
                    conn.execute("UPDATE sessions SET id='fixture-session-b'", [])
                        .unwrap();
                }
            }
            let b = choice(&db, agent);
            assert_ne!(a.source, b.source, "{case}: a DB is not a session");
            assert!(
                store.sync(&[b.observation.clone()], NOW).unwrap().is_none(),
                "unbound new session must not update the old task"
            );
            let data = store
                .create("第二个会话", None, data.revision, NOW)
                .unwrap();
            let second = data.tasks[1].id.clone();
            let data = store
                .link(&second, b.source.clone(), data.revision, NOW)
                .unwrap();
            let data = store.sync(&[b.observation], NOW).unwrap().unwrap();
            assert_eq!(data.runs.len(), 2);
            assert_eq!(
                data.navigation_source(&first, Some(&first_run), None, data.revision)
                    .unwrap(),
                &a.source
            );
            assert!(data
                .runs
                .iter()
                .all(|run| run.status != tasks::RunStatus::Accepted));
            data.validate().unwrap();
        }
    }
    #[test]
    fn composite_workspace_keys_and_event_revisions_do_not_collapse() {
        let sandbox = crate::testutil::Sandbox::new("zcode-source-composite");
        let path = sandbox.path().join("index.sqlite");
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch("CREATE TABLE tasks(workspace_key TEXT,task_id TEXT,task_status TEXT,updated_at INTEGER,deleted INTEGER,archived INTEGER,PRIMARY KEY(workspace_key,task_id));").unwrap();
        conn.execute(
            "INSERT INTO tasks VALUES('workspace-a','fixture-session','awaiting_approval',?1,0,0)",
            [NOW],
        )
        .unwrap();
        let mut db = crate::registry::builtin()
            .into_iter()
            .find(|p| p.id == "zcode")
            .unwrap()
            .session_database
            .unwrap();
        db.path = path.to_string_lossy().into();
        let a = choice(&db, "zcode");
        conn.execute("UPDATE tasks SET updated_at=?1", [NOW + 1])
            .unwrap();
        let updated = choice(&db, "zcode");
        assert_eq!(a.source, updated.source);
        assert_ne!(a.observation.fingerprint, updated.observation.fingerprint);
        assert_eq!(
            updated.observation.fingerprint,
            choice(&db, "zcode").observation.fingerprint
        );
        conn.execute("UPDATE tasks SET workspace_key='workspace-b'", [])
            .unwrap();
        let b = choice(&db, "zcode");
        assert_ne!(a.source, b.source);
        conn.execute("UPDATE tasks SET workspace_key=''", [])
            .unwrap();
        let mut engine = ActivityEngine::new(Default::default(), std::sync::mpsc::channel().1);
        let (probe, context) =
            engine.probe_cached_multi("zcode", SessionDialect::GenericTail, &[], Some(&db), NOW);
        assert!(
            probe.signal.is_some(),
            "missing identity preserves status evidence"
        );
        assert!(context.source_keys.is_none());
        assert!(task_sources::from_context(
            &profile("zcode", &db),
            &context,
            probe.signal.as_ref()
        )
        .is_none());
    }
}

#[cfg(test)]
#[test]
#[ignore = "只读本机第三方会话库，不输出正文、路径或主键"]
fn real_database_task_source_identity() {
    let mut engine = ActivityEngine::new(Default::default(), std::sync::mpsc::channel().1);
    let now = crate::tokens::now_ms();
    for profile in crate::registry::builtin() {
        let Some(db) = profile
            .session_database
            .as_ref()
            .filter(|db| std::path::Path::new(&db.path).is_file())
        else {
            continue;
        };
        let (probe, context) =
            engine.probe_cached_multi(&profile.id, profile.session_dialect, &[], Some(db), now);
        let choice = crate::task_sources::from_context(&profile, &context, probe.signal.as_ref());
        println!(
            "agent={} semantic={} identity={} read_failure={}",
            profile.id,
            probe.signal.is_some(),
            choice.is_some(),
            probe.health.is_some()
        );
        if context.source_keys.is_some() {
            assert!(choice.is_some());
        }
        if let Some(choice) = choice {
            assert!(choice.source.thread_id.is_none());
            assert_eq!(choice.source.session_id.len(), 64);
        }
    }
}

#[cfg(test)]
mod resource_diagnostic_tests {
    use super::*;
    fn engine() -> ActivityEngine {
        let (_, rx) = std::sync::mpsc::channel();
        ActivityEngine::new(Settings::default(), rx)
    }
    fn event() -> AgentTaskEvent {
        AgentTaskEvent {
            id: "fixture-event".into(),
            agent_id: "fixture-agent".into(),
            agent_name: "Fixture".into(),
            event_type: "attention".into(),
            message: Some("fixture-private-body".into()),
            detail: None,
            duration: 0.0,
            timestamp: now_ms(),
            externally_delivered: false,
        }
    }
    #[test]
    fn diagnostic_observer_skips_outbound_but_keeps_local_events() {
        let mut observer = engine();
        observer.settings.remote_policy.master_enabled = true;
        observer.enable_resource_diagnostics();
        observer.push_event(event());
        observer.notify_outbound_with_presence(&event(), Default::default());
        assert!(observer.latest_event.is_some());
        assert!(observer.notifier.recent().is_empty());
        // Ordinary engine still passes events to policy/ledger: observer mode is not a global mute.
        let mut normal = engine();
        normal.settings.remote_policy.master_enabled = false;
        normal.notify_outbound_with_presence(&event(), Default::default());
        let until = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while normal.notifier.recent().is_empty() && std::time::Instant::now() < until {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(normal.notifier.recent().len(), 1);
        assert!(matches!(
            normal.notifier.recent()[0].outcome,
            notifier::Outcome::Suppressed { .. }
        ));
        assert!(observer.notifier.recent().is_empty());
    }
    #[test]
    fn diagnostics_describe_fixture_caches_without_identifiers_or_text_and_reset_for_demo() {
        let sandbox = crate::testutil::Sandbox::new("resource-diag");
        let path = sandbox.path().join("fixture-private-file.jsonl");
        std::fs::write(&path,r#"{"type":"assistant","uuid":"fixture-private-id","message":{"model":"fixture-private-model","content":"fixture-private-body","usage":{"input_tokens":10,"output_tokens":2}}}
"#).unwrap();
        let mut e = engine();
        let mut profile = crate::registry::builtin()
            .into_iter()
            .find(|p| p.id == "claude")
            .unwrap();
        let root = sandbox.path().to_string_lossy().into_owned();
        profile.session_dirs = vec![root.clone()];
        profile.token_roots = vec![root.clone()];
        profile.session_database = None;
        e.profiles = vec![profile];
        e.enable_resource_diagnostics();
        e.tick();
        let first = e.resource_diagnostics().unwrap();
        assert_eq!(first.profiles, 1);
        assert_eq!(first.candidates, 1);
        assert_eq!(first.file_cache.roots, 1);
        assert_eq!(first.file_cache.files, 1);
        assert_eq!(first.token_cache.files, 1);
        assert_eq!(first.token_cache.entries, 1);
        assert!(first.token_cache.entry_capacity >= 1);
        assert_eq!(e.grand_total.tokens_total, 12);
        let encoded = serde_json::to_string(first).unwrap();
        assert!(!encoded.contains("fixture-private"));
        assert!(!encoded.contains(&root));
        assert!(!encoded.contains("claude"));
        assert!(
            first.total_us
                >= first.process_us
                    + first.installed_us
                    + first.files_us
                    + first.sessions_us
                    + first.tokens_us
        );
        e.tick();
        assert_eq!(e.resource_diagnostics().unwrap().token_cache.entries, 1);
        assert_eq!(e.grand_total.tokens_total, 12);
        e.demo_mode = true;
        e.tick();
        let demo = e.resource_diagnostics().unwrap();
        assert!(demo.demo);
        assert_eq!(demo.token_cache.files, 0);
        assert_eq!(demo.candidates, 0);
        assert!(engine().resource_diagnostics().is_none());
    }
}
