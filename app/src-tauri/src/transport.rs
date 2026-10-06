//! 外发传输：把 [`Request`] 真的送出去。
//!
//! 对齐 Swift `HTTPTransport`：固定 10 秒超时（外发在后台跑，绝不能让一次卡住的请求
//! 拖住事件管线）、**拒绝重定向**（URL 里带着 SendKey/token，跟着 302 走等于把凭据
//! 发给一个用户从没配置过的主机）、以及状态码的可重试判据。
//!
//! **https 走系统 TLS 栈**（`native-tls`：macOS = Security.framework，与 Swift 侧
//! URLSession 同一套），明文与 TLS 共用同一段往返代码。SMTP over 465 的会话还没写，
//! 那条路**如实报「未接入」**，而不是把请求静默丢掉——
//! 「开关开着却永远收不到」是本仓最难查的那类失效。

use crate::notifier::{Outcome, Transport};
use crate::render::Request;
use native_tls::TlsConnector;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

/// 固定 10 秒：与 Swift `req.timeoutInterval = 10` 同值
pub const TIMEOUT: Duration = Duration::from_secs(10);

/// 读写流：明文 TCP 与 TLS 都走这一层，于是 HTTP 的往返代码不必分两套
trait ReadWrite: Read + Write + Send {}
impl<T: Read + Write + Send> ReadWrite for T {}

pub struct HttpTransport {
    /// 系统 TLS 连接器。取不到就如实报错（而不是 panic）——
    /// 它是进程级资源，初始化失败说明这台机器上没法出 https
    connector: Option<TlsConnector>,
}

impl Default for HttpTransport {
    fn default() -> Self {
        HttpTransport::new()
    }
}

impl HttpTransport {
    pub fn new() -> Self {
        HttpTransport {
            connector: TlsConnector::new().ok(),
        }
    }

    /// 注入连接器（测试用它信任本地自签证书，从而**离线**验证 HTTPS 真能连上）
    #[cfg(test)]
    pub fn with_connector(connector: TlsConnector) -> Self {
        HttpTransport {
            connector: Some(connector),
        }
    }

    /// 连上目标：http 直接返回 TCP，https 再叠一层 TLS。
    ///
    /// **逐个地址试**，而不是只用解析出的第一个：双栈主机（`localhost` → `::1` 与 `127.0.0.1`）
    /// 只试第一个会在服务只监听另一族时直接失败。Swift 侧 URLSession 有 happy-eyeballs，
    /// 这里是最小等价物。
    fn connect(&self, url: &Parsed) -> Result<Box<dyn ReadWrite>, String> {
        let addresses: Vec<_> = (url.host.as_str(), url.port)
            .to_socket_addrs()
            .map_err(|error| format!("地址解析失败：{error}"))?
            .collect();
        if addresses.is_empty() {
            return Err("地址解析为空".to_string());
        }
        let mut last_error = String::new();
        for address in &addresses {
            let stream = match TcpStream::connect_timeout(address, TIMEOUT) {
                Ok(stream) => stream,
                Err(error) => {
                    last_error = format!("连接失败：{error}");
                    continue;
                }
            };
            let _ = stream.set_read_timeout(Some(TIMEOUT));
            let _ = stream.set_write_timeout(Some(TIMEOUT));
            if url.scheme != "https" {
                return Ok(Box::new(stream));
            }
            let Some(connector) = &self.connector else {
                return Err("系统 TLS 初始化失败".to_string());
            };
            // 主机名交给 TLS 栈做校验：证书对不上就该失败，而不是静默降级
            let tls = connector
                .connect(&url.host, stream)
                .map_err(|error| format!("TLS 握手失败：{error}"))?;
            return Ok(Box::new(tls));
        }
        Err(last_error)
    }
}

/// 这个状态码值不值得再试一次。
///
/// 4xx（除 408 请求超时 / 425 数据未就绪 / 429 限流）与 3xx 是「对方看懂了但拒绝」：
/// 主题不存在、token 无效、要求订阅——再发一次也是同一个结果。而且 ntfy.sh 这类公开
/// 中转按条数限流，白重试会让用户更快撞上上限。5xx 与未知一律按可重试：那是服务端的临时状况。
pub fn is_permanent(status: u16) -> bool {
    match status {
        300..=399 => true,
        400..=499 => !matches!(status, 408 | 425 | 429),
        _ => false,
    }
}

