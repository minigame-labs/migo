//! Color conversion between the render-protocol `shared::protocol::color::Color`
//! (normalised `f32` RGBA) and the Skia native colour types.
//!
//! The protocol colour is always **un-premultiplied sRGB** (matching the CSS
//! `fillStyle = "#RRGGBB"` / `rgba(…)` parsing done on the JS side).  Skia's
//! [`Color4f`] uses the same convention, so conversion is direct — but the
//! *legacy* 32-bit [`Color`] (ARGB_8888, premultiplied) is **not** what we
//! want when feeding `SkPaint::setColor`; we always route through `Color4f`.
//!
//! The helpers here also implement Canvas2D's `globalAlpha` semantics:
//! globalAlpha multiplies the *source* alpha only.  Applied at paint-build
//! time rather than at style-set time so changing `globalAlpha` between
//! draws does not force refetching the style.

use shared::protocol::color::Color as ProtocolColor;
use skia_safe::Color4f;

/// Convert a protocol colour to Skia's linear-friendly `Color4f`.
///
/// Values are *clamped* to `[0.0, 1.0]` because upstream JS may emit out-of-
/// range alpha (e.g. `rgba(255, 0, 0, 1.5)` — browsers clamp silently).
#[inline]
pub fn to_sk_color4f(c: ProtocolColor) -> Color4f {
    Color4f {
        r: clamp01(c.r),
        g: clamp01(c.g),
        b: clamp01(c.b),
        a: clamp01(c.a),
    }
}

/// Convert a protocol colour to Skia's `Color4f`, modulating alpha by
/// `global_alpha` (as per Canvas2D spec §2.4 "drawing state").
///
/// `global_alpha` itself is clamped; out-of-range values are treated as
/// `1.0` (matching browser behaviour per WHATWG Canvas step 4.1).
#[inline]
pub fn to_sk_color4f_modulated(c: ProtocolColor, global_alpha: f32) -> Color4f {
    let ga = if (0.0..=1.0).contains(&global_alpha) {
        global_alpha
    } else {
        1.0
    };
    Color4f {
        r: clamp01(c.r),
        g: clamp01(c.g),
        b: clamp01(c.b),
        a: clamp01(c.a) * ga,
    }
}

#[inline]
fn clamp01(x: f32) -> f32 {
    x.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx_eq(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn opaque_red_roundtrip() {
        let c = ProtocolColor::rgb(255, 0, 0);
        let sk = to_sk_color4f(c);
        assert!(approx_eq(sk.r, 1.0));
        assert!(approx_eq(sk.g, 0.0));
        assert!(approx_eq(sk.b, 0.0));
        assert!(approx_eq(sk.a, 1.0));
    }

    #[test]
    fn transparent_black_roundtrip() {
        let c = ProtocolColor::transparent();
        let sk = to_sk_color4f(c);
        assert_eq!(sk.a, 0.0);
    }

    #[test]
    fn alpha_modulation_scales_only_alpha() {
        let c = ProtocolColor::rgba(1.0, 0.5, 0.25, 0.8);
        let sk = to_sk_color4f_modulated(c, 0.5);
        assert!(approx_eq(sk.r, 1.0));
        assert!(approx_eq(sk.g, 0.5));
        assert!(approx_eq(sk.b, 0.25));
        assert!(approx_eq(sk.a, 0.4)); // 0.8 * 0.5
    }

    #[test]
    fn modulation_with_full_alpha_is_identity() {
        let c = ProtocolColor::rgba(0.2, 0.4, 0.6, 0.8);
        let sk = to_sk_color4f_modulated(c, 1.0);
        assert!(approx_eq(sk.a, 0.8));
    }

    #[test]
    fn modulation_with_zero_alpha_makes_transparent() {
        let c = ProtocolColor::rgb(255, 255, 255);
        let sk = to_sk_color4f_modulated(c, 0.0);
        assert_eq!(sk.a, 0.0);
    }

    #[test]
    fn out_of_range_alpha_channel_is_clamped() {
        let c = ProtocolColor::rgba(1.0, 1.0, 1.0, 1.5);
        let sk = to_sk_color4f(c);
        assert_eq!(sk.a, 1.0);
    }

    #[test]
    fn out_of_range_global_alpha_reverts_to_one() {
        // WHATWG: "if the new value is infinite or NaN, the operation is
        // aborted"; browsers silently ignore such values rather than crashing.
        // Our wrapper treats out-of-range as 1.0 to keep rendering defined.
        let c = ProtocolColor::rgba(0.5, 0.5, 0.5, 0.5);
        let bad_ga = to_sk_color4f_modulated(c, 2.0);
        let good_ga = to_sk_color4f_modulated(c, 1.0);
        assert!(approx_eq(bad_ga.a, good_ga.a));
    }

    #[test]
    fn negative_components_clamp_to_zero() {
        let c = ProtocolColor::rgba(-0.5, 0.5, 0.5, 1.0);
        let sk = to_sk_color4f(c);
        assert_eq!(sk.r, 0.0);
    }

    #[test]
    fn css_hex_parsed_color_maps_correctly() {
        // Spot-check: CSS "#80FF0040" → r=0.502, g=1.0, b=0.0, a=0.251
        let c = ProtocolColor::hex("#80FF0040");
        let sk = to_sk_color4f(c);
        assert!(approx_eq(sk.r, 128.0 / 255.0));
        assert!(approx_eq(sk.g, 1.0));
        assert!(approx_eq(sk.b, 0.0));
        assert!(approx_eq(sk.a, 64.0 / 255.0));
    }
}
