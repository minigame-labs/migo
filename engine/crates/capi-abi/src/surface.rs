//! Platform-neutral validation for public surface descriptors.
//!
//! This module deliberately stops before any native API call. It copies the
//! complete v1 envelope and matching platform payload into Rust-owned values,
//! validates every semantic field, and only then exposes typed native handles
//! to the platform adapter.

use std::{
    ffi::c_void,
    mem::{offset_of, size_of},
    ptr::NonNull,
};

use crate::{
    AbiStruct, MIGO_ERROR_INVALID_ARGUMENT, MIGO_ERROR_UNSUPPORTED_CAPABILITY,
    MIGO_ERROR_UNSUPPORTED_PLATFORM, MIGO_OK, MigoResult, OutputVersionPolicy, VersionedHeader,
    copy_versioned,
    validate::{validate_flags, validate_positive_scale, validate_reserved},
    write_versioned_output,
};

pub const MIGO_PLATFORM_UNKNOWN: u32 = 0;
pub const MIGO_PLATFORM_ANDROID_NATIVE_WINDOW: u32 = 1;
pub const MIGO_PLATFORM_WIN32_HWND: u32 = 2;
pub const MIGO_PLATFORM_WINUI_SWAP_CHAIN_PANEL: u32 = 3;
pub const MIGO_PLATFORM_MACOS_NS_VIEW: u32 = 4;
pub const MIGO_PLATFORM_MACOS_CA_METAL_LAYER: u32 = 5;
pub const MIGO_PLATFORM_X11_WINDOW: u32 = 6;
pub const MIGO_PLATFORM_WAYLAND_SURFACE: u32 = 7;
pub const MIGO_PLATFORM_OPENHARMONY_NATIVE_WINDOW: u32 = 8;
pub const MIGO_PLATFORM_IOS_UI_VIEW: u32 = 9;
pub const MIGO_PLATFORM_IOS_CA_METAL_LAYER: u32 = 10;

pub const MIGO_COLOR_SPACE_UNSPECIFIED: u32 = 0;
pub const MIGO_COLOR_SPACE_SRGB: u32 = 1;
pub const MIGO_COLOR_SPACE_DISPLAY_P3: u32 = 2;
pub const MIGO_COLOR_SPACE_EXTENDED_SRGB: u32 = 3;

pub const MIGO_ALPHA_MODE_UNSPECIFIED: u32 = 0;
pub const MIGO_ALPHA_MODE_OPAQUE: u32 = 1;
pub const MIGO_ALPHA_MODE_PREMULTIPLIED: u32 = 2;
pub const MIGO_ALPHA_MODE_POSTMULTIPLIED: u32 = 3;

pub const MIGO_PRESENTATION_MODE_DEFAULT: u32 = 0;
pub const MIGO_PRESENTATION_MODE_FIFO: u32 = 1;
pub const MIGO_PRESENTATION_MODE_MAILBOX: u32 = 2;
pub const MIGO_PRESENTATION_MODE_IMMEDIATE: u32 = 3;

pub const MIGO_SURFACE_CAPABILITY_WIDE_COLOR: u64 = 1 << 0;
pub const MIGO_SURFACE_CAPABILITY_TRANSPARENT: u64 = 1 << 1;
pub const MIGO_SURFACE_CAPABILITY_MAILBOX_PRESENT: u64 = 1 << 2;

pub const MIGO_SURFACE_LOSS_UNKNOWN: u32 = 0;
pub const MIGO_SURFACE_LOSS_HOST_DESTROYED: u32 = 1;
pub const MIGO_SURFACE_LOSS_DEVICE_LOST: u32 = 2;
pub const MIGO_SURFACE_LOSS_PLATFORM_ERROR: u32 = 3;
const MIGO_SURFACE_CAPABILITY_KNOWN_MASK: u64 = MIGO_SURFACE_CAPABILITY_WIDE_COLOR
    | MIGO_SURFACE_CAPABILITY_TRANSPARENT
    | MIGO_SURFACE_CAPABILITY_MAILBOX_PRESENT;

