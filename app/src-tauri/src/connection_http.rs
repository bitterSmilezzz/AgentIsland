//! Bounded, read-only service transport. Peer bodies and transport diagnostics stay internal.
use reqwest::{
    blocking::Client,
    header::{HeaderValue, ACCEPT, ACCEPT_ENCODING},
};
use serde_json::Value;
use std::sync::{Arc, Mutex};
use std::{io::Read, time::Duration};

const MAX_BODY: usize = 512 * 1024;
const TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, PartialEq)]
pub enum Failure {
    InvalidAddress,
    /// 非本机地址只允许 HTTPS：明文 HTTP 会把凭据与响应暴露在链路上。
    InsecureTransport,
    InvalidCredential,
    CredentialUnavailable,
    Unavailable,
    Timeout,
    Tls,
    Redirect,
    AuthFailed,
    Forbidden,
    RateLimited,
    HttpStatus(u16),
    TooLarge,
    InvalidJson,
    UnsupportedEncoding,
}
impl Failure {
    /// 稳定的机器可读分类，供探针回执与门禁断言使用；改这里等于改对外契约。
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidAddress => "invalid_address",
            Self::InsecureTransport => "insecure_transport",
            Self::InvalidCredential => "invalid_credential",
            Self::CredentialUnavailable => "credential_unavailable",
            Self::Unavailable => "unavailable",
            Self::Timeout => "timeout",
            Self::Tls => "tls",
            Self::Redirect => "redirect",
            Self::AuthFailed => "auth_failed",
            Self::Forbidden => "forbidden",
            Self::RateLimited => "rate_limited",
            Self::HttpStatus(_) => "http_status",
            Self::TooLarge => "too_large",
            Self::InvalidJson => "invalid_json",
            Self::UnsupportedEncoding => "unsupported_encoding",
        }
    }
    pub fn reason(&self) -> &'static str {
        match self {
            Self::InvalidAddress => "服务地址无效",
            Self::InsecureTransport => "非本机地址必须使用 HTTPS，明文 HTTP 会明文发送凭据",
            Self::InvalidCredential => "凭据格式无效，请检查凭据来源",
            Self::CredentialUnavailable => "凭据环境变量不可用，请检查应用启动环境",
            Self::Unavailable => "服务不可达或读取中断",
            Self::Timeout => "服务读取超时",
            Self::Tls => "TLS 连接失败，请检查服务证书",
            Self::Redirect => "服务返回重定向，请填写最终地址",
            Self::AuthFailed => "服务拒绝凭据",
            Self::Forbidden => "凭据没有此项读取权限",
            Self::RateLimited => "服务限流，请稍后手动重试",
            Self::HttpStatus(_) => "服务返回错误状态",
            Self::TooLarge => "服务响应过大，已停止读取",
            Self::InvalidJson => "服务响应不是受支持的 JSON",
            Self::UnsupportedEncoding => "服务返回了不支持的压缩响应",
        }
    }
}

/// 本机回环判定：本地模型网关允许明文 HTTP，其余地址一律要求 HTTPS。
pub fn loopback(host: &str) -> bool {
    let host = host.trim_start_matches('[').trim_end_matches(']');
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    match host.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(v4)) => v4.is_loopback(),
        Ok(std::net::IpAddr::V6(v6)) => v6.is_loopback(),
        Err(_) => false,
    }
}

/// 传输层策略：在任何 socket 建立之前决定这次读取是否允许发生。
///
/// 为什么必须在建连之前拒绝：一旦发出请求，凭据已经写进了报文——
/// 「先连上再检查协议」等于已经泄漏。自定义端点与本机模型网关共用这条判据。
pub fn transport_policy(url: &url::Url) -> Result<(), Failure> {
    match url.scheme() {
        "https" => Ok(()),
        "http" => {
            if url.host_str().is_some_and(loopback) {
                Ok(())
            } else {
                Err(Failure::InsecureTransport)
            }
        }
        _ => Err(Failure::InvalidAddress),
    }
}

