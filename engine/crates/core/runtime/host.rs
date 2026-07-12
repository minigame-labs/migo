use std::{
    collections::HashMap,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use deno_core::ModuleLoader;
use deno_core::serde_json::Value;
use tracing::{error, info, warn};

use shared::{
    config::InitOptions,
    error::EngineResult,
    js_escape::{HOST_BRIDGE_EXPR, escape_for_js_string},
    op_state::{ContextLostState, HostOpState, HostTx, RafRx},
    protocol::host_cmd::HostCommand,
    protocol::render_cmd::{CanvasCmd, RenderCommand},
    render_event::{RenderEvent, RenderEventReceiver},
    surface::SurfaceRef,
};

use crate::{
    runtime::{HostId, code_cache, loader::MyModuleLoader, vsync},
    services::{AudioService, PlatformServices, RenderService},
};

use js_runtime::HostJsRuntime;

#[cfg(feature = "v8-limits")]
use crate::runtime::watchdog::{self, WatchdogConfig, WatchdogHandle};
#[cfg(feature = "v8-limits")]
use js_runtime::V8LimitsConfig;

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
    pending_on_show_script: Option<String>,

    /// Per-session GPU caps shared with the render thread.
    /// Survives JS runtime restarts (same GL context).
    gpu_caps: Arc<shared::device::gpu_caps::GpuCaps>,

    /// Watchdog handle for JS execution timeout detection.
    /// Present only when `v8-limits` feature is enabled.
    #[cfg(feature = "v8-limits")]
    pub(crate) watchdog: Option<WatchdogHandle>,
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

        // Clear process-global caches to prevent stale state leaking into
        // the next session (host_id increments, but caches are static).
        js_runtime::clear_shared_image_cache();
        io::global_cache().clear();

        info!("[Host {}] host cleanup complete.", self.id);
    }
}

