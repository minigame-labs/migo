//! Honest boundary for targets whose native presenter has not landed yet.
//!
//! Public headers remain useful as a compile-only contract on these targets,
//! but a locally built library must neither advertise Linux window kinds nor
//! attempt to load Linux EGL. Each future platform replaces this module with a
//! target-specific presenter and descriptor implementation.

use migo_capi_abi::surface::SurfaceDescriptorRef;
use shared::surface::SurfaceRef;

use migo_capi_abi::{MIGO_ERROR_UNSUPPORTED_PLATFORM, MigoResult};

#[derive(Clone, Copy)]
pub(crate) enum PlatformTarget {
    #[cfg(test)]
    TestOnly,
}

#[derive(Clone, Debug)]
pub(crate) enum PlatformContext {
    #[cfg(test)]
    TestOnly,
}

pub(crate) const fn supported_platform_kinds() -> u64 {
    0
}

pub(crate) fn rebuild_surface(
    target: PlatformTarget,
    _width: u32,
    _height: u32,
) -> Result<SurfaceRef, MigoResult> {
    match target {
        #[cfg(test)]
        PlatformTarget::TestOnly => Err(MIGO_ERROR_UNSUPPORTED_PLATFORM),
    }
}

pub(crate) fn build_target(
    _descriptor: SurfaceDescriptorRef,
    _existing: Option<&PlatformContext>,
) -> Result<
    (
        SurfaceRef,
        graphics::egl_platform::GraphicsPlatform,
        PlatformTarget,
        PlatformContext,
    ),
    MigoResult,
> {
    Err(MIGO_ERROR_UNSUPPORTED_PLATFORM)
}

#[cfg(test)]
pub(crate) fn test_platform_target() -> PlatformTarget {
    PlatformTarget::TestOnly
}

#[cfg(test)]
pub(crate) fn test_platform_context() -> PlatformContext {
    PlatformContext::TestOnly
}
