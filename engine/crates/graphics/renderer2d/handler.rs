use std::collections::HashMap;

use crate::CanvasManager;
use shared::{
    error::{EngineError, EngineResult, ErrorCode},
    protocol::render_cmd::{Canvas2DCmd, CanvasId, DrawImageEntry},
};
use tracing::trace;

use super::{
    display_list::{DisplayList, DisplayOp},
    font::{
        lookup_text_layout, store_text_layout, text_layout_font_key, CachedTextLayout,
        TextLayoutKey,
    },
    layer_cache::LayerCache,
    sprite_batch::SpriteBatcher,
    Canvas2DContext, Canvas2DState,
};

fn compact_display_list(list: DisplayList) -> DisplayList {
    list.compact()
}

fn can_use_direct_image_batch(state: &Canvas2DState) -> bool {
    let is_identity = state.transform == femtovg::Transform2D::identity();
    let is_source_over = state.composite_op == femtovg::CompositeOperation::SourceOver;
    let has_shadow = state.shadow_color.a > 0.0;
    is_identity && is_source_over && !has_shadow
}

/// Check whether the current Canvas2D state allows safe partial damage tracking.
/// Returns false when transform, shadow, or clip make bounds unpredictable.
pub(crate) fn state_allows_partial(state: &Canvas2DState) -> bool {
    let is_identity = state.transform == femtovg::Transform2D::identity();
    let has_shadow = state.shadow_color.a > 0.0
        && (state.shadow_blur > 0.0
            || state.shadow_offset_x != 0.0
            || state.shadow_offset_y != 0.0);
    is_identity && !has_shadow
}

/// Compute a conservative `DamageEffect` for a single Canvas2DCmd in a given state.
/// Used by execute_canvas_batch to feed the render-thread damage accumulator.
pub(crate) fn classify_draw_damage(
    cmd: &Canvas2DCmd,
    state: &Canvas2DState,
) -> crate::damage_effect::DamageEffect {
    use crate::damage_effect::DamageEffect;

    match cmd {
        // ── Rect-based draws ──
        Canvas2DCmd::FillRect { x, y, w, h } | Canvas2DCmd::ClearRect { x, y, w, h } => {
            if state_allows_partial(state) {
                DamageEffect::OnscreenRect {
                    x: x.floor() as i32,
                    y: y.floor() as i32,
                    width: w.ceil() as i32,
                    height: h.ceil() as i32,
                }
            } else {
                DamageEffect::FullSurface
            }
        }
        Canvas2DCmd::StrokeRect { x, y, w, h } => {
            if state_allows_partial(state) {
                let half_lw = (state.line_width / 2.0).ceil();
                DamageEffect::OnscreenRect {
                    x: (*x - half_lw).floor() as i32,
                    y: (*y - half_lw).floor() as i32,
                    width: (*w + state.line_width).ceil() as i32,
                    height: (*h + state.line_width).ceil() as i32,
                }
            } else {
                DamageEffect::FullSurface
            }
        }

        // ── Image draws ──
        Canvas2DCmd::DrawImage { dx, dy, dw, dh, .. } => {
            if state_allows_partial(state) {
                DamageEffect::OnscreenRect {
                    x: dx.floor() as i32,
                    y: dy.floor() as i32,
                    width: dw.ceil() as i32,
                    height: dh.ceil() as i32,
                }
            } else {
                DamageEffect::FullSurface
            }
        }
        Canvas2DCmd::DrawImageBatch { draws } => {
            if state_allows_partial(state) && !draws.is_empty() {
                let mut min_x = f32::MAX;
                let mut min_y = f32::MAX;
                let mut max_x = f32::MIN;
                let mut max_y = f32::MIN;
                for d in draws {
                    min_x = min_x.min(d.dx);
                    min_y = min_y.min(d.dy);
                    max_x = max_x.max(d.dx + d.dw);
                    max_y = max_y.max(d.dy + d.dh);
                }
                DamageEffect::OnscreenRect {
                    x: min_x.floor() as i32,
                    y: min_y.floor() as i32,
                    width: (max_x - min_x).ceil() as i32,
                    height: (max_y - min_y).ceil() as i32,
                }
            } else if draws.is_empty() {
                DamageEffect::NoDamage
            } else {
                DamageEffect::FullSurface
            }
        }

        // ── Always full-surface ──
        Canvas2DCmd::Fill | Canvas2DCmd::Stroke => DamageEffect::FullSurface,
        Canvas2DCmd::FillText { .. } | Canvas2DCmd::StrokeText { .. } => DamageEffect::FullSurface,

        // ── Non-draw commands ──
        _ => DamageEffect::NoDamage,
    }
}

