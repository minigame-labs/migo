use std::{rc::Rc, sync::Arc, time::Instant};

use deno_core::{FsModuleLoader, ModuleLoader};
use tracing::{error, info};

use shared::{
    config::InitOptions, error::EngineResult,
    op_state::{HostOpState, RafRx},
    protocol::host_cmd::HostCommand, surface::SurfaceRef,
};

use crate::{
    runtime::{HostId, loader::MyModuleLoader, vsync},
    services::{AudioService, IoService, PlatformServices, RenderService},
};

use js_runtime::HostJsRuntime;

#[cfg(feature = "v8-limits")]
use js_runtime::V8LimitsConfig;
#[cfg(feature = "v8-limits")]
use crate::runtime::watchdog::{self, WatchdogConfig, WatchdogHandle};

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
        debug_assert!(self.0.is_none(), "JsRuntimeSlot: replacing without dropping first");
        self.0 = Some(js);
    }
}

impl std::ops::Deref for JsRuntimeSlot {
    type Target = HostJsRuntime;
    fn deref(&self) -> &HostJsRuntime {
        self.0.as_ref().expect("[BUG] JsRuntime accessed after drop")
    }
}

impl std::ops::DerefMut for JsRuntimeSlot {
    fn deref_mut(&mut self) -> &mut HostJsRuntime {
        self.0.as_mut().expect("[BUG] JsRuntime accessed after drop")
    }
}

pub(crate) struct Host {
    pub(crate) id: HostId,

    pub(crate) io: IoService,
    pub(crate) audio: AudioService,
    pub(crate) render: RenderService,

    pub(crate) js: JsRuntimeSlot,

    /// Shared RAF receiver — survives JS runtime restarts.
    raf_rx: RafRx,

    /// Sender back to the host event loop (for JS-initiated restart/exit).
    host_tx: tokio::sync::mpsc::Sender<HostCommand>,

    platform: Arc<dyn PlatformServices>,
    init_options: InitOptions,

