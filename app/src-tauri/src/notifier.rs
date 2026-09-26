//! 外发调度：闸门 → 节流 → 传输 → 记账。
//!
//! 对齐 Swift `RemoteNotifier` 的 `attempt` / `send` / `claimThrottle` / `releaseThrottle` /
//! `record` 与 `OutboundOutcome` / `OutboundAttempt`。
//!
//! **传输层用 trait 注入**（Swift 侧是 `protocol RemoteTransport`）：真实现要选 TLS crate
//! （HTTPS 与 SMTP over 465），是下一轮的独立决定；本轮给的是 [`UnwiredTransport`]——
//! 它**如实说「没接入」**，而不是假装送达或静默丢弃。
//!
//! 三条硬规矩在这里落地：
//! ① **失败如实**：任何非 delivered 都记账并回传，界面看得到，绝不显示「已发送」；
//! ② **失败必须能立刻重试**：节流占位只登记在「已经送出去」之前的那一瞬间，
//!    没送达就撤回——否则一次网络抖动会吞掉后面一整段（默认 90 秒）的通知；
//! ③ **被策略挡下不算失败**，也不该记成成功：它记的是「为什么没发」。

use crate::remote::{Channel, ChannelConfig, Now, Policy, PresenceSignals};
use crate::render::{self, Inputs, Request};
use serde::Serialize;
use std::collections::{HashMap, VecDeque};

/// 一次外发的结果
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Outcome {
    Delivered,
    /// 被策略挡下（总开关关、该类事件关、节流命中、静默时段）——不算失败，也不该记成功
    Suppressed { reason: String },
    /// 配置不完整（没填 key / 主机 / 收件人）
    NotConfigured { reason: String },
    /// 送出但对方没接受，带可给人看的短说明（HTTP 状态码或 SMTP 回复码）。
    /// `permanent` = 对端**明确拒绝**（404 主题不存在、535 授权码错、550 拒绝中继）：
    /// 再试一次也是同一个结果。这一位既决定要不要重试，也决定界面怎么说——
    /// 把「配置就是错的」显示成「链路在抖」会把人引向完全错误的排查方向。
    Failed { reason: String, permanent: bool },
}

impl Outcome {
    pub fn is_delivered(&self) -> bool {
        matches!(self, Outcome::Delivered)
    }

    /// 只对 `Failed` 有意义；`Suppressed` / `NotConfigured` 不是「对端拒绝」
    pub fn is_permanent(&self) -> bool {
        matches!(
            self,
            Outcome::Failed {
                permanent: true,
                ..
            }
        )
    }

    fn base_text(&self) -> String {
        match self {
            Outcome::Delivered => "已送达".to_string(),
            Outcome::Suppressed { reason } => format!("未发：{reason}"),
            Outcome::NotConfigured { reason } => format!("未配置：{reason}"),
            Outcome::Failed { reason, .. } => format!("失败：{reason}"),
        }
    }
}

/// 一条外发记录（供界面显示「最近外发」，也供岛内提示——失败必须看得见）
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Attempt {
    pub at_ms: i64,
    pub title: String,
    pub outcome: Outcome,
    /// 真的往网络上试了几次（1 = 一次就出结果）。重试必须留痕：
    /// 「重试后送达」和「一次就送达」是不同的通道健康度，用户看得见才不会误判
    pub tries: i64,
}

impl Attempt {
    pub fn short_text(&self) -> String {
        let base = self.outcome.base_text();
        if self.tries > 1 {
            return if self.outcome.is_delivered() {
                format!("{base}（重试 {} 次后）", self.tries - 1)
            } else {
                format!("{base}；重试 {} 次仍未送达", self.tries - 1)
            };
        }
        // 只试了一次就停下的两种含义要分开：对端明确拒绝（去改配置）与
        // 「发送测试」按设计不重试（立刻给你这一次的真实结果）
        if self.outcome.is_permanent() {
            return format!("{base}（对端明确拒绝，重试无用）");
        }
        base
    }
}

/// 通道执行者。Swift 侧是 `protocol RemoteTransport: Sendable`（测试注入假实现）。
/// 引擎是跨线程共享的（`Arc<Mutex<ActivityEngine>>`），所以这里也要 `Send`。
pub trait Transport: Send {
    fn perform(&self, request: &Request) -> Outcome;
}

/// 给界面看的一条记录：**句子在 Rust 侧拼好**（与 `Attempt::short_text` 同一处口径），
/// 不然 JS 得自己把 outcome/tries 拼一遍，「重试后送达」这类说法就会有两份实现。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Recent {
    pub at_ms: i64,
    pub title: String,
    pub text: String,
    pub delivered: bool,
}

