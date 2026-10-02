//! 外发渲染层：事件 → 消息 → 具体的请求（HTTP / SMTP），以及「发送预览」。
//!
//! 对齐 Swift `RemoteNotifier.render` / `renderRequest` / `clamp` / `WireText` /
//! `RenderedRequest.maskedPreview` 与 `RemoteSecret` 的掩码规则。本轮**不含传输**
//! （HTTP 客户端与 SMTP 会话要选 TLS crate，是独立决定），但渲染结果已经能被
//! 「发送预览」原样展示——这正是 Swift 侧这个功能存在的理由：
//! **让用户在真发之前看见到底哪些字节会离开这台机器**。
//!
//! 三条硬规矩（与 Swift 注释同一套）：
//! ① 默认只带最小内容（Agent 名 + 状态 + 多久前）；命令文本与路径必须
//!    `includeActionDetail` 显式打开才进得来；
//! ② 上线前的文本整形必须按目标格式分别转义——同一个值在 HTTP 头、查询串、
//!    表单体、JSON 里需要**四种不同的**转义，混用即参数注入或静默丢字；
//! ③ 预览与日志里的密钥一律打码：泄漏最常见的路径就是「把完整 URL 打印出来了」。

use crate::remote::{Channel, ChannelConfig, EventKind};
use serde::{Deserialize, Serialize};

/// ntfy 公开服务器单条上限（超出直接 400）
pub const NTFY_BYTE_LIMIT: usize = 4096;

// ── 上线前的文本整形（HTTP 头 / URL / 表单 / JSON / SMTP 行）────────────────────

/// HTTP 头值允许的字符（RFC 7230 ttext 的可打印子集，去掉空格与分隔符）。
/// 非 ASCII 必须编码：`URLSession` 这类实现会**静默丢弃**非 ASCII 字符，
/// 实测中文 `X-Title: Qoder · 等待你确认` 到达对端只剩 `Qoder · `——
/// 通知最要紧的那几个字没了，而且本地看不出来。
fn header_allowed(c: char) -> bool {
    matches!(c, '!'..='#' | '$'..='\'' | '*' | '+' | '-' | '.' | '^' | '_' | '`' | '|' | '~')
        || c.is_ascii_alphanumeric()
}

/// RFC 3986 unreserved 集之外的都要百分号编码。
/// **不能复用头值那套**：头值允许 `& # % +`，把它们原样放进查询串就是参数注入——
/// 勾选「附带最后一条动作」后正文里是真命令，
/// `git commit -m a && curl …` 这类文本会在接收端被切成两个字段。
fn query_allowed(c: char) -> bool {
    matches!(c, '-' | '.' | '_' | '~') || c.is_ascii_alphanumeric()
}

fn percent_encode(text: &str, allowed: fn(char) -> bool, space: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if allowed(c) {
            out.push(c);
        } else if c == ' ' {
            out.push_str(space);
        } else {
            let mut buffer = [0u8; 4];
            for byte in c.encode_utf8(&mut buffer).as_bytes() {
                out.push_str(&format!("%{byte:02X}"));
            }
        }
    }
    out
}

/// HTTP 头值：百分号编码成纯 ASCII（空格编成 `%20`，非 ASCII 走 UTF-8 字节序列）。
/// ntfy 这类接收端会按文档对头值做 URL 解码。
pub fn header_value(text: &str) -> String {
    percent_encode(text, header_allowed, "%20")
}

/// 查询串里的值：空格按 `application/x-www-form-urlencoded` 编成 `+`。
/// 刻意不把「 unreserved 以外全编码」用到头值上，也刻意不用「只放行字母数字」——
/// 后者会把 `-_.~` 也编掉，一条 6 字中文标题从 54 字节涨到更大，
/// 而各家中转服务对 URL 长度都有上限（Server酱 免费版最紧）。
pub fn form_value(text: &str) -> String {
    query_value(text).replace("%20", "+")
}

/// 查询串里的值：`%20` 编空格（不是 `+`），其余按 unreserved 集
pub fn query_value(text: &str) -> String {
    percent_encode(text, query_allowed, "%20")
}

