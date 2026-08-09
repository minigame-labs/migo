use std::borrow::Cow;
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use serde_json::Value;
use tracing::{debug, error, info, warn};

/// Whether `cmd` was produced for a runtime that is no longer the current one.
///
/// The only implementation of the rule, called by `handle_command_inner`; it is
/// a free function purely so it can be driven without a live `Host`, which needs
/// V8, a render thread and an audio thread to exist.
///
/// A command carrying no generation is never retired — see
/// [`HostCommand::callback_generation`] for the two reasons it may carry none,
/// neither of which means "stale".
fn is_retired_callback(cmd: &HostCommand, current_generation: i64) -> bool {
    matches!(cmd.callback_generation(), Some(produced_for) if produced_for != current_generation)
}

use shared::{
    config::InitOptions,
    error::EngineResult,
    host_channel::CriticalHostCommandSender,
    js_escape::{HOOK_ARGS_NONE, hook_args_one},
    op_state::{ContextLostState, HostOpState, HostTx, RafRx},
    protocol::host_cmd::HostCommand,
    protocol::render_cmd::{CanvasCmd, RenderCommand},
    render_event::{RenderEvent, RenderEventReceiver},
    surface::{PixelRatio, SurfaceLease},
};

use crate::{
    runtime::{
        HostId,
        input_state::{InputRetraction, InputState},
        vsync,
    },
    services::{AudioService, PlatformServices, RenderService},
};

use runtime_v8::HostJsRuntime;

#[cfg(feature = "v8-limits")]
use runtime_v8::V8LimitsConfig;
#[cfg(feature = "v8-limits")]
use runtime_v8::watchdog::DeadlineWatchdogConfig;

const GPU_INIT_TIMEOUT: Duration = Duration::from_secs(2);

/// Wrapper around `Option<HostJsRuntime>` with `Deref`/`DerefMut`.
///
/// During `on_restart`, the old v8 isolate must be fully destroyed **before**
/// the new one is created — two concurrent isolates on the same thread causes
/// "Cannot create a handle without a HandleScope" in the old isolate's cleanup.
/// This wrapper allows `take_and_drop()` → `set()` sequencing while keeping
/// all existing `self.js.xxx()` call sites working transparently via Deref.
pub(crate) struct JsRuntimeSlot(Option<HostJsRuntime>);

impl JsRuntimeSlot {
    fn new(js: HostJsRuntime) -> Self {
        Self(Some(js))
    }

    /// Drop the current runtime. Must be followed by `set()`.
    fn take_and_drop(&mut self) {
        self.0.take(); // moves out and drops
    }

    /// Install a new runtime (after `take_and_drop`).
    fn set(&mut self, js: HostJsRuntime) {
        debug_assert!(
            self.0.is_none(),
            "JsRuntimeSlot: replacing without dropping first"
        );
        self.0 = Some(js);
    }
}

impl std::ops::Deref for JsRuntimeSlot {
    type Target = HostJsRuntime;
    fn deref(&self) -> &HostJsRuntime {
        self.0
            .as_ref()
            .expect("[BUG] JsRuntime accessed after drop")
    }
}

impl std::ops::DerefMut for JsRuntimeSlot {
    fn deref_mut(&mut self) -> &mut HostJsRuntime {
        self.0
            .as_mut()
            .expect("[BUG] JsRuntime accessed after drop")
    }
}

/// Cleans process-global registrations if `Host::new` exits before ownership
/// transfers to the fully assembled `Host` and its normal `Drop` path.
struct HostStartupGuard {
    id: HostId,
    armed: bool,
    vsync_registered: bool,
    console_registered: bool,
}

impl HostStartupGuard {
    fn new(id: HostId) -> Self {
        Self {
            id,
            armed: true,
            vsync_registered: false,
            console_registered: false,
        }
    }

    fn mark_vsync_registered(&mut self) {
        self.vsync_registered = true;
    }

    fn mark_console_registered(&mut self) {
        self.console_registered = true;
    }

    fn disarm(&mut self) {
        self.armed = false;
        self.vsync_registered = false;
        self.console_registered = false;
    }
}

impl Drop for HostStartupGuard {
    fn drop(&mut self) {
        if self.console_registered {
            shared::console_log::unregister_console_log(self.id);
        }
        if self.vsync_registered {
            vsync::unregister_vsync_sender(self.id);
        }
        // The text texture cache registers itself lazily, from whichever
        // of the render thread or the JS extension reaches this session
        // first, so there is no `mark_*_registered` edge to hang this on.
        // A `Host::new` that failed after either could have created it and
        // there is no assembled `Host` to run the normal teardown, so
        // clear it whenever the guard is still armed. Unregistering an id
        // that was never registered is a no-op.
        //
        // The image alias table is registered from one place only — the JS
        // extension's bring-up — but leaks the same way for the same reason, and
        // this guard drops after the runtime it would have been registered by,
        // so nothing can register it again behind us.
        if self.armed {
            shared::text_texture_cache::unregister_text_cache(self.id);
            runtime_v8::unregister_image_cache(self.id);
            shared::services::forget_downloaded_zips(self.id);
        }
    }
}

pub(crate) struct Host {
    pub(crate) id: HostId,

    pub(crate) audio: AudioService,
    pub(crate) render: RenderService,

    pub(crate) js: JsRuntimeSlot,

    /// Shared RAF receiver — survives JS runtime restarts.
    raf_rx: RafRx,

    /// R1 RAF waiter demand latch — shared with the render thread; re-cloned
    /// into each new HostOpState so it survives JS runtime restarts.
    raf_demand: shared::raf_signal::RafDemandRef,

    /// R1 one-shot vsync arm closure (platform-agnostic). Stored so restart can
    /// re-clone it into the new HostOpState. `None` on platforms without a
    /// demand-driven display clock.
    request_vsync: Option<Arc<dyn Fn() + Send + Sync>>,

    /// Sender back to the host event loop (for JS-initiated restart/exit).
    host_tx: HostTx,

    platform: Arc<dyn PlatformServices>,
    init_options: InitOptions,
    network_policy: shared::op_state::NetworkPolicy,

    /// The Host's one callback-id space. Cloned into every `HostOpState`,
    /// including each Worker's, and never rebuilt on restart.
    callback_ids: Arc<shared::callback_id::CallbackIdAllocator>,

    /// The only thing that may advance the runtime generation. Everything else
    /// holds a `RuntimeGenerationReader`.
    restart_boundary: crate::runtime::restart_boundary::RestartBoundary,
    render_events: RenderEventReceiver,

    last_game_id: Option<String>,
    last_entry: Option<String>,

    /// Shared flag: `true` while the app is backgrounded (OnHide).
    /// Network polling ops check this to throttle CPU usage.
    backgrounded: Arc<AtomicBool>,
    /// Shared timer lifecycle level. It remains hidden until OnShow is
    /// delivered to JS, including the deferred-Surface path.
    timer_backgrounded: Arc<AtomicBool>,
    /// Shared flag mirrored into `HostOpState.context_lost`: set `true`
    /// between a render `ContextLost` and a successful `ContextRecovered`
    /// so JS `gl.isContextLost()` reflects reality.
    ///
    /// Written exclusively by the render thread (authoritative). The host only
    /// reads it — see `reconcile_context_lost`.
    context_lost: Arc<ContextLostState>,

    /// Last GL-context-lost level the host has dispatched to JS
    /// (`webglcontextlost` = `true`, `webglcontextrestored` = `false`).
    last_dispatched_context_lost: bool,

    /// Last `ContextLostState::epoch` the host has reconciled. A jump beyond
    /// what the delivered render events accounted for means a `ContextLost` /
    /// `ContextRecovered` edge was dropped by the bounded channel, and
    /// `reconcile_context_lost` synthesizes the missing lifecycle event(s).
    last_context_epoch: u64,

    /// Last time each render-error kind fired a Java `notify_error`. Now that
    /// `drain_render_events` is wired, a sustained GL/Canvas2D error stream (a
    /// game hammering a broken pipeline) would flood the Java layer with
    /// callbacks; this throttles `notify_error` to at most one per kind per
    /// `ERROR_NOTIFY_MIN_INTERVAL`. The `warn!` log is left unthrottled.
    render_error_throttle: HashMap<&'static str, Instant>,

