//! Host-runtime blocking-pool policy, pinned against the source.

const THREAD: &str = include_str!("../thread.rs");

#[test]
fn host_runtime_does_not_prewarm_tokio_blocking_threads() {
    assert!(!THREAD.contains("tokio::task::spawn_blocking"));
    assert!(!THREAD.contains("Pre-warm the blocking thread pool"));
}

#[test]
fn host_runtime_uses_four_slot_lazy_blocking_fallback() {
    assert!(THREAD.contains("const HOST_BLOCKING_FALLBACK_THREADS: usize = 4;"));
    assert!(THREAD.contains(".max_blocking_threads(HOST_BLOCKING_FALLBACK_THREADS)"));
    assert!(!THREAD.contains(".max_blocking_threads(32)"));
}
