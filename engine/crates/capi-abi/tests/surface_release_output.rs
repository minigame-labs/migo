use std::mem::{offset_of, size_of};

use migo_capi_abi::{
    MIGO_ABI_VERSION_CURRENT, MIGO_ERROR_INVALID_ARGUMENT, MIGO_ERROR_UNSUPPORTED_ABI, MIGO_OK,
    VersionedHeader,
    surface::{
        MIGO_SURFACE_RELEASE_PENDING, MIGO_SURFACE_RELEASE_RELEASED, MigoSurfaceReleaseStatus,
        write_surface_release_status,
    },
};

fn status_output(size: u32, abi_version: u32) -> MigoSurfaceReleaseStatus {
    MigoSurfaceReleaseStatus {
        header: VersionedHeader {
            struct_size: size,
            abi_version,
        },
        generation: u64::MAX,
        state: u32::MAX,
        reserved0: u32::MAX,
    }
}

#[test]
fn release_status_has_a_fixed_c_layout() {
    assert_eq!(size_of::<MigoSurfaceReleaseStatus>(), 24);
    assert_eq!(offset_of!(MigoSurfaceReleaseStatus, generation), 8);
    assert_eq!(offset_of!(MigoSurfaceReleaseStatus, state), 16);
}

#[test]
fn release_query_writes_the_authoritative_level_and_preserves_the_header() {
    let mut out = status_output(24, MIGO_ABI_VERSION_CURRENT);

    assert_eq!(
        unsafe { write_surface_release_status(&mut out, 41, MIGO_SURFACE_RELEASE_PENDING) },
        MIGO_OK,
    );
    assert_eq!(out.header.struct_size, 24);
    assert_eq!(out.header.abi_version, MIGO_ABI_VERSION_CURRENT);
    assert_eq!(out.generation, 41);
    assert_eq!(out.state, MIGO_SURFACE_RELEASE_PENDING);
    assert_eq!(out.reserved0, 0);

    assert_eq!(
        unsafe { write_surface_release_status(&mut out, 41, MIGO_SURFACE_RELEASE_RELEASED) },
        MIGO_OK,
    );
    assert_eq!(out.state, MIGO_SURFACE_RELEASE_RELEASED);
}

#[test]
fn release_query_rejects_invalid_output_without_partial_writes() {
    let mut short = status_output(16, MIGO_ABI_VERSION_CURRENT);
    let before = short;
    assert_eq!(
        unsafe { write_surface_release_status(&mut short, 7, MIGO_SURFACE_RELEASE_PENDING) },
        MIGO_ERROR_INVALID_ARGUMENT,
    );
    assert_eq!(short.header, before.header);
    assert_eq!(short.generation, before.generation);
    assert_eq!(short.state, before.state);
    assert_eq!(short.reserved0, before.reserved0);

    let mut future = status_output(32, MIGO_ABI_VERSION_CURRENT);
    let before = future;
    assert_eq!(
        unsafe { write_surface_release_status(&mut future, 7, MIGO_SURFACE_RELEASE_PENDING) },
        MIGO_ERROR_UNSUPPORTED_ABI,
    );
    assert_eq!(future.header, before.header);
    assert_eq!(future.generation, before.generation);
    assert_eq!(future.state, before.state);
    assert_eq!(future.reserved0, before.reserved0);
}

#[test]
fn release_query_rejects_zero_generation_and_unknown_state() {
    let mut out = status_output(24, MIGO_ABI_VERSION_CURRENT);
    assert_eq!(
        unsafe { write_surface_release_status(&mut out, 0, MIGO_SURFACE_RELEASE_PENDING) },
        MIGO_ERROR_INVALID_ARGUMENT,
    );
    assert_eq!(
        unsafe { write_surface_release_status(&mut out, 1, 99) },
        MIGO_ERROR_INVALID_ARGUMENT,
    );
}
