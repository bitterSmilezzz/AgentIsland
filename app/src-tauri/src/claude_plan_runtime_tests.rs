use super::*;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
struct Fixture {
    _sandbox: crate::testutil::Sandbox,
    runtime: Runtime,
    config: PathBuf,
    parent: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let sandbox = crate::testutil::Sandbox::new("plan-runtime");
        let root = sandbox.path().canonicalize().unwrap();
        let config = root.join("claude");
        let parent = root.join("app");
        std::fs::create_dir(&config).unwrap();
        std::fs::create_dir(&parent).unwrap();
        let binary = root.join("agentisland");
        std::fs::write(&binary, b"fixture-only").unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
        let endpoint = PathBuf::from(format!(
            "/private/tmp/ai-plan-runtime-{}-{}",
            std::process::id(),
            &uuid::Uuid::new_v4().simple().to_string()[..8]
        ));
        let store =
            config::native::Store::new(config.clone(), parent.clone(), binary, root.join(".Trash"));
        Self {
            _sandbox: sandbox,
            runtime: Runtime::new(store, endpoint, vec![root]),
            config,
            parent,
        }
    }
    fn install(&mut self) {
        let revision = self.runtime.config.inspect().unwrap().revision;
        let p = self
            .runtime
            .preview(config::Action::Enable, &revision, None)
            .unwrap();
        self.runtime.apply(&p.plan_id).unwrap();
    }
    fn resume(&mut self) {
        let revision = self.runtime.config.observe().unwrap().revision;
        self.runtime.resume(&revision).unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.runtime.stop("paused", "fixture");
        let _ = std::fs::remove_dir(&self.runtime.endpoint);
    }
}
#[test]
fn default_install_cancel_and_explicit_resume_pause_are_distinct() {
    let mut f = Fixture::new();
    assert!(!f.runtime.endpoint.exists());
    f.runtime.resume_saved().unwrap();
    assert!(!f.runtime.endpoint.exists());
    let revision = f.runtime.config.observe().unwrap().revision;
    let p = f
        .runtime
        .preview(config::Action::Enable, &revision, None)
        .unwrap();
    f.runtime.status();
    f.runtime.cancel();
    assert!(f.runtime.apply(&p.plan_id).is_err());
    assert!(!f.config.join("settings.json").exists());
    f.install();
    assert!(!f.runtime.endpoint.exists());
    assert_eq!(f.runtime.status().state, "paused");
    f.resume();
    assert_eq!(f.runtime.status().state, "running");
    assert!(f.runtime.endpoint.exists());
    let cache = f.runtime.receiver.as_ref().unwrap().cache();
    f.runtime.pause().unwrap();
    assert!(!f.runtime.endpoint.exists());
    assert_eq!(f.runtime.status().state, "paused");
    assert!(
        !f.runtime
            .config
            .runtime_preference()
            .unwrap()
            .unwrap()
            .enabled
    );
    // A previously held cache handle cannot remain enabled after stopping the controller.
    assert!(matches!(cache.lock().unwrap().ingest(b"{}", std::time::Instant::now()), Err(capture::Error::Disabled)));
}
#[test]
fn configuration_drift_clears_receiver_and_invalidates_saved_restart_binding() {
    let mut f = Fixture::new();
    f.install();
    f.resume();
    let preference = f.runtime.config.runtime_preference().unwrap().unwrap();
    let mut value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(f.config.join("settings.json")).unwrap()).unwrap();
    value["theme"] = "external".into();
    std::fs::write(f.config.join("settings.json"), value.to_string()).unwrap();
    assert!(f.runtime.with_cache(|cache| cache.is_none()));
    assert_eq!(f.runtime.status().state, "changed");
    assert!(!f.runtime.endpoint.exists());
    assert!(preference.enabled);
    f.runtime.resume_saved().unwrap();
    assert!(!f.runtime.endpoint.exists());
    assert_eq!(f.runtime.status().state, "changed");
}
#[test]
fn saved_opt_in_resumes_only_exact_owned_current_config_and_pause_survives_restart() {
    let mut f = Fixture::new();
    f.install();
    f.resume();
    f.runtime.stop("paused", "fixture restart");
    f.runtime.resume_saved().unwrap();
    assert_eq!(f.runtime.status().state, "running");
    f.runtime.pause().unwrap();
    f.runtime.resume_saved().unwrap();
    assert_eq!(f.runtime.status().state, "paused");
    assert!(!f.runtime.endpoint.exists());
    std::fs::write(
        f.parent.join("claude-plan-runtime.json"),
        br#"{"schema_version":1,"enabled":true,"binding":"unknown"}"#,
    )
    .unwrap();
    assert!(f.runtime.resume_saved().is_err());
    assert!(!f.runtime.endpoint.exists());
}
#[test]
fn foreign_control_file_is_not_overwritten_and_service_does_not_start() {
    let mut f = Fixture::new();
    f.install();
    let path = f.parent.join("claude-plan-runtime.json");
    std::fs::write(&path, b"unverified-fixture").unwrap();
    let revision = f.runtime.config.observe().unwrap().revision;
    assert!(f.runtime.resume(&revision).is_err());
    assert_eq!(std::fs::read(path).unwrap(), b"unverified-fixture");
    assert!(!f.runtime.endpoint.exists());
}
#[test]
fn invalid_confirmation_does_not_interrupt_active_receiver_and_cleanup_preserves_it() {
    let mut f = Fixture::new();
    f.install();
    f.resume();
    assert!(f.runtime.apply("expired-confirmation").is_err());
    assert_eq!(f.runtime.status().state, "running");
    let state = f.runtime.status();
    let cfg = state.configuration.unwrap();
    let p = f
        .runtime
        .preview(
            config::Action::TrashBackup,
            &cfg.revision,
            Some(&cfg.backups[0]),
        )
        .unwrap();
    f.runtime.apply(&p.plan_id).unwrap();
    assert_eq!(f.runtime.status().state, "running");
    assert!(f.runtime.endpoint.exists());
}
#[test]
fn poisoned_cache_cannot_deadlock_stop_or_retain_enabled_collection() {
    let mut f = Fixture::new();
    f.install();
    f.resume();
    let cache = f.runtime.receiver.as_ref().unwrap().cache();
    let held = cache.clone();
    let _ = std::thread::spawn(move || {
        let _guard = held.lock().unwrap();
        panic!("fixture cache poison");
    })
    .join();
    assert!(f.runtime.with_cache(|cache| cache.is_none()));
    assert!(!f.runtime.endpoint.exists());
    let mut disabled = cache.lock().err().unwrap().into_inner();
    assert!(matches!(disabled.ingest(b"{}", std::time::Instant::now()), Err(capture::Error::Disabled)));
}
#[test]
fn idle_receiver_stops_on_configuration_drift_without_consumer_polling() {
    let mut f = Fixture::new();
    f.install();
    f.resume();
    let receiver = f.runtime.receiver.as_ref().unwrap();
    let cache = receiver.cache();
    let mut value: serde_json::Value = serde_json::from_slice(
        &std::fs::read(f.config.join("settings.json")).unwrap(),
    ).unwrap();
    value["theme"] = serde_json::json!("fixture-changed");
    std::fs::write(f.config.join("settings.json"), serde_json::to_vec(&value).unwrap()).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(4);
    while receiver.status().unwrap().running && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    assert!(!receiver.status().unwrap().running);
    // The worker itself clears cache and releases its socket, without Runtime::status/with_cache.
    assert!(matches!(cache.lock().unwrap().ingest(b"{}", std::time::Instant::now()), Err(capture::Error::Disabled)));
    while f.runtime.endpoint.exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(!f.runtime.endpoint.exists());
    assert_eq!(f.runtime.status().state, "changed");
}
