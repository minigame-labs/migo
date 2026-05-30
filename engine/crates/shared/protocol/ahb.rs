//! Owned `AHardwareBuffer` handle and helpers — the cross-crate
//! protocol type for "decoded image lives in a GPU-importable buffer
//! instead of a `Vec<u8>`".
//!
//! On Android (NDK API 26+) `OwnedAhb` is a thin RAII wrapper over
//! `AHardwareBuffer*`: `allocate` creates a fresh owned handle,
//! `from_raw_acquire` adopts a borrowed raw pointer by bumping the
//! refcount, `from_raw_owned` adopts an already-owned raw pointer,
//! and `Drop` releases it. The pointer is opaque to safe Rust;
//! consumers (graphics' `eglCreateImageKHR(EGL_NATIVE_BUFFER_ANDROID, ahb)`,
//! Java's `Bitmap.wrapHardwareBuffer`) reach in via [`OwnedAhb::raw`]
//! and trust the RAII to outlive their borrow.
//!
//! On non-Android targets the type is a **mock** backed by a `Vec<u8>`.
//! The mock exists for two reasons:
//!
//! 1. **TDD**. The Rust-side decode-→-AHB→-upload pipeline is the
//!    bulk of the M2 change set; we want to drive it from unit tests
//!    without requiring an emulator round-trip.
//! 2. **Desktop dev builds** (where `cfg!(not(target_os="android"))`
//!    keeps the Rust image decoders) can still produce a uniform
//!    `DecodedImage::HardwareBuffer` so downstream code paths are
//!    exercised on the host.
//!
//! # Thread safety
//!
//! `AHardwareBuffer_acquire` / `_release` / `_lock` / `_unlock` are
//! all documented thread-safe (the NDK uses an atomic refcount).
//! [`OwnedAhb`] is therefore `Send + Sync`. The mock holds a `Vec<u8>`
//! behind an `Arc<Mutex>` so it is `Send + Sync` for the same
//! contract.
//!
//! # What this module is NOT
//!
//! * Not a Vulkan importer — that lives in `graphics/`.
//! * Not a Java `HardwareBuffer` bridge — `platform/android/jni`
//!   wraps `_acquire` and hands the raw pointer through `jlong`.
//! * Not a fence/sync primitive — sync fences travel separately
//!   (`fence_fd: i32` field on `DecodedImage::HardwareBuffer`).

use std::fmt;

#[cfg(target_os = "android")]
mod sys {
    //! Hand-rolled FFI for the small slice of the AHB ABI we use.
    //!
    //! Why not `ndk-sys`? It would pull a much larger NDK Rust
    //! surface for the sake of half a dozen function signatures.
    //! Linking against `libandroid` (which AHB lives in) is already
    //! handled elsewhere in the platform crate; this module just
    //! declares the C entry points.

    use std::os::raw::{c_int, c_void};

    /// Opaque NDK type. We never construct it directly.
    #[repr(C)]
    pub struct AHardwareBuffer {
        _private: [u8; 0],
    }

    #[repr(C)]
    pub struct AHardwareBuffer_Desc {
        pub width: u32,
        pub height: u32,
        pub layers: u32,
        pub format: u32,
        pub usage: u64,
        pub stride: u32,
        pub rfu0: u32,
        pub rfu1: u64,
    }

    #[repr(C)]
    pub struct ARect {
        pub left: i32,
        pub top: i32,
        pub right: i32,
        pub bottom: i32,
    }

    // libandroid is already linked transitively via `tracing-android`
    // and the AHB call sites in `crates/graphics`; a duplicate
    // `#[link(name = "android")]` is harmless thanks to the linker
    // dedup, but we omit it to avoid pulling the attribute into
    // `shared` (a "neutral" crate). The functions are still resolved
    // because every Android binary links libandroid.
    //
    // Edition 2024 requires `extern` blocks be spelled `unsafe extern`.
    unsafe extern "C" {
        pub fn AHardwareBuffer_allocate(
            desc: *const AHardwareBuffer_Desc,
            out: *mut *mut AHardwareBuffer,
        ) -> c_int;
        pub fn AHardwareBuffer_acquire(buffer: *mut AHardwareBuffer);
        pub fn AHardwareBuffer_release(buffer: *mut AHardwareBuffer);
        pub fn AHardwareBuffer_describe(
            buffer: *const AHardwareBuffer,
            out_desc: *mut AHardwareBuffer_Desc,
        );
        pub fn AHardwareBuffer_lock(
            buffer: *mut AHardwareBuffer,
            usage: u64,
            fence: c_int,
            rect: *const ARect,
            out_addr: *mut *mut c_void,
        ) -> c_int;
        pub fn AHardwareBuffer_unlock(buffer: *mut AHardwareBuffer, out_fence: *mut c_int)
        -> c_int;
    }
}

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Pixel formats we currently produce or consume. The integer
/// representation matches `AHARDWAREBUFFER_FORMAT_*` exactly so it
/// can be passed directly to `AHardwareBuffer_allocate` on Android.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum AhbFormat {
    /// 32-bit RGBA, byte order R, G, B, A. The single format the
    /// image pipeline produces today; mirrors
    /// `AHARDWAREBUFFER_FORMAT_R8G8B8A8_UNORM = 1`.
    R8g8b8a8Unorm = 1,
}

