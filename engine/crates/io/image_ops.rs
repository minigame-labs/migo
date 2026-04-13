//! Public async image operations.
//!
//! Provides `read_image_rgba8`, `preload_images`, `clear_image_cache`, and
//! `get_image_cache_stats` as standalone async functions called directly
//! from js-runtime ops.
//!
//! Budget limiting and decode-concurrency semaphore keep memory behaviour
//! bounded.

use std::sync::{Arc, OnceLock};

use tokio::sync::Semaphore;
use tracing::{debug, warn};

use shared::{
    device::gpu_caps::GpuCapsSnapshot,
    error::{EngineError, ErrorCode},
    protocol::io_cmd::{
        DecodedImage, ImageCacheStats, MAX_READ_LENGTH, NormalizedImage, VARIANT_EXTENSIONS,
        path_stem,
    },
    vfs::MountTable,
};

use crate::{
    derived_cache, image_cache,
    pools::PoolError,
    scheduler::IoScheduler,
    task::{BackendKind, IoRequest, PriorityClass, RequestKind},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImageSource {
    Filesystem,
    MountCode {
        virtual_path: String,
        relative_path: String,
    },
    Pack {
        relative_path: String,
    },
}

// ---------------------------------------------------------------------------
// Budget + semaphore
// ---------------------------------------------------------------------------

/// Maximum number of concurrent image decode tasks across all callers
/// (read_image_rgba8, preload_images, etc.).
const MAX_CONCURRENT_IMAGE_DECODES: usize = 3;

/// Default budget: 48 MB.
const DEFAULT_IO_BUDGET_BYTES: usize = 48 * 1024 * 1024;

fn image_decode_semaphore() -> &'static Semaphore {
    static SEM: OnceLock<Semaphore> = OnceLock::new();
    SEM.get_or_init(|| Semaphore::new(MAX_CONCURRENT_IMAGE_DECODES))
}

/// Tracks aggregate bytes consumed by in-flight heavy IO tasks.
struct IoBudget {
    active_bytes: std::sync::atomic::AtomicUsize,
    max_bytes: usize,
    notify: tokio::sync::Notify,
}

impl IoBudget {
    fn new(max_bytes: usize) -> Self {
        Self {
            active_bytes: std::sync::atomic::AtomicUsize::new(0),
            max_bytes,
            notify: tokio::sync::Notify::new(),
        }
    }

    async fn acquire(&self, bytes: usize) -> IoBudgetGuard<'_> {
        use std::sync::atomic::Ordering;
        loop {
            let current = self.active_bytes.load(Ordering::Acquire);
            let can_proceed = if bytes <= self.max_bytes {
                current
                    .checked_add(bytes)
                    .map_or(false, |sum| sum <= self.max_bytes)
            } else {
                current == 0
            };
            if can_proceed {
                if self
                    .active_bytes
                    .compare_exchange_weak(
                        current,
                        current + bytes,
                        Ordering::AcqRel,
                        Ordering::Acquire,
                    )
                    .is_ok()
                {
                    return IoBudgetGuard {
                        budget: self,
                        bytes,
                    };
                }
                continue;
            }
            tokio::time::timeout(
                std::time::Duration::from_millis(200),
                self.notify.notified(),
            )
            .await
            .ok();
        }
    }

    fn release(&self, bytes: usize) {
        use std::sync::atomic::Ordering;
        self.active_bytes.fetch_sub(bytes, Ordering::Release);
        self.notify.notify_waiters();
    }
}

struct IoBudgetGuard<'a> {
    budget: &'a IoBudget,
    bytes: usize,
}

impl Drop for IoBudgetGuard<'_> {
    fn drop(&mut self) {
        self.budget.release(self.bytes);
    }
}

fn io_budget() -> &'static IoBudget {
    static BUDGET: OnceLock<IoBudget> = OnceLock::new();
    BUDGET.get_or_init(|| IoBudget::new(DEFAULT_IO_BUDGET_BYTES))
}

fn pool_err(err: PoolError) -> EngineError {
    EngineError::from(err)
}

