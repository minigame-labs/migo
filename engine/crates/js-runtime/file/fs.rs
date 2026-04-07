use std::{cell::RefCell, io::Write as _, path::PathBuf, rc::Rc, sync::{Arc, atomic::{AtomicU64, Ordering}}};

use deno_core::{JsBuffer, OpState, ToJsBuffer, op2, serde_json, v8};
use shared::{
    codec,
    error::{EngineError, ErrorCode},
    op_state::HostOpState,
    protocol::{
        self,
        io_cmd::{
            FileId, FileStat, IOCmd, IOCmdResp, OpenFlag, SavedFileInfo, StatResult, WriteMode,
        },
    },
    vfs::{FileOp, VfsError, VirtualFS},
};
use tokio::sync::mpsc::UnboundedSender;

const MAX_MATERIALIZE_LENGTH: u64 = 512 * 1024 * 1024;

#[derive(Debug, thiserror::Error, deno_error::JsError)]
pub enum IOError {
    #[class("IOError")]
    #[error("{0}")]
    Message(String),
}

impl From<&str> for IOError {
    #[inline]
    fn from(value: &str) -> Self {
        IOError::Message(value.to_string())
    }
}

impl From<String> for IOError {
    #[inline]
    fn from(value: String) -> Self {
        IOError::Message(value)
    }
}

impl From<ErrorCode> for IOError {
    #[inline]
    fn from(e: ErrorCode) -> Self {
        IOError::Message(e.default_message().to_string())
    }
}

impl From<EngineError> for IOError {
    #[inline]
    fn from(e: EngineError) -> Self {
        match &e.detail {
            Some(d) => IOError::Message(format!("[{:?}] {} ({})", e.code, e.msg, d)),
            None => IOError::Message(format!("[{:?}] {}", e.code, e.msg)),
        }
    }
}

#[inline]
fn ioerr(msg: impl Into<String>) -> IOError {
    IOError::Message(msg.into())
}

/// Get IO channel from async op state.
#[inline]
fn get_io_tx_async(state: Rc<RefCell<OpState>>) -> UnboundedSender<IOCmd> {
    let st = state.borrow();
    st.borrow::<HostOpState>().io_tx.clone()
}

/// Get IO channel from sync op state.
#[inline]
fn get_io_tx_sync(state: &OpState) -> &UnboundedSender<IOCmd> {
    &state.borrow::<HostOpState>().io_tx
}

/// Get VFS + MountTable from async op state.
#[inline]
fn get_vfs_async(state: &Rc<RefCell<OpState>>) -> (Option<Arc<VirtualFS>>, Option<Arc<shared::vfs::MountTable>>) {
    let st = state.borrow();
    let host = st.borrow::<HostOpState>();
    (host.vfs.clone(), host.mount_table.clone())
}

/// Get VFS + MountTable from sync op state.
#[inline]
fn get_vfs_sync(state: &OpState) -> (Option<Arc<VirtualFS>>, Option<Arc<shared::vfs::MountTable>>) {
    let host = state.borrow::<HostOpState>();
    (host.vfs.clone(), host.mount_table.clone())
}

/// Result of path resolution: either a real filesystem path for IO-thread
/// operations, or a virtual path backed by a package (data read inline).
enum ResolvedPath {
    /// Real filesystem path — send to IO thread.
    Filesystem(String),
    /// Pack-backed — data must be read via MountTable::read(), not IO thread.
    Pack { virtual_path: String },
}

/// Resolve a path using VFS + MountTable.
///
/// Virtual paths must start with /user, /cache, /code, or /tmp.
/// Relative paths (not starting with '/') are treated as relative to the
/// game code directory (/code/), matching the target platform's readFile
/// semantics where relative paths resolve against the game package root.
///
/// `/code` paths are resolved through the mount table when available,
/// falling back to VFS if not.  Other virtual paths go through VFS.
#[inline]
fn resolve_path_vfs(
    vfs: Option<&VirtualFS>,
    mount_table: Option<&shared::vfs::MountTable>,
    path: &str,
    op: FileOp,
) -> Result<ResolvedPath, IOError> {
    // Platform compat: relative paths map to /code/ (game package directory)
    let resolved;
    let virtual_path = if !path.starts_with('/') {
        resolved = format!("/code/{}", path);
        &resolved
    } else {
        path
    };

    // /code paths: prefer mount table.
    if virtual_path == "/code" || virtual_path.starts_with("/code/") {
        // /code is read-only — reject writes early.
        if matches!(op, FileOp::Write | FileOp::Create | FileOp::Delete) {
            return Err(ioerr(format!("Permission denied: {}", path)));
        }
        if let Some(mt) = mount_table {
            let res = mt.resolve_code_path(virtual_path).ok_or_else(|| {
                ioerr(format!("Path resolution failed: {}", path))
            })?;
            return match res.real_path {
                Some(real) => Ok(ResolvedPath::Filesystem(real.to_string_lossy().into_owned())),
                None => Ok(ResolvedPath::Pack { virtual_path: virtual_path.to_string() }),
            };
        }
        // Fallback to VFS for /code if no mount table.
    }

    // /user, /cache, /tmp (and /code fallback) — go through VFS.
    let vfs = vfs.ok_or_else(|| ioerr("File system not initialized"))?;
    vfs.resolve(virtual_path, op)
        .map(|p| ResolvedPath::Filesystem(p.to_string_lossy().into_owned()))
        .map_err(|e| match e {
            VfsError::PathNotAllowed => ioerr(format!(
                "Path not allowed: {}. Use /user, /cache, /code, or /tmp",
                path
            )),
            VfsError::PermissionDenied => ioerr(format!("Permission denied: {}", path)),
            VfsError::PathTraversal => ioerr(format!("Path traversal detected: {}", path)),
            VfsError::SymlinkEscape => ioerr(format!("Symlink resolves outside sandbox: {}", path)),
            VfsError::SymlinkNotAllowed => {
                ioerr(format!("Symlinks not allowed in this directory: {}", path))
            }
            VfsError::InvalidPath => ioerr(format!("Invalid path: {}", path)),
        })
}

/// Extract a filesystem path from a resolved path, returning an error for
/// pack-backed paths.  Used by ops that haven't been adapted for pack reads.
#[inline]
fn require_fs_path(resolved: ResolvedPath) -> Result<String, IOError> {
    match resolved {
        ResolvedPath::Filesystem(p) => Ok(p),
        ResolvedPath::Pack { virtual_path } => Err(ioerr(format!(
            "Operation not supported on pack-backed path: {}",
            virtual_path,
        ))),
    }
}

/// Strip the `/code/` prefix to get a mount-table relative path.
#[inline]
fn code_relative(virtual_path: &str) -> &str {
    virtual_path.strip_prefix("/code/").unwrap_or("")
}

