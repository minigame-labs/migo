use std::{cell::RefCell, path::Path, rc::Rc};

use deno_core::{OpState, extension, op2};
use deno_error::JsErrorBox;
use tracing::{error, info, warn};

use shared::{
    error::{EngineError, EngineResult, ErrorCode},
    op_state::{CanvasOpState, HostOpState},
    protocol::{
        io_cmd::{IOCmd, IOCmdResp},
        render_cmd::{CanvasCmd, RenderCommand},
        send_fs_with_resp_async, send_render_with_resp_async, send_render_with_resp_sync,
    },
    vfs::FileOp,
};

pub(crate) mod cache;

const OP_CREATE_IMAGE: &str = "canvas create image";
const OP_LOAD_IMAGE: &str = "canvas load image";

#[inline]
fn js_err_from_engine(e: EngineError) -> JsErrorBox {
    match &e.detail {
        Some(d) => JsErrorBox::generic(format!("[{:?}] {} ({})", e.code, e.msg, d)),
        None => JsErrorBox::generic(format!("[{:?}] {}", e.code, e.msg)),
    }
}

#[inline]
fn engine_err_to_text(e: &EngineError) -> String {
    match &e.detail {
        Some(d) => format!("[{:?}] {} ({})", e.code, e.msg, d),
        None => format!("[{:?}] {}", e.code, e.msg),
    }
}

fn resolve_local_src(
    code_dir: &str,
    vfs: Option<&shared::vfs::VirtualFS>,
    src: &str,
) -> EngineResult<String> {
    shared::ensure!(
        !(src.starts_with("http://") || src.starts_with("https://")),
        ErrorCode::Unsupported,
        "http image not supported yet",
        src
    );

    if let Some(vfs) = vfs {
        if !src.starts_with('/') {
            let vpath = format!("/code/{}", src);
            return vfs
                .resolve(&vpath, FileOp::Read)
                .map(|p| p.to_string_lossy().into_owned())
                .map_err(|e| {
                    EngineError::new(ErrorCode::PermissionDenied)
                        .with_msg("image path resolve failed")
                        .with_detail(format!("src={}, vpath={}, err={}", src, vpath, e))
                });
        }

        let is_virtual = src == "/code"
            || src.starts_with("/code/")
            || src == "/user"
            || src.starts_with("/user/")
            || src == "/cache"
            || src.starts_with("/cache/")
            || src == "/tmp"
            || src.starts_with("/tmp/");

        if is_virtual {
            return vfs
                .resolve(src, FileOp::Read)
                .map(|p| p.to_string_lossy().into_owned())
                .map_err(|e| {
                    EngineError::new(ErrorCode::PermissionDenied)
                        .with_msg("image path resolve failed")
                        .with_detail(format!("src={}, err={}", src, e))
                });
        }
    }

    if Path::new(src).is_absolute() {
        return Ok(src.to_string());
    }

    Ok(Path::new(code_dir).join(src).to_string_lossy().into_owned())
}

#[op2(fast)]
pub fn op_create_image(state: &mut OpState) -> u32 {
    let ctx = state.borrow::<CanvasOpState>();
    match send_render_with_resp_sync(ctx, OP_CREATE_IMAGE, |resp| {
        RenderCommand::Canvas(CanvasCmd::CreateImage { resp })
    }) {
        Ok(id) => id,
        Err(e) => {
            error!("{OP_CREATE_IMAGE} failed: {:?}", e);
            0
        }
    }
}

