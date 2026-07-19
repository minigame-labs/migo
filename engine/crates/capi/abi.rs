//! ABI boundary discipline: result codes, versioned-struct validation, and the
//! panic barrier.
//!
//! Everything a C caller can reach goes through here, so the rules that make
//! the boundary safe live in one place instead of being restated (and
//! eventually mis-stated) in every entry point.

use std::{ffi::CStr, os::raw::c_char, panic::AssertUnwindSafe};

/// Mirrors `MigoResult` in `include/migo/types.h`.
pub type MigoResult = i32;

pub const MIGO_OK: MigoResult = 0;
pub const MIGO_ERROR_INVALID_ARGUMENT: MigoResult = -1;
pub const MIGO_ERROR_UNSUPPORTED_ABI: MigoResult = -2;
pub const MIGO_ERROR_UNSUPPORTED_PLATFORM: MigoResult = -3;
#[allow(dead_code)]
pub const MIGO_ERROR_UNSUPPORTED_CAPABILITY: MigoResult = -4;
pub const MIGO_ERROR_INVALID_STATE: MigoResult = -5;
#[allow(dead_code)]
pub const MIGO_ERROR_WRONG_THREAD: MigoResult = -6;
#[allow(dead_code)]
pub const MIGO_ERROR_STALE_SURFACE: MigoResult = -7;
#[allow(dead_code)]
pub const MIGO_ERROR_CANCELLED: MigoResult = -8;
#[allow(dead_code)]
pub const MIGO_ERROR_DISPATCH_REJECTED: MigoResult = -9;
#[allow(dead_code)]
pub const MIGO_ERROR_OUT_OF_MEMORY: MigoResult = -10;
pub const MIGO_ERROR_INTERNAL: MigoResult = -11;

/// The host command queue was full and the event was not delivered. Transient:
/// the same call may succeed later. Reported rather than swallowed because a
/// dropped `MIGO_TOUCH_END` leaves content believing a finger is still down.
pub const MIGO_ERROR_WOULD_BLOCK: MigoResult = -12;

pub const MIGO_ABI_VERSION_CURRENT: u32 = 1;

/// Header shared by every versioned struct the caller passes in.
///
/// Reading it requires only that the pointer is non-null and readable for these
/// eight bytes, which is exactly what `validate_header` checks before anything
/// trusts `struct_size`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VersionedHeader {
    pub struct_size: u32,
    pub abi_version: u32,
}

/// Validate a caller-supplied versioned struct.
///
/// `expected_size` is the size this build compiled; a caller from a newer or
/// older header is rejected rather than silently reinterpreted. ABI v1 requires
/// an exact match — growth happens through new flagged fields in a later ABI
/// version, not by accepting short structs today.
///
/// # Safety
/// `header` must either be null or point to a readable [`VersionedHeader`].
pub unsafe fn validate_header(
    header: *const VersionedHeader,
    expected_size: usize,
) -> Result<(), MigoResult> {
    let Some(header) = (unsafe { header.as_ref() }) else {
        return Err(MIGO_ERROR_INVALID_ARGUMENT);
    };
    if header.abi_version != MIGO_ABI_VERSION_CURRENT {
        return Err(MIGO_ERROR_UNSUPPORTED_ABI);
    }
    if header.struct_size as usize != expected_size {
        return Err(MIGO_ERROR_INVALID_ARGUMENT);
    }
    Ok(())
}

/// Copy a caller-owned UTF-8 C string.
///
/// The ABI borrows strings for the duration of a call only, so every entry
/// point that keeps one copies it here. Invalid UTF-8 is an argument error, not
/// a lossy conversion: a mangled path would fail later somewhere far less
/// obvious.
///
/// # Safety
/// `text` must be null or a NUL-terminated string valid for the call.
pub unsafe fn copy_utf8(text: *const c_char) -> Result<String, MigoResult> {
    if text.is_null() {
        return Err(MIGO_ERROR_INVALID_ARGUMENT);
    }
    unsafe { CStr::from_ptr(text) }
        .to_str()
        .map(str::to_owned)
        .map_err(|_| MIGO_ERROR_INVALID_ARGUMENT)
}

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