/// Read bytes for a /code path via MountTable.  Used by read-oriented ops
/// when the path resolves to a pack backend.
#[inline]
fn read_pack_bytes(
    mount_table: Option<&shared::vfs::MountTable>,
    virtual_path: &str,
) -> Result<Vec<u8>, IOError> {
    let mt = mount_table.ok_or_else(|| ioerr("mount table not initialized"))?;
    let relative = code_relative(virtual_path);
    let max_len = shared::protocol::io_cmd::MAX_READ_LENGTH;
    if let Some(size) = mt.entry_size(relative) {
        if size > max_len {
            return Err(ioerr(format!("file size {} exceeds limit {}", size, max_len)));
        }
    }
    mt.read_range_limited(relative, 0, None, max_len)
        .map_err(|e| ioerr(format!("pack read failed: {e}")))
}

fn materialize_pack_to_temp(
    mount_table: Option<&shared::vfs::MountTable>,
    virtual_path: &str,
    suffix: &str,
) -> Result<String, IOError> {
    static NEXT_TMP_ID: AtomicU64 = AtomicU64::new(1);

    let mt = mount_table.ok_or_else(|| ioerr("mount table not initialized"))?;
    let relative = code_relative(virtual_path);
    if let Some(size) = mt.entry_size(relative) {
        if size > MAX_MATERIALIZE_LENGTH {
            return Err(ioerr(format!(
                "file size {} exceeds materialize limit {}",
                size, MAX_MATERIALIZE_LENGTH,
            )));
        }
    }
    let mut dir = mt.code_dir();
    let parent = dir.parent().map(|p| p.to_path_buf()).unwrap_or_else(std::env::temp_dir);
    dir = parent.join(".migo-pack-materialized");
    std::fs::create_dir_all(&dir).map_err(|e| ioerr(format!("temp dir create failed: {e}")))?;

    for _ in 0..32 {
        let mut path = PathBuf::from(&dir);
        let id = NEXT_TMP_ID.fetch_add(1, Ordering::Relaxed);
        path.push(format!("pack_{}_{}{}", std::process::id(), id, suffix));
        let file = match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(ioerr(format!("temp file create failed: {e}"))),
        };
        let mut writer = std::io::BufWriter::new(file);
        if let Err(e) = mt.copy_to_writer(relative, &mut writer) {
            let _ = std::fs::remove_file(&path);
            return Err(ioerr(format!("pack materialize failed: {e}")));
        }
        if let Err(e) = writer.flush() {
            let _ = std::fs::remove_file(&path);
            return Err(ioerr(format!("temp flush failed: {e}")));
        }
        return Ok(path.to_string_lossy().into_owned());
    }

    Err(ioerr("failed to allocate unique temp file for pack materialization"))
}

/// Construct a StatResult for a pack-backed /code path.
/// Returns IOError for not-found, matching filesystem stat behavior.
fn pack_stat(
    mount_table: Option<&shared::vfs::MountTable>,
    virtual_path: &str,
    recursive: bool,
) -> Result<StatResult, IOError> {
    use shared::protocol::io_cmd::{FileStat, StatEntry, StatResult};
    let rel = code_relative(virtual_path);
    let mt = mount_table.ok_or_else(|| ioerr("mount table not initialized"))?;

    // Check if it's a file entry (not a directory).
    if mt.is_file(rel) {
        let size = mt.entry_size(rel).unwrap_or(0);
        return Ok(StatResult::Single(FileStat {
            mode: 0o444,
            size,
            atime: 0,
            mtime: 0,
            is_file: true,
            is_directory: false,
        }));
    }

    // Check if it's a directory prefix (has children).
    let children = mt.list_dir(rel);
    if !children.is_empty() || rel.is_empty() {
        if recursive {
            // Match filesystem stat_dir_recursive semantics:
            // - Only files, no directory entries
            // - Paths relative to queried dir
            // - Sorted by path (BTreeMap)
            use std::collections::BTreeMap;
            let mut out: BTreeMap<String, FileStat> = BTreeMap::new();
            let mut stack: Vec<(String, String)> = children.iter()
                .map(|name| {
                    let full = if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") };
                    (name.clone(), full)
                })
                .collect();
            while let Some((rel_name, full_path)) = stack.pop() {
                if mt.is_file(&full_path) {
                    // File entry — add to output.
                    let size = mt.entry_size(&full_path).unwrap_or(0);
                    out.insert(rel_name, FileStat {
                        mode: 0o444,
                        size,
                        atime: 0,
                        mtime: 0,
                        is_file: true,
                        is_directory: false,
                    });
                } else {
                    // Directory — recurse but don't add to output.
                    for child in mt.list_dir(&full_path) {
                        let child_rel = format!("{rel_name}/{child}");
                        let child_full = format!("{full_path}/{child}");
                        stack.push((child_rel, child_full));
                    }
                }
            }
            return Ok(StatResult::Recursive(
                out.into_iter()
                    .map(|(path, stat)| StatEntry { path, stat })
                    .collect(),
            ));
        }
        return Ok(StatResult::Single(FileStat {
            mode: 0o555,
            size: 0,
            atime: 0,
            mtime: 0,
            is_file: false,
            is_directory: true,
        }));
    }

    // Not found — return error matching filesystem stat behavior.
    Err(ioerr(format!("No such file or directory: {}", virtual_path)))
}

// pack_get_file_info removed — pack getFileInfo now routes through IO thread
// with pack_data to compute real digests (md5/sha1/sha256).

#[inline]
fn parse_open_flag(flag: &str) -> Result<OpenFlag, IOError> {
    match flag {
        "r" => Ok(OpenFlag::Read),
        "r+" => Ok(OpenFlag::ReadWrite),
        "w" => Ok(OpenFlag::WriteTruncateCreate),
        "w+" => Ok(OpenFlag::ReadWriteTruncateCreate),
        "a" => Ok(OpenFlag::AppendCreate),
        "a+" => Ok(OpenFlag::ReadAppendCreate),
        "ax" => Ok(OpenFlag::AppendExclusive),
        "ax+" => Ok(OpenFlag::ReadAppendExclusive),
        "as" => Ok(OpenFlag::AppendSyncCreate),
        "as+" => Ok(OpenFlag::ReadAppendSyncCreate),
        "wx" => Ok(OpenFlag::WriteExclusive),
        "wx+" => Ok(OpenFlag::ReadWriteExclusive),
        _ => Err(ioerr(format!("Invalid open flag: {flag}"))),
    }
}

/// Convert OpenFlag to VFS FileOp for permission checking.
#[inline]
fn open_flag_to_vfs_op(flag: &OpenFlag) -> FileOp {
    match flag {
        OpenFlag::Read => FileOp::Read,
        OpenFlag::ReadWrite
        | OpenFlag::ReadWriteTruncateCreate
        | OpenFlag::ReadAppendCreate
        | OpenFlag::ReadAppendExclusive
        | OpenFlag::ReadAppendSyncCreate
        | OpenFlag::ReadWriteExclusive => FileOp::Write,
        OpenFlag::WriteTruncateCreate
        | OpenFlag::AppendCreate
        | OpenFlag::AppendExclusive
        | OpenFlag::AppendSyncCreate
        | OpenFlag::WriteExclusive => FileOp::Create,
    }
}