async fn op_load_image_inner(
    state: Rc<RefCell<OpState>>,
    image_id: u32,
    src: String,
) -> EngineResult<(u32, (usize, usize))> {
    let (io_tx, code_dir, vfs) = {
        let op = state.borrow();
        let host = op.borrow::<HostOpState>();
        (host.io_tx.clone(), host.code_dir.clone(), host.vfs.clone())
    };

    let code_dir = code_dir.unwrap_or_default();
    shared::ensure!(
        !code_dir.is_empty(),
        ErrorCode::InvalidArgument,
        "code_dir not available",
        src.clone()
    );

    let canvas_ctx: CanvasOpState = {
        let op = state.borrow();
        op.borrow::<CanvasOpState>().clone()
    };

    let src = resolve_local_src(&code_dir, vfs.as_deref(), &src)?;
    info!(
        "op_load_image begin: image_id={}, src={}",
        image_id,
        src
    );

    // remove previous alias and possibly destroy old shared
    if let Some(to_destroy) = {
        let mut c = cache::IMAGE_CACHE.lock();
        c.remove_previous_alias(image_id)
    } {
        let _ = canvas_ctx
            .tx
            .send(RenderCommand::Canvas(CanvasCmd::DestroyImage {
                image_id: to_destroy,
            }));
    }

    match {
        let mut c = cache::IMAGE_CACHE.lock();
        c.begin_load(image_id, &src)
    } {
        cache::BeginLoadResult::AlreadyLoaded((shared_id, dims)) => {
            info!(
                "op_load_image cache hit: image_id={}, shared_id={}, src={}, dims={}x{}",
                image_id,
                shared_id,
                src,
                dims.0,
                dims.1
            );
            Ok((shared_id, dims))
        }

        cache::BeginLoadResult::Join(rx) => {
            info!(
                "op_load_image join pending load: image_id={}, src={}",
                image_id,
                src
            );
            match rx.await {
                Ok(Ok((shared_id, dims))) => {
                    // IMPORTANT: bind alias for this caller image_id so destroy works even if JS does not replace IDs
                    {
                        let mut c = cache::IMAGE_CACHE.lock();
                        c.bind_alias_existing(image_id, &src, shared_id);
                    }
                    info!(
                        "op_load_image join resolved: image_id={}, shared_id={}, src={}, dims={}x{}",
                        image_id,
                        shared_id,
                        src,
                        dims.0,
                        dims.1
                    );
                    Ok((shared_id, dims))
                }
                Ok(Err(msg)) => {
                    warn!(
                        "op_load_image join failed: image_id={}, src={}, err={}",
                        image_id,
                        src,
                        msg
                    );
                    shared::bail!(ErrorCode::ImageReadError, "cache join failed", msg)
                }
                Err(_) => {
                    warn!(
                        "op_load_image join canceled: image_id={}, src={}",
                        image_id,
                        src
                    );
                    shared::bail!(ErrorCode::Cancelled, "wait canceled")
                }
            }
        }

        cache::BeginLoadResult::StartLoading => {
            info!(
                "op_load_image start loader: image_id={}, src={}",
                image_id,
                src
            );
            // NOTE: Double-allocation window.
            // Between ReadImageRgba8 completing and the render thread's
            // LoadImage consuming the RGBA buffer, both the IO cache and the
            // `img` value hold copies of the decoded pixel data in CPU memory.
            // For a 1024x1024 RGBA8 image this is ~4 MB duplicated.
            //
            // This is architecturally necessary because:
            //  1. The IO layer decodes synchronously and caches the result
            //     for potential re-use by other concurrent loaders (Join path).
            //  2. The render thread owns the GL context and must receive the
            //     pixel data to perform glTexImage2D.
            //  3. The two threads (Host/IO and Render) cannot share a
            //     zero-copy buffer without unsafe lifetime management that
            //     would couple their lifecycles.
            //
            // Mitigation: we eagerly evict the IO cache entry immediately
            // after GPU upload succeeds (see below), limiting the overlap
            // window to the duration of the LoadImage render command.
            let img = match send_fs_with_resp_async(&io_tx, |resp_tx| IOCmd::ReadImageRgba8 {
                path: src.clone(),
                resp: IOCmdResp::Async(resp_tx),
            })
            .await
            {
                Ok(img) => img,
                Err(e) => {
                    // IMPORTANT: begin_load() inserted this src into loading_map.
                    // If we return early here, joiners on the same src will wait forever.
                    // Always resolve loading_map on error.
                    let msg = engine_err_to_text(&e);
                    warn!(
                        "op_load_image io decode failed: image_id={}, src={}, err={}",
                        image_id,
                        src,
                        msg
                    );
                    let mut c = cache::IMAGE_CACHE.lock();
                    c.finish_load(image_id, &src, Err(msg));
                    return Err(e);
                }
            };

            let res = send_render_with_resp_async(&canvas_ctx, OP_LOAD_IMAGE, |resp| {
                RenderCommand::Canvas(CanvasCmd::LoadImage {
                    image_id,
                    image: img,
                    resp,
                })
            })
            .await;

            {
                let mut c = cache::IMAGE_CACHE.lock();
                match &res {
                    Ok((w, h)) => c.finish_load(image_id, &src, Ok((*w as usize, *h as usize))),
                    Err(e) => c.finish_load(image_id, &src, Err(engine_err_to_text(e))),
                }
            }

            // Keep decoded RGBA in IO cache for WebGL texImage2D/texSubImage2D
            // source-image uploads, which may need synchronous pixel access
            // after Image onload has fired.

            match res {
                Ok((w, h)) => {
                    info!(
                        "op_load_image loader resolved: image_id={}, src={}, dims={}x{}",
                        image_id,
                        src,
                        w,
                        h
                    );
                    Ok((image_id, (w as usize, h as usize)))
                }
                Err(e) => {
                    warn!(
                        "op_load_image gpu upload failed: image_id={}, src={}, err={}",
                        image_id,
                        src,
                        engine_err_to_text(&e)
                    );
                    Err(e)
                }
            }
        }
    }
}

