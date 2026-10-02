//! Reclaim Rust-owned parsing buffers on the sampling thread after a tick.
//! AppKit/WebKit retain their own system allocator; this does not free live objects.

#[cfg(target_os = "macos")]
pub fn reclaim_idle_pages() {
    // Public mimalloc API, linked by the macOS global allocator in main.rs.
    // Called on the same thread that parsed the transcripts, after temporary
    // Values/tail buffers have dropped. Forcing collection purges unused pages.
    // https://microsoft.github.io/mimalloc/group__malloc.html
    extern "C" { fn mi_collect(force: bool); }
    // SAFETY: no pointers or ownership transfer; mi_collect is thread-safe and
    // operates on the calling thread's heap. Live Rust allocations remain valid.
    unsafe { mi_collect(true); }
}

#[cfg(not(target_os = "macos"))]
pub fn reclaim_idle_pages() {}
