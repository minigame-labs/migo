//! Tile-based GPU path rasterizer.
//!
//! Two-pass compute shader architecture:
//! - Pass 1 (coverage): determines which 16x16 tiles each path covers
//! - Pass 2 (composite): renders covered tiles to the output framebuffer

use glow::HasContext;

use super::capabilities::GpuCanvas2DTier;

/// Tile-based GPU path rasterizer backed by GLES 3.1+ compute shaders.
///
/// Manages compute shader programs and SSBO resources for the two-pass
/// tile rendering pipeline. On devices without compute support, all
/// operations are no-ops and [`is_supported`](Self::is_supported) returns false.
pub struct TileRasterizer {
    tier: GpuCanvas2DTier,
    tile_size: i32,
    coverage_program: Option<glow::Program>,
    composite_program: Option<glow::Program>,
    path_ssbo: Option<glow::Buffer>,
    coverage_ssbo: Option<glow::Buffer>,
}

impl TileRasterizer {
    /// Create a new rasterizer for the given capability tier.
    ///
    /// Does not allocate any GL resources -- call [`init`](Self::init) once a
    /// GL context is available.
    pub fn new(tier: GpuCanvas2DTier) -> Self {
        Self {
            tier,
            tile_size: 16,
            coverage_program: None,
            composite_program: None,
            path_ssbo: None,
            coverage_ssbo: None,
        }
    }

    /// Whether this rasterizer can run on the current device.
    pub fn is_supported(&self) -> bool {
        self.tier != GpuCanvas2DTier::Unsupported
    }

    /// The detected capability tier.
    pub fn tier(&self) -> GpuCanvas2DTier {
        self.tier
    }

    /// Tile edge length in pixels.
    pub fn tile_size(&self) -> i32 {
        self.tile_size
    }

    /// Initialize compute shader programs and SSBOs.
    ///
    /// Must be called with a valid GL context current on the calling thread.
    /// Safe to call multiple times -- subsequent calls are no-ops if already
    /// initialized.
    pub fn init(&mut self, gl: &glow::Context) {
        if !self.is_supported() {
            return;
        }
        // Guard against double-init
        if self.coverage_program.is_some() {
            return;
        }

        unsafe {
            self.coverage_program =
                compile_compute(gl, super::shaders::TILE_COVERAGE_SHADER);
            self.composite_program =
                compile_compute(gl, super::shaders::TILE_COMPOSITE_SHADER);
            self.path_ssbo = gl.create_buffer().ok();
            self.coverage_ssbo = gl.create_buffer().ok();
        }

        if self.coverage_program.is_none() || self.composite_program.is_none() {
            tracing::warn!(
                "GPU Canvas2D: compute shader compilation failed, \
                 falling back to femtovg"
            );
            // Clean up partial state
            self.destroy(gl);
            self.tier = GpuCanvas2DTier::Unsupported;
        }
    }

    /// Returns `true` if GL resources have been successfully initialized.
    pub fn is_initialized(&self) -> bool {
        self.coverage_program.is_some() && self.composite_program.is_some()
    }

    /// Release all GL resources.
    ///
    /// Must be called with the same GL context that was used for [`init`](Self::init).
    pub fn destroy(&mut self, gl: &glow::Context) {
        unsafe {
            if let Some(p) = self.coverage_program.take() {
                gl.delete_program(p);
            }
            if let Some(p) = self.composite_program.take() {
                gl.delete_program(p);
            }
            if let Some(b) = self.path_ssbo.take() {
                gl.delete_buffer(b);
            }
            if let Some(b) = self.coverage_ssbo.take() {
                gl.delete_buffer(b);
            }
        }
    }
}

/// Compile a GLSL compute shader source into a linked program.
///
/// Returns `None` on compilation or link failure (errors are logged).
///
/// # Safety
///
/// Caller must ensure a valid GL context is current on this thread.
unsafe fn compile_compute(
    gl: &glow::Context,
    source: &str,
) -> Option<glow::Program> {
    let shader = unsafe { gl.create_shader(glow::COMPUTE_SHADER).ok()? };

    unsafe {
        gl.shader_source(shader, source);
        gl.compile_shader(shader);
    }

    if !unsafe { gl.get_shader_compile_status(shader) } {
        let log = unsafe { gl.get_shader_info_log(shader) };
        tracing::error!("Compute shader compile error: {log}");
        unsafe { gl.delete_shader(shader) };
        return None;
    }

    let program = unsafe { gl.create_program().ok()? };

    unsafe {
        gl.attach_shader(program, shader);
        gl.link_program(program);
        gl.delete_shader(shader);
    }

    if !unsafe { gl.get_program_link_status(program) } {
        let log = unsafe { gl.get_program_info_log(program) };
        tracing::error!("Compute program link error: {log}");
        unsafe { gl.delete_program(program) };
        return None;
    }

    Some(program)
}
