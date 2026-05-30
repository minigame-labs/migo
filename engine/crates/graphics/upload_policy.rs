//! Centralised upload-path policy selection.
//!
//! Previously, the "which upload path do we use" decision was
//! scattered across three files:
//!
//! * `canvas/manager/image.rs` - AHB zero-copy vs RGBA download
//!   (`load_ahb_image` guard on `device_caps.ahb_available` and
//!   `egl_display_ptr.is_null()`).
//! * `canvas/manager/pbo_upload.rs::upload_texture_tiered` - PBO
//!   vs direct `glTexSubImage2D` (guards on PBO support + image
//!   size tiering).
//! * `compressed_upload.rs` - ETC2/ASTC compressed vs RGBA
//!   (guards on `CompressedFormatSupport::is_supported`).
//!
//! Each call site duplicated a subset of the "is this supported?"
//! logic.  This module answers that question once, given a
//! `DecodedImage` and `DeviceCapabilities`, and hands back a
//! [`UploadStrategy`] the caller can dispatch on.
//!
//! # Why a policy module and not a trait?
//!
//! The caller already owns `&glow::Context`, `&DeviceCaps`, and
//! the decoded source; a trait would force every upload impl to
//! accept the same signatures.  A plain enum keeps the GL layer
//! free to evolve upload signatures independently while the
//! *selection* logic stays uniform.

use shared::protocol::io_cmd::{AhbImage, DecodedImage};

use crate::compressed_upload::{CompressedFormat, CompressedFormatSupport};
use crate::device_caps::DeviceCapabilities;

/// Upload strategy selected by [`select`] for a given source.
///
/// The caller inspects the variant and dispatches to the matching
/// uploader.  `Fallback` is returned when no specialised path
/// applies; call sites should drop through to a plain RGBA upload
/// via `pbo_upload::upload_texture_tiered`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UploadStrategy {
    /// Android zero-copy GPU import.  Caller binds the AHB via
    /// `eglCreateImageKHR` + `glEGLImageTargetTexture2DOES`.
    AndroidHardwareBuffer,

    /// GPU-direct compressed upload (ETC2 / ASTC).  Caller calls
    /// `compressed_upload::upload_compressed_texture` with the
    /// selected format.
    CompressedGpu(CompressedFormat),

    /// RGBA PBO upload.  Caller uses
    /// `pbo_upload::upload_texture_tiered` with `use_pbo = true`.
    /// Preferred when the source fits the PBO pool and the
    /// driver advertises PBO support.
    RgbaPbo,

    /// Direct `glTexSubImage2D` - the legacy path.  Chosen when
    /// PBO is unavailable or the driver's PBO pool is exhausted.
    RgbaDirect,

    /// No specialised path applies.  Caller should fall back to
    /// `pbo_upload::upload_texture_tiered` which runs its own
    /// internal PBO-vs-direct tiering.
    Fallback,
}

/// Inputs the selector considers when choosing a strategy.
///
/// Deliberately narrow - anything wider (e.g. full `&DecodedImage`)
/// would pull render-thread types into callers that only want to
/// *query* the strategy without owning the decoded bytes yet.
#[derive(Debug, Clone, Copy)]
pub struct UploadInputs {
    /// Pixel width of the source image.
    pub width: u32,
    /// Pixel height.
    pub height: u32,
    /// `true` when the decoder handed us an AHB
    /// ([`DecodedImage::HardwareBuffer`]).
    pub is_ahb: bool,
    /// Detected compressed format of the source, or `None` when
    /// the source is plain RGBA.
    pub compressed_format: Option<CompressedFormat>,
    /// Whether an EGL display pointer is available for the AHB
    /// import path.
    pub egl_display_present: bool,
    /// Whether the render thread's PBO pool is ready to serve an
    /// upload of this size.
    pub pbo_available: bool,
}

impl UploadInputs {
    /// Convenience builder for the common "normalised image" case.
    pub fn from_rgba_dims(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            is_ahb: false,
            compressed_format: None,
            egl_display_present: false,
            pbo_available: true,
        }
    }

    /// Convenience builder for an AHB source.
    pub fn from_ahb(ahb: &AhbImage, egl_display_present: bool) -> Self {
        Self {
            width: ahb.width,
            height: ahb.height,
            is_ahb: true,
            compressed_format: None,
            egl_display_present,
            pbo_available: false,
        }
    }

    /// Derive upload inputs directly from a `DecodedImage` plus the
    /// ambient EGL/PBO state.  Sets `compressed_format` when the
    /// decoded payload is a recognised compressed container.
    pub fn from_decoded(
        decoded: &DecodedImage,
        egl_display_present: bool,
        pbo_available: bool,
    ) -> Self {
        match decoded {
            DecodedImage::HardwareBuffer(ahb) => Self::from_ahb(ahb, egl_display_present),
            DecodedImage::Rgba(img) => Self {
                width: img.width,
                height: img.height,
                is_ahb: false,
                compressed_format: None,
                egl_display_present,
                pbo_available,
            },
            DecodedImage::Compressed(c) => Self {
                width: c.width,
                height: c.height,
                is_ahb: false,
                compressed_format: CompressedFormat::from_vk_format(c.vk_format),
                egl_display_present,
                pbo_available,
            },
        }
    }
}