#[inline]
fn mode_from_append(append: bool) -> WriteMode {
    if append {
        WriteMode::Append
    } else {
        WriteMode::Overwrite
    }
}

/// Prepare write data:
/// - If `data_buf` exists: returns (store, range)
/// - Else if `data_str` exists: encodes to Vec<u8>
/// - Else: error
fn prepare_data(
    data_buf: Option<JsBuffer>,
    data_str: Option<String>,
    encoding: Option<String>,
) -> Result<
    (
        Option<(v8::SharedRef<v8::BackingStore>, std::ops::Range<usize>)>,
        Option<Vec<u8>>,
    ),
    IOError,
> {
    if let Some(js_buf) = data_buf {
        let (store, range) = js_buf.into_parts().into_parts();
        Ok((Some((store, range)), None))
    } else if let Some(s) = data_str {
        let enc = encoding.as_deref().unwrap_or("utf8");
        let data = codec::encode_string(&s, enc).map_err(|e| ioerr(e.to_string()))?;
        Ok((None, Some(data)))
    } else {
        Err(ioerr("No data provided"))
    }
}

//
// Access - check if path exists (uses VFS)
//
#[op2(async(lazy), fast)]
pub async fn op_access(
    state: Rc<RefCell<OpState>>,
    #[string] path: String,
) -> Result<bool, IOError> {
    let (vfs, mt) = get_vfs_async(&state);
    match resolve_path_vfs(vfs.as_deref(), mt.as_deref(), &path, FileOp::Read)? {
        ResolvedPath::Pack { virtual_path } => {
            // Pack-backed: check existence via mount table.
            Ok(mt.as_deref()
                .map(|m| {
                    let rel = code_relative(&virtual_path);
                    m.exists_or_is_dir(rel)
                })
                .unwrap_or(false))
        }
        ResolvedPath::Filesystem(full_path) => {
            let tx = get_io_tx_async(state);
            protocol::send_fs_with_resp_async(&tx, |resp_tx| IOCmd::Access {
                path: full_path,
                resp: IOCmdResp::Async(resp_tx),
            })
            .await
            .map(|(is_file, is_dir, _size)| is_file || is_dir)
            .map_err(IOError::from)
        }
    }
}

#[op2(fast)]
pub fn op_access_sync(state: &mut OpState, #[string] path: String) -> Result<bool, IOError> {
    let (vfs, mt) = get_vfs_sync(state);
    match resolve_path_vfs(vfs.as_deref(), mt.as_deref(), &path, FileOp::Read)? {
        ResolvedPath::Pack { virtual_path } => {
            Ok(mt.as_deref()
                .map(|m| {
                    let rel = code_relative(&virtual_path);
                    m.exists_or_is_dir(rel)
                })
                .unwrap_or(false))
        }
        ResolvedPath::Filesystem(full_path) => {
            let tx = get_io_tx_sync(state);
            protocol::send_fs_with_resp_sync(tx, |resp_tx| IOCmd::Access {
                path: full_path,
                resp: IOCmdResp::Sync(resp_tx),
            })
            .map(|(is_file, is_dir, _size)| is_file || is_dir)
            .map_err(IOError::from)
        }
    }
}

//
// Write / append (path) - uses VFS with Write/Create permission
//
#[op2(async(lazy))]
pub async fn op_write_or_append_file(
    state: Rc<RefCell<OpState>>,
    #[string] path: String,
    #[buffer] data_buf: Option<JsBuffer>,
    #[string] data_str: Option<String>,
    #[string] encoding: Option<String>,
    append: bool,
) -> Result<bool, IOError> {
    let (vfs, mt) = get_vfs_async(&state);
    let full_path = require_fs_path(resolve_path_vfs(vfs.as_deref(), mt.as_deref(), &path, FileOp::Write)?)?;
    let tx: UnboundedSender<IOCmd> = get_io_tx_async(state);
    let mode = mode_from_append(append);
    let (buf_opt, data_opt) = prepare_data(data_buf, data_str, encoding)?;

    if let Some((store, range)) = buf_opt {
        let r = protocol::send_fs_with_resp_async(&tx, move |resp_tx| IOCmd::WriteShared {
            path: full_path,
            store,
            range,
            mode,
            resp: IOCmdResp::Async(resp_tx),
        })
        .await;
        return r.map_err(IOError::from);
    }

    if let Some(data) = data_opt {
        let r = protocol::send_fs_with_resp_async(&tx, move |resp_tx| IOCmd::Write {
            path: full_path,
            data,
            mode,
            resp: IOCmdResp::Async(resp_tx),
        })
        .await;
        return r.map_err(IOError::from);
    }

    Err(ioerr("No data provided"))
}

#[op2]
pub fn op_write_or_append_file_sync(
    state: &mut OpState,
    #[string] path: String,
    #[buffer] data_buf: Option<JsBuffer>,
    #[string] data_str: Option<String>,
    #[string] encoding: Option<String>,
    append: bool,
) -> Result<bool, IOError> {
    let (vfs, mt) = get_vfs_sync(state);
    let full_path = require_fs_path(resolve_path_vfs(vfs.as_deref(), mt.as_deref(), &path, FileOp::Write)?)?;
    let tx = get_io_tx_sync(state);
    let mode = mode_from_append(append);
    let (buf_opt, data_opt) = prepare_data(data_buf, data_str, encoding)?;

    if let Some((store, range)) = buf_opt {
        let r = protocol::send_fs_with_resp_sync(tx, move |resp_tx| IOCmd::WriteShared {
            path: full_path,
            store,
            range,
            mode,
            resp: IOCmdResp::Sync(resp_tx),
        });
        return r.map_err(IOError::from);
    }

    if let Some(data) = data_opt {
        let r = protocol::send_fs_with_resp_sync(tx, move |resp_tx| IOCmd::Write {
            path: full_path,
            data,
            mode,
            resp: IOCmdResp::Sync(resp_tx),
        });
        return r.map_err(IOError::from);
    }

    Err(ioerr("No data provided"))
}

//
// Open / close - uses VFS with appropriate permission based on open flag
//
#[op2(async(lazy), fast)]
pub async fn op_open_file(
    state: Rc<RefCell<OpState>>,
    #[string] path: String,
    #[string] flag: String,
) -> Result<u32, IOError> {
    let (vfs, mt) = get_vfs_async(&state);
    let open_flag = parse_open_flag(&flag)?;
    let vfs_op = open_flag_to_vfs_op(&open_flag);
    let (full_path, cleanup_path, synthetic_stat) = match resolve_path_vfs(vfs.as_deref(), mt.as_deref(), &path, vfs_op)? {
        ResolvedPath::Filesystem(p) => (p, None, None),
        ResolvedPath::Pack { virtual_path } => {
            if open_flag != OpenFlag::Read {
                return Err(ioerr(format!("pack-backed open only supports read mode: {}", virtual_path)));
            }
            let temp_path = materialize_pack_to_temp(mt.as_deref(), &virtual_path, ".open")?;
            let stat = match pack_stat(mt.as_deref(), &virtual_path, false)? {
                StatResult::Single(stat) => stat,
                StatResult::Recursive(_) => unreachable!(),
            };
            (temp_path.clone(), Some(temp_path), Some(stat))
        }
    };
    let tx = get_io_tx_async(state);
    let cleanup_on_error = cleanup_path.clone();

    protocol::send_fs_with_resp_async(&tx, move |resp_tx| IOCmd::Open {
        path: full_path,
        flag: open_flag,
        cleanup_path,
        synthetic_stat,
        resp: IOCmdResp::Async(resp_tx),
    })
    .await
    .map_err(|e| {
        if let Some(path) = cleanup_on_error {
            let _ = std::fs::remove_file(path);
        }
        IOError::from(e)
    })
}