fn mounted_variant_source_version_token(
    real_path: &std::path::Path,
    virtual_path: &str,
    mount_table: Option<&MountTable>,
) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut h = DefaultHasher::new();
    let path = real_path.to_string_lossy();
    let stem = path_stem(&path);
    let virtual_stem = path_stem(virtual_path);

    let mut candidates: Vec<(String, String)> = Vec::with_capacity(VARIANT_EXTENSIONS.len() + 1);
    candidates.push((path.to_string(), virtual_path.to_string()));
    for ext in VARIANT_EXTENSIONS {
        let candidate = format!("{}.{}", stem, ext);
        if candidate != path {
            candidates.push((candidate, format!("{}.{}", virtual_stem, ext)));
        }
    }

    for (candidate, virtual_candidate) in candidates {
        candidate.hash(&mut h);
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
        if let Some(mt) = mount_table {
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

fn image_decode_request(source: &ImageSource, encoded_bytes: usize, cache_hit: bool) -> IoRequest {
    IoRequest::DecodeImage {
        backend: match source {
            ImageSource::Filesystem => BackendKind::Filesystem,
            ImageSource::MountCode { .. } => BackendKind::Filesystem,
            ImageSource::Pack { .. } => BackendKind::Pack,
        },
        request: RequestKind::Async,
        priority: PriorityClass::ForegroundAsync,
        encoded_bytes,
        cache_hit,
    }
}

#[cfg(test)]
static TEST_SCHEDULER_RUNS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

#[cfg(test)]
static TEST_PRELOAD_CACHE_HOOK: std::sync::Mutex<
    Option<(
        std::sync::Arc<tokio::sync::Notify>,
        std::sync::Arc<tokio::sync::Notify>,
    )>,
> = std::sync::Mutex::new(None);

#[cfg(test)]
static TEST_PRELOAD_DECODE_STARTED: std::sync::Mutex<Option<std::sync::Arc<tokio::sync::Notify>>> =
    std::sync::Mutex::new(None);

pub struct ReadImageResult {
    pub cache_path: String,
    pub image: DecodedImage,
    pub source_generation: u64,
}

struct WorkerImageSource {
    cache_path: String,
    read_path: String,
    source: ImageSource,
    source_generation: u64,
}

async fn run_image_job_with_scheduler<T, F>(
    scheduler: Arc<IoScheduler>,
    encoded_bytes: usize,
    cache_hit: bool,
    source: ImageSource,
    job: F,
) -> Result<T, EngineError>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    #[cfg(test)]
    TEST_SCHEDULER_RUNS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

    scheduler
        .run_async(image_decode_request(&source, encoded_bytes, cache_hit), job)
        .await
        .map_err(pool_err)
}

async fn cached_preload_result_with_scheduler(
    scheduler: Arc<IoScheduler>,
    path: String,
    generation: u64,
    source: ImageSource,
    game_cache_dir: Option<String>,
    gpu_caps: GpuCapsSnapshot,
    mount_table: Option<Arc<MountTable>>,
) -> PreloadResult {
    let key = match current_image_cache_key(&path, generation, &source, mount_table.as_deref()) {
        Ok(key) => key,
        Err(err) => return (path, Err(format!("{:?}", err))),
    };
    let cached = {
        let mut cache = image_cache::global_cache();
        cache.get(&key)
    };

    match cached {
        Some(cached) => {
            let dims = (cached.image.width, cached.image.height);
            let encoded_bytes = cached.image.rgba.len();
            match run_image_job_with_scheduler(scheduler, encoded_bytes, true, source, move || dims)
                .await
            {
                Ok(dims) => (path, Ok(dims)),
                Err(err) => (path, Err(format!("{:?}", err))),
            }
        }
        None => {
            decode_preload_result_with_scheduler(
                scheduler,
                path,
                generation,
                source,
                game_cache_dir,
                gpu_caps,
                mount_table,
            )
            .await
        }
    }
}

#[cfg(test)]
async fn pause_after_preload_cache_classification() {
    let hook = TEST_PRELOAD_CACHE_HOOK.lock().unwrap().clone();
    if let Some((classified, resume)) = hook {
        classified.notify_waiters();
        resume.notified().await;
    }
}

#[cfg(test)]
fn notify_preload_decode_started() {
    if let Some(notify) = TEST_PRELOAD_DECODE_STARTED.lock().unwrap().clone() {
        notify.notify_waiters();
    }
}

fn worker_image_source(
    path: &str,
    expected_generation: u64,
    source: &ImageSource,
    mount_table: Option<&MountTable>,
) -> Result<WorkerImageSource, EngineError> {
    match source {
        ImageSource::Filesystem => Ok(WorkerImageSource {
            cache_path: path.to_string(),
            read_path: path.to_string(),
            source: source.clone(),
            source_generation: expected_generation,
        }),
        ImageSource::MountCode {
            virtual_path,
            relative_path,
        } => {
            let mt = mount_table.ok_or_else(|| {
                EngineError::new(ErrorCode::ImageReadError)
                    .with_detail(format!("missing mount table for image '{}'", virtual_path))
            })?;
            let resolved = mt.resolve_code_path(virtual_path).ok_or_else(|| {
                EngineError::new(ErrorCode::ImageReadError)
                    .with_detail(format!("failed to re-resolve image '{}'", virtual_path))
            })?;

            match resolved.real_path {
                Some(real_path) => Ok(WorkerImageSource {
                    cache_path: virtual_path.clone(),
                    read_path: real_path.to_string_lossy().into_owned(),
                    source: ImageSource::Filesystem,
                    source_generation: mounted_variant_source_version_token(
                        &real_path,
                        virtual_path,
                        mount_table,
                    ),
                }),
                None => Ok(WorkerImageSource {
                    cache_path: virtual_path.clone(),
                    read_path: virtual_path.clone(),
                    source: ImageSource::Pack {
                        relative_path: relative_path.clone(),
                    },
                    source_generation: resolved.source_mounted_at,
                }),
            }
        }
        ImageSource::Pack { relative_path } => {
            let mt = mount_table.ok_or_else(|| {
                EngineError::new(ErrorCode::ImageReadError)
                    .with_detail(format!("missing mount table for pack image '{}'", path))
            })?;
            let resolved = mt.resolve_code_path(path).ok_or_else(|| {
                EngineError::new(ErrorCode::ImageReadError)
                    .with_detail(format!("failed to re-resolve pack image '{}'", path))
            })?;

            match resolved.real_path {
                Some(real_path) => Ok(WorkerImageSource {
                    cache_path: path.to_string(),
                    read_path: real_path.to_string_lossy().into_owned(),
                    source: ImageSource::Filesystem,
                    source_generation: resolved.source_mounted_at,
                }),
                None => Ok(WorkerImageSource {
                    cache_path: path.to_string(),
                    read_path: path.to_string(),
                    source: ImageSource::Pack {
                        relative_path: relative_path.clone(),
                    },
                    source_generation: resolved.source_mounted_at,
                }),
            }
        }
    }
}

fn read_image_source(
    path: &str,
    source: &ImageSource,
    mount_table: Option<&MountTable>,
) -> Result<Vec<u8>, EngineError> {
    match source {
        ImageSource::Filesystem => std::fs::read(path).map_err(|e| {
            EngineError::new(ErrorCode::ImageReadError)
                .with_detail(format!("failed to read file: {}", e))
        }),
        ImageSource::MountCode { virtual_path, .. } => Err(EngineError::new(
            ErrorCode::ImageReadError,
        )
        .with_detail(format!(
            "mount-backed image '{}' must be re-resolved before read",
            virtual_path
        ))),
        ImageSource::Pack { relative_path } => {
            let mt = mount_table.ok_or_else(|| {
                EngineError::new(ErrorCode::ImageReadError)
                    .with_detail(format!("missing mount table for pack image '{}'", path))
            })?;
            mt.read_range_limited(relative_path, 0, None, MAX_READ_LENGTH)
                .map_err(|e| {
                    EngineError::new(ErrorCode::ImageReadError)
                        .with_detail(format!("failed to read pack image '{}': {}", path, e))
                })
        }
    }
}

fn estimate_image_source_size(
    path: &str,
    source: &ImageSource,
    mount_table: Option<&MountTable>,
) -> usize {
    match source {
        ImageSource::Filesystem => std::fs::metadata(path)
            .map(|m| m.len() as usize)
            .unwrap_or(2048 * 2048 * 4),
        ImageSource::MountCode {
            virtual_path,
            relative_path,
        } => mount_table
            .and_then(|mt| mt.resolve_code_path(virtual_path))
            .and_then(|resolved| match resolved.real_path {
                Some(real_path) => std::fs::metadata(real_path).ok().map(|m| m.len() as usize),
                None => mount_table
                    .and_then(|mt| mt.entry_size(relative_path))
                    .map(|size| size as usize),
            })
            .unwrap_or(2048 * 2048 * 4),
        ImageSource::Pack { relative_path } => mount_table
            .and_then(|mt| mt.entry_size(relative_path))
            .map(|size| size as usize)
            .unwrap_or(2048 * 2048 * 4),
    }
}

fn current_image_cache_key(
    path: &str,
    expected_generation: u64,
    source: &ImageSource,
    mount_table: Option<&MountTable>,
) -> Result<image_cache::ImageCacheKey, EngineError> {
    match source {
        ImageSource::Filesystem => Ok((path.to_string(), expected_generation)),
        ImageSource::MountCode { .. } | ImageSource::Pack { .. } => {
            let worker_source =
                worker_image_source(path, expected_generation, source, mount_table)?;
            Ok((worker_source.cache_path, worker_source.source_generation))
        }
    }
}

// ---------------------------------------------------------------------------
// Public async operations
// ---------------------------------------------------------------------------

/// Decode a single image to RGBA8 or GPU-compressed format.
///
/// Returns the decoded image (RGBA or compressed).  Handles:
/// - LRU cache lookup
/// - Budget acquisition
/// - Decode semaphore
/// - Variant selection (KTX2 compressed, companion upgrade, RGBA fallback)
/// - Derived disk cache
/// - LRU cache insert
pub async fn read_image_rgba8(
    scheduler: Arc<IoScheduler>,
    path: String,
    target_width: Option<u32>,
    target_height: Option<u32>,
    cache_generation: u64,
    source: ImageSource,
    game_cache_dir: Option<String>,
    gpu_caps: GpuCapsSnapshot,
    mount_table: Option<Arc<MountTable>>,
) -> Result<ReadImageResult, EngineError> {
    let has_resize = target_width.is_some() && target_height.is_some();
    let io_cache_key =
        current_image_cache_key(&path, cache_generation, &source, mount_table.as_deref())?;

    // LRU cache fast path (full-resolution decodes only).
    if !has_resize {
        if let Some(cached) = image_cache::global_cache().get(&io_cache_key) {
            debug!(
                "read_image_rgba8 cache hit: {} g{}",
                io_cache_key.0, io_cache_key.1
            );
            let cached_image = cached.image;
            let encoded_bytes = cached_image.rgba.len();
            return run_image_job_with_scheduler(
                scheduler,
                encoded_bytes,
                true,
                source,
                move || ReadImageResult {
                    cache_path: io_cache_key.0,
                    image: DecodedImage::Rgba(cached_image),
                    source_generation: io_cache_key.1,
                },
            )
            .await;
        }
    }

    let start = tokio::time::Instant::now();

    // Budget estimation.
    let primary_size = estimate_image_source_size(&path, &source, mount_table.as_deref());
    let pre_estimate = max_variant_source_size(&path, primary_size, mount_table.as_deref())
        .saturating_mul(16)
        .clamp(16 * 1024, 256 * 1024 * 1024);
    let _budget = io_budget().acquire(pre_estimate).await;

    // Limit concurrent decodes.
    let _permit = image_decode_semaphore().acquire().await.unwrap();

    let gcd = game_cache_dir.clone();
    let mt = mount_table.clone();
    let path_for_decode = path.clone();
    let task = run_image_job_with_scheduler(
        scheduler,
        primary_size,
        false,
        source.clone(),
        move || -> Result<ReadImageResult, EngineError> {
            let worker_source =
                worker_image_source(&path_for_decode, cache_generation, &source, mt.as_deref())?;
            let data = read_image_source(
                &worker_source.read_path,
                &worker_source.source,
                mt.as_deref(),
            )?;

            let tw = target_width.unwrap_or(0);
            let th = target_height.unwrap_or(0);
            let variant =
                select_variant(&path_for_decode, data, &gpu_caps, has_resize, mt.as_deref())?;

            let variant_kind = variant.variant_kind();
            let cache_fmt = variant.gpu_format();

            let cache_key = derived_cache::DerivedKey {
                asset_path: worker_source.cache_path.clone(),
                source_generation: worker_source.source_generation,
                gpu_format: cache_fmt,
                variant_kind,
                target_width: tw,
                target_height: th,
            };

            if let Some(ref cache_dir) = gcd {
                if let Some(cached) =
                    derived_cache::load_derived(std::path::Path::new(cache_dir), &cache_key)
                {
                    tracing::debug!("derived cache hit: {}", path_for_decode);
                    return Ok(ReadImageResult {
                        cache_path: worker_source.cache_path,
                        image: cached,
                        source_generation: worker_source.source_generation,
                    });
                }
            }

            let result = decode_selected_variant(variant, has_resize, target_width, target_height)?;

            if let Some(ref cache_dir) = gcd {
                derived_cache::save_derived(std::path::Path::new(cache_dir), &cache_key, &result);
            }

            let _ = variant_kind;
            Ok(ReadImageResult {
                cache_path: worker_source.cache_path,
                image: result,
                source_generation: worker_source.source_generation,
            })
        },
    );

    match task.await.and_then(|result| result) {
        Ok(decoded) => {
            if !has_resize {
                if let DecodedImage::Rgba(ref rgba_img) = decoded.image {
                    image_cache::global_cache().insert(
                        (decoded.cache_path.clone(), decoded.source_generation),
                        rgba_img.clone(),
                    );
                }
            }
            debug!(
                "read_image_rgba8: {} ({}x{}) {:?} in {:.2?}",
                path,
                decoded.image.width(),
                decoded.image.height(),
                match &decoded.image {
                    DecodedImage::Rgba(_) => "RGBA",
                    DecodedImage::Compressed(_) => "compressed",
                },
                start.elapsed()
            );
            Ok(decoded)
        }
        Err(e) => {
            warn!("read_image_rgba8 decode error: {:?}", e);
            Err(e)
        }
    }
}

/// Preload multiple images in parallel.
///
/// Returns `Vec<(path, Result<(width, height), error_msg>)>`.
type PreloadResult = (String, Result<(u32, u32), String>);

async fn decode_preload_result_with_scheduler(
    scheduler: Arc<IoScheduler>,
    path: String,
    cache_generation: u64,
    source: ImageSource,
    game_cache_dir: Option<String>,
    gpu_caps: GpuCapsSnapshot,
    mount_table: Option<Arc<MountTable>>,
) -> PreloadResult {
    let fallback_path = path.clone();
    #[cfg(test)]
    notify_preload_decode_started();
    let primary_size = estimate_image_source_size(&path, &source, mount_table.as_deref());
    let pre_estimate = max_variant_source_size(&path, primary_size, mount_table.as_deref())
        .saturating_mul(16)
        .clamp(16 * 1024, 256 * 1024 * 1024);
    let _budget = io_budget().acquire(pre_estimate).await;
    let _permit = image_decode_semaphore().acquire().await.unwrap();

    run_image_job_with_scheduler(scheduler, primary_size, false, source.clone(), move || {
        let worker_source =
            match worker_image_source(&path, cache_generation, &source, mount_table.as_deref()) {
                Ok(source) => source,
                Err(e) => return (path, Err(format!("{:?}", e))),
            };
        let data = match read_image_source(
            &worker_source.read_path,
            &worker_source.source,
            mount_table.as_deref(),
        ) {
            Ok(data) => data,
            Err(e) => return (path, Err(format!("{:?}", e))),
        };

        match select_variant(&path, data, &gpu_caps, false, mount_table.as_deref()) {
            Ok(variant) => {
                let variant_kind = variant.variant_kind();
                let cache_fmt = variant.gpu_format();
                let cache_key = derived_cache::DerivedKey {
                    asset_path: path.clone(),
                    source_generation: worker_source.source_generation,
                    gpu_format: cache_fmt,
                    variant_kind,
                    target_width: 0,
                    target_height: 0,
                };

                if let Some(ref cache_dir) = game_cache_dir {
                    if let Some(cached) =
                        derived_cache::load_derived(std::path::Path::new(cache_dir), &cache_key)
                    {
                        tracing::debug!("preload_images derived cache hit: {}", path);
                        let dims = (cached.width(), cached.height());
                        if let DecodedImage::Rgba(ref rgba) = cached {
                            image_cache::global_cache().insert(
                                (path.clone(), worker_source.source_generation),
                                rgba.clone(),
                            );
                        }
                        return (path, Ok(dims));
                    }
                }

                let decoded = match decode_selected_variant(variant, false, None, None) {
                    Ok(v) => v,
                    Err(e) => return (path, Err(format!("{:?}", e))),
                };

                let dims = (decoded.width(), decoded.height());
                if let DecodedImage::Rgba(ref rgba) = decoded {
                    image_cache::global_cache().insert(
                        (path.clone(), worker_source.source_generation),
                        rgba.clone(),
                    );
                }
                if let Some(ref cache_dir) = game_cache_dir {
                    derived_cache::save_derived(
                        std::path::Path::new(cache_dir),
                        &cache_key,
                        &decoded,
                    );
                }
                (path, Ok(dims))
            }
            Err(e) => (path, Err(format!("{:?}", e))),
        }
    })
    .await
    .unwrap_or_else(|err| (fallback_path, Err(format!("{:?}", err))))
}

pub async fn preload_images(
    scheduler: Arc<IoScheduler>,
    entries: Vec<(String, u64, ImageSource)>,
    game_cache_dir: Option<String>,
    gpu_caps: GpuCapsSnapshot,
    mount_table: Option<Arc<MountTable>>,
) -> Vec<(String, Result<(u32, u32), String>)> {
    let start = tokio::time::Instant::now();
    let total = entries.len();
    debug!("preload_images: {} images", total);

    let mut slots: Vec<Option<PreloadResult>> = vec![None; total];
    let mut handles: Vec<(usize, String, tokio::task::JoinHandle<PreloadResult>)> = Vec::new();
    {
        let cache = image_cache::global_cache();
        for (i, (path, generation, source)) in entries.into_iter().enumerate() {
            let key =
                match current_image_cache_key(&path, generation, &source, mount_table.as_deref()) {
                    Ok(key) => key,
                    Err(_) => {
                        handles.push((
                            i,
                            path.clone(),
                            tokio::spawn(decode_preload_result_with_scheduler(
                                Arc::clone(&scheduler),
                                path,
                                generation,
                                source,
                                game_cache_dir.clone(),
                                gpu_caps.clone(),
                                mount_table.clone(),
                            )),
                        ));
                        continue;
                    }
                };
            let scheduler = Arc::clone(&scheduler);
            let path_for_error = path.clone();
            let gcd = game_cache_dir.clone();
            let gpu = gpu_caps.clone();
            let mt = mount_table.clone();
            let handle = if cache.contains(&key) {
                tokio::spawn(async move {
                    #[cfg(test)]
                    pause_after_preload_cache_classification().await;

                    cached_preload_result_with_scheduler(
                        scheduler, path, key.1, source, gcd, gpu, mt,
                    )
                    .await
                })
            } else {
                tokio::spawn(decode_preload_result_with_scheduler(
                    scheduler, path, generation, source, gcd, gpu, mt,
                ))
            };
            handles.push((i, path_for_error, handle));
        }
    }

    for (idx, fallback_path, handle) in handles {
        let result: PreloadResult = match handle.await {
            Ok(r) => r,
            Err(task_err) => {
                warn!("preload_images task panic/cancel: {}", task_err);
                (fallback_path, Err(format!("task error: {}", task_err)))
            }
        };
        slots[idx] = Some(result);
    }

    let results: Vec<PreloadResult> = slots
        .into_iter()
        .map(|s| s.expect("[BUG] preload_images: unfilled slot"))
        .collect();

    debug!(
        "preload_images completed: {}/{} images in {:.2?}",
        results.len(),
        total,
        start.elapsed()
    );
    results
}

/// Clear all image caches.
///
/// Clears the in-memory LRU cache and the per-game derived texture disk cache.
pub fn clear_image_cache(game_cache_dir: Option<&str>) {
    image_cache::global_cache().clear();
    if let Some(dir) = game_cache_dir {
        let derived = derived_cache::derived_cache_dir(std::path::Path::new(dir));
        if derived.exists() {
            let _ = std::fs::remove_dir_all(&derived);
            debug!("Derived texture cache cleared: {}", derived.display());
        }
    }
    debug!("Image cache cleared");
}

/// Get image cache statistics.
pub fn get_image_cache_stats() -> ImageCacheStats {
    let stats = image_cache::global_cache().stats();
    ImageCacheStats {
        entries: stats.entries,
        size_bytes: stats.size_bytes,
        max_bytes: stats.max_bytes,
        hits: stats.hits,
        misses: stats.misses,
        hit_rate: stats.hit_rate(),
    }
}

// ---------------------------------------------------------------------------
// Variant selection helpers
// ---------------------------------------------------------------------------

const VARIANT_PRIMARY_RGBA: u8 = 0;
const VARIANT_PRIMARY_COMPRESSED: u8 = 1;
const VARIANT_FALLBACK_RGBA: u8 = 2;
const VARIANT_COMPANION_COMPRESSED: u8 = 3;

/// Result of variant selection.
enum VariantDecision {
    Compressed(shared::protocol::io_cmd::CompressedImage),
    CompressedFromCompanion(shared::protocol::io_cmd::CompressedImage),
    DecodeRgba {
        data: Vec<u8>,
        path_hint: String,
        variant_kind: u8,
    },
}

impl VariantDecision {
    fn gpu_format(&self) -> u32 {
        match self {
            Self::Compressed(img) | Self::CompressedFromCompanion(img) => img.vk_format,
            Self::DecodeRgba { .. } => 0,
        }
    }

    fn variant_kind(&self) -> u8 {
        match self {
            Self::Compressed(_) => VARIANT_PRIMARY_COMPRESSED,
            Self::CompressedFromCompanion(_) => VARIANT_COMPANION_COMPRESSED,
            Self::DecodeRgba { variant_kind, .. } => *variant_kind,
        }
    }
}

fn ktx2_vk_format_code(format: &crate::ktx2::VkFormat) -> u32 {
    match format {
        crate::ktx2::VkFormat::Etc2R8G8B8UnormBlock => 147,
        crate::ktx2::VkFormat::Etc2R8G8B8A8UnormBlock => 151,
        crate::ktx2::VkFormat::Astc4x4UnormBlock => 157,
        crate::ktx2::VkFormat::Astc6x6UnormBlock => 163,
        crate::ktx2::VkFormat::Astc8x8UnormBlock => 169,
        crate::ktx2::VkFormat::Unknown(_) => 0,
    }
}

fn gpu_supports_vk_format(vk_format: u32, gpu_caps: &GpuCapsSnapshot) -> bool {
    match vk_format {
        147 | 151 => gpu_caps.etc2,
        157 | 163 | 169 => gpu_caps.astc,
        _ => false,
    }
}

fn try_ktx2_as_compressed(
    data: &[u8],
    gpu_caps: &GpuCapsSnapshot,
) -> Option<shared::protocol::io_cmd::CompressedImage> {
    let ktx2 = crate::ktx2::parse_ktx2(data).ok()?;
    let vk_format = ktx2_vk_format_code(&ktx2.header.format);
    if vk_format == 0 || !gpu_supports_vk_format(vk_format, gpu_caps) {
        return None;
    }
    Some(shared::protocol::io_cmd::CompressedImage {
        width: ktx2.header.width,
        height: ktx2.header.height,
        vk_format,
        data: Arc::new(ktx2.data.to_vec()),
    })
}

fn mount_relative_path(path: &str, mt: &MountTable) -> Option<String> {
    if let Some(relative) = path.strip_prefix("/code/") {
        return Some(relative.to_string());
    }
    let code_dir = mt.code_dir();
    std::path::Path::new(path)
        .strip_prefix(&code_dir)
        .ok()
        .and_then(|p| p.to_str().map(|s| s.to_string()))
}

fn read_filesystem_companion(path: &str) -> std::io::Result<Vec<u8>> {
    let candidate = std::path::Path::new(path);
    let parent = candidate
        .parent()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "missing parent"))?;
    let canonical_parent = std::fs::canonicalize(parent)?;
    let canonical_file = std::fs::canonicalize(candidate)?;
    if !canonical_file.starts_with(&canonical_parent) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "companion escapes parent directory",
        ));
    }
    let mut file = std::fs::File::open(&canonical_file)?;
    let size = file.metadata()?.len();
    if size > MAX_READ_LENGTH {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "companion exceeds read limit",
        ));
    }
    let mut buf = Vec::with_capacity(size as usize);
    std::io::Read::read_to_end(&mut file, &mut buf)?;
    Ok(buf)
}

