use std::{path::PathBuf, rc::Rc, sync::Arc, time::Instant};

use deno_core::{
    Extension, JsRuntime, ModuleLoader, PollEventLoopOptions, RuntimeOptions, resolve_path, v8,
};

use shared::{
    error::{EngineError, EngineResult, ErrorCode},
    op_state::HostOpState,
    protocol::host_cmd::{TouchPoint, TouchType},
    vfs::{GamePaths, VirtualFS},
};

#[cfg(feature = "code-signing")]
use shared::vfs::integrity::IntegrityVerifier;

use crate::{js_bindings::JsBindings, main_extensions};

/// Configuration for V8 heap limits and execution watchdog.
///
/// When `v8-limits` feature is enabled, these settings are applied during
/// runtime creation to protect the host process from runaway JS code.
#[derive(Debug, Clone)]
pub struct V8LimitsConfig {
    /// Maximum V8 heap size in bytes. Default: 256 MB.
    pub max_heap_size: usize,
    /// Initial V8 heap size in bytes. 0 = V8 default.
    pub initial_heap_size: usize,
}

impl Default for V8LimitsConfig {
    fn default() -> Self {
        Self {
            max_heap_size: 256 * 1024 * 1024, // 256 MB
            initial_heap_size: 0,             // V8 default
        }
    }
}

impl V8LimitsConfig {
    /// Create config from `InitOptions.max_memory_mb`.
    pub fn from_max_memory_mb(mb: i32) -> Self {
        let mb = mb.clamp(64, 2048) as usize;
        Self {
            max_heap_size: mb * 1024 * 1024,
            initial_heap_size: 0,
        }
    }
}

pub struct HostJsRuntime {
    host_id: i32,
    rt: JsRuntime,
    bindings: JsBindings,
    /// Shared termination state for OOM callback + watchdog integration.
    /// Only present when `v8-limits` feature is enabled.
    #[cfg(feature = "v8-limits")]
    oom_terminated: Arc<std::sync::atomic::AtomicBool>,
    /// Code integrity verifier (Ed25519 + SHA256).
    /// Present when code signing is enabled and configured correctly.
    #[cfg(feature = "code-signing")]
    integrity_verifier: Option<IntegrityVerifier>,
    /// Sticky configuration error for code signing.
    ///
    /// We don't fail runtime construction to keep API shape unchanged, but
    /// module evaluation will fail closed if this is set.
    #[cfg(feature = "code-signing")]
    code_signing_error: Option<EngineError>,
    /// Whether code signing enforcement is enabled for this runtime.
    #[cfg(feature = "code-signing")]
    code_signing_enabled: bool,
}

