//! Socket-level regressions: run without Tauri on every supported host.
use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;
use tiny_http::Server;

const IO_TIMEOUT: Duration = Duration::from_millis(250);
const DEADLINE: Duration = Duration::from_secs(2);

fn fixture(timeout: Option<Duration>) -> (Server, TcpStream) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = Server::from_listener_with_timeout(listener, None, timeout).unwrap();
    let client = TcpStream::connect(address).unwrap();
    client.set_read_timeout(Some(DEADLINE)).unwrap();
    (server, client)
}

fn partial_body(client: &mut TcpStream, connection: &str) {
    write!(
        client,
        "POST / HTTP/1.1\r\nContent-Length: 4096\r\nConnection: {connection}\r\n\r\nx"
    )
    .unwrap();
}

fn timed_out(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
    )
}

#[test]
fn partial_headers_and_buffered_body_time_out() {
    for prefix in [
        "POST / HTTP/1.1\r\nHost:",
        "POST / HTTP/1.1\r\nContent-Length: 100\r\nConnection: close\r\n\r\nx",
    ] {
        let (server, mut client) = fixture(Some(IO_TIMEOUT));
        client.write_all(prefix.as_bytes()).unwrap();
        let read = client.read_to_end(&mut Vec::new());
        // Rescue a broken implementation before asserting so no parser stays blocked.
        server.shutdown_handle().shutdown();
        let _ = client.shutdown(Shutdown::Both);
        assert!(
            !read.as_ref().is_err_and(timed_out),
            "parser socket remained open: {:?}",
            read
        );
        assert!(
            server.try_recv().unwrap().is_none(),
            "partial request was delivered"
        );
    }
}

fn streamed_body_times_out(connection: &str) {
    let (server, mut client) = fixture(Some(IO_TIMEOUT));
    partial_body(&mut client, connection);
    let mut request = server
        .recv_timeout(DEADLINE)
        .unwrap()
        .expect("streamed request");
    // Consume the prefix before waiting, so the next read must touch the socket.
    let mut first = [0];
    request.as_reader().read_exact(&mut first).unwrap();
    assert_eq!(first, [b'x']);
    let (tx, rx) = mpsc::channel();
    let reader = thread::spawn(move || {
        let result = request.as_reader().read_to_end(&mut Vec::new());
        drop(request); // Its body-draining destructor must also terminate.
        tx.send(result).unwrap();
    });
    let result = rx.recv_timeout(DEADLINE);
    server.shutdown_handle().shutdown();
    let _ = client.shutdown(Shutdown::Both);
    reader.join().unwrap();
    let read = result.expect("streamed body read/destructor did not time out");
    assert!(
        read.as_ref().is_err_and(timed_out),
        "expected body timeout: {:?}",
        read
    );
}

#[test]
fn streamed_close_body_times_out() {
    streamed_body_times_out("close");
}

#[test]
fn streamed_keep_alive_body_times_out_before_parser_waits_idle() {
    streamed_body_times_out("keep-alive");
}

/// 依赖 POSIX 套接字中断语义：在另一个线程调用 shutdown(Both) 能立即唤醒阻塞中的 read()。
/// Windows Winsock 同步阻塞 recv() 不会被本端 shutdown() 打断（需依赖 SO_RCVTIMEO 或 closesocket），
/// 因此该用例只在 Unix 目标上验收。
#[cfg(unix)]
#[test]
fn shutdown_interrupts_streamed_body_on_live_client() {
    // No IO timeout: only explicit cancellation can release the blocked reader.
    let (server, mut client) = fixture(None);
    partial_body(&mut client, "close");
    let mut request = server
        .recv_timeout(DEADLINE)
        .unwrap()
        .expect("streamed request");
    let mut first = [0];
    request.as_reader().read_exact(&mut first).unwrap();
    let (started_tx, started_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    let reader = thread::spawn(move || {
        started_tx.send(()).unwrap();
        let result = request.as_reader().read_to_end(&mut Vec::new());
        drop(request);
        done_tx.send(result).unwrap();
    });
    started_rx.recv_timeout(DEADLINE).unwrap();
    thread::sleep(Duration::from_millis(25));
    server.shutdown_handle().shutdown();
    let result = done_rx.recv_timeout(DEADLINE);
    let _ = client.shutdown(Shutdown::Both);
    reader.join().unwrap();
    let read = result.expect("shutdown did not interrupt streamed body read/destructor");
    assert!(
        !read.as_ref().is_err_and(timed_out),
        "read expired instead of being cancelled: {:?}",
        read
    );
}
