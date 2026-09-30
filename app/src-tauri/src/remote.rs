//! 外发策略层：该不该发、配齐了没有、静默时段、人在不在。
//!
//! 对齐 Swift `RemoteNotification.swift` 的**纯函数部分**（v0.0.x 起）与
//! `RemoteNotifyStore.swift` 的读取口径。本轮**不含**钥匙串读写
//! （Swift `RemoteSecret.write/read/exists/delete`，要 `security-framework`，是独立决定）
//! 与传输层（HTTP / SMTP）。
//!
//! 红线（ADR 0009）：本项目不持有非本机既有登录态的密钥、不主动打厂商端点。
//! 通道凭据一律**用户自填**、只用于该通道；本模块只处理**非密钥**字段，
//! 密钥的占位形式（`{key}`）只做「填对了吗」的检查，值永远不经过这里。

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};

/// 通道种类
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Channel {
    #[serde(rename = "ntfy")]
    Ntfy,
    #[serde(rename = "customHTTP")]
    CustomHttp,
    #[serde(rename = "smtpEmail")]
    SmtpEmail,
}

impl Channel {
    pub fn as_str(self) -> &'static str {
        match self {
            Channel::Ntfy => "ntfy",
            Channel::CustomHttp => "customHTTP",
            Channel::SmtpEmail => "smtpEmail",
        }
    }

    pub fn parse(raw: &str) -> Option<Channel> {
        match raw {
            "ntfy" => Some(Channel::Ntfy),
            "customHTTP" => Some(Channel::CustomHttp),
            "smtpEmail" => Some(Channel::SmtpEmail),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Channel::Ntfy => "ntfy 推送",
            Channel::CustomHttp => "自定义 HTTP（微信 Server酱 / PushPlus / 企微 / 钉钉…）",
            Channel::SmtpEmail => "邮箱（SMTP）",
        }
    }

    /// 该通道的密钥在钥匙串里的固定条目名。
    /// 刻意不做成可配字段：「密钥存了但条目名对不上」是查不出来的失效——
    /// 界面全绿、每次发送都失败。条目名由通道种类唯一决定。
    pub fn default_secret_name(self) -> String {
        // 单一来源：钥匙串那一侧也用同一个函数，两处各写一次必然漂移
        crate::secret::default_secret_name(self)
    }

    /// 端点是否明文过网。与「配齐」检查**分开**的独立警告：
    /// 配齐检查里混进安全提示会让两条都变模糊。
    pub fn insecure_endpoint(self, config: &ChannelConfig) -> Option<&'static str> {
        let url = match self {
            Channel::Ntfy => &config.topic_or_url,
            // 只支持 465，一定是 TLS
            Channel::SmtpEmail => return None,
            Channel::CustomHttp => &config.url_template,
        };
        url.to_lowercase().starts_with("http://").then_some(
            "地址是明文 http://：密钥与通知内容会明文经过路径上的每一跳，建议换成 https://",
        )
    }

    /// 地址里像是**直接粘了凭据**（没有 `{key}` 占位，却含 `key=` / `token=` / `.send` / 高熵片段）。
    /// 这种填法会让凭据明文落盘、并显示在设置页与预览里；掩码只能保证「看得见的那份」不泄漏，
    /// 落盘那份救不回来，所以要说破。
    pub fn plaintext_secret_in_template(self, config: &ChannelConfig) -> Option<&'static str> {
        let url = if self == Channel::Ntfy {
            &config.topic_or_url
        } else {
            &config.url_template
        };
        if url.is_empty() || contains_placeholder(url) {
            return None;
        }
        let lowered = url.to_lowercase();
        let suspicious = lowered.contains("key=")
            || lowered.contains("token=")
            || lowered.contains(".send")
            || url.split('/').any(looks_like_token);
        suspicious.then_some(
            "地址里像是直接粘了密钥：它会明文存进配置并显示在预览里。请把密钥那段改成 {key}，值放钥匙串",
        )
    }

    /// 该通道是否已经配齐到「可以试发」。
    /// `has_secret` 由调用方给（发送路径用注入的读取器、设置页用钥匙串存在性查询），
    /// 判据本身保持纯函数：它每次外发都会被调，不该在里面碰钥匙串。
    pub fn missing_field(self, config: &ChannelConfig, has_secret: bool) -> Option<&'static str> {
        match self {
            Channel::Ntfy => {
                let value = config.topic_or_url.trim();
                if value.is_empty() {
                    return Some("缺主题名或服务器地址");
                }
                // 判定只写「以 http 开头」的话，`ntfy.mine.local/island`（少贴了协议头）
                // 会被拼成 https://ntfy.sh/ntfy.mine.local/island——内容跑到一台用户
                // 没选过的公网服务器上，主题名还是他的内网主机名
                if value.contains('/') && !value.to_lowercase().contains("://") {
                    return Some("像是要自建服务器的地址，但少了 http:// 或 https:// 前缀");
                }
                if value.contains(' ') || !value.is_ascii() {
                    return Some("主题名只能是无空格的 ASCII 字符");
                }
                None
            }
            Channel::CustomHttp => {
                if config.url_template.is_empty() {
                    return Some("缺发送地址");
                }
                // 空模板时本可以自己组一套字段名，但那等于把没核实过的字段名
                // （body / content / desp）写成预设——各家中转服务的字段必须用户自己填
                if config.body_template.trim().is_empty() {
                    return Some("缺请求体模板：各家字段名不同，请填写类似 \"title={title}&desp={body}\" 的形状");
                }
                if contains_placeholder(&config.url_template) && !has_secret {
                    return Some("地址里有 {key} 占位，但钥匙串里还没有密钥");
                }
                None
            }
            Channel::SmtpEmail => {
                if config.smtp_host.is_empty() {
                    return Some("缺 SMTP 服务器地址");
                }
                if config.smtp_user.is_empty() {
                    return Some("缺发信账号");
                }
                if config.smtp_to.is_empty() {
                    return Some("缺收件地址");
                }
                // 端口先于密钥检查：非 465 是「这条路根本不走」，缺授权码只是「还差最后一步」
                if config.smtp_port != 465 {
                    return Some("仅支持 465（隐式 TLS）；25/587 的 STARTTLS 不支持");
                }
                if !has_secret {
                    return Some("缺 SMTP 密码/授权码（存钥匙串）");
                }
                None
            }
        }
    }
}

