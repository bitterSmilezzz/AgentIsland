use super::*;
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::{symlink, PermissionsExt};
use std::sync::atomic::AtomicU64;
static NEXT: AtomicU64 = AtomicU64::new(0);
const SESSION: &str = "019c6e27-e55b-73d1-87d8-4e01f1f75043";

/// Injected monotonic clock. `pause()` advances the clock instead of blocking,
/// so expiry is driven by explicit test control rather than a wall-clock race.
struct TestClock {
    base: Instant,
    offset: AtomicU64,
}
impl TestClock {
    fn new() -> Self {
        Self {
            base: Instant::now(),
            offset: AtomicU64::new(0),
        }
    }
    fn advance(&self, duration: Duration) {
        self.offset
            .fetch_add(duration.as_millis().max(1) as u64, Ordering::Relaxed);
    }
}
impl Timing for TestClock {
    fn now(&self) -> Instant {
        self.base + Duration::from_millis(self.offset.load(Ordering::Relaxed))
    }
    fn pause(&self, duration: Duration) {
        self.advance(duration);
    }
}

/// Like TestClock, but on the FIRST lock-wait pause it signals the test and then
/// blocks until the test releases it. The clock does not advance while blocked,
/// so the only way the worker can leave the wait is the cancel the test sets —
/// the observed cause is causally determined, not a wall-clock guess.
struct SignalingClock {
    inner: TestClock,
    first: Mutex<Option<std::sync::mpsc::Sender<()>>>,
    release: Mutex<std::sync::mpsc::Receiver<()>>,
}
impl SignalingClock {
    fn new(paused: std::sync::mpsc::Sender<()>, release: std::sync::mpsc::Receiver<()>) -> Self {
        Self {
            inner: TestClock::new(),
            first: Mutex::new(Some(paused)),
            release: Mutex::new(release),
        }
    }
}
impl Timing for SignalingClock {
    fn now(&self) -> Instant {
        self.inner.now()
    }
    fn pause(&self, duration: Duration) {
        if let Some(paused) = self.first.lock().unwrap().take() {
            let _ = paused.send(());
            let gate = self.release.lock().unwrap();
            let _ = gate.recv_timeout(COLLECT);
        }
        self.inner.pause(duration);
    }
}
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
fn rejected_ack_and_counter_do_not_prove_a_bodyless_or_private_capture() {
    let f = Fixture::new();
    let mut receiver = f.receiver();
    let cache = receiver.cache();
    let mut bodyless: serde_json::Value = serde_json::from_slice(&f.payload("empty")).unwrap();
    bodyless["tool_input"] = serde_json::json!({});
    assert_eq!(
        send(
            &f.root,
            &serde_json::to_vec(&bodyless).unwrap(),
            Instant::now() + COLLECT
        ),
        Err(Error::Rejected)
    );
    let doc = f.doc("empty");
    assert!(cache
        .lock()
        .unwrap()
        .bind_selected(&f.path, &doc, &doc["message"]["content"][0], Instant::now())
        .is_none());
    // Invalid input did not insert a tombstone: a later valid capture of this call succeeds.
    send(&f.root, &f.payload("empty"), Instant::now() + COLLECT).unwrap();
    let reference = cache
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
            &reference,
            Instant::now()
        )
        .is_some());

    let mut private: serde_json::Value = serde_json::from_slice(&f.payload("private")).unwrap();
    private["tool_input"]["isSecret"] = true.into();
    assert_eq!(
        send(
            &f.root,
            &serde_json::to_vec(&private).unwrap(),
            Instant::now() + COLLECT
        ),
        Err(Error::Rejected)
    );
    // The same wire rejection also covers privacy revocation; it is not a successful ACK.
    assert_eq!(
        send(&f.root, &f.payload("private"), Instant::now() + COLLECT),
        Err(Error::Rejected)
    );
    let private_doc = f.doc("private");
    assert!(cache
        .lock()
        .unwrap()
        .bind_selected(
            &f.path,
            &private_doc,
            &private_doc["message"]["content"][0],
            Instant::now()
        )
        .is_none());
    let deadline = Instant::now() + Duration::from_secs(1);
    while receiver.status().unwrap().rejected < 3 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    let status = receiver.status().unwrap();
    assert_eq!((status.received, status.rejected), (1, 3));
    receiver.stop().unwrap();
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
    receiver.stop().unwrap();
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
    receiver.stop().unwrap();
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
    receiver.stop().unwrap();
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
        half_close_malformed(&client);
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
    half_close_malformed(&client);
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
    receiver.stop().unwrap();
}
// An invalid header can be rejected before the client sends EOF. macOS may
// report ENOTCONN for this half-close; receipt and cache assertions still apply.
fn half_close_malformed(client: &UnixStream) {
    if let Err(error) = client.shutdown(std::net::Shutdown::Write) {
        assert_eq!(error.kind(), std::io::ErrorKind::NotConnected);
    }
}
#[test]
fn oversized_header_rejected_before_half_close_cannot_bind() {
    let f = Fixture::new();
    let mut receiver = f.receiver();
    let mut client = connect(&f.root.join("receiver.sock"), Instant::now() + COLLECT).unwrap();
    client
        .write_all(&(MAX_FRAME as u32 + 1).to_be_bytes())
        .unwrap();
    let mut ack = [0];
    assert!(read_exact(&mut client, &mut ack, Instant::now() + COLLECT).is_err());
    half_close_malformed(&client);
    let doc = f.doc("p");
    assert!(receiver
        .cache()
        .lock()
        .unwrap()
        .bind_selected(&f.path, &doc, &doc["message"]["content"][0], Instant::now())
        .is_none());
    receiver.stop().unwrap();
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
    receiver.stop().unwrap();
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
    receiver.stop().unwrap();
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
    receiver.stop().unwrap();
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
    receiver.stop().unwrap();
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

#[test]
fn complete_hook_waits_for_initial_transcript_creation_before_acknowledging() {
    let f = Fixture::new();
    std::fs::remove_file(&f.path).unwrap();
    let mut receiver = f.receiver();
    let mut stream = connect(&f.root.join("receiver.sock"), Instant::now() + COLLECT).unwrap();
    let payload = f.payload("first-write");
    write_all(
        &mut stream,
        &(payload.len() as u32).to_be_bytes(),
        Instant::now() + COLLECT,
    )
    .unwrap();
    write_all(&mut stream, &payload, Instant::now() + COLLECT).unwrap();
    stream.shutdown(std::net::Shutdown::Write).unwrap();
    // The official client may create this transcript after it starts PreToolUse.
    // An immediate negative ACK loses a valid first event. No file exists yet.
    let mut poll = libc::pollfd {
        fd: stream.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    assert_eq!(
        unsafe { libc::poll(&mut poll, 1, 50) },
        0,
        "ACK preceded source creation"
    );
    std::fs::write(&f.path, "").unwrap();
    let mut ack = [255];
    read_exact(&mut stream, &mut ack, Instant::now() + COLLECT).unwrap();
    assert_eq!(ack, [0]);
    let doc = f.doc("first-write");
    assert!(receiver
        .cache()
        .lock()
        .unwrap()
        .bind_selected(&f.path, &doc, &doc["message"]["content"][0], Instant::now())
        .is_some());
    receiver.stop().unwrap();
}

#[test]
fn missing_transcript_times_out_without_poisoning_the_next_valid_call() {
    let f = Fixture::new();
    std::fs::remove_file(&f.path).unwrap();
    let mut receiver = f.receiver();
    let started = Instant::now();
    assert_eq!(
        send(&f.root, &f.payload("late"), started + COLLECT),
        Err(Error::Rejected)
    );
    assert!(started.elapsed() < EXCHANGE + Duration::from_millis(200));
    let doc = f.doc("late");
    assert!(receiver
        .cache()
        .lock()
        .unwrap()
        .bind_selected(&f.path, &doc, &doc["message"]["content"][0], Instant::now())
        .is_none());
    std::fs::write(&f.path, "").unwrap();
    send(&f.root, &f.payload("late"), Instant::now() + COLLECT).unwrap();
    assert!(receiver
        .cache()
        .lock()
        .unwrap()
        .bind_selected(&f.path, &doc, &doc["message"]["content"][0], Instant::now())
        .is_some());
    receiver.stop().unwrap();
}

// The previous readiness clock-race test is replaced by TestClock-driven causal
// cases below (MARKER_READINESS).

/// Shared setup: claim an endpoint, enable a cache, and run one exchange on a
/// worker thread with an injected clock. Returns the client stream plus the
/// server join handle so a test can assert the exact internal failure reason.
fn contended_exchange(
    f: &Fixture,
    guard: Option<LifecycleGuard>,
) -> (
    Arc<Mutex<Store>>,
    UnixStream,
    std::thread::JoinHandle<Result<(), ReceiveFailure>>,
) {
    let (endpoint, listener) = Endpoint::claim(&f.root).unwrap();
    listener.set_nonblocking(false).unwrap();
    let cache = Arc::new(Mutex::new(Store::default()));
    cache
        .lock()
        .unwrap()
        .enable(vec![f.source.clone()])
        .unwrap();
    let worker_cache = cache.clone();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        receive_with_timing(
            &mut stream,
            &worker_cache,
            Instant::now() + EXCHANGE,
            &AtomicBool::new(false),
            &endpoint,
            guard.as_ref(),
            &TestClock::new(),
        )
    });
    let mut stream = connect(&f.root.join("receiver.sock"), Instant::now() + COLLECT).unwrap();
    let payload = f.payload("contended");
    write_all(
        &mut stream,
        &(payload.len() as u32).to_be_bytes(),
        Instant::now() + COLLECT,
    )
    .unwrap();
    write_all(&mut stream, &payload, Instant::now() + COLLECT).unwrap();
    stream.shutdown(std::net::Shutdown::Write).unwrap();
    (cache, stream, server)
}