    /// Pending `onShow` script captured while Android has resumed the Activity
    /// but has not yet delivered a fresh Surface.  WeChat/Chromium effectively
    /// dispatch visibility callbacks only once the page can render again; doing
    /// it earlier lets game code run against a paused render/audio subsystem.
    pending_on_show_args: Option<Cow<'static, str>>,

    /// Accepted input state retained at the sole FIFO consumer so focus loss
    /// can retract it without sending synthetic work through a saturated queue.
    input_state: InputState,

    /// Per-session GPU caps shared with the render thread.
    /// Survives JS runtime restarts (same GL context).
    gpu_caps: Arc<shared::device::gpu_caps::GpuCaps>,

    /// Coalescing render-feedback wake. The render thread's event channel calls
    /// `notify_one()` on every successfully enqueued event; the host loop selects
    /// on this to drain + reconcile promptly instead of polling. Replaces the
    /// deleted 3-second heartbeat's render-drain. Tokio latches one permit, so an
    /// event emitted during a JS poll is not lost.
    render_notify: Arc<tokio::sync::Notify>,
}

impl Drop for Host {
    fn drop(&mut self) {
        info!(
            "[Host {}] dropping host, shutting down services...",
            self.id
        );
        let js_drop_started = Instant::now();
        self.js.take_and_drop();
        info!(
            "[Host {}] JsRuntime drop during shutdown: {:.1}ms",
            self.id,
            js_drop_started.elapsed().as_secs_f64() * 1000.0
        );
        self.render.shutdown();
        self.audio.shutdown();
        vsync::unregister_vsync_sender(self.id);
        // NOTE: stats lifecycle is owned by the render thread — it registers
        // on entry and unregisters on all exit paths (Shutdown, channel close,
        // panic). Do not call unregister_stats here to avoid a double-free.
        shared::console_log::unregister_console_log(self.id);

        // Drop this session's text texture cache. Both holders of the
        // handle are gone by now — the JS `CanvasOpState` with the
        // isolate above, the render thread's `CanvasManager` when
        // `render.shutdown()` joined — so this releases the last
        // reference. Its entries hold GL texture names from an EGL
        // context that no longer exists plus their byte accounting;
        // neither may be retained for process life or inherited by a
        // later session that reuses this id.
        shared::text_texture_cache::unregister_text_cache(self.id);

        // This session's GPU image aliases. `IMAGE_CACHE` maps src to a shared
        // image id, and those ids name textures in an EGL context that is gone, so
        // like the text cache they are session-owned and must not outlive it.
        //
        // Only this session's table goes: it is keyed per host, so another live
        // game's aliases are not reachable from here to discard. No `DestroyImage`
        // dispatch accompanies it either, because `render.shutdown()` above has
        // already joined the thread that owned the context these ids named. The
        // io-side pins those aliases held are released as the table drains, which
        // matters precisely because the decoded bytes they pin are shared and do
        // survive this session.
        runtime_v8::unregister_image_cache(self.id);

        // The decoded-bytes cache is deliberately NOT cleared. Its entries are
        // context-independent RGBA and its key carries the resource's real identity
        // -- resolved path, size, mtime, mount origin -- so a later session loading
        // the same file legitimately reuses them and a changed file gets a different
        // key. That invalidation is what the token tests
        // (`extensionless_primary_size_change_invalidates_token` and its siblings)
        // cover, and it is what makes clearing unnecessary.
        //
        // Clearing here was written for a single-session world -- the comment it
        // replaced reasoned about "the next session" -- and with concurrent games it
        // threw away every *live* session's decoded images too. Total memory stays
        // bounded by the cache's own LRU byte budget either way, so not clearing
        // costs nothing in the worst case and keeps the sharing that running several
        // games in one process is for.
        //
        // What is dropped is this session's *claim* on those entries, so its bytes
        // stop being attributed to a game that no longer exists. Nothing is evicted:
        // an entry no one claims still serves the next session to ask for the same
        // unchanged file, and ages out through the LRU like any other.
        migo_io::image_cache::global_cache().release_session(self.id);

        // Paths the host reported for downloads this session never installed. They
        // are this session's alone -- the store is keyed per host so no live game's
        // pending download is reachable from here -- and a path outlives its file,
        // so keeping one would name a host temp file that is already gone.
        shared::services::forget_downloaded_zips(self.id);

        info!("[Host {}] host cleanup complete.", self.id);
    }
}

