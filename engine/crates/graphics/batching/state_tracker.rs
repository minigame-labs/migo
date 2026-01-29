//! GPU state tracking to minimize redundant state changes

use glow::HasContext;

/// Tracked GL state to avoid redundant calls
#[derive(Debug, Clone, Default)]
pub struct GLState {
    /// Current bound texture (slot 0)
    bound_texture: Option<glow::NativeTexture>,
    /// Current bound program
    bound_program: Option<glow::NativeProgram>,
    /// Current bound VBO
    bound_vbo: Option<glow::NativeBuffer>,
    /// Current bound IBO
    bound_ibo: Option<glow::NativeBuffer>,
    /// Current bound VAO (if available)
    bound_vao: Option<glow::NativeVertexArray>,
    /// Current viewport
    viewport: (i32, i32, i32, i32),
    /// Current scissor
    scissor: Option<(i32, i32, i32, i32)>,
    /// Scissor test enabled
    scissor_enabled: bool,
    /// Blend enabled
    blend_enabled: bool,
    /// Blend function (src, dst)
    blend_func: (u32, u32),
    /// Depth test enabled
    depth_test_enabled: bool,
    /// Clear color
    clear_color: (f32, f32, f32, f32),
}

impl GLState {
    /// Create a new state tracker
    pub fn new() -> Self {
        Self {
            bound_texture: None,
            bound_program: None,
            bound_vbo: None,
            bound_ibo: None,
            bound_vao: None,
            viewport: (0, 0, 0, 0),
            scissor: None,
            scissor_enabled: false,
            blend_enabled: false,
            blend_func: (glow::ONE, glow::ZERO),
            depth_test_enabled: false,
            clear_color: (0.0, 0.0, 0.0, 1.0),
        }
    }
    
    /// Bind texture if not already bound
    pub fn bind_texture(&mut self, gl: &glow::Context, texture: Option<glow::NativeTexture>) {
        if self.bound_texture != texture {
            unsafe { gl.bind_texture(glow::TEXTURE_2D, texture); }
            self.bound_texture = texture;
        }
    }
    
    /// Bind program if not already bound
    pub fn use_program(&mut self, gl: &glow::Context, program: Option<glow::NativeProgram>) {
        if self.bound_program != program {
            unsafe { gl.use_program(program); }
            self.bound_program = program;
        }
    }
    
    /// Bind VBO if not already bound
    pub fn bind_array_buffer(&mut self, gl: &glow::Context, buffer: Option<glow::NativeBuffer>) {
        if self.bound_vbo != buffer {
            unsafe { gl.bind_buffer(glow::ARRAY_BUFFER, buffer); }
            self.bound_vbo = buffer;
        }
    }
    
    /// Bind IBO if not already bound
    pub fn bind_element_buffer(&mut self, gl: &glow::Context, buffer: Option<glow::NativeBuffer>) {
        if self.bound_ibo != buffer {
            unsafe { gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, buffer); }
            self.bound_ibo = buffer;
        }
    }
    
    /// Bind VAO if available and not already bound
    pub fn bind_vertex_array(&mut self, gl: &glow::Context, vao: Option<glow::NativeVertexArray>) {
        if self.bound_vao != vao {
            unsafe { gl.bind_vertex_array(vao); }
            self.bound_vao = vao;
        }
    }
    
    /// Set viewport if changed
    pub fn set_viewport(&mut self, gl: &glow::Context, x: i32, y: i32, w: i32, h: i32) {
        if self.viewport != (x, y, w, h) {
            unsafe { gl.viewport(x, y, w, h); }
            self.viewport = (x, y, w, h);
        }
    }
    
    /// Set scissor rect
    pub fn set_scissor(&mut self, gl: &glow::Context, scissor: Option<(i32, i32, i32, i32)>) {
        match scissor {
            Some((x, y, w, h)) => {
                if !self.scissor_enabled {
                    unsafe { gl.enable(glow::SCISSOR_TEST); }
                    self.scissor_enabled = true;
                }
                if self.scissor != scissor {
                    unsafe { gl.scissor(x, y, w, h); }
                    self.scissor = scissor;
                }
            }
            None => {
                if self.scissor_enabled {
                    unsafe { gl.disable(glow::SCISSOR_TEST); }
                    self.scissor_enabled = false;
                    self.scissor = None;
                }
            }
        }
    }
    
    /// Enable/disable blending
    pub fn set_blend_enabled(&mut self, gl: &glow::Context, enabled: bool) {
        if self.blend_enabled != enabled {
            if enabled {
                unsafe { gl.enable(glow::BLEND); }
            } else {
                unsafe { gl.disable(glow::BLEND); }
            }
            self.blend_enabled = enabled;
        }
    }
    
    /// Set blend function
    pub fn set_blend_func(&mut self, gl: &glow::Context, src: u32, dst: u32) {
        if self.blend_func != (src, dst) {
            unsafe { gl.blend_func(src, dst); }
            self.blend_func = (src, dst);
        }
    }
    
    /// Setup standard alpha blending
    pub fn set_alpha_blend(&mut self, gl: &glow::Context) {
        self.set_blend_enabled(gl, true);
        self.set_blend_func(gl, glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);
    }
    
    /// Enable/disable depth test
    pub fn set_depth_test_enabled(&mut self, gl: &glow::Context, enabled: bool) {
        if self.depth_test_enabled != enabled {
            if enabled {
                unsafe { gl.enable(glow::DEPTH_TEST); }
            } else {
                unsafe { gl.disable(glow::DEPTH_TEST); }
            }
            self.depth_test_enabled = enabled;
        }
    }
    
    /// Set clear color
    pub fn set_clear_color(&mut self, gl: &glow::Context, r: f32, g: f32, b: f32, a: f32) {
        if self.clear_color != (r, g, b, a) {
            unsafe { gl.clear_color(r, g, b, a); }
            self.clear_color = (r, g, b, a);
        }
    }
    
    /// Reset all tracked state (call after context loss)
    pub fn invalidate(&mut self) {
        *self = Self::new();
    }
    
    /// Get current bound texture
    pub fn current_texture(&self) -> Option<glow::NativeTexture> {
        self.bound_texture
    }
    
    /// Get current viewport
    pub fn current_viewport(&self) -> (i32, i32, i32, i32) {
        self.viewport
    }
}

/// Statistics for state changes
#[derive(Debug, Clone, Default)]
pub struct StateChangeStats {
    pub texture_binds: u32,
    pub program_switches: u32,
    pub buffer_binds: u32,
    pub state_changes: u32,
}

impl StateChangeStats {
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}