pub const MIGO_CAPI_IMPLEMENTED_PLATFORM_KINDS: u64 = (1 << MIGO_PLATFORM_ANDROID_NATIVE_WINDOW)
    | (1 << MIGO_PLATFORM_WIN32_HWND)
    | (1 << MIGO_PLATFORM_X11_WINDOW)
    | (1 << MIGO_PLATFORM_WAYLAND_SURFACE)
    // Added only after an attach succeeded on a device, not when the backend
    // compiled. The Windows incident is why: a published SDK loaded, resolved
    // every entry point, and could attach nothing, because every gate agreed
    // with every other gate while no implementation existed. The evidence here
    // is an OpenHarmony emulator log reading "surface attached, generation 1"
    // after "surface created 1316 x 2598".
    | (1 << MIGO_PLATFORM_OPENHARMONY_NATIVE_WINDOW);

/// Authoritative, level-triggered state returned by a release query.
pub const MIGO_SURFACE_RELEASE_PENDING: u32 = 0;
pub const MIGO_SURFACE_RELEASE_RELEASED: u32 = 1;

/// Validate a new attachment's public generation against the Session's
/// monotonic high-water mark.
#[inline]
pub fn validate_attach_generation(candidate: u64, last_accepted: u64) -> Result<u64, MigoResult> {
    if candidate == 0 {
        return Err(MIGO_ERROR_INVALID_ARGUMENT);
    }
    if candidate <= last_accepted {
        return Err(crate::MIGO_ERROR_STALE_SURFACE);
    }
    Ok(candidate)
}

/// Validate that a metrics update targets exactly the active attachment.
#[inline]
pub fn validate_update_generation(candidate: u64, active: u64) -> Result<(), MigoResult> {
    if candidate == 0 || active == 0 || candidate > active {
        return Err(MIGO_ERROR_INVALID_ARGUMENT);
    }
    if candidate < active {
        return Err(crate::MIGO_ERROR_STALE_SURFACE);
    }
    Ok(())
}

/// Public snapshot of one asynchronous native-Surface release.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MigoSurfaceReleaseStatus {
    pub header: VersionedHeader,
    pub generation: u64,
    pub state: u32,
    pub reserved0: u32,
}

// SAFETY: this is a fixed-width C record with an all-zero-valid bit pattern.
// ABI v1 requires the complete record so callers cannot omit the level state.
unsafe impl AbiStruct for MigoSurfaceReleaseStatus {}

/// Write one release-state snapshot without exposing Rust layout or padding.
///
/// # Safety
/// `out` must satisfy [`write_versioned_output`]'s output contract.
pub unsafe fn write_surface_release_status(
    out: *mut MigoSurfaceReleaseStatus,
    generation: u64,
    state: u32,
) -> MigoResult {
    if generation == 0
        || !matches!(
            state,
            MIGO_SURFACE_RELEASE_PENDING | MIGO_SURFACE_RELEASE_RELEASED
        )
    {
        return MIGO_ERROR_INVALID_ARGUMENT;
    }

    let value = MigoSurfaceReleaseStatus {
        header: VersionedHeader {
            struct_size: size_of::<MigoSurfaceReleaseStatus>() as u32,
            abi_version: crate::MIGO_ABI_VERSION_CURRENT,
        },
        generation,
        state,
        reserved0: 0,
    };
    // SAFETY: forwarded from this function's contract; `value` is a distinct
    // local, fully initialized ABI record.
    let result = unsafe { write_versioned_output(out, &value, OutputVersionPolicy::CurrentAbi) };
    debug_assert!(result != MIGO_OK || !out.is_null());
    result
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct MigoSurfaceDescriptor {
    pub header: VersionedHeader,
    pub generation: u64,
    pub platform_kind: u32,
    pub flags: u32,
    pub width_pixels: u32,
    pub height_pixels: u32,
    pub scale_factor: f32,
    pub color_space: u32,
    pub alpha_mode: u32,
    pub preferred_presentation_mode: u32,
    pub capability_flags: u64,
    pub platform_descriptor_size: u32,
    pub reserved0: u32,
    pub platform_descriptor: *const c_void,
}

// SAFETY: every field has an all-zero representation and v1 requires the
// complete record.
unsafe impl AbiStruct for MigoSurfaceDescriptor {}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct MigoSurfaceMetrics {
    pub header: VersionedHeader,
    pub generation: u64,
    pub width_pixels: u32,
    pub height_pixels: u32,
    pub scale_factor: f32,
    pub color_space: u32,
    pub alpha_mode: u32,
    pub preferred_presentation_mode: u32,
    pub flags: u32,
    pub reserved0: u32,
}

// SAFETY: every field has an all-zero representation and v1 requires the
// complete record.
unsafe impl AbiStruct for MigoSurfaceMetrics {}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct MigoAndroidNativeWindowDescriptor {
    pub header: VersionedHeader,
    pub platform_kind: u32,
    pub flags: u32,
    pub native_window: *mut c_void,
}

// SAFETY: every field has an all-zero representation and v1 requires the
// complete record.
unsafe impl AbiStruct for MigoAndroidNativeWindowDescriptor {}

/// Layout-identical to the Android descriptor, and kept a separate type anyway.
///
/// The two platforms present the same shape -- one opaque native window handle
/// -- but they are different ABIs with different ownership calls, and the
/// `platform_kind` in the envelope is what selects between them. Collapsing
/// them into one type would make a host that set the wrong kind compile and
/// parse cleanly, only to hand a `OHNativeWindow*` to Android's reference
/// counting.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct MigoOpenHarmonyNativeWindowDescriptor {
    pub header: VersionedHeader,
    pub platform_kind: u32,
    pub flags: u32,
    pub native_window: *mut c_void,
}

