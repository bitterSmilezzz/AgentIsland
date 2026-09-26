//! 外发传输：把 [`Request`] 真的送出去。
//!
//! 对齐 Swift `HTTPTransport`：固定 10 秒超时（外发在后台跑，绝不能让一次卡住的请求
//! 拖住事件管线）、**拒绝重定向**（URL 里带着 SendKey/token，跟着 302 走等于把凭据
//! 发给一个用户从没配置过的主机）、以及状态码的可重试判据。
//!
//! **本轮只接明文 http**：https 与 SMTP over 465 都要 TLS crate（`rustls` / `native-tls`），
//! 是下一轮的独立依赖决定。今天这两条路**如实报「未接入」**，而不是把请求静默丢掉——
//! 「开关开着却永远收不到」是本仓最难查的那类失效。

use crate::notifier::{Outcome, Transport};
use crate::render::Request;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

/// 固定 10 秒：与 Swift `req.timeoutInterval = 10` 同值
pub const TIMEOUT: Duration = Duration::from_secs(10);

pub struct HttpTransport;

impl Default for HttpTransport {
    fn default() -> Self {
        HttpTransport
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

/// URL 解析（只认 http/https）。不引 URL crate：需要的只是「协议 / 主机 / 端口 / 路径」，
/// 而多一个依赖要单独论证。
fn parse_url(url: &str) -> Option<Parsed> {
    let (scheme, rest) = url.split_once("://")?;
    let scheme = scheme.to_lowercase();
    if scheme != "http" && scheme != "https" {
        return None;
    }
    let (authority, path) = match rest.find('/') {
        Some(index) => (&rest[..index], &rest[index..]),
        None => (rest, "/"),
    };
    // 端口缺省：https 443、http 80。刻意用 host:port 的字面形式而不是先剥用户信息——
    // 本仓的通道地址里不会有 user@，出现了就当解析失败更好
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) => (host.to_string(), port.parse::<u16>().ok()?),
        None => (
            authority.to_string(),
            if scheme == "https" { 443 } else { 80 },
        ),
    };
    if host.is_empty() || path.is_empty() {
        return None;
    }
    Some(Parsed {
        scheme,
        host,
        port,
        path: path.to_string(),
    })
}

