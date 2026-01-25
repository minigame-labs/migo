use std::{path::Path, rc::Rc};

use deno_core::{
    Extension, JsRuntime, ModuleLoader, PollEventLoopOptions, RuntimeOptions, resolve_path,
};

use shared::{
    error::{EngineError, EngineResult, ErrorCode},
    op_state::HostOpState,
    protocol::host_cmd::{TouchPoint, TouchType},
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

    pub fn reload_bindings(&mut self) {
        self.bindings.reload(&mut self.rt, self.host_id);
    }

    // ---- JS global calls ----

    pub fn schedule_raf(&mut self) {
        self.bindings.call_schedule_raf(&mut self.rt);
    }

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

    // ---- Scripts / modules ----

    pub async fn exec_script_and_pump(
        &mut self,
        name: &'static str,
        source: String,
    ) -> EngineResult<()> {
        self.rt.execute_script(name, source).map_err(|e| {
            EngineError::new(ErrorCode::JsException)
                .with_msg(name)
                .with_detail(e.to_string())
        })?;

        self.rt
            .run_event_loop(PollEventLoopOptions::default())
            .await
            .map_err(|e| {
                EngineError::new(ErrorCode::JsException)
                    .with_msg(name)
                    .with_detail(e.to_string())
            })?;

        Ok(())
    }

    pub async fn evaluate_module(&mut self, dir: String, entry: String) -> EngineResult<()> {
        let resolved = resolve_path(&entry, Path::new(&dir)).map_err(|e| {
            EngineError::new(ErrorCode::InvalidArgument)
                .with_msg("resolve module path")
                .with_detail(e.to_string())
        })?;

        let module_id = self.rt.load_main_es_module(&resolved).await.map_err(|e| {
            EngineError::new(ErrorCode::ModuleLoadError)
                .with_msg("load main es module")
                .with_detail(e.to_string())
        })?;

        self.set_code_dir(Some(dir));

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
