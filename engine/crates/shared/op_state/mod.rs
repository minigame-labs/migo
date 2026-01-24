use std::path::PathBuf;

use tokio::sync::mpsc::UnboundedSender;

use crate::protocol::{io_cmd::IOCmd, render_cmd::RenderCommand};

/// Host-side operational state shared across runtime layers.
pub type RenderTx = crossbeam_channel::Sender<RenderCommand>;
pub type IoTx = UnboundedSender<IOCmd>;

#[derive(Debug, Clone)]
pub struct HostOpState {
    pub id: i32,
    pub app_tmp_dir: PathBuf,
    pub code_dir: Option<String>,
    pub render_tx: RenderTx,
    pub io_tx: IoTx,
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