/// 一个通道的**非密钥**配置。密钥（sendkey / token / webhook 里的 key / SMTP 授权码）
/// 不进这个结构：它只存钥匙串条目名的推导规则，值永远不落盘。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ChannelConfig {
    #[serde(rename = "topicOrURL")]
    pub topic_or_url: String,
    pub url_template: String,
    pub body_template: String,
    #[serde(rename = "useJSONBody")]
    pub use_json_body: bool,
    pub smtp_host: String,
    pub smtp_port: i64,
    pub smtp_user: String,
    pub smtp_to: String,
    /// 是否把「最后一条动作」一起送出（默认否：那会把命令内容送出这台机器）
    pub include_action_detail: bool,
}

impl Default for ChannelConfig {
    fn default() -> Self {
        ChannelConfig {
            topic_or_url: String::new(),
            url_template: String::new(),
            body_template: String::new(),
            use_json_body: true,
            smtp_host: String::new(),
            smtp_port: 465,
            smtp_user: String::new(),
            smtp_to: String::new(),
            include_action_detail: false,
        }
    }
}

fn text(map: &Map<String, Value>, key: &str) -> String {
    map.get(key)
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string()
}

fn flag(map: &Map<String, Value>, key: &str, fallback: bool) -> bool {
    map.get(key).and_then(|v| v.as_bool()).unwrap_or(fallback)
}

fn integer(map: &Map<String, Value>, key: &str, fallback: i64) -> i64 {
    map.get(key).and_then(|v| v.as_i64()).unwrap_or(fallback)
}

/// 手写解码：每个字段各自回落默认值（与 Swift 的 `decodeIfPresent` 同口径）。
/// 用 `#[serde(default)]` 做不到这一点——**一个字段类型写错会让整条配置解不开**，
/// 而配置的默认值是「全空 + 关闭」，用户看到的是自己配好的通道凭空清空。
/// 手改 plist / JSON 把端口写成字符串这种笔误，只该作废那一个字段。
impl<'de> Deserialize<'de> for ChannelConfig {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        let map = value
            .as_object()
            .ok_or_else(|| D::Error::custom("通道配置必须是一个对象"))?;
        Ok(ChannelConfig {
            topic_or_url: text(map, "topicOrURL"),
            url_template: text(map, "urlTemplate"),
            body_template: text(map, "bodyTemplate"),
            use_json_body: flag(map, "useJSONBody", true),
            smtp_host: text(map, "smtpHost"),
            smtp_port: integer(map, "smtpPort", 465),
            smtp_user: text(map, "smtpUser"),
            smtp_to: text(map, "smtpTo"),
            include_action_detail: flag(map, "includeActionDetail", false),
        })
    }
}

/// 外发策略：总开关、事件类型过滤、节流、静默时段。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Policy {
    pub master_enabled: bool,
    pub send_completed: bool,
    pub send_attention: bool,
    pub send_cost_spike: bool,
    pub throttle_seconds: i64,
    pub quiet_start: String,
    pub quiet_end: String,
    pub only_when_away: bool,
    pub away_idle_seconds: i64,
}

impl Default for Policy {
    fn default() -> Self {
        Policy {
            master_enabled: false,
            send_completed: true,
            send_attention: true,
            send_cost_spike: true,
            throttle_seconds: 90,
            quiet_start: String::new(),
            quiet_end: String::new(),
            only_when_away: false,
            away_idle_seconds: 120,
        }
    }
}