impl Host {
    pub(crate) fn new(
        id: HostId,
        host_tx: HostTx,
        critical_host_tx: CriticalHostCommandSender,
        surface: SurfaceLease,
        graphics_platform: graphics::egl_platform::GraphicsPlatform,
        platform: Arc<dyn PlatformServices>,
        init_options: InitOptions,
        surface_control: Arc<shared::surface::SurfaceControl>,
        restart_boundary: crate::runtime::restart_boundary::RestartBoundary,
    ) -> EngineResult<Self> {
        // ---- Startup timing instrumentation ----
        let t_start = Instant::now();

        // ---- RAF signal (render thread → JS async op) ----
        // On Android: eventfd (low-latency epoll wake).
        // Other platforms: tokio mpsc channel (unchanged behavior).
        let (raf_tx, raf_rx) = shared::raf_signal::create_raf_pair();

        // ---- R1 RAF demand latch (host op <-> render thread) ----
        let raf_demand = Arc::new(shared::raf_signal::RafDemand::new());
        // Allocate the first session ticket up front so pre-signals (free-run
        // RAF) have a stable ticket to match from the very first frame.
        raf_demand.begin_session();

        // ---- VSync channel (Choreographer JNI → render thread) ----
        // Only platforms that actually publish external timestamps get this
        // receiver. Passing a never-fed `Some(receiver)` on desktop would make
        // RenderThread wait for frame timestamps nobody sends, freezing the loop.
        let uses_external_vsync = platform.uses_external_vsync();
        let mut startup_guard = HostStartupGuard::new(id);
        let vsync_rx = if uses_external_vsync {
            let (vsync_tx, vsync_rx) = crossbeam_channel::bounded::<f64>(2);
            vsync::register_vsync_sender(id, vsync_tx);
            startup_guard.mark_vsync_registered();
            Some(vsync_rx)
        } else {
            None
        };

        // Immutable per-host policy. Audio captures this in a lazy client
        // factory; JS network ops keep the same snapshot across restarts.
        let network_policy = {
            use shared::op_state::NetworkPolicy;
            let mut policy = NetworkPolicy::default();
            if let Some(wl) = init_options.extras().get("domain_whitelist") {
                if let Some(arr) = wl.as_array() {
                    policy.domain_whitelist = arr
                        .iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect();
                }
            }
            if let Some(v) = init_options.extras().get("enforce_https") {
                policy.enforce_https = v.as_bool().unwrap_or(false);
            }
            policy
        };

        // ---- Services ----
        // AudioService is lazy — no thread spawned until the first
        // real audio command.  Saves ~80 ms on cold start.
        let audio = AudioService::new(host_tx.clone(), network_policy.clone());
        let gpu_caps = shared::device::gpu_caps::GpuCaps::new();

        // Authoritative GL-context-loss state. Created here (before the render
        // thread) and written exclusively by the render thread (edge-triggered
        // level + epoch); the host and JS `op_gl_is_context_lost` only read it.
        // Survives JS runtime restarts (same GL context / render thread) via the
        // shared `Arc`.
        let context_lost = Arc::new(shared::op_state::ContextLostState::default());

        // Coalescing render-feedback wake: the render thread's event channel
        // fires this after every successfully enqueued event, so the host loop
        // drains + reconciles promptly (Canvas/GL/swap/RAF-backpressure/context)
        // without a polling timer. Tokio `Notify` coalesces a burst to one permit
        // and latches a permit if no waiter is registered, so an event emitted
        // inside a JS poll is delivered on the next select iteration rather than
        // lost.
        let render_notify = Arc::new(tokio::sync::Notify::new());
        let render_wake: Option<Arc<dyn Fn() + Send + Sync>> = {
            let notify = render_notify.clone();
            Some(Arc::new(move || {
                notify.notify_one();
            }))
        };

        // R1: one-shot frame arm, one route per platform.
        //
        // With an external vsync source this routes to `platform.request_vsync(id)`
        // (Android posts a single Choreographer frame callback via JNI). Without
        // one the engine paces frames itself and its clock stops whenever nothing
        // is animating, so the arm has to wake the render thread instead: a
        // demand nudge it selects on. The payload is the wakeup — demand itself is
        // read from the latch — so a nudge already pending is not worth queueing
        // twice, which is what the single slot and `try_send` express.
        //
        // Either way the closure keeps `graphics` decoupled from `platform`: the
        // render thread and `op_await_next_frame` only invoke `Arc<dyn Fn()>`.
        let (frame_demand_rx, request_vsync): (
            Option<crossbeam_channel::Receiver<()>>,
            Option<Arc<dyn Fn() + Send + Sync>>,
        ) = if uses_external_vsync {
            let platform = platform.clone();
            (None, Some(Arc::new(move || platform.request_vsync(id))))
        } else {
            let (nudge_tx, nudge_rx) = crossbeam_channel::bounded::<()>(1);
            (
                Some(nudge_rx),
                Some(Arc::new(move || {
                    let _ = nudge_tx.try_send(());
                })),
            )
        };
        let report_surface_loss: Arc<
            dyn Fn(shared::surface::PublicSurfaceGeneration, shared::surface::SurfaceLossReason)
                + Send
                + Sync,
        > = Arc::new(move |public_generation, reason| {
            let _ = critical_host_tx.send(HostCommand::SurfaceLost {
                public_generation,
                reason,
            });
        });

        let mut render = RenderService::new(
            raf_tx,
            vsync_rx,
            frame_demand_rx,
            id,
            surface,
            graphics_platform,
            init_options.pixel_ratio(),
            init_options.target_fps(),
            Some(init_options.cache_dir().to_path_buf()),
            gpu_caps.clone(),
            context_lost.clone(),
            render_wake,
            raf_demand.clone(),
            // The render thread's own arm route is its clock when the engine paces
            // frames, so it is deliberately not handed the nudge: a thread nudging
            // itself is a wakeup that arms nothing.
            uses_external_vsync.then(|| request_vsync.clone()).flatten(),
            surface_control,
            report_surface_loss,
        )?;
        // Preserve the old two-second render-startup budget. V8 construction
        // below consumes this same deadline while the render thread initializes.
        let gpu_init_started = Instant::now();
        let render_events = render.events();

        // ---- HostOpState for extensions ----
        let device_services = platform.create_device_services(id);
        let backgrounded = Arc::new(AtomicBool::new(false));
        let timer_backgrounded = Arc::new(AtomicBool::new(false));
        let webgl_context_created = Arc::new(AtomicBool::new(false));
        // `context_lost` was created above (before the render thread, which is
        // its authoritative writer).

        let callback_ids = Arc::new(shared::callback_id::CallbackIdAllocator::default());
        let host_state = HostOpState {
            callback_ids: Arc::clone(&callback_ids),
            runtime_generation: restart_boundary.current(),
            id,
            code_dir: None,
            game_paths: None,  // Set when evaluating a module
            vfs: None,         // Set when evaluating a module
            mount_table: None, // Set when evaluating a module
            app_cache_dir: init_options.cache_dir().to_path_buf(),
            app_files_dir: init_options.files_dir().to_path_buf(),
            render_tx: render.sender(),
            // F-2: hand the shared TextMeasurer down from the
            // render thread so the JS-thread fast path (JS-side
            // LRU + inline measurement) can bypass the
            // cross-thread RPC entirely.
            text_measurer: Some(render.text_measurer()),
            audio_tx: audio.sender(),
            host_tx: host_tx.clone(),
            device_services,
            raf_rx: Some(raf_rx.clone()),
            raf_demand: raf_demand.clone(),
            request_vsync: request_vsync.clone(),
            sub_packages: init_options.sub_packages().to_vec(),
            workers_path: init_options.workers_path().map(|s| s.to_string()),
            network_policy: network_policy.clone(),
            backgrounded: backgrounded.clone(),
            timer_backgrounded: timer_backgrounded.clone(),
            webgl_context_created: webgl_context_created.clone(),
            context_lost: context_lost.clone(),
            #[cfg(feature = "code-signing")]
            code_signing_enabled: init_options.code_signing_enabled(),
            #[cfg(not(feature = "code-signing"))]
            code_signing_enabled: false,
            gpu_caps: gpu_caps.clone(),
        };

        // ---- Console log buffer (debug only) ----
        if init_options.debug_enabled() {
            shared::console_log::register_console_log(id);
            startup_guard.mark_console_registered();
        }

        let t_pre_js_done = Instant::now();
        info!(
            "[Host {}] pre-JS services: {:.1}ms (render launched + host state wired)",
            id,
            t_pre_js_done.duration_since(t_start).as_secs_f64() * 1000.0
        );

        // ---- V8 limits config ----
        #[cfg(feature = "v8-limits")]
        let v8_limits = V8LimitsConfig::from_max_memory_mb(init_options.max_memory_mb());

        // ---- JS runtime + bindings cache ----
        let t_js_start = Instant::now();
        let mut js = HostJsRuntime::new(
            id as i32,
            host_state,
            init_options.cache_dir(),
            #[cfg(feature = "v8-limits")]
            v8_limits,
            #[cfg(feature = "code-signing")]
            init_options.code_signing_enabled(),
            #[cfg(feature = "code-signing")]
            init_options.code_signing_pubkey(),
        );
        let t_js_done = Instant::now();
        info!(
            "[Host {}] JsRuntime init: {:.1}ms (V8 isolate + extensions + bindings)",
            id,
            t_js_done.duration_since(t_js_start).as_secs_f64() * 1000.0
        );

        // The render thread has been initializing EGL/GL/Skia concurrently with
        // HostJsRuntime::new. Join before watchdog installation, Host publication,
        // the caller's ready signal, or any untrusted game evaluation. This keeps
        // image ops from observing the provisional all-false capability snapshot.
        let t_gpu_join_start = Instant::now();
        let gpu_join_error = match gpu_caps.wait_ready_until(gpu_init_started, GPU_INIT_TIMEOUT) {
            shared::device::gpu_caps::GpuCapsReadyState::Ready => None,
            shared::device::gpu_caps::GpuCapsReadyState::Failed(detail) => Some(
                shared::error::EngineError::new(shared::error::ErrorCode::Render2DInitError)
                    .with_detail(detail),
            ),
            shared::device::gpu_caps::GpuCapsReadyState::Timeout => Some(
                shared::error::EngineError::new(shared::error::ErrorCode::Timeout)
                    .with_detail("render thread did not publish GPU caps within 2 seconds"),
            ),
        };
        if let Some(error) = gpu_join_error {
            // Preserve V8 thread affinity and the normal Host teardown order:
            // destroy the isolate while still on its owner thread, then stop GL.
            drop(js);
            render.shutdown_detached();
            return Err(error);
        }
        let t_gpu_ready = Instant::now();
        info!(
            "[Host {}] GPU init joined: {:.1}ms since readiness budget start, residual wait {:.1}ms",
            id,
            t_gpu_ready.duration_since(gpu_init_started).as_secs_f64() * 1000.0,
            t_gpu_ready.duration_since(t_gpu_join_start).as_secs_f64() * 1000.0,
        );

        // ---- Process deadline watchdog (v8-limits) ----
        // Install AFTER trusted runtime/bootstrap construction and BEFORE any
        // game prelude or module executes. The one process monitor thread is
        // shared across all isolates; a failed install fails host creation
        // (matching the old policy).
        #[cfg(feature = "v8-limits")]
        if init_options.watchdog_enabled() {
            let secs = init_options.watchdog_timeout_secs() as u64;
            let config = DeadlineWatchdogConfig::new(
                std::time::Duration::from_secs(secs),
                format!("host-{id}"),
            );
            js.install_watchdog(config)?;
        } else {
            info!("[Host {}] deadline watchdog disabled via InitOptions", id);
        }

        let t_total = Instant::now();
        info!(
            "[Host {}] Host::new() total: {:.1}ms (pre_js={:.1}ms, JsRuntime={:.1}ms, gpu_join_wait={:.1}ms, post_join={:.1}ms)",
            id,
            t_total.duration_since(t_start).as_secs_f64() * 1000.0,
            t_pre_js_done.duration_since(t_start).as_secs_f64() * 1000.0,
            t_js_done.duration_since(t_js_start).as_secs_f64() * 1000.0,
            t_gpu_ready.duration_since(t_gpu_join_start).as_secs_f64() * 1000.0,
            t_total.duration_since(t_gpu_ready).as_secs_f64() * 1000.0,
        );

        let host = Self {
            id,
            callback_ids,
            restart_boundary,
            render,
            audio,
            js: JsRuntimeSlot::new(js),
            raf_rx,
            raf_demand,
            request_vsync,
            host_tx,
            platform,
            init_options,
            network_policy,
            render_events,
            last_game_id: None,
            last_entry: None,
            backgrounded,
            timer_backgrounded,
            context_lost,
            last_dispatched_context_lost: false,
            last_context_epoch: 0,
            render_error_throttle: HashMap::new(),
            pending_on_show_args: None,
            input_state: InputState::default(),
            gpu_caps,
            render_notify,
        };
        startup_guard.disarm();
        Ok(host)
    }