// SAFETY: every field has an all-zero representation and v1 requires the
// complete record.
unsafe impl AbiStruct for MigoOpenHarmonyNativeWindowDescriptor {}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct MigoWin32HwndDescriptor {
    pub header: VersionedHeader,
    pub platform_kind: u32,
    pub flags: u32,
    pub hwnd: *mut c_void,
}

// SAFETY: every field has an all-zero representation and v1 requires the
// complete record.
unsafe impl AbiStruct for MigoWin32HwndDescriptor {}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct MigoX11WindowDescriptor {
    pub header: VersionedHeader,
    pub platform_kind: u32,
    pub flags: u32,
    pub display: *mut c_void,
    pub window: usize,
    pub screen: i32,
    pub reserved0: u32,
}

// SAFETY: every field has an all-zero representation and v1 requires the
// complete record.
unsafe impl AbiStruct for MigoX11WindowDescriptor {}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct MigoWaylandSurfaceDescriptor {
    pub header: VersionedHeader,
    pub platform_kind: u32,
    pub flags: u32,
    pub display: *mut c_void,
    pub surface: *mut c_void,
}

// SAFETY: every field has an all-zero representation and v1 requires the
// complete record.
unsafe impl AbiStruct for MigoWaylandSurfaceDescriptor {}

/// The four Apple descriptors below are layout-identical to each other and to
/// the Android one, and are four separate types for the reason
/// [`MigoOpenHarmonyNativeWindowDescriptor`] is separate from Android's: the
/// `platform_kind` in the envelope is what selects the ownership calls, and a
/// shared type would let a host that set the wrong kind compile and parse
/// cleanly before handing a `UIView*` to code that calls `nextDrawable` on it.
///
/// `ns_view` is an `NSView*`. Never dereferenced here.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct MigoMacosNsViewDescriptor {
    pub header: VersionedHeader,
    pub platform_kind: u32,
    pub flags: u32,
    pub ns_view: *mut c_void,
}

// SAFETY: every field has an all-zero representation and v1 requires the
// complete record.
unsafe impl AbiStruct for MigoMacosNsViewDescriptor {}

/// `ca_metal_layer` is a `CAMetalLayer*`. Never dereferenced here.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct MigoMacosMetalLayerDescriptor {
    pub header: VersionedHeader,
    pub platform_kind: u32,
    pub flags: u32,
    pub ca_metal_layer: *mut c_void,
}

// SAFETY: every field has an all-zero representation and v1 requires the
// complete record.
unsafe impl AbiStruct for MigoMacosMetalLayerDescriptor {}

/// `ui_view` is a `UIView*`. Never dereferenced here.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct MigoIosUiViewDescriptor {
    pub header: VersionedHeader,
    pub platform_kind: u32,
    pub flags: u32,
    pub ui_view: *mut c_void,
}

// SAFETY: every field has an all-zero representation and v1 requires the
// complete record.
unsafe impl AbiStruct for MigoIosUiViewDescriptor {}

/// `ca_metal_layer` is a `CAMetalLayer*`. Never dereferenced here.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct MigoIosMetalLayerDescriptor {
    pub header: VersionedHeader,
    pub platform_kind: u32,
    pub flags: u32,
    pub ca_metal_layer: *mut c_void,
}

