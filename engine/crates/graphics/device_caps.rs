//! Runtime device capability detection and tier classification.
//!
//! Probed once at EGL context creation time.  The resulting `DeviceTier`
//! controls which optimisation paths are enabled for the session lifetime.

use glow::HasContext;

/// Runtime-detected device capabilities.
#[derive(Debug, Clone)]
pub struct DeviceCapabilities {
    pub gles_version: (u32, u32),
    /// `GL_NV_pixel_buffer_object` or ES 3.0+.
    pub has_pbo: bool,
    /// `glFenceSync` available (ES 3.0+).
    pub has_fence_sync: bool,
    /// Compute shaders available (ES 3.1+).
    pub has_compute: bool,
    /// `AHardwareBuffer` available (Android API 26+).
    pub ahb_available: bool,
}

/// Coarse device classification that gates optimisation paths.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceTier {
    /// ES 3.0+, PBO, fence sync — eligible for upload thread, AHB, ring buffer.
    TierA,
    /// ES 2.0 or broken drivers — falls back to current paths (no regression).
    TierB,
}

impl DeviceCapabilities {
    /// Detect capabilities from the current EGL/GL context.
    ///
    /// `egl_extensions` should be the result of `eglQueryString(display, EGL_EXTENSIONS)`.
    /// Pass an empty string if unavailable — AHB detection will fall back to
    /// GL extension check only.
    ///
    /// Must be called with a valid GL context current on the calling thread.
    pub fn detect(gl: &glow::Context, egl_extensions: &str) -> Self {
        let version_str = unsafe { gl.get_parameter_string(glow::VERSION) };
        let gles_version = parse_gles_version(&version_str);

        let gl_extensions = unsafe { gl.get_parameter_string(glow::EXTENSIONS) };
        let has_pbo =
            gles_version >= (3, 0) || gl_extensions.contains("GL_NV_pixel_buffer_object");
        let has_fence_sync = gles_version >= (3, 0);
        let has_compute = gles_version >= (3, 1);

        // AHB texture import requires:
        // - Android API 26+
        // - GL_OES_EGL_image (GL side can consume EGLImage)
        // - EGL_ANDROID_image_native_buffer (EGL side can wrap AHB)
        let ahb_available = cfg!(target_os = "android")
            && android_api_level() >= 26
            && gl_extensions.contains("GL_OES_EGL_image")
            && egl_extensions.contains("EGL_ANDROID_image_native_buffer");

        Self {
            gles_version,
            has_pbo,
            has_fence_sync,
            has_compute,
            ahb_available,
        }
    }

    pub fn tier(&self) -> DeviceTier {
        if self.gles_version >= (3, 0) && self.has_fence_sync && self.has_pbo {
            DeviceTier::TierA
        } else {
            DeviceTier::TierB
        }
    }
}

/// Parse "OpenGL ES X.Y ..." into (X, Y).  Returns (2, 0) on failure.
fn parse_gles_version(version: &str) -> (u32, u32) {
    // Typical: "OpenGL ES 3.1 v1.r28p0-01rel0.xxx"
    // Desktop: "4.6.0 NVIDIA 535.129.03"
    let digits = version
        .split(|c: char| !c.is_ascii_digit() && c != '.')
        .find(|s| s.contains('.'));
    if let Some(d) = digits {
        let mut parts = d.split('.');
        let major = parts.next().and_then(|s| s.parse().ok()).unwrap_or(2);
        let minor = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
        (major, minor)
    } else {
        (2, 0)
    }
}

/// Returns the Android API level at runtime, or 0 on non-Android.
#[cfg(target_os = "android")]
fn android_api_level() -> u32 {
    // android_get_device_api_level() is available in all NDK API levels.
    unsafe extern "C" {
        fn android_get_device_api_level() -> i32;
    }
    let level = unsafe { android_get_device_api_level() };
    level.max(0) as u32
}

#[cfg(not(target_os = "android"))]
fn android_api_level() -> u32 {
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_gles_versions() {
        assert_eq!(parse_gles_version("OpenGL ES 3.1 v1.r28p0"), (3, 1));
        assert_eq!(parse_gles_version("OpenGL ES 3.0"), (3, 0));
        assert_eq!(parse_gles_version("OpenGL ES 2.0"), (2, 0));
        assert_eq!(parse_gles_version("4.6.0 NVIDIA 535"), (4, 6));
        assert_eq!(parse_gles_version("garbage"), (2, 0));
    }
}
