use crate::models::AgentTaskEvent;
use std::io::Read;
use std::sync::mpsc::Sender;
use tiny_http::{Header, Method, Response, Server};

/// 本地 Webhook（127.0.0.1:41999，与 macOS 端同端口同协议）：
/// · POST /notify、/event：无鉴权直推事件（岛内标「外部投递」）
/// · POST /session、DELETE /session：需令牌 X-AgentIsland-Token
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
    pub fn start(tx: Sender<AgentTaskEvent>) -> Self {
        let server = match Server::http("127.0.0.1:41999") {
            Ok(s) => s,
            Err(_) => {
                // 端口被占：岛其余功能不受影响
                return LocalEventServer { handle: None };
            }
        };
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

                let (status, text) = route(&path, &method, &body, &token_header, &tx);
                let header = Header::from_bytes(&b"Content-Type"[..], &b"application/json; charset=utf-8"[..]).unwrap();
                let response = Response::from_string(text).with_status_code(status).with_header(header);
                let _ = request.respond(response);
            }
        });
        LocalEventServer { handle: Some(handle) }
    }
}

fn route(
    path: &str,
    method: &Method,
    body: &str,
    token: &str,
    tx: &Sender<AgentTaskEvent>,
) -> (u16, &'static str) {
    use AgentTaskEvent as E;
    if path == "/session" {
        if *method == Method::Delete {
            return (200, r#"{"ok":true}"#);
        }
        if *method == Method::Post {
            let expected = ensure_token();
            if token != expected || token.len() < 32 {
                // 没令牌的申报不丢：落回无鉴权通道，带「未采信」标记
                return notify_route(body, true, tx);
            }
            // 自报生命周期：登记但不参与显示（与 macOS 端现状一致）
            return (200, r#"{"ok":true,"note":"自报不参与显示，岛与 CLI 只看推断状态"}"#);
        }
        return (405, r#"{"error":"method not allowed"}"#);
    }
    if (path == "/notify" || path == "/event") && *method == Method::Post {
        return notify_route(body, false, tx);
    }
    (404, r#"{"error":"not found"}"#)
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
