//! Backend abstraction layer for the Migo renderer.
//!
//! The rendering pipeline is organised into a pluggable `Backend` trait that
//! owns both the Canvas2D (Skia Ganesh GL) and WebGL (glow + StateTracker)
//! paths.  Today only the GL backend exists; the trait keeps the door open
//! for a future Vulkan/Graphite backend without disturbing callers.
//!
//! # Layering
//!
//! ```text
//! RenderThread  ─────────── drives frame lifecycle
//!     │
//!     │ FramePacket (Canvas2DCmd + GLCmd)
//!     ▼
//! Backend (trait)
//!     │
//!     ├── gl::SkiaCanvasBackend   — Canvas2D → SkCanvas
//!     └── gl::WebGlBackend        — GLCmd    → glow + StateTracker
//! ```
//!
//! Both sub-backends share the same `CanvasManager` (EGL contexts, onscreen
//! DrawingBuffer FBO, image registry) so 2D and 3D draw into the same
//! framebuffer without intermediate blits.

pub mod gl;
