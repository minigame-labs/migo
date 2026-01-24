use std::{collections::HashMap, io::SeekFrom, ops::Range, path::PathBuf};

use deno_core::v8::{BackingStore, SharedRef};
use tokio::{
    fs::{self, OpenOptions},
    io::{AsyncSeekExt, AsyncWriteExt},
    time::Instant,
};
use tracing::{trace, warn};

use shared::{
    error::{EngineError, ErrorCode, io_error_to_error_code},
    protocol::io_cmd::{FileId, FileStat, IOCmd, IOCmdResp, NormalizedImage, OpenFlag, WriteMode},
};

pub struct IoCmdHandler {
    next_id: FileId,
    files: HashMap<FileId, fs::File>,
}

impl IoCmdHandler {
    pub fn new() -> Self {
        Self {
            next_id: 3, // 0,1,2 reserved for stdio
            files: HashMap::new(),
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

            IOCmd::ReadFile { path, resp } => {
                let r = fs::read(&path).await.map_err(Self::io_err);
                Self::send_resp(resp, r);
            }

            IOCmd::ReadImageRgba8 { path, resp } => {
                let start = Instant::now();

                let task =
                    tokio::task::spawn_blocking(move || -> Result<NormalizedImage, EngineError> {
                        let img = image::ImageReader::open(&path)
                            .map_err(|e| {
                                EngineError::new(ErrorCode::ImageReadError)
                                    .with_detail(e.to_string())
                            })?
                            .with_guessed_format()
                            .map_err(|e| {
                                EngineError::new(ErrorCode::ImageReadError)
                                    .with_detail(e.to_string())
                            })?
                            .decode()
                            .map_err(|e| {
                                EngineError::new(ErrorCode::ImageReadError)
                                    .with_detail(e.to_string())
                            })?;

                        let rgba = img.into_rgba8();
                        let (w, h) = rgba.dimensions();
                        let raw = rgba.into_raw();

                        debug_assert_eq!(
                            raw.len(),
                            (w as usize) * (h as usize) * 4,
                            "rgba buffer size mismatch"
                        );

                        let out = NormalizedImage {
                            width: w,
                            height: h,
                            rgba: raw,
                        };

                        Ok(out)
                    });

                let r = match task.await {
                    Ok(v) => v,
                    Err(join_err) => {
                        warn!("ReadImageRgba8 spawn_blocking join error: {join_err}");
                        Err(EngineError::new(ErrorCode::ImageReadError)
                            .with_detail(format!("spawn_blocking join error: {join_err}")))
                    }
                };

                trace!("ReadImageRgba8 total={:.2?}", start.elapsed());
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

        let root_meta = fs::metadata(&root).await.map_err(Self::io_err)?;
        if root_meta.is_file() {
            return Ok(json!(Self::build_stat(root_meta)));
        }

        let mut out: Vec<(String, Value)> = Vec::new();
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
                        .to_string();

                    out.push((rel, json!(stat)));
                }
            }
        }

        out.sort_by(|a, b| a.0.cmp(&b.0));

        Ok(Value::Array(
            out.into_iter()
                .map(|(path, stat)| json!({ "path": path, "stat": stat }))
                .collect(),
        ))
    }
}