/// 最近外发记录的条数上限（与 Swift `record` 的 20 一致）
pub const HISTORY_LIMIT: usize = 20;
/// 节流键空间的上界：Agent 数 × 事件类型，用户会增删档案，顺手收个上界
const THROTTLE_KEYS_LIMIT: usize = 200;

pub struct Notifier {
    /// key = `agentId|kind` → 上次**真的送出去**的时刻
    throttle: HashMap<String, i64>,
    history: VecDeque<Attempt>,
    transport: Box<dyn Transport + Send>,
}

impl Default for Notifier {
    fn default() -> Self {
        Notifier {
            throttle: HashMap::new(),
            history: VecDeque::new(),
            // 默认就是真传输：它今天能发明文 http（自建/内网端点），
            // 对 https 与 SMTP 如实报「未接入」——见 `transport.rs`
            transport: Box::new(crate::transport::HttpTransport::new()),
        }
    }
}

impl Notifier {
    pub fn new() -> Self {
        Notifier::default()
    }

    /// 注入假传输的入口。**只在测试里用**：生产只有 `HttpTransport` 一个实现，
    /// 留一个生产侧没人调的构造器就是一段没人走的代码（Swift 侧靠 `protocol` 注入，
    /// 那边没有这个问题）。等真传输多起来（TLS / SMTP）再放开。
    #[cfg(test)]
    pub fn with_transport(transport: Box<dyn Transport + Send>) -> Self {
        Notifier {
            transport,
            ..Notifier::default()
        }
    }

    /// 五道闸。**顺序与 Swift `attempt` 逐条一致**，每条都有既有理由（见各分支注释）。
    /// 通过则返回节流键。
    #[allow(clippy::too_many_arguments)]
    fn gate(
        &mut self,
        inputs: &Inputs,
        policy: &Policy,
        channel: Channel,
        config: &ChannelConfig,
        has_secret: bool,
        now: Now,
        presence: &PresenceSignals,
        bypass_policy: bool,
    ) -> Result<String, Outcome> {
        let normalized = policy.normalized();
        // 总开关连「发送测试」一起挡：它是这个功能的隐私闸门，若按一次测试就能出本机，
        // 开关本身就不可信。也正因为排在前面，关掉时不会去碰钥匙串
        if !normalized.master_enabled {
            return Err(Outcome::Suppressed {
                reason: "总开关未开".into(),
            });
        }
        // 事件类型开关排在绕过范围之外：关掉「等待你确认」的人不该被测试按钮代发一条
        if !normalized.allows(inputs.kind) {
            return Err(Outcome::Suppressed {
                reason: "该类事件已关闭".into(),
            });
        }
        if !bypass_policy {
            // 绕过只覆盖「什么时候打扰用户」这三条里的后两条（节流 / 静默 / 在场）。
            // 本地分钟取不到时按**不静默**降级（fail-open）：宁可发出去，
            // 也不要出现「开关开着却永远不发」。
            let quiet = now
                .minutes_of_day
                .map(|minutes| normalized.in_quiet_hours(minutes))
                .unwrap_or(false);
            if quiet {
                return Err(Outcome::Suppressed {
                    reason: "静默时段".into(),
                });
            }
            // 与静默时段同级：都挡在配置检查之前，否则坐在机器前时会看到
            // 「缺主题」这种根本没走到的提示
            if normalized.only_when_away && !normalized.is_away(presence) {
                return Err(Outcome::Suppressed {
                    reason: format!("有人在机器前（{}）", normalized.present_reason(presence)),
                });
            }
        }
        // 配置检查放在节流之前：配置坏了是每次都发不出去，
        // 若先判节流，用户会看到「节流命中」而实际是根本没配好
        if let Some(missing) = channel.missing_field(config, has_secret) {
            return Err(Outcome::NotConfigured {
                reason: missing.to_string(),
            });
        }
        let key = format!("{}|{}", inputs.agent_id, inputs.kind.as_str());
        if !bypass_policy && !self.claim(&key, now.ms, normalized.throttle_seconds) {
            return Err(Outcome::Suppressed {
                reason: "节流命中".into(),
            });
        }
        Ok(key)
    }