#[test]
fn readiness_retry_observes_revocation_without_holding_the_cache_lock() {
    let f = Fixture::new();
    std::fs::remove_file(&f.path).unwrap();
    let (checked_tx, checked_rx) = std::sync::mpsc::channel();
    let calls = AtomicU64::new(0);
    let guard: LifecycleGuard = Arc::new(move || {
        let call = calls.fetch_add(1, Ordering::Relaxed);
        let _ = checked_tx.send(call);
        call == 0
    });
    let (cache, mut stream, server) = contended_exchange(&f, Some(guard));
    // Callback #1 is the pre-ingest check; #2 is the retry after SourceMissing
    // released the cache lock. Exact cause, not a wall-clock race.
    assert_eq!(checked_rx.recv_timeout(COLLECT).unwrap(), 0);
    assert_eq!(checked_rx.recv_timeout(COLLECT).unwrap(), 1);
    assert!(
        cache.try_lock().is_ok(),
        "readiness retry held the cache lock while the callback ran"
    );
    assert!(!f.path.exists());
    // A closed connection also polls readable (EOF), so `poll` cannot tell "ACK
    // arrived" from "the server gave up and closed". Read the actual byte instead:
    // the contract is that no ACK is written before the source exists.
    let mut ack = [255u8];
    let read = read_exact(&mut stream, &mut ack, Instant::now() + COLLECT);
    assert!(
        read.is_err() || ack[0] == 255,
        "ACK preceded source creation: read={read:?} byte={}",
        ack[0]
    );
    assert_eq!(server.join().unwrap(), Err(ReceiveFailure::Revoked));
    let doc = f.doc("contended");
    assert!(cache
        .lock()
        .unwrap()
        .bind_selected(&f.path, &doc, &doc["message"]["content"][0], Instant::now())
        .is_none());
}

