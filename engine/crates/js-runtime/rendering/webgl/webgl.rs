use std::mem;

use bytemuck::allocation::cast_vec;
use deno_core::{op2, OpState};
use tracing::{error, warn};

use crate::rendering::image::cache::IMAGE_CACHE;

use shared::{
    error::EngineError,
    js_escape::escape_for_json_string,
    op_state::CanvasOpState,
    protocol::{
        render_cmd::{GLCmd, RenderCmdResp, RenderCommand, ShaderType},
        send_gl_with_resp_sync,
    },
};

/// Initial capacity for the per-frame GL command buffer.  Most frames
/// issue 100-500 GL commands; 256 avoids early re-allocations while
/// keeping memory usage modest.
const GL_BATCH_INITIAL_CAPACITY: usize = 256;

pub(crate) struct GlBatchCollector {
    commands: Vec<GLCmd>,
}

impl GlBatchCollector {
    pub(crate) fn new() -> Self {
        Self {
            commands: Vec::with_capacity(GL_BATCH_INITIAL_CAPACITY),
        }
    }

    /// Push a command into the per-frame buffer.  Commands are never
    /// auto-flushed; they accumulate until `op_gl_flush()` at frame end.
    /// This reduces cross-thread channel sends from ~5-15 per frame to 1.
    fn push(&mut self, cmd: GLCmd) {
        self.commands.push(cmd);
    }

    /// Drain all pending commands. Preserves the allocated capacity.
    fn take_all(&mut self) -> Vec<GLCmd> {
        let mut batch = Vec::new();
        mem::swap(&mut self.commands, &mut batch);
        batch
    }
}

#[inline]
fn send_gl_batch_now(state: &mut OpState, commands: Vec<GLCmd>) {
    if commands.is_empty() {
        return;
    }
    let ctx = state.borrow::<CanvasOpState>();
    if let Err(e) = ctx.tx.send(RenderCommand::GLBatch { commands }) {
        error!("send_gl_batch failed: {e}");
    }
}

#[inline]
fn queue_gl_fire_and_forget(state: &mut OpState, cmd: GLCmd) {
    let Some(collector) = state.try_borrow_mut::<GlBatchCollector>() else {
        error!("GlBatchCollector missing in op state");
        return;
    };
    collector.push(cmd);
}

fn flush_gl_batch(state: &mut OpState) {
    let commands = {
        let Some(collector) = state.try_borrow_mut::<GlBatchCollector>() else {
            error!("GlBatchCollector missing in op state");
            return;
        };
        collector.take_all()
    };
    send_gl_batch_now(state, commands);
}

#[inline]
fn send_gl_sync_with_flush<T>(
    state: &mut OpState,
    build: impl FnOnce(RenderCmdResp<T>) -> RenderCommand,
) -> Result<T, EngineError> {
    flush_gl_batch(state);
    let ctx = state.borrow::<CanvasOpState>();
    send_gl_with_resp_sync(ctx, build)
}

#[inline]
fn load_cached_image_rgba(image_id: u32) -> Option<(i32, i32, Vec<u8>)> {
    let src = {
        let c = IMAGE_CACHE.lock();
        c.source_for_image_id(image_id)
    }?;

    let cached = {
        let mut cache = io::global_cache();
        cache.get(&src)
    }?;

    Some((
        cached.image.width as i32,
        cached.image.height as i32,
        cached.image.rgba.as_ref().clone(),
    ))
}

#[op2(fast)]
pub fn op_gl_flush(state: &mut OpState) {
    flush_gl_batch(state);
}

#[op2(fast)]
pub fn op_viewport(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] x: i32,
    #[smi] y: i32,
    #[smi] width: u32,
    #[smi] height: u32,
) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::Viewport {
            canvas_id,
            x,
            y,
            width,
            height,
        },
    );
}

#[op2(fast)]
pub fn op_clear_color(state: &mut OpState, #[smi] canvas_id: u32, r: f32, g: f32, b: f32, a: f32) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::ClearColor {
            canvas_id,
            r,
            g,
            b,
            a,
        },
    );
}

#[op2(fast)]
pub fn op_clear(state: &mut OpState, #[smi] canvas_id: u32, #[smi] bit_field: u32) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::Clear {
            canvas_id,
            bit_field,
        },
    );
}

#[op2(fast)]
pub fn op_create_program(state: &mut OpState, #[smi] client_id: u32) {
    queue_gl_fire_and_forget(state, GLCmd::CreateProgram { client_id });
}

#[op2(fast)]
pub fn op_use_program(state: &mut OpState, #[smi] canvas_id: u32, #[smi] program_id: u32) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::UseProgram {
            canvas_id,
            program_id,
        },
    );
}

