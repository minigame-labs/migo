//! V8 startup snapshot support for faster cold-start.
//!
//! # Overview
//!
//! V8 startup snapshots capture the heap state after all extension JS files
//! have been parsed, compiled and executed.  Loading from a snapshot skips
//! these steps entirely, reducing cold-start by ~150–300 ms on mid-range
//! Android devices (~230 KB of JS across 75 extension files).
//!
//! # How it works
//!
//! 1. **Build-time** — the `migo-snapshot-gen` binary creates a snapshot:
//!    ```text
//!    cargo run -p migo-snapshot-gen
//!    ```
//!    It calls [`lazy_extensions()`] to get extensions with JS but without
//!    runtime state, feeds them to `deno_core::create_snapshot()`, and writes
//!    the output to `SNAPSHOT.bin`.
//!
//! 2. **Compile-time** — `SNAPSHOT.bin` is embedded via `include_bytes!`.
//!    Release builds require the snapshot (compile error if missing).
//!    Debug builds allow fallback to JS source loading for faster iteration.
//!
//! 3. **Runtime** — `HostJsRuntime::new()` passes the snapshot bytes to
//!    `RuntimeOptions::startup_snapshot`.  Extensions are created via
//!    [`lazy_extensions()`] (same set, same order), and their state callbacks
//!    are applied afterwards via [`extension_args()`] +
//!    `JsRuntime::lazy_init_extensions()`.

use deno_core::{Extension, ExtensionArguments};
use shared::op_state::HostOpState;

use crate::{
    audio, base, console, device, event, file, input, media, network, rendering, storage, url,
    utility, web, worker,
};

/// Embedded snapshot bytes.
///
/// Currently disabled: the Android V8 is a custom termux-packages build
/// whose internal configuration (pointer compression, sandbox flags, etc.)
/// differs from the official denoland/rusty_v8 releases.  Since the
/// official releases don't include `aarch64-linux-android` targets,
/// cross-platform snapshot generation is not yet feasible.
///
/// When a compatible V8 build is available for both host and Android,
/// re-enable by using `include_bytes!("SNAPSHOT.bin")` gated behind
/// `migo_has_snapshot` cfg.
///
/// The snapshot generator (`migo-snapshot-gen`) and build infrastructure
/// (`build-snapshot.ps1`) are kept in place for future use.
pub static SNAPSHOT_BYTES: Option<&'static [u8]> = None;

/// Create all extensions in **lazy-init** mode (JS loaded, ops registered,
/// state callbacks deferred).
///
/// Used for both snapshot creation and snapshot-based runtime startup.
/// The order MUST match [`extension_args()`].
pub fn lazy_extensions() -> Vec<Extension> {
    let runtime_ext = vec![super::runtime::lazy_init()];

    vec![base::host_v8_base::lazy_init()]
        .into_iter()
        .chain(console::console_lazy_extensions())
        .chain(event::event_lazy_extensions())
        .chain(utility::utility_lazy_extensions())
        .chain(device::device_lazy_extensions())
        .chain(storage::storage_lazy_extensions())
        .chain(input::touch_lazy_extensions())
        .chain(file::file_lazy_extensions())
        .chain(rendering::rendering_lazy_extensions())
        .chain(web::web_lazy_extensions())
        .chain(url::url_lazy_extensions())
        .chain(network::network_lazy_extensions())
        .chain(media::media_lazy_extensions())
        .chain(audio::audio_lazy_extensions())
        .chain(worker::worker_lazy_extensions())
        .chain(runtime_ext)
        .collect()
}

/// Create [`ExtensionArguments`] with actual runtime state for all extensions.
///
/// Must be passed to `JsRuntime::lazy_init_extensions()` after snapshot
/// restoration.  The order MUST match [`lazy_extensions()`].
pub fn extension_args(host: HostOpState) -> Vec<ExtensionArguments> {
    vec![
        // host_v8_base — needs HostOpState
        base::host_v8_base::args(host),
        // host_v8_console — no state
        console::host_v8_console::args(),
        // host_v8_event — no state
        event::host_v8_event::args(),
        // host_v8_utility — no state
        utility::host_v8_utility::args(),
        // host_v8_device — no state
        device::host_v8_device::args(),
        // host_v8_storage — no state
        storage::host_v8_storage::args(),
        // host_v8_touch — no state
        input::host_v8_touch::args(),
        // host_v8_file — no state
        file::host_v8_file::args(),
        // host_v8_image — no state
        rendering::image::host_v8_image::args(),
        // host_v8_webgl — state (FrameCommandCollector, GlBatchCollector)
        rendering::webgl::host_v8_webgl::args(),
        // host_v8_web — state (StartTime, CanvasOpState)
        web::host_v8_web::args(),
        // host_v8_url — no state
        url::host_v8_url::args(),
        // host_v8_network — options (Default)
        network::network_extension_args(),
        // host_v8_media — no state
        media::host_v8_media::args(),
        // host_v8_audio — no state
        audio::host_v8_audio::args(),
        // host_v8_worker — no state
        worker::host_v8_worker::args(),
        // runtime — no state
        super::runtime::args(),
    ]
}