    pub(crate) async fn handle_command(&mut self, cmd: HostCommand) {
        // Drain render-thread events on every host command so state they
        // carry (notably `ContextLost` -> `context_lost`, which backs
        // `gl.isContextLost()`) is synced on a stable path. The host loop's
        // render Notify branch covers idle periods with no incoming commands.
        self.drain_render_events();
        if let Err(e) = self.handle_command_inner(cmd).await {
            error!("[Host {}] handle_command failed: e={} ", self.id, e);
        }
    }

    /// Clone of the render-feedback wake `Notify` for the host loop to select on.
    pub(crate) fn render_notify(&self) -> Arc<tokio::sync::Notify> {
        self.render_notify.clone()
    }

    pub(crate) fn drain_render_events(&mut self) {
        while let Ok(event) = self.render_events.try_recv() {
            self.handle_render_event(event);
        }
        // Safety net: even if a `ContextLost` / `ContextRecovered` event was
        // dropped by the bounded render-event channel, the render thread has
        // written the authoritative `context_lost` atomic. Reconcile the JS
        // lifecycle event against it so `gl.isContextLost()` and the last
        // dispatched `webglcontext{lost,restored}` never disagree.
        self.reconcile_context_lost();
    }

    /// Reconcile JS `webglcontext{lost,restored}` events against the render-
    /// thread-owned authoritative state. Called after every render-event drain
    /// (command, idle parking, or render Notify) and is the SOLE dispatcher of
    /// these events, so a `ContextLost` / `ContextRecovered`
    /// render event dropped by the bounded channel still results in the correct
    /// JS lifecycle events.
    ///
    /// Uses `epoch` (bumped by the render thread on every transition) to detect
    /// dropped *edges*: if the epoch advanced but the level is back to what we
    /// last dispatched, a full lost→recovered cycle was missed and we synthesize
    /// the pair so the engine still invalidates + rebuilds its GL resources.
    /// Idempotent: a no-op when already in sync.
    fn reconcile_context_lost(&mut self) {
        // Single consistent snapshot: `lost` and `epoch` are packed in one
        // atomic, so there is no torn (new level / old epoch) read window.
        let (lost, epoch) = self.context_lost.snapshot();

        if epoch == self.last_context_epoch && lost == self.last_dispatched_context_lost {
            return; // fully in sync
        }

        if lost {
            // Currently lost: ensure the engine has seen `webglcontextlost`.
            if !self.last_dispatched_context_lost {
                self.last_dispatched_context_lost = true;
                self.js.dispatch_webgl_context_event("webglcontextlost");
            }
        } else if self.last_dispatched_context_lost {
            // We dispatched lost earlier; the context is back → dispatch restored.
            self.last_dispatched_context_lost = false;
            self.js.dispatch_webgl_context_event("webglcontextrestored");
        } else if epoch != self.last_context_epoch {
            // Currently recovered and we never told the engine it was lost, yet
            // the epoch advanced: a full lost→recovered cycle happened while the
            // render event(s) were dropped. Synthesize the missing pair so the
            // engine invalidates + rebuilds against the fresh share group. Order
            // matters: lost before restored.
            self.js.dispatch_webgl_context_event("webglcontextlost");
            self.js.dispatch_webgl_context_event("webglcontextrestored");
            // `last_dispatched_context_lost` stays false (net level = recovered).
        }

        self.last_context_epoch = epoch;
    }

    /// Minimum spacing between Java `notify_error` callbacks of the same kind.
    const ERROR_NOTIFY_MIN_INTERVAL: Duration = Duration::from_millis(1000);

    /// True (and records `now`) if a `notify_error` of `kind` may fire; false if
    /// one already fired within `ERROR_NOTIFY_MIN_INTERVAL`. Keeps a sustained
    /// render-error stream from flooding the Java layer with callbacks.
    fn should_notify_error(&mut self, kind: &'static str) -> bool {
        let now = Instant::now();
        match self.render_error_throttle.get(kind) {
            Some(&last) if now.duration_since(last) < Self::ERROR_NOTIFY_MIN_INTERVAL => false,
            _ => {
                self.render_error_throttle.insert(kind, now);
                true
            }
        }
    }

    fn on_render_error(&mut self, kind: &'static str, code: u16, message: &str) {
        warn!("[Host {}] render event ({}): {}", self.id, kind, message);
        if self.should_notify_error(kind) {
            self.platform
                .notify_error(self.id, code, "render command failed", message);
        }
    }

    fn handle_render_event(&mut self, event: RenderEvent) {
        match event {
            RenderEvent::Canvas2DError { code, message } => {
                self.on_render_error("canvas2d", code.as_u16(), &message);
            }
            RenderEvent::GlError { code, message } => {
                self.on_render_error("gl", code.as_u16(), &message);
            }
            RenderEvent::CanvasError { code, message } => {
                self.on_render_error("canvas", code.as_u16(), &message);
            }
            RenderEvent::SwapFailed { message } => {
                warn!("[Host {}] swap failed: {}", self.id, message);
                if self.should_notify_error("swap") {
                    self.platform.notify_error(
                        self.id,
                        shared::error::ErrorCode::RenderBackendError.as_u16(),
                        "render swap failed",
                        &message,
                    );
                }
            }
            RenderEvent::ContextLost => {
                warn!("[Host {}] render context lost", self.id);
                // A context loss is a *recoverable* condition: the render thread
                // immediately rebuilds the share group and always follows up with
                // `ContextRecovered { success }`. The JS-visible `webglcontextlost`
                // event is dispatched centrally by `reconcile_context_lost` (run at
                // the end of every drain), robust to this event being dropped by
                // the bounded channel.
                //
                // We deliberately do NOT raise a Java-level `notify_error` here.
                // That callback is delivered as non-recoverable (see GameSession:
                // "all native fatal errors are non-recoverable"), so signalling it
                // on every loss would tell a spec-compliant app to tear down the
                // session even when recovery succeeds a few milliseconds later
                // (e.g. a `WEBGL_lose_context.loseContext()` robustness probe or a
                // transient GPU reset). The genuine unrecoverable case is surfaced
                // from the `ContextRecovered { success: false }` arm below.
            }
            RenderEvent::ContextRecovered { success } => {
                // On success, the render thread already cleared the authoritative
                // state; `reconcile_context_lost` dispatches `webglcontextrestored`.
                // On failure the state stays lost (isContextLost() remains true);
                // surface the error, but throttle it (the render thread already
                // rate-limits per loss episode; this is defense-in-depth so a
                // burst can't flood the Java layer) via `should_notify_error`.
                if !success && self.should_notify_error("context_recovery") {
                    self.platform.notify_error(
                        self.id,
                        shared::error::ErrorCode::RenderBackendError.as_u16(),
                        "render context recovery failed",
                        "",
                    );
                }
            }
            RenderEvent::RafBackpressure { consecutive_drops } => {
                warn!(
                    "[Host {}] RAF backpressure: consecutive_drops={}",
                    self.id, consecutive_drops
                );
            }
            _ => {}
        }
    }

