use std::{cell::RefCell, rc::Rc};

use deno_core::{OpState, extension, op2};
use deno_error::JsErrorBox;
use tracing::{error, info, warn};

use shared::{
    error::{EngineError, EngineResult, ErrorCode},
    op_state::{CanvasOpState, HostOpState},
    protocol::{
        render_cmd::{CanvasCmd, RenderCommand},
        send_render_with_resp_async, send_render_with_resp_sync,
    },
    vfs::FileOp,
};

use crate::io_state::IoSchedulerState;

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

/// Resolved image source: real path + version identity (for cache keying).
struct ResolvedSrc {
    path: String,
    /// Source version for cache invalidation.
    /// For mount-backed: mount source_mounted_at.
    /// For mutable filesystem paths: derived from file mtime+size.
    source_version: u64,
    source: io::image_ops::ImageSource,
}

use shared::protocol::io_cmd::{VARIANT_EXTENSIONS, path_stem};

/// Compute a cache/version token for a filesystem-backed image source and all
/// of its known sibling variants.  Uses metadata only (mtime + size) — never
/// reads file content — so it is safe to call on the event loop thread.
fn variant_source_version_token(
    path: &str,
    virtual_src: Option<&str>,
    mount_table: Option<&shared::vfs::MountTable>,
) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut h = DefaultHasher::new();
    let stem = path_stem(path);
    let virtual_stem = virtual_src.map(path_stem);

    let mut candidates: Vec<(String, Option<String>)> =
        Vec::with_capacity(VARIANT_EXTENSIONS.len() + 1);
    candidates.push((path.to_string(), virtual_src.map(|s| s.to_string())));
    for ext in VARIANT_EXTENSIONS {
        let candidate = format!("{}.{}", stem, ext);
        if candidate != path {
            let virtual_candidate = virtual_stem
                .as_ref()
                .map(|vstem| format!("{}.{}", vstem, ext));
            candidates.push((candidate, virtual_candidate));
        }
    }

    for (candidate, virtual_candidate) in candidates {
        candidate.hash(&mut h);
        // Metadata-only versioning: (exists, size, mtime).
        // Never reads file content — keeps this function cheap enough for
        // the event loop thread.
        match std::fs::metadata(&candidate) {
            Ok(meta) => {
                1u8.hash(&mut h);
                meta.len().hash(&mut h);
                let mtime = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_nanos())
                    .unwrap_or(0);
                mtime.hash(&mut h);
            }
            Err(_) => {
                0u8.hash(&mut h);
            }
        }
        if let (Some(mt), Some(virtual_candidate)) = (mount_table, virtual_candidate) {
            virtual_candidate.hash(&mut h);
            if let Some(resolved) = mt.resolve_code_path(&virtual_candidate) {
                1u8.hash(&mut h);
                resolved.source_mounted_at.hash(&mut h);
            } else {
                0u8.hash(&mut h);
            }
        }
    }
    h.finish()
}

fn resized_rgba_io_cache_key(
    src: &str,
    target_width: Option<u32>,
    target_height: Option<u32>,
    source_generation: u64,
) -> cache::ImageCacheKey {
    cache::make_cache_key(src, target_width, target_height, source_generation)
}

