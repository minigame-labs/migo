//! # Graphics Rendering Module
//!
//! This crate provides 2D and WebGL rendering capabilities for the Migo engine,
//! implementing Canvas 2D and WebGL 1.0 APIs.
//!
//! ## Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────────┐
//! │                           JS Thread                                  │
//! │                                                                      │
//! │   RAF → ctx.fillRect() → ctx.drawImage() → ... → RAF ends          │
//! │         UnifiedFrameCollector batches all commands per frame         │
//! │                                            │                         │
//! └────────────────────────────────────────────┼─────────────────────────┘
//!                                              │
//!                            FramePacket (single IPC per frame)
//!                                              │
//!                                              ▼
//! ┌─────────────────────────────────────────────────────────────────────┐
//! │                         Render Thread                                │
//! │                                                                      │
//! │   RenderThread receives Canvas2DBatch / GL / Canvas commands         │
//! │   → renderer2d (femtovg) for 2D                                     │
//! │   → renderergl (glow) for WebGL                                     │
//! │   → EGL context management via canvas module                        │
//! └─────────────────────────────────────────────────────────────────────┘
//! ```
//!
//! ## Module Structure
//!
//! - [`render_thread`]: Render thread loop and command dispatch
//! - [`canvas`]: Canvas and EGL context management
//! - [`renderer2d`]: Canvas 2D rendering via femtovg
//! - [`renderergl`]: WebGL command handler via glow

#[cfg(target_os = "android")]
pub(crate) mod ahb;
pub mod atlas;
mod canvas;
pub mod compressed_upload;
pub(crate) mod damage_effect;
pub mod device_caps;
pub mod device_profile;
pub mod dirty_region;
pub mod frame_scheduler;
mod legacy_frame_bridge;
mod render_server;
mod render_thread;
mod renderer2d;
mod renderergl;
pub(crate) mod shader_cache;
pub mod surface_system;
pub mod texture_import;
pub(crate) mod upload_server;
pub mod upload_thread;

pub mod gpu_canvas2d;
pub mod webgpu;

pub(crate) use canvas::*;
pub(crate) use legacy_frame_bridge::LegacyFrameBridge;
pub use render_server::RenderServer;
pub use render_thread::*;
pub use surface_system::{SurfaceLifecycleState, SurfaceSystem};

pub(crate) use renderer2d::*;
pub(crate) use renderergl::*;

use raw_window_handle::RawWindowHandle;
use shared::error::{EngineResult, ErrorCode};
use shared::surface::Surface;

/// Tracks which EGL context is currently bound.
///
/// Used to avoid redundant `eglMakeCurrent` calls and ensure proper
/// context switching when rendering to multiple canvases.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(crate) enum BoundContext {
    /// The shared resource context is bound (for loading textures, compiling shaders).
    Resource,
    /// A specific canvas context is bound for rendering.
    Canvas(shared::protocol::render_cmd::CanvasId),
}

/// Convert a platform-agnostic Surface into the onscreen "window" handle.
///
/// This extracts the native window handle from the platform abstraction,
/// which is required by EGL to create a window surface.
///
/// # Platform Details
///
/// - **Android**: Returns `ANativeWindow*` as `usize`
/// - **Other platforms**: Not yet supported
///
/// # Errors
///
/// Returns `ErrorCode::Unsupported` if the window handle type is not
/// supported by the current graphics backend.
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

/// WebGL-only FramePacket via for_gl_batch triggers should_present
/// when the GL batch handler reports onscreen activity (returns true).
#[cfg(test)]
#[test]
fn gl_only_packet_triggers_present_when_batch_hits_onscreen() {
    use shared::protocol::render_cmd::GlBatchPayload;
    use shared::FramePacket;

    let packet = FramePacket::for_gl_batch(0, 0.0, Vec::new());

    // Simulate: the GL batch handler returns true (hit onscreen).
    let mut hit_count = 0u32;
    let should_present = render_thread::execute_frame_packet_with_present_tracking(
        packet,
        &mut hit_count,
        |_state, _payload| -> bool {
            panic!("should not receive canvas batch in GL-only packet");
        },
        |state, _payload| -> bool {
            *state += 1;
            true // simulate onscreen GL rendering
        },
    );

    assert!(should_present);
    assert_eq!(hit_count, 1);
}

/// WebGL-only FramePacket that doesn't hit onscreen does NOT trigger present.
#[cfg(test)]
#[test]
fn gl_only_packet_no_present_when_batch_is_offscreen() {
    use shared::FramePacket;

    let packet = FramePacket::for_gl_batch(0, 0.0, Vec::new());

    let should_present = render_thread::execute_frame_packet_with_present_tracking(
        packet,
        &mut (),
        |_, _| -> bool { panic!("no canvas batch expected") },
        |_, _| -> bool { false }, // offscreen GL only
    );

    assert!(!should_present);
}

/// Mixed packet (Canvas2D + GL) via builder — both batches contribute to present.
#[cfg(test)]
#[test]
fn mixed_canvas2d_and_gl_packet_unions_present_signals() {
    use shared::protocol::render_cmd::{Canvas2DCmd, CanvasBatchPayload, GlBatchPayload};
    use shared::{FrameOp, FramePacketBuilder};

    let packet = FramePacketBuilder::new(0, 0.0)
        .push(FrameOp::BeginFrame)
        .push(FrameOp::CanvasBatch(CanvasBatchPayload {
            canvas_id: 1,
            commands: vec![Canvas2DCmd::Save],
            present: true,
            dirty_rect: None,
        }))
        .push(FrameOp::GlBatch(GlBatchPayload {
            commands: Vec::new(),
        }))
        .finish();

    let should_present = render_thread::execute_frame_packet_with_present_tracking(
        packet,
        &mut (0u32, 0u32),
        |state, _| -> bool {
            state.0 += 1;
            true // Canvas2D hit onscreen
        },
        |state, _| -> bool {
            state.1 += 1;
            true // GL hit onscreen
        },
    );

    assert!(should_present);
}
