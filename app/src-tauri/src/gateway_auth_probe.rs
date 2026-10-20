//! GW-01 认证网关的**真实二进制端到端**探针。
//!
//! 与 `remote_delivery_probe`（NOT-05）同一形态：显式 argv 探针、headless、
//! **从不读用户配置/钥匙串**、只对 `127.0.0.1` 上自建的对端说话。
//!
//! 为什么需要它：`connections` / `connection_http` 的判据此前只有 `#[cfg(test)]`
//! 单测与注入闭包。**明文拒发、认证方案、代理隔离、凭据轮换**这四条都属于
//! 「配置看起来对、实际读取时行为不同」的类别——只有在**发布产物**里对着真实
//! socket 跑一遍，才能证明它们真的成立。`scripts/test-gateway-auth.py`
//! （registry 检查项 `gw01.authgate`）读这份回执并把每一项都断言一遍。
//!
//! 回执的键名与取值是**对外契约**：改这里等于改门禁，必须同步改门禁脚本。

use crate::connection_http::{transport_policy, AuthHeader, Failure, Http};
use crate::connection_probe::{self, Probe, Status};
use crate::connections::{AuthScheme, Connection, CredentialRef, Kind};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// 探针自己的夹具凭据来源。值不是凭据，只是「能被观察到变了没有」的标记串。
const FIXTURE_ENV: &str = "AGENTISLAND_GW01_FIXTURE";
/// 刻意**不设置**的环境变量，用来验证「凭据来源不可用」这个分支。
const MISSING_ENV: &str = "AGENTISLAND_GW01_ABSENT";
const FIRST_SECRET: &str = "fixture-value-one";
const SECOND_SECRET: &str = "fixture-value-two";

pub(crate) fn cli(argv: &[String]) -> Option<i32> {
    if argv.get(1).map(String::as_str) != Some("--gateway-auth-probe") {
        return None;
    }
    let result = if argv.len() == 2 {
        run()
    } else {
        Err("usage: --gateway-auth-probe".to_string())
    };
    match result {
        Ok(report) => {
            println!("{report}");
            Some(0)
        }
        Err(reason) => {
            eprintln!("gateway auth probe failed: {reason}");
            Some(2)
        }
    }
}

