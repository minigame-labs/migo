//! Unified frame command collector for Canvas2D + WebGL interleaving.
//!
//! Replaces the separate `FrameCommandCollector` + `GlBatchCollector` with a
//! single ordered timeline of segments. At frame end, one `FramePacket` is
//! built with `Materialize` barriers inserted at Canvas2D→GL transitions.

use std::collections::HashSet;

use shared::protocol::frame_packet::FrameOp;
use shared::protocol::render_cmd::{
    Canvas2DCmd, CanvasBatchPayload, DirtyRect, GLCmd, GlBatchPayload,
};

// ── Segment types ──────────────────────────────────────────────────

pub(crate) struct Canvas2DSegment {
    pub canvas_id: u32,
    pub commands: Vec<Canvas2DCmd>,
    pub dirty_rect: Option<DirtyRect>,
    /// Once true, dirty_rect is permanently None for this segment.
    /// Set when a command with unknowable bounds is encountered (text,
    /// path draw, clip). Prevents the scissor hint from being used.
    dirty_poisoned: bool,
}

impl Canvas2DSegment {
    #[inline]
    fn mark_dirty(&mut self, x: f32, y: f32, w: f32, h: f32) {
        if self.dirty_poisoned {
            return;
        }
        self.dirty_rect = Some(match self.dirty_rect {
            Some(rect) => {
                let nx = rect.x.min(x);
                let ny = rect.y.min(y);
                let nx2 = (rect.x + rect.width).max(x + w);
                let ny2 = (rect.y + rect.height).max(y + h);
                DirtyRect {
                    x: nx,
                    y: ny,
                    width: nx2 - nx,
                    height: ny2 - ny,
                }
            }
            None => DirtyRect {
                x,
                y,
                width: w,
                height: h,
            },
        });
    }

    /// Permanently invalidate dirty_rect for this segment.
    /// Called when a command has unknowable bounds (text, path, clip).
    #[inline]
    fn poison_dirty(&mut self) {
        self.dirty_rect = None;
        self.dirty_poisoned = true;
    }

    #[inline]
    fn mark_dirty_for_cmd(&mut self, cmd: &Canvas2DCmd) {
        match cmd {
            // ── Safe rect/image draws (known bounds, no state dependency) ──
            Canvas2DCmd::FillRect { x, y, w, h }
            | Canvas2DCmd::ClearRect { x, y, w, h } => {
                self.mark_dirty(*x, *y, *w, *h);
            }
            Canvas2DCmd::DrawImage { dx, dy, dw, dh, .. } => {
                self.mark_dirty(*dx, *dy, *dw, *dh);
            }
            Canvas2DCmd::DrawImageBatch { draws } => {
                for d in draws {
                    self.mark_dirty(d.dx, d.dy, d.dw, d.dh);
                }
            }

            // ── Draws with unknowable bounds — poison scissor hint ──
            Canvas2DCmd::StrokeRect { .. } // lineWidth expansion unknown JS-side
            | Canvas2DCmd::FillText { .. }
            | Canvas2DCmd::StrokeText { .. }
            | Canvas2DCmd::Fill
            | Canvas2DCmd::Stroke
            | Canvas2DCmd::Clip => {
                self.poison_dirty();
            }

            // ── State changes that invalidate subsequent draw bounds ──
            // Transform commands: subsequent rect/image coords no longer
            // map 1:1 to screen pixels.
            Canvas2DCmd::SetTransform { .. }
            | Canvas2DCmd::Translate { .. }
            | Canvas2DCmd::Rotate { .. }
            | Canvas2DCmd::Scale { .. }
            | Canvas2DCmd::ResetTransform => {
                self.poison_dirty();
            }
            // Shadow commands: any of these becoming active expands draw bounds.
            Canvas2DCmd::SetShadowBlur { .. }
            | Canvas2DCmd::SetShadowColor { .. }
            | Canvas2DCmd::SetShadowOffsetX { .. }
            | Canvas2DCmd::SetShadowOffsetY { .. } => {
                self.poison_dirty();
            }
            // LineWidth: affects StrokeRect bounds (already poisoned above)
            // but also affects Stroke (path) which is already poisoned.
            // Poison anyway to be safe with any future stroke-related draws.
            Canvas2DCmd::SetLineWidth { .. } => {
                self.poison_dirty();
            }

            // ── Everything else (path building, other setters) — no dirty impact ──
            _ => {}
        }
    }
}

pub(crate) struct GlSegment {
    pub commands: Vec<GLCmd>,
}

pub(crate) enum FrameSegment {
    Canvas2D(Canvas2DSegment),
    GL(GlSegment),
}

// ── Current-segment tracker ────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
enum CurrentKind {
    None,
    Canvas2D(u32), // canvas_id
    GL,
}

// ── Collector ──────────────────────────────────────────────────────

pub(crate) struct UnifiedFrameCollector {
    segments: Vec<FrameSegment>,
    current: CurrentKind,
    /// Rough byte count of all commands pushed since the last flush.
    /// Incremented in `push_canvas2d` / `push_gl` and zeroed when
    /// `build_frame_packet_inner` consumes the segments.  See
    /// [`AUTO_FLUSH_SOFT_BUDGET_BYTES`] for the flush policy.
    pending_bytes: usize,
}

/// Soft cap on the approximate byte-size of pending commands in the
/// collector.  When a single-frame batch crosses this threshold, call
/// sites should invoke [`UnifiedFrameCollector::flush_as_barrier`]
/// to hand the accumulated work off to the render thread rather than
/// keeping it stacked in JS-side memory.
///
/// Chromium's `CanvasResourceProvider::auto_flush` drives its backlog
/// cutoff from per-recording pinned-image byte budgets, not command
/// counts: a few MB of `bufferData` pinned by a single call can
/// outweigh thousands of tiny `fillRect`s.  We mirror that model by
/// tracking bytes rather than just segment count.
///
/// 4 MB is a conservative starting point — large enough to cover
/// typical small-game frames (hundreds of draws, kilobytes of
/// uniforms) without waking the render thread mid-frame for every
/// workload, small enough that a runaway `bufferData(4_MB_mesh)` or a
/// gigantic DrawImageBatch can't pin the JS heap unbounded while the
/// render thread is blocked.
pub(crate) const AUTO_FLUSH_SOFT_BUDGET_BYTES: usize = 4 * 1024 * 1024;

