//! Render-thread → host event feedback channel.
//!
//! The render thread runs autonomously on its own OS thread and historically
//! only communicated failures through `tracing::error!()` — JS callers never
//! learned about `swap_buffers` failures, EGL context loss, RAF backpressure,
//! or command errors.  Real-world symptoms (a black screen, a hung UI) were
//! indistinguishable from each other because the diagnostic never crossed the
//! thread boundary.
//!
//! This module exposes a small structured event type and a one-to-many
//! broadcast-ish channel (bounded crossbeam MPMC) that the render thread
//! pushes events onto and any number of host-side consumers can drain.  The
//! channel is intentionally unidirectional and lossy-by-bound: if consumers
//! fall behind, the render thread never stalls — it drops the oldest event
//! and bumps a counter (see [`RenderEventSender::dropped`]).
//!
//! The event shape is kept intentionally small (no large payloads) so that
//! forwarding into JS can be a pure data copy instead of an allocation.
//! The consumer (typically the `core` runtime) translates the variant into
//! a JS-visible error event or a `performance.mark` entry as it sees fit.
//!
//! Audit rationale: P1-1 "Render 线程 RenderEvent 回流 channel" and P2-10
//! "raf 背压连续 3 帧触发 RenderEvent".

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crossbeam_channel::{Receiver, Sender, TrySendError, bounded};

use crate::error::ErrorCode;

/// Maximum pending events the render thread can buffer.  A small
/// number is fine because events are advisory — dropping an older
/// one only costs telemetry detail.  The bound also guarantees the
/// render thread's [`RenderEventSender::emit`] path is
/// constant-time: either the slot is free and we push, or we drop
/// the oldest in place.
const EVENT_CHANNEL_CAPACITY: usize = 64;

/// Structured render-thread event delivered to the host.
///
/// Intentionally `#[non_exhaustive]` — producers may add variants
/// without a semver break; consumers must always have a `_` arm.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub enum RenderEvent {
    /// A `Canvas2DBatch` / single `Canvas2D` command failed on
    /// the render thread after the dispatcher swallowed it.
    /// Carries the engine error code so JS can react
    /// selectively (e.g. re-raise only on `RenderBackendError`).
    Canvas2DError { code: ErrorCode, message: String },
    /// A `GL` / `GLBatch` command failed.
    GlError { code: ErrorCode, message: String },
    /// A `CanvasCmd` (create / destroy / resize / recreate)
    /// failed.  Usually downstream of an EGL / surface issue.
    CanvasError { code: ErrorCode, message: String },
    /// `eglSwapBuffers` returned an error that is **not**
    /// `EGL_CONTEXT_LOST`.  Recoverable by the render loop on
    /// its own; included in the channel so overlays / tests can
    /// observe the rate.
    SwapFailed { message: String },
    /// `EGL_CONTEXT_LOST` seen during swap.  A recovery attempt
    /// will be made at the top of the next frame loop.
    ContextLost,
    /// An EGL context recovery attempt completed with the given
    /// outcome.  Emitted exactly once per attempt.
    ContextRecovered { success: bool },
    /// RAF delivery has been dropped for ≥ 3 consecutive frames.
    /// Signals to the JS scheduler that the host event loop is
    /// saturated and probably should not issue more work until
    /// the overlay clears.
    RafBackpressure { consecutive_drops: u32 },
}

impl RenderEvent {
    /// Short identifier used in string logging; kept stable for
    /// tests asserting event shapes without mocking variants.
    pub fn kind(&self) -> &'static str {
        match self {
            RenderEvent::Canvas2DError { .. } => "canvas2d_error",
            RenderEvent::GlError { .. } => "gl_error",
            RenderEvent::CanvasError { .. } => "canvas_error",
            RenderEvent::SwapFailed { .. } => "swap_failed",
            RenderEvent::ContextLost => "context_lost",
            RenderEvent::ContextRecovered { .. } => "context_recovered",
            RenderEvent::RafBackpressure { .. } => "raf_backpressure",
        }
    }
}

/// Sender half of the render event channel.  Cheap to `Clone`; the
/// render thread holds the canonical instance and each `emit` call
/// is O(1) even when the consumer is backed up.
#[derive(Clone)]
pub struct RenderEventSender {
    tx: Sender<RenderEvent>,
    /// Counter of events that were dropped because the channel
    /// was full at emit time.  Useful to distinguish "no events"
    /// from "consumer drowning".
    dropped: Arc<AtomicU64>,
}

impl RenderEventSender {
    /// Push an event.  On full channel, drops the NEW event and
    /// bumps the internal counter — the consumer keeps seeing the
    /// older (still more informative) events.  Returning `()`
    /// keeps call sites terse and signals that the send is
    /// advisory.
    pub fn emit(&self, ev: RenderEvent) {
        match self.tx.try_send(ev) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                self.dropped.fetch_add(1, Ordering::Relaxed);
            }
            Err(TrySendError::Disconnected(_)) => {
                // Receiver went away.  Record the drop but
                // otherwise ignore — the render thread outlives
                // the last consumer during shutdown.
                self.dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// Cumulative number of events that were dropped because the
    /// consumer was not reading fast enough.  Monotonic.
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

/// Receiver half.  `Clone`able so multiple subscribers (e.g. JS
/// runtime + Java overlay bridge) can drain concurrently — events
/// are delivered to whichever consumer calls `try_recv` first, so
/// the intended deployment is one consumer per engine host.  The
/// design deliberately picks MPMC-crossbeam semantics over proper
/// multi-subscribe broadcast to keep the backend dependency
/// surface small; upstream projects that actually need fan-out can
/// layer a broadcast adapter on top.
pub type RenderEventReceiver = Receiver<RenderEvent>;

/// Construct a new `(sender, receiver)` pair.  One pair is created
/// per `RenderThread` at spawn time.
pub fn channel() -> (RenderEventSender, RenderEventReceiver) {
    let (tx, rx) = bounded(EVENT_CHANNEL_CAPACITY);
    (
        RenderEventSender {
            tx,
            dropped: Arc::new(AtomicU64::new(0)),
        },
        rx,
    )
}