/// JSON 字符串字面量（**带**引号）。自己写转义而不是拼字符串：
/// 正文里一个裸引号或换行就能让整条 JSON 非法，而接收端只会回一个看不懂的 4xx
pub fn json_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04X}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// 模板里用的 JSON 片段（去掉外层引号）
pub fn dropping_json_quotes(text: &str) -> &str {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() >= 2 && chars[0] == '"' && chars[chars.len() - 1] == '"' {
        let start = text.char_indices().nth(1).map(|(i, _)| i).unwrap_or(0);
        let end = text.char_indices().nth(chars.len() - 1).map(|(i, _)| i).unwrap_or(text.len());
        return &text[start..end];
    }
    text
}

/// 「3 分前 / 2 小时前」——与岛内文案同口径（Core 不能依赖 UI，所以自己实现）
pub fn duration_short(seconds: f64) -> String {
    let s = seconds.max(0.0) as i64;
    if s < 60 {
        return format!("{s}秒");
    }
    let m = s / 60;
    if m < 60 {
        return format!("{m}分");
    }
    let h = m / 60;
    if h < 24 {
        return format!("{h}小时");
    }
    format!("{}天", h / 24)
}

// ── 掩码（只用于界面回显，绝不用它做日志或诊断输出）────────────────────────────

/// 掩码：留首尾各 2 位，中间一律 `•`（最多 12 个）
pub fn masked(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    if chars.len() <= 4 {
        return "•".repeat(chars.len().max(1));
    }
    let head: String = chars[..2].iter().collect();
    let tail: String = chars[chars.len() - 2..].iter().collect();
    format!("{head}{}{tail}", "•".repeat((chars.len() - 4).min(12)))
}

/// 固定三点的短掩码（用于 URL 段里的高熵片段）
fn mask_token(token: &str) -> String {
    let chars: Vec<char> = token.chars().collect();
    let head: String = chars.iter().take(2).collect();
    let tail: String = chars.iter().skip(chars.len().saturating_sub(2)).collect();
    format!("{head}•••{tail}")
}

/// 预览里必须打码的请求头（其余如 X-Title / X-Priority 原样显示才有意义）
pub fn sensitive_header(name: &str) -> bool {
    let n = name.to_lowercase();
    n.contains("authorization") || n.contains("token") || n.contains("key") || n == "x-webhook-key"
}

/// URL 里的密钥常见形态：`?token=xxx` / `&key=xxx` / `.../<SendKey>.send`。
/// 预览与日志都必须过这一层——密钥泄漏最常见的路径就是「把完整 URL 打印出来了」。
pub fn masked_url(url: &str) -> String {
    if let Some((base, _)) = url.split_once("/bot/v2/hook/") { return format!("{base}/bot/v2/hook/••••••"); }
    let mut out = url.to_string();
    for query in ["access_token", "token", "sendkey", "key", "webhook"] {
        let needle = format!("{query}=");
        let mut from = 0usize;
        while let Some(rel) = out[from..].find(&needle) {
            let value_start = from + rel + needle.len();
            let value_end = out[value_start..]
                .find('&')
                .map(|i| value_start + i)
                .unwrap_or(out.len());
            let value = out[value_start..value_end].to_string();
            if value.is_empty() {
                from = value_end;
                continue;
            }
            let replacement = masked(&value);
            out.replace_range(value_start..value_end, &replacement);
            from = value_start + replacement.len();
        }
    }
    // 一条规则同时管主机名与路径：Server酱 把 SendKey 放在主机名首段
    // （https://<32位key>.send），各家 webhook 放在路径段（/notify/<32位token>），
    // 而帮助文本就叫用户「把控制台给出的地址整段粘进来」——两条路都会被走到。
    // 判据是「按 . ? & = : / 切开后的某一段是 16+ 位纯字母数字」：
    // sctapi.ftqq.com 与 my-agent-island-topic 都不会命中。
    let segments: Vec<&str> = out.split('/').filter(|s| !s.is_empty()).collect();
    if segments.len() < 2 {
        return out;
    }
    let mut tokens: Vec<String> = Vec::new();
    for segment in &segments[1..] {
        if let Some(token) = segment
            .split(|c| ".?=&:".contains(c))
            .find(|part| crate::remote::looks_like_token(part))
        {
            tokens.push(token.to_string());
        }
    }
    for token in tokens {
        out = out.replace(&token, &mask_token(&token));
    }
    out
}

