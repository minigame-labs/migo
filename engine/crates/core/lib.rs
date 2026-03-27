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
//! | **Blocking pool** | File system ops, image decode, zip extract (`spawn_blocking`) |
//!
//! The IO handler runs as a `tokio::spawn` task on the Host runtime, sharing
//! the same epoll fd, timer wheel, and blocking thread pool.  Heavy I/O work
//! is offloaded to the blocking pool via `tokio::fs` / `spawn_blocking`.
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
pub use runtime::{send_command_to_host, shutdown_host, spawn_host_thread};
pub use services::PlatformServices;
