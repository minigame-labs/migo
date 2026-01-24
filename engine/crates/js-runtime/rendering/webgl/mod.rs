use deno_core::extension;

mod context2d;
mod raf;
mod webgl;

use context2d::*;
use raf::*;
use webgl::*;

extension!(host_v8_webgl,
    deps = [host_v8_console, host_v8_base],
    ops = [
        op_viewport,
        op_clear,
        op_clear_color,

        op_register_raf_callback,
        op_cancel_raf_callback,

        op_create_program,
        op_use_program,
        op_link_program,
        op_get_program_parameter,
        op_get_program_info_log,
        op_delete_program,

        op_create_shader,
        op_shader_source,
        op_compile_shader,
        op_attach_shader,
        op_delete_shader,
        op_get_shader_parameter,
        op_get_shader_info_log,

        op_draw_arrays,
        op_draw_elements,

        op_get_attrib_location,
        op_enable_vertex_attrib_array,
        op_vertex_attrib_pointer,

        op_create_buffer,
        op_bind_buffer,
        op_buffer_data,

        op_get_uniform_location,
        op_uniform3f,
        op_uniform_matrix_3fv,

        // 2d
        op_create_context_2d,
        op_arc,
        op_set_fill_style,
        op_set_stroke_style,
        op_set_line_width,
        op_fill,
        op_line_to,
        op_fill_rect,
        op_stroke_rect,
        op_clear_rect,
        op_begin_path,
        op_move_to,
        op_stroke,
        op_fill_text,
        op_stroke_text,
        op_set_font,
        op_draw_image,
    ],
    esm = [
        dir "rendering/webgl",
        "01_constants.js",
        "02_2d_context.js",
        "02_webgl_context.js",
        "03_raf.js",
    ],
    state = |state| {
        state.put(CallbackMap::default());
    }
);

pub(super) fn webgl_extensions() -> Vec<deno_core::Extension> {
    vec![host_v8_webgl::init_ops_and_esm()]
}