#[op2(fast)]
pub fn op_open_file_sync(
    state: &mut OpState,
    #[string] path: String,
    #[string] flag: String,
) -> Result<u32, IOError> {
    let (vfs, mt) = get_vfs_sync(state);
    let open_flag = parse_open_flag(&flag)?;
    let vfs_op = open_flag_to_vfs_op(&open_flag);
    let (full_path, cleanup_path, synthetic_stat) = match resolve_path_vfs(vfs.as_deref(), mt.as_deref(), &path, vfs_op)? {
        ResolvedPath::Filesystem(p) => (p, None, None),
        ResolvedPath::Pack { virtual_path } => {
            if open_flag != OpenFlag::Read {
                return Err(ioerr(format!("pack-backed open only supports read mode: {}", virtual_path)));
            }
            let temp_path = materialize_pack_to_temp(mt.as_deref(), &virtual_path, ".open")?;
            let stat = match pack_stat(mt.as_deref(), &virtual_path, false)? {
                StatResult::Single(stat) => stat,
                StatResult::Recursive(_) => unreachable!(),
            };
            (temp_path.clone(), Some(temp_path), Some(stat))
        }
    };
    let tx = get_io_tx_sync(state);
    let cleanup_on_error = cleanup_path.clone();

    protocol::send_fs_with_resp_sync(tx, move |resp_tx| IOCmd::Open {
        path: full_path,
        flag: open_flag,
        cleanup_path,
        synthetic_stat,
        resp: IOCmdResp::Sync(resp_tx),
    })
    .map_err(|e| {
        if let Some(path) = cleanup_on_error {
            let _ = std::fs::remove_file(path);
        }
        IOError::from(e)
    })
}

#[op2(async(lazy), fast)]
pub async fn op_close_file(state: Rc<RefCell<OpState>>, #[smi] rid: FileId) -> Result<(), IOError> {
    let tx = get_io_tx_async(state);

    protocol::send_fs_with_resp_async(&tx, move |resp_tx| IOCmd::Close {
        rid,
        resp: IOCmdResp::Async(resp_tx),
    })
    .await
    .map_err(IOError::from)
}

#[op2(fast)]
pub fn op_close_file_sync(state: &mut OpState, #[smi] rid: FileId) -> Result<(), IOError> {
    let tx = get_io_tx_sync(state);

    protocol::send_fs_with_resp_sync(tx, move |resp_tx| IOCmd::Close {
        rid,
        resp: IOCmdResp::Sync(resp_tx),
    })
    .map_err(IOError::from)
}

//
// Copy - uses VFS for both source (Read) and destination (Create)
//
#[op2(async(lazy), fast)]
pub async fn op_copy_file(
    state: Rc<RefCell<OpState>>,
    #[string] src_path: String,
    #[string] dest_path: String,
) -> Result<(), IOError> {
    let (vfs, mt) = get_vfs_async(&state);
    let src_resolved = resolve_path_vfs(vfs.as_deref(), mt.as_deref(), &src_path, FileOp::Read)?;
    let dest_full = require_fs_path(resolve_path_vfs(vfs.as_deref(), mt.as_deref(), &dest_path, FileOp::Create)?)?;
    let tx = get_io_tx_async(state);

    match src_resolved {
        ResolvedPath::Filesystem(src_full) => {
            protocol::send_fs_with_resp_async(&tx, move |resp_tx| IOCmd::Copy {
                src_path: src_full,
                dest_path: dest_full,
                resp: IOCmdResp::Async(resp_tx),
            })
            .await
            .map_err(IOError::from)
        }
        ResolvedPath::Pack { virtual_path } => {
            let m = mt.as_deref().ok_or_else(|| ioerr("mount table not initialized"))?;
            let rel = code_relative(&virtual_path);
            m.copy_to_path(rel, std::path::Path::new(&dest_full))
                .map_err(|e| ioerr(format!("pack copy failed: {e}")))
        }
    }
}

#[op2(fast)]
pub fn op_copy_file_sync(
    state: &mut OpState,
    #[string] src_path: String,
    #[string] dest_path: String,
) -> Result<(), IOError> {
    let (vfs, mt) = get_vfs_sync(state);
    let src_resolved = resolve_path_vfs(vfs.as_deref(), mt.as_deref(), &src_path, FileOp::Read)?;
    let dest_full = require_fs_path(resolve_path_vfs(vfs.as_deref(), mt.as_deref(), &dest_path, FileOp::Create)?)?;
    let tx = get_io_tx_sync(state);

    match src_resolved {
        ResolvedPath::Filesystem(src_full) => {
            protocol::send_fs_with_resp_sync(tx, move |resp_tx| IOCmd::Copy {
                src_path: src_full,
                dest_path: dest_full,
                resp: IOCmdResp::Sync(resp_tx),
            })
            .map_err(IOError::from)
        }
        ResolvedPath::Pack { virtual_path } => {
            let m = mt.as_deref().ok_or_else(|| ioerr("mount table not initialized"))?;
            let rel = code_relative(&virtual_path);
            m.copy_to_path(rel, std::path::Path::new(&dest_full))
                .map_err(|e| ioerr(format!("pack copy failed: {e}")))
        }
    }
}

//
// fstat / ftruncate
//
#[op2(async(lazy), fast)]
#[serde]
pub async fn op_fstat(
    state: Rc<RefCell<OpState>>,
    #[smi] rid: FileId,
) -> Result<FileStat, IOError> {
    let tx = get_io_tx_async(state);

    protocol::send_fs_with_resp_async(&tx, move |resp_tx| IOCmd::Fstat {
        rid,
        resp: IOCmdResp::Async(resp_tx),
    })
    .await
    .map_err(IOError::from)
}

#[op2]
#[serde]
pub fn op_fstat_sync(state: &mut OpState, #[smi] rid: FileId) -> Result<FileStat, IOError> {
    let tx = get_io_tx_sync(state);

    protocol::send_fs_with_resp_sync(tx, move |resp_tx| IOCmd::Fstat {
        rid,
        resp: IOCmdResp::Sync(resp_tx),
    })
    .map_err(IOError::from)
}

