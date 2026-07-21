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

// A library-written struct that has grown: `appended` was added after the v1
// shape ended at `second`. None of the real output structs have grown yet, so
// this stands in for the first one that does -- the mirror image of the input
// side's `MigoHostCallbacks`, whose `MINIMUM_SIZE` records the shape before its
// own appended fields.
#[repr(C)]
#[derive(Clone, Copy)]
struct GrownOutput {
    header: VersionedHeader,
    first: u32,
    second: u32,
    appended: u64,
}

// The shape ended at `second` before `appended` was appended: header(8) +
// first(4) + second(4).
unsafe impl AbiStruct for GrownOutput {
    const MINIMUM_SIZE: usize = 16;
}

#[test]
fn an_old_client_gets_its_prefix_and_its_absent_appended_field_is_left_untouched() {
    // A host compiled against the header before `appended` existed passes a
    // 16-byte struct. The library is newer and its record is 24 bytes.
    const OLD_SIZE: u32 = 16;
    assert!(
        size_of::<GrownOutput>() > OLD_SIZE as usize,
        "the struct must have grown for this test to mean anything"
    );

    let mut storage = guarded(OLD_SIZE, MIGO_ABI_VERSION_CURRENT);
    let value = GrownOutput {
        header: VersionedHeader {
            struct_size: size_of::<GrownOutput>() as u32,
            abi_version: MIGO_ABI_VERSION_CURRENT,
        },
        first: 0xAABB_CCDD,
        second: 0x1122_3344,
        // A sentinel the old client has no field for; it must never appear in
        // the old client's buffer.
        appended: 0xDEAD_BEEF_F00D_CAFE,
    };

    let result = unsafe {
        write_versioned_output(
            storage.0.as_mut_ptr().cast::<GrownOutput>(),
            &value,
            OutputVersionPolicy::CapabilityNegotiation,
        )
    };

    assert_eq!(result, MIGO_OK);

    // The header is the caller's, unchanged: it still describes a 16-byte record.
    let header = unsafe { std::ptr::read(storage.0.as_ptr().cast::<VersionedHeader>()) };
    assert_eq!(header.struct_size, OLD_SIZE);
    assert_eq!(header.abi_version, MIGO_ABI_VERSION_CURRENT);

    // The prefix the old client does have was written.
    let first = u32::from_ne_bytes(storage.0[8..12].try_into().unwrap());
    let second = u32::from_ne_bytes(storage.0[12..16].try_into().unwrap());
    assert_eq!(first, 0xAABB_CCDD);
    assert_eq!(second, 0x1122_3344);

    // The appended field the old client never had must be untouched -- writing
    // it would run off the end of the buffer that host actually allocated.
    assert_eq!(
        &storage.0[16..24],
        &[0xA5; 8],
        "the library wrote past the old client's struct_size into a field it does not have"
    );
}