fn find_companion(
    path: &str,
    try_extensions: &[&str],
    mount_table: Option<&MountTable>,
) -> Option<(String, Vec<u8>)> {
    let stem = path_stem(path);
    for ext in try_extensions {
        let companion = format!("{}.{}", stem, ext);
        if let Some(mt) = mount_table {
            if let Some(relative) = mount_relative_path(&companion, mt) {
                if let Some(size) = mt.entry_size(&relative) {
                    if size <= MAX_READ_LENGTH {
                        if let Ok(data) = mt.read_range_limited(&relative, 0, None, MAX_READ_LENGTH)
                        {
                            return Some((companion, data));
                        }
                    }
                }
            }
        }
        if let Ok(data) = read_filesystem_companion(&companion) {
            return Some((companion, data));
        }
    }
    None
}

fn max_variant_source_size(
    path: &str,
    primary_size: usize,
    mount_table: Option<&MountTable>,
) -> usize {
    let mut max_size = primary_size;
    let stem = path_stem(path);
    for ext in VARIANT_EXTENSIONS {
        let candidate = format!("{}.{}", stem, ext);
        if candidate == path {
            continue;
        }
        if let Ok(meta) = std::fs::metadata(&candidate) {
            max_size = max_size.max(meta.len() as usize);
            continue;
        }
        if let Some(mt) = mount_table {
            if let Some(relative) = mount_relative_path(&candidate, mt) {
                if let Some(size) = mt.entry_size(&relative) {
                    max_size = max_size.max(size as usize);
                }
            }
        }
    }
    max_size
}