/// 本地对端：按脚本依次回状态码，并记录收到的认证头值。
struct Peer {
    port: u16,
    requests: Arc<AtomicUsize>,
    authored: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
}
impl Peer {
    /// `plan` 里每个请求取一项，用完后重复最后一项。
    fn start(plan: Vec<(u16, &'static str)>) -> std::io::Result<Self> {
        assert!(!plan.is_empty(), "plan must not be empty");
        let listener = TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        let requests = Arc::new(AtomicUsize::new(0));
        let authored = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (counts, seen, flag) = (requests.clone(), authored.clone(), stop.clone());
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<()>();
        std::thread::spawn(move || {
            // 先报告「已在接受循环里」，再开始接受：否则 bind 完就返回，调用方
            // 立刻 connect 时依赖的是 backlog 而不是已就绪的接受者。
            let _ = ready_tx.send(());
            while !flag.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let index = counts.fetch_add(1, Ordering::SeqCst);
                        let (status, body) = plan[index.min(plan.len() - 1)];
                        serve(stream, status, body, &seen);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => break,
                }
            }
        });
        // 有界等待：线程若起不来，让调用方看到明确的失败而不是偶发超时。
        ready_rx
            .recv_timeout(Duration::from_secs(5))
            .map_err(|_| std::io::Error::other("peer thread did not start"))?;
        Ok(Self {
            port,
            requests,
            authored,
            stop,
        })
    }
    fn url(&self) -> String {
        format!("http://127.0.0.1:{}/", self.port)
    }
    fn requests(&self) -> usize {
        self.requests.load(Ordering::SeqCst)
    }
    fn authored(&self) -> Vec<String> {
        self.authored.lock().map(|v| v.clone()).unwrap_or_default()
    }
}
impl Drop for Peer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// 单个连接上读请求头、记下认证头、回一个固定响应。
///
/// 刻意**不 clone 句柄**：Windows 上克隆出来的句柄不共享 `SO_RCVTIMEO`，
/// 一旦对端不回数据就会永久阻塞（本仓在 HTTP 层踩过同一个坑）。
fn serve(mut stream: TcpStream, status: u16, body: &str, authored: &Mutex<Vec<String>>) {
    // `accept()` 自非阻塞 listener 取出的 socket 会**继承 O_NONBLOCK**（macOS 实测；
    // 本仓 `update_e2e_tests` 踩过同一个坑）。不显式关掉它，`read` 立刻返回
    // WouldBlock：请求没被消费就回响应，对端随后可能收到 RST，客户端表现为
    // `Unavailable`/`Timeout`——也就是回执里偶发的 `"offline"`。
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let mut head = Vec::new();
    let mut buffer = [0u8; 512];
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while head.len() < 8192 && std::time::Instant::now() < deadline {
        match stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => {
                head.extend_from_slice(&buffer[..count]);
                if head.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::Interrupted | std::io::ErrorKind::WouldBlock
                ) =>
            {
                continue
            }
            Err(_) => break,
        }
    }
    let text = String::from_utf8_lossy(&head).into_owned();
    if let Ok(mut values) = authored.lock() {
        for line in text.lines() {
            let Some((name, value)) = line.split_once(':') else {
                continue;
            };
            if name.eq_ignore_ascii_case("authorization") || name.eq_ignore_ascii_case("x-api-key") {
                values.push(value.trim().to_owned());
            }
        }
    }
    let reason = if (200..300).contains(&status) {
        "OK"
    } else {
        "Status"
    };
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    // 一次写完整响应，再半关闭写端：让对端读到干净的 EOF，
    // 而不是「连接被直接回收」导致的截断/RST。
    if stream.write_all(response.as_bytes()).is_ok() {
        let _ = stream.flush();
        let _ = stream.shutdown(std::net::Shutdown::Write);
    }
}

fn connection(kind: Kind, url: &str, auth: AuthScheme, env_name: &str) -> Connection {
    Connection {
        id: uuid::Uuid::new_v4().to_string(),
        kind,
        name: "Fixture".into(),
        base_url: url.into(),
        credential_ref: Some(CredentialRef::Environment {
            name: env_name.into(),
        }),
        auth,
        enabled: true,
        created_ms: 1,
        updated_ms: 1,
    }
}

fn status_name(status: &Status) -> &'static str {
    match status {
        Status::Readable => "readable",
        Status::Disabled => "disabled",
        Status::Unsupported => "unsupported",
        Status::Offline => "offline",
        Status::AuthFailed => "auth_failed",
        Status::PermissionDenied => "permission_denied",
        Status::Error => "error",
    }
}

/// 把一次读取的结果压成一个标签。
///
/// 规则：状态枚举里有专门取值的报状态名；被 `Status::Error` 归并掉的（限流、
/// 凭据来源不可用、明文拒绝…）报**失败码**——否则这些在界面上全是同一句
/// 「服务读取失败」，门禁就分不出到底是哪一种。
fn outcome_label(probe: &Probe) -> String {
    match &probe.status {
        // 被 Status::Error 归并掉的类别，靠探针带回来的失败码区分。
        Status::Error => probe
            .failure_code
            .clone()
            .unwrap_or_else(|| "error".to_owned()),
        other => status_name(other).to_owned(),
    }
}


