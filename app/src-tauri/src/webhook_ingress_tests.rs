//! Real socket regressions for parser/queue ingress and cancellation.
use super::*;
use crate::engine::{ActivityEngine, HookScheduler};
use crate::settings::Settings;
use std::io::Write;
use std::net::{Shutdown, SocketAddr, TcpStream};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

fn fixture() -> (
    LocalEventServer,
    SocketAddr,
    crate::SharedEngine,
    mpsc::Receiver<()>,
) {
    let http = LocalEventServer::bind_http("127.0.0.1:0").unwrap();
    let addr = http.server_addr().to_ip().unwrap();
    let (tx, rx) = mpsc::channel();
    let (wake_tx, wake_rx) = mpsc::sync_channel(1);
    let mut activity = ActivityEngine::new(Settings::default(), rx);
    activity.disable_outbound();
    let engine = Arc::new(Mutex::new(activity));
    let server = LocalEventServer::serve_bound(http, tx, engine.clone(), wake_tx);
    (server, addr, engine, wake_rx)
}

fn connect(addr: SocketAddr) -> TcpStream {
    let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(1)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(4)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(1)))
        .unwrap();
    stream
}

fn header(addr: SocketAddr, length: usize) -> String {
    format!("POST /notify HTTP/1.1\r\nHost: {addr}\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n")
}

fn response(stream: &mut TcpStream) {
    let mut text = String::new();
    stream.read_to_string(&mut text).unwrap();
    assert!(text.starts_with("HTTP/1.1 200 "), "{text}");
}

/// Reads exactly one response (headers + Content-Length body), leaving the socket open.
fn keep_alive_response(stream: &mut TcpStream) {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        assert_eq!(stream.read(&mut byte).unwrap(), 1, "响应头未读完");
        head.push(byte[0]);
    }
    let head = String::from_utf8_lossy(&head).to_string();
    assert!(head.starts_with("HTTP/1.1 200 "), "{head}");
    let length: usize = head
        .lines()
        .find_map(|line| line.strip_prefix("Content-Length: "))
        .expect("响应须带 Content-Length")
        .trim()
        .parse()
        .unwrap();
    let mut body = vec![0u8; length];
    stream.read_exact(&mut body).unwrap();
}

