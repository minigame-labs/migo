use std::{collections::HashMap, io::SeekFrom, ops::Range, path::PathBuf};

use deno_core::v8::{BackingStore, SharedRef};
use tokio::{
    fs::{self, OpenOptions},
    io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt},
    time::Instant,
};
use tracing::{debug, trace, warn};

use shared::{
    error::{EngineError, ErrorCode, io_error_to_error_code},
    protocol::io_cmd::{
        FileId, FileStat, IOCmd, IOCmdResp, NormalizedImage, OpenFlag, SavedFileInfo, StatEntry,
        StatResult, WriteMode, ZipEntryResult, MAX_READ_LENGTH,
    },
};

#[cfg(feature = "zip-extract")]
use crate::zip_extract;
use crate::{fast_image_decoder, image_cache};

pub struct IoCmdHandler {
    next_id: FileId,
    free_ids: Vec<FileId>,
    files: HashMap<FileId, fs::File>,
    /// Cached total byte size per storage directory, avoiding O(n) re-scan
    /// on every `StorageSet`.  Populated lazily on first write, then
    /// maintained incrementally by Set / Remove / Clear operations.
    storage_totals: HashMap<PathBuf, usize>,
}

impl IoCmdHandler {
    /// Initial capacity for the file handle map.
    /// Most games use a small number of concurrent file handles.
    const INITIAL_FILE_CAPACITY: usize = 8;

    /// Maximum number of concurrent image decode tasks for PreloadImages.
    const MAX_CONCURRENT_IMAGE_DECODES: usize = 8;

    pub fn new() -> Self {
        Self {
            next_id: 3, // 0,1,2 reserved for stdio
            free_ids: Vec::new(),
            files: HashMap::with_capacity(Self::INITIAL_FILE_CAPACITY),
            storage_totals: HashMap::new(),
        }
    }

    #[inline]
    fn alloc_id(&mut self) -> Result<FileId, EngineError> {
        if let Some(id) = self.free_ids.pop() {
            return Ok(id);
        }
        let id = self.next_id;
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or_else(|| EngineError::new(ErrorCode::ExceedMaxConcurrentFdLimit))?;
        Ok(id)
    }

    #[inline]
    fn send_resp<T>(resp: IOCmdResp<T>, result: Result<T, EngineError>) {
        resp.send(result)
    }

    #[inline]
    fn io_err(e: std::io::Error) -> EngineError {
        let detail = e.to_string();
        let code = io_error_to_error_code(&e);
        EngineError::new(code).with_detail(detail)
    }

    #[inline]
    fn code_err(code: ErrorCode) -> EngineError {
        EngineError::new(code)
    }

    pub fn close_all(&mut self) {
        self.files.clear();
        self.free_ids.clear();
        self.storage_totals.clear();
    }

