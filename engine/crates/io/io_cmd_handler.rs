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
    protocol::io_cmd::{FileId, FileStat, IOCmd, IOCmdResp, NormalizedImage, OpenFlag, WriteMode},
};

use crate::{fast_image_decoder, image_cache};
#[cfg(feature = "zip-extract")]
use crate::zip_extract;

pub struct IoCmdHandler {
    next_id: FileId,
    files: HashMap<FileId, fs::File>,
}

impl IoCmdHandler {
    /// Initial capacity for the file handle map.
    /// Most games use a small number of concurrent file handles.
    const INITIAL_FILE_CAPACITY: usize = 8;

    pub fn new() -> Self {
        Self {
            next_id: 3, // 0,1,2 reserved for stdio
            files: HashMap::with_capacity(Self::INITIAL_FILE_CAPACITY),
        }
    }

    #[inline]
    fn alloc_id(&mut self) -> Result<FileId, EngineError> {
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
                    .map(|_| ())
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

            // Readdir returns ALL files recursively under dir_path (relative paths), sorted.
            IOCmd::Readdir { dir_path, resp } => {
                let root = PathBuf::from(&dir_path);
                let r = Self::read_dir_files_recursive(root)
                    .await
                    .map_err(Self::io_err);
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
                        Ok(meta) => Ok(deno_core::serde_json::json!(Self::build_stat(meta))),
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

            IOCmd::ReadFile {
                path,
                position,
                length,
                resp,
            } => {
                let r: Result<Vec<u8>, EngineError> = (async {
                    // If no position/length, read entire file (fast path)
                    if position.is_none() && length.is_none() {
                        return fs::read(&path).await.map_err(Self::io_err);
                    }

                    // Open file and seek/read specific range
                    let mut file = fs::File::open(&path).await.map_err(Self::io_err)?;

                    // Seek to position if specified
                    if let Some(pos) = position {
                        file.seek(SeekFrom::Start(pos)).await.map_err(Self::io_err)?;
                    }

                    // Read specified length or rest of file
                    let data = if let Some(len) = length {
                        let mut buf = vec![0u8; len as usize];
                        let n = file.read(&mut buf).await.map_err(Self::io_err)?;
                        buf.truncate(n);
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

            IOCmd::ReadImageRgba8 { path, resp } => {
                let start = Instant::now();

                // Check LRU cache first (Arc clone: O(1) ref-count increment)
                if let Some(cached) = image_cache::global_cache().get(&path) {
                    debug!(
                        "ReadImageRgba8 cache hit: {} ({}x{}) in {:.2?}",
                        path,
                        cached.image.width,
                        cached.image.height,
                        start.elapsed()
                    );
                    Self::send_resp(resp, Ok(cached.image));
                    return;
                }

                // Cache miss - read and decode
                let path_clone = path.clone();
                let task =
                    tokio::task::spawn_blocking(move || -> Result<NormalizedImage, EngineError> {
                        // Read file into memory
                        let data = std::fs::read(&path_clone).map_err(|e| {
                            EngineError::new(ErrorCode::ImageReadError)
                                .with_detail(format!("failed to read file: {}", e))
                        })?;

                        // Use fast decoder with path hint for format detection
                        fast_image_decoder::decode_image_fast(&data, Some(&path_clone))
                    });

                let r = match task.await {
                    Ok(Ok(img)) => {
                        // Cache the decoded image
                        image_cache::global_cache().insert(path.clone(), img.clone());
                        debug!(
                            "ReadImageRgba8 decoded: {} ({}x{}) in {:.2?}",
                            path,
                            img.width,
                            img.height,
                            start.elapsed()
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

                trace!("ReadImageRgba8 total={:.2?}", start.elapsed());
                Self::send_resp(resp, r);
            }

            IOCmd::PreloadImages { paths, resp } => {
                let start = Instant::now();
                debug!("PreloadImages: {} images", paths.len());

                // Process all images in parallel using spawn_blocking for each
                let handles: Vec<_> = paths
                    .into_iter()
                    .map(|path| {
                        let path_clone = path.clone();
                        let handle = tokio::task::spawn_blocking(move || {
                            // Check cache first
                            if let Some(cached) = image_cache::global_cache().get(&path_clone) {
                                return (
                                    path_clone,
                                    Ok((cached.image.width, cached.image.height)),
                                );
                            }

                            // Read and decode
                            match std::fs::read(&path_clone) {
                                Ok(data) => {
                                    match fast_image_decoder::decode_image_fast(&data, Some(&path_clone)) {
                                        Ok(img) => {
                                            let dims = (img.width, img.height);
                                            // Cache the result
                                            image_cache::global_cache().insert(path_clone.clone(), img);
                                            (path_clone, Ok(dims))
                                        }
                                        Err(e) => (path_clone, Err(format!("{:?}", e))),
                                    }
                                }
                                Err(e) => (path_clone, Err(format!("read error: {}", e))),
                            }
                        });
                        handle
                    })
                    .collect();

                // Wait for all to complete
                let mut results = Vec::with_capacity(handles.len());
                for handle in handles {
                    match handle.await {
                        Ok(result) => results.push(result),
                        Err(e) => {
                            warn!("PreloadImages task join error: {}", e);
                        }
                    }
                }

                debug!(
                    "PreloadImages completed: {} images in {:.2?}",
                    results.len(),
                    start.elapsed()
                );
                Self::send_resp(resp, Ok(results));
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
                let start = Instant::now();
                debug!("Unzip: {} -> {}", zip_path, dest_dir);

                let zip_path_clone = zip_path.clone();
                let dest_dir_clone = dest_dir.clone();

                let task = tokio::task::spawn_blocking(move || {
                    let zip_path = PathBuf::from(&zip_path_clone);
                    let dest_dir = PathBuf::from(&dest_dir_clone);

                    // Use Arc<AtomicUsize> for shared ownership with 'static lifetime
                    let file_count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
                    let file_count_clone = file_count.clone();

                    let progress_cb = Box::new(move |_prog: f32, current: usize, _total: usize| {
                        file_count_clone.store(current, std::sync::atomic::Ordering::Relaxed);
                    });

                    match zip_extract::extract_zip(&zip_path, &dest_dir, Some(progress_cb)) {
                        Ok(()) => Ok(file_count.load(std::sync::atomic::Ordering::Relaxed)),
                        Err(e) => Err(EngineError::new(ErrorCode::IoError).with_detail(e.to_string())),
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

                Self::send_resp(resp, r);
            }

            #[cfg(not(feature = "zip-extract"))]
            IOCmd::Unzip { resp, .. } => {
                Self::send_resp(
                    resp,
                    Err(EngineError::new(ErrorCode::Unsupported)
                        .with_detail("zip-extract feature is not enabled")),
                );
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

    /// Async + iterative traversal (no recursion => no boxing).
    /// Returns relative file paths under root, sorted.
    async fn read_dir_files_recursive(root: PathBuf) -> Result<Vec<String>, std::io::Error> {
        use std::collections::BTreeSet;

        // Use BTreeSet for automatic sorting, avoiding O(n log n) sort at the end
        let mut result: BTreeSet<String> = BTreeSet::new();
        let mut stack: Vec<PathBuf> = vec![root.clone()];

        while let Some(dir) = stack.pop() {
            let mut rd = fs::read_dir(&dir).await?;

            while let Some(entry) = rd.next_entry().await? {
                let path = entry.path();
                let ft = entry.file_type().await?;

                if ft.is_dir() {
                    stack.push(path);
                } else if ft.is_file() {
                    // Use into_owned() to avoid double allocation from to_string_lossy().to_string()
                    let rel = path
                        .strip_prefix(&root)
                        .unwrap_or(&path)
                        .to_string_lossy()
                        .into_owned();
                    result.insert(rel);
                }
            }
        }

        Ok(result.into_iter().collect())
    }

    /// Stat recursively for files under a directory; returns JSON array:
    /// [{ "path": "...", "stat": {..} }, ...]
    async fn stat_dir_recursive(
        root: PathBuf,
    ) -> Result<deno_core::serde_json::Value, EngineError> {
        use deno_core::serde_json::{Value, json};
        use std::collections::BTreeMap;

        let root_meta = fs::metadata(&root).await.map_err(Self::io_err)?;
        if root_meta.is_file() {
            return Ok(json!(Self::build_stat(root_meta)));
        }

        // Use BTreeMap for automatic sorting by key, avoiding O(n log n) sort at the end
        let mut out: BTreeMap<String, Value> = BTreeMap::new();
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
                    // Use into_owned() to avoid double allocation
                    let rel = path
                        .strip_prefix(&root)
                        .unwrap_or(&path)
                        .to_string_lossy()
                        .into_owned();

                    out.insert(rel, json!(stat));
                }
            }
        }

        // BTreeMap iteration is already sorted by key
        Ok(Value::Array(
            out.into_iter()
                .map(|(path, stat)| json!({ "path": path, "stat": stat }))
                .collect(),
        ))
    }
}
