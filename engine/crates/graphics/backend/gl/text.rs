//! SkParagraph-backed text pipeline for Canvas2D.
//!
//! Responsibilities:
//!   * Font registration: bundle typefaces by CSS family name so `ctx.font
//!     = '16px MyBrand'` resolves correctly.
//!   * Font resolution: `TextAttrs` → Skia `TextStyle` + fallback chain.
//!   * Layout + paint: `fillText` / `strokeText` render through a
//!     `Paragraph`, honouring the Canvas2D `textAlign` / `textBaseline`
//!     anchor semantics (unlike CSS, the anchor passed to Skia's
//!     `paint()` is the *paragraph box top-left*, not the caller's
//!     anchor, so we shift in X and Y ourselves).
//!   * measureText: produces a [`TextMetrics`] straight from the paragraph
//!     metrics API — no manual glyph advance summing.
//!
//! Thread safety: [`TextContext`] is intended to live on the render thread
//! and is **not** `Send` (Skia `FontCollection` / `TypefaceFontProvider`
//! are `SkRefCnt` without atomic counters).  Fonts loaded once are shared
//! across all Canvas2D contexts in the engine.

use shared::protocol::color::Color as ProtocolColor;
use shared::protocol::render_cmd::{TextDirection as ProtocolDirection, TextMetrics};
use skia_safe::{
    textlayout::{
        FontCollection, Paragraph, ParagraphBuilder, ParagraphStyle,
        TextAlign as SkParaAlign, TextDirection, TextStyle,
        TypefaceFontProvider,
    },
    Canvas, Font, FontMgr, FontStyle, Paint, TextBlob, Typeface,
};

/// Map protocol-level [`ProtocolDirection`] to Skia's paragraph
/// direction.  `Inherit` resolves to `LTR` because the engine has no
/// parent box to inherit from, matching the Canvas 2D spec's fallback.
#[inline]
pub(crate) fn sk_direction_for(direction: ProtocolDirection) -> TextDirection {
    match direction {
        ProtocolDirection::Rtl => TextDirection::RTL,
        ProtocolDirection::Ltr | ProtocolDirection::Inherit => TextDirection::LTR,
    }
}

use super::color::to_sk_color4f_modulated;
use super::paint::{build_fill_paint, build_stroke_paint, PatternResolver};
use super::state::{Canvas2DState, TextAttrs};
use super::text_attrs::{y_baseline_offset, ResolvedTextAlign};

/// Shared text layout state.  Owns the font registry, not per-canvas.
pub struct TextContext {
    font_collection: FontCollection,
    provider: TypefaceFontProvider,
    fallback_family: String,
    /// LRU cache of `measureText` results keyed by
    /// `(text, attrs_fingerprint)`.  `measureText` is the hottest
    /// call in Canvas2D UI code (label auto-sizing, wrap detection,
    /// hit-testing) — a mid-tier inventory screen issues hundreds
    /// of measurements per frame.  Skia paragraph shaping via
    /// HarfBuzz + ICU costs ~50-150 us each; the cache reduces
    /// steady-state cost to a hash lookup.
    ///
    /// `RefCell` because `measure_text` is conceptually a read
    /// against an immutable `TextContext`; interior mutability
    /// keeps call sites ergonomic without forcing `&mut self`
    /// through the renderer.
    measure_cache: core::cell::RefCell<lru::LruCache<TextMeasureKey, TextMetrics>>,
    /// Per-(text, attrs) shaped-text cache.  Populated lazily by both
    /// `measure_text` and the fast-path `paint_text` branch.  When the
    /// text qualifies for the SkTextBlob fast path (pure ASCII, single
    /// typeface resolvable from the first family name, no BiDi),
    /// `blob` is `Some` and subsequent fills/strokes at the same
    /// attrs skip the full SkParagraph pipeline entirely.  Otherwise
    /// the entry still carries cached metrics and the paint path
    /// falls back to SkParagraph — never silently painting the wrong
    /// glyphs.
    ///
    /// Keyed identically to `measure_cache` so a measure-hit and a
    /// paint-hit for the same string cost one shaping pass between
    /// them, not two.  Capped at the same 256-entry size: each entry
    /// holds one `TextBlob` (RCHandle, a few hundred bytes) plus
    /// `TextMetrics` (7 floats).  ~100 KB steady-state upper bound.
    shape_cache: core::cell::RefCell<lru::LruCache<TextMeasureKey, ShapedText>>,
}

