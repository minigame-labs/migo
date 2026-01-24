use shared::{error::EngineResult, protocol::render_cmd::CanvasCmd};

use crate::{CanvasManager, onscreen_window_from_surface};

pub(crate) struct CanvasHandler;

impl CanvasHandler {
    pub(crate) fn new() -> Self {
        CanvasHandler
    }

    pub(crate) fn handle_command(
        &mut self,
        cm: &mut CanvasManager,
        cmd: CanvasCmd,
    ) -> EngineResult<()> {
        match cmd {
            CanvasCmd::CreateOffscreen {
                width,
                height,
                resp,
            } => {
                let res = cm.create_offscreen(width, height);
                let _ = resp.send(res);
            }

            CanvasCmd::DestroyCanvas { id, resp } => {
                let res = cm.destroy_canvas(id);
                let _ = resp.send(res);
            }

            CanvasCmd::RecreateOnscreen { surface, resp } => {
                let res = (|| -> EngineResult<()> {
                    let win = onscreen_window_from_surface(surface.as_ref())?;
                    cm.create_onscreen(win)?;
                    Ok(())
                })();
                let _ = resp.send(res);
            }

            CanvasCmd::ResizeCanvas { id, w, h } => {
                let _ = cm.resize_canvas(id, w, h);
            }

            CanvasCmd::MakeCurrent { id, resp } => {
                let res = cm.make_current_needed(id);
                let _ = resp.send(res);
            }

            CanvasCmd::SwapBuffers {
                id,
                wait_for_vsync,
                resp,
            } => {
                let res = cm.swap_buffers_no_restore(id, wait_for_vsync);
                let _ = resp.send(res);
            }

            CanvasCmd::GetInfo { id, resp } => {
                let res = cm.get_info(id);
                let size = res.map(|info| info.size());
                let _ = resp.send(size);
            }

            CanvasCmd::CreateImage { resp } => {
                let res = cm.generate_img_id();
                let _ = resp.send(Ok(res));
            }

            CanvasCmd::LoadImage {
                image_id,
                image,
                resp,
            } => {
                let res = cm.load_shared_fv_image(image_id, image);
                let _ = resp.send(res);
            }

            CanvasCmd::DestroyImage { image_id } => {
                let _ = cm.destroy_shared_fv_image(image_id);
            }

            _ => {}
        }
        Ok(())
    }
}