fn prepare_display_list(draws: Vec<DrawImageEntry>) -> DisplayList {
    let mut list = DisplayList::new();
    for draw in draws {
        list.push(DisplayOp::draw_image(
            draw.image_id,
            draw.sx,
            draw.sy,
            draw.sw,
            draw.sh,
            draw.dx,
            draw.dy,
            draw.dw,
            draw.dh,
        ));
    }
    compact_display_list(list)
}

fn text_metrics_from_cached_layout(
    context: &mut Canvas2DContext,
    cached: &CachedTextLayout,
) -> shared::protocol::render_cmd::TextMetrics {
    let mut paint = context.build_fill_paint();
    if let Some(font_id) = context.state.font_id {
        paint.set_font(&[font_id]);
    }
    paint.set_font_size(context.state.font_size);

    let (ascender, descender) = context
        .canvas
        .measure_font(&paint)
        .map(|m| (m.ascender(), m.descender()))
        .unwrap_or((
            context.state.font_size * 0.8,
            context.state.font_size * -0.2,
        ));

    shared::protocol::render_cmd::TextMetrics {
        width: cached.width,
        actual_bounding_box_left: 0.0,
        actual_bounding_box_right: cached.width,
        actual_bounding_box_ascent: ascender,
        actual_bounding_box_descent: -descender,
        font_bounding_box_ascent: ascender,
        font_bounding_box_descent: -descender,
    }
}

fn measure_text_with_cache(
    context: &mut Canvas2DContext,
    text: &str,
    max_width: f32,
) -> shared::protocol::render_cmd::TextMetrics {
    let key = TextLayoutKey::new(text, &context.state.font_cache_key, max_width);

    if let Some(cached) = lookup_text_layout(&key) {
        return text_metrics_from_cached_layout(context, &cached);
    }

    let metrics = context.measure_text(text);
    store_text_layout(
        key,
        CachedTextLayout {
            width: metrics.width,
            height: metrics.font_bounding_box_ascent + metrics.font_bounding_box_descent,
        },
    );
    metrics
}

fn draw_single_image(
    cm: &mut CanvasManager,
    canvas_id: CanvasId,
    draw: DrawImageEntry,
) -> EngineResult<()> {
    if let Some((fv_id, _, info)) = cm.get_owned_fv_image(draw.image_id, canvas_id) {
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
        return Ok(());
    }

    let res = cm.get_shared_fv_image(draw.image_id).ok_or_else(|| {
        EngineError::from_detail(
            ErrorCode::NotFound,
            format!("shared image not found: {}", draw.image_id),
        )
    })?;
    let ctx = cm.get_2d_context_mut(canvas_id)?;
    let new_id = ctx
        .canvas
        .create_image_from_native_texture(res.1, res.2)
        .map_err(|e| {
            EngineError::from_detail(ErrorCode::Render2DResourceError, format!("{:?}", e))
        })?;
    let (iw, ih) = (res.2.width() as f32, res.2.height() as f32);
    ctx.draw_image_rect(
        new_id,
        (iw, ih),
        draw.sx,
        draw.sy,
        draw.sw,
        draw.sh,
        draw.dx,
        draw.dy,
        draw.dw,
        draw.dh,
    );
    cm.fv_images_mut()
        .entry(draw.image_id)
        .or_insert_with(HashMap::new)
        .insert(canvas_id, (new_id, res.1, res.2));
    Ok(())
}

