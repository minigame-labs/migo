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
//! 1. **Build-time** -- the `migo-snapshot-gen` binary creates a snapshot.
//!    It calls [`lazy_extensions()`] to get extensions with JS but without
//!    runtime state, feeds them to `deno_core::create_snapshot()`, and writes
//!    the output to `snapshots/SNAPSHOT-<arch>.bin`. Because snapshots are
//!    platform-bound, the generator is cross-compiled to each Android ABI and
//!    run on that ABI's emulator/device (see `crates/snapshot-gen`).
//!
//! 2. **Compile-time** -- for android targets, `build.rs` picks
//!    `snapshots/SNAPSHOT-<target arch>.bin` and embeds it via `include_bytes!`.
//!    Missing snapshot or host builds fall back to JS source loading.
//!
//! 3. **Runtime** -- `HostJsRuntime::new()` passes the snapshot bytes to
//!    `RuntimeOptions::startup_snapshot`.  Extensions are created via
//!    [`lazy_extensions()`] (same set, same order), and their state callbacks
//!    are applied afterwards via [`extension_args()`] +
//!    `JsRuntime::lazy_init_extensions()`.

use deno_core::{Extension, ExtensionArguments};
use shared::op_state::HostOpState;

use crate::{
    base, console, env, event, file, input, io_state, lifecycle, network, rendering, storage, url,
    utility, web,
};

#[cfg(feature = "api-sensors")]
use crate::device;
#[cfg(feature = "api-connectivity")]
use crate::system;
#[cfg(feature = "api-system")]
use crate::worker;
#[cfg(feature = "api-system")]
use crate::{ad, ui, update};
#[cfg(feature = "api-media")]
use crate::{audio, media};
#[cfg(feature = "api-commerce")]
use crate::{payment, share};

/// Embedded snapshot bytes.
///
/// Snapshots are produced by `migo-snapshot-gen` (cross-compiled to a target
/// ABI and run on that ABI's emulator/device) and stored per-arch under
/// `snapshots/SNAPSHOT-<arch>.bin`. V8 startup snapshots are **platform-bound**
/// (OS + CPU arch): an android-<arch> snapshot only loads in that exact
/// android-<arch> V8, so `build.rs` embeds the file matching
/// `CARGO_CFG_TARGET_ARCH` and ONLY for android targets. Host builds
/// (`cargo test`, dev) never embed and fall back to loading extension JS from
/// source.
///
/// When the matching snapshot exists, `build.rs` sets the `migo_has_snapshot`
/// cfg + `MIGO_SNAPSHOT_PATH` and we embed it via `include_bytes!`; otherwise
/// `SNAPSHOT_BYTES` is `None` (from-source fallback, slower cold start).
#[cfg(migo_has_snapshot)]
pub static SNAPSHOT_BYTES: Option<&'static [u8]> = Some(include_bytes!(env!("MIGO_SNAPSHOT_PATH")));

#[cfg(not(migo_has_snapshot))]
pub static SNAPSHOT_BYTES: Option<&'static [u8]> = None;

/// Create all extensions in **lazy-init** mode (JS loaded, ops registered,
/// state callbacks deferred).
///
/// Used for both snapshot creation and snapshot-based runtime startup.
/// The order MUST match [`extension_args()`] and [`super::main_extensions()`].
pub fn lazy_extensions() -> Vec<Extension> {
    let mut exts: Vec<Extension> = Vec::new();

    // ---- CORE extensions (always loaded) ----
    exts.push(base::host_v8_base::lazy_init());
    exts.push(io_state::host_v8_io_state::lazy_init());
    exts.extend(console::console_lazy_extensions());
    exts.extend(event::event_lazy_extensions());
    exts.extend(utility::utility_lazy_extensions());

    // ---- OPTIONAL: api-sensors ----
    #[cfg(feature = "api-sensors")]
    exts.extend(device::device_lazy_extensions());

    // ---- OPTIONAL: api-system ----
    #[cfg(feature = "api-system")]
    exts.push(ui::host_v8_ui::lazy_init());

    // ---- OPTIONAL: api-connectivity ----
    #[cfg(feature = "api-connectivity")]
    exts.push(system::host_v8_system::lazy_init());

    // ---- CORE (continued) ----
    exts.push(env::host_v8_env::lazy_init());
    exts.push(lifecycle::host_v8_lifecycle::lazy_init());

    // ---- OPTIONAL: api-system (continued) ----
    #[cfg(feature = "api-system")]
    exts.push(update::host_v8_update::lazy_init());

    // ---- CORE (continued) ----
    exts.extend(storage::storage_lazy_extensions());
    exts.extend(input::touch_lazy_extensions());
    exts.extend(file::file_lazy_extensions());
    exts.extend(rendering::rendering_lazy_extensions());
    exts.extend(web::web_lazy_extensions());
    exts.extend(url::url_lazy_extensions());
    exts.extend(network::network_lazy_extensions());

    // ---- OPTIONAL: api-media ----
    #[cfg(feature = "api-media")]
    exts.extend(media::media_lazy_extensions());
    #[cfg(feature = "api-media")]
    exts.extend(audio::audio_lazy_extensions());

    // ---- OPTIONAL: api-system (continued) ----
    #[cfg(feature = "api-system")]
    exts.extend(worker::worker_lazy_extensions());

    // ---- OPTIONAL: api-commerce ----
    #[cfg(feature = "api-commerce")]
    exts.push(share::host_v8_share::lazy_init());
    #[cfg(feature = "api-commerce")]
    exts.push(payment::host_v8_payment::lazy_init());

    // ---- OPTIONAL: api-system (continued) ----
    #[cfg(feature = "api-system")]
    exts.push(ad::host_v8_ad::lazy_init());

    // ---- CORE: runtime (must be last) ----
    exts.push(super::runtime::lazy_init());

    exts
}

