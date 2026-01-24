use bytemuck::cast_slice;
use deno_core::{JsBuffer, OpState, op2};
use tracing::error;

use shared::{
    op_state::CanvasOpState,
    protocol::{
        render_cmd::{GLCmd, RenderCommand, ShaderType},
        send_gl, send_gl_with_resp_sync,
    },
};

#[op2(fast)]
pub fn op_viewport(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] x: i32,
    #[smi] y: i32,
    #[smi] width: u32,
    #[smi] height: u32,
) {
    let ctx = state.borrow::<CanvasOpState>();
    send_gl(
        ctx,
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
    let ctx = state.borrow::<CanvasOpState>();
    send_gl(
        ctx,
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
    let ctx = state.borrow::<CanvasOpState>();
    send_gl(
        ctx,
        GLCmd::Clear {
            canvas_id,
            bit_field,
        },
    );
}

#[op2(fast)]
pub fn op_create_program(state: &mut OpState) -> i32 {
    let ctx = state.borrow::<CanvasOpState>();
    match send_gl_with_resp_sync(ctx, |resp| RenderCommand::GL(GLCmd::CreateProgram { resp })) {
        Ok(id) => id as i32,
        Err(e) => {
            error!("create program failed: {e}");
            -1
        }
    }
}

#[op2(fast)]
pub fn op_use_program(state: &mut OpState, #[smi] canvas_id: u32, #[smi] program_id: u32) {
    let ctx = state.borrow::<CanvasOpState>();
    send_gl(
        ctx,
        GLCmd::UseProgram {
            canvas_id,
            program_id,
        },
    );
}

#[op2(fast)]
pub fn op_link_program(state: &mut OpState, #[smi] program_id: u32) -> bool {
    let ctx = state.borrow::<CanvasOpState>();
    send_gl_with_resp_sync(ctx, |resp| {
        RenderCommand::GL(GLCmd::LinkProgram { program_id, resp })
    })
    .unwrap_or(false)
}

#[op2(fast)]
pub fn op_get_program_parameter(
    state: &mut OpState,
    #[smi] program_id: u32,
    #[smi] pname: u32,
) -> i32 {
    let ctx = state.borrow::<CanvasOpState>();
    send_gl_with_resp_sync(ctx, |resp| {
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
    let ctx = state.borrow::<CanvasOpState>();
    send_gl_with_resp_sync(ctx, |resp| {
        RenderCommand::GL(GLCmd::GetProgramInfoLog { program_id, resp })
    })
    .ok()
    .flatten()
    .unwrap_or_default()
}

#[op2(fast)]
pub fn op_delete_program(state: &mut OpState, #[smi] program_id: u32) {
    let ctx = state.borrow::<CanvasOpState>();
    send_gl(ctx, GLCmd::DeleteProgram { program_id });
}

#[op2(fast)]
pub fn op_create_shader(state: &mut OpState, #[smi] ty: u32) -> i32 {
    let ctx = state.borrow::<CanvasOpState>();

    let shader_type = match ty {
        glow::VERTEX_SHADER => ShaderType::Vertex,
        glow::FRAGMENT_SHADER => ShaderType::Fragment,
        _ => {
            error!("unknown shader type: {}", ty);
            return -1;
        }
    };

    match send_gl_with_resp_sync(ctx, |resp| {
        RenderCommand::GL(GLCmd::CreateShader { shader_type, resp })
    }) {
        Ok(id) => id as i32,
        Err(e) => {
            error!("create shader failed: {e}");
            -1
        }
    }
}

#[op2(fast)]
pub fn op_shader_source(state: &mut OpState, #[smi] shader_id: u32, #[string] source: String) {
    let ctx = state.borrow::<CanvasOpState>();
    let _ = send_gl_with_resp_sync(ctx, |resp| {
        RenderCommand::GL(GLCmd::ShaderSource {
            shader_id,
            source,
            resp,
        })
    });
}

#[op2(fast)]
pub fn op_compile_shader(state: &mut OpState, #[smi] shader_id: u32) -> bool {
    let ctx = state.borrow::<CanvasOpState>();
    send_gl_with_resp_sync(ctx, |resp| {
        RenderCommand::GL(GLCmd::CompileShader { shader_id, resp })
    })
    .unwrap_or(false)
}

#[op2(fast)]
pub fn op_attach_shader(state: &mut OpState, #[smi] program_id: u32, #[smi] shader_id: u32) {
    let ctx = state.borrow::<CanvasOpState>();
    let _ = send_gl_with_resp_sync(ctx, |resp| {
        RenderCommand::GL(GLCmd::AttachShader {
            program_id,
            shader_id,
            resp,
        })
    });
}

#[op2(fast)]
pub fn op_get_shader_parameter(
    state: &mut OpState,
    #[smi] shader_id: u32,
    #[smi] pname: u32,
) -> i32 {
    let ctx = state.borrow::<CanvasOpState>();
    send_gl_with_resp_sync(ctx, |resp| {
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
    let ctx = state.borrow::<CanvasOpState>();
    send_gl_with_resp_sync(ctx, |resp| {
        RenderCommand::GL(GLCmd::GetShaderInfoLog { shader_id, resp })
    })
    .ok()
    .flatten()
    .unwrap_or_default()
}

#[op2(fast)]
pub fn op_delete_shader(state: &mut OpState, #[smi] shader_id: u32) {
    let ctx = state.borrow::<CanvasOpState>();
    send_gl(ctx, GLCmd::DeleteShader { shader_id });
}

#[op2(fast)]
pub fn op_draw_arrays(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] mode: u32,
    #[smi] first: i32,
    #[smi] count: i32,
) {
    let ctx = state.borrow::<CanvasOpState>();
    send_gl(
        ctx,
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
    let ctx = state.borrow::<CanvasOpState>();
    send_gl(
        ctx,
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
    let ctx = state.borrow::<CanvasOpState>();
    send_gl_with_resp_sync(ctx, |resp| {
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

#[op2(fast)]
pub fn op_enable_vertex_attrib_array(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] index: u32,
) {
    let ctx = state.borrow::<CanvasOpState>();
    send_gl(ctx, GLCmd::EnableVertexAttribArray { canvas_id, index });
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
    let ctx = state.borrow::<CanvasOpState>();
    send_gl(
        ctx,
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
pub fn op_create_buffer(state: &mut OpState) -> i32 {
    let ctx = state.borrow::<CanvasOpState>();
    send_gl_with_resp_sync(ctx, |resp| RenderCommand::GL(GLCmd::CreateBuffer { resp }))
        .map(|id| id as i32)
        .unwrap_or(-1)
}

#[op2(fast)]
pub fn op_bind_buffer(state: &mut OpState, #[smi] canvas_id: u32, #[smi] target: u32, buffer: i32) {
    let ctx = state.borrow::<CanvasOpState>();
    let buffer = if buffer < 0 {
        None
    } else {
        Some(buffer as u32)
    };
    send_gl(ctx, GLCmd::BindBuffer {canvas_id, target, buffer });
}

#[op2]
pub fn op_buffer_data(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] target: u32,
    #[smi] size: i32,
    #[buffer] data: Option<JsBuffer>,
    #[smi] usage: u32,
) {
    let ctx = state.borrow::<CanvasOpState>();

    let data = data.map(|b| b.as_ref().to_vec());
    if data.is_none() && size <= 0 {
        error!("op_buffer_data: size must > 0 when data is None");
        return;
    }

    send_gl(
        ctx,
        GLCmd::BufferData {
            canvas_id,
            target,
            size,
            data,
            usage,
        },
    );
}

#[op2]
pub fn op_get_uniform_location(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] program_id: u32,
    #[string] name: String,
) -> Option<u32> {
    let ctx = state.borrow::<CanvasOpState>();
    send_gl_with_resp_sync(ctx, |resp| {
        RenderCommand::GL(GLCmd::GetUniformLocation {
            canvas_id,
            program_id,
            name,
            resp,
        })
    })
    .ok()
    .flatten()
}

#[op2]
pub fn op_uniform3f(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    location: Option<u32>,
    x: f32,
    y: f32,
    z: f32,
) {
    let ctx = state.borrow::<CanvasOpState>();
    send_gl(
        ctx,
        GLCmd::Uniform3f {
            canvas_id,
            location,
            x,
            y,
            z,
        },
    );
}

#[op2]
pub fn op_uniform_matrix_3fv(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    location: Option<u32>,
    transpose: bool,
    #[buffer(copy)] value: Vec<u32>,
) {
    let ctx = state.borrow::<CanvasOpState>();

    let value: Vec<f32> = cast_slice(&value).to_vec();

    send_gl(
        ctx,
        GLCmd::UniformMatrix3fv {
            canvas_id,
            location,
            transpose,
            value,
        },
    );
}