impl UnifiedFrameCollector {
    pub(crate) fn new() -> Self {
        Self {
            segments: Vec::new(),
            current: CurrentKind::None,
            pending_bytes: 0,
        }
    }

    /// Rough upper bound on the bytes currently retained by pending
    /// commands.  Counted per-push using `std::mem::size_of` plus a
    /// conservative per-variant overhead: we overshoot slightly on
    /// small scalar commands and undercount when a variant owns a
    /// long-lived heap buffer (a future refinement could inspect the
    /// `Vec` capacities), so the value is a *heuristic* for flush
    /// decisions — never load-bearing for correctness.
    #[inline]
    /// `#[allow(dead_code)]`: only exercised by tests today, but
    /// exposed as public-in-crate so the debug overlay / future
    /// profiling hooks can read the live byte budget without
    /// poking `self.pending_bytes` directly.
    #[allow(dead_code)]
    pub(crate) fn approx_pending_bytes(&self) -> usize {
        self.pending_bytes
    }

    /// `true` when the accumulated batch has grown past the soft
    /// budget — call sites that reach a frame boundary should treat
    /// this as a hint to flush a barrier instead of continuing to
    /// accumulate.  Purely advisory: skipping the flush can't break
    /// correctness, only degrade memory / latency.
    #[inline]
    pub(crate) fn should_auto_flush(&self) -> bool {
        self.pending_bytes >= AUTO_FLUSH_SOFT_BUDGET_BYTES
    }

    pub(crate) fn push_canvas2d(&mut self, canvas_id: u32, cmd: Canvas2DCmd) {
        if self.current != CurrentKind::Canvas2D(canvas_id) {
            self.segments.push(FrameSegment::Canvas2D(Canvas2DSegment {
                canvas_id,
                commands: Vec::with_capacity(256),
                dirty_rect: None,
                dirty_poisoned: false,
            }));
            self.current = CurrentKind::Canvas2D(canvas_id);
        }
        // Walk the full deep size (enum base + heap payload) so the
        // budget reflects reality.  The previous implementation
        // only added `size_of::<Canvas2DCmd>()`, missing text
        // strings, line dash vectors, gradient stops, and
        // DrawImageBatch slot arrays — a single `fillText` with a
        // 1 KB user string reported ~150 B instead of 1.15 KB.
        self.pending_bytes = self
            .pending_bytes
            .saturating_add(cmd.approx_deep_size_bytes());
        shared::stats::set_collector_pending_bytes(self.pending_bytes as u32);
        if let Some(FrameSegment::Canvas2D(seg)) = self.segments.last_mut() {
            seg.mark_dirty_for_cmd(&cmd);
            seg.commands.push(cmd);
        }
    }

    pub(crate) fn push_gl(&mut self, cmd: GLCmd) {
        if self.current != CurrentKind::GL {
            self.segments.push(FrameSegment::GL(GlSegment {
                commands: Vec::with_capacity(256),
            }));
            self.current = CurrentKind::GL;
        }
        // Deep size walk - see the Canvas2D counterpart.  This is
        // especially critical for `BufferData(Vec<u8>)` /
        // `TexImage2D(Arc<Vec<u8>>)` / `ShaderSource(String)`, the
        // three GL variants most likely to single-handedly blow
        // the budget.
        self.pending_bytes = self
            .pending_bytes
            .saturating_add(cmd.approx_deep_size_bytes());
        shared::stats::set_collector_pending_bytes(self.pending_bytes as u32);
        if let Some(FrameSegment::GL(seg)) = self.segments.last_mut() {
            seg.commands.push(cmd);
        }
    }

    /// Fast-path variant of [`Self::push_gl`] for scalar-only
    /// commands (viewport, bind*, uniform scalars, enable/disable,
    /// draw, scissor...).
    ///
    /// Scalar ops are the majority of a WebGL frame by count —
    /// Cocos/three.js emit hundreds per frame — and every byte of
    /// overhead multiplies.  The fast path drops two pieces of
    /// work versus `push_gl`:
    ///
    /// 1. No `approx_deep_size_bytes()` match.  The enum base size
    ///    is a compile-time constant and the heap payload is known
    ///    to be zero, so we add `size_of::<GLCmd>()` directly.
    /// 2. No auto-flush guard.  Only fat payloads
    ///    (`BufferData` / `TexImage2D` / `ShaderSource`) can push
    ///    the batch past the 4 MiB soft budget; scalar commands
    ///    add at most ~128 B apiece, so it takes tens of thousands
    ///    of them in a single frame to matter.  Callers that do
    ///    produce that many still get the check on the next fat
    ///    command or at frame flush.
    ///
    /// Callers MUST uphold the precondition that the pushed
    /// variant carries no heap payload.  Passing a
    /// `BufferData { data: Some(_) }` through this path would
    /// under-count the batch and defeat the auto-flush guard.
    #[inline]
    pub(crate) fn push_gl_fast(&mut self, cmd: GLCmd) {
        const BASE_BYTES: usize = std::mem::size_of::<GLCmd>();
        if self.current != CurrentKind::GL {
            self.segments.push(FrameSegment::GL(GlSegment {
                commands: Vec::with_capacity(256),
            }));
            self.current = CurrentKind::GL;
        }
        self.pending_bytes = self.pending_bytes.saturating_add(BASE_BYTES);
        shared::stats::set_collector_pending_bytes(self.pending_bytes as u32);
        if let Some(FrameSegment::GL(seg)) = self.segments.last_mut() {
            seg.commands.push(cmd);
        }
    }

