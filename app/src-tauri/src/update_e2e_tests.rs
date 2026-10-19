//! 更新清单与包体的本地 HTTP 分发回归。
//!
//! 使用合成夹具，普通 cargo test 不依赖 dist 中上一次发布的产物。
//! 这里只证明传输字节完整；真实发布包的版本、架构和签名由发布门禁校验，
//! 不把本地 HTTP 服务测试称作旧客户端安装升级验收。

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::thread;
    use std::time::Duration;

    fn fetch(addr: std::net::SocketAddr, path: &str) -> Vec<u8> {
        let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2)).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        stream.set_write_timeout(Some(Duration::from_secs(2))).unwrap();
        write!(stream, "GET {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n").unwrap();
        let mut bytes = Vec::new();
        stream.read_to_end(&mut bytes).unwrap();
        let boundary = bytes.windows(4).position(|part| part == b"\r\n\r\n").unwrap();
        let headers = std::str::from_utf8(&bytes[..boundary]).unwrap();
        assert!(headers.starts_with("HTTP/1.1 200 OK\r\n"));
        let body = bytes[boundary + 4..].to_vec();
        assert!(headers.contains(&format!("Content-Length: {}", body.len())));
        body
    }

    #[test]
    fn local_server_distributes_complete_manifest_and_binary_bytes() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let addr = listener.local_addr().unwrap();
        let archive: Vec<u8> = (0..131_072).map(|index| (index % 256) as u8).collect();
        let manifest = serde_json::to_vec(&serde_json::json!({
            "version": env!("CARGO_PKG_VERSION"),
            "platforms": {"darwin-aarch64": {
                "url": format!("http://{addr}/AgentIsland.app.tar.gz"),
                "signature": "synthetic-transport-fixture"
            }}
        })).unwrap();
        let expected_manifest = manifest.clone();
        let expected_archive = archive.clone();
        let server = thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            for (expected_path, body) in [("/latest.json", manifest), ("/AgentIsland.app.tar.gz", archive)] {
                let mut stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            assert!(std::time::Instant::now() < deadline, "missing fixture request");
                            thread::sleep(Duration::from_millis(5));
                        }
                        Err(error) => panic!("fixture accept failed: {error}"),
                    }
                };
                stream.set_nonblocking(false).unwrap();
                stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                stream.set_write_timeout(Some(Duration::from_secs(5))).unwrap();
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    let mut byte = [0];
                    stream.read_exact(&mut byte).unwrap();
                    request.push(byte[0]);
                    assert!(request.len() < 4096);
                }
                assert!(request.starts_with(format!("GET {expected_path} HTTP/1.1\r\n").as_bytes()));
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).unwrap();
                stream.write_all(&body).unwrap();
            }
        });
        assert_eq!(fetch(addr, "/latest.json"), expected_manifest);
        assert_eq!(fetch(addr, "/AgentIsland.app.tar.gz"), expected_archive);
        server.join().unwrap();
    }
}
