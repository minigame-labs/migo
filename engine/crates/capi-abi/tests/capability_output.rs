use migo_capi_abi::{
    MIGO_ABI_VERSION_CURRENT, MIGO_ERROR_INVALID_ARGUMENT, MIGO_OK, config::write_capabilities,
};

#[repr(C, align(8))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct GuardedCapabilities([u8; 32]);

fn guarded_capabilities(size: u32, abi: u32, fill: u8) -> GuardedCapabilities {
    let mut output = GuardedCapabilities([fill; 32]);
    output.0[..4].copy_from_slice(&size.to_ne_bytes());
    output.0[4..8].copy_from_slice(&abi.to_ne_bytes());
    output
}

fn read_u32(output: &GuardedCapabilities, offset: usize) -> u32 {
    u32::from_ne_bytes(output.0[offset..offset + 4].try_into().unwrap())
}

fn read_u64(output: &GuardedCapabilities, offset: usize) -> u64 {
    u64::from_ne_bytes(output.0[offset..offset + 8].try_into().unwrap())
}

#[test]
fn capability_query_accepts_a_newer_caller_and_leaves_unknown_tail_untouched() {
    let mut output = guarded_capabilities(32, 99, 0xA5);

    assert_eq!(
        unsafe {
            write_capabilities(
                output.0.as_mut_ptr(),
                MIGO_ABI_VERSION_CURRENT,
                MIGO_ABI_VERSION_CURRENT,
                0xC0,
            )
        },
        MIGO_OK,
    );
    assert_eq!(read_u32(&output, 0), 32);
    assert_eq!(read_u32(&output, 4), 99);
    assert_eq!(read_u32(&output, 8), MIGO_ABI_VERSION_CURRENT);
    assert_eq!(read_u32(&output, 12), MIGO_ABI_VERSION_CURRENT);
    assert_eq!(read_u64(&output, 16), 0xC0);
    assert_eq!(&output.0[24..32], &[0xA5; 8]);
}

#[test]
fn capability_query_rejects_less_than_the_v1_prefix_without_writing() {
    let mut output = guarded_capabilities(16, 1, 0xA5);
    let before = output;

    assert_eq!(
        unsafe { write_capabilities(output.0.as_mut_ptr(), 1, 1, 1) },
        MIGO_ERROR_INVALID_ARGUMENT,
    );
    assert_eq!(output, before);
}

#[test]
fn capability_query_rejects_null_without_writing() {
    assert_eq!(
        unsafe { write_capabilities(std::ptr::null_mut(), 1, 1, 1) },
        MIGO_ERROR_INVALID_ARGUMENT,
    );
}