struct Parsed {
    scheme: String,
    host: String,
    port: u16,
    path: String,
}

/// Reuse the URL parser already in Tauri's dependency graph: IPv6, escaping,
/// queries and fragments share one implementation; userinfo remains unsupported.
fn parse_url(address: &str) -> Option<Parsed> {
    if address.chars().any(|ch| ch.is_control()) {
        return None;
    }
    let url = url::Url::parse(address).ok()?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return None;
    }
    let host = match url.host()? {
        url::Host::Domain(domain) => domain.to_string(),
        url::Host::Ipv4(address) => address.to_string(),
        url::Host::Ipv6(address) => address.to_string(),
    };
    let mut path = url.path().to_string();
    if let Some(query) = url.query() {
        path.push('?');
        path.push_str(query);
    }
    Some(Parsed {
        scheme: url.scheme().into(),
        host,
        port: url.port_or_known_default()?,
        path,
    })
}

impl Transport for HttpTransport {
    fn perform(&self, request: &Request) -> Outcome {
        // SMTP 走另一条会话（Swift 的 `HTTPTransport` 也是这么分的：先看 smtp 字段）
        if request.method == "SMTP" || request.smtp.is_some() {
            let Some(target) = &request.smtp else {
                // 没有目标 = 渲染时就缺密钥（钥匙串未接入）：闸门本该拦住，
                // 真走到这里就如实说清楚，而不是拿空凭据去撞对端
                return Outcome::Failed {
                    reason: "SMTP 目标缺失（通道未配齐密钥）".into(),
                    permanent: true,
                };
            };
            let subject = request
                .headers
                .iter()
                .find(|field| field.name == "Subject")
                .map(|field| field.value.clone())
                .unwrap_or_default();
            return match crate::smtp::TlsSession::connect(target) {
                Err(reason) => Outcome::Failed {
                    reason,
                    permanent: false,
                },
                Ok(mut session) => crate::smtp::run(
                    &mut session,
                    target,
                    &subject,
                    &request.body,
                    crate::tokens::now_ms(),
                ),
            };
        }
        let Some(url) = parse_url(&request.url) else {
            return Outcome::Failed {
                reason: "地址无法解析".into(),
                permanent: false,
            };
        };
        match self.round_trip(&url, request) {
            Ok((status, body)) => {
                if (200..300).contains(&status) {
                    // 注意语义：这是「对方接受了这条请求」。多数中转服务即使内部失败
                    // 也回 200 + 一段错误 JSON，本层不去猜——设置页的文案对此写明
                    if let Some(channel) = request.response_check {
                        crate::im::response(channel, &body)
                    } else {
                        Outcome::Delivered
                    }
                } else if (300..400).contains(&status) {
                    Outcome::Failed {
                        reason: format!(
                            "服务端返回 {status}：目标回了重定向，本 App 不跟着走（密钥不外送给第三方）"
                        ),
                        permanent: true,
                    }
                } else {
                    Outcome::Failed {
                        reason: format!("服务端返回 {status}"),
                        permanent: is_permanent(status),
                    }
                }
            }
            Err(reason) => Outcome::Failed {
                reason,
                permanent: false,
            },
        }
    }
}

