//! The panic barrier every C entry point runs its body behind.
//!
//! A panic unwinding into C is undefined behaviour, so this is the one rule of
//! the boundary that has to live with the implementation rather than in
//! `migo-capi-abi`: it wraps engine calls, which is exactly what that crate is
//! kept free of. Everything else the boundary enforces — versioned structs,
//! borrowed-for-the-call strings, output size policy — is defined there and
//! imported directly by each entry point, not re-exported through here.

use std::panic::AssertUnwindSafe;

use migo_capi_abi::{MIGO_ERROR_INTERNAL, MigoResult};

/// Run an entry point's body with a panic barrier.
///
/// A panic becomes `MIGO_ERROR_INTERNAL` after being logged -- **in an
/// unwinding build**. Every shipping profile sets `panic = "abort"` (see
/// `[profile.release]`, which `release-hot2`/`release-hot3` inherit), and under
/// abort a panic terminates the process before any landing pad runs, so
/// `catch_unwind` here never observes it. Only the unwinding profiles --
/// `dev`, and therefore `cargo test` -- reach the `Err` arm below. Do not read
/// this barrier as a promise that a panicking engine leaves the host's process
/// alive; in a shipped artifact it does not.
///
/// The abort is deliberate rather than an oversight to be corrected by flipping
/// the profile. Rust code here is reached *through* C++ frames -- ops invoked
/// from V8, callbacks invoked from Skia -- and a panic unwinding through those
/// frames is undefined behaviour that a barrier way out at the C ABI boundary
/// cannot prevent, because the unwind has already crossed them by the time it
/// arrives. Aborting is the memory-safe outcome for that shape.
///
/// The barrier is kept because it is the correct construct for the builds that
/// do unwind, it costs nothing under abort (no landing pads are emitted), and
/// it keeps the boundary's intent stated where the boundary is.
pub fn guard(entry: &'static str, body: impl FnOnce() -> MigoResult) -> MigoResult {
    match std::panic::catch_unwind(AssertUnwindSafe(body)) {
        Ok(result) => result,
        Err(payload) => {
            let reason = payload
                .downcast_ref::<&str>()
                .map(|s| (*s).to_string())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "non-string panic payload".to_string());
            tracing::error!("panic crossed {entry}, contained: {reason}");
            MIGO_ERROR_INTERNAL
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use migo_capi_abi::MIGO_OK;

    /// Covers the unwinding profiles only, which is all it can cover.
    ///
    /// `cargo test` builds with `dev`, which unwinds, so the barrier engages
    /// here. Shipping profiles set `panic = "abort"` and would terminate the
    /// process at the `panic!` below, never reaching the assertion -- so a
    /// green run of this test says nothing about a shipped artifact's
    /// behaviour. See the note on [`guard`].
    #[test]
    fn panics_become_an_error_code_when_the_profile_unwinds() {
        assert!(
            cfg!(panic = "unwind"),
            "this test is only meaningful under an unwinding profile; \
             under panic=abort the panic below would abort the test process"
        );
        let result = guard("test_entry", || panic!("boom"));
        assert_eq!(result, MIGO_ERROR_INTERNAL);
    }

    #[test]
    fn guard_passes_through_normal_results() {
        assert_eq!(guard("test_entry", || MIGO_OK), MIGO_OK);
    }
}
