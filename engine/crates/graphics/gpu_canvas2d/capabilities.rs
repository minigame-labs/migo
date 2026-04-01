//! Device capability detection for GPU Canvas2D.

use glow::HasContext;

/// GPU Canvas2D rendering tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuCanvas2DTier {
    /// No compute shader support (GLES 2.0 / 3.0) -- use femtovg
    Unsupported,
    /// GLES 3.1+ with compute shaders
    Compute,
    /// GLES 3.1+ with compute shaders and SSBO (optimal path)
    ComputeWithSsbo,
}

/// Detect the GPU Canvas2D tier from the current GL context.
///
/// Must be called with a valid GL context current on the calling thread.
pub fn detect_tier(gl: &glow::Context) -> GpuCanvas2DTier {
    unsafe {
        let version_str = gl.get_parameter_string(glow::VERSION);

        // Parse GLES version -- need 3.1+ for compute shaders
        let has_31 = version_str.contains("OpenGL ES 3.1")
            || version_str.contains("OpenGL ES 3.2")
            || version_str.contains("4.");

        if !has_31 {
            return GpuCanvas2DTier::Unsupported;
        }

        // Verify compute shader support via work group count
        let max_compute_work_group =
            gl.get_parameter_i32(glow::MAX_COMPUTE_WORK_GROUP_COUNT);
        if max_compute_work_group <= 0 {
            return GpuCanvas2DTier::Unsupported;
        }

        // Check SSBO support (need at least 4 bindings for our two-pass setup)
        let max_ssbo =
            gl.get_parameter_i32(glow::MAX_SHADER_STORAGE_BUFFER_BINDINGS);
        if max_ssbo >= 4 {
            GpuCanvas2DTier::ComputeWithSsbo
        } else {
            GpuCanvas2DTier::Compute
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tier_variants_are_comparable() {
        assert_ne!(GpuCanvas2DTier::Unsupported, GpuCanvas2DTier::Compute);
        assert_ne!(GpuCanvas2DTier::Compute, GpuCanvas2DTier::ComputeWithSsbo);
        assert_eq!(GpuCanvas2DTier::Unsupported, GpuCanvas2DTier::Unsupported);
    }
}