impl Host {
    pub(crate) fn new(
        id: HostId,
        host_tx: HostTx,
        surface: SurfaceRef,
        platform: Arc<dyn PlatformServices>,
        init_options: InitOptions,
    ) -> EngineResult<Self> {
        // ---- Startup timing instrumentation ----
        let t_start = Instant::now();

        // ---- RAF signal (render thread → JS async op) ----
        // On Android: eventfd (low-latency epoll wake).
        // Other platforms: tokio mpsc channel (unchanged behavior).
        let (raf_tx, raf_rx) = shared::raf_signal::create_raf_pair();

        // ---- R1 RAF demand latch (host op <-> render thread) ----
        let raf_demand = Arc::new(shared::raf_signal::RafDemand::new());

        // ---- VSync channel (Choreographer JNI → render thread) ----
        // Only platforms that actually publish external timestamps get this
        // receiver. Passing a never-fed `Some(receiver)` on desktop would make
        // RenderThread disable its software ticker and freeze the frame loop.
        let uses_external_vsync = platform.uses_external_vsync();
        let vsync_rx = if uses_external_vsync {
            let (vsync_tx, vsync_rx) = crossbeam_channel::bounded::<f64>(2);
            vsync::register_vsync_sender(id, vsync_tx);
            Some(vsync_rx)
        } else {
            None
        };

        // ---- Services ----
        // AudioService is lazy — no thread spawned until the first
        // real audio command.  Saves ~80 ms on cold start.
        let audio = AudioService::new(host_tx.clone());
        let gpu_caps = shared::device::gpu_caps::GpuCaps::new();

        // Authoritative GL-context-loss state. Created here (before the render
        // thread) and written exclusively by the render thread (edge-triggered
        // level + epoch); the host and JS `op_gl_is_context_lost` only read it.
        // Survives JS runtime restarts (same GL context / render thread) via the
        // shared `Arc`.
        let context_lost = Arc::new(shared::op_state::ContextLostState::default());

        // Wake nudge: the render thread invokes this after a context
        // lost/recovered transition so the host drains + reconciles the
        // (lossy, non-select-able) render-event channel promptly instead of
        // waiting for the heartbeat. Best-effort — a full command queue falls
        // back to the heartbeat drain. Decoupled via a closure so `graphics`
        // never depends on `HostCommand`.
        let render_wake: Option<Arc<dyn Fn() + Send + Sync>> = {
            let wake_tx = host_tx.clone();
            Some(Arc::new(move || {
                let _ = wake_tx.try_send(HostCommand::DrainRenderEvents);
            }))
        };

        // R1: one-shot vsync arm. Routes to `platform.request_vsync(id)` (a
        // no-op by default; Android posts a single Choreographer frame callback
        // via JNI). The closure keeps `graphics` decoupled from `platform` — the
        // render thread and `op_await_next_frame` only invoke `Arc<dyn Fn()>`.
        let request_vsync: Option<Arc<dyn Fn() + Send + Sync>> = if uses_external_vsync {
            let platform = platform.clone();
            Some(Arc::new(move || platform.request_vsync(id)))
        } else {
            None
        };

        let mut render = RenderService::new(
            raf_tx,
            vsync_rx,
            id,
            surface,
            init_options.pixel_ratio(),
            init_options.target_fps(),
            Some(init_options.cache_dir().to_path_buf()),
            gpu_caps.clone(),
            context_lost.clone(),
            render_wake,
            raf_demand.clone(),
            request_vsync.clone(),
        )?;
        let render_events = render.events();

        // Block until the render thread has completed GL init and
        // populated GPU compressed-format caps.  Prevents early image
        // loads from seeing all-false caps.  Typically < 50 ms.
        match gpu_caps.wait_ready(std::time::Duration::from_secs(2)) {
            shared::device::gpu_caps::GpuCapsReadyState::Ready => {}
            shared::device::gpu_caps::GpuCapsReadyState::Failed(detail) => {
                render.shutdown_detached();
                vsync::unregister_vsync_sender(id);
                return Err(shared::error::EngineError::new(
                    shared::error::ErrorCode::Render2DInitError,
                )
                .with_detail(detail));
            }
            shared::device::gpu_caps::GpuCapsReadyState::Timeout => {
                render.shutdown_detached();
                vsync::unregister_vsync_sender(id);
                return Err(
                    shared::error::EngineError::new(shared::error::ErrorCode::Timeout)
                        .with_detail("render thread did not publish GPU caps within 2 seconds"),
                );
            }
        }

        // ---- HostOpState for extensions ----
        let device_services = platform.create_device_services(id);
        // Build network policy from InitOptions extras.
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

        let backgrounded = Arc::new(AtomicBool::new(false));
        let timer_backgrounded = Arc::new(AtomicBool::new(false));
        let webgl_context_created = Arc::new(AtomicBool::new(false));
        // `context_lost` was created above (before the render thread, which is
        // its authoritative writer).

        let host_state = HostOpState {
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
        }

        // ---- Code cache (V8 bytecode persistence) ----
        let shared_cache = code_cache::create_code_cache(init_options.cache_dir());

        // SharedMountTableRef: created here, shared with the module loader
        // and HostJsRuntime. evaluate_module() populates it later.
        let loader_mount_ref: js_runtime::SharedMountTableRef =
            Rc::new(std::cell::RefCell::new(None));
        let module_loader: Option<Rc<dyn ModuleLoader>> = Some(Rc::new(MyModuleLoader::new(
            Some(shared_cache.clone()),
            loader_mount_ref.clone(),
        )));

        let ext_code_cache: Option<Rc<dyn deno_core::ExtCodeCache>> =
            Some(code_cache::ExtCodeCacheAdapter::new(shared_cache));

        // ---- Extensions ----
        let extra_ext = platform.extensions(&init_options);

        let t_services = Instant::now();
        info!(
            "[Host {}] services init: {:.1}ms (render + audio channels)",
            id,
            t_services.duration_since(t_start).as_secs_f64() * 1000.0
        );

        // ---- V8 limits config ----
        #[cfg(feature = "v8-limits")]
        let v8_limits = V8LimitsConfig::from_max_memory_mb(init_options.max_memory_mb());

        // ---- JS runtime + bindings cache ----
        let t_js_start = Instant::now();
        let mut js = HostJsRuntime::new(
            id as i32,
            host_state,
            extra_ext,
            module_loader,
            ext_code_cache,
            loader_mount_ref,
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

        // ---- Watchdog (v8-limits) ----
        #[cfg(feature = "v8-limits")]
        let watchdog = if init_options.watchdog_enabled() {
            let isolate_handle = js.isolate_handle();
            let config = WatchdogConfig::default().with_timeout(std::time::Duration::from_secs(
                init_options.watchdog_timeout_secs() as u64,
            ));
            Some(watchdog::spawn_watchdog(isolate_handle, config, id as i32)?)
        } else {
            info!("[Host {}] ANR watchdog disabled via InitOptions", id);
            None
        };

        let t_total = Instant::now();
        info!(
            "[Host {}] Host::new() total: {:.1}ms (services={:.1}ms, JsRuntime={:.1}ms, watchdog={:.1}ms)",
            id,
            t_total.duration_since(t_start).as_secs_f64() * 1000.0,
            t_services.duration_since(t_start).as_secs_f64() * 1000.0,
            t_js_done.duration_since(t_js_start).as_secs_f64() * 1000.0,
            t_total.duration_since(t_js_done).as_secs_f64() * 1000.0,
        );

        Ok(Self {
            id,
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
            pending_on_show_script: None,
            gpu_caps,
            #[cfg(feature = "v8-limits")]
            watchdog,
        })
    }

    pub(crate) async fn handle_command(&mut self, cmd: HostCommand) {
        // Drain render-thread events on every host command so state they
        // carry (notably `ContextLost` -> `context_lost`, which backs
        // `gl.isContextLost()`) is synced on a stable path. The event loop
        // also drains on the heartbeat tick (see `thread.rs`) to cover idle
        // periods with no incoming commands.
        self.drain_render_events();
        if let Err(e) = self.handle_command_inner(cmd).await {
            error!("[Host {}] handle_command failed: e={} ", self.id, e);
        }
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
    /// (command / idle-parking / heartbeat / wake nudge) and is the SOLE
    /// dispatcher of these events, so a `ContextLost` / `ContextRecovered`
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
                // JS `webglcontextlost` dispatch is handled centrally by
                // `reconcile_context_lost` (run at the end of every drain), which
                // reads the render-owned authoritative state and is robust to this
                // event being dropped by the bounded channel. Here we only surface
                // the one-shot Java notification.
                self.platform.notify_error(
                    self.id,
                    shared::error::ErrorCode::RenderBackendError.as_u16(),
                    "render context lost",
                    "",
                );
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
        match cmd {
            HostCommand::EvaluateModule { game_id, entry } => {
                self.on_evaluate_module(game_id, entry).await
            }
            HostCommand::EvalScript { source } => self.on_eval_script(source),

            HostCommand::Restart => self.on_restart().await,

            HostCommand::OnShow { options_json } => {
                // Mark foreground so network polling ops resume normal rate.
                self.backgrounded.store(false, Ordering::Relaxed);

                let script = self.build_on_show_script(options_json.as_deref());

                if self.render.has_live_surface() {
                    // The SurfaceView surface survived the hide (Android does not
                    // guarantee a destroy/recreate across every pause/resume). No
                    // UpdateSurface will arrive to drive the resume, so resume
                    // render/audio and fire onShow now; otherwise the app would
                    // stay frozen and onShow would never fire.
                    self.enter_foreground();
                    self.js.set_timer_backgrounded(false);
                    let _ = self.js.exec_script("onshow", &script);
                } else {
                    // Android fires Activity.onResume before surfaceCreated: the
                    // old surface is already gone. Defer render/audio resume and
                    // onShow until the new surface arrives (on_update_surface).
                    self.pending_on_show_script = Some(script);
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
                self.pending_on_show_script = None;

                let result = self.js.exec_script(
                    "onhide",
                    &format!("{HOST_BRIDGE_EXPR}._internalTriggerOnHide()"),
                );
                self.js.set_timer_backgrounded(true);
                result
            }

            HostCommand::OnAudioInterruptionBegin => self.js.exec_script(
                "audio_interruption_begin",
                &format!("{HOST_BRIDGE_EXPR}._internalTriggerAudioInterruptionBegin()"),
            ),

            HostCommand::OnAudioInterruptionEnd => self.js.exec_script(
                "audio_interruption_end",
                &format!("{HOST_BRIDGE_EXPR}._internalTriggerAudioInterruptionEnd()"),
            ),

            HostCommand::OnTouch(touch) => {
                let count = (touch.count as usize).min(touch.points.len());
                self.js.dispatch_touch(
                    touch.touch_type,
                    &touch.points[..count],
                    touch.timestamp_ms,
                );
                Ok(())
            }

            HostCommand::UpdateSurface { surface } => self.on_update_surface(surface),
            HostCommand::SurfaceDestroyed => {
                self.render.on_surface_destroyed();
                Ok(())
            }

            HostCommand::Shutdown => Ok(()),

            // Wake nudge from the render thread. The actual drain + reconcile
            // already ran in `handle_command` before dispatching here, so this
            // is an intentional no-op that only served to wake the select loop.
            HostCommand::DrainRenderEvents => Ok(()),

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

            HostCommand::OnKeyboardInput { value } => {
                self.js.dispatch_keyboard_input(&value);
                Ok(())
            }

            HostCommand::OnKeyboardHeightChange { height } => {
                self.js.dispatch_keyboard_height_change(height);
                Ok(())
            }

            HostCommand::OnKeyboardConfirm { value } => {
                self.js.dispatch_keyboard_confirm(&value);
                Ok(())
            }

            HostCommand::OnKeyboardComplete { value } => {
                self.js.dispatch_keyboard_complete(&value);
                Ok(())
            }

            HostCommand::OnKeyDown {
                key,
                code,
                timestamp_ms,
            } => {
                self.js.dispatch_key_down(&key, &code, timestamp_ms);
                Ok(())
            }

            HostCommand::OnKeyUp {
                key,
                code,
                timestamp_ms,
            } => {
                self.js.dispatch_key_up(&key, &code, timestamp_ms);
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
                    &ble.device_id,
                    &ble.service_id,
                    &ble.characteristic_id,
                    &ble.value,
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
                let trim = io::image_cache::TrimLevel::from_android(level);
                let freed = io::image_cache::global_cache().trim(trim);
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

            HostCommand::OnUserCaptureScreen => self.js.exec_script(
                "user_capture_screen",
                &format!("{HOST_BRIDGE_EXPR}._internalTriggerUserCaptureScreen()"),
            ),

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

    /// Build the JS snippet that fires the game's `onShow`, embedding launch
    /// options (if any) via `JSON.parse` of a safely escaped string.
    fn build_on_show_script(&self, options_json: Option<&str>) -> String {
        let Some(options_json) = options_json else {
            return format!("{HOST_BRIDGE_EXPR}._internalTriggerOnShow()");
        };
        let options_json = options_json.trim();
        if options_json.is_empty() {
            return format!("{HOST_BRIDGE_EXPR}._internalTriggerOnShow()");
        }
        match deno_core::serde_json::from_str::<Value>(options_json) {
            Ok(value) if value.is_object() => {
                // Round-trip through serde_json::to_string and pass via
                // JSON.parse() with proper JS string escaping. Display on a
                // serde_json::Value is *mostly* JS-safe, but U+2028/U+2029 are
                // valid JSON yet act as line terminators in JS source, so
                // JSON.parse(escaped_string) is universally safe.
                let json_str =
                    deno_core::serde_json::to_string(&value).unwrap_or_else(|_| "{}".to_string());
                let escaped = escape_for_js_string(&json_str);
                format!("{HOST_BRIDGE_EXPR}._internalTriggerOnShow(JSON.parse('{escaped}'))")
            }
            Ok(_) => format!("{HOST_BRIDGE_EXPR}._internalTriggerOnShow()"),
            Err(e) => {
                warn!(
                    "[Host {}] invalid onShow options JSON, fallback to default: {}",
                    self.id, e
                );
                format!("{HOST_BRIDGE_EXPR}._internalTriggerOnShow()")
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
        let _ = self.js.exec_script(
            "window_resize",
            &format!("{HOST_BRIDGE_EXPR}._internalTriggerWindowResize()"),
        );
    }

    fn on_update_surface(&mut self, surface: SurfaceRef) -> EngineResult<()> {
        let (w, h) = surface.size();
        info!(
            "[Host {}] on_update_surface: requested={}x{}",
            self.id, w, h
        );

        // Retry a bounded number of times: a transiently full render command
        // queue can make the bounded-blocking recreate send/reply time out, and a
        // dropped recreate would strand the app on a black frame with no further
        // surface callback from Java. Surface updates are rare, so a few host-
        // thread retries are an acceptable tradeoff for not losing the surface.
        let mut result = self.render.update_surface(surface.clone());
        let mut attempts = 1u32;
        while result.is_err() && attempts < 3 {
            attempts += 1;
            warn!(
                "[Host {}] on_update_surface attempt {} after error: {:?}",
                self.id, attempts, result
            );
            result = self.render.update_surface(surface.clone());
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
                if let Some(script) = self.pending_on_show_script.take() {
                    self.js.set_timer_backgrounded(false);
                    let _ = self.js.exec_script("onshow", &script);
                }
            }
            info!("[Host {}] on_update_surface completed", self.id);
        } else if let Err(ref e) = result {
            warn!("[Host {}] on_update_surface failed: {}", self.id, e);
        }

        result
    }

    async fn on_restart(&mut self) -> EngineResult<()> {
        // Pause subsystems to ensure a clean restart
        self.render.pause();
        self.audio.pause();

        // Recreate JS runtime with fresh state
        let (files_dir, cache_dir) = self.js.get_base_dirs();
        let device_services = self.platform.create_device_services(self.id);

        let host_state = HostOpState {
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

        // ---- Code cache (V8 bytecode persistence) ----
        let shared_cache = code_cache::create_code_cache(self.init_options.cache_dir());

        let loader_mount_ref: js_runtime::SharedMountTableRef =
            Rc::new(std::cell::RefCell::new(None));
        let module_loader: Option<Rc<dyn ModuleLoader>> = Some(Rc::new(MyModuleLoader::new(
            Some(shared_cache.clone()),
            loader_mount_ref.clone(),
        )));

        let ext_code_cache: Option<Rc<dyn deno_core::ExtCodeCache>> =
            Some(code_cache::ExtCodeCacheAdapter::new(shared_cache));

        let extra_ext = self.platform.extensions(&self.init_options);

        // drain_shared_image_cache() returns shared IDs and clears the JS-side
        // bookkeeping (process-global).  We must send DestroyImage for each ID
        // *before* clearing the IO cache — otherwise the render thread holds
        // orphaned GPU textures that no one will ever release. Batch them into a
        // single must-deliver command so a large image set doesn't block restart
        // up to the send deadline per image.
        let shared_ids = js_runtime::drain_shared_image_cache();
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
        io::global_cache().clear();

        // Drop the old watchdog before dropping the runtime.
        #[cfg(feature = "v8-limits")]
        {
            self.watchdog.take();
        }

        // ---- V8 limits config ----
        #[cfg(feature = "v8-limits")]
        let v8_limits = V8LimitsConfig::from_max_memory_mb(self.init_options.max_memory_mb());

        // Close the IO scheduler before dropping the runtime.  This ensures
        // in-flight async IO tasks are rejected immediately rather than racing
        // with the new session's scheduler after the old runtime is gone.
        self.js.close_io_scheduler();

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
            extra_ext,
            module_loader,
            ext_code_cache,
            loader_mount_ref,
            #[cfg(feature = "v8-limits")]
            v8_limits,
            #[cfg(feature = "code-signing")]
            self.init_options.code_signing_enabled(),
            #[cfg(feature = "code-signing")]
            self.init_options.code_signing_pubkey(),
        );

        // Recreate watchdog for the new isolate
        #[cfg(feature = "v8-limits")]
        let mut new_watchdog = None;
        #[cfg(feature = "v8-limits")]
        if self.init_options.watchdog_enabled() {
            let isolate_handle = new_js.isolate_handle();
            let config = WatchdogConfig::default().with_timeout(std::time::Duration::from_secs(
                self.init_options.watchdog_timeout_secs() as u64,
            ));
            match watchdog::spawn_watchdog(isolate_handle, config, self.id as i32) {
                Ok(handle) => new_watchdog = Some(handle),
                Err(e) => {
                    error!(
                        "[Host {}] failed to start watchdog after restart: {} (continuing without watchdog)",
                        self.id, e
                    );
                }
            }
        }

        self.js.set(new_js);
        #[cfg(feature = "v8-limits")]
        {
            self.watchdog = new_watchdog;
        }

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