    pub async fn handle_cmd(&mut self, cmd: IOCmd) {
        trace!("handle io cmd: {:?}", cmd);

        match cmd {
            IOCmd::Shutdown => unreachable!("Shutdown is handled by IOThread loop"),

            IOCmd::Access { path, resp } => {
                let r = fs::metadata(&path)
                    .await
                    .map(|m| (m.is_file(), m.is_dir(), m.len()))
                    .map_err(Self::io_err);
                Self::send_resp(resp, r);
            }

            IOCmd::Write {
                path,
                data,
                mode,
                resp,
            } => {
                let r = Self::write_file(&path, &data, mode).await;
                Self::send_resp(resp, r);
            }

            IOCmd::WriteShared {
                path,
                store,
                range,
                mode,
                resp,
            } => {
                let r = Self::write_shared(&path, &store, range, mode).await;
                Self::send_resp(resp, r);
            }

            IOCmd::Open { path, flag, resp } => {
                let r = self.open_file(&path, flag).await;
                Self::send_resp(resp, r);
            }

            IOCmd::Close { rid, resp } => {
                let r = self
                    .files
                    .remove(&rid)
                    .map(|_| {
                        self.free_ids.push(rid);
                    })
                    .ok_or_else(|| Self::code_err(ErrorCode::BadFileDescriptor));
                Self::send_resp(resp, r);
            }

            IOCmd::Copy {
                src_path,
                dest_path,
                resp,
            } => {
                let r = fs::copy(&src_path, &dest_path)
                    .await
                    .map(|_| ())
                    .map_err(Self::io_err);
                Self::send_resp(resp, r);
            }

            IOCmd::Fstat { rid, resp } => {
                let r = match self.files.get(&rid) {
                    Some(file) => match file.metadata().await {
                        Ok(meta) => Ok(Self::build_stat(meta)),
                        Err(e) => Err(Self::io_err(e)),
                    },
                    None => Err(Self::code_err(ErrorCode::BadFileDescriptor)),
                };
                Self::send_resp(resp, r);
            }

            IOCmd::Ftruncate { rid, len, resp } => {
                let r: Result<(), EngineError> = (async {
                    let file = self
                        .files
                        .get_mut(&rid)
                        .ok_or_else(|| Self::code_err(ErrorCode::BadFileDescriptor))?;

                    file.set_len(len).await.map_err(Self::io_err)?;

                    // Best-effort move cursor to end.
                    let _ = file.seek(SeekFrom::End(0)).await;
                    Ok(())
                })
                .await;

                Self::send_resp(resp, r);
            }

            IOCmd::Mkdir {
                dir_path,
                recursive,
                resp,
            } => {
                let r = if recursive {
                    fs::create_dir_all(&dir_path).await
                } else {
                    fs::create_dir(&dir_path).await
                }
                .map_err(Self::io_err);
                Self::send_resp(resp, r);
            }

            // Readdir returns direct children (file and directory names), sorted.
            IOCmd::Readdir { dir_path, resp } => {
                let r: Result<Vec<String>, EngineError> = (async {
                    let mut entries = Vec::new();
                    let mut rd = fs::read_dir(&dir_path).await.map_err(Self::io_err)?;
                    while let Some(entry) = rd.next_entry().await.map_err(Self::io_err)? {
                        if let Some(name) = entry.file_name().to_str() {
                            entries.push(name.to_string());
                        }
                    }
                    entries.sort_unstable();
                    Ok(entries)
                })
                .await;
                Self::send_resp(resp, r);
            }

            IOCmd::Unlink { file_path, resp } => {
                let r = fs::remove_file(&file_path).await.map_err(Self::io_err);
                Self::send_resp(resp, r);
            }

            IOCmd::Rename {
                old_path,
                new_path,
                resp,
            } => {
                let r = fs::rename(&old_path, &new_path).await.map_err(Self::io_err);
                Self::send_resp(resp, r);
            }

            IOCmd::Rmdir {
                dir_path,
                recursive,
                resp,
            } => {
                let r = if recursive {
                    fs::remove_dir_all(&dir_path).await
                } else {
                    fs::remove_dir(&dir_path).await
                }
                .map_err(Self::io_err);
                Self::send_resp(resp, r);
            }

            IOCmd::Stat {
                path,
                recursive,
                resp,
            } => {
                let r = if !recursive {
                    match fs::metadata(&path).await {
                        Ok(meta) => Ok(StatResult::Single(Self::build_stat(meta))),
                        Err(e) => Err(Self::io_err(e)),
                    }
                } else {
                    Self::stat_dir_recursive(PathBuf::from(&path)).await
                };

                Self::send_resp(resp, r);
            }

            IOCmd::WriteFd {
                rid,
                data,
                position,
                resp,
            } => {
                let r: Result<usize, EngineError> = (async {
                    let file = self
                        .files
                        .get_mut(&rid)
                        .ok_or_else(|| Self::code_err(ErrorCode::BadFileDescriptor))?;

                    if let Some(pos) = position {
                        file.seek(SeekFrom::Start(pos))
                            .await
                            .map_err(Self::io_err)?;
                    }

                    file.write_all(&data).await.map_err(Self::io_err)?;
                    Ok(data.len())
                })
                .await;

                Self::send_resp(resp, r);
            }

            IOCmd::WriteFdShared {
                rid,
                store,
                range,
                position,
                resp,
            } => {
                let r: Result<usize, EngineError> = (async {
                    let file = self
                        .files
                        .get_mut(&rid)
                        .ok_or_else(|| Self::code_err(ErrorCode::BadFileDescriptor))?;

                    // Copy bytes first (never hold V8 memory across await).
                    let data = Self::copy_backing_store_range(&store, range)?;

                    if let Some(pos) = position {
                        file.seek(SeekFrom::Start(pos))
                            .await
                            .map_err(Self::io_err)?;
                    }

                    file.write_all(&data).await.map_err(Self::io_err)?;
                    Ok(data.len())
                })
                .await;

                Self::send_resp(resp, r);
            }

            IOCmd::ReadFd {
                rid,
                length,
                position,
                resp,
            } => {
                let r: Result<Vec<u8>, EngineError> = (async {
                    if length > MAX_READ_LENGTH {
                        return Err(EngineError::new(ErrorCode::InvalidArgument)
                            .with_detail(format!("read length {} exceeds limit {}", length, MAX_READ_LENGTH)));
                    }

                    let file = self
                        .files
                        .get_mut(&rid)
                        .ok_or_else(|| Self::code_err(ErrorCode::BadFileDescriptor))?;

                    if let Some(pos) = position {
                        file.seek(SeekFrom::Start(pos))
                            .await
                            .map_err(Self::io_err)?;
                    }

                    let mut buf = vec![0u8; length as usize];
                    let mut total = 0;
                    while total < buf.len() {
                        match file.read(&mut buf[total..]).await {
                            Ok(0) => break,
                            Ok(n) => total += n,
                            Err(e) => return Err(Self::io_err(e)),
                        }
                    }
                    buf.truncate(total);
                    Ok(buf)
                })
                .await;

                Self::send_resp(resp, r);
            }

            IOCmd::ReadFile {
                path,
                position,
                length,
                resp,
            } => {
                let r: Result<Vec<u8>, EngineError> = (async {
                    if let Some(len) = length {
                        if len > MAX_READ_LENGTH {
                            return Err(EngineError::new(ErrorCode::InvalidArgument)
                                .with_detail(format!("read length {} exceeds limit {}", len, MAX_READ_LENGTH)));
                        }
                    }

                    // If no position/length, read entire file (fast path).
                    // Check file size first to enforce the 100 MiB limit.
                    if position.is_none() && length.is_none() {
                        let meta = fs::metadata(&path).await.map_err(Self::io_err)?;
                        if meta.len() > MAX_READ_LENGTH {
                            return Err(EngineError::new(ErrorCode::InvalidArgument)
                                .with_detail(format!(
                                    "file size {} exceeds limit {}",
                                    meta.len(),
                                    MAX_READ_LENGTH
                                )));
                        }
                        return fs::read(&path).await.map_err(Self::io_err);
                    }

                    // Open file and seek/read specific range
                    let mut file = fs::File::open(&path).await.map_err(Self::io_err)?;

                    // Seek to position if specified
                    if let Some(pos) = position {
                        file.seek(SeekFrom::Start(pos))
                            .await
                            .map_err(Self::io_err)?;
                    }

                    // Read specified length or rest of file
                    let data = if let Some(len) = length {
                        let mut buf = vec![0u8; len as usize];
                        let mut total = 0;
                        while total < buf.len() {
                            match file.read(&mut buf[total..]).await {
                                Ok(0) => break, // EOF
                                Ok(n) => total += n,
                                Err(e) => return Err(Self::io_err(e)),
                            }
                        }
                        buf.truncate(total);
                        buf
                    } else {
                        let mut buf = Vec::new();
                        file.read_to_end(&mut buf).await.map_err(Self::io_err)?;
                        buf
                    };

                    Ok(data)
                })
                .await;

                Self::send_resp(resp, r);
            }

            // --- Heavy ops: spawned concurrently so the IO loop is not blocked ---

            #[cfg(feature = "compress-brotli")]
            IOCmd::ReadCompressedFile { path, resp } => {
                tokio::spawn(async move {
                    let r: Result<Vec<u8>, EngineError> = async {
                        let compressed = fs::read(&path).await.map_err(|e| {
                            EngineError::new(io_error_to_error_code(&e))
                                .with_detail(e.to_string())
                        })?;
                        tokio::task::spawn_blocking(move || {
                            let mut decompressed = Vec::new();
                            let mut reader =
                                brotli::Decompressor::new(compressed.as_slice(), 4096);
                            std::io::Read::read_to_end(&mut reader, &mut decompressed)
                                .map_err(|e| {
                                    EngineError::new(ErrorCode::IoError)
                                        .with_detail(format!("brotli decompress failed: {}", e))
                                })?;
                            Ok(decompressed)
                        })
                        .await
                        .map_err(|e| {
                            EngineError::new(ErrorCode::IoError)
                                .with_detail(format!("task join error: {}", e))
                        })?
                    }
                    .await;
                    resp.send(r);
                });
            }

            #[cfg(not(feature = "compress-brotli"))]
            IOCmd::ReadCompressedFile { resp, .. } => {
                resp.send(Err(EngineError::new(ErrorCode::IoError)
                    .with_detail("brotli decompression not available (compress-brotli feature disabled)")));
            }

            #[cfg(feature = "zip-extract")]
            IOCmd::ReadZipEntry {
                zip_path,
                entries_json,
                resp,
            } => {
                tokio::spawn(async move {
                    let r = match tokio::task::spawn_blocking(move || {
                        IoCmdHandler::read_zip_entries(&zip_path, &entries_json)
                    })
                    .await
                    {
                        Ok(inner) => inner,
                        Err(e) => Err(EngineError::new(ErrorCode::IoError)
                            .with_detail(format!("task join error: {}", e))),
                    };
                    resp.send(r);
                });
            }

            #[cfg(not(feature = "zip-extract"))]
            IOCmd::ReadZipEntry { resp, .. } => {
                Self::send_resp(
                    resp,
                    Err(EngineError::new(ErrorCode::IoError)
                        .with_msg("readZipEntry not available (zip feature disabled)")),
                );
            }

            IOCmd::GetFileInfo {
                path,
                algorithm,
                resp,
            } => {
                tokio::spawn(async move {
                    let r = tokio::task::spawn_blocking(move || {
                        IoCmdHandler::get_file_info(&path, &algorithm)
                    })
                    .await
                    .map_err(|e| {
                        EngineError::new(ErrorCode::IoError)
                            .with_detail(format!("task join error: {e}"))
                    })
                    .and_then(|inner| inner);
                    resp.send(r);
                });
            }

            IOCmd::ReadImageRgba8 { path, resp } => {
                // Check LRU cache first (fast path avoids spawning)
                if let Some(cached) = image_cache::global_cache().get(&path) {
                    debug!("ReadImageRgba8 cache hit: {}", path);
                    Self::send_resp(resp, Ok(cached.image));
                    return;
                }

                tokio::spawn(async move {
                    let start = Instant::now();
                    let path_clone = path.clone();
                    let task = tokio::task::spawn_blocking(
                        move || -> Result<NormalizedImage, EngineError> {
                            let data = std::fs::read(&path_clone).map_err(|e| {
                                EngineError::new(ErrorCode::ImageReadError)
                                    .with_detail(format!("failed to read file: {}", e))
                            })?;
                            fast_image_decoder::decode_image_fast(&data, Some(&path_clone))
                        },
                    );

                    let r = match task.await {
                        Ok(Ok(img)) => {
                            image_cache::global_cache().insert(path.clone(), img.clone());
                            debug!(
                                "ReadImageRgba8 decoded: {} ({}x{}) in {:.2?}",
                                path, img.width, img.height, start.elapsed()
                            );
                            Ok(img)
                        }
                        Ok(Err(e)) => {
                            warn!("ReadImageRgba8 decode error: {:?}", e);
                            Err(e)
                        }
                        Err(join_err) => {
                            warn!("ReadImageRgba8 spawn_blocking join error: {join_err}");
                            Err(EngineError::new(ErrorCode::ImageReadError)
                                .with_detail(format!("spawn_blocking join error: {join_err}")))
                        }
                    };
                    resp.send(r);
                });
            }

            IOCmd::PreloadImages { paths, resp } => {
                tokio::spawn(async move {
                    let start = Instant::now();
                    let total = paths.len();
                    debug!("PreloadImages: {} images", total);

                    // Separate cache hits from misses to avoid wasting
                    // semaphore permits and blocking threads on lookups.
                    let mut results = Vec::with_capacity(total);
                    let mut decode_paths = Vec::new();
                    {
                        let mut cache = image_cache::global_cache();
                        for path in paths {
                            if let Some(cached) = cache.get(&path) {
                                results.push((
                                    path,
                                    Ok((cached.image.width, cached.image.height)),
                                ));
                            } else {
                                decode_paths.push(path);
                            }
                        }
                    }

                    let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(
                        IoCmdHandler::MAX_CONCURRENT_IMAGE_DECODES,
                    ));

                    let handles: Vec<_> = decode_paths
                        .into_iter()
                        .map(|path| {
                            let sem = semaphore.clone();
                            tokio::spawn(async move {
                                // safe: semaphore is never closed
                                let _permit = sem.acquire().await.unwrap();
                                tokio::task::spawn_blocking(move || {
                                    match std::fs::read(&path) {
                                        Ok(data) => {
                                            match fast_image_decoder::decode_image_fast(
                                                &data,
                                                Some(&path),
                                            ) {
                                                Ok(img) => {
                                                    let dims = (img.width, img.height);
                                                    image_cache::global_cache()
                                                        .insert(path.clone(), img);
                                                    (path, Ok(dims))
                                                }
                                                Err(e) => (path, Err(format!("{:?}", e))),
                                            }
                                        }
                                        Err(e) => (path, Err(format!("read error: {}", e))),
                                    }
                                })
                                .await
                            })
                        })
                        .collect();

                    for handle in handles {
                        match handle.await {
                            Ok(Ok(result)) => results.push(result),
                            Ok(Err(e)) | Err(e) => {
                                warn!("PreloadImages task error: {}", e);
                            }
                        }
                    }

                    debug!(
                        "PreloadImages completed: {}/{} images in {:.2?}",
                        results.len(),
                        total,
                        start.elapsed()
                    );
                    resp.send(Ok(results));
                });
            }

            IOCmd::ClearImageCache { resp } => {
                image_cache::global_cache().clear();
                debug!("Image cache cleared");
                Self::send_resp(resp, Ok(()));
            }

            IOCmd::GetImageCacheStats { resp } => {
                use shared::protocol::io_cmd::ImageCacheStats;
                let stats = image_cache::global_cache().stats();
                let result = ImageCacheStats {
                    entries: stats.entries,
                    size_bytes: stats.size_bytes,
                    max_bytes: stats.max_bytes,
                    hits: stats.hits,
                    misses: stats.misses,
                    hit_rate: stats.hit_rate(),
                };
                Self::send_resp(resp, Ok(result));
            }

            #[cfg(feature = "zip-extract")]
            IOCmd::Unzip {
                zip_path,
                dest_dir,
                resp,
            } => {
                tokio::spawn(async move {
                    let start = Instant::now();
                    debug!("Unzip: {} -> {}", zip_path, dest_dir);

                    let zip_path_clone = zip_path.clone();
                    let dest_dir_clone = dest_dir.clone();

                    let task = tokio::task::spawn_blocking(move || {
                        let zip_path = PathBuf::from(&zip_path_clone);
                        let dest_dir = PathBuf::from(&dest_dir_clone);

                        let file_count =
                            std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
                        let file_count_clone = file_count.clone();

                        let progress_cb =
                            Box::new(move |_prog: f32, current: usize, _total: usize| {
                                file_count_clone
                                    .store(current, std::sync::atomic::Ordering::Relaxed);
                            });

                        match zip_extract::extract_zip(&zip_path, &dest_dir, Some(progress_cb)) {
                            Ok(()) => {
                                Ok(file_count.load(std::sync::atomic::Ordering::Relaxed))
                            }
                            Err(e) => Err(
                                EngineError::new(ErrorCode::IoError).with_detail(e.to_string())
                            ),
                        }
                    });

                    let r = match task.await {
                        Ok(result) => {
                            debug!("Unzip completed in {:.2?}", start.elapsed());
                            result
                        }
                        Err(join_err) => {
                            warn!("Unzip spawn_blocking join error: {join_err}");
                            Err(EngineError::new(ErrorCode::IoError)
                                .with_detail(format!("spawn_blocking join error: {join_err}")))
                        }
                    };
                    resp.send(r);
                });
            }

            #[cfg(not(feature = "zip-extract"))]
            IOCmd::Unzip { resp, .. } => {
                Self::send_resp(
                    resp,
                    Err(EngineError::new(ErrorCode::Unsupported)
                        .with_detail("zip-extract feature is not enabled")),
                );
            }

            // ── Storage (KV) ─────────────────────────────────────────
            IOCmd::StorageGet { path, resp } => {
                let r = match fs::read_to_string(&path).await {
                    Ok(content) => Ok(content),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
                    Err(e) => Err(Self::io_err(e)),
                };
                Self::send_resp(resp, r);
            }

            IOCmd::StorageSet {
                dir,
                path,
                data,
                max_total,
                resp,
            } => {
                let r: Result<(), EngineError> = (async {
                    fs::create_dir_all(&dir).await.map_err(Self::io_err)?;

                    // Existing size of the target key (0 if new).
                    let existing_size = fs::metadata(&path)
                        .await
                        .map(|m| m.len() as usize)
                        .unwrap_or(0);

                    // Use cached total if available, otherwise do a full scan
                    // and cache the result for subsequent writes.
                    let dir_key = PathBuf::from(&dir);
                    let total = match self.storage_totals.get(&dir_key) {
                        Some(&cached) => cached,
                        None => {
                            let mut sum: usize = 0;
                            let mut rd = fs::read_dir(&dir).await.map_err(Self::io_err)?;
                            while let Some(entry) = rd.next_entry().await.map_err(Self::io_err)? {
                                sum += entry
                                    .metadata()
                                    .await
                                    .map(|m| m.len() as usize)
                                    .unwrap_or(0);
                            }
                            self.storage_totals.insert(dir_key.clone(), sum);
                            sum
                        }
                    };

                    if total.saturating_sub(existing_size) + data.len() > max_total {
                        return Err(EngineError::new(ErrorCode::IoError)
                            .with_detail("setStorage:fail storage limit exceeded"));
                    }

                    fs::write(&path, &data).await.map_err(Self::io_err)?;

                    let new_total = total.saturating_sub(existing_size) + data.len();
                    self.storage_totals.insert(dir_key, new_total);
                    Ok(())
                })
                .await;
                Self::send_resp(resp, r);
            }

            IOCmd::StorageRemove { path, resp } => {
                // Only query file size when the cache is populated for this
                // directory — avoids an extra syscall when it would be wasted.
                let parent_key = PathBuf::from(&path);
                let parent_key = parent_key.parent().map(|p| p.to_path_buf());
                let need_size = parent_key
                    .as_ref()
                    .is_some_and(|k| self.storage_totals.contains_key(k));
                let removed_size = if need_size {
                    fs::metadata(&path)
                        .await
                        .map(|m| m.len() as usize)
                        .unwrap_or(0)
                } else {
                    0
                };
                let r = match fs::remove_file(&path).await {
                    Ok(()) => {
                        if let (Some(key), true) = (parent_key, removed_size > 0) {
                            if let Some(total) = self.storage_totals.get_mut(&key) {
                                *total = total.saturating_sub(removed_size);
                            }
                        }
                        Ok(())
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                    Err(e) => Err(Self::io_err(e)),
                };
                Self::send_resp(resp, r);
            }

            IOCmd::StorageClear { dir, resp } => {
                let r: Result<(), EngineError> = (async {
                    match fs::read_dir(&dir).await {
                        Ok(mut rd) => {
                            while let Some(entry) = rd.next_entry().await.map_err(Self::io_err)? {
                                let _ = fs::remove_file(entry.path()).await;
                            }
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                        Err(e) => return Err(Self::io_err(e)),
                    }
                    self.storage_totals.insert(PathBuf::from(&dir), 0);
                    Ok(())
                })
                .await;
                Self::send_resp(resp, r);
            }

            IOCmd::StorageInfo {
                dir,
                limit_size_kb,
                resp,
            } => {
                let r: Result<String, EngineError> = (async {
                    let mut keys: Vec<String> = Vec::new();
                    let mut total_bytes: u64 = 0;

                    match fs::read_dir(&dir).await {
                        Ok(mut rd) => {
                            while let Some(entry) = rd.next_entry().await.map_err(Self::io_err)? {
                                if let Some(name) = entry.file_name().to_str() {
                                    if let Some(key) = Self::hex_to_key(name) {
                                        keys.push(key);
                                    }
                                }
                                total_bytes +=
                                    entry.metadata().await.map(|m| m.len()).unwrap_or(0);
                            }
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                        Err(e) => return Err(Self::io_err(e)),
                    }

                    let keys_json: String = keys
                        .iter()
                        .map(|k| format!("\"{}\"", Self::json_escape(k)))
                        .collect::<Vec<_>>()
                        .join(",");

                    let current_size_kb = (total_bytes + 1023) / 1024;

                    Ok(format!(
                        "{{\"keys\":[{keys_json}],\"currentSize\":{current_size_kb},\"limitSize\":{limit_size_kb}}}"
                    ))
                })
                .await;
                Self::send_resp(resp, r);
            }

            IOCmd::ListSavedFiles {
                dir,
                prefix,
                virtual_dir,
                resp,
            } => {
                let r: Result<Vec<SavedFileInfo>, EngineError> = (async {
                    let mut file_list = Vec::new();
                    let mut rd = match fs::read_dir(&dir).await {
                        Ok(rd) => rd,
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                            return Ok(file_list);
                        }
                        Err(e) => return Err(Self::io_err(e)),
                    };
                    while let Some(entry) = rd.next_entry().await.map_err(Self::io_err)? {
                        let name = match entry.file_name().to_str() {
                            Some(n) => n.to_string(),
                            None => continue,
                        };
                        if !name.starts_with(&prefix) {
                            continue;
                        }
                        if let Ok(meta) = entry.metadata().await {
                            if !meta.is_file() {
                                continue;
                            }
                            let mtime = meta
                                .modified()
                                .ok()
                                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                                .map(|d| d.as_secs())
                                .unwrap_or(0);
                            file_list.push(SavedFileInfo {
                                file_path: format!("{}/{}", virtual_dir, name),
                                size: meta.len(),
                                create_time: mtime,
                            });
                        }
                    }
                    Ok(file_list)
                })
                .await;
                Self::send_resp(resp, r);
            }
        }
    }

    async fn open_file(&mut self, path: &str, flag: OpenFlag) -> Result<FileId, EngineError> {
        let mut opts = OpenOptions::new();

        match flag {
            OpenFlag::Read => {
                opts.read(true);
            }
            OpenFlag::ReadWrite => {
                opts.read(true).write(true);
            }
            OpenFlag::WriteTruncateCreate => {
                opts.write(true).create(true).truncate(true);
            }
            OpenFlag::ReadWriteTruncateCreate => {
                opts.read(true).write(true).create(true).truncate(true);
            }
            OpenFlag::AppendCreate => {
                opts.append(true).create(true);
            }
            OpenFlag::ReadAppendCreate => {
                opts.read(true).append(true).create(true);
            }
            OpenFlag::AppendExclusive => {
                opts.append(true).create_new(true);
            }
            OpenFlag::ReadAppendExclusive => {
                opts.read(true).append(true).create_new(true);
            }
            OpenFlag::AppendSyncCreate => {
                // 'as' – sync hint; treated as append+create (sync is implicit in our model)
                opts.append(true).create(true);
            }
            OpenFlag::ReadAppendSyncCreate => {
                // 'as+' – sync hint; treated as read+append+create
                opts.read(true).append(true).create(true);
            }
            OpenFlag::WriteExclusive => {
                opts.write(true).create_new(true);
            }
            OpenFlag::ReadWriteExclusive => {
                opts.read(true).write(true).create_new(true);
            }
        }

        let file = opts.open(path).await.map_err(Self::io_err)?;
        let id = self.alloc_id()?;
        self.files.insert(id, file);
        Ok(id)
    }

    async fn write_file(path: &str, data: &[u8], mode: WriteMode) -> Result<bool, EngineError> {
        match mode {
            WriteMode::Overwrite => fs::write(path, data)
                .await
                .map(|_| true)
                .map_err(Self::io_err),

            WriteMode::Append => {
                let mut file = OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)
                    .await
                    .map_err(Self::io_err)?;

                file.write_all(data)
                    .await
                    .map(|_| true)
                    .map_err(Self::io_err)
            }
        }
    }

