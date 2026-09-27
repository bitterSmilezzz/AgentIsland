use crate::filemon::{time_ago_text, FileMonitor};
use crate::installed::{self, InstalledApps};
use crate::models::*;
use crate::observability::{self, Evidence};
use crate::health;
use crate::notifier;
use crate::remote;
use crate::render;
use crate::resilience;
use crate::procmon::{memory_text, ProcessMonitor};
use crate::session::{self, Signal};
use crate::settings::Settings;
use crate::tokens::now_ms;
use crate::tokens::TokenUsageMonitor;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::mpsc::Receiver;
use std::time::{Duration, SystemTime};

/// 五态状态机引擎（与 macOS 端 ActivityEngine 同规则）：
/// · 未解决的确认/授权请求            → attention
/// · 本轮明确结束（有写入证据）        → completed
/// · 进程在 + 写入 60s 内 或 CPU≥阈值 → working
/// · 进程在但静默                      → idle
/// · 进程不在                          → offline（可见口径隐藏）
/// 连续多少档超阈值才发 token 暴涨告警。与 Swift `ActivityEngine.tokenSpikeConfirmations` 同值。
pub const TOKEN_SPIKE_CONFIRMATIONS: u32 = 3;

pub struct ActivityEngine {
    pub settings: Settings,
    pub demo_mode: bool,
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
    probe_cache: HashMap<String, (u64, SystemTime, Option<Signal>, usize)>,
    token_cache: HashMap<String, (i64, TokenReport)>,
    pub event_rx: Option<Receiver<AgentTaskEvent>>,

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
    pub snapshots: Vec<AgentSnapshot>,
}

