//! Build Skia [`Paint`] objects from a [`Canvas2DState`] snapshot.
//!
//! Canvas2D fillStyle / strokeStyle / globalAlpha / shadow / blend-mode all
//! translate into SkPaint settings; gradients and patterns additionally
//! require an `SkShader`.  We intentionally rebuild the paint per draw
//! rather than caching it on the state — SkPaint is a value type (cheap
//! copies, no allocation for the common flat-colour case) and rebuilding
//! avoids the "state ⇆ cache coherency" complexity that a stale cache
//! would introduce.

use super::color::{to_sk_color4f, to_sk_color4f_modulated};
use super::state::{Canvas2DState, StyleKind};
use skia_safe::{
    gradient_shader, image_filters, path_effect::PathEffect, BlendMode, Color,
    Paint, Point, Shader, TileMode,
};
use shared::protocol::render_cmd::GradientStop;

/// Build a Skia `Shader` from the fill/stroke `StyleKind`, if any.
///
/// Returns `None` when the kind is a flat colour, or when the gradient has
/// fewer than two stops (spec-invalid, silently suppressed to match browser
/// behaviour), or when a pattern image is not yet loaded.
pub fn build_shader<R>(
    kind: &StyleKind,
    global_alpha: f32,
    image_resolver: &R,
) -> Option<Shader>
where
    R: PatternResolver,
{
    match kind {
        StyleKind::Color(_) => None,
        StyleKind::LinearGradient {
            x0,
            y0,
            x1,
            y1,
            stops,
        } => build_linear_gradient(*x0, *y0, *x1, *y1, stops, global_alpha),
        StyleKind::RadialGradient {
            x0,
            y0,
            r0,
            x1,
            y1,
            r1,
            stops,
        } => build_radial_gradient(*x0, *y0, *r0, *x1, *y1, *r1, stops, global_alpha),
        StyleKind::ConicGradient {
            cx,
            cy,
            start_angle,
            stops,
        } => build_conic_gradient(*cx, *cy, *start_angle, stops, global_alpha),
        StyleKind::Pattern {
            image_id,
            repeat_x,
            repeat_y,
        } => image_resolver
            .resolve_pattern(*image_id, *repeat_x, *repeat_y, global_alpha),
    }
}

/// Abstraction over image registry lookup, factored out so the handler
/// core can be unit-tested without a real GPU image store.
pub trait PatternResolver {
    fn resolve_pattern(
        &self,
        image_id: u32,
        repeat_x: bool,
        repeat_y: bool,
        global_alpha: f32,
    ) -> Option<Shader>;
}

/// Test-only resolver that always reports "image not loaded".
pub struct NullPatternResolver;

impl PatternResolver for NullPatternResolver {
    fn resolve_pattern(&self, _: u32, _: bool, _: bool, _: f32) -> Option<Shader> {
        None
    }
}

fn stops_to_colors_positions(
    stops: &[GradientStop],
    global_alpha: f32,
) -> Option<(Vec<Color>, Vec<f32>)> {
    if stops.len() < 2 {
        return None;
    }
    let colors = stops
        .iter()
        .map(|s| to_sk_color4f_modulated(s.color, global_alpha).to_color())
        .collect::<Vec<_>>();
    let positions = stops.iter().map(|s| s.offset.clamp(0.0, 1.0)).collect();
    Some((colors, positions))
}

fn build_linear_gradient(
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    stops: &[GradientStop],
    global_alpha: f32,
) -> Option<Shader> {
    let (colors, positions) = stops_to_colors_positions(stops, global_alpha)?;
    gradient_shader::linear(
        (Point::new(x0, y0), Point::new(x1, y1)),
        gradient_shader::GradientShaderColors::Colors(&colors),
        Some(&positions[..]),
        TileMode::Clamp,
        None,
        None,
    )
}

fn build_radial_gradient(
    x0: f32,
    y0: f32,
    r0: f32,
    x1: f32,
    y1: f32,
    r1: f32,
    stops: &[GradientStop],
    global_alpha: f32,
) -> Option<Shader> {
    let (colors, positions) = stops_to_colors_positions(stops, global_alpha)?;
    gradient_shader::two_point_conical(
        Point::new(x0, y0),
        r0,
        Point::new(x1, y1),
        r1,
        gradient_shader::GradientShaderColors::Colors(&colors),
        Some(&positions[..]),
        TileMode::Clamp,
        None,
        None,
    )
}

