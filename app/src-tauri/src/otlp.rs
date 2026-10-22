//! OTLP 度量导出（OpenTelemetry metrics，OTLP/HTTP JSON）。
//!
//! 口径（与 GW-08 需求一致）：
//! * **默认关闭**：没有显式端点就不导出，应用不会自己去连任何地方。
//! * **不带凭据**：OTLP 端点只接受地址，不读环境变量、不带 Authorization；
//!   要鉴权请在前面放本机 collector，凭据不出本应用（与通知/连接的边界一致）。
//! * **传输策略复用既有判据**：HTTPS，或仅回环的 HTTP——在建连之前判定。
//! * **只发聚合值**：不相对于会话正文、不含路径与用户名；指标名与单位固定。
//!
//! 这里只负责「把一副快照变成 OTLP 报文并发出去」，采集与调度不在此处：
//! 调用方决定什么时候导（当前是显式的探针/命令），没有后台定时器。

use crate::connection_http::{transport_policy, Failure};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

/// OTLP 指标名：前缀 + 单位后缀，命名一次定死，改它等于改对外契约。
pub const METRIC_TOKENS: &str = "agentisland.tokens.total";
pub const METRIC_COST: &str = "agentisland.cost.total";
pub const METRIC_AGENTS_RUNNING: &str = "agentisland.agents.running";
pub const METRIC_PENDING_EVENTS: &str = "agentisland.notifications.pending";

const TIMEOUT: Duration = Duration::from_secs(5);

/// 一副要导出的快照。**只放聚合值**：会话 id、路径、用户名都不进来。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Snapshot {
    pub tokens_total: i64,
    pub cost_total: f64,
    pub agents_running: u32,
    pub pending_events: usize,
}

/// 端点解析：先按连接那套规范化（拒凭据/控制字符/超长），再过传输策略。
///
/// 两处判据都必须在**建连之前**跑完——一旦发出请求，地址本身就已经是泄漏面。
pub fn endpoint(raw: &str) -> Result<url::Url, Failure> {
    let normalized = crate::connections::normalize_url(raw).map_err(|_| Failure::InvalidAddress)?;
    let url = url::Url::parse(&normalized).map_err(|_| Failure::InvalidAddress)?;
    transport_policy(&url)?;
    Ok(url)
}

/// 组装 OTLP/HTTP JSON 报文（metrics v1，gauge）。
///
/// 用 gauge 而不是 counter：导出的是一时刻的总量，调用方不该看到「每次导出又加一遍」。
/// 这正是「本地账本不重复相加」在导出侧的同一条理由。
pub fn payload(snapshot: &Snapshot, now_ms: i64) -> Value {
    let nanos = (now_ms.max(0) as u128) * 1_000_000;
    let gauge = |name: &str, unit: &str, value: Value| {
        json!({
            "name": name,
            "unit": unit,
            "gauge": {
                "dataPoints": [{
                    "asDouble": value,
                    "timeUnixNano": nanos.to_string(),
                }]
            }
        })
    };
    json!({
        "resourceMetrics": [{
            "resource": {
                "attributes": [
                    { "key": "service.name", "value": { "stringValue": "agentisland" } },
                    { "key": "telemetry.sdk.language", "value": { "stringValue": "rust" } }
                ]
            },
            "scopeMetrics": [{
                "scope": { "name": "agentisland", "version": env!("CARGO_PKG_VERSION") },
                "metrics": [
                    gauge(METRIC_TOKENS, "1", json!(snapshot.tokens_total)),
                    gauge(METRIC_COST, "USD", json!(snapshot.cost_total)),
                    gauge(METRIC_AGENTS_RUNNING, "1", json!(snapshot.agents_running)),
                    gauge(METRIC_PENDING_EVENTS, "1", json!(snapshot.pending_events)),
                ]
            }]
        }]
    })
}

