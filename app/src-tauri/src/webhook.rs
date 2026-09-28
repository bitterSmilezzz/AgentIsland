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

/// 令牌**目录**权限：0o700。
/// 写成 0o600 会没有执行位，里面根本创建不了文件（Swift 侧第一版就这么
/// 「修好了目录、弄坏了生成」）。
const OWNER_ONLY_DIR_MODE: u32 = 0o700;
/// 令牌文件权限：0o600。这是这个文件**唯一**的合法形态——
/// 它是「谁在说话」的全部依据。
const OWNER_ONLY_MODE: u32 = 0o600;
/// 令牌字节数（十六进制后 48 个字符）。与 Swift `randomToken()` 同长，
/// 于是两端生成的令牌互相认得。
const TOKEN_BYTES: usize = 24;

/// 令牌形态不合格的原因（与 Swift `SelfReportTokenDefect` 同名同义）。
///
/// **「读不到」与「读到的不合格」是两件事**：前者说通道没起来，
/// 后者说有人动过这个文件——两者混为一谈的话，用户会去查网络而不是查文件权限。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenDefect {
    Missing,
    /// 不是常规文件（符号链接、目录、fifo…）
    NotRegular,
    /// 属主不是当前用户
    ForeignOwner,
    /// 权限带组/其他位
    LooseMode,
    BadShape,
}

impl TokenDefect {
    pub fn label(self) -> &'static str {
        match self {
            Self::Missing => "文件不存在",
            Self::NotRegular => "不是常规文件",
            Self::ForeignOwner => "属主不是当前用户",
            Self::LooseMode => "权限带组或其他位",
            Self::BadShape => "内容不是足够长的十六进制串",
        }
    }
}

/// 验明正身：**「文件里有串就用串」在这台机器上不成立**。
///
/// `Application Support/AgentIsland/` 在首启之前归用户可写，任何本机进程都能
/// 抢先把 `report.token` 写成它已知的值，于是它拿着「令牌」就能绑一条可信自报，
/// 而我们以为是自己生成的。所以先验：常规文件、属主是当前用户、权限不带组/其他位、
/// 内容是一段足够长的十六进制。任何一条不对都当作**没有令牌**，并把原因带出去。
///
/// **诚实的边界**：这验的是**形态**不是**来源**——同一个用户、同样 0600 的合法
/// 十六进制串仍可能是抢先写好的。本机要真做到不可伪造，得把令牌放进钥匙串，
/// 并放弃「抄进第三方配置」这件事。
#[cfg(unix)]
pub fn inspect_token(path: &std::path::Path) -> Result<String, TokenDefect> {
    use std::os::unix::fs::MetadataExt;
    use std::os::unix::fs::PermissionsExt;
    // **symlink_metadata 而不是 metadata**：后者会穿过符号链接，
    // 于是「链接指向一个 0600 的真文件」会被判成合格——而那正是要挡住的形态。
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return Err(TokenDefect::Missing);
    };
    if !meta.file_type().is_file() {
        return Err(TokenDefect::NotRegular);
    }
    if meta.uid() != unsafe { libc::geteuid() } {
        return Err(TokenDefect::ForeignOwner);
    }
    if meta.permissions().mode() & 0o077 != 0 {
        return Err(TokenDefect::LooseMode);
    }
    let Ok(text) = std::fs::read_to_string(path) else {
        return Err(TokenDefect::BadShape);
    };
    let trimmed = text.trim();
    if trimmed.len() < 32 || !trimmed.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(TokenDefect::BadShape);
    }
    Ok(trimmed.to_string())
}

#[cfg(not(unix))]
pub fn inspect_token(_path: &std::path::Path) -> Result<String, TokenDefect> {
    Err(TokenDefect::Missing)
}