#[op2(async(lazy), fast)]
pub async fn op_ftruncate(
    state: Rc<RefCell<OpState>>,
    #[smi] rid: FileId,
    #[smi] len: u64,
) -> Result<(), IOError> {
    let tx = get_io_tx_async(state);

    protocol::send_fs_with_resp_async(&tx, move |resp_tx| IOCmd::Ftruncate {
        rid,
        len,
        resp: IOCmdResp::Async(resp_tx),
    })
    .await
    .map_err(IOError::from)
}

#[op2(fast)]
pub fn op_ftruncate_sync(
    state: &mut OpState,
    #[smi] rid: FileId,
    #[smi] len: u64,
) -> Result<(), IOError> {
    let tx = get_io_tx_sync(state);

    protocol::send_fs_with_resp_sync(tx, move |resp_tx| IOCmd::Ftruncate {
        rid,
        len,
        resp: IOCmdResp::Sync(resp_tx),
    })
    .map_err(IOError::from)
}

//
// mkdir / readdir - uses VFS with Create/Read permissions
//
#[op2(async(lazy), fast)]
pub async fn op_mkdir(
    state: Rc<RefCell<OpState>>,
    #[string] dir_path: String,
    recursive: bool,
) -> Result<(), IOError> {
    let (vfs, mt) = get_vfs_async(&state);
    let full_path = require_fs_path(resolve_path_vfs(vfs.as_deref(), mt.as_deref(), &dir_path, FileOp::Create)?)?;
    let tx = get_io_tx_async(state);

    protocol::send_fs_with_resp_async(&tx, move |resp_tx| IOCmd::Mkdir {
        dir_path: full_path,
        recursive,
        resp: IOCmdResp::Async(resp_tx),
    })
    .await
    .map_err(IOError::from)
}

#[op2(fast)]
pub fn op_mkdir_sync(
    state: &mut OpState,
    #[string] dir_path: String,
    recursive: bool,
) -> Result<(), IOError> {
    let (vfs, mt) = get_vfs_sync(state);
    let full_path = require_fs_path(resolve_path_vfs(vfs.as_deref(), mt.as_deref(), &dir_path, FileOp::Create)?)?;
    let tx = get_io_tx_sync(state);

    protocol::send_fs_with_resp_sync(tx, move |resp_tx| IOCmd::Mkdir {
        dir_path: full_path,
        recursive,
        resp: IOCmdResp::Sync(resp_tx),
    })
    .map_err(IOError::from)
}

#[op2(async(lazy), fast)]
#[serde]
pub async fn op_readdir(
    state: Rc<RefCell<OpState>>,
    #[string] dir_path: String,
) -> Result<Vec<String>, IOError> {
    let (vfs, mt) = get_vfs_async(&state);
    match resolve_path_vfs(vfs.as_deref(), mt.as_deref(), &dir_path, FileOp::Read)? {
        ResolvedPath::Pack { virtual_path } => {
            let m = mt.as_deref().ok_or_else(|| ioerr("mount table not initialized"))?;
            let rel = code_relative(&virtual_path);
            if !m.exists_or_is_dir(rel) {
                return Err(ioerr(format!("No such file or directory: {}", virtual_path)));
            }
            if m.is_file(rel) {
                return Err(ioerr(format!("Not a directory: {}", virtual_path)));
            }
            Ok(m.list_dir(rel))
        }
        ResolvedPath::Filesystem(full_path) => {
            let tx = get_io_tx_async(state);
            protocol::send_fs_with_resp_async(&tx, move |resp_tx| IOCmd::Readdir {
                dir_path: full_path,
                resp: IOCmdResp::Async(resp_tx),
            })
            .await
            .map_err(IOError::from)
        }
    }
}

#[op2]
#[serde]
pub fn op_readdir_sync(
    state: &mut OpState,
    #[string] dir_path: String,
) -> Result<Vec<String>, IOError> {
    let (vfs, mt) = get_vfs_sync(state);
    match resolve_path_vfs(vfs.as_deref(), mt.as_deref(), &dir_path, FileOp::Read)? {
        ResolvedPath::Pack { virtual_path } => {
            let m = mt.as_deref().ok_or_else(|| ioerr("mount table not initialized"))?;
            let rel = code_relative(&virtual_path);
            if !m.exists_or_is_dir(rel) {
                return Err(ioerr(format!("No such file or directory: {}", virtual_path)));
            }
            if m.is_file(rel) {
                return Err(ioerr(format!("Not a directory: {}", virtual_path)));
            }
            Ok(m.list_dir(rel))
        }
        ResolvedPath::Filesystem(full_path) => {
            let tx = get_io_tx_sync(state);
            protocol::send_fs_with_resp_sync(tx, move |resp_tx| IOCmd::Readdir {
                dir_path: full_path,
                resp: IOCmdResp::Sync(resp_tx),
            })
            .map_err(IOError::from)
        }
    }
}

//
// unlink / rename / rmdir - uses VFS with Delete/Write permissions
//
#[op2(async(lazy), fast)]
pub async fn op_unlink(
    state: Rc<RefCell<OpState>>,
    #[string] file_path: String,
) -> Result<(), IOError> {
    let (vfs, mt) = get_vfs_async(&state);
    let full_path = require_fs_path(resolve_path_vfs(vfs.as_deref(), mt.as_deref(), &file_path, FileOp::Delete)?)?;
    let tx = get_io_tx_async(state);

    protocol::send_fs_with_resp_async(&tx, move |resp_tx| IOCmd::Unlink {
        file_path: full_path,
        resp: IOCmdResp::Async(resp_tx),
    })
    .await
    .map_err(IOError::from)
}

#[op2(fast)]
pub fn op_unlink_sync(state: &mut OpState, #[string] file_path: String) -> Result<(), IOError> {
    let (vfs, mt) = get_vfs_sync(state);
    let full_path = require_fs_path(resolve_path_vfs(vfs.as_deref(), mt.as_deref(), &file_path, FileOp::Delete)?)?;
    let tx = get_io_tx_sync(state);

    protocol::send_fs_with_resp_sync(tx, move |resp_tx| IOCmd::Unlink {
        file_path: full_path,
        resp: IOCmdResp::Sync(resp_tx),
    })
    .map_err(IOError::from)
}

#[op2(async(lazy), fast)]
pub async fn op_rename(
    state: Rc<RefCell<OpState>>,
    #[string] old_path: String,
    #[string] new_path: String,
) -> Result<(), IOError> {
    let (vfs, mt) = get_vfs_async(&state);
    // Rename needs delete on source and create on destination
    let old_full = require_fs_path(resolve_path_vfs(vfs.as_deref(), mt.as_deref(), &old_path, FileOp::Delete)?)?;
    let new_full = require_fs_path(resolve_path_vfs(vfs.as_deref(), mt.as_deref(), &new_path, FileOp::Create)?)?;
    let tx = get_io_tx_async(state);

    protocol::send_fs_with_resp_async(&tx, move |resp_tx| IOCmd::Rename {
        old_path: old_full,
        new_path: new_full,
        resp: IOCmdResp::Async(resp_tx),
    })
    .await
    .map_err(IOError::from)
}