    async fn handle_command_inner(&mut self, cmd: HostCommand) -> EngineResult<()> {
        // A callback produced for a runtime that has since been replaced is
        // dropped here, before any JavaScript runs.
        //
        // This is the only place it can be done once for every producer. The
        // alternative -- each platform checking before it enqueues -- loses the
        // race by construction: the command sits in the queue while `on_restart`
        // runs on this same thread, so whatever was true at enqueue can stop
        // being true before dispatch. Here the comparison and the dispatch are
        // not separated by anything.
        //
        // `None` is delivered: see `callback_generation` for the two distinct
        // reasons a command carries no generation.
        let current_generation = self.restart_boundary.current();
        if is_retired_callback(&cmd, current_generation) {
            debug!(
                "[Host {}] dropping callback from generation {:?} (current {})",
                self.id,
                cmd.callback_generation(),
                current_generation
            );
            return Ok(());
        }

        match cmd {
            HostCommand::EvaluateModule { game_id, entry } => {
                self.on_evaluate_module(game_id, entry).await
            }
            HostCommand::EvalScript { source } => self.on_eval_script(source),

            HostCommand::InvokeHostHook { hook, args_json } => {
                self.js.invoke_host_hook(hook, &args_json);
                Ok(())
            }

            HostCommand::Restart => self.on_restart().await,

            HostCommand::OnShow { options_json } => {
                // Mark foreground so network polling ops resume normal rate.
                self.backgrounded.store(false, Ordering::Relaxed);

                let on_show_args = self.build_on_show_args(options_json.as_deref());

                if self.render.has_live_surface() {
                    // The SurfaceView surface survived the hide (Android does not
                    // guarantee a destroy/recreate across every pause/resume). No
                    // UpdateSurface will arrive to drive the resume, so resume
                    // render/audio and fire onShow now; otherwise the app would
                    // stay frozen and onShow would never fire.
                    self.enter_foreground();
                    self.js.set_timer_backgrounded(false);
                    self.js
                        .invoke_host_hook("_internalTriggerOnShow", &on_show_args);
                } else {
                    // Android fires Activity.onResume before surfaceCreated: the
                    // old surface is already gone. Defer render/audio resume and
                    // onShow until the new surface arrives (on_update_surface).
                    self.pending_on_show_args = Some(on_show_args);
                }
                Ok(())
            }

            HostCommand::OnHide => {
                // Mark backgrounded so network polling ops throttle their rate.
                self.backgrounded.store(true, Ordering::Relaxed);

                // Pause render and audio threads to save resources while backgrounded.
                // The render thread stops its RAF ticker (no more frames).
                // The audio thread stops processing (no audio output).
                // The host/V8 thread stays alive for timers, network, etc.
                self.render.pause();
                self.audio.pause();
                self.pending_on_show_args = None;

                self.js
                    .invoke_host_hook("_internalTriggerOnHide", HOOK_ARGS_NONE);
                self.js.set_timer_backgrounded(true);
                Ok(())
            }

            HostCommand::OnFocusChanged { focused } => {
                if !focused {
                    self.retract_input_for_focus_loss();
                }
                self.js.dispatch_focus_changed(focused);
                Ok(())
            }

            HostCommand::OnAudioInterruptionBegin => {
                self.js
                    .invoke_host_hook("_internalTriggerAudioInterruptionBegin", HOOK_ARGS_NONE);
                Ok(())
            }

            HostCommand::OnAudioInterruptionEnd => {
                self.js
                    .invoke_host_hook("_internalTriggerAudioInterruptionEnd", HOOK_ARGS_NONE);
                Ok(())
            }

            HostCommand::OnTouch(touch) => {
                let count = (touch.count as usize).min(touch.points.len());
                self.input_state.observe_touch(&touch);
                self.js.dispatch_touch(
                    touch.touch_type,
                    &touch.points[..count],
                    touch.timestamp_ms,
                );
                Ok(())
            }

            HostCommand::UpdateSurface { lease, pixel_ratio } => {
                self.on_update_surface(lease, pixel_ratio)
            }
            HostCommand::SurfaceDestroyed { generation } => {
                self.render.on_surface_destroyed(generation);
                Ok(())
            }
            HostCommand::SurfaceLost {
                public_generation,
                reason,
            } => {
                self.platform
                    .notify_surface_lost(self.id, public_generation, reason);
                Ok(())
            }

            HostCommand::Shutdown => Ok(()),

            HostCommand::InnerAudioEvent {
                id,
                event_type,
                current_time,
            } => {
                self.js
                    .dispatch_inner_audio_event(id, event_type.as_str(), current_time);
                Ok(())
            }

            HostCommand::OnDeviceMotionChange { alpha, beta, gamma } => {
                self.js.dispatch_device_motion(alpha, beta, gamma);
                Ok(())
            }

            HostCommand::OnGyroscopeChange { x, y, z } => {
                self.js.dispatch_gyroscope(x, y, z);
                Ok(())
            }

            HostCommand::OnDeviceOrientationChange { value } => {
                self.js.dispatch_device_orientation(&value);
                Ok(())
            }

            HostCommand::OnCompassChange {
                direction,
                accuracy,
            } => {
                self.js.dispatch_compass(direction, &accuracy);
                Ok(())
            }

            HostCommand::OnAccelerometerChange { x, y, z } => {
                self.js.dispatch_accelerometer(x, y, z);
                Ok(())
            }

            HostCommand::OnNetworkStatusChange {
                is_connected,
                network_type,
            } => {
                self.js.dispatch_network_status(is_connected, &network_type);
                Ok(())
            }

            HostCommand::RecorderEvent {
                event_type,
                json_payload,
            } => {
                self.js.dispatch_recorder_event(&event_type, &json_payload);
                Ok(())
            }

            HostCommand::RecorderFrameData {
                data,
                is_last_frame,
            } => {
                self.js.dispatch_recorder_frame_data(&data, is_last_frame);
                Ok(())
            }

            HostCommand::CameraEvent {
                camera_id,
                event_type,
                json_payload,
            } => {
                self.js
                    .dispatch_camera_event(camera_id, &event_type, &json_payload);
                Ok(())
            }

            HostCommand::CameraFrameData {
                camera_id,
                data,
                width,
                height,
            } => {
                self.js
                    .dispatch_camera_frame_data(camera_id, data, width, height);
                Ok(())
            }

            HostCommand::OnKeyboardInput { value, .. } => {
                self.js.dispatch_keyboard_input(&value);
                Ok(())
            }

            HostCommand::OnKeyboardHeightChange { height, .. } => {
                self.js.dispatch_keyboard_height_change(height);
                Ok(())
            }

            HostCommand::OnKeyboardConfirm { value, .. } => {
                self.js.dispatch_keyboard_confirm(&value);
                Ok(())
            }

            HostCommand::OnKeyboardComplete { value, .. } => {
                self.js.dispatch_keyboard_complete(&value);
                Ok(())
            }

            HostCommand::OnCompositionStart { data } => {
                self.js.dispatch_composition_start(&data);
                self.input_state.observe_composition_start();
                Ok(())
            }

            HostCommand::OnCompositionUpdate { data } => {
                self.js.dispatch_composition_update(&data);
                Ok(())
            }

            HostCommand::OnCompositionEnd { data } => {
                self.js.dispatch_composition_end(&data);
                self.input_state.observe_composition_end();
                Ok(())
            }

            HostCommand::OnGamepadConnected {
                index,
                id,
                mapping,
                axis_count,
                button_count,
            } => {
                self.js
                    .dispatch_gamepad_connected(index, &id, &mapping, axis_count, button_count);
                Ok(())
            }

            HostCommand::OnGamepadDisconnected { index } => {
                self.js.dispatch_gamepad_disconnected(index);
                Ok(())
            }

            HostCommand::OnGamepadState(state) => {
                self.js.dispatch_gamepad_state(&state);
                Ok(())
            }

            HostCommand::OnKeyDown {
                key,
                code,
                timestamp_ms,
                modifiers,
                repeat,
            } => {
                self.js
                    .dispatch_key_down(&key, &code, timestamp_ms, modifiers, repeat);
                self.input_state
                    .observe_key_down(key, code, timestamp_ms, modifiers);
                Ok(())
            }

            HostCommand::OnKeyUp {
                key,
                code,
                timestamp_ms,
                modifiers,
                repeat,
            } => {
                self.js
                    .dispatch_key_up(&key, &code, timestamp_ms, modifiers, repeat);
                self.input_state.observe_key_up(&code);
                Ok(())
            }

            HostCommand::OnMouseDown {
                x,
                y,
                button,
                timestamp_ms,
            } => {
                self.input_state
                    .observe_mouse_down(x, y, button, timestamp_ms);
                self.js.dispatch_mouse_down(x, y, button, timestamp_ms);
                Ok(())
            }

            HostCommand::OnMouseMove {
                x,
                y,
                button,
                timestamp_ms,
            } => {
                self.input_state
                    .observe_mouse_move(x, y, button, timestamp_ms);
                self.js.dispatch_mouse_move(x, y, button, timestamp_ms);
                Ok(())
            }

            HostCommand::OnMouseUp {
                x,
                y,
                button,
                timestamp_ms,
            } => {
                self.input_state
                    .observe_mouse_up(x, y, button, timestamp_ms);
                self.js.dispatch_mouse_up(x, y, button, timestamp_ms);
                Ok(())
            }

            HostCommand::OnWheel {
                delta_x,
                delta_y,
                delta_z,
                delta_mode,
                timestamp_ms,
            } => {
                self.js
                    .dispatch_wheel(delta_x, delta_y, delta_z, delta_mode, timestamp_ms);
                Ok(())
            }

            HostCommand::OnBLEConnectionStateChange {
                device_id,
                connected,
            } => {
                self.js
                    .dispatch_ble_connection_state_change(&device_id, connected);
                Ok(())
            }

            HostCommand::OnBLECharacteristicValueChange(ble) => {
                self.js.dispatch_ble_characteristic_value_change(
                    ble.device_id(),
                    ble.service_id(),
                    ble.characteristic_id(),
                    ble.value(),
                );
                Ok(())
            }

            HostCommand::OnBLEMTUChange { device_id, mtu } => {
                self.js.dispatch_ble_mtu_change(&device_id, mtu);
                Ok(())
            }

            HostCommand::OnBluetoothAdapterStateChange {
                available,
                discovering,
            } => {
                self.js
                    .dispatch_bluetooth_adapter_state_change(available, discovering);
                Ok(())
            }

            HostCommand::OnBluetoothDeviceFound { devices_json } => {
                self.js.dispatch_bluetooth_device_found(&devices_json);
                Ok(())
            }

            HostCommand::OnBeaconUpdate { beacons_json } => {
                self.js.dispatch_beacon_update(&beacons_json);
                Ok(())
            }

            HostCommand::OnBeaconServiceChange {
                available,
                discovering,
            } => {
                self.js
                    .dispatch_beacon_service_change(available, discovering);
                Ok(())
            }

            HostCommand::OnMemoryWarning { level } => {
                // Android ComponentCallbacks2 trim memory levels.
                // Release a proportional slice of the image cache
                // *before* dispatching to JS so the game's
                // `onMemoryWarning` handler sees a fresher
                // `getPerformance().memory` reading and doesn't
                // panic-free anything we already freed.
                let trim = migo_io::image_cache::TrimLevel::from_android(level);
                let freed = migo_io::image_cache::global_cache().trim(trim);
                // Text texture cache holds GL textures; it can only be
                // trimmed where there's a current EGL context, so hand
                // the level to the render thread (best-effort, lifecycle
                // class — a dropped trim just gets retried on the next
                // pressure signal).
                let _ = self
                    .render
                    .sender()
                    .dispatch(RenderCommand::TrimTextCache { level });
                match level {
                    5 => tracing::info!(
                        "Memory pressure: RUNNING_MODERATE (image cache freed {freed}B)"
                    ),
                    10 => {
                        tracing::warn!("Memory pressure: RUNNING_LOW (image cache freed {freed}B)")
                    }
                    15 => tracing::warn!(
                        "Memory pressure: RUNNING_CRITICAL (image cache freed {freed}B)"
                    ),
                    _ => {
                        tracing::debug!("Memory warning level {level} (image cache freed {freed}B)")
                    }
                }
                self.js.dispatch_memory_warning(level);
                Ok(())
            }

            HostCommand::OnUserCaptureScreen => {
                self.js
                    .invoke_host_hook("_internalTriggerUserCaptureScreen", HOOK_ARGS_NONE);
                Ok(())
            }

            // Android ADPF thermal status (PowerManager.THERMAL_STATUS_*).
            HostCommand::OnThermalStatusChanged { status } => {
                match status {
                    0 => tracing::debug!("Thermal: NONE"),
                    1 => tracing::info!("Thermal: LIGHT"),
                    2 => tracing::warn!("Thermal: MODERATE"),
                    3 | 4 => tracing::warn!("Thermal: SEVERE/CRITICAL ({status})"),
                    5 | 6 => tracing::error!("Thermal: EMERGENCY/SHUTDOWN ({status})"),
                    _ => tracing::debug!("Thermal: unknown ({status})"),
                }
                Ok(())
            }

            HostCommand::OnVideoStateChange {
                video_id,
                event_type,
                data,
            } => {
                self.js.dispatch_video_event(video_id, &event_type, &data);
                Ok(())
            }

            HostCommand::SetDisplayRefreshRate { period_nanos } => {
                let hz = if period_nanos > 0 {
                    1_000_000_000.0 / period_nanos as f64
                } else {
                    60.0
                };
                tracing::info!(
                    "Display refresh rate: {:.1}Hz (period={}ns)",
                    hz,
                    period_nanos
                );
                Ok(())
            }

            HostCommand::SendToHost { json } => {
                self.platform.notify_host_message(self.id, &json);
                Ok(())
            }

            other => {
                tracing::warn!("[Host {}] unhandled HostCommand: {:?}", self.id, other);
                Ok(())
            }
        }
    }

