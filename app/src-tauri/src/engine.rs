use crate::filemon::{time_ago_text, FileMonitor};
use crate::models::*;
use crate::procmon::{memory_text, ProcessMonitor};
use crate::session::{self, Signal};
use crate::settings::Settings;
use crate::tokens::now_ms;
use crate::tokens::TokenUsageMonitor;
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::Receiver;
use std::time::{SystemTime};

/// 五态状态机引擎（与 macOS 端 ActivityEngine 同规则）：
/// · 未解决的确认/授权请求            → attention
/// · 本轮明确结束（有写入证据）        → completed
/// · 进程在 + 写入 60s 内 或 CPU≥阈值 → working
/// · 进程在但静默                      → idle
/// · 进程不在                          → offline（可见口径隐藏）
pub struct ActivityEngine {
    pub settings: Settings,
    pub demo_mode: bool,
    profiles: Vec<AgentProfile>,
    procmon: ProcessMonitor,
    filemon: FileMonitor,
    tokens: TokenUsageMonitor,
    phase_since: HashMap<String, i64>,
    work_started_at: HashMap<String, i64>,
    alerted_fingerprints: HashSet<String>,
    last_completed_fp: HashMap<String, String>,
    high_cpu_since: HashMap<String, i64>,
    last_cost_spike: HashMap<String, i64>,
    token_rate: HashMap<String, (i64, i64)>,
    probe_cache: HashMap<String, (u64, SystemTime, Option<Signal>, usize)>,
    token_cache: HashMap<String, (i64, TokenReport)>,
    pub event_rx: Option<Receiver<AgentTaskEvent>>,

    pub latest_event: Option<AgentTaskEvent>,
    pub grand_total: TokenUsage,
    pub snapshots: Vec<AgentSnapshot>,
}