/// Create [`ExtensionArguments`] with actual runtime state for all extensions.
///
/// Must be passed to `JsRuntime::lazy_init_extensions()` after snapshot
/// restoration.  The order MUST match [`lazy_extensions()`].
pub fn extension_args(host: HostOpState) -> Vec<ExtensionArguments> {
    let mut args: Vec<ExtensionArguments> = Vec::new();

    // ---- CORE extensions (always loaded) ----
    args.push(base::host_v8_base::args(host));
    args.push(io_state::host_v8_io_state::args());
    args.push(console::host_v8_console::args());
    args.push(event::host_v8_event::args());
    args.push(utility::host_v8_utility::args());

    // ---- OPTIONAL: api-sensors ----
    #[cfg(feature = "api-sensors")]
    args.push(device::host_v8_device::args());

    // ---- OPTIONAL: api-system ----
    #[cfg(feature = "api-system")]
    args.push(ui::host_v8_ui::args());

    // ---- OPTIONAL: api-connectivity ----
    #[cfg(feature = "api-connectivity")]
    args.push(system::host_v8_system::args());

    // ---- CORE (continued) ----
    args.push(env::host_v8_env::args());
    args.push(lifecycle::host_v8_lifecycle::args());

    // ---- OPTIONAL: api-system (continued) ----
    #[cfg(feature = "api-system")]
    args.push(update::host_v8_update::args());

    // ---- CORE (continued) ----
    args.push(storage::host_v8_storage::args());
    args.push(input::host_v8_touch::args());
    args.push(file::host_v8_file::args());
    args.push(rendering::image::host_v8_image::args());
    args.push(rendering::webgl::host_v8_webgl::args());
    args.push(web::host_v8_web::args());
    args.push(url::host_v8_url::args());
    args.push(network::network_extension_args());

    // ---- OPTIONAL: api-media ----
    #[cfg(feature = "api-media")]
    args.push(media::host_v8_media::args());
    #[cfg(feature = "api-media")]
    args.push(audio::host_v8_audio::args());

    // ---- OPTIONAL: api-system (continued) ----
    #[cfg(feature = "api-system")]
    args.push(worker::host_v8_worker::args());

    // ---- OPTIONAL: api-commerce ----
    #[cfg(feature = "api-commerce")]
    args.push(share::host_v8_share::args());
    #[cfg(feature = "api-commerce")]
    args.push(payment::host_v8_payment::args());

    // ---- OPTIONAL: api-system (continued) ----
    #[cfg(feature = "api-system")]
    args.push(ad::host_v8_ad::args());

    // ---- CORE: runtime (must be last) ----
    args.push(super::runtime::args());

    args
}

#[cfg(test)]
mod tests {
    use std::{path::PathBuf, sync::Arc, sync::atomic::AtomicBool};

    use shared::{
        channel::ThreadWakeup,
        device::gpu_caps::GpuCaps,
        op_state::{AudioSender, HostOpState, NetworkPolicy},
        render_command_sender::CommandSender,
    };
    use tokio::sync::mpsc;

    fn test_host_state() -> HostOpState {
        let (render_tx, _render_rx) = CommandSender::new();
        let (audio_raw_tx, _audio_rx) = mpsc::unbounded_channel();
        let (host_tx, _host_rx) = mpsc::channel(1);

        HostOpState {
            id: 1,
            app_cache_dir: PathBuf::from("/tmp/cache"),
            app_files_dir: PathBuf::from("/tmp/files"),
            code_dir: None,
            game_paths: None,
            vfs: None,
            mount_table: None,
            render_tx,
            text_measurer: None,
            audio_tx: AudioSender::new(audio_raw_tx, ThreadWakeup::new()),
            host_tx,
            device_services: None,
            raf_rx: None,
            sub_packages: Vec::new(),
            workers_path: None,
            network_policy: NetworkPolicy::default(),
            backgrounded: Arc::new(AtomicBool::new(false)),
            webgl_context_created: Arc::new(AtomicBool::new(false)),
            code_signing_enabled: false,
            gpu_caps: GpuCaps::new(),
        }
    }

    #[test]
    fn snapshot_extensions_match_main_runtime_order() {
        let main_names: Vec<_> = crate::main_extensions(test_host_state())
            .into_iter()
            .map(|ext| ext.name)
            .collect();
        let lazy_names: Vec<_> = super::lazy_extensions()
            .into_iter()
            .map(|ext| ext.name)
            .collect();
        let arg_names: Vec<_> = super::extension_args(test_host_state())
            .into_iter()
            .map(|arg| arg.name)
            .collect();

        assert_eq!(lazy_names, main_names);
        assert_eq!(arg_names, main_names);
    }
}
