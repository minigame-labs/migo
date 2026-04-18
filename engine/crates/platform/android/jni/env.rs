use jni::{JNIEnv, JavaVM};
use std::{cell::RefCell, sync::OnceLock};

use crate::android::jni::{cache::JavaMethodCache, register_java_exports, register_native_exports};

static JVM: OnceLock<JavaVM> = OnceLock::new();

thread_local! {
    // Daemon-attached JNIEnv: unlike AttachGuard, does not call
    // DetachCurrentThread on drop, avoiding JVM bookkeeping overhead
    // for transient threads (e.g. Tokio blocking pool).
    static THREAD_CTX: RefCell<Option<JNIEnv<'static>>> = RefCell::new(None);
}

pub(crate) static JAVA_METHOD_CACHE: OnceLock<JavaMethodCache> = OnceLock::new();

pub fn init_jni_env(jvm: JavaVM) -> Result<(), String> {
    JVM.set(jvm).map_err(|_| "Failed to set JVM".to_string())?;

    with_env(|env| register_java_exports(env))?;
    with_env(|env| register_native_exports(env))?;

    // Register Android BitmapFactory as the platform image decoder.
    // This replaces Rust image/zune-image decoders on Android, saving ~2-4 MB in the binary.
    io::register_platform_decoder(|data| {
        crate::android::jni::outbound::decode_image_rgba_jni(data)
    });

    // Register the zero-copy AHB decoder (API 28+ writes directly
    // into an AHardwareBuffer; on API 26/27 falls back to BitmapFactory
    // + Bitmap.copy(Config.HARDWARE); Rust imports via EGLImage).
    // `decode_image_to_any` picks this path preferentially and
    // transparently falls back to `PLATFORM_DECODER` per-image if AHB
    // decode fails (e.g. vendor driver quirk).
    io::register_platform_ahb_decoder(|data| {
        crate::android::jni::outbound::decode_image_ahb_jni(data)
    });

    Ok(())
}

/// Run a closure with a thread-attached JNIEnv and allow error propagation.
///
/// Every invocation pushes a JNI local-reference frame so that all Java
/// objects created inside `f` are freed when the closure returns.  Without
/// this, daemon-attached threads (which never "return from a native method")
/// would leak every local reference, eventually exhausting the Java heap.
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
            return invoke_in_local_frame(guard, f);
        }

        let jvm = JVM
            .get()
            .ok_or_else(|| E::from("JVM not initialized".to_string()))?;

        // Use daemon attachment: daemon-attached threads don't prevent JVM
        // shutdown and have less overhead for transient threads (no
        // DetachCurrentThread on Drop, which avoids JVM bookkeeping).
        let mut new_guard = jvm
            .attach_current_thread_as_daemon()
            .map_err(|e| E::from(format!("Failed to attach thread: {:?}", e)))?;

        let r = invoke_in_local_frame(&mut new_guard, f);

        // Cache guard for next call on this thread.
        cell.borrow_mut().replace(new_guard);

        r
    })
}

/// Push a JNI local-reference frame, run `f`, then pop the frame.
///
/// All JNI local references created inside `f` are freed when the frame
/// is popped, regardless of whether `f` succeeds or fails.
///
/// # Safety
/// Callers must not return JNI local references (JObject, JByteArray, …)
/// from `f` — they become invalid after `pop_local_frame`.  All existing
/// call-sites return Rust-owned types (String, Vec<u8>, NormalizedImage, …)
/// so this is satisfied.
fn invoke_in_local_frame<F, R, E>(env: &mut JNIEnv, f: F) -> Result<R, E>
where
    F: FnOnce(&mut JNIEnv) -> Result<R, E>,
    E: From<String>,
{
    // 16 slots is enough for typical outbound calls (5-10 JNI refs each).
    // The JVM will auto-expand if more are needed.
    env.push_local_frame(16)
        .map_err(|e| E::from(format!("PushLocalFrame failed: {e}")))?;

    let r = f(env);

    // Safety: no JNI local reference escapes `f` — all callers return
    // Rust-owned types.  Passing JObject::null() means nothing is
    // promoted to the outer frame.
    //
    // If pop_local_frame fails (e.g. pending exception from `f`),
    // clear the exception so it doesn't leak into subsequent JNI calls.
    if let Err(e) = unsafe { env.pop_local_frame(&jni::objects::JObject::null()) } {
        if env.exception_check().unwrap_or(false) {
            env.exception_clear().ok();
        }
        return Err(E::from(format!("PopLocalFrame failed: {e}")));
    }

    r
}