fn draw_batch_op(
    renderer: &mut Renderer2d,
    cm: &mut CanvasManager,
    canvas_id: CanvasId,
    image_id: u32,
    draws: &[crate::renderer2d::display_list::ImageDrawRect],
) -> EngineResult<bool> {
    if draws.is_empty() {
        return Ok(false);
    }

    if draws.len() >= 2 {
        let ctx = cm.get_2d_context_mut(canvas_id)?;
        let alpha = ctx.state.global_alpha;

        if can_use_direct_image_batch(&ctx.state) {
            let native_tex = cm
                .get_owned_fv_image(image_id, canvas_id)
                .map(|(_, tex, info)| (tex, info))
                .or_else(|| {
                    cm.get_shared_fv_image(image_id)
                        .map(|(_, tex, info)| (tex, info))
                });

            if let Some((texture, info)) = native_tex {
                let img_w = info.width() as f32;
                let img_h = info.height() as f32;
                let sprites: Vec<_> = draws
                    .iter()
                    .map(|d| {
                        (d.sx, d.sy, d.sw, d.sh, d.dx, d.dy, d.dw, d.dh, img_w, img_h)
                    })
                    .collect();
                let (vp_w, vp_h) = cm.get_canvas_size(canvas_id).unwrap_or((800, 600));
                cm.get_2d_context_mut(canvas_id)?.canvas.flush();
                renderer.sprite_batcher.draw_batch(
                    cm.gl(),
                    texture,
                    &sprites,
                    vp_w as f32,
                    vp_h as f32,
                    alpha,
                );
                return Ok(true);
            }
        }
    }

    for d in draws.iter() {
        draw_single_image(
            cm,
            canvas_id,
            DrawImageEntry {
                image_id,
                sx: d.sx,
                sy: d.sy,
                sw: d.sw,
                sh: d.sh,
                dx: d.dx,
                dy: d.dy,
                dw: d.dw,
                dh: d.dh,
            },
        )?;
    }

    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::damage_effect::DamageEffect;

    #[test]
    fn sharp_shadow_disables_direct_image_batch_fast_path() {
        let mut state = Canvas2DState::default();
        state.shadow_blur = 0.0;
        state.shadow_color = femtovg::Color::rgba(0, 0, 0, 255);

        assert!(!can_use_direct_image_batch(&state));
    }

    // ── Canvas2D damage classification tests ──

    #[test]
    fn fill_rect_identity_no_shadow_produces_partial_damage() {
        let state = Canvas2DState::default(); // identity transform, no shadow
        let cmd = Canvas2DCmd::FillRect { x: 10.0, y: 20.0, w: 100.0, h: 50.0 };
        let damage = classify_draw_damage(&cmd, &state);
        assert!(matches!(damage, DamageEffect::OnscreenRect { x: 10, y: 20, width: 100, height: 50 }));
    }

    #[test]
    fn fill_rect_with_transform_produces_full_surface() {
        let mut state = Canvas2DState::default();
        state.transform = femtovg::Transform2D::new(2.0, 0.0, 0.0, 2.0, 0.0, 0.0); // scale 2x
        let cmd = Canvas2DCmd::FillRect { x: 10.0, y: 20.0, w: 100.0, h: 50.0 };
        let damage = classify_draw_damage(&cmd, &state);
        assert!(matches!(damage, DamageEffect::FullSurface));
    }

    #[test]
    fn fill_rect_with_active_shadow_produces_full_surface() {
        let mut state = Canvas2DState::default();
        state.shadow_color = femtovg::Color::rgba(0, 0, 0, 128);
        state.shadow_blur = 5.0;
        let cmd = Canvas2DCmd::FillRect { x: 10.0, y: 20.0, w: 100.0, h: 50.0 };
        let damage = classify_draw_damage(&cmd, &state);
        assert!(matches!(damage, DamageEffect::FullSurface));
    }

    #[test]
    fn stroke_rect_identity_expands_by_line_width() {
        let mut state = Canvas2DState::default();
        state.line_width = 4.0;
        let cmd = Canvas2DCmd::StrokeRect { x: 10.0, y: 20.0, w: 100.0, h: 50.0 };
        let damage = classify_draw_damage(&cmd, &state);
        // Expanded by lineWidth/2 = 2.0 on each side
        assert!(matches!(damage, DamageEffect::OnscreenRect { x: 8, y: 18, width: 104, height: 54 }));
    }

    #[test]
    fn draw_image_identity_no_shadow_produces_partial_damage() {
        let state = Canvas2DState::default();
        let cmd = Canvas2DCmd::DrawImage {
            image_id: 1, sx: 0.0, sy: 0.0, sw: 32.0, sh: 32.0,
            dx: 50.0, dy: 60.0, dw: 64.0, dh: 64.0,
        };
        let damage = classify_draw_damage(&cmd, &state);
        assert!(matches!(damage, DamageEffect::OnscreenRect { x: 50, y: 60, width: 64, height: 64 }));
    }

    #[test]
    fn fill_text_always_produces_full_surface() {
        let state = Canvas2DState::default();
        let cmd = Canvas2DCmd::FillText { text: "hello".into(), x: 10.0, y: 20.0, max_width: f32::INFINITY };
        let damage = classify_draw_damage(&cmd, &state);
        assert!(matches!(damage, DamageEffect::FullSurface));
    }

    #[test]
    fn path_fill_always_produces_full_surface() {
        let state = Canvas2DState::default();
        let cmd = Canvas2DCmd::Fill;
        let damage = classify_draw_damage(&cmd, &state);
        assert!(matches!(damage, DamageEffect::FullSurface));
    }

    #[test]
    fn clear_rect_identity_produces_partial_damage() {
        let state = Canvas2DState::default();
        let cmd = Canvas2DCmd::ClearRect { x: 0.0, y: 0.0, w: 320.0, h: 240.0 };
        let damage = classify_draw_damage(&cmd, &state);
        assert!(matches!(damage, DamageEffect::OnscreenRect { x: 0, y: 0, width: 320, height: 240 }));
    }

    #[test]
    fn shadow_with_offset_only_produces_full_surface() {
        let mut state = Canvas2DState::default();
        state.shadow_color = femtovg::Color::rgba(0, 0, 0, 128);
        state.shadow_offset_x = 5.0;
        // shadow_blur is 0 but offset is nonzero — still active shadow
        let cmd = Canvas2DCmd::FillRect { x: 10.0, y: 20.0, w: 100.0, h: 50.0 };
        let damage = classify_draw_damage(&cmd, &state);
        assert!(matches!(damage, DamageEffect::FullSurface));
    }

    #[test]
    fn state_command_is_not_a_draw() {
        let state = Canvas2DState::default();
        let cmd = Canvas2DCmd::Save;
        let damage = classify_draw_damage(&cmd, &state);
        assert!(matches!(damage, DamageEffect::NoDamage));
    }
}