/// 一次读取要带的认证头。方案由连接自身声明，不从响应或环境推断。
pub enum AuthHeader {
    None,
    Bearer(String),
    Custom { name: String, value: String },
}
impl AuthHeader {
    /// 认证值统一走同一条校验：非空、有界、全为可见 ASCII。
    fn credential(value: &str) -> Result<&str, Failure> {
        if value.is_empty()
            || value.len() > 8192
            || !value.bytes().all(|b| b.is_ascii_graphic())
        {
            return Err(Failure::InvalidCredential);
        }
        Ok(value)
    }
    pub fn header_name(&self) -> Option<&str> {
        match self {
            Self::None => None,
            Self::Bearer(_) => Some("authorization"),
            Self::Custom { name, .. } => Some(name),
        }
    }
    pub fn header_value(&self) -> Result<Option<HeaderValue>, Failure> {
        let text = match self {
            Self::None => return Ok(None),
            Self::Bearer(value) => format!("Bearer {}", Self::credential(value)?),
            Self::Custom { value, .. } => Self::credential(value)?.to_owned(),
        };
        let mut header = HeaderValue::from_str(&text).map_err(|_| Failure::InvalidCredential)?;
        // 让日志/调试输出不打印凭据本身。
        header.set_sensitive(true);
        Ok(Some(header))
    }
}