/// 报文里出现过的指标名（探针回执用；也让「发了什么」可被断言，而不是只能相信代码）。
pub fn metric_names(body: &Value) -> Vec<String> {
    body["resourceMetrics"][0]["scopeMetrics"][0]["metrics"]
        .as_array()
        .map(|metrics| {
            metrics
                .iter()
                .filter_map(|metric| metric["name"].as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

/// 一次导出的回执：成功/失败都必须说清「发了多少、带没带凭据」。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Receipt {
    pub status: u16,
    pub content_type: String,
    pub bytes_sent: usize,
    pub metrics: Vec<String>,
    /// 恒为 false：本模块没有任何认证路径。写进回执是为了让「没带凭据」可被核对，
    /// 而不是只靠读代码相信。
    pub sent_authorization: bool,
}

/// 把一份快照导到显式端点。同步、单次、有界：不重试、不跟随重定向、不排队。
pub fn export(raw_endpoint: &str, snapshot: &Snapshot, now_ms: i64) -> Result<Receipt, Failure> {
    let url = endpoint(raw_endpoint)?;
    let body = payload(snapshot, now_ms);
    let encoded = serde_json::to_vec(&body).map_err(|_| Failure::InvalidJson)?;
    send(&url, &encoded, body)
}

fn send(url: &url::Url, encoded: &[u8], body: Value) -> Result<Receipt, Failure> {
    // 自己建连而不是复用连接池：导出是一次性的显式动作，池化只会让「上一次的
    // 连接参数」有机会影响这一次——而参数里有地址与超时。
    let mut stream = connect(url)?;
    stream
        .set_read_timeout(Some(TIMEOUT))
        .and_then(|_| stream.set_write_timeout(Some(TIMEOUT)))
        .map_err(|_| Failure::Unavailable)?;
    let host = url.host_str().unwrap_or_default();
    let host = if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_string()
    };
    let port_part = url.port().map(|port| format!(":{port}")).unwrap_or_default();
    let path = if url.path().is_empty() { "/" } else { url.path() };
    let head = format!(
        "POST {path} HTTP/1.1\r\nHost: {host}{port_part}\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n",
        encoded.len()
    );
    stream
        .write_all(head.as_bytes())
        .and_then(|_| stream.write_all(encoded))
        .and_then(|_| stream.flush())
        .map_err(|_| Failure::Unavailable)?;
    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .map_err(|_| Failure::Unavailable)?;
    let text = String::from_utf8_lossy(&response);
    let status = text
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or(Failure::InvalidJson)?;
    if !(200..300).contains(&status) {
        return Err(Failure::HttpStatus(status));
    }
    let metrics = metric_names(&body);
    Ok(Receipt {
        status,
        content_type: "application/json".into(),
        bytes_sent: encoded.len(),
        metrics,
        sent_authorization: false,
    })
}

fn connect(url: &url::Url) -> Result<TcpStream, Failure> {
    let host = url.host_str().ok_or(Failure::InvalidAddress)?;
    let port = url.port_or_known_default().ok_or(Failure::InvalidAddress)?;
    let addrs = (host, port)
        .to_socket_addrs()
        .map_err(|_| Failure::Unavailable)?;
    for addr in addrs {
        if let Ok(stream) = TcpStream::connect_timeout(&addr, TIMEOUT) {
            return Ok(stream);
        }
    }
    Err(Failure::Unavailable)
}

use std::net::ToSocketAddrs;

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::thread;

    fn snapshot() -> Snapshot {
        Snapshot {
            tokens_total: 1_234,
            cost_total: 0.25,
            agents_running: 2,
            pending_events: 3,
        }
    }

    #[test]
    fn payload_is_otlp_metrics_json_with_the_fixed_metric_names() {
        let body = payload(&snapshot(), 1_700_000_000_000);
        assert_eq!(
            body["resourceMetrics"][0]["resource"]["attributes"][0]["key"],
            "service.name"
        );
        assert_eq!(
            body["resourceMetrics"][0]["resource"]["attributes"][0]["value"]["stringValue"],
            "agentisland"
        );
        assert_eq!(
            metric_names(&body),
            vec![
                METRIC_TOKENS.to_string(),
                METRIC_COST.to_string(),
                METRIC_AGENTS_RUNNING.to_string(),
                METRIC_PENDING_EVENTS.to_string()
            ]
        );
        let points = &body["resourceMetrics"][0]["scopeMetrics"][0]["metrics"][0]["gauge"]["dataPoints"][0];
        assert_eq!(points["asDouble"], 1234.0);
        assert_eq!(points["timeUnixNano"], "1700000000000000000");
        // 单位写死在报文里：读的人不需要猜 tokens 是不是「千」。
        assert_eq!(
            body["resourceMetrics"][0]["scopeMetrics"][0]["metrics"][1]["unit"],
            "USD"
        );
    }

    #[test]
    fn endpoint_policy_rejects_plaintext_off_loopback_and_credentialed_urls() {
        assert!(endpoint("http://127.0.0.1:4318/v1/metrics").is_ok(), "回环 HTTP 允许");
        assert!(endpoint("https://collector.example.invalid/v1/metrics").is_ok());
        assert_eq!(
            endpoint("http://collector.example.invalid/v1/metrics").unwrap_err().code(),
            "insecure_transport",
            "非回环明文必须建连前拒绝"
        );
        for bad in ["ftp://x/y", "https://user:pw@host/v1/metrics", "not-a-url"] {
            assert!(endpoint(bad).is_err(), "{bad} 不该被接受");
        }
    }

    #[test]
    fn export_posts_one_bounded_json_request_without_credentials() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            // 读到「头部完成 + Content-Length 指定的正文」为止，再回响应。
            // 只读一次就关闭会让客户端在 read_to_end 时撞上 RST（Unavailable）——
            // 那是夹具的错，不是被测代码的错。
            let mut request = Vec::new();
            let mut chunk = [0u8; 1024];
            loop {
                let read = stream.read(&mut chunk).unwrap();
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&chunk[..read]);
                let text = String::from_utf8_lossy(&request).to_string();
                if let Some(head_end) = text.find("\r\n\r\n") {
                    let length = text
                        .lines()
                        .find_map(|line| {
                            line.strip_prefix("Content-Length:")
                                .and_then(|value| value.trim().parse::<usize>().ok())
                        })
                        .unwrap_or(0);
                    if request.len() >= head_end + 4 + length {
                        break;
                    }
                }
            }
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}")
                .unwrap();
            stream.flush().unwrap();
            // 半关闭后把剩余输入读完再放手：带未读数据的 close 会发 RST。
            let _ = stream.shutdown(std::net::Shutdown::Write);
            let mut sink = [0u8; 256];
            while matches!(stream.read(&mut sink), Ok(size) if size > 0) {}
            String::from_utf8_lossy(&request).to_string()
        });

        let url = format!("http://127.0.0.1:{port}/v1/metrics");
        let receipt = export(&url, &snapshot(), 1_700_000_000_000)
            .unwrap_or_else(|error| panic!("回环导出应成功（{url}）：{error:?}"));
        let request = handle.join().unwrap();
        // 路径以 `/v1/metrics` 起始即可：`normalize_url` 可能补尾斜杠，
        // 那不是本模块的判据（collector 两种写法都收）。
        assert!(request.starts_with("POST /v1/metrics"), "{request}");
        assert!(request.contains("Content-Type: application/json"));
        assert!(
            !request.to_ascii_lowercase().contains("authorization"),
            "导出不许带凭据头：{request}"
        );
        assert!(request.contains(METRIC_TOKENS), "报文里应含指标名");
        assert_eq!(receipt.status, 200);
        assert_eq!(receipt.metrics.len(), 4);
        assert!(!receipt.sent_authorization);
        assert!(receipt.bytes_sent > 0);
    }
}