impl ActivityEngine {
    pub fn new(settings: Settings, event_rx: Receiver<AgentTaskEvent>) -> Self {
        ActivityEngine {
            settings,
            demo_mode: false,
            profiles: crate::registry::builtin(),
            procmon: ProcessMonitor::new(),
            filemon: FileMonitor::new(),
            tokens: TokenUsageMonitor::new(),
            phase_since: HashMap::new(),
            work_started_at: HashMap::new(),
            alerted_fingerprints: HashSet::new(),
            last_completed_fp: HashMap::new(),
            high_cpu_since: HashMap::new(),
            last_cost_spike: HashMap::new(),
            token_rate: HashMap::new(),
            probe_cache: HashMap::new(),
            token_cache: HashMap::new(),
            event_rx: Some(event_rx),
            latest_event: None,
            grand_total: TokenUsage::default(),
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
        let working_window = 60.0;
        let min_working_hold = 10.0;
        let active_window_secs = 600.0;

        self.procmon.refresh();
        let now = now_ms();
        let mut list: Vec<AgentSnapshot> = Vec::new();
        let mut total24 = 0i64;
        let mut total_all = 0i64;
        let mut cost24 = 0f64;
        let mut cost_all = 0f64;

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

            let token_usage = self.token_usage_cached(&profile);
            if let Some(u) = &token_usage {
                total24 += u.tokens24h;
                total_all += u.tokens_total;
                cost24 += u.cost24h;
                cost_all += u.cost_total;
            }

            let last_ago = file_result
                .latest_write
                .and_then(|t| t.elapsed().ok())
                .map(|d| d.as_secs_f64());

            list.push(AgentSnapshot {
                id: profile.id.clone(),
                name: profile.name.clone(),
                glyph: profile.glyph.clone(),
                emoji: profile.emoji.clone(),
                level,
                level_label: level.label().to_string(),
                process_running,
                cpu_percent: cpu,
                memory_bytes: memory,
                memory_text: memory_text(memory),
                last_activity_text: last_ago
                    .map(time_ago_text)
                    .unwrap_or_else(|| "—".into()),
                token_usage,
                pid,
                current_action: match &probe.signal {
                    Some(Signal::Active(_, action)) => action.clone(),
                    Some(Signal::Attention(_, msg)) => Some(msg.clone()),
                    _ => None,
                },
                subagent_count: probe.subagent_count,
            });
        }

        list.sort_by(|a, b| {
            b.level
                .cmp(&a.level)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        self.snapshots = list;
        self.grand_total = TokenUsage {
            tokens24h: total24,
            tokens_total: total_all,
            cost24h: cost24,
            cost_total: cost_all,
        };
    }

    #[allow(clippy::too_many_arguments)]
    fn decide_level(
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
        if !process_running {
            self.phase_since.insert(key.clone(), now);
            return ActivityLevel::Offline;
        }

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
                    externally_delivered: false,
                });
                if self.alerted_fingerprints.len() > 800 {
                    self.alerted_fingerprints.clear();
                }
            }
            self.phase_since.insert(key.clone(), now);
            return ActivityLevel::Attention;
        }

        // 强语义：本轮明确结束（有写入证据才宣布完成）
        let write_evidence = file
            .latest_write
            .and_then(|t| t.elapsed().ok())
            .map(|d| d.as_secs_f64() < working_window)
            .unwrap_or(false);
        if let Some(Signal::Completed(fp)) = &probe.signal {
            if write_evidence {
                let fp = fp.clone();
                let changed = self
                    .phase_since
                    .get(&key)
                    .map(|s| (now - *s) / 1000)
                    .unwrap_or(0);
                let is_new = match self.last_completed_fp.get(&key) {
                    Some(prev) => *prev != fp,
                    None => true,
                };
                if is_new {
                    self.last_completed_fp.insert(key.clone(), fp.clone());
                    self.push_event(AgentTaskEvent {
                        id: fp,
                        agent_id: key.clone(),
                        agent_name: profile.name.clone(),
                        event_type: "completed".into(),
                        timestamp: now,
                        message: None,
                        detail: None,
                        externally_delivered: false,
                    });
                }
                self.phase_since.insert(key.clone(), now);
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
                .phase_since
                .get(&key)
                .map(|s| (now - *s) as f64 / 1000.0 < min_working_hold)
                .unwrap_or(false);

        let level = if in_flight || write_evidence || cpu_hot || holding {
            self.work_started_at.entry(key.clone()).or_insert(now);
            self.phase_since.insert(key.clone(), now);
            ActivityLevel::Working
        } else {
            self.work_started_at.remove(&key);
            self.phase_since.insert(key.clone(), now);
            ActivityLevel::Idle
        };

        // 熔断：CPU 连续 70% 以上达 5 分钟
        if let Some(c) = cpu {
            if c >= 70.0 {
                let since = *self.high_cpu_since.entry(key.clone()).or_insert(now);
                if now - since > 300_000 {
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
                    if rate > self.settings.token_alert_threshold as f64 {
                        self.raise_cost_spike(
                            profile,
                            pid,
                            &format!(
                                "Token 消耗突增（近 {:.0} 分钟约 {} tokens/分钟）",
                                elapsed_min,
                                compact(rate as i64)
                            ),
                            now,
                            "token-rate",
                        );
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
            externally_delivered: false,
        });
    }

    pub fn push_event(&mut self, event: AgentTaskEvent) {
        self.latest_event = Some(event);
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
        if let Some((_, report)) = self.token_cache.get(agent_id) {
            return Some(report.clone());
        }
        let profile = self.profiles.iter().find(|p| p.id == agent_id)?.clone();
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
            cmdline_hints: vec![],
            path_excludes: vec![],
            cpu_floor: None,
            session_dirs: vec![],
            token_roots: vec![],
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
                process_running: true,
                cpu_percent: Some(if level == ActivityLevel::Working { 34.0 } else { 1.2 }),
                memory_bytes: mem,
                memory_text: memory_text(mem),
                last_activity_text: if level == ActivityLevel::Working { "刚刚".into() } else { "—".into() },
                token_usage: (t24 > 0).then(|| TokenUsage {
                    tokens24h: t24,
                    tokens_total: t24 * 23,
                    cost24h: t24 as f64 / 14_000_000.0,
                    cost_total: t24 as f64 / 14_000.0,
                }),
                pid: Some(0),
                current_action: action,
                subagent_count: 0,
            })
            .collect();
        self.grand_total = TokenUsage {
            tokens24h: 12_080_000,
            tokens_total: 280_000_000,
            cost24h: 0.84,
            cost_total: 19.37,
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
                externally_delivered: false,
            });
        }
    }

    pub fn state(&self) -> EngineState {
        EngineState {
            snapshots: self.snapshots.clone(),
            latest_event: self.latest_event.clone(),
            grand_total: self.grand_total.clone(),
            dock_edge: DockEdge::parse(&self.settings.dock_edge),
            appearance: self.settings.appearance.clone(),
            any_working: self.snapshots.iter().any(|s| s.level == ActivityLevel::Working),
            has_attention: self.snapshots.iter().any(|s| s.level == ActivityLevel::Attention),
            demo: self.demo_mode,
        }
    }
}

