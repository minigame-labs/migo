use std::mem::size_of;

use migo_capi_abi::{
    AbiStruct, MIGO_ABI_VERSION_CURRENT, MIGO_ERROR_INVALID_ARGUMENT, MIGO_OK, OutputVersionPolicy,
    VersionedHeader, write_versioned_output,
};

#[repr(C)]
#[derive(Clone, Copy)]
struct TestOutput {
    header: VersionedHeader,
    first: u32,
    second: u32,
    bits: u64,
}

unsafe impl AbiStruct for TestOutput {}

#[repr(C, align(8))]
struct Guarded([u8; 32]);

fn guarded(size: u32, abi: u32) -> Guarded {
    let mut storage = Guarded([0xA5; 32]);
    let header = VersionedHeader {
        struct_size: size,
        abi_version: abi,
    };
    unsafe {
        std::ptr::write(storage.0.as_mut_ptr().cast::<VersionedHeader>(), header);
    }
    storage
}

#[test]
fn negotiation_writes_only_the_known_prefix_and_preserves_the_caller_header() {
    let mut storage = guarded(32, 9_999);
    let value = TestOutput {
        header: VersionedHeader {
            struct_size: size_of::<TestOutput>() as u32,
            abi_version: MIGO_ABI_VERSION_CURRENT,
        },
        first: 11,
        second: 22,
        bits: 0x1122_3344_5566_7788,
    };

    let result = unsafe {
        write_versioned_output(
            storage.0.as_mut_ptr().cast::<TestOutput>(),
            &value,
            OutputVersionPolicy::CapabilityNegotiation,
        )
    };

    assert_eq!(result, MIGO_OK);
    let header = unsafe { std::ptr::read(storage.0.as_ptr().cast::<VersionedHeader>()) };
    assert_eq!(header.struct_size, 32);
    assert_eq!(header.abi_version, 9_999);
    assert_eq!(&storage.0[24..], &[0xA5; 8]);
}

#[test]
fn undersized_output_is_rejected_without_writing() {
    let mut storage = guarded(16, MIGO_ABI_VERSION_CURRENT);
    let before = storage.0;
    let value = TestOutput {
        header: VersionedHeader {
            struct_size: size_of::<TestOutput>() as u32,
            abi_version: MIGO_ABI_VERSION_CURRENT,
        },
        first: 11,
        second: 22,
        bits: 33,
    };

    let result = unsafe {
        write_versioned_output(
            storage.0.as_mut_ptr().cast::<TestOutput>(),
            &value,
            OutputVersionPolicy::CapabilityNegotiation,
        )
    };

    assert_eq!(result, MIGO_ERROR_INVALID_ARGUMENT);
    assert_eq!(storage.0, before);
}