/// Pick an [`UploadStrategy`] for the given source under the
/// given device capabilities and the supported compressed
/// formats.
///
/// Decision order (matches the production preference):
/// 1. AHB zero-copy when both device and EGL permit.
/// 2. Compressed GPU upload when the format is supported.
/// 3. PBO-backed RGBA when the pool is ready.
/// 4. Direct `glTexSubImage2D` as the always-works fallback.
/// 5. `Fallback` when none of the above applies (exotic edge
///    cases - caller dispatches to `upload_texture_tiered`).
pub fn select(
    inputs: &UploadInputs,
    caps: &DeviceCapabilities,
    compressed_caps: &CompressedFormatSupport,
) -> UploadStrategy {
    if inputs.is_ahb && caps.ahb_available && inputs.egl_display_present {
        return UploadStrategy::AndroidHardwareBuffer;
    }
    if let Some(fmt) = inputs.compressed_format {
        if compressed_caps.is_supported(fmt) {
            return UploadStrategy::CompressedGpu(fmt);
        }
    }
    if inputs.pbo_available {
        return UploadStrategy::RgbaPbo;
    }
    UploadStrategy::RgbaDirect
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caps(ahb: bool) -> DeviceCapabilities {
        DeviceCapabilities {
            gles_version: (3, 0),
            has_pbo: true,
            has_fence_sync: true,
            has_compute: false,
            ahb_available: ahb,
            has_buffer_age: false,
            has_partial_update: false,
            compressed_format_support: CompressedFormatSupport {
                etc2: false,
                astc: false,
            },
        }
    }

    fn comp(etc2: bool, astc: bool) -> CompressedFormatSupport {
        CompressedFormatSupport { etc2, astc }
    }

    #[test]
    fn ahb_chosen_when_supported() {
        let inputs = UploadInputs {
            width: 512,
            height: 512,
            is_ahb: true,
            compressed_format: None,
            egl_display_present: true,
            pbo_available: true,
        };
        assert_eq!(
            select(&inputs, &caps(true), &comp(false, false)),
            UploadStrategy::AndroidHardwareBuffer
        );
    }

    #[test]
    fn ahb_falls_back_without_display() {
        let inputs = UploadInputs {
            width: 512,
            height: 512,
            is_ahb: true,
            compressed_format: None,
            egl_display_present: false,
            pbo_available: true,
        };
        assert_eq!(
            select(&inputs, &caps(true), &comp(false, false)),
            UploadStrategy::RgbaPbo
        );
    }

    #[test]
    fn compressed_chosen_when_supported() {
        let inputs = UploadInputs {
            width: 256,
            height: 256,
            is_ahb: false,
            compressed_format: Some(CompressedFormat::Astc4x4),
            egl_display_present: true,
            pbo_available: true,
        };
        assert_eq!(
            select(&inputs, &caps(false), &comp(false, true)),
            UploadStrategy::CompressedGpu(CompressedFormat::Astc4x4)
        );
    }

    #[test]
    fn compressed_falls_through_when_unsupported() {
        let inputs = UploadInputs {
            width: 256,
            height: 256,
            is_ahb: false,
            compressed_format: Some(CompressedFormat::Astc4x4),
            egl_display_present: true,
            pbo_available: true,
        };
        // astc=false -> skip compressed -> pbo
        assert_eq!(
            select(&inputs, &caps(false), &comp(true, false)),
            UploadStrategy::RgbaPbo
        );
    }

    #[test]
    fn direct_path_when_pbo_unavailable() {
        let inputs = UploadInputs {
            width: 128,
            height: 128,
            is_ahb: false,
            compressed_format: None,
            egl_display_present: false,
            pbo_available: false,
        };
        assert_eq!(
            select(&inputs, &caps(false), &comp(false, false)),
            UploadStrategy::RgbaDirect
        );
    }
}
