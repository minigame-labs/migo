//! Thread-safe measureText façade (F-2).
//!
//! The render thread's `TextContext` owns HarfBuzz + ICU + Skia
//! shaping state that is naturally on the render side for
//! `fillText` / `strokeText` (both need a live GL canvas).  But
//! `measureText` / `getTextLineHeight` — the hot path for UI
//! auto-sizing — never touches GL; making JS go through a
//! cross-thread RPC just to read a cached paragraph metric is
//! pure latency.
//!
//! This module exposes the trait the JS-thread measure op goes
//! through, plus a lightweight shared-handle type, so:
//!
//!   * `shared` stays free of `skia-safe` (no cycle into
//!     `graphics` would be possible otherwise).
//!   * `graphics` provides the concrete implementation (registered
//!     at render-thread startup) and hides the Skia types behind
//!     the trait.
//!   * `js-runtime` holds an `Arc<dyn TextMeasurer>` on
//!     `CanvasOpState` and skips the RenderCommand round-trip for
//!     every `op_measure_text*` call, falling back to the
//!     existing sync-op path only when the handle is missing
//!     (older tests / custom embedders).
//!
//! Font state parity: the trait takes **parsed** font attrs on
//! every call (family, size, weight, italic) so the JS side can
//! drive measurement without having to keep the CSS `ctx.font`
//! parser consistent with the graphics crate.  The concrete
//! implementation owns a `parking_lot::Mutex<TextContext>`
//! internally, and `op_load_font` dispatches through the trait
//! to keep JS and render-thread views of the font registry in
//! sync.

use crate::protocol::render_cmd::TextMetrics;

/// Thread-safe measurement handle.
///
/// Implementations are expected to wrap an internally-mutable
/// shaping context (typically `parking_lot::Mutex<TextContext>`)
/// so the trait can be `Send + Sync` even when the underlying
/// Skia handles aren't individually thread-safe — serialised
/// access through the mutex is what makes it OK to move between
/// threads.
///
/// The trait is **intentionally minimal**: only the operations
/// the JS hot path actually needs (measure + line-height + font
/// registration).  Adding new methods is a wire-format break
/// between `shared` and the `graphics` impl, which is why
/// `#[non_exhaustive]`-style guarantees aren't offered.
pub trait TextMeasurer: Send + Sync + 'static {
    /// Measure `text` using the given font descriptor.
    ///
    /// `font_family` is the head of the CSS family list
    /// (`ctx.font` post-split); `weight` and `italic` come from
    /// the shorthand parser.  Returns the same `TextMetrics`
    /// shape the `Canvas2DCmd::MeasureText` path produces, so
    /// the JS side doesn't have to branch on which path served
    /// the metric.
    fn measure(
        &self,
        text: &str,
        font_family: &str,
        font_size: f32,
        weight: u16,
        italic: bool,
    ) -> TextMetrics;

    /// Line-height helper paralleling `RenderCommand::GetTextLineHeight`.
    fn line_height(&self, font_family: &str, font_size: f32, weight: u16, italic: bool) -> f32;

    /// Register a font byte blob under one or more aliases.
    /// Returns the canonical family name (typically the font's
    /// internal `name` table entry) on success, or `None` on
    /// parse failure.
    fn register_font(&self, aliases: &[String], bytes: &[u8]) -> Option<String>;

    /// G-2: convenience overload that takes a raw CSS font
    /// shorthand and forwards it through [`crate::css_font::
    /// parse_css_font`] so callers on the JS thread don't need
    /// their own parser.  Default impl so existing implementors
    /// pick it up automatically.
    fn measure_css(&self, text: &str, css_font: &str) -> TextMetrics {
        let p = crate::css_font::parse_css_font(css_font);
        self.measure(text, &p.family, p.size, p.weight, p.italic)
    }

    /// G-2 companion to [`Self::measure_css`] — same parse flow
    /// but for `getTextLineHeight`.
    fn line_height_css(&self, css_font: &str) -> f32 {
        let p = crate::css_font::parse_css_font(css_font);
        self.line_height(&p.family, p.size, p.weight, p.italic)
    }
}

/// Process-wide shared handle.  Cheap to `Clone`: single
/// refcount bump; every clone dispatches through the same
/// underlying mutex-guarded context.
pub type SharedTextMeasurer = std::sync::Arc<dyn TextMeasurer>;