    /// 判定 + 真发 + 记账。
    /// `has_secret` / `presence` 由调用方给：本模块不碰钥匙串、也不取窗口服务器信号，
    /// 判据保持纯函数（每次外发都会调它）。
    ///
    /// **已知限制（会随传输层一起改）**：这里是**同步**的，而传输有 10 秒超时
    /// （Swift 同值）。Swift 侧 `deliver` 是 async、在后台 Task 里跑，Rust 的引擎是同步的，
    /// 于是这一调用会占住引擎那一拍。今天**打不到**这条路径：总开关默认关（判据在传输之前
    /// 就返回），而且 https / SMTP 在 `HttpTransport` 里是**立即失败、不发网络 I/O** ——
    /// 只有用户手动打开总开关、又配了一个 `http://` 且会挂住的端点才可能卡住那一拍。
    /// 改成「起线程发送 + 结果回填」是传输层那一轮的事（重试也需要它）。
    #[allow(clippy::too_many_arguments)]
    pub fn attempt(
        &mut self,
        inputs: &Inputs,
        policy: &Policy,
        channel: Channel,
        config: &ChannelConfig,
        has_secret: bool,
        now: Now,
        presence: &PresenceSignals,
        bypass_policy: bool,
    ) -> Outcome {
        // 标题用渲染出来的那个（与 Swift 同一处取值），被挡下时也要有标题
        let title = render::render(inputs, config).title;
        let throttle_key =
            match self.gate(inputs, policy, channel, config, has_secret, now, presence, bypass_policy)
            {
                Ok(key) => key,
                Err(outcome) => {
                    self.record(Attempt {
                        at_ms: now.ms,
                        title,
                        outcome: outcome.clone(),
                        tries: 1,
                    });
                    return outcome;
                }
            };

        // 密钥本轮一律 `None`（钥匙串未接入）：渲染出的请求不会带任何凭据，
        // `{key}` 位置留空——由传输层如实失败，而不是拿一个空密钥去撞对端
        let message = render::render(inputs, config);
        let request = render::render_request(&message, channel, config, None, false);
        let outcome = self.transport.perform(&request);
        if !outcome.is_delivered() {
            // 没发出去就撤掉预登记：失败必须能立刻重试；
            // 撤在「重试之后」是 Swift 的既定顺序（占位在整个初次+重试期间都握着）
            self.release(&throttle_key, now.ms);
        }
        self.record(Attempt {
            at_ms: now.ms,
            title,
            outcome: outcome.clone(),
            tries: 1,
        });
        outcome
    }

    /// 原子地「查节流 + 预登记」。返回 false 表示落在窗口内，本次不该发
    fn claim(&mut self, key: &str, now_ms: i64, window_seconds: i64) -> bool {
        if let Some(last) = self.throttle.get(key) {
            if now_ms - *last < window_seconds * 1000 {
                return false;
            }
        }
        self.throttle.insert(key.to_string(), now_ms);
        if self.throttle.len() > THROTTLE_KEYS_LIMIT {
            // 顺手收个上界：一天前的登记已经没有意义
            self.throttle
                .retain(|_, at| now_ms - *at <= 86_400_000);
        }
        true
    }

    /// 撤掉预登记。只撤「还是本次登记」的那一条——按时刻比对，
    /// 否则会把同一键上别人刚登记的时间戳抹掉
    fn release(&mut self, key: &str, at_ms: i64) {
        if self.throttle.get(key) == Some(&at_ms) {
            self.throttle.remove(key);
        }
    }

    fn record(&mut self, attempt: Attempt) {
        self.history.push_front(attempt);
        while self.history.len() > HISTORY_LIMIT {
            self.history.pop_back();
        }
    }

    /// 最近若干次外发结果（有界，新的在前）
    pub fn recent(&self) -> Vec<Attempt> {
        self.history.iter().cloned().collect()
    }

    /// 给界面看的最近若干次（句子已拼好）
    pub fn recent_view(&self) -> Vec<Recent> {
        self.recent()
            .iter()
            .map(|attempt| Recent {
                at_ms: attempt.at_ms,
                title: attempt.title.clone(),
                text: attempt.short_text(),
                delivered: attempt.outcome.is_delivered(),
            })
            .collect()
    }