#[test]
fn slow_guard_past_the_absolute_deadline_never_commits_a_body() {
    let f = Fixture::new();
    let (endpoint, listener) = Endpoint::claim(&f.root).unwrap();
    listener.set_nonblocking(false).unwrap();
    let cache = Arc::new(Mutex::new(Store::default()));
    cache
        .lock()
        .unwrap()
        .enable(vec![f.source.clone()])
        .unwrap();
    // The callback is the slow step: it first returns true (pre-ingest), then
    // advances the logical clock far past the exchange while ingest is in flight.
    // Advancing by EXCHANGE+1s keeps the outcome independent of sub-millisecond
    // skew between the clock base and the worker's Instant::now().
    let clock = Arc::new(TestClock::new());
    let calls = AtomicU64::new(0);
    let slow = clock.clone();
    let guard: LifecycleGuard = Arc::new(move || {
        if calls.fetch_add(1, Ordering::Relaxed) > 0 {
            slow.advance(EXCHANGE + Duration::from_secs(1));
        }
        true
    });
    let worker_clock = clock.clone();
    let worker_cache = cache.clone();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        receive_with_timing(
            &mut stream,
            &worker_cache,
            Instant::now() + EXCHANGE,
            &AtomicBool::new(false),
            &endpoint,
            Some(&guard),
            &worker_clock,
        )
    });
    let mut stream = connect(&f.root.join("receiver.sock"), Instant::now() + COLLECT).unwrap();
    let payload = f.payload("slow");
    write_all(
        &mut stream,
        &(payload.len() as u32).to_be_bytes(),
        Instant::now() + COLLECT,
    )
    .unwrap();
    write_all(&mut stream, &payload, Instant::now() + COLLECT).unwrap();
    stream.shutdown(std::net::Shutdown::Write).unwrap();
    // Exact reason: the exchange expired (Deadline), not a generic rejection.
    assert_eq!(server.join().unwrap(), Err(ReceiveFailure::Deadline));
    let mut ack = [255];
    assert!(
        read_exact(&mut stream, &mut ack, Instant::now() + COLLECT).is_err() || ack[0] != 0,
        "an expired exchange acknowledged success"
    );
    let doc = f.doc("slow");
    assert!(
        cache
            .lock()
            .unwrap()
            .bind_selected(&f.path, &doc, &doc["message"]["content"][0], Instant::now())
            .is_none(),
        "an expired exchange left a body in the cache"
    );
}