fn build_conic_gradient(
    cx: f32,
    cy: f32,
    start_angle_rad: f32,
    stops: &[GradientStop],
    global_alpha: f32,
) -> Option<Shader> {
    let (colors, positions) = stops_to_colors_positions(stops, global_alpha)?;
    let start_deg = start_angle_rad.to_degrees();
    let end_deg = start_deg + 360.0;
    gradient_shader::sweep(
        Point::new(cx, cy),
        gradient_shader::GradientShaderColors::Colors(&colors),
        Some(&positions[..]),
        TileMode::Clamp,
        Some((start_deg, end_deg)),
        None,
        None,
    )
}

/// If a visible shadow is present in `state`, install a drop-shadow
/// `ImageFilter` on `paint`.
///
/// Canvas2D's shadow is a drop-shadow (rendered *behind* the primary
/// draw).  We convert `shadowBlur` to sigma via `sigma = blur / 2` which
/// matches Chrome's interpretation of the CSS "blur length" as
/// `2 × stddev`.
pub fn apply_shadow_to_paint(paint: &mut Paint, state: &Canvas2DState) {
    if !state.shadow.is_visible() {
        return;
    }
    let sigma = state.shadow.blur * 0.5;
    let color = to_sk_color4f_modulated(state.shadow.color, state.global_alpha).to_color();
    if let Some(filter) = image_filters::drop_shadow(
        (state.shadow.offset_x, state.shadow.offset_y),
        (sigma, sigma),
        color,
        None, // color space
        None, // input filter
        None, // crop rect
    ) {
        paint.set_image_filter(filter);
    }
}

/// Build a `Paint` preset for the Canvas2D *fill* side.
pub fn build_fill_paint<R: PatternResolver>(
    state: &Canvas2DState,
    resolver: &R,
) -> Paint {
    let mut paint = Paint::default();
    paint.set_anti_alias(state.antialias);
    paint.set_style(skia_safe::paint::Style::Fill);
    paint.set_blend_mode(state.blend_mode);

    match &state.fill {
        StyleKind::Color(c) => {
            let c4f = to_sk_color4f_modulated(*c, state.global_alpha);
            paint.set_color4f(c4f, None);
        }
        _ => {
            if let Some(shader) = build_shader(&state.fill, state.global_alpha, resolver) {
                paint.set_shader(shader);
            } else {
                paint.set_color4f(
                    to_sk_color4f(shared::protocol::color::Color::transparent()),
                    None,
                );
            }
        }
    }
    apply_shadow_to_paint(&mut paint, state);
    paint
}

/// Build a `Paint` preset for the Canvas2D *stroke* side.
pub fn build_stroke_paint<R: PatternResolver>(
    state: &Canvas2DState,
    resolver: &R,
) -> Paint {
    let mut paint = Paint::default();
    paint.set_anti_alias(state.antialias);
    paint.set_style(skia_safe::paint::Style::Stroke);
    paint.set_stroke_width(state.line_width.max(0.0));
    paint.set_stroke_cap(state.line_cap);
    paint.set_stroke_join(state.line_join);
    paint.set_stroke_miter(state.miter_limit);
    paint.set_blend_mode(state.blend_mode);

    match &state.stroke {
        StyleKind::Color(c) => {
            let c4f = to_sk_color4f_modulated(*c, state.global_alpha);
            paint.set_color4f(c4f, None);
        }
        _ => {
            if let Some(shader) = build_shader(&state.stroke, state.global_alpha, resolver) {
                paint.set_shader(shader);
            } else {
                paint.set_color4f(
                    to_sk_color4f(shared::protocol::color::Color::transparent()),
                    None,
                );
            }
        }
    }

    if !state.line_dash.is_empty() {
        if let Some(effect) =
            PathEffect::dash(&state.line_dash, state.line_dash_offset)
        {
            paint.set_path_effect(effect);
        }
    }
    apply_shadow_to_paint(&mut paint, state);
    paint
}

/// `clearRect`'s Paint: writes transparent black with blend mode Clear,
/// which sets the destination to fully transparent regardless of current
/// state.
pub fn build_clear_paint() -> Paint {
    let mut paint = Paint::default();
    paint.set_anti_alias(false);
    paint.set_style(skia_safe::paint::Style::Fill);
    paint.set_color4f(skia_safe::Color4f::new(0.0, 0.0, 0.0, 0.0), None);
    paint.set_blend_mode(BlendMode::Clear);
    paint
}

