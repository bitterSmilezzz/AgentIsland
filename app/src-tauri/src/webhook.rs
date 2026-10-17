use crate::models::AgentTaskEvent;
use crate::hook_event::{HookArrival, HookEvent};

const HTTP_IO_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);
// 只在测试里直接对 TcpStream 调 Read 方法；生产路径读的是 trait object，不需要它。
#[cfg(test)]
use std::io::Read;
use std::sync::mpsc::{Sender, SyncSender};
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
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    shutdown: Option<tiny_http::ShutdownHandle>,
}

impl Drop for LocalEventServer {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Release);
        if let Some(shutdown) = self.shutdown.take() {
            shutdown.shutdown(); // Interrupt header/body reads and response writes before join.
        }
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
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
///
/// ⚠️ **Windows 侧是显式失败，不是「跳过权限设置」。** 0600/0700 是 POSIX
/// 权限位，Windows 的 ACL 语义完全不同（没有 group/other 的等价物），
/// 拿 `set_permissions` 去凑只会得到一个**看起来设过、实际谁都能读**的文件。
/// 令牌的全部价值就是「猜不到 + 别人读不到」，所以这里宁可让调用方走
/// 「拒绝可信自报」那条分支，也不产出一个权限不明的令牌文件。
#[cfg(unix)]
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

