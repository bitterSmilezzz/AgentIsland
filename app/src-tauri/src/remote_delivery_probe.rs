//! 外发链路的**真实二进制端到端**验收（NOT-05）。
//!
//! 与 `notify_ledger_probe`（入站账本）同一形态：显式 argv 探针、headless、
//! **从不读用户配置/钥匙串**、只对 `127.0.0.1` 上自建的本地服务说话。
//!
//! 为什么需要它：外发链路此前只有 `#[cfg(test)]` 单测（`ScriptedTransport` 注入假传输），
//! 而「真实的 `HttpTransport` 连出去、真实的对端回 429/503、真实的 Retry-After 头」
//! 这条路径在**发布产物**里没有任何验收。入站有 `not01.ledgerrestart`，
//! 外发没有对应物——这个探针补上。
//!
//! 它验证 NOT-05 的三条核心语义，且都在**真实 HTTP**上：
//! 1. 对端 429 + `Retry-After: 12` → `Throttled` 且界面文案含「12 秒后可重试」；
//! 2. 连续三次 5xx → 通道升级为故障，且**升级后不再重试**（对端只再收到 1 次请求）；
//! 3. 200 受理 → 交付态是「已受理，未确认送达」，**不冒充已送达**。
use crate::notifier::{DeliveryState, Notifier, Outcome};
use crate::remote::{Channel, ChannelConfig, Now, Policy, PresenceSignals};
use crate::render::Inputs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub(crate) static ACTIVE: AtomicBool = AtomicBool::new(false);

pub(crate) fn cli(argv: &[String]) -> Option<i32> {
    if argv.get(1).map(String::as_str) != Some("--remote-delivery-probe") {
        return None;
    }
    let result = if argv.len() == 2 {
        ACTIVE.store(true, Ordering::Relaxed);
        run()
    } else {
        Err("usage: --remote-delivery-probe".to_string())
    };
    match result {
        Ok(report) => {
            println!("{report}");
            Some(0)
        }
        Err(reason) => {
            eprintln!("remote delivery probe failed: {reason}");
            Some(2)
        }
    }
}

/// 本地对端：按脚本回状态码，并记录**每个请求的正文**（用于断言重试次数与内容）。
struct Peer {
    port: u16,
    seen: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl Peer {
    /// `script` 是逐次响应的状态码；用光后一律 200。
    fn start(script: Vec<u16>) -> Result<Peer, String> {
        let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| format!("绑定失败：{e}"))?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        listener
            .set_nonblocking(true)
            .map_err(|e| format!("非阻塞设置失败：{e}"))?;
        let seen = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (seen_thread, stop_thread) = (seen.clone(), stop.clone());
        let handle = std::thread::spawn(move || {
            let mut remaining: std::collections::VecDeque<u16> = script.into();
            while !stop_thread.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let code = remaining.pop_front().unwrap_or(200);
                        let _ = serve_one(stream, code, &seen_thread);
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(20));
                    }
                    Err(_) => break,
                }
            }
        });
        Ok(Peer {
            port,
            seen,
            stop,
            handle: Some(handle),
        })
    }

    fn url(&self) -> String {
        format!("http://127.0.0.1:{}/hook", self.port)
    }

    fn requests(&self) -> Vec<String> {
        self.seen.lock().expect("对端记录锁不应中毒").clone()
    }

    fn shutdown(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn serve_one(mut stream: TcpStream, code: u16, seen: &Arc<Mutex<Vec<String>>>) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut content_length = 0usize;
    let mut retry_after: Option<&'static str> = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            return Ok(());
        }
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            break;
        }
        let lower = trimmed.to_ascii_lowercase();
        if let Some(value) = lower.strip_prefix("content-length:") {
            content_length = value.trim().parse().unwrap_or(0);
        }
        // 只在 429 场景给 Retry-After，模拟真实限流对端
        if code == 429 {
            retry_after = Some("12");
        }
    }
    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        reader.read_exact(&mut body)?;
    }
    seen.lock()
        .expect("对端记录锁不应中毒")
        .push(String::from_utf8_lossy(&body).to_string());
    let reason = match code {
        429 => "Too Many Requests",
        503 => "Service Unavailable",
        200 => "OK",
        _ => "Error",
    };
    let extra = match retry_after {
        Some(seconds) => format!("Retry-After: {seconds}\r\n"),
        None => String::new(),
    };
    write!(
        stream,
        "HTTP/1.1 {code} {reason}\r\nContent-Length: 0\r\n{extra}Connection: close\r\n\r\n"
    )?;
    stream.flush()?;
    Ok(())
}

fn policy() -> Policy {
    Policy {
        master_enabled: true,
        ..Policy::default()
    }
}

fn custom_http(url: String) -> ChannelConfig {
    ChannelConfig {
        url_template: url,
        body_template: "title={title}&desp={body}".into(),
        ..ChannelConfig::default()
    }
}

fn inputs() -> Inputs {
    Inputs::new("Claude", crate::remote::EventKind::Attention, 0.0)
}

fn attempt(notifier: &Notifier, config: &ChannelConfig, at_ms: i64) -> Outcome {
    notifier.attempt(
        &inputs(),
        &policy(),
        Channel::CustomHttp,
        config,
        Now::at(at_ms),
        &PresenceSignals::unavailable(),
        false,
    )
}

