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

    platform: Arc<dyn PlatformServices>,
    init_options: InitOptions,

    last_game_id: Option<String>,
    last_entry: Option<String>,
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
            device_services,
        };

        let module_loader: Option<Rc<dyn ModuleLoader>> =
            Some(Rc::new(MyModuleLoader(FsModuleLoader)));

        // ---- Extensions ----
        let extra_ext = platform.extensions(&init_options);

        // ---- JS runtime + bindings cache ----
        let js = HostJsRuntime::new(id as i32, host_state, extra_ext, module_loader);

        Ok(Self {
            id,
            render,
            io,
            audio,
            js,
            platform,
            init_options,
            last_game_id: None,
            last_entry: None,
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
            HostCommand::EvalScript { source } => self.on_eval_script(source).await,

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
                    .exec_script_and_pump("onshow", "_internalTriggerOnShow()".to_string())
                    .await
            }

            HostCommand::OnHide => {
                // Pause render and audio threads to save resources while backgrounded.
                // The render thread stops its RAF ticker (no more frames).
                // The audio thread stops processing (no audio output).
                // The host/V8 thread stays alive for timers, network, etc.
                self.render.pause();
                self.audio.pause();

                self.js
                    .exec_script_and_pump("onhide", "_internalTriggerOnHide()".to_string())
                    .await
            }

            HostCommand::OnAudioInterruptionBegin => {
                self.js
                    .exec_script_and_pump(
                        "audio_interruption_begin",
                        "_internalTriggerAudioInterruptionBegin()".to_string(),
                    )
                    .await
            }

            HostCommand::OnAudioInterruptionEnd => {
                self.js
                    .exec_script_and_pump(
                        "audio_interruption_end",
                        "_internalTriggerAudioInterruptionEnd()".to_string(),
                    )
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

            HostCommand::OnDeviceMotionChange {
                alpha,
                beta,
                gamma,
            } => {
                self.js
                    .exec_script_and_pump(
                        "device_motion",
                        format!(
                            "_internalTriggerDeviceMotionChange({},{},{})",
                            alpha, beta, gamma
                        ),
                    )
                    .await
            }

            HostCommand::OnGyroscopeChange { x, y, z } => {
                self.js
                    .exec_script_and_pump(
                        "gyroscope",
                        format!("_internalTriggerGyroscopeChange({},{},{})", x, y, z),
                    )
                    .await
            }

            HostCommand::OnDeviceOrientationChange { value } => {
                self.js
                    .exec_script_and_pump(
                        "device_orientation",
                        format!(
                            "_internalTriggerDeviceOrientationChange('{}')",
                            value
                        ),
                    )
                    .await
            }

            HostCommand::OnCompassChange {
                direction,
                accuracy,
            } => {
                self.js
                    .exec_script_and_pump(
                        "compass",
                        format!(
                            "_internalTriggerCompassChange({},'{}')",
                            direction, accuracy
                        ),
                    )
                    .await
            }

            HostCommand::OnAccelerometerChange { x, y, z } => {
                self.js
                    .exec_script_and_pump(
                        "accelerometer",
                        format!("_internalTriggerAccelerometerChange({},{},{})", x, y, z),
                    )
                    .await
            }

            HostCommand::OnNetworkStatusChange {
                is_connected,
                network_type,
            } => {
                self.js
                    .exec_script_and_pump(
                        "network_status",
                        format!(
                            "_internalTriggerNetworkStatusChange({},'{}')",
                            is_connected, network_type
                        ),
                    )
                    .await
            }

            _ => Ok(()),
        }
    }

    async fn on_evaluate_module(&mut self, game_id: String, entry: String) -> EngineResult<()> {
        self.last_game_id = Some(game_id.clone());
        self.last_entry = Some(entry.clone());

        self.js.evaluate_module(game_id, entry).await?;

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
            device_services,
        };

        let module_loader: Option<Rc<dyn ModuleLoader>> =
            Some(Rc::new(MyModuleLoader(FsModuleLoader)));

        let extra_ext = self.platform.extensions(&self.init_options);

        self.js = HostJsRuntime::new(self.id as i32, host_state, extra_ext, module_loader);

        // If we have a last evaluated module, reload it
        if let (Some(game_id), Some(entry)) = (self.last_game_id.clone(), self.last_entry.clone()) {
            self.on_evaluate_module(game_id, entry).await?;
        }

        // Resume audio; render will resume upon surface update if needed
        self.audio.resume();

        Ok(())
    }
}
