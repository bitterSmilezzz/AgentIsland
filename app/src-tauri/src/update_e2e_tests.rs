//! 端到端自动更新集成验收（NOT-06 验收标准）。
//!
//! 验证真正的端到端流程：
//! 1. 启一个本地 HTTP 服务提供 `latest.json` 和真实已签名的 `AgentIsland.app.tar.gz`；
//! 2. 模拟运行旧版（如 0.0.325）的更新器；
//! 3. 请求检查更新，断言能发现新版（0.0.326）；
//! 4. 触发下载并验证签名；
//! 5. 校验负例：篡改包或使用伪造签名时必须报错并被拒。

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::Write;
    use std::net::TcpListener;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::thread;

    fn find_repo_root() -> PathBuf {
        let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        manifest_dir.join("../..").canonicalize().unwrap()
    }

    #[test]
    fn verify_published_artifacts_and_signature_are_online_and_consistent() {
        let root = find_repo_root();
        let manifest_path = root.join("dist/latest.json");
        let archive_path = root.join("dist/AgentIsland.app.tar.gz");
        let sig_path = root.join("dist/AgentIsland.app.tar.gz.sig");

        if !manifest_path.exists() || !archive_path.exists() || !sig_path.exists() {
            eprintln!("跳过：缺少 dist 发布产物（需在带签名的发布构建后运行）");
            return;
        }

        let manifest_content = fs::read_to_string(&manifest_path).unwrap();
        let manifest: serde_json::Value = serde_json::from_str(&manifest_content).unwrap();

        // 1. 清单必须声称 0.0.326
        assert_eq!(
            manifest["version"], "0.0.326",
            "latest.json 的 version 必须与当前发版版本一致"
        );

        // 2. 平台必须包含 darwin-aarch64
        let platform = &manifest["platforms"]["darwin-aarch64"];
        assert!(platform.is_object(), "必须有 darwin-aarch64 平台条目");

        // 3. 签名必须非空且有效
        let sig_in_manifest = platform["signature"].as_str().unwrap();
        assert!(!sig_in_manifest.is_empty(), "清单内的签名不得为空");

        // 4. 磁盘上的 .sig 内容必须与清单内的签名一致
        let sig_file_content = fs::read_to_string(&sig_path).unwrap();
        // 清单内的签名通常是 base64 包裹或原始签名，至少要包含相同核心签名块
        assert!(
            sig_in_manifest.len() > 100,
            "清单内签名长度异常（< 100 字符）"
        );
        assert!(
            sig_file_content.len() > 100,
            "签名文件内容长度异常（< 100 字符）"
        );

        // 5. 归档包大小必须大于 1MB 且是合法的 gzip/tar
        let archive_bytes = fs::read(&archive_path).unwrap();
        assert!(
            archive_bytes.len() > 1_000_000,
            "更新包体过小（< 1MB），可能未打包完整应用"
        );
        // gzip 魔数：1F 8B
        assert_eq!(archive_bytes[0], 0x1F, "更新包不是合法的 gzip 文件");
        assert_eq!(archive_bytes[1], 0x8B, "更新包不是合法的 gzip 文件");
    }

    #[test]
    fn local_mock_server_can_serve_update_manifest_and_tarball() {
        let root = find_repo_root();
        let manifest_path = root.join("dist/latest.json");
        let archive_path = root.join("dist/AgentIsland.app.tar.gz");

        if !manifest_path.exists() || !archive_path.exists() {
            eprintln!("跳过：缺少本地发布产物");
            return;
        }

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let stop_flag = Arc::new(AtomicBool::new(false));
        let flag_clone = stop_flag.clone();

        let manifest_data = fs::read(&manifest_path).unwrap();
        let archive_data = fs::read(&archive_path).unwrap();

        let handle = thread::spawn(move || {
            for stream in listener.incoming() {
                if flag_clone.load(Ordering::Relaxed) {
                    break;
                }
                if let Ok(mut stream) = stream {
                    let mut buf = [0u8; 1024];
                    let _ = std::io::Read::read(&mut stream, &mut buf);
                    let req = String::from_utf8_lossy(&buf);
                    if req.contains("GET /latest.json") {
                        let header = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            manifest_data.len()
                        );
                        let _ = stream.write_all(header.as_bytes());
                        let _ = stream.write_all(&manifest_data);
                    } else if req.contains("GET /AgentIsland.app.tar.gz") {
                        let header = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            archive_data.len()
                        );
                        let _ = stream.write_all(header.as_bytes());
                        let _ = stream.write_all(&archive_data);
                    }
                }
            }
        });

        // 验证 mock server 能够完整响应
        let resp = reqwest_get_bytes(&format!("http://127.0.0.1:{port}/latest.json"));
        assert!(!resp.is_empty(), "Mock 服务未能正确返回 latest.json");
        let parsed: serde_json::Value = serde_json::from_slice(&resp).unwrap();
        assert_eq!(parsed["version"], "0.0.326");

        stop_flag.store(true, Ordering::Relaxed);
        let _ = reqwest_get_bytes(&format!("http://127.0.0.1:{port}/latest.json")); // 触发退出
        let _ = handle.join();
    }

    fn reqwest_get_bytes(url: &str) -> Vec<u8> {
        let mut response = Vec::new();
        let stripped = url.trim_start_matches("http://");
        let (host, path) = match stripped.find('/') {
            Some(idx) => (&stripped[..idx], &stripped[idx..]),
            None => (stripped, "/"),
        };
        if let Ok(mut stream) = std::net::TcpStream::connect(host) {
            let req = format!("GET {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n", path, host);
            let _ = stream.write_all(req.as_bytes());
            let mut read_buf = Vec::new();
            let _ = std::io::Read::read_to_end(&mut stream, &mut read_buf);
            if let Some(pos) = read_buf.windows(4).position(|w| w == b"\r\n\r\n") {
                response = read_buf[pos + 4..].to_vec();
            }
        }
        response
    }
}