// SAFETY: every field has an all-zero representation and v1 requires the
// complete record.
unsafe impl AbiStruct for MigoIosMetalLayerDescriptor {}

/// Canonical surface parameters accepted by the current renderer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceConfiguration {
    width_pixels: u32,
    height_pixels: u32,
    scale_factor: f32,
    color_space: u32,
    alpha_mode: u32,
    presentation_mode: u32,
}

impl SurfaceConfiguration {
    #[inline]
    pub const fn width_pixels(self) -> u32 {
        self.width_pixels
    }

    #[inline]
    pub const fn height_pixels(self) -> u32 {
        self.height_pixels
    }

    #[inline]
    pub const fn scale_factor(self) -> f32 {
        self.scale_factor
    }

    #[inline]
    pub const fn color_space(self) -> u32 {
        self.color_space
    }

    #[inline]
    pub const fn alpha_mode(self) -> u32 {
        self.alpha_mode
    }

    #[inline]
    pub const fn presentation_mode(self) -> u32 {
        self.presentation_mode
    }
}

/// Copied native identity. The pointers are opaque tokens and are never
/// dereferenced by the ABI validator.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ValidatedPlatformSurface {
    Android {
        native_window: NonNull<c_void>,
    },
    Win32 {
        hwnd: NonNull<c_void>,
    },
    X11 {
        display: NonNull<c_void>,
        window: usize,
        screen: i32,
    },
    Wayland {
        display: NonNull<c_void>,
        surface: NonNull<c_void>,
    },
    OpenHarmony {
        native_window: NonNull<c_void>,
    },
    MacosNsView {
        ns_view: NonNull<c_void>,
    },
    MacosMetalLayer {
        ca_metal_layer: NonNull<c_void>,
    },
    IosUiView {
        ui_view: NonNull<c_void>,
    },
    IosMetalLayer {
        ca_metal_layer: NonNull<c_void>,
    },
}

/// An owned snapshot of a caller's borrowed surface records.
///
/// The historical `Ref` suffix denotes the native objects' borrowed ownership,
/// not a Rust borrow: no pointer to either caller-owned descriptor is retained.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceDescriptorRef {
    public_generation: u64,
    configuration: SurfaceConfiguration,
    platform: ValidatedPlatformSurface,
}

impl SurfaceDescriptorRef {
    /// Parse using every platform payload implemented by the C ABI crate.
    ///
    /// # Safety
    /// `descriptor` and its matching platform payload must be aligned and
    /// readable for the byte counts announced in their headers for this call.
    pub unsafe fn parse(descriptor: *const MigoSurfaceDescriptor) -> Result<Self, MigoResult> {
        // SAFETY: forwarded from this method's contract.
        unsafe { Self::parse_for_platforms(descriptor, MIGO_CAPI_IMPLEMENTED_PLATFORM_KINDS) }
    }