#[op2(fast)]
pub fn op_link_program(state: &mut OpState, #[smi] program_id: u32) {
    queue_gl_fire_and_forget(state, GLCmd::LinkProgram { program_id });
}

#[op2(fast)]
pub fn op_get_program_parameter(
    state: &mut OpState,
    #[smi] program_id: u32,
    #[smi] pname: u32,
) -> i32 {
    send_gl_sync_with_flush(state, |resp| {
        RenderCommand::GL(GLCmd::GetProgramParameter {
            program_id,
            pname,
            resp,
        })
    })
    .unwrap_or(0)
}

#[op2]
#[string]
pub fn op_get_program_info_log(state: &mut OpState, #[smi] program_id: u32) -> String {
    send_gl_sync_with_flush(state, |resp| {
        RenderCommand::GL(GLCmd::GetProgramInfoLog { program_id, resp })
    })
    .ok()
    .flatten()
    .unwrap_or_default()
}

#[op2(fast)]
pub fn op_delete_program(state: &mut OpState, #[smi] program_id: u32) {
    queue_gl_fire_and_forget(state, GLCmd::DeleteProgram { program_id });
}

#[op2(fast)]
pub fn op_create_shader(state: &mut OpState, #[smi] client_id: u32, #[smi] ty: u32) {
    let shader_type = match ty {
        glow::VERTEX_SHADER => ShaderType::Vertex,
        glow::FRAGMENT_SHADER => ShaderType::Fragment,
        _ => {
            error!("unknown shader type: {}", ty);
            return;
        }
    };
    queue_gl_fire_and_forget(
        state,
        GLCmd::CreateShader {
            client_id,
            shader_type,
        },
    );
}

#[op2(fast)]
pub fn op_shader_source(state: &mut OpState, #[smi] shader_id: u32, #[string] source: String) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::ShaderSource {
            shader_id,
            source,
            resp: None,
        },
    );
}

#[op2(fast)]
pub fn op_compile_shader(state: &mut OpState, #[smi] shader_id: u32) {
    queue_gl_fire_and_forget(state, GLCmd::CompileShader { shader_id });
}

#[op2(fast)]
pub fn op_attach_shader(state: &mut OpState, #[smi] program_id: u32, #[smi] shader_id: u32) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::AttachShader {
            program_id,
            shader_id,
            resp: None,
        },
    );
}

#[op2(fast)]
pub fn op_get_shader_parameter(
    state: &mut OpState,
    #[smi] shader_id: u32,
    #[smi] pname: u32,
) -> i32 {
    send_gl_sync_with_flush(state, |resp| {
        RenderCommand::GL(GLCmd::GetShaderParameter {
            shader_id,
            pname,
            resp,
        })
    })
    .unwrap_or(0)
}

#[op2]
#[string]
pub fn op_get_shader_info_log(state: &mut OpState, #[smi] shader_id: u32) -> String {
    send_gl_sync_with_flush(state, |resp| {
        RenderCommand::GL(GLCmd::GetShaderInfoLog { shader_id, resp })
    })
    .ok()
    .flatten()
    .unwrap_or_default()
}

#[op2(fast)]
pub fn op_delete_shader(state: &mut OpState, #[smi] shader_id: u32) {
    queue_gl_fire_and_forget(state, GLCmd::DeleteShader { shader_id });
}

#[op2(fast)]
pub fn op_draw_arrays(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] mode: u32,
    #[smi] first: i32,
    #[smi] count: i32,
) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::DrawArrays {
            canvas_id,
            mode,
            first,
            count,
        },
    );
}

#[op2(fast)]
pub fn op_draw_elements(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] mode: u32,
    #[smi] count: i32,
    #[smi] index_type: u32,
    #[smi] offset: i32,
) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::DrawElements {
            canvas_id,
            mode,
            count,
            index_type,
            offset,
        },
    );
}

#[op2(fast)]
pub fn op_get_attrib_location(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] program_id: u32,
    #[string] name: String,
) -> i32 {
    send_gl_sync_with_flush(state, |resp| {
        RenderCommand::GL(GLCmd::GetAttribLocation {
            canvas_id,
            program_id,
            name,
            resp,
        })
    })
    .ok()
    .flatten()
    .map(|v| v as i32)
    .unwrap_or(-1)
}

#[op2]
#[string]
pub fn op_get_active_attrib(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] program_id: u32,
    #[smi] index: u32,
) -> String {
    let info = send_gl_sync_with_flush(state, |resp| {
        RenderCommand::GL(GLCmd::GetActiveAttrib {
            canvas_id,
            program_id,
            index,
            resp,
        })
    })
    .ok()
    .flatten();

    if let Some((name, size, type_)) = info {
        let escaped_name = escape_for_json_string(&name);
        return format!(
            "{{\"name\":\"{}\",\"size\":{},\"type\":{}}}",
            escaped_name, size, type_
        );
    }

    String::new()
}

