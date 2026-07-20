//! Selection between EGL 1.5/EXT platform entry points and EGL 1.4 native
//! platform bindings.
//!
//! `eglGetProcAddress` returning a pointer is not proof that the current EGL
//! implementation supports that entry point for a particular platform. Some
//! loaders expose global stubs which return `EGL_NO_DISPLAY`/`EGL_NO_SURFACE`.
//! A failed preferred call must therefore fall through to the legacy binding,
//! not be mistaken for definitive platform rejection.

#[inline]
pub(crate) fn preferred_or_fallback<F, T>(
    preferred: Option<F>,
    invoke_preferred: impl FnOnce(F) -> Option<T>,
    invoke_fallback: impl FnOnce() -> Option<T>,
) -> Option<T> {
    preferred
        .and_then(invoke_preferred)
        .or_else(invoke_fallback)
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::preferred_or_fallback;

    #[test]
    fn a_resolved_but_unsupported_platform_call_falls_back() {
        let fallback_calls = Cell::new(0);
        let value = preferred_or_fallback(
            Some(7_u8),
            |_entry| None,
            || {
                fallback_calls.set(fallback_calls.get() + 1);
                Some(11_u8)
            },
        );
        assert_eq!(value, Some(11));
        assert_eq!(fallback_calls.get(), 1);
    }

    #[test]
    fn a_successful_platform_call_does_not_touch_legacy_egl() {
        let fallback_calls = Cell::new(0);
        let value = preferred_or_fallback(
            Some(7_u8),
            |entry| Some(entry + 1),
            || {
                fallback_calls.set(fallback_calls.get() + 1);
                Some(11_u8)
            },
        );
        assert_eq!(value, Some(8));
        assert_eq!(fallback_calls.get(), 0);
    }
}