#[op2(fast)]
pub fn op_rename_sync(
    state: &mut OpState,
    #[string] old_path: String,
    #[string] new_path: String,
) -> Result<(), IOError> {
    let (vfs, mt) = get_vfs_sync(state);
    let old_full = require_fs_path(resolve_path_vfs(vfs.as_deref(), mt.as_deref(), &old_path, FileOp::Delete)?)?;
    let new_full = require_fs_path(resolve_path_vfs(vfs.as_deref(), mt.as_deref(), &new_path, FileOp::Create)?)?;
    let tx = get_io_tx_sync(state);

    protocol::send_fs_with_resp_sync(tx, move |resp_tx| IOCmd::Rename {
        old_path: old_full,
        new_path: new_full,
        resp: IOCmdResp::Sync(resp_tx),
    })
    .map_err(IOError::from)
}

#[op2(async(lazy), fast)]
pub async fn op_rmdir(
    state: Rc<RefCell<OpState>>,
    #[string] dir_path: String,
    recursive: bool,
) -> Result<(), IOError> {
    let (vfs, mt) = get_vfs_async(&state);
    let full_path = require_fs_path(resolve_path_vfs(vfs.as_deref(), mt.as_deref(), &dir_path, FileOp::Delete)?)?;
    let tx = get_io_tx_async(state);

    protocol::send_fs_with_resp_async(&tx, move |resp_tx| IOCmd::Rmdir {
        dir_path: full_path,
        recursive,
        resp: IOCmdResp::Async(resp_tx),
    })
    .await
    .map_err(IOError::from)
}

#[op2(fast)]
pub fn op_rmdir_sync(
    state: &mut OpState,
    #[string] dir_path: String,
    recursive: bool,
) -> Result<(), IOError> {
    let (vfs, mt) = get_vfs_sync(state);
    let full_path = require_fs_path(resolve_path_vfs(vfs.as_deref(), mt.as_deref(), &dir_path, FileOp::Delete)?)?;
    let tx = get_io_tx_sync(state);

    protocol::send_fs_with_resp_sync(tx, move |resp_tx| IOCmd::Rmdir {
        dir_path: full_path,
        recursive,
        resp: IOCmdResp::Sync(resp_tx),
    })
    .map_err(IOError::from)
}

//
// stat - uses VFS with Read permission
//
#[op2(async(lazy), fast)]
#[serde]
pub async fn op_stat(
    state: Rc<RefCell<OpState>>,
    #[string] path: String,
    recursive: bool,
) -> Result<StatResult, IOError> {
    let (vfs, mt) = get_vfs_async(&state);
    match resolve_path_vfs(vfs.as_deref(), mt.as_deref(), &path, FileOp::Read)? {
        ResolvedPath::Pack { virtual_path } => {
            pack_stat(mt.as_deref(), &virtual_path, recursive)
        }
        ResolvedPath::Filesystem(full_path) => {
            let tx = get_io_tx_async(state);
            protocol::send_fs_with_resp_async(&tx, move |resp_tx| IOCmd::Stat {
                path: full_path,
                recursive,
                resp: IOCmdResp::Async(resp_tx),
            })
            .await
            .map_err(IOError::from)
        }
    }
}

#[op2]
#[serde]
pub fn op_stat_sync(
    state: &mut OpState,
    #[string] path: String,
    recursive: bool,
) -> Result<StatResult, IOError> {
    let (vfs, mt) = get_vfs_sync(state);
    match resolve_path_vfs(vfs.as_deref(), mt.as_deref(), &path, FileOp::Read)? {
        ResolvedPath::Pack { virtual_path } => {
            pack_stat(mt.as_deref(), &virtual_path, recursive)
        }
        ResolvedPath::Filesystem(full_path) => {
            let tx = get_io_tx_sync(state);
            protocol::send_fs_with_resp_sync(tx, move |resp_tx| IOCmd::Stat {
                path: full_path,
                recursive,
                resp: IOCmdResp::Sync(resp_tx),
            })
            .map_err(IOError::from)
        }
    }
}

//
// write(fd)
//
#[op2(async(lazy))]
#[bigint]
pub async fn op_write_file(
    state: Rc<RefCell<OpState>>,
    #[smi] rid: FileId,
    #[buffer] data_buf: Option<JsBuffer>,
    #[string] data_str: Option<String>,
    #[string] encoding: Option<String>,
    #[bigint] position: Option<u64>,
) -> Result<usize, IOError> {
    let tx = get_io_tx_async(state);
    let (buf_opt, data_opt) = prepare_data(data_buf, data_str, encoding)?;

    let r = if let Some((store, range)) = buf_opt {
        protocol::send_fs_with_resp_async(&tx, move |resp_tx| IOCmd::WriteFdShared {
            rid,
            store,
            range,
            position,
            resp: IOCmdResp::Async(resp_tx),
        })
        .await
    } else if let Some(data) = data_opt {
        protocol::send_fs_with_resp_async(&tx, move |resp_tx| IOCmd::WriteFd {
            rid,
            data,
            position,
            resp: IOCmdResp::Async(resp_tx),
        })
        .await
    } else {
        return Err(ioerr("No data provided"));
    };

    r.map_err(IOError::from)
}

#[op2]
#[bigint]
pub fn op_write_file_sync(
    state: &mut OpState,
    #[smi] rid: FileId,
    #[buffer] data_buf: Option<JsBuffer>,
    #[string] data_str: Option<String>,
    #[string] encoding: Option<String>,
    #[bigint] position: Option<u64>,
) -> Result<usize, IOError> {
    let tx = get_io_tx_sync(state);
    let (buf_opt, data_opt) = prepare_data(data_buf, data_str, encoding)?;

    let r = if let Some((store, range)) = buf_opt {
        protocol::send_fs_with_resp_sync(tx, move |resp_tx| IOCmd::WriteFdShared {
            rid,
            store,
            range,
            position,
            resp: IOCmdResp::Sync(resp_tx),
        })
    } else if let Some(data) = data_opt {
        protocol::send_fs_with_resp_sync(tx, move |resp_tx| IOCmd::WriteFd {
            rid,
            data,
            position,
            resp: IOCmdResp::Sync(resp_tx),
        })
    } else {
        return Err(ioerr("No data provided"));
    };

    r.map_err(IOError::from)
}

