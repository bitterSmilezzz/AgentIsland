use super::*;
use crate::claude_plan_capture::Store;
use serde::Serialize;
use std::{
    fs::{DirBuilder, File},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{
            ffi::OsStrExt,
            fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
            net::{UnixListener, UnixStream},
        },
    },
    path::{Component, Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread::JoinHandle,
};
const SOCKET: &[u8] = b"receiver.sock\0";
const EXCHANGE: Duration = Duration::from_millis(750);
const COLLECT: Duration = Duration::from_secs(2);
const PURGE: Duration = Duration::from_secs(30);
const SOURCE_READY: Duration = Duration::from_millis(200);
#[derive(Clone, PartialEq, Eq)]
struct Identity {
    device: u64,
    inode: u64,
    birth: std::time::SystemTime,
}
fn identity(meta: &std::fs::Metadata) -> Result<Identity, Error> {
    Ok(Identity {
        device: meta.dev(),
        inode: meta.ino(),
        birth: meta.created().map_err(|_| Error::Namespace)?,
    })
}
fn allowed(meta: &std::fs::Metadata, mode: u32) -> bool {
    meta.uid() == unsafe { libc::geteuid() } && meta.mode() & 0o7777 == mode
}
fn plain(path: &Path) -> bool {
    path.is_absolute()
        && path
            .components()
            .all(|c| matches!(c, Component::RootDir | Component::Normal(_)))
}
fn peer(stream: &UnixStream) -> Result<(), Error> {
    let mut uid = 0;
    let mut gid = 0;
    if unsafe { libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) } != 0
        || uid != unsafe { libc::geteuid() }
    {
        return Err(Error::Unavailable);
    }
    Ok(())
}
fn wait(fd: i32, event: i16, deadline: Instant) -> Result<(), Error> {
    loop {
        let ms = remaining(deadline)?
            .as_millis()
            .min(i32::MAX as u128)
            .max(1) as i32;
        let mut poll = libc::pollfd {
            fd,
            events: event,
            revents: 0,
        };
        let result = unsafe { libc::poll(&mut poll, 1, ms) };
        if result > 0 {
            if poll.revents & (event | libc::POLLHUP) != 0 {
                return Ok(());
            }
            return Err(Error::Unavailable);
        }
        if result < 0 && std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
            continue;
        }
        return Err(Error::Unavailable);
    }
}
fn listener_ready(listener: &UnixListener) -> Result<bool, Error> {
    let mut poll = libc::pollfd {
        fd: listener.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    let result = unsafe { libc::poll(&mut poll, 1, 100) };
    if result == 0 {
        return Ok(false);
    }
    if result < 0 {
        return if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
            Ok(false)
        } else {
            Err(Error::Unavailable)
        };
    }
    if poll.revents & libc::POLLIN != 0 {
        Ok(true)
    } else {
        Err(Error::Unavailable)
    }
}
fn connect(path: &Path, deadline: Instant) -> Result<UnixStream, Error> {
    let bytes = path.as_os_str().as_bytes();
    let mut addr: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    if bytes.len() >= addr.sun_path.len() || bytes.contains(&0) {
        return Err(Error::Namespace);
    }
    addr.sun_family = libc::AF_UNIX as _;
    addr.sun_len = std::mem::size_of::<libc::sockaddr_un>() as _;
    for (slot, b) in addr.sun_path.iter_mut().zip(bytes) {
        *slot = *b as _;
    }
    let fd = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0) };
    if fd < 0 {
        return Err(Error::Unavailable);
    }
    let stream = unsafe { UnixStream::from_raw_fd(fd) };
    if unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
        return Err(Error::Unavailable);
    }
    stream
        .set_nonblocking(true)
        .map_err(|_| Error::Unavailable)?;
    let result = unsafe {
        libc::connect(
            fd,
            &addr as *const _ as *const libc::sockaddr,
            std::mem::size_of_val(&addr) as _,
        )
    };
    if result < 0 {
        if std::io::Error::last_os_error().raw_os_error() != Some(libc::EINPROGRESS) {
            return Err(Error::Unavailable);
        }
        wait(fd, libc::POLLOUT, deadline)?;
        let mut errno: i32 = 0;
        let mut len = std::mem::size_of_val(&errno) as libc::socklen_t;
        if unsafe {
            libc::getsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_ERROR,
                &mut errno as *mut _ as *mut _,
                &mut len,
            )
        } < 0
            || errno != 0
        {
            return Err(Error::Unavailable);
        }
    }
    peer(&stream)?;
    stream
        .set_nonblocking(false)
        .map_err(|_| Error::Unavailable)?;
    Ok(stream)
}
fn read_some(stream: &UnixStream, bytes: &mut [u8], deadline: Instant) -> Result<usize, Error> {
    loop {
        wait(stream.as_raw_fd(), libc::POLLIN, deadline)?;
        let size = unsafe {
            libc::recv(
                stream.as_raw_fd(),
                bytes.as_mut_ptr() as *mut _,
                bytes.len(),
                libc::MSG_DONTWAIT,
            )
        };
        if size >= 0 {
            return Ok(size as usize);
        }
        let error = std::io::Error::last_os_error();
        if matches!(
            error.kind(),
            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
        ) {
            continue;
        }
        return Err(Error::Unavailable);
    }
}
fn read_exact(
    stream: &mut UnixStream,
    mut bytes: &mut [u8],
    deadline: Instant,
) -> Result<(), Error> {
    while !bytes.is_empty() {
        let n = read_some(stream, bytes, deadline)?;
        if n == 0 {
            return Err(Error::Input);
        }
        bytes = &mut bytes[n..];
    }
    Ok(())
}
fn write_all(stream: &mut UnixStream, mut bytes: &[u8], deadline: Instant) -> Result<(), Error> {
    while !bytes.is_empty() {
        wait(stream.as_raw_fd(), libc::POLLOUT, deadline)?;
        let size = unsafe {
            libc::send(
                stream.as_raw_fd(),
                bytes.as_ptr() as *const _,
                bytes.len(),
                libc::MSG_DONTWAIT | libc::MSG_NOSIGNAL,
            )
        };
        if size > 0 {
            bytes = &bytes[size as usize..];
            continue;
        }
        if size < 0
            && matches!(
                std::io::Error::last_os_error().kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
            )
        {
            continue;
        }
        return Err(Error::Unavailable);
    }
    Ok(())
}
fn stdin_bytes(fd: i32, deadline: Instant) -> Result<Vec<u8>, Error> {
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 8192];
    loop {
        wait(fd, libc::POLLIN, deadline)?;
        let n = unsafe { libc::read(fd, buffer.as_mut_ptr() as *mut _, buffer.len()) };
        if n == 0 {
            return if bytes.is_empty() {
                Err(Error::Input)
            } else {
                Ok(bytes)
            };
        }
        if n < 0 {
            if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(Error::Input);
        }
        if bytes.len() + n as usize > MAX_FRAME {
            return Err(Error::Input);
        }
        bytes.extend_from_slice(&buffer[..n as usize]);
    }
}
struct Endpoint {
    root: PathBuf,
    dir: File,
    id: Identity,
    socket_id: Option<Identity>,
    created: bool,
}
impl Endpoint {
    fn open(root: &Path, create: bool) -> Result<Self, Error> {
        if !plain(root) || root.join("receiver.sock").as_os_str().as_bytes().len() >= 104 {
            return Err(Error::Namespace);
        }
        let mut prefix = PathBuf::new();
        for part in root.parent().ok_or(Error::Namespace)?.components() {
            prefix.push(part.as_os_str());
            let meta = std::fs::symlink_metadata(&prefix).map_err(|_| Error::Namespace)?;
            if !meta.is_dir() || meta.file_type().is_symlink() {
                return Err(Error::Namespace);
            }
        }
        let created = if create {
            match DirBuilder::new().mode(0o700).create(root) {
                Ok(()) => true,
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => false,
                Err(_) => return Err(Error::Namespace),
            }
        } else {
            false
        };
        let dir = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(root)
            .map_err(|_| Error::Namespace)?;
        let meta = dir.metadata().map_err(|_| Error::Namespace)?;
        if !meta.is_dir() || !allowed(&meta, 0o700) {
            return Err(Error::Namespace);
        }
        let endpoint = Self {
            root: root.into(),
            dir,
            id: identity(&meta)?,
            socket_id: None,
            created,
        };
        endpoint.current()?;
        Ok(endpoint)
    }
    fn path(&self) -> PathBuf {
        self.root.join("receiver.sock")
    }
    fn current(&self) -> Result<(), Error> {
        let meta = std::fs::symlink_metadata(&self.root).map_err(|_| Error::Namespace)?;
        if !meta.is_dir()
            || meta.file_type().is_symlink()
            || !allowed(&meta, 0o700)
            || identity(&meta)? != self.id
        {
            return Err(Error::Namespace);
        }
        Ok(())
    }
    fn stat_socket(&self) -> Result<libc::stat, Error> {
        let mut stat: libc::stat = unsafe { std::mem::zeroed() };
        if unsafe {
            libc::fstatat(
                self.dir.as_raw_fd(),
                SOCKET.as_ptr() as *const _,
                &mut stat,
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } != 0
        {
            return Err(Error::Namespace);
        }
        Ok(stat)
    }
    fn socket_identity(&self) -> Result<Identity, Error> {
        #[cfg(unix)]
        use std::os::unix::fs::FileTypeExt;
        self.current()?;
        let meta = std::fs::symlink_metadata(self.path()).map_err(|_| Error::Namespace)?;
        if !meta.file_type().is_socket() || !allowed(&meta, 0o600) {
            return Err(Error::Namespace);
        }
        let stat = self.stat_socket()?;
        if stat.st_dev as u64 != meta.dev() || stat.st_ino != meta.ino() {
            return Err(Error::Namespace);
        }
        identity(&meta)
    }
    fn unlink_owned(&self, id: &Identity) -> Result<(), Error> {
        let stat = self.stat_socket()?;
        let birth = std::time::SystemTime::UNIX_EPOCH
            + Duration::new(
                stat.st_birthtime.try_into().map_err(|_| Error::Namespace)?,
                stat.st_birthtime_nsec
                    .try_into()
                    .map_err(|_| Error::Namespace)?,
            );
        if stat.st_dev as u64 != id.device
            || stat.st_ino != id.inode
            || birth != id.birth
            || stat.st_mode & libc::S_IFMT != libc::S_IFSOCK
            || stat.st_uid != unsafe { libc::geteuid() }
            || stat.st_mode & 0o7777 != 0o600
        {
            return Err(Error::Namespace);
        }
        if unsafe { libc::unlinkat(self.dir.as_raw_fd(), SOCKET.as_ptr() as *const _, 0) } != 0 {
            return Err(Error::Namespace);
        }
        Ok(())
    }
    fn claim(root: &Path) -> Result<(Self, UnixListener), Error> {
        let mut endpoint = Self::open(root, true)?;
        if std::fs::symlink_metadata(endpoint.path()).is_ok() {
            let id = endpoint.socket_identity()?;
            // An existing listener of any kind is not ours to replace. Only a positively refused
            // connection permits removal, after rechecking the same owned filesystem object.
            let fd = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0) };
            if fd < 0 {
                return Err(Error::Unavailable);
            }
            let probe = unsafe { UnixStream::from_raw_fd(fd) };
            if unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
                return Err(Error::Unavailable);
            }
            probe
                .set_nonblocking(true)
                .map_err(|_| Error::Unavailable)?;
            let bytes = endpoint.path().as_os_str().as_bytes().to_vec();
            let mut addr: libc::sockaddr_un = unsafe { std::mem::zeroed() };
            addr.sun_family = libc::AF_UNIX as _;
            addr.sun_len = std::mem::size_of_val(&addr) as _;
            for (slot, b) in addr.sun_path.iter_mut().zip(bytes) {
                *slot = b as _;
            }
            let result = unsafe {
                libc::connect(
                    fd,
                    &addr as *const _ as *const libc::sockaddr,
                    std::mem::size_of_val(&addr) as _,
                )
            };
            if result == 0
                || std::io::Error::last_os_error().raw_os_error() != Some(libc::ECONNREFUSED)
            {
                return Err(Error::Busy);
            }
            endpoint.current()?;
            if endpoint.socket_identity()? != id {
                return Err(Error::Namespace);
            }
            endpoint.unlink_owned(&id)?;
        }
        endpoint.current()?;
        let listener = UnixListener::bind(endpoint.path()).map_err(|_| Error::Unavailable)?;
        // Parent is private before binding; no other UID can access the transient umask mode.
        endpoint.current()?;
        let stat = endpoint.stat_socket()?;
        if stat.st_mode & libc::S_IFMT != libc::S_IFSOCK
            || stat.st_uid != unsafe { libc::geteuid() }
        {
            return Err(Error::Namespace);
        }
        if unsafe {
            libc::fchmodat(
                endpoint.dir.as_raw_fd(),
                SOCKET.as_ptr() as *const _,
                0o600,
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } != 0
        {
            return Err(Error::Namespace);
        }
        endpoint.socket_id = Some(endpoint.socket_identity()?);
        listener
            .set_nonblocking(true)
            .map_err(|_| Error::Unavailable)?;
        Ok((endpoint, listener))
    }
}
impl Drop for Endpoint {
    fn drop(&mut self) {
        if let Some(id) = &self.socket_id {
            let _ = self.unlink_owned(id);
        }
        if self.created && self.current().is_ok() {
            let _ = std::fs::remove_dir(&self.root);
        }
    }
}
#[derive(Clone, Serialize)]
pub(crate) struct Status {
    pub running: bool,
    pub received: u64,
    pub rejected: u64,
    pub last_received_ms: Option<i64>,
}
// Synchronous callbacks MUST return in bounded time, must not acquire the cache,
// and must not dispatch synchronously to the UI. A Rust closure cannot be killed.
// The production configuration probe bounds file sizes/path depth, but filesystem
// latency is not a hard wall-clock bound. Never reuse a prior successful check.
type LifecycleGuard = Arc<dyn Fn() -> bool + Send + Sync>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReceiveFailure {
    Transport(Error),
    Deadline,
    Cancelled,
    Revoked,
    GuardPanicked,
}
impl From<Error> for ReceiveFailure {
    fn from(error: Error) -> Self {
        Self::Transport(error)
    }
}
impl ReceiveFailure {
    fn wire(self) -> Error {
        match self {
            Self::Transport(error) => error,
            Self::Deadline => Error::Rejected,
            Self::Cancelled | Self::Revoked | Self::GuardPanicked => Error::Unavailable,
        }
    }
}

