use std::{rc::Rc, sync::Arc};

use deno_core::{FsModuleLoader, ModuleLoader, PollEventLoopOptions};
use tracing::{error, info};

use shared::{
    config::InitOptions, error::EngineResult, op_state::HostOpState,
    protocol::host_cmd::HostCommand, surface::SurfaceRef,
};

use crate::{
    runtime::{HostId, loader::MyModuleLoader},
    services::{AudioService, IoService, PlatformServices, RenderService},
};

use js_runtime::HostJsRuntime;

pub(crate) struct Host {
    pub(crate) id: HostId,

    pub(crate) io: IoService,
    pub(crate) audio: AudioService,
    pub(crate) render: RenderService,

    pub(crate) js: HostJsRuntime,
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
        info!("[Host {}] host cleanup complete.", self.id);
    }
}

impl Host {
    pub(crate) fn new(
        id: HostId,
        js_tx: tokio::sync::mpsc::Sender<HostCommand>,
        surface: SurfaceRef,
        platform: Arc<dyn PlatformServices>,
        init_options: InitOptions,
    ) -> EngineResult<Self> {
        // ---- Services ----
        let io = IoService::new()?;
        let audio = AudioService::new(js_tx.clone())?;
        let render = RenderService::new(js_tx.clone(), surface, init_options.pixel_ratio());

        // ---- HostOpState for extensions ----
        let host_state = HostOpState {
            id,
            code_dir: None,
            app_tmp_dir: init_options.tmp_dir().to_path_buf(),
            render_tx: render.sender(),
            io_tx: io.sender(),
            audio_tx: audio.sender(),
        };

        let module_loader: Option<Rc<dyn ModuleLoader>> =
            Some(Rc::new(MyModuleLoader(FsModuleLoader)));

        // ---- Extensions ----
        let extra_ext = platform.extensions(&init_options);

        // ---- JS runtime + bindings cache ----
        let js = HostJsRuntime::new(id as i32, host_state, extra_ext, module_loader);

        Ok(Self { id, render, io, audio, js })
    }

    pub(crate) async fn handle_command(&mut self, cmd: HostCommand) {
        if let Err(e) = self.handle_command_inner(cmd).await {
            error!("[Host {}] handle_command failed: e={} ", self.id, e);
        }
    }

    async fn handle_command_inner(&mut self, cmd: HostCommand) -> EngineResult<()> {
        match cmd {
            HostCommand::EvaluateModule { dir, entry } => self.on_evaluate_module(dir, entry).await,
            HostCommand::EvalScript { source } => self.on_eval_script(source).await,

            HostCommand::OnShow => {
                self.js
                    .exec_script_and_pump("onshow", "_internalTriggerOnShow()".to_string())
                    .await
            }

            HostCommand::OnHide => {
                self.js
                    .exec_script_and_pump("onhide", "_internalTriggerOnHide()".to_string())
                    .await
            }

            HostCommand::OnTouch {
                touch_type,
                points,
                timestamp_ms,
            } => {
                self.js.dispatch_touch(touch_type, &points, timestamp_ms);
                Ok(())
            }

            HostCommand::RequestAnimationFrame(_ts) => {
                self.js.schedule_raf();
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

            _ => Ok(()),
        }
    }

    async fn on_evaluate_module(&mut self, dir: String, entry: String) -> EngineResult<()> {
        self.js.evaluate_module(dir, entry).await?;

        self.js
            .run_event_loop(PollEventLoopOptions::default())
            .await?;

        Ok(())
    }

    async fn on_eval_script(&mut self, source: String) -> EngineResult<()> {
        self.js.exec_script_and_pump("eval-script", source).await?;
        Ok(())
    }

    fn on_update_surface(&mut self, surface: SurfaceRef) -> EngineResult<()> {
        self.render.update_surface(surface)
    }
}