    async fn on_evaluate_module(&mut self, game_id: String, entry: String) -> EngineResult<()> {
        let t_eval_start = Instant::now();
        self.last_game_id = Some(game_id.clone());
        self.last_entry = Some(entry.clone());

        // Run boot prelude scripts (e.g. BOM/DOM adapter for browser-style
        // games) before the main module loads. Prelude failures abort the
        // launch with the same error path as the main module — a partially
        // adapted globalThis is worse than no game at all.
        let prelude_count = self.init_options.prelude_scripts().len();
        if prelude_count > 0 {
            let t_prelude = Instant::now();
            // Clone out of init_options to release the borrow on `self`
            // before the &mut self.js call below.
            let scripts: Vec<(String, String)> = self.init_options.prelude_scripts().to_vec();
            for (name, source) in &scripts {
                self.js
                    .exec_script_owned(name.clone(), source)
                    .map_err(|e| {
                        error!("[Host {}] prelude script '{}' failed: {}", self.id, name, e);
                        e
                    })?;
            }
            info!(
                "[Host {}] {} prelude script(s) executed: {:.1}ms",
                self.id,
                prelude_count,
                t_prelude.elapsed().as_secs_f64() * 1000.0,
            );
        }

        self.js
            .evaluate_module(game_id.clone(), entry.clone())
            .await?;
        let eval_ms = t_eval_start.elapsed().as_secs_f64() * 1000.0;
        info!(
            "[Host {}] evaluate_module('{}', '{}'): {:.1}ms",
            self.id, game_id, entry, eval_ms,
        );

        // TIMING NOTE: notify_game_ready fires here, after JS module evaluation
        // completes but BEFORE the first frame is rendered. The render thread has
        // not yet received a RAF tick or called swap_buffers at this point.
        // Perceived startup time (what the user sees) is typically 16-50ms longer
        // than the value reported by game_ready, because it takes at least one
        // vsync interval for the render thread to produce and present the first
        // frame. See DebugStats.first_frame_ms for the render-side measurement.
        self.platform.notify_game_ready(self.id);

        // NOTE: Do NOT call run_event_loop() here. The op-based RAF
        // (op_await_next_frame) creates a permanently-pending op that keeps
        // the event loop alive forever. Calling run_event_loop() would block
        // the host thread indefinitely, preventing all subsequent commands
        // (UpdateSurface, OnHide, touch, etc.) from being processed.
        //
        // The main tokio::select! loop in thread.rs continuously drives the
        // event loop via its run_event_loop branch, which handles all pending
        // ops including RAF, microtasks, and timers.

        Ok(())
    }