pub struct Http {
    client: Client,
}
static SHARED: Mutex<Option<Arc<Http>>> = Mutex::new(None);
impl Http {
    /// Initialized by the first manual worker job. Retain the client so a timed-out
    /// OS DNS task cannot stall each operation in blocking Client's runtime Drop.
    pub fn shared() -> Result<Arc<Self>, Failure> {
        let mut shared = SHARED.lock().map_err(|_| Failure::Unavailable)?;
        if let Some(http) = shared.as_ref() {
            return Ok(http.clone());
        }
        let http = Arc::new(Self::new()?);
        *shared = Some(http.clone());
        Ok(http)
    }
    /// Construct and use only on a worker; blocking reqwest owns a runtime internally.
    fn new() -> Result<Self, Failure> {
        Self::with_timeout(TIMEOUT)
    }
    fn with_timeout(timeout: Duration) -> Result<Self, Failure> {
        let client = Client::builder()
            .tls_backend_native()
            .timeout(timeout)
            .connect_timeout(timeout)
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            // 永不继承环境代理：`HTTP_PROXY`/`ALL_PROXY` 可以把带凭据的请求整体
            // 改道到第三方端点，是「不泄漏凭据」这条红线上最容易漏的一个入口。
            // 需要代理的用户应显式配置到服务地址里，而不是靠进程环境静默接管。
            .no_proxy()
            .pool_max_idle_per_host(0)
            .build()
            .map_err(classify)?;
        Ok(Self { client })
    }
    /// 探针专用：**绕过共享客户端缓存**重新构造，使代理与超时策略是在当前进程
    /// 环境下重新求值的——否则「进程启动时建的客户端」会让代理隔离用例变成空断言。
    pub(crate) fn fresh() -> Result<Self, Failure> {
        Self::new()
    }
    /// 唯一的读取入口：认证方案必须由调用方显式给出，不存在隐式默认头。
    pub fn get_json(&self, url: url::Url, auth: &AuthHeader) -> Result<Value, Failure> {
        crate::connections::normalize_url(url.as_str()).map_err(|_| Failure::InvalidAddress)?;
        // 先判策略再建连：明文地址一旦发出请求，凭据已经进了报文。
        transport_policy(&url)?;
        let mut request = self
            .client
            .get(url)
            .header(ACCEPT, "application/json")
            .header(ACCEPT_ENCODING, "identity");
        if let Some(name) = auth.header_name() {
            let name = reqwest::header::HeaderName::from_bytes(name.as_bytes())
                .map_err(|_| Failure::InvalidCredential)?;
            let value = auth.header_value()?.ok_or(Failure::InvalidCredential)?;
            request = request.header(name, value);
        }
        let mut response = request.send().map_err(classify)?;
        let status = response.status().as_u16();
        match status {
            200..=299 => {}
            300..=399 => return Err(Failure::Redirect),
            401 => return Err(Failure::AuthFailed),
            403 => return Err(Failure::Forbidden),
            429 => return Err(Failure::RateLimited),
            _ => return Err(Failure::HttpStatus(status)),
        }
        if response
            .content_length()
            .is_some_and(|size| size > MAX_BODY as u64)
        {
            return Err(Failure::TooLarge);
        }
        if response
            .headers()
            .get(reqwest::header::CONTENT_ENCODING)
            .is_some_and(|value| !value.as_bytes().eq_ignore_ascii_case(b"identity"))
        {
            return Err(Failure::UnsupportedEncoding);
        }
        if response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .is_some_and(|value| {
                let Ok(text) = value.to_str() else {
                    return true;
                };
                let media = text
                    .split(';')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_ascii_lowercase();
                media != "application/json"
                    && !(media.starts_with("application/") && media.ends_with("+json"))
            })
        {
            return Err(Failure::InvalidJson);
        }
        // Bound decoded allocation even with absent/chunked Content-Length.
        let mut body = Vec::new();
        response
            .by_ref()
            .take(MAX_BODY as u64 + 1)
            .read_to_end(&mut body)
            .map_err(|error| {
                error
                    .get_ref()
                    .and_then(|source| source.downcast_ref::<reqwest::Error>())
                    .map(|error| classify_ref(error))
                    .unwrap_or(Failure::Unavailable)
            })?;
        if body.len() > MAX_BODY {
            return Err(Failure::TooLarge);
        }
        serde_json::from_slice(&body).map_err(|_| Failure::InvalidJson)
    }
}
fn classify(error: reqwest::Error) -> Failure {
    classify_ref(&error)
}
fn classify_ref(error: &reqwest::Error) -> Failure {
    if error.is_timeout() {
        return Failure::Timeout;
    }
    let mut source = std::error::Error::source(error);
    for _ in 0..8 {
        let Some(current) = source else {
            break;
        };
        if current.downcast_ref::<native_tls::Error>().is_some() {
            return Failure::Tls;
        }
        source = current.source();
    }
    Failure::Unavailable
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{BufRead, BufReader, Write},
        net::TcpListener,
        sync::{Arc, Mutex},
    };
    fn server(response: Vec<u8>) -> (url::Url, Arc<Mutex<String>>, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let captured = Arc::new(Mutex::new(String::new()));
        let output = captured.clone();
        let worker = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut request = String::new();
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 {
                    break;
                }
                request.push_str(&line);
                if line == "\r\n" {
                    break;
                }
            }
            *output.lock().unwrap() = request;
            let _ = stream.write_all(&response);
        });
        (
            url::Url::parse(&format!("http://{address}/api/status")).unwrap(),
            captured,
            worker,
        )
    }
    fn response(status: u16, body: &[u8]) -> Vec<u8> {
        let mut bytes = format!("HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).into_bytes();
        bytes.extend_from_slice(body);
        bytes
    }
    #[test]
    fn get_reads_json_without_cookie_body_or_write_methods() {
        let (url, captured, worker) = server(response(
            200,
            br#"{"success":true,"data":{"version":"fixture"}}"#,
        ));
        let value = Http::new()
            .unwrap()
            .get_json(url, &AuthHeader::Bearer("fixture-auth".into()))
            .unwrap();
        worker.join().unwrap();
        assert_eq!(value["success"], true);
        let request = captured.lock().unwrap();
        assert!(request.starts_with("GET /api/status HTTP/1.1\r\n"));
        assert!(request
            .to_lowercase()
            .contains("authorization: bearer fixture-auth"));
        assert!(!request.to_lowercase().contains("cookie:"));
        assert!(!request.to_lowercase().contains("content-length:"));
    }
    #[test]
    fn redirects_are_not_followed_and_credentials_do_not_reach_target() {
        let target = TcpListener::bind("127.0.0.1:0").unwrap();
        target.set_nonblocking(true).unwrap();
        let bytes = format!("HTTP/1.1 302 Found\r\nLocation: http://{}/other\r\nContent-Length: 0\r\nConnection: close\r\n\r\n", target.local_addr().unwrap()).into_bytes();
        let (url, _, worker) = server(bytes);
        assert_eq!(
            Http::new()
                .unwrap()
                .get_json(url, &AuthHeader::Bearer("fixture-auth".into()))
                .unwrap_err(),
            Failure::Redirect
        );
        worker.join().unwrap();
        assert_eq!(
            target.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
    }
    #[test]
    fn errors_are_classified_without_peer_text() {
        for (status, expected) in [
            (401, Failure::AuthFailed),
            (403, Failure::Forbidden),
            (429, Failure::RateLimited),
            (500, Failure::HttpStatus(500)),
        ] {
            let (url, _, worker) = server(response(status, b"private-fixture-marker"));
            let failure = Http::new().unwrap().get_json(url, &AuthHeader::None).unwrap_err();
            worker.join().unwrap();
            assert_eq!(failure, expected);
            assert!(!failure.reason().contains("private-fixture-marker"));
        }
        let (url, _, worker) = server(response(200, b"private-fixture-marker"));
        assert_eq!(
            Http::new().unwrap().get_json(url, &AuthHeader::None).unwrap_err(),
            Failure::InvalidJson
        );
        worker.join().unwrap();
    }
    #[test]
    fn declared_and_streamed_oversize_and_compression_are_refused() {
        let oversized = vec![b'x'; MAX_BODY + 1];
        let declared = response(200, &oversized);
        let mut unframed = b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n".to_vec();
        unframed.extend_from_slice(&oversized);
        for bytes in [declared, unframed] {
            let (url, _, worker) = server(bytes);
            assert_eq!(
                Http::new().unwrap().get_json(url, &AuthHeader::None).unwrap_err(),
                Failure::TooLarge
            );
            worker.join().unwrap();
        }
        let (url, _, worker) = server(b"HTTP/1.1 200 OK\r\nContent-Encoding: gzip\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}".to_vec());
        assert_eq!(
            Http::new().unwrap().get_json(url, &AuthHeader::None).unwrap_err(),
            Failure::UnsupportedEncoding
        );
        worker.join().unwrap();
    }
    #[test]
    fn timeout_covers_delayed_headers_and_incomplete_body() {
        for body_started in [false, true] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let (release, hold) = std::sync::mpsc::channel();
            let worker = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut request = BufReader::new(stream.try_clone().unwrap());
                // Consume request headers before modelling a slow response; otherwise
                // the peer races the client's request write instead of its read timeout.
                loop {
                    let mut line = String::new();
                    match request.read_line(&mut line) {
                        Ok(0) | Err(_) => break,
                        Ok(_) if line == "\r\n" => break,
                        Ok(_) => {}
                    }
                }
                if body_started {
                    let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\n{}");
                }
                // Keep the connection open until the client returns. A fixed sleep
                // can expire while the client thread is descheduled and test EOF
                // classification instead of the intended timeout.
                // The sender also drops on assertion unwind, so this does not retain
                // the peer after a failed test. Never expire the peer while a
                // descheduled client still owns the transaction.
                let _ = hold.recv();
            });
            let url = url::Url::parse(&format!("http://{address}/")).unwrap();
            assert_eq!(
                Http::with_timeout(Duration::from_millis(60))
                    .unwrap()
                    .get_json(url, &AuthHeader::None)
                    .unwrap_err(),
                Failure::Timeout,
                "body_started={body_started}"
            );
            let _ = release.send(());
            worker.join().unwrap();
        }
    }
    #[test]
    fn invalid_credentials_are_refused_before_any_network_access() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = url::Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
        let http = Http::new().unwrap();
        for value in ["", "two values", "fixture\r\nInjected: header"] {
            assert_eq!(
                http.get_json(url.clone(), &AuthHeader::Bearer(value.to_owned())).unwrap_err(),
                Failure::InvalidCredential
            );
        }
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
    }

    #[test]
    fn plaintext_is_refused_for_non_loopback_addresses() {
        for allowed in [
            "https://gateway.example/",
            "http://127.0.0.1:11434/",
            "http://[::1]:11434/",
            "http://localhost:11434/",
        ] {
            let url = url::Url::parse(allowed).unwrap();
            assert!(transport_policy(&url).is_ok(), "{allowed}");
        }
        assert_eq!(
            transport_policy(&url::Url::parse("http://gateway.example/").unwrap()).unwrap_err(),
            Failure::InsecureTransport
        );
        assert_eq!(
            transport_policy(&url::Url::parse("ftp://gateway.example/").unwrap()).unwrap_err(),
            Failure::InvalidAddress
        );
        // 回环判定不接受「看起来像」的写法：子域与相似字符串都不是本机。
        for not_local in ["http://localhost.example/", "http://127.0.0.1.example/", "http://0.0.0.0/"] {
            assert_eq!(
                transport_policy(&url::Url::parse(not_local).unwrap()).unwrap_err(),
                Failure::InsecureTransport,
                "{not_local}"
            );
        }
    }

    #[test]
    fn refused_transport_never_opens_a_socket() {
        // 远端明文地址在策略层就被拒，连接根本不会建立：拿到的一定是
        // InsecureTransport 而不是「连不上」——两者在界面上必须能分开。
        let http = Http::new().unwrap();
        let url = url::Url::parse("http://no-such-host.invalid/").unwrap();
        assert_eq!(
            http.get_json(url, &AuthHeader::None).unwrap_err(),
            Failure::InsecureTransport
        );
    }

    #[test]
    fn auth_schemes_map_to_exactly_one_header_each() {
        assert!(AuthHeader::None.header_name().is_none());
        assert!(AuthHeader::None.header_value().unwrap().is_none());

        let bearer = AuthHeader::Bearer("fixture-value".into());
        assert_eq!(bearer.header_name(), Some("authorization"));
        let value = bearer.header_value().unwrap().unwrap();
        assert_eq!(value.to_str().unwrap(), "Bearer fixture-value");
        assert!(value.is_sensitive(), "凭据头必须标记为敏感，避免被日志打印");

        let custom = AuthHeader::Custom {
            name: "x-api-key".into(),
            value: "fixture-value".into(),
        };
        assert_eq!(custom.header_name(), Some("x-api-key"));
        assert_eq!(
            custom.header_value().unwrap().unwrap().to_str().unwrap(),
            "fixture-value"
        );
        assert!(custom.header_value().unwrap().unwrap().is_sensitive());

        for bad in ["", "two words", "非 ASCII", "line\r\nInjected: value"] {
            assert_eq!(
                AuthHeader::Bearer(bad.to_owned()).header_value().unwrap_err(),
                Failure::InvalidCredential,
                "{bad}"
            );
        }
    }

    #[test]
    fn failure_codes_are_unique_and_non_empty() {
        // 这些码是探针回执与门禁断言之间的对外契约：必须互不重复且稳定非空。
        let all = [
            Failure::InvalidAddress,
            Failure::InsecureTransport,
            Failure::InvalidCredential,
            Failure::CredentialUnavailable,
            Failure::Unavailable,
            Failure::Timeout,
            Failure::Tls,
            Failure::Redirect,
            Failure::AuthFailed,
            Failure::Forbidden,
            Failure::RateLimited,
            Failure::HttpStatus(500),
            Failure::TooLarge,
            Failure::InvalidJson,
            Failure::UnsupportedEncoding,
        ];
        let codes: Vec<&str> = all.iter().map(Failure::code).collect();
        let unique: std::collections::HashSet<&&str> = codes.iter().collect();
        assert_eq!(unique.len(), codes.len(), "错误码必须唯一：{codes:?}");
        assert!(codes.iter().all(|code| !code.is_empty()));
        assert_eq!(Failure::InsecureTransport.code(), "insecure_transport");
    }
}
