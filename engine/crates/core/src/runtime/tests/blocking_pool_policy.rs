//! Host-runtime blocking-pool policy, pinned against the source.

const THREAD: &str = include_str!("../thread.rs");
// The Tokio runtime is built by the shared session-thread module: it is the
// same runtime whichever execution mode runs on top of it, and tokio is not a
// JavaScript engine, so it belongs on the side both modes compile.
const SESSION_THREAD: &str = include_str!("../session_thread.rs");

#[test]
fn host_runtime_does_not_prewarm_tokio_blocking_threads() {
    for source in [THREAD, SESSION_THREAD] {
        assert!(!source.contains("tokio::task::spawn_blocking"));
        assert!(!source.contains("Pre-warm the blocking thread pool"));
    }
}

#[test]
fn host_runtime_uses_four_slot_lazy_blocking_fallback() {
    assert!(SESSION_THREAD.contains("const HOST_BLOCKING_FALLBACK_THREADS: usize = 4;"));
    assert!(SESSION_THREAD.contains(".max_blocking_threads(HOST_BLOCKING_FALLBACK_THREADS)"));
    for source in [THREAD, SESSION_THREAD] {
        assert!(!source.contains(".max_blocking_threads(32)"));
    }
}