impl<'de> Deserialize<'de> for Policy {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        let map = value
            .as_object()
            .ok_or_else(|| D::Error::custom("外发策略必须是一个对象"))?;
        Ok(Policy {
            master_enabled: flag(map, "masterEnabled", false),
            send_completed: flag(map, "sendCompleted", true),
            send_attention: flag(map, "sendAttention", true),
            send_cost_spike: flag(map, "sendCostSpike", true),
            throttle_seconds: integer(map, "throttleSeconds", 90),
            quiet_start: text(map, "quietStart"),
            quiet_end: text(map, "quietEnd"),
            only_when_away: flag(map, "onlyWhenAway", false),
            away_idle_seconds: integer(map, "awayIdleSeconds", 120),
        })
    }
}

/// 节流与离开门槛的允许区间。与 Swift `normalizedThrottle` / `normalizedIdle` 同值。
pub const THROTTLE_RANGE: (i64, i64) = (15, 3600);
pub const IDLE_RANGE: (i64, i64) = (30, 3600);

impl Policy {
    /// 读出即归一化：损坏或越界的值可能在落盘时就写进去了，读这条路是唯一防线。
    pub fn normalized(&self) -> Policy {
        let mut copy = self.clone();
        copy.throttle_seconds = copy.throttle_seconds.clamp(THROTTLE_RANGE.0, THROTTLE_RANGE.1);
        copy.away_idle_seconds = copy.away_idle_seconds.clamp(IDLE_RANGE.0, IDLE_RANGE.1);
        // 写坏的静默时段必须退化成「不静默」，而不是整天把通知吞掉——
        // 后者是静默失效，用户永远看不见，正是最难查的那类 bug
        if !is_hhmm(&copy.quiet_start) {
            copy.quiet_start.clear();
        }
        if !is_hhmm(&copy.quiet_end) {
            copy.quiet_end.clear();
        }
        if copy.quiet_start.is_empty() != copy.quiet_end.is_empty() {
            copy.quiet_start.clear();
            copy.quiet_end.clear();
        }
        copy
    }

    /// 当前时刻（**本地时间的当天第几分钟**）是否落在静默时段。
    /// `start > end` 表示跨零点；未配置即不静默；`start == end` 视为**不静默**。
    pub fn in_quiet_hours(&self, minutes_of_day: u32) -> bool {
        if self.quiet_start.is_empty() || self.quiet_end.is_empty() {
            return false;
        }
        let start = minutes(&self.quiet_start);
        let end = minutes(&self.quiet_end);
        if start == end {
            return false;
        }
        let now = minutes_of_day as i64;
        if start < end {
            now >= start && now < end
        } else {
            now >= start || now < end
        }
    }

    /// 人是否不在机器前：锁屏 / 显示器睡眠 / 无输入超过阈值，任一成立即算离开。
    ///
    /// 三条里对远程桌面真正起作用的是第三条。**取不到信号时按「已离开」处理**：
    /// 这一条 fail-open 与节流那条相反——宁可多发一条，也不要出现
    /// 「开关看着开了、其实永远不发」那种查不出来的失效。
    pub fn is_away(&self, signals: &PresenceSignals) -> bool {
        if signals.screen_locked || signals.display_asleep {
            return true;
        }
        match signals.idle_seconds {
            None => true,
            Some(idle) => idle >= self.normalized().away_idle_seconds as f64,
        }
    }

    /// 判成「有人在」时给出依据（写进「最近外发」，被挡下的通知要说得出为什么被挡）
    pub fn present_reason(&self, signals: &PresenceSignals) -> String {
        match signals.idle_seconds {
            None => "取不到输入时长".to_string(),
            Some(idle) => format!(
                "距上次输入 {} 秒，未达 {} 秒",
                idle as i64,
                self.normalized().away_idle_seconds
            ),
        }
    }