impl HttpTransport {
    fn round_trip(&self, url: &Parsed, request: &Request) -> Result<(u16, Vec<u8>), String> {
        let mut stream = self.connect(url)?;

        let host = if url.host.contains(':') {
            format!("[{}]", url.host)
        } else {
            url.host.clone()
        };
        let host_header = if (url.scheme == "http" && url.port == 80)
            || (url.scheme == "https" && url.port == 443)
        {
            host
        } else {
            format!("{host}:{}", url.port)
        };
        // `Connection: close` 让对端在响应后关连接，于是 BufReader 能直接读到状态行，
        // 不必实现 chunked 解码——我们只要状态码
        let mut head = format!(
            "{} {} HTTP/1.1\r\nHost: {host_header}\r\nConnection: close\r\nContent-Length: {}\r\n",
            request.method,
            url.path,
            request.body.len()
        );
        for field in &request.headers {
            head.push_str(&format!("{}: {}\r\n", field.name, field.value));
        }
        head.push_str("\r\n");
        stream
            .write_all(head.as_bytes())
            .and_then(|_| stream.write_all(request.body.as_bytes()))
            .map_err(|error| format!("写出失败：{error}"))?;
        let _ = stream.flush();

        let mut reader = BufReader::new(stream);
        const MAX_STATUS_LINE: u64 = 8192;
        let mut status_line = String::new();
        Read::by_ref(&mut reader)
            .take(MAX_STATUS_LINE + 1)
            .read_line(&mut status_line)
            .map_err(|error| format!("读响应失败：{error}"))?;
        if status_line.len() as u64 > MAX_STATUS_LINE || !status_line.ends_with('\n') {
            return Err("HTTP 响应状态行过长或不完整".into());
        }
        let mut parts = status_line.split_whitespace();
        let version = parts.next();
        let code = parts
            .next()
            .filter(|code| code.len() == 3 && code.bytes().all(|byte| byte.is_ascii_digit()));
        if !matches!(version, Some("HTTP/1.0" | "HTTP/1.1")) {
            return Err("HTTP 响应状态行无效".into());
        }
        // Never echo a peer's arbitrary status text into UI history: it could
        // contain request credentials or private payload content.
        let status = code
            .and_then(|code| code.parse::<u16>().ok())
            .filter(|code| (100..=599).contains(code))
            .ok_or_else(|| "HTTP 响应状态码无效".to_string())?;
        let body = if request.response_check.is_some() && (200..300).contains(&status) {
            provider_body(&mut reader)?
        } else {
            Vec::new()
        };
        Ok((status, body))
    }
}