const RGBA_FALLBACK_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "webp"];

fn select_variant(
    primary_path: &str,
    primary_data: Vec<u8>,
    gpu_caps: &GpuCapsSnapshot,
    has_resize: bool,
    mount_table: Option<&MountTable>,
) -> Result<VariantDecision, EngineError> {
    // KTX2 primary
    if crate::ktx2::is_ktx2(&primary_data) {
        if !has_resize {
            if let Some(compressed) = try_ktx2_as_compressed(&primary_data, gpu_caps) {
                tracing::info!(
                    "variant: compressed direct -- {} ({}x{} fmt={})",
                    primary_path,
                    compressed.width,
                    compressed.height,
                    compressed.vk_format,
                );
                return Ok(VariantDecision::Compressed(compressed));
            }
        }

        if let Some((fb_path, fb_data)) =
            find_companion(primary_path, RGBA_FALLBACK_EXTENSIONS, mount_table)
        {
            tracing::info!("variant: RGBA fallback -- {} -> {}", primary_path, fb_path);
            return Ok(VariantDecision::DecodeRgba {
                data: fb_data,
                path_hint: fb_path,
                variant_kind: VARIANT_FALLBACK_RGBA,
            });
        }

        let reason = if has_resize {
            "resize requested but no standard-image fallback found"
        } else {
            "GPU does not support this compressed format and no fallback found"
        };
        return Err(EngineError::new(ErrorCode::ImageReadError)
            .with_detail(format!("{} for '{}'", reason, primary_path)));
    }

    // Standard image primary -- compressed upgrade
    if !has_resize && (gpu_caps.etc2 || gpu_caps.astc) {
        if let Some((_, companion_data)) = find_companion(primary_path, &["ktx2"], mount_table) {
            if let Some(compressed) = try_ktx2_as_compressed(&companion_data, gpu_caps) {
                tracing::info!(
                    "variant: compressed upgrade -- {} ({}x{} fmt={})",
                    primary_path,
                    compressed.width,
                    compressed.height,
                    compressed.vk_format,
                );
                return Ok(VariantDecision::CompressedFromCompanion(compressed));
            }
        }
    }

    Ok(VariantDecision::DecodeRgba {
        data: primary_data,
        path_hint: primary_path.to_string(),
        variant_kind: VARIANT_PRIMARY_RGBA,
    })
}