pub(crate) struct Renderer2d {
    layer_cache: LayerCache,
    sprite_batcher: SpriteBatcher,
}

fn apply_pattern_style(
    cm: &mut CanvasManager,
    canvas_id: shared::protocol::render_cmd::CanvasId,
    image_id: u32,
    repeat_x: bool,
    repeat_y: bool,
    stroke: bool,
) -> EngineResult<()> {
    let mut flags = femtovg::ImageFlags::empty();
    if repeat_x {
        flags |= femtovg::ImageFlags::REPEAT_X;
    }
    if repeat_y {
        flags |= femtovg::ImageFlags::REPEAT_Y;
    }

    if let Some((fv_id, native_tex, info)) = cm.get_owned_fv_image(image_id, canvas_id) {
        let (w, h) = (info.width() as f32, info.height() as f32);

        // Skip re-registration when repeat flags already match
        if info.flags() == flags {
            let ctx = cm.get_2d_context_mut(canvas_id)?;
            if stroke {
                ctx.set_stroke_style_pattern(fv_id, repeat_x, repeat_y, w, h);
            } else {
                ctx.set_fill_style_pattern(fv_id, repeat_x, repeat_y, w, h);
            }
            return Ok(());
        }

        let new_info = femtovg::ImageInfo::new(flags, info.width(), info.height(), info.format());
        let ctx = cm.get_2d_context_mut(canvas_id)?;
        ctx.canvas.delete_image(fv_id);
        match ctx
            .canvas
            .create_image_from_native_texture(native_tex, new_info)
        {
            Ok(new_fv_id) => {
                if stroke {
                    ctx.set_stroke_style_pattern(new_fv_id, repeat_x, repeat_y, w, h);
                } else {
                    ctx.set_fill_style_pattern(new_fv_id, repeat_x, repeat_y, w, h);
                }
                cm.fv_images_mut()
                    .entry(image_id)
                    .or_insert_with(HashMap::new)
                    .insert(canvas_id, (new_fv_id, native_tex, new_info));
            }
            Err(_) => {
                // Re-register without repeat flags so the image entry is not lost
                if let Ok(fallback_id) = ctx
                    .canvas
                    .create_image_from_native_texture(native_tex, info)
                {
                    if stroke {
                        ctx.set_stroke_style_pattern(fallback_id, false, false, w, h);
                    } else {
                        ctx.set_fill_style_pattern(fallback_id, false, false, w, h);
                    }
                    cm.fv_images_mut()
                        .entry(image_id)
                        .or_insert_with(HashMap::new)
                        .insert(canvas_id, (fallback_id, native_tex, info));
                }
            }
        }
        return Ok(());
    }

    if let Some(res) = cm.get_shared_fv_image(image_id) {
        let new_info =
            femtovg::ImageInfo::new(flags, res.2.width(), res.2.height(), res.2.format());
        let (w, h) = (res.2.width() as f32, res.2.height() as f32);
        let ctx = cm.get_2d_context_mut(canvas_id)?;
        match ctx.canvas.create_image_from_native_texture(res.1, new_info) {
            Ok(new_id) => {
                if stroke {
                    ctx.set_stroke_style_pattern(new_id, repeat_x, repeat_y, w, h);
                } else {
                    ctx.set_fill_style_pattern(new_id, repeat_x, repeat_y, w, h);
                }
                cm.fv_images_mut()
                    .entry(image_id)
                    .or_insert_with(HashMap::new)
                    .insert(canvas_id, (new_id, res.1, new_info));
            }
            Err(e) => {
                return Err(EngineError::from_detail(
                    ErrorCode::Render2DResourceError,
                    format!("pattern image register failed: {:?}", e),
                ));
            }
        }
    }

    Ok(())
}