#[test]
fn cache_lock_contention_reaches_the_absolute_deadline_without_deadlock() {
    let f = Fixture::new();
    let (endpoint, listener) = Endpoint::claim(&f.root).unwrap();
    listener.set_nonblocking(false).unwrap();
    let cache = Arc::new(Mutex::new(Store::default()));
    cache
        .lock()
        .unwrap()
        .enable(vec![f.source.clone()])
        .unwrap();
    let held = cache.lock().unwrap();
    let worker_cache = cache.clone();
    let clock_for_test = Arc::new(TestClock::new());
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        receive_with_timing(
            &mut stream,
            &worker_cache,
            Instant::now() + EXCHANGE,
            &AtomicBool::new(false),
            &endpoint,
            None,
            &clock_for_test,
        )
    });
    let mut stream = connect(&f.root.join("receiver.sock"), Instant::now() + COLLECT).unwrap();
    let payload = f.payload("contended");
    write_all(
        &mut stream,
        &(payload.len() as u32).to_be_bytes(),
        Instant::now() + COLLECT,
    )
    .unwrap();
    write_all(&mut stream, &payload, Instant::now() + COLLECT).unwrap();
    stream.shutdown(std::net::Shutdown::Write).unwrap();
    // Bounded try_lock + injected clock: reaching the deadline is deterministic,
    // and the result is Deadline rather than Unavailable or Rejected.
    assert_eq!(server.join().unwrap(), Err(ReceiveFailure::Deadline));
    drop(held);
    assert!(cache.try_lock().is_ok());
    let doc = f.doc("contended");
    assert!(cache
        .lock()
        .unwrap()
        .bind_selected(&f.path, &doc, &doc["message"]["content"][0], Instant::now())
        .is_none());
}

#[test]
fn cancel_during_lock_contention_reports_exactly_cancelled() {
    let f = Fixture::new();
    let (endpoint, listener) = Endpoint::claim(&f.root).unwrap();
    listener.set_nonblocking(false).unwrap();
    let cache = Arc::new(Mutex::new(Store::default()));
    cache
        .lock()
        .unwrap()
        .enable(vec![f.source.clone()])
        .unwrap();
    // Hold the lock so the worker can only wait, and learn the moment it starts
    // waiting from the injected clock instead of guessing with a wall-clock sleep.
    let held = cache.lock().unwrap();
    let (paused_tx, paused_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let clock = Arc::new(SignalingClock::new(paused_tx, release_rx));
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cache = cache.clone();
    let worker_cancel = cancel.clone();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        receive_with_timing(
            &mut stream,
            &worker_cache,
            Instant::now() + EXCHANGE,
            &worker_cancel,
            &endpoint,
            None,
            &clock,
        )
    });
    let mut stream = connect(&f.root.join("receiver.sock"), Instant::now() + COLLECT).unwrap();
    let payload = f.payload("cancel");
    write_all(
        &mut stream,
        &(payload.len() as u32).to_be_bytes(),
        Instant::now() + COLLECT,
    )
    .unwrap();
    write_all(&mut stream, &payload, Instant::now() + COLLECT).unwrap();
    stream.shutdown(std::net::Shutdown::Write).unwrap();
    paused_rx
        .recv_timeout(COLLECT)
        .expect("worker never waited on the contended cache");
    cancel.store(true, Ordering::Release);
    // Release only after cancel is visible; the clock is frozen while blocked, so
    // the exact observed cause is Cancelled and nothing else.
    release_tx.send(()).unwrap();
    assert_eq!(server.join().unwrap(), Err(ReceiveFailure::Cancelled));
    drop(held);
    let doc = f.doc("cancel");
    assert!(cache
        .lock()
        .unwrap()
        .bind_selected(&f.path, &doc, &doc["message"]["content"][0], Instant::now())
        .is_none());
}