    /// Build a single FramePacket from all accumulated segments.
    /// Inserts `Materialize` ops at each Canvas2D→GL boundary.
    /// When `materialize_trailing` is true, also materializes any remaining
    /// pending 2D canvases at the end (needed for sync barriers).
    /// Resets the collector for the next frame.
    fn build_frame_packet_inner(
        &mut self,
        present: bool,
        materialize_trailing: bool,
    ) -> Option<shared::FramePacket> {
        if self.segments.is_empty() {
            // Defence in depth: if a prior `push_*` incremented
            // `pending_bytes` but a bug prevented the segment
            // from landing in `segments`, the early return here
            // would otherwise leave a stale counter that tricks
            // `should_auto_flush` into firing forever.  Practical
            // reachability is zero (every push creates a segment
            // before incrementing the counter) but the invariant
            // ("segments empty ⇒ pending_bytes == 0") is cheap to
            // enforce and makes the flush protocol easier to
            // audit.
            self.pending_bytes = 0;
            shared::stats::set_collector_pending_bytes(0);
            self.current = CurrentKind::None;
            return None;
        }

        let segments = std::mem::take(&mut self.segments);
        self.current = CurrentKind::None;
        self.pending_bytes = 0;
        shared::stats::set_collector_pending_bytes(0);

        let mut builder = shared::FramePacketBuilder::new(0, 0.0).push(FrameOp::BeginFrame);

        // Track which canvases have unmaterialized 2D work as we scan.
        // Canvas2D segments add to the set; GL segments consume it.
        let mut pending_2d: HashSet<u32> = HashSet::new();

        for seg in segments {
            match seg {
                FrameSegment::Canvas2D(s) => {
                    pending_2d.insert(s.canvas_id);
                    builder = builder.push(FrameOp::CanvasBatch(CanvasBatchPayload {
                        canvas_id: s.canvas_id,
                        commands: s.commands,
                        present,
                        dirty_rect: s.dirty_rect,
                    }));
                }
                FrameSegment::GL(s) => {
                    // Insert Materialize for all canvases with pending 2D work.
                    for &cid in &pending_2d {
                        builder = builder.push(FrameOp::Materialize { canvas_id: cid });
                    }
                    pending_2d.clear();
                    builder = builder.push(FrameOp::GlBatch(GlBatchPayload {
                        commands: s.commands,
                    }));
                }
            }
        }

        // Barrier mode: materialize any trailing pending 2D canvases so
        // a subsequent sync readback (readPixels, getImageData) sees results.
        if materialize_trailing && !pending_2d.is_empty() {
            for &cid in &pending_2d {
                builder = builder.push(FrameOp::Materialize { canvas_id: cid });
            }
        }

        if present {
            builder = builder.push(FrameOp::Present);
        }

        Some(builder.finish())
    }

    /// Build a single FramePacket from all accumulated segments.
    /// Inserts `Materialize` ops at each Canvas2D→GL boundary.
    /// Resets the collector for the next frame.
    pub(crate) fn build_frame_packet(&mut self, present: bool) -> Option<shared::FramePacket> {
        self.build_frame_packet_inner(present, false)
    }

    /// Flush all pending segments as a non-presenting partial FramePacket.
    /// Used before sync operations (getImageData, readPixels, GL queries).
    /// Materializes ALL trailing pending 2D canvases so the sync op sees results.
    pub(crate) fn flush_as_barrier(&mut self) -> Option<shared::FramePacket> {
        self.build_frame_packet_inner(false, true)
    }

    /// Reset the buffer for a specific canvas at frame-begin.
    pub(crate) fn frame_begin(&mut self, _canvas_id: u32) {
        // Currently a no-op for the unified collector —
        // segments are cleared by build_frame_packet at frame end.
        // Kept for API compatibility with op_frame_begin.
    }

    // ── Canvas2D forwarding methods ────────────────────────────────
    // These mirror FrameCommandCollector's API so ops only need a type
    // change in their borrow call. Dedup is intentionally omitted in
    // this first pass; the render thread handles redundant state-sets
    // correctly. Dedup can be re-added as a follow-up optimization.

    #[inline]
    pub(crate) fn push(&mut self, canvas_id: u32, cmd: Canvas2DCmd) {
        self.push_canvas2d(canvas_id, cmd);
    }

    #[inline]
    pub(crate) fn set_fill_color(&mut self, canvas_id: u32, color: shared::protocol::color::Color) {
        self.push_canvas2d(canvas_id, Canvas2DCmd::SetFillStyle { color });
    }

    #[inline]
    pub(crate) fn set_stroke_color(
        &mut self,
        canvas_id: u32,
        color: shared::protocol::color::Color,
    ) {
        self.push_canvas2d(canvas_id, Canvas2DCmd::SetStrokeStyle { color });
    }

    #[inline]
    pub(crate) fn set_line_width(&mut self, canvas_id: u32, width: f32) {
        self.push_canvas2d(canvas_id, Canvas2DCmd::SetLineWidth { width });
    }

    #[inline]
    pub(crate) fn set_global_alpha(&mut self, canvas_id: u32, alpha: f32) {
        self.push_canvas2d(canvas_id, Canvas2DCmd::SetGlobalAlpha { alpha });
    }

    #[inline]
    pub(crate) fn set_composite_operation(&mut self, canvas_id: u32, op: u8) {
        self.push_canvas2d(canvas_id, Canvas2DCmd::SetCompositeOperation { op });
    }

    #[inline]
    pub(crate) fn set_line_dash(&mut self, canvas_id: u32, segments: Vec<f32>) {
        self.push_canvas2d(canvas_id, Canvas2DCmd::SetLineDash { segments });
    }

    #[inline]
    pub(crate) fn set_line_dash_offset(&mut self, canvas_id: u32, offset: f32) {
        self.push_canvas2d(canvas_id, Canvas2DCmd::SetLineDashOffset { offset });
    }

    #[inline]
    pub(crate) fn set_shadow_blur(&mut self, canvas_id: u32, blur: f32) {
        self.push_canvas2d(canvas_id, Canvas2DCmd::SetShadowBlur { blur });
    }

    #[inline]
    pub(crate) fn set_shadow_color(
        &mut self,
        canvas_id: u32,
        color: shared::protocol::color::Color,
    ) {
        self.push_canvas2d(canvas_id, Canvas2DCmd::SetShadowColor { color });
    }

    #[inline]
    pub(crate) fn set_shadow_offset_x(&mut self, canvas_id: u32, offset: f32) {
        self.push_canvas2d(canvas_id, Canvas2DCmd::SetShadowOffsetX { offset });
    }

    #[inline]
    pub(crate) fn set_shadow_offset_y(&mut self, canvas_id: u32, offset: f32) {
        self.push_canvas2d(canvas_id, Canvas2DCmd::SetShadowOffsetY { offset });
    }

