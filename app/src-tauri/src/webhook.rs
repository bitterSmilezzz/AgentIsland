use crate::models::AgentTaskEvent;
use std::io::Read;
use std::sync::mpsc::Sender;
use tiny_http::{Header, Method, Response, Server};

/// 本地 Webhook（127.0.0.1，协议与 macOS 端相同）：
/// · POST /notify、/event：无鉴权直推事件（岛内标「外部投递」）
/// · POST /session、DELETE /session：需令牌 X-AgentIsland-Token
///
/// **端口与 macOS 端不同**（它用 41999，见 ADR 0013）：
/// 两个应用都用一个端口时，先启动的占住、后启动的**静默失效**——而请求还会「正常返回」，
/// 只是答话的是另一个进程。迁移期两件并存，所以各用各的：
/// · 41999 是**交付物**（Swift 本体）对外的承诺，README 里写的就是它，不能动；
/// · Rust 端用 42000，第三方配置指向 41999 时说的仍然是交付物。
/// Swift 退场后（ADR 0010 的迁移收尾）把这个默认值改回 41999 即可。
pub const DEFAULT_EVENT_PORT: u16 = 42000;

/// 端口解析：环境变量优先（`AGENTISLAND_EVENT_PORT`），认不出就用默认值。
///
/// 抽成纯函数是为了可测——`std::env::set_var` 在并行测试里是进程级共享状态，
/// 拿它测等于给自己埋一个偶发。**认不出时回默认而不是 panic**：一个手滑的环境变量
/// 不该让本地入口整个起不来。
pub fn parse_event_port(raw: Option<&str>) -> u16 {
    match raw.map(str::trim) {
        Some(value) if !value.is_empty() => value.parse::<u16>().unwrap_or(DEFAULT_EVENT_PORT),
        _ => DEFAULT_EVENT_PORT,
    }
}

pub fn event_port() -> u16 {
    parse_event_port(std::env::var("AGENTISLAND_EVENT_PORT").ok().as_deref())
}
pub struct LocalEventServer {
    handle: Option<std::thread::JoinHandle<()>>,
}

pub fn token_file_path() -> std::path::PathBuf {
    crate::settings::config_dir().join("report.token")
}

pub fn ensure_token() -> String {
    let path = token_file_path();
    if let Ok(existing) = std::fs::read_to_string(&path) {
        let t = existing.trim().to_string();
        if t.len() >= 32 && t.chars().all(|c| c.is_ascii_hexdigit()) {
            return t;
        }
    }
    let token = format!("{:x}{:x}", rand_u64(), rand_u64());
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(&path, &token);
    token
}

fn rand_u64() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let n = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    (n as u64).wrapping_mul(0x9E3779B97F4A7C15).rotate_left(17)
}

impl LocalEventServer {
    pub fn start(tx: Sender<AgentTaskEvent>, engine: crate::SharedEngine) -> Self {
        let port = event_port();
        let server = match Server::http(format!("127.0.0.1:{port}")) {
            Ok(s) => s,
            Err(error) => {
                // 端口被占：岛其余功能不受影响，但**必须说出来**。
                // 这里是静默退化的高危点：Swift 本体与 Rust 端用同一个端口（41999），
                // 先启动的那个占住，后启动的本地入口就没了——而所有请求会看起来「正常返回」，
                // 只是答话的是另一个进程（我实测时就被这件事骗过一次）。
                crate::log_line(&format!(
                    "[webhook] 127.0.0.1:{port} 绑定失败（{error}）：本地 /notify、/event、/session 入口本次不可用"
                ));
                return LocalEventServer { handle: None };
            }
        };
        crate::log_line(&format!(
            "[webhook] 本地入口监听 127.0.0.1:{port}（协议与 macOS 端相同；端口不同见 ADR 0013）"
        ));
        let handle = std::thread::spawn(move || {
            ensure_token();
            for mut request in server.incoming_requests() {
                let method = request.method().clone();
                let url = request.url().to_string();
                let path = url.split('?').next().unwrap_or("/").to_string();
                let token_header = request
                    .headers()
                    .iter()
                    .find(|h| h.field.equiv("X-AgentIsland-Token"))
                    .map(|h| h.value.as_str().to_string())
                    .unwrap_or_default();

                let mut body = String::new();
                if method == Method::Post {
                    let _ = request.as_reader().read_to_string(&mut body);
                }

                // `/session` 要写引擎里的自报登记表；其余路由只用事件通道
                let (status, text) =
                    route(&path, &method, &body, &token_header, &engine, &url, &tx);
                let header = Header::from_bytes(&b"Content-Type"[..], &b"application/json; charset=utf-8"[..]).unwrap();
                let response = Response::from_string(text).with_status_code(status).with_header(header);
                let _ = request.respond(response);
            }
        });
        LocalEventServer { handle: Some(handle) }
    }
}

