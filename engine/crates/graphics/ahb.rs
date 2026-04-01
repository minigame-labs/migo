//! AHardwareBuffer NDK FFI bindings (API 26+).
//!
//! Provides safe wrappers around `AHardwareBuffer_*` functions for
//! zero-GL-upload texture import via EGLImage.
//!
//! All functions are dynamically checked at runtime — calling on API < 26
//! returns an error rather than crashing.

#![cfg(target_os = "android")]

use std::ffi::c_void;
use std::ptr;

// --- Raw FFI (linked against libandroid.so) ---

#[repr(C)]
pub struct AHardwareBuffer {
    _opaque: [u8; 0],
}

/// Matches `AHardwareBuffer_Desc` from <android/hardware_buffer.h>.
#[repr(C)]
#[derive(Debug, Clone)]
pub struct AHardwareBufferDesc {
    pub width: u32,
    pub height: u32,
    pub layers: u32,
    pub format: u32,
    pub usage: u64,
    pub stride: u32,
    pub rfu0: u32,
    pub rfu1: u64,
}

/// AHARDWAREBUFFER_FORMAT_R8G8B8A8_UNORM
pub const FORMAT_RGBA_8888: u32 = 1;

/// Usage flags
pub const USAGE_GPU_SAMPLED_IMAGE: u64 = 1 << 8;
pub const USAGE_CPU_WRITE_OFTEN: u64 = 1 << 6;

#[link(name = "android")]
unsafe extern "C" {
    fn AHardwareBuffer_allocate(
        desc: *const AHardwareBufferDesc,
        out: *mut *mut AHardwareBuffer,
    ) -> i32;
    fn AHardwareBuffer_release(buffer: *mut AHardwareBuffer);
    fn AHardwareBuffer_lock(
        buffer: *mut AHardwareBuffer,
        usage: u64,
        fence: i32,
        rect: *const c_void, // NULL = entire buffer
        out_virtual_address: *mut *mut c_void,
    ) -> i32;
    fn AHardwareBuffer_unlock(
        buffer: *mut AHardwareBuffer,
        fence: *mut i32, // NULL = synchronous
    ) -> i32;
    fn AHardwareBuffer_describe(
        buffer: *const AHardwareBuffer,
        out_desc: *mut AHardwareBufferDesc,
    );

    fn android_get_device_api_level() -> i32;
}

/// Owned handle to an AHardwareBuffer.  Releases on drop.
pub struct OwnedAHardwareBuffer {
    ptr: *mut AHardwareBuffer,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
}

unsafe impl Send for OwnedAHardwareBuffer {}

impl OwnedAHardwareBuffer {
    /// Allocate a new RGBA8888 buffer suitable for GPU texture import.
    pub fn allocate(width: u32, height: u32) -> Result<Self, String> {
        if unsafe { android_get_device_api_level() } < 26 {
            return Err("AHardwareBuffer requires API 26+".into());
        }

        let desc = AHardwareBufferDesc {
            width,
            height,
            layers: 1,
            format: FORMAT_RGBA_8888,
            usage: USAGE_GPU_SAMPLED_IMAGE | USAGE_CPU_WRITE_OFTEN,
            stride: 0,
            rfu0: 0,
            rfu1: 0,
        };

        let mut ptr: *mut AHardwareBuffer = ptr::null_mut();
        let status = unsafe { AHardwareBuffer_allocate(&desc, &mut ptr) };
        if status != 0 || ptr.is_null() {
            return Err(format!(
                "AHardwareBuffer_allocate failed: status={status}, {width}x{height}"
            ));
        }

        // Read back actual stride (may differ from width on some SoCs).
        let mut actual_desc = AHardwareBufferDesc {
            width: 0,
            height: 0,
            layers: 0,
            format: 0,
            usage: 0,
            stride: 0,
            rfu0: 0,
            rfu1: 0,
        };
        unsafe { AHardwareBuffer_describe(ptr, &mut actual_desc) };

        Ok(Self {
            ptr,
            width,
            height,
            stride: actual_desc.stride,
        })
    }

    /// Lock the buffer for CPU write, copy RGBA data, then unlock.
    ///
    /// `rgba` must be exactly `width * height * 4` bytes.  If the buffer's
    /// stride > width, each row is padded accordingly.
    pub fn write_rgba(&self, rgba: &[u8]) -> Result<(), String> {
        let expected = (self.width as usize) * (self.height as usize) * 4;
        if rgba.len() != expected {
            return Err(format!(
                "AHB write_rgba: expected {expected} bytes, got {}",
                rgba.len()
            ));
        }

        let mut vaddr: *mut c_void = ptr::null_mut();
        let status = unsafe {
            AHardwareBuffer_lock(
                self.ptr,
                USAGE_CPU_WRITE_OFTEN,
                -1,          // no fence
                ptr::null(), // entire buffer
                &mut vaddr,
            )
        };
        if status != 0 || vaddr.is_null() {
            return Err(format!("AHardwareBuffer_lock failed: status={status}"));
        }

        let row_bytes = self.width as usize * 4;
        let stride_bytes = self.stride as usize * 4;

        if stride_bytes == row_bytes {
            // No padding — single memcpy.
            unsafe {
                ptr::copy_nonoverlapping(rgba.as_ptr(), vaddr as *mut u8, rgba.len());
            }
        } else {
            // Row-by-row copy respecting stride padding.
            for y in 0..self.height as usize {
                unsafe {
                    let src = rgba.as_ptr().add(y * row_bytes);
                    let dst = (vaddr as *mut u8).add(y * stride_bytes);
                    ptr::copy_nonoverlapping(src, dst, row_bytes);
                }
            }
        }

        let status = unsafe { AHardwareBuffer_unlock(self.ptr, ptr::null_mut()) };
        if status != 0 {
            return Err(format!("AHardwareBuffer_unlock failed: status={status}"));
        }

        Ok(())
    }

    /// Raw pointer for EGL / Vulkan import.
    pub fn as_ptr(&self) -> *mut AHardwareBuffer {
        self.ptr
    }
}

impl Drop for OwnedAHardwareBuffer {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            unsafe { AHardwareBuffer_release(self.ptr) };
        }
    }
}
