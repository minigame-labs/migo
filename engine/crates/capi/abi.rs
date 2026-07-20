//! ABI boundary discipline: result codes, versioned-struct validation, and the
//! panic barrier.
//!
//! Everything a C caller can reach goes through here, so the rules that make
//! the boundary safe live in one place instead of being restated (and
//! eventually mis-stated) in every entry point.

use std::panic::AssertUnwindSafe;

#[cfg(test)]
use std::mem::size_of;

// The staged boundary migration deliberately keeps this module as a facade so
// existing runtime modules do not each import the new crate differently.
#[allow(unused_imports)]
pub use migo_capi_abi::{
    AbiStruct, MIGO_ABI_VERSION_CURRENT, MIGO_ERROR_CANCELLED, MIGO_ERROR_DISPATCH_REJECTED,
    MIGO_ERROR_INTERNAL, MIGO_ERROR_INVALID_ARGUMENT, MIGO_ERROR_INVALID_STATE,
    MIGO_ERROR_OUT_OF_MEMORY, MIGO_ERROR_STALE_SURFACE, MIGO_ERROR_UNSUPPORTED_ABI,
    MIGO_ERROR_UNSUPPORTED_CAPABILITY, MIGO_ERROR_UNSUPPORTED_PLATFORM, MIGO_ERROR_WOULD_BLOCK,
    MIGO_ERROR_WRONG_THREAD, MIGO_OK, MigoResult, OutputVersionPolicy, VersionedHeader, copy_utf8,
    copy_versioned, validate_header, write_versioned_output,
};

/// Run an entry point's body with a panic barrier.
///
/// A panic unwinding into C is undefined behaviour, so every boundary function
/// wraps its body: a panic becomes `MIGO_ERROR_INTERNAL` after being logged,
/// which keeps the host's process alive and debuggable.
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
    use std::os::raw::c_char;

    #[repr(C)]
    struct Sized8 {
        header: VersionedHeader,
    }

    fn header(struct_size: u32, abi_version: u32) -> Sized8 {
        Sized8 {
            header: VersionedHeader {
                struct_size,
                abi_version,
            },
        }
    }

    #[test]
    fn null_struct_is_an_argument_error() {
        let result = unsafe { validate_header(std::ptr::null(), size_of::<Sized8>()) };
        assert_eq!(result, Err(MIGO_ERROR_INVALID_ARGUMENT));
    }

    #[test]
    fn foreign_abi_version_is_rejected_before_size() {
        // Distinct from a size mismatch on purpose: a host built against a
        // different ABI needs to know that, not "bad argument".
        let value = header(size_of::<Sized8>() as u32, MIGO_ABI_VERSION_CURRENT + 1);
        let result = unsafe {
            validate_header(
                &value as *const Sized8 as *const VersionedHeader,
                size_of::<Sized8>(),
            )
        };
        assert_eq!(result, Err(MIGO_ERROR_UNSUPPORTED_ABI));
    }

    #[test]
    fn mismatched_struct_size_is_rejected() {
        // A short struct would leave later fields uninitialised if trusted.
        let value = header(4, MIGO_ABI_VERSION_CURRENT);
        let result = unsafe {
            validate_header(
                &value as *const Sized8 as *const VersionedHeader,
                size_of::<Sized8>(),
            )
        };
        assert_eq!(result, Err(MIGO_ERROR_INVALID_ARGUMENT));
    }

    #[test]
    fn matching_header_is_accepted() {
        let value = header(size_of::<Sized8>() as u32, MIGO_ABI_VERSION_CURRENT);
        let result = unsafe {
            validate_header(
                &value as *const Sized8 as *const VersionedHeader,
                size_of::<Sized8>(),
            )
        };
        assert_eq!(result, Ok(()));
    }

    #[test]
    fn panics_become_an_error_code_instead_of_unwinding_into_c() {
        let result = guard("test_entry", || panic!("boom"));
        assert_eq!(result, MIGO_ERROR_INTERNAL);
    }

    #[test]
    fn guard_passes_through_normal_results() {
        assert_eq!(guard("test_entry", || MIGO_OK), MIGO_OK);
    }

    #[test]
    fn null_and_invalid_utf8_strings_are_argument_errors() {
        assert_eq!(
            unsafe { copy_utf8(std::ptr::null()) },
            Err(MIGO_ERROR_INVALID_ARGUMENT)
        );
        let invalid = [0xffu8, 0xfe, 0x00];
        assert_eq!(
            unsafe { copy_utf8(invalid.as_ptr() as *const c_char) },
            Err(MIGO_ERROR_INVALID_ARGUMENT)
        );
    }

    #[test]
    fn valid_string_is_copied_not_borrowed() {
        let source = std::ffi::CString::new("/tmp/migo").expect("cstring");
        let copied = unsafe { copy_utf8(source.as_ptr()) }.expect("copy");
        drop(source);
        assert_eq!(copied, "/tmp/migo");
    }
}