    /// Parse without touching payloads that this binary cannot support.
    ///
    /// # Safety
    /// Same contract as [`Self::parse`]. For a kind present in
    /// `supported_platform_kinds`, its matching payload must be readable.
    pub unsafe fn parse_for_platforms(
        descriptor: *const MigoSurfaceDescriptor,
        supported_platform_kinds: u64,
    ) -> Result<Self, MigoResult> {
        // SAFETY: forwarded from this method's contract.
        let raw = unsafe { copy_versioned::<MigoSurfaceDescriptor>(descriptor.cast()) }?;
        let configuration = validate_configuration(
            raw.generation,
            raw.width_pixels,
            raw.height_pixels,
            raw.scale_factor,
            raw.color_space,
            raw.alpha_mode,
            raw.preferred_presentation_mode,
            raw.flags,
            raw.reserved0,
        )?;
        validate_capabilities(raw.capability_flags)?;

        validate_platform_kind(raw.platform_kind)?;
        let kind_bit = 1u64 << raw.platform_kind;
        if supported_platform_kinds & kind_bit == 0 {
            return Err(MIGO_ERROR_UNSUPPORTED_PLATFORM);
        }

        let platform = match raw.platform_kind {
            MIGO_PLATFORM_ANDROID_NATIVE_WINDOW => {
                require_payload_size::<MigoAndroidNativeWindowDescriptor>(
                    raw.platform_descriptor_size,
                )?;
                // SAFETY: the caller contract covers the selected payload.
                let payload = unsafe {
                    copy_versioned::<MigoAndroidNativeWindowDescriptor>(
                        raw.platform_descriptor.cast(),
                    )
                }?;
                validate_payload_prefix(payload.platform_kind, payload.flags, raw.platform_kind)?;
                let native_window =
                    NonNull::new(payload.native_window).ok_or(MIGO_ERROR_INVALID_ARGUMENT)?;
                ValidatedPlatformSurface::Android { native_window }
            }
            MIGO_PLATFORM_OPENHARMONY_NATIVE_WINDOW => {
                require_payload_size::<MigoOpenHarmonyNativeWindowDescriptor>(
                    raw.platform_descriptor_size,
                )?;
                // SAFETY: the caller contract covers the selected payload.
                let payload = unsafe {
                    copy_versioned::<MigoOpenHarmonyNativeWindowDescriptor>(
                        raw.platform_descriptor.cast(),
                    )
                }?;
                validate_payload_prefix(payload.platform_kind, payload.flags, raw.platform_kind)?;
                let native_window =
                    NonNull::new(payload.native_window).ok_or(MIGO_ERROR_INVALID_ARGUMENT)?;
                ValidatedPlatformSurface::OpenHarmony { native_window }
            }
            MIGO_PLATFORM_WIN32_HWND => {
                require_payload_size::<MigoWin32HwndDescriptor>(raw.platform_descriptor_size)?;
                // SAFETY: the caller contract covers the selected payload.
                let payload = unsafe {
                    copy_versioned::<MigoWin32HwndDescriptor>(raw.platform_descriptor.cast())
                }?;
                validate_payload_prefix(payload.platform_kind, payload.flags, raw.platform_kind)?;
                let hwnd = NonNull::new(payload.hwnd).ok_or(MIGO_ERROR_INVALID_ARGUMENT)?;
                ValidatedPlatformSurface::Win32 { hwnd }
            }
            MIGO_PLATFORM_X11_WINDOW => {
                require_payload_size::<MigoX11WindowDescriptor>(raw.platform_descriptor_size)?;
                // SAFETY: the caller contract covers the selected payload.
                let payload = unsafe {
                    copy_versioned::<MigoX11WindowDescriptor>(raw.platform_descriptor.cast())
                }?;
                validate_payload_prefix(payload.platform_kind, payload.flags, raw.platform_kind)?;
                validate_reserved(payload.reserved0.into())?;
                let display = NonNull::new(payload.display).ok_or(MIGO_ERROR_INVALID_ARGUMENT)?;
                if payload.window == 0 {
                    return Err(MIGO_ERROR_INVALID_ARGUMENT);
                }
                ValidatedPlatformSurface::X11 {
                    display,
                    window: payload.window,
                    screen: payload.screen,
                }
            }
            MIGO_PLATFORM_WAYLAND_SURFACE => {
                require_payload_size::<MigoWaylandSurfaceDescriptor>(raw.platform_descriptor_size)?;
                // SAFETY: the caller contract covers the selected payload.
                let payload = unsafe {
                    copy_versioned::<MigoWaylandSurfaceDescriptor>(raw.platform_descriptor.cast())
                }?;
                validate_payload_prefix(payload.platform_kind, payload.flags, raw.platform_kind)?;
                let display = NonNull::new(payload.display).ok_or(MIGO_ERROR_INVALID_ARGUMENT)?;
                let surface = NonNull::new(payload.surface).ok_or(MIGO_ERROR_INVALID_ARGUMENT)?;
                ValidatedPlatformSurface::Wayland { display, surface }
            }
            MIGO_PLATFORM_MACOS_NS_VIEW => {
                require_payload_size::<MigoMacosNsViewDescriptor>(raw.platform_descriptor_size)?;
                // SAFETY: the caller contract covers the selected payload.
                let payload = unsafe {
                    copy_versioned::<MigoMacosNsViewDescriptor>(raw.platform_descriptor.cast())
                }?;
                validate_payload_prefix(payload.platform_kind, payload.flags, raw.platform_kind)?;
                let ns_view = NonNull::new(payload.ns_view).ok_or(MIGO_ERROR_INVALID_ARGUMENT)?;
                ValidatedPlatformSurface::MacosNsView { ns_view }
            }
            MIGO_PLATFORM_MACOS_CA_METAL_LAYER => {
                require_payload_size::<MigoMacosMetalLayerDescriptor>(
                    raw.platform_descriptor_size,
                )?;
                // SAFETY: the caller contract covers the selected payload.
                let payload = unsafe {
                    copy_versioned::<MigoMacosMetalLayerDescriptor>(raw.platform_descriptor.cast())
                }?;
                validate_payload_prefix(payload.platform_kind, payload.flags, raw.platform_kind)?;
                let ca_metal_layer =
                    NonNull::new(payload.ca_metal_layer).ok_or(MIGO_ERROR_INVALID_ARGUMENT)?;
                ValidatedPlatformSurface::MacosMetalLayer { ca_metal_layer }
            }
            MIGO_PLATFORM_IOS_UI_VIEW => {
                require_payload_size::<MigoIosUiViewDescriptor>(raw.platform_descriptor_size)?;
                // SAFETY: the caller contract covers the selected payload.
                let payload = unsafe {
                    copy_versioned::<MigoIosUiViewDescriptor>(raw.platform_descriptor.cast())
                }?;
                validate_payload_prefix(payload.platform_kind, payload.flags, raw.platform_kind)?;
                let ui_view = NonNull::new(payload.ui_view).ok_or(MIGO_ERROR_INVALID_ARGUMENT)?;
                ValidatedPlatformSurface::IosUiView { ui_view }
            }
            MIGO_PLATFORM_IOS_CA_METAL_LAYER => {
                require_payload_size::<MigoIosMetalLayerDescriptor>(raw.platform_descriptor_size)?;
                // SAFETY: the caller contract covers the selected payload.
                let payload = unsafe {
                    copy_versioned::<MigoIosMetalLayerDescriptor>(raw.platform_descriptor.cast())
                }?;
                validate_payload_prefix(payload.platform_kind, payload.flags, raw.platform_kind)?;
                let ca_metal_layer =
                    NonNull::new(payload.ca_metal_layer).ok_or(MIGO_ERROR_INVALID_ARGUMENT)?;
                ValidatedPlatformSurface::IosMetalLayer { ca_metal_layer }
            }
            // validate_platform_kind and supported_platform_kinds establish
            // that only implemented payloads can reach this match.
            _ => return Err(MIGO_ERROR_UNSUPPORTED_PLATFORM),
        };

        Ok(Self {
            public_generation: raw.generation,
            configuration,
            platform,
        })
    }

