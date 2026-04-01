//! GPU adapter detection and selection.

use super::types::*;

/// Check if WebGPU is available on this device.
///
/// Requires Vulkan 1.0 (Android API 24+) or GLES 3.1+.
pub fn is_webgpu_available() -> bool {
    // On Android, check API level and Vulkan support
    // For now, return false as wgpu is not yet integrated
    false
}

/// Get the preferred canvas format for this device.
pub fn get_preferred_canvas_format() -> PreferredCanvasFormat {
    // Most Android devices prefer RGBA8
    PreferredCanvasFormat::Rgba8Unorm
}

/// Adapter capabilities (populated when wgpu is available).
#[derive(Debug, Clone)]
pub struct AdapterCapabilities {
    pub info: GpuAdapterInfo,
    pub limits: GpuLimits,
    pub features: Vec<GpuFeatureName>,
}

impl AdapterCapabilities {
    /// Create a stub adapter for when WebGPU is not available.
    pub fn unavailable() -> Self {
        Self {
            info: GpuAdapterInfo {
                vendor: String::new(),
                architecture: String::new(),
                device: String::new(),
                description: "WebGPU not available".to_string(),
            },
            limits: GpuLimits::default(),
            features: Vec::new(),
        }
    }
}
