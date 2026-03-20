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
        // Handle CreateContext2D separately
        if let Canvas2DCmd::CreateContext2D { resp } = cmd {
            cm.init_femtovg_for_canvas(canvas_id)?;
            let _ = resp.send(Ok(canvas_id));
            return Ok(false);
        }

        // Handle MeasureText (synchronous response)
        if let Canvas2DCmd::MeasureText { text, resp } = cmd {
            cm.make_current_needed(canvas_id)?;
            let context = cm.get_2d_context_mut(canvas_id)?;
            let metrics = context.measure_text(&text);
            let _ = resp.send(Ok(metrics));
            return Ok(false);
        }

        // Handle GetImageData (synchronous response) - read pixels from framebuffer
        if let Canvas2DCmd::GetImageData {
            x,
            y,
            width,
            height,
            resp,
        } = cmd
        {
            cm.make_current_needed(canvas_id)?;
            // Flush pending 2D draw commands so the framebuffer is up to date
            if let Ok(ctx) = cm.get_2d_context_mut(canvas_id) {
                ctx.flush();
            }
            let data = cm.read_pixels(x, y, width, height);
            let _ = resp.send(Ok(data));
            return Ok(false);
        }

        cm.make_current_needed(canvas_id)?;
        let context = cm.get_2d_context_mut(canvas_id)?;

        match cmd {
            Canvas2DCmd::CreateContext2D { .. }
            | Canvas2DCmd::MeasureText { .. }
            | Canvas2DCmd::GetImageData { .. } => unreachable!(),

            // Path methods
            Canvas2DCmd::BeginPath => {
                context.begin_path();
                Ok(false)
            }
            Canvas2DCmd::ClosePath => {
                context.close_path();
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
            Canvas2DCmd::QuadraticCurveTo { cpx, cpy, x, y } => {
                context.quadratic_curve_to(cpx, cpy, x, y);
                Ok(false)
            }
            Canvas2DCmd::BezierCurveTo {
                cp1x,
                cp1y,
                cp2x,
                cp2y,
                x,
                y,
            } => {
                context.bezier_curve_to(cp1x, cp1y, cp2x, cp2y, x, y);
                Ok(false)
            }
            Canvas2DCmd::Arc {
                x,
                y,
                radius,
                start_angle,
                end_angle,
                counterclockwise,
            } => {
                context.arc(x, y, radius, start_angle, end_angle, counterclockwise);
                Ok(false)
            }
            Canvas2DCmd::ArcTo {
                x1,
                y1,
                x2,
                y2,
                radius,
            } => {
                context.arc_to(x1, y1, x2, y2, radius);
                Ok(false)
            }
            Canvas2DCmd::Rect { x, y, w, h } => {
                context.rect(x, y, w, h);
                Ok(false)
            }
            Canvas2DCmd::Ellipse {
                x,
                y,
                radius_x,
                radius_y,
                rotation,
                start_angle,
                end_angle,
                counterclockwise,
            } => {
                context.ellipse(
                    x,
                    y,
                    radius_x,
                    radius_y,
                    rotation,
                    start_angle,
                    end_angle,
                    counterclockwise,
                );
                Ok(false)
            }

            // Drawing methods
            Canvas2DCmd::Fill => {
                context.fill();
                Ok(true)
            }
            Canvas2DCmd::Stroke => {
                context.stroke();
                Ok(true)
            }
            Canvas2DCmd::Clip => {
                context.clip();
                Ok(false)
            }

            // Rectangle methods
            Canvas2DCmd::FillRect { x, y, w, h } => {
                context.fill_rect(x, y, w, h);
                Ok(true)
            }
            Canvas2DCmd::StrokeRect { x, y, w, h } => {
                context.stroke_rect(x, y, w, h);
                Ok(true)
            }
            Canvas2DCmd::ClearRect { x, y, w, h } => {
                context.clear_rect(x, y, w, h);
                Ok(true)
            }

            // Text methods
            Canvas2DCmd::FillText {
                text,
                x,
                y,
                max_width,
            } => {
                context.fill_text(&text, x, y, max_width);
                Ok(true)
            }
            Canvas2DCmd::StrokeText {
                text,
                x,
                y,
                max_width,
            } => {
                context.stroke_text(&text, x, y, max_width);
                Ok(true)
            }

            // Style setters
            Canvas2DCmd::SetFillStyle { color } => {
                trace!("SetFillStyle: {:?}", color);
                context.set_fill_style_color(color);
                Ok(false)
            }
            Canvas2DCmd::SetStrokeStyle { color } => {
                trace!("SetStrokeStyle: {:?}", color);
                context.set_stroke_style_color(color);
                Ok(false)
            }
            Canvas2DCmd::SetLineWidth { width } => {
                context.set_line_width(width);
                Ok(false)
            }
            Canvas2DCmd::SetLineCap { cap } => {
                context.set_line_cap(cap);
                Ok(false)
            }
            Canvas2DCmd::SetLineJoin { join } => {
                context.set_line_join(join);
                Ok(false)
            }
            Canvas2DCmd::SetMiterLimit { limit } => {
                context.set_miter_limit(limit);
                Ok(false)
            }
            Canvas2DCmd::SetGlobalAlpha { alpha } => {
                context.set_global_alpha(alpha);
                Ok(false)
            }
            Canvas2DCmd::SetCompositeOperation { op } => {
                context.set_composite_operation(op);
                Ok(false)
            }
            Canvas2DCmd::SetLineDash { segments } => {
                context.set_line_dash(segments);
                Ok(false)
            }
            Canvas2DCmd::SetLineDashOffset { offset } => {
                context.set_line_dash_offset(offset);
                Ok(false)
            }
            Canvas2DCmd::SetFillStyleGradient {
                x0,
                y0,
                x1,
                y1,
                stops,
            } => {
                context.set_fill_style_gradient(x0, y0, x1, y1, stops);
                Ok(false)
            }
            Canvas2DCmd::SetTextAlign { align } => {
                context.set_text_align(align);
                Ok(false)
            }
            Canvas2DCmd::SetTextBaseline { baseline } => {
                context.set_text_baseline(baseline);
                Ok(false)
            }
            Canvas2DCmd::SetFont { font } => {
                // Fast path: skip parsing if identical to last SetFont
                if font == context.last_font_str {
                    context.state.font_id = context.last_font_id;
                    context.state.font_size = context.last_font_size;
                    return Ok(false);
                }
                let (family, size, bold, italic) = context.font_manager.parse_font_string(&font);
                let font_id = context
                    .font_manager
                    .get_font_id_with_style(&family, bold, italic)
                    .or_else(|| context.font_manager.get_default_font_id());
                match font_id {
                    Some(id) => {
                        let sz = size.unwrap_or(16.0);
                        context.state.font_id = Some(id);
                        context.state.font_size = sz;
                        // Cache for next call
                        context.last_font_id = Some(id);
                        context.last_font_size = sz;
                        context.last_font_str = font;
                        Ok(false)
                    }
                    None => {
                        shared::bail!(ErrorCode::NotFound, "font not found", font);
                    }
                }
            }

            // State methods
            Canvas2DCmd::Save => {
                context.save();
                Ok(false)
            }
            Canvas2DCmd::Restore => {
                context.restore();
                Ok(false)
            }

            // Transform methods
            Canvas2DCmd::SetTransform { a, b, c, d, e, f } => {
                context.set_transform(a, b, c, d, e, f);
                Ok(false)
            }
            Canvas2DCmd::ResetTransform => {
                context.reset_transform();
                Ok(false)
            }
            Canvas2DCmd::Translate { x, y } => {
                context.translate(x, y);
                Ok(false)
            }
            Canvas2DCmd::Rotate { angle } => {
                context.rotate(angle);
                Ok(false)
            }
            Canvas2DCmd::Scale { x, y } => {
                context.scale(x, y);
                Ok(false)
            }

            // Image methods
            Canvas2DCmd::DrawImage {
                image_id,
                sx,
                sy,
                sw,
                sh,
                dx,
                dy,
                dw,
                dh,
            } => {
                if let Some((fv_id, _, info)) = cm.get_owned_fv_image(image_id, canvas_id) {
                    let (w, h) = (info.width() as f32, info.height() as f32);
                    cm.get_2d_context_mut(canvas_id)?.draw_image_rect(
                        fv_id,
                        (w, h),
                        sx,
                        sy,
                        sw,
                        sh,
                        dx,
                        dy,
                        dw,
                        dh,
                    );
                    return Ok(true);
                }
                let res = cm.get_shared_fv_image(image_id).ok_or_else(|| {
                    EngineError::from_detail(
                        ErrorCode::NotFound,
                        format!("shared image not found: {}", image_id),
                    )
                })?;
                let ctx = cm.get_2d_context_mut(canvas_id)?;
                let new_id = ctx
                    .canvas
                    .create_image_from_native_texture(res.1, res.2)
                    .map_err(|e| {
                        EngineError::from_detail(
                            ErrorCode::Render2DResourceError,
                            format!("{:?}", e),
                        )
                    })?;
                let (iw, ih) = (res.2.width() as f32, res.2.height() as f32);
                ctx.draw_image_rect(new_id, (iw, ih), sx, sy, sw, sh, dx, dy, dw, dh);
                cm.fv_images_mut()
                    .entry(image_id)
                    .or_insert_with(HashMap::new)
                    .insert(canvas_id, (new_id, res.1, res.2));
                Ok(true)
            }

            // Batch image drawing for better performance
            Canvas2DCmd::DrawImageBatch { draws } => {
                if draws.is_empty() {
                    return Ok(false);
                }

                // Process all draws, ensuring images are ready
                for draw in &draws {
                    // Ensure image is available in canvas
                    if cm.get_owned_fv_image(draw.image_id, canvas_id).is_none() {
                        if let Some(res) = cm.get_shared_fv_image(draw.image_id) {
                            let ctx = cm.get_2d_context_mut(canvas_id)?;
                            let new_id = ctx
                                .canvas
                                .create_image_from_native_texture(res.1, res.2)
                                .map_err(|e| {
                                    EngineError::from_detail(
                                        ErrorCode::Render2DResourceError,
                                        format!("{:?}", e),
                                    )
                                })?;
                            cm.fv_images_mut()
                                .entry(draw.image_id)
                                .or_insert_with(HashMap::new)
                                .insert(canvas_id, (new_id, res.1, res.2));
                        }
                    }
                }

                // Now draw all images
                for draw in draws {
                    if let Some((fv_id, _, info)) = cm.get_owned_fv_image(draw.image_id, canvas_id)
                    {
                        let (w, h) = (info.width() as f32, info.height() as f32);
                        cm.get_2d_context_mut(canvas_id)?.draw_image_rect(
                            fv_id,
                            (w, h),
                            draw.sx,
                            draw.sy,
                            draw.sw,
                            draw.sh,
                            draw.dx,
                            draw.dy,
                            draw.dw,
                            draw.dh,
                        );
                    }
                }
                Ok(true)
            }

            #[allow(unreachable_patterns)]
            _ => {
                shared::bail!(
                    ErrorCode::NotImplemented,
                    "Canvas2D command not implemented"
                );
            }
        }
    }
}