impl HostJsRuntime {
    /// Create a fully initialized JS runtime + bindings cache.
    ///
    /// - `host_state` will be consumed by js-runtime extensions
    /// - `extra_extensions` is for platform-specific extensions (Android, etc.)
    /// - `module_loader` is supplied by core (e.g., MyModuleLoader(FsModuleLoader))
    /// - `v8_limits` configures heap limits when `v8-limits` feature is enabled
    pub fn new(
        host_id: i32,
        host_state: HostOpState,
        extra_extensions: Vec<Extension>,
        module_loader: Option<Rc<dyn ModuleLoader>>,
        #[cfg(feature = "v8-limits")] v8_limits: V8LimitsConfig,
        #[cfg(feature = "code-signing")] code_signing_enabled: bool,
        #[cfg(feature = "code-signing")] code_signing_pubkey: Option<&str>,
    ) -> Self {
        // V8 startup snapshot support.
        //
        // When a snapshot is available, we use lazy_extensions() to create
        // extensions with JS already captured in the snapshot (no re-parsing).
        // The state callbacks are deferred and applied via
        // lazy_init_extensions() after the runtime is created.
        //
        // When no snapshot is available, we fall back to main_extensions()
        // which creates fully-initialized extensions with JS from source.
        let t0 = Instant::now();
        let snapshot_bytes = crate::snapshot::SNAPSHOT_BYTES;
        let use_snapshot = snapshot_bytes.is_some();

        let exts = if use_snapshot {
            tracing::info!(
                "[Host {}] using V8 startup snapshot ({} bytes)",
                host_id,
                snapshot_bytes.map(|b| b.len()).unwrap_or(0)
            );
            crate::snapshot::lazy_extensions()
                .into_iter()
                .chain(extra_extensions)
                .collect::<Vec<_>>()
        } else {
            tracing::info!("[Host {}] no snapshot, loading JS from source", host_id);
            main_extensions(host_state.clone())
                .into_iter()
                .chain(extra_extensions)
                .collect::<Vec<_>>()
        };
        let t_exts = Instant::now();
        tracing::info!(
            "[Host {}] extensions assembled: {:.1}ms (snapshot={})",
            host_id,
            t_exts.duration_since(t0).as_secs_f64() * 1000.0,
            use_snapshot,
        );

        // Build create_params with heap limits when feature is enabled
        #[cfg(feature = "v8-limits")]
        let create_params = {
            Some(
                v8::Isolate::create_params()
                    .heap_limits(v8_limits.initial_heap_size, v8_limits.max_heap_size),
            )
        };
        #[cfg(not(feature = "v8-limits"))]
        let create_params: Option<v8::CreateParams> = None;

        let t_rt_start = Instant::now();
        let mut rt = JsRuntime::new(RuntimeOptions {
            module_loader,
            extensions: exts,
            create_params,
            startup_snapshot: snapshot_bytes,
            ..Default::default()
        });
        let t_rt_created = Instant::now();
        tracing::info!(
            "[Host {}] JsRuntime::new: {:.1}ms",
            host_id,
            t_rt_created.duration_since(t_rt_start).as_secs_f64() * 1000.0,
        );

        // When using snapshot, apply deferred state callbacks now
        if use_snapshot {
            let ext_args = crate::snapshot::extension_args(host_state);
            if let Err(e) = rt.lazy_init_extensions(ext_args) {
                tracing::error!(
                    "[Host {}] failed to init extensions from snapshot: {}, falling back",
                    host_id,
                    e
                );
                // NOTE: In a production scenario, we'd recreate without snapshot.
                // For now, the error is logged and the runtime continues (ops
                // without state will panic when called — this is a build/deploy
                // mismatch that should be caught in CI).
            }
            tracing::info!(
                "[Host {}] lazy_init_extensions: {:.1}ms",
                host_id,
                t_rt_created.elapsed().as_secs_f64() * 1000.0,
            );
        }

        // Register near-heap-limit callback that terminates execution on OOM
        #[cfg(feature = "v8-limits")]
        let oom_terminated = {
            let terminated = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let cb_terminated = Arc::clone(&terminated);
            let cb_handle = rt.v8_isolate().thread_safe_handle();
            let hard_cap = v8_limits.max_heap_size.saturating_add(8 * 1024 * 1024);

            rt.add_near_heap_limit_callback(move |current_limit, _initial_limit| {
                // Mark OOM once, then terminate execution. Keep a tiny
                // bounded headroom instead of unbounded growth.
                let first = cb_terminated
                    .compare_exchange(
                        false,
                        true,
                        std::sync::atomic::Ordering::SeqCst,
                        std::sync::atomic::Ordering::SeqCst,
                    )
                    .is_ok();
                if first {
                    cb_handle.terminate_execution();
                }
                current_limit.saturating_add(1024 * 1024).min(hard_cap)
            });

            terminated
        };

        // Set thread-local host ID so op_console can route logs to the
        // per-session ring buffer without accessing OpState.
        crate::console::set_thread_host_id(host_id);

        let bindings = JsBindings::new(&mut rt, host_id);
        tracing::info!(
            "[Host {}] HostJsRuntime::new total: {:.1}ms",
            host_id,
            t0.elapsed().as_secs_f64() * 1000.0,
        );

        // Initialize code signing verifier from hex-encoded public key.
        // Enforced fail-closed: when code signing is enabled but misconfigured,
        // module load returns a deterministic signature/config error.
        #[cfg(feature = "code-signing")]
        let (integrity_verifier, code_signing_error) = if code_signing_enabled {
            match code_signing_pubkey {
                Some(hex_key) if !hex_key.is_empty() => {
                    match IntegrityVerifier::from_hex_pubkey(hex_key) {
                        Ok(v) => {
                            tracing::info!("[Host {}] code signing enabled", host_id);
                            (Some(v), None)
                        }
                        Err(e) => {
                            tracing::error!("[Host {}] code signing key invalid: {}", host_id, e);
                            (None, Some(e))
                        }
                    }
                }
                _ => {
                    let err = EngineError::new(ErrorCode::CodeSignatureInvalid)
                        .with_msg("code signing enabled but public key is missing")
                        .with_detail(
                            "set InitOptions.code_signing_pubkey (hex Ed25519 public key)",
                        );
                    tracing::error!("[Host {}] {}", host_id, err);
                    (None, Some(err))
                }
            }
        } else {
            tracing::info!("[Host {}] code signing disabled by init options", host_id);
            (None, None)
        };

        Self {
            host_id,
            rt,
            bindings,
            #[cfg(feature = "v8-limits")]
            oom_terminated,
            #[cfg(feature = "code-signing")]
            integrity_verifier,
            #[cfg(feature = "code-signing")]
            code_signing_error,
            #[cfg(feature = "code-signing")]
            code_signing_enabled,
        }
    }

