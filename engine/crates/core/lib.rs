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
//! │  │  │   Render    │  │    Audio    │  │     I/O     │             ││
//! │  │  │   Thread    │  │    Thread   │  │   (Tokio)   │             ││
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
//! send_command_to_host(host_id, HostCommand::OnTouch { ... })?;
//!
//! // 4. Shutdown when done
//! shutdown_host(host_id)?;
//! ```
//!
//! ## Thread Model
//!
//! The engine uses a multi-threaded architecture:
//!
//! | Thread | Responsibility |
//! |--------|---------------|
//! | **Host** | JS runtime, event loop, command dispatch |
//! | **Render** | OpenGL/Canvas2D rendering, vsync |
//! | **Audio** | Audio decoding, mixing, output |
//! | **I/O** | File system, network (via Tokio) |
//!
//! All threads communicate via typed channels, ensuring thread safety
//! without shared mutable state.
//!
//! ## Module Structure
//!
//! - [`runtime`]: Host thread lifecycle and registry
//! - [`services`]: Service abstractions and platform interface

mod runtime;
mod services;

pub use runtime::{send_command_to_host, shutdown_host, spawn_host_thread};
pub use services::PlatformServices;