#[op2]
#[string]
pub fn op_get_active_uniform(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] program_id: u32,
    #[smi] index: u32,
) -> String {
    let info = send_gl_sync_with_flush(state, |resp| {
        RenderCommand::GL(GLCmd::GetActiveUniform {
            canvas_id,
            program_id,
            index,
            resp,
        })
    })
    .ok()
    .flatten();

    if let Some((name, size, type_)) = info {
        let escaped_name = escape_for_json_string(&name);
        return format!(
            "{{\"name\":\"{}\",\"size\":{},\"type\":{}}}",
            escaped_name, size, type_
        );
    }

    String::new()
}

#[op2(fast)]
pub fn op_enable_vertex_attrib_array(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] index: u32,
) {
    queue_gl_fire_and_forget(state, GLCmd::EnableVertexAttribArray { canvas_id, index });
}

#[op2(fast)]
pub fn op_vertex_attrib_pointer(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] index: u32,
    #[smi] size: i32,
    #[smi] type_: u32,
    normalized: bool,
    #[smi] stride: i32,
    #[smi] offset: i32,
) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::VertexAttribPointer {
            canvas_id,
            index,
            size,
            type_,
            normalized,
            stride,
            offset,
        },
    );
}

#[op2(fast)]
pub fn op_create_buffer(state: &mut OpState, #[smi] client_id: u32) {
    queue_gl_fire_and_forget(state, GLCmd::CreateBuffer { client_id });
}

#[op2(fast)]
pub fn op_bind_buffer(state: &mut OpState, #[smi] canvas_id: u32, #[smi] target: u32, buffer: i32) {
    let buffer = if buffer < 0 {
        None
    } else {
        Some(buffer as u32)
    };
    queue_gl_fire_and_forget(
        state,
        GLCmd::BindBuffer {
            canvas_id,
            target,
            buffer,
        },
    );
}

#[op2]
pub fn op_buffer_data(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] target: u32,
    #[smi] size: i32,
    // #[buffer(copy)] gives us an owned Vec<u8> directly, avoiding an
    // intermediate JsBuffer wrapper + separate .to_vec() heap allocation.
    // The copy itself is unavoidable: V8 owns the ArrayBuffer backing store
    // and we must send owned data to the render thread.
    #[buffer(copy)] data: Option<Vec<u8>>,
    #[smi] usage: u32,
) {
    if data.is_none() && size <= 0 {
        error!("op_buffer_data: size must > 0 when data is None");
        return;
    }

    queue_gl_fire_and_forget(
        state,
        GLCmd::BufferData {
            canvas_id,
            target,
            size,
            data,
            usage,
        },
    );
}

#[op2(fast)]
pub fn op_get_uniform_location(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] program_id: u32,
    #[string] name: String,
) -> i32 {
    send_gl_sync_with_flush(state, |resp| {
        RenderCommand::GL(GLCmd::GetUniformLocation {
            canvas_id,
            program_id,
            name,
            resp,
        })
    })
    .ok()
    .flatten()
    .map(|v| v as i32)
    .unwrap_or(-1)
}

#[op2(fast)]
pub fn op_uniform3f(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    location: i32,
    x: f32,
    y: f32,
    z: f32,
) {
    let location = if location < 0 {
        None
    } else {
        Some(location as u32)
    };
    queue_gl_fire_and_forget(
        state,
        GLCmd::Uniform3f {
            canvas_id,
            location,
            x,
            y,
            z,
        },
    );
}

#[op2(fast)]
pub fn op_uniform_matrix_3fv(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    location: i32,
    transpose: bool,
    #[buffer(copy)] value: Vec<u32>,
) {
    let location = if location < 0 {
        None
    } else {
        Some(location as u32)
    };
    let value: Vec<f32> = cast_vec(value);

    queue_gl_fire_and_forget(
        state,
        GLCmd::UniformMatrix3fv {
            canvas_id,
            location,
            transpose,
            value,
        },
    );
}

// ---------------------------------------------------------------------------
// Phase 1A: GL State
// ---------------------------------------------------------------------------

#[op2(fast)]
pub fn op_enable(state: &mut OpState, #[smi] canvas_id: u32, #[smi] cap: u32) {
    queue_gl_fire_and_forget(state, GLCmd::Enable { canvas_id, cap });
}

#[op2(fast)]
pub fn op_disable(state: &mut OpState, #[smi] canvas_id: u32, #[smi] cap: u32) {
    queue_gl_fire_and_forget(state, GLCmd::Disable { canvas_id, cap });
}

#[op2(fast)]
pub fn op_is_enabled(state: &mut OpState, #[smi] canvas_id: u32, #[smi] cap: u32) -> u32 {
    send_gl_sync_with_flush(state, |resp| {
        RenderCommand::GL(GLCmd::IsEnabled {
            canvas_id,
            cap,
            resp,
        })
    })
    .map(|v| if v { 1 } else { 0 })
    .unwrap_or(0)
}

