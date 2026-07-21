//! Lightweight ATrace scopes for the render thread.
//!
//! Wraps the NDK's `ATrace_beginSection` / `ATrace_endSection`
//! (API 23+, we're pinned to minSdk 26) so Perfetto / systrace /
//! AGI can visualise render-loop phases without going through
//! the full `tracing` subscriber stack.
//!
//! Every call site uses the [`scope!`] macro which creates an RAII
//! guard:
//!
//! ```ignore
//! use crate::atrace;
//! {
//!     atrace::scope!("migo.render.drain_cmds");
//!     drain_cmds(...);
//! } // end scope emits ATrace_endSection
//! ```
//!
//! On non-Android platforms the whole module compiles to no-ops
//! (including the macro), so host unit tests don't need to deal
//! with `dlsym` or the NDK.

#[cfg(target_os = "android")]
mod imp {
    use std::ffi::CString;

    // NDK `android/trace.h` prototype.  Linked via `-landroid`
    // which our build script already passes for other symbols.
    #[link(name = "android")]
    unsafe extern "C" {
        fn ATrace_beginSection(sectionName: *const std::os::raw::c_char);
        fn ATrace_endSection();
    }

    /// RAII guard.  Holding a live guard keeps the trace section
    /// open; dropping it closes the section.  Equivalent to
    /// `Trace.beginSection` / `Trace.endSection` in Java.
    pub struct Scope {
        // We don't need to store the CString across the lifetime
        // of the section because `ATrace_beginSection` copies the
        // name immediately — but keeping it around silences the
        // "temporary dropped while borrowed" diagnostic at call
        // sites that pass a `format!` result.
        _name: CString,
    }

    impl Scope {
        #[inline]
        pub fn new(name: &str) -> Self {
            // CString::new fails only on interior NULs; for our
            // static section names this is never the case, but
            // defensively truncate if ever it happens so tracing
            // doesn't crash a production build.
            let cname = CString::new(name)
                .unwrap_or_else(|_| CString::new("migo.atrace.invalid_name").unwrap());
            unsafe { ATrace_beginSection(cname.as_ptr()) };
            Self { _name: cname }
        }
    }

    impl Drop for Scope {
        #[inline]
        fn drop(&mut self) {
            unsafe { ATrace_endSection() };
        }
    }
}

#[cfg(not(target_os = "android"))]
mod imp {
    /// No-op stand-in so host builds compile without libandroid.
    pub struct Scope;
    impl Scope {
        #[inline]
        pub fn new(_name: &str) -> Self {
            Self
        }
    }
}

pub use imp::Scope;

/// Open an ATrace section for the remainder of the enclosing
/// block.  Picks up the current file/line at zero runtime cost on
/// non-Android targets (the macro expands to a unit binding).
#[macro_export]
macro_rules! atrace_scope {
    ($name:expr) => {
        let _atrace_guard = $crate::atrace::Scope::new($name);
    };
}