    #[inline]
    pub(crate) fn set_fill_style_gradient(
        &mut self,
        canvas_id: u32,
        gradient_type: shared::protocol::render_cmd::GradientType,
        x0: f32,
        y0: f32,
        r0: f32,
        x1: f32,
        y1: f32,
        r1: f32,
        stops: Vec<shared::protocol::render_cmd::GradientStop>,
    ) {
        self.push_canvas2d(
            canvas_id,
            Canvas2DCmd::SetFillStyleGradient {
                gradient_type,
                x0,
                y0,
                r0,
                x1,
                y1,
                r1,
                stops,
            },
        );
    }

    #[inline]
    pub(crate) fn set_stroke_style_gradient(
        &mut self,
        canvas_id: u32,
        gradient_type: shared::protocol::render_cmd::GradientType,
        x0: f32,
        y0: f32,
        r0: f32,
        x1: f32,
        y1: f32,
        r1: f32,
        stops: Vec<shared::protocol::render_cmd::GradientStop>,
    ) {
        self.push_canvas2d(
            canvas_id,
            Canvas2DCmd::SetStrokeStyleGradient {
                gradient_type,
                x0,
                y0,
                r0,
                x1,
                y1,
                r1,
                stops,
            },
        );
    }

    #[inline]
    pub(crate) fn set_fill_style_pattern(
        &mut self,
        canvas_id: u32,
        image_id: u32,
        repeat_x: bool,
        repeat_y: bool,
    ) {
        self.push_canvas2d(
            canvas_id,
            Canvas2DCmd::SetFillStylePattern {
                image_id,
                repeat_x,
                repeat_y,
            },
        );
    }

    #[inline]
    pub(crate) fn set_stroke_style_pattern(
        &mut self,
        canvas_id: u32,
        image_id: u32,
        repeat_x: bool,
        repeat_y: bool,
    ) {
        self.push_canvas2d(
            canvas_id,
            Canvas2DCmd::SetStrokeStylePattern {
                image_id,
                repeat_x,
                repeat_y,
            },
        );
    }

    #[inline]
    pub(crate) fn set_line_cap(&mut self, canvas_id: u32, cap: u8) {
        self.push_canvas2d(canvas_id, Canvas2DCmd::SetLineCap { cap });
    }

    #[inline]
    pub(crate) fn set_line_join(&mut self, canvas_id: u32, join: u8) {
        self.push_canvas2d(canvas_id, Canvas2DCmd::SetLineJoin { join });
    }

    #[inline]
    pub(crate) fn set_miter_limit(&mut self, canvas_id: u32, limit: f32) {
        self.push_canvas2d(canvas_id, Canvas2DCmd::SetMiterLimit { limit });
    }

    #[inline]
    pub(crate) fn set_text_align(
        &mut self,
        canvas_id: u32,
        align: shared::protocol::render_cmd::TextAlign,
    ) {
        self.push_canvas2d(canvas_id, Canvas2DCmd::SetTextAlign { align });
    }

    #[inline]
    pub(crate) fn set_text_baseline(
        &mut self,
        canvas_id: u32,
        baseline: shared::protocol::render_cmd::TextBaseline,
    ) {
        self.push_canvas2d(canvas_id, Canvas2DCmd::SetTextBaseline { baseline });
    }

    #[inline]
    pub(crate) fn set_font(&mut self, canvas_id: u32, font: String) {
        self.push_canvas2d(canvas_id, Canvas2DCmd::SetFont { font });
    }

    /// Reset dedup state (no-op without dedup) and push Restore.
    pub(crate) fn restore(&mut self, canvas_id: u32) {
        self.push_canvas2d(canvas_id, Canvas2DCmd::Restore);
    }
}