    /// 该事件类型是否允许外发
    pub fn allows(&self, kind: EventKind) -> bool {
        match kind {
            EventKind::Completed => self.send_completed,
            EventKind::Attention => self.send_attention,
            EventKind::CostSpike => self.send_cost_spike,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EventKind {
    #[serde(rename = "completed")]
    Completed,
    #[serde(rename = "attention")]
    Attention,
    #[serde(rename = "costSpike")]
    CostSpike,
}

impl EventKind {
    /// 从事件类型字符串解析（引擎的事件、界面传来的参数都是字符串）
    pub fn parse(raw: &str) -> Option<EventKind> {
        match raw {
            "completed" => Some(EventKind::Completed),
            "attention" => Some(EventKind::Attention),
            "costSpike" => Some(EventKind::CostSpike),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            EventKind::Completed => "completed",
            EventKind::Attention => "attention",
            EventKind::CostSpike => "costSpike",
        }
    }
}

/// 「这台机器前现在有没有人」的原始信号。判定逻辑在 [`Policy::is_away`]（纯函数、可离线测），
/// 取信号那一层不掺判断——它要碰窗口服务器。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PresenceSignals {
    pub screen_locked: bool,
    pub display_asleep: bool,
    /// 距上一次键盘/鼠标输入的秒数；`None` = 取不到
    pub idle_seconds: Option<f64>,
}

impl PresenceSignals {
    /// 一个信号都取不到（非 GUI 进程、窗口服务器拒绝等）
    pub fn unavailable() -> Self {
        PresenceSignals {
            screen_locked: false,
            display_asleep: false,
            idle_seconds: None,
        }
    }
}

/// `HH:mm` 是否合法（`0..=23` 时、`0..=59` 分）
pub fn is_hhmm(text: &str) -> bool {
    let mut parts = text.split(':');
    let (Some(hour), Some(minute), None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    let (Ok(hour), Ok(minute)) = (hour.parse::<i64>(), minute.parse::<i64>()) else {
        return false;
    };
    (0..=23).contains(&hour) && (0..=59).contains(&minute)
}

fn minutes(hhmm: &str) -> i64 {
    let mut parts = hhmm.split(':');
    let (Some(hour), Some(minute)) = (parts.next(), parts.next()) else {
        return -1;
    };
    match (hour.parse::<i64>(), minute.parse::<i64>()) {
        (Ok(hour), Ok(minute)) => hour * 60 + minute,
        _ => -1,
    }
}

/// 模板里允许出现的密钥占位（值永远不落盘，运行时从钥匙串取出替换）
pub fn contains_placeholder(text: &str) -> bool {
    text.contains("{key}")
}

/// 像不像一条凭据：16 位以上的纯字母数字。ntfy 主题这类用户自取的短名不会命中。
pub fn looks_like_token(text: &str) -> bool {
    text.chars().count() >= 16 && text.chars().all(|c| c.is_ascii_alphanumeric())
}

/// 把存档里的通道原文解析成通道。
///
/// 不认识时（版本回退、手改 JSON、将来加新通道后降级）**不能只是「当没发生过」**：
/// 回落成 ntfy 之后，用户在界面上敲的每个字符都会写进 ntfy 的键，而他真正的配置还留在
/// 原来那个键下——两份并存，且没人说过。所以把不认识的原文一并回吐，让界面能明说
/// 「现在显示的是 ntfy，改动会写 ntfy」。
pub fn resolve_kind(raw: Option<&str>) -> (Channel, Option<String>) {
    match raw {
        None => (Channel::Ntfy, None),
        Some(text) => match Channel::parse(text) {
            Some(channel) => (channel, None),
            None => (Channel::Ntfy, Some(text.to_string())),
        },
    }
}

/// 判定要用的「现在」：毫秒时间戳 + **本地**当天第几分钟。
///
/// 两者分开给，是因为本地分钟取不到时要按「不静默」降级（fail-open），这与时间戳无关；
/// 也正因为分开，静默时段的口径可以离线测（用例传一个固定的分钟数）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Now {
    pub ms: i64,
    pub minutes_of_day: Option<u32>,
}

impl Now {
    /// 生产路径：从系统时钟取，本地分钟取不到就是 `None`
    pub fn at(ms: i64) -> Now {
        Now {
            ms,
            minutes_of_day: local_minutes_of_day(ms),
        }
    }
}

/// 本地时间的「当天第几分钟」。Swift 用 `Calendar.current`；Rust 的 std 里没有本地时区，
/// 所以由 `localclock` 统一读取系统日历。
///
/// 取不到就返回 `None`，**调用方必须把它当成「不静默」**（fail-open）——这与 Swift
/// 对写坏的静默时段的降级方向一致：宁可发出去，也不要出现「开关开着却永远不发」。
pub fn local_minutes_of_day(now_ms: i64) -> Option<u32> {
    let tm = crate::localclock::local_time(now_ms)?;
    let (hour, minute) = (tm.tm_hour, tm.tm_min);
    if !(0..24).contains(&hour) || !(0..60).contains(&minute) {
        return None;
    }
    Some((hour * 60 + minute) as u32)
}

/// 「外发现在什么状态」——设置页要用的一整张快照。
///
/// **不含任何密钥值**：`has_secret` 是布尔，凭据本身永远不经过这里（ADR 0009）。
/// 把「配齐了没有」「地址是不是明文」「是不是直接粘了密钥」三条**分开**给：
/// Swift 侧它们是三个独立的红色提示，混成一条会让每条都变模糊。
/// **能力边界**，逐字上屏（与 Provider 页同一条纪律：界面不自己编一句
/// 「我们支持什么」——那份文案是约束，编错了就是骗）。
///
/// 内容对齐 Swift `RemoteNotifySettingsView` 的三段 help 与默认说明，
/// 外加 Rust 侧**特有**的一条：在场信号层还没接，那一条必须写出来而不是让人猜。
pub const REMOTE_LIMITATIONS: &str = "默认只送「哪个 Agent + 什么状态」，不含命令内容、路径与消息原文。三个通道：ntfy（订阅一个主题名）、自定义 HTTP 模板（微信 Server酱 / PushPlus、企微、钉钉、飞书都走这条——密钥写 {key}、标题 {title}、正文 {body}）、邮箱 SMTP。邮箱只支持 465（隐式 TLS）：25/587 的 STARTTLS 需要在已建立的 TCP 上原地升级，本仓不提供，填了会被拦住并说明原因。密钥只进系统钥匙串，界面与日志只显示掩码，设置文件里零密钥。本平台暂未接入屏幕锁定与显示器睡眠信号，「人不在」一律按「已离开」放行（fail-open）。";

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    /// 解析后的通道名（存档里不认识时是回落后的那个）
    pub kind: String,
    /// 存档里的原文——不认识时才非空，界面据此明说「现在显示的是回落后的通道」
    pub unrecognized_kind: Option<String>,
    pub label: String,
    /// 密钥在钥匙串里的条目名（不是密钥本身）
    pub secret_name: String,
    /// 配齐了没有；`None` = 已配齐
    pub readiness: Option<String>,
    pub insecure_endpoint: Option<String>,
    pub plaintext_secret: Option<String>,
    pub policy: Policy,
    /// 此刻是否落在静默时段（本地时间取不到时按**不静默**降级）
    pub quiet_now: bool,
    /// 此刻是否判成「人不在」。注意 Rust 还没接 macOS 的在场信号层，
    /// 所以今天它一律是 `true`（fail-open），依据见 `away_reason`
    pub away_now: bool,
    pub away_reason: String,
    /// 事件类型 → 该类是否允许外发
    pub allows: Vec<(&'static str, bool)>,
    /// 当前被节流窗口握住的条目数（由命令层从 notifier 填；判定层不持有节流状态）
    pub throttled: usize,
    /// 能力边界原文（见 [`REMOTE_LIMITATIONS`]）
    pub limitations: &'static str,
}

/// 组装上面那张快照。
///
/// `now` 由调用方给（生产 = `Now::at(now_ms)`，用例 = `Now::fixed`）：
/// 判定层不自己去取本地时间，这样静默时段的口径可以离线测。
#[allow(clippy::too_many_arguments)]
pub fn status(
    kind_raw: Option<&str>,
    channels: &std::collections::HashMap<String, ChannelConfig>,
    policy: &Policy,
    has_secret: bool,
    now: Now,
    presence: &PresenceSignals,
) -> Status {
    let (channel, unrecognized) = resolve_kind(kind_raw);
    let empty = ChannelConfig::default();
    let config = channels.get(channel.as_str()).unwrap_or(&empty);
    let policy = policy.normalized();
    Status {
        limitations: REMOTE_LIMITATIONS,
        kind: channel.as_str().to_string(),
        unrecognized_kind: unrecognized,
        label: channel.label().to_string(),
        secret_name: channel.default_secret_name(),
        readiness: channel.missing_field(config, has_secret).map(str::to_string),
        insecure_endpoint: channel.insecure_endpoint(config).map(str::to_string),
        plaintext_secret: channel
            .plaintext_secret_in_template(config)
            .map(str::to_string),
        quiet_now: now
            .minutes_of_day
            .map(|minutes| policy.in_quiet_hours(minutes))
            .unwrap_or(false),
        away_now: policy.is_away(presence),
        away_reason: policy.present_reason(presence),
        allows: vec![
            (EventKind::Completed.as_str(), policy.allows(EventKind::Completed)),
            (EventKind::Attention.as_str(), policy.allows(EventKind::Attention)),
            (EventKind::CostSpike.as_str(), policy.allows(EventKind::CostSpike)),
        ],
        throttled: 0,
        policy,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> ChannelConfig {
        ChannelConfig::default()
    }

    fn ntfy(topic: &str) -> ChannelConfig {
        ChannelConfig {
            topic_or_url: topic.into(),
            ..ChannelConfig::default()
        }
    }

    fn http(url: &str, body: &str) -> ChannelConfig {
        ChannelConfig {
            url_template: url.into(),
            body_template: body.into(),
            ..ChannelConfig::default()
        }
    }

    fn smtp(port: i64) -> ChannelConfig {
        ChannelConfig {
            smtp_host: "smtp.example.com".into(),
            smtp_user: "me@example.com".into(), // nosec: 测试夹具里的假邮箱（保留域名），不是真地址
            smtp_to: "you@example.com".into(), // nosec: 测试夹具里的假邮箱（保留域名），不是真地址
            smtp_port: port,
            ..ChannelConfig::default()
        }
    }

    // ── 归一化 ───────────────────────────────────────────────────────────────

    #[test]
    fn normalized_clamps_ranges_and_degrades_broken_quiet_hours_to_no_quiet() {
        let dirty = Policy {
            throttle_seconds: 5,
            away_idle_seconds: 9999,
            quiet_start: "25:00".into(),
            quiet_end: "07:00".into(),
            ..Policy::default()
        };
        let clean = dirty.normalized();
        assert_eq!(clean.throttle_seconds, THROTTLE_RANGE.0, "节流下限");
        assert_eq!(clean.away_idle_seconds, IDLE_RANGE.1, "离开阈值上限");
        assert!(
            clean.quiet_start.is_empty() && clean.quiet_end.is_empty(),
            "写坏的静默时段必须退化成「不静默」，而不是整天把通知吞掉"
        );

        // 只有一端合法时整对作废：半截窗口的含义没人说得清
        let half = Policy {
            quiet_start: "22:00".into(),
            quiet_end: "oops".into(),
            ..Policy::default()
        }
        .normalized();
        assert!(half.quiet_start.is_empty() && half.quiet_end.is_empty());

        // 合法值原样留着
        let ok = Policy {
            quiet_start: "22:00".into(),
            quiet_end: "07:00".into(),
            ..Policy::default()
        }
        .normalized();
        assert_eq!((ok.quiet_start.as_str(), ok.quiet_end.as_str()), ("22:00", "07:00"));
    }

    #[test]
    fn normalized_accepts_only_real_clock_times() {
        assert!(is_hhmm("00:00"));
        assert!(is_hhmm("23:59"));
        assert!(is_hhmm("07:05"));
        assert!(!is_hhmm("24:00"));
        assert!(!is_hhmm("07:60"));
        assert!(!is_hhmm("7"));
        assert!(!is_hhmm("07:05:00"));
        assert!(!is_hhmm(""));
    }

    // ── 静默时段 ─────────────────────────────────────────────────────────────

    #[test]
    fn quiet_hours_handle_the_midnight_wrap_and_the_boundaries() {
        let same = Policy {
            quiet_start: "08:00".into(),
            quiet_end: "08:00".into(),
            ..Policy::default()
        };
        assert!(!same.in_quiet_hours(8 * 60), "start == end 视为不静默");

        let day = Policy {
            quiet_start: "09:00".into(),
            quiet_end: "18:00".into(),
            ..Policy::default()
        };
        assert!(!day.in_quiet_hours(8 * 60 + 59));
        assert!(day.in_quiet_hours(9 * 60), "起点含");
        assert!(day.in_quiet_hours(17 * 60 + 59));
        assert!(!day.in_quiet_hours(18 * 60), "终点不含");

        let night = Policy {
            quiet_start: "22:00".into(),
            quiet_end: "07:00".into(),
            ..Policy::default()
        };
        assert!(night.in_quiet_hours(23 * 60), "跨零点：起点之后");
        assert!(night.in_quiet_hours(6 * 60), "跨零点：终点之前");
        assert!(!night.in_quiet_hours(12 * 60), "白天不静默");

        let unset = Policy::default();
        assert!(!unset.in_quiet_hours(3 * 60), "没配就是不静默");
    }

    // ── 在场判定 ─────────────────────────────────────────────────────────────

    #[test]
    fn away_is_fail_open_when_the_signal_is_missing() {
        let policy = Policy::default();
        assert!(policy.is_away(&PresenceSignals {
            screen_locked: true,
            ..PresenceSignals::default()
        }));
        assert!(policy.is_away(&PresenceSignals {
            display_asleep: true,
            ..PresenceSignals::default()
        }));
        // 取不到输入时长 ⇒ 按「已离开」处理：宁可多发一条，也不要出现
        // 「开关看着开了、其实永远不发」那种查不出来的失效
        assert!(policy.is_away(&PresenceSignals::unavailable()));
        assert_eq!(
            policy.present_reason(&PresenceSignals::unavailable()),
            "取不到输入时长"
        );

        // 阈值默认 120 秒
        assert!(policy.is_away(&PresenceSignals {
            idle_seconds: Some(120.0),
            ..PresenceSignals::default()
        }));
        assert!(!policy.is_away(&PresenceSignals {
            idle_seconds: Some(119.9),
            ..PresenceSignals::default()
        }));
        assert_eq!(
            policy.present_reason(&PresenceSignals {
                idle_seconds: Some(119.9),
                ..PresenceSignals::default()
            }),
            "距上次输入 119 秒，未达 120 秒"
        );
    }

    #[test]
    fn event_type_switches_are_independent() {
        let policy = Policy {
            send_attention: false,
            ..Policy::default()
        };
        assert!(!policy.allows(EventKind::Attention));
        assert!(policy.allows(EventKind::Completed));
        assert!(policy.allows(EventKind::CostSpike));
    }

    // ── 通道校验 ─────────────────────────────────────────────────────────────

    #[test]
    fn ntfy_rejects_a_schemeless_self_hosted_address() {
        // 判定只写「以 http 开头」的话，`ntfy.mine.local/island` 会被拼成
        // https://ntfy.sh/ntfy.mine.local/island——内容跑到用户没选过的公网服务器上
        assert_eq!(
            Channel::Ntfy.missing_field(&ntfy("ntfy.mine.local/island"), false),
            Some("像是要自建服务器的地址，但少了 http:// 或 https:// 前缀")
        );
        assert_eq!(
            Channel::Ntfy.missing_field(&ntfy("https://ntfy.mine.local/island"), false),
            None
        );
        assert_eq!(
            Channel::Ntfy.missing_field(&ntfy("island topic"), false),
            Some("主题名只能是无空格的 ASCII 字符")
        );
        assert_eq!(
            Channel::Ntfy.missing_field(&ntfy("岛上主题"), false),
            Some("主题名只能是无空格的 ASCII 字符")
        );
        assert_eq!(
            Channel::Ntfy.missing_field(&ntfy(""), false),
            Some("缺主题名或服务器地址")
        );
        // ntfy 不需要密钥
        assert_eq!(Channel::Ntfy.missing_field(&ntfy("island"), false), None);
    }

    #[test]
    fn custom_http_requires_a_body_template_and_an_actual_secret_for_placeholders() {
        assert_eq!(
            Channel::CustomHttp.missing_field(&http("", "x"), false),
            Some("缺发送地址")
        );
        assert_eq!(
            Channel::CustomHttp.missing_field(&http("https://x/y", "  "), false),
            Some("缺请求体模板：各家字段名不同，请填写类似 \"title={title}&desp={body}\" 的形状")
        );
        assert_eq!(
            Channel::CustomHttp.missing_field(&http("https://x/y?key={key}", "a={title}"), false),
            Some("地址里有 {key} 占位，但钥匙串里还没有密钥")
        );
        assert_eq!(
            Channel::CustomHttp.missing_field(&http("https://x/y?key={key}", "a={title}"), true),
            None
        );
        // 没有占位就不需要密钥
        assert_eq!(
            Channel::CustomHttp.missing_field(&http("https://x/y", "a={title}"), false),
            None
        );
    }

    #[test]
    fn smtp_checks_the_port_before_the_secret() {
        // 非 465 是「这条路根本不走」，缺授权码只是「还差最后一步」——顺序不能反
        assert_eq!(
            Channel::SmtpEmail.missing_field(&smtp(587), false),
            Some("仅支持 465（隐式 TLS）；25/587 的 STARTTLS 不支持")
        );
        assert_eq!(
            Channel::SmtpEmail.missing_field(&smtp(465), false),
            Some("缺 SMTP 密码/授权码（存钥匙串）")
        );
        assert_eq!(Channel::SmtpEmail.missing_field(&smtp(465), true), None);

        let mut missing_host = smtp(465);
        missing_host.smtp_host.clear();
        assert_eq!(
            Channel::SmtpEmail.missing_field(&missing_host, true),
            Some("缺 SMTP 服务器地址")
        );
    }

    #[test]
    fn security_warnings_are_separate_from_readiness() {
        // 明文 http：配齐了也要单独警告
        let plaintext = Channel::Ntfy.insecure_endpoint(&ntfy("http://ntfy.sh/island"));
        assert!(plaintext.unwrap().starts_with("地址是明文 http://"));
        assert_eq!(Channel::Ntfy.insecure_endpoint(&ntfy("https://ntfy.sh/island")), None);
        // SMTP 只走 465，一定是 TLS，没有这一条警告
        assert_eq!(Channel::SmtpEmail.insecure_endpoint(&smtp(465)), None);

        // 直接粘了密钥：三种形状都要认出来
        assert!(Channel::CustomHttp
            .plaintext_secret_in_template(&http("https://x/y?key=abc", "b"))
            .is_some());
        assert!(Channel::CustomHttp
            .plaintext_secret_in_template(&http("https://x/y?token=abc", "b"))
            .is_some());
        assert!(Channel::CustomHttp
            .plaintext_secret_in_template(&http("https://sc.ftqq.com/ABC123.send", "b"))
            .is_some());
        // 16 位以上的纯字母数字片段也算（用户自取的短主题名不会命中）
        assert!(Channel::CustomHttp
            .plaintext_secret_in_template(&http("https://x/y/ABCDEFGHIJKLMNOP", "b"))
            .is_some());
        assert!(!looks_like_token("island"));
        // 用了占位就没事：值在钥匙串里
        assert_eq!(
            Channel::CustomHttp.plaintext_secret_in_template(&http("https://x/y?key={key}", "b")),
            None
        );
    }

    // ── 容错解码 ─────────────────────────────────────────────────────────────

    #[test]
    fn a_wrongly_typed_field_only_invalidates_that_field() {
        // 手改 JSON 把端口写成字符串：只作废端口，其余照读
        // （Swift 逐字段 decodeIfPresent 同口径；`serde(default)` 做不到，那会整条解不开）
        let json = r#"{
            "topicOrURL":"island","urlTemplate":"u","bodyTemplate":"b",
            "useJSONBody":"yes","smtpHost":"h","smtpPort":"587",
            "smtpUser":"me","smtpTo":"you","includeActionDetail":true
        }"#;
        let parsed: ChannelConfig = serde_json::from_str(json).expect("坏一个字段不该让整条解不开");
        assert_eq!(parsed.smtp_port, 465, "端口类型错 ⇒ 回落默认，而不是解析出 587");
        assert!(parsed.use_json_body, "布尔类型错 ⇒ 回落默认");
        assert_eq!(parsed.smtp_to, "you", "其余字段照读");
        assert!(parsed.include_action_detail);
    }

    #[test]
    fn a_partial_archive_keeps_the_keys_it_has() {
        let config: ChannelConfig =
            serde_json::from_str(r#"{"topicOrURL":"island"}"#).expect("缺字段不该整条解不开");
        assert_eq!(config.topic_or_url, "island");
        assert_eq!(config.smtp_port, 465);
        assert!(config.use_json_body);

        let policy: Policy =
            serde_json::from_str(r#"{"masterEnabled":true,"throttleSeconds":5}"#).expect("同上");
        assert!(policy.master_enabled);
        assert!(policy.send_attention, "没写的键取默认");
        assert_eq!(policy.normalized().throttle_seconds, 15, "越界在读时被钳");
        // 以后新增的字段出现在旧存档里 ⇒ 直接被忽略
        let future: Policy =
            serde_json::from_str(r#"{"masterEnabled":true,"brandNewKnob":42}"#).expect("未知键应忽略");
        assert!(future.master_enabled);
    }

    // ── 通道解析与状态 ───────────────────────────────────────────────────────

    #[test]
    fn an_unrecognized_channel_falls_back_but_says_so() {
        assert_eq!(resolve_kind(None), (Channel::Ntfy, None));
        assert_eq!(resolve_kind(Some("smtpEmail")), (Channel::SmtpEmail, None));
        let (channel, raw) = resolve_kind(Some("telegram"));
        assert_eq!(channel, Channel::Ntfy);
        assert_eq!(
            raw.as_deref(),
            Some("telegram"),
            "不认识的原文必须回吐：否则界面写的是回落通道，用户以为改的是自己那个"
        );
        assert_eq!(Channel::SmtpEmail.default_secret_name(), "remote.smtpEmail");
    }

    #[test]
    fn status_reports_each_warning_separately_and_never_a_secret_value() {
        let mut channels = std::collections::HashMap::new();
        channels.insert(
            "ntfy".to_string(),
            // 下面这条地址是**故意**构造成「像直接粘了密钥」的样子，用来测那条检测本身；
            // 里面的值不是任何真实凭据。
            //
            // 值刻意取**短**：被测的是「有没有用 {key} 占位」，判据是地址里含不含
            // `key=`，与熵无关。而 12 位以上的值会被脱敏扫描器的 `?key=<12+字符>`
            // 规则当成真的 token 参数——那会让每次改这个文件都产生一个新 blob、
            // 要重新备案一次。夹具不该靠豁免活着。
            ntfy("http://ntfy.sh/island?key=PasteMe"),
        );
        let policy = Policy {
            master_enabled: true,
            quiet_start: "22:00".into(),
            quiet_end: "07:00".into(),
            ..Policy::default()
        };
        let snapshot = status(
            Some("ntfy"),
            &channels,
            &policy,
            false,
            Now { ms: 0, minutes_of_day: Some(23 * 60) },
            &PresenceSignals::unavailable(),
        );
        assert_eq!(snapshot.kind, "ntfy");
        assert_eq!(snapshot.unrecognized_kind, None);
        assert_eq!(snapshot.secret_name, "remote.ntfy");
        assert!(snapshot.readiness.is_none(), "ntfy 不需要密钥，算配齐");
        assert!(snapshot.insecure_endpoint.is_some(), "明文 http 单独一条");
        assert!(snapshot.plaintext_secret.is_some(), "直接粘密钥单独一条");
        assert!(snapshot.quiet_now, "23:00 落在 22:00–07:00");
        assert!(snapshot.away_now, "取不到在场信号 ⇒ fail-open 判成离开");
        assert_eq!(snapshot.away_reason, "取不到输入时长");
        assert!(snapshot.allows.iter().all(|(_, allowed)| *allowed));
        // 序列化里不含任何密钥值：粘进来的那串只出现在被判定为「明文粘了密钥」的
        // **配置**里，而状态快照只带布尔与条目名
        let json = serde_json::to_string(&snapshot).unwrap();
        assert!(!json.contains("PasteMe"), "状态快照不许带凭据原文");
    }

    #[test]
    fn local_minutes_of_day_is_a_clock_reading_or_nothing() {
        let now = crate::tokens::now_ms();
        match local_minutes_of_day(now) {
            Some(minutes) => assert!(minutes < 24 * 60),
            None => panic!("本机应能取到本地时间"),
        }
        // 同一时刻两次调用一致（取不到就只能是环境不支持，不是随机）
        assert_eq!(local_minutes_of_day(now), local_minutes_of_day(now));
    }
}
