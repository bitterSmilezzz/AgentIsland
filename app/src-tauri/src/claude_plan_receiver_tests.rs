use super::*;
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::{symlink, PermissionsExt};
use std::sync::atomic::AtomicU64;
static NEXT: AtomicU64 = AtomicU64::new(0);
const SESSION: &str = "019c6e27-e55b-73d1-87d8-4e01f1f75043";
struct Fixture {
    folder: PathBuf,
    root: PathBuf,
    _sandbox: crate::testutil::Sandbox,
    source: PathBuf,
    path: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let folder = PathBuf::from(format!(
            "/private/tmp/agentisland-plan-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        assert!(!folder.exists());
        DirBuilder::new().mode(0o700).create(&folder).unwrap();
        let root = folder.join("endpoint");
        let sandbox = crate::testutil::Sandbox::new("plan-source");
        let source = sandbox.path().canonicalize().unwrap();
        let path = source.join(format!("{SESSION}.jsonl"));
        std::fs::write(&path, "").unwrap();
        Self {
            folder,
            root,
            _sandbox: sandbox,
            source,
            path,
        }
    }
    fn receiver(&self) -> Receiver {
        Receiver::start(&self.root, vec![self.source.clone()]).unwrap()
    }
    fn payload(&self, call: &str) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({"hook_event_name":"PreToolUse","tool_name":"ExitPlanMode","session_id":SESSION,"tool_use_id":call,"transcript_path":self.path,"tool_input":{"plan":"# 对应方案\n只读展示。"}})).unwrap()
    }
    fn doc(&self, call: &str) -> serde_json::Value {
        serde_json::json!({"type":"assistant","uuid":"019c7714-3b77-74d1-9866-e1f484aae2ab","sessionId":SESSION,"isSidechain":false,"message":{"content":[{"type":"tool_use","id":call,"name":"ExitPlanMode","input":{}}]}})
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.folder);
    }
}
#[test]
fn kernel_transport_accepts_exact_plan_then_stop_clears_body_and_owned_endpoint() {
    let f = Fixture::new();
    let mut receiver = f.receiver();
    let cache = receiver.cache();
    assert_eq!(std::fs::metadata(&f.root).unwrap().mode() & 0o777, 0o700);
    assert_eq!(
        std::fs::metadata(f.root.join("receiver.sock"))
            .unwrap()
            .mode()
            & 0o777,
        0o600
    );
    send(&f.root, &f.payload("p"), Instant::now() + COLLECT).unwrap();
    let doc = f.doc("p");
    let r = cache
        .lock()
        .unwrap()
        .bind_selected(&f.path, &doc, &doc["message"]["content"][0], Instant::now())
        .unwrap();
    assert!(cache
        .lock()
        .unwrap()
        .read_bound(
            &f.path,
            &doc,
            &doc["message"]["content"][0],
            &r,
            Instant::now()
        )
        .unwrap()
        .contains("对应方案"));
    send(&f.root, &f.payload("p"), Instant::now() + COLLECT).unwrap();
    assert!(receiver.status().unwrap().running);
    receiver.stop();
    assert!(!receiver.status().unwrap().running);
    assert!(!f.root.exists());
    assert!(cache
        .lock()
        .unwrap()
        .read_bound(
            &f.path,
            &doc,
            &doc["message"]["content"][0],
            &r,
            Instant::now()
        )
        .is_none());
    assert!(send(&f.root, &f.payload("p"), Instant::now() + COLLECT).is_err());
    assert!(
        !f.root.exists(),
        "collector never creates/starts missing app endpoint"
    );
}
#[test]
fn active_listener_is_never_replaced_and_existing_directory_is_preserved_on_stop() {
    let f = Fixture::new();
    DirBuilder::new().mode(0o700).create(&f.root).unwrap();
    let mut receiver = f.receiver();
    let before = identity(&std::fs::metadata(f.root.join("receiver.sock")).unwrap()).unwrap();
    assert!(matches!(
        Receiver::start(&f.root, vec![f.source.clone()]),
        Err(Error::Busy)
    ));
    assert!(identity(&std::fs::metadata(f.root.join("receiver.sock")).unwrap()).unwrap() == before);
    send(&f.root, &f.payload("p"), Instant::now() + COLLECT).unwrap();
    receiver.stop();
    assert!(f.root.is_dir());
    assert!(!f.root.join("receiver.sock").exists());
}
#[test]
fn only_verified_stale_socket_can_be_reclaimed_not_regular_file_or_symlink() {
    let f = Fixture::new();
    DirBuilder::new().mode(0o700).create(&f.root).unwrap();
    let path = f.root.join("receiver.sock");
    let listener = UnixListener::bind(&path).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    drop(listener);
    let mut receiver = f.receiver();
    send(&f.root, &f.payload("p"), Instant::now() + COLLECT).unwrap();
    receiver.stop();
    std::fs::write(&path, "existing unrelated file").unwrap();
    assert!(matches!(
        Receiver::start(&f.root, vec![f.source.clone()]),
        Err(Error::Namespace)
    ));
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "existing unrelated file"
    );
    std::fs::remove_file(&path).unwrap();
    symlink(&f.path, &path).unwrap();
    assert!(matches!(
        Receiver::start(&f.root, vec![f.source.clone()]),
        Err(Error::Namespace)
    ));
    assert!(std::fs::symlink_metadata(&path)
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(std::fs::read_to_string(&f.path).unwrap(), "");
}
#[test]
fn permissions_symlink_roots_and_path_length_fail_closed() {
    let f = Fixture::new();
    DirBuilder::new().mode(0o755).create(&f.root).unwrap();
    assert!(matches!(
        Receiver::start(&f.root, vec![f.source.clone()]),
        Err(Error::Namespace)
    ));
    assert_eq!(std::fs::metadata(&f.root).unwrap().mode() & 0o777, 0o755);
    std::fs::remove_dir(&f.root).unwrap();
    symlink(&f.source, &f.root).unwrap();
    assert!(matches!(
        Receiver::start(&f.root, vec![f.source.clone()]),
        Err(Error::Namespace)
    ));
    std::fs::remove_file(&f.root).unwrap();
    let long = PathBuf::from(format!("/private/tmp/{}", "a".repeat(120)));
    assert!(matches!(
        Receiver::start(&long, vec![f.source.clone()]),
        Err(Error::Namespace)
    ));
    assert!(!long.exists());
}
#[test]
fn truncation_overflow_multiple_frames_and_private_payload_never_bind() {
    let f = Fixture::new();
    let mut receiver = f.receiver();
    for frame in [
        (MAX_FRAME as u32 + 1).to_be_bytes().to_vec(),
        vec![0, 0],
        vec![0, 0, 0, 0],
    ] {
        let mut client = connect(&f.root.join("receiver.sock"), Instant::now() + COLLECT).unwrap();
        client.write_all(&frame).unwrap();
        client.shutdown(std::net::Shutdown::Write).unwrap();
        let mut ack = [0];
        assert!(
            read_exact(&mut client, &mut ack, Instant::now() + COLLECT).is_err() || ack[0] != 0
        );
    }
    let body = f.payload("p");
    let mut client = connect(&f.root.join("receiver.sock"), Instant::now() + COLLECT).unwrap();
    client
        .write_all(&(body.len() as u32).to_be_bytes())
        .unwrap();
    client.write_all(&body).unwrap();
    client.write_all(&[1]).unwrap();
    client.shutdown(std::net::Shutdown::Write).unwrap();
    let mut ack = [0];
    assert!(read_exact(&mut client, &mut ack, Instant::now() + COLLECT).is_err() || ack[0] != 0);
    let mut private: serde_json::Value = serde_json::from_slice(&body).unwrap();
    private["tool_input"]["isSecret"] = true.into();
    assert_eq!(
        send(
            &f.root,
            &serde_json::to_vec(&private).unwrap(),
            Instant::now() + COLLECT
        ),
        Err(Error::Rejected)
    );
    let doc = f.doc("p");
    assert!(receiver
        .cache()
        .lock()
        .unwrap()
        .bind_selected(&f.path, &doc, &doc["message"]["content"][0], Instant::now())
        .is_none());
    receiver.stop();
}
#[test]
fn held_connection_has_total_deadline_and_listener_serves_next_request() {
    let f = Fixture::new();
    let mut receiver = f.receiver();
    let held = connect(&f.root.join("receiver.sock"), Instant::now() + COLLECT).unwrap();
    let start = Instant::now();
    send(&f.root, &f.payload("p"), Instant::now() + COLLECT).unwrap();
    assert!(start.elapsed() < Duration::from_secs(2));
    drop(held);
    receiver.stop();
}
#[test]
fn namespace_replacement_stops_receiver_and_cleans_old_socket_without_deleting_new_objects() {
    let f = Fixture::new();
    let mut receiver = f.receiver();
    send(&f.root, &f.payload("p"), Instant::now() + COLLECT).unwrap();
    let cache = receiver.cache();
    let doc = f.doc("p");
    let r = cache
        .lock()
        .unwrap()
        .bind_selected(&f.path, &doc, &doc["message"]["content"][0], Instant::now())
        .unwrap();
    let old = f.root.with_extension("moved");
    std::fs::rename(&f.root, &old).unwrap();
    DirBuilder::new().mode(0o700).create(&f.root).unwrap();
    std::fs::write(f.root.join("receiver.sock"), "new unrelated file").unwrap();
    let limit = Instant::now() + Duration::from_secs(2);
    while receiver.status().unwrap().running && Instant::now() < limit {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(!receiver.status().unwrap().running);
    receiver.stop();
    assert!(!old.join("receiver.sock").exists());
    assert_eq!(
        std::fs::read_to_string(f.root.join("receiver.sock")).unwrap(),
        "new unrelated file"
    );
    assert!(cache
        .lock()
        .unwrap()
        .read_bound(
            &f.path,
            &doc,
            &doc["message"]["content"][0],
            &r,
            Instant::now()
        )
        .is_none());
    std::fs::remove_dir_all(old).unwrap();
}
#[test]
fn stdin_is_bounded_and_held_pipe_cannot_block_forever() {
    let (reader, mut writer) = UnixStream::pair().unwrap();
    writer.write_all(b"{} ").unwrap();
    writer.shutdown(std::net::Shutdown::Write).unwrap();
    assert_eq!(
        stdin_bytes(reader.as_raw_fd(), Instant::now() + COLLECT).unwrap(),
        b"{} "
    );
    let (reader, writer) = UnixStream::pair().unwrap();
    let start = Instant::now();
    assert!(stdin_bytes(
        reader.as_raw_fd(),
        Instant::now() + Duration::from_millis(40)
    )
    .is_err());
    assert!(start.elapsed() < Duration::from_secs(1));
    drop(writer);
    let (reader, mut writer) = UnixStream::pair().unwrap();
    let thread = std::thread::spawn(move || {
        let _ = writer.write_all(&vec![b'a'; MAX_FRAME + 1]);
    });
    assert_eq!(
        stdin_bytes(reader.as_raw_fd(), Instant::now() + COLLECT),
        Err(Error::Input)
    );
    drop(reader);
    thread.join().unwrap();
}
#[test]
fn status_is_metadata_only_and_invalid_roots_unwind_endpoint_creation() {
    let f = Fixture::new();
    assert!(matches!(
        Receiver::start(&f.root, vec![]),
        Err(Error::Input)
    ));
    assert!(!f.root.exists());
    let receiver = f.receiver();
    let status = serde_json::to_string(&receiver.status().unwrap()).unwrap();
    assert!(
        !status.contains("plan") && !status.contains("transcript") && !status.contains("对应方案")
    );
}
#[test]
fn scheduled_purge_removes_expired_body_without_any_new_request() {
    let f = Fixture::new();
    let mut receiver =
        Receiver::start_inner(&f.root, vec![f.source.clone()], Duration::from_millis(20)).unwrap();
    let cache = receiver.cache();
    let doc = f.doc("p");
    let old = Instant::now() - Duration::from_secs(15 * 60 + 1);
    let r = {
        let mut store = cache.lock().unwrap();
        store.ingest(&f.payload("p"), old).unwrap();
        store
            .bind_selected(&f.path, &doc, &doc["message"]["content"][0], old)
            .unwrap()
    };
    let limit = Instant::now() + COLLECT;
    loop {
        // A backward logical timestamp would still find an unpurged entry.
        if cache
            .lock()
            .unwrap()
            .read_bound(&f.path, &doc, &doc["message"]["content"][0], &r, old)
            .is_none()
        {
            break;
        }
        assert!(Instant::now() < limit, "scheduled purge did not run");
        std::thread::sleep(Duration::from_millis(10));
    }
    receiver.stop();
}
#[test]
fn trickled_frame_and_slow_ack_use_absolute_deadline_not_per_byte_timeout() {
    let (mut reader, mut writer) = UnixStream::pair().unwrap();
    let thread = std::thread::spawn(move || {
        for _ in 0..10 {
            if writer.write_all(b"x").is_err() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    });
    let start = Instant::now();
    let mut bytes = [0; 20];
    assert!(read_exact(&mut reader, &mut bytes, start + Duration::from_millis(60)).is_err());
    assert!(start.elapsed() < Duration::from_millis(200));
    drop(reader);
    thread.join().unwrap();
}
#[test]
fn unexpected_ack_and_unclosed_reply_cannot_confirm_delivery() {
    for (ack, close) in [(2u8, true), (0u8, false)] {
        let f = Fixture::new();
        let (endpoint, listener) = Endpoint::claim(&f.root).unwrap();
        listener.set_nonblocking(false).unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut header = [0; 4];
            read_exact(&mut stream, &mut header, Instant::now() + COLLECT).unwrap();
            let mut bytes = vec![0; u32::from_be_bytes(header) as usize];
            read_exact(&mut stream, &mut bytes, Instant::now() + COLLECT).unwrap();
            let mut eof = [0];
            assert_eq!(
                read_some(&stream, &mut eof, Instant::now() + COLLECT).unwrap(),
                0
            );
            write_all(&mut stream, &[ack], Instant::now() + COLLECT).unwrap();
            if !close {
                std::thread::sleep(Duration::from_millis(150));
            }
            drop(stream);
            drop(endpoint);
        });
        assert!(send(
            &f.root,
            &f.payload("p"),
            Instant::now() + Duration::from_millis(80)
        )
        .is_err());
        server.join().unwrap();
    }
}
#[test]
fn hidden_collector_rejects_extra_arguments_before_reading_stdin_or_opening_ui() {
    assert_eq!(
        super::super::cli(&["agentisland".into(), FLAG.into(), "extra".into()]),
        Some(1)
    );
    assert_eq!(
        super::super::cli(&["agentisland".into(), "status".into()]),
        None
    );
}
#[test]
fn namespace_change_during_frame_rejects_before_capture_and_preserves_replacement() {
    let f = Fixture::new();
    let mut receiver = f.receiver();
    let mut client = connect(&f.root.join("receiver.sock"), Instant::now() + COLLECT).unwrap();
    let body = f.payload("p");
    write_all(
        &mut client,
        &(body.len() as u32).to_be_bytes(),
        Instant::now() + COLLECT,
    )
    .unwrap();
    write_all(&mut client, &body[..1], Instant::now() + COLLECT).unwrap();
    let old = f.root.with_extension("moved");
    std::fs::rename(&f.root, &old).unwrap();
    DirBuilder::new().mode(0o700).create(&f.root).unwrap();
    std::fs::write(f.root.join("keep"), "replacement").unwrap();
    let _ = write_all(&mut client, &body[1..], Instant::now() + COLLECT);
    let _ = client.shutdown(std::net::Shutdown::Write);
    let mut ack = [0];
    assert!(read_exact(&mut client, &mut ack, Instant::now() + COLLECT).is_err() || ack[0] != 0);
    receiver.stop();
    let doc = f.doc("p");
    assert!(receiver
        .cache()
        .lock()
        .unwrap()
        .bind_selected(&f.path, &doc, &doc["message"]["content"][0], Instant::now())
        .is_none());
    assert_eq!(
        std::fs::read_to_string(f.root.join("keep")).unwrap(),
        "replacement"
    );
    assert!(!old.join("receiver.sock").exists());
}
