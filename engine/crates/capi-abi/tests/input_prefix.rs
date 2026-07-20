use std::mem::size_of;

use migo_capi_abi::{
    AbiStruct, MIGO_ABI_VERSION_CURRENT, MIGO_ERROR_INVALID_ARGUMENT, MIGO_ERROR_UNSUPPORTED_ABI,
    VersionedHeader, copy_utf8, copy_versioned, validate_header,
};

#[repr(C)]
#[derive(Debug, Eq, PartialEq)]
struct TestInput {
    header: VersionedHeader,
    required: u32,
    optional: u32,
}

unsafe impl AbiStruct for TestInput {
    const MINIMUM_SIZE: usize = 12;
}

#[repr(C, align(8))]
struct CallerBytes([u8; 24]);

fn caller_bytes(size: u32, abi: u32, required: u32) -> CallerBytes {
    let mut storage = CallerBytes([0xA5; 24]);
    unsafe {
        std::ptr::write(
            storage.0.as_mut_ptr().cast::<VersionedHeader>(),
            VersionedHeader {
                struct_size: size,
                abi_version: abi,
            },
        );
        std::ptr::write(storage.0.as_mut_ptr().add(8).cast::<u32>(), required);
    }
    storage
}

#[test]
fn an_old_short_input_is_zero_extended_without_reading_the_tail() {
    let storage = caller_bytes(12, MIGO_ABI_VERSION_CURRENT, 7);
    let copied =
        unsafe { copy_versioned::<TestInput>(storage.0.as_ptr().cast()) }.expect("old prefix");

    assert_eq!(copied.required, 7);
    assert_eq!(copied.optional, 0);
}

#[test]
fn a_newer_larger_input_is_rejected() {
    let storage = caller_bytes(24, MIGO_ABI_VERSION_CURRENT, 7);

    assert_eq!(
        unsafe { copy_versioned::<TestInput>(storage.0.as_ptr().cast()) }.unwrap_err(),
        MIGO_ERROR_UNSUPPORTED_ABI,
    );
}

#[test]
fn copy_rejects_null_wrong_abi_and_below_minimum_inputs() {
    assert_eq!(
        unsafe { copy_versioned::<TestInput>(std::ptr::null()) }.unwrap_err(),
        MIGO_ERROR_INVALID_ARGUMENT,
    );

    let wrong_abi = caller_bytes(12, MIGO_ABI_VERSION_CURRENT + 1, 7);
    assert_eq!(
        unsafe { copy_versioned::<TestInput>(wrong_abi.0.as_ptr().cast()) }.unwrap_err(),
        MIGO_ERROR_UNSUPPORTED_ABI,
    );

    let too_short = caller_bytes(11, MIGO_ABI_VERSION_CURRENT, 7);
    assert_eq!(
        unsafe { copy_versioned::<TestInput>(too_short.0.as_ptr().cast()) }.unwrap_err(),
        MIGO_ERROR_INVALID_ARGUMENT,
    );
}

#[test]
fn exact_header_validation_classifies_size_and_version_failures() {
    let exact = caller_bytes(size_of::<TestInput>() as u32, MIGO_ABI_VERSION_CURRENT, 7);
    assert_eq!(
        unsafe { validate_header(exact.0.as_ptr().cast(), size_of::<TestInput>()) },
        Ok(()),
    );

    let short = caller_bytes(12, MIGO_ABI_VERSION_CURRENT, 7);
    assert_eq!(
        unsafe { validate_header(short.0.as_ptr().cast(), size_of::<TestInput>()) },
        Err(MIGO_ERROR_INVALID_ARGUMENT),
    );

    let long = caller_bytes(24, MIGO_ABI_VERSION_CURRENT, 7);
    assert_eq!(
        unsafe { validate_header(long.0.as_ptr().cast(), size_of::<TestInput>()) },
        Err(MIGO_ERROR_UNSUPPORTED_ABI),
    );

    let wrong_abi = caller_bytes(
        size_of::<TestInput>() as u32,
        MIGO_ABI_VERSION_CURRENT + 1,
        7,
    );
    assert_eq!(
        unsafe { validate_header(wrong_abi.0.as_ptr().cast(), size_of::<TestInput>()) },
        Err(MIGO_ERROR_UNSUPPORTED_ABI),
    );
    assert_eq!(
        unsafe { validate_header(std::ptr::null(), size_of::<TestInput>()) },
        Err(MIGO_ERROR_INVALID_ARGUMENT),
    );
}

#[test]
fn utf8_copy_owns_valid_text_and_rejects_null_or_invalid_bytes() {
    let valid = b"migo\0";
    assert_eq!(
        unsafe { copy_utf8(valid.as_ptr().cast()) }.as_deref(),
        Ok("migo"),
    );

    let invalid = b"\xFF\0";
    assert_eq!(
        unsafe { copy_utf8(invalid.as_ptr().cast()) },
        Err(MIGO_ERROR_INVALID_ARGUMENT),
    );
    assert_eq!(
        unsafe { copy_utf8(std::ptr::null()) },
        Err(MIGO_ERROR_INVALID_ARGUMENT),
    );
}