trait Timing: Send + Sync {
    fn now(&self) -> Instant;
    fn pause(&self, duration: Duration);
}
// A shared clock is a clock: tests hold `Arc<TestClock>` so the worker and the test
// observe the same timeline, and forwarding here keeps call sites free of `&*`.
impl<T: Timing + ?Sized> Timing for std::sync::Arc<T> {
    fn now(&self) -> Instant {
        (**self).now()
    }
    fn pause(&self, duration: Duration) {
        (**self).pause(duration)
    }
}
struct Monotonic;
impl Timing for Monotonic {
    fn now(&self) -> Instant {
        Instant::now()
    }
    fn pause(&self, duration: Duration) {
        std::thread::sleep(duration);
    }
}
thread_local! {
    static PRIVATE_GUARD: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}
fn lifecycle(guard: Option<&LifecycleGuard>) -> Result<(), ReceiveFailure> {
    let Some(check) = guard else { return Ok(()) };
    // catch_unwind alone runs the ordinary hook first, potentially printing a
    // callback's panic payload. Suppress ONLY this thread's private callback;
    // preserve the pre-existing hook for all other panics/threads.
    static HOOK: std::sync::Once = std::sync::Once::new();
    HOOK.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if !PRIVATE_GUARD.with(|private| private.get()) {
                previous(info);
            }
        }));
    });
    struct Reset(bool);
    impl Drop for Reset {
        fn drop(&mut self) {
            PRIVATE_GUARD.with(|private| private.set(self.0));
        }
    }
    let _reset = Reset(PRIVATE_GUARD.with(|private| private.replace(true)));
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| check())) {
        Ok(true) => Ok(()),
        Ok(false) => Err(ReceiveFailure::Revoked),
        Err(_) => Err(ReceiveFailure::GuardPanicked),
    }
}
fn checkpoint(
    timing: &dyn Timing,
    deadline: Instant,
    cancel: &AtomicBool,
    endpoint: &Endpoint,
) -> Result<(), ReceiveFailure> {
    if cancel.load(Ordering::Acquire) {
        return Err(ReceiveFailure::Cancelled);
    }
    if timing.now() >= deadline {
        return Err(ReceiveFailure::Deadline);
    }
    endpoint.current()?;
    if endpoint
        .socket_id
        .as_ref()
        .is_none_or(|id| endpoint.socket_identity().as_ref() != Ok(id))
    {
        return Err(Error::Namespace.into());
    }
    // Filesystem checks themselves can consume the remaining budget.
    if cancel.load(Ordering::Acquire) {
        return Err(ReceiveFailure::Cancelled);
    }
    if timing.now() >= deadline {
        return Err(ReceiveFailure::Deadline);
    }
    Ok(())
}
fn live_checkpoint(
    timing: &dyn Timing,
    deadline: Instant,
    cancel: &AtomicBool,
    endpoint: &Endpoint,
    guard: Option<&LifecycleGuard>,
) -> Result<(), ReceiveFailure> {
    checkpoint(timing, deadline, cancel, endpoint)?;
    let live = lifecycle(guard);
    // A slow successful check is not a fresh authorization for an expired exchange.
    checkpoint(timing, deadline, cancel, endpoint)?;
    live
}
fn cache_until<'a>(
    cache: &'a Mutex<Store>,
    timing: &dyn Timing,
    deadline: Instant,
    cancel: &AtomicBool,
    endpoint: &Endpoint,
) -> Result<std::sync::MutexGuard<'a, Store>, ReceiveFailure> {
    loop {
        checkpoint(timing, deadline, cancel, endpoint)?;
        match cache.try_lock() {
            Ok(store) => {
                checkpoint(timing, deadline, cancel, endpoint)?;
                return Ok(store);
            }
            Err(std::sync::TryLockError::Poisoned(_)) => return Err(Error::Unavailable.into()),
            Err(std::sync::TryLockError::WouldBlock) => {
                timing.pause(
                    deadline
                        .saturating_duration_since(timing.now())
                        .min(Duration::from_millis(2)),
                );
            }
        }
    }
}