/// PERF: Architectural limitation -- this is a synchronous cross-thread call.
/// `op_get_parameter` flushes the pending GL command batch, sends a
/// `GetParameter` request to the render thread, and blocks the JS thread
/// until the render thread processes it and responds.  This causes a full
/// pipeline stall: JS cannot execute while waiting, and the render thread
/// must drain its queue to reach this request.
///
/// Frequent calls (e.g. inside a draw loop) will significantly degrade
/// frame rate.  Games should cache parameter values on the JS side
/// when possible.
///
/// Note: `gl.getError()` is currently stubbed to always return 0 on the JS
/// side (`02_webgl_context.js`), so it does not hit this path.  If a real
/// implementation is ever needed, consider maintaining a last-error cache
/// on the render thread updated by each GL call, and reading it via a
/// lock-free atomic instead of a sync round-trip.
#[op2]
#[string]
pub fn op_get_parameter(state: &mut OpState, #[smi] canvas_id: u32, #[smi] pname: u32) -> String {
    send_gl_sync_with_flush(state, |resp| {
        RenderCommand::GL(GLCmd::GetParameter {
            canvas_id,
            pname,
            resp,
        })
    })
    .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Phase 1B: Textures
// ---------------------------------------------------------------------------

#[op2(fast)]
pub fn op_create_texture(state: &mut OpState, #[smi] client_id: u32) {
    queue_gl_fire_and_forget(state, GLCmd::CreateTexture { client_id });
}

#[op2(fast)]
pub fn op_delete_texture(state: &mut OpState, #[smi] texture_id: u32) {
    queue_gl_fire_and_forget(state, GLCmd::DeleteTexture { texture_id });
}

#[op2(fast)]
pub fn op_bind_texture(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] target: u32,
    texture: i32,
) {
    let texture = if texture < 0 {
        None
    } else {
        Some(texture as u32)
    };
    queue_gl_fire_and_forget(
        state,
        GLCmd::BindTexture {
            canvas_id,
            target,
            texture,
        },
    );
}

#[op2(fast)]
pub fn op_active_texture(state: &mut OpState, #[smi] canvas_id: u32, #[smi] unit: u32) {
    queue_gl_fire_and_forget(state, GLCmd::ActiveTexture { canvas_id, unit });
}

#[op2]
pub fn op_tex_image_2d(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] target: u32,
    #[smi] level: i32,
    #[smi] internalformat: i32,
    #[smi] width: i32,
    #[smi] height: i32,
    #[smi] border: i32,
    #[smi] format: u32,
    #[smi] type_: u32,
    // #[buffer(copy)] -> owned Vec<u8>; avoids intermediate JsBuffer + .to_vec().
    #[buffer(copy)] data: Option<Vec<u8>>,
) {
    queue_gl_fire_and_forget(
        state,
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
        },
    );
}

#[op2(fast)]
pub fn op_tex_image_2d_from_image(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] target: u32,
    #[smi] level: i32,
    #[smi] internalformat: i32,
    #[smi] format: u32,
    #[smi] type_: u32,
    #[smi] image_id: u32,
) {
    let Some((width, height, data)) = load_cached_image_rgba(image_id) else {
        warn!(
            "op_tex_image_2d_from_image cache miss: image_id={}",
            image_id
        );
        return;
    };

    queue_gl_fire_and_forget(
        state,
        GLCmd::TexImage2D {
            canvas_id,
            target,
            level,
            internalformat,
            width,
            height,
            border: 0,
            format,
            type_,
            data: Some(data),
        },
    );
}

#[op2(fast)]
pub fn op_tex_sub_image_2d(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] target: u32,
    #[smi] level: i32,
    #[smi] xoffset: i32,
    #[smi] yoffset: i32,
    #[smi] width: i32,
    #[smi] height: i32,
    #[smi] format: u32,
    #[smi] type_: u32,
    // #[buffer(copy)] -> owned Vec<u8>; avoids intermediate JsBuffer + .to_vec().
    #[buffer(copy)] data: Vec<u8>,
) {
    queue_gl_fire_and_forget(
        state,
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
        },
    );
}

#[op2(fast)]
pub fn op_tex_sub_image_2d_from_image(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] target: u32,
    #[smi] level: i32,
    #[smi] xoffset: i32,
    #[smi] yoffset: i32,
    #[smi] format: u32,
    #[smi] type_: u32,
    #[smi] image_id: u32,
) {
    let Some((width, height, data)) = load_cached_image_rgba(image_id) else {
        warn!(
            "op_tex_sub_image_2d_from_image cache miss: image_id={}",
            image_id
        );
        return;
    };

    queue_gl_fire_and_forget(
        state,
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
        },
    );
}