    /// 当前握着的节流占位数（诊断用：正常应当只有「刚发出去还没过窗口」的那些）
    pub fn throttle_keys(&self) -> usize {
        self.throttle.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote::{EventKind, Policy};
    use std::sync::{Arc, Mutex};

    /// 假传输：记录收到的请求，返回预设结果（Swift 侧 `MockTransport` 同一角色）
    struct FakeTransport {
        outcome: Outcome,
        seen: Arc<Mutex<Vec<Request>>>,
    }

    impl Transport for FakeTransport {
        fn perform(&self, request: &Request) -> Outcome {
            self.seen.lock().unwrap().push(request.clone());
            self.outcome.clone()
        }
    }

    fn fixture(outcome: Outcome) -> (Notifier, Arc<Mutex<Vec<Request>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let transport = FakeTransport {
            outcome,
            seen: seen.clone(),
        };
        (Notifier::with_transport(Box::new(transport)), seen)
    }

    fn enabled() -> Policy {
        Policy {
            master_enabled: true,
            ..Policy::default()
        }
    }

    fn ntfy() -> ChannelConfig {
        ChannelConfig {
            topic_or_url: "island".into(),
            ..ChannelConfig::default()
        }
    }

    fn inputs(kind: EventKind) -> Inputs {
        Inputs::new("Claude", kind, 0.0)
    }

    fn now(ms: i64) -> Now {
        Now {
            ms,
            minutes_of_day: Some(12 * 60),
        }
    }

    fn unavailable() -> PresenceSignals {
        PresenceSignals::unavailable()
    }

    fn run(
        notifier: &mut Notifier,
        kind: EventKind,
        policy: &Policy,
        at: i64,
    ) -> Outcome {
        notifier.attempt(
            &inputs(kind),
            policy,
            Channel::Ntfy,
            &ntfy(),
            false,
            now(at),
            &unavailable(),
            false,
        )
    }

    #[test]
    fn the_master_switch_blocks_everything_including_bypass() {
        // 总开关是这个功能的隐私闸门：若按一次测试就能出本机，开关本身就不可信
        let (mut notifier, seen) = fixture(Outcome::Delivered);
        let policy = Policy::default(); // 默认关
        let outcome = run(&mut notifier, EventKind::Attention, &policy, 1_000);
        assert_eq!(
            outcome,
            Outcome::Suppressed {
                reason: "总开关未开".into()
            }
        );
        // 连 bypass（发送测试）也挡住
        let bypassed = notifier.attempt(
            &inputs(EventKind::Attention),
            &policy,
            Channel::Ntfy,
            &ntfy(),
            false,
            now(2_000),
            &unavailable(),
            true,
        );
        assert_eq!(
            bypassed,
            Outcome::Suppressed {
                reason: "总开关未开".into()
            }
        );
        assert!(seen.lock().unwrap().is_empty(), "被挡下就不该碰传输层");
        assert_eq!(notifier.throttle_keys(), 0, "被挡下不该占节流位");
    }

    #[test]
    fn the_event_type_switch_is_outside_the_bypass_range() {
        let (mut notifier, seen) = fixture(Outcome::Delivered);
        let policy = Policy {
            send_attention: false,
            ..enabled()
        };
        let outcome = notifier.attempt(
            &inputs(EventKind::Attention),
            &policy,
            Channel::Ntfy,
            &ntfy(),
            false,
            now(1_000),
            &unavailable(),
            true, // 发送测试也代发不了被关掉的类型
        );
        assert_eq!(
            outcome,
            Outcome::Suppressed {
                reason: "该类事件已关闭".into()
            }
        );
        assert!(seen.lock().unwrap().is_empty());
    }

    #[test]
    fn quiet_hours_and_presence_are_bypassed_but_the_config_check_is_not() {
        let (mut notifier, seen) = fixture(Outcome::Delivered);
        let policy = Policy {
            quiet_start: "22:00".into(),
            quiet_end: "07:00".into(),
            only_when_away: true,
            ..enabled()
        };
        // 静默时段内、人在机器前 ⇒ 正常路径被挡
        let at = Now {
            ms: 1_000,
            minutes_of_day: Some(23 * 60),
        };
        let quiet = notifier.attempt(
            &inputs(EventKind::Attention),
            &policy,
            Channel::Ntfy,
            &ntfy(),
            false,
            at,
            &PresenceSignals {
                idle_seconds: Some(1.0),
                ..PresenceSignals::default()
            },
            false,
        );
        assert_eq!(quiet, Outcome::Suppressed { reason: "静默时段".into() });

        // bypass 绕过这两条 ⇒ 这次要真的走传输
        let bypassed = notifier.attempt(
            &inputs(EventKind::Attention),
            &policy,
            Channel::Ntfy,
            &ntfy(),
            false,
            at,
            &PresenceSignals {
                idle_seconds: Some(1.0),
                ..PresenceSignals::default()
            },
            true,
        );
        assert_eq!(bypassed, Outcome::Delivered);
        assert_eq!(seen.lock().unwrap().len(), 1);

        // 但配置检查不在绕过范围内：测试按钮也发不出一条没配好的通道
        let (mut second, _) = fixture(Outcome::Delivered);
        let unconfigured = second.attempt(
            &inputs(EventKind::Attention),
            &policy,
            Channel::Ntfy,
            &ChannelConfig::default(), // 缺主题名
            false,
            at,
            &unavailable(),
            true,
        );
        assert_eq!(
            unconfigured,
            Outcome::NotConfigured {
                reason: "缺主题名或服务器地址".into()
            }
        );
    }

    #[test]
    fn presence_blocks_only_when_the_policy_asks_for_it() {
        let (mut notifier, _) = fixture(Outcome::Delivered);
        let present = PresenceSignals {
            idle_seconds: Some(3.0), // 刚动过键盘
            ..PresenceSignals::default()
        };
        let by_default = notifier.attempt(
            &inputs(EventKind::Attention),
            &enabled(),
            Channel::Ntfy,
            &ntfy(),
            false,
            now(1_000),
            &present,
            false,
        );
        assert_eq!(by_default, Outcome::Delivered, "默认不要求「人不在」");

        let (mut strict, _) = fixture(Outcome::Delivered);
        let policy = Policy {
            only_when_away: true,
            ..enabled()
        };
        let blocked = strict.attempt(
            &inputs(EventKind::Attention),
            &policy,
            Channel::Ntfy,
            &ntfy(),
            false,
            now(1_000),
            &present,
            false,
        );
        match blocked {
            Outcome::Suppressed { reason } => {
                assert!(reason.starts_with("有人在机器前（"), "{reason}");
                assert!(reason.contains("距上次输入 3 秒"), "要说得清为什么被挡：{reason}");
            }
            other => panic!("应被挡下，实际 {other:?}"),
        }
    }

    /// 配置检查必须**排在节流之前**：配置坏了是每次都发不出去，
    /// 若先判节流，用户会看到「节流命中」而实际是根本没配好——
    /// 于是他会去等窗口，而不是去补主题名。
    #[test]
    fn a_broken_config_beats_the_throttle_in_the_reported_reason() {
        let (mut notifier, _) = fixture(Outcome::Delivered);
        let policy = enabled();
        // 先配好、发成功一次 ⇒ 节流窗口被占住
        assert_eq!(run(&mut notifier, EventKind::Attention, &policy, 1_000), Outcome::Delivered);
        // 同一时刻把配置改坏再试：必须报配置问题，不许报「节流命中」
        let outcome = notifier.attempt(
            &inputs(EventKind::Attention),
            &policy,
            Channel::Ntfy,
            &ChannelConfig::default(), // 缺主题名
            false,
            now(1_500),
            &unavailable(),
            false,
        );
        assert_eq!(
            outcome,
            Outcome::NotConfigured {
                reason: "缺主题名或服务器地址".into()
            },
            "配置问题要盖过节流：否则用户去等窗口，而不是去补配置"
        );
    }

    #[test]
    fn the_throttle_window_holds_and_a_failure_releases_it() {
        let (mut notifier, seen) = fixture(Outcome::Delivered);
        let policy = enabled(); // 节流 90 秒
        assert_eq!(run(&mut notifier, EventKind::Attention, &policy, 1_000), Outcome::Delivered);
        assert_eq!(
            run(&mut notifier, EventKind::Attention, &policy, 1_000 + 89_999),
            Outcome::Suppressed {
                reason: "节流命中".into()
            },
            "窗口内不重复发"
        );
        assert_eq!(seen.lock().unwrap().len(), 1);
        assert_eq!(
            run(&mut notifier, EventKind::Attention, &policy, 1_000 + 90_000),
            Outcome::Delivered,
            "窗口一到就放行"
        );

        // 不同事件类型各有各的窗口
        assert_eq!(run(&mut notifier, EventKind::CostSpike, &policy, 1_000 + 90_000), Outcome::Delivered);

        // 失败必须撤回占位，否则一次抖动会吞掉后面一整段
        let (mut failing, _) = fixture(Outcome::Failed {
            reason: "连接被拒".into(),
            permanent: false,
        });
        let first = run(&mut failing, EventKind::Attention, &policy, 5_000);
        assert!(matches!(first, Outcome::Failed { .. }));
        assert_eq!(failing.throttle_keys(), 0, "没送达就该撤掉登记");
        assert_eq!(
            run(&mut failing, EventKind::Attention, &policy, 5_100),
            Outcome::Failed {
                reason: "连接被拒".into(),
                permanent: false
            },
            "立刻重试不该被自己的节流挡住"
        );
    }

    #[test]
    fn records_keep_the_newest_twenty_and_read_like_a_sentence() {
        let (mut notifier, _) = fixture(Outcome::Delivered);
        let policy = enabled();
        for i in 0..25 {
            // 每次换一个 agent，绕开节流，专测记账
            let mut who = inputs(EventKind::Completed);
            who.agent_id = format!("a{i}");
            notifier.attempt(
                &who,
                &policy,
                Channel::Ntfy,
                &ntfy(),
                false,
                now(1_000 + i),
                &unavailable(),
                false,
            );
        }
        let recent = notifier.recent();
        assert_eq!(recent.len(), HISTORY_LIMIT);
        assert_eq!(recent[0].at_ms, 1_024, "新的在前");
        assert_eq!(recent[0].short_text(), "已送达");
        assert_eq!(recent[0].title, "Claude · 任务完成");

        let suppressed = Attempt {
            at_ms: 0,
            title: "t".into(),
            outcome: Outcome::Suppressed {
                reason: "静默时段".into(),
            },
            tries: 1,
        };
        assert_eq!(suppressed.short_text(), "未发：静默时段");
        let permanent = Attempt {
            at_ms: 0,
            title: "t".into(),
            outcome: Outcome::Failed {
                reason: "404 主题不存在".into(),
                permanent: true,
            },
            tries: 1,
        };
        assert_eq!(
            permanent.short_text(),
            "失败：404 主题不存在（对端明确拒绝，重试无用）"
        );
        let retried = Attempt {
            at_ms: 0,
            title: "t".into(),
            outcome: Outcome::Delivered,
            tries: 3,
        };
        assert_eq!(retried.short_text(), "已送达（重试 2 次后）");
        let gave_up = Attempt {
            at_ms: 0,
            title: "t".into(),
            outcome: Outcome::Failed {
                reason: "超时".into(),
                permanent: false,
            },
            tries: 2,
        };
        assert_eq!(gave_up.short_text(), "失败：超时；重试 1 次仍未送达");
    }

    #[test]
    fn the_default_transport_really_attempts_and_records_into_the_ledger() {
        // 默认传输 = `HttpTransport`（真连）。指向一个刚释放的本地端口：
        // 走真传输、真失败、真进账本，但**不碰外网**——用例不该依赖网络。
        let closed = {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            drop(listener);
            port
        };
        let config = ChannelConfig {
            url_template: format!("http://127.0.0.1:{closed}/notify"),
            body_template: "t={title}".into(),
            ..ChannelConfig::default()
        };
        let mut notifier = Notifier::new();
        let outcome = notifier.attempt(
            &inputs(EventKind::Attention),
            &enabled(),
            Channel::CustomHttp,
            &config,
            false,
            now(1_000),
            &unavailable(),
            false,
        );
        match &outcome {
            Outcome::Failed { reason, .. } => assert!(reason.contains("连接失败"), "{reason}"),
            other => panic!("应报连接失败，实际 {other:?}"),
        }
        let recent = notifier.recent_view();
        assert_eq!(recent.len(), 1);
        assert!(recent[0].text.starts_with("失败："), "{}", recent[0].text);
        assert!(!recent[0].delivered);
    }

    #[test]
    fn the_rendered_request_never_carries_a_secret_while_the_keychain_is_unwired() {
        // 钥匙串未接入 ⇒ 密钥传 None：请求里不许出现任何凭据占位被填上值
        let seen = Arc::new(Mutex::new(Vec::new()));
        let transport = FakeTransport {
            outcome: Outcome::Delivered,
            seen: seen.clone(),
        };
        let mut notifier = Notifier::with_transport(Box::new(transport));
        let config = ChannelConfig {
            url_template: "https://x/y?key={key}".into(),
            body_template: "t={title}".into(),
            ..ChannelConfig::default()
        };
        let outcome = notifier.attempt(
            &inputs(EventKind::Attention),
            &enabled(),
            Channel::CustomHttp,
            &config,
            true,
            now(1_000),
            &unavailable(),
            false,
        );
        assert_eq!(outcome, Outcome::Delivered);
        let request = seen.lock().unwrap()[0].clone();
        assert!(
            request.url.ends_with("key="),
            "没有密钥就留空，不要编一个：{}",
            request.url
        );
    }
}