/// 取令牌；不合格就换一份新的。返回 `None` 表示**这次拿不到可信通道**
/// （目录建不出来、磁盘写不进），调用方必须走「拒绝可信自报」那条分支，
/// **不能当成「令牌不匹配」**——后者会让每个接入方都去重配令牌，而病根在磁盘。
pub fn ensure_token() -> Option<String> {
    let path = token_file_path();
    match inspect_token(&path) {
        Ok(existing) => return Some(existing),
        Err(TokenDefect::Missing) => {}
        Err(defect) => {
            // 「静默换掉」是这条通道最坏的失效方式：用户看到的是
            // 「昨天还好使，今天全废」，而真实原因只是有人对令牌目录跑过
            // chmod -R 或同步盘把权限写宽了。所以先把原因说出来。
            crate::log_line(&format!(
                "[webhook] report.token 形态不合格（{}），已换成新生成的令牌——接入命令需要重新配置",
                defect.label()
            ));
            // 符号链接/目录这类非常规文件要**删掉它自己**（不是它指向的目标）。
            // `remove_file` 对符号链接删的是链接本身，与 `unlink` 同义。
            let _ = std::fs::remove_file(&path);
        }
    }
    let token = random_token()?;
    write_new_token(&path, &token)?;
    Some(token)
}

/// 写一份新令牌。**`create_new` 保证不穿过任何已存在的路径**——
/// 这是与上一版最大的差别：`fs::write` 会跟着符号链接写进它指向的文件。
fn write_new_token(path: &std::path::Path, token: &str) -> Option<()> {
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;
    let dir = path.parent()?;
    std::fs::create_dir_all(dir).ok()?;
    // 目录权限显式收一次：`create_dir_all` 走 umask，默认可能是 0755
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(OWNER_ONLY_DIR_MODE)).ok()?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .ok()?;
    file.write_all(token.as_bytes()).ok()?;
    file.sync_all().ok()?;
    drop(file);
    // **写完再收一次权限**：新建文件的权限同样受 umask 影响，
    // 而 0600 是唯一合法形态
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(OWNER_ONLY_MODE)).ok()?;
    Some(())
}

