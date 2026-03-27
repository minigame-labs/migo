use glow::{HasContext, NativeUniformLocation};
use shared::{
    error::{EngineError, EngineResult, ErrorCode},
    protocol::render_cmd::{CanvasId, FramebufferId, GLCmd, RenderbufferId, ShaderType, TextureId},
};
use tracing::trace;

use crate::CanvasManager;

#[inline]
fn ee(code: ErrorCode, detail: impl Into<String>) -> EngineError {
    EngineError::from_detail(code, detail)
}

#[inline]
fn to_native_uniform_location(location: Option<u32>) -> Option<NativeUniformLocation> {
    location.map(NativeUniformLocation)
}

/// Identity after logical/physical coordinate unification.
/// Kept as a wrapper so the Scissor call site reads clearly.
#[inline]
fn logical_to_physical_i32(_cm: &CanvasManager, v: i32) -> i32 {
    v
}

pub(crate) struct RendererGL;

impl RendererGL {
    pub(crate) fn new() -> Self {
        Self
    }

    #[inline]
    fn bind_for_contextless_gl(&mut self, cm: &mut CanvasManager) -> EngineResult<CanvasId> {
        cm.ensure_any_canvas_current()
    }

    fn current_owner_canvas(cm: &CanvasManager) -> Option<CanvasId> {
        cm.current_canvas_id()
    }