    last_game_id: Option<String>,
    last_entry: Option<String>,

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
        self.render.shutdown();
        self.audio.shutdown();
        self.io.shutdown();
        vsync::unregister_vsync_sender(self.id);
        shared::stats::unregister_stats(self.id);

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
        host_tx: tokio::sync::mpsc::Sender<HostCommand>,
        surface: SurfaceRef,
        platform: Arc<dyn PlatformServices>,
        init_options: InitOptions,
    ) -> EngineResult<Self> {
        // ---- Startup timing instrumentation ----
        let t_start = Instant::now();

        // ---- RAF channel (render thread → JS async op) ----
        let (raf_tx, raf_rx_raw) = tokio::sync::mpsc::channel::<f64>(2);
        let raf_rx: RafRx = Arc::new(tokio::sync::Mutex::new(raf_rx_raw));

        // ---- VSync channel (Choreographer JNI → render thread) ----
        let (vsync_tx, vsync_rx) = crossbeam_channel::bounded::<f64>(1);
        vsync::register_vsync_sender(id, vsync_tx);

        // ---- Services ----
        // IoService only creates the channel here; the handler task
        // is spawned later inside `runtime.block_on()` by `spawn_handler()`.
        let io = IoService::new();
        // AudioService is lazy — no thread spawned until the first
        // real audio command.  Saves ~80 ms on cold start.
        let audio = AudioService::new(host_tx.clone());
        let render = RenderService::new(
            raf_tx,
            Some(vsync_rx),
            id,
            surface,
            init_options.pixel_ratio(),
        );

        // ---- HostOpState for extensions ----
        let device_services = platform.create_device_services(id);
        let host_state = HostOpState {
            id,
            code_dir: None,
            game_paths: None, // Set when evaluating a module
            vfs: None,        // Set when evaluating a module
            app_cache_dir: init_options.cache_dir().to_path_buf(),
            app_files_dir: init_options.files_dir().to_path_buf(),
            render_tx: render.sender(),
            io_tx: io.sender(),
            audio_tx: audio.sender(),
            host_tx: host_tx.clone(),
            device_services,
            raf_rx: Some(raf_rx.clone()),
        };

        let module_loader: Option<Rc<dyn ModuleLoader>> =
            Some(Rc::new(MyModuleLoader(FsModuleLoader)));

        // ---- Extensions ----
        let extra_ext = platform.extensions(&init_options);

        let t_services = Instant::now();
        info!(
            "[Host {}] services init: {:.1}ms (render + IO + audio channels)",
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
            let config = WatchdogConfig::default()
                .with_timeout(std::time::Duration::from_secs(
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
            io,
            audio,
            js: JsRuntimeSlot::new(js),
            raf_rx,
            host_tx,
            platform,
            init_options,
            last_game_id: None,
            last_entry: None,
            #[cfg(feature = "v8-limits")]
            watchdog,
        })
    }

    pub(crate) async fn handle_command(&mut self, cmd: HostCommand) {
        if let Err(e) = self.handle_command_inner(cmd).await {
            error!("[Host {}] handle_command failed: e={} ", self.id, e);
        }
    }

    async fn handle_command_inner(&mut self, cmd: HostCommand) -> EngineResult<()> {
        match cmd {
            HostCommand::EvaluateModule { game_id, entry } => {
                self.on_evaluate_module(game_id, entry).await
            }
            HostCommand::EvalScript { source } => self.on_eval_script(source),

            HostCommand::Restart => self.on_restart().await,

            HostCommand::OnShow => {
                // Resume audio thread before notifying JS so the game can
                // immediately start playing audio in its onShow callback.
                //
                // The render thread is NOT resumed here. On Android, onResume
                // fires before surfaceCreated, so the old surface is already
                // destroyed at this point. The render thread will be resumed
                // when UpdateSurface arrives with the new valid surface.
                self.audio.resume();

                self.js
                    .exec_script("onshow", "_internalTriggerOnShow()".to_string())
            }

            HostCommand::OnHide => {
                // Pause render and audio threads to save resources while backgrounded.
                // The render thread stops its RAF ticker (no more frames).
                // The audio thread stops processing (no audio output).
                // The host/V8 thread stays alive for timers, network, etc.
                self.render.pause();
                self.audio.pause();

                self.js
                    .exec_script("onhide", "_internalTriggerOnHide()".to_string())
            }

            HostCommand::OnAudioInterruptionBegin => {
                self.js
                    .exec_script(
                        "audio_interruption_begin",
                        "_internalTriggerAudioInterruptionBegin()".to_string(),
                    )
            }

            HostCommand::OnAudioInterruptionEnd => {
                self.js
                    .exec_script(
                        "audio_interruption_end",
                        "_internalTriggerAudioInterruptionEnd()".to_string(),
                    )
            }

            HostCommand::OnTouch {
                touch_type,
                count,
                points,
                timestamp_ms,
            } => {
                self.js.dispatch_touch(touch_type, &points[..count as usize], timestamp_ms);
                Ok(())
            }

            HostCommand::UpdateSurface { surface } => self.on_update_surface(surface),

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

            HostCommand::OnDeviceMotionChange {
                alpha,
                beta,
                gamma,
            } => {
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
                self.js.dispatch_camera_event(camera_id, &event_type, &json_payload);
                Ok(())
            }

            HostCommand::CameraFrameData {
                camera_id,
                data,
                width,
                height,
            } => {
                self.js.dispatch_camera_frame_data(camera_id, &data, width, height);
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

            HostCommand::OnUserCaptureScreen => {
                self.js
                    .exec_script(
                        "user_capture_screen",
                        "_internalTriggerUserCaptureScreen()".to_string(),
                    )
            }

            _ => Ok(()),
        }
    }

    async fn on_evaluate_module(&mut self, game_id: String, entry: String) -> EngineResult<()> {
        let t_eval_start = Instant::now();
        self.last_game_id = Some(game_id.clone());
        self.last_entry = Some(entry.clone());

        self.js.evaluate_module(game_id.clone(), entry.clone()).await?;
        info!(
            "[Host {}] evaluate_module('{}', '{}'): {:.1}ms",
            self.id,
            game_id,
            entry,
            t_eval_start.elapsed().as_secs_f64() * 1000.0,
        );

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
        self.js.exec_script("eval-script", source)
    }

    fn on_update_surface(&mut self, surface: SurfaceRef) -> EngineResult<()> {
        let result = self.render.update_surface(surface);

        // Resume the render thread after the surface is successfully recreated.
        // This handles the Android lifecycle where onResume fires before
        // surfaceCreated: OnHide pauses the render thread, and it stays paused
        // until a valid surface arrives here. If the render thread wasn't paused
        // (e.g., normal orientation change), resume() is a no-op.
        if result.is_ok() {
            self.render.resume();
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
            app_cache_dir: cache_dir,
            app_files_dir: files_dir,
            render_tx: self.render.sender(),
            io_tx: self.io.sender(),
            audio_tx: self.audio.sender(),
            host_tx: self.host_tx.clone(),
            device_services,
            raf_rx: Some(self.raf_rx.clone()),
        };

        let module_loader: Option<Rc<dyn ModuleLoader>> =
            Some(Rc::new(MyModuleLoader(FsModuleLoader)));

        let extra_ext = self.platform.extensions(&self.init_options);

        // Clear process-global caches before recreating runtime.
        js_runtime::clear_shared_image_cache();
        io::global_cache().clear();

        // Drop the old watchdog before dropping the runtime.
        #[cfg(feature = "v8-limits")]
        {
            self.watchdog.take();
        }

        // ---- V8 limits config ----
        #[cfg(feature = "v8-limits")]
        let v8_limits = V8LimitsConfig::from_max_memory_mb(self.init_options.max_memory_mb());

        // CRITICAL: Drop the old JsRuntime BEFORE creating the new one.
        // Two v8 isolates on the same thread during drop cleanup causes
        // "Cannot create a handle without a HandleScope" crash — the old
        // isolate's cleanup handler can't create a HandleScope when v8's
        // thread-local state was modified by the new isolate's initialization.
        self.js.take_and_drop();
        let mut new_js = HostJsRuntime::new(
            self.id as i32,
            host_state,
            extra_ext,
            module_loader,
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
            let config = WatchdogConfig::default()
                .with_timeout(std::time::Duration::from_secs(
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

        // If we have a last evaluated module, reload it
        if let (Some(game_id), Some(entry)) = (self.last_game_id.clone(), self.last_entry.clone()) {
            self.on_evaluate_module(game_id, entry).await?;
        }

        // Resume render and audio so the new runtime can start producing frames.
        //
        // Pause sets `has_surface = false` on the render thread (designed for
        // the OnHide flow where the Android surface is destroyed). In the normal
        // OnHide→OnShow→UpdateSurface flow, `RecreateOnscreen` restores it.
        // But restart doesn't go through UpdateSurface (the surface is unchanged),
        // so we must explicitly re-signal the surface to restore `has_surface`.
        // Without this, VSync frames are discarded and the RAF loop never fires.
        if let Err(e) = self.render.restore_surface() {
            error!("[Host {}] on_restart: restore_surface failed: {}", self.id, e);
        }
        self.render.resume();
        self.audio.resume();

        Ok(())
    }
}