    #[inline]
    pub const fn public_generation(self) -> u64 {
        self.public_generation
    }

    #[inline]
    pub const fn configuration(self) -> SurfaceConfiguration {
        self.configuration
    }

    #[inline]
    pub const fn platform(self) -> ValidatedPlatformSurface {
        self.platform
    }
}

/// Validated resize parameters copied from a full metrics record.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ValidatedSurfaceMetrics {
    public_generation: u64,
    configuration: SurfaceConfiguration,
}

impl ValidatedSurfaceMetrics {
    #[inline]
    pub const fn public_generation(self) -> u64 {
        self.public_generation
    }

    #[inline]
    pub const fn configuration(self) -> SurfaceConfiguration {
        self.configuration
    }
}

impl MigoSurfaceMetrics {
    /// Copy and validate a caller-owned metrics record.
    ///
    /// # Safety
    /// `metrics` must be aligned and readable for the byte count announced in
    /// its fixed header.
    pub unsafe fn parse(metrics: *const Self) -> Result<ValidatedSurfaceMetrics, MigoResult> {
        // SAFETY: forwarded from this method's contract.
        let raw = unsafe { copy_versioned::<Self>(metrics.cast()) }?;
        raw.validate_fields()
    }

    pub fn validate(&self) -> Result<ValidatedSurfaceMetrics, MigoResult> {
        // SAFETY: a reference to a complete local Self satisfies the copy
        // contract and every bit pattern of this raw record is valid.
        let raw = unsafe { copy_versioned::<Self>((self as *const Self).cast()) }?;
        raw.validate_fields()
    }

    fn validate_fields(self) -> Result<ValidatedSurfaceMetrics, MigoResult> {
        let configuration = validate_configuration(
            self.generation,
            self.width_pixels,
            self.height_pixels,
            self.scale_factor,
            self.color_space,
            self.alpha_mode,
            self.preferred_presentation_mode,
            self.flags,
            self.reserved0,
        )?;
        Ok(ValidatedSurfaceMetrics {
            public_generation: self.generation,
            configuration,
        })
    }
}

