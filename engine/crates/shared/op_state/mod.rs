use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::mpsc::UnboundedSender;

use crate::protocol::{audio_cmd::AudioCmd, io_cmd::IOCmd, render_cmd::RenderCommand};
use crate::services::DeviceServices;
use crate::vfs::{GamePaths, VirtualFS};

/// Host-side operational state shared across runtime layers.
pub type RenderTx = crossbeam_channel::Sender<RenderCommand>;
pub type IoTx = UnboundedSender<IOCmd>;
pub type AudioTx = UnboundedSender<AudioCmd>;

#[derive(Clone)]
pub struct HostOpState {
    pub id: i32,
    /// App-level cache directory (Context.getCacheDir()).
    pub app_cache_dir: PathBuf,
    /// App-level files directory (Context.getFilesDir()).
    pub app_files_dir: PathBuf,
    /// Game code directory (set after EvaluateModule).
    pub code_dir: Option<String>,
    /// Game-specific paths (set after EvaluateModule).
    pub game_paths: Option<Arc<GamePaths>>,
    /// Virtual file system for path sandboxing (set after EvaluateModule).
    pub vfs: Option<Arc<VirtualFS>>,
    pub render_tx: RenderTx,
    pub io_tx: IoTx,
    pub audio_tx: AudioTx,
    /// Platform device services (clipboard, sensors, etc.)
    pub device_services: Option<Arc<dyn DeviceServices>>,
}

impl fmt::Debug for HostOpState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HostOpState")
            .field("id", &self.id)
            .field("app_cache_dir", &self.app_cache_dir)
            .field("app_files_dir", &self.app_files_dir)
            .field("code_dir", &self.code_dir)
            .field("game_paths", &self.game_paths)
            .field("vfs", &self.vfs)
            .field("device_services", &self.device_services.as_ref().map(|_| "..."))
            .finish()
    }
}

#[derive(Debug, Clone)]
pub struct CanvasOpState {
    pub tx: RenderTx,
    pub has_onscreen: bool,
}

impl CanvasOpState {
    #[inline]
    pub fn new(tx: RenderTx, has_onscreen: bool) -> Self {
        Self { tx, has_onscreen }
    }
}
