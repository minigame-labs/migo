//! # Graphics Rendering Module
//!
//! Canvas2D + WebGL rendering for the Migo engine.
//!
//! ## Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────────┐
//! │                           JS Thread                                  │
//! │                                                                      │
//! │   RAF → ctx.fillRect() → ctx.drawImage() → ... → RAF ends           │
//! │         UnifiedFrameCollector batches all commands per frame        │
//! └────────────────────────────────────────────┬─────────────────────────┘
//!                                              │
//!                          FramePacket (single IPC per frame)
//!                                              │
//!                                              ▼
//! ┌─────────────────────────────────────────────────────────────────────┐
//! │                         Render Thread                                │
//! │                                                                      │
//! │   RenderThread receives Canvas2DBatch / GL / Canvas commands        │
//! │   → backend::gl (Skia Canvas2D via Ganesh GL)                       │
//! │   → renderergl (WebGL via glow)                                     │
//! │   → EGL context management via canvas module                        │
//! └─────────────────────────────────────────────────────────────────────┘
//! ```
//!
//! ## Module Structure
//!
//! - [`render_thread`]: Render thread loop and command dispatch.
//! - [`canvas`]: `CanvasManager` (EGL contexts, DrawingBuffer FBO, image
//!   registry).
//! - [`backend`]: Pluggable rendering backends.  Currently only
//!   `backend::gl` (Skia Ganesh on GL ES 3.0) is implemented.
//! - [`renderergl`]: WebGL 1.0 / 2.0 command handler (glow-backed).

#[cfg(target_os = "android")]
pub(crate) mod ahb;
pub mod atlas;
#[doc(hidden)]
pub mod backend;
mod canvas;
mod canvas2d_dispatcher;
pub mod compressed_upload;
pub(crate) mod damage_effect;
pub mod device_caps;
pub mod device_profile;
pub mod dirty_region;
pub mod frame_scheduler;
mod legacy_frame_bridge;
mod render_server;
mod render_thread;
mod renderergl;
pub(crate) mod shader_cache;
pub mod surface_system;
pub mod texture_import;
pub(crate) mod upload_server;
pub mod upload_thread;

pub(crate) use canvas::*;
pub(crate) use legacy_frame_bridge::LegacyFrameBridge;
pub use render_server::RenderServer;
pub use render_thread::*;
pub use surface_system::{SurfaceLifecycleState, SurfaceSystem};

pub(crate) use renderergl::*;

use raw_window_handle::RawWindowHandle;
use shared::error::{EngineResult, ErrorCode};
use shared::surface::Surface;

/// Tracks which EGL context is currently bound.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(crate) enum BoundContext {
    /// Shared resource context (texture uploads, shader compilation).
    Resource,
    /// A specific canvas's context.
    Canvas(shared::protocol::render_cmd::CanvasId),
}

/// Extract the native window handle from a platform-agnostic Surface.
pub(crate) fn onscreen_window_from_surface(surface: &dyn Surface) -> EngineResult<usize> {
    match surface.raw_window_handle() {
        RawWindowHandle::AndroidNdk(h) => Ok(h.a_native_window.as_ptr() as usize),
        other => {
            shared::bail!(
                ErrorCode::Unsupported,
                "unsupported RawWindowHandle for current backend",
                format!("{:?}", other)
            );
        }
    }
}

#[cfg(test)]
#[test]
fn defers_jobs_when_budget_is_exhausted() {
    upload_server::assert_defers_jobs_when_budget_is_exhausted();
}

#[cfg(test)]
#[test]
fn bridge_flushes_pending_ops_without_packet_level_present_boundary() {
    legacy_frame_bridge::assert_bridge_flushes_pending_ops_without_packet_level_present_boundary();
}

#[cfg(test)]
#[test]
fn hits_60fps_on_90hz_without_jittering_to_45fps() {
    frame_scheduler::assert_hits_60fps_on_90hz_without_jittering_to_45fps();
}

#[cfg(test)]
#[test]
fn session_relative_time_starts_from_first_vsync() {
    frame_scheduler::assert_session_relative_time_starts_from_first_vsync();
}

#[cfg(test)]
#[test]
fn does_not_present_when_surface_is_not_ready() {
    frame_scheduler::assert_does_not_present_when_surface_is_not_ready();
}

#[cfg(test)]
#[test]
fn gl_only_packet_triggers_present_when_batch_hits_onscreen() {
    use shared::FramePacket;

    let packet = FramePacket::for_gl_batch(0, 0.0, Vec::new());

    let mut hit_count = 0u32;
    let should_present = render_thread::execute_frame_packet_with_present_tracking(
        packet,
        &mut hit_count,
        |_state, _payload| -> bool {
            panic!("should not receive canvas batch in GL-only packet");
        },
        |state, _payload| -> bool {
            *state += 1;
            true
        },
    );

    assert!(should_present);
    assert_eq!(hit_count, 1);
}

#[cfg(test)]
#[test]
fn gl_only_packet_no_present_when_batch_is_offscreen() {
    use shared::FramePacket;

    let packet = FramePacket::for_gl_batch(0, 0.0, Vec::new());

    let should_present = render_thread::execute_frame_packet_with_present_tracking(
        packet,
        &mut (),
        |_, _| -> bool { panic!("no canvas batch expected") },
        |_, _| -> bool { false },
    );

    assert!(!should_present);
}

#[cfg(test)]
#[test]
fn mixed_canvas2d_and_gl_packet_unions_present_signals() {
    use shared::protocol::render_cmd::{Canvas2DCmd, CanvasBatchPayload};
    use shared::{FrameOp, FramePacketBuilder};

    let packet = FramePacketBuilder::new(0, 0.0)
        .push(FrameOp::BeginFrame)
        .push(FrameOp::CanvasBatch(CanvasBatchPayload {
            canvas_id: 1,
            commands: vec![Canvas2DCmd::Save],
            present: true,
            dirty_rect: None,
        }))
        .push(FrameOp::GlBatch(
            shared::protocol::render_cmd::GlBatchPayload {
                commands: Vec::new(),
            },
        ))
        .finish();

    let should_present = render_thread::execute_frame_packet_with_present_tracking(
        packet,
        &mut (0u32, 0u32),
        |state, _| -> bool {
            state.0 += 1;
            true
        },
        |state, _| -> bool {
            state.1 += 1;
            true
        },
    );

    assert!(should_present);
}