    async fn write_shared(
        path: &str,
        store: &SharedRef<BackingStore>,
        range: Range<usize>,
        mode: WriteMode,
    ) -> Result<bool, EngineError> {
        let data = Self::copy_backing_store_range(store, range)?;
        Self::write_file(path, &data, mode).await
    }

    /// Copy a byte range out of a V8 BackingStore safely.
    /// Uses `byte_length()` (fix for NonNull<c_void> no `.len()`).
    fn copy_backing_store_range(
        store: &SharedRef<BackingStore>,
        range: Range<usize>,
    ) -> Result<Vec<u8>, EngineError> {
        let nn = store
            .data()
            .ok_or_else(|| EngineError::new(ErrorCode::ArrayBufferDoesNotExist))?;
        let total = store.byte_length();

        if range.start > range.end || range.end > total {
            return Err(EngineError::new(ErrorCode::InvalidArgument)
                .with_detail(format!("invalid range: {:?}, total={}", range, total)));
        }

        let len = range.end - range.start;
        let ptr = nn.as_ptr() as *const u8;

        // SAFETY: bounds validated by byte_length.
        let slice = unsafe { std::slice::from_raw_parts(ptr.add(range.start), len) };
        Ok(slice.to_vec())
    }

    #[inline]
    fn build_stat(meta: std::fs::Metadata) -> FileStat {
        let mode = Self::get_mode(&meta);
        let size = meta.len();

        let atime = meta
            .accessed()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);

