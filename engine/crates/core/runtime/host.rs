use std::{
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

use deno_core::serde_json::Value;
use deno_core::{FsModuleLoader, ModuleLoader};
use tracing::{error, info, warn};

use shared::{
    config::InitOptions,
    error::EngineResult,
    js_escape::escape_for_js_string,
    op_state::{HostOpState, RafRx},
    protocol::host_cmd::HostCommand,
    surface::SurfaceRef,
};

use crate::{
    runtime::{HostId, loader::MyModuleLoader, vsync},
    services::{AudioService, IoService, PlatformServices, RenderService},
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
    network_policy: shared::op_state::NetworkPolicy,

    last_game_id: Option<String>,
    last_entry: Option<String>,

    /// Shared flag: `true` while the app is backgrounded (OnHide).
    /// Network polling ops check this to throttle CPU usage.
    backgrounded: Arc<AtomicBool>,

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
        let (vsync_tx, vsync_rx) = crossbeam_channel::bounded::<f64>(2);
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
        )?;

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
            sub_packages: init_options.sub_packages().to_vec(),
            workers_path: init_options.workers_path().map(|s| s.to_string()),
            network_policy: network_policy.clone(),
            backgrounded: backgrounded.clone(),
        };

        // ---- Console log buffer (debug only) ----
        if init_options.debug_enabled() {
            shared::console_log::register_console_log(id);
        }

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
            io,
            audio,
            js: JsRuntimeSlot::new(js),
            raf_rx,
            host_tx,
            platform,
            init_options,
            network_policy,
            last_game_id: None,
            last_entry: None,
            backgrounded,
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

            HostCommand::OnShow { options_json } => {
                // Mark foreground so network polling ops resume normal rate.
                self.backgrounded.store(false, Ordering::Relaxed);

                // Resume audio thread before notifying JS so the game can
                // immediately start playing audio in its onShow callback.
                //
                // The render thread is NOT resumed here. On Android, onResume
                // fires before surfaceCreated, so the old surface is already
                // destroyed at this point. The render thread will be resumed
                // when UpdateSurface arrives with the new valid surface.
                self.audio.resume();

                let script = if let Some(options_json) = options_json.as_deref() {
                    let options_json = options_json.trim();
                    if options_json.is_empty() {
                        "_internalTriggerOnShow()".to_string()
                    } else {
                        match deno_core::serde_json::from_str::<Value>(options_json) {
                            Ok(value) if value.is_object() => {
                                // Serialize back through serde_json::to_string and pass
                                // via JSON.parse() with proper JS string escaping.
                                // Using Display on serde_json::Value is *mostly* JS-safe,
                                // but edge cases exist (U+2028/U+2029 are valid JSON but
                                // act as line terminators in JS source). Going through
                                // JSON.parse(escaped_string) is universally safe.
                                let json_str = deno_core::serde_json::to_string(&value)
                                    .unwrap_or_else(|_| "{}".to_string());
                                let escaped = escape_for_js_string(&json_str);
                                format!("_internalTriggerOnShow(JSON.parse('{}'))", escaped)
                            }
                            Ok(_) => "_internalTriggerOnShow()".to_string(),
                            Err(e) => {
                                warn!(
                                    "[Host {}] invalid onShow options JSON, fallback to default: {}",
                                    self.id, e
                                );
                                "_internalTriggerOnShow()".to_string()
                            }
                        }
                    }
                } else {
                    "_internalTriggerOnShow()".to_string()
                };

                self.js.exec_script("onshow", &script)
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

                self.js.exec_script("onhide", "_internalTriggerOnHide()")
            }

            HostCommand::OnAudioInterruptionBegin => self.js.exec_script(
                "audio_interruption_begin",
                "_internalTriggerAudioInterruptionBegin()",
            ),

            HostCommand::OnAudioInterruptionEnd => self.js.exec_script(
                "audio_interruption_end",
                "_internalTriggerAudioInterruptionEnd()",
            ),

            HostCommand::OnTouch(touch) => {
                let count = (touch.count as usize).min(touch.points.len());
                self.js
                    .dispatch_touch(touch.touch_type, &touch.points[..count], touch.timestamp_ms);
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
                    .dispatch_camera_frame_data(camera_id, &data, width, height);
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
                self.js.dispatch_memory_warning(level);
                Ok(())
            }

            HostCommand::OnUserCaptureScreen => self
                .js
                .exec_script("user_capture_screen", "_internalTriggerUserCaptureScreen()"),

            HostCommand::OnVideoStateChange {
                video_id,
                event_type,
                data,
            } => {
                self.js
                    .dispatch_video_event(video_id, &event_type, &data);
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

    fn on_update_surface(&mut self, surface: SurfaceRef) -> EngineResult<()> {
        let (w, h) = surface.size();
        info!(
            "[Host {}] on_update_surface: requested={}x{}",
            self.id, w, h
        );

        let result = self.render.update_surface(surface);

        // Resume the render thread after the surface is successfully recreated.
        // This handles the Android lifecycle where onResume fires before
        // surfaceCreated: OnHide pauses the render thread, and it stays paused
        // until a valid surface arrives here. If the render thread wasn't paused
        // (e.g., normal orientation change), resume() is a no-op.
        if result.is_ok() {
            self.render.resume();
            let _ = self
                .js
                .exec_script("window_resize", "_internalTriggerWindowResize()");
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
            app_cache_dir: cache_dir,
            app_files_dir: files_dir,
            render_tx: self.render.sender(),
            io_tx: self.io.sender(),
            audio_tx: self.audio.sender(),
            host_tx: self.host_tx.clone(),
            device_services,
            raf_rx: Some(self.raf_rx.clone()),
            sub_packages: self.init_options.sub_packages().to_vec(),
            workers_path: self.init_options.workers_path().map(|s| s.to_string()),
            network_policy: self.network_policy.clone(),
            backgrounded: self.backgrounded.clone(),
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

        // If we have a last evaluated module, reload it
        if let (Some(game_id), Some(entry)) = (self.last_game_id.take(), self.last_entry.take()) {
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
        //
        // Synchronization note: `restore_surface()` delegates to
        // `update_surface()` which sends a `RenderCommand::Canvas(RecreateOnscreen)`
        // through the crossbeam command channel and waits for the render thread's
        // response (bounded channel recv with timeout). The render thread processes
        // this command, sets its local `has_surface = true`, and sends back the
        // result. This request-response exchange over the command channel provides
        // the necessary cross-thread synchronization -- the host thread does not
        // proceed to `resume()` until the render thread has acknowledged the
        // surface restoration.
        if let Err(e) = self.render.restore_surface() {
            error!(
                "[Host {}] on_restart: restore_surface failed: {}",
                self.id, e
            );
        }
        self.render.resume();
        self.audio.resume();

        Ok(())
    }
}