//
// readFile (path) - uses VFS for path resolution
//
#[op2(async(lazy))]
#[serde]
pub async fn op_read_file(
    state: Rc<RefCell<OpState>>,
    #[string] path: String,
    #[bigint] position: Option<u64>,
    #[bigint] length: Option<u64>,
) -> Result<ToJsBuffer, IOError> {
    let (vfs, mt) = get_vfs_async(&state);
    match resolve_path_vfs(vfs.as_deref(), mt.as_deref(), &path, FileOp::Read)? {
        ResolvedPath::Pack { virtual_path } => {
            let mt = mt.as_deref().ok_or_else(|| ioerr("mount table not initialized"))?;
            let rel = code_relative(&virtual_path);
            let max_len = shared::protocol::io_cmd::MAX_READ_LENGTH;
            // Reject if explicit length > MAX_READ_LENGTH.
            if let Some(len) = length {
                if len > max_len {
                    return Err(ioerr(format!(
                        "read length {} exceeds limit {}", len, max_len,
                    )));
                }
            }
            // When length is not specified, check the effective read size
            // (entry_size - position) against the limit.  This prevents
            // unbounded reads from oversized pack entries regardless of
            // whether position is specified.
            if length.is_none() {
                if let Some(entry_sz) = mt.entry_size(rel) {
                    let effective = entry_sz.saturating_sub(position.unwrap_or(0));
                    if effective > max_len {
                        return Err(ioerr(format!(
                            "file size {} exceeds limit {}", effective, max_len,
                        )));
                    }
                }
            }
            let data = mt.read_range_limited(rel, position.unwrap_or(0), length, max_len)
                .map_err(|e| ioerr(format!("pack read failed: {e}")))?;
            Ok(data.into())
        }
        ResolvedPath::Filesystem(full_path) => {
            let tx = get_io_tx_async(state);
            protocol::send_fs_with_resp_async(&tx, move |resp_tx| IOCmd::ReadFile {
                path: full_path,
                position,
                length,
                resp: IOCmdResp::Async(resp_tx),
            })
            .await
            .map(|data| data.into())
            .map_err(IOError::from)
        }
    }
}

#[op2]
#[serde]
pub fn op_read_file_sync(
    state: &mut OpState,
    #[string] path: String,
    #[bigint] position: Option<u64>,
    #[bigint] length: Option<u64>,
) -> Result<ToJsBuffer, IOError> {
    let (vfs, mt) = get_vfs_sync(state);
    match resolve_path_vfs(vfs.as_deref(), mt.as_deref(), &path, FileOp::Read)? {
        ResolvedPath::Pack { virtual_path } => {
            let mt = mt.as_deref().ok_or_else(|| ioerr("mount table not initialized"))?;
            let rel = code_relative(&virtual_path);
            let max_len = shared::protocol::io_cmd::MAX_READ_LENGTH;
            // Reject if explicit length > MAX_READ_LENGTH.
            if let Some(len) = length {
                if len > max_len {
                    return Err(ioerr(format!(
                        "read length {} exceeds limit {}", len, max_len,
                    )));
                }
            }
            // When length is not specified, check the effective read size
            // (entry_size - position) against the limit.  This prevents
            // unbounded reads from oversized pack entries regardless of
            // whether position is specified.
            if length.is_none() {
                if let Some(entry_sz) = mt.entry_size(rel) {
                    let effective = entry_sz.saturating_sub(position.unwrap_or(0));
                    if effective > max_len {
                        return Err(ioerr(format!(
                            "file size {} exceeds limit {}", effective, max_len,
                        )));
                    }
                }
            }
            let data = mt.read_range_limited(rel, position.unwrap_or(0), length, max_len)
                .map_err(|e| ioerr(format!("pack read failed: {e}")))?;
            Ok(data.into())
        }
        ResolvedPath::Filesystem(full_path) => {
            let tx = get_io_tx_sync(state);
            protocol::send_fs_with_resp_sync(tx, move |resp_tx| IOCmd::ReadFile {
                path: full_path,
                position,
                length,
                resp: IOCmdResp::Sync(resp_tx),
            })
            .map(|data| data.into())
            .map_err(IOError::from)
        }
    }
}

//
// read(fd) - fd-based read into buffer
//
#[op2(async(lazy))]
#[serde]
pub async fn op_read_fd(
    state: Rc<RefCell<OpState>>,
    #[smi] rid: FileId,
    #[bigint] length: u64,
    #[bigint] position: Option<u64>,
) -> Result<ToJsBuffer, IOError> {
    let tx = get_io_tx_async(state);

    protocol::send_fs_with_resp_async(&tx, move |resp_tx| IOCmd::ReadFd {
        rid,
        length,
        position,
        resp: IOCmdResp::Async(resp_tx),
    })
    .await
    .map(|data| data.into())
    .map_err(IOError::from)
}

#[op2]
#[serde]
pub fn op_read_fd_sync(
    state: &mut OpState,
    #[smi] rid: FileId,
    #[bigint] length: u64,
    #[bigint] position: Option<u64>,
) -> Result<ToJsBuffer, IOError> {
    let tx = get_io_tx_sync(state);

    protocol::send_fs_with_resp_sync(tx, move |resp_tx| IOCmd::ReadFd {
        rid,
        length,
        position,
        resp: IOCmdResp::Sync(resp_tx),
    })
    .map(|data| data.into())
    .map_err(IOError::from)
}

//
// readCompressedFile (path) - read brotli-compressed file
//
#[op2(async(lazy), fast)]
#[serde]
pub async fn op_read_compressed_file(
    state: Rc<RefCell<OpState>>,
    #[string] path: String,
) -> Result<ToJsBuffer, IOError> {
    let (vfs, mt) = get_vfs_async(&state);
    let (full_path, pack_data) = match resolve_path_vfs(vfs.as_deref(), mt.as_deref(), &path, FileOp::Read)? {
        ResolvedPath::Filesystem(p) => (p, None),
        ResolvedPath::Pack { virtual_path } => {
            let data = read_pack_bytes(mt.as_deref(), &virtual_path)?;
            (virtual_path, Some(data))
        }
    };
    let tx = get_io_tx_async(state);

    protocol::send_fs_with_resp_async(&tx, move |resp_tx| IOCmd::ReadCompressedFile {
        path: full_path,
        pack_data,
        resp: IOCmdResp::Async(resp_tx),
    })
    .await
    .map(|data| data.into())
    .map_err(IOError::from)
}

#[op2]
#[serde]
pub fn op_read_compressed_file_sync(
    state: &mut OpState,
    #[string] path: String,
) -> Result<ToJsBuffer, IOError> {
    let (vfs, mt) = get_vfs_sync(state);
    let (full_path, pack_data) = match resolve_path_vfs(vfs.as_deref(), mt.as_deref(), &path, FileOp::Read)? {
        ResolvedPath::Filesystem(p) => (p, None),
        ResolvedPath::Pack { virtual_path } => {
            let data = read_pack_bytes(mt.as_deref(), &virtual_path)?;
            (virtual_path, Some(data))
        }
    };
    let tx = get_io_tx_sync(state);

    protocol::send_fs_with_resp_sync(tx, move |resp_tx| IOCmd::ReadCompressedFile {
        path: full_path,
        pack_data,
        resp: IOCmdResp::Sync(resp_tx),
    })
    .map(|data| data.into())
    .map_err(IOError::from)
}

// ============================ ReadZipEntry ============================

