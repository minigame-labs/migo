use jni::{AttachGuard, JNIEnv, JavaVM};
use std::{cell::RefCell, sync::OnceLock};

use crate::android::jni::{cache::JavaMethodCache, register_java_exports, register_native_exports};

static JVM: OnceLock<JavaVM> = OnceLock::new();

thread_local! {
    static THREAD_CTX: RefCell<Option<AttachGuard<'static>>> = RefCell::new(None);
}

pub(crate) static JAVA_METHOD_CACHE: OnceLock<JavaMethodCache> = OnceLock::new();

pub fn init_jni_env(jvm: JavaVM) -> Result<(), String> {
    JVM.set(jvm).map_err(|_| "Failed to set JVM".to_string())?;

    with_env(|env| register_java_exports(env))?;
    with_env(|env| register_native_exports(env))?;

    Ok(())
}

/// Run a closure with a thread-attached JNIEnv and allow error propagation.
///
/// # Errors
/// Returns an error if:
/// - JVM is not initialized (call `init_jni_env` first)
/// - Failed to attach the current thread to JVM
/// - The closure returns an error
pub fn with_env<F, R, E>(f: F) -> Result<R, E>
where
    F: FnOnce(&mut JNIEnv) -> Result<R, E>,
    E: From<String>,
{
    THREAD_CTX.with(|cell| {
        // If already attached on this thread, reuse it.
        if let Some(guard) = cell.borrow_mut().as_mut() {
            return f(guard);
        }

        let jvm = JVM
            .get()
            .ok_or_else(|| E::from("JVM not initialized".to_string()))?;

        let mut new_guard = jvm
            .attach_current_thread()
            .map_err(|e| E::from(format!("Failed to attach thread: {:?}", e)))?;

        let r = f(&mut new_guard);

        // Cache guard for next call on this thread.
        cell.borrow_mut().replace(new_guard);

        r
    })
}
