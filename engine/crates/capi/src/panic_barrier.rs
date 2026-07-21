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
/// A panic becomes `MIGO_ERROR_INTERNAL` after being logged, which keeps the
/// host's process alive and debuggable.
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

    #[test]
    fn panics_become_an_error_code_instead_of_unwinding_into_c() {
        let result = guard("test_entry", || panic!("boom"));
        assert_eq!(result, MIGO_ERROR_INTERNAL);
    }

    #[test]
    fn guard_passes_through_normal_results() {
        assert_eq!(guard("test_entry", || MIGO_OK), MIGO_OK);
    }
}