impl AhbFormat {
    /// Bytes per pixel for the format.  Used to size CPU-side row
    /// pitches and validate `desc.stride * height` against the
    /// expected payload length.
    #[inline]
    pub const fn bytes_per_pixel(self) -> u32 {
        match self {
            AhbFormat::R8g8b8a8Unorm => 4,
        }
    }
}

bitflags::bitflags! {
    /// `AHARDWAREBUFFER_USAGE_*` bit flags. The values mirror the
    /// NDK header so we can pass through to `_allocate` unchanged.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct AhbUsage: u64 {
        /// `AHARDWAREBUFFER_USAGE_CPU_READ_OFTEN`
        const CPU_READ_OFTEN = 0x3;
        /// `AHARDWAREBUFFER_USAGE_CPU_READ_RARELY`
        const CPU_READ_RARELY = 0x2;
        /// `AHARDWAREBUFFER_USAGE_CPU_WRITE_OFTEN`
        const CPU_WRITE_OFTEN = 0x30;
        /// `AHARDWAREBUFFER_USAGE_CPU_WRITE_RARELY`
        const CPU_WRITE_RARELY = 0x20;
        /// `AHARDWAREBUFFER_USAGE_GPU_SAMPLED_IMAGE`
        const GPU_SAMPLED_IMAGE = 0x100;
        /// `AHARDWAREBUFFER_USAGE_GPU_COLOR_OUTPUT`
        const GPU_COLOR_OUTPUT = 0x200;
    }
}

/// Description of an AHB. Mirrors the NDK `AHardwareBuffer_Desc`.
///
/// `stride_pixels` is **output-only**: callers should pass `0` to
/// [`OwnedAhb::allocate`] and read the actual stride from the
/// descriptor on the returned handle. (The driver may pad rows for
/// alignment; the actual stride is rarely equal to `width`.)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AhbDesc {
    pub width: u32,
    pub height: u32,
    pub layers: u32,
    pub format: AhbFormat,
    pub usage: AhbUsage,
    /// Row stride in **pixels**. Set to 0 when calling `allocate`;
    /// populated by the driver and visible via [`OwnedAhb::desc`].
    pub stride_pixels: u32,
}

impl AhbDesc {
    /// Convenience for the canonical "decoded image, GPU-sampled,
    /// occasionally CPU-read for `getImageData`" usage.
    pub fn rgba_sampled(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            layers: 1,
            format: AhbFormat::R8g8b8a8Unorm,
            usage: AhbUsage::GPU_SAMPLED_IMAGE | AhbUsage::CPU_READ_RARELY,
            stride_pixels: 0,
        }
    }
}

/// Result type for AHB operations. Errors are coarse on purpose —
/// the NDK only ever returns `int` status codes; this enum keeps
/// the meaningful failure modes distinct from one another.
#[derive(Debug)]
pub enum AhbError {
    /// `AHardwareBuffer_allocate` returned non-zero.
    AllocateFailed { status: i32 },
    /// `AHardwareBuffer_lock` returned non-zero.
    LockFailed { status: i32 },
    /// `AHardwareBuffer_unlock` returned non-zero.
    UnlockFailed { status: i32 },
    /// Operation requires Android; called on a non-Android build.
    NotAndroid,
    /// Caller passed a null pointer to `from_raw_acquire` /
    /// `from_raw_owned`.
    NullHandle,
    /// CPU lock requested with usage flags that exclude both
    /// `CPU_READ_*` and `CPU_WRITE_*`.
    InvalidLockUsage,
}

