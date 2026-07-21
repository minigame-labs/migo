//! `libmigo.so` — the Android JNI entry point, and nothing else.
//!
//! Android loads exactly one symbol from this library: `JNI_OnLoad`. Every
//! other native method is bound at load time through `RegisterNatives` (see
//! `platform::android::jni::registration`), so this crate stays a one-function
//! shim over the real implementation in `platform`.
//!
//! It is a separate crate purely to keep the cdylib boundary out of
//! `platform`: a crate that is `cdylib` is built as one for every target, and
//! on a glibc host the Linux V8 archive cannot be linked into a shared object
//! (`R_X86_64_TPOFF32 ... cannot be used with -shared`). With `platform` an
//! rlib, host builds and `cargo test -p platform` work, and the Linux player
//! links it directly instead of the build script rewriting its crate-type.

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
