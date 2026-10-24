//! 外发调度：闸门 → 节流 → 传输 → 记账。
//!
//! 对齐 Swift `RemoteNotifier` 的 `attempt` / `send` / `claimThrottle` / `releaseThrottle` /
//! `record` 与 `OutboundOutcome` / `OutboundAttempt`。
//!
//! **不阻塞采样**：`attempt` 同步跑完一次外发（含重试），但引擎走的是
//! [`Notifier::dispatch`]——把它丢到工作线程，账本随后回填。Swift 侧靠 async Task 达到同一效果，
//! Rust 的引擎是同步的，所以用线程。传输有 10 秒超时（Swift 同值），
//! 而 https 接进来之后「会真去连」是常态路径，这一步不能省。
//!
//! 三条硬规矩在这里落地：
//! ① **失败如实**：任何非 delivered 都记账并回传，界面看得到，绝不显示「已发送」；
//! ② **失败必须能立刻重试**：节流占位只在「已经要发」的那一瞬间落下，没送达就撤回——
//!    否则一次网络抖动会吞掉后面一整段（默认 90 秒）的通知；
//! ③ **被策略挡下不算失败**，也不该记成成功：它记的是「为什么没发」。

use crate::remote::{Channel, ChannelConfig, Now, Policy, PresenceSignals};
use crate::render::{self, Inputs, Request};
use crate::secret::{self, SecretStore};
use serde::Serialize;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// 一次外发的结果
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Outcome {
    /// 对端**受理**了这条请求（HTTP 2xx / SMTP 2xx）。
    ///
    /// 名字刻意不叫 `Delivered`：受理 ≠ 收到 ≠ 已读（NOT-05）。
    /// 本产品绝大多数渠道只能走到「受理」这一段——把受理显示成「已送达」
    /// 就是替对端做了它没做过的承诺。收到/已读要等回执能力接入
    /// （见 [`DeliveryState`]）。
    Accepted,
    /// 被策略挡下（总开关关、该类事件关、节流命中、静默时段）——不算失败，也不该记成功
    Suppressed {
        reason: String,
    },
    /// 配置不完整（没填 key / 主机 / 收件人）
    NotConfigured {
        reason: String,
    },
    /// 限流：对端明确要求「过一会儿再来」（HTTP 429/503 带 `Retry-After`）。
    /// 与 [`Failed`] 分开是**行为**上的差别：限流不该计入故障升级，
    /// 也不该被当成「通道坏了」去排查配置。
    Throttled {
        reason: String,
        /// 对端要求的等待时长（毫秒）。`None` = 头缺失或解析不了，只显示限流本身
        retry_after_ms: Option<i64>,
    },
    /// 送出但对方没接受，带可给人看的短说明（HTTP 状态码或 SMTP 回复码）。
    /// `permanent` = 对端**明确拒绝**（404 主题不存在、535 授权码错、550 拒绝中继）：
    /// 再试一次也是同一个结果。这一位既决定要不要重试，也决定界面怎么说——
    /// 把「配置就是错的」显示成「链路在抖」会把人引向完全错误的排查方向。
    Failed {
        reason: String,
        permanent: bool,
    },
}

/// 交付状态三段：**受理 → 收到 → 已读**（NOT-05 要求「受理/收到/已读分开」）。
///
/// 当前所有渠道的自动外发都只能走到第一段；`Received`/`Read` 需要对端回执，
/// 在回执能力接入前**一律不填**——留空态（`Unknown`）比填一个假的「已读」诚实。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DeliveryState {
    /// 平台受理（2xx），送达未知
    Accepted,
    /// 对端确认收到（需回执；当前未接入任何回执渠道）
    Received,
    /// 对端确认已读（需已读回执；当前未接入）
    Read,
    /// 未走到任何一段（被挡下/未配置/失败）
    Unknown,
}

impl DeliveryState {
    /// 给界面的短文案。与 Rust 侧口径一致，界面不自己拼。
    pub fn label(self) -> &'static str {
        match self {
            DeliveryState::Accepted => "已受理，未确认送达",
            DeliveryState::Received => "已送达",
            DeliveryState::Read => "已读",
            DeliveryState::Unknown => "未确认",
        }
    }
}

impl Outcome {
    /// 「外发成功」= 对端受理。**不等于**送达或已读——那是 [`DeliveryState`] 的事。
    pub fn is_accepted(&self) -> bool {
        matches!(self, Outcome::Accepted)
    }

    /// 只对 `Failed` 有意义；`Suppressed` / `NotConfigured` / `Throttled` 不是「对端拒绝」
    pub fn is_permanent(&self) -> bool {
        matches!(
            self,
            Outcome::Failed {
                permanent: true,
                ..
            }
        )
    }

    /// 本次结果落在交付状态的哪一段。
    pub fn delivery_state(&self) -> DeliveryState {
        match self {
            Outcome::Accepted => DeliveryState::Accepted,
            _ => DeliveryState::Unknown,
        }
    }