#[allow(clippy::too_many_arguments)]
fn validate_configuration(
    generation: u64,
    width_pixels: u32,
    height_pixels: u32,
    scale_factor: f32,
    color_space: u32,
    alpha_mode: u32,
    presentation_mode: u32,
    flags: u32,
    reserved0: u32,
) -> Result<SurfaceConfiguration, MigoResult> {
    if generation == 0
        || width_pixels == 0
        || height_pixels == 0
        || width_pixels > i32::MAX as u32
        || height_pixels > i32::MAX as u32
    {
        return Err(MIGO_ERROR_INVALID_ARGUMENT);
    }
    validate_flags(flags.into(), 0)?;
    validate_reserved(reserved0.into())?;
    let scale_factor = validate_positive_scale(scale_factor)?;

    let color_space = match color_space {
        MIGO_COLOR_SPACE_UNSPECIFIED | MIGO_COLOR_SPACE_SRGB => MIGO_COLOR_SPACE_SRGB,
        MIGO_COLOR_SPACE_DISPLAY_P3 | MIGO_COLOR_SPACE_EXTENDED_SRGB => {
            return Err(MIGO_ERROR_UNSUPPORTED_CAPABILITY);
        }
        _ => return Err(MIGO_ERROR_INVALID_ARGUMENT),
    };
    let alpha_mode = match alpha_mode {
        MIGO_ALPHA_MODE_UNSPECIFIED | MIGO_ALPHA_MODE_OPAQUE => MIGO_ALPHA_MODE_OPAQUE,
        MIGO_ALPHA_MODE_PREMULTIPLIED | MIGO_ALPHA_MODE_POSTMULTIPLIED => {
            return Err(MIGO_ERROR_UNSUPPORTED_CAPABILITY);
        }
        _ => return Err(MIGO_ERROR_INVALID_ARGUMENT),
    };
    let presentation_mode = match presentation_mode {
        MIGO_PRESENTATION_MODE_DEFAULT | MIGO_PRESENTATION_MODE_FIFO => MIGO_PRESENTATION_MODE_FIFO,
        MIGO_PRESENTATION_MODE_MAILBOX | MIGO_PRESENTATION_MODE_IMMEDIATE => {
            return Err(MIGO_ERROR_UNSUPPORTED_CAPABILITY);
        }
        _ => return Err(MIGO_ERROR_INVALID_ARGUMENT),
    };

    Ok(SurfaceConfiguration {
        width_pixels,
        height_pixels,
        scale_factor,
        color_space,
        alpha_mode,
        presentation_mode,
    })
}

fn validate_capabilities(capabilities: u64) -> Result<(), MigoResult> {
    validate_flags(capabilities, MIGO_SURFACE_CAPABILITY_KNOWN_MASK)?;
    if capabilities == 0 {
        Ok(())
    } else {
        // Capability bits are requirements, never hints. The current renderer
        // has not yet plumbed any of these semantics end-to-end.
        Err(MIGO_ERROR_UNSUPPORTED_CAPABILITY)
    }
}

fn validate_platform_kind(kind: u32) -> Result<(), MigoResult> {
    match kind {
        MIGO_PLATFORM_ANDROID_NATIVE_WINDOW
        | MIGO_PLATFORM_WIN32_HWND
        | MIGO_PLATFORM_WINUI_SWAP_CHAIN_PANEL
        | MIGO_PLATFORM_MACOS_NS_VIEW
        | MIGO_PLATFORM_MACOS_CA_METAL_LAYER
        | MIGO_PLATFORM_X11_WINDOW
        | MIGO_PLATFORM_WAYLAND_SURFACE
        | MIGO_PLATFORM_OPENHARMONY_NATIVE_WINDOW
        | MIGO_PLATFORM_IOS_UI_VIEW
        | MIGO_PLATFORM_IOS_CA_METAL_LAYER => Ok(()),
        MIGO_PLATFORM_UNKNOWN | 11..=u32::MAX => Err(MIGO_ERROR_INVALID_ARGUMENT),
    }
}

fn require_payload_size<T>(announced: u32) -> Result<(), MigoResult> {
    if announced as usize == size_of::<T>() {
        Ok(())
    } else {
        Err(MIGO_ERROR_INVALID_ARGUMENT)
    }
}

