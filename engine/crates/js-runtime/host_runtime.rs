use std::{path::PathBuf, rc::Rc, sync::Arc};

use deno_core::{
    Extension, JsRuntime, ModuleLoader, PollEventLoopOptions, RuntimeOptions, resolve_path,
};

use shared::{
    error::{EngineError, EngineResult, ErrorCode},
    op_state::HostOpState,
    protocol::host_cmd::{TouchPoint, TouchType},
    vfs::{GamePaths, VirtualFS},
};

use crate::{js_bindings::JsBindings, main_extensions};

pub struct HostJsRuntime {
    host_id: i32,
    rt: JsRuntime,
    bindings: JsBindings,
}

impl HostJsRuntime {
    /// Create a fully initialized JS runtime + bindings cache.
    ///
    /// - `host_state` will be consumed by js-runtime extensions
    /// - `extra_extensions` is for platform-specific extensions (Android, etc.)
    /// - `module_loader` is supplied by core (e.g., MyModuleLoader(FsModuleLoader))
    pub fn new(
        host_id: i32,
        host_state: HostOpState,
        extra_extensions: Vec<Extension>,
        module_loader: Option<Rc<dyn ModuleLoader>>,
    ) -> Self {
        let exts = main_extensions(host_state)
            .into_iter()
            .chain(extra_extensions)
            .collect::<Vec<_>>();

        let mut rt = JsRuntime::new(RuntimeOptions {
            module_loader,
            extensions: exts,
            ..Default::default()
        });

        let bindings = JsBindings::new(&mut rt, host_id);

        Self {
            host_id,
            rt,
            bindings,
        }
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
        self.bindings
            .dispatch_inner_audio_event(&mut self.rt, self.host_id, id, event_type, current_time);
    }

    // ---- Sensor event dispatch (zero-alloc, no JS parsing) ----

    #[inline]
    pub fn dispatch_device_motion(&mut self, alpha: f64, beta: f64, gamma: f64) {
        self.bindings.dispatch_device_motion(&mut self.rt, alpha, beta, gamma);
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
        self.bindings.dispatch_compass(&mut self.rt, direction, accuracy);
    }

    #[inline]
    pub fn dispatch_device_orientation(&mut self, value: &str) {
        self.bindings.dispatch_device_orientation(&mut self.rt, value);
    }

    #[inline]
    pub fn dispatch_network_status(&mut self, is_connected: bool, network_type: &str) {
        self.bindings.dispatch_network_status(&mut self.rt, is_connected, network_type);
    }

    // ---- Recorder event dispatch ----

    pub fn dispatch_recorder_event(&mut self, event_type: &str, json_payload: &str) {
        self.bindings.dispatch_recorder_event(&mut self.rt, self.host_id, event_type, json_payload);
    }

    pub fn dispatch_recorder_frame_data(&mut self, data: &[u8], is_last_frame: bool) {
        self.bindings.dispatch_recorder_frame_data(&mut self.rt, self.host_id, data, is_last_frame);
    }

    // ---- Camera event dispatch ----

    pub fn dispatch_camera_event(&mut self, camera_id: u32, event_type: &str, json_payload: &str) {
        self.bindings.dispatch_camera_event(&mut self.rt, self.host_id, camera_id, event_type, json_payload);
    }

    pub fn dispatch_camera_frame_data(&mut self, camera_id: u32, data: &[u8], width: u32, height: u32) {
        self.bindings.dispatch_camera_frame_data(&mut self.rt, self.host_id, camera_id, data, width, height);
    }

    // ---- Scripts / modules ----

    /// Execute a script without pumping the event loop.
    ///
    /// The op-based RAF (op_await_next_frame) creates a permanently-pending op,
    /// so run_event_loop() would block forever once the RAF loop is active.
    /// The main tokio::select! loop in thread.rs continuously drives the event
    /// loop, handling any resulting microtasks, promises, or timers.
    pub fn exec_script(&mut self, name: &'static str, source: String) -> EngineResult<()> {
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

        let evaluation = self.rt.mod_evaluate(module_id);
        if let Err(e) = evaluation.await {
            return Err(EngineError::new(ErrorCode::ModuleLoadError)
                .with_msg("load main es module")
                .with_detail(e.to_string()));
        }
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