#[op2(fast)]
pub fn op_tex_parameteri(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] target: u32,
    #[smi] pname: u32,
    #[smi] param: i32,
) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::TexParameteri {
            canvas_id,
            target,
            pname,
            param,
        },
    );
}

#[op2(fast)]
pub fn op_tex_parameterf(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] target: u32,
    #[smi] pname: u32,
    param: f32,
) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::TexParameterf {
            canvas_id,
            target,
            pname,
            param,
        },
    );
}

#[op2(fast)]
pub fn op_generate_mipmap(state: &mut OpState, #[smi] canvas_id: u32, #[smi] target: u32) {
    queue_gl_fire_and_forget(state, GLCmd::GenerateMipmap { canvas_id, target });
}

#[op2(fast)]
pub fn op_pixel_storei(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] pname: u32,
    #[smi] param: i32,
) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::PixelStorei {
            canvas_id,
            pname,
            param,
        },
    );
}

#[op2(fast)]
pub fn op_compressed_tex_image_2d(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] target: u32,
    #[smi] level: i32,
    #[smi] internalformat: u32,
    #[smi] width: i32,
    #[smi] height: i32,
    #[smi] border: i32,
    // #[buffer(copy)] -> owned Vec<u8>; avoids intermediate JsBuffer + .to_vec().
    #[buffer(copy)] data: Vec<u8>,
) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::CompressedTexImage2D {
            canvas_id,
            target,
            level,
            internalformat,
            width,
            height,
            border,
            data,
        },
    );
}

#[op2(fast)]
pub fn op_compressed_tex_sub_image_2d(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] target: u32,
    #[smi] level: i32,
    #[smi] xoffset: i32,
    #[smi] yoffset: i32,
    #[smi] width: i32,
    #[smi] height: i32,
    #[smi] format: u32,
    // #[buffer(copy)] -> owned Vec<u8>; avoids intermediate JsBuffer + .to_vec().
    #[buffer(copy)] data: Vec<u8>,
) {
    queue_gl_fire_and_forget(
        state,
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
        },
    );
}

// ---------------------------------------------------------------------------
// Phase 1C: Buffer & Vertex Extensions
// ---------------------------------------------------------------------------

#[op2(fast)]
pub fn op_buffer_sub_data(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] target: u32,
    #[smi] offset: i32,
    // #[buffer(copy)] -> owned Vec<u8>; avoids intermediate JsBuffer + .to_vec().
    #[buffer(copy)] data: Vec<u8>,
) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::BufferSubData {
            canvas_id,
            target,
            offset,
            data,
        },
    );
}

#[op2(fast)]
pub fn op_disable_vertex_attrib_array(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] index: u32,
) {
    queue_gl_fire_and_forget(state, GLCmd::DisableVertexAttribArray { canvas_id, index });
}

#[op2(fast)]
pub fn op_clear_depth(state: &mut OpState, #[smi] canvas_id: u32, depth: f32) {
    queue_gl_fire_and_forget(state, GLCmd::ClearDepth { canvas_id, depth });
}

#[op2(fast)]
pub fn op_clear_stencil(state: &mut OpState, #[smi] canvas_id: u32, #[smi] s: i32) {
    queue_gl_fire_and_forget(state, GLCmd::ClearStencil { canvas_id, s });
}

// ---------------------------------------------------------------------------
// Phase 2A: Blend / Depth / Stencil / Cull State
// ---------------------------------------------------------------------------

#[op2(fast)]
pub fn op_blend_func(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] sfactor: u32,
    #[smi] dfactor: u32,
) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::BlendFunc {
            canvas_id,
            sfactor,
            dfactor,
        },
    );
}

#[op2(fast)]
pub fn op_blend_func_separate(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] src_rgb: u32,
    #[smi] dst_rgb: u32,
    #[smi] src_alpha: u32,
    #[smi] dst_alpha: u32,
) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::BlendFuncSeparate {
            canvas_id,
            src_rgb,
            dst_rgb,
            src_alpha,
            dst_alpha,
        },
    );
}

#[op2(fast)]
pub fn op_blend_equation(state: &mut OpState, #[smi] canvas_id: u32, #[smi] mode: u32) {
    queue_gl_fire_and_forget(state, GLCmd::BlendEquation { canvas_id, mode });
}

#[op2(fast)]
pub fn op_blend_equation_separate(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] mode_rgb: u32,
    #[smi] mode_alpha: u32,
) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::BlendEquationSeparate {
            canvas_id,
            mode_rgb,
            mode_alpha,
        },
    );
}

#[op2(fast)]
pub fn op_blend_color(state: &mut OpState, #[smi] canvas_id: u32, r: f32, g: f32, b: f32, a: f32) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::BlendColor {
            canvas_id,
            r,
            g,
            b,
            a,
        },
    );
}