impl fmt::Display for AhbError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AhbError::AllocateFailed { status } => write!(f, "AHB allocate failed: {status}"),
            AhbError::LockFailed { status } => write!(f, "AHB lock failed: {status}"),
            AhbError::UnlockFailed { status } => write!(f, "AHB unlock failed: {status}"),
            AhbError::NotAndroid => write!(f, "AHB not available on this target"),
            AhbError::NullHandle => write!(f, "AHB null handle"),
            AhbError::InvalidLockUsage => write!(f, "AHB lock usage missing CPU_READ/WRITE bit"),
        }
    }
}

impl std::error::Error for AhbError {}

// ---------------------------------------------------------------------------
// OwnedAhb — Android backend
// ---------------------------------------------------------------------------

#[cfg(target_os = "android")]
mod imp {
    use super::*;
    use std::os::raw::c_void;
    use std::ptr;
    use std::sync::Arc;

    /// Refcount-owned AHB. Cheap to clone (one `_acquire` call).
    pub struct OwnedAhb {
        // `Arc` so `Clone` is one atomic increment + one
        // `_acquire`; the inner box owns the AHB drop.
        inner: Arc<AhbBox>,
        desc: AhbDesc,
    }

    struct AhbBox {
        ptr: *mut sys::AHardwareBuffer,
    }

    // SAFETY: `AHardwareBuffer_*` calls are documented thread-safe.
    unsafe impl Send for AhbBox {}
    unsafe impl Sync for AhbBox {}

    impl Drop for AhbBox {
        fn drop(&mut self) {
            // SAFETY: we own one strong refcount handed to us by
            // either `_allocate` or our own `_acquire`. Releasing
            // matches that count exactly.
            unsafe { sys::AHardwareBuffer_release(self.ptr) }
        }
    }

    impl OwnedAhb {
        pub fn allocate(mut desc: AhbDesc) -> Result<Self, AhbError> {
            // The NDK ignores `stride` on input but writes it on the
            // descriptor it stores; we still zero it for clarity.
            desc.stride_pixels = 0;
            let c_desc = sys::AHardwareBuffer_Desc {
                width: desc.width,
                height: desc.height,
                layers: desc.layers,
                format: desc.format as u32,
                usage: desc.usage.bits(),
                stride: 0,
                rfu0: 0,
                rfu1: 0,
            };
            let mut out = ptr::null_mut();
            let status = unsafe { sys::AHardwareBuffer_allocate(&c_desc, &mut out) };
            if status != 0 || out.is_null() {
                return Err(AhbError::AllocateFailed { status });
            }
            // Read back the *actual* stride (driver may have padded).
            let mut described = sys::AHardwareBuffer_Desc {
                width: 0,
                height: 0,
                layers: 0,
                format: 0,
                usage: 0,
                stride: 0,
                rfu0: 0,
                rfu1: 0,
            };
            unsafe { sys::AHardwareBuffer_describe(out, &mut described) };
            desc.stride_pixels = described.stride;
            Ok(Self {
                inner: Arc::new(AhbBox { ptr: out }),
                desc,
            })
        }

        /// Adopt an externally-allocated AHB pointer.  Typically used
        /// when Java passed `Bitmap.getHardwareBuffer()` back through
        /// JNI as a `jlong` — the Java side held a reference, so we
        /// `_acquire` to gain our own.
        pub fn from_raw_acquire(ptr: *mut c_void, desc: AhbDesc) -> Result<Self, AhbError> {
            if ptr.is_null() {
                return Err(AhbError::NullHandle);
            }
            let ahb = ptr as *mut sys::AHardwareBuffer;
            unsafe { sys::AHardwareBuffer_acquire(ahb) };
            Ok(Self {
                inner: Arc::new(AhbBox { ptr: ahb }),
                desc,
            })
        }

        /// Adopt an externally-allocated AHB pointer that already
        /// owns one strong refcount.
        ///
        /// This is the transfer-ownership counterpart to
        /// [`Self::from_raw_acquire`]: use it when the producer has
        /// already called `AHardwareBuffer_acquire` (or equivalent)
        /// before handing the pointer across an FFI boundary.
        pub fn from_raw_owned(ptr: *mut c_void, desc: AhbDesc) -> Result<Self, AhbError> {
            if ptr.is_null() {
                return Err(AhbError::NullHandle);
            }
            Ok(Self {
                inner: Arc::new(AhbBox {
                    ptr: ptr as *mut sys::AHardwareBuffer,
                }),
                desc,
            })
        }

