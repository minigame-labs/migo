use deno_core::extension;

mod context2d;
mod context2d_v2;
mod raf;
mod webgl;

use context2d::*;
use context2d_v2::*;
use raf::*;
use webgl::*;

// Re-export FrameCommandCollector for use in OpState initialization
pub use context2d_v2::FrameCommandCollector;

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

        // 2D Context (shared: context creation, sync queries)
        op_create_context_2d,
        op_measure_text,
        op_get_image_data,

        // Frame lifecycle
        op_frame_begin,
        op_frame_end,
        op_frame_end_all,
        op_invalidate,

        // Batched path methods
        op_begin_path_v2,
        op_close_path_v2,
        op_move_to_v2,
        op_line_to_v2,
        op_quadratic_curve_to_v2,
        op_bezier_curve_to_v2,
        op_arc_v2,
        op_arc_to_v2,
        op_rect_v2,
        op_ellipse_v2,

        // Batched drawing methods
        op_fill_v2,
        op_stroke_v2,
        op_clip_v2,

        // Batched rectangle methods
        op_fill_rect_v2,
        op_stroke_rect_v2,
        op_clear_rect_v2,

        // Batched text methods
        op_fill_text_v2,
        op_stroke_text_v2,

        // Batched style setters
        op_set_fill_style_v2,
        op_set_stroke_style_v2,
        op_set_line_width_v2,
        op_set_line_cap_v2,
        op_set_line_join_v2,
        op_set_miter_limit_v2,
        op_set_global_alpha_v2,
        op_set_font_v2,
        op_set_text_align_v2,
        op_set_text_baseline_v2,

        // Batched state methods
        op_save_v2,
        op_restore_v2,

        // Batched transform methods
        op_translate_v2,
        op_rotate_v2,
        op_scale_v2,
        op_set_transform_v2,
        op_reset_transform_v2,

        // Batched image methods
        op_draw_image_v2,
        op_draw_image_batch_v2,
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
        state.put(FrameCommandCollector::new());
    }
);

pub(super) fn webgl_extensions() -> Vec<deno_core::Extension> {
    vec![host_v8_webgl::init_ops_and_esm()]
}