impl Renderer2d {
    pub(crate) fn new() -> Self {
        Self {
            layer_cache: LayerCache::new(),
            sprite_batcher: SpriteBatcher::new(),
        }
    }

    fn flush_for_sync_readback_if_needed(
        &mut self,
        cm: &mut CanvasManager,
        canvas_id: CanvasId,
    ) -> EngineResult<()> {
        if self.layer_cache.take_flush_for_readback(canvas_id) {
            cm.get_2d_context_mut(canvas_id)?.flush();
        }
        Ok(())
    }

    pub(crate) fn clear_dirty_layer(&mut self, canvas_id: CanvasId) {
        self.layer_cache.clear_dirty(canvas_id);
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
            let _gl_scope = cm.begin_canvas2d_gl_scope();
            let context = cm.get_2d_context_mut(canvas_id)?;
            let metrics = measure_text_with_cache(context, &text, f32::INFINITY);
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
            // Signal default-FBO readback for the onscreen canvas so bypass
            // is disabled and the DrawingBuffer preserves content across swaps.
            let onscreen_id = shared::protocol::render_cmd::CanvasId::from(1u32);
            if canvas_id == onscreen_id {
                cm.signal_default_fbo_readback();
            }
            cm.make_current_needed(canvas_id)?;
            let _gl_scope = cm.begin_canvas2d_gl_scope();
            self.flush_for_sync_readback_if_needed(cm, canvas_id)?;
            let data = cm.read_pixels(x, y, width, height);
            let _ = resp.send(Ok(data));
            return Ok(false);
        }

        let needs_gl_reset = matches!(
            &cmd,
            Canvas2DCmd::FillText { .. } | Canvas2DCmd::StrokeText { .. }
        );

        cm.make_current_needed(canvas_id)?;
        let _gl_scope = needs_gl_reset.then(|| cm.begin_canvas2d_gl_scope());
        let context = cm.get_2d_context_mut(canvas_id)?;