    /// Get a thread-safe handle to the V8 isolate for cross-thread termination.
    pub fn isolate_handle(&mut self) -> v8::IsolateHandle {
        self.rt.v8_isolate().thread_safe_handle()
    }

    /// Check if the runtime was terminated due to OOM (near-heap-limit callback).
    #[cfg(feature = "v8-limits")]
    pub fn was_oom_terminated(&self) -> bool {
        self.oom_terminated
            .load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Reset OOM termination state. Called after `cancel_terminate_execution`
    /// during restart to allow the new runtime to operate normally.
    #[cfg(feature = "v8-limits")]
    pub fn reset_oom_state(&self) {
        self.oom_terminated
            .store(false, std::sync::atomic::Ordering::SeqCst);
    }

    /// Access HostOpState for mutation.
    pub fn update_host_op_state<F>(&mut self, updater: F)
    where
        F: FnOnce(&mut HostOpState),
    {
        let op_state_rc = self.rt.op_state();
        let mut op_state = op_state_rc.borrow_mut();
        updater(op_state.borrow_mut::<HostOpState>());
    }

    pub fn set_code_dir(&mut self, dir: Option<String>) {
        self.update_host_op_state(|s| s.code_dir = dir);
    }

    /// Set the VirtualFS for sandboxed file access.
    pub fn set_vfs(&mut self, vfs: Option<Arc<VirtualFS>>) {
        self.update_host_op_state(|s| s.vfs = vfs);
    }

    /// Set the GamePaths for the current game.
    pub fn set_game_paths(&mut self, paths: Option<Arc<GamePaths>>) {
        self.update_host_op_state(|s| s.game_paths = paths);
    }

    /// Read HostOpState values.
    pub fn get_base_dirs(&self) -> (PathBuf, PathBuf) {
        let op_state_rc = self.rt.op_state();
        let op_state = op_state_rc.borrow();
        let host = op_state.borrow::<HostOpState>();
        (host.app_files_dir.clone(), host.app_cache_dir.clone())
    }

    pub fn reload_bindings(&mut self) {
        self.bindings.reload(&mut self.rt, self.host_id);
    }

    // ---- JS global calls ----

    pub fn dispatch_touch(
        &mut self,
        touch_type: TouchType,
        points: &[TouchPoint],
        timestamp_ms: i64,
    ) {
        self.bindings
            .dispatch_touch(&mut self.rt, self.host_id, touch_type, points, timestamp_ms);
    }

    pub fn dispatch_inner_audio_event(&mut self, id: u32, event_type: &str, current_time: f64) {
        self.bindings.dispatch_inner_audio_event(
            &mut self.rt,
            self.host_id,
            id,
            event_type,
            current_time,
        );
    }

    // ---- Sensor event dispatch (zero-alloc, no JS parsing) ----

    #[inline]
    pub fn dispatch_device_motion(&mut self, alpha: f64, beta: f64, gamma: f64) {
        self.bindings
            .dispatch_device_motion(&mut self.rt, alpha, beta, gamma);
    }

    #[inline]
    pub fn dispatch_gyroscope(&mut self, x: f64, y: f64, z: f64) {
        self.bindings.dispatch_gyroscope(&mut self.rt, x, y, z);
    }

    #[inline]
    pub fn dispatch_accelerometer(&mut self, x: f64, y: f64, z: f64) {
        self.bindings.dispatch_accelerometer(&mut self.rt, x, y, z);
    }

    #[inline]
    pub fn dispatch_compass(&mut self, direction: f64, accuracy: &str) {
        self.bindings
            .dispatch_compass(&mut self.rt, direction, accuracy);
    }

    #[inline]
    pub fn dispatch_device_orientation(&mut self, value: &str) {
        self.bindings
            .dispatch_device_orientation(&mut self.rt, value);
    }

    #[inline]
    pub fn dispatch_network_status(&mut self, is_connected: bool, network_type: &str) {
        self.bindings
            .dispatch_network_status(&mut self.rt, is_connected, network_type);
    }

    // ---- Recorder event dispatch ----

    pub fn dispatch_recorder_event(&mut self, event_type: &str, json_payload: &str) {
        self.bindings
            .dispatch_recorder_event(&mut self.rt, self.host_id, event_type, json_payload);
    }

    pub fn dispatch_recorder_frame_data(&mut self, data: &[u8], is_last_frame: bool) {
        self.bindings
            .dispatch_recorder_frame_data(&mut self.rt, self.host_id, data, is_last_frame);
    }

    // ---- Camera event dispatch ----

    pub fn dispatch_camera_event(&mut self, camera_id: u32, event_type: &str, json_payload: &str) {
        self.bindings.dispatch_camera_event(
            &mut self.rt,
            self.host_id,
            camera_id,
            event_type,
            json_payload,
        );
    }

    pub fn dispatch_camera_frame_data(
        &mut self,
        camera_id: u32,
        data: &[u8],
        width: u32,
        height: u32,
    ) {
        self.bindings.dispatch_camera_frame_data(
            &mut self.rt,
            self.host_id,
            camera_id,
            data,
            width,
            height,
        );
    }

    // ---- Bluetooth event dispatch ----

    #[inline]
    pub fn dispatch_bluetooth_adapter_state_change(&mut self, available: bool, discovering: bool) {
        self.bindings
            .dispatch_bluetooth_adapter_state_change(&mut self.rt, available, discovering);
    }

    #[inline]
    pub fn dispatch_bluetooth_device_found(&mut self, devices_json: &str) {
        self.bindings
            .dispatch_bluetooth_device_found(&mut self.rt, devices_json);
    }

    #[inline]
    pub fn dispatch_beacon_update(&mut self, beacons_json: &str) {
        self.bindings
            .dispatch_beacon_update(&mut self.rt, beacons_json);
    }

    #[inline]
    pub fn dispatch_beacon_service_change(&mut self, available: bool, discovering: bool) {
        self.bindings
            .dispatch_beacon_service_change(&mut self.rt, available, discovering);
    }

    // ---- BLE GATT event dispatch ----

    #[inline]
    pub fn dispatch_ble_connection_state_change(&mut self, device_id: &str, connected: bool) {
        self.bindings
            .dispatch_ble_connection_state_change(&mut self.rt, device_id, connected);
    }

    #[inline]
    pub fn dispatch_ble_characteristic_value_change(
        &mut self,
        device_id: &str,
        service_id: &str,
        characteristic_id: &str,
        value: &[u8],
    ) {
        self.bindings.dispatch_ble_characteristic_value_change(
            &mut self.rt,
            device_id,
            service_id,
            characteristic_id,
            value,
        );
    }

    #[inline]
    pub fn dispatch_ble_mtu_change(&mut self, device_id: &str, mtu: u32) {
        self.bindings
            .dispatch_ble_mtu_change(&mut self.rt, device_id, mtu);
    }

    // ---- Memory warning dispatch ----

    #[inline]
    pub fn dispatch_memory_warning(&mut self, level: i32) {
        self.bindings.dispatch_memory_warning(&mut self.rt, level);
    }

    // ---- Keyboard event dispatch ----

    #[inline]
    pub fn dispatch_keyboard_input(&mut self, value: &str) {
        self.bindings.dispatch_keyboard_input(&mut self.rt, value);
    }

    #[inline]
    pub fn dispatch_keyboard_height_change(&mut self, height: f64) {
        self.bindings
            .dispatch_keyboard_height_change(&mut self.rt, height);
    }

    #[inline]
    pub fn dispatch_keyboard_confirm(&mut self, value: &str) {
        self.bindings.dispatch_keyboard_confirm(&mut self.rt, value);
    }

    #[inline]
    pub fn dispatch_keyboard_complete(&mut self, value: &str) {
        self.bindings
            .dispatch_keyboard_complete(&mut self.rt, value);
    }

    #[inline]
    pub fn dispatch_key_down(&mut self, key: &str, code: &str, timestamp_ms: f64) {
        self.bindings
            .dispatch_key_down(&mut self.rt, key, code, timestamp_ms);
    }

    #[inline]
    pub fn dispatch_key_up(&mut self, key: &str, code: &str, timestamp_ms: f64) {
        self.bindings
            .dispatch_key_up(&mut self.rt, key, code, timestamp_ms);
    }

    // ---- Video event dispatch ----

    #[inline]
    pub fn dispatch_video_event(&mut self, video_id: u32, event_type: &str, data: &str) {
        self.bindings
            .dispatch_video_event(&mut self.rt, video_id, event_type, data);
    }

    // ---- Scripts / modules ----

    /// Execute a script without pumping the event loop.
    ///
    /// The op-based RAF (op_await_next_frame) creates a permanently-pending op,
    /// so run_event_loop() would block forever once the RAF loop is active.
    /// The main tokio::select! loop in thread.rs continuously drives the event
    /// loop, handling any resulting microtasks, promises, or timers.
    pub fn exec_script(&mut self, name: &'static str, source: &str) -> EngineResult<()> {
        let source = deno_core::FastString::from(source.to_string());
        self.rt.execute_script(name, source).map_err(|e| {
            EngineError::new(ErrorCode::JsException)
                .with_msg(name)
                .with_detail(e.to_string())
        })?;

        Ok(())
    }

    /// Evaluate a module with VFS sandboxing.
    ///
    /// Creates game-specific paths from the game_id and base directories,
    /// then sets up the VFS for sandboxed file access.
    ///
    /// # Arguments
    /// * `game_id` - Unique game identifier (1-64 alphanumeric, underscore, hyphen)
    /// * `entry` - Entry point file (e.g., "main.js")
    pub async fn evaluate_module(&mut self, game_id: String, entry: String) -> EngineResult<()> {
        let t_eval = Instant::now();
        // Get base directories from HostOpState
        let (files_dir, cache_dir) = self.get_base_dirs();

        // Create GamePaths from base dirs + game_id
        let game_paths = GamePaths::new(&files_dir, &cache_dir, &game_id).map_err(|e| {
            EngineError::new(ErrorCode::InvalidArgument)
                .with_msg("create game paths")
                .with_detail(e.to_string())
        })?;

        // Ensure directories exist
        game_paths.ensure_directories().map_err(|e| {
            EngineError::new(ErrorCode::IoError)
                .with_msg("create game directories")
                .with_detail(e.to_string())
        })?;

        // Create VFS from game paths
        let vfs = VirtualFS::from_game_paths(&game_paths);
        let code_dir = game_paths.code_dir().to_path_buf();
        let code_dir_str = code_dir.to_string_lossy().into_owned();
        let game_paths = Arc::new(game_paths);

        // ---- Code signing verification (before loading any JS) ----
        #[cfg(feature = "code-signing")]
        if self.code_signing_enabled {
            if let Some(err) = &self.code_signing_error {
                return Err(err.clone());
            }
            let verifier = self.integrity_verifier.as_mut().ok_or_else(|| {
                EngineError::new(ErrorCode::CodeSignatureInvalid)
                    .with_msg("code signing verifier is not initialized")
            })?;
            tracing::info!(
                "[Host {}] verifying code integrity for entry '{}'",
                self.host_id,
                entry
            );
            let manifest = verifier.verify_entry(&code_dir, &entry).map_err(|e| {
                tracing::error!(
                    "[Host {}] code integrity verification failed: {}",
                    self.host_id,
                    e
                );
                e
            })?;
            // P1: Full-package verification — ensure every file listed in the
            // manifest matches its declared SHA256 hash, not just the entry.
            verifier
                .verify_all_files(&code_dir, &manifest)
                .map_err(|e| {
                    tracing::error!(
                        "[Host {}] full package integrity verification failed: {}",
                        self.host_id,
                        e
                    );
                    e
                })?;
            tracing::info!(
                "[Host {}] code integrity verification passed ({} files)",
                self.host_id,
                manifest.files.len()
            );
        }

        // Store paths and VFS in op state
        self.set_game_paths(Some(game_paths));
        self.set_vfs(Some(Arc::new(vfs)));
        self.set_code_dir(Some(code_dir_str));

        // Resolve and load module
        let resolved = resolve_path(&entry, &code_dir).map_err(|e| {
            EngineError::new(ErrorCode::InvalidArgument)
                .with_msg("resolve module path")
                .with_detail(e.to_string())
        })?;

        let module_id = self.rt.load_main_es_module(&resolved).await.map_err(|e| {
            EngineError::new(ErrorCode::ModuleLoadError)
                .with_msg("load main es module")
                .with_detail(e.to_string())
        })?;

        let t_mod_loaded = Instant::now();
        tracing::info!(
            "[Host {}] module loaded: {:.1}ms",
            self.host_id,
            t_mod_loaded.duration_since(t_eval).as_secs_f64() * 1000.0,
        );

        // Keep pumping the event loop until module evaluation actually resolves.
        //
        // `run_event_loop()` may return early while module evaluation is still pending
        // (for example with top-level await waiting on later callbacks). Returning on
        // the first `run_event_loop()` completion would leave the module half-initialized.
        let mut evaluation = std::pin::pin!(self.rt.mod_evaluate(module_id));
        loop {
            tokio::select! {
                result = &mut evaluation => {
                    if let Err(e) = result {
                        return Err(EngineError::new(ErrorCode::ModuleLoadError)
                            .with_msg("load main es module")
                            .with_detail(e.to_string()));
                    }
                    break;
                }
                result = self.rt.run_event_loop(PollEventLoopOptions::default()) => {
                    if let Err(e) = result {
                        return Err(EngineError::new(ErrorCode::JsException)
                            .with_msg("event loop error during module evaluation")
                            .with_detail(e.to_string()));
                    }
                }
            }
        }
        tracing::info!(
            "[Host {}] module evaluated: {:.1}ms (total evaluate_module={:.1}ms)",
            self.host_id,
            t_mod_loaded.elapsed().as_secs_f64() * 1000.0,
            t_eval.elapsed().as_secs_f64() * 1000.0,
        );
        Ok(())
    }

    /// Run one tick of the event loop (used by host thread main loop).
    pub async fn run_event_loop(&mut self, opt: PollEventLoopOptions) -> EngineResult<()> {
        self.rt.run_event_loop(opt).await.map_err(|e| {
            EngineError::new(ErrorCode::JsException)
                .with_msg("run_event_loop")
                .with_detail(e.to_string())
        })
    }
}