    /// Process a single GL command.
    ///
    /// PERF: Per-command `make_current_needed` overhead.
    /// Every per-canvas GL command calls `cm.make_current_needed(canvas_id)`.
    /// In the common single-canvas case this is a cheap `BoundContext`
    /// enum comparison (already O(1) short-circuit when the canvas is
    /// already current).  In multi-canvas scenarios, consecutive commands
    /// targeting the same canvas also short-circuit after the first call.
    /// The only real EGL cost (`eglMakeCurrent`) is paid on actual canvas
    /// switches, which are rare within a single batch.
    pub(crate) fn handle_command(
        &mut self,
        cm: &mut CanvasManager,
        gl: &glow::Context,
        cmd: GLCmd,
    ) -> EngineResult<bool> {
        match cmd {
            // ---------- Per-canvas stateful calls ----------
            GLCmd::Viewport {
                canvas_id,
                x,
                y,
                width,
                height,
            } => {
                cm.make_current_needed(canvas_id)?;
                // Values are in physical (buffer) pixels — no DPR scaling needed,
                // matching browser WebGL semantics.
                unsafe { gl.viewport(x, y, width as i32, height as i32) };
                Ok(false)
            }

            GLCmd::Clear {
                canvas_id,
                bit_field,
            } => {
                cm.make_current_needed(canvas_id)?;
                unsafe { gl.clear(bit_field) };
                Ok(true)
            }

            GLCmd::ClearColor {
                canvas_id,
                r,
                g,
                b,
                a,
            } => {
                cm.make_current_needed(canvas_id)?;
                unsafe { gl.clear_color(r, g, b, a) };
                Ok(false)
            }

            // ---------- Program (stateful) ----------
            GLCmd::UseProgram {
                canvas_id,
                program_id,
            } => {
                cm.make_current_needed(canvas_id)?;
                let meta = cm.programs.get(&program_id).ok_or_else(|| {
                    ee(
                        ErrorCode::NotFound,
                        format!("program not found: {program_id:?}"),
                    )
                })?;
                cm.check_owner(meta.owner_canvas, canvas_id, "program")?;

                if meta.deleted {
                    shared::bail!(
                        ErrorCode::InvalidOperation,
                        "use_program on deleted program"
                    );
                }

                if let Some(ph) = meta.gl_handle {
                    unsafe { gl.use_program(Some(ph)) };
                    cm.gl_state.entry(canvas_id).or_default().current_program = Some(program_id);
                }
                Ok(false)
            }

            GLCmd::GetAttribLocation {
                canvas_id,
                program_id,
                name,
                resp,
            } => {
                cm.make_current_needed(canvas_id)?;
                let meta = cm.programs.get(&program_id).ok_or_else(|| {
                    ee(
                        ErrorCode::NotFound,
                        format!("program not found: {program_id:?}"),
                    )
                })?;
                if let Err(e) = cm.check_owner(meta.owner_canvas, canvas_id, "program") {
                    let _ = resp.send(Err(e));
                    return Ok(false);
                }
                if meta.deleted {
                    let _ = resp.send(Ok(None));
                    return Ok(false);
                }

                if let Some(ph) = meta.gl_handle {
                    unsafe {
                        let loc = gl.get_attrib_location(ph, &name);
                        let _ = resp.send(Ok(loc));
                    }
                } else {
                    let _ = resp.send(Ok(None));
                }
                Ok(false)
            }

            GLCmd::GetActiveAttrib {
                canvas_id,
                program_id,
                index,
                resp,
            } => {
                cm.make_current_needed(canvas_id)?;
                let meta = cm.programs.get(&program_id).ok_or_else(|| {
                    ee(
                        ErrorCode::NotFound,
                        format!("program not found: {program_id:?}"),
                    )
                })?;
                if let Err(e) = cm.check_owner(meta.owner_canvas, canvas_id, "program") {
                    let _ = resp.send(Err(e));
                    return Ok(false);
                }
                if meta.deleted {
                    let _ = resp.send(Ok(None));
                    return Ok(false);
                }

                if let Some(ph) = meta.gl_handle {
                    unsafe {
                        let info = gl
                            .get_active_attribute(ph, index)
                            .map(|it| (it.name, it.size, it.atype));
                        let _ = resp.send(Ok(info));
                    }
                } else {
                    let _ = resp.send(Ok(None));
                }
                Ok(false)
            }

            GLCmd::GetActiveUniform {
                canvas_id,
                program_id,
                index,
                resp,
            } => {
                cm.make_current_needed(canvas_id)?;
                let meta = cm.programs.get(&program_id).ok_or_else(|| {
                    ee(
                        ErrorCode::NotFound,
                        format!("program not found: {program_id:?}"),
                    )
                })?;
                if let Err(e) = cm.check_owner(meta.owner_canvas, canvas_id, "program") {
                    let _ = resp.send(Err(e));
                    return Ok(false);
                }
                if meta.deleted {
                    let _ = resp.send(Ok(None));
                    return Ok(false);
                }

                if let Some(ph) = meta.gl_handle {
                    unsafe {
                        let info = gl
                            .get_active_uniform(ph, index)
                            .map(|it| (it.name, it.size, it.utype));
                        let _ = resp.send(Ok(info));
                    }
                } else {
                    let _ = resp.send(Ok(None));
                }
                Ok(false)
            }

            GLCmd::EnableVertexAttribArray { canvas_id, index } => {
                cm.make_current_needed(canvas_id)?;
                unsafe { gl.enable_vertex_attrib_array(index) };
                Ok(false)
            }

            GLCmd::VertexAttribPointer {
                canvas_id,
                index,
                size,
                type_,
                normalized,
                stride,
                offset,
            } => {
                cm.make_current_needed(canvas_id)?;
                trace!(
                    "VertexAttribPointer: canvas={:?}, index={}, size={}, type={}, norm={}, stride={}, offset={}",
                    canvas_id, index, size, type_, normalized, stride, offset
                );
                unsafe {
                    gl.vertex_attrib_pointer_f32(index, size, type_, normalized, stride, offset);
                }
                Ok(false)
            }

            GLCmd::GetUniformLocation {
                canvas_id,
                program_id,
                name,
                resp,
            } => {
                cm.make_current_needed(canvas_id)?;
                let meta = cm.programs.get(&program_id).ok_or_else(|| {
                    ee(
                        ErrorCode::NotFound,
                        format!("program not found: {program_id:?}"),
                    )
                })?;
                if let Err(e) = cm.check_owner(meta.owner_canvas, canvas_id, "program") {
                    let _ = resp.send(Err(e));
                    return Ok(false);
                }
                if meta.deleted {
                    let _ = resp.send(Ok(None));
                    return Ok(false);
                }

                if let Some(ph) = meta.gl_handle {
                    unsafe {
                        let loc = gl.get_uniform_location(ph, &name);
                        let raw = loc.map(|l| l.0); // NativeUniformLocation(pub u32)
                        let _ = resp.send(Ok(raw));
                    }
                } else {
                    let _ = resp.send(Ok(None));
                }
                Ok(false)
            }

            GLCmd::Uniform3f {
                canvas_id,
                location,
                x,
                y,
                z,
            } => {
                cm.make_current_needed(canvas_id)?;
                unsafe { gl.uniform_3_f32(to_native_uniform_location(location).as_ref(), x, y, z) };
                Ok(false)
            }

            GLCmd::UniformMatrix3fv {
                canvas_id,
                location,
                transpose,
                value,
            } => {
                cm.make_current_needed(canvas_id)?;
                unsafe {
                    gl.uniform_matrix_3_f32_slice(
                        to_native_uniform_location(location).as_ref(),
                        transpose,
                        &value,
                    )
                };
                Ok(false)
            }

            GLCmd::DrawArrays {
                canvas_id,
                mode,
                first,
                count,
            } => {
                cm.make_current_needed(canvas_id)?;
                unsafe { gl.draw_arrays(mode, first, count) };
                Ok(true)
            }

            GLCmd::DrawElements {
                canvas_id,
                mode,
                count,
                index_type,
                offset,
            } => {
                cm.make_current_needed(canvas_id)?;
                unsafe { gl.draw_elements(mode, count, index_type, offset) };
                Ok(true)
            }

            // ---------- Buffers (stateful) ----------
            GLCmd::BindBuffer {
                canvas_id,
                target,
                buffer,
            } => {
                cm.make_current_needed(canvas_id)?;
                let native = if let Some(id) = buffer {
                    let meta = cm.buffers.get(&id).ok_or_else(|| {
                        ee(ErrorCode::NotFound, format!("buffer not found: {id:?}"))
                    })?;
                    cm.check_owner(meta.owner_canvas, canvas_id, "buffer")?;
                    if meta.deleted {
                        shared::bail!(ErrorCode::InvalidOperation, "bind_buffer on deleted buffer");
                    }
                    meta.gl_handle
                } else {
                    None
                };
                unsafe { gl.bind_buffer(target, native) };
                Ok(false)
            }

            GLCmd::BufferData {
                canvas_id,
                target,
                size,
                data,
                usage,
            } => {
                cm.make_current_needed(canvas_id)?;
                unsafe {
                    if let Some(data) = data {
                        if data.is_empty() {
                            gl.buffer_data_size(target, 0, usage);
                        } else {
                            gl.buffer_data_u8_slice(target, &data, usage);
                        }
                    } else {
                        gl.buffer_data_size(target, size, usage);
                    }
                }
                Ok(false)
            }

            // ---------- Context-less-ish calls (need some current context) ----------
            // Program
            GLCmd::CreateProgram { client_id } => {
                let _ = self.bind_for_contextless_gl(cm)?;
                let owner = Self::current_owner_canvas(cm);

                unsafe {
                    match gl.create_program() {
                        Ok(p) => {
                            cm.programs.insert(
                                client_id,
                                crate::canvas::ProgramMeta {
                                    gl_handle: Some(p),
                                    owner_canvas: owner,
                                    deleted: false,
                                },
                            );
                        }
                        Err(e) => {
                            tracing::error!("gl.create_program failed for id {client_id}: {e:?}");
                        }
                    }
                }
                Ok(false)
            }

            GLCmd::LinkProgram { program_id } => {
                let _ = self.bind_for_contextless_gl(cm)?;
                if let Some(meta) = cm.programs.get(&program_id) {
                    if !meta.deleted {
                        if let Some(ph) = meta.gl_handle {
                            unsafe { gl.link_program(ph) };
                        }
                    }
                }
                Ok(false)
            }

            GLCmd::GetProgramParameter {
                program_id,
                pname,
                resp,
            } => {
                let _ = self.bind_for_contextless_gl(cm)?;
                let Some(meta) = cm.programs.get(&program_id) else {
                    let _ = resp.send(Err(ee(
                        ErrorCode::NotFound,
                        format!("program not found: {program_id:?}"),
                    )));
                    return Ok(false);
                };

                if meta.deleted {
                    let _ = resp.send(Ok(0));
                    return Ok(false);
                }

                let Some(ph) = meta.gl_handle else {
                    let _ = resp.send(Ok(0));
                    return Ok(false);
                };

                let v: i32 = unsafe {
                    match pname {
                        glow::LINK_STATUS => {
                            if gl.get_program_link_status(ph) {
                                1
                            } else {
                                0
                            }
                        }
                        glow::INFO_LOG_LENGTH => gl.get_program_info_log(ph).len() as i32,
                        _ => gl.get_program_parameter_i32(ph, pname),
                    }
                };

                let _ = resp.send(Ok(v));
                Ok(false)
            }

            GLCmd::GetProgramInfoLog { program_id, resp } => {
                let _ = self.bind_for_contextless_gl(cm)?;
                if let Some(meta) = cm.programs.get(&program_id) {
                    if meta.deleted {
                        let _ = resp.send(Ok(None));
                        return Ok(false);
                    }
                    if let Some(ph) = meta.gl_handle {
                        unsafe {
                            let log = gl.get_program_info_log(ph);
                            let _ = resp.send(Ok(Some(log)));
                        }
                    } else {
                        let _ = resp.send(Ok(None));
                    }
                } else {
                    let _ = resp.send(Err(ee(
                        ErrorCode::NotFound,
                        format!("program not found: {program_id:?}"),
                    )));
                }
                Ok(false)
            }

            GLCmd::DeleteProgram { program_id } => {
                let _ = self.bind_for_contextless_gl(cm)?;
                if let Some(mut meta) = cm.programs.remove(&program_id) {
                    meta.deleted = true;
                    if let Some(ph) = meta.gl_handle {
                        unsafe { gl.delete_program(ph) };
                    }
                }
                Ok(false)
            }

            // Shader
            GLCmd::CreateShader {
                client_id,
                shader_type,
            } => {
                let _ = self.bind_for_contextless_gl(cm)?;
                let owner = Self::current_owner_canvas(cm);

                let gl_ty = match shader_type {
                    ShaderType::Vertex => glow::VERTEX_SHADER,
                    ShaderType::Fragment => glow::FRAGMENT_SHADER,
                };

                unsafe {
                    match gl.create_shader(gl_ty) {
                        Ok(s) => {
                            cm.shaders.insert(
                                client_id,
                                crate::canvas::ShaderMeta {
                                    gl_handle: Some(s),
                                    owner_canvas: owner,
                                    shader_type,
                                    gl_shader_type: gl_ty,
                                    deleted: false,
                                    source_len: 0,
                                },
                            );
                        }
                        Err(e) => {
                            tracing::error!("gl.create_shader failed for id {client_id}: {e:?}");
                        }
                    }
                }
                Ok(false)
            }

            GLCmd::ShaderSource {
                shader_id,
                source,
                resp,
            } => {
                let _ = self.bind_for_contextless_gl(cm)?;
                let Some(meta) = cm.shaders.get_mut(&shader_id) else {
                    if let Some(r) = resp {
                        r.send(Err(ee(
                            ErrorCode::NotFound,
                            format!("shader not found: {shader_id:?}"),
                        )));
                    }
                    return Ok(false);
                };

                if meta.deleted {
                    if let Some(r) = resp {
                        r.send(Err(ee(
                            ErrorCode::InvalidOperation,
                            "shader already deleted",
                        )));
                    }
                    return Ok(false);
                }

                if let Some(sh) = meta.gl_handle {
                    meta.source_len = source.len();
                    unsafe { gl.shader_source(sh, &source) };
                    if let Some(r) = resp {
                        r.send(Ok(()));
                    }
                } else if let Some(r) = resp {
                    r.send(Err(ee(
                        ErrorCode::InvalidOperation,
                        "shader handle missing",
                    )));
                }
                Ok(false)
            }

            GLCmd::CompileShader { shader_id } => {
                let _ = self.bind_for_contextless_gl(cm)?;
                if let Some(meta) = cm.shaders.get(&shader_id) {
                    if !meta.deleted {
                        if let Some(sh) = meta.gl_handle {
                            unsafe { gl.compile_shader(sh) };
                        }
                    }
                }
                Ok(false)
            }

            GLCmd::AttachShader {
                program_id,
                shader_id,
                resp,
            } => {
                let _ = self.bind_for_contextless_gl(cm)?;
                let p = cm.programs.get(&program_id).ok_or_else(|| {
                    ee(
                        ErrorCode::NotFound,
                        format!("program not found: {program_id:?}"),
                    )
                })?;
                let s = cm.shaders.get(&shader_id).ok_or_else(|| {
                    ee(
                        ErrorCode::NotFound,
                        format!("shader not found: {shader_id:?}"),
                    )
                })?;

                if p.deleted || s.deleted {
                    if let Some(r) = resp {
                        r.send(Err(ee(
                            ErrorCode::InvalidOperation,
                            "program/shader deleted",
                        )));
                    }
                    return Ok(false);
                }

                // WebGL-ish: must belong to same owner canvas
                if p.owner_canvas != s.owner_canvas {
                    if let Some(r) = resp {
                        r.send(Err(ee(
                            ErrorCode::InvalidOperation,
                            "attach shader across different contexts",
                        )));
                    }
                    return Ok(false);
                }

                if let (Some(ph), Some(sh)) = (p.gl_handle, s.gl_handle) {
                    unsafe { gl.attach_shader(ph, sh) };
                    if let Some(r) = resp {
                        r.send(Ok(()));
                    }
                } else if let Some(r) = resp {
                    r.send(Err(ee(
                        ErrorCode::InvalidOperation,
                        "program/shader handle missing",
                    )));
                }
                Ok(false)
            }

            GLCmd::GetShaderParameter {
                shader_id,
                pname,
                resp,
            } => {
                let _ = self.bind_for_contextless_gl(cm)?;
                let Some(meta) = cm.shaders.get(&shader_id) else {
                    let _ = resp.send(Err(ee(
                        ErrorCode::NotFound,
                        format!("shader not found: {shader_id:?}"),
                    )));
                    return Ok(false);
                };

                if meta.deleted {
                    let _ = resp.send(Ok(0));
                    return Ok(false);
                }

                let Some(sh) = meta.gl_handle else {
                    let _ = resp.send(Ok(0));
                    return Ok(false);
                };

                let v: i32 = match pname {
                    glow::COMPILE_STATUS => unsafe {
                        if gl.get_shader_compile_status(sh) {
                            1
                        } else {
                            0
                        }
                    },
                    glow::SHADER_TYPE => meta.gl_shader_type as i32,
                    glow::DELETE_STATUS => {
                        if meta.deleted {
                            1
                        } else {
                            0
                        }
                    }
                    glow::INFO_LOG_LENGTH => unsafe { gl.get_shader_info_log(sh).len() as i32 },
                    glow::SHADER_SOURCE_LENGTH => meta.source_len as i32,

                    _ => {
                        let _ = resp.send(Err(ee(
                            ErrorCode::InvalidArgument,
                            format!("GetShaderParameter: unsupported pname=0x{pname:04x}"),
                        )));
                        return Ok(false);
                    }
                };

                let _ = resp.send(Ok(v));
                Ok(false)
            }

            GLCmd::GetShaderInfoLog { shader_id, resp } => {
                let _ = self.bind_for_contextless_gl(cm)?;
                if let Some(meta) = cm.shaders.get(&shader_id) {
                    if meta.deleted {
                        let _ = resp.send(Ok(None));
                        return Ok(false);
                    }
                    if let Some(sh) = meta.gl_handle {
                        unsafe {
                            let log = gl.get_shader_info_log(sh);
                            let _ = resp.send(Ok(Some(log)));
                        }
                    } else {
                        let _ = resp.send(Ok(None));
                    }
                } else {
                    let _ = resp.send(Err(ee(
                        ErrorCode::NotFound,
                        format!("shader not found: {shader_id:?}"),
                    )));
                }
                Ok(false)
            }

            GLCmd::DeleteShader { shader_id } => {
                let _ = self.bind_for_contextless_gl(cm)?;
                if let Some(mut meta) = cm.shaders.remove(&shader_id) {
                    meta.deleted = true;
                    if let Some(sh) = meta.gl_handle {
                        unsafe { gl.delete_shader(sh) };
                    }
                }
                Ok(false)
            }

            // Buffers
            GLCmd::CreateBuffer { client_id } => {
                let _ = self.bind_for_contextless_gl(cm)?;
                let owner = Self::current_owner_canvas(cm);

                unsafe {
                    match gl.create_buffer() {
                        Ok(buf) => {
                            cm.buffers.insert(
                                client_id,
                                crate::canvas::BufferMeta {
                                    gl_handle: Some(buf),
                                    owner_canvas: owner,
                                    deleted: false,
                                },
                            );
                        }
                        Err(e) => {
                            tracing::error!("gl.create_buffer failed for id {client_id}: {e:?}");
                        }
                    }
                }
                Ok(false)
            }

            // ========== Phase 1A: GL State ==========
            GLCmd::Enable { canvas_id, cap } => {
                cm.make_current_needed(canvas_id)?;
                unsafe { gl.enable(cap) };
                Ok(false)
            }

            GLCmd::Disable { canvas_id, cap } => {
                cm.make_current_needed(canvas_id)?;
                unsafe { gl.disable(cap) };
                Ok(false)
            }

            GLCmd::IsEnabled {
                canvas_id,
                cap,
                resp,
            } => {
                cm.make_current_needed(canvas_id)?;
                let val = unsafe { gl.is_enabled(cap) };
                let _ = resp.send(Ok(val));
                Ok(false)
            }

            GLCmd::GetParameter {
                canvas_id,
                pname,
                resp,
            } => {
                cm.make_current_needed(canvas_id)?;
                let json = unsafe {
                    match pname {
                        // String params: VENDOR / RENDERER / VERSION / SHADING_LANGUAGE_VERSION
                        glow::VENDOR
                        | glow::RENDERER
                        | glow::VERSION
                        | glow::SHADING_LANGUAGE_VERSION => {
                            let val = gl.get_parameter_string(pname);
                            format!("\"{}\"", val)
                        }
                        // Boolean params
                        glow::DEPTH_WRITEMASK
                        | glow::SAMPLE_COVERAGE_INVERT
                        | glow::DITHER
                        | glow::BLEND
                        | glow::CULL_FACE
                        | glow::DEPTH_TEST
                        | glow::POLYGON_OFFSET_FILL
                        | glow::SAMPLE_ALPHA_TO_COVERAGE
                        | glow::SAMPLE_COVERAGE
                        | glow::SCISSOR_TEST
                        | glow::STENCIL_TEST => {
                            let val = gl.get_parameter_i32(pname) != 0;
                            if val {
                                "true".to_string()
                            } else {
                                "false".to_string()
                            }
                        }
                        // Float params
                        glow::DEPTH_CLEAR_VALUE
                        | glow::LINE_WIDTH
                        | glow::POLYGON_OFFSET_FACTOR
                        | glow::POLYGON_OFFSET_UNITS
                        | glow::SAMPLE_COVERAGE_VALUE => {
                            let val = gl.get_parameter_f32(pname);
                            format!("{}", val)
                        }
                        // Float32Array[4] params
                        glow::COLOR_CLEAR_VALUE | glow::BLEND_COLOR => {
                            let mut buf = [0f32; 4];
                            gl.get_parameter_f32_slice(pname, &mut buf);
                            format!("[{},{},{},{}]", buf[0], buf[1], buf[2], buf[3])
                        }
                        // Int32Array[4] params
                        glow::VIEWPORT | glow::SCISSOR_BOX => {
                            let mut buf = [0i32; 4];
                            gl.get_parameter_i32_slice(pname, &mut buf);
                            format!("[{},{},{},{}]", buf[0], buf[1], buf[2], buf[3])
                        }
                        // Boolean[4] param
                        glow::COLOR_WRITEMASK => {
                            let mut buf = [0i32; 4];
                            gl.get_parameter_i32_slice(pname, &mut buf);
                            format!(
                                "[{},{},{},{}]",
                                buf[0] != 0,
                                buf[1] != 0,
                                buf[2] != 0,
                                buf[3] != 0
                            )
                        }
                        // Float32Array[2] params
                        glow::DEPTH_RANGE
                        | glow::ALIASED_LINE_WIDTH_RANGE
                        | glow::ALIASED_POINT_SIZE_RANGE => {
                            let mut buf = [0f32; 2];
                            gl.get_parameter_f32_slice(pname, &mut buf);
                            format!("[{},{}]", buf[0], buf[1])
                        }
                        // Default: integer params (MAX_TEXTURE_SIZE, MAX_VERTEX_ATTRIBS, etc.)
                        _ => {
                            let val = gl.get_parameter_i32(pname);
                            format!("{}", val)
                        }
                    }
                };
                let _ = resp.send(Ok(json));
                Ok(false)
            }

            // ========== Phase 1B: Textures ==========
            GLCmd::CreateTexture { client_id } => {
                let _ = self.bind_for_contextless_gl(cm)?;
                let owner = Self::current_owner_canvas(cm);
                unsafe {
                    match gl.create_texture() {
                        Ok(tex) => {
                            cm.textures.insert(
                                client_id,
                                crate::canvas::TextureMeta {
                                    gl_handle: Some(tex),
                                    owner_canvas: owner,
                                    deleted: false,
                                },
                            );
                        }
                        Err(e) => {
                            tracing::error!("gl.create_texture failed for id {client_id}: {e:?}")
                        }
                    }
                }
                Ok(false)
            }

            GLCmd::DeleteTexture { texture_id } => {
                let _ = self.bind_for_contextless_gl(cm)?;
                if let Some(meta) = cm.textures.remove(&texture_id) {
                    if let Some(h) = meta.gl_handle {
                        unsafe { gl.delete_texture(h) };
                    }
                }
                Ok(false)
            }

            GLCmd::BindTexture {
                canvas_id,
                target,
                texture,
            } => {
                cm.make_current_needed(canvas_id)?;
                let native = if let Some(id) = texture {
                    let meta = cm.textures.get(&id).ok_or_else(|| {
                        ee(ErrorCode::NotFound, format!("texture not found: {id:?}"))
                    })?;
                    if meta.deleted {
                        shared::bail!(
                            ErrorCode::InvalidOperation,
                            "bind_texture on deleted texture"
                        );
                    }
                    meta.gl_handle
                } else {
                    None
                };
                unsafe { gl.bind_texture(target, native) };
                Ok(false)
            }

            GLCmd::ActiveTexture { canvas_id, unit } => {
                cm.make_current_needed(canvas_id)?;
                unsafe { gl.active_texture(unit) };
                Ok(false)
            }

            GLCmd::TexImage2D {
                canvas_id,
                target,
                level,
                internalformat,
                width,
                height,
                border,
                format,
                type_,
                data,
            } => {
                cm.make_current_needed(canvas_id)?;
                unsafe {
                    gl.tex_image_2d(
                        target,
                        level,
                        internalformat,
                        width,
                        height,
                        border,
                        format,
                        type_,
                        glow::PixelUnpackData::Slice(data.as_deref()),
                    );
                }
                Ok(false)
            }

            GLCmd::TexSubImage2D {
                canvas_id,
                target,
                level,
                xoffset,
                yoffset,
                width,
                height,
                format,
                type_,
                data,
            } => {
                cm.make_current_needed(canvas_id)?;
                unsafe {
                    gl.tex_sub_image_2d(
                        target,
                        level,
                        xoffset,
                        yoffset,
                        width,
                        height,
                        format,
                        type_,
                        glow::PixelUnpackData::Slice(Some(&data)),
                    );
                }
                Ok(false)
            }

            GLCmd::TexParameteri {
                canvas_id,
                target,
                pname,
                param,
            } => {
                cm.make_current_needed(canvas_id)?;
                unsafe { gl.tex_parameter_i32(target, pname, param) };
                Ok(false)
            }

            GLCmd::TexParameterf {
                canvas_id,
                target,
                pname,
                param,
            } => {
                cm.make_current_needed(canvas_id)?;
                unsafe { gl.tex_parameter_f32(target, pname, param) };
                Ok(false)
            }

            GLCmd::GenerateMipmap { canvas_id, target } => {
                cm.make_current_needed(canvas_id)?;
                unsafe { gl.generate_mipmap(target) };
                Ok(false)
            }

            GLCmd::PixelStorei {
                canvas_id,
                pname,
                param,
            } => {
                cm.make_current_needed(canvas_id)?;
                unsafe { gl.pixel_store_i32(pname, param) };
                Ok(false)
            }

            GLCmd::CompressedTexImage2D {
                canvas_id,
                target,
                level,
                internalformat,
                width,
                height,
                border,
                data,
            } => {
                cm.make_current_needed(canvas_id)?;
                unsafe {
                    gl.compressed_tex_image_2d(
                        target,
                        level,
                        internalformat as i32,
                        width,
                        height,
                        border,
                        data.len() as i32,
                        &data,
                    );
                }
                Ok(false)
            }

            GLCmd::CompressedTexSubImage2D {
                canvas_id,
                target,
                level,
                xoffset,
                yoffset,
                width,
                height,
                format,
                data,
            } => {
                cm.make_current_needed(canvas_id)?;
                unsafe {
                    gl.compressed_tex_sub_image_2d(
                        target,
                        level,
                        xoffset,
                        yoffset,
                        width,
                        height,
                        format,
                        glow::CompressedPixelUnpackData::Slice(&data),
                    );
                }
                Ok(false)
            }

            // ========== Phase 1C: Buffer & Vertex Extensions ==========
            GLCmd::BufferSubData {
                canvas_id,
                target,
                offset,
                data,
            } => {
                cm.make_current_needed(canvas_id)?;
                unsafe { gl.buffer_sub_data_u8_slice(target, offset, &data) };
                Ok(false)
            }

            GLCmd::DisableVertexAttribArray { canvas_id, index } => {
                cm.make_current_needed(canvas_id)?;
                unsafe { gl.disable_vertex_attrib_array(index) };
                Ok(false)
            }

            GLCmd::ClearDepth { canvas_id, depth } => {
                cm.make_current_needed(canvas_id)?;
                unsafe { gl.clear_depth_f32(depth) };
                Ok(false)
            }

            GLCmd::ClearStencil { canvas_id, s } => {
                cm.make_current_needed(canvas_id)?;
                unsafe { gl.clear_stencil(s) };
                Ok(false)
            }

            // ========== Phase 2A: Blend/Depth/Stencil/Cull ==========
            GLCmd::BlendFunc {
                canvas_id,
                sfactor,
                dfactor,
            } => {
                cm.make_current_needed(canvas_id)?;
                unsafe { gl.blend_func(sfactor, dfactor) };
                Ok(false)
            }

            GLCmd::BlendFuncSeparate {
                canvas_id,
                src_rgb,
                dst_rgb,
                src_alpha,
                dst_alpha,
            } => {
                cm.make_current_needed(canvas_id)?;
                unsafe { gl.blend_func_separate(src_rgb, dst_rgb, src_alpha, dst_alpha) };
                Ok(false)
            }

            GLCmd::BlendEquation { canvas_id, mode } => {
                cm.make_current_needed(canvas_id)?;
                unsafe { gl.blend_equation(mode) };
                Ok(false)
            }

            GLCmd::BlendEquationSeparate {
                canvas_id,
                mode_rgb,
                mode_alpha,
            } => {
                cm.make_current_needed(canvas_id)?;
                unsafe { gl.blend_equation_separate(mode_rgb, mode_alpha) };
                Ok(false)
            }

            GLCmd::BlendColor {
                canvas_id,
                r,
                g,
                b,
                a,
            } => {
                cm.make_current_needed(canvas_id)?;
                unsafe { gl.blend_color(r, g, b, a) };
                Ok(false)
            }

            GLCmd::DepthFunc { canvas_id, func } => {
                cm.make_current_needed(canvas_id)?;
                unsafe { gl.depth_func(func) };
                Ok(false)
            }

            GLCmd::DepthMask { canvas_id, flag } => {
                cm.make_current_needed(canvas_id)?;
                unsafe { gl.depth_mask(flag) };
                Ok(false)
            }

            GLCmd::DepthRange {
                canvas_id,
                near,
                far,
            } => {
                cm.make_current_needed(canvas_id)?;
                unsafe { gl.depth_range_f32(near, far) };
                Ok(false)
            }

            GLCmd::StencilFunc {
                canvas_id,
                func,
                ref_,
                mask,
            } => {
                cm.make_current_needed(canvas_id)?;
                unsafe { gl.stencil_func(func, ref_, mask) };
                Ok(false)
            }

            GLCmd::StencilFuncSeparate {
                canvas_id,
                face,
                func,
                ref_,
                mask,
            } => {
                cm.make_current_needed(canvas_id)?;
                unsafe { gl.stencil_func_separate(face, func, ref_, mask) };
                Ok(false)
            }

            GLCmd::StencilOp {
                canvas_id,
                fail,
                zfail,
                zpass,
            } => {
                cm.make_current_needed(canvas_id)?;
                unsafe { gl.stencil_op(fail, zfail, zpass) };
                Ok(false)
            }

            GLCmd::StencilOpSeparate {
                canvas_id,
                face,
                fail,
                zfail,
                zpass,
            } => {
                cm.make_current_needed(canvas_id)?;
                unsafe { gl.stencil_op_separate(face, fail, zfail, zpass) };
                Ok(false)
            }

            GLCmd::StencilMask { canvas_id, mask } => {
                cm.make_current_needed(canvas_id)?;
                unsafe { gl.stencil_mask(mask) };
                Ok(false)
            }

            GLCmd::StencilMaskSeparate {
                canvas_id,
                face,
                mask,
            } => {
                cm.make_current_needed(canvas_id)?;
                unsafe { gl.stencil_mask_separate(face, mask) };
                Ok(false)
            }

            GLCmd::CullFace { canvas_id, mode } => {
                cm.make_current_needed(canvas_id)?;
                unsafe { gl.cull_face(mode) };
                Ok(false)
            }

            GLCmd::FrontFace { canvas_id, mode } => {
                cm.make_current_needed(canvas_id)?;
                unsafe { gl.front_face(mode) };
                Ok(false)
            }

            GLCmd::ColorMask {
                canvas_id,
                r,
                g,
                b,
                a,
            } => {
                cm.make_current_needed(canvas_id)?;
                unsafe { gl.color_mask(r, g, b, a) };
                Ok(false)
            }

            GLCmd::Scissor {
                canvas_id,
                x,
                y,
                width,
                height,
            } => {
                cm.make_current_needed(canvas_id)?;
                let px = logical_to_physical_i32(cm, x);
                let py = logical_to_physical_i32(cm, y);
                let pw = logical_to_physical_i32(cm, width);
                let ph = logical_to_physical_i32(cm, height);
                unsafe { gl.scissor(px, py, pw, ph) };
                Ok(false)
            }

            GLCmd::LineWidth { canvas_id, width } => {
                cm.make_current_needed(canvas_id)?;
                unsafe { gl.line_width(width) };
                Ok(false)
            }

            GLCmd::PolygonOffset {
                canvas_id,
                factor,
                units,
            } => {
                cm.make_current_needed(canvas_id)?;
                unsafe { gl.polygon_offset(factor, units) };
                Ok(false)
            }

            // ========== Phase 2B: Uniform Variants ==========
            GLCmd::Uniform1i {
                canvas_id,
                location,
                x,
            } => {
                cm.make_current_needed(canvas_id)?;
                unsafe { gl.uniform_1_i32(to_native_uniform_location(location).as_ref(), x) };
                Ok(false)
            }

            GLCmd::Uniform1f {
                canvas_id,
                location,
                x,
            } => {
                cm.make_current_needed(canvas_id)?;
                unsafe { gl.uniform_1_f32(to_native_uniform_location(location).as_ref(), x) };
                Ok(false)
            }

            GLCmd::Uniform2f {
                canvas_id,
                location,
                x,
                y,
            } => {
                cm.make_current_needed(canvas_id)?;
                unsafe { gl.uniform_2_f32(to_native_uniform_location(location).as_ref(), x, y) };
                Ok(false)
            }

            GLCmd::Uniform4f {
                canvas_id,
                location,
                x,
                y,
                z,
                w,
            } => {
                cm.make_current_needed(canvas_id)?;
                unsafe {
                    gl.uniform_4_f32(to_native_uniform_location(location).as_ref(), x, y, z, w)
                };
                Ok(false)
            }

            GLCmd::Uniform1iv {
                canvas_id,
                location,
                value,
            } => {
                cm.make_current_needed(canvas_id)?;
                unsafe {
                    gl.uniform_1_i32_slice(to_native_uniform_location(location).as_ref(), &value)
                };
                Ok(false)
            }

            GLCmd::Uniform1fv {
                canvas_id,
                location,
                value,
            } => {
                cm.make_current_needed(canvas_id)?;
                unsafe {
                    gl.uniform_1_f32_slice(to_native_uniform_location(location).as_ref(), &value)
                };
                Ok(false)
            }

            GLCmd::Uniform2iv {
                canvas_id,
                location,
                value,
            } => {
                cm.make_current_needed(canvas_id)?;
                unsafe {
                    gl.uniform_2_i32_slice(to_native_uniform_location(location).as_ref(), &value)
                };
                Ok(false)
            }

            GLCmd::Uniform2fv {
                canvas_id,
                location,
                value,
            } => {
                cm.make_current_needed(canvas_id)?;
                unsafe {
                    gl.uniform_2_f32_slice(to_native_uniform_location(location).as_ref(), &value)
                };
                Ok(false)
            }

            GLCmd::Uniform3iv {
                canvas_id,
                location,
                value,
            } => {
                cm.make_current_needed(canvas_id)?;
                unsafe {
                    gl.uniform_3_i32_slice(to_native_uniform_location(location).as_ref(), &value)
                };
                Ok(false)
            }

            GLCmd::Uniform3fv {
                canvas_id,
                location,
                value,
            } => {
                cm.make_current_needed(canvas_id)?;
                unsafe {
                    gl.uniform_3_f32_slice(to_native_uniform_location(location).as_ref(), &value)
                };
                Ok(false)
            }

            GLCmd::Uniform4iv {
                canvas_id,
                location,
                value,
            } => {
                cm.make_current_needed(canvas_id)?;
                unsafe {
                    gl.uniform_4_i32_slice(to_native_uniform_location(location).as_ref(), &value)
                };
                Ok(false)
            }

            GLCmd::Uniform4fv {
                canvas_id,
                location,
                value,
            } => {
                cm.make_current_needed(canvas_id)?;
                unsafe {
                    gl.uniform_4_f32_slice(to_native_uniform_location(location).as_ref(), &value)
                };
                Ok(false)
            }

            GLCmd::UniformMatrix2fv {
                canvas_id,
                location,
                transpose,
                value,
            } => {
                cm.make_current_needed(canvas_id)?;
                unsafe {
                    gl.uniform_matrix_2_f32_slice(
                        to_native_uniform_location(location).as_ref(),
                        transpose,
                        &value,
                    )
                };
                Ok(false)
            }

            GLCmd::UniformMatrix4fv {
                canvas_id,
                location,
                transpose,
                value,
            } => {
                cm.make_current_needed(canvas_id)?;
                unsafe {
                    gl.uniform_matrix_4_f32_slice(
                        to_native_uniform_location(location).as_ref(),
                        transpose,
                        &value,
                    )
                };
                Ok(false)
            }

            // ========== Phase 3A: Framebuffer/Renderbuffer ==========
            GLCmd::CreateFramebuffer { client_id } => {
                let _ = self.bind_for_contextless_gl(cm)?;
                let owner = Self::current_owner_canvas(cm);
                unsafe {
                    match gl.create_framebuffer() {
                        Ok(fb) => {
                            cm.framebuffers.insert(
                                client_id,
                                crate::canvas::FramebufferMeta {
                                    gl_handle: Some(fb),
                                    owner_canvas: owner,
                                    deleted: false,
                                },
                            );
                        }
                        Err(e) => tracing::error!(
                            "gl.create_framebuffer failed for id {client_id}: {e:?}"
                        ),
                    }
                }
                Ok(false)
            }

            GLCmd::DeleteFramebuffer { framebuffer_id } => {
                let _ = self.bind_for_contextless_gl(cm)?;
                if let Some(meta) = cm.framebuffers.remove(&framebuffer_id) {
                    if let Some(h) = meta.gl_handle {
                        unsafe { gl.delete_framebuffer(h) };
                    }
                }
                Ok(false)
            }

            GLCmd::BindFramebuffer {
                canvas_id,
                target,
                framebuffer,
            } => {
                cm.make_current_needed(canvas_id)?;
                let native = if let Some(id) = framebuffer {
                    let meta = cm.framebuffers.get(&id).ok_or_else(|| {
                        ee(
                            ErrorCode::NotFound,
                            format!("framebuffer not found: {id:?}"),
                        )
                    })?;
                    if meta.deleted {
                        shared::bail!(
                            ErrorCode::InvalidOperation,
                            "bind_framebuffer on deleted framebuffer"
                        );
                    }
                    meta.gl_handle
                } else {
                    None
                };
                unsafe { gl.bind_framebuffer(target, native) };
                Ok(false)
            }

            GLCmd::FramebufferTexture2D {
                canvas_id,
                target,
                attachment,
                textarget,
                texture,
                level,
            } => {
                cm.make_current_needed(canvas_id)?;
                let tex_handle = if let Some(id) = texture {
                    let meta = cm.textures.get(&id).ok_or_else(|| {
                        ee(ErrorCode::NotFound, format!("texture not found: {id:?}"))
                    })?;
                    if meta.deleted {
                        shared::bail!(
                            ErrorCode::InvalidOperation,
                            "framebufferTexture2D on deleted texture"
                        );
                    }
                    meta.gl_handle
                } else {
                    None
                };
                unsafe {
                    gl.framebuffer_texture_2d(target, attachment, textarget, tex_handle, level)
                };
                Ok(false)
            }

            GLCmd::FramebufferRenderbuffer {
                canvas_id,
                target,
                attachment,
                renderbuffertarget,
                renderbuffer,
            } => {
                cm.make_current_needed(canvas_id)?;
                let rb_handle = if let Some(id) = renderbuffer {
                    let meta = cm.renderbuffers.get(&id).ok_or_else(|| {
                        ee(
                            ErrorCode::NotFound,
                            format!("renderbuffer not found: {id:?}"),
                        )
                    })?;
                    if meta.deleted {
                        shared::bail!(
                            ErrorCode::InvalidOperation,
                            "framebufferRenderbuffer on deleted renderbuffer"
                        );
                    }
                    meta.gl_handle
                } else {
                    None
                };
                unsafe {
                    gl.framebuffer_renderbuffer(target, attachment, renderbuffertarget, rb_handle)
                };
                Ok(false)
            }

            GLCmd::CheckFramebufferStatus {
                canvas_id,
                target,
                resp,
            } => {
                cm.make_current_needed(canvas_id)?;
                let status = unsafe { gl.check_framebuffer_status(target) };
                let _ = resp.send(Ok(status));
                Ok(false)
            }

            GLCmd::CreateRenderbuffer { client_id } => {
                let _ = self.bind_for_contextless_gl(cm)?;
                let owner = Self::current_owner_canvas(cm);
                unsafe {
                    match gl.create_renderbuffer() {
                        Ok(rb) => {
                            cm.renderbuffers.insert(
                                client_id,
                                crate::canvas::RenderbufferMeta {
                                    gl_handle: Some(rb),
                                    owner_canvas: owner,
                                    deleted: false,
                                },
                            );
                        }
                        Err(e) => tracing::error!(
                            "gl.create_renderbuffer failed for id {client_id}: {e:?}"
                        ),
                    }
                }
                Ok(false)
            }

            GLCmd::DeleteRenderbuffer { renderbuffer_id } => {
                let _ = self.bind_for_contextless_gl(cm)?;
                if let Some(meta) = cm.renderbuffers.remove(&renderbuffer_id) {
                    if let Some(h) = meta.gl_handle {
                        unsafe { gl.delete_renderbuffer(h) };
                    }
                }
                Ok(false)
            }

            GLCmd::BindRenderbuffer {
                canvas_id,
                target,
                renderbuffer,
            } => {
                cm.make_current_needed(canvas_id)?;
                let native = if let Some(id) = renderbuffer {
                    let meta = cm.renderbuffers.get(&id).ok_or_else(|| {
                        ee(
                            ErrorCode::NotFound,
                            format!("renderbuffer not found: {id:?}"),
                        )
                    })?;
                    if meta.deleted {
                        shared::bail!(
                            ErrorCode::InvalidOperation,
                            "bind_renderbuffer on deleted renderbuffer"
                        );
                    }
                    meta.gl_handle
                } else {
                    None
                };
                unsafe { gl.bind_renderbuffer(target, native) };
                Ok(false)
            }

            GLCmd::RenderbufferStorage {
                canvas_id,
                target,
                internalformat,
                width,
                height,
            } => {
                cm.make_current_needed(canvas_id)?;
                unsafe { gl.renderbuffer_storage(target, internalformat, width, height) };
                Ok(false)
            }

            // ========== Phase 3B: Misc ==========
            GLCmd::ReadPixels {
                canvas_id,
                x,
                y,
                width,
                height,
                format,
                type_,
                resp,
            } => {
                cm.make_current_needed(canvas_id)?;
                // Calculate buffer size: width * height * bytes_per_pixel
                // bpp depends on both format (component count) and type (bytes per component)
                let components: i32 = match format {
                    glow::RGBA => 4,
                    glow::RGB => 3,
                    glow::LUMINANCE_ALPHA => 2,
                    glow::LUMINANCE | glow::ALPHA => 1,
                    _ => 4,
                };
                let bytes_per_pixel: i32 = match type_ {
                    glow::UNSIGNED_BYTE => components,
                    glow::UNSIGNED_SHORT_5_6_5
                    | glow::UNSIGNED_SHORT_4_4_4_4
                    | glow::UNSIGNED_SHORT_5_5_5_1 => 2,
                    glow::FLOAT => components * 4,
                    _ => components,
                };
                let byte_size = (width * height * bytes_per_pixel) as usize;
                let mut buf = vec![0u8; byte_size];
                unsafe {
                    gl.read_pixels(
                        x,
                        y,
                        width,
                        height,
                        format,
                        type_,
                        glow::PixelPackData::Slice(Some(&mut buf)),
                    );
                }
                let _ = resp.send(Ok(buf));
                Ok(false)
            }

            GLCmd::Hint {
                canvas_id,
                target,
                mode,
            } => {
                cm.make_current_needed(canvas_id)?;
                unsafe { gl.hint(target, mode) };
                Ok(false)
            }

            _ => {
                shared::bail!(
                    ErrorCode::NotImplemented,
                    "GL command not covered by RendererGL"
                );
            }
        }
    }
}