#[op2(fast)]
pub fn op_depth_func(state: &mut OpState, #[smi] canvas_id: u32, #[smi] func: u32) {
    queue_gl_fire_and_forget(state, GLCmd::DepthFunc { canvas_id, func });
}

#[op2(fast)]
pub fn op_depth_mask(state: &mut OpState, #[smi] canvas_id: u32, flag: bool) {
    queue_gl_fire_and_forget(state, GLCmd::DepthMask { canvas_id, flag });
}

#[op2(fast)]
pub fn op_depth_range(state: &mut OpState, #[smi] canvas_id: u32, near: f32, far: f32) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::DepthRange {
            canvas_id,
            near,
            far,
        },
    );
}

#[op2(fast)]
pub fn op_stencil_func(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] func: u32,
    #[smi] ref_: i32,
    #[smi] mask: u32,
) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::StencilFunc {
            canvas_id,
            func,
            ref_,
            mask,
        },
    );
}

#[op2(fast)]
pub fn op_stencil_func_separate(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] face: u32,
    #[smi] func: u32,
    #[smi] ref_: i32,
    #[smi] mask: u32,
) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::StencilFuncSeparate {
            canvas_id,
            face,
            func,
            ref_,
            mask,
        },
    );
}

#[op2(fast)]
pub fn op_stencil_op(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] fail: u32,
    #[smi] zfail: u32,
    #[smi] zpass: u32,
) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::StencilOp {
            canvas_id,
            fail,
            zfail,
            zpass,
        },
    );
}

#[op2(fast)]
pub fn op_stencil_op_separate(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] face: u32,
    #[smi] fail: u32,
    #[smi] zfail: u32,
    #[smi] zpass: u32,
) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::StencilOpSeparate {
            canvas_id,
            face,
            fail,
            zfail,
            zpass,
        },
    );
}

#[op2(fast)]
pub fn op_stencil_mask(state: &mut OpState, #[smi] canvas_id: u32, #[smi] mask: u32) {
    queue_gl_fire_and_forget(state, GLCmd::StencilMask { canvas_id, mask });
}

#[op2(fast)]
pub fn op_stencil_mask_separate(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] face: u32,
    #[smi] mask: u32,
) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::StencilMaskSeparate {
            canvas_id,
            face,
            mask,
        },
    );
}

#[op2(fast)]
pub fn op_cull_face(state: &mut OpState, #[smi] canvas_id: u32, #[smi] mode: u32) {
    queue_gl_fire_and_forget(state, GLCmd::CullFace { canvas_id, mode });
}

#[op2(fast)]
pub fn op_front_face(state: &mut OpState, #[smi] canvas_id: u32, #[smi] mode: u32) {
    queue_gl_fire_and_forget(state, GLCmd::FrontFace { canvas_id, mode });
}

#[op2(fast)]
pub fn op_color_mask(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    r: bool,
    g: bool,
    b: bool,
    a: bool,
) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::ColorMask {
            canvas_id,
            r,
            g,
            b,
            a,
        },
    );
}

#[op2(fast)]
pub fn op_scissor(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] x: i32,
    #[smi] y: i32,
    #[smi] width: i32,
    #[smi] height: i32,
) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::Scissor {
            canvas_id,
            x,
            y,
            width,
            height,
        },
    );
}

#[op2(fast)]
pub fn op_line_width(state: &mut OpState, #[smi] canvas_id: u32, width: f32) {
    queue_gl_fire_and_forget(state, GLCmd::LineWidth { canvas_id, width });
}

#[op2(fast)]
pub fn op_polygon_offset(state: &mut OpState, #[smi] canvas_id: u32, factor: f32, units: f32) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::PolygonOffset {
            canvas_id,
            factor,
            units,
        },
    );
}

// ---------------------------------------------------------------------------
// Phase 2B: Uniform Variants
// ---------------------------------------------------------------------------

#[op2(fast)]
pub fn op_uniform1i(state: &mut OpState, #[smi] canvas_id: u32, location: i32, #[smi] x: i32) {
    let location = if location < 0 {
        None
    } else {
        Some(location as u32)
    };
    queue_gl_fire_and_forget(
        state,
        GLCmd::Uniform1i {
            canvas_id,
            location,
            x,
        },
    );
}

#[op2(fast)]
pub fn op_uniform1f(state: &mut OpState, #[smi] canvas_id: u32, location: i32, x: f32) {
    let location = if location < 0 {
        None
    } else {
        Some(location as u32)
    };
    queue_gl_fire_and_forget(
        state,
        GLCmd::Uniform1f {
            canvas_id,
            location,
            x,
        },
    );
}

