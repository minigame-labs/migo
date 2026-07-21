//! Versioned-header and borrowed-string rules at the C boundary.
//!
//! These moved here with `validate_header`/`copy_utf8` themselves. While they
//! lived in `capi` they could not run in CI at all — that crate pulls in
//! graphics, so it needs a device or a full Skia/EGL stack to build, and the
//! rules being checked need neither.

use std::{ffi::CString, mem::size_of, os::raw::c_char};

use migo_capi_abi::{
    MIGO_ABI_VERSION_CURRENT, MIGO_ERROR_INVALID_ARGUMENT, MIGO_ERROR_UNSUPPORTED_ABI,
    VersionedHeader, copy_utf8, validate_header,
};

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
    let source = CString::new("/tmp/migo").expect("cstring");
    let copied = unsafe { copy_utf8(source.as_ptr()) }.expect("copy");
    drop(source);
    assert_eq!(copied, "/tmp/migo");
}