    fn on_eval_script(&mut self, source: String) -> EngineResult<()> {
        self.js.exec_script("eval-script", &source)
    }

    /// Build the argument list for the game's `onShow`, carrying the launch
    /// options (if any) as a value rather than as text.
    ///
    /// This used to build JS source, which made U+2028/U+2029 a hazard: both
    /// are valid inside a JSON string yet terminate a line in JS source, so the
    /// options had to be escaped for a syntax they no longer travel through.
    /// Nothing here is parsed as source, so that class of escaping bug is gone
    /// rather than handled.
    fn build_on_show_args(&self, options_json: Option<&str>) -> Cow<'static, str> {
        let Some(options_json) = options_json else {
            return Cow::Borrowed(HOOK_ARGS_NONE);
        };
        let options_json = options_json.trim();
        if options_json.is_empty() {
            return Cow::Borrowed(HOOK_ARGS_NONE);
        }
        match serde_json::from_str::<Value>(options_json) {
            Ok(value) if value.is_object() => hook_args_one(value),
            Ok(_) => Cow::Borrowed(HOOK_ARGS_NONE),
            Err(e) => {
                warn!(
                    "[Host {}] invalid onShow options JSON, fallback to default: {}",
                    self.id, e
                );
                Cow::Borrowed(HOOK_ARGS_NONE)
            }
        }
    }

    /// Resume the render/audio threads and nudge the JS RAF loop + window resize
    /// after the app returns to the foreground with a live surface. Safe to call
    /// when nothing was paused (resume is a no-op then). Does NOT fire onShow —
    /// the caller owns onShow sequencing.
    fn enter_foreground(&mut self) {
        self.render.resume();
        self.audio.resume();
        // The RAF loop self-stops after a few idle frames while hidden; this is a
        // low-cost nudge to restart it now that the surface/foreground is back.
        let _ = self.js.exec_script(
            "raf_resume_kick",
            "globalThis.__migo_restart_raf_loop && globalThis.__migo_restart_raf_loop()",
        );
        self.js
            .invoke_host_hook("_internalTriggerWindowResize", HOOK_ARGS_NONE);
    }

    fn on_update_surface(
        &mut self,
        lease: SurfaceLease,
        pixel_ratio: Option<PixelRatio>,
    ) -> EngineResult<()> {
        let (w, h) = lease.size();
        info!(
            "[Host {}] on_update_surface: requested={}x{}",
            self.id, w, h
        );

        // Retry a bounded number of times: a transiently full render command
        // queue can make the bounded-blocking recreate send/reply time out, and a
        // dropped recreate would strand the app on a black frame with no further
        // surface callback from Java. Surface updates are rare, so a few host-
        // thread retries are an acceptable tradeoff for not losing the surface.
        let mut result = self.render.update_surface(lease.clone(), pixel_ratio);
        let mut attempts = 1u32;
        while result.is_err() && lease.is_live() && attempts < 3 {
            attempts += 1;
            warn!(
                "[Host {}] on_update_surface attempt {} after error: {:?}",
                self.id, attempts, result
            );
            result = self.render.update_surface(lease.clone(), pixel_ratio);
        }

        // Resume the foreground after the surface is (re)created — but only when
        // actually foregrounded. Android can deliver surfaceCreated/Changed while
        // still hidden (before onResume) or recreate a surface while backgrounded;
        // resuming then would run render/audio in the background. In that case the
        // surface is marked live but stays paused, and the OnShow live-surface
        // path drives the resume once `backgrounded` clears.
        if result.is_ok() {
            if !self.backgrounded.load(Ordering::Relaxed) {
                self.enter_foreground();
                if let Some(args) = self.pending_on_show_args.take() {
                    self.js.set_timer_backgrounded(false);
                    self.js.invoke_host_hook("_internalTriggerOnShow", &args);
                }
            }
            info!("[Host {}] on_update_surface completed", self.id);
        } else if let Err(ref e) = result {
            warn!("[Host {}] on_update_surface failed: {}", self.id, e);
        }

        result
    }

    fn retract_input_for_focus_loss(&mut self) {
        let js = &mut self.js;
        self.input_state
            .retract_for_focus_loss(|retraction| match retraction {
                InputRetraction::TouchCancel(touch) => {
                    let count = usize::from(touch.count).min(touch.points.len());
                    js.dispatch_touch(touch.touch_type, &touch.points[..count], touch.timestamp_ms);
                }
                InputRetraction::MouseUp {
                    x,
                    y,
                    button,
                    timestamp_ms,
                } => js.dispatch_mouse_up(x, y, button, timestamp_ms),
                InputRetraction::KeyUp {
                    key,
                    code,
                    timestamp_ms,
                    modifiers,
                } => js.dispatch_key_up(&key, &code, timestamp_ms, modifiers, false),
                InputRetraction::CompositionEnd => js.dispatch_composition_end(""),
            });
    }

    async fn on_restart(&mut self) -> EngineResult<()> {
        // Pause subsystems to ensure a clean restart
        self.render.pause();
        self.audio.pause();
        self.input_state = InputState::default();

        // Bump the RAF session ticket so the fresh isolate ignores any signal
        // (stale timestamp) produced for the old one on the shared eventfd, and
        // the old isolate's in-flight `recv` never matches the new session.
        self.raf_demand.begin_session();

        // Recreate JS runtime with fresh state
        let (files_dir, cache_dir) = self.js.get_base_dirs();
        let device_services = self.platform.create_device_services(self.id);

        // The replacement runtime is a new generation, and it is a *candidate*
        // until it exists: `candidate_generation` reads without mutating, so a
        // restart that fails to build a runtime leaves the live generation
        // exactly where it was and nothing downstream is fenced against an
        // isolate that never appeared. The commit is at the bottom, once the new
        // isolate is installed.
        let retired_generation = self.restart_boundary.current();
        let candidate_generation = self.restart_boundary.candidate_generation()?;

        let host_state = HostOpState {
            // The allocator is never rebuilt -- ids outlive the isolate on
            // purpose -- but the generation does advance, and it is what tells a
            // retired runtime's still-queued callbacks from this one's.
            callback_ids: Arc::clone(&self.callback_ids),
            runtime_generation: candidate_generation,
            id: self.id,
            code_dir: None,
            game_paths: None,
            vfs: None,
            mount_table: None,
            app_cache_dir: cache_dir,
            app_files_dir: files_dir,
            render_tx: self.render.sender(),
            text_measurer: Some(self.render.text_measurer()),
            audio_tx: self.audio.sender(),
            host_tx: self.host_tx.clone(),
            device_services,
            raf_rx: Some(self.raf_rx.clone()),
            raf_demand: self.raf_demand.clone(),
            request_vsync: self.request_vsync.clone(),
            sub_packages: self.init_options.sub_packages().to_vec(),
            workers_path: self.init_options.workers_path().map(|s| s.to_string()),
            network_policy: self.network_policy.clone(),
            backgrounded: self.backgrounded.clone(),
            timer_backgrounded: self.timer_backgrounded.clone(),
            webgl_context_created: Arc::new(AtomicBool::new(false)),
            context_lost: self.context_lost.clone(),
            #[cfg(feature = "code-signing")]
            code_signing_enabled: self.init_options.code_signing_enabled(),
            #[cfg(not(feature = "code-signing"))]
            code_signing_enabled: false,
            gpu_caps: self.gpu_caps.clone(),
        };

        // drain_shared_image_cache() returns this session's shared IDs and clears
        // its JS-side bookkeeping.  We must send DestroyImage for each ID
        // *before* clearing the IO cache — otherwise the render thread holds
        // orphaned GPU textures that no one will ever release. Batch them into a
        // single must-deliver command so a large image set doesn't block restart
        // up to the send deadline per image. The registration survives the drain
        // because the isolate this restart is about to build resolves the same
        // handle.
        let shared_ids = runtime_v8::drain_shared_image_cache(self.id);
        if !shared_ids.is_empty() {
            if let Err(e) =
                self.render
                    .sender()
                    .dispatch(RenderCommand::Canvas(CanvasCmd::DestroyImages {
                        image_ids: shared_ids,
                    }))
            {
                warn!(
                    "[Host {}] on_restart: DestroyImages dispatch failed (textures may leak): {}",
                    self.id, e
                );
            }
        }
        // Not cleared, for the reason given in the teardown path: the decoded-bytes
        // key carries real identity, so a restart reloading the same unchanged files
        // should reuse them rather than pay to decode again, and clearing would have
        // discarded every other running game's images as well.
        //
        // This session's *claim* on those entries is dropped further down, once the
        // IO scheduler is closed.

        // ---- V8 limits config ----
        #[cfg(feature = "v8-limits")]
        let v8_limits = V8LimitsConfig::from_max_memory_mb(self.init_options.max_memory_mb());

        // Close the IO scheduler before dropping the runtime.  This ensures
        // in-flight async IO tasks are rejected immediately rather than racing
        // with the new session's scheduler after the old runtime is gone.
        self.js.close_io_scheduler();

        // Drop this session's claim on the decoded bytes, so what it is reported as
        // holding reflects what its new isolate has actually loaded rather than what
        // the previous one did. Nothing is evicted.
        //
        // Ordered after the scheduler closes rather than before, because a decode
        // queued by the old isolate would otherwise insert after this scan and hand
        // the new isolate a claim it never made. Closing first rejects the queued
        // work; a closure already running in a worker can still land afterwards, and
        // that residue is accepted rather than designed against. A host id names the
        // *session*, not the isolate, so such an entry is attributed to the same game
        // that decoded it either way -- it cannot cross to another game. What it can
        // do is make a restart boundary slightly untidy in that game's own figures,
        // and incarnation-tagged owner identity is a large amount of plumbing to buy
        // exactness in a diagnostic.
        migo_io::image_cache::global_cache().release_session(self.id);

        // CRITICAL: Drop the old JsRuntime BEFORE creating the new one.
        // Two v8 isolates on the same thread during drop cleanup causes
        // "Cannot create a handle without a HandleScope" crash — the old
        // isolate's cleanup handler can't create a HandleScope when v8's
        // thread-local state was modified by the new isolate's initialization.
        let js_drop_started = Instant::now();
        self.js.take_and_drop();
        info!(
            "[Host {}] JsRuntime drop during restart: {:.1}ms",
            self.id,
            js_drop_started.elapsed().as_secs_f64() * 1000.0
        );
        let mut new_js = HostJsRuntime::new(
            self.id as i32,
            host_state,
            self.init_options.cache_dir(),
            #[cfg(feature = "v8-limits")]
            v8_limits,
            #[cfg(feature = "code-signing")]
            self.init_options.code_signing_enabled(),
            #[cfg(feature = "code-signing")]
            self.init_options.code_signing_pubkey(),
        );

        // Recreate watchdog for the new isolate
        #[cfg(feature = "v8-limits")]
        if self.init_options.watchdog_enabled() {
            // The old runtime (with its watchdog field, dropped first) already
            // disarmed + unregistered before `take_and_drop()` above, so the new
            // isolate registers a fresh target with no overlap. A failed install
            // on restart logs and continues without a watchdog.
            let secs = self.init_options.watchdog_timeout_secs() as u64;
            let config = DeadlineWatchdogConfig::new(
                std::time::Duration::from_secs(secs),
                format!("host-{}", self.id),
            );
            if let Err(e) = new_js.install_watchdog(config) {
                error!(
                    "[Host {}] failed to install watchdog after restart: {} (continuing without watchdog)",
                    self.id, e
                );
            }
        }

        self.js.set(new_js);

        // Publish the candidate. Every callback still queued behind this restart
        // was produced for `retired_generation` and is dropped at dispatch from
        // here on; anything the new isolate creates carries the new value.
        //
        // After `js.set` rather than before: until the isolate exists there is
        // nothing for the new generation to name, and a commit that ran first
        // would fence the retired runtime's callbacks against a runtime that
        // might still fail to appear.
        self.restart_boundary
            .commit(retired_generation, candidate_generation)?;

        // If we have a last evaluated module, reload it. Even if re-evaluation
        // fails, resume render/audio below so the session doesn't stay paused.
        let reload_result = if let (Some(game_id), Some(entry)) =
            (self.last_game_id.clone(), self.last_entry.clone())
        {
            self.on_evaluate_module(game_id, entry).await
        } else {
            Ok(())
        };

        // Resume render and audio so the new runtime can start producing frames.
        //
        // If the Android surface survived restart, explicitly re-signal that live
        // surface before resume so the render thread can present again without
        // waiting for a fresh surface callback. If the surface was already
        // destroyed, `restore_surface()` now fails instead of reusing a stale
        // handle, and the later UpdateSurface path will restore presentation.
        if let Err(e) = self.render.restore_surface() {
            error!(
                "[Host {}] on_restart: restore_surface failed: {}",
                self.id, e
            );
        }
        self.render.resume();
        self.audio.resume();

        // The JS runtime was recreated: its canvas carries no context-loss state
        // yet. Reset our dispatch bookkeeping and reconcile against the (render-
        // owned) authoritative atomic, so a genuinely still-lost context surfaces
        // `webglcontextlost` to the fresh runtime while a healthy one dispatches
        // nothing. The `context_lost` atomic itself is NOT reset here — the render
        // thread remains its sole authority across restarts.
        self.last_dispatched_context_lost = false;
        self.reconcile_context_lost();

        reload_result
    }
}

