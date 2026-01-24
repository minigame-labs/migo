use glow::{HasContext, NativeUniformLocation};
use shared::{
    error::{EngineError, EngineResult, ErrorCode},
    protocol::render_cmd::{BufferId, CanvasId, GLCmd, ProgramId, ShaderId, ShaderType},
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
                unsafe { gl.viewport(x, y, width as i32, height as i32) };
                Ok(false)
            }

            GLCmd::Clear { canvas_id, bit_field } => {
                cm.make_current_needed(canvas_id)?;
                unsafe { gl.clear(bit_field) };
                Ok(true)
            }

            GLCmd::ClearColor { canvas_id, r, g, b, a } => {
                cm.make_current_needed(canvas_id)?;
                unsafe { gl.clear_color(r, g, b, a) };
                Ok(false)
            }

            // ---------- Program (stateful) ----------
            GLCmd::UseProgram { canvas_id, program_id } => {
                cm.make_current_needed(canvas_id)?;
                let meta = cm
                    .programs
                    .get(&program_id)
                    .ok_or_else(|| ee(ErrorCode::NotFound, format!("program not found: {program_id:?}")))?;
                cm.check_owner(meta.owner_canvas, canvas_id, "program")?;

                if meta.deleted {
                    shared::bail!(ErrorCode::InvalidOperation, "use_program on deleted program");
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
                let meta = cm
                    .programs
                    .get(&program_id)
                    .ok_or_else(|| ee(ErrorCode::NotFound, format!("program not found: {program_id:?}")))?;
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
                    canvas_id,
                    index,
                    size,
                    type_,
                    normalized,
                    stride,
                    offset
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
                let meta = cm
                    .programs
                    .get(&program_id)
                    .ok_or_else(|| ee(ErrorCode::NotFound, format!("program not found: {program_id:?}")))?;
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
                    let meta = cm
                        .buffers
                        .get(&id)
                        .ok_or_else(|| ee(ErrorCode::NotFound, format!("buffer not found: {id:?}")))?;
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
            GLCmd::CreateProgram { resp } => {
                let _ = self.bind_for_contextless_gl(cm)?;
                let owner = Self::current_owner_canvas(cm);

                unsafe {
                    match gl.create_program() {
                        Ok(p) => {
                            let id: ProgramId = ProgramId::from(p.0.get());
                            cm.programs.insert(
                                id,
                                crate::canvas::ProgramMeta {
                                    gl_handle: Some(p),
                                    owner_canvas: owner,
                                    deleted: false,
                                },
                            );
                            let _ = resp.send(Ok(id));
                        }
                        Err(e) => {
                            let _ = resp.send(Err(ee(
                                ErrorCode::RenderBackendError,
                                format!("gl.create_program failed: {:?}", e),
                            )));
                        }
                    }
                }
                Ok(false)
            }

            GLCmd::LinkProgram { program_id, resp } => {
                let _ = self.bind_for_contextless_gl(cm)?;
                let meta = cm
                    .programs
                    .get(&program_id)
                    .ok_or_else(|| ee(ErrorCode::NotFound, format!("program not found: {program_id:?}")))?;
                if meta.deleted {
                    let _ = resp.send(Ok(false));
                    return Ok(false);
                }

                if let Some(ph) = meta.gl_handle {
                    unsafe {
                        gl.link_program(ph);
                        let ok = gl.get_program_link_status(ph);
                        let _ = resp.send(Ok(ok));
                    }
                } else {
                    let _ = resp.send(Ok(false));
                }
                Ok(false)
            }

            GLCmd::GetProgramParameter { program_id, pname, resp } => {
                let _ = self.bind_for_contextless_gl(cm)?;
                let Some(meta) = cm.programs.get(&program_id) else {
                    let _ = resp.send(Err(ee(ErrorCode::NotFound, format!("program not found: {program_id:?}"))));
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
                        glow::LINK_STATUS => if gl.get_program_link_status(ph) { 1 } else { 0 },
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
                    let _ = resp.send(Err(ee(ErrorCode::NotFound, format!("program not found: {program_id:?}"))));
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
            GLCmd::CreateShader { shader_type, resp } => {
                let _ = self.bind_for_contextless_gl(cm)?;
                let owner = Self::current_owner_canvas(cm);

                let gl_ty = match shader_type {
                    ShaderType::Vertex => glow::VERTEX_SHADER,
                    ShaderType::Fragment => glow::FRAGMENT_SHADER,
                };

                unsafe {
                    match gl.create_shader(gl_ty) {
                        Ok(s) => {
                            let id: ShaderId = ShaderId::from(s.0.get());
                            cm.shaders.insert(
                                id,
                                crate::canvas::ShaderMeta {
                                    gl_handle: Some(s),
                                    owner_canvas: owner,
                                    shader_type,
                                    gl_shader_type: gl_ty,
                                    deleted: false,
                                    source_len: 0,
                                },
                            );
                            let _ = resp.send(Ok(id));
                        }
                        Err(e) => {
                            let _ = resp.send(Err(ee(
                                ErrorCode::RenderBackendError,
                                format!("gl.create_shader failed: {:?}", e),
                            )));
                        }
                    }
                }
                Ok(false)
            }

            GLCmd::ShaderSource { shader_id, source, resp } => {
                let _ = self.bind_for_contextless_gl(cm)?;
                let Some(meta) = cm.shaders.get_mut(&shader_id) else {
                    let _ = resp.send(Err(ee(ErrorCode::NotFound, format!("shader not found: {shader_id:?}"))));
                    return Ok(false);
                };

                if meta.deleted {
                    let _ = resp.send(Err(ee(ErrorCode::InvalidOperation, "shader already deleted")));
                    return Ok(false);
                }

                if let Some(sh) = meta.gl_handle {
                    meta.source_len = source.len();
                    unsafe { gl.shader_source(sh, &source) };
                    let _ = resp.send(Ok(()));
                } else {
                    let _ = resp.send(Err(ee(ErrorCode::InvalidOperation, "shader handle missing")));
                }
                Ok(false)
            }

            GLCmd::CompileShader { shader_id, resp } => {
                let _ = self.bind_for_contextless_gl(cm)?;
                if let Some(meta) = cm.shaders.get(&shader_id) {
                    if meta.deleted {
                        let _ = resp.send(Ok(false));
                        return Ok(false);
                    }
                    if let Some(sh) = meta.gl_handle {
                        unsafe {
                            gl.compile_shader(sh);
                            let ok = gl.get_shader_compile_status(sh);
                            let _ = resp.send(Ok(ok));
                        }
                    } else {
                        let _ = resp.send(Ok(false));
                    }
                } else {
                    let _ = resp.send(Err(ee(ErrorCode::NotFound, format!("shader not found: {shader_id:?}"))));
                }
                Ok(false)
            }

            GLCmd::AttachShader { program_id, shader_id, resp } => {
                let _ = self.bind_for_contextless_gl(cm)?;
                let p = cm
                    .programs
                    .get(&program_id)
                    .ok_or_else(|| ee(ErrorCode::NotFound, format!("program not found: {program_id:?}")))?;
                let s = cm
                    .shaders
                    .get(&shader_id)
                    .ok_or_else(|| ee(ErrorCode::NotFound, format!("shader not found: {shader_id:?}")))?;

                if p.deleted || s.deleted {
                    let _ = resp.send(Err(ee(ErrorCode::InvalidOperation, "program/shader deleted")));
                    return Ok(false);
                }

                // WebGL-ish: must belong to same owner canvas
                if p.owner_canvas != s.owner_canvas {
                    let _ = resp.send(Err(ee(
                        ErrorCode::InvalidOperation,
                        "attach shader across different contexts",
                    )));
                    return Ok(false);
                }

                if let (Some(ph), Some(sh)) = (p.gl_handle, s.gl_handle) {
                    unsafe { gl.attach_shader(ph, sh) };
                    let _ = resp.send(Ok(()));
                } else {
                    let _ = resp.send(Err(ee(ErrorCode::InvalidOperation, "program/shader handle missing")));
                }
                Ok(false)
            }

            GLCmd::GetShaderParameter { shader_id, pname, resp } => {
                let _ = self.bind_for_contextless_gl(cm)?;
                let Some(meta) = cm.shaders.get(&shader_id) else {
                    let _ = resp.send(Err(ee(ErrorCode::NotFound, format!("shader not found: {shader_id:?}"))));
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
                    glow::COMPILE_STATUS => unsafe { if gl.get_shader_compile_status(sh) { 1 } else { 0 } },
                    glow::SHADER_TYPE => meta.gl_shader_type as i32,
                    glow::DELETE_STATUS => if meta.deleted { 1 } else { 0 },
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
                    let _ = resp.send(Err(ee(ErrorCode::NotFound, format!("shader not found: {shader_id:?}"))));
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
            GLCmd::CreateBuffer { resp } => {
                let _ = self.bind_for_contextless_gl(cm)?;
                let owner = Self::current_owner_canvas(cm);

                unsafe {
                    match gl.create_buffer() {
                        Ok(buf) => {
                            let id: BufferId = BufferId::from(buf.0.get());
                            cm.buffers.insert(
                                id,
                                crate::canvas::BufferMeta {
                                    gl_handle: Some(buf),
                                    owner_canvas: owner,
                                    deleted: false,
                                },
                            );
                            let _ = resp.send(Ok(id));
                        }
                        Err(e) => {
                            let _ = resp.send(Err(ee(
                                ErrorCode::RenderBackendError,
                                format!("gl.create_buffer failed: {:?}", e),
                            )));
                        }
                    }
                }
                Ok(false)
            }

            _ => {
                shared::bail!(ErrorCode::NotImplemented, "GL command not covered by RendererGL");
            }
        }
    }
}