        let was_render: EngineResult<bool> = match cmd {
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
                let _ = measure_text_with_cache(context, &text, max_width);
                context.fill_text(&text, x, y, max_width);
                Ok(true)
            }
            Canvas2DCmd::StrokeText {
                text,
                x,
                y,
                max_width,
            } => {
                let _ = measure_text_with_cache(context, &text, max_width);
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
            Canvas2DCmd::SetShadowBlur { blur } => {
                context.set_shadow_blur(blur);
                Ok(false)
            }
            Canvas2DCmd::SetShadowColor { color } => {
                context.set_shadow_color(color);
                Ok(false)
            }
            Canvas2DCmd::SetShadowOffsetX { offset } => {
                context.set_shadow_offset_x(offset);
                Ok(false)
            }
            Canvas2DCmd::SetShadowOffsetY { offset } => {
                context.set_shadow_offset_y(offset);
                Ok(false)
            }
            Canvas2DCmd::SetFillStyleGradient {
                gradient_type,
                x0,
                y0,
                r0,
                x1,
                y1,
                r1,
                stops,
            } => {
                context.set_fill_style_gradient(gradient_type, x0, y0, r0, x1, y1, r1, stops);
                Ok(false)
            }
            Canvas2DCmd::SetStrokeStyleGradient {
                gradient_type,
                x0,
                y0,
                r0,
                x1,
                y1,
                r1,
                stops,
            } => {
                context.set_stroke_style_gradient(gradient_type, x0, y0, r0, x1, y1, r1, stops);
                Ok(false)
            }
            Canvas2DCmd::SetFillStylePattern {
                image_id,
                repeat_x,
                repeat_y,
            } => {
                apply_pattern_style(cm, canvas_id, image_id, repeat_x, repeat_y, false)?;
                Ok(false)
            }
            Canvas2DCmd::SetStrokeStylePattern {
                image_id,
                repeat_x,
                repeat_y,
            } => {
                apply_pattern_style(cm, canvas_id, image_id, repeat_x, repeat_y, true)?;
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
                    context.state.font_cache_key = context.last_font_cache_key.clone();
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
                        let font_cache_key = text_layout_font_key(&family, bold, italic, sz);
                        context.state.font_id = Some(id);
                        context.state.font_size = sz;
                        context.state.font_cache_key = font_cache_key.clone();
                        // Cache for next call
                        context.last_font_id = Some(id);
                        context.last_font_size = sz;
                        context.last_font_cache_key = font_cache_key;
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
                draw_single_image(
                    cm,
                    canvas_id,
                    DrawImageEntry {
                        image_id,
                        sx,
                        sy,
                        sw,
                        sh,
                        dx,
                        dy,
                        dw,
                        dh,
                    },
                )?;
                Ok(true)
            }

            // Batch image drawing — try direct-GL SpriteBatcher for same-texture
            // batches when Canvas2D state is simple (identity transform, SourceOver).
            // Falls back to per-draw femtovg path otherwise.
            Canvas2DCmd::DrawImageBatch { draws } => {
                let display_list = prepare_display_list(draws);
                if display_list.ops().is_empty() {
                    return Ok(false);
                }

                for op in display_list.ops() {
                    match op {
                        DisplayOp::DrawImage {
                            image_id,
                            sx,
                            sy,
                            sw,
                            sh,
                            dx,
                            dy,
                            dw,
                            dh,
                        } => draw_single_image(
                            cm,
                            canvas_id,
                            DrawImageEntry {
                                image_id: *image_id,
                                sx: *sx,
                                sy: *sy,
                                sw: *sw,
                                sh: *sh,
                                dx: *dx,
                                dy: *dy,
                                dw: *dw,
                                dh: *dh,
                            },
                        )?,
                        DisplayOp::DrawImageBatch { image_id, draws } => {
                            draw_batch_op(self, cm, canvas_id, *image_id, draws)?;
                        }
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
        };

        let was_render = was_render?;

        if was_render {
            self.layer_cache.mark_dirty(canvas_id);
        }

        Ok(was_render)
    }
}