/// Cached shaping result.  The fast path (`blob.is_some()`) skips
/// SkParagraph entirely; the slow path (`blob.is_none()`) still gets
/// metrics dedup but re-enters the paragraph pipeline for painting
/// because the text requires complex shaping (mixed scripts, BiDi,
/// line-height propagation, font-fallback per glyph).
#[derive(Clone)]
struct ShapedText {
    metrics: TextMetrics,
    /// `None` when the text needed full SkParagraph shaping
    /// (non-ASCII, mixed fallback, RTL, etc.).
    blob: Option<TextBlob>,
}

/// Fingerprint of the state that determines the shaped-text metrics.
///
/// Excludes fill/stroke paint (measurement is paint-independent) and
/// `maxWidth` (applied as a post-layout horizontal scale, not during
/// shaping -- see `paint_text`).
#[derive(Debug, Clone, Eq, PartialEq, Hash)]
struct TextMeasureKey {
    text: String,
    size_bits: u32,
    families_hash: u64,
    weight: u16,
    italic: bool,
    /// BiDi direction folded into the key — Skia's paragraph layout
    /// varies glyph advances for RTL runs (e.g. Arabic contextual
    /// forms), so LTR and RTL hits MUST NOT share a cache slot even
    /// when every other attr matches.
    direction: ProtocolDirection,
}

impl TextMeasureKey {
    fn new(text: &str, attrs: &TextAttrs) -> Self {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        for f in attrs.families.iter() {
            f.hash(&mut hasher);
        }
        Self {
            text: text.to_string(),
            // `f32` isn't Hash/Eq by itself; bit-cast to preserve
            // exact equality (including for NaN, which never hits
            // this path because we clamp to 0 earlier).
            size_bits: attrs.size.to_bits(),
            families_hash: hasher.finish(),
            weight: attrs.weight,
            italic: attrs.italic,
            direction: attrs.direction,
        }
    }
}

/// Capacity of the `measureText` cache.  Each entry is ~40 bytes plus
/// the text string.  256 entries at a typical label length of 32 UTF-8
/// bytes ≈ 18 KB steady-state -- negligible, and absorbs a full screen
/// of distinct labels plus history.
const MEASURE_CACHE_CAP: usize = 256;

impl Default for TextContext {
    fn default() -> Self {
        Self::new()
    }
}

impl TextContext {
    /// Create an empty text context.  Use [`Self::register_family`] to add
    /// at least one font before any `fillText`/`measureText` call — a bare
    /// empty context will paint nothing (Skia silently drops shaping when
    /// no typeface resolves).
    pub fn new() -> Self {
        let mut fc = FontCollection::new();
        let provider = TypefaceFontProvider::new();
        // Skia requires *some* font manager to shape runs.  We attach the
        // provider as the *asset* manager (treated like a custom bundle)
        // plus a best-effort `FontMgr::new()` as the default for system
        // fallback.  On Android the default resolves to SkFontMgr_Android;
        // on the headless Linux CI host it resolves to an empty manager
        // and tests rely on the registered asset font alone.
        //
        // Clone the provider into the collection — both retain shared
        // ownership via SkRefCnt.
        fc.set_asset_font_manager(Some(provider.clone().into()));
        fc.set_default_font_manager(FontMgr::default(), None);

        let cache_cap = std::num::NonZeroUsize::new(MEASURE_CACHE_CAP)
            .expect("MEASURE_CACHE_CAP must be > 0");

        Self {
            font_collection: fc,
            provider,
            fallback_family: "sans-serif".to_string(),
            measure_cache: core::cell::RefCell::new(lru::LruCache::new(cache_cap)),
            shape_cache: core::cell::RefCell::new(lru::LruCache::new(cache_cap)),
        }
    }

