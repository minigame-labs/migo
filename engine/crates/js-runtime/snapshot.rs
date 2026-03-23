//! V8 startup snapshot support for faster cold-start.
//!
//! # Overview
//!
//! V8 startup snapshots capture the heap state after all extension JS files
//! have been parsed, compiled and executed.  Loading from a snapshot skips
//! these steps entirely, reducing cold-start by ~150-300 ms on mid-range
//! Android devices (~230 KB of JS across 75 extension files).
//!
//! # How it works
//!
//! 1. **Build-time** -- the `migo-snapshot-gen` binary creates a snapshot:
//!    ```text
//!    cargo run -p migo-snapshot-gen
//!    ```
//!    It calls [`lazy_extensions()`] to get extensions with JS but without
//!    runtime state, feeds them to `deno_core::create_snapshot()`, and writes
//!    the output to `SNAPSHOT.bin`.
//!
//! 2. **Compile-time** -- `SNAPSHOT.bin` is embedded via `include_bytes!`.
//!    Release builds require the snapshot (compile error if missing).
//!    Debug builds allow fallback to JS source loading for faster iteration.
//!
//! 3. **Runtime** -- `HostJsRuntime::new()` passes the snapshot bytes to
//!    `RuntimeOptions::startup_snapshot`.  Extensions are created via
//!    [`lazy_extensions()`] (same set, same order), and their state callbacks
//!    are applied afterwards via [`extension_args()`] +
//!    `JsRuntime::lazy_init_extensions()`.

use deno_core::{Extension, ExtensionArguments};
use shared::op_state::HostOpState;

use crate::{base, console, event, file, input, network, rendering, storage, url, utility, web};

#[cfg(feature = "api-sensors")]
use crate::device;
#[cfg(feature = "api-media")]
use crate::{audio, media};
#[cfg(feature = "api-system")]
use crate::worker;

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
    let mut exts: Vec<Extension> = Vec::new();

    // CORE
    exts.push(base::host_v8_base::lazy_init());
    exts.extend(console::console_lazy_extensions());
    exts.extend(event::event_lazy_extensions());
    exts.extend(utility::utility_lazy_extensions());

    // OPTIONAL: api-sensors
    #[cfg(feature = "api-sensors")]
    exts.extend(device::device_lazy_extensions());

    // CORE (continued)
    exts.extend(storage::storage_lazy_extensions());
    exts.extend(input::touch_lazy_extensions());
    exts.extend(file::file_lazy_extensions());
    exts.extend(rendering::rendering_lazy_extensions());
    exts.extend(web::web_lazy_extensions());
    exts.extend(url::url_lazy_extensions());
    exts.extend(network::network_lazy_extensions());

    // OPTIONAL: api-media
    #[cfg(feature = "api-media")]
    exts.extend(media::media_lazy_extensions());
    #[cfg(feature = "api-media")]
    exts.extend(audio::audio_lazy_extensions());

    // OPTIONAL: api-system
    #[cfg(feature = "api-system")]
    exts.extend(worker::worker_lazy_extensions());

    // CORE: runtime (must be last)
    exts.push(super::runtime::lazy_init());

    exts
}

/// Create [`ExtensionArguments`] with actual runtime state for all extensions.
///
/// Must be passed to `JsRuntime::lazy_init_extensions()` after snapshot
/// restoration.  The order MUST match [`lazy_extensions()`].
pub fn extension_args(host: HostOpState) -> Vec<ExtensionArguments> {
    let mut args: Vec<ExtensionArguments> = Vec::new();

    // CORE
    args.push(base::host_v8_base::args(host));
    args.push(console::host_v8_console::args());
    args.push(event::host_v8_event::args());
    args.push(utility::host_v8_utility::args());

    // OPTIONAL: api-sensors
    #[cfg(feature = "api-sensors")]
    args.push(device::host_v8_device::args());

    // CORE (continued)
    args.push(storage::host_v8_storage::args());
    args.push(input::host_v8_touch::args());
    args.push(file::host_v8_file::args());
    args.push(rendering::image::host_v8_image::args());
    args.push(rendering::webgl::host_v8_webgl::args());
    args.push(web::host_v8_web::args());
    args.push(url::host_v8_url::args());
    args.push(network::network_extension_args());

    // OPTIONAL: api-media
    #[cfg(feature = "api-media")]
    args.push(media::host_v8_media::args());
    #[cfg(feature = "api-media")]
    args.push(audio::host_v8_audio::args());

    // OPTIONAL: api-system
    #[cfg(feature = "api-system")]
    args.push(worker::host_v8_worker::args());

    // CORE: runtime (must be last)
    args.push(super::runtime::args());

    args
}
