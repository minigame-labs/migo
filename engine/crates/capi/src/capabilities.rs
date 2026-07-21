//! What this library supports, asked of the library itself.
//!
//! Everything else in the ABI describes what a host wants to do. This describes
//! what the linked library can do, which a host cannot otherwise find out: the
//! headers' `MIGO_C_ABI_HAS_RUNTIME` is a preprocessor macro that reports the
//! platform the host compiled on, and the attachable surface kinds were only
//! discoverable by attempting an attach — after building an engine, a session
//! and a window.

use crate::panic_barrier::guard;
use crate::platform::supported_platform_kinds;
use migo_capi_abi::MigoResult;

pub use migo_capi_abi::config::MigoCapabilities;

/// The oldest ABI version this library still accepts on an entry point.
const MIGO_ABI_VERSION_MIN: u32 = migo_capi_abi::MIGO_ABI_VERSION_CURRENT;
/// The newest it implements.
const MIGO_ABI_VERSION_MAX: u32 = migo_capi_abi::MIGO_ABI_VERSION_CURRENT;

/// Report what this library supports.
///
/// # The version-negotiation exception
///
/// Every other entry point rejects an `abi_version` it does not recognise. This
/// one must not: a caller asking which versions exist has, by definition, not
/// established that yet, and rejecting the question would leave it with no way
/// to find the answer. So `abi_version` is accepted as-is and the supported
/// range is reported back. `struct_size` is still validated, because it decides
/// how many bytes may be written into the caller's storage.
///
/// # Safety
/// `out` must point at writable storage of at least `out->struct_size` bytes,
/// with `struct_size` set by the caller before the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn migo_query_capabilities(out: *mut MigoCapabilities) -> MigoResult {
    guard("migo_query_capabilities", || unsafe {
        migo_capi_abi::config::write_capabilities(
            out.cast::<u8>(),
            MIGO_ABI_VERSION_MIN,
            MIGO_ABI_VERSION_MAX,
            supported_platform_kinds(),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use migo_capi_abi::{
        MIGO_ABI_VERSION_CURRENT, MIGO_ERROR_INVALID_ARGUMENT, MIGO_OK, VersionedHeader,
    };

    fn request() -> MigoCapabilities {
        MigoCapabilities {
            header: VersionedHeader {
                struct_size: size_of::<MigoCapabilities>() as u32,
                abi_version: MIGO_ABI_VERSION_CURRENT,
            },
            abi_version_min: 0,
            abi_version_max: 0,
            platform_kinds: 0,
        }
    }

    #[test]
    fn a_well_formed_query_reports_the_supported_range_and_kinds() {
        let mut caps = request();
        assert_eq!(unsafe { migo_query_capabilities(&mut caps) }, MIGO_OK);
        assert_eq!(caps.abi_version_min, MIGO_ABI_VERSION_CURRENT);
        assert_eq!(caps.abi_version_max, MIGO_ABI_VERSION_CURRENT);
        if cfg!(any(
            target_os = "android",
            all(target_os = "linux", not(target_env = "ohos"))
        )) {
            assert_ne!(
                caps.platform_kinds, 0,
                "a build with a runtime must be able to attach something"
            );
        } else {
            assert_eq!(
                caps.platform_kinds, 0,
                "a compile-only target must not advertise another OS's windows"
            );
        }
    }

    /// The documented exception, asserted so it cannot be "tidied up" into
    /// consistency with the other entry points.
    ///
    /// A caller that does not yet know which ABI version to declare is exactly
    /// the caller this query exists for. Rejecting it would make the one call
    /// that resolves version disagreement unusable during a version
    /// disagreement.
    #[test]
    fn an_unrecognised_abi_version_is_answered_not_rejected() {
        let mut caps = request();
        caps.header.abi_version = 9_999;
        assert_eq!(unsafe { migo_query_capabilities(&mut caps) }, MIGO_OK);
        assert_eq!(caps.abi_version_max, MIGO_ABI_VERSION_CURRENT);
    }

    #[test]
    fn an_unknown_struct_size_is_rejected_without_writing_anything() {
        let mut caps = request();
        caps.header.struct_size = 8;
        assert_eq!(
            unsafe { migo_query_capabilities(&mut caps) },
            MIGO_ERROR_INVALID_ARGUMENT
        );
        // Untouched: writing a partial answer into storage the library does not
        // understand the shape of is how a query corrupts its caller.
        assert_eq!(caps.abi_version_min, 0);
        assert_eq!(caps.abi_version_max, 0);
        assert_eq!(caps.platform_kinds, 0);
    }

    #[test]
    fn a_null_destination_is_an_argument_error_not_a_crash() {
        assert_eq!(
            unsafe { migo_query_capabilities(std::ptr::null_mut()) },
            MIGO_ERROR_INVALID_ARGUMENT
        );
    }

    /// The mask and the attach path are checked against each other rather than
    /// against a literal: drift is the failure this pair exists to catch, and a
    /// literal on both sides would drift together.
    #[test]
    fn every_advertised_kind_is_one_the_attach_path_accepts() {
        let mask = supported_platform_kinds();
        for kind in 0..u64::BITS {
            let advertised = mask & (1u64 << kind) != 0;
            assert_eq!(
                advertised,
                crate::platform::kind_is_supported(kind),
                "kind {kind}: advertised={advertised} but the attach path disagrees"
            );
        }
    }
}