/// Bounded HTTP framing for preset JSON receipts. Never expose response text.
fn provider_body(reader: &mut impl BufRead) -> Result<Vec<u8>, String> {
    const LIMIT: usize = 65536;
    fn line(reader: &mut impl BufRead) -> Result<String, String> {
        let mut line = String::new();
        Read::by_ref(reader)
            .take(8193)
            .read_line(&mut line)
            .map_err(|_| "读取平台回执失败".to_string())?;
        if line.len() > 8192 || !line.ends_with("\r\n") {
            return Err("平台回执头不完整或过长".into());
        }
        Ok(line)
    }
    let mut length = None;
    let mut chunked = false;
    let mut total = 0;
    loop {
        let header = line(reader)?;
        total += header.len();
        if total > 32768 {
            return Err("平台回执头过长".into());
        }
        if header == "\r\n" {
            break;
        }
        let (name, value) = header.split_once(':').ok_or("平台回执头无效")?;
        if name.eq_ignore_ascii_case("content-length") {
            let size = value
                .trim()
                .parse::<usize>()
                .map_err(|_| "平台回执长度无效")?;
            if size > LIMIT || length.is_some() {
                return Err("平台回执长度无效或过大".into());
            }
            length = Some(size);
        }
        if name.eq_ignore_ascii_case("transfer-encoding") {
            if chunked || !value.trim().eq_ignore_ascii_case("chunked") {
                return Err("平台回执编码不支持".into());
            }
            chunked = true;
        }
    }
    if chunked && length.is_some() {
        return Err("平台回执长度冲突".into());
    }
    let mut body = Vec::new();
    if chunked {
        // Bound chunk count as well as data: empty metadata must not run forever.
        for _ in 0..1024 {
            let header = line(reader)?;
            let size = usize::from_str_radix(header.trim().split(';').next().unwrap_or(""), 16)
                .map_err(|_| "平台回执分块无效")?;
            if size == 0 {
                return Ok(body);
            }
            if size > LIMIT - body.len() {
                return Err("平台回执过大".into());
            }
            let start = body.len();
            body.resize(start + size, 0);
            reader
                .read_exact(&mut body[start..])
                .map_err(|_| "平台回执不完整")?;
            let mut end = [0u8; 2];
            reader.read_exact(&mut end).map_err(|_| "平台回执不完整")?;
            if end != *b"\r\n" {
                return Err("平台回执分块无效".into());
            }
        }
        return Err("平台回执分块过多".into());
    }
    Read::by_ref(reader)
        .take(length.unwrap_or(LIMIT + 1) as u64)
        .read_to_end(&mut body)
        .map_err(|_| "读取平台回执失败")?;
    if body.len() > LIMIT || length.is_some_and(|size| size != body.len()) {
        return Err("平台回执过大或不完整".into());
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::HttpField;
    use std::sync::mpsc;

    #[test]
    fn provider_receipt_framing_is_bounded_and_chunked() {
        let json = br#"{"code":0}"#;
        let wire = format!(
            "Content-Length: {}\r\n\r\n{}",
            json.len(),
            String::from_utf8_lossy(json)
        );
        assert_eq!(
            provider_body(&mut std::io::Cursor::new(wire)).unwrap(),
            json
        );
        let wire = "Transfer-Encoding: chunked\r\n\r\nA\r\n{\"code\":0}\r\n0\r\n\r\n";
        assert_eq!(
            provider_body(&mut std::io::Cursor::new(wire)).unwrap(),
            json
        );
        for invalid in [
            "Content-Length: 65537\r\n\r\n",
            "Content-Length: 10\r\n\r\nx",
            "Content-Length: 10\r\nTransfer-Encoding: chunked\r\n\r\n",
            "Transfer-Encoding: chunked\r\n\r\n10001\r\n",
        ] {
            assert!(provider_body(&mut std::io::Cursor::new(invalid)).is_err());
        }
    }

    #[test]
    fn http_200_with_provider_error_is_failure_over_tcp() {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let port = server.server_addr().to_ip().unwrap().port();
        let handle = std::thread::spawn(move || {
            let request = server.recv().unwrap();
            request
                .respond(tiny_http::Response::from_string(r#"{"code":401}"#))
                .unwrap();
        });
        let request = Request {
            url: format!("http://127.0.0.1:{port}/send"),
            method: "POST".into(),
            response_check: Some(crate::remote::Channel::WechatPushPlus),
            ..Request::default()
        };
        let outcome = HttpTransport::new().perform(&request);
        assert!(!outcome.is_delivered());
        assert!(outcome.is_permanent());
        handle.join().unwrap();
    }

    /// 起一个真的本地 HTTP 服务器（tiny_http 已是依赖），回一个固定状态码，
    /// 并把收到的请求原样交回来——这是**端到端**验证：请求真的出过 TCP。
    struct LocalServer {
        port: u16,
        seen: mpsc::Receiver<(String, String, Vec<(String, String)>, String)>,
        handle: Option<std::thread::JoinHandle<()>>,
    }

    impl LocalServer {
        fn start(status: u16) -> LocalServer {
            let server = tiny_http::Server::http("127.0.0.1:0").expect("本地服务器应能监听");
            let port = server
                .server_addr()
                .to_ip()
                .expect("应拿到 IP 监听地址")
                .port();
            let (tx, rx) = mpsc::channel();
            let handle = std::thread::spawn(move || {
                if let Some(mut request) = server.incoming_requests().next() {
                    let method = format!("{:?}", request.method());
                    let url = request.url().to_string();
                    let headers = request
                        .headers()
                        .iter()
                        .map(|h| {
                            (
                                h.field.as_str().as_str().to_string(),
                                h.value.as_str().to_string(),
                            )
                        })
                        .collect();
                    let mut body = String::new();
                    use std::io::Read;
                    let _ = request.as_reader().read_to_string(&mut body);
                    let _ = tx.send((method, url, headers, body));
                    let _ = request
                        .respond(tiny_http::Response::from_string("ok").with_status_code(status));
                }
            });
            LocalServer {
                port,
                seen: rx,
                handle: Some(handle),
            }
        }

        fn collect(&mut self) -> (String, String, Vec<(String, String)>, String) {
            let got = self
                .seen
                .recv_timeout(Duration::from_secs(5))
                .expect("服务器应收到一条请求");
            if let Some(handle) = self.handle.take() {
                let _ = handle.join();
            }
            got
        }
    }

    fn post(port: u16, path: &str, body: &str) -> Request {
        Request {
            url: format!("http://127.0.0.1:{port}{path}"),
            method: "POST".into(),
            headers: vec![
                HttpField::new("X-Title", "Qoder%20%C2%B7%20%E7%AD%89%E5%BE%85"),
                HttpField::new("X-Priority", "4"),
            ],
            body: body.into(),
            smtp: None,
            response_check: None,
        }
    }

    #[test]
    fn a_two_hundred_means_the_peer_accepted_the_request() {
        let mut server = LocalServer::start(200);
        let request = post(server.port, "/island", "Qoder · 等待你确认");
        let outcome = HttpTransport::new().perform(&request);
        assert_eq!(outcome, Outcome::Delivered);

        // 端到端核对：方法、路径、头、正文都真的过了 TCP
        let (method, url, headers, body) = server.collect();
        assert_eq!(method, "Post");
        assert_eq!(url, "/island");
        assert!(headers
            .iter()
            .any(|(n, v)| n == "X-Title" && v == "Qoder%20%C2%B7%20%E7%AD%89%E5%BE%85"));
        assert!(headers.iter().any(|(n, v)| n == "X-Priority" && v == "4"));
        assert_eq!(body, "Qoder · 等待你确认", "正文必须是裸 UTF-8 字节");
    }

    #[test]
    fn status_codes_keep_swifts_permanence_rule() {
        // 4xx 是「对方看懂了但拒绝」：再发一次也是同一个结果
        let mut not_found = LocalServer::start(404);
        let outcome = HttpTransport::new().perform(&post(not_found.port, "/x", "b"));
        assert_eq!(
            outcome,
            Outcome::Failed {
                reason: "服务端返回 404".into(),
                permanent: true
            }
        );
        let _ = not_found.collect();

        // 5xx 是服务端的临时状况 ⇒ 可重试
        let mut broken = LocalServer::start(503);
        let outcome = HttpTransport::new().perform(&post(broken.port, "/x", "b"));
        assert_eq!(
            outcome,
            Outcome::Failed {
                reason: "服务端返回 503".into(),
                permanent: false
            }
        );
        let _ = broken.collect();

        // 429 限流：白重试会更快撞上 ntfy 的条数上限 ⇒ 可重试但不永久
        assert!(!is_permanent(429));
        assert!(!is_permanent(408));
        assert!(!is_permanent(425));
        assert!(is_permanent(400));
        assert!(is_permanent(403));
        assert!(is_permanent(404));
        assert!(is_permanent(302));
        assert!(!is_permanent(500));
        assert!(!is_permanent(200));
    }

    #[test]
    fn a_redirect_is_refused_because_the_url_carries_the_secret() {
        let mut server = LocalServer::start(302);
        let outcome = HttpTransport::new().perform(&post(server.port, "/x?key=S3cretValue", "b"));
        match outcome {
            Outcome::Failed { reason, permanent } => {
                assert!(
                    permanent,
                    "跟着重定向走会把凭据发给第三方，重试也是同样的决定"
                );
                assert!(reason.contains("不跟着走"), "{reason}");
                assert!(reason.contains("302"), "{reason}");
            }
            other => panic!("重定向必须被拒绝，实际 {other:?}"),
        }
        let _ = server.collect();
    }

    #[test]
    fn https_is_actually_attempted_and_smtp_fails_honestly_without_a_target() {
        // https 现在真的去连（另有自签证书的端到端用例）：连不上时报的是连接错误，
        // 而不是「未接入」——这两句话对应完全不同的排查方向。
        // 用 1 号端口（特权端口，测试不会绑它）保证拒绝连接是确定性的：
        // 「绑定后释放」那一招在并行套件里会被别的用例抢走端口。
        let https = Request {
            url: "https://127.0.0.1:1/x".into(),
            ..post(1, "/", "b")
        };
        match HttpTransport::new().perform(&https) {
            Outcome::Failed { reason, permanent } => {
                assert!(reason.contains("连接失败"), "https 应真去连：{reason}");
                assert!(!permanent, "连接失败是链路问题");
            }
            other => panic!("应报连接失败，实际 {other:?}"),
        }

        // 没有目标（渲染时就没密钥）⇒ 如实说清楚，而不是拿空凭据去撞对端
        let without_target = Request {
            method: "SMTP".into(),
            url: String::new(),
            smtp: None,
            response_check: None,
            ..post(1, "/", "b")
        };
        match HttpTransport::new().perform(&without_target) {
            Outcome::Failed { reason, permanent } => {
                assert!(reason.contains("SMTP 目标缺失"), "{reason}");
                assert!(permanent);
            }
            other => panic!("应如实报缺目标，实际 {other:?}"),
        }

        // 非 465 在连接之前就被挡下（STARTTLS 的原地升级不支持）——**不碰网络**
        let starttls = Request {
            method: "SMTP".into(),
            url: String::new(),
            smtp: Some(crate::render::SmtpTarget {
                host: "smtp.example.com".into(),
                port: 587,
                user: "me@example.com".into(), // nosec: 测试夹具里的假邮箱（保留域名），不是真地址
                password: "p".into(),
                from: "me@example.com".into(), // nosec: 测试夹具里的假邮箱（保留域名），不是真地址
                to: "you@example.com".into(),  // nosec: 测试夹具里的假邮箱（保留域名），不是真地址
            }),
            ..post(1, "/", "b")
        };
        match HttpTransport::new().perform(&starttls) {
            Outcome::Failed { reason, .. } => assert!(reason.contains("465"), "{reason}"),
            other => panic!("587 应在连接前被挡下，实际 {other:?}"),
        }
    }

    /// 本地 TLS 服务器用的 **RSA** 自签证书。
    ///
    /// 为什么绕道系统 `openssl`：macOS 上 `native_tls::Identity::from_pkcs8` 只吃 RSA 的
    /// PKCS#8（native-tls 自己的测试也是这么造的），而 rcgen 只会生成 EC 密钥——
    /// 喂 EC 会得到 -25257「Unknown format in import」，与 PEM/DER、参数顺序都无关（四种组合都试过）。
    /// 证书只活在这一条用例里、写在临时目录，绝不进仓库、也不出本机。
    fn rsa_self_signed() -> Option<(Vec<u8>, Vec<u8>)> {
        // Each parallel test owns its certificate/key pair. A process-only directory
        // lets another test replace one half while this test reads the other.
        struct CertificateDirectory(std::path::PathBuf);
        impl Drop for CertificateDirectory {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let dir = std::env::temp_dir().join(format!("agentisland-tls-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).ok()?;
        let _cleanup = CertificateDirectory(dir.clone());
        let key = dir.join("key.pem");
        let crt = dir.join("cert.pem");
        let status = std::process::Command::new("/usr/bin/openssl")
            .args([
                "req",
                "-x509",
                "-newkey",
                "rsa:2048",
                "-nodes",
                "-keyout",
                key.to_str()?,
                "-out",
                crt.to_str()?,
                "-days",
                "1",
                "-subj",
                "/CN=localhost",
                "-addext",
                "subjectAltName=DNS:localhost",
                // 少了 serverAuth 用途，Security.framework 会拒绝这张证书
                // （"The extended key usage is not valid."）——LibreSSL 不会自动加
                "-addext",
                "extendedKeyUsage=serverAuth",
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .ok()?;
        if !status.success() {
            return None;
        }
        Some((std::fs::read(crt).ok()?, std::fs::read(key).ok()?))
    }

    /// 本地 TLS 服务器：RSA 自签证书 + native-tls 的 acceptor。
    /// 客户端注入一个「信任这张证书」的连接器，于是 **HTTPS 这条路能被离线验证**——
    /// 不必去碰任何公网端点（外发每字节都会离开本机，验证也不该例外）。
    struct LocalTlsServer {
        port: u16,
        cert_pem: Vec<u8>,
        seen: mpsc::Receiver<(String, String, String)>,
        handle: Option<std::thread::JoinHandle<()>>,
    }

    impl LocalTlsServer {
        /// 系统没有可用 openssl 时返回 None（用例会打一行说明并跳过，而不是假过）
        fn start(status: u16) -> Option<LocalTlsServer> {
            let (cert_pem, key_pem) = rsa_self_signed()?;
            let identity = native_tls::Identity::from_pkcs8(&cert_pem, &key_pem)
                .expect("RSA 证书应能组装 TLS 身份");
            let acceptor = native_tls::TlsAcceptor::new(identity).expect("应能建 acceptor");
            let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("应能监听");
            let port = listener.local_addr().unwrap().port();
            let (tx, rx) = mpsc::channel();
            let handle = std::thread::spawn(move || {
                let Ok((tcp, _)) = listener.accept() else {
                    return;
                };
                let Ok(mut tls) = acceptor.accept(tcp) else {
                    return;
                };
                // 读请求头直到空行，再按 Content-Length 读正文
                let mut raw = Vec::new();
                let mut byte = [0u8; 1];
                while !raw.ends_with(b"\r\n\r\n") {
                    match tls.read(&mut byte) {
                        Ok(0) | Err(_) => break,
                        Ok(_) => raw.push(byte[0]),
                    }
                }
                let head = String::from_utf8_lossy(&raw).to_string();
                let length: usize = head
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        if name.eq_ignore_ascii_case("content-length") {
                            value.trim().parse().ok()
                        } else {
                            None
                        }
                    })
                    .unwrap_or(0);
                let mut body = vec![0u8; length];
                if length > 0 {
                    let _ = tls.read_exact(&mut body);
                }
                let first = head.lines().next().unwrap_or("").to_string();
                let title = head
                    .lines()
                    .find(|line| line.to_ascii_lowercase().starts_with("x-title:"))
                    .map(|line| line[8..].trim().to_string())
                    .unwrap_or_default();
                let _ = tx.send((first, title, String::from_utf8_lossy(&body).to_string()));
                let response = format!(
                    "HTTP/1.1 {status} OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok"
                );
                let _ = tls.write_all(response.as_bytes());
                let _ = tls.flush();
            });
            Some(LocalTlsServer {
                port,
                cert_pem,
                seen: rx,
                handle: Some(handle),
            })
        }

        fn client(&self) -> HttpTransport {
            let certificate =
                native_tls::Certificate::from_pem(&self.cert_pem).expect("证书应能载入");
            let connector = TlsConnector::builder()
                .add_root_certificate(certificate)
                .build()
                .expect("应能建连接器");
            HttpTransport::with_connector(connector)
        }

        fn collect(&mut self) -> (String, String, String) {
            let got = self
                .seen
                .recv_timeout(Duration::from_secs(10))
                .expect("TLS 服务器应收到一条请求");
            if let Some(handle) = self.handle.take() {
                let _ = handle.join();
            }
            got
        }
    }

    #[test]
    fn https_really_connects_end_to_end_against_a_local_certificate() {
        let Some(mut server) = LocalTlsServer::start(200) else {
            eprintln!("跳过：本机 /usr/bin/openssl 不可用，无法造自签证书");
            return;
        };
        let request = Request {
            url: format!("https://localhost:{}/island", server.port),
            method: "POST".into(),
            headers: vec![HttpField::new(
                "X-Title",
                "Qoder%20%C2%B7%20%E7%AD%89%E5%BE%85",
            )],
            body: "Qoder · 等待你确认".into(),
            smtp: None,
            response_check: None,
        };
        let outcome = server.client().perform(&request);
        assert_eq!(outcome, Outcome::Delivered, "HTTPS 应真能连上并拿到 200");

        let (first_line, title, body) = server.collect();
        assert_eq!(first_line, "POST /island HTTP/1.1");
        assert_eq!(
            title, "Qoder%20%C2%B7%20%E7%AD%89%E5%BE%85",
            "头值要原样到达"
        );
        assert_eq!(body, "Qoder · 等待你确认", "正文是裸 UTF-8");
    }

    #[test]
    fn https_against_an_untrusted_certificate_fails_loudly() {
        // 同一个本地 TLS 服务器，但客户端用**默认**连接器（不信任自签证书）：
        // 必须失败并说清是 TLS 的问题——静默降级成明文是不可接受的
        let Some(mut server) = LocalTlsServer::start(200) else {
            eprintln!("跳过：本机 /usr/bin/openssl 不可用，无法造自签证书");
            return;
        };
        let request = Request {
            url: format!("https://localhost:{}/island", server.port),
            method: "POST".into(),
            headers: vec![],
            body: "b".into(),
            smtp: None,
            response_check: None,
        };
        match HttpTransport::new().perform(&request) {
            Outcome::Failed { reason, .. } => {
                assert!(reason.contains("TLS 握手失败"), "{reason}");
            }
            other => panic!("证书不可信时必须失败，实际 {other:?}"),
        }
        drop(server.handle.take());
    }

    #[test]
    fn a_dead_endpoint_is_a_retryable_failure_with_a_readable_reason() {
        // 连 1 号端口（特权端口，测试不会绑它）：确定性拒绝连接，
        // 且要说得出是哪一步失败
        match HttpTransport::new().perform(&post(1, "/x", "b")) {
            Outcome::Failed { reason, permanent } => {
                assert!(!permanent, "连不上是链路问题，值得重试");
                assert!(
                    reason.contains("连接失败") || reason.contains("地址解析失败"),
                    "{reason}"
                );
            }
            other => panic!("应失败，实际 {other:?}"),
        }
    }

    #[test]
    fn an_unparseable_url_is_reported_instead_of_panicking() {
        for bad in [
            "",
            "not a url",
            "ftp://x/y",
            "http://",
            "http://host:notaport/",
        ] {
            let request = Request {
                url: bad.into(),
                ..post(1, "/", "b")
            };
            assert!(
                matches!(
                    HttpTransport::new().perform(&request),
                    Outcome::Failed { .. }
                ),
                "{bad} 应报失败而不是 panic"
            );
        }
    }
}

#[cfg(test)]
mod optimization_regressions {
    use super::*;
    #[test]
    fn malformed_status_lines_are_not_delivery_or_echoed_into_history() {
        for response in [
            "NOTHTTP 200 OK\r\n",
            "HTTP/1.1 700 INVALID\r\n",
            "HTTP/1.1 invalid private-test-marker\r\n",
        ] {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            let server = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut buffer = [0; 4096];
                let _ = stream.read(&mut buffer);
                stream.write_all(response.as_bytes()).unwrap();
            });
            let request = Request {
                url: format!("http://127.0.0.1:{port}/notify"),
                method: "POST".into(),
                headers: vec![],
                body: "fixture".into(),
                smtp: None,
                response_check: None,
            };
            let outcome = HttpTransport::new().perform(&request);
            server.join().unwrap();
            assert!(
                matches!(outcome, Outcome::Failed { .. }),
                "invalid HTTP response cannot count as delivered"
            );
            if let Outcome::Failed { reason, .. } = outcome {
                assert!(
                    !reason.contains("private-test-marker"),
                    "peer response content must not be copied to the visible history"
                );
            }
        }
    }

    #[test]
    fn query_without_slash_and_ipv6_are_valid_notification_addresses() {
        let parsed = parse_url("https://example.invalid?mode=test").expect("valid query URL");
        assert_eq!(parsed.host, "example.invalid");
        assert_eq!(parsed.path, "/?mode=test");
        let parsed = parse_url("http://[::1]:8080/notify#local").expect("valid IPv6 URL");
        assert_eq!(parsed.host, "::1");
        assert_eq!(
            parsed.path, "/notify",
            "fragments are not sent to the server"
        );
    }
    #[test]
    fn userinfo_and_line_breaks_are_rejected_before_connecting() {
        assert!(parse_url("https://user@example.invalid/notify").is_none());
        assert!(parse_url("https://example.invalid/notify\r\nInjected: value").is_none());
    }
}