fn decode_rgba(
    data: &[u8],
    path_hint: &str,
    has_resize: bool,
    target_width: Option<u32>,
    target_height: Option<u32>,
) -> Result<NormalizedImage, EngineError> {
    let mut img = crate::fast_image_decoder::decode_image_fast(data, Some(path_hint))?;
    if has_resize {
        let tw = target_width.unwrap();
        let th = target_height.unwrap();
        if img.width > tw || img.height > th {
            img = crate::fast_image_decoder::resize_image(img, tw, th);
        }
    }
    Ok(img)
}

fn decode_selected_variant(
    variant: VariantDecision,
    has_resize: bool,
    target_width: Option<u32>,
    target_height: Option<u32>,
) -> Result<DecodedImage, EngineError> {
    match variant {
        VariantDecision::Compressed(img) | VariantDecision::CompressedFromCompanion(img) => {
            Ok(DecodedImage::Compressed(img))
        }
        VariantDecision::DecodeRgba {
            data, path_hint, ..
        } => {
            let img = decode_rgba(&data, &path_hint, has_resize, target_width, target_height)?;
            Ok(DecodedImage::Rgba(img))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, atomic::Ordering};

    use shared::{
        device::gpu_caps::GpuCapsSnapshot,
        protocol::io_cmd::NormalizedImage,
        vfs::{DirSource, MountTable, PackSource},
    };
    use shared::vfs::package::PackageWriter;
    use tokio::sync::Notify;

    use super::{
        preload_images, read_image_rgba8, ImageSource,
        run_image_job_with_scheduler, worker_image_source, read_image_source,
        mounted_variant_source_version_token,
        TEST_SCHEDULER_RUNS, TEST_PRELOAD_CACHE_HOOK, TEST_PRELOAD_DECODE_STARTED,
    };
    use crate::scheduler::IoScheduler;

    fn scheduler_run_count() -> usize {
        TEST_SCHEDULER_RUNS.load(Ordering::Relaxed)
    }

    fn reset_scheduler_run_count() {
        TEST_SCHEDULER_RUNS.store(0, Ordering::Relaxed);
    }

    fn install_preload_cache_hook() -> (Arc<Notify>, Arc<Notify>) {
        let classified = Arc::new(Notify::new());
        let resume = Arc::new(Notify::new());
        *TEST_PRELOAD_CACHE_HOOK.lock().unwrap() = Some((classified.clone(), resume.clone()));
        (classified, resume)
    }

    fn clear_preload_cache_hook() {
        *TEST_PRELOAD_CACHE_HOOK.lock().unwrap() = None;
    }

    fn install_preload_decode_started_hook() -> Arc<Notify> {
        let notify = Arc::new(Notify::new());
        *TEST_PRELOAD_DECODE_STARTED.lock().unwrap() = Some(notify.clone());
        notify
    }

    fn clear_preload_decode_started_hook() {
        *TEST_PRELOAD_DECODE_STARTED.lock().unwrap() = None;
    }

    fn write_test_package(path: &std::path::Path, entry: &str, data: &[u8]) {
        let file = std::fs::File::create(path).unwrap();
        let mut writer = PackageWriter::new(std::io::BufWriter::new(file)).unwrap();
        writer.add_entry(entry, data).unwrap();
        writer.finish("test", "1.0").unwrap();
    }

    fn tiny_png() -> [u8; 70] {
        [
            137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1,
            8, 6, 0, 0, 0, 31, 21, 196, 137, 0, 0, 0, 13, 73, 68, 65, 84, 120, 156, 99, 248, 207,
            192, 240, 31, 0, 5, 0, 1, 255, 137, 153, 61, 29, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66,
            96, 130,
        ]
    }

    #[test]
    fn uncached_image_decode_requests_use_image_pool() {
        let scheduler = Arc::new(IoScheduler::new(53));
        reset_scheduler_run_count();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        let thread_name = runtime.block_on(run_image_job_with_scheduler(
            Arc::clone(&scheduler),
            64 * 1024,
            false,
            ImageSource::Filesystem,
            || {
                std::thread::current()
                    .name()
                    .unwrap_or("unnamed")
                    .to_string()
            },
        ));

        assert!(thread_name.unwrap().contains("io-image-host-53"));
        assert_eq!(scheduler_run_count(), 1);
    }

    #[test]
    fn cached_read_image_path_still_flows_through_scheduler_helper() {
        let scheduler = Arc::new(IoScheduler::new(59));
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let path = "/tmp/cached-image.png".to_string();
        let cache_generation = 7;
        let cached = NormalizedImage::new(1, 1, vec![255, 0, 0, 255]);

        reset_scheduler_run_count();
        crate::image_cache::global_cache().clear();
        crate::image_cache::global_cache().insert((path.clone(), cache_generation), cached.clone());

        let result = runtime.block_on(read_image_rgba8(
            Arc::clone(&scheduler),
            path.clone(),
            None,
            None,
            cache_generation,
            ImageSource::Filesystem,
            None,
            GpuCapsSnapshot::default(),
            None,
        ));

        let decoded = result.expect("cached read_image_rgba8 should succeed");
        assert_eq!(decoded.source_generation, cache_generation);
        match decoded.image {
            shared::protocol::io_cmd::DecodedImage::Rgba(image) => {
                assert_eq!(image.width, 1);
                assert_eq!(image.height, 1);
            }
            shared::protocol::io_cmd::DecodedImage::Compressed(_) => {
                panic!("expected cached RGBA image")
            }
        }
        assert_eq!(scheduler_run_count(), 1);
        assert_eq!(scheduler.pools().spawned_pool_count(), 0);
        crate::image_cache::global_cache().clear();
    }

    #[test]
    fn mixed_preload_batches_keep_decode_work_running_while_cached_tasks_wait() {
        let scheduler = Arc::new(IoScheduler::new(63));
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let dir = std::env::temp_dir().join("migo_mixed_preload_parallelism");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let uncached_path = dir.join("tiny.png");
        std::fs::write(
            &uncached_path,
            [
                137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0,
                1, 8, 6, 0, 0, 0, 31, 21, 196, 137, 0, 0, 0, 13, 73, 68, 65, 84, 120, 156, 99, 248,
                207, 192, 240, 31, 0, 5, 0, 1, 255, 137, 153, 61, 29, 0, 0, 0, 0, 73, 69, 78, 68,
                174, 66, 96, 130,
            ],
        )
        .unwrap();
        let uncached_path = uncached_path.to_string_lossy().into_owned();
        let cached_path = "/tmp/cached-preload-parallel.png".to_string();

        reset_scheduler_run_count();
        crate::image_cache::global_cache().clear();
        crate::image_cache::global_cache().insert(
            (cached_path.clone(), 1),
            NormalizedImage::new(2, 2, vec![255; 2 * 2 * 4]),
        );

        let (classified, resume) = install_preload_cache_hook();
        let decode_started = install_preload_decode_started_hook();
        let results = runtime.block_on(async {
            let preload = tokio::spawn(preload_images(
                Arc::clone(&scheduler),
                vec![
                    (cached_path.clone(), 1, ImageSource::Filesystem),
                    (uncached_path.clone(), 2, ImageSource::Filesystem),
                ],
                None,
                GpuCapsSnapshot::default(),
                None,
            ));

            classified.notified().await;
            tokio::time::timeout(
                std::time::Duration::from_millis(200),
                decode_started.notified(),
            )
            .await
            .expect("uncached decode should start before cached task resumes");
            resume.notify_waiters();
            preload.await.unwrap()
        });
        clear_preload_cache_hook();
        clear_preload_decode_started_hook();

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].1, Ok((2, 2)));
        assert_eq!(results[1].1, Ok((1, 1)));
        crate::image_cache::global_cache().clear();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cached_preload_entries_still_flow_through_scheduler_helper() {
        let scheduler = Arc::new(IoScheduler::new(61));
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let path = "/tmp/cached-preload-image.png".to_string();
        let cache_generation = 9;
        let cached = NormalizedImage::new(2, 3, vec![255; 2 * 3 * 4]);

        reset_scheduler_run_count();
        crate::image_cache::global_cache().clear();
        crate::image_cache::global_cache().insert((path.clone(), cache_generation), cached);

        let results = runtime.block_on(preload_images(
            Arc::clone(&scheduler),
            vec![(path.clone(), cache_generation, ImageSource::Filesystem)],
            None,
            GpuCapsSnapshot::default(),
            None,
        ));

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, path);
        assert_eq!(results[0].1, Ok((2, 3)));
        assert_eq!(scheduler_run_count(), 1);
        assert_eq!(scheduler.pools().spawned_pool_count(), 0);
        crate::image_cache::global_cache().clear();
    }

    #[test]
    fn cached_preload_entries_fall_back_cleanly_under_cache_churn() {
        let scheduler = Arc::new(IoScheduler::new(67));
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let dir = std::env::temp_dir().join("migo_cached_preload_fallback");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("tiny.png");
        std::fs::write(
            &path,
            [
                137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0,
                1, 8, 6, 0, 0, 0, 31, 21, 196, 137, 0, 0, 0, 13, 73, 68, 65, 84, 120, 156, 99, 248,
                207, 192, 240, 31, 0, 5, 0, 1, 255, 137, 153, 61, 29, 0, 0, 0, 0, 73, 69, 78, 68,
                174, 66, 96, 130,
            ],
        )
        .unwrap();
        let path = path.to_string_lossy().into_owned();
        let cache_generation = 11;

        reset_scheduler_run_count();
        crate::image_cache::global_cache().clear();
        crate::image_cache::global_cache().insert(
            (path.clone(), cache_generation),
            NormalizedImage::new(9, 9, vec![255; 9 * 9 * 4]),
        );

        let (classified, resume) = install_preload_cache_hook();
        let results = runtime.block_on(async {
            let preload = tokio::spawn(preload_images(
                Arc::clone(&scheduler),
                vec![(path.clone(), cache_generation, ImageSource::Filesystem)],
                None,
                GpuCapsSnapshot::default(),
                None,
            ));

            classified.notified().await;
            crate::image_cache::global_cache().clear();
            resume.notify_waiters();

            preload.await.unwrap()
        });
        clear_preload_cache_hook();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, path);
        assert_eq!(results[0].1, Ok((1, 1)));
        assert_eq!(scheduler_run_count(), 1);
        assert_eq!(scheduler.pools().spawned_pool_count(), 1);
        crate::image_cache::global_cache().clear();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pack_worker_source_refreshes_generation_after_remount() {
        let dir = std::env::temp_dir().join("migo_pack_image_generation_refresh");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let pkg1 = dir.join("base_v1.mpkg");
        let pkg2 = dir.join("base_v2.mpkg");
        write_test_package(&pkg1, "tex.png", b"v1-bytes");
        write_test_package(&pkg2, "tex.png", b"v2-bytes-longer");

        let mount_table = MountTable::new(dir.clone());
        mount_table.swap_base(Arc::new(PackSource::open(&pkg1, "test", "1.0").unwrap()));
        let resolved_v1 = mount_table.resolve_code_path("/code/tex.png").unwrap();
        let generation_v1 = resolved_v1.source_mounted_at;

        mount_table.swap_base(Arc::new(PackSource::open(&pkg2, "test", "1.0").unwrap()));

        let worker_source = worker_image_source(
            "/code/tex.png",
            generation_v1,
            &ImageSource::Pack {
                relative_path: "tex.png".to_string(),
            },
            Some(&mount_table),
        )
        .unwrap();
        let bytes = read_image_source(
            &worker_source.read_path,
            &worker_source.source,
            Some(&mount_table),
        )
        .unwrap();

        assert!(worker_source.source_generation > generation_v1);
        assert_eq!(bytes, b"v2-bytes-longer");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn directory_mount_worker_source_refreshes_generation_after_remount() {
        let dir = std::env::temp_dir().join("migo_dir_image_generation_refresh");
        let _ = std::fs::remove_dir_all(&dir);
        let v1 = dir.join("v1");
        let v2 = dir.join("v2");
        std::fs::create_dir_all(&v1).unwrap();
        std::fs::create_dir_all(&v2).unwrap();
        std::fs::write(v1.join("tex.png"), b"dir-v1").unwrap();
        std::fs::write(v2.join("tex.png"), b"dir-v2-longer").unwrap();

        let mount_table = MountTable::new(dir.clone());
        mount_table.swap_base(Arc::new(DirSource::new(v1.clone())));
        let resolved_v1 = mount_table.resolve_code_path("/code/tex.png").unwrap();
        let generation_v1 = resolved_v1.source_mounted_at;

        mount_table.swap_base(Arc::new(DirSource::new(v2.clone())));

        let worker_source = worker_image_source(
            "/code/tex.png",
            generation_v1,
            &ImageSource::MountCode {
                virtual_path: "/code/tex.png".to_string(),
                relative_path: "tex.png".to_string(),
            },
            Some(&mount_table),
        )
        .unwrap();
        let bytes = read_image_source(
            &worker_source.read_path,
            &worker_source.source,
            Some(&mount_table),
        )
        .unwrap();

        assert_eq!(worker_source.cache_path, "/code/tex.png");
        assert!(worker_source.source_generation > generation_v1);
        assert_eq!(bytes, b"dir-v2-longer");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn directory_mount_worker_source_preserves_variant_token_without_remount() {
        let dir = std::env::temp_dir().join("migo_dir_image_variant_identity");
        let _ = std::fs::remove_dir_all(&dir);
        let base = dir.join("base");
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("tex.png"), b"dir-v1").unwrap();

        let mount_table = MountTable::new(dir.clone());
        mount_table.swap_base(Arc::new(DirSource::new(base.clone())));
        let resolved = mount_table.resolve_code_path("/code/tex.png").unwrap();
        let requested_generation = mounted_variant_source_version_token(
            resolved.real_path.as_ref().unwrap(),
            "/code/tex.png",
            Some(&mount_table),
        );

        let worker_source = worker_image_source(
            "/code/tex.png",
            requested_generation,
            &ImageSource::MountCode {
                virtual_path: "/code/tex.png".to_string(),
                relative_path: "tex.png".to_string(),
            },
            Some(&mount_table),
        )
        .unwrap();

        assert_eq!(worker_source.source_generation, requested_generation);
        assert_eq!(worker_source.cache_path, "/code/tex.png");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn directory_mount_worker_source_detects_in_place_variant_changes_without_remount() {
        let dir = std::env::temp_dir().join("migo_dir_image_variant_change");
        let _ = std::fs::remove_dir_all(&dir);
        let base = dir.join("base");
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("tex.png"), b"dir-v1").unwrap();

        let mount_table = MountTable::new(dir.clone());
        mount_table.swap_base(Arc::new(DirSource::new(base.clone())));
        let resolved = mount_table.resolve_code_path("/code/tex.png").unwrap();
        let requested_generation = mounted_variant_source_version_token(
            resolved.real_path.as_ref().unwrap(),
            "/code/tex.png",
            Some(&mount_table),
        );

        std::fs::write(base.join("tex.ktx2"), b"companion-added").unwrap();

        let worker_source = worker_image_source(
            "/code/tex.png",
            requested_generation,
            &ImageSource::MountCode {
                virtual_path: "/code/tex.png".to_string(),
                relative_path: "tex.png".to_string(),
            },
            Some(&mount_table),
        )
        .unwrap();

        assert_ne!(worker_source.source_generation, requested_generation);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn mount_backed_read_image_cache_hit_is_revalidated_after_directory_remount() {
        let scheduler = Arc::new(IoScheduler::new(71));
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let dir = std::env::temp_dir().join("migo_dir_mount_cache_revalidate_read");
        let _ = std::fs::remove_dir_all(&dir);
        let v1 = dir.join("v1");
        let v2 = dir.join("v2");
        std::fs::create_dir_all(&v1).unwrap();
        std::fs::create_dir_all(&v2).unwrap();
        std::fs::write(v1.join("tex.png"), tiny_png()).unwrap();
        std::fs::write(v2.join("tex.png"), tiny_png()).unwrap();

        let mount_table = Arc::new(MountTable::new(dir.clone()));
        mount_table.swap_base(Arc::new(DirSource::new(v1.clone())));
        let resolved = mount_table.resolve_code_path("/code/tex.png").unwrap();
        let old_generation = mounted_variant_source_version_token(
            resolved.real_path.as_ref().unwrap(),
            "/code/tex.png",
            Some(mount_table.as_ref()),
        );

        crate::image_cache::global_cache().clear();
        crate::image_cache::global_cache().insert(
            ("/code/tex.png".to_string(), old_generation),
            NormalizedImage::new(9, 9, vec![255; 9 * 9 * 4]),
        );

        mount_table.swap_base(Arc::new(DirSource::new(v2.clone())));
        std::fs::write(v2.join("tex.ktx2"), b"changed-companion").unwrap();

        let result = runtime.block_on(read_image_rgba8(
            Arc::clone(&scheduler),
            "/code/tex.png".to_string(),
            None,
            None,
            old_generation,
            ImageSource::MountCode {
                virtual_path: "/code/tex.png".to_string(),
                relative_path: "tex.png".to_string(),
            },
            None,
            GpuCapsSnapshot::default(),
            Some(Arc::clone(&mount_table)),
        ));

        let decoded = result.expect("read_image_rgba8 should revalidate mount-backed cache hit");
        assert_eq!(decoded.cache_path, "/code/tex.png");
        assert_ne!(decoded.source_generation, old_generation);
        assert_eq!(decoded.image.width(), 1);
        assert_eq!(decoded.image.height(), 1);
        crate::image_cache::global_cache().clear();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn mount_backed_preload_cache_hit_is_revalidated_after_directory_remount() {
        let scheduler = Arc::new(IoScheduler::new(73));
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let dir = std::env::temp_dir().join("migo_dir_mount_cache_revalidate_preload");
        let _ = std::fs::remove_dir_all(&dir);
        let v1 = dir.join("v1");
        let v2 = dir.join("v2");
        std::fs::create_dir_all(&v1).unwrap();
        std::fs::create_dir_all(&v2).unwrap();
        std::fs::write(v1.join("tex.png"), tiny_png()).unwrap();
        std::fs::write(v2.join("tex.png"), tiny_png()).unwrap();

        let mount_table = Arc::new(MountTable::new(dir.clone()));
        mount_table.swap_base(Arc::new(DirSource::new(v1.clone())));
        let resolved = mount_table.resolve_code_path("/code/tex.png").unwrap();
        let old_generation = mounted_variant_source_version_token(
            resolved.real_path.as_ref().unwrap(),
            "/code/tex.png",
            Some(mount_table.as_ref()),
        );

        crate::image_cache::global_cache().clear();
        crate::image_cache::global_cache().insert(
            ("/code/tex.png".to_string(), old_generation),
            NormalizedImage::new(9, 9, vec![255; 9 * 9 * 4]),
        );

        mount_table.swap_base(Arc::new(DirSource::new(v2.clone())));
        std::fs::write(v2.join("tex.ktx2"), b"changed-companion").unwrap();

        let results = runtime.block_on(preload_images(
            Arc::clone(&scheduler),
            vec![(
                "/code/tex.png".to_string(),
                old_generation,
                ImageSource::MountCode {
                    virtual_path: "/code/tex.png".to_string(),
                    relative_path: "tex.png".to_string(),
                },
            )],
            None,
            GpuCapsSnapshot::default(),
            Some(Arc::clone(&mount_table)),
        ));

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, "/code/tex.png");
        assert_eq!(results[0].1, Ok((1, 1)));
        crate::image_cache::global_cache().clear();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn mount_backed_preload_cached_task_revalidates_after_remount_before_consumption() {
        let scheduler = Arc::new(IoScheduler::new(79));
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let dir = std::env::temp_dir().join("migo_dir_mount_cache_revalidate_cached_task");
        let _ = std::fs::remove_dir_all(&dir);
        let v1 = dir.join("v1");
        let v2 = dir.join("v2");
        std::fs::create_dir_all(&v1).unwrap();
        std::fs::create_dir_all(&v2).unwrap();
        std::fs::write(v1.join("tex.png"), tiny_png()).unwrap();
        std::fs::write(v2.join("tex.png"), tiny_png()).unwrap();

        let mount_table = Arc::new(MountTable::new(dir.clone()));
        mount_table.swap_base(Arc::new(DirSource::new(v1.clone())));
        let resolved = mount_table.resolve_code_path("/code/tex.png").unwrap();
        let old_generation = mounted_variant_source_version_token(
            resolved.real_path.as_ref().unwrap(),
            "/code/tex.png",
            Some(mount_table.as_ref()),
        );

        crate::image_cache::global_cache().clear();
        crate::image_cache::global_cache().insert(
            ("/code/tex.png".to_string(), old_generation),
            NormalizedImage::new(9, 9, vec![255; 9 * 9 * 4]),
        );

        let (classified, resume) = install_preload_cache_hook();
        let results = runtime.block_on(async {
            let preload = tokio::spawn(preload_images(
                Arc::clone(&scheduler),
                vec![(
                    "/code/tex.png".to_string(),
                    old_generation,
                    ImageSource::MountCode {
                        virtual_path: "/code/tex.png".to_string(),
                        relative_path: "tex.png".to_string(),
                    },
                )],
                None,
                GpuCapsSnapshot::default(),
                Some(Arc::clone(&mount_table)),
            ));

            classified.notified().await;
            mount_table.swap_base(Arc::new(DirSource::new(v2.clone())));
            std::fs::write(v2.join("tex.ktx2"), b"changed-companion").unwrap();
            resume.notify_waiters();

            preload.await.unwrap()
        });
        clear_preload_cache_hook();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, "/code/tex.png");
        assert_eq!(results[0].1, Ok((1, 1)));
        crate::image_cache::global_cache().clear();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
