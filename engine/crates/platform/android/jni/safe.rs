/// Extract a human-readable message from a `catch_unwind` panic payload.
fn panic_message(e: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = e.downcast_ref::<&str>() {
        s.to_string()
    } else if let Some(s) = e.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic".to_string()
    }
}

/// Wraps a JNI `extern "system"` function body in `catch_unwind` to prevent
/// Rust panics from unwinding across the JNI boundary (which is undefined
/// behavior on Android and typically causes SIGABRT).
///
/// On panic, logs the panic message via `tracing::error!` and returns the
/// provided default value. For `void` JNI functions, use `()` as default.
macro_rules! jni_safe {
    // Void variant (no return value)
    ($name:expr, $body:expr) => {{
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            $body
        }));
        match result {
            Ok(val) => val,
            Err(e) => {
                tracing::error!("JNI panic in {}: {}", $name, $crate::android::jni::safe::panic_message(&e));
            }
        }
    }};
    // Return-value variant (with default on panic)
    ($name:expr, $default:expr, $body:expr) => {{
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            $body
        }));
        match result {
            Ok(val) => val,
            Err(e) => {
                tracing::error!("JNI panic in {}: {}", $name, $crate::android::jni::safe::panic_message(&e));
                $default
            }
        }
    }};
}

pub(crate) use jni_safe;
pub(crate) use panic_message;