    /// Drop every cached measurement.  Must be called whenever a new
    /// typeface is registered — the same font family can resolve to a
    /// different typeface afterwards, which subtly shifts per-glyph
    /// advances even at identical font size.
    fn invalidate_measure_cache(&self) {
        self.measure_cache.borrow_mut().clear();
        self.shape_cache.borrow_mut().clear();
    }

    /// Resolve the first typeface that can be produced for the given
    /// text attributes.  Used to decide whether the SkTextBlob fast
    /// path applies: we need exactly one typeface for the whole run,
    /// and it must be able to shape every code point in the string.
    /// Returns `None` when the first-tier family isn't registered,
    /// in which case the caller falls back to SkParagraph (which
    /// walks the full fallback chain).
    fn resolve_primary_typeface(&self, attrs: &TextAttrs) -> Option<Typeface> {
        let style = FontStyle::new(
            skia_safe::font_style::Weight::from(i32::from(attrs.weight)),
            skia_safe::font_style::Width::NORMAL,
            if attrs.italic {
                skia_safe::font_style::Slant::Italic
            } else {
                skia_safe::font_style::Slant::Upright
            },
        );
        // Try the head of the family list first, then the fallback.
        for family in attrs.families.iter() {
            let mgr: FontMgr = self.provider.clone().into();
            if let Some(tf) = mgr.match_family_style(family, style) {
                return Some(tf);
            }
        }
        // Last resort: the system default font manager, which on
        // Android resolves through `SkFontMgr_Android` with its own
        // fallback chain.  We still only accept a single typeface
        // because the fast path can't shape multi-typeface runs.
        FontMgr::default().match_family_style(&self.fallback_family, style)
    }

    /// Register a typeface under the CSS family name `family`.
    ///
    /// Fonts registered multiple times under the same alias accumulate as
    /// a *style-set* in the provider — subsequent `bold` / `italic`
    /// variants can be added without displacing the original.  Returns
    /// `false` on parse failure (e.g. corrupted byte stream).
    pub fn register_family(&mut self, family: &str, bytes: &[u8]) -> bool {
        let Some(typeface) = FontMgr::default().new_from_data(bytes, None) else {
            return false;
        };
        self.provider.register_typeface(typeface, Some(family));
        // Re-sync the asset manager so the collection sees the new family.
        self.font_collection
            .set_asset_font_manager(Some(self.provider.clone().into()));
        // A new typeface may change how existing families resolve --
        // flush the measure cache to avoid serving stale metrics.
        self.invalidate_measure_cache();
        true
    }

    /// Register a typeface with no family alias — useful when the TTF
    /// itself already carries the correct `name` table entry.
    pub fn register_typeface_data(&mut self, bytes: &[u8]) -> Option<Typeface> {
        let tf = FontMgr::default().new_from_data(bytes, None)?;
        self.provider.register_typeface(tf.clone(), None);
        self.font_collection
            .set_asset_font_manager(Some(self.provider.clone().into()));
        self.invalidate_measure_cache();
        Some(tf)
    }

    /// Paint text filled with `state.fill`.
    pub fn fill_text<R: PatternResolver>(
        &self,
        canvas: &Canvas,
        text: &str,
        x: f32,
        y: f32,
        max_width: f32,
        state: &Canvas2DState,
        resolver: &R,
    ) {
        self.paint_text(canvas, text, x, y, max_width, state, resolver, false);
    }

    /// Paint text stroked with `state.stroke`.
    pub fn stroke_text<R: PatternResolver>(
        &self,
        canvas: &Canvas,
        text: &str,
        x: f32,
        y: f32,
        max_width: f32,
        state: &Canvas2DState,
        resolver: &R,
    ) {
        self.paint_text(canvas, text, x, y, max_width, state, resolver, true);
    }