#[cfg(test)]
mod tests {
    use super::super::state::{Canvas2DState, Shadow, StyleKind};
    use super::*;
    use shared::protocol::color::Color as ProtocolColor;

    #[test]
    fn build_fill_paint_uses_color_when_style_is_flat() {
        let mut s = Canvas2DState::default();
        s.fill = StyleKind::Color(ProtocolColor::rgb(10, 20, 30));
        s.global_alpha = 1.0;
        let p = build_fill_paint(&s, &NullPatternResolver);
        assert!(p.shader().is_none());
        assert_eq!(p.style(), skia_safe::paint::Style::Fill);
    }

    #[test]
    fn build_stroke_paint_carries_line_attributes() {
        let mut s = Canvas2DState::default();
        s.stroke = StyleKind::Color(ProtocolColor::black());
        s.line_width = 5.0;
        s.miter_limit = 3.5;
        s.line_cap = skia_safe::PaintCap::Round;
        s.line_join = skia_safe::PaintJoin::Bevel;
        s.line_dash = std::sync::Arc::new(vec![4.0, 2.0]);
        s.line_dash_offset = 1.0;

        let p = build_stroke_paint(&s, &NullPatternResolver);
        assert_eq!(p.style(), skia_safe::paint::Style::Stroke);
        assert_eq!(p.stroke_width(), 5.0);
        assert_eq!(p.stroke_miter(), 3.5);
        assert_eq!(p.stroke_cap(), skia_safe::PaintCap::Round);
        assert_eq!(p.stroke_join(), skia_safe::PaintJoin::Bevel);
        assert!(p.path_effect().is_some(), "dash effect must be installed");
    }

    #[test]
    fn build_stroke_paint_no_dash_when_empty() {
        let mut s = Canvas2DState::default();
        s.stroke = StyleKind::Color(ProtocolColor::black());
        s.line_dash = std::sync::Arc::new(Vec::new());
        let p = build_stroke_paint(&s, &NullPatternResolver);
        assert!(p.path_effect().is_none());
    }

    #[test]
    fn build_shader_returns_none_for_flat_color() {
        let s = StyleKind::Color(ProtocolColor::black());
        let sh = build_shader(&s, 1.0, &NullPatternResolver);
        assert!(sh.is_none());
    }

    #[test]
    fn build_shader_rejects_single_stop_gradient() {
        let s = StyleKind::LinearGradient {
            x0: 0.0,
            y0: 0.0,
            x1: 1.0,
            y1: 0.0,
            stops: std::sync::Arc::new(vec![GradientStop {
                offset: 0.5,
                color: ProtocolColor::black(),
            }]),
        };
        assert!(build_shader(&s, 1.0, &NullPatternResolver).is_none());
    }

    #[test]
    fn build_shader_accepts_two_stop_linear() {
        let s = StyleKind::LinearGradient {
            x0: 0.0,
            y0: 0.0,
            x1: 1.0,
            y1: 0.0,
            stops: std::sync::Arc::new(vec![
                GradientStop {
                    offset: 0.0,
                    color: ProtocolColor::rgb(255, 0, 0),
                },
                GradientStop {
                    offset: 1.0,
                    color: ProtocolColor::rgb(0, 0, 255),
                },
            ]),
        };
        assert!(build_shader(&s, 1.0, &NullPatternResolver).is_some());
    }

    #[test]
    fn pattern_returns_none_from_null_resolver() {
        let s = StyleKind::Pattern {
            image_id: 1,
            repeat_x: true,
            repeat_y: true,
        };
        assert!(build_shader(&s, 1.0, &NullPatternResolver).is_none());
    }

    #[test]
    fn clear_paint_uses_blend_mode_clear() {
        let p = build_clear_paint();
        assert_eq!(p.as_blend_mode(), Some(BlendMode::Clear));
    }

    #[test]
    fn fill_paint_carries_shadow_filter_when_visible() {
        let mut s = Canvas2DState::default();
        s.shadow = Shadow {
            blur: 4.0,
            color: ProtocolColor::black(),
            offset_x: 2.0,
            offset_y: 2.0,
        };
        let p = build_fill_paint(&s, &NullPatternResolver);
        assert!(p.image_filter().is_some(), "shadow ImageFilter missing");
    }

    #[test]
    fn fill_paint_has_no_shadow_filter_when_shadow_is_off() {
        let s = Canvas2DState::default();
        let p = build_fill_paint(&s, &NullPatternResolver);
        assert!(p.image_filter().is_none());
    }
}
