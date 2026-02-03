//! # JavaScript Runtime Module
//!
//! This crate provides the JavaScript execution environment for the Migo engine,
//! implementing Mini Program and standard Web APIs on top of Deno Core (V8).
//!
//! ## Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────────┐
//! │                     JavaScript Runtime (V8)                          │
//! │                                                                      │
//! │  ┌────────────────────────────────────────────────────────────────┐ │
//! │  │                      Global Scope                              │ │
//! │  │  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐            │ │
//! │  │  │     wx      │  │   console   │  │   canvas    │    ...     │ │
//! │  │  │  (Mini App) │  │   (logging) │  │  (2D/WebGL) │            │ │
//! │  │  └──────┬──────┘  └──────┬──────┘  └──────┬──────┘            │ │
//! │  └─────────│────────────────│────────────────│────────────────────┘ │
//! │            │                │                │                       │
//! │  ┌─────────▼────────────────▼────────────────▼────────────────────┐ │
//! │  │                   Native Ops (Rust)                            │ │
//! │  │  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐            │ │
//! │  │  │    file     │  │   network   │  │    audio    │    ...     │ │
//! │  │  │   (fs ops)  │  │ (fetch/ws)  │  │  (WebAudio) │            │ │
//! │  │  └─────────────┘  └─────────────┘  └─────────────┘            │ │
//! │  └────────────────────────────────────────────────────────────────┘ │
//! │                                                                      │
//! │  ┌────────────────────────────────────────────────────────────────┐ │
//! │  │                       Deno Core                                │ │
//! │  │  - Module loading (ESM)                                        │ │
//! │  │  - Op dispatch (sync/async)                                    │ │
//! │  │  - Event loop integration                                      │ │
//! │  └────────────────────────────────────────────────────────────────┘ │
//! └─────────────────────────────────────────────────────────────────────┘
//! ```
//!
//! ## Implemented APIs
//!
//! - **File System**: `migo.getFileSystemManager()` - read, write, stat, mkdir, etc.
//! - **Storage**: `migo.setStorageSync()`, `migo.getStorageSync()`
//! - **Audio**: `migo.createInnerAudioContext()` - streaming audio playback
//! - **Canvas**: `migo.createCanvas()`, `migo.createOffscreenCanvas()`
//! - **Image**: `migo.createImage()` - async image loading
//! - **System**: `migo.getSystemInfoSync()`, `migo.getWindowInfo()`
//! - **Lifecycle**: `migo.onShow()`, `migo.onHide()`
//!
//! ### Web Standard APIs
//!
//! - **Console**: `console.log()`, `console.warn()`, `console.error()`, etc.
//! - **Timers**: `setTimeout()`, `setInterval()`, `requestAnimationFrame()`
//! - **Network**: `fetch()` with streaming support
//! - **URL**: `URL`, `URLSearchParams`
//! - **Encoding**: `TextEncoder`, `TextDecoder`
//! - **Canvas 2D**: Full CanvasRenderingContext2D API
//! - **WebGL**: WebGLRenderingContext (WebGL 1.0)
//! - **WebAudio**: AudioContext, AudioBufferSourceNode, GainNode
//!
//! ## Extension System
//!
//! The runtime is built from modular extensions:
//!
//! | Extension | Purpose |
//! |-----------|---------|
//! | `host_v8_base` | Core ops, timers, system info |
//! | `host_v8_console` | Console logging |
//! | `host_v8_file` | File system operations |
//! | `host_v8_network` | HTTP fetch, WebSocket |
//! | `host_v8_rendering` | Canvas, WebGL, Image |
//! | `host_v8_audio` | WebAudio, InnerAudioContext |
//! | `host_v8_input` | Touch event dispatch |
//!
//! ## Usage
//!
//! The js-runtime is created by the core module and not directly instantiated:
//!
//! ```rust,ignore
//! use js_runtime::{HostJsRuntime, main_extensions};
//!
//! let extensions = main_extensions(host_state);
//! let mut runtime = HostJsRuntime::new(host_id, host_state, extensions, module_loader);
//!
//! // Load and run a game
//! runtime.evaluate_module("/game".into(), "main.js".into()).await?;
//!
//! // Run the event loop
//! runtime.run_event_loop(PollEventLoopOptions::default()).await?;
//! ```

use shared::op_state::HostOpState;

mod audio;
mod base;
mod console;
mod device;
mod event;
mod file;
mod input;
mod network;
mod rendering;
mod url;
mod utility;
mod web;

mod host_runtime;
mod js_bindings;

pub use host_runtime::HostJsRuntime;

deno_core::extension!(
    runtime,
    esm_entry_point = "ext:runtime/99_main.js",
    esm = [
        dir "",
        "98_global_scope_shared.js",
        "98_global_scope_window.js",
        "99_main.js",
    ],
);

/// Creates all JavaScript runtime extensions with the given host state.
///
/// This assembles the complete set of extensions needed for the Migo runtime,
/// including both Mini Program APIs and Web standard APIs.
///
/// # Arguments
///
/// * `host` - Shared operational state containing channels to other services
///
/// # Returns
///
/// A vector of Deno extensions, ordered to respect dependencies.
///
/// # Extension Order
///
/// Extensions are chained in dependency order:
/// 1. `base` - Core ops (must be first)
/// 2. `console` - Depends on base
/// 3. `utility` - Encoding utilities
/// 4. `input` - Touch handling
/// 5. `file` - File system
/// 6. `rendering` - Canvas/WebGL
/// 7. `web` - Timers, performance
/// 8. `url` - URL parsing
/// 9. `network` - Fetch API
/// 10. `audio` - WebAudio/InnerAudio
/// 11. `runtime` - Entry point script
pub fn main_extensions(host: HostOpState) -> Vec<deno_core::Extension> {
    let runtime_extensions = vec![runtime::init_ops_and_esm()];

    base::base_extensions(host)
        .into_iter()
        .chain(console::console_extensions())
        .chain(event::event_extensions())
        .chain(utility::utility_extensions())
        .chain(device::device_extensions())
        .chain(input::touch_extensions())
        .chain(file::file_extensions())
        .chain(rendering::rendering_extensions())
        .chain(web::web_extensions())
        .chain(url::url_extensions())
        .chain(network::network_extensions())
        .chain(audio::audio_extensions())
        .chain(runtime_extensions)
        .collect()
}