#[op2(fast)]
pub fn op_uniform2f(state: &mut OpState, #[smi] canvas_id: u32, location: i32, x: f32, y: f32) {
    let location = if location < 0 {
        None
    } else {
        Some(location as u32)
    };
    queue_gl_fire_and_forget(
        state,
        GLCmd::Uniform2f {
            canvas_id,
            location,
            x,
            y,
        },
    );
}

#[op2(fast)]
pub fn op_uniform4f(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    location: i32,
    x: f32,
    y: f32,
    z: f32,
    w: f32,
) {
    let location = if location < 0 {
        None
    } else {
        Some(location as u32)
    };
    queue_gl_fire_and_forget(
        state,
        GLCmd::Uniform4f {
            canvas_id,
            location,
            x,
            y,
            z,
            w,
        },
    );
}

#[op2(fast)]
pub fn op_uniform1iv(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    location: i32,
    #[buffer(copy)] value: Vec<u32>,
) {
    let location = if location < 0 {
        None
    } else {
        Some(location as u32)
    };
    let value: Vec<i32> = cast_vec(value);
    queue_gl_fire_and_forget(
        state,
        GLCmd::Uniform1iv {
            canvas_id,
            location,
            value,
        },
    );
}

#[op2(fast)]
pub fn op_uniform1fv(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    location: i32,
    #[buffer(copy)] value: Vec<u32>,
) {
    let location = if location < 0 {
        None
    } else {
        Some(location as u32)
    };
    let value: Vec<f32> = cast_vec(value);
    queue_gl_fire_and_forget(
        state,
        GLCmd::Uniform1fv {
            canvas_id,
            location,
            value,
        },
    );
}

#[op2(fast)]
pub fn op_uniform2iv(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    location: i32,
    #[buffer(copy)] value: Vec<u32>,
) {
    let location = if location < 0 {
        None
    } else {
        Some(location as u32)
    };
    let value: Vec<i32> = cast_vec(value);
    queue_gl_fire_and_forget(
        state,
        GLCmd::Uniform2iv {
            canvas_id,
            location,
            value,
        },
    );
}

#[op2(fast)]
pub fn op_uniform2fv(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    location: i32,
    #[buffer(copy)] value: Vec<u32>,
) {
    let location = if location < 0 {
        None
    } else {
        Some(location as u32)
    };
    let value: Vec<f32> = cast_vec(value);
    queue_gl_fire_and_forget(
        state,
        GLCmd::Uniform2fv {
            canvas_id,
            location,
            value,
        },
    );
}

#[op2(fast)]
pub fn op_uniform3iv(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    location: i32,
    #[buffer(copy)] value: Vec<u32>,
) {
    let location = if location < 0 {
        None
    } else {
        Some(location as u32)
    };
    let value: Vec<i32> = cast_vec(value);
    queue_gl_fire_and_forget(
        state,
        GLCmd::Uniform3iv {
            canvas_id,
            location,
            value,
        },
    );
}

#[op2(fast)]
pub fn op_uniform3fv(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    location: i32,
    #[buffer(copy)] value: Vec<u32>,
) {
    let location = if location < 0 {
        None
    } else {
        Some(location as u32)
    };
    let value: Vec<f32> = cast_vec(value);
    queue_gl_fire_and_forget(
        state,
        GLCmd::Uniform3fv {
            canvas_id,
            location,
            value,
        },
    );
}

#[op2(fast)]
pub fn op_uniform4iv(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    location: i32,
    #[buffer(copy)] value: Vec<u32>,
) {
    let location = if location < 0 {
        None
    } else {
        Some(location as u32)
    };
    let value: Vec<i32> = cast_vec(value);
    queue_gl_fire_and_forget(
        state,
        GLCmd::Uniform4iv {
            canvas_id,
            location,
            value,
        },
    );
}

#[op2(fast)]
pub fn op_uniform4fv(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    location: i32,
    #[buffer(copy)] value: Vec<u32>,
) {
    let location = if location < 0 {
        None
    } else {
        Some(location as u32)
    };
    let value: Vec<f32> = cast_vec(value);
    queue_gl_fire_and_forget(
        state,
        GLCmd::Uniform4fv {
            canvas_id,
            location,
            value,
        },
    );
}

#[op2(fast)]
pub fn op_uniform_matrix_2fv(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    location: i32,
    transpose: bool,
    #[buffer(copy)] value: Vec<u32>,
) {
    let location = if location < 0 {
        None
    } else {
        Some(location as u32)
    };
    let value: Vec<f32> = cast_vec(value);
    queue_gl_fire_and_forget(
        state,
        GLCmd::UniformMatrix2fv {
            canvas_id,
            location,
            transpose,
            value,
        },
    );
}