fn transport_section() -> Result<(Value, Vec<String>), String> {
    let parse = |text: &str| url::Url::parse(text).map_err(|_| format!("bad fixture url {text}"));
    let https = parse("https://gateway.example/")?;
    let loopback = parse("http://127.0.0.1:11434/")?;
    let remote = parse("http://gateway.example/")?;
    let scheme = parse("ftp://gateway.example/")?;
    let code = |result: Result<(), Failure>| match result {
        Ok(()) => "accepted".to_string(),
        Err(failure) => failure.code().to_string(),
    };
    Ok((
        json!({
            "https_accepted": transport_policy(&https).is_ok(),
            "loopback_http_accepted": transport_policy(&loopback).is_ok(),
            "remote_plaintext": code(transport_policy(&remote)),
            "unsupported_scheme": code(transport_policy(&scheme)),
        }),
        vec![
            transport_policy(&remote.clone()).err().map(|f| f.reason().to_owned()),
            transport_policy(&scheme.clone()).err().map(|f| f.reason().to_owned()),
        ]
        .into_iter()
        .flatten()
        .collect(),
    ))
}

/// 每个场景都用**真实 socket**跑一遍真实 `probe`。
fn auth_section() -> Result<(Value, Vec<String>), String> {
    let mut reasons: Vec<String> = Vec::new();
    let mut row = |plan: Vec<(u16, &'static str)>,
                   kind: Kind,
                   auth: AuthScheme,
                   env_name: &str|
     -> Result<(String, Probe), String> {
        let peer = Peer::start(plan).map_err(|error| format!("peer start failed: {error}"))?;
        let item = connection(kind, &peer.url(), auth, env_name);
        let probe = connection_probe::inspect(&item, 1_700_000_000_000);
        let label = outcome_label(&probe);
        drop(peer);
        reasons.push(probe.reason.clone());
        Ok((label, probe))
    };

    let (accepted, _) = row(
        vec![(200, r#"{"object":"list","data":[{"id":"local-model"}]}"#)],
        Kind::CustomEndpoint,
        AuthScheme::Bearer,
        FIXTURE_ENV,
    )?;
    let (rejected_401, probe_401) = row(
        vec![(401, "{}")],
        Kind::CustomEndpoint,
        AuthScheme::Bearer,
        FIXTURE_ENV,
    )?;
    let (forbidden_403, probe_403) = row(
        vec![(403, "{}")],
        Kind::CustomEndpoint,
        AuthScheme::Bearer,
        FIXTURE_ENV,
    )?;
    let (rate_limited_429, probe_429) = row(
        vec![(429, "{}")],
        Kind::CustomEndpoint,
        AuthScheme::Bearer,
        FIXTURE_ENV,
    )?;
    let (credential_missing, probe_missing) = row(
        vec![(200, "{}")],
        Kind::CustomEndpoint,
        AuthScheme::Bearer,
        MISSING_ENV,
    )?;
    let (custom_header_scheme, probe_header) = row(
        vec![(200, r#"{"object":"list"}"#)],
        Kind::CustomEndpoint,
        AuthScheme::Header {
            name: "x-api-key".into(),
        },
        FIXTURE_ENV,
    )?;

    // 401/403 有专门状态；其余被归并进 Error 的必须靠失败码区分。
    let mut reasons_all = reasons.clone();
    reasons_all.extend([
        probe_401.reason.clone(),
        probe_403.reason.clone(),
        probe_429.reason.clone(),
        probe_missing.reason.clone(),
        probe_header.reason.clone(),
    ]);
    Ok((
        json!({
            "accepted": accepted,
            "rejected_401": rejected_401,
            "forbidden_403": forbidden_403,
            "rate_limited_429": rate_limited_429,
            "credential_missing": credential_missing,
            "custom_header_scheme": custom_header_scheme,
        }),
        reasons_all,
    ))
}

/// 轮换：同一个连接读两次，中间换掉环境变量的值。
fn rotation_section() -> Result<(Value, Vec<String>), String> {
    let peer = Peer::start(vec![(401, "{}"), (200, r#"{"object":"list"}"#)])
        .map_err(|error| format!("peer start failed: {error}"))?;
    let item = connection(
        Kind::CustomEndpoint,
        &peer.url(),
        AuthScheme::Bearer,
        FIXTURE_ENV,
    );

    set_env(FIXTURE_ENV, FIRST_SECRET);
    let before = connection_probe::inspect(&item, 1);
    let seen_before = peer.authored();
    set_env(FIXTURE_ENV, SECOND_SECRET);
    let after = connection_probe::inspect(&item, 2);
    let seen_after = peer.authored();

    let distinct = match (seen_before.first(), seen_after.last()) {
        (Some(first), Some(second)) => first != second,
        _ => false,
    };
    let reasons = vec![before.reason.clone(), after.reason.clone()];
    let value = json!({
        "before": outcome_label(&before),
        "after": outcome_label(&after),
        // 探针每次读取都重新取环境变量，不缓存上一次的值。
        "probe_reads_source_each_time": distinct,
        "distinct_bearer_values": distinct,
    });
    drop(peer);
    Ok((value, reasons))
}

/// 代理隔离：把进程代理环境变量指向一个「代理对端」，请求必须仍然打到目标对端。
fn proxy_section() -> Result<(Value, Vec<String>), String> {
    let target = Peer::start(vec![(200, r#"{"object":"list"}"#)])
        .map_err(|error| format!("target peer failed: {error}"))?;
    let proxy = Peer::start(vec![(200, "{}")]).map_err(|error| format!("proxy peer failed: {error}"))?;
    let proxy_url = proxy.url();
    for name in ["HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "http_proxy", "all_proxy"] {
        set_env(name, &proxy_url);
    }

    // 必须是**新构造**的客户端：共享实例是进程启动时建的，
    // 复用它会让这条断言变成「验证一个跟环境无关的缓存」。
    let http = Http::fresh().map_err(|failure| failure.reason().to_owned())?;
    let url = url::Url::parse(&target.url()).map_err(|_| "bad target url".to_owned())?;
    let result = http.get_json(url, &AuthHeader::Bearer(FIRST_SECRET.to_owned()));
    let reasons = match &result {
        Ok(_) => Vec::new(),
        Err(failure) => vec![failure.reason().to_owned()],
    };
    let value = json!({
        "env_proxy_configured": std::env::var("ALL_PROXY").is_ok(),
        "target_peer_requests": target.requests(),
        "proxy_peer_requests": proxy.requests(),
        "request_succeeded": result.is_ok(),
    });
    Ok((value, reasons))
}

/// 探针自己改进程环境：headless 单次运行、随即退出。
/// 这不是产品路径上的行为，只用来证明「环境里的代理与轮换值会被正确对待」。
fn set_env(name: &str, value: &str) {
    std::env::set_var(name, value);
}

fn run() -> Result<String, String> {
    set_env(FIXTURE_ENV, FIRST_SECRET);
    std::env::remove_var(MISSING_ENV);

    let (transport, mut reasons) = transport_section()?;
    let (auth, auth_reasons) = auth_section()?;
    reasons.extend(auth_reasons);
    let (rotation, rotation_reasons) = rotation_section()?;
    reasons.extend(rotation_reasons);
    let (proxy, proxy_reasons) = proxy_section()?;
    reasons.extend(proxy_reasons);

    let mut receipt = json!({
        "schema_version": 1,
        "pid": std::process::id(),
        "transport_policy": transport,
        "auth_taxonomy": auth,
        "rotation": rotation,
        "proxy_isolation": proxy,
        "leakage": {
            "receipt_contains_secret": false,
            "status_reason_contains_secret": false,
        },
    });

    // 泄漏自检：把真实回执与状态文案分别过一遍夹具值。两者都必须看不到它。
    let text = serde_json::to_string(&receipt).map_err(|_| "receipt encode failed".to_owned())?;
    let in_receipt = [FIRST_SECRET, SECOND_SECRET]
        .iter()
        .any(|secret| text.contains(secret));
    let in_reasons = [FIRST_SECRET, SECOND_SECRET]
        .iter()
        .any(|secret| reasons.iter().any(|reason| reason.contains(secret)));
    receipt["leakage"] = json!({
        "receipt_contains_secret": in_receipt,
        "status_reason_contains_secret": in_reasons,
    });

    serde_json::to_string(&receipt).map_err(|_| "receipt encode failed".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 门禁（scripts/test-gateway-auth.py）断言的键与取值在这里**再钉一遍**。
    /// 两道门读的是同一份契约：只靠 Python 那份，Rust 侧改了回执不会在本仓
    /// 自己的全量测试里出现红——那正是「改了没人发现」的老问题。
    #[test]
    fn receipt_matches_the_frozen_gate_contract() {
        let text = run().unwrap();
        let receipt: Value = serde_json::from_str(&text).unwrap();

        assert_eq!(receipt["schema_version"], 1);
        assert!(receipt["pid"].as_u64().is_some_and(|pid| pid > 0));

        let transport = &receipt["transport_policy"];
        assert_eq!(transport["https_accepted"], true);
        assert_eq!(transport["loopback_http_accepted"], true);
        assert_eq!(transport["remote_plaintext"], "insecure_transport");
        assert_eq!(transport["unsupported_scheme"], "invalid_address");

        let auth = &receipt["auth_taxonomy"];
        assert_eq!(auth["accepted"], "readable");
        assert_eq!(auth["rejected_401"], "auth_failed");
        assert_eq!(auth["forbidden_403"], "permission_denied");
        assert_eq!(auth["rate_limited_429"], "rate_limited");
        assert_eq!(auth["credential_missing"], "credential_unavailable");
        assert_eq!(auth["custom_header_scheme"], "readable");

        let rotation = &receipt["rotation"];
        assert_eq!(rotation["before"], "auth_failed");
        assert_eq!(rotation["after"], "readable");
        assert_eq!(rotation["distinct_bearer_values"], true);
        assert_eq!(rotation["probe_reads_source_each_time"], true);

        let proxy = &receipt["proxy_isolation"];
        assert_eq!(proxy["env_proxy_configured"], true);
        assert_eq!(proxy["target_peer_requests"], 1);
        assert_eq!(proxy["proxy_peer_requests"], 0);
        assert_eq!(proxy["request_succeeded"], true);

        assert_eq!(receipt["leakage"]["receipt_contains_secret"], false);
        assert_eq!(receipt["leakage"]["status_reason_contains_secret"], false);
    }

    /// 契约的键集也必须被钉住：多一个键就是门禁与探针各说各话的开始。
    #[test]
    fn receipt_key_set_is_exactly_the_contract() {
        let receipt: Value = serde_json::from_str(&run().unwrap()).unwrap();
        let keys = |value: &Value| -> Vec<String> {
            value
                .as_object()
                .map(|map| map.keys().cloned().collect())
                .unwrap_or_default()
        };
        assert_eq!(
            keys(&receipt),
            vec![
                "auth_taxonomy",
                "leakage",
                "pid",
                "proxy_isolation",
                "rotation",
                "schema_version",
                "transport_policy",
            ]
        );
        assert_eq!(
            keys(&receipt["transport_policy"]),
            vec![
                "https_accepted",
                "loopback_http_accepted",
                "remote_plaintext",
                "unsupported_scheme"
            ]
        );
        assert_eq!(
            keys(&receipt["proxy_isolation"]),
            vec![
                "env_proxy_configured",
                "proxy_peer_requests",
                "request_succeeded",
                "target_peer_requests"
            ]
        );
        assert_eq!(
            keys(&receipt["leakage"]),
            vec![
                "receipt_contains_secret",
                "status_reason_contains_secret"
            ]
        );
    }
}