    fn base_text(&self) -> String {
        match self {
            // 刻意不说「已送达」：受理与送达之间隔着对端自己那一步
            Outcome::Accepted => "平台已受理".to_string(),
            Outcome::Suppressed { reason } => format!("未发：{reason}"),
            Outcome::NotConfigured { reason } => format!("未配置：{reason}"),
            Outcome::Throttled {
                reason,
                retry_after_ms,
            } => match retry_after_ms {
                Some(ms) => format!("{reason}（约 {} 秒后可重试）", (ms / 1000).max(0)),
                None => reason.clone(),
            },
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
            return if self.outcome.is_accepted() {
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
/// 引擎跨线程共享，而且工作线程要拿着它去发，所以这里要 `Send + Sync`。
pub trait Transport: Send + Sync {
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
    /// 外发成功（对端**受理**）。名字不叫 `delivered`：受理 ≠ 送达 ≠ 已读（NOT-05），
    /// 三段见 `delivery`。保留布尔是因为「是否成功」是列表排序与筛选最常用的判断。
    pub accepted: bool,
    /// 交付状态三段（NOT-05：受理 ≠ 收到 ≠ 已读）。当前渠道只能到 accepted。
    pub delivery: DeliveryState,
    /// 交付状态的**界面文案**，由 Rust 侧拼好（`DeliveryState::label`）。
    /// 界面不再自己维护一份映射——两份口径必然漂移，而漂移的后果是
    /// 「受理成功的消息被显示成未确认」这种静默降级（复审 A4 抓到过）。
    pub delivery_label: &'static str,
}

/// 最近外发记录的条数上限（与 Swift `record` 的 20 一致）
pub const HISTORY_LIMIT: usize = 20;
/// 节流键空间的上界：Agent 数 × 事件类型，用户会增删档案，顺手收个上界
const THROTTLE_KEYS_LIMIT: usize = 200;
/// 自动外发失败后的重试间隔（Swift `retryDelay` 默认 5 秒）
pub const RETRY_DELAY_MS: i64 = 5_000;
/// **连续**临时失败多少次升级为通道故障（NOT-05「故障升级」）。
/// 取 3：一次抖动由重试吸收，两次仍抖说明链路不稳，三次就该让用户看见。
pub const ESCALATE_AFTER_FAILURES: i64 = 3;

#[derive(Default)]
struct Inner {
    /// key = `agentId|kind` → 上次**真的送出去**的时刻
    throttle: HashMap<String, i64>,
    history: VecDeque<Attempt>,
    /// key = 通道种类 → 连续临时失败次数（对端拒绝/限流/被挡下都清零或不计）
    failure_streak: HashMap<String, i64>,
    /// key = 通道种类 → 是否已升级为「通道故障」
    escalated: HashMap<String, bool>,
    /// 来源事件 → 外发尝试完成的延迟样本（NOT-01「两段 p95」的外发段）。
    /// 只记**真的往网络上试过**的那次（闸门挡下的早退不记——那段没有外发）。
    outbound_latency: crate::notify_latency::NotifyLatency,
}

/// 外发调度器。内部状态是共享的（`Arc<Mutex<_>>`），所以它可克隆、
/// 可以交给工作线程，而引擎侧读账本不必等外发结束。
#[derive(Clone)]
pub struct Notifier {
    inner: Arc<Mutex<Inner>>,
    transport: Arc<dyn Transport>,
    /// 密钥来源（生产 = macOS 钥匙串）。**只在闸门放行之后才读它**：
    /// 总开关关着时不该碰钥匙串——那是这个功能的隐私闸门。
    secrets: Arc<dyn SecretStore>,
    /// 失败后重试的间隔；0 = 不睡（测试）。**要不要重试**由 `retry_attempts` 控制
    retry_delay_ms: i64,
    /// 初次之外再试几次（Swift 是 1）
    retry_attempts: i64,
}

impl Default for Notifier {
    fn default() -> Self {
        Notifier {
            inner: Arc::new(Mutex::new(Inner::default())),
            // 默认就是真传输：它能发明文 http 与 https（系统 TLS 栈），
            // 对 SMTP 如实报「会话未接入」——见 `transport.rs`
            transport: Arc::new(crate::transport::HttpTransport::new()),
            secrets: Arc::new(secret::Keychain),
            retry_delay_ms: RETRY_DELAY_MS,
            retry_attempts: 1,
        }
    }
}

impl Notifier {
    pub fn new() -> Self {
        Notifier::default()
    }

    /// 注入假传输与重试参数。**只在测试里用**：生产只有 `HttpTransport` 一个实现。
    #[cfg(test)]
    pub fn with_transport(transport: Box<dyn Transport>) -> Self {
        Notifier {
            transport: Arc::from(transport),
            // **用例绝不碰真钥匙串**：默认一个空键盘串，要测密钥的用 `with_secrets`。
            // 真钥匙串在 ad-hoc 签名下可能弹授权框，用例不该触发它。
            secrets: Arc::new(EmptySecrets),
            retry_delay_ms: 0,
            // 既有用例关心的是闸门与记账，不该被重试改变语义；要测重试的自己开
            retry_attempts: 0,
            ..Notifier::default()
        }
    }

    /// 注入密钥源（测试用）
    #[cfg(test)]
    pub fn with_secrets(mut self, secrets: Arc<dyn SecretStore>) -> Self {
        self.secrets = secrets;
        self
    }

    /// 打开重试（测试用）
    #[cfg(test)]
    pub fn with_retry(mut self, attempts: i64, delay_ms: i64) -> Self {
        self.retry_attempts = attempts;
        self.retry_delay_ms = delay_ms;
        self
    }

    /// 闸门第一段：**不需要密钥**的那几道。
    ///
    /// 顺序与 Swift `attempt` 逐条一致，而且这个「先做这一段」的切分本身就是口径：
    /// 总开关 / 类型 / 静默 / 在场都判完才去碰钥匙串——总开关关着时不该读密钥
    /// （它是这个功能的隐私闸门），也正因为排在最前面，关掉时连钥匙串都不碰。
    fn gate_policy(
        &self,
        inputs: &Inputs,
        policy: &Policy,
        now: Now,
        presence: &PresenceSignals,
        bypass_policy: bool,
    ) -> Result<(), Outcome> {
        let normalized = policy.normalized();
        // 总开关连「发送测试」一起挡：若按一次测试就能出本机，开关本身就不可信
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
            // 绕过只覆盖「什么时候打扰用户」这两条。本地分钟取不到时按**不静默**降级
            // （fail-open）：宁可发出去，也不要出现「开关开着却永远不发」。
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
        Ok(())
    }

    /// 闸门第二段：配置检查与节流。**必须排在密钥读取之后**——
    /// 「配齐了没有」的第一条就是「有没有密钥」。
    fn gate_config_and_claim(
        &self,
        inputs: &Inputs,
        policy: &Policy,
        channel: Channel,
        config: &ChannelConfig,
        has_secret: bool,
        now: Now,
        bypass_policy: bool,
    ) -> Result<String, Outcome> {
        let normalized = policy.normalized();
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

    /// 判定 + 真发 + 记账（**同步**，含重试）。
    /// `presence` 由调用方给：本模块不取窗口服务器信号，判据保持纯函数。
    /// 密钥**自己读**（读在策略闸门之后，见下），所以调用方不必也不该先把它读出来。
    ///
    /// 引擎**不该直接调它**——传输有 10 秒超时，会占住采样那一拍；走 [`Notifier::dispatch`]。
    #[allow(clippy::too_many_arguments)]
    pub fn attempt(
        &self,
        inputs: &Inputs,
        policy: &Policy,
        channel: Channel,
        config: &ChannelConfig,
        now: Now,
        presence: &PresenceSignals,
        bypass_policy: bool,
    ) -> Outcome {
        // 标题用渲染出来的那个（与 Swift 同一处取值），被挡下时也要有标题
        let title = render::render(inputs, config).title;
        if let Err(outcome) = self.gate_policy(inputs, policy, now, presence, bypass_policy) {
            self.record(Attempt {
                at_ms: now.ms,
                title,
                outcome: outcome.clone(),
                tries: 1,
            });
            return outcome;
        }

        // **到这一步才碰钥匙串**。读的是该通道固定的条目名（不是可配字段）：
        // 「密钥存了但条目名对不上」是查不出来的失效，所以条目名由通道种类唯一决定。
        let secret = self.secrets.read(&secret::default_secret_name(channel));

        let throttle_key = match self.gate_config_and_claim(
            inputs,
            policy,
            channel,
            config,
            secret.is_some(),
            now,
            bypass_policy,
        ) {
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

        // 密钥只进这一个地方：渲染出来的请求。它不进账本、不进日志、不进界面
        // （`Request::masked_preview` 另有掩码）。
        let message = render::render(inputs, config);
        let request = render::render_request(&message, channel, config, secret.as_deref(), false);
        let mut tries = 1;
        let mut outcome = self.transport.perform(&request);
        // 自动外发失败重试一次：这个功能存在的意义就是「人不在机器前也要收到」，
        // 而一次网络抖动就把那条唯一提醒永久丢掉。**「发送测试」不重试**——
        // 用户盯着界面等，立刻拿到如实结果更有用；对端明确拒绝（404/535/550）也不重试，
        // 再要一次只是给已经说过「不」的服务器加负载，而且 ntfy.sh 这类公开中转按条数限流。
        //
        // **限流（Throttled）不重试**：对端已经指定了「什么时候再来」（Retry-After），
        // 立刻重试既不尊重它也大概率再吃一次 429；**已升级为通道故障时不重试**——
        // 那是「不循环」：连续抖动说明链路本身不稳，再试一次不会变好，只会把
        // 失败风暴堆在用户屏幕上。
        if !bypass_policy
            && self.retry_attempts > 0
            && !self.is_escalated(channel)
            && matches!(
                outcome,
                Outcome::Failed {
                    permanent: false,
                    ..
                }
            )
        {
            if self.retry_delay_ms > 0 {
                std::thread::sleep(Duration::from_millis(self.retry_delay_ms as u64));
            }
            tries += 1;
            outcome = self.transport.perform(&request);
        }
        if !outcome.is_accepted() {
            // 没发出去就撤掉预登记：失败必须能立刻重试。
            // 撤在**重试之后**是 Swift 的既定顺序：占位在整个「初次 + 重试」期间都握着，
            // 同一键的并发事件挤不进第二条
            self.release(&throttle_key, now.ms);
        }
        self.note_channel_result(channel, &outcome);
        // NOT-01「两段 p95」的外发段：事件发生（`now.ms`，调用方传事件时间戳）
        // → 外发尝试完成（真实墙钟）。到得了这一行的都是**真的往网络上试过**的
        // （闸门挡下在两处早退，根本到不了这里）；失败与限流同样计时——
        // 「多久知道发不出去」和「多久发出去」是同一条链路的健康度。
        // 时钟回拨产生负值时由记录器丢弃并计数，不冒充样本。
        self.inner
            .lock()
            .expect("外发状态锁不应中毒")
            .outbound_latency
            .record(crate::tokens::now_ms() - now.ms);
        self.record(Attempt {
            at_ms: now.ms,
            title,
            outcome: outcome.clone(),
            tries,
        });
        outcome
    }

    /// 外发段延迟摘要（事件发生 → 外发尝试完成）。与入队段同一份记录器，
    /// 界面与自检共用同一个形状，不另造第二套统计。
    pub fn outbound_latency_summary(&self) -> crate::notify_latency::LatencySummary {
        self.inner
            .lock()
            .expect("外发状态锁不应中毒")
            .outbound_latency
            .summary()
    }

    /// 把一次外发丢给工作线程，**立刻返回**。
    /// 引擎每一拍都调它：传输最长 10 秒，直接在采样线程里跑会让界面卡住。
    /// 结果由工作线程写进账本（`recent_view` 读得到）。
    #[allow(clippy::too_many_arguments)]
    pub fn dispatch(
        &self,
        inputs: Inputs,
        policy: Policy,
        channel: Channel,
        config: ChannelConfig,
        now: Now,
        presence: PresenceSignals,
        bypass_policy: bool,
    ) {
        let worker = self.clone();
        std::thread::spawn(move || {
            worker.attempt(
                &inputs,
                &policy,
                channel,
                &config,
                now,
                &presence,
                bypass_policy,
            );
        });
    }

    /// 该通道当前是否已升级为「通道故障」。
    fn is_escalated(&self, channel: Channel) -> bool {
        self.inner
            .lock()
            .expect("外发状态锁不应中毒")
            .escalated
            .get(channel.as_str())
            .copied()
            .unwrap_or(false)
    }

    /// 按本次结果维护通道的失败连击与升级态（NOT-05「故障升级不循环」）。
    ///
    /// - 受理成功 → 清零并复位升级（通道恢复了就该恢复自动外发）；
    /// - **连续**临时失败 → 连击 +1，达到阈值升级；
    /// - 对端明确拒绝 / 限流 / 被策略挡下 → 清零：它们不是链路故障，
    ///   不该让「上一次抖动」的连击继续累加到完全无关的原因上。
    fn note_channel_result(&self, channel: Channel, outcome: &Outcome) {
        let key = channel.as_str().to_string();
        let mut inner = self.inner.lock().expect("外发状态锁不应中毒");
        match outcome {
            Outcome::Accepted => {
                inner.failure_streak.remove(&key);
                inner.escalated.remove(&key);
            }
            Outcome::Failed {
                permanent: false,
                ..
            } => {
                let streak = inner.failure_streak.entry(key.clone()).or_insert(0);
                *streak += 1;
                if *streak >= ESCALATE_AFTER_FAILURES {
                    inner.escalated.insert(key, true);
                }
            }
            // 对端明确拒绝（配置错）、限流：**都不是链路抖动**，把连击归零。
            // 归零而非「保留不动」：这两类说明这一次的失败原因与先前那串抖动无关，
            // 继续累加会把「配置错一次 + 抖动两次」误判成通道故障。
            // 被策略挡下（Suppressed/NotConfigured）走闸门早退，根本到不了这里——
            // 那才是真正的「保留不动」（一次都没往网络上试）。
            _ => {
                inner.failure_streak.remove(&key);
            }
        }
    }

    /// 原子地「查节流 + 预登记」。返回 false 表示落在窗口内，本次不该发
    fn claim(&self, key: &str, now_ms: i64, window_seconds: i64) -> bool {
        let mut inner = self.inner.lock().expect("外发状态锁不应中毒");
        if let Some(last) = inner.throttle.get(key) {
            if now_ms - *last < window_seconds * 1000 {
                return false;
            }
        }
        inner.throttle.insert(key.to_string(), now_ms);
        if inner.throttle.len() > THROTTLE_KEYS_LIMIT {
            // 顺手收个上界：一天前的登记已经没有意义
            inner.throttle.retain(|_, at| now_ms - *at <= 86_400_000);
        }
        true
    }

    /// 撤掉预登记。只撤「还是本次登记」的那一条——按时刻比对，
    /// 否则会把同一键上别人刚登记的时间戳抹掉
    fn release(&self, key: &str, at_ms: i64) {
        let mut inner = self.inner.lock().expect("外发状态锁不应中毒");
        if inner.throttle.get(key) == Some(&at_ms) {
            inner.throttle.remove(key);
        }
    }

    fn record(&self, attempt: Attempt) {
        let mut inner = self.inner.lock().expect("外发状态锁不应中毒");
        inner.history.push_front(attempt);
        while inner.history.len() > HISTORY_LIMIT {
            inner.history.pop_back();
        }
    }

    /// 最近若干次外发结果（有界，新的在前）
    pub fn recent(&self) -> Vec<Attempt> {
        self.inner
            .lock()
            .expect("外发状态锁不应中毒")
            .history
            .iter()
            .cloned()
            .collect()
    }

    /// 给界面看的最近若干次（句子已拼好）
    pub fn recent_view(&self) -> Vec<Recent> {
        self.recent()
            .iter()
            .map(|attempt| Recent {
                at_ms: attempt.at_ms,
                title: attempt.title.clone(),
                text: attempt.short_text(),
                accepted: attempt.outcome.is_accepted(),
                delivery: attempt.outcome.delivery_state(),
                delivery_label: attempt.outcome.delivery_state().label(),
            })
            .collect()
    }

    /// 通道故障升级态：**连续临时失败达到阈值**的通道种类（NOT-05「故障升级」）。
    ///
    /// 升级后的行为：该通道的自动外发**不再重试**（不循环），直到一次成功把它复位。
    /// 限流（`Throttled`）与对端明确拒绝（`permanent`）**不计入**——它们不是链路故障，
    /// 把「配置就是错的」升级成「通道在抖」会把人引向错误的排查方向。
    pub fn escalated_channels(&self) -> Vec<String> {
        let inner = self.inner.lock().expect("外发状态锁不应中毒");
        inner
            .escalated
            .iter()
            .filter(|(_, on)| **on)
            .map(|(channel, _)| channel.clone())
            .collect()
    }

    /// 当前握着的节流占位数（诊断用：正常应当只有「刚发出去还没过窗口」的那些）
    pub fn throttle_keys(&self) -> usize {
        self.inner
            .lock()
            .expect("外发状态锁不应中毒")
            .throttle
            .len()
    }
}

/// 用例默认的密钥源：**永远是「没有密钥」**。
/// 存在的意义是让用例绝不碰真钥匙串——真钥匙串在 ad-hoc 签名下可能弹授权框。
#[cfg(test)]
struct EmptySecrets;

#[cfg(test)]
impl SecretStore for EmptySecrets {
    fn read(&self, _name: &str) -> Option<String> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote::{EventKind, Policy};
    use std::sync::atomic::{AtomicUsize, Ordering};
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

    fn run(notifier: &mut Notifier, kind: EventKind, policy: &Policy, at: i64) -> Outcome {
        notifier.attempt(
            &inputs(kind),
            policy,
            Channel::Ntfy,
            &ntfy(),
            now(at),
            &unavailable(),
            false,
        )
    }

    /// NOT-01「两段 p95」的外发段：**只有真的往网络上试过的尝试**才计时。
    /// 闸门挡下（开关关、静默、在场）没有外发，不该有样本——否则统计把
    /// 「没发」算成「秒发」，p95 变成给没发生的事贴数字。
    #[test]
    fn outbound_latency_samples_only_network_attempts() {
        // ① 开关关：被挡下，零样本
        let (mut notifier, _seen) = fixture(Outcome::Accepted);
        let disabled = Policy::default();
        let blocked = run(&mut notifier, EventKind::Attention, &disabled, 1_000);
        assert!(matches!(blocked, Outcome::Suppressed { .. }));
        let summary = notifier.outbound_latency_summary();
        assert_eq!(summary.samples, 0, "被闸门挡下的尝试不得计时");
        assert_eq!(summary.discarded, 0);

        // ② 开关开、传输成功：一条样本，非负
        let outcome = run(&mut notifier, EventKind::Attention, &enabled(), 2_000);
        assert!(outcome.is_accepted());
        let summary = notifier.outbound_latency_summary();
        assert_eq!(summary.samples, 1, "真的外发过必须计时");
        assert_eq!(summary.discarded, 0);
        let p95 = summary.p95_ms.expect("一条有效样本就有 p95");
        assert!(p95 >= 0, "耗时非负（真实墙钟对事件时刻的差）");
        // 判定要等 20 条样本——两条之内不得提前达标/未达标
        assert!(!summary.verdict_available, "样本不足时不得判定");

        // ③ 失败的外发同样计时：多久知道发不出去也是链路健康度
        let (mut failing, _seen) = fixture(Outcome::Failed {
            permanent: false,
            reason: "timeout".into(),
        });
        let failed = run(&mut failing, EventKind::Attention, &enabled(), 3_000);
        assert!(matches!(failed, Outcome::Failed { .. }));
        assert_eq!(
            failing.outbound_latency_summary().samples,
            1,
            "失败尝试必须计时"
        );
    }

    #[test]
    fn the_master_switch_blocks_everything_including_bypass() {
        // 总开关是这个功能的隐私闸门：若按一次测试就能出本机，开关本身就不可信
        let (mut notifier, seen) = fixture(Outcome::Accepted);
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
        let (mut notifier, seen) = fixture(Outcome::Accepted);
        let policy = Policy {
            send_attention: false,
            ..enabled()
        };
        let outcome = notifier.attempt(
            &inputs(EventKind::Attention),
            &policy,
            Channel::Ntfy,
            &ntfy(),
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
        let (mut notifier, seen) = fixture(Outcome::Accepted);
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
            at,
            &PresenceSignals {
                idle_seconds: Some(1.0),
                ..PresenceSignals::default()
            },
            false,
        );
        assert_eq!(
            quiet,
            Outcome::Suppressed {
                reason: "静默时段".into()
            }
        );

        // bypass 绕过这两条 ⇒ 这次要真的走传输
        let bypassed = notifier.attempt(
            &inputs(EventKind::Attention),
            &policy,
            Channel::Ntfy,
            &ntfy(),
            at,
            &PresenceSignals {
                idle_seconds: Some(1.0),
                ..PresenceSignals::default()
            },
            true,
        );
        assert_eq!(bypassed, Outcome::Accepted);
        assert_eq!(seen.lock().unwrap().len(), 1);

        // 但配置检查不在绕过范围内：测试按钮也发不出一条没配好的通道
        let (mut second, _) = fixture(Outcome::Accepted);
        let unconfigured = second.attempt(
            &inputs(EventKind::Attention),
            &policy,
            Channel::Ntfy,
            &ChannelConfig::default(),
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
        let (mut notifier, _) = fixture(Outcome::Accepted);
        let present = PresenceSignals {
            idle_seconds: Some(3.0), // 刚动过键盘
            ..PresenceSignals::default()
        };
        let by_default = notifier.attempt(
            &inputs(EventKind::Attention),
            &enabled(),
            Channel::Ntfy,
            &ntfy(),
            now(1_000),
            &present,
            false,
        );
        assert_eq!(by_default, Outcome::Accepted, "默认不要求「人不在」");

        let (mut strict, _) = fixture(Outcome::Accepted);
        let policy = Policy {
            only_when_away: true,
            ..enabled()
        };
        let blocked = strict.attempt(
            &inputs(EventKind::Attention),
            &policy,
            Channel::Ntfy,
            &ntfy(),
            now(1_000),
            &present,
            false,
        );
        match blocked {
            Outcome::Suppressed { reason } => {
                assert!(reason.starts_with("有人在机器前（"), "{reason}");
                assert!(
                    reason.contains("距上次输入 3 秒"),
                    "要说得清为什么被挡：{reason}"
                );
            }
            other => panic!("应被挡下，实际 {other:?}"),
        }
    }

    /// 配置检查必须**排在节流之前**：配置坏了是每次都发不出去，
    /// 若先判节流，用户会看到「节流命中」而实际是根本没配好——
    /// 于是他会去等窗口，而不是去补主题名。
    #[test]
    fn a_broken_config_beats_the_throttle_in_the_reported_reason() {
        let (mut notifier, _) = fixture(Outcome::Accepted);
        let policy = enabled();
        // 先配好、发成功一次 ⇒ 节流窗口被占住
        assert_eq!(
            run(&mut notifier, EventKind::Attention, &policy, 1_000),
            Outcome::Accepted
        );
        // 同一时刻把配置改坏再试：必须报配置问题，不许报「节流命中」
        let outcome = notifier.attempt(
            &inputs(EventKind::Attention),
            &policy,
            Channel::Ntfy,
            &ChannelConfig::default(),
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
        let (mut notifier, seen) = fixture(Outcome::Accepted);
        let policy = enabled(); // 节流 90 秒
        assert_eq!(
            run(&mut notifier, EventKind::Attention, &policy, 1_000),
            Outcome::Accepted
        );
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
            Outcome::Accepted,
            "窗口一到就放行"
        );

        // 不同事件类型各有各的窗口
        assert_eq!(
            run(&mut notifier, EventKind::CostSpike, &policy, 1_000 + 90_000),
            Outcome::Accepted
        );

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
        let (mut notifier, _) = fixture(Outcome::Accepted);
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
                now(1_000 + i),
                &unavailable(),
                false,
            );
        }
        let recent = notifier.recent();
        assert_eq!(recent.len(), HISTORY_LIMIT);
        assert_eq!(recent[0].at_ms, 1_024, "新的在前");
        assert_eq!(recent[0].short_text(), "平台已受理");
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
            outcome: Outcome::Accepted,
            tries: 3,
        };
        assert_eq!(retried.short_text(), "平台已受理（重试 2 次后）");
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
        // 1 号端口（特权端口，测试不会绑它）：确定性拒绝连接。
        // 「绑定后释放」那一招在并行测试下会被别的用例抢走端口，不可用。
        let config = ChannelConfig {
            url_template: "http://127.0.0.1:1/notify".into(),
            body_template: "t={title}".into(),
            ..ChannelConfig::default()
        };
        // 关掉重试：这条用例测的是「真传输真的去连了、并如实进账本」，
        // 不该为了它等 5 秒重试（重试本身另有专门用例）
        let notifier = Notifier::new().with_retry(0, 0);
        let outcome = notifier.attempt(
            &inputs(EventKind::Attention),
            &enabled(),
            Channel::CustomHttp,
            &config,
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
        assert!(!recent[0].accepted);
    }

    /// 记录密钥读取次数的假密钥源：用来钉住「总开关关着时不碰钥匙串」。
    struct CountingSecrets {
        value: Option<String>,
        reads: Arc<AtomicUsize>,
    }

    impl SecretStore for CountingSecrets {
        fn read(&self, _name: &str) -> Option<String> {
            self.reads.fetch_add(1, Ordering::SeqCst);
            self.value.clone()
        }
    }

    fn counting_secrets(value: Option<&str>) -> (Arc<dyn SecretStore>, Arc<AtomicUsize>) {
        let reads = Arc::new(AtomicUsize::new(0));
        let store = CountingSecrets {
            value: value.map(str::to_string),
            reads: reads.clone(),
        };
        (Arc::new(store), reads)
    }

    fn custom_config() -> ChannelConfig {
        ChannelConfig {
            url_template: "https://x/y?key={key}".into(),
            body_template: "t={title}".into(),
            ..ChannelConfig::default()
        }
    }

    #[test]
    fn a_key_placeholder_without_a_stored_secret_is_never_sent() {
        // 模板要密钥、钥匙串里却没有 ⇒ 在**配置检查**那一步就被挡下。
        // 旧行为是「留空照样发」——那等于拿一个空密钥去撞对端，还让用户以为发成功了
        let seen = Arc::new(Mutex::new(Vec::new()));
        let transport = FakeTransport {
            outcome: Outcome::Accepted,
            seen: seen.clone(),
        };
        let mut notifier = Notifier::with_transport(Box::new(transport));
        let outcome = notifier.attempt(
            &inputs(EventKind::Attention),
            &enabled(),
            Channel::CustomHttp,
            &custom_config(),
            now(1_000),
            &unavailable(),
            false,
        );
        assert!(
            matches!(outcome, Outcome::NotConfigured { .. }),
            "缺密钥应报未配置，实际 {outcome:?}"
        );
        assert!(
            seen.lock().unwrap().is_empty(),
            "被挡下的请求一个字节都不该出去"
        );
    }

    #[test]
    fn a_stored_secret_reaches_the_request_and_nowhere_else() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let transport = FakeTransport {
            outcome: Outcome::Accepted,
            seen: seen.clone(),
        };
        let (store, reads) = counting_secrets(Some("s3cret-value-9"));
        let _ = &reads;
        let mut notifier = Notifier::with_transport(Box::new(transport)).with_secrets(store);
        let outcome = notifier.attempt(
            &inputs(EventKind::Attention),
            &enabled(),
            Channel::CustomHttp,
            &custom_config(),
            now(1_000),
            &unavailable(),
            false,
        );
        assert_eq!(outcome, Outcome::Accepted);
        // ① 请求里带的是真密钥（这是它唯一该出现的地方）
        let request = seen.lock().unwrap()[0].clone();
        assert!(request.url.contains("s3cret-value-9"), "{}", request.url);
        // ② 账本与给界面看的文案里不许出现它
        let recent = notifier.recent_view();
        assert_eq!(recent.len(), 1);
        assert!(
            !recent[0].text.contains("s3cret-value-9")
                && !recent[0].title.contains("s3cret-value-9"),
            "账本里不许出现密钥：{:?}",
            recent[0]
        );
        // ③ 掩码预览里也不许出现
        let masked = crate::render::render_request(
            &crate::render::render(&inputs(EventKind::Attention), &custom_config()),
            Channel::CustomHttp,
            &custom_config(),
            Some("s3cret-value-9"),
            true,
        );
        assert!(
            !masked.url.contains("s3cret-value-9"),
            "掩码预览不许带真密钥：{}",
            masked.url
        );
    }

    #[test]
    fn the_ledger_never_contains_the_secret_even_when_the_send_fails() {
        // 失败文案最容易夹带请求上下文，所以单独钉一条
        let seen = Arc::new(Mutex::new(Vec::new()));
        let transport = FakeTransport {
            outcome: Outcome::Failed {
                reason: "服务端返回 401".into(),
                permanent: true,
            },
            seen: seen.clone(),
        };
        let (store, _) = counting_secrets(Some("s3cret-value-9"));
        let mut notifier = Notifier::with_transport(Box::new(transport)).with_secrets(store);
        let outcome = notifier.attempt(
            &inputs(EventKind::Attention),
            &enabled(),
            Channel::CustomHttp,
            &custom_config(),
            now(1_000),
            &unavailable(),
            false,
        );
        assert!(matches!(outcome, Outcome::Failed { .. }));
        let rendered = format!("{outcome:?}");
        assert!(!rendered.contains("s3cret-value-9"), "{rendered}");
        let recent = notifier.recent_view();
        assert!(
            !recent[0].text.contains("s3cret-value-9"),
            "{}",
            recent[0].text
        );
    }

    #[test]
    fn the_keychain_is_not_read_while_the_master_switch_is_off() {
        // 总开关是这个功能的隐私闸门：关着的时候，连「有没有密钥」都不该去问钥匙串。
        // 这条守的是「关掉开关就不会有任何东西离开本机」这句话的字面含义。
        let seen = Arc::new(Mutex::new(Vec::new()));
        let transport = FakeTransport {
            outcome: Outcome::Accepted,
            seen: seen.clone(),
        };
        let (store, reads) = counting_secrets(Some("s3cret-value-9"));
        let mut notifier = Notifier::with_transport(Box::new(transport)).with_secrets(store);
        let mut policy = enabled();
        policy.master_enabled = false;
        let outcome = notifier.attempt(
            &inputs(EventKind::Attention),
            &policy,
            Channel::CustomHttp,
            &custom_config(),
            now(1_000),
            &unavailable(),
            false,
        );
        assert_eq!(
            outcome,
            Outcome::Suppressed {
                reason: "总开关未开".into()
            }
        );
        assert_eq!(
            reads.load(Ordering::SeqCst),
            0,
            "总开关关着时不该去读钥匙串：闸门顺序就是这条口径"
        );
        assert!(seen.lock().unwrap().is_empty());
    }

    // ── NOT-05：受理≠收到≠已读 / Retry-After / 故障升级不循环 ────────────

    /// 按脚本返回 outcome 的传输：升级测试需要「失败三次再成功」这类序列，
    /// 固定 outcome 的 FakeTransport 给不了。
    struct ScriptedTransport {
        script: Mutex<VecDeque<Outcome>>,
        seen: Arc<Mutex<Vec<Request>>>,
    }

    impl Transport for ScriptedTransport {
        fn perform(&self, request: &Request) -> Outcome {
            self.seen.lock().unwrap().push(request.clone());
            let mut script = self.script.lock().expect("脚本锁不应中毒");
            script.pop_front().unwrap_or_else(|| Outcome::Accepted)
        }
    }

    fn scripted(
        script: Vec<Outcome>,
        retry_attempts: i64,
    ) -> (Notifier, Arc<Mutex<Vec<Request>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let transport = ScriptedTransport {
            script: Mutex::new(script.into()),
            seen: seen.clone(),
        };
        (
            Notifier::with_transport(Box::new(transport)).with_retry(retry_attempts, 0),
            seen,
        )
    }

    fn transient() -> Outcome {
        Outcome::Failed {
            reason: "连接失败".into(),
            permanent: false,
        }
    }

    /// 三次**连续**临时失败 → 通道升级为故障（NOT-05「故障升级」）。
    #[test]
    fn three_consecutive_transient_failures_escalate_the_channel() {
        let (mut notifier, _seen) = scripted(
            vec![transient(), transient(), transient(), transient()],
            0,
        );
        let policy = enabled();
        assert!(
            notifier.escalated_channels().is_empty(),
            "起始不该有升级态"
        );
        // 事件之间跨过默认节流窗口（90 秒）→ 用 100_000ms 间距。
        // 注意：失败路径本来就会撤掉节流占位（`if !outcome.is_accepted()` 里的 release），
        // 所以这个间距在**失败**用例里是冗余的；但升级用例的脚本里混有成功态，
        // 保持同一间距可以让序列语义不依赖「哪一步是否撤了占位」这个实现细节。
        for index in 0..3 {
            run(
                &mut notifier,
                EventKind::Attention,
                &policy,
                1_000 + index * 100_000,
            );
            if index < 2 {
                assert!(
                    notifier.escalated_channels().is_empty(),
                    "第 {} 次失败后不该升级（阈值是 3）",
                    index + 1
                );
            }
        }
        assert_eq!(notifier.escalated_channels(), vec!["ntfy".to_string()]);
        // 升级态不吞结果：第 4 次失败仍如实返回、升级态保持
        let last = run(&mut notifier, EventKind::Attention, &policy, 301_000);
        assert!(matches!(last, Outcome::Failed { permanent: false, .. }));
        assert_eq!(notifier.escalated_channels(), vec!["ntfy".to_string()]);
    }

    /// 通道恢复（一次受理成功）必须复位升级，否则用户会被永久卡在「故障」里。
    #[test]
    fn a_success_resets_the_escalation() {
        let (mut notifier, _seen) = scripted(
            vec![
                transient(),
                transient(),
                transient(),
                Outcome::Accepted,
            ],
            0,
        );
        let policy = enabled();
        for index in 0..3 {
            run(
                &mut notifier,
                EventKind::Attention,
                &policy,
                1_000 + index * 100_000,
            );
        }
        assert_eq!(notifier.escalated_channels(), vec!["ntfy".to_string()]);
        let outcome = run(&mut notifier, EventKind::Attention, &policy, 301_000);
        assert!(outcome.is_accepted());
        assert!(
            notifier.escalated_channels().is_empty(),
            "成功后必须复位升级"
        );
    }

    /// 对端明确拒绝（配置错）与限流都不是链路故障，**不计升级**——
    /// 否则「配置就是错的」会被显示成「通道在抖」，把人引向错误的排查方向。
    #[test]
    fn permanent_rejection_and_throttling_never_escalate() {
        let (mut notifier, _seen) = scripted(
            vec![
                Outcome::Failed {
                    reason: "404 主题不存在".into(),
                    permanent: true,
                },
                Outcome::Failed {
                    reason: "404 主题不存在".into(),
                    permanent: true,
                },
                Outcome::Failed {
                    reason: "404 主题不存在".into(),
                    permanent: true,
                },
            ],
            0,
        );
        let policy = enabled();
        for index in 0..3 {
            run(
                &mut notifier,
                EventKind::Attention,
                &policy,
                1_000 + index * 100_000,
            );
        }
        assert!(notifier.escalated_channels().is_empty(), "永久失败不该升级");

        let throttled = || Outcome::Throttled {
            reason: "服务端返回 429".into(),
            retry_after_ms: Some(30_000),
        };
        let (mut notifier, _seen) = scripted(vec![throttled(), throttled(), throttled()], 0);
        for index in 0..3 {
            run(
                &mut notifier,
                EventKind::Attention,
                &policy,
                1_000 + index * 100_000,
            );
        }
        assert!(notifier.escalated_channels().is_empty(), "限流不该升级");
    }

    /// **渠道独立**（NOT-05 矩阵明列场景）：升级态与连击按通道分键，
    /// 一个通道连续失败升级后，另一个通道**不受牵连**。
    ///
    /// 此前所有 notifier 测试都只用 ntfy（`run()` 硬编码 `Channel::Ntfy`），
    /// 于是「渠道独立」在矩阵里是一个**镜面声称**、没有断言兜着。这条补上。
    #[test]
    fn escalation_is_tracked_per_channel_not_globally() {
        // 脚本**显式**给出第 4 个结果（customHTTP 那次为 Accepted）。
        // 不能靠脚本耗尽后的兜底默认值：`ScriptedTransport` 用光脚本时返回 `Accepted`，
        // 那样这条断言就变成「兜底值恰好是 Accepted」的巧合，测不出被提前挡下的情况。
        let (mut notifier, seen) = scripted(
            vec![transient(), transient(), transient(), Outcome::Accepted],
            0,
        );
        let policy = enabled();
        let custom = ChannelConfig {
            url_template: "https://example.com/hook".into(),
            body_template: "title={title}&desp={body}".into(),
            ..ChannelConfig::default()
        };
        // ntfy 上连打三次临时失败 → 只有 ntfy 升级
        for index in 0..3 {
            notifier.attempt(
                &inputs(EventKind::Attention),
                &policy,
                Channel::Ntfy,
                &ntfy(),
                now(1_000 + index * 100_000),
                &unavailable(),
                false,
            );
        }
        assert_eq!(
            notifier.escalated_channels(),
            vec!["ntfy".to_string()],
            "ntfy 连续三次失败后该升级"
        );
        assert!(
            !notifier.is_escalated(Channel::CustomHttp),
            "ntfy 的故障不得牵连 customHTTP——每个通道各有各的状态"
        );

        // **同一个 Notifier 实例**换成另一个通道再发一次——这才是「渠道独立」的核查点。
        //
        // ⚠️ 语义要点（别读错）：升级态**只阻止「重试」**（`attempt` 里
        // `self.retry_attempts > 0 && !self.is_escalated(channel)`），
        // **不阻止发送本身**。所以这里断言的不是「换通道还能发」（那本来就成立），
        // 而是两件与通道分键直接相关的事：
        //   ① customHTTP 这次真的走到了传输层（没被 ntfy 的状态误伤）；
        //   ② customHTTP 成功**不会复位 ntfy 的升级态**（状态互不干扰）。
        // 若把状态键做成全局（变异实测过），② 会立刻挂。
        let outcome = notifier.attempt(
            &inputs(EventKind::Attention),
            &policy,
            Channel::CustomHttp,
            &custom,
            now(301_000),
            &unavailable(),
            false,
        );
        assert!(
            matches!(outcome, Outcome::Accepted),
            "customHTTP 该真的发出并成功，实际：{outcome:?}"
        );
        assert_eq!(
            notifier.escalated_channels(),
            vec!["ntfy".to_string()],
            "customHTTP 成功不得复位 ntfy 的升级态——两个通道的状态互不干扰"
        );
        // 四次尝试都真的走到了传输层：三次 ntfy 失败 + 一次 customHTTP
        assert_eq!(
            seen.lock().unwrap().len(),
            4,
            "customHTTP 那次必须真的发出（被提前挡下就不会进 seen）"
        );
    }

    /// 升级态**只阻止重试、不阻止发送**——这个边界必须有断言兜着，
    /// 否则「升级后停止重试」这句文档声称就只是自述。
    #[test]
    fn escalation_stops_retrying_but_never_blocks_the_send_itself() {
        // retry_attempts = 1：未升级时一次失败会重试一次。
        // 脚本要给足——用光后 `ScriptedTransport` 会兜底返回 `Accepted`，
        // 那样第三次 attempt 会「成功」并把连击清零，测出来的不是升级行为。
        // 3 次 attempt × 每次 2 发（初次 + 重试）= 6 个；第 7 个给「升级后只发一次」那轮。
        let (mut notifier, seen) = scripted(
            vec![
                transient(), transient(), // #1 初次 + 重试
                transient(), transient(), // #2 初次 + 重试
                transient(), transient(), // #3 初次 + 重试 → 升级
                transient(),              // #4 升级后：只发一次、不重试
            ],
            1,
        );
        let policy = enabled();
        for index in 0..3 {
            notifier.attempt(
                &inputs(EventKind::Attention),
                &policy,
                Channel::Ntfy,
                &ntfy(),
                now(1_000 + index * 100_000),
                &unavailable(),
                false,
            );
        }
        assert!(notifier.is_escalated(Channel::Ntfy), "三次连续失败该升级");
        let before = seen.lock().unwrap().len();

        // 升级后再发一次：**发送本身必须照常进行**（进 seen），
        // 但**不再重试**——所以这次只 +1 而不是 +2。
        let outcome = notifier.attempt(
            &inputs(EventKind::Attention),
            &policy,
            Channel::Ntfy,
            &ntfy(),
            now(401_000),
            &unavailable(),
            false,
        );
        assert!(
            matches!(outcome, Outcome::Failed { permanent: false, .. }),
            "升级不吞结果：仍如实返回失败，实际：{outcome:?}"
        );
        assert_eq!(
            seen.lock().unwrap().len() - before,
            1,
            "升级后这次只发一次、不重试（未升级时同样的失败会发两次）"
        );
    }

    /// 交错序列的口径：**非链路故障的失败会把连击清零**，所以
    /// 「抖动、抖动、配置错、抖动、抖动」不该升级——那 5 次里从没有过连续 3 次抖动。
    /// 被策略挡下（Suppressed）不走这条路径（闸门早退，一次都没试网络），
    /// 因此它不会打断连击。两种行为都要钉住，否则「不计入」会被读成两种意思。
    #[test]
    fn interleaved_other_outcomes_reset_or_hold_the_streak_as_documented() {
        let permanent = || Outcome::Failed {
            reason: "404 主题不存在".into(),
            permanent: true,
        };
        let (mut notifier, _seen) = scripted(
            vec![
                transient(),
                transient(),
                permanent(),
                transient(),
                transient(),
            ],
            0,
        );
        let policy = enabled();
        for index in 0..5 {
            run(
                &mut notifier,
                EventKind::Attention,
                &policy,
                1_000 + index * 100_000,
            );
        }
        assert!(
            notifier.escalated_channels().is_empty(),
            "配置错把连击清零后，后面只剩两次抖动，不该升级"
        );

        // 被策略挡下：闸门早退、不碰传输，连击保留 ⇒ 两次抖动 + 一次被挡 + 一次抖动 = 连续 3 次
        let (mut notifier, _seen) = scripted(
            vec![transient(), transient(), transient()],
            0,
        );
        let blocked_policy = Policy {
            send_attention: false,
            ..enabled()
        };
        run(&mut notifier, EventKind::Attention, &policy, 1_000);
        run(&mut notifier, EventKind::Attention, &policy, 101_000);
        // 第三次被策略挡下（不消耗脚本，也不清零）
        run(&mut notifier, EventKind::Attention, &blocked_policy, 201_000);
        run(&mut notifier, EventKind::Attention, &policy, 301_000);
        assert_eq!(
            notifier.escalated_channels(),
            vec!["ntfy".to_string()],
            "被挡下不是链路结果，连击应保留，第三次抖动升级"
        );
    }

    /// `NotConfigured`（配置缺失早退）与 `Suppressed`（策略挡下）同属
    /// 「一次都没试网络」，连击**保留**——第二轮复审用变异证明这条语义
    /// 此前只有 `Suppressed` 的交错用例兜着（让 `NotConfigured` 清零不会有任何测试响）。
    /// 这条补上：抖动、抖动、配置缺、抖动 → 应升级（若 `NotConfigured` 清零则不会）。
    #[test]
    fn not_configured_keeps_the_streak_unlike_a_real_outcome() {
        let (mut notifier, seen) = scripted(vec![transient(), transient(), transient()], 0);
        let policy = enabled();
        let broken = ChannelConfig {
            topic_or_url: "   ".into(), // 空白主题 → missing_field 命中，闸门早退
            ..ChannelConfig::default()
        };
        for index in 0..4 {
            let (channel, config) = if index == 2 {
                (Channel::Ntfy, &broken) // 第三次：配置缺失，不进传输层
            } else {
                (Channel::Ntfy, &ntfy())
            };
            notifier.attempt(
                &inputs(EventKind::Attention),
                &policy,
                channel,
                config,
                now(1_000 + index * 100_000),
                &unavailable(),
                false,
            );
        }
        assert_eq!(
            notifier.escalated_channels(),
            vec!["ntfy".to_string()],
            "配置缺失不是链路结果、不该清零连击；第 4 次抖动（连击第 3 次）应升级"
        );
        assert_eq!(
            seen.lock().unwrap().len(),
            3,
            "配置缺失那次不进传输层，只有 3 次真实尝试"
        );
    }

    /// 已升级的通道**不再重试**（NOT-05「故障升级不循环」）：
    /// 再试一次不会让抖动的链路变好，只会把失败风暴堆在用户屏幕上。
    #[test]
    fn escalation_stops_the_retry_loop() {
        // 每次重试会再走一次传输，所以脚本每个未升级的 attempt 要消耗 2 个元素
        let (mut notifier, seen) = scripted(
            vec![
                transient(),
                transient(),
                transient(),
                transient(),
                transient(),
                transient(),
                transient(),
            ],
            1, // 打开重试
        );
        let policy = enabled();
        for index in 0..3 {
            run(
                &mut notifier,
                EventKind::Attention,
                &policy,
                1_000 + index * 100_000,
            );
        }
        assert_eq!(notifier.escalated_channels(), vec!["ntfy".to_string()]);
        let calls_before = seen.lock().unwrap().len();
        run(&mut notifier, EventKind::Attention, &policy, 301_000);
        let calls_after = seen.lock().unwrap().len();
        assert_eq!(
            calls_after - calls_before,
            1,
            "升级后一次尝试就该停，重试循环必须停住"
        );
        let recent = notifier.recent();
        assert_eq!(recent[0].tries, 1, "升级后不该再有重试次数");
    }

    /// 限流把 Retry-After 带到界面文案，并如实落在「未确认」交付态。
    #[test]
    fn throttled_shows_the_wait_time_and_stays_unknown() {
        let (mut notifier, _seen) = scripted(
            vec![Outcome::Throttled {
                reason: "服务端返回 429".into(),
                retry_after_ms: Some(12_000),
            }],
            0,
        );
        run(&mut notifier, EventKind::Attention, &enabled(), 1_000);
        let recent = notifier.recent_view();
        assert!(
            recent[0].text.contains("12 秒后可重试"),
            "界面必须显示对端给的等待时长：{}",
            recent[0].text
        );
        assert_eq!(recent[0].delivery, DeliveryState::Unknown);
        assert!(!recent[0].accepted);
        assert_eq!(recent[0].delivery_label, "未确认");
    }

    /// 受理 ≠ 收到 ≠ 已读（NOT-05）：受理成功显示「已受理，未确认送达」，
    /// 不显示「已送达」；失败/被挡一律「未确认」，不冒充任何一段。
    #[test]
    fn delivery_state_distinguishes_accepted_from_unknown() {
        let (mut notifier, _seen) = scripted(vec![Outcome::Accepted], 0);
        run(&mut notifier, EventKind::Attention, &enabled(), 1_000);
        let recent = notifier.recent_view();
        assert_eq!(recent[0].delivery, DeliveryState::Accepted);
        assert!(recent[0].accepted);
        assert_eq!(recent[0].text, "平台已受理");
        // 界面文案由 Rust 下发，且**不得说成「已送达」**
        assert_eq!(recent[0].delivery_label, "已受理，未确认送达");
        assert_ne!(recent[0].delivery_label, "已送达");

        let (mut notifier, _seen) = scripted(vec![transient()], 0);
        run(&mut notifier, EventKind::Attention, &enabled(), 1_000);
        let recent = notifier.recent_view();
        assert_eq!(
            recent[0].delivery,
            DeliveryState::Unknown,
            "失败不能冒充受理/送达/已读"
        );
        assert!(!recent[0].accepted);
        assert_eq!(recent[0].delivery_label, "未确认");
    }
}
