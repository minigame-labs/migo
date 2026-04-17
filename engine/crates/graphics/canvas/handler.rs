use shared::{
    error::{EngineError, EngineResult, ErrorCode},
    protocol::render_cmd::CanvasCmd,
};

use crate::{onscreen_window_from_surface, CanvasManager};

pub(crate) struct CanvasHandler;

impl CanvasHandler {
    pub(crate) fn new() -> Self {
        CanvasHandler
    }

    /// Handle a canvas command, returning the outcome to the render thread.
    ///
    /// For `RecreateOnscreen`, the result is propagated so the render thread
    /// can correctly track `has_surface`. Other commands communicate their
    /// results through the `resp` channel and always return `Ok` here.
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
                let requested_size = surface.size();
                tracing::info!(
                    "CanvasCmd::RecreateOnscreen: requested={}x{}",
                    requested_size.0,
                    requested_size.1
                );
                let res = (|| -> EngineResult<()> {
                    let win = onscreen_window_from_surface(surface.as_ref())?;
                    cm.create_onscreen(win, Some(requested_size))?;
                    Ok(())
                })();
                // Propagate to both host (via resp) and render thread (via return).
                let err_detail = res.as_ref().err().map(|e| e.to_string());
                let _ = resp.send(res);
                if let Some(detail) = err_detail {
                    tracing::warn!(
                        "CanvasCmd::RecreateOnscreen failed: requested={}x{}, err={}",
                        requested_size.0,
                        requested_size.1,
                        detail
                    );
                    return Err(EngineError::from_detail(
                        ErrorCode::RenderBackendError,
                        detail,
                    ));
                }
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
                let res = cm.swap_buffers_no_restore(id, wait_for_vsync).map(|_| ());
                let _ = resp.send(res);
            }

            CanvasCmd::GetInfo { id, resp } => {
                let size = cm.get_canvas_size(id);
                let _ = resp.send(size);
            }

            CanvasCmd::CreateImage { resp } => {
                let res = cm.generate_img_id();
                let _ = resp.send(Ok(res));
            }

            CanvasCmd::LoadImage {
                image_id,
                image,
                priority,
                resp,
            } => {
                use shared::protocol::io_cmd::{DecodedImage, ImagePriority};
                match image {
                    DecodedImage::Compressed(compressed) => {
                        // GPU-direct compressed upload — always sync (fast, no PBO needed).
                        let res = cm.load_compressed_image(image_id, &compressed);
                        let _ = resp.send(res);
                    }
                    DecodedImage::Rgba(rgba_image) => {
                        // For Critical priority: always sync upload (don't defer).
                        // For Normal/Background: try async upload thread, fall back to sync.
                        if priority == ImagePriority::Critical {
                            let res = cm.load_shared_image(image_id, rgba_image);
                            let _ = resp.send(res);
                        } else {
                            match cm.submit_async_upload(image_id, &rgba_image, resp) {
                                Ok(()) => {}
                                Err(resp) => {
                                    let res = cm.load_shared_image(image_id, rgba_image);
                                    let _ = resp.send(res);
                                }
                            }
                        }
                    }
                }
            }

            CanvasCmd::DestroyImage { image_id } => {
                cm.cancel_pending_load(image_id);
                let _ = cm.destroy_shared_image(image_id);
            }

            _ => {}
        }
        Ok(())
    }
}