        /// Raw pointer for FFI use (e.g. EGL / Vulkan import).
        /// Borrows from `self`; never outlive the [`OwnedAhb`].
        pub fn raw(&self) -> *mut c_void {
            self.inner.ptr as *mut c_void
        }

        pub fn desc(&self) -> &AhbDesc {
            &self.desc
        }

        /// Convert into a raw pointer **without** decrementing the
        /// refcount — the caller is responsible for matching with
        /// `AHardwareBuffer_release`.  For tightly-controlled
        /// hand-offs, e.g. when transferring across an unsafe ABI.
        pub fn into_raw(self) -> *mut c_void {
            // We can't move out of an Arc easily; clone-and-leak is
            // the standard pattern. Safety: caller takes over the
            // refcount we held.
            let ptr = self.raw();
            // Acquire one extra ref before the Arc drops below.
            unsafe { sys::AHardwareBuffer_acquire(ptr as *mut sys::AHardwareBuffer) };
            drop(self);
            ptr
        }

        /// Lock the buffer for CPU access. `Drop` of the returned
        /// guard unlocks. The lock is plane-0 only (suitable for
        /// `R8G8B8A8_UNORM`); planar / YUV formats need a different
        /// API which we don't currently use.
        pub fn lock_cpu(&self, usage: AhbUsage) -> Result<AhbLock<'_>, AhbError> {
            let cpu_bits = AhbUsage::CPU_READ_OFTEN
                | AhbUsage::CPU_READ_RARELY
                | AhbUsage::CPU_WRITE_OFTEN
                | AhbUsage::CPU_WRITE_RARELY;
            if !usage.intersects(cpu_bits) {
                return Err(AhbError::InvalidLockUsage);
            }
            let mut addr: *mut c_void = ptr::null_mut();
            let status = unsafe {
                sys::AHardwareBuffer_lock(
                    self.inner.ptr,
                    usage.bits(),
                    -1,          // no input fence
                    ptr::null(), // entire buffer
                    &mut addr,
                )
            };
            if status != 0 || addr.is_null() {
                return Err(AhbError::LockFailed { status });
            }
            Ok(AhbLock {
                ahb: self,
                addr: addr as *mut u8,
                stride_bytes: self.desc.stride_pixels * self.desc.format.bytes_per_pixel(),
            })
        }
    }

    impl Clone for OwnedAhb {
        fn clone(&self) -> Self {
            Self {
                inner: Arc::clone(&self.inner),
                desc: self.desc,
            }
        }
    }

    impl fmt::Debug for OwnedAhb {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("OwnedAhb")
                .field("ptr", &self.inner.ptr)
                .field("desc", &self.desc)
                .finish()
        }
    }

    pub struct AhbLock<'a> {
        ahb: &'a OwnedAhb,
        addr: *mut u8,
        stride_bytes: u32,
    }

    impl<'a> AhbLock<'a> {
        /// Raw pointer to row 0, byte 0. Lifetime is tied to `self`.
        pub fn as_ptr(&self) -> *mut u8 {
            self.addr
        }
        pub fn stride_bytes(&self) -> u32 {
            self.stride_bytes
        }
    }

    impl<'a> Drop for AhbLock<'a> {
        fn drop(&mut self) {
            // We don't propagate an out-fence here; if the caller
            // needs a fence-back protocol they should use the lock
            // form that exposes it. Best-effort unlock; on error
            // there's nothing the destructor can do.
            let mut out_fence: std::os::raw::c_int = -1;
            unsafe {
                let _ = sys::AHardwareBuffer_unlock(self.ahb.inner.ptr, &mut out_fence);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// OwnedAhb — Mock backend (every other target)
// ---------------------------------------------------------------------------

#[cfg(not(target_os = "android"))]
mod imp {
    use super::*;
    use std::os::raw::c_void;
    use std::sync::Arc;

    /// Mock AHB: a heap-allocated pixel buffer with the same
    /// surface API. Used in unit tests and on dev hosts.
    #[derive(Clone)]
    pub struct OwnedAhb {
        inner: Arc<MockInner>,
        desc: AhbDesc,
    }

    struct MockInner {
        // `RwLock` would let multi-reader but we keep parity with
        // the Android lock semantics (exclusive access during a
        // CPU lock). `parking_lot::Mutex` for cheap unpoisoned
        // locking.
        pixels: parking_lot::Mutex<Vec<u8>>,
    }

    impl OwnedAhb {
        pub fn allocate(mut desc: AhbDesc) -> Result<Self, AhbError> {
            // Simulate driver-chosen stride: align to 64 px (a value
            // close to what real Mali / Adreno drivers tend to pick).
            let aligned = (desc.width + 63) & !63;
            desc.stride_pixels = aligned.max(desc.width);
            let bytes = (desc.stride_pixels as usize)
                .checked_mul(desc.height as usize)
                .and_then(|p| p.checked_mul(desc.format.bytes_per_pixel() as usize))
                .ok_or(AhbError::AllocateFailed { status: -1 })?;
            Ok(Self {
                inner: Arc::new(MockInner {
                    pixels: parking_lot::Mutex::new(vec![0u8; bytes]),
                }),
                desc,
            })
        }

        pub fn from_raw_acquire(_ptr: *mut c_void, _desc: AhbDesc) -> Result<Self, AhbError> {
            // The mock has no notion of an external pointer to adopt.
            Err(AhbError::NotAndroid)
        }

        pub fn from_raw_owned(_ptr: *mut c_void, _desc: AhbDesc) -> Result<Self, AhbError> {
            // The mock has no notion of an external pointer to adopt.
            Err(AhbError::NotAndroid)
        }

        pub fn raw(&self) -> *mut c_void {
            // The mock returns the address of the heap pixel buffer
            // so tests can `as_ptr` round-trip via FFI plumbing
            // without exercising AHB syscalls.
            let g = self.inner.pixels.lock();
            g.as_ptr() as *mut c_void
        }

        pub fn desc(&self) -> &AhbDesc {
            &self.desc
        }

        pub fn into_raw(self) -> *mut c_void {
            self.raw()
        }

        pub fn lock_cpu(&self, usage: AhbUsage) -> Result<AhbLock<'_>, AhbError> {
            let cpu_bits = AhbUsage::CPU_READ_OFTEN
                | AhbUsage::CPU_READ_RARELY
                | AhbUsage::CPU_WRITE_OFTEN
                | AhbUsage::CPU_WRITE_RARELY;
            if !usage.intersects(cpu_bits) {
                return Err(AhbError::InvalidLockUsage);
            }
            let guard = self.inner.pixels.lock();
            let stride_bytes = self.desc.stride_pixels * self.desc.format.bytes_per_pixel();
            Ok(AhbLock {
                _ahb: self,
                guard,
                stride_bytes,
            })
        }
    }

    impl fmt::Debug for OwnedAhb {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("OwnedAhb (mock)")
                .field("desc", &self.desc)
                .finish()
        }
    }

    pub struct AhbLock<'a> {
        _ahb: &'a OwnedAhb,
        guard: parking_lot::MutexGuard<'a, Vec<u8>>,
        stride_bytes: u32,
    }

    impl<'a> AhbLock<'a> {
        pub fn as_ptr(&self) -> *mut u8 {
            self.guard.as_ptr() as *mut u8
        }
        pub fn stride_bytes(&self) -> u32 {
            self.stride_bytes
        }
    }
}