const STOP_BUDGET: Duration = Duration::from_secs(1);
fn clear_until(cache: &Mutex<Store>, deadline: Instant) -> Result<(), Error> {
    loop {
        match cache.try_lock() {
            Ok(mut cache) => {
                cache.disable();
                return Ok(());
            }
            Err(std::sync::TryLockError::Poisoned(error)) => {
                error.into_inner().disable();
                return Ok(());
            }
            Err(std::sync::TryLockError::WouldBlock) => {
                if Instant::now() >= deadline {
                    return Err(Error::Busy);
                }
                std::thread::sleep(Duration::from_millis(2));
            }
        }
    }
}
fn retire(endpoint: &Endpoint) {
    if let Some(id) = &endpoint.socket_id {
        let _ = endpoint.unlink_owned(id);
    }
    if endpoint.created && endpoint.current().is_ok() {
        let _ = std::fs::remove_dir(&endpoint.root);
    }
}
struct WorkerExit {
    cache: Arc<Mutex<Store>>,
    cancel: Arc<AtomicBool>,
    status: Arc<Mutex<Status>>,
    endpoint: Arc<Endpoint>,
}
impl Drop for WorkerExit {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
        // Revoke the address before waiting for a cache consumer.
        retire(&self.endpoint);
        let _ = clear_until(&self.cache, Instant::now() + STOP_BUDGET);
        if let Ok(mut status) = self.status.lock() {
            status.running = false;
        }
    }
}
pub(crate) struct Receiver {
    cache: Arc<Mutex<Store>>,
    cancel: Arc<AtomicBool>,
    status: Arc<Mutex<Status>>,
    endpoint: Arc<Endpoint>,
    active: Arc<Mutex<Option<UnixStream>>>,
    thread: Option<JoinHandle<()>>,
}
impl Receiver {
    pub(crate) fn start(root: &Path, roots: Vec<PathBuf>) -> Result<Self, Error> {
        Self::start_inner(root, roots, PURGE)
    }
    fn start_inner(
        root: &Path,
        roots: Vec<PathBuf>,
        purge_interval: Duration,
    ) -> Result<Self, Error> {
        Self::start_inner_guarded(root, roots, purge_interval, None)
    }
    pub(crate) fn start_guarded(
        root: &Path,
        roots: Vec<PathBuf>,
        guard: LifecycleGuard,
    ) -> Result<Self, Error> {
        Self::start_inner_guarded(root, roots, PURGE, Some(guard))
    }
    fn start_inner_guarded(
        root: &Path,
        roots: Vec<PathBuf>,
        purge_interval: Duration,
        guard: Option<LifecycleGuard>,
    ) -> Result<Self, Error> {
        lifecycle(guard.as_ref()).map_err(ReceiveFailure::wire)?;
        let (endpoint, listener) = Endpoint::claim(root)?;
        let endpoint = Arc::new(endpoint);
        let active = Arc::new(Mutex::new(None::<UnixStream>));
        let cache = Arc::new(Mutex::new(Store::default()));
        cache
            .lock()
            .map_err(|_| Error::Unavailable)?
            .enable(roots)
            .map_err(|_| Error::Input)?;
        let cancel = Arc::new(AtomicBool::new(false));
        let status = Arc::new(Mutex::new(Status {
            running: true,
            received: 0,
            rejected: 0,
            last_received_ms: None,
        }));
        let worker_cache = cache.clone();
        let worker_cancel = cancel.clone();
        let worker_status = status.clone();
        let worker_endpoint = endpoint.clone();
        let worker_active = active.clone();
        let handle = std::thread::Builder::new()
            .name("claude-plan-receiver".into())
            .spawn(move || {
                let endpoint = worker_endpoint;
                let _exit = WorkerExit {
                    cache: worker_cache.clone(),
                    cancel: worker_cancel.clone(),
                    status: worker_status.clone(),
                    endpoint: endpoint.clone(),
                };
                let mut purged = Instant::now();
                let mut checked = Instant::now();
                while !worker_cancel.load(Ordering::Acquire) {
                    if checked.elapsed() >= Duration::from_secs(1) {
                        if lifecycle(guard.as_ref()).is_err() {
                            break;
                        }
                        checked = Instant::now();
                    }
                    if endpoint.current().is_err()
                        || endpoint
                            .socket_id
                            .as_ref()
                            .is_none_or(|id| endpoint.socket_identity().as_ref() != Ok(id))
                    {
                        break;
                    }
                    if purged.elapsed() >= purge_interval {
                        if let Ok(mut cache) = worker_cache.try_lock() {
                            cache.purge_expired(Instant::now());
                        }
                        purged = Instant::now();
                    }
                    match listener_ready(&listener) {
                        Ok(true) => {}
                        Ok(false) => continue,
                        Err(_) => break,
                    }
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            if let Ok(mut active) = worker_active.lock() {
                                *active = stream.try_clone().ok();
                            }
                            let deadline = Instant::now() + EXCHANGE;
                            let result = receive_with_timing(
                                &mut stream,
                                &worker_cache,
                                deadline,
                                &worker_cancel,
                                &endpoint,
                                guard.as_ref(),
                                &Monotonic,
                            );
                            if let Ok(mut active) = worker_active.lock() {
                                *active = None;
                            }
                            let fatal = matches!(
                                result,
                                Err(ReceiveFailure::GuardPanicked
                                    | ReceiveFailure::Revoked
                                    | ReceiveFailure::Cancelled)
                            );
                            if let Ok(mut status) = worker_status.lock() {
                                match result {
                                    Ok(()) => {
                                        status.received = status.received.saturating_add(1);
                                        status.last_received_ms = Some(crate::tokens::now_ms());
                                    }
                                    Err(ReceiveFailure::Transport(Error::Input)) => {}
                                    Err(_) => {
                                        status.rejected = status.rejected.saturating_add(1);
                                    }
                                }
                            }
                            if fatal {
                                break;
                            }
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
                        Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                        Err(_) => break,
                    }
                }
                drop(listener);
                // _exit performs cleanup for every return and unwind path.
            })
            .map_err(|_| Error::Unavailable)?;
        Ok(Self {
            cache,
            cancel,
            status,
            endpoint,
            active,
            thread: Some(handle),
        })
    }
    pub(crate) fn cache(&self) -> Arc<Mutex<Store>> {
        self.cache.clone()
    }
    pub(crate) fn status(&self) -> Result<Status, Error> {
        self.status
            .lock()
            .map(|s| s.clone())
            .map_err(|_| Error::Unavailable)
    }
    /// Ok proves both body clearing and worker exit. Busy means a caller violated
    /// the bounded cache/guard contract: retain this Receiver and retry stop;
    /// Runtime must not announce that cleanup completed or start another receiver.
    pub(crate) fn stop(&mut self) -> Result<(), Error> {
        self.cancel.store(true, Ordering::Release);
        retire(&self.endpoint);
        if let Ok(active) = self.active.lock() {
            if let Some(stream) = active.as_ref() {
                let _ = stream.shutdown(std::net::Shutdown::Both);
            }
        }
        let deadline = Instant::now() + STOP_BUDGET;
        let cleared = clear_until(&self.cache, deadline);
        let mut exited = Ok(());
        while self
            .thread
            .as_ref()
            .is_some_and(|thread| !thread.is_finished())
        {
            if Instant::now() >= deadline {
                // Cancel and endpoint teardown are already done, but a blocked cache
                // consumer or in-flight callback can still delay exit. Report it.
                exited = Err(Error::Busy);
                break;
            }
            // Re-attempt while waiting: on the first pass the worker may not have
            // registered the accepted stream yet (accept/registration race).
            if let Ok(active) = self.active.lock() {
                if let Some(stream) = active.as_ref() {
                    let _ = stream.shutdown(std::net::Shutdown::Both);
                }
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        if exited.is_ok() {
            if let Some(thread) = self.thread.take() {
                thread.join().map_err(|_| Error::Unavailable)?;
            }
        }
        cleared.and(exited)
    }
}
impl Drop for Receiver {
    fn drop(&mut self) {
        // Drop cannot report a boundary failure; a caller that must prove cleanup
        // uses stop() and checks the result.
        let _ = self.stop();
    }
}
fn receive_with_timing(
    stream: &mut UnixStream,
    cache: &Arc<Mutex<Store>>,
    deadline: Instant,
    cancel: &AtomicBool,
    endpoint: &Endpoint,
    guard: Option<&LifecycleGuard>,
    timing: &dyn Timing,
) -> Result<(), ReceiveFailure> {
    checkpoint(timing, deadline, cancel, endpoint)?;
    peer(stream)?;
    let mut len = [0; 4];
    read_exact(stream, &mut len, deadline)?;
    let size = u32::from_be_bytes(len) as usize;
    if size == 0 || size > MAX_FRAME {
        return Err(Error::Input.into());
    }
    let mut body = vec![0; size];
    read_exact(stream, &mut body, deadline)?;
    // Require exactly one complete frame, EOF included. Extra data cannot become a second event.
    let mut extra = [0; 1];
    if read_some(stream, &mut extra, deadline)? != 0 {
        return Err(Error::Input.into());
    }
    let ready_deadline = deadline.min(timing.now() + SOURCE_READY);
    let mut readiness = crate::claude_plan_capture::SourceReadiness::default();
    let result = (|| {
        loop {
            // Guard runs without the cache lock; the lock is revalidated cheaply
            // inside cache_until and again before commit.
            live_checkpoint(timing, ready_deadline, cancel, endpoint, guard)?;
            let mut store = cache_until(cache, timing, ready_deadline, cancel, endpoint)?;
            checkpoint(timing, ready_deadline, cancel, endpoint)?;
            // Required capture API (companion diff in claude_plan_capture.rs, generic
            // over the error type so capture does not depend on this module):
            //   fn transaction<T, E>(&mut self, f: impl FnOnce(&mut Store) -> Result<T, E>)
            //       -> Result<T, E>
            // Restores the EXACT pre-attempt Store on Err (including eviction,
            // conflict and TTL effects); other valid entries/refs are untouched.
            // The mutex stays held through validation and ACK so no reader sees a
            // rolled-back body; the guard call below is the only callback under the
            // lock and must be bounded (never acquire this cache or dispatch to UI).
            let attempt = store.transaction::<Option<bool>, ReceiveFailure>(|candidate| {
                let ingested = candidate.ingest_ready(&body, timing.now(), &mut readiness);
                // Absolute exchange deadline (unchanged): a slow parse/validation
                // that still finished inside SOURCE_READY+EXCHANGE may commit;
                // anything past the deadline rolls back instead of entering the
                // cache. SOURCE_READY still governs only the readiness retry.
                checkpoint(timing, deadline, cancel, endpoint)?;
                if ingested == Err(crate::claude_plan_capture::Error::SourceMissing) {
                    return Ok(None);
                }
                live_checkpoint(timing, deadline, cancel, endpoint, guard)?;
                let accepted = ingested.is_ok();
                // Last recheck BEFORE the ACK: once the client observes success the
                // transaction must commit, or the client would hold a confirmation
                // for a body that no longer exists.
                checkpoint(timing, deadline, cancel, endpoint)?;
                write_all(stream, &[if accepted { 0 } else { 1 }], deadline)
                    .map_err(ReceiveFailure::from)?;
                // commit
                Ok(Some(accepted))
            });
            drop(store);
            match attempt? {
                Some(accepted) => {
                    return if accepted {
                        Ok(())
                    } else {
                        Err(Error::Rejected.into())
                    };
                }
                None => {}
            }
            checkpoint(timing, ready_deadline, cancel, endpoint)?;
            timing.pause(
                ready_deadline
                    .saturating_duration_since(timing.now())
                    .min(Duration::from_millis(10)),
            );
        }
    })();
    if result == Err(ReceiveFailure::Deadline) {
        // A readiness timeout has an exact internal reason and a negative ACK
        // when the original exchange still permits it. No transaction survives.
        if timing.now() < deadline && !cancel.load(Ordering::Acquire) {
            let _ = write_all(stream, &[1], deadline);
        }
    }
    result
}
fn send(root: &Path, bytes: &[u8], deadline: Instant) -> Result<(), Error> {
    if bytes.is_empty() || bytes.len() > MAX_FRAME {
        return Err(Error::Input);
    }
    let endpoint = Endpoint::open(root, false)?;
    let id = endpoint.socket_identity()?;
    let mut stream = connect(&endpoint.path(), deadline)?;
    endpoint.current()?;
    if endpoint.socket_identity()? != id {
        return Err(Error::Namespace);
    }
    write_all(&mut stream, &(bytes.len() as u32).to_be_bytes(), deadline)?;
    write_all(&mut stream, bytes, deadline)?;
    stream
        .shutdown(std::net::Shutdown::Write)
        .map_err(|_| Error::Unavailable)?;
    let mut ack = [0; 1];
    read_exact(&mut stream, &mut ack, deadline)?;
    if ack[0] != 0 {
        return Err(Error::Rejected);
    }
    let mut tail = [0; 1];
    if read_some(&stream, &mut tail, deadline)? == 0 {
        Ok(())
    } else {
        Err(Error::Unavailable)
    }
}

pub(super) fn collect_default() -> Result<(), Error> {
    let bytes = stdin_bytes(libc::STDIN_FILENO, Instant::now() + COLLECT)?;
    send(&super::default_root(), &bytes, Instant::now() + COLLECT)
}

#[cfg(test)]
#[path = "claude_plan_receiver_tests.rs"]
mod tests;