#[op2(fast)]
pub fn op_uniform_matrix_4fv(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    location: i32,
    transpose: bool,
    #[buffer(copy)] value: Vec<u32>,
) {
    let location = if location < 0 {
        None
    } else {
        Some(location as u32)
    };
    let value: Vec<f32> = cast_vec(value);
    queue_gl_fire_and_forget(
        state,
        GLCmd::UniformMatrix4fv {
            canvas_id,
            location,
            transpose,
            value,
        },
    );
}

// ---------------------------------------------------------------------------
// Phase 3A: Framebuffer / Renderbuffer
// ---------------------------------------------------------------------------

#[op2(fast)]
pub fn op_create_framebuffer(state: &mut OpState, #[smi] client_id: u32) {
    queue_gl_fire_and_forget(state, GLCmd::CreateFramebuffer { client_id });
}

#[op2(fast)]
pub fn op_delete_framebuffer(state: &mut OpState, #[smi] framebuffer_id: u32) {
    queue_gl_fire_and_forget(state, GLCmd::DeleteFramebuffer { framebuffer_id });
}

#[op2(fast)]
pub fn op_bind_framebuffer(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] target: u32,
    framebuffer: i32,
) {
    let framebuffer = if framebuffer < 0 {
        None
    } else {
        Some(framebuffer as u32)
    };
    queue_gl_fire_and_forget(
        state,
        GLCmd::BindFramebuffer {
            canvas_id,
            target,
            framebuffer,
        },
    );
}

#[op2(fast)]
pub fn op_framebuffer_texture_2d(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] target: u32,
    #[smi] attachment: u32,
    #[smi] textarget: u32,
    texture: i32,
    #[smi] level: i32,
) {
    let texture = if texture < 0 {
        None
    } else {
        Some(texture as u32)
    };
    queue_gl_fire_and_forget(
        state,
        GLCmd::FramebufferTexture2D {
            canvas_id,
            target,
            attachment,
            textarget,
            texture,
            level,
        },
    );
}

#[op2(fast)]
pub fn op_framebuffer_renderbuffer(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] target: u32,
    #[smi] attachment: u32,
    #[smi] renderbuffertarget: u32,
    renderbuffer: i32,
) {
    let renderbuffer = if renderbuffer < 0 {
        None
    } else {
        Some(renderbuffer as u32)
    };
    queue_gl_fire_and_forget(
        state,
        GLCmd::FramebufferRenderbuffer {
            canvas_id,
            target,
            attachment,
            renderbuffertarget,
            renderbuffer,
        },
    );
}

#[op2(fast)]
pub fn op_check_framebuffer_status(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] target: u32,
) -> u32 {
    send_gl_sync_with_flush(state, |resp| {
        RenderCommand::GL(GLCmd::CheckFramebufferStatus {
            canvas_id,
            target,
            resp,
        })
    })
    .unwrap_or(0)
}

#[op2(fast)]
pub fn op_create_renderbuffer(state: &mut OpState, #[smi] client_id: u32) {
    queue_gl_fire_and_forget(state, GLCmd::CreateRenderbuffer { client_id });
}

#[op2(fast)]
pub fn op_delete_renderbuffer(state: &mut OpState, #[smi] renderbuffer_id: u32) {
    queue_gl_fire_and_forget(state, GLCmd::DeleteRenderbuffer { renderbuffer_id });
}

#[op2(fast)]
pub fn op_bind_renderbuffer(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] target: u32,
    renderbuffer: i32,
) {
    let renderbuffer = if renderbuffer < 0 {
        None
    } else {
        Some(renderbuffer as u32)
    };
    queue_gl_fire_and_forget(
        state,
        GLCmd::BindRenderbuffer {
            canvas_id,
            target,
            renderbuffer,
        },
    );
}

#[op2(fast)]
pub fn op_renderbuffer_storage(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] target: u32,
    #[smi] internalformat: u32,
    #[smi] width: i32,
    #[smi] height: i32,
) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::RenderbufferStorage {
            canvas_id,
            target,
            internalformat,
            width,
            height,
        },
    );
}

// ---------------------------------------------------------------------------
// Phase 3B: Misc
// ---------------------------------------------------------------------------

#[op2]
#[buffer]
pub fn op_read_pixels(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] x: i32,
    #[smi] y: i32,
    #[smi] width: i32,
    #[smi] height: i32,
    #[smi] format: u32,
    #[smi] type_: u32,
) -> Vec<u8> {
    send_gl_sync_with_flush(state, |resp| {
        RenderCommand::GL(GLCmd::ReadPixels {
            canvas_id,
            x,
            y,
            width,
            height,
            format,
            type_,
            resp,
        })
    })
    .unwrap_or_default()
}

#[op2(fast)]
pub fn op_hint(state: &mut OpState, #[smi] canvas_id: u32, #[smi] target: u32, #[smi] mode: u32) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::Hint {
            canvas_id,
            target,
            mode,
        },
    );
}