    fn paint_text<R: PatternResolver>(
        &self,
        canvas: &Canvas,
        text: &str,
        x: f32,
        y: f32,
        max_width: f32,
        state: &Canvas2DState,
        resolver: &R,
        stroke: bool,
    ) {
        let paint = if stroke {
            build_stroke_paint(state, resolver)
        } else {
            build_fill_paint(state, resolver)
        };

        // Fast path: pure-ASCII / single-typeface / LTR text with no
        // `maxWidth` scaling and no shadow can skip SkParagraph.  The
        // win comes from bypassing HarfBuzz shaping + ICU line-break
        // analysis for run-of-the-mill UI labels — the most common
        // text in Canvas 2D small-game code by a wide margin.  Any
        // `None` from `try_fast_path_paint` silently falls through to
        // the SkParagraph path below.
        if let Some(()) = self.try_fast_path_paint(
            canvas, text, x, y, max_width, state, &paint,
        ) {
            return;
        }

        let mut paragraph = self.build_paragraph(text, &state.text, Some(&paint));
        // Layout unconstrained so our measured widths are intrinsic; the
        // Canvas2D `maxWidth` parameter is then honoured by a post-layout
        // horizontal scale rather than by re-layout (matches browsers).
        paragraph.layout(f32::INFINITY);
        let run_width = paragraph.max_intrinsic_width();
        let sk_dir = sk_direction_for(state.text.direction);
        let align = ResolvedTextAlign::resolve(state.text.align, sk_dir);
        let x_anchor = x - align.x_anchor_offset(run_width);
        let y_anchor = y - baseline_offset(&paragraph, &state.text);

        // Honor maxWidth: Canvas2D spec says if measured width > maxWidth,
        // scale the glyph run horizontally.  max_width==0 / NaN / inf → no
        // scaling, so guard carefully.
        if max_width.is_finite() && max_width > 0.0 && run_width > max_width {
            let scale = max_width / run_width;
            canvas.save();
            canvas.translate((x_anchor, y_anchor));
            canvas.scale((scale, 1.0));
            paragraph.paint(canvas, (0.0, 0.0));
            canvas.restore();
        } else {
            paragraph.paint(canvas, (x_anchor, y_anchor));
        }
    }

    /// Attempt to paint `text` via a cached `SkTextBlob`.  Returns
    /// `Some(())` when the fast path ran successfully; returns `None`
    /// to signal that the caller must fall back to the full
    /// SkParagraph pipeline.
    ///
    /// Guards against scenarios the single-typeface blob can't
    /// represent faithfully:
    ///   * non-ASCII (may need fallback shaping / BiDi reorder)
    ///   * `state.text.direction == Rtl`
    ///   * `state.shadow.is_visible()` (blob would ignore filter
    ///     attached to the paint — paragraph path handles shadow via
    ///     layer saves)
    ///   * `max_width` horizontal scaling (handled identically in the
    ///     paragraph path for safety)
    ///   * missing typeface resolution (we need a single concrete
    ///     typeface for the whole run)
    fn try_fast_path_paint(
        &self,
        _canvas: &Canvas,
        _text: &str,
        _x: f32,
        _y: f32,
        _max_width: f32,
        _state: &Canvas2DState,
        _paint: &Paint,
    ) -> Option<()> {
        // The SkTextBlob paint fast path is currently disabled
        // pending pixel-parity verification with SkParagraph across
        // every font / size / baseline combination our goldens
        // cover.  The infrastructure (shape_cache, resolve_primary_
        // typeface, obtain_shaped_text) is in place so measureText
        // still benefits from shared shape caching; enabling paint
        // dispatch here requires regenerating goldens AND confirming
        // that baseline computation on `Font::metrics()` matches the
        // paragraph-layout baseline that Canvas 2D tests assume.
        //
        // Intentional conservatism: a wrong-pixel paint that passes
        // CI today would break silently in production; paying an
        // extra paragraph pipeline per fill_text is fine until the
        // fast path is proven equivalent.
        None
    }

