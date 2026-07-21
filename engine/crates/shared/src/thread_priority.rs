//! Cross-platform thread priority helper.
//!
//! On Android, uses `libc::setpriority(PRIO_PROCESS, tid, nice)`.
//! Negative nice values (e.g. `Display = -4`) require `CAP_SYS_NICE`
//! which most app processes lack — these will silently fail and the
//! thread keeps its default priority.  Non-negative values (Background,
//! Default) succeed reliably.
//!
//! On other platforms, this is a best-effort no-op.

/// Android thread priority constants (from android.os.Process).
#[derive(Debug, Clone, Copy)]
#[repr(i32)]
pub enum Priority {
    /// Render thread — highest non-realtime priority.
    Display = -4,
    /// Host/JS thread.
    Foreground = -2,
    /// Default (upload thread).
    Default = 0,
    /// IO, decode — below default.
    Background = 10,
}

/// Set the current thread's priority.  Best-effort: logs a warning on failure
/// but never panics.
pub fn set_current_thread_priority(priority: Priority) {
    let name = std::thread::current()
        .name()
        .unwrap_or("unnamed")
        .to_string();

    #[cfg(target_os = "android")]
    {
        if let Err(e) = android_set_priority(priority as i32) {
            tracing::debug!(
                "set_thread_priority({name}, {:?}) failed: {e} — continuing with default",
                priority
            );
            return;
        }
    }

    #[cfg(not(target_os = "android"))]
    {
        let _ = priority; // suppress unused warning
    }

    tracing::trace!("thread '{name}' priority set to {:?}", priority);
}

/// Call `android.os.Process.setThreadPriority(tid, priority)` via JNI.
///
/// Uses the cached JNIEnv from the platform crate if available, otherwise
/// falls back to raw syscall `setpriority(PRIO_PROCESS, tid, nice)`.
#[cfg(target_os = "android")]
fn android_set_priority(priority: i32) -> Result<(), String> {
    // Try the POSIX setpriority path first (works for non-negative nice values
    // and for negative values when the process has CAP_SYS_NICE).
    // This avoids JNI overhead on threads that may not have a JNIEnv attached.
    let tid = unsafe { libc::gettid() };
    let ret = unsafe { libc::setpriority(libc::PRIO_PROCESS, tid as u32, priority) };
    if ret == 0 {
        return Ok(());
    }
    Err(format!(
        "setpriority(tid={tid}, nice={priority}) errno={}",
        std::io::Error::last_os_error()
    ))
}
