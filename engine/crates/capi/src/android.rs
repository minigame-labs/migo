//! Android-only entry point: supplying the JNI context a C host has none of.
//!
//! Every other Android capability this ABI exposes reaches the engine through
//! `ANativeWindow`, `ANativeActivity`, or a plain scalar -- none of it needs a
//! `JavaVM`. The Android audio backend is the one exception: cpal's
//! oboe/AAudio path calls into `ndk-context`, which the JNI-based platform
//! library initializes for itself in `on_load`. A pure C `NativeActivity`
//! host never runs that path, so without this call its first audio playback
//! aborts the process rather than failing an op -- `ndk-context` panics on
//! first use if nothing initialized it, and a panic here would cross an FFI
//! boundary as undefined behaviour.

use crate::panic_barrier::guard;
use migo_capi_abi::{MIGO_ERROR_INVALID_ARGUMENT, MIGO_OK, MigoResult};

/// Register this process's `JavaVM` and an Activity/Context `jobject` so the
/// Android audio backend can open an output stream.
///
/// Call once, before creating any session that will play audio -- the earliest
/// convenient point is right after `ANativeActivity_onCreate` hands the host
/// its `ANativeActivity`, which already carries both pointers (`vm` and
/// `clazz`). `activity` may be null for an audio-only embedding.
///
/// # Safety
/// `vm` must be null or a valid `JavaVM*` for this process for the duration
/// of the call. `activity`, if non-null, must be a valid `jobject`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn migo_android_init_context(
    vm: *mut std::ffi::c_void,
    activity: *mut std::ffi::c_void,
) -> MigoResult {
    guard("migo_android_init_context", || {
        // A null vm would still be stored by ndk-context and later
        // dereferenced by the JNI calls the audio backend makes through it --
        // a raw null-pointer crash far from this call site, not the
        // catchable panic the module doc above describes for skipping this
        // call entirely. Reject it here instead, like every other
        // pointer-taking entry point in this crate.
        if vm.is_null() {
            return MIGO_ERROR_INVALID_ARGUMENT;
        }
        unsafe { platform::android::init_context(vm, activity) };
        MIGO_OK
    })
}
