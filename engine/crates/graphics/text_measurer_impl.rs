//! Concrete [`shared::text_measurer::TextMeasurer`] implementation
//! backed by the graphics crate's `TextContext` (F-2).
//!
//! Architecture: a single `Arc<parking_lot::Mutex<TextContext>>`
//! is owned by the render thread and cloned into the JS side via
//! `CanvasOpState::text_measurer`.  Every call enters the mutex
//! for the duration of a measurement; the shaping pipeline is
//! already O(50-150 μs) per miss and sub-μs per cache hit, so
//! even under a worst-case JS + render-thread contention burst
//! the mutex wait time is dwarfed by the work it guards.
//!
//! Font registration (`register_font`) goes through the same
//! mutex, which keeps the shared `font_epoch` counter
//! consistent between the render thread's `fillText` shaping
//! and the JS-thread `measureText` lookup.  JS side also bumps
//! `globalThis.__migoFontEpoch` immediately so its per-canvas
//! LRU cache (R-10) invalidates within the same tick —
//! `register_font` on the shared context and the JS-side bump
//! are wired from the same `loadFont` call, so ordering is
//! deterministic.

use std::sync::Arc;

use parking_lot::Mutex;
use shared::protocol::render_cmd::{TextAlign, TextBaseline, TextDirection, TextMetrics};
use shared::text_measurer::{SharedTextMeasurer, TextMeasurer};

use crate::backend::gl::state::TextAttrs;
use crate::backend::gl::text::TextContext;

/// Public handle the render thread hands to `CanvasOpState`.  A
/// process-wide `Arc` keeps the context alive for the lifetime
/// of the engine; dropping all clones tears down the
/// `TextContext` together with its Skia font managers and LRU
/// caches.
pub type SharedTextContext = Arc<Mutex<TextContext>>;

/// Wrap a `TextContext` behind the process-wide mutex and
/// hand back a type-erased `SharedTextMeasurer` pointing at it.
/// The render thread also keeps the same `Arc<Mutex<_>>`
/// directly so `fillText` / `strokeText` can borrow the
/// context mutably without going through the trait object.
pub fn into_shared_measurer(ctx: TextContext) -> (SharedTextContext, SharedTextMeasurer) {
    let shared = Arc::new(Mutex::new(ctx));
    let measurer: SharedTextMeasurer = Arc::new(TextMeasurerAdapter {
        inner: shared.clone(),
    });
    (shared, measurer)
}

/// Newtype adapter so the `impl TextMeasurer` doesn't appear
/// on `Arc<Mutex<TextContext>>` directly (which would conflict
/// with other downstream blanket impls).
struct TextMeasurerAdapter {
    inner: SharedTextContext,
}

impl TextMeasurer for TextMeasurerAdapter {
    fn measure(
        &self,
        text: &str,
        font_family: &str,
        font_size: f32,
        weight: u16,
        italic: bool,
    ) -> TextMetrics {
        let attrs = build_attrs(font_family, font_size, weight, italic);
        let ctx = self.inner.lock();
        ctx.measure_text(text, &attrs)
    }

    fn line_height(&self, font_family: &str, font_size: f32, weight: u16, italic: bool) -> f32 {
        let attrs = build_attrs(font_family, font_size, weight, italic);
        let ctx = self.inner.lock();
        let m = ctx.measure_text(" ", &attrs);
        let line = m.font_bounding_box_ascent + m.font_bounding_box_descent;
        if line > 0.0 { line } else { font_size * 1.2 }
    }

    fn register_font(&self, aliases: &[String], bytes: &[u8]) -> Option<String> {
        let mut ctx = self.inner.lock();
        ctx.register_family_aliases(aliases, bytes).and_then(|reg| {
            reg.internal_family
                .or_else(|| reg.aliases.into_iter().next())
        })
    }
}

fn build_attrs(font_family: &str, font_size: f32, weight: u16, italic: bool) -> TextAttrs {
    TextAttrs {
        size: font_size,
        families: std::sync::Arc::new(vec![font_family.to_string(), "sans-serif".into()]),
        weight,
        italic,
        align: TextAlign::Start,
        baseline: TextBaseline::Alphabetic,
        direction: TextDirection::Inherit,
    }
}
