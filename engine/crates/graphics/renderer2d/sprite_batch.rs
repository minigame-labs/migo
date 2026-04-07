//! Direct GL sprite batcher for Canvas2D DrawImageBatch.
//!
//! Bypasses femtovg for batched same-texture draws, reducing N draw calls to 1.
//! Used when all entries in a DrawImageBatch share the same native GL texture.
//!
//! Vertex layout per quad: 6 vertices (2 triangles), each vertex = (x, y, u, v).

#![allow(unsafe_op_in_unsafe_fn)]

use glow::HasContext;

/// Pre-allocated vertex buffer for batched sprite rendering.
/// Each sprite = 6 vertices x 4 floats (x, y, u, v) = 24 floats.
///
/// GL resources (program, VAO, VBO) are lazily created on first use and
/// must be cleaned up via `destroy(gl)` before the GL context is torn down.
/// `Renderer2d` owns this and lives for the render thread's lifetime, so
/// cleanup happens in the render thread shutdown path.
pub(crate) struct SpriteBatcher {
    program: Option<glow::Program>,
    vao: Option<glow::VertexArray>,
    vbo: Option<glow::Buffer>,
    /// CPU-side vertex buffer, reused across frames.
    vertices: Vec<f32>,
    /// Max sprites the VBO can hold without reallocation.
    vbo_capacity: usize,
}

impl SpriteBatcher {
    pub fn new() -> Self {
        Self {
            program: None,
            vao: None,
            vbo: None,
            vertices: Vec::with_capacity(24 * 256),
            vbo_capacity: 0,
        }
    }

    /// Ensure shader program and VAO/VBO are created.
    fn ensure_resources(&mut self, gl: &glow::Context) {
        if self.program.is_some() {
            return;
        }

        unsafe {
            let vs = gl.create_shader(glow::VERTEX_SHADER).unwrap();
            gl.shader_source(vs, VERTEX_SHADER);
            gl.compile_shader(vs);

            let fs = gl.create_shader(glow::FRAGMENT_SHADER).unwrap();
            gl.shader_source(fs, FRAGMENT_SHADER);
            gl.compile_shader(fs);

            let prog = gl.create_program().unwrap();
            gl.attach_shader(prog, vs);
            gl.attach_shader(prog, fs);
            // Bind attribute location before linking
            gl.bind_attrib_location(prog, 0, "a_pos_uv");
            gl.link_program(prog);

            gl.delete_shader(vs);
            gl.delete_shader(fs);

            if !gl.get_program_link_status(prog) {
                tracing::error!(
                    "SpriteBatcher shader link error: {}",
                    gl.get_program_info_log(prog)
                );
                gl.delete_program(prog);
                return;
            }

            let vao = gl.create_vertex_array().ok();
            let vbo = gl.create_buffer().ok();

            self.program = Some(prog);
            self.vao = vao;
            self.vbo = vbo;
        }
    }

