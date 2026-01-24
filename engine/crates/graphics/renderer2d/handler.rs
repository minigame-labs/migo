use std::collections::HashMap;

use crate::CanvasManager;
use shared::{
    error::{EngineError, EngineResult, ErrorCode},
    protocol::render_cmd::Canvas2DCmd,
};
use tracing::trace;

pub(crate) struct Renderer2d;

impl Renderer2d {
    pub(crate) fn new() -> Self {
        Self
    }

    pub(crate) fn handle_command(
        &mut self,
        cm: &mut CanvasManager,
        canvas_id: shared::protocol::render_cmd::CanvasId,
        cmd: Canvas2DCmd,
    ) -> EngineResult<bool> {
        if let Canvas2DCmd::CreateContext2D { resp } = cmd {
            cm.init_femtovg_for_canvas(canvas_id)?;
            let _ = resp.send(Ok(canvas_id));
            return Ok(false);
        }

        cm.make_current_needed(canvas_id)?;
        let context = cm.get_2d_context_mut(canvas_id)?;

        match cmd {
            Canvas2DCmd::CreateContext2D { .. } => unreachable!(),

            Canvas2DCmd::BeginPath => {
                context.begin_path();
                Ok(false)
            }
            Canvas2DCmd::MoveTo { x, y } => {
                context.move_to(x, y);
                Ok(false)
            }
            Canvas2DCmd::LineTo { x, y } => {
                context.line_to(x, y);
                Ok(false)
            }
            Canvas2DCmd::ClosePath => {
                context.close_path();
                Ok(false)
            }
            Canvas2DCmd::Arc { x, y, radius, start_angle, end_angle, counterclockwise } => {
                context.current_path.arc(
                    x,
                    y,
                    radius,
                    start_angle,
                    end_angle,
                    if counterclockwise { femtovg::Solidity::Hole } else { femtovg::Solidity::Solid },
                );
                Ok(false)
            }

            Canvas2DCmd::SetFillStyle { color } => {
                trace!("SetFillStyle: canvas_id={:?}, color={:?}", canvas_id, color);
                context.set_fill_style_color(color);
                Ok(false)
            }

            Canvas2DCmd::SetStrokeStyle { color } => {
                trace!("SetStrokeStyle: canvas_id={:?}, color={:?}", canvas_id, color);
                context.set_stroke_style_color(color);
                Ok(false)
            }

            Canvas2DCmd::SetLineWidth { width } => {
                context.set_line_width(width);
                Ok(false)
            }

            Canvas2DCmd::Save => {
                context.save();
                Ok(false)
            }

            Canvas2DCmd::Restore => {
                context.restore();
                Ok(false)
            }

            Canvas2DCmd::ClearRect { x, y, w, h } => {
                context.clear_rect(x, y, w, h);
                Ok(true)
            }

            Canvas2DCmd::Fill => {
                context.fill();
                Ok(true)
            }

            Canvas2DCmd::Stroke => {
                context.stroke();
                Ok(true)
            }

            Canvas2DCmd::SetTransform { a, b, c, d, e, f } => {
                context.set_transform(a, b, c, d, e, f);
                Ok(false)
            }

            Canvas2DCmd::ResetTransform => {
                context.reset_transform();
                Ok(false)
            }

            #[allow(unused)]
            Canvas2DCmd::FillText { text, x, y , max_width} => {
                context.fill_text(&text, x, y);
                Ok(true)
            }

            Canvas2DCmd::SetFont { font } => {
                let (family, size, bold, italic) = context.font_manager.parse_font_string(&font);
                let font_id = match context.font_manager.get_font_id_with_style(&family, bold, italic) {
                    Some(id) => Some(id),
                    None => context.font_manager.get_default_font_id(),
                };

                let font_id = match font_id {
                    Some(id) => id,
                    None => {
                        shared::bail!(
                            ErrorCode::NotFound,
                            "font not found (and default font not found)",
                            font
                        );
                    }
                };

                context.state.font_id = Some(font_id);
                context.state.font_size = size.unwrap_or(16.0);

                Ok(false)
            }

            Canvas2DCmd::DrawImage { image_id, sx, sy, sw, sh, dx, dy, dw, dh } => {
                // Check if image is already owned on this canvas
                if let Some((fv_id, _native_tex, info)) = cm.get_owned_fv_image(image_id, canvas_id) {
                    let (width, height) = (info.width() as f32, info.height() as f32);
                    cm.get_2d_context_mut(canvas_id)?.draw_image_rect(
                        fv_id, (width, height), sx, sy, sw, sh, dx, dy, dw, dh,
                    );
                    return Ok(true);
                }

                // Get shared image resource
                let resource_state = cm.get_shared_fv_image(image_id).ok_or_else(|| {
                    EngineError::from_detail(
                        ErrorCode::NotFound,
                        format!("shared image not found: image_id={}", image_id),
                    )
                })?;

                // Create texture from native and draw in one context access
                let ctx = cm.get_2d_context_mut(canvas_id)?;
                let new_id = ctx
                    .canvas
                    .create_image_from_native_texture(resource_state.1, resource_state.2)
                    .map_err(|e| {
                        EngineError::from_detail(
                            ErrorCode::Render2DResourceError,
                            format!("create_image_from_native_texture failed: {:?}", e),
                        )
                    })?;

                // Cache image dimensions before releasing context borrow
                let (img_width, img_height) = (
                    resource_state.2.width() as f32,
                    resource_state.2.height() as f32,
                );

                // Draw immediately while we have the context
                ctx.draw_image_rect(
                    new_id, (img_width, img_height), sx, sy, sw, sh, dx, dy, dw, dh,
                );

                // Now register the owned image (context borrow released)
                cm.fv_images_mut()
                    .entry(image_id)
                    .or_insert_with(HashMap::new)
                    .insert(canvas_id, (new_id, resource_state.1, resource_state.2));

                Ok(true)
            }

            _ => {
                shared::bail!(ErrorCode::NotImplemented, "Canvas2D command not covered by Renderer2d");
            }
        }
    }
}