/// Windows：**不写**。见上面的说明——产不出权限可信的令牌文件，
/// 就不该产出一个令牌文件。
#[cfg(not(unix))]
fn write_new_token(_path: &std::path::Path, _token: &str) -> Option<()> {
    None
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
    /// `wake` 是给引擎循环的唤醒信号：本地入口收到一条**真正入队**的事件后
    /// 立刻通知它，别等下一拍。
    ///
    /// 为什么需要它：引擎循环原本 `try_recv` 收完事件就 `sleep(interval)`，
    /// 而 interval 是 2 秒（有活动）到 5 秒（全闲置）。于是 hook 事件最坏要等
    /// 2~5 秒才入队，而需求要求「进入服务 → 入队 p95 ≤1 秒」——那条指标在
    /// 这个调度下**结构上不可能达标**，不是测量问题。唤醒让入队不再等下一拍。
    pub fn start(
        tx: Sender<HookEvent>,
        engine: crate::SharedEngine,
        wake: SyncSender<()>,
    ) -> Self {
        let port = event_port();
        let server = match Self::bind_http(format!("127.0.0.1:{port}")) {
            Ok(s) => s,
            Err(error) => {
                // 端口被占：岛其余功能不受影响，但**必须说出来**。
                // 这里是静默退化的高危点：Swift 本体与 Rust 端用同一个端口（41999），
                // 先启动的那个占住，后启动的本地入口就没了——而所有请求会看起来「正常返回」，
                // 只是答话的是另一个进程（我实测时就被这件事骗过一次）。
                crate::log_line(&format!(
                    "[webhook] 127.0.0.1:{port} 绑定失败（{error}）：本地 /notify、/event、/session 入口本次不可用"
                ));
                return LocalEventServer {
                    handle: None,
                    stop: Default::default(),
                    shutdown: None,
                };
            }
        };
        crate::log_line(&format!(
            "[webhook] 本地入口监听 127.0.0.1:{port}（协议与 macOS 端相同；端口不同见 ADR 0013）"
        ));
        // 令牌在**接受循环之前**准备好，失败就说出来（拿不到令牌 ⇒ 可信自报通道
        // 在这台机器上不可用；仍然起服务，让接入方看到 401 noToken 而不是「连接被拒」）。
        //
        // 放在这里而不是循环里，是为了让验收测试能拿一个**已绑定的临时端口**跑
        // `serve_bound`，从而不触碰真实配置目录（`ensure_token` 会写 report.token）。
        if ensure_token().is_none() {
            crate::log_line(
                "[webhook] report.token 写不下去：可信自报通道本次不可用（接入方会拿到 noToken）",
            );
        }
        Self::serve_bound(server, tx, engine, wake)
    }

    pub(crate) fn bind_http(addr: impl std::net::ToSocketAddrs) -> Result<Server, Box<dyn std::error::Error + Send + Sync>> {
        Server::from_listener_with_timeout(std::net::TcpListener::bind(addr)?, None, Some(HTTP_IO_TIMEOUT))
    }

    /// 接受循环本体：已绑定的 `server` + 事件通道 + 唤醒端。
    ///
    /// 与 [`Self::start`] 分开，是为了让端到端验收能绑**临时端口**（`:0`）跑真实 HTTP。
    /// 直接用 `start` 会绑配置端口（可能撞上正在运行的应用）并写真实配置目录。
    pub(crate) fn serve_bound(
        server: Server,
        tx: Sender<HookEvent>,
        engine: crate::SharedEngine,
        wake: SyncSender<()>,
    ) -> Self {
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let thread_stop = stop.clone();
        let shutdown = server.shutdown_handle();
        let handle = std::thread::spawn(move || {
            while !thread_stop.load(std::sync::atomic::Ordering::Acquire) {
                let mut request = match server.recv_timeout(std::time::Duration::from_millis(50)) {
                    Ok(Some(request)) => request,
                    Ok(None) => continue,
                    Err(_) => break,
                };
                // tiny_http 在 accept/解析前保存的时钟，包含小 body 预读和内部排队。
                let received_at = HookArrival::from(request.arrival());
                if thread_stop.load(std::sync::atomic::Ordering::Acquire) { break; }
                let method = request.method().clone();
                let url = request.url().to_string();
                let path = url.split('?').next().unwrap_or("/").to_string();
                // **每收到一个请求都记一行。**
                //
                // 这条不是为了排查服务本身，而是为了给「webview 里的 JS 到底跑没跑」
                // 提供一个**不经过 Tauri IPC** 的观测点：`log_from_ui` 走的是
                // `__TAURI__.core.invoke`，而 invoke 正是可能失败的那一环，
                // 用它证明「JS 跑过」是循环论证。走 HTTP 就完全独立了——
                // 探针页 fetch 一下本机端口，这里就会留下一行。
                crate::log_line(&format!("[http] 收到 {method} {path}"));

                let token_header = request
                    .headers()
                    .iter()
                    .find(|h| h.field.equiv("X-AgentIsland-Token"))
                    .map(|h| h.value.as_str().to_string())
                    .unwrap_or_default();

                let mut body = String::new();
                if method == Method::Post && request.as_reader().read_to_string(&mut body).is_err() {
                    // Never route a partial/invalid UTF-8 body, even when its prefix is valid JSON.
                    let _ = request.respond(Response::from_string(r#"{"error":"incomplete request body"}"#).with_status_code(400));
                    continue;
                }
                if thread_stop.load(std::sync::atomic::Ordering::Acquire) { break; }

                // `/session` 要写引擎里的自报登记表；其余路由只用事件通道
                let (status, text) = route(
                    &path,
                    &method,
                    &body,
                    &token_header,
                    &engine,
                    &url,
                    &tx,
                    &wake,
                    received_at,
                );
                let header = Header::from_bytes(
                    &b"Content-Type"[..],
                    &b"application/json; charset=utf-8"[..],
                )
                .unwrap();
                let response = Response::from_string(text)
                    .with_status_code(status)
                    .with_header(header);
                let _ = request.respond(response);
            }
        });
        LocalEventServer {
            handle: Some(handle),
            stop,
            shutdown: Some(shutdown),
        }
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
    event_tx: &Sender<HookEvent>,
    wake: &SyncSender<()>,
    received_at: HookArrival,
) -> (u16, String) {
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
                let (status, text) = notify_route(body, true, event_tx, wake, received_at);
                if status == 400 { return (status, text.to_string()); }
            }
            return (401, r#"{"bound":false,"reason":"noToken"}"#.to_string());
        }
        let (agent_id, session_id) = resolve_session_ids(url, body);
        if *method == Method::Delete {
            let mut guard = engine.lock().unwrap();
            let removed = guard.self_reports.remove(agent_id.trim());
            return (
                200,
                format!(
                    r#"{{"revoked":{}}}"#,
                    if removed { "true" } else { "false" }
                ),
            );
        }
        if *method != Method::Post {
            return (405, r#"{"error":"method not allowed"}"#.to_string());
        }
        return bind_self_report(body, &agent_id, &session_id, engine);
    }
    if (path == "/notify" || path == "/event") && *method == Method::Post {
        let (status, text) = notify_route(body, false, event_tx, wake, received_at);
        return (status, text.to_string());
    }
    (404, r#"{"error":"not found"}"#.to_string())
}

/// Normalize a client-supplied event kind. The single source of truth for the
/// `/notify` contract: `notify_route` rejects anything `None` with 400, and the CLI
/// uses the same normalization so it can refuse a bad `--kind` locally instead of
/// sending it and then printing success. `notify_ledger::normalized_kind` keeps a
/// separate, deliberately lossy projection for persisted identities (it folds unknown
/// kinds to `completed`); this one must NOT fold, because a caller needs to be told
/// its kind was rejected.
pub(crate) fn normalize_event_kind(kind: &str) -> Option<&'static str> {
    match kind {
        "attention" => Some("attention"),
        "costSpike" | "cost_spike" | "alert" => Some("costSpike"),
        "completed" => Some("completed"),
        // NOT-01 要求的事件类型：提问、失败、审查、调度、Chief 消息与分配。
        // 这里**不能**折叠未知值——调用方要被告知它的类型被拒了（见上面注释）。
        "question" => Some("question"),
        "failure" => Some("failure"),
        "review" => Some("review"),
        "schedule" => Some("schedule"),
        "chief" => Some("chief"),
        _ => None,
    }
}

/// `/notify` 拒绝未知类型时返回的响应体，类型清单从 `remote::EventKind::ALL` 派生。
/// 手写字面量会随新增类型漂移，而这句话正是调用方唯一的自查依据。
pub(crate) fn invalid_event_kind_body() -> &'static str {
    static BODY: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    BODY.get_or_init(|| {
        let list = crate::remote::EventKind::ALL
            .iter()
            .map(|kind| kind.as_str())
            .collect::<Vec<_>>()
            .join("|");
        format!(r#"{{"error":"event 须为 {list}"}}"#)
    })
    .as_str()
}

pub(crate) fn notify_route(
    body: &str,
    untrusted: bool,
    tx: &Sender<HookEvent>,
    wake: &SyncSender<()>,
    received_at: HookArrival,
) -> (u16, &'static str) {
    let Ok(doc) = serde_json::from_str::<serde_json::Value>(body) else {
        return (400, r#"{"error":"invalid json"}"#);
    };
    let client_id = match doc.get("id") {
        None => None,
        Some(serde_json::Value::String(id)) if crate::notify_ledger::valid_id(id) => Some(id.clone()),
        Some(_) => return (400, r#"{"error":"invalid id"}"#),
    };
    let agent = doc.get("agent").and_then(|v| v.as_str()).unwrap_or("");
    let event = doc
        .get("event")
        .and_then(|v| v.as_str())
        .unwrap_or("completed");
    let message = doc
        .get("message")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let detail = doc
        .get("detail")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    if agent.is_empty() {
        return (400, r#"{"error":"agent 必填"}"#);
    }
    let Some(event_type) = normalize_event_kind(event) else {
        // 文案从 `EventKind::ALL` **派生**，不再手写字面量：手写的那份会随新增类型漂移，
        // 而调用方靠这句话才知道自己能发什么（本轮就漂移过一次）。
        return (400, invalid_event_kind_body());
    };
    if tx.send(HookEvent {
        client_id,
        received_at: received_at.instant,
        event: AgentTaskEvent {
        id: uuid_lite(),
        agent_id: agent.to_string(),
        agent_name: agent.to_string(),
        event_type: event_type.to_string(),
        // 到达墙钟用于事件展示，延迟使用 envelope 携带的单调时钟。
        timestamp: received_at.timestamp_ms,
        message,
        detail,
        // 外部事件没有「本次任务用时」这个概念：0 = 不适用（报告里印 —，而不是 0 秒）
        duration: 0.0,
        externally_delivered: true,
    }}).is_err() {
        return (503, r#"{"error":"event consumer unavailable"}"#);
    }
    // 容量 1 的槽合并唤醒；Full 表示已有待处理信号，绝不阻塞 HTTP。
    let _ = wake.try_send(());
    if untrusted {
        (200, r#"{"ok":true,"accepted":"untrusted"}"#)
    } else {
        (200, r#"{"ok":true,"accepted":"external"}"#)
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
        json.get("detail")
            .and_then(|v| v.as_str())
            .map(str::to_string),
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
    let record = guard
        .self_reports
        .submit(submission, crate::tokens::now_ms());
    (
        200,
        format!(
            r#"{{"bound":true,"expiresAt":{}}}"#,
            record.expires_ms / 1000
        ),
    )
}

#[cfg(test)]
#[path = "webhook_ingress_tests.rs"]
mod ingress_tests;

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
        assert_eq!(
            record.expires_ms - record.received_ms,
            crate::selfreport::TTL_MAX_MS
        );
        // 决定显示哪一档：自报说在工作、观测是空闲且**没有**强语义 ⇒ 采信自报
        let (level, provenance) =
            crate::selfreport::resolve(ActivityLevel::Idle, false, Some(&record));
        assert_eq!(level, ActivityLevel::Working);
        assert_eq!(
            provenance,
            Some(crate::selfreport::Provenance::SelfReported)
        );
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
            engine
                .lock()
                .unwrap()
                .self_reports
                .record("claude")
                .is_none(),
            "pid 不符的申报不许登记"
        );
        // pid 一致就放行
        let (status, body) =
            bind_self_report(r#"{"state":"idle","pid":4242}"#, "claude", "s-1", &engine);
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

    // ── 结构化 hook 进入服务 → 通知入队的延迟（需求「即时通知指标」）────────────
    //
    // 这些测试走**真实链路**（`route` → channel → `push_hook_event`），不是直接往
    // 记录器里塞数字：延迟由「请求到达时刻」与「入队时刻」两个不同调用点决定，
    // 只测 `NotifyLatency` 本身会漏掉「谁在什么时候调它」。
    //
    // 路由单测钉住来源与时间戳契约；下方真实 TCP 验收使用生产 HookScheduler，
    // 临时端口绕开 start 的真实令牌准备，且显式关闭外发。

    /// 把一次请求送进 `route`，再消费 channel 让引擎按 hook 路径入队。
    fn deliver(body: &str, path: &str, received_at: HookArrival) -> crate::SharedEngine {
        let (tx, rx) = mpsc::channel();
        let (wake_tx, wake_rx) = mpsc::sync_channel::<()>(1);
        let engine: crate::SharedEngine = Arc::new(Mutex::new(
            crate::engine::ActivityEngine::new(Settings::default(), rx),
        ));
        let (status, _) = route(
            path,
            &Method::Post,
            body,
            "",
            &engine,
            path,
            &tx,
            &wake_tx,
            received_at,
        );
        assert_eq!(status, 200, "该请求应当被接受（路径 {path}）");
        // 真实入口会在发送事件后唤醒引擎循环；这里必须同样消费唤醒信号，
        // 否则测试无法发现「忘了唤醒」这种回归。
        assert!(
            wake_rx.try_recv().is_ok(),
            "入队一条事件后必须唤醒引擎循环，否则它要等满一拍"
        );
        engine.lock().unwrap().consume_hook_events();
        engine
    }

    #[test]
    fn a_delivered_event_records_the_hook_to_queue_latency() {
        // 单调起点提前 40ms；真实调度可能增加耗时，因此只约束下界。
        // 精确统计公式由 `notify_latency` 的纯单测钉住。
        let received_at = HookArrival::before(40);
        let engine = deliver(r#"{"agent":"dim","event":"completed"}"#, "/notify", received_at);
        let guard = engine.lock().unwrap();
        let samples = guard.notify_latency.samples();
        assert_eq!(samples, 1, "hook 事件必须留下一个样本");
        assert_eq!(guard.notify_latency.discarded(), 0, "不该有丢弃");
        let p95 = guard.notify_latency.p95_ms().expect("有一个样本就有 p95");
        assert!(p95 >= 40, "延迟至少是 40ms 的起点差（实测 {p95}ms）");
    }

    #[test]
    fn the_event_path_also_feeds_the_same_metric() {
        // `/event` 与 `/notify` 走同一段入队逻辑，两条入口都要被测量。
        let engine = deliver(
            r#"{"agent":"dim","event":"attention"}"#,
            "/event",
            HookArrival::before(10),
        );
        let guard = engine.lock().unwrap();
        assert_eq!(guard.notify_latency.samples(), 1, "/event 也必须记延迟");
    }

    #[test]
    fn a_rejected_request_neither_sends_an_event_nor_records_a_sample() {
        let (tx, rx) = mpsc::channel();
        let (wake_tx, wake_rx) = mpsc::sync_channel::<()>(1);
        let engine: crate::SharedEngine = Arc::new(Mutex::new(
            crate::engine::ActivityEngine::new(Settings::default(), rx),
        ));
        let (status, _) = route(
            "/notify",
            &Method::Post,
            r#"{"agent":""}"#,
            "",
            &engine,
            "/notify",
            &tx,
            &wake_tx,
            HookArrival::before(0),
        );
        assert_eq!(status, 400, "agent 必填");
        // **必须检查 channel 本身**：只断言 samples()==0 时，即使回归成
        // 「先错误地发送事件、再返回 400」，引擎没消费事件也照样通过——
        // 那是个用错误理由通过的测试。
        assert!(
            matches!(engine.lock().unwrap().event_rx.as_ref().map(|r| r.try_recv()),
                     Some(Err(std::sync::mpsc::TryRecvError::Empty))),
            "被拒请求不得发送任何事件"
        );
        assert!(wake_rx.try_recv().is_err(), "没有事件就不该唤醒引擎");
        assert_eq!(engine.lock().unwrap().notify_latency.samples(), 0);
    }

    // 即使断言失败也等待 consumer 退出；最大仅一个受控的 5s 采样周期。
    struct TestConsumer(Option<std::thread::JoinHandle<()>>);
    impl Drop for TestConsumer {
        fn drop(&mut self) {
            if let Some(handle) = self.0.take() {
                handle.join().expect("生产 consumer 线程正常退出");
            }
        }
    }

    fn http_latency_run(wake_enabled: bool) -> crate::models::EngineState {
        use std::io::Write;
        use std::time::{Duration, Instant};
        const REQUESTS: usize = 20;
        let server = LocalEventServer::bind_http("127.0.0.1:0").expect("绑定临时端口");
        let addr = match server.server_addr() {
            tiny_http::ListenAddr::IP(addr) => addr,
            other => panic!("临时端口不是 IP: {other:?}"),
        };
        let (tx, rx) = mpsc::channel();
        let (wake_tx, wake_rx) = mpsc::sync_channel::<()>(1);
        // 反例只切断 server → consumer 唤醒连线，consumer 与请求代码完全一致。
        // 保留真实 sender，避免以 Disconnected 分支代替正常的 5s 等待。
        let (mut server_wake, unused_wake_rx) = mpsc::sync_channel::<()>(1);
        if wake_enabled {
            server_wake = wake_tx.clone();
        }
        let mut activity = crate::engine::ActivityEngine::new(Settings::default(), rx);
        activity.disable_outbound();
        let engine: crate::SharedEngine = Arc::new(Mutex::new(activity));
        let server = LocalEventServer::serve_bound(server, tx, engine.clone(), server_wake);
        let (waiting_tx, waiting_rx) = mpsc::sync_channel(0);
        let (done_tx, done_rx) = mpsc::channel();
        let shared = engine.clone();
        let consumer = TestConsumer(Some(std::thread::spawn(move || {
            let scheduler = crate::engine::HookScheduler::new(wake_rx);
            let sample_at = Instant::now() + Duration::from_secs(5);
            let mut first_wait = true;
            loop {
                let state = scheduler.wait_and_consume(&shared, sample_at, || {
                    if first_wait {
                        // 来自实际等待点的握手；测试拿到信号后才可发送请求。
                        waiting_tx.send(()).unwrap();
                    }
                });
                first_wait = false;
                match state {
                    Some(state) if state.notify_latency.samples >= REQUESTS => {
                        done_tx.send(state).unwrap();
                        return;
                    }
                    Some(_) => {}
                    None => {
                        done_tx.send(shared.lock().unwrap().state()).unwrap();
                        return;
                    }
                }
            }
        })));
        waiting_rx.recv_timeout(Duration::from_secs(2)).expect("consumer 已进入长等待");
        // 证明等待未占引擎锁；真实请求不需要用锁驱动 consumer。
        assert!(engine.try_lock().is_ok(), "所有阻塞等待必须在引擎锁外");
        let started = Instant::now();
        for index in 0..REQUESTS {
            let body = format!(r#"{{"agent":"latency-fixture","event":"completed","message":"request-{index}"}}"#);
            let path = if index % 2 == 0 { "/notify" } else { "/event" };
            let request = format!(
                "POST {path} HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let mut stream = std::net::TcpStream::connect_timeout(&addr, Duration::from_secs(1)).expect("连接本地入口");
            stream.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
            stream.set_write_timeout(Some(Duration::from_secs(1))).unwrap();
            stream.write_all(request.as_bytes()).expect("发送请求");
            let mut response = String::new();
            stream.read_to_string(&mut response).expect("读取响应");
            assert!(response.starts_with("HTTP/1.1 200 "), "事件应被接受：{response}");
        }
        // 请求必须在长等待的早期送完，反例才真正在验证等待延迟。
        assert!(started.elapsed() < Duration::from_secs(1), "20 条本机请求应在 1s 内送完");
        let state = done_rx.recv_timeout(Duration::from_secs(6)).expect("生产 consumer 自动入队");
        drop(consumer); // join consumer
        drop(server); // 停止接受循环并 join server
        drop((wake_tx, unused_wake_rx));
        assert_eq!(state.notify_latency.samples, REQUESTS);
        assert_eq!(state.notify_latency.discarded, 0);
        assert!(state.notify_latency.verdict_available, "至少 20 个样本才允许判定");
        assert_eq!(state.pending_events + usize::from(state.latest_event.is_some()), REQUESTS);
        state
    }

    /// 真实 TCP → HTTP 解析 → 生产等待/consumer → EngineState，不执行全量 tick。
    #[test]
    fn a_real_http_request_reaches_the_queue_within_the_target() {
        let state = http_latency_run(true);
        eprintln!("wake enabled: {:?}", state.notify_latency);
        assert_eq!(state.notify_latency.meets_target, Some(true));
        assert!(state.notify_latency.p95_ms.unwrap() <= crate::notify_latency::TARGET_P95_MS);
    }

    #[test]
    fn real_http_without_wake_fails_the_latency_target() {
        let state = http_latency_run(false);
        eprintln!("wake disabled: {:?}", state.notify_latency);
        assert_eq!(state.notify_latency.meets_target, Some(false));
        assert!(state.notify_latency.p95_ms.unwrap() > crate::notify_latency::TARGET_P95_MS);
    }

    #[test]
    fn hook_wakes_coalesce_without_blocking_or_losing_events() {
        let (tx, rx) = mpsc::channel();
        let (wake_tx, wake_rx) = mpsc::sync_channel(1);
        let mut engine = crate::engine::ActivityEngine::new(Settings::default(), rx);
        engine.disable_outbound();
        for index in 0..20 {
            let body = format!(r#"{{"agent":"fixture","id":"wake-{index}"}}"#);
            assert_eq!(notify_route(&body, false, &tx, &wake_tx, HookArrival::before(0)).0, 200);
        }
        assert!(wake_rx.try_recv().is_ok());
        assert!(matches!(wake_rx.try_recv(), Err(mpsc::TryRecvError::Empty)), "20 次投递最多积存 1 枚唤醒");
        assert_eq!(engine.consume_hook_events(), 20, "合并的是唤醒，不能丢事件");
        assert_eq!(engine.state().notify_latency.samples, 20);
    }

    #[test]
    fn empty_hook_wakes_preserve_the_full_sample_deadline() {
        use std::time::{Duration, Instant};
        let (_tx, rx) = mpsc::channel();
        let engine = Arc::new(Mutex::new(crate::engine::ActivityEngine::new(Settings::default(), rx)));
        let (wake_tx, wake_rx) = mpsc::sync_channel(1);
        wake_tx.try_send(()).unwrap();
        let scheduler = crate::engine::HookScheduler::new(wake_rx);
        let started = Instant::now();
        let sample_at = started + Duration::from_millis(80);
        let result = scheduler.wait_and_consume(&engine, sample_at, || {
            assert!(engine.try_lock().is_ok(), "等待点必须释放引擎锁");
        });
        assert!(result.is_none(), "空唤醒不发布状态或提前触发全量采样");
        assert!(Instant::now() >= sample_at, "必须保留原采样期限");
        assert_eq!(engine.lock().unwrap().state().notify_latency.samples, 0);
    }

    #[test]
    fn disconnected_hook_wakes_wait_until_the_sample_deadline() {
        use std::time::{Duration, Instant};
        let (_tx, rx) = mpsc::channel();
        let engine = Arc::new(Mutex::new(crate::engine::ActivityEngine::new(Settings::default(), rx)));
        let (wake_tx, wake_rx) = mpsc::sync_channel(1);
        drop(wake_tx);
        let sample_at = Instant::now() + Duration::from_millis(80);
        assert!(crate::engine::HookScheduler::new(wake_rx)
            .wait_and_consume(&engine, sample_at, || {})
            .is_none());
        assert!(Instant::now() >= sample_at, "入口没启动时不能空转");
    }

    #[test]
    fn unavailable_hook_consumer_returns_failure_without_waking() {
        let (tx, rx) = mpsc::channel();
        let (wake_tx, wake_rx) = mpsc::sync_channel(1);
        drop(rx);
        assert_eq!(notify_route(r#"{"agent":"fixture"}"#, false, &tx, &wake_tx, HookArrival::before(0)).0, 503);
        assert!(matches!(wake_rx.try_recv(), Err(mpsc::TryRecvError::Empty)));
    }

    #[test]
    fn non_hook_events_do_not_feed_the_hook_metric() {
        // 这条钉住的是**来源隔离**（独立审查的 P1）：延迟指标只统计「结构化 hook
        // 进入本地入口」那一段。深链事件（`main.rs` 的 `handle_deep_link`）与引擎
        // 内部事件（预算告警）都走 `push_event` 且**也**把 `externally_delivered`
        // 置 true，但它们根本没经过 HTTP。
        //
        // 若判据退化成 `if event.externally_delivered`，批量深链会灌进一批近零样本，
        // 把真实慢样本挤出 p95 —— 指标看起来达标，而那正是它要回答的问题。
        let (_tx, rx) = mpsc::channel();
        let engine: crate::SharedEngine = Arc::new(Mutex::new(
            crate::engine::ActivityEngine::new(Settings::default(), rx),
        ));
        {
            let mut guard = engine.lock().unwrap();
            // 深链形状：externally_delivered = true，timestamp 是当时的 now。
            guard.push_event(AgentTaskEvent {
                id: "deeplink-1".into(),
                agent_id: "dim".into(),
                agent_name: "dim".into(),
                event_type: "completed".into(),
                timestamp: crate::tokens::now_ms(),
                message: None,
                detail: None,
                duration: 0.0,
                externally_delivered: true,
            });
            // 引擎内部形状：预算告警，timestamp 是判定时刻（早 999ms）。
            guard.push_event(AgentTaskEvent {
                id: "internal-1".into(),
                agent_id: "system".into(),
                agent_name: "Token 预算".into(),
                event_type: "costSpike".into(),
                timestamp: crate::tokens::now_ms() - 999,
                message: None,
                detail: None,
                duration: 0.0,
                externally_delivered: false,
            });
        }
        let guard = engine.lock().unwrap();
        assert_eq!(
            guard.notify_latency.samples(),
            0,
            "只有本地入口入队的那条才算「hook 进入服务」；深链与内部事件都不得计入"
        );
    }

    #[test]
    fn wall_clock_changes_do_not_corrupt_monotonic_hook_latency() {
        for timestamp_ms in [i64::MIN, i64::MAX] {
            let mut arrival = HookArrival::before(40);
            arrival.timestamp_ms = timestamp_ms;
            let engine = deliver(r#"{"agent":"dim","event":"completed"}"#, "/notify", arrival);
            let guard = engine.lock().unwrap();
            assert_eq!(guard.notify_latency.samples(), 1);
            assert_eq!(guard.notify_latency.discarded(), 0);
            assert!(guard.notify_latency.p95_ms().unwrap() >= 40);
            assert_eq!(guard.latest_event.as_ref().unwrap().timestamp, timestamp_ms);
        }
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
            resolve_session_ids(
                "/session?agent=dim&session=from-query",
                r#"{"session":"from-body"}"#
            ),
            ("dim".to_string(), "from-body".to_string())
        );
        // 正文没给 session 时回落到查询串
        assert_eq!(
            resolve_session_ids(
                "/session?agent=dim&session=from-query",
                r#"{"state":"idle"}"#
            ),
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

// POSIX permissions and /dev/urandom are the supported trusted-token path.
#[cfg(all(test, unix))]
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
        assert_eq!(
            inspect_token(&path),
            Err(TokenDefect::BadShape),
            "非十六进制也不行"
        );
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
        let dir_mode = std::fs::metadata(sandbox.path())
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
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

#[cfg(all(test, not(unix)))]
mod unsupported_token_tests {
    use super::*;

    #[test]
    fn unsupported_token_storage_refuses_without_writing_or_replacing_files() {
        let sandbox = crate::testutil::Sandbox::new("unsupported-token");
        let missing = sandbox.path().join("new-directory").join("report.token");
        assert_eq!(write_new_token(&missing, "synthetic"), None);
        assert!(!missing.parent().unwrap().exists());
        let existing = sandbox.path().join("existing.token");
        std::fs::write(&existing, b"unchanged").unwrap();
        assert_eq!(write_new_token(&existing, "synthetic"), None);
        assert_eq!(std::fs::read(&existing).unwrap(), b"unchanged");
        assert!(inspect_token(&existing).is_err());
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
        assert_ne!(
            a, b,
            "连着取两次撞了 ⇒ 熵源没有生效（时钟播种的两个相邻值极可能相同）"
        );
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
