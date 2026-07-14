//! # Core Runtime Module
//!
//! The central orchestration module of the Migo engine, responsible for:
//!
//! - Spawning and managing the host thread (JS runtime)
//! - Coordinating services (render, audio, I/O)
//! - Platform abstraction via [`PlatformServices`]
//!
//! ## Architecture Overview
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────────┐
//! │                           Platform Layer                             │
//! │  ┌─────────────────────────────────────────────────────────────────┐│
//! │  │                        MiniGameSDK (Java)                       ││
//! │  │  - Surface management                                           ││
//! │  │  - Touch event handling                                         ││
//! │  │  - Lifecycle events (show/hide)                                 ││
//! │  └────────────────────────────┬────────────────────────────────────┘│
//! │                               │ JNI                                  │
//! │  ┌────────────────────────────▼────────────────────────────────────┐│
//! │  │                    PlatformServices (Rust)                      ││
//! │  │  - Platform-specific extensions                                 ││
//! │  │  - System settings                                              ││
//! │  └────────────────────────────┬────────────────────────────────────┘│
//! └───────────────────────────────│─────────────────────────────────────┘
//!                                 │
//! ┌───────────────────────────────▼─────────────────────────────────────┐
//! │                           Core Runtime                               │
//! │  ┌─────────────────────────────────────────────────────────────────┐│
//! │  │                         Host Thread                             ││
//! │  │  ┌───────────────┐  ┌───────────────┐  ┌───────────────┐      ││
//! │  │  │  JS Runtime   │  │ HostOpState   │  │  JsBindings   │      ││
//! │  │  │  (Deno Core)  │  │  (channels)   │  │ (V8 globals)  │      ││
//! │  │  └───────────────┘  └───────────────┘  └───────────────┘      ││
//! │  └────────────────────────────┬────────────────────────────────────┘│
//! │                               │ Commands                             │
//! │  ┌────────────────────────────┴────────────────────────────────────┐│
//! │  │                          Services                               ││
//! │  │  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐             ││
//! │  │  │   Render    │  │    Audio    │  │     I/O     │             ││
//! │  │  │   Service   │  │   Service   │  │   Service   │             ││
//! │  │  └──────┬──────┘  └──────┬──────┘  └──────┬──────┘             ││
//! │  │         │                │                │                     ││
//! │  │  ┌──────▼──────┐  ┌──────▼──────┐  ┌──────▼──────┐             ││
//! │  │  │   Render    │  │    Audio    │  │  I/O Task   │             ││
//! │  │  │   Thread    │  │    Thread   │  │(Host tokio) │             ││
//! │  │  └─────────────┘  └─────────────┘  └─────────────┘             ││
//! │  └─────────────────────────────────────────────────────────────────┘│
//! └─────────────────────────────────────────────────────────────────────┘
//! ```
//!
//! ## Usage
//!
//! The core module is the main entry point for native code:
//!
//! ```rust,ignore
//! use core::{spawn_host_thread, send_command_to_host, PlatformServices};
//! use shared::protocol::host_cmd::HostCommand;
//!
//! // 1. Spawn the host thread
//! let host_id = spawn_host_thread(surface, platform, init_options)?;
//!
//! // 2. Send commands to run a game
//! send_command_to_host(host_id, HostCommand::EvaluateModule {
//!     dir: "/data/game".into(),
//!     entry: "main.js".into(),
//! })?;
//!
//! // 3. Forward platform events
//! send_command_to_host(host_id, HostCommand::OnTouch(Box::new(...)))?;
//!
//! // 4. Shutdown when done
//! shutdown_host(host_id)?;
//! ```
//!
//! ## Thread Model
//!
//! The engine uses a multi-threaded architecture:
//!
//! | Thread / Task | Responsibility |
//! |--------------|---------------|
//! | **Host** | JS runtime, event loop, command dispatch, I/O dispatch (tokio task) |
//! | **Render** | OpenGL/Canvas2D rendering, vsync |
//! | **Audio** | Audio decoding, mixing, output |
//! | **Process I/O executor** | Bounded file ops, image decode, archive work (`Migo-IO-*`) |
//! | **Tokio blocking fallback** | Lazy `tokio::fs` and resolver compatibility work |
//!
//! The host event loop shares one Tokio epoll fd and timer wheel. Heavy engine
//! I/O is routed through the process-wide bounded I/O executor; the host's
//! small Tokio blocking pool is created only if a remaining `tokio::fs` or
//! resolver compatibility path needs it.
//!
//! All threads/tasks communicate via typed channels, ensuring thread safety
//! without shared mutable state.
//!
//! ## Module Structure
//!
//! - [`runtime`]: Host thread lifecycle and registry
//! - [`services`]: Service abstractions and platform interface

mod runtime;
pub mod services;

pub use runtime::vsync::send_vsync;
pub use runtime::{
    bump_destroy_epoch, current_destroy_epoch, send_command_to_host, send_critical_command_to_host,
    shutdown_host, spawn_host_thread,
};
pub use services::PlatformServices;
#[cfg(all(feature = "profile-full", feature = "profile-slim"))]
compile_error!("profile-full and profile-slim are mutually exclusive");
#[cfg(all(feature = "worker-snapshot", not(feature = "profile-full")))]
compile_error!("worker-snapshot requires profile-full");
#[cfg(all(
    feature = "profile-slim",
    any(
        feature = "api-sensors",
        feature = "api-media",
        feature = "api-connectivity",
        feature = "api-commerce",
        feature = "api-system"
    )
))]
compile_error!("profile-slim is exact and cannot be combined with optional API groups");
