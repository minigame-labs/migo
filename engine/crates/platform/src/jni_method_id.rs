//! Resolve a JNI static-method id once, past the name-keyed method cache.
//!
//! `JavaMethodCache` maps every `NativeExports` method name to its
//! `JStaticMethodID`, and the generic outbound helpers look one up by name on
//! every call — a `HashMap<String, _>` probe, so a SipHash of the method name
//! per call. That is invisible for the calls a user action drives, and not for
//! `requestVsync`, which the render thread issues once per frame while content
//! is animating.
//!
//! [`once_get_or_try_init`] is `OnceLock::get_or_try_init` (still nightly): the
//! first call resolves the id out of the cache and stores it, every call after
//! reads the stored copy. It lives here rather than inline in `outbound` so the
//! store-once / retry-on-error behaviour has a test that does not need a JVM.

use std::sync::OnceLock;

/// Return the value in `slot`, or run `init` once and store its `Ok`.
///
/// An `Err` leaves the slot empty, so a transient failure to resolve (the cache
/// not populated yet, say) is retried on the next call rather than cached
/// forever.
pub(crate) fn once_get_or_try_init<T, E>(
    slot: &OnceLock<T>,
    init: impl FnOnce() -> Result<T, E>,
) -> Result<&T, E> {
    if let Some(value) = slot.get() {
        return Ok(value);
    }
    let value = init()?;
    // A concurrent initialiser may have won the race; its value is kept and
    // ours dropped. For a `JStaticMethodID` the two are identical, so which one
    // wins does not matter.
    Ok(slot.get_or_init(|| value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn the_first_ok_is_stored_and_init_never_runs_again() {
        let slot: OnceLock<u32> = OnceLock::new();
        let runs = Cell::new(0);

        let first = once_get_or_try_init(&slot, || {
            runs.set(runs.get() + 1);
            Ok::<u32, ()>(42)
        });
        let second = once_get_or_try_init(&slot, || {
            runs.set(runs.get() + 1);
            Ok::<u32, ()>(99)
        });

        assert_eq!(first, Ok(&42));
        assert_eq!(
            second,
            Ok(&42),
            "the stored value must win over a later init"
        );
        assert_eq!(runs.get(), 1, "init ran more than once");
    }

    #[test]
    fn an_error_leaves_the_slot_empty_so_the_next_call_retries() {
        let slot: OnceLock<u32> = OnceLock::new();

        let failed = once_get_or_try_init(&slot, || Err::<u32, &str>("not ready"));
        assert_eq!(failed, Err("not ready"));
        assert!(slot.get().is_none(), "a failed init poisoned the slot");

        let recovered = once_get_or_try_init(&slot, || Ok::<u32, &str>(7));
        assert_eq!(recovered, Ok(&7));
    }

    /// `request_vsync` is the one outbound call on the render thread's per-frame
    /// path. Every other one is fine going through the name-keyed cache because
    /// a user action paces it; this one must resolve its id once. Reverting it
    /// to `jni_void!` is the regression this catches -- the JNI layer has no
    /// test execution anywhere, only compilation, so a source check is the
    /// honest instrument here.
    #[test]
    fn request_vsync_resolves_its_method_id_once_not_per_frame() {
        let src = include_str!("android/jni/outbound.rs");
        let start = src
            .find("pub fn request_vsync(")
            .expect("request_vsync must be a hand-written function");
        let body = &src[start
            ..src[start..]
                .find("\n}\n")
                .map(|offset| start + offset)
                .expect("request_vsync body must close")];

        assert!(
            body.contains("static METHOD_ID: OnceLock<JStaticMethodID>"),
            "request_vsync must cache its resolved method id in a static"
        );
        assert!(
            body.contains("once_get_or_try_init(&METHOD_ID"),
            "request_vsync must resolve through the cached slot, not the name-keyed lookup"
        );
    }
}
