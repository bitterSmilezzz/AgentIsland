# Vendored dependency: Tao 0.37.1

Source: [official crate release](https://crates.io/crates/tao/0.37.1), [upstream tag](https://github.com/tauri-apps/tao/tree/tao-v0.37.1).
The published archive SHA256 is `f37f381f4e048e6cdf038b5705f8cf14ad108279d46eb968140a7b291aba9400`, matching the original Cargo lock checksum. Sources, manifest, examples, README and Apache-2.0 LICENSE are retained; Cargo cache metadata and the dependency's own lockfile are omitted.

AgentIsland changes three macOS source files (2026-10-08). In `tao/src/platform_impl/macos/window.rs`, adopt the `+1` pointer returned by `alloc`/`initWithContentRect:` with `Retained::from_raw`, instead of adding another retain. In `view.rs` and `window_delegate.rs`, finish the custom `dealloc` callbacks by invoking the statically declared superclass after releasing Rust state. These callbacks are not compiled with Objective-C ARC; [Apple's memory management contract](https://developer.apple.com/library/archive/documentation/Cocoa/Conceptual/MemoryMgmt/Articles/mmRules.html) requires this final superclass call. Window styles, events and cross-platform implementations are unchanged. The ordinary application and isolated regression fixture both use this same path patch.

The isolated `scripts/window-lifetime-probe` fixture runs ten actual Tao creation/drop cycles. It checks zeroing weak references and the actual `heap -s` allocation counts for windows, views and delegates before/after; a cleared weak reference alone does not prove the allocation was freed. It contains no WebView, application business logic or material layer. It is compiled and run by the VM native gates, never by ordinary unit tests; the existing explicit `ALLOW_DESKTOP_UI=1` release path supplies `--allow-desktop`, otherwise the fixture refuses a host desktop. This is a lifetime regression, not a memory budget or proof of the application's full resource stability.

Replace the path patch with an upstream release when the ownership fix is published, retain the native regression, and verify formal application UI, Dock, single-instance, CLI, animation and long-duration resource behavior before removing this directory. Do not modify Cargo registry caches as a delivery mechanism.

## Vendored dependency: tiny_http 0.12.0

Source: registry crate `tiny_http-0.12.0` (crates.io, `MIT OR Apache-2.0`), archive SHA256
`389915df6413a2e74fb181895f933386023c71110878cd0825588928e64cdc82` — identical to the checksum
the original `Cargo.lock` pinned, which is the proof this copy is unmodified upstream plus the
patch below. `Cargo.lock` now records the path dependency without a registry checksum.

AgentIsland patches it (2026-10-16) because two required behaviours cannot be expressed through
`tiny_http`'s public API, and the HTTP parser itself must not be rewritten:

- **Ingress timing.** `Request::arrival()` returns `RequestArrival` (monotonic `Instant` plus wall
  clock) captured *before* parsing and before the library's internal messages queue: at `accept()`
  for the first request on a connection, and at the first byte of the request line for later
  keep-alive requests, so idle keep-alive time is not charged as latency. Without this, a request
  whose header or small (≤1024-byte) body is still being read, or which waits in `recv_timeout`'s
  queue behind a slow predecessor, is timestamped only after that wait and reads as ~0 ms.
- **Bounded shutdown.** `Server::shutdown_handle()` returns a `ShutdownHandle` that shuts down the
  accepted sockets, and `Server::from_listener_with_timeout` installs per-socket timeouts. The
  accept thread registers a `Weak` socket guard that is held by the IO wrappers, so dropping the
  server cancels in-flight header/body reads and response writes instead of blocking on a client
  that keeps a partial request open. The write timeout applies from the first write; the read
  timeout is armed only while a request is in flight — it is *disarmed* while a kept-alive
  connection waits idle for its next request, so idle wait is not reported as a 408 and not charged
  to the next request's latency. Helpers: `EqualReader` reports a short `Content-Length` body as
  `UnexpectedEof` instead of silently treating EOF as a complete body (`Request::drop` also emits a
  500 in that case); `Connection::read` shuts the socket down on timeout so destructor draining
  cannot re-block. Header parsing, routing, method/URL handling and the rest of the library are
  unchanged.

Existing upstream `tests/` are retained (they need dev-dependencies such as `fdlimit`,
`rustc_serialize` and `sha1` that are not vendored, so they are not run here). Omitted from the
copy: `examples/`, `benches/`, the crate's own `CHANGELOG.md`/lockfile and CI metadata. The
application-side regressions live in `app/src-tauri/src/webhook_ingress_tests.rs` and run under
`cargo test --locked --manifest-path app/src-tauri/Cargo.toml webhook::ingress_tests`, which also
pins the shorter-body rejection.