#[op2(async(lazy), fast)]
#[serde]
pub async fn op_load_image(
    state: Rc<RefCell<OpState>>,
    #[smi] image_id: u32,
    #[string] src: String,
) -> Result<(u32, (usize, usize)), JsErrorBox> {
    op_load_image_inner(state, image_id, src)
        .await
        .map_err(js_err_from_engine)
}

#[op2(fast)]
pub fn op_destroy_image(state: &mut OpState, #[smi] image_id: u32) -> bool {
    let to_destroy = {
        let mut c = cache::IMAGE_CACHE.lock();
        c.try_release_and_get_destroy_rid(image_id)
    };

    if let Some(rid) = to_destroy {
        let ctx = state.borrow::<CanvasOpState>();
        let _ = ctx.tx.send(RenderCommand::Canvas(CanvasCmd::DestroyImage {
            image_id: rid,
        }));
    }
    true
}

/// Preload multiple images in parallel
/// Returns array of [path, success, width, height, error_msg]
#[op2(async(lazy))]
#[serde]
pub async fn op_preload_images(
    state: Rc<RefCell<OpState>>,
    #[serde] paths: Vec<String>,
) -> Result<Vec<(String, bool, u32, u32, String)>, JsErrorBox> {
    let (io_tx, code_dir, vfs) = {
        let op = state.borrow();
        let host = op.borrow::<HostOpState>();
        (host.io_tx.clone(), host.code_dir.clone(), host.vfs.clone())
    };

    let code_dir = code_dir.unwrap_or_default();
    if code_dir.is_empty() {
        return Err(JsErrorBox::generic("code_dir not available"));
    }

    // Resolve all paths
    let resolved_paths: Vec<String> = paths
        .iter()
        .map(|p| resolve_local_src(&code_dir, vfs.as_deref(), p).unwrap_or_else(|_| p.clone()))
        .collect();

    // Send preload command to IO thread
    let results = send_fs_with_resp_async(&io_tx, |resp_tx| IOCmd::PreloadImages {
        paths: resolved_paths.clone(),
        resp: IOCmdResp::Async(resp_tx),
    })
    .await
    .map_err(js_err_from_engine)?;

    // Convert results to JS-friendly format
    let output: Vec<(String, bool, u32, u32, String)> = results
        .into_iter()
        .map(|(path, result)| match result {
            Ok((w, h)) => (path, true, w, h, String::new()),
            Err(msg) => (path, false, 0, 0, msg),
        })
        .collect();

    Ok(output)
}

/// Clear the image cache
#[op2(async(lazy), fast)]
pub async fn op_clear_image_cache(state: Rc<RefCell<OpState>>) -> Result<(), JsErrorBox> {
    let io_tx = {
        let op = state.borrow();
        let host = op.borrow::<HostOpState>();
        host.io_tx.clone()
    };

    send_fs_with_resp_async(&io_tx, |resp_tx| IOCmd::ClearImageCache {
        resp: IOCmdResp::Async(resp_tx),
    })
    .await
    .map_err(js_err_from_engine)
}

/// Get image cache statistics
#[op2(async(lazy), fast)]
#[serde]
pub async fn op_get_image_cache_stats(
    state: Rc<RefCell<OpState>>,
) -> Result<shared::protocol::io_cmd::ImageCacheStats, JsErrorBox> {
    let io_tx = {
        let op = state.borrow();
        let host = op.borrow::<HostOpState>();
        host.io_tx.clone()
    };

    send_fs_with_resp_async(&io_tx, |resp_tx| IOCmd::GetImageCacheStats {
        resp: IOCmdResp::Async(resp_tx),
    })
    .await
    .map_err(js_err_from_engine)
}

extension!(host_v8_image,
    deps = [host_v8_console, host_v8_base, host_v8_file],
    ops = [
        op_create_image,
        op_load_image,
        op_destroy_image,
        op_preload_images,
        op_clear_image_cache,
        op_get_image_cache_stats
    ],
    esm = [
        dir "rendering/image",
        "01_image.js",
        "01_image_data.js",
    ],
);

pub(super) fn image_extensions() -> Vec<deno_core::Extension> {
    vec![host_v8_image::init()]
}

pub(super) fn image_lazy_extensions() -> Vec<deno_core::Extension> {
    vec![host_v8_image::lazy_init()]
}