impl Transport for HttpTransport {
    fn perform(&self, request: &Request) -> Outcome {
        // SMTP 走的是另一条会话（下一轮）。这里如实地说不支持，而不是假装送达
        if request.smtp.is_some() || request.method == "SMTP" {
            return Outcome::Failed {
                reason: "SMTP 会话未接入".into(),
                permanent: true,
            };
        }
        let Some(url) = parse_url(&request.url) else {
            return Outcome::Failed {
                reason: "地址无法解析".into(),
                permanent: false,
            };
        };
        if url.scheme == "https" {
            return Outcome::Failed {
                reason: "https 需要 TLS 支持（本轮只接了明文 http）".into(),
                // 重试在 TLS 接进来之前不会有不同结果
                permanent: true,
            };
        }
        match self.round_trip(&url, request) {
            Ok(status) => {
                if (200..300).contains(&status) {
                    // 注意语义：这是「对方接受了这条请求」。多数中转服务即使内部失败
                    // 也回 200 + 一段错误 JSON，本层不去猜——设置页的文案对此写明
                    Outcome::Delivered
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
    fn round_trip(&self, url: &Parsed, request: &Request) -> Result<u16, String> {
        let target = (url.host.as_str(), url.port)
            .to_socket_addrs()
            .map_err(|error| format!("地址解析失败：{error}"))?
            .next()
            .ok_or_else(|| "地址解析为空".to_string())?;
        let mut stream = TcpStream::connect_timeout(&target, TIMEOUT)
            .map_err(|error| format!("连接失败：{error}"))?;
        let _ = stream.set_read_timeout(Some(TIMEOUT));
        let _ = stream.set_write_timeout(Some(TIMEOUT));

        let host_header = if url.port == 80 {
            url.host.clone()
        } else {
            format!("{}:{}", url.host, url.port)
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
        let mut status_line = String::new();
        reader
            .read_line(&mut status_line)
            .map_err(|error| format!("读响应失败：{error}"))?;
        // "HTTP/1.1 200 OK"
        let mut parts = status_line.split_whitespace();
        let _version = parts.next();
        parts
            .next()
            .and_then(|code| code.parse::<u16>().ok())
            .ok_or_else(|| format!("响应无法解析：{}", status_line.trim()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::HttpField;
    use std::sync::mpsc;

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
                        .map(|h| (h.field.as_str().as_str().to_string(), h.value.as_str().to_string()))
                        .collect();
                    let mut body = String::new();
                    use std::io::Read;
                    let _ = request.as_reader().read_to_string(&mut body);
                    let _ = tx.send((method, url, headers, body));
                    let _ = request.respond(
                        tiny_http::Response::from_string("ok").with_status_code(status),
                    );
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
        }
    }

    #[test]
    fn a_two_hundred_means_the_peer_accepted_the_request() {
        let mut server = LocalServer::start(200);
        let request = post(server.port, "/island", "Qoder · 等待你确认");
        let outcome = HttpTransport.perform(&request);
        assert_eq!(outcome, Outcome::Delivered);

        // 端到端核对：方法、路径、头、正文都真的过了 TCP
        let (method, url, headers, body) = server.collect();
        assert_eq!(method, "Post");
        assert_eq!(url, "/island");
        assert!(headers.iter().any(|(n, v)| n == "X-Title" && v == "Qoder%20%C2%B7%20%E7%AD%89%E5%BE%85"));
        assert!(headers.iter().any(|(n, v)| n == "X-Priority" && v == "4"));
        assert_eq!(body, "Qoder · 等待你确认", "正文必须是裸 UTF-8 字节");
    }

    #[test]
    fn status_codes_keep_swifts_permanence_rule() {
        // 4xx 是「对方看懂了但拒绝」：再发一次也是同一个结果
        let mut not_found = LocalServer::start(404);
        let outcome = HttpTransport.perform(&post(not_found.port, "/x", "b"));
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
        let outcome = HttpTransport.perform(&post(broken.port, "/x", "b"));
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
        let outcome = HttpTransport.perform(&post(server.port, "/x?key=S3cretValue", "b"));
        match outcome {
            Outcome::Failed { reason, permanent } => {
                assert!(permanent, "跟着重定向走会把凭据发给第三方，重试也是同样的决定");
                assert!(reason.contains("不跟着走"), "{reason}");
                assert!(reason.contains("302"), "{reason}");
            }
            other => panic!("重定向必须被拒绝，实际 {other:?}"),
        }
        let _ = server.collect();
    }

    #[test]
    fn https_and_smtp_are_reported_as_unwired_instead_of_silently_dropped() {
        let https = Request {
            url: "https://ntfy.sh/island".into(),
            ..post(1, "/", "b")
        };
        match HttpTransport.perform(&https) {
            Outcome::Failed { reason, permanent } => {
                assert!(reason.contains("TLS"), "{reason}");
                assert!(permanent, "TLS 接进来之前重试不会有不同结果");
            }
            other => panic!("应如实报未接入，实际 {other:?}"),
        }

        let smtp = Request {
            method: "SMTP".into(),
            url: String::new(),
            smtp: Some(crate::render::SmtpTarget {
                host: "smtp.example.com".into(),
                port: 465,
                user: "me@example.com".into(), // nosec: 测试夹具里的假邮箱（保留域名），不是真地址
                password: "p".into(),
                from: "me@example.com".into(), // nosec: 测试夹具里的假邮箱（保留域名），不是真地址
                to: "you@example.com".into(), // nosec: 测试夹具里的假邮箱（保留域名），不是真地址
            }),
            ..post(1, "/", "b")
        };
        match HttpTransport.perform(&smtp) {
            Outcome::Failed { reason, permanent } => {
                assert!(reason.contains("SMTP"), "{reason}");
                assert!(permanent);
            }
            other => panic!("应如实报未接入，实际 {other:?}"),
        }
    }

    #[test]
    fn a_dead_endpoint_is_a_retryable_failure_with_a_readable_reason() {
        // 连一个刚被释放的端口：拿不到响应，但要说得出是哪一步失败
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        match HttpTransport.perform(&post(port, "/x", "b")) {
            Outcome::Failed { reason, permanent } => {
                assert!(!permanent, "连不上是链路问题，值得重试");
                assert!(reason.contains("连接失败") || reason.contains("地址解析失败"), "{reason}");
            }
            other => panic!("应失败，实际 {other:?}"),
        }
    }

    #[test]
    fn an_unparseable_url_is_reported_instead_of_panicking() {
        for bad in ["", "not a url", "ftp://x/y", "http://", "http://host:notaport/"] {
            let request = Request {
                url: bad.into(),
                ..post(1, "/", "b")
            };
            assert!(
                matches!(HttpTransport.perform(&request), Outcome::Failed { .. }),
                "{bad} 应报失败而不是 panic"
            );
        }
    }
}