pub fn compact(tokens: i64) -> String {
    let n = tokens as f64;
    if tokens < 0 {
        return "0".into();
    }
    if tokens < 1_000 {
        format!("{tokens}")
    } else if tokens < 1_000_000 {
        format!("{}k", trim_zero(format!("{:.1}", n / 1_000.0)))
    } else if tokens < 1_000_000_000 {
        let m = n / 1_000_000.0;
        if m >= 100.0 {
            format!("{:.0}M", m)
        } else {
            format!("{}M", trim_zero(format!("{m:.2}")))
        }
    } else {
        trim_zero(format!("{:.2}G", n / 1_000_000_000.0))
    }
}

/// 只处理纯数字字符串：剥掉尾部的 `.` 与多余的 `0`。
/// **单位后缀必须由调用方在外层拼**——因为 `trim_end_matches('0')` 遇到
/// `"1.0k"` 这种末尾是字母的串会直接不匹配，假精度就漏了出去。
fn trim_zero(s: String) -> String {
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        s
    }
}

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
        ModelUsage { model: "codex-5".into(), tokens: 5_340_000, cost: 6.10 },
        ModelUsage { model: "claude-sonnet-4-5".into(), tokens: 2_760_000, cost: 3.22 },
        ModelUsage { model: "claude-opus-4".into(), tokens: 2_070_000, cost: 9.41 },
        ModelUsage { model: "gpt-4o".into(), tokens: 1_860_000, cost: 2.05 },
        ModelUsage { model: "claude-haiku-4".into(), tokens: 489_000, cost: 0.31 },
    ];
    TokenReport {
        usage: TokenUsage {
            tokens24h: 12_080_000,
            tokens_total: 280_000_000,
            cost24h: 0.84,
            cost_total: 19.37,
        },
        models24h: models.clone(),
        models_total: models,
        hourly30d: hourly,
    }
}

#[cfg(test)]
mod tests {
    use super::compact;

    /// `compact` 是 token 数的显示格式化。契约三条：**短、不引入歧义、负数不出现**。
    /// `1.0k` 这种假精度会让人以为精确到百位；负数来自统计回绕，显示出来等于说谎。
    #[test]
    fn compact_short_unambiguous_never_negative() {
        assert_eq!(compact(0), "0");
        assert_eq!(compact(999), "999");
        assert_eq!(compact(-1), "0", "负数是统计回绕，必须显示 0 而不是 -1");

        let one_k = compact(1000);
        assert!(one_k.contains('k'), "1000 未格式化为 k 形式：{one_k}");
        assert!(!one_k.contains(".0"), "{one_k} 带了假精度（1.0k 读起来像精确到百位）");

        for raw in [0i64, 1, 999, 1000, 1500, 999_999, 1_000_000, 12_345_678] {
            let got = compact(raw);
            assert!(!got.trim().is_empty(), "compact({raw}) 为空");
            assert!(got.len() <= 8, "compact({raw}) = {got:?} 过长");
        }
    }

    /// 量级必须单调不降：`compact` 只做缩写不做取舍，
    /// 若 a <= b 却 compact(a) 在量级上大于 compact(b)，是分桶边界写错了。
    #[test]
    fn compact_magnitude_is_monotonic() {
        let seq = [500i64, 999, 1000, 1500, 999_999, 1_000_000, 1_500_000, 1_000_000_000];
        let mut prev_unit = 'd';
        for &v in &seq {
            let s = compact(v);
            let unit = s.chars().rev().find(|c| c.is_ascii_alphabetic()).unwrap_or('d');
            // k < M < G：单位只能往上升或不变，不能倒退
            let rank = |u: char| match u {
                'G' => 3,
                'M' => 2,
                'k' => 1,
                _ => 0,
            };
            assert!(
                rank(unit) >= rank(prev_unit),
                "量级倒退：{prev_unit} -> {unit}（token={v}）"
            );
            prev_unit = unit;
        }
    }
}