#[cfg(test)]
mod retired_callback_tests {
    use super::is_retired_callback;
    use shared::HostCommand;

    fn keyboard(generation: Option<i64>) -> HostCommand {
        HostCommand::OnKeyboardComplete {
            value: "done".to_owned(),
            runtime_generation: generation,
        }
    }

    #[test]
    fn retired_callback_generation_is_dropped_at_host_dispatch() {
        // The whole property in one place: the same command, the same current
        // generation, differing only in what it was produced for.
        assert!(is_retired_callback(&keyboard(Some(1)), 2));
        assert!(!is_retired_callback(&keyboard(Some(2)), 2));
    }

    #[test]
    fn a_generation_stays_retired_however_far_the_runtime_moves_on() {
        // Not "one behind": a callback held by a slow platform across several
        // restarts is as stale as one held across a single restart, and an
        // off-by-one comparison would let the older one through.
        for current in [2, 3, 10, i64::MAX] {
            assert!(
                is_retired_callback(&keyboard(Some(1)), current),
                "{current}"
            );
        }
    }

    #[test]
    fn a_command_that_carries_no_generation_is_delivered() {
        // Two distinct cases, and neither means stale: a command that is not a
        // runtime-owned callback at all, and a producer this plan has not fenced
        // yet. Dropping either would lose input nobody is fencing.
        assert!(!is_retired_callback(&keyboard(None), 7));
        assert!(!is_retired_callback(&HostCommand::OnHide, 7));
        assert!(!is_retired_callback(
            &HostCommand::OnKeyUp {
                key: "a".to_owned(),
                code: "KeyA".to_owned(),
                timestamp_ms: 0.0,
                modifiers: 0,
                repeat: false,
            },
            7
        ));
    }
}