fn validate_payload_prefix(kind: u32, flags: u32, envelope_kind: u32) -> Result<(), MigoResult> {
    if kind != envelope_kind {
        return Err(MIGO_ERROR_INVALID_ARGUMENT);
    }
    validate_flags(flags.into(), 0)
}

const _: () = assert!(offset_of!(MigoSurfaceDescriptor, header) == 0);
const _: () = assert!(offset_of!(MigoSurfaceReleaseStatus, header) == 0);
const _: () = assert!(offset_of!(MigoSurfaceReleaseStatus, generation) == 8);
const _: () = assert!(offset_of!(MigoSurfaceReleaseStatus, state) == 16);
const _: () = assert!(size_of::<MigoSurfaceReleaseStatus>() == 24);
const _: () = assert!(offset_of!(MigoSurfaceDescriptor, generation) == 8);
const _: () = assert!(offset_of!(MigoSurfaceDescriptor, platform_kind) == 16);
const _: () = assert!(offset_of!(MigoSurfaceDescriptor, scale_factor) == 32);
const _: () = assert!(offset_of!(MigoSurfaceDescriptor, capability_flags) == 48);
const _: () = assert!(offset_of!(MigoSurfaceMetrics, header) == 0);
const _: () = assert!(offset_of!(MigoSurfaceMetrics, scale_factor) == 24);
const _: () = assert!(size_of::<MigoSurfaceMetrics>() == 48);
const _: () = assert!(offset_of!(MigoAndroidNativeWindowDescriptor, header) == 0);
const _: () = assert!(offset_of!(MigoWin32HwndDescriptor, header) == 0);
const _: () = assert!(offset_of!(MigoX11WindowDescriptor, header) == 0);
const _: () = assert!(offset_of!(MigoWaylandSurfaceDescriptor, header) == 0);

#[cfg(target_pointer_width = "64")]
const _: () = assert!(size_of::<MigoSurfaceDescriptor>() == 72);
#[cfg(target_pointer_width = "64")]
const _: () = assert!(offset_of!(MigoSurfaceDescriptor, platform_descriptor) == 64);
#[cfg(target_pointer_width = "64")]
const _: () = assert!(size_of::<MigoAndroidNativeWindowDescriptor>() == 24);
#[cfg(target_pointer_width = "64")]
const _: () = assert!(offset_of!(MigoAndroidNativeWindowDescriptor, native_window) == 16);
#[cfg(target_pointer_width = "64")]
const _: () = assert!(size_of::<MigoWin32HwndDescriptor>() == 24);
#[cfg(target_pointer_width = "64")]
const _: () = assert!(offset_of!(MigoWin32HwndDescriptor, hwnd) == 16);
// The OpenHarmony descriptor is pinned independently rather than by reference
// to Android's. If either moves, the one that moved fails here -- which is the
// point of asserting a layout twice.
//
// The size and the pointer offset depend on the pointer width and are gated
// like every other one here; only the header offset holds on both. Written
// ungated, they were true on this machine and broke the ILP32 lane, where the
// descriptor is 20 bytes. That lane pins both widths for this type already
// (tests/c_abi/platform_contract.c), so gating loses no coverage -- it is also
// the only lane that compiles for ILP32 at all.
const _: () = assert!(offset_of!(MigoOpenHarmonyNativeWindowDescriptor, header) == 0);
#[cfg(target_pointer_width = "64")]
const _: () = assert!(size_of::<MigoOpenHarmonyNativeWindowDescriptor>() == 24);
#[cfg(target_pointer_width = "64")]
const _: () = assert!(offset_of!(MigoOpenHarmonyNativeWindowDescriptor, native_window) == 16);
#[cfg(target_pointer_width = "64")]
const _: () = assert!(size_of::<MigoX11WindowDescriptor>() == 40);
#[cfg(target_pointer_width = "64")]
const _: () = assert!(offset_of!(MigoX11WindowDescriptor, display) == 16);
#[cfg(target_pointer_width = "64")]
const _: () = assert!(size_of::<MigoWaylandSurfaceDescriptor>() == 32);
#[cfg(target_pointer_width = "64")]
const _: () = assert!(offset_of!(MigoWaylandSurfaceDescriptor, display) == 16);
#[cfg(target_pointer_width = "64")]
const _: () = assert!(offset_of!(MigoWaylandSurfaceDescriptor, surface) == 24);