#[op2(async(lazy), fast)]
#[serde]
pub async fn op_read_zip_entry(
    state: Rc<RefCell<OpState>>,
    #[string] zip_path: String,
    #[string] entries_json: String,
) -> Result<serde_json::Value, IOError> {
    let (vfs, mt) = get_vfs_async(&state);
    let (full_path, pack_data, cleanup_path) = match resolve_path_vfs(vfs.as_deref(), mt.as_deref(), &zip_path, FileOp::Read)? {
        ResolvedPath::Filesystem(p) => (p, None, None),
        ResolvedPath::Pack { virtual_path } => {
            let temp_path = materialize_pack_to_temp(mt.as_deref(), &virtual_path, ".zip")?;
            (temp_path.clone(), None, Some(temp_path))
        }
    };
    let tx = get_io_tx_async(state);

    let results = protocol::send_fs_with_resp_async(&tx, move |resp_tx| IOCmd::ReadZipEntry {
        zip_path: full_path,
        entries_json,
        pack_data,
        resp: IOCmdResp::Async(resp_tx),
    }).await;

    if let Some(path) = &cleanup_path {
        let _ = std::fs::remove_file(path);
    }

    let results = results.map_err(IOError::from)?;

    // Build serde_json::Value — serde_v8 serializes this directly to a V8 object
    // (no JSON stringify/parse round-trip).
    let mut entries_map = serde_json::Map::with_capacity(results.len());
    for entry in results {
        let data_val = match entry.data {
            Some(s) => serde_json::Value::String(s),
            None => serde_json::Value::Null,
        };
        entries_map.insert(
            entry.path,
            serde_json::json!({ "data": data_val, "errMsg": entry.err_msg }),
        );
    }
    Ok(serde_json::json!({ "entries": entries_map }))
}

// ============================ Unzip ============================

/// Unzip operation with platform service dispatch.
///
/// If the platform provides a `FileService` (e.g. Android's JNI `java.util.zip`),
/// uses that. Otherwise falls back to `IOCmd::Unzip` (Rust `zip` crate on IO thread).
#[op2(async(lazy), fast)]
pub async fn op_unzip(
    state: Rc<RefCell<OpState>>,
    #[string] zip_file_path: String,
    #[string] target_path: String,
) -> Result<(), IOError> {
    let (io_tx, vfs, mt, file_svc) = {
        let st = state.borrow();
        let host = st.borrow::<HostOpState>();
        let file_svc = host.device_services.as_ref().and_then(|s| s.file());
        (host.io_tx.clone(), host.vfs.clone(), host.mount_table.clone(), file_svc)
    };

    let (full_zip_path, cleanup_path) = match resolve_path_vfs(vfs.as_deref(), mt.as_deref(), &zip_file_path, FileOp::Read)? {
        ResolvedPath::Filesystem(p) => (p, None),
        ResolvedPath::Pack { virtual_path } => {
            let temp_path = materialize_pack_to_temp(mt.as_deref(), &virtual_path, ".unzip")?;
            (temp_path.clone(), Some(temp_path))
        }
    };
    let full_dest_dir = require_fs_path(resolve_path_vfs(vfs.as_deref(), mt.as_deref(), &target_path, FileOp::Write)?)?;

    // Platform-specific unzip (e.g. Android JNI)
    if let Some(svc) = file_svc {
        let zip = full_zip_path.clone();
        let dest = full_dest_dir.clone();
        let result = tokio::task::spawn_blocking(move || svc.unzip(&zip, &dest))
            .await
            .map_err(|e| IOError::Message(format!("unzip task join error: {e}")))?
            .map(|_| ())
            .map_err(|e| IOError::Message(e.to_string()));
        if let Some(path) = cleanup_path {
            let _ = std::fs::remove_file(path);
        }
        return result;
    }

    // Default: Rust zip crate via IO thread
    let result = protocol::send_fs_with_resp_async(&io_tx, move |resp_tx| IOCmd::Unzip {
        zip_path: full_zip_path,
        dest_dir: full_dest_dir,
        resp: IOCmdResp::Async(resp_tx),
    })
    .await
    .map(|_| ()) // Discard file count, JS doesn't need it
    .map_err(IOError::from);

    if let Some(path) = cleanup_path {
        let _ = std::fs::remove_file(path);
    }
    result
}

// ============================ GetFileInfo ============================

#[op2(async(lazy), fast)]
#[serde]
pub async fn op_get_file_info(
    state: Rc<RefCell<OpState>>,
    #[string] path: String,
    #[string] algorithm: String,
) -> Result<(u64, String), IOError> {
    let (vfs, mt) = get_vfs_async(&state);
    let (full_path, pack_data) = match resolve_path_vfs(vfs.as_deref(), mt.as_deref(), &path, FileOp::Read)? {
        ResolvedPath::Pack { virtual_path } => {
            let m = mt.as_deref().ok_or_else(|| ioerr("mount table not initialized"))?;
            let rel = code_relative(&virtual_path);
            return m
                .get_file_info(rel, &algorithm)
                .map_err(|e| ioerr(format!("pack getFileInfo failed: {e}")));
        }
        ResolvedPath::Filesystem(fp) => (fp, None),
    };
    let tx = get_io_tx_async(state);
    protocol::send_fs_with_resp_async(&tx, move |resp_tx| IOCmd::GetFileInfo {
        path: full_path,
        algorithm,
        pack_data,
        resp: IOCmdResp::Async(resp_tx),
    })
    .await
    .map_err(IOError::from)
}

#[op2]
#[serde]
pub fn op_get_file_info_sync(
    state: &mut OpState,
    #[string] path: String,
    #[string] algorithm: String,
) -> Result<(u64, String), IOError> {
    let (vfs, mt) = get_vfs_sync(state);
    let (full_path, pack_data) = match resolve_path_vfs(vfs.as_deref(), mt.as_deref(), &path, FileOp::Read)? {
        ResolvedPath::Pack { virtual_path } => {
            let m = mt.as_deref().ok_or_else(|| ioerr("mount table not initialized"))?;
            let rel = code_relative(&virtual_path);
            return m
                .get_file_info(rel, &algorithm)
                .map_err(|e| ioerr(format!("pack getFileInfo failed: {e}")));
        }
        ResolvedPath::Filesystem(fp) => (fp, None),
    };
    let tx = get_io_tx_sync(state);
    protocol::send_fs_with_resp_sync(tx, move |resp_tx| IOCmd::GetFileInfo {
        path: full_path,
        algorithm,
        pack_data,
        resp: IOCmdResp::Sync(resp_tx),
    })
    .map_err(IOError::from)
}

// ============================ ListSavedFiles ============================

#[op2(async(lazy), fast)]
#[serde]
pub async fn op_list_saved_files(
    state: Rc<RefCell<OpState>>,
    #[string] dir: String,
    #[string] prefix: String,
) -> Result<Vec<SavedFileInfo>, IOError> {
    let (vfs, mt) = get_vfs_async(&state);
    let full_dir = require_fs_path(resolve_path_vfs(vfs.as_deref(), mt.as_deref(), &dir, FileOp::Read)?)?;
    let virtual_dir = dir;
    let tx = get_io_tx_async(state);

    protocol::send_fs_with_resp_async(&tx, move |resp_tx| IOCmd::ListSavedFiles {
        dir: full_dir,
        prefix,
        virtual_dir,
        resp: IOCmdResp::Async(resp_tx),
    })
    .await
    .map_err(IOError::from)
}