/// 24 字节真随机 → 48 个十六进制字符。
///
/// **不走时钟**：上一版是 `as_nanos() * 常数 rotate`，那是一个由系统时钟推导的值，
/// 拿到令牌的进程读一次 `date +%s%N` 就能算出同一串。令牌的全部价值就是「猜不到」，
/// 用时钟播种等于把这件事直接取消。
/// 读 `/dev/urandom` 而不是引依赖：这是系统自带的熵源，各平台都有同名文件。
fn random_token() -> Option<String> {
    let mut bytes = [0u8; TOKEN_BYTES];
    let mut file = std::fs::File::open("/dev/urandom").ok()?;
    std::io::Read::read_exact(&mut file, &mut bytes).ok()?;
    Some(bytes.iter().map(|b| format!("{b:02x}")).collect())
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
            if ensure_token().is_none() {
                // 拿不到令牌 ⇒ 整条可信通道**在这台机器上**不可用。
                // 仍然起服务：拒绝可信自报的那条（401 noToken）要让接入方看得见，
                // 静默不监听的话它们只会看到「连接被拒」，猜不到是令牌的问题。
                crate::log_line(
                    "[webhook] report.token 写不下去：可信自报通道本次不可用（接入方会拿到 noToken）",
                );
            }
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
            // `None`（写不出令牌）与 `Some(别的)` 一样都算不可信：
            // 「没有可信通道」不能退化成「拿请求里的串比对」——
            // 那会让「服务端没令牌」变成「谁都能自报」。
            match ensure_token() {
                Some(expected) => token == expected && token.len() >= 32,
                None => false,
            }
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

/// 短标识（上报 / 请求 id）。
///
/// **走 `/dev/urandom` 而不是时钟**：上一版的 `as_nanos() * 常数 rotate`
/// 是由系统时钟推导的，同一进程里连着调几次会撞，而它在别处还被拿去当令牌用。
/// 这里只取 8 字节熵，熵源读不到时退回时钟——标识符撞了只是日志难读，
/// 不涉及安全，所以这条降级可以接受（**令牌那条不接受**，见 `random_token`）。
fn uuid_lite() -> String {
    let mut bytes = [0u8; 8];
    if let Ok(mut file) = std::fs::File::open("/dev/urandom") {
        let _ = std::io::Read::read_exact(&mut file, &mut bytes);
    } else {
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0) as u64;
        bytes = n.to_le_bytes();
    }
    let a = u16::from_le_bytes([bytes[0], bytes[1]]);
    let b = u16::from_le_bytes([bytes[2], bytes[3]]);
    let c = u16::from_le_bytes([bytes[4], bytes[5]]);
    let d = u16::from_le_bytes([bytes[6], bytes[7]]);
    format!("{a:04x}{b:04x}-{c:04x}-{d:04x}")
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
            token_alert_floor: None,
            session_dialect: crate::models::SessionDialect::GenericTail,
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
            session_probe_health: None,
            background_tasks: vec![],
            subagents: vec![],
            token_breakdown: None,
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

#[cfg(test)]
mod token_tests {
    use super::*;
    use crate::testutil::Sandbox;

    fn token_path(tag: &str) -> (Sandbox, std::path::PathBuf) {
        let sandbox = Sandbox::new(tag);
        let path = sandbox.path().join("report.token");
        (sandbox, path)
    }

    fn good_token() -> String {
        "0123456789abcdef0123456789abcdef0123456789abcdef".to_string()
    }

    /// 合格的一���必须原样返回——这是所有反例的前提，
    /// 少了它，「全部拒了」也能让上面每条用例变绿。
    #[test]
    fn a_well_formed_token_passes_inspection() {
        let (_sandbox, path) = token_path("token-good");
        write_new_token(&path, &good_token()).expect("应当写得下");
        assert_eq!(inspect_token(&path), Ok(good_token()));
    }

    /// **权限带组位就不认**——这一条是 v0.0.223 的重点。
    ///
    /// 上一版 `ensure_token` 从不设权限，于是同步盘 / `chmod -R` 一跑，
    /// 令牌就变成 0644；而「文件里有串就用串」意味着**任何本机用户都能读到它**，
    /// 读到就能绑一条可信自报。
    #[test]
    fn a_group_readable_token_is_refused() {
        use std::os::unix::fs::PermissionsExt;
        let (_sandbox, path) = token_path("token-loose");
        write_new_token(&path, &good_token()).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(inspect_token(&path), Err(TokenDefect::LooseMode));
    }

    /// 符号链接要被认出来，**而且不能跟着它写**。
    ///
    /// 这是上一版最危险的一处：`fs::write` 会穿过符号链接，
    /// 于是「别的路径上已经有个文件」会被就地覆盖。
    /// 这里造的是一条指向真实文件的链接，并检查它被判 `NotRegular`。
    #[test]
    fn a_symlink_is_refused_rather_than_followed() {
        let (_sandbox, path) = token_path("token-symlink");
        let victim = path.parent().unwrap().join("victim.txt");
        std::fs::write(&victim, b"important").unwrap();
        std::os::unix::fs::symlink(&victim, &path).unwrap();
        assert_eq!(
            inspect_token(&path),
            Err(TokenDefect::NotRegular),
            "链接指向一个 0600 的真文件也不该算合格"
        );
        // 关键：替换令牌时删的是**链接自己**，不是它指向的目标
        ensure_token_at(&path);
        assert_eq!(
            std::fs::read(&victim).unwrap(),
            b"important",
            "换一个不合格的令牌不该动到别人路径上的文件"
        );
    }

    /// 形状不合格的几种都要说清是哪一种，别一律当「没有令牌」。
    #[test]
    fn content_and_absence_have_distinct_reasons() {
        let (_sandbox, path) = token_path("token-shape");
        assert_eq!(inspect_token(&path), Err(TokenDefect::Missing));
        write_new_token(&path, "太短").unwrap();
        assert_eq!(inspect_token(&path), Err(TokenDefect::BadShape));
        std::fs::write(&path, "zzzz0123456789abcdef0123456789abcdef01234567").unwrap();
        assert_eq!(inspect_token(&path), Err(TokenDefect::BadShape), "非十六进制也不行");
    }

    /// 新写的令牌必须是 0600、目录 0700 —— **哪怕 umask 放得更宽**。
    #[test]
    fn a_new_token_is_owner_only_even_under_a_loose_umask() {
        use std::os::unix::fs::PermissionsExt;
        let (sandbox, path) = token_path("token-mode");
        let previous = unsafe { libc::umask(0o000) };
        write_new_token(&path, &good_token()).unwrap();
        unsafe { libc::umask(previous) };
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "宽 umask 下也必须是 0600，实际 {mode:o}");
        let dir_mode = std::fs::metadata(sandbox.path()).unwrap().permissions().mode() & 0o777;
        assert_eq!(dir_mode, 0o700, "目录必须是 0700，实际 {dir_mode:o}");
    }

    /// 令牌必须来自系统熵源，**不能由时钟推导**。
    ///
    /// 上一版是 `as_nanos() * 常数 rotate`：拿到文件的进程读一次时钟就能算出同一串，
    /// 而「令牌的全部价值就是猜不到」。这条钉住长度与字符集，
    /// 时钟播种的实现同样能满足，所以它挡不住全部退化——
    /// **诚实地说：可预测性要靠读 `/dev/urandom` 的代码本身来保证，不是靠这条用例。**
    #[test]
    fn a_generated_token_is_24_bytes_of_hex() {
        let token = random_token().expect("熵源应当可读");
        assert_eq!(token.len(), TOKEN_BYTES * 2, "48 个十六进制字符 = 24 字节");
        assert!(token.chars().all(|c| c.is_ascii_hexdigit()));
        // 两次取值应当不同（熵源坏了的话这里会红）
        assert_ne!(token, random_token().unwrap_or_default());
    }

    /// 把 `ensure_token` 的替换逻辑单独拿出来测：给定路径换一份新的。
    /// 真实 `ensure_token` 用的是配置目录，测试不能去动用户那份。
    fn ensure_token_at(path: &std::path::Path) -> String {
        if inspect_token(path).is_ok() {
            return inspect_token(path).unwrap_or_default();
        }
        let _ = std::fs::remove_file(path);
        let token = random_token().unwrap_or_else(|| good_token());
        write_new_token(path, &token).unwrap();
        token
    }
}

/// `webhook_uuid` 换过熵源（v0.0.223：时钟播种 ⇒ `/dev/urandom`），
/// 所以「两次取值不同」这件事值得单独钉住——
/// 它是那次改动**唯一**能被自动证明的部分。
#[cfg(test)]
mod uuid_tests {
    use super::*;

    #[test]
    fn two_calls_do_not_collide() {
        let a = webhook_uuid();
        let b = webhook_uuid();
        assert_ne!(a, b, "连着取两次撞了 ⇒ 熵源没有生效（时钟播种的两个相邻值极可能相同）");
        assert_eq!(a.len(), b.len());
    }

    /// 形状稳定：日志与「按 id 查事件」都依赖它。
    #[test]
    fn the_shape_is_stable() {
        let id = webhook_uuid();
        assert!(id.contains('-'), "应当保留分段：{id}");
        assert!(
            id.chars().all(|c| c.is_ascii_hexdigit() || c == '-'),
            "只该有小写十六进制与连字符：{id}"
        );
        // 两次形状一致 ⇒ 分段数不变
        assert_eq!(id.matches('-').count(), webhook_uuid().matches('-').count());
    }
}