// ── 渲染 ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub title: String,
    pub body: String,
    pub urgent: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HttpField {
    pub name: String,
    pub value: String,
}

impl HttpField {
    pub fn new(name: &str, value: impl Into<String>) -> Self {
        HttpField {
            name: name.to_string(),
            value: value.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SmtpTarget {
    pub host: String,
    pub port: i64,
    pub user: String,
    pub password: String,
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Request {
    pub url: String,
    pub method: String,
    pub headers: Vec<HttpField>,
    pub body: String,
    /// SMTP 用：连接与认证参数（HTTP 通道为 `None`）
    pub smtp: Option<SmtpTarget>,
    pub response_check: Option<Channel>,
}

impl Request {
    /// 供界面显示的脱敏文本：密钥/授权码/含 key 的 URL 一律打码
    pub fn masked_preview(&self) -> String {
        let mut lines = vec![format!("{} {}", self.method, masked_url(&self.url))];
        for field in &self.headers {
            let value = if sensitive_header(&field.name) {
                masked(&field.value)
            } else {
                field.value.clone()
            };
            lines.push(format!("{}: {value}", field.name));
        }
        if let Some(smtp) = &self.smtp {
            lines.push(format!(
                "SMTP {}:{} 发信={} 收信={}",
                smtp.host, smtp.port, smtp.from, smtp.to
            ));
            lines.push("密码=••••••".to_string());
        }
        if !self.body.is_empty() {
            lines.push(String::new());
            let mut body = self.body.clone();
            if self.response_check.is_some() {
                if let Ok(mut json) = serde_json::from_str::<serde_json::Value>(&body) {
                    for key in ["token", "sign"] { if json.get(key).is_some() { json[key] = serde_json::json!("••••••"); } }
                    body = json.to_string();
                }
            }
            lines.push(body);
        }
        lines.join("\n")
    }
}

/// 事件 → 外发的输入。`agent_id` 与 `agent_name` 分开：节流键用 id——
/// 显示名可以被外部投递路径随便填，每次换一个名字就等于绕过节流。
#[derive(Debug, Clone, PartialEq)]
pub struct Inputs {
    pub agent_name: String,
    pub agent_id: String,
    pub kind: EventKind,
    /// completed 当作「本次任务用时」；其余类型当作「事件发生至今」。
    /// 两条语义分开是因为外发的那一刻同类事件总是刚发生，写成「3 分前」是谎报。
    pub seconds: f64,
    /// 仅当通道配置打开 `includeActionDetail` 时才会进入正文
    pub action_detail: Option<String>,
    pub message: Option<String>,
}

impl Inputs {
    pub fn new(agent_name: impl Into<String>, kind: EventKind, seconds: f64) -> Inputs {
        let agent_name = agent_name.into();
        Inputs {
            agent_id: agent_name.clone(),
            agent_name,
            kind,
            seconds,
            action_detail: None,
            message: None,
        }
    }
}

fn first_chars(text: &str, limit: usize) -> String {
    text.chars().take(limit).collect()
}

fn utf8_clip(text: &str, limit: usize) -> String {
    let mut bytes = 0usize;
    let mut out = String::new();
    for c in text.chars() {
        bytes += c.len_utf8();
        if bytes > limit {
            break;
        }
        out.push(c);
    }
    out
}

/// 纯函数：策略已放行 + 通道配置 → 要发的内容。
/// 拆出来是因为它能被完整测试（不碰网络、不读钥匙串）。
pub fn render(inputs: &Inputs, config: &ChannelConfig) -> Message {
    let state = match inputs.kind {
        EventKind::Completed => "任务完成",
        EventKind::Attention => "等待你确认",
        EventKind::CostSpike => "消耗告警",
    };
    let title = format!("{} · {}", inputs.agent_name, state);
    let qualifier = if inputs.kind == EventKind::Completed {
        format!("用时 {}", duration_short(inputs.seconds))
    } else {
        "刚刚".to_string()
    };
    // 正文首行自带 Agent 名：`X-Title` 这类头字段在 ASCII 约束下可能被接收端截断，
    // 而正文是裸 UTF-8 字节，一定到得了
    let mut lines = vec![format!("{} · {}（{}）", inputs.agent_name, state, qualifier)];
    if config.include_action_detail {
        if let Some(detail) = inputs.action_detail.as_deref().filter(|d| !d.is_empty()) {
            lines.push(format!("动作：{}", first_chars(detail, 80)));
        }
        if let Some(message) = inputs.message.as_deref().filter(|m| !m.is_empty()) {
            lines.push(first_chars(message, 120));
        }
    }
    // 不带 includeActionDetail 时正文里绝不出现动作/路径/命令——
    // 这是「哪些字节离开这台机器」的唯一开关
    Message {
        title,
        body: lines.join("\n"),
        urgent: inputs.kind != EventKind::Completed,
    }
}

struct Capped {
    url: String,
    title: String,
    body: String,
}

/// ntfy 公开服务器单条消息上限 4096 字节（超出直接 400）。
/// 截断优先保住首行（哪个 Agent + 什么状态），因为那是唯一必须到得了的信息。
fn clamp(message: &Message, url: &str) -> Capped {
    let mut body = message.body.clone();
    if body.len() > NTFY_BYTE_LIMIT {
        let lines: Vec<&str> = message.body.split('\n').collect();
        let head = lines.first().copied().unwrap_or("");
        let room = NTFY_BYTE_LIMIT.saturating_sub(head.len() + 1);
        body = if lines.len() == 1 {
            utf8_clip(head, NTFY_BYTE_LIMIT)
        } else {
            format!("{head}\n{}", utf8_clip(&lines[1..].join("\n"), room))
        };
    }
    Capped {
        url: url.to_string(),
        // 先编码再截：头值必须 ASCII（不编码时接收端会静默丢掉整段中文），
        // 而编码后的长度是原文的三倍量级，所以预算按编码后的字节算
        title: utf8_clip(&header_value(&message.title), 255),
        body,
    }
}

/// 模板替换：`{key}` 原值/掩码，`{title}` / `{body}` 由调用方**先按目标格式编码好**再传进来。
/// 编码规则放在调用方而不是这里，因为同一对占位符在查询串、表单体与 JSON 里
/// 需要三种完全不同的转义。
fn fill(template: &str, key: &str, title: &str, body: &str) -> String {
    template
        .replace("{key}", key)
        .replace("{title}", title)
        .replace("{body}", body)
}

/// 渲染成实际请求（含密钥替换）。`masked_preview` 为真时用于预览。
pub fn render_request(
    message: &Message,
    channel: Channel,
    config: &ChannelConfig,
    secret: Option<&str>,
    masked_preview: bool,
) -> Request {
    match channel {
        Channel::FeishuBot | Channel::WechatPushPlus | Channel::QqPushPlus | Channel::QqOneBot => crate::im::request(message, channel, config, secret, masked_preview),
        Channel::Ntfy => {
            // 形态按官方文档：POST https://<服务器>/<主题>，标题与优先级走 X-Title / X-Priority
            let starts_with_http = config.topic_or_url.starts_with("http");
            let base = if starts_with_http {
                config.topic_or_url.clone()
            } else {
                "https://ntfy.sh".to_string()
            };
            let topic = if starts_with_http {
                String::new()
            } else {
                config.topic_or_url.clone()
            };
            let url = if topic.is_empty() {
                base
            } else {
                format!("{base}/{topic}")
            };
            let capped = clamp(message, &url);
            Request {
                url: capped.url,
                method: "POST".into(),
                headers: vec![
                    HttpField::new("X-Title", capped.title),
                    HttpField::new("X-Priority", if message.urgent { "4" } else { "3" }),
                ],
                body: capped.body,
                smtp: None, response_check: None,
            }
        }
        Channel::CustomHttp => {
            let key = secret.unwrap_or("");
            let key_text = if masked_preview {
                masked(key)
            } else {
                key.to_string()
            };
            // 地址里的值一律百分号编码；密钥本身按原值替换（各家都把 SendKey 放在路径里，
            // 它本来就是 URL 的一部分）
            let filled = fill(
                &config.url_template,
                &key_text,
                &query_value(&message.title),
                &query_value(&message.body),
            );
            let mut headers = Vec::new();
            let body = if config.use_json_body {
                headers.push(HttpField::new("Content-Type", "application/json"));
                if config.body_template.is_empty() {
                    format!(
                        "{{\"title\":{},\"body\":{}}}",
                        json_string(&message.title),
                        json_string(&message.body)
                    )
                } else {
                    fill(
                        &config.body_template,
                        &key_text,
                        dropping_json_quotes(&json_string(&message.title)),
                        dropping_json_quotes(&json_string(&message.body)),
                    )
                }
            } else {
                // 表单编码：值必须编码，否则正文里的 `&` 会凭空多出一个字段、
                // 一个换行会把请求体切成两段
                headers.push(HttpField::new(
                    "Content-Type",
                    "application/x-www-form-urlencoded",
                ));
                if config.body_template.is_empty() {
                    format!(
                        "title={}&content={}",
                        form_value(&message.title),
                        form_value(&message.body)
                    )
                } else {
                    fill(
                        &config.body_template,
                        &key_text,
                        &form_value(&message.title),
                        &form_value(&message.body),
                    )
                }
            };
            Request {
                url: filled,
                method: "POST".into(),
                headers,
                body,
                smtp: None, response_check: None,
            }
        }
        Channel::SmtpEmail => {
            let Some(secret) = secret else {
                // 没有密钥就渲染不出可连的目标：只留下预览可读的主题
                return Request {
                    method: "SMTP".into(),
                    headers: vec![HttpField::new("Subject", message.title.clone())],
                    body: message.body.clone(),
                    ..Request::default()
                };
            };
            let target = SmtpTarget {
                host: config.smtp_host.clone(),
                port: config.smtp_port,
                user: config.smtp_user.clone(),
                password: if masked_preview {
                    "••••••".to_string()
                } else {
                    secret.to_string()
                },
                from: config.smtp_user.clone(),
                to: config.smtp_to.clone(),
            };
            // Subject 放头里只为预览可读；真正发信由 SMTP 会话用 message.title 组头
            Request {
                method: "SMTP".into(),
                headers: vec![HttpField::new("Subject", message.title.clone())],
                body: message.body.clone(),
                smtp: Some(target),
                ..Request::default()
            }
        }
    }
}

/// 「发送预览」：渲染出来的消息 + **已脱敏**的请求描述。
/// 密钥由调用方给（生产 = 钥匙串，测试 = 查表）；本轮 Rust 侧还没有钥匙串，
/// 所以一律传 `None`，预览里 `{key}` 会显示成掩码而不是假装有值。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    pub title: String,
    pub body: String,
    pub request_summary: String,
}

pub fn preview(
    inputs: &Inputs,
    channel: Channel,
    config: &ChannelConfig,
    secret: Option<&str>,
) -> Preview {
    let message = render(inputs, config);
    let request = render_request(&message, channel, config, secret, true);
    Preview {
        title: message.title.clone(),
        body: message.body.clone(),
        request_summary: request.masked_preview(),
    }
}

/// 供命令层反序列化的可选入参（界面传什么就渲染什么；缺项用示例值）
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PreviewArgs {
    pub agent_name: String,
    pub kind: String,
    pub seconds: f64,
    pub action_detail: Option<String>,
    pub message: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote::ChannelConfig;

    fn ntfy(topic: &str) -> ChannelConfig {
        ChannelConfig {
            topic_or_url: topic.into(),
            ..ChannelConfig::default()
        }
    }

    fn attention() -> Inputs {
        Inputs::new("Qoder", EventKind::Attention, 0.0)
    }

    // ── 四类转义必须分开 ─────────────────────────────────────────────────────

    #[test]
    fn header_values_are_pure_ascii_so_nothing_is_silently_dropped() {
        // 实测过的坑：中文头值会被静默丢弃，对端只收到 `Qoder · `
        let encoded = header_value("Qoder · 等待你确认");
        assert!(encoded.is_ascii(), "头值必须纯 ASCII：{encoded}");
        assert_eq!(encoded, "Qoder%20%C2%B7%20%E7%AD%89%E5%BE%85%E4%BD%A0%E7%A1%AE%E8%AE%A4");
        assert_eq!(header_value("a b"), "a%20b");
    }

    #[test]
    fn query_values_encode_the_separators_that_headers_may_keep() {
        // 勾了「附带最后一条动作」后正文里是真命令：`&` 不编码就会在接收端多切一个字段
        let command = "git commit -m a && curl x#y";
        let query = query_value(command);
        assert!(!query.contains('&'), "查询串里不许出现裸 &：{query}");
        assert!(!query.contains('#'), "也不许出现裸 #");
        assert!(!query.contains('='), "也不许出现裸 =");
        assert_eq!(query, "git%20commit%20-m%20a%20%26%26%20curl%20x%23y");
        // 头值那套允许这些字符，正是不能复用的原因
        assert!(header_value(command).contains('&'));
        // unreserved 集原样保留（-_.~），否则 URL 长度白涨
        assert_eq!(query_value("a-b_c.d~e"), "a-b_c.d~e");
    }

    #[test]
    fn form_values_use_plus_for_spaces() {
        assert_eq!(form_value("a b"), "a+b");
        assert_eq!(form_value("a&b"), "a%26b");
    }

    #[test]
    fn json_strings_escape_what_would_break_the_document() {
        assert_eq!(json_string(r#"a"b"#), r#""a\"b""#);
        assert_eq!(json_string("a\\b"), r#""a\\b""#);
        assert_eq!(json_string("a\nb"), r#""a\nb""#);
        assert_eq!(json_string("a\tb"), r#""a\tb""#);
        assert_eq!(json_string("a\u{1}b"), r#""a\u0001b""#);
        // 中文不必转义：JSON 是 UTF-8
        assert_eq!(json_string("等待"), "\"等待\"");
        assert_eq!(dropping_json_quotes(&json_string("等待")), "等待");
        assert_eq!(dropping_json_quotes("裸"), "裸");
    }

    // ── 掩码 ────────────────────────────────────────────────────────────────

    #[test]
    fn masking_keeps_only_the_ends() {
        assert_eq!(masked(""), "•");
        assert_eq!(masked("abc"), "•••");
        assert_eq!(masked("abcde"), "ab•de");
        assert_eq!(masked("abcdefghijklmnopqrstuvwxyz"), "ab••••••••••••yz");
        assert_eq!(mask_token("ABCDEFGHIJKLMNOP"), "AB•••OP");
        assert!(sensitive_header("Authorization"));
        assert!(sensitive_header("X-Webhook-Key"));
        assert!(sensitive_header("X-Api-Token"));
        assert!(!sensitive_header("X-Title"));
    }

    #[test]
    fn urls_get_their_secrets_masked_before_anyone_prints_them() {
        // ?key= 形态
        // 下面这条地址是**故意**构造成「像直接粘了密钥」的样子，用来测遮蔽函数本身
        let masked_query = masked_url("https://ntfy.sh/island?key=ABCDEFGHIJKLMNOP"); // nosec: 夹具，值为占位串
        assert!(!masked_query.contains("ABCDEFGHIJKLMNOP"), "{masked_query}");
        assert!(masked_query.contains("key=AB"), "留首尾便于核对自己填的是哪一条");
        // Server酱 形态：密钥在主机名首段
        let masked_host = masked_url("https://ABCDEFGHIJKLMNOPQRSTUVWXYZ012345.send/notify");
        assert!(!masked_host.contains("ABCDEFGHIJKLMNOPQRSTUVWXYZ012345"), "{masked_host}");
        // 正常域名与主题名照原样
        assert_eq!(
            masked_url("https://sctapi.ftqq.com/my-agent-island-topic"),
            "https://sctapi.ftqq.com/my-agent-island-topic"
        );
    }

    // ── 渲染 ────────────────────────────────────────────────────────────────

    #[test]
    fn the_body_carries_the_agent_name_and_only_minimal_content() {
        let mut inputs = attention();
        inputs.action_detail = Some("git push --force".into());
        inputs.message = Some("要不要继续".into());

        let quiet = render(&inputs, &ChannelConfig::default());
        assert_eq!(quiet.title, "Qoder · 等待你确认");
        assert_eq!(quiet.body, "Qoder · 等待你确认（刚刚）");
        assert!(
            !quiet.body.contains("git push"),
            "默认正文里绝不出现动作/路径/命令——这是「哪些字节离开这台机器」的唯一开关"
        );
        assert!(quiet.urgent, "等待确认是紧急的");

        let loud = ChannelConfig {
            include_action_detail: true,
            ..ChannelConfig::default()
        };
        let detailed = render(&inputs, &loud);
        assert_eq!(detailed.body, "Qoder · 等待你确认（刚刚）\n动作：git push --force\n要不要继续");
    }

    #[test]
    fn only_completed_is_non_urgent_and_it_reports_the_task_duration() {
        let done = render(&Inputs::new("Claude", EventKind::Completed, 200.0), &ChannelConfig::default());
        assert_eq!(done.title, "Claude · 任务完成");
        assert_eq!(done.body, "Claude · 任务完成（用时 3分）");
        assert!(!done.urgent, "完成不算紧急：不该在半夜用最高优先级叫醒人");
        assert_eq!(duration_short(59.0), "59秒");
        assert_eq!(duration_short(3600.0), "1小时");
        assert_eq!(duration_short(86_400.0), "1天");
    }

    #[test]
    fn action_detail_and_message_are_clipped_to_their_budgets() {
        let mut inputs = attention();
        // 80 与 120 是 Swift 侧既有的预算：正文里带的是命令片段，不该无限长
        inputs.action_detail = Some("x".repeat(200));
        inputs.message = Some("y".repeat(300));
        let rendered = render(
            &inputs,
            &ChannelConfig {
                include_action_detail: true,
                ..ChannelConfig::default()
            },
        );
        let lines: Vec<&str> = rendered.body.split('\n').collect();
        assert_eq!(lines[1], format!("动作：{}", "x".repeat(80)));
        assert_eq!(lines[2], "y".repeat(120));
    }

    // ── 请求渲染 ─────────────────────────────────────────────────────────────

    #[test]
    fn ntfy_assembles_the_documented_post_and_caps_the_body() {
        // 裸主题名 ⇒ 拼到 ntfy.sh
        let request = render_request(
            &render(&attention(), &ntfy("island")),
            Channel::Ntfy,
            &ntfy("island"),
            None,
            false,
        );
        assert_eq!(request.url, "https://ntfy.sh/island");
        assert_eq!(request.method, "POST");
        assert_eq!(request.headers[0].name, "X-Title");
        assert_eq!(request.headers[1].value, "4", "等待确认走高优先级");
        assert!(request.smtp.is_none());

        // 完整地址 ⇒ 原样用
        let full = render_request(
            &render(&attention(), &ntfy("https://ntfy.mine.local/island")),
            Channel::Ntfy,
            &ntfy("https://ntfy.mine.local/island"),
            None,
            false,
        );
        assert_eq!(full.url, "https://ntfy.mine.local/island");

        // 超长正文：**保住首行**，其余按剩余预算截断（公开服务器单条 4096 字节）
        let mut inputs = attention();
        inputs.action_detail = Some("汉".repeat(3000));
        let config = ChannelConfig {
            include_action_detail: true,
            ..ntfy("island")
        };
        let long = render_request(
            &render(&inputs, &config),
            Channel::Ntfy,
            &config,
            None,
            false,
        );
        assert!(long.body.len() <= NTFY_BYTE_LIMIT, "正文 {} 字节超限", long.body.len());
        assert!(long.body.starts_with("Qoder · 等待你确认（刚刚）"), "首行必须留住");
        assert!(long.headers[0].value.len() <= 255, "头值有 255 字节预算");
    }

    #[test]
    fn custom_http_picks_json_or_form_and_encodes_each_one_differently() {
        let inputs = attention();
        let message = render(&inputs, &ChannelConfig::default());

        let json_config = ChannelConfig {
            url_template: "https://x/y?key={key}".into(),
            ..ChannelConfig::default()
        };
        let json = render_request(&message, Channel::CustomHttp, &json_config, Some("S3cret"), false);
        assert_eq!(json.url, "https://x/y?key=S3cret", "密钥本身不编码：它本来就是 URL 的一部分");
        assert_eq!(json.headers[0].value, "application/json");
        assert_eq!(json.body, "{\"title\":\"Qoder · 等待你确认\",\"body\":\"Qoder · 等待你确认（刚刚）\"}");

        let form = render_request(
            &message,
            Channel::CustomHttp,
            &ChannelConfig {
                use_json_body: false,
                ..json_config.clone()
            },
            Some("S3cret"),
            false,
        );
        assert_eq!(form.headers[0].value, "application/x-www-form-urlencoded");
        // 表单体里空格是 `+`（不是 %20），但其余仍然百分号编码
        assert!(form.body.starts_with("title=Qoder+%C2%B7+"), "{}", form.body);
        assert!(form.body.contains("content=Qoder+"), "{}", form.body);
    }

    #[test]
    fn the_preview_masks_the_secret_instead_of_showing_it() {
        let message = render(&attention(), &ChannelConfig::default());
        let config = ChannelConfig {
            url_template: "https://x/y?key={key}".into(),
            ..ChannelConfig::default()
        };
        let request = render_request(&message, Channel::CustomHttp, &config, Some("S3cret"), true);
        let summary = request.masked_preview();
        assert!(!summary.contains("S3cret"), "预览里不许出现密钥原文：{summary}");
        assert!(summary.contains("key=S3••et"), "留首尾便于核对：{summary}");
        // 没密钥时也走掩码，而不是留一个空洞
        let without = render_request(&message, Channel::CustomHttp, &config, None, true);
        assert!(without.url.contains("key=•"));
    }

    #[test]
    fn smtp_renders_a_target_and_masks_the_password() {
        let config = ChannelConfig {
            smtp_host: "smtp.example.com".into(),
            smtp_user: "me@example.com".into(), // nosec: 测试夹具里的假邮箱（保留域名），不是真地址
            smtp_to: "you@example.com".into(), // nosec: 测试夹具里的假邮箱（保留域名），不是真地址
            ..ChannelConfig::default()
        };
        let message = render(&attention(), &config);

        // 没密钥 ⇒ 渲染不出可连的目标（只有预览可读的主题）
        let without = render_request(&message, Channel::SmtpEmail, &config, None, true);
        assert!(without.smtp.is_none());
        assert_eq!(without.method, "SMTP");

        let with = render_request(&message, Channel::SmtpEmail, &config, Some("hunter2"), true);
        let target = with.smtp.as_ref().expect("有密钥就该给出目标");
        assert_eq!((target.host.as_str(), target.port), ("smtp.example.com", 465));
        assert_eq!(target.password, "••••••", "预览里的密码恒为固定掩码");
        let summary = with.masked_preview();
        assert!(!summary.contains("hunter2"));
        assert!(summary.contains("SMTP smtp.example.com:465"));

        let real = render_request(&message, Channel::SmtpEmail, &config, Some("hunter2"), false);
        assert_eq!(real.smtp.unwrap().password, "hunter2", "真发时才是原值");
    }

    #[test]
    fn the_preview_command_input_defaults_are_sane() {
        let args = PreviewArgs::default();
        assert!(args.agent_name.is_empty(), "缺项由命令层兜底成示例值");
        let parsed: PreviewArgs =
            serde_json::from_str(r#"{"agentName":"Claude","kind":"costSpike","seconds":90}"#).unwrap();
        assert_eq!(parsed.agent_name, "Claude");
        assert_eq!(EventKind::parse(&parsed.kind), Some(EventKind::CostSpike));
        assert_eq!(EventKind::parse("nonsense"), None);
    }
}