/// Flush all pending unified segments to the render thread as a barrier.
/// Call before any sync operation that observes prior drawing results.
pub(crate) fn flush_unified_barrier(state: &mut deno_core::OpState) {
    let packet = {
        if let Some(collector) = state.try_borrow_mut::<UnifiedFrameCollector>() {
            collector.flush_as_barrier()
        } else {
            None
        }
    };

    if let Some(packet) = packet {
        // Diag: how many GL ops did the caller pile up before the
        // sync barrier?  A spike here correlates with the
        // [MigoPerf][SyncOp] wait time on the render-thread side.
        let gl_ops = packet
            .ops()
            .iter()
            .map(|op| match op {
                shared::protocol::FrameOp::GlBatch(p) => p.commands.len(),
                _ => 0,
            })
            .sum::<usize>();
        if gl_ops >= 64 {
            tracing::warn!(
                "[MigoPerf][SyncFlush] sync barrier flushed {gl_ops} pending GL ops"
            );
        }
        let ctx = state.borrow::<shared::op_state::CanvasOpState>();
        if let Err(e) = ctx
            .tx
            .send(shared::protocol::render_cmd::RenderCommand::FramePacket(
                packet,
            ))
        {
            tracing::error!("flush_unified_barrier: send failed: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_collector_returns_none() {
        let mut c = UnifiedFrameCollector::new();
        assert!(c.build_frame_packet(true).is_none());
    }

    #[test]
    fn canvas2d_only_frame_produces_single_canvas_batch() {
        let mut c = UnifiedFrameCollector::new();
        c.push_canvas2d(1, Canvas2DCmd::Save);
        c.push_canvas2d(
            1,
            Canvas2DCmd::FillRect {
                x: 0.0,
                y: 0.0,
                w: 10.0,
                h: 10.0,
            },
        );

        let packet = c.build_frame_packet(true).unwrap();
        let ops = packet.ops();
        assert!(matches!(ops[0], FrameOp::BeginFrame));
        assert!(
            matches!(&ops[1], FrameOp::CanvasBatch(p) if p.canvas_id == 1 && p.commands.len() == 2)
        );
        assert!(matches!(ops.last().unwrap(), FrameOp::Present));
    }

    #[test]
    fn gl_only_frame_produces_single_gl_batch_no_present() {
        let mut c = UnifiedFrameCollector::new();
        c.push_gl(GLCmd::Clear {
            canvas_id: 1,
            bit_field: 0x4000,
        });

        let packet = c.build_frame_packet(false).unwrap();
        let ops = packet.ops();
        assert!(matches!(ops[0], FrameOp::BeginFrame));
        assert!(matches!(&ops[1], FrameOp::GlBatch(p) if p.commands.len() == 1));
        assert!(!matches!(ops.last().unwrap(), FrameOp::Present));
    }

    #[test]
    fn canvas2d_then_gl_inserts_materialize_barrier() {
        let mut c = UnifiedFrameCollector::new();
        c.push_canvas2d(
            1,
            Canvas2DCmd::FillRect {
                x: 0.0,
                y: 0.0,
                w: 10.0,
                h: 10.0,
            },
        );
        c.push_gl(GLCmd::Clear {
            canvas_id: 1,
            bit_field: 0x4000,
        });

        let packet = c.build_frame_packet(true).unwrap();
        let ops = packet.ops();
        // [BeginFrame, CanvasBatch(1), Materialize(1), GlBatch, Present]
        assert_eq!(ops.len(), 5);
        assert!(matches!(ops[0], FrameOp::BeginFrame));
        assert!(matches!(&ops[1], FrameOp::CanvasBatch(p) if p.canvas_id == 1));
        assert!(matches!(ops[2], FrameOp::Materialize { canvas_id: 1 }));
        assert!(matches!(&ops[3], FrameOp::GlBatch(_)));
        assert!(matches!(ops[4], FrameOp::Present));
    }

    #[test]
    fn gl_then_canvas2d_does_not_insert_materialize() {
        let mut c = UnifiedFrameCollector::new();
        c.push_gl(GLCmd::Clear {
            canvas_id: 1,
            bit_field: 0x4000,
        });
        c.push_canvas2d(
            1,
            Canvas2DCmd::FillRect {
                x: 0.0,
                y: 0.0,
                w: 10.0,
                h: 10.0,
            },
        );

        let packet = c.build_frame_packet(true).unwrap();
        let ops = packet.ops();
        // [BeginFrame, GlBatch, CanvasBatch(1), Present] — no Materialize
        assert_eq!(ops.len(), 4);
        assert!(matches!(ops[0], FrameOp::BeginFrame));
        assert!(matches!(&ops[1], FrameOp::GlBatch(_)));
        assert!(matches!(&ops[2], FrameOp::CanvasBatch(p) if p.canvas_id == 1));
        assert!(matches!(ops[3], FrameOp::Present));
    }

    #[test]
    fn interleaved_2d_gl_2d_produces_correct_sequence() {
        let mut c = UnifiedFrameCollector::new();
        c.push_canvas2d(
            1,
            Canvas2DCmd::FillRect {
                x: 0.0,
                y: 0.0,
                w: 10.0,
                h: 10.0,
            },
        );
        c.push_gl(GLCmd::Clear {
            canvas_id: 1,
            bit_field: 0x4000,
        });
        c.push_canvas2d(
            1,
            Canvas2DCmd::FillRect {
                x: 0.0,
                y: 0.0,
                w: 20.0,
                h: 20.0,
            },
        );

        let packet = c.build_frame_packet(true).unwrap();
        let ops = packet.ops();
        // [BeginFrame, CanvasBatch(1), Materialize(1), GlBatch, CanvasBatch(1), Present]
        assert_eq!(ops.len(), 6);
        assert!(matches!(&ops[1], FrameOp::CanvasBatch(p) if p.commands.len() == 1));
        assert!(matches!(ops[2], FrameOp::Materialize { canvas_id: 1 }));
        assert!(matches!(&ops[3], FrameOp::GlBatch(p) if p.commands.len() == 1));
        assert!(matches!(&ops[4], FrameOp::CanvasBatch(p) if p.commands.len() == 1));
    }

    #[test]
    fn multiple_canvases_all_get_materialized_before_gl() {
        let mut c = UnifiedFrameCollector::new();
        c.push_canvas2d(1, Canvas2DCmd::Save);
        c.push_canvas2d(2, Canvas2DCmd::Save);
        c.push_gl(GLCmd::Clear {
            canvas_id: 1,
            bit_field: 0x4000,
        });

        let packet = c.build_frame_packet(true).unwrap();
        let ops = packet.ops();
        let mat_count = ops
            .iter()
            .filter(|op| matches!(op, FrameOp::Materialize { .. }))
            .count();
        assert_eq!(mat_count, 2);
    }

    #[test]
    fn build_resets_collector_for_next_frame() {
        let mut c = UnifiedFrameCollector::new();
        c.push_canvas2d(1, Canvas2DCmd::Save);
        let _ = c.build_frame_packet(true);
        assert!(c.build_frame_packet(true).is_none());
    }

    #[test]
    fn approx_bytes_grows_monotonically_until_flush() {
        let mut c = UnifiedFrameCollector::new();
        assert_eq!(c.approx_pending_bytes(), 0);
        c.push_canvas2d(1, Canvas2DCmd::Save);
        let after_one = c.approx_pending_bytes();
        assert!(after_one > 0, "after one push should be non-zero");
        c.push_gl(GLCmd::Clear {
            canvas_id: 1u32.into(),
            bit_field: 0x4000,
        });
        assert!(c.approx_pending_bytes() > after_one, "second push must add");
        // A barrier flush drains the counter back to zero.
        let _ = c.flush_as_barrier();
        assert_eq!(c.approx_pending_bytes(), 0, "flush must reset bytes");
    }

    // ---- Deep-size budgeting regression (P0-2) ----------------------
    //
    // Old impl added `size_of::<Cmd>()` only, so a single
    // `bufferData(8MB)` pushed ~200 B into pending_bytes and
    // `should_auto_flush()` never tripped.  These tests pin the
    // correct behaviour: payload heap bytes must contribute to the
    // budget within a factor of 2 of the true payload size.

    #[test]
    fn single_large_buffer_data_trips_auto_flush() {
        let mut c = UnifiedFrameCollector::new();
        let data = vec![0u8; 8 * 1024 * 1024]; // 8 MB mesh
        c.push_gl(GLCmd::BufferData {
            canvas_id: 1u32.into(),
            target: 0x8892,
            size: data.len() as i32,
            data: Some(data),
            usage: 0x88E4,
        });
        assert!(
            c.approx_pending_bytes() >= 8 * 1024 * 1024,
            "8MB bufferData under-accounted: {}",
            c.approx_pending_bytes(),
        );
        assert!(
            c.should_auto_flush(),
            "8MB single push must trip the 4MB soft budget"
        );
    }

    #[test]
    fn many_small_shader_sources_sum_near_their_total_payload() {
        // Regression for "count pages, not ops": 1000 shader
        // sources of ~4 KB each — assert the budget reflects the
        // aggregate text bytes, not just the enum wrappers.
        let mut c = UnifiedFrameCollector::new();
        for i in 0..1000u32 {
            let mut source = String::with_capacity(5000);
            source.push_str(&format!("// sh{}\n", i));
            source.push_str(&"x".repeat(4000));
            c.push_gl(GLCmd::ShaderSource {
                shader_id: (i + 1).into(),
                source,
                resp: None,
            });
        }
        let bytes = c.approx_pending_bytes();
        let lower = 1000 * 4000; // at least the text body per source
        assert!(
            bytes >= lower,
            "1000 sources should account for >={lower} bytes; got {bytes}",
        );
        assert!(
            c.should_auto_flush(),
            "{bytes} bytes must trip 4MB auto-flush",
        );
    }

    #[test]
    fn draw_image_batch_capacity_changes_reflect_in_budget() {
        let mut c1 = UnifiedFrameCollector::new();
        c1.push_canvas2d(
            1,
            Canvas2DCmd::DrawImageBatch {
                draws: vec![entry(); 10],
            },
        );
        let small = c1.approx_pending_bytes();

        let mut c2 = UnifiedFrameCollector::new();
        c2.push_canvas2d(
            1,
            Canvas2DCmd::DrawImageBatch {
                draws: vec![entry(); 1000],
            },
        );
        let large = c2.approx_pending_bytes();

        assert!(
            large
                > small + 900 * std::mem::size_of::<shared::protocol::render_cmd::DrawImageEntry>(),
            "1000-entry batch ({}b) should far exceed 10-entry batch ({}b)",
            large,
            small,
        );
    }

    fn entry() -> shared::protocol::render_cmd::DrawImageEntry {
        shared::protocol::render_cmd::DrawImageEntry {
            image_id: 1,
            sx: 0.0,
            sy: 0.0,
            sw: 1.0,
            sh: 1.0,
            dx: 0.0,
            dy: 0.0,
            dw: 1.0,
            dh: 1.0,
        }
    }

    #[test]
    fn empty_flush_does_not_leak_pending_bytes() {
        // Regression: `build_frame_packet_inner` early-returned
        // when segments was empty WITHOUT resetting the bytes
        // counter.  A bug that bumps `pending_bytes` without
        // landing a segment would then make `should_auto_flush`
        // stick on "true" forever.  The unconditional reset on
        // the empty path pins the invariant:
        //   segments.is_empty() ⇒ pending_bytes == 0.
        let mut c = UnifiedFrameCollector::new();
        c.pending_bytes = 7777;
        assert!(c.build_frame_packet(true).is_none());
        assert_eq!(c.approx_pending_bytes(), 0);
        assert!(!c.should_auto_flush());
    }

    #[test]
    fn flush_as_barrier_then_build_frame_packet_keeps_counter_zero() {
        // Successive flushes must repeatedly land at zero.  The
        // old path would occasionally double-reset via two
        // codepaths that both wrote to `pending_bytes`; this
        // test pins the idempotence.
        let mut c = UnifiedFrameCollector::new();
        c.push_canvas2d(1, Canvas2DCmd::Save);
        c.push_gl(GLCmd::Clear {
            canvas_id: 1u32.into(),
            bit_field: 0x4000,
        });
        let _ = c.flush_as_barrier();
        assert_eq!(c.approx_pending_bytes(), 0);

        c.push_canvas2d(1, Canvas2DCmd::Save);
        let _ = c.build_frame_packet(true);
        assert_eq!(c.approx_pending_bytes(), 0);

        assert!(c.build_frame_packet(true).is_none());
        assert_eq!(c.approx_pending_bytes(), 0);
    }

    #[test]
    fn should_auto_flush_trips_only_past_budget() {
        let mut c = UnifiedFrameCollector::new();
        assert!(!c.should_auto_flush());
        // Pushing a handful of small commands must stay below budget.
        for _ in 0..8 {
            c.push_canvas2d(1, Canvas2DCmd::Save);
        }
        assert!(
            !c.should_auto_flush(),
            "small frames must not trip the auto-flush signal"
        );
        // Manually push the counter past the threshold to verify
        // the predicate fires without allocating megabytes in the
        // test.
        c.pending_bytes = AUTO_FLUSH_SOFT_BUDGET_BYTES + 1;
        assert!(c.should_auto_flush());
    }

    #[test]
    fn flush_as_barrier_produces_non_presenting_packet() {
        let mut c = UnifiedFrameCollector::new();
        c.push_canvas2d(1, Canvas2DCmd::Save);
        c.push_gl(GLCmd::Clear {
            canvas_id: 1,
            bit_field: 0x4000,
        });

        let packet = c.flush_as_barrier().unwrap();
        let ops = packet.ops();
        // Should NOT have Present at the end
        assert!(!matches!(ops.last().unwrap(), FrameOp::Present));
        // But should still have Materialize
        assert!(
            ops.iter()
                .any(|op| matches!(op, FrameOp::Materialize { .. }))
        );
    }

    #[test]
    fn second_gl_segment_after_more_2d_gets_new_materialize() {
        let mut c = UnifiedFrameCollector::new();
        // 2D → GL → 2D → GL
        c.push_canvas2d(1, Canvas2DCmd::Save);
        c.push_gl(GLCmd::Clear {
            canvas_id: 1,
            bit_field: 0x4000,
        });
        c.push_canvas2d(1, Canvas2DCmd::Restore);
        c.push_gl(GLCmd::Clear {
            canvas_id: 1,
            bit_field: 0x4000,
        });

        let packet = c.build_frame_packet(true).unwrap();
        let ops = packet.ops();
        // [BeginFrame, CB(1), Mat(1), GB, CB(1), Mat(1), GB, Present]
        let mat_count = ops
            .iter()
            .filter(|op| matches!(op, FrameOp::Materialize { .. }))
            .count();
        assert_eq!(
            mat_count, 2,
            "each 2D→GL transition needs its own Materialize"
        );
    }

    #[test]
    fn consecutive_canvas2d_same_canvas_stay_in_one_segment() {
        let mut c = UnifiedFrameCollector::new();
        c.push_canvas2d(1, Canvas2DCmd::Save);
        c.push_canvas2d(1, Canvas2DCmd::Restore);
        c.push_canvas2d(
            1,
            Canvas2DCmd::FillRect {
                x: 0.0,
                y: 0.0,
                w: 1.0,
                h: 1.0,
            },
        );

        let packet = c.build_frame_packet(true).unwrap();
        let ops = packet.ops();
        // [BeginFrame, CanvasBatch(1, 3 cmds), Present]
        assert_eq!(ops.len(), 3);
        assert!(matches!(&ops[1], FrameOp::CanvasBatch(p) if p.commands.len() == 3));
    }

    #[test]
    fn consecutive_gl_ops_stay_in_one_segment() {
        let mut c = UnifiedFrameCollector::new();
        c.push_gl(GLCmd::Clear {
            canvas_id: 1,
            bit_field: 0x4000,
        });
        c.push_gl(GLCmd::Clear {
            canvas_id: 1,
            bit_field: 0x4100,
        });

        let packet = c.build_frame_packet(false).unwrap();
        let ops = packet.ops();
        // [BeginFrame, GlBatch(2 cmds)]
        assert_eq!(ops.len(), 2);
        assert!(matches!(&ops[1], FrameOp::GlBatch(p) if p.commands.len() == 2));
    }

    // ── Blocker 1: barrier must materialize trailing pending 2D ──

    #[test]
    fn barrier_materializes_trailing_pending_2d_for_sync_readback() {
        // Scenario: only 2D work pending, then sync GL readback (readPixels).
        // The barrier packet must include Materialize so the sync read sees 2D results.
        let mut c = UnifiedFrameCollector::new();
        c.push_canvas2d(
            1,
            Canvas2DCmd::FillRect {
                x: 0.0,
                y: 0.0,
                w: 10.0,
                h: 10.0,
            },
        );

        let packet = c.flush_as_barrier().unwrap();
        let ops = packet.ops();
        // Must have Materialize(1) AFTER the CanvasBatch
        let has_materialize = ops
            .iter()
            .any(|op| matches!(op, FrameOp::Materialize { canvas_id: 1 }));
        assert!(
            has_materialize,
            "barrier must materialize trailing pending 2D canvases; got: {ops:?}"
        );
    }

    #[test]
    fn barrier_materializes_multiple_trailing_2d_canvases() {
        let mut c = UnifiedFrameCollector::new();
        c.push_canvas2d(1, Canvas2DCmd::Save);
        c.push_canvas2d(2, Canvas2DCmd::Save);

        let packet = c.flush_as_barrier().unwrap();
        let ops = packet.ops();
        let mat_count = ops
            .iter()
            .filter(|op| matches!(op, FrameOp::Materialize { .. }))
            .count();
        assert_eq!(
            mat_count, 2,
            "barrier must materialize all pending 2D canvases"
        );
    }

    // ── Blocker 2: dirty_rect must be tracked per segment ──

    #[test]
    fn canvas2d_segment_tracks_dirty_rect_for_fill_rects() {
        let mut c = UnifiedFrameCollector::new();
        c.push_canvas2d(
            1,
            Canvas2DCmd::FillRect {
                x: 10.0,
                y: 20.0,
                w: 100.0,
                h: 50.0,
            },
        );
        c.push_canvas2d(
            1,
            Canvas2DCmd::ClearRect {
                x: 200.0,
                y: 30.0,
                w: 50.0,
                h: 60.0,
            },
        );

        let packet = c.build_frame_packet(true).unwrap();
        let ops = packet.ops();
        let dirty = match &ops[1] {
            FrameOp::CanvasBatch(p) => p.dirty_rect,
            _ => panic!("expected CanvasBatch"),
        };
        // Bounding box of (10,20,100,50) union (200,30,50,60) = (10,20,240,70)
        let d = dirty.expect("dirty_rect must be Some for safe draw commands");
        assert!((d.x - 10.0).abs() < 0.001);
        assert!((d.y - 20.0).abs() < 0.001);
        assert!((d.width - 240.0).abs() < 0.001);
        assert!((d.height - 70.0).abs() < 0.001);
    }

    #[test]
    fn canvas2d_segment_dirty_rect_none_for_state_only_commands() {
        let mut c = UnifiedFrameCollector::new();
        c.push_canvas2d(1, Canvas2DCmd::Save);
        c.push_canvas2d(1, Canvas2DCmd::Restore);

        let packet = c.build_frame_packet(true).unwrap();
        let ops = packet.ops();
        let dirty = match &ops[1] {
            FrameOp::CanvasBatch(p) => p.dirty_rect,
            _ => panic!("expected CanvasBatch"),
        };
        assert!(dirty.is_none(), "no draws = no dirty_rect");
    }

    #[test]
    fn canvas2d_draw_image_tracks_dirty_rect() {
        let mut c = UnifiedFrameCollector::new();
        c.push_canvas2d(
            1,
            Canvas2DCmd::DrawImage {
                image_id: 1,
                sx: 0.0,
                sy: 0.0,
                sw: 32.0,
                sh: 32.0,
                dx: 50.0,
                dy: 60.0,
                dw: 32.0,
                dh: 32.0,
            },
        );

        let packet = c.build_frame_packet(true).unwrap();
        let ops = packet.ops();
        let dirty = match &ops[1] {
            FrameOp::CanvasBatch(p) => p.dirty_rect,
            _ => panic!("expected CanvasBatch"),
        };
        let d = dirty.expect("DrawImage must mark dirty");
        assert!((d.x - 50.0).abs() < 0.001);
        assert!((d.y - 60.0).abs() < 0.001);
    }

    // ── Scissor hint poisoning tests ──

    #[test]
    fn fill_text_poisons_dirty_rect_to_none() {
        let mut c = UnifiedFrameCollector::new();
        // FillRect first — establishes a valid dirty_rect
        c.push_canvas2d(
            1,
            Canvas2DCmd::FillRect {
                x: 0.0,
                y: 0.0,
                w: 10.0,
                h: 10.0,
            },
        );
        // FillText — bounds unknown, must poison
        c.push_canvas2d(
            1,
            Canvas2DCmd::FillText {
                text: "hi".into(),
                x: 0.0,
                y: 0.0,
                max_width: f32::INFINITY,
            },
        );

        let packet = c.build_frame_packet(true).unwrap();
        let dirty = match &packet.ops()[1] {
            FrameOp::CanvasBatch(p) => p.dirty_rect,
            _ => panic!("expected CanvasBatch"),
        };
        assert!(dirty.is_none(), "FillText must poison dirty_rect to None");
    }

    #[test]
    fn path_fill_poisons_dirty_rect_to_none() {
        let mut c = UnifiedFrameCollector::new();
        c.push_canvas2d(
            1,
            Canvas2DCmd::FillRect {
                x: 0.0,
                y: 0.0,
                w: 10.0,
                h: 10.0,
            },
        );
        c.push_canvas2d(1, Canvas2DCmd::Fill);

        let packet = c.build_frame_packet(true).unwrap();
        let dirty = match &packet.ops()[1] {
            FrameOp::CanvasBatch(p) => p.dirty_rect,
            _ => panic!("expected CanvasBatch"),
        };
        assert!(dirty.is_none(), "path Fill must poison dirty_rect to None");
    }

    #[test]
    fn clip_poisons_dirty_rect_to_none() {
        let mut c = UnifiedFrameCollector::new();
        c.push_canvas2d(
            1,
            Canvas2DCmd::FillRect {
                x: 0.0,
                y: 0.0,
                w: 10.0,
                h: 10.0,
            },
        );
        c.push_canvas2d(1, Canvas2DCmd::Clip);

        let packet = c.build_frame_packet(true).unwrap();
        let dirty = match &packet.ops()[1] {
            FrameOp::CanvasBatch(p) => p.dirty_rect,
            _ => panic!("expected CanvasBatch"),
        };
        assert!(dirty.is_none(), "Clip must poison dirty_rect to None");
    }

    #[test]
    fn rect_after_poison_stays_none() {
        let mut c = UnifiedFrameCollector::new();
        c.push_canvas2d(1, Canvas2DCmd::Fill); // poison
        c.push_canvas2d(
            1,
            Canvas2DCmd::FillRect {
                x: 0.0,
                y: 0.0,
                w: 10.0,
                h: 10.0,
            },
        ); // should NOT recover

        let packet = c.build_frame_packet(true).unwrap();
        let dirty = match &packet.ops()[1] {
            FrameOp::CanvasBatch(p) => p.dirty_rect,
            _ => panic!("expected CanvasBatch"),
        };
        assert!(dirty.is_none(), "once poisoned, dirty_rect stays None");
    }

    #[test]
    fn stroke_rect_poisons_dirty_rect() {
        let mut c = UnifiedFrameCollector::new();
        c.push_canvas2d(
            1,
            Canvas2DCmd::StrokeRect {
                x: 10.0,
                y: 20.0,
                w: 100.0,
                h: 50.0,
            },
        );

        let packet = c.build_frame_packet(true).unwrap();
        let dirty = match &packet.ops()[1] {
            FrameOp::CanvasBatch(p) => p.dirty_rect,
            _ => panic!("expected CanvasBatch"),
        };
        assert!(
            dirty.is_none(),
            "StrokeRect must poison (lineWidth unknown JS-side)"
        );
    }

    #[test]
    fn set_transform_poisons_dirty_rect() {
        let mut c = UnifiedFrameCollector::new();
        c.push_canvas2d(
            1,
            Canvas2DCmd::SetTransform {
                a: 2.0,
                b: 0.0,
                c: 0.0,
                d: 2.0,
                e: 0.0,
                f: 0.0,
            },
        );
        c.push_canvas2d(
            1,
            Canvas2DCmd::FillRect {
                x: 0.0,
                y: 0.0,
                w: 10.0,
                h: 10.0,
            },
        );

        let packet = c.build_frame_packet(true).unwrap();
        let dirty = match &packet.ops()[1] {
            FrameOp::CanvasBatch(p) => p.dirty_rect,
            _ => panic!("expected CanvasBatch"),
        };
        assert!(dirty.is_none(), "SetTransform must poison scissor hint");
    }

    #[test]
    fn translate_poisons_dirty_rect() {
        let mut c = UnifiedFrameCollector::new();
        c.push_canvas2d(1, Canvas2DCmd::Translate { x: 10.0, y: 10.0 });
        c.push_canvas2d(
            1,
            Canvas2DCmd::FillRect {
                x: 0.0,
                y: 0.0,
                w: 10.0,
                h: 10.0,
            },
        );

        let packet = c.build_frame_packet(true).unwrap();
        let dirty = match &packet.ops()[1] {
            FrameOp::CanvasBatch(p) => p.dirty_rect,
            _ => panic!("expected CanvasBatch"),
        };
        assert!(dirty.is_none(), "Translate must poison scissor hint");
    }

    #[test]
    fn set_shadow_blur_poisons_dirty_rect() {
        let mut c = UnifiedFrameCollector::new();
        c.push_canvas2d(1, Canvas2DCmd::SetShadowBlur { blur: 5.0 });
        c.push_canvas2d(
            1,
            Canvas2DCmd::FillRect {
                x: 0.0,
                y: 0.0,
                w: 10.0,
                h: 10.0,
            },
        );

        let packet = c.build_frame_packet(true).unwrap();
        let dirty = match &packet.ops()[1] {
            FrameOp::CanvasBatch(p) => p.dirty_rect,
            _ => panic!("expected CanvasBatch"),
        };
        assert!(dirty.is_none(), "SetShadowBlur must poison scissor hint");
    }

    #[test]
    fn set_line_width_poisons_dirty_rect() {
        let mut c = UnifiedFrameCollector::new();
        c.push_canvas2d(1, Canvas2DCmd::SetLineWidth { width: 4.0 });
        c.push_canvas2d(
            1,
            Canvas2DCmd::FillRect {
                x: 0.0,
                y: 0.0,
                w: 10.0,
                h: 10.0,
            },
        );

        let packet = c.build_frame_packet(true).unwrap();
        let dirty = match &packet.ops()[1] {
            FrameOp::CanvasBatch(p) => p.dirty_rect,
            _ => panic!("expected CanvasBatch"),
        };
        assert!(dirty.is_none(), "SetLineWidth must poison scissor hint");
    }
}