fn keep_alive_request(body: &str) -> String {
    format!(
        "POST /notify HTTP/1.1\r\nConnection: keep-alive\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
}

// A consumer running the production wait/consume path; no test-driven manual dequeue.
fn consumer(
    engine: crate::SharedEngine,
    wake: mpsc::Receiver<()>,
    count: usize,
) -> std::thread::JoinHandle<crate::notify_latency::LatencySummary> {
    std::thread::spawn(move || {
        let scheduler = HookScheduler::new(wake);
        let deadline = Instant::now() + Duration::from_secs(6);
        loop {
            match scheduler.wait_and_consume(&engine, deadline, || {}) {
                Some(state) if state.notify_latency.samples >= count => {
                    return state.notify_latency
                }
                Some(_) => (),
                None => panic!("consumer did not receive {count} events"),
            }
        }
    })
}

#[test]
fn internal_http_queue_congestion_is_included_in_latency() {
    let (server, addr, engine, wake) = fixture();
    let consumer = consumer(engine, wake, 21);
    // Large body is streamed in the application and occupies the sole HTTP consumer.
    let mut blocker = connect(addr);
    let body = format!("{}{}", r#"{"agent":"ingress-blocker"}"#, " ".repeat(2048));
    blocker
        .write_all(header(addr, body.len()).as_bytes())
        .unwrap();
    blocker.write_all(&body.as_bytes()[..1]).unwrap();
    // Complete small bodies are parsed in tiny_http and wait in its messages queue.
    let mut queued: Vec<_> = (0..20)
        .map(|index| {
            let small = format!(r#"{{"agent":"ingress-queued","id":"queued-{index}"}}"#);
            let mut stream = connect(addr);
            stream
                .write_all(format!("{}{small}", header(addr, small.len())).as_bytes())
                .unwrap();
            stream
        })
        .collect();
    std::thread::sleep(Duration::from_millis(1250));
    blocker.write_all(&body.as_bytes()[1..]).unwrap();
    response(&mut blocker);
    for stream in &mut queued {
        response(stream);
    }
    let summary = consumer.join().unwrap();
    drop(server);
    eprintln!("internal queue congestion: {summary:?}");
    assert_eq!(summary.samples, 21);
    assert_eq!(
        summary.meets_target,
        Some(false),
        "queued slow requests must not look fast"
    );
    assert!(summary.p95_ms.unwrap() >= 1000);
}

fn slow_parse(headers: bool) {
    let (server, addr, engine, wake) = fixture();
    let consumer = consumer(engine, wake, 20);
    let bodies: Vec<_> = (0..20).map(|index|
        format!(r#"{{"agent":"ingress-slow-parser","id":"parse-{index}"}}"#)).collect();
    let mut streams: Vec<_> = bodies.iter()
        .map(|body| {
            let mut stream = connect(addr);
            let prefix = if headers {
                "POST /notify HTTP/1.1\r\n".to_string()
            } else {
                format!("{}{}", header(addr, body.len()), &body[..1])
            };
            stream.write_all(prefix.as_bytes()).unwrap();
            stream
        })
        .collect();
    std::thread::sleep(Duration::from_millis(1250));
    for (stream, body) in streams.iter_mut().zip(&bodies) {
        let rest = if headers {
            format!(
                "Host: {addr}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
        } else {
            body[1..].to_string()
        };
        stream.write_all(rest.as_bytes()).unwrap();
    }
    for stream in &mut streams {
        response(stream);
    }
    let summary = consumer.join().unwrap();
    drop(server);
    eprintln!("slow parser headers={headers}: {summary:?}");
    assert_eq!(summary.samples, 20);
    assert_eq!(
        summary.meets_target,
        Some(false),
        "parser delay must be measured before recv_timeout"
    );
    assert!(summary.p95_ms.unwrap() >= 1000);
}

#[test]
fn slow_small_body_is_included_in_latency() {
    slow_parse(false);
}

#[test]
fn slow_headers_are_included_in_latency() {
    slow_parse(true);
}

#[test]
fn shutdown_cancels_partial_body_while_client_stays_connected() {
    let (server, addr, engine, _wake) = fixture();
    let mut stream = connect(addr);
    stream.write_all(header(addr, 4096).as_bytes()).unwrap();
    stream.write_all(b"{").unwrap();
    std::thread::sleep(Duration::from_millis(100));
    let (done_tx, done_rx) = mpsc::channel();
    let started = Instant::now();
    let dropper = std::thread::spawn(move || {
        drop(server);
        done_tx.send(()).unwrap();
    });
    let finished = done_rx.recv_timeout(Duration::from_millis(750)).is_ok();
    let elapsed = started.elapsed();
    // Rescue the baseline implementation on failure so the test itself never hangs.
    let _ = stream.shutdown(Shutdown::Both);
    dropper.join().unwrap();
    assert!(
        finished,
        "Drop blocked on a live partial body for {elapsed:?}"
    );
    eprintln!("partial body shutdown: {elapsed:?}");
    assert_eq!(engine.lock().unwrap().notify_latency.samples(), 0);
}

#[test]
fn keep_alive_idle_wait_is_not_charged_to_the_next_request() {
    let (server, addr, engine, wake) = fixture();
    let consumer = consumer(engine, wake, 2);
    let mut stream = connect(addr);
    stream
        .write_all(keep_alive_request(r#"{"agent":"keep-alive"}"#).as_bytes())
        .unwrap();
    keep_alive_response(&mut stream);
    // Idle longer than the 2s request read timeout on the open connection: the
    // next request must arrive intact and must not look late.
    std::thread::sleep(Duration::from_millis(2500));
    stream
        .write_all(keep_alive_request(r#"{"agent":"keep-alive"}"#).as_bytes())
        .unwrap();
    keep_alive_response(&mut stream);
    let summary = consumer.join().unwrap();
    drop(stream);
    drop(server);
    eprintln!("keep-alive idle: {summary:?}");
    assert_eq!(summary.samples, 2);
    assert_eq!(summary.discarded, 0, "空闲等待不得变成负样本");
    assert!(
        summary.p95_ms.unwrap() < 1000,
        "1.5s keep-alive 空闲不得计入下一次请求延迟：{summary:?}"
    );
}

#[test]
fn partial_header_and_small_body_connections_are_cancelled_on_shutdown() {
    let (server, addr, engine, _wake) = fixture();
    let mut header_stream = connect(addr);
    header_stream
        .write_all(b"POST /notify HTTP/1.1\r\nHost:")
        .unwrap();
    let mut body_stream = connect(addr);
    body_stream
        .write_all(format!("{}{{", header(addr, 100)).as_bytes())
        .unwrap();
    std::thread::sleep(Duration::from_millis(100));
    let started = Instant::now();
    drop(server);
    assert!(started.elapsed() < Duration::from_millis(750));
    for stream in [&mut header_stream, &mut body_stream] {
        let mut result = Vec::new();
        let read = stream.read_to_end(&mut result);
        assert!(
            !read.as_ref().is_err_and(|error| matches!(
                error.kind(),
                std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
            )),
            "server must cancel parser sockets"
        );
    }
    assert_eq!(engine.lock().unwrap().consume_hook_events(), 0);
}

#[test]
fn partial_header_and_bodies_time_out_without_becoming_events() {
    let (server, addr, engine, _wake) = fixture();
    let mut streams: Vec<_> = [None, Some(100), Some(4096)]
        .into_iter()
        .map(|length| {
            let mut stream = connect(addr);
            let bytes = length
                .map(|n| format!("{}{{", header(addr, n)))
                .unwrap_or_else(|| "POST /notify HTTP/1.1\r\nHost:".into());
            stream.write_all(bytes.as_bytes()).unwrap();
            stream
        })
        .collect();
    let started = Instant::now();
    for stream in &mut streams {
        let mut result = Vec::new();
        let read = stream.read_to_end(&mut result);
        assert!(
            !read.as_ref().is_err_and(|error| matches!(
                error.kind(),
                std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
            )),
            "server must close timed out sockets"
        );
    }
    assert!(started.elapsed() < Duration::from_secs(4));
    let mut engine = engine.lock().unwrap();
    assert_eq!(engine.consume_hook_events(), 0);
    assert_eq!(engine.notify_latency.samples(), 0);
    drop(engine);
    drop(server);
}

#[test]
fn truncated_large_body_with_valid_json_prefix_is_rejected() {
    let (server, addr, engine, _wake) = fixture();
    let mut stream = connect(addr);
    let prefix = r#"{"agent":"must-not-enqueue"}"#;
    stream
        .write_all(format!("{}{prefix}", header(addr, 4096)).as_bytes())
        .unwrap();
    stream.shutdown(Shutdown::Write).unwrap();
    let mut result = String::new();
    stream.read_to_string(&mut result).unwrap();
    assert!(result.starts_with("HTTP/1.1 400 "), "{result}");
    let mut engine = engine.lock().unwrap();
    assert_eq!(engine.consume_hook_events(), 0);
    assert_eq!(engine.notify_latency.samples(), 0);
    drop(engine);
    drop(server);
}