        FileStat {
            mode,
            size,
            atime,
            mtime,
            is_file: meta.is_file(),
            is_directory: meta.is_dir(),
        }
    }

    #[inline]
    fn get_mode(meta: &std::fs::Metadata) -> u32 {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            meta.mode()
        }
        #[cfg(not(unix))]
        {
            if meta.permissions().readonly() {
                0o444
            } else {
                0o666
            }
        }
    }

    // ── Storage helpers ────────────────────────────────────────

    /// Decode a hex filename back to the original storage key.
    fn hex_to_key(hex: &str) -> Option<String> {
        let hex = hex.as_bytes();
        if hex.len() % 2 != 0 {
            return None;
        }
        let mut bytes = Vec::with_capacity(hex.len() / 2);
        for pair in hex.chunks_exact(2) {
            let hi = Self::hex_digit(pair[0])?;
            let lo = Self::hex_digit(pair[1])?;
            bytes.push((hi << 4) | lo);
        }
        String::from_utf8(bytes).ok()
    }

    #[inline]
    fn hex_digit(c: u8) -> Option<u8> {
        match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            b'A'..=b'F' => Some(c - b'A' + 10),
            _ => None,
        }
    }

    /// Escape a string for safe embedding in a JSON string literal.
    fn json_escape(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        for c in s.chars() {
            match c {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                c if (c as u32) < 0x20 => {
                    out.push_str(&format!("\\u{:04x}", c as u32));
                }
                c => out.push(c),
            }
        }
        out
    }

    /// Stat recursively for files under a directory.
    async fn stat_dir_recursive(
        root: PathBuf,
    ) -> Result<StatResult, EngineError> {
        use std::collections::BTreeMap;

        let root_meta = fs::metadata(&root).await.map_err(Self::io_err)?;
        if root_meta.is_file() {
            return Ok(StatResult::Single(Self::build_stat(root_meta)));
        }

        // Use BTreeMap for automatic sorting by key, avoiding O(n log n) sort at the end
        let mut out: BTreeMap<String, FileStat> = BTreeMap::new();
        let mut stack: Vec<PathBuf> = vec![root.clone()];

        while let Some(dir) = stack.pop() {
            let mut rd = fs::read_dir(&dir).await.map_err(Self::io_err)?;

            while let Some(entry) = rd.next_entry().await.map_err(Self::io_err)? {
                let path = entry.path();
                let ft = entry.file_type().await.map_err(Self::io_err)?;

                if ft.is_dir() {
                    stack.push(path);
                } else if ft.is_file() {
                    let meta = entry.metadata().await.map_err(Self::io_err)?;
                    let stat = Self::build_stat(meta);
                    let rel = path
                        .strip_prefix(&root)
                        .unwrap_or(&path)
                        .to_string_lossy()
                        .into_owned();

                    out.insert(rel, stat);
                }
            }
        }

        // BTreeMap iteration is already sorted by key
        Ok(StatResult::Recursive(
            out.into_iter()
                .map(|(path, stat)| StatEntry { path, stat })
                .collect(),
        ))
    }

    /// Compute file size + digest in a single pass (streaming, 8 KB buffer).
    fn get_file_info(path: &str, algorithm: &str) -> Result<(u64, String), EngineError> {
        use digest::Digest;
        use std::io::Read;

        let meta = std::fs::metadata(path).map_err(Self::io_err)?;
        let size = meta.len();

        let mut file = std::io::BufReader::new(std::fs::File::open(path).map_err(Self::io_err)?);
        let mut buf = [0u8; 8192];

        macro_rules! hash_loop {
            ($hasher:expr) => {{
                let mut h = $hasher;
                loop {
                    let n = file.read(&mut buf).map_err(Self::io_err)?;
                    if n == 0 {
                        break;
                    }
                    h.update(&buf[..n]);
                }
                hex::encode(h.finalize())
            }};
        }

        let digest_hex = match algorithm {
            "md5" => hash_loop!(md5::Md5::new()),
            "sha1" => hash_loop!(sha1::Sha1::new()),
            "sha256" => hash_loop!(sha2::Sha256::new()),
            _ => {
                return Err(EngineError::new(ErrorCode::InvalidArgument)
                    .with_detail(format!("unsupported digestAlgorithm: {algorithm}")))
            }
        };

        Ok((size, digest_hex))
    }

    #[cfg(feature = "zip-extract")]
    fn read_zip_entries(
        zip_path: &str,
        entries_json: &str,
    ) -> Result<Vec<ZipEntryResult>, EngineError> {
        use deno_core::serde_json;
        use std::io::{BufReader, Read as _};

        let file = std::fs::File::open(zip_path).map_err(Self::io_err)?;
        let reader = BufReader::new(file);
        let mut archive = zip::ZipArchive::new(reader).map_err(|e| {
            EngineError::new(ErrorCode::IoError).with_detail(format!("invalid zip: {}", e))
        })?;

        let req: serde_json::Value = serde_json::from_str(entries_json).map_err(|e| {
            EngineError::new(ErrorCode::InvalidArgument)
                .with_detail(format!("invalid entries_json: {}", e))
        })?;

        let global_encoding = req
            .get("encoding")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let entries_val = req.get("entries");

        let read_all = entries_val
            .and_then(|v| v.as_str())
            .map(|s| s == "all")
            .unwrap_or(false);

        let mut results = Vec::new();

        if read_all {
            for i in 0..archive.len() {
                let mut entry = match archive.by_index(i) {
                    Ok(e) => e,
                    Err(_) => continue,
                };
                if entry.is_dir() {
                    continue;
                }
                let name = entry.name().to_string();
                let mut buf = Vec::with_capacity(entry.size() as usize);
                match entry.read_to_end(&mut buf) {
                    Ok(_) => {
                        let data = Self::encode_zip_data(&buf, global_encoding.as_deref());
                        results.push(ZipEntryResult {
                            path: name,
                            data: Some(data),
                            err_msg: String::new(),
                        });
                    }
                    Err(e) => {
                        results.push(ZipEntryResult {
                            path: name,
                            data: None,
                            err_msg: format!("read failed: {}", e),
                        });
                    }
                }
            }
        } else if let Some(arr) = entries_val.and_then(|v| v.as_array()) {
            for item in arr {
                let path = match item.get("path").and_then(|v| v.as_str()) {
                    Some(p) => p.to_string(),
                    None => continue,
                };
                let encoding = item
                    .get("encoding")
                    .and_then(|v: &serde_json::Value| v.as_str())
                    .or(global_encoding.as_deref());
                let position =
                    item.get("position").and_then(|v: &serde_json::Value| v.as_u64());
                let length = item.get("length").and_then(|v: &serde_json::Value| v.as_u64());

                match archive.by_name(&path) {
                    Ok(mut entry) => {
                        let mut buf = Vec::with_capacity(entry.size() as usize);
                        match entry.read_to_end(&mut buf) {
                            Ok(_) => {
                                let start =
                                    position.map(|p| p as usize).unwrap_or(0).min(buf.len());
                                let end = length
                                    .map(|l| (start + l as usize).min(buf.len()))
                                    .unwrap_or(buf.len());
                                let sliced = &buf[start..end];

                                let data = Self::encode_zip_data(sliced, encoding);
                                results.push(ZipEntryResult {
                                    path,
                                    data: Some(data),
                                    err_msg: String::new(),
                                });
                            }
                            Err(e) => {
                                results.push(ZipEntryResult {
                                    path,
                                    data: None,
                                    err_msg: format!("read failed: {}", e),
                                });
                            }
                        }
                    }
                    Err(e) => {
                        results.push(ZipEntryResult {
                            path,
                            data: None,
                            err_msg: format!("entry not found: {}", e),
                        });
                    }
                }
            }
        }

        Ok(results)
    }

    #[cfg(feature = "zip-extract")]
    fn encode_zip_data(data: &[u8], encoding: Option<&str>) -> String {
        use base64::Engine;
        match encoding {
            // No encoding → binary, return base64 for transport
            None => base64::engine::general_purpose::STANDARD.encode(data),
            Some(enc) => {
                // Delegate to codec for full encoding coverage (utf8, utf16le, ucs2, etc.)
                match shared::codec::decode_bytes(data, enc) {
                    Ok(s) => s,
                    // If codec doesn't support it, fall back to base64
                    Err(_) => base64::engine::general_purpose::STANDARD.encode(data),
                }
            }
        }
    }
}