fn resolve_local_src(
    vfs: Option<&shared::vfs::VirtualFS>,
    mount_table: Option<&shared::vfs::MountTable>,
    src: &str,
) -> EngineResult<ResolvedSrc> {
    shared::ensure!(
        !(src.starts_with("http://") || src.starts_with("https://")),
        ErrorCode::Unsupported,
        "http image not supported yet",
        src
    );

    // Relative path → normalize to /code/{src} and fall into the /code branch.
    // This ensures "a.png" and "/code/a.png" always take the same path.
    let owned_vpath;
    let effective_src = if !src.starts_with('/') {
        owned_vpath = format!("/code/{}", src);
        owned_vpath.as_str()
    } else {
        src
    };

    // /code paths → resolve through mount table (preferred) or VFS fallback.
    if effective_src == "/code" || effective_src.starts_with("/code/") {
        if let Some(mt) = mount_table {
            let resolved = mt.resolve_code_path(effective_src).ok_or_else(|| {
                EngineError::new(ErrorCode::PermissionDenied)
                    .with_msg("image path resolve failed")
                    .with_detail(format!(
                        "src={}, mount resolve_code_path returned None",
                        src
                    ))
            })?;
            match resolved.real_path {
                Some(real) => {
                    let relative = effective_src.strip_prefix("/code/").unwrap_or("");
                    return Ok(ResolvedSrc {
                        source_version: variant_source_version_token(
                            &real.to_string_lossy(),
                            Some(effective_src),
                            mount_table,
                        ),
                        path: effective_src.to_string(),
                        source: io::image_ops::ImageSource::MountCode {
                            virtual_path: effective_src.to_string(),
                            relative_path: relative.to_string(),
                        },
                    });
                }
                None => {
                    // Pack-backed: carry the relative path so the image worker
                    // pool performs the package read instead of the host thread.
                    let relative = effective_src.strip_prefix("/code/").unwrap_or("");
                    let max_len = shared::protocol::io_cmd::MAX_READ_LENGTH;
                    if let Some(size) = mt.entry_size(relative) {
                        if size > max_len {
                            return Err(EngineError::new(ErrorCode::IoError)
                                .with_msg("pack image too large")
                                .with_detail(format!(
                                    "src={}, size={}, limit={}",
                                    src, size, max_len
                                )));
                        }
                    }
                    return Ok(ResolvedSrc {
                        path: effective_src.to_string(),
                        source_version: resolved.source_mounted_at,
                        source: io::image_ops::ImageSource::MountCode {
                            virtual_path: effective_src.to_string(),
                            relative_path: relative.to_string(),
                        },
                    });
                }
            }
        }
        // Fallback: no mount table, use VFS + file-version token.
        if let Some(vfs) = vfs {
            return vfs
                .resolve(effective_src, FileOp::Read)
                .map(|p| {
                    let path_str = p.to_string_lossy().into_owned();
                    let ver =
                        variant_source_version_token(&path_str, Some(effective_src), mount_table);
                    ResolvedSrc {
                        path: path_str,
                        source_version: ver,
                        source: io::image_ops::ImageSource::Filesystem,
                    }
                })
                .map_err(|e| {
                    EngineError::new(ErrorCode::PermissionDenied)
                        .with_msg("image path resolve failed")
                        .with_detail(format!("src={}, err={}", src, e))
                });
        }
    }

    // /user, /cache, /tmp: mutable paths, use file-version token.
    let is_other_virtual = effective_src == "/user"
        || effective_src.starts_with("/user/")
        || effective_src == "/cache"
        || effective_src.starts_with("/cache/")
        || effective_src == "/tmp"
        || effective_src.starts_with("/tmp/");

    if is_other_virtual {
        if let Some(vfs) = vfs {
            return vfs
                .resolve(effective_src, FileOp::Read)
                .map(|p| {
                    let path_str = p.to_string_lossy().into_owned();
                    let ver = variant_source_version_token(&path_str, None, mount_table);
                    ResolvedSrc {
                        path: path_str,
                        source_version: ver,
                        source: io::image_ops::ImageSource::Filesystem,
                    }
                })
                .map_err(|e| {
                    EngineError::new(ErrorCode::PermissionDenied)
                        .with_msg("image path resolve failed")
                        .with_detail(format!("src={}, err={}", src, e))
                });
        }
    }

    // Non-virtual absolute path: BLOCKED.
    Err(EngineError::new(ErrorCode::PermissionDenied)
        .with_msg("image path not allowed")
        .with_detail(format!(
            "src={}: absolute host paths are not permitted; use /code, /user, /cache, or /tmp",
            src
        )))
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
    target_width: Option<u32>,
    target_height: Option<u32>,
) -> EngineResult<(u32, (usize, usize))> {
    let (scheduler, vfs, mount_table, game_cache_dir, gpu_caps) = {
        let op = state.borrow();
        let host = op.borrow::<HostOpState>();
        let gcd = host
            .game_paths
            .as_ref()
            .map(|gp| gp.cache_dir().to_string_lossy().into_owned());
        (
            op.borrow::<IoSchedulerState>().0.clone(),
            host.vfs.clone(),
            host.mount_table.clone(),
            gcd,
            host.gpu_caps.snapshot(),
        )
    };

    let canvas_ctx: CanvasOpState = {
        let op = state.borrow();
        op.borrow::<CanvasOpState>().clone()
    };

    // Resolve path + generation atomically from a single mount table read.
    let resolved = resolve_local_src(vfs.as_deref(), mount_table.as_deref(), &src)?;
    let src = resolved.path;
    let mount_generation = resolved.source_version;
    let image_source = resolved.source;
    info!("op_load_image begin: image_id={}, src={}", image_id, src);

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

    // Structured cache key: (path\0WxH, generation) — no delimiter collision.
    let cache_key = cache::make_cache_key(&src, target_width, target_height, mount_generation);

    match {
        let mut c = cache::IMAGE_CACHE.lock();
        c.begin_load(image_id, &cache_key)
    } {
        cache::BeginLoadResult::AlreadyLoaded((shared_id, dims)) => {
            info!(
                "op_load_image cache hit: image_id={}, shared_id={}, src={}, dims={}x{}",
                image_id, shared_id, src, dims.0, dims.1
            );
            Ok((shared_id, dims))
        }

        cache::BeginLoadResult::Join(rx) => {
            info!(
                "op_load_image join pending load: image_id={}, src={}",
                image_id, src
            );
            match rx.await {
                Ok(Ok((actual_cache_key, shared_id, dims))) => {
                    // IMPORTANT: bind alias for this caller image_id so destroy works even if JS does not replace IDs
                    {
                        let mut c = cache::IMAGE_CACHE.lock();
                        c.bind_alias_existing(image_id, &cache_key, &actual_cache_key, shared_id);
                    }
                    info!(
                        "op_load_image join resolved: image_id={}, shared_id={}, src={}, dims={}x{}",
                        image_id, shared_id, src, dims.0, dims.1
                    );
                    Ok((shared_id, dims))
                }
                Ok(Err(msg)) => {
                    warn!(
                        "op_load_image join failed: image_id={}, src={}, err={}",
                        image_id, src, msg
                    );
                    shared::bail!(ErrorCode::ImageReadError, "cache join failed", msg)
                }
                Err(_) => {
                    warn!(
                        "op_load_image join canceled: image_id={}, src={}",
                        image_id, src
                    );
                    shared::bail!(ErrorCode::Cancelled, "wait canceled")
                }
            }
        }

        cache::BeginLoadResult::StartLoading => {
            // Allocate an independent shared ID for the GPU texture.
            // This must NOT be the caller's image_id — see cache.rs docs.
            let shared_id = cache::alloc_shared_id();
            // Register the alias now so that a concurrent op_destroy_image
            // resolves to the correct shared_id during the upload window.
            {
                let mut c = cache::IMAGE_CACHE.lock();
                c.register_inflight_alias(image_id, shared_id);
            }
            info!(
                "op_load_image start loader: image_id={}, shared_id={}, src={}",
                image_id, shared_id, src
            );

            let decoded = match io::image_ops::read_image_rgba8(
                scheduler,
                src.clone(),
                target_width,
                target_height,
                mount_generation,
                image_source.clone(),
                game_cache_dir.clone(),
                gpu_caps,
                mount_table.clone(),
            )
            .await
            {
                Ok(decoded) => decoded,
                Err(e) => {
                    let msg = engine_err_to_text(&e);
                    warn!(
                        "op_load_image io decode failed: image_id={}, src={}, err={}",
                        image_id, src, msg
                    );
                    let mut c = cache::IMAGE_CACHE.lock();
                    let _ = c.finish_load(image_id, shared_id, &cache_key, &cache_key, Err(msg));
                    return Err(e);
                }
            };

            let actual_cache_key = resized_rgba_io_cache_key(
                &src,
                target_width,
                target_height,
                decoded.source_generation,
            );
            let img = decoded.image;

            // For scaled RGBA decodes, store in IO cache.
            // Compressed images skip the IO cache (fast to re-read).
            if target_width.is_some() && target_height.is_some() {
                if let shared::protocol::io_cmd::DecodedImage::Rgba(ref rgba) = img {
                    io::global_cache().insert(actual_cache_key.clone(), rgba.clone());
                }
            }

            // Upload texture under shared_id (not caller image_id).
            let res = send_render_with_resp_async(&canvas_ctx, OP_LOAD_IMAGE, |resp| {
                RenderCommand::Canvas(CanvasCmd::LoadImage {
                    image_id: shared_id,
                    image: img,
                    priority: shared::protocol::io_cmd::ImagePriority::Normal,
                    resp,
                })
            })
            .await;

            let maybe_destroy = {
                let mut c = cache::IMAGE_CACHE.lock();
                match &res {
                    Ok((w, h)) => c.finish_load(
                        image_id,
                        shared_id,
                        &cache_key,
                        &actual_cache_key,
                        Ok((*w as usize, *h as usize)),
                    ),
                    Err(e) => c.finish_load(
                        image_id,
                        shared_id,
                        &cache_key,
                        &actual_cache_key,
                        Err(engine_err_to_text(e)),
                    ),
                }
            };

            if let Some(to_destroy) = maybe_destroy {
                let _ = canvas_ctx
                    .tx
                    .send(RenderCommand::Canvas(CanvasCmd::DestroyImage {
                        image_id: to_destroy,
                    }));
            }

            match res {
                Ok((w, h)) => {
                    info!(
                        "op_load_image loader resolved: image_id={}, shared_id={}, src={}, dims={}x{}",
                        image_id, shared_id, src, w, h
                    );
                    Ok((shared_id, (w as usize, h as usize)))
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
    #[smi] target_width: u32,
    #[smi] target_height: u32,
) -> Result<(u32, (usize, usize)), JsErrorBox> {
    let tw = if target_width > 0 {
        Some(target_width)
    } else {
        None
    };
    let th = if target_height > 0 {
        Some(target_height)
    } else {
        None
    };
    op_load_image_inner(state, image_id, src, tw, th)
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
    let (scheduler, vfs, mount_table, game_cache_dir, gpu_caps) = {
        let op = state.borrow();
        let host = op.borrow::<HostOpState>();
        let gcd = host
            .game_paths
            .as_ref()
            .map(|gp| gp.cache_dir().to_string_lossy().into_owned());
        (
            op.borrow::<IoSchedulerState>().0.clone(),
            host.vfs.clone(),
            host.mount_table.clone(),
            gcd,
            host.gpu_caps.snapshot(),
        )
    };

    // Resolve all paths atomically (path + generation per resolve call).
    // Rejected paths become error entries; never sent to decode.
    let mut io_entries: Vec<(String, u64, io::image_ops::ImageSource)> =
        Vec::with_capacity(paths.len());
    let mut early_errors: Vec<(usize, String)> = Vec::new();
    for (i, p) in paths.iter().enumerate() {
        match resolve_local_src(vfs.as_deref(), mount_table.as_deref(), p) {
            Ok(resolved) => {
                io_entries.push((resolved.path, resolved.source_version, resolved.source))
            }
            Err(e) => {
                early_errors.push((i, engine_err_to_text(&e)));
                io_entries.push((String::new(), 0, io::image_ops::ImageSource::Filesystem)); // placeholder
            }
        }
    }

    // Filter out failed entries, keeping per-path generation.
    let io_entries_filtered: Vec<(String, u64, io::image_ops::ImageSource)> = io_entries
        .iter()
        .enumerate()
        .filter(|(i, _)| !early_errors.iter().any(|(ei, _)| ei == i))
        .map(|(_, entry)| entry.clone())
        .collect();

    // Decode successfully resolved entries via the image scheduler pool.
    let io_results = if !io_entries_filtered.is_empty() {
        io::image_ops::preload_images(
            scheduler,
            io_entries_filtered,
            game_cache_dir.clone(),
            gpu_caps,
            mount_table.clone(),
        )
        .await
    } else {
        Vec::new()
    };

    // Merge results with early errors into the output, preserving
    // original order matching the input `paths`.
    let mut io_iter = io_results.into_iter();
    let mut output: Vec<(String, bool, u32, u32, String)> = Vec::with_capacity(paths.len());
    for (i, original_path) in paths.iter().enumerate() {
        if let Some(err_entry) = early_errors.iter().find(|(ei, _)| *ei == i) {
            output.push((original_path.clone(), false, 0, 0, err_entry.1.clone()));
        } else if let Some((_, result)) = io_iter.next() {
            match result {
                Ok((w, h)) => output.push((original_path.clone(), true, w, h, String::new())),
                Err(msg) => output.push((original_path.clone(), false, 0, 0, msg)),
            }
        }
    }

    Ok(output)
}

/// Clear all image caches: JS shared cache + IO in-memory cache + derived disk cache.
#[op2(fast)]
pub fn op_clear_image_cache(state: &mut OpState) -> Result<(), JsErrorBox> {
    // 1. Destroy live shared GPU textures, then clear JS-side shared cache.
    let gcd = {
        let host = state.borrow::<HostOpState>();
        host.game_paths
            .as_ref()
            .map(|gp| gp.cache_dir().to_string_lossy().into_owned())
    };
    let render_tx = state.borrow::<HostOpState>().render_tx.clone();

    for shared_id in cache::drain_shared_image_cache() {
        let _ = render_tx.send(RenderCommand::Canvas(CanvasCmd::DestroyImage {
            image_id: shared_id,
        }));
    }

    // 2. Clear IO in-memory cache + derived disk cache directly.
    io::image_ops::clear_image_cache(gcd.as_deref());
    Ok(())
}

/// Get image cache statistics
#[op2]
#[serde]
pub fn op_get_image_cache_stats(_state: &mut OpState) -> shared::protocol::io_cmd::ImageCacheStats {
    io::image_ops::get_image_cache_stats()
}

extension!(host_v8_image,
    deps = [host_v8_console, host_v8_base, host_v8_file, host_v8_io_state],
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

#[cfg(test)]
mod tests {
    use super::{resized_rgba_io_cache_key, variant_source_version_token};

    #[test]
    fn companion_appearing_invalidates_variant_token() {
        let dir = std::env::temp_dir().join("migo_image_variant_token");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let png = dir.join("tex.png");
        let ktx2 = dir.join("tex.ktx2");
        std::fs::write(&png, b"png-v1").unwrap();

        // v1: only PNG exists.
        let v1 = variant_source_version_token(png.to_str().unwrap(), None, None);
        // v2: KTX2 companion appears (different file set = different token).
        std::fs::write(&ktx2, b"ktx2-v1").unwrap();
        let v2 = variant_source_version_token(png.to_str().unwrap(), None, None);

        assert_ne!(v1, v2, "adding a companion must change the version token");

        // v3: KTX2 companion removed.
        std::fs::remove_file(&ktx2).unwrap();
        let v3 = variant_source_version_token(png.to_str().unwrap(), None, None);
        assert_eq!(v1, v3, "removing companion should restore original token");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn companion_size_change_invalidates_variant_token() {
        let dir = std::env::temp_dir().join("migo_image_variant_token_size");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let png = dir.join("tex.png");
        let ktx2 = dir.join("tex.ktx2");
        std::fs::write(&png, b"png-v1").unwrap();
        std::fs::write(&ktx2, b"ktx2-v1").unwrap();
        let v1 = variant_source_version_token(png.to_str().unwrap(), None, None);

        // Write different-size content to companion.
        std::fs::write(&ktx2, b"ktx2-v2-much-larger-content").unwrap();
        let v2 = variant_source_version_token(png.to_str().unwrap(), None, None);

        assert_ne!(v1, v2, "companion size change must invalidate token");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn extensionless_primary_size_change_invalidates_token() {
        let dir = std::env::temp_dir().join("migo_image_variant_token_extless");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let raw = dir.join("tex");
        std::fs::write(&raw, b"raw-v1-short").unwrap();
        let v1 = variant_source_version_token(raw.to_str().unwrap(), None, None);
        // Write different-size content.
        std::fs::write(&raw, b"raw-v2-much-longer-content-here").unwrap();
        let v2 = variant_source_version_token(raw.to_str().unwrap(), None, None);

        assert_ne!(v1, v2, "primary size change must invalidate token");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resized_rgba_cache_key_uses_decoded_source_generation() {
        let stale = resized_rgba_io_cache_key("/code/tex.png", Some(64), Some(64), 10);
        let refreshed = resized_rgba_io_cache_key("/code/tex.png", Some(64), Some(64), 20);

        assert_ne!(stale, refreshed);
        assert_eq!(refreshed.1, 20);
    }
}