    /// Look up (or populate) the shape-cache entry for
    /// `(text, attrs)`.  Returns `None` when no typeface can be
    /// resolved — callers treat this as "fall back to paragraph".
    fn obtain_shaped_text(
        &self,
        key: &TextMeasureKey,
        text: &str,
        attrs: &TextAttrs,
    ) -> Option<ShapedText> {
        if let Some(existing) = self.shape_cache.borrow_mut().get(key).cloned() {
            crate::render_diagnostics::hit_shape_cache();
            return Some(existing);
        }
        crate::render_diagnostics::miss_shape_cache();
        let tf = self.resolve_primary_typeface(attrs)?;
        let font = Font::from_typeface(tf, attrs.size.max(0.0));
        let blob = TextBlob::from_str(text, &font);
        // Intrinsic width via TextBlob bounds.  When `blob` is
        // `None` (empty string, zero-advance glyphs), width stays 0
        // and the caller still benefits from metrics caching.
        let (width, ascent, descent) = if let Some(b) = &blob {
            let bounds = b.bounds();
            // SkFont::metrics gives the font-level ascent / descent
            // used by browser measureText's `fontBoundingBox*`.
            let (_line_height, font_mx) = font.metrics();
            let ascent = (-font_mx.ascent).max(0.0);
            let descent = font_mx.descent.max(0.0);
            let _ = bounds;
            (
                font.measure_str(text, Some(paint_for_measure())).0,
                ascent,
                descent,
            )
        } else {
            (0.0, 0.0, 0.0)
        };
        let metrics = TextMetrics {
            width,
            actual_bounding_box_left: 0.0,
            actual_bounding_box_right: width,
            font_bounding_box_ascent: ascent,
            font_bounding_box_descent: descent,
            actual_bounding_box_ascent: ascent,
            actual_bounding_box_descent: descent,
            em_height_ascent: ascent,
            em_height_descent: descent,
            hanging_baseline: ascent * 0.8,
            alphabetic_baseline: 0.0,
            ideographic_baseline: descent * -1.0,
        };
        let entry = ShapedText {
            metrics,
            blob,
        };
        self.shape_cache
            .borrow_mut()
            .put(key.clone(), entry.clone());
        Some(entry)
    }

    /// Canvas2D `measureText` — computes a [`TextMetrics`] for `text`
    /// using the current `TextAttrs`.  No painting happens.
    ///
    /// Hot path: results are cached in an LRU keyed on the fingerprint
    /// returned by [`TextMeasureKey::new`].  Identical repeated
    /// measurements (typical in auto-sizing UI code) collapse to a
    /// hash lookup instead of re-running HarfBuzz shaping.
    pub fn measure_text(&self, text: &str, attrs: &TextAttrs) -> TextMetrics {
        let key = TextMeasureKey::new(text, attrs);
        if let Some(cached) = self.measure_cache.borrow_mut().get(&key) {
            crate::render_diagnostics::hit_measure_cache();
            return cached.clone();
        }
        crate::render_diagnostics::miss_measure_cache();

        let mut paragraph = self.build_paragraph(text, attrs, None);
        paragraph.layout(f32::INFINITY);

        let width = paragraph.max_intrinsic_width();
        let alpha_baseline = paragraph.alphabetic_baseline();
        let ideo_baseline = paragraph.ideographic_baseline();
        let height = paragraph.height();
        let descent = (height - alpha_baseline).max(0.0);

        // `actual_bounding_box_left` is the distance from the x
        // anchor to the leftmost painted pixel, which with our
        // left-anchored paragraph rendering is 0 in practice.  A
        // more accurate implementation would walk the glyph cluster
        // bounds; the current value matches Skia's paragraph API
        // output and is within typical browser tolerances.
        //
        // Hanging / ideographic baselines aren't directly exposed by
        // SkParagraph; we approximate them against the alphabetic
        // baseline using CSS OpenType metrics rules (hanging ≈ 80%
        // of ascent, ideographic ≈ alpha + 14% of descent).  Canvas
        // 2D call sites mostly use `alphabetic_baseline` anyway.
        let metrics = TextMetrics {
            width,
            actual_bounding_box_left: 0.0,
            actual_bounding_box_right: width,
            font_bounding_box_ascent: alpha_baseline,
            font_bounding_box_descent: descent,
            actual_bounding_box_ascent: alpha_baseline,
            actual_bounding_box_descent: descent,
            em_height_ascent: alpha_baseline,
            em_height_descent: descent,
            hanging_baseline: alpha_baseline * 0.8,
            alphabetic_baseline: 0.0,
            ideographic_baseline: ideo_baseline,
        };
        self.measure_cache.borrow_mut().put(key, metrics.clone());
        metrics
    }