/// 从契约夹具取四态文案——**唯一来源**，探针不自建一份。
fn contract_label(state: &str) -> Result<String, String> {
    let contract: serde_json::Value = serde_json::from_str(include_str!(
        "../tests/fixtures/contracts/notify-keys.json"
    ))
    .map_err(|e| format!("契约夹具不是合法 JSON：{e}"))?;
    contract["delivery_labels"][state]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| format!("契约夹具缺 delivery_labels.{state}"))
}

fn run() -> Result<serde_json::Value, String> {
    // ① 429 + Retry-After：真实对端给等待时长，界面文案必须显示它
    let mut limited = Peer::start(vec![429])?;
    let throttled = {
        let notifier = Notifier::new();
        let outcome = attempt(&notifier, &custom_http(limited.url()), 1_000);
        let recent = notifier.recent_view();
        let state = recent.first().map(|entry| entry.delivery.clone());
        let label = recent.first().map(|entry| entry.delivery_label.to_string());
        let text = recent.first().map(|entry| entry.text.clone()).unwrap_or_default();
        if !matches!(outcome, Outcome::Throttled { .. }) {
            return Err(format!("429 应判为 Throttled，实际 {outcome:?}"));
        }
        if !text.contains("约 12 秒后可重试") {
            return Err(format!("界面必须显示对端给的等待时长，实际文案：{text}"));
        }
        let unknown_label = contract_label("unknown")?;
        if state != Some(DeliveryState::Unknown) || label.as_deref() != Some(unknown_label.as_str())
        {
            return Err(format!("限流不能冒充任何一段交付态，实际 {state:?}/{label:?}"));
        }
        serde_json::json!({"outcome": "throttled", "text": text, "deliveryLabel": label})
    };
    let limited_requests = limited.requests().len();
    limited.shutdown();

    // ② 连续三次**临时失败** → 升级为通道故障；升级后**不再重试**（对端请求数不再增长）。
    //
    // ⚠️ 必须用 **500**，不能用 503：`transport.rs` 把 429 与 **503** 都判为 `Throttled`
    // （「服务端明确说了现在不行」），而 `Throttled` 按 NOT-05 设计**不计入升级**。
    // 探针第一版用 503，实测升级列表为空——这不是缺陷，是 503 的既定语义。
    // 真正的链路抖动（500 / 网络层错误）才累计连击。
    let mut failing = Peer::start(vec![500, 500, 500, 500, 500, 500, 500, 500])?;
    let escalation = {
        let notifier = Notifier::new();
        let config = custom_http(failing.url());
        for index in 0..3 {
            attempt(&notifier, &config, 1_000 + index * 100_000);
        }
        let after_three = failing.requests().len();
        // 生产 `Notifier::new()` = `default()`，`retry_attempts = 1`：
        // 三次临时失败各发 1 次 + 重试 1 次 = 6。显式断言，别让「碰巧对上」当证据。
        if after_three != 6 {
            return Err(format!(
                "三次临时失败应为 3×(初次+重试)=6 次请求，实际 {after_three}"
            ));
        }
        if notifier.escalated_channels() != vec!["customHTTP".to_string()] {
            return Err(format!(
                "连续三次 500 应升级 customHTTP，实际 {:?}",
                notifier.escalated_channels()
            ));
        }
        // 升级后再发一次：必须**只发一次**（不再重试），且如实返回失败
        let outcome = attempt(&notifier, &config, 401_000);
        let after_fourth = failing.requests().len();
        if !matches!(outcome, Outcome::Failed { permanent: false, .. }) {
            return Err(format!("升级不吞结果：应如实返回失败，实际 {outcome:?}"));
        }
        if after_fourth != after_three + 1 {
            return Err(format!(
                "升级后必须只发一次不重试，实际新增 {} 次请求",
                after_fourth - after_three
            ));
        }
        serde_json::json!({
            "escalated": notifier.escalated_channels(),
            "requestsAfterThree": after_three,
            "requestsAfterFourth": after_fourth,
        })
    };
    failing.shutdown();

    // ③ 200 受理 → 「已受理，未确认送达」，不冒充已送达
    let mut accepting = Peer::start(vec![200])?;
    let accepted = {
        let notifier = Notifier::new();
        let outcome = attempt(&notifier, &custom_http(accepting.url()), 1_000);
        let recent = notifier.recent_view();
        let entry = recent.first().ok_or("受理后必须有最近外发记录")?;
        if !matches!(outcome, Outcome::Accepted) {
            return Err(format!("200 应判为受理，实际 {outcome:?}"));
        }
        if !entry.accepted {
            return Err("受理后 accepted 必须为真".into());
        }
        if entry.delivery != DeliveryState::Accepted {
            return Err(format!("受理态应为 accepted，实际 {:?}", entry.delivery));
        }
        let accepted_label = contract_label("accepted")?;
        let delivered_label = contract_label("received")?;
        if entry.delivery_label != accepted_label || entry.delivery_label == delivered_label {
            return Err(format!(
                "受理不得说成已送达，实际文案：{}",
                entry.delivery_label
            ));
        }
        serde_json::json!({
            "delivery": "accepted",
            "deliveryLabel": entry.delivery_label,
            "accepted": entry.accepted,
        })
    };
    accepting.shutdown();

    Ok(serde_json::json!({
        "schemaVersion": 1,
        "pid": std::process::id(),
        "throttled": throttled,
        "throttledPeerRequests": limited_requests,
        "escalation": escalation,
        "accepted": accepted,
    }))
}