#[allow(clippy::too_many_arguments)]
fn route(
    path: &str,
    method: &Method,
    body: &str,
    token: &str,
    engine: &crate::SharedEngine,
    url: &str,
    tx: &Sender<AgentTaskEvent>,
) -> (u16, String) {
    use AgentTaskEvent as E;
    if path == "/session" {
        let trusted = {
            let expected = ensure_token();
            token == expected && token.len() >= 32
        };
        if !trusted {
            // 没令牌（或令牌不对）的申报**不丢**：落回无鉴权通道，带「未采信」标记。
            // 响应形状也统一成拒绝——`/session` 不能变成「这台机器装了哪些 Agent」的枚举口
            // （Swift 同一条顾虑：macOS 13 的构建路径下端口对局域网可达）。
            if *method == Method::Post {
                let _ = notify_route(body, true, tx);
            }
            return (401, r#"{"bound":false,"reason":"noToken"}"#.to_string());
        }
        let (agent_id, session_id) = resolve_session_ids(url, body);
        if *method == Method::Delete {
            let mut guard = engine.lock().unwrap();
            let removed = guard.self_reports.remove(agent_id.trim());
            return (
                200,
                format!(r#"{{"revoked":{}}}"#, if removed { "true" } else { "false" }),
            );
        }
        if *method != Method::Post {
            return (405, r#"{"error":"method not allowed"}"#.to_string());
        }
        return bind_self_report(body, &agent_id, &session_id, engine);
    }
    if (path == "/notify" || path == "/event") && *method == Method::Post {
        let (status, text) = notify_route(body, false, tx);
        return (status, text.to_string());
    }
    (404, r#"{"error":"not found"}"#.to_string())
}

fn notify_route(body: &str, untrusted: bool, tx: &Sender<AgentTaskEvent>) -> (u16, &'static str) {
    let Ok(doc) = serde_json::from_str::<serde_json::Value>(body) else {
        return (400, r#"{"error":"invalid json"}"#);
    };
    let agent = doc.get("agent").and_then(|v| v.as_str()).unwrap_or("");
    let event = doc.get("event").and_then(|v| v.as_str()).unwrap_or("completed");
    let message = doc.get("message").and_then(|v| v.as_str()).map(|s| s.to_string());
    let detail = doc.get("detail").and_then(|v| v.as_str()).map(|s| s.to_string());
    if agent.is_empty() {
        return (400, r#"{"error":"agent 必填"}"#);
    }
    let event_type = match event {
        "attention" => "attention",
        "costSpike" | "cost_spike" | "alert" => "costSpike",
        "completed" => "completed",
        _ => return (400, r#"{"error":"event 须为 completed|attention|costSpike"}"#),
    };
    let _ = tx.send(AgentTaskEvent {
        id: uuid_lite(),
        agent_id: agent.to_string(),
        agent_name: agent.to_string(),
        event_type: event_type.to_string(),
        timestamp: crate::tokens::now_ms(),
        message,
        detail,
        // 外部事件没有「本次任务用时」这个概念：0 = 不适用（报告里印 —，而不是 0 秒）
        duration: 0.0,
        externally_delivered: true,
    });
    if untrusted {
        (200, r#"{"ok":true,"delivered":"untrusted"}"#)
    } else {
        (200, r#"{"ok":true,"delivered":"external"}"#)
    }
}

fn uuid_lite() -> String {
    format!(
        "{:04x}{:04x}-{:04x}-{:04x}",
        rand_u64() as u16,
        (rand_u64() >> 16) as u16,
        (rand_u64() >> 32) as u16,
        rand_u64() as u16
    )
}

/// 供引擎生成事件 id
pub fn webhook_uuid() -> String {
    uuid_lite()
}

/// `/session` 的两个标识走**查询串**（`?agent=…&session=…`），与 Swift 侧一致。
/// Rust 没有 URL 解析库，这里手写一个够用的取值器：只认 `key=value`、`&` 分隔、
/// 百分号解码只处理 `%XX`（够 key 与简单 id 用；不处理 `+` 当空格——那是表单编码的老规矩，
/// 而这里两边都是我们自己人）。
fn session_query(url: &str) -> (String, String) {
    let mut agent = String::new();
    let mut session = String::new();
    let Some(query) = url.split_once('?').map(|(_, q)| q) else {
        return (agent, session);
    };
    for pair in query.split('&') {
        let Some((key, value)) = pair.split_once('=') else {
            continue;
        };
        let decoded = percent_decode(value);
        match key {
            "agent" => agent = decoded,
            "session" => session = decoded,
            _ => {}
        }
    }
    (agent, session)
}

/// 从请求里取 `agent` 与 `session`。**与 Swift 侧一致**：`agent` 走查询串、`session` 走正文。
///
/// 这不只是风格问题——hook 的 http 配置只会原样 POST 事件正文，主会话的 payload 里
/// 没有我们的档案 id，所以 `agent` 必须能从查询串带；而 `session` 在正文里。
/// 我第一版两个都从查询串取，于是**按 Swift 格式发过来的请求会被拒掉**——
/// 这个缺口是靠「拿真 Swift 应用对打」发现的，自己的测试发现不了。
/// 两种形式都收（查询串那份作为回落）：比 Swift 更宽容不会伤人。
pub fn resolve_session_ids(url: &str, body: &str) -> (String, String) {
    let (query_agent, query_session) = session_query(url);
    let json: Option<serde_json::Value> = serde_json::from_str(body).ok();
    let from_body = |key: &str| -> String {
        json.as_ref()
            .and_then(|value| value.get(key))
            .and_then(|value| value.as_str())
            .unwrap_or("")
            .trim()
            .to_string()
    };
    let agent = if query_agent.trim().is_empty() {
        from_body("agent")
    } else {
        query_agent
    };
    let session = {
        let in_body = from_body("session");
        if in_body.is_empty() {
            query_session
        } else {
            in_body
        }
    };
    (agent, session)
}

fn percent_decode(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).unwrap_or("");
            if let Ok(byte) = u8::from_str_radix(hex, 16) {
                out.push(byte);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

/// 带令牌的自报：校验 → 登记 → 回 `{bound, expiresAt}`（unix **秒**，与 Swift 同一形状）。
///
/// 拒绝时**给理由**（`malformed` / `unknownState` / `badPid` / `unknownAgent` / `pidMismatch`）：
/// 只回一句「失败」会让调用方去猜自己哪里写错了。
fn bind_self_report(
    body: &str,
    agent_id: &str,
    session_id: &str,
    engine: &crate::SharedEngine,
) -> (u16, String) {
    use crate::selfreport::{parse_pid, Rejection, ReportedState, Submission};
    let reject = |status: u16, reason: &str, message: &str| {
        (
            status,
            format!(
                r#"{{"bound":false,"reason":"{reason}","message":"{}"}}"#,
                message.replace('"', "'")
            ),
        )
    };

    let json: serde_json::Value = match serde_json::from_str(body) {
        Ok(value) => value,
        Err(_) => return reject(400, Rejection::Malformed.reason(), "正文不是 JSON"),
    };
    let state = json
        .get("state")
        .and_then(|v| v.as_str())
        .and_then(ReportedState::parse);
    let Some(state) = state else {
        return reject(
            400,
            Rejection::UnknownState.reason(),
            "state 只能是 working / attention / completed / idle",
        );
    };
    let pid = match parse_pid(json.get("pid")) {
        Ok(pid) => pid,
        Err(reason) => return reject(400, reason.reason(), "pid 必须是正整数或纯数字字符串"),
    };
    let submission = Submission::new(
        agent_id.to_string(),
        session_id.to_string(),
        pid,
        state,
        json.get("detail").and_then(|v| v.as_str()).map(str::to_string),
        json.get("ask").and_then(|v| v.as_str()).map(str::to_string),
        json.get("ttl").and_then(|v| v.as_i64()),
    );
    if let Err(reason) = submission.validate() {
        return reject(
            400,
            reason.reason(),
            "agent 要带在查询串上（?agent=<id>）、session 放在正文里",
        );
    }

    let mut guard = engine.lock().unwrap();
    // 档案必须存在：**不自动建档**（否则 `/session` 就成了「随便造一个 agent」的入口）
    if !crate::registry::builtin()
        .iter()
        .any(|profile| profile.id == submission.agent_id)
    {
        return reject(400, "unknownAgent", "没有这个 agent 档案");
    }
    // 自报里带了 pid，就要与这一拍实际匹配到的进程号一致：
    // 不一致说明它在替别的进程说话（Swift 的 `pidMismatch` 同义）。
    if let Some(claimed) = submission.pid {
        let actual = guard
            .snapshots
            .iter()
            .find(|snapshot| snapshot.id == submission.agent_id)
            .and_then(|snapshot| snapshot.pid);
        if let Some(actual) = actual {
            if actual as i32 != claimed {
                return reject(200, "pidMismatch", "自报的 pid 与这一拍匹配到的进程不一致");
            }
        }
    }
    let record = guard.self_reports.submit(submission, crate::tokens::now_ms());
    (
        200,
        format!(
            r#"{{"bound":true,"expiresAt":{}}}"#,
            record.expires_ms / 1000
        ),
    )
}

#[cfg(test)]
mod session_tests {
    use super::*;
    use crate::models::{ActivityLevel, AgentProfile, AgentSnapshot, TokenUsage};
    use crate::settings::Settings;
    use std::sync::{mpsc, Arc, Mutex};

    fn shared_engine() -> crate::SharedEngine {
        let (_tx, rx) = mpsc::channel();
        Arc::new(Mutex::new(crate::engine::ActivityEngine::new(
            Settings::default(),
            rx,
        )))
    }

    fn snapshot_with_pid(id: &str, pid: Option<u32>) -> AgentSnapshot {
        let profile = AgentProfile {
            bundle_ids: vec![],
            id: id.into(),
            name: "Fixture".into(),
            glyph: String::new(),
            emoji: String::new(),
            process_names: vec![],
            cmdline_hints: vec![],
            path_excludes: vec![],
            path_contains: vec![],
            cpu_floor: None,
            session_dirs: vec![],
            token_roots: vec![],
            session_database: None,
            category: "assistant".into(),
        };
        let level = ActivityLevel::Idle;
        AgentSnapshot {
            installed: None,
            id: profile.id.clone(),
            name: profile.name.clone(),
            glyph: String::new(),
            emoji: String::new(),
            level,
            level_label: level.label().into(),
            observability: crate::observability::Verdict {
                code: crate::observability::Code::Observed,
                summary: "",
                evidence: Vec::new(),
            },
            is_hung: None,
            health: crate::health::Report::not_running(),
            process_running: true,
            work_stats: crate::duration::Stats::empty(),
            provenance: None,
            provenance_suffix: String::new(),
            cpu_percent: None,
            memory_bytes: 0,
            memory_text: "—".into(),
            last_activity_text: "—".into(),
            token_usage: Some(TokenUsage::default()),
            pid,
            current_action: None,
            subagent_count: 0,
        }
    }

    #[test]
    fn a_query_without_ids_is_rejected_as_malformed() {
        let engine = shared_engine();
        let (status, body) = bind_self_report(r#"{"state":"working"}"#, "", "", &engine);
        assert_eq!(status, 400);
        assert!(body.contains(r#""reason":"malformed""#), "{body}");
    }

    #[test]
    fn an_unknown_state_or_bad_pid_is_named_in_the_rejection() {
        let engine = shared_engine();
        let (status, body) = bind_self_report(r#"{"state":"干活中"}"#, "claude", "s-1", &engine);
        assert_eq!(status, 400);
        assert!(body.contains(r#""reason":"unknownState""#), "{body}");

        let (status, body) = bind_self_report(
            r#"{"state":"working","pid":"abc"}"#,
            "claude",
            "s-1",
            &engine,
        );
        assert_eq!(status, 400);
        assert!(body.contains(r#""reason":"badPid""#), "{body}");
    }

    #[test]
    fn an_unknown_agent_is_refused_so_session_cannot_create_profiles() {
        let engine = shared_engine();
        let (status, body) = bind_self_report(
            r#"{"state":"working"}"#,
            "not-a-real-profile",
            "s-1",
            &engine,
        );
        assert_eq!(status, 400);
        assert!(body.contains(r#""reason":"unknownAgent""#), "{body}");
    }

    #[test]
    fn a_bound_report_is_registered_and_expiry_is_clamped() {
        let engine = shared_engine();
        let (status, body) = bind_self_report(
            r#"{"state":"working","detail":"  正在改 view  ","ttl":100000000}"#,
            "claude",
            "s-1",
            &engine,
        );
        assert_eq!(status, 200);
        assert!(body.contains(r#""bound":true"#), "{body}");
        let guard = engine.lock().unwrap();
        let record = guard.self_reports.record("claude").expect("应已登记");
        assert_eq!(record.state.as_str(), "working");
        assert_eq!(
            record.detail.as_deref(),
            Some("正在改 view"),
            "文本要裁空白"
        );
        // TTL 被钳到上限：一亿毫秒那种「我一直都在」不许过
        assert_eq!(record.expires_ms - record.received_ms, crate::selfreport::TTL_MAX_MS);
        // 决定显示哪一档：自报说在工作、观测是空闲且**没有**强语义 ⇒ 采信自报
        let (level, provenance) = crate::selfreport::resolve(ActivityLevel::Idle, false, Some(&record));
        assert_eq!(level, ActivityLevel::Working);
        assert_eq!(provenance, Some(crate::selfreport::Provenance::SelfReported));
        // 报表与界面拿到的就是拼好的后缀
        assert_eq!(
            crate::selfreport::Provenance::badge_suffix(provenance),
            " · 自报"
        );
    }

    #[test]
    fn a_report_claiming_someone_elses_pid_is_refused() {
        let engine = shared_engine();
        engine
            .lock()
            .unwrap()
            .snapshots
            .push(snapshot_with_pid("claude", Some(4242)));
        let (status, body) = bind_self_report(
            r#"{"state":"working","pid":9999}"#,
            "claude",
            "s-1",
            &engine,
        );
        assert_eq!(status, 200, "形状仍是 200，但 bound 为 false");
        assert!(body.contains(r#""bound":false"#), "{body}");
        assert!(body.contains(r#""reason":"pidMismatch""#), "{body}");
        assert!(
            engine.lock().unwrap().self_reports.record("claude").is_none(),
            "pid 不符的申报不许登记"
        );
        // pid 一致就放行
        let (status, body) = bind_self_report(
            r#"{"state":"idle","pid":4242}"#,
            "claude",
            "s-1",
            &engine,
        );
        assert_eq!(status, 200);
        assert!(body.contains(r#""bound":true"#), "{body}");
    }

    #[test]
    fn the_query_string_carries_the_two_ids() {
        assert_eq!(
            session_query("/session?agent=claude&session=s-1"),
            ("claude".to_string(), "s-1".to_string())
        );
        assert_eq!(
            session_query("/session?session=s-1&agent=claude"),
            ("claude".to_string(), "s-1".to_string()),
            "顺序无关"
        );
        assert_eq!(
            session_query("/session?agent=claude%2Dcode"),
            ("claude-code".to_string(), String::new()),
            "百分号解码"
        );
        assert_eq!(session_query("/session"), (String::new(), String::new()));
    }
}

#[cfg(test)]
mod port_tests {
    use super::*;

    #[test]
    fn the_event_port_defaults_away_from_the_shipped_app() {
        // 交付物（Swift 本体）用 41999，Rust 端不该去抢它
        assert_eq!(parse_event_port(None), DEFAULT_EVENT_PORT);
        assert_ne!(DEFAULT_EVENT_PORT, 41999, "Rust 端不能与交付物同端口");
        assert_eq!(parse_event_port(Some("")), DEFAULT_EVENT_PORT, "空串算没给");
        assert_eq!(parse_event_port(Some("   ")), DEFAULT_EVENT_PORT);
    }

    #[test]
    fn a_hand_written_port_is_honoured_but_garbage_falls_back_instead_of_panicking() {
        assert_eq!(parse_event_port(Some("42100")), 42100);
        assert_eq!(parse_event_port(Some(" 42100 ")), 42100, "两侧空白容错");
        // 认不出的：回默认值而不是 panic——一个手滑的环境变量不该让本地入口起不来
        for bad in ["abc", "-1", "70000", "41999abc", "1.5"] {
            assert_eq!(
                parse_event_port(Some(bad)),
                DEFAULT_EVENT_PORT,
                "{bad:?} 应回落默认值"
            );
        }
        // 合法但特殊的值照收（0 = 让内核选端口，测试里有用）
        assert_eq!(parse_event_port(Some("0")), 0);
    }
}

#[cfg(test)]
mod interop_tests {
    use super::*;
    use crate::settings::Settings;
    use std::sync::{mpsc, Arc, Mutex};

    fn shared_engine() -> crate::SharedEngine {
        let (_tx, rx) = mpsc::channel();
        Arc::new(Mutex::new(crate::engine::ActivityEngine::new(
            Settings::default(),
            rx,
        )))
    }

    /// Swift 的调用约定：`?agent=` 在查询串上、`session` 在正文里。
    /// 这条用例守的是**跨实现互操作**——按 Swift 格式发过来必须被接受，
    /// 否则第三方按文档配好也送不进来（我第一版就是这样：两个都从查询串取）。
    #[test]
    fn the_swift_wire_shape_is_understood() {
        assert_eq!(
            resolve_session_ids(
                "/session?agent=dim",
                r#"{"session":"hook-session-1","state":"working"}"#
            ),
            ("dim".to_string(), "hook-session-1".to_string())
        );
        // 更宽容的一侧：两处都给时正文为准（与 Swift 一致）
        assert_eq!(
            resolve_session_ids("/session?agent=dim&session=from-query", r#"{"session":"from-body"}"#),
            ("dim".to_string(), "from-body".to_string())
        );
        // 正文没给 session 时回落到查询串
        assert_eq!(
            resolve_session_ids("/session?agent=dim&session=from-query", r#"{"state":"idle"}"#),
            ("dim".to_string(), "from-query".to_string())
        );
        // agent 也能从正文来（我们没有这条需求，但收下不伤人）
        assert_eq!(
            resolve_session_ids("/session", r#"{"agent":"codex","session":"s"}"#),
            ("codex".to_string(), "s".to_string())
        );
        // 都没有 ⇒ 空，交给校验去报 malformed
        assert_eq!(
            resolve_session_ids("/session", "{}"),
            (String::new(), String::new())
        );
    }

    /// 真正接通一条「Swift 格式」的申报（端到端那一层由 `bind_self_report` 的用例覆盖）
    #[test]
    fn a_swift_shaped_request_binds_end_to_end() {
        let engine = shared_engine();
        let (agent, session) = resolve_session_ids(
            "/session?agent=dim",
            r#"{"session":"hook-session-1","state":"working"}"#,
        );
        let (status, body) = bind_self_report(
            r#"{"session":"hook-session-1","state":"working"}"#,
            &agent,
            &session,
            &engine,
        );
        assert_eq!(status, 200, "{body}");
        assert!(body.contains(r#""bound":true"#), "{body}");
        let guard = engine.lock().unwrap();
        let record = guard.self_reports.record("dim").expect("应已登记");
        assert_eq!(record.session_id, "hook-session-1");
    }
}
