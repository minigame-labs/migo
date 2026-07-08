use shared::{
    error::{EngineError, EngineResult, ErrorCode},
    protocol::render_cmd::CanvasCmd,
};

use crate::{CanvasManager, onscreen_window_from_surface};

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

            CanvasCmd::RegisterOffscreen { id, width, height } => {
                if let Err(e) = cm.register_offscreen(id, width, height) {
                    tracing::warn!(
                        "CanvasCmd::RegisterOffscreen failed: id={:?}, {}x{}, err={}",
                        id,
                        width,
                        height,
                        e
                    );
                }
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
                    DecodedImage::HardwareBuffer(ahb_image) => {
                        // Zero-copy: the decoder wrote directly into
                        // this AHB; we hand it to EGLImage without
                        // another memcpy. `load_ahb_image` itself
                        // contains the fallback to a CPU round-trip
                        // when the device lacks AHB support.
                        //
                        // Priority is effectively "always critical"
                        // because the AHB path is already the
                        // lowest-latency option we have — async
                        // upload via PBO can't beat a direct
                        // `glEGLImageTargetTexture2DOES`.
                        let _ = priority;
                        let res = cm.load_ahb_image(image_id, ahb_image);
                        let _ = resp.send(res);
                    }
                    DecodedImage::Rgba(rgba_image) => {
                        // For Critical priority: always sync upload (don't defer).
                        // For Normal/Background: try async upload thread.
                        // On budget rejection (healthy upload thread,
                        // temporary squeeze) the request is deferred
                        // to next frame instead of falling back to a
                        // synchronous `glTexImage2D` on the render
                        // thread -- the sync path was the exact
                        // frame spike the async thread was meant to
                        // avoid.  The sync fallback is reserved for
                        // cases where waiting can't help: permanent
                        // degradation (no upload thread, or upload
                        // thread reported unrecoverable failure) and
                        // images that can never fit the async upload
                        // budget window.
                        if priority == ImagePriority::Critical {
                            let res = cm.load_shared_image(image_id, rgba_image);
                            let _ = resp.send(res);
                        } else {
                            match cm.submit_async_upload(image_id, &rgba_image, resp) {
                                Ok(()) => {}
                                Err(resp) => {
                                    match cm.async_upload_reject_action(rgba_image.rgba.len()) {
                                        crate::canvas::manager::AsyncUploadRejectAction::SyncFallback => {
                                            let res = cm.load_shared_image(image_id, rgba_image);
                                            let _ = resp.send(res);
                                        }
                                        crate::canvas::manager::AsyncUploadRejectAction::DeferRetry => {
                                            // Budget squeeze: queue for retry.
                                            // If the deferred queue is
                                            // already full (pathological
                                            // burst), last-resort sync.
                                            match cm.defer_upload(
                                                image_id,
                                                rgba_image.clone(),
                                                resp,
                                            ) {
                                                Ok(()) => {}
                                                Err((img, resp)) => {
                                                    let res = cm.load_shared_image(image_id, img);
                                                    let _ = resp.send(res);
                                                }
                                            }
                                        }
                                    }
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

            CanvasCmd::DestroyImages { image_ids } => {
                for image_id in image_ids {
                    cm.cancel_pending_load(image_id);
                    let _ = cm.destroy_shared_image(image_id);
                }
            }

            _ => {}
        }
        Ok(())
    }
}