pub use imp::{AhbLock, OwnedAhb};

// ---------------------------------------------------------------------------
// Helpers usable on every target (operate through the AhbLock surface)
// ---------------------------------------------------------------------------

/// Copy a tightly-packed RGBA buffer into the AHB, accounting for
/// driver-imposed row stride.  Panics if `rgba.len()` doesn't match
/// `width * height * bpp` exactly (this is a programming error, not
/// a runtime input).
pub fn write_rgba_into_ahb(ahb: &OwnedAhb, rgba: &[u8]) -> Result<(), AhbError> {
    let desc = *ahb.desc();
    let bpp = desc.format.bytes_per_pixel() as usize;
    let row_bytes_src = desc.width as usize * bpp;
    assert_eq!(
        rgba.len(),
        row_bytes_src * desc.height as usize,
        "write_rgba_into_ahb: source size mismatch (got {}, want {}x{}x{})",
        rgba.len(),
        desc.width,
        desc.height,
        bpp,
    );
    let lock = ahb.lock_cpu(AhbUsage::CPU_WRITE_OFTEN)?;
    let stride_bytes = lock.stride_bytes() as usize;
    let dst_base = lock.as_ptr();
    // SAFETY: the lock guarantees exclusive access to `desc.height`
    // rows of `stride_bytes` each; we copy at most `row_bytes_src`
    // (≤ stride_bytes) per row from a slice we statically know is
    // `row_bytes_src * height` long.
    unsafe {
        for y in 0..desc.height as usize {
            let src = rgba.as_ptr().add(y * row_bytes_src);
            let dst = dst_base.add(y * stride_bytes);
            std::ptr::copy_nonoverlapping(src, dst, row_bytes_src);
        }
    }
    Ok(())
}

