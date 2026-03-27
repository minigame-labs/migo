#![allow(dead_code)]

use jni::sys::jobject;
use raw_window_handle::{
    AndroidDisplayHandle, AndroidNdkWindowHandle, RawDisplayHandle, RawWindowHandle,
};
use shared::surface::Surface;
use std::{ffi::c_void, fmt, ptr::NonNull};

#[repr(C)]
pub struct ANativeWindow(c_void);

#[link(name = "android")]
unsafe extern "C" {
    pub fn ANativeWindow_release(window: *mut ANativeWindow);
    pub fn ANativeWindow_acquire(window: *mut ANativeWindow);
    pub fn ANativeWindow_setBuffersGeometry(
        window: *mut ANativeWindow,
        width: i32,
        height: i32,
        format: i32,
    ) -> i32;

    pub(crate) fn ANativeWindow_fromSurface(
        env: *mut jni::sys::JNIEnv,
        surface: jobject,
    ) -> *mut ANativeWindow;

    pub(crate) fn ANativeWindow_getHeight(window: *mut ANativeWindow) -> i32;
    pub(crate) fn ANativeWindow_getWidth(window: *mut ANativeWindow) -> i32;
}

/// Android Surface wrapper:
/// - Owns an `ANativeWindow*` strong ref.
/// - Releases it in `Drop`.
/// - Stores physical pixel size.
pub struct AndroidSurfaceWrapper {
    handle: NonNull<ANativeWindow>,
    dimension: (u32, u32), // physical pixels
}

unsafe impl Send for AndroidSurfaceWrapper {}
unsafe impl Sync for AndroidSurfaceWrapper {}

impl fmt::Debug for AndroidSurfaceWrapper {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AndroidSurfaceWrapper")
            .field("handle", &format_args!("{:p}", self.handle.as_ptr()))
            .field("dimension", &self.dimension)
            .finish()
    }
}

impl AndroidSurfaceWrapper {
    /// Use this when you already own a strong ref (e.g. `ANativeWindow_fromSurface()`).
    /// Do NOT call `acquire()` again.
    pub unsafe fn from_surface_owned(
        handle: *mut ANativeWindow,
        width: u32,
        height: u32,
    ) -> Result<Self, &'static str> {
        let handle = NonNull::new(handle).ok_or("ANativeWindow pointer is null")?;
        Ok(Self {
            handle,
            dimension: (width, height),
        })
    }

    /// Use this when you only borrowed a pointer and must acquire a strong ref.
    pub unsafe fn from_borrowed_acquire(
        handle: *mut ANativeWindow,
        width: u32,
        height: u32,
    ) -> Result<Self, &'static str> {
        let handle = NonNull::new(handle).ok_or("ANativeWindow pointer is null")?;
        unsafe { ANativeWindow_acquire(handle.as_ptr()) };
        Ok(Self {
            handle,
            dimension: (width, height),
        })
    }

    #[inline]
    pub fn native_handle(&self) -> *mut ANativeWindow {
        self.handle.as_ptr()
    }

    #[inline]
    pub fn size_physical(&self) -> (u32, u32) {
        self.dimension
    }

    #[inline]
    pub fn set_size_physical(&mut self, width: u32, height: u32) {
        self.dimension = (width, height);
    }

    pub fn set_buffers_geometry(&self, width: i32, height: i32, format: i32) -> Result<(), String> {
        let r = unsafe {
            ANativeWindow_setBuffersGeometry(self.handle.as_ptr(), width, height, format)
        };
        if r == 0 {
            Ok(())
        } else {
            Err(format!("ANativeWindow_setBuffersGeometry failed: code={r}"))
        }
    }
}

impl Drop for AndroidSurfaceWrapper {
    fn drop(&mut self) {
        unsafe { ANativeWindow_release(self.handle.as_ptr()) };
    }
}

impl Surface for AndroidSurfaceWrapper {
    fn size(&self) -> (u32, u32) {
        self.dimension
    }

    fn raw_window_handle(&self) -> RawWindowHandle {
        // raw_window_handle expects NonNull<c_void>
        let ptr: NonNull<c_void> = self.handle.cast::<c_void>();
        RawWindowHandle::AndroidNdk(AndroidNdkWindowHandle::new(ptr))
    }

    fn raw_display_handle(&self) -> RawDisplayHandle {
        RawDisplayHandle::Android(AndroidDisplayHandle::new())
    }
}
