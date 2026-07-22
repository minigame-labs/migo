//! `libmigo.so` — the Android JNI entry point, and nothing else.
//!
//! Android loads exactly one symbol from this library: `JNI_OnLoad`. Every
//! other native method is bound at load time through `RegisterNatives` (see
//! `platform::android::jni::registration`), so this crate stays a one-function
//! shim over the real implementation in `platform`.
//!
//! It is a separate crate to keep Android's `cdylib` boundary out of
//! `platform`. With `platform` an rlib, each target chooses its own delivery
//! artifact, host builds can test the implementation directly, and the Linux
//! player links the same policy without inheriting a JNI library shape.

#![allow(non_snake_case)]

#[cfg(target_os = "android")]
mod android_entry {
    use jni::JavaVM;
    use jni::sys::jint;
    use std::ffi::c_void;

    /// Called by the Android runtime when `System.loadLibrary("migo")` runs.
    ///
    /// # Safety
    /// Invoked by the JVM with a valid `JavaVM`; the signature is fixed by the
    /// JNI specification.
    #[unsafe(no_mangle)]
    pub extern "system" fn JNI_OnLoad(vm: JavaVM, _reserved: *mut c_void) -> jint {
        platform::android::jni::on_load(vm)
    }
}