/// Read the AHB pixels back into a tightly-packed RGBA `Vec<u8>`.
/// Used by `getImageData` / `readPixels` paths after an upload.
pub fn read_rgba_from_ahb(ahb: &OwnedAhb) -> Result<Vec<u8>, AhbError> {
    let desc = *ahb.desc();
    let bpp = desc.format.bytes_per_pixel() as usize;
    let row_bytes_src = desc.width as usize * bpp;
    let mut out = vec![0u8; row_bytes_src * desc.height as usize];
    let lock = ahb.lock_cpu(AhbUsage::CPU_READ_OFTEN)?;
    let stride_bytes = lock.stride_bytes() as usize;
    let src_base = lock.as_ptr();
    // SAFETY: see `write_rgba_into_ahb`. Same row-stride caveat.
    unsafe {
        for y in 0..desc.height as usize {
            let src = src_base.add(y * stride_bytes);
            let dst = out.as_mut_ptr().add(y * row_bytes_src);
            std::ptr::copy_nonoverlapping(src, dst, row_bytes_src);
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocate_returns_handle_with_described_dims() {
        let ahb = OwnedAhb::allocate(AhbDesc::rgba_sampled(64, 32)).expect("alloc");
        let d = ahb.desc();
        assert_eq!(d.width, 64);
        assert_eq!(d.height, 32);
        assert!(d.stride_pixels >= d.width, "stride must be >= width");
        assert_eq!(d.format, AhbFormat::R8g8b8a8Unorm);
    }

    #[test]
    fn clone_is_cheap_and_independent_lifetime() {
        let a = OwnedAhb::allocate(AhbDesc::rgba_sampled(8, 8)).unwrap();
        let b = a.clone();
        // Drop `a`; `b` must still be valid.
        drop(a);
        assert_eq!(b.desc().width, 8);
    }

    #[test]
    fn lock_requires_cpu_usage_bit() {
        let ahb = OwnedAhb::allocate(AhbDesc::rgba_sampled(4, 4)).unwrap();
        let r = ahb.lock_cpu(AhbUsage::GPU_SAMPLED_IMAGE);
        assert!(matches!(r, Err(AhbError::InvalidLockUsage)));
    }

    #[test]
    fn write_then_read_roundtrips_pixels() {
        let ahb = OwnedAhb::allocate(AhbDesc::rgba_sampled(3, 2)).unwrap();
        // 3x2 image, RGBA bytes.
        #[rustfmt::skip]
        let pixels: Vec<u8> = vec![
            255,   0,   0, 255,    0, 255,   0, 255,    0,   0, 255, 255,
              0,   0,   0, 255,  255, 255, 255, 255,  128, 128, 128, 255,
        ];
        write_rgba_into_ahb(&ahb, &pixels).unwrap();
        let got = read_rgba_from_ahb(&ahb).unwrap();
        assert_eq!(got, pixels);
    }

    #[test]
    #[should_panic(expected = "source size mismatch")]
    fn write_panics_on_size_mismatch() {
        let ahb = OwnedAhb::allocate(AhbDesc::rgba_sampled(2, 2)).unwrap();
        let _ = write_rgba_into_ahb(&ahb, &[0u8; 8]); // 2 rows of 8 bytes != 2*2*4
    }

    #[test]
    fn from_raw_acquire_rejects_null_on_android() {
        // On non-Android the mock always returns NotAndroid; on
        // Android the same call with a null pointer must error too.
        let r = OwnedAhb::from_raw_acquire(std::ptr::null_mut(), AhbDesc::rgba_sampled(1, 1));
        assert!(matches!(
            r,
            Err(AhbError::NullHandle) | Err(AhbError::NotAndroid)
        ));
    }

    #[test]
    fn from_raw_owned_rejects_null_on_android() {
        // Mirrors the transfer-ownership path used by the Android
        // Java bridge after it acquires its own native refcount.
        let r = OwnedAhb::from_raw_owned(std::ptr::null_mut(), AhbDesc::rgba_sampled(1, 1));
        assert!(matches!(
            r,
            Err(AhbError::NullHandle) | Err(AhbError::NotAndroid)
        ));
    }

    #[test]
    fn ahb_is_send_and_sync() {
        // Compile-time assertion via trait bounds.
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<OwnedAhb>();
        assert_send_sync::<AhbDesc>();
        assert_send_sync::<AhbFormat>();
    }
}