    /// Batch-draw sprites sharing the same texture.
    ///
    /// `texture` -- the shared GL texture for all sprites
    /// `sprites` -- (sx, sy, sw, sh, dx, dy, dw, dh, img_w, img_h) per sprite
    /// `viewport_w`, `viewport_h` -- canvas dimensions for NDC conversion
    /// `global_alpha` -- current global alpha
    pub fn draw_batch(
        &mut self,
        gl: &glow::Context,
        texture: glow::NativeTexture,
        sprites: &[(f32, f32, f32, f32, f32, f32, f32, f32, f32, f32)],
        viewport_w: f32,
        viewport_h: f32,
        global_alpha: f32,
    ) {
        if sprites.is_empty() {
            return;
        }
        self.ensure_resources(gl);
        let (Some(program), Some(_vao), Some(vbo)) = (self.program, self.vao, self.vbo) else {
            return;
        };

        // Build vertex data: 6 verts per sprite, 4 floats per vert (x, y, u, v)
        self.vertices.clear();
        for &(sx, sy, sw, sh, dx, dy, dw, dh, img_w, img_h) in sprites {
            let u0 = sx / img_w;
            let v0 = sy / img_h;
            let u1 = (sx + sw) / img_w;
            let v1 = (sy + sh) / img_h;

            // Convert pixel coords to NDC: x' = x * 2/w - 1, y' = 1 - y * 2/h
            let x0 = dx * 2.0 / viewport_w - 1.0;
            let y0 = 1.0 - dy * 2.0 / viewport_h;
            let x1 = (dx + dw) * 2.0 / viewport_w - 1.0;
            let y1 = 1.0 - (dy + dh) * 2.0 / viewport_h;

            // Triangle 1: top-left, top-right, bottom-left
            self.vertices
                .extend_from_slice(&[x0, y0, u0, v0, x1, y0, u1, v0, x0, y1, u0, v1]);
            // Triangle 2: top-right, bottom-right, bottom-left
            self.vertices
                .extend_from_slice(&[x1, y0, u1, v0, x1, y1, u1, v1, x0, y1, u0, v1]);
        }

        unsafe {
            // Setup state
            gl.use_program(Some(program));
            gl.bind_vertex_array(self.vao);

            gl.enable(glow::BLEND);
            // Premultiplied alpha (matches femtovg's blending convention).
            gl.blend_func(glow::ONE, glow::ONE_MINUS_SRC_ALPHA);
            gl.disable(glow::DEPTH_TEST);

            // Bind texture
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(texture));

            // Set uniforms
            let tex_loc = gl.get_uniform_location(program, "u_texture");
            gl.uniform_1_i32(tex_loc.as_ref(), 0);
            let alpha_loc = gl.get_uniform_location(program, "u_alpha");
            gl.uniform_1_f32(alpha_loc.as_ref(), global_alpha);

            // Upload vertices
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
            let byte_len = self.vertices.len() * std::mem::size_of::<f32>();
            let byte_data: &[u8] =
                std::slice::from_raw_parts(self.vertices.as_ptr() as *const u8, byte_len);

            let needed = sprites.len();
            if needed > self.vbo_capacity {
                gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, byte_data, glow::DYNAMIC_DRAW);
                self.vbo_capacity = needed;
            } else {
                gl.buffer_sub_data_u8_slice(glow::ARRAY_BUFFER, 0, byte_data);
            }

            // Setup vertex attribs: location 0 = vec4(x, y, u, v)
            gl.enable_vertex_attrib_array(0);
            gl.vertex_attrib_pointer_f32(0, 4, glow::FLOAT, false, 4 * 4, 0);

            // Draw all sprites in one call
            gl.draw_arrays(glow::TRIANGLES, 0, (sprites.len() * 6) as i32);

            // Cleanup - femtovg will rebind its own state on next use
            gl.bind_buffer(glow::ARRAY_BUFFER, None);
            gl.bind_vertex_array(None);
            gl.bind_texture(glow::TEXTURE_2D, None);
            gl.use_program(None);
        }
    }

    pub fn destroy(&mut self, gl: &glow::Context) {
        unsafe {
            if let Some(p) = self.program.take() {
                gl.delete_program(p);
            }
            if let Some(v) = self.vao.take() {
                gl.delete_vertex_array(v);
            }
            if let Some(b) = self.vbo.take() {
                gl.delete_buffer(b);
            }
        }
    }
}

const VERTEX_SHADER: &str = "\
precision highp float;
attribute vec4 a_pos_uv;
varying vec2 v_uv;
void main() {
    gl_Position = vec4(a_pos_uv.xy, 0.0, 1.0);
    v_uv = a_pos_uv.zw;
}
";

const FRAGMENT_SHADER: &str = "\
precision mediump float;
varying vec2 v_uv;
uniform sampler2D u_texture;
uniform float u_alpha;
void main() {
    vec4 color = texture2D(u_texture, v_uv);
    gl_FragColor = color * u_alpha;
}
";
// NOTE: The shader multiplies all channels by u_alpha, which is correct for
// premultiplied-alpha textures (PMA). femtovg outputs PMA textures, and the
// blend func is set to (ONE, ONE_MINUS_SRC_ALPHA) to match.