#[test]
fn callback_panic_under_the_lock_returns_its_own_reason_without_a_body() {
    let f = Fixture::new();
    let (endpoint, listener) = Endpoint::claim(&f.root).unwrap();
    listener.set_nonblocking(false).unwrap();
    let cache = Arc::new(Mutex::new(Store::default()));
    cache
        .lock()
        .unwrap()
        .enable(vec![f.source.clone()])
        .unwrap();
    // First callback (pre-ingest) succeeds; the SECOND one runs inside the
    // transaction, after ingest, and panics. The body must be rolled back.
    let calls = AtomicU64::new(0);
    let guard: LifecycleGuard = Arc::new(move || {
        if calls.fetch_add(1, Ordering::Relaxed) > 0 {
            panic!("lifecycle callback must never unwind the worker");
        }
        true
    });
    let worker_cache = cache.clone();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        receive_with_timing(
            &mut stream,
            &worker_cache,
            Instant::now() + EXCHANGE,
            &AtomicBool::new(false),
            &endpoint,
            Some(&guard),
            &TestClock::new(),
        )
    });
    let mut stream = connect(&f.root.join("receiver.sock"), Instant::now() + COLLECT).unwrap();
    let payload = f.payload("panic");
    write_all(
        &mut stream,
        &(payload.len() as u32).to_be_bytes(),
        Instant::now() + COLLECT,
    )
    .unwrap();
    write_all(&mut stream, &payload, Instant::now() + COLLECT).unwrap();
    stream.shutdown(std::net::Shutdown::Write).unwrap();
    // Exact cause travels as GuardPanicked so the worker can hard-stop and clean up.
    assert_eq!(server.join().unwrap(), Err(ReceiveFailure::GuardPanicked));
    let mut ack = [255];
    assert!(read_exact(&mut stream, &mut ack, Instant::now() + COLLECT).is_err() || ack[0] != 0);
    let doc = f.doc("panic");
    assert!(cache
        .lock()
        .unwrap()
        .bind_selected(&f.path, &doc, &doc["message"]["content"][0], Instant::now())
        .is_none());
    assert!(cache.try_lock().is_ok(), "panic left the cache lock held");
}

#[test]
fn callback_panic_runs_the_worker_exit_cleanup() {
    let f = Fixture::new();
    let panic_now = Arc::new(AtomicBool::new(false));
    let trigger = panic_now.clone();
    let receiver = Receiver::start_inner_guarded(
        &f.root,
        vec![f.source.clone()],
        Duration::from_millis(20),
        Some(Arc::new(move || {
            assert!(
                !trigger.load(Ordering::Acquire),
                "lifecycle callback panicked"
            );
            true
        })),
    )
    .unwrap();
    let cache = receiver.cache();
    let doc = f.doc("panic");
    let r = {
        let mut store = cache.lock().unwrap();
        store.ingest(&f.payload("panic"), Instant::now()).unwrap();
        store
            .bind_selected(&f.path, &doc, &doc["message"]["content"][0], Instant::now())
            .unwrap()
    };
    panic_now.store(true, Ordering::Release);
    // Drive one exchange so the panic lands inside the worker's own receive path
    // (idle probes only run once per second): deterministic and immediate.
    assert!(send(&f.root, &f.payload("panic"), Instant::now() + COLLECT).is_err());
    let limit = Instant::now() + COLLECT;
    while receiver.status().unwrap().running && Instant::now() < limit {
        std::thread::sleep(Duration::from_millis(5));
    }
    // The panic path must run the same exit scope as a normal break: running=false,
    // body cleared, owned endpoint removed, and stop() must not deadlock.
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
    let mut receiver = receiver;
    receiver.stop().unwrap();
}

#[test]
fn stop_is_bounded_and_shuts_the_live_exchange() {
    let f = Fixture::new();
    let mut receiver = f.receiver();
    // A client that never completes its frame would otherwise hold the worker for
    // the full exchange budget; stop must cancel and shut the socket first.
    let held = connect(&f.root.join("receiver.sock"), Instant::now() + COLLECT).unwrap();
    let start = Instant::now();
    receiver.stop().unwrap();
    assert!(
        start.elapsed() < STOP_BUDGET,
        "stop waited for the exchange deadline instead of cancelling it"
    );
    assert!(!receiver.status().unwrap().running);
    assert!(!f.root.exists());
    drop(held);
}

#[test]
fn stop_reports_a_cache_consumer_that_exceeds_the_cleanup_budget() {
    let f = Fixture::new();
    let mut receiver = f.receiver();
    let cache = receiver.cache();
    let held = cache.lock().unwrap();
    // Must not hang, must not claim syntax-level success.
    assert!(matches!(receiver.stop(), Err(Error::Busy)));
    drop(held);
    receiver.stop().unwrap();
    assert!(!receiver.status().unwrap().running);
    assert!(!f.root.exists());
}
