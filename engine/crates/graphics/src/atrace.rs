//! Lightweight ATrace scopes for the render thread.
//!
//! Wraps the NDK's `ATrace_beginSection` / `ATrace_endSection`
//! (API 23+, we're pinned to minSdk 26) so Perfetto / systrace /
//! AGI can visualise render-loop phases without going through
//! the full `tracing` subscriber stack.
//!
//! Every call site uses the [`atrace_scope!`] macro which creates an
//! RAII guard:
//!
//! ```ignore
//! {
//!     crate::atrace_scope!(c"migo.render.drain_cmds");
//!     drain_cmds(...);
//! } // end scope emits ATrace_endSection
//! ```
//!
//! **The name is a C string literal, so opening a scope allocates
//! nothing.** It used to be a `&str` that [`Scope::new`] copied into a
//! `CString`, which is a `malloc` and a `free` per scope on the render
//! thread — paid on every frame of every Android build, whether or not
//! anything was tracing, because nothing here consults
//! `ATrace_isEnabled`. Four scopes on the present path made that eight
//! heap operations per frame that no shipped device ever benefited from.
//!
//! Requiring a literal also forecloses the shape that reintroduces the
//! cost: a `format!`-ed section name cannot be passed at all, and a
//! per-frame trace label built on the render thread is the same
//! allocation wearing a different hat. Scope names are for phases, and
//! phases are known at compile time.
//!
//! On non-Android platforms the whole module compiles to no-ops
//! (including the macro), so host unit tests don't need to deal
//! with `dlsym` or the NDK.

#[cfg(target_os = "android")]
mod imp {
    use std::ffi::CStr;

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
    ///
    /// Carries no data: `ATrace_beginSection` copies the name before it
    /// returns, and the name is `'static` regardless.
    pub struct Scope;

    impl Scope {
        #[inline]
        pub fn new(name: &'static CStr) -> Self {
            unsafe { ATrace_beginSection(name.as_ptr()) };
            Self
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
    use std::ffi::CStr;

    /// No-op stand-in so host builds compile without libandroid.
    pub struct Scope;
    impl Scope {
        #[inline]
        pub fn new(_name: &'static CStr) -> Self {
            Self
        }
    }
}

pub use imp::Scope;

/// The guard owns nothing, on every target.
///
/// Outside the `cfg` blocks on purpose: the Android arm of this module is the
/// one that matters and the one a host build cannot compile, so the property
/// is asserted where *both* builds check it. A field on `Scope` is a `malloc`
/// and a `free` per scope on the render thread, which is what this replaced.
const _: () = assert!(
    std::mem::size_of::<Scope>() == 0,
    "atrace::Scope must own nothing"
);

/// Open an ATrace section for the remainder of the enclosing block.
///
/// Takes a C string literal (`c"…"`). Zero runtime cost on non-Android
/// targets (the macro expands to a unit binding) and zero allocation on
/// Android.
#[macro_export]
macro_rules! atrace_scope {
    ($name:literal) => {
        let _atrace_guard = $crate::atrace::Scope::new($name);
    };
}