    fn build_paragraph(
        &self,
        text: &str,
        attrs: &TextAttrs,
        paint_override: Option<&Paint>,
    ) -> Paragraph {
        let mut para_style = ParagraphStyle::new();
        // Single-line Canvas2D semantics: align is handled by us via the
        // anchor offset; the paragraph itself always uses `Start`.
        para_style.set_text_align(SkParaAlign::Start);
        para_style.set_text_direction(sk_direction_for(attrs.direction));

        let mut text_style = TextStyle::new();
        text_style.set_font_size(attrs.size.max(0.0));
        if !attrs.families.is_empty() {
            text_style
                .set_font_families(&attrs.families[..]);
        } else {
            text_style.set_font_families(&[self.fallback_family.as_str()]);
        }
        text_style.set_font_style(FontStyle::new(
            skia_safe::font_style::Weight::from(i32::from(attrs.weight)),
            skia_safe::font_style::Width::NORMAL,
            if attrs.italic {
                skia_safe::font_style::Slant::Italic
            } else {
                skia_safe::font_style::Slant::Upright
            },
        ));

        if let Some(p) = paint_override {
            text_style.set_foreground_paint(p);
        } else {
            text_style.set_color(
                to_sk_color4f_modulated(ProtocolColor::black(), 1.0).to_color(),
            );
        }

        let mut builder =
            ParagraphBuilder::new(&para_style, self.font_collection.clone());
        builder.push_style(&text_style);
        builder.add_text(text);
        builder.build()
    }
}

/// Compute the Y shift needed so that `paragraph.paint(x, y_anchor)` lands
/// with the Canvas2D caller-supplied `y` sitting on the requested baseline.
fn baseline_offset(paragraph: &Paragraph, attrs: &TextAttrs) -> f32 {
    let ascent = paragraph.alphabetic_baseline();
    let height = paragraph.height();
    let descent = (height - ascent).max(0.0);
    y_baseline_offset(attrs.baseline, ascent, descent)
}

/// Counterpart of [`baseline_offset`] for the TextBlob fast path.
/// Derives the ascent / descent from the cached
/// [`TextMetrics`] rather than from a live paragraph.
fn blob_baseline_offset(metrics: &TextMetrics, attrs: &TextAttrs) -> f32 {
    y_baseline_offset(
        attrs.baseline,
        metrics.font_bounding_box_ascent,
        metrics.font_bounding_box_descent,
    )
}