impl ActivityEngine {
    pub fn new(settings: Settings, event_rx: Receiver<AgentTaskEvent>) -> Self {
        let profiles = crate::registry::builtin();
        let installed = InstalledApps::new(InstalledApps::cli_names_for(&profiles));
        ActivityEngine {
            settings,
            demo_mode: false,
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
            probe_cache: HashMap::new(),
            token_cache: HashMap::new(),
            event_rx: Some(event_rx),
            latest_event: None,
            pending_events: VecDeque::new(),
            grand_total: TokenUsage::default(),
            self_reports: crate::selfreport::Registry::new(),
            durations: crate::duration::TaskDurationTracker::new(),
            budget: crate::budget::BudgetTracker::new(),
            budget_status: crate::budget::BudgetStatus::Disabled,
            snapshots: vec![],
        }
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
        if self.demo_mode {
            self.apply_demo();
            return;
        }

        let cpu_threshold = self.settings.cpu_threshold;
        // 这两个此前是硬编码字面量，Swift 侧同名字段是可在设置页调的
        let working_window = self.settings.working_window;
        let min_working_hold = self.settings.min_working_hold;


        self.procmon.refresh();
        let now = now_ms();

        // 装机探测按 TTL 刷（默认 5 分钟）：`/Applications` 枚举 + 逐个读 Info.plist
        // 在应用多时要几十毫秒，不该每拍都做。未热时全部档案的 `installed` 是 `None`，
        // 也就是「未核实」——这正是我们要的默认：宁可先不给结论。
        self.installed.refresh_if_needed(installed::DEFAULT_MAX_AGE);

        // 自报到期：**只盖戳，不删记录**，且放在采样里而不是另开定时器——
        // 到期与否只取决于墙钟，而采样每拍本来就在读同一个 `now`（Swift 同一条理由）
        self.self_reports.sweep_expired(now);
        let mut list: Vec<AgentSnapshot> = Vec::new();
        let mut total24 = 0i64;
        let mut total_all = 0i64;
        let mut cost24 = 0f64;
        let mut cost_all = 0f64;
        // 汇总位也要带估算标记：只要有一个档案的成本是估的，汇总就不是记录值
        let mut cost_estimated = false;

        for profile in self.enabled_profiles() {
            let hits = self.procmon.match_profile(&profile);
            let process_running = !hits.is_empty();
            let pid = hits.iter().max_by_key(|h| h.memory).map(|h| h.pid);
            let memory: u64 = hits.iter().map(|h| h.memory).sum();
            // 多进程档案（Electron）取最大 CPU 分量作代表
            let cpu: Option<f64> = hits.iter().filter_map(|h| h.cpu).fold(None, |acc: Option<f64>, c| {
                Some(match acc {
                    Some(a) => a.max(c),
                    None => c,
                })
            });

            let file_result = self.filemon.probe(&profile);
            let candidates = self.filemon.probe_files(&profile);
            let probe = self.probe_cached_multi(&profile.id, &candidates);
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
            let report = self
                .self_reports
                .believable(&profile.id, now)
                .cloned();
            let has_session_signal = probe.signal.is_some();
            let (level, provenance) =
                crate::selfreport::resolve(level, has_session_signal, report.as_ref());

            let token_usage = self.token_usage_cached(&profile);
            let installed_state = self.installed.is_installed(&profile);
            let observability = observability::evaluate(Evidence {
                level,
                process_running,
                installed: installed_state,
                provenance,
                source_unreadable: process_running
                    && level == ActivityLevel::Idle
                    && observability::has_unreadable_source(&profile),
                has_local_detail_source: observability::has_local_detail_source(&profile),
                recent_session_write: observability::recent_write(
                    file_result.latest_write.as_ref(),
                    SystemTime::UNIX_EPOCH + Duration::from_millis(now.max(0) as u64),
                ),
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

            let work_stats = self.durations.stats(
                &profile.id,
                crate::duration::DEFAULT_WINDOW_MS,
                now,
            );
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
                        _ => report.as_ref().and_then(|r| r.ask.clone().or_else(|| r.detail.clone())),
                    }
                },
                provenance,
                // 后缀由 Rust 拼好（" · 自报" / " · 自报冲突" / 空）：界面各自拼一遍就会漂
                provenance_suffix: crate::selfreport::Provenance::badge_suffix(provenance),
                subagent_count: probe.subagent_count,
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
        self.observed_running_since.entry(key.clone()).or_insert(now);

        // 强语义：attention 优先（同一指纹只提醒一次）
        if let Some(Signal::Attention(fp, message)) = &probe.signal {
            let fp = fp.clone();
            let message = message.clone();
            if self.alerted_fingerprints.insert(fp.clone()) {
                self.push_event(AgentTaskEvent {
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
                    self.push_event(AgentTaskEvent {
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

    fn raise_cost_spike(&mut self, profile: &AgentProfile, pid: Option<u32>, message: &str, now: i64, once: &str) {
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
        self.push_event(AgentTaskEvent {
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
            self.pending_events.pop_front();
        }
        self.pending_events.push_back(event);
    }

    /// 每个事件过一次外发闸门并记账。
    ///
    /// `completed` 的 `seconds` 是「本次任务用时」：此刻 `work_started_at` 还没被清
    /// （`decide_level` 里先 `push_event` 再 `remove`），正好拿得到；其余类型按「刚刚」。
    fn notify_outbound(&mut self, event: &AgentTaskEvent) {
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
            // Rust 还没接 macOS 的在场信号层：按 fail-open 判成「人不在」
            remote::PresenceSignals::unavailable(),
            false);
    }

    /// 确认当前这条，推下一条（前端关掉横幅时调用）
    pub fn ack_latest_event(&mut self) {
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
            return;
        }
        let alerts = self.resilience.evaluate(snapshots, now);
        for alert in alerts {
            self.push_event(AgentTaskEvent {
                id: crate::webhook::webhook_uuid(),
                agent_id: alert.agent_id,
                agent_name: alert.agent_name,
                event_type: "attention".into(),
                timestamp: now,
                message: Some(alert.message),
                detail: Some(format!("kind={} elapsed_ms={}", alert.kind, alert.elapsed_ms)),
                duration: 0.0,
                externally_delivered: false,
            });
        }
    }

    /// 按候选顺序（新→旧）逐个探测，返回第一个有信号的；全无信号返回空探测。
    /// 最新的文件不一定是语义文件（如 CLI 日志比 rollout 更新）。
    fn probe_cached_multi(
        &mut self,
        profile_id: &str,
        paths: &[String],
    ) -> session::SessionProbe {
        for path in paths {
            if path.is_empty() {
                continue;
            }
            let meta = std::fs::metadata(path);
            let (len, mtime) = match meta {
                Ok(m) => (m.len(), m.modified().unwrap_or(SystemTime::UNIX_EPOCH)),
                Err(_) => continue,
            };
            if let Some((l, t, signal, sc)) = self.probe_cache.get(path) {
                if *l == len && *t == mtime {
                    if signal.is_some() {
                        return session::SessionProbe {
                            signal: signal.clone(),
                            subagent_count: *sc,
                        };
                    }
                    continue;
                }
            }
            let probe = session::probe(profile_id, path);
            let signal = probe.signal.clone();
            let sc = probe.subagent_count;
            if self.probe_cache.len() > 200 {
                self.probe_cache.clear();
            }
            self.probe_cache.insert(path.clone(), (len, mtime, signal.clone(), sc));
            if signal.is_some() {
                return probe;
            }
        }
        session::SessionProbe { signal: None, subagent_count: 0 }
    }

    fn token_usage_cached(&mut self, profile: &AgentProfile) -> Option<TokenUsage> {
        if profile.token_roots.is_empty() {
            return None;
        }
        if let Some((fetched, report)) = self.token_cache.get(&profile.id) {
            if now_ms() - fetched < 20_000 {
                return Some(report.usage.clone());
            }
        }
        let report = self.tokens.monitor(profile);
        let usage = report.usage.clone();
        self.token_cache.insert(profile.id.clone(), (now_ms(), report));
        Some(usage)
    }

    pub fn get_report(&mut self, agent_id: &str) -> Option<TokenReport> {
        if self.demo_mode {
            return Some(demo_report());
        }
        let profile = self.profiles.iter().find(|p| p.id == agent_id)?.clone();
        if profile.token_roots.is_empty() {
            return None;
        }
        if let Some((_, report)) = self.token_cache.get(agent_id) {
            return Some(report.clone());
        }
        let report = self.tokens.monitor(&profile);
        self.token_cache.insert(agent_id.to_string(), (now_ms(), report.clone()));
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
            session_database: None,
            category: "assistant".into(),
        };
        let demo: Vec<(AgentProfile, ActivityLevel, u64, Option<String>, i64)> = vec![
            (mk("claude", "Claude", "\u{E8BD}", "🧠"), ActivityLevel::Attention, 430 << 20, Some("需要确认: …ift test".into()), 12_080_000),
            (mk("antigravity", "Antigravity", "\u{E72C}", "⚛️"), ActivityLevel::Working, 310 << 20, Some("正在修改: IslandView.swift".into()), 1_600_000_000),
            (mk("qoder", "Qoder", "\u{E943}", "🖥️"), ActivityLevel::Working, 877 << 20, Some("运行: chmod +x .scr.te-shots/shoot4.sh …".into()), 1_200_000_000),
            (mk("dim", "DimAgent", "\u{E945}", "✨"), ActivityLevel::Idle, 277 << 20, None, 2_070_000),
            (mk("workbuddy", "WorkBuddy", "\u{E756}", "💼"), ActivityLevel::Idle, 322 << 20, None, 2_760_000),
            (mk("workbuddyai", "WorkBuddy AI", "\u{E774}", "🌐"), ActivityLevel::Idle, 362 << 20, None, 1_860_000),
            (mk("chatgpt", "ChatGPT", "\u{E99A}", "🤖"), ActivityLevel::Idle, 131 << 20, None, 0),
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
                observability: observability::evaluate(Evidence {
                    level,
                    process_running: true,
                    installed: Some(true),
                    provenance: None,
                    source_unreadable: false,
                    has_local_detail_source: true,
                    recent_session_write: true,
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
                cpu_percent: Some(if level == ActivityLevel::Working { 34.0 } else { 1.2 }),
                memory_bytes: mem,
                memory_text: memory_text(mem),
                last_activity_text: if level == ActivityLevel::Working { "刚刚".into() } else { "—".into() },
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
                detail: Some("会话请求执行 shell 命令 swift test，等待用户批准。可在岛内直达终端或忽略。".into()),
                duration: 0.0,
                externally_delivered: false,
            });
        }
    }

    pub fn state(&self) -> EngineState {
        EngineState {
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
            any_working: self.snapshots.iter().any(|s| s.level == ActivityLevel::Working),
            has_attention: self.snapshots.iter().any(|s| s.level == ActivityLevel::Attention),
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
        ModelUsage { model: "codex-5".into(), tokens: 5_340_000, cost: 6.10, cost_estimated: true },
        ModelUsage { model: "claude-sonnet-4-5".into(), tokens: 2_760_000, cost: 3.22, cost_estimated: true },
        ModelUsage { model: "claude-opus-4".into(), tokens: 2_070_000, cost: 9.41, cost_estimated: true },
        ModelUsage { model: "gpt-4o".into(), tokens: 1_860_000, cost: 2.05, cost_estimated: true },
        ModelUsage { model: "claude-haiku-4".into(), tokens: 489_000, cost: 0.31, cost_estimated: true },
    ];
    TokenReport {
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
            (200_000, Some(100_000), 200_000),      // 下限更低 ⇒ 取全局
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