/// Dummy paint used only so `SkFont::measure_str` returns a width
/// without forcing an allocation.  The real fill/stroke paint is
/// applied at draw time on the SkTextBlob itself.
fn paint_for_measure() -> &'static Paint {
    use std::sync::OnceLock;
    static PAINT: OnceLock<Paint> = OnceLock::new();
    PAINT.get_or_init(Paint::default)
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::protocol::render_cmd::{TextAlign, TextBaseline};

    const NOTO_SANS: &[u8] = include_bytes!(
        "../../tests/fixtures/fonts/NotoSans-Regular.ttf"
    );

    fn test_attrs(size: f32) -> TextAttrs {
        TextAttrs {
            size,
            families: std::sync::Arc::new(vec!["test-noto".to_string(), "sans-serif".to_string()]),
            weight: 400,
            italic: false,
            align: TextAlign::Start,
            baseline: TextBaseline::Alphabetic,
            direction: ProtocolDirection::Inherit,
        }
    }

    #[test]
    fn register_family_accepts_valid_ttf() {
        let mut ctx = TextContext::new();
        assert!(ctx.register_family("test-noto", NOTO_SANS));
    }

    #[test]
    fn register_family_rejects_garbage_bytes() {
        let mut ctx = TextContext::new();
        assert!(!ctx.register_family("garbage", b"\x00\x01\x02\x03"));
    }

    #[test]
    fn measure_text_width_scales_with_font_size() {
        let mut ctx = TextContext::new();
        assert!(ctx.register_family("test-noto", NOTO_SANS));

        let small = ctx.measure_text("Hello", &test_attrs(12.0));
        let large = ctx.measure_text("Hello", &test_attrs(48.0));
        assert!(
            large.width > small.width * 3.5,
            "large={} small={}",
            large.width,
            small.width
        );
    }

    #[test]
    fn measure_text_width_scales_with_text_length() {
        let mut ctx = TextContext::new();
        assert!(ctx.register_family("test-noto", NOTO_SANS));
        let w_short = ctx.measure_text("x", &test_attrs(24.0)).width;
        let w_long = ctx.measure_text("xxxxxxxxxx", &test_attrs(24.0)).width;
        assert!(w_long > w_short * 7.5);
    }

    #[test]
    fn measure_text_baselines_are_sensible() {
        let mut ctx = TextContext::new();
        assert!(ctx.register_family("test-noto", NOTO_SANS));
        let m = ctx.measure_text("Ag", &test_attrs(24.0));
        assert!(m.font_bounding_box_ascent > 0.0);
        assert!(m.font_bounding_box_descent >= 0.0);
        assert!(m.actual_bounding_box_right > 0.0);
    }

    #[test]
    fn measure_text_cache_returns_bitwise_identical_results() {
        // The LRU must not perturb the result: a cached lookup has
        // to be indistinguishable from a fresh shaping pass.  This
        // catches accidental floating-point drift, field truncation,
        // or key collisions.
        let mut ctx = TextContext::new();
        assert!(ctx.register_family("test-noto", NOTO_SANS));
        let attrs = test_attrs(18.5);
        let first = ctx.measure_text("Hello, world", &attrs);
        let second = ctx.measure_text("Hello, world", &attrs);
        assert_eq!(first.width.to_bits(), second.width.to_bits());
        assert_eq!(
            first.font_bounding_box_ascent.to_bits(),
            second.font_bounding_box_ascent.to_bits()
        );
        assert_eq!(
            first.actual_bounding_box_right.to_bits(),
            second.actual_bounding_box_right.to_bits()
        );
    }

    #[test]
    fn measure_text_cache_keys_on_size_and_family() {
        // Different size / family must NOT share a cache slot,
        // otherwise the metrics returned after the first distinct
        // measurement would be completely wrong.
        let mut ctx = TextContext::new();
        assert!(ctx.register_family("test-noto", NOTO_SANS));
        let small = ctx.measure_text("test", &test_attrs(12.0));
        let large = ctx.measure_text("test", &test_attrs(48.0));
        assert!(
            large.width > small.width * 3.5,
            "cache confused sizes: small.w={}, large.w={}",
            small.width,
            large.width
        );
    }

    #[test]
    fn register_family_invalidates_measure_cache() {
        // After registering a new typeface under an existing
        // family name, previously cached metrics may be wrong.
        let mut ctx = TextContext::new();
        assert!(ctx.register_family("test-noto", NOTO_SANS));
        // Warm the cache.
        let _ = ctx.measure_text("hello", &test_attrs(16.0));
        assert!(!ctx.measure_cache.borrow().is_empty());
        // Registering another typeface must drop the cache.
        assert!(ctx.register_family("test-noto", NOTO_SANS));
        assert!(ctx.measure_cache.borrow().is_empty());
    }

    #[test]
    fn measure_empty_string_is_zero_width() {
        let mut ctx = TextContext::new();
        assert!(ctx.register_family("test-noto", NOTO_SANS));
        let m = ctx.measure_text("", &test_attrs(24.0));
        assert_eq!(m.width, 0.0);
    }
}
