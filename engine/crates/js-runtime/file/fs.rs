use std::{cell::RefCell, rc::Rc, sync::Arc};

use deno_core::{JsBuffer, OpState, ToJsBuffer, op2, serde_json, v8};
use shared::{
    codec,
    error::{EngineError, ErrorCode},
    op_state::HostOpState,
    protocol::{
        self,
        io_cmd::{FileId, FileStat, IOCmd, IOCmdResp, OpenFlag, WriteMode},
    },
    vfs::{FileOp, VfsError, VirtualFS},
};
use tokio::sync::mpsc::UnboundedSender;

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

/// Get VFS from async op state.
#[inline]
fn get_vfs_async(state: &Rc<RefCell<OpState>>) -> Option<Arc<VirtualFS>> {
    state.borrow().borrow::<HostOpState>().vfs.clone()
}

/// Get VFS from sync op state.
#[inline]
fn get_vfs_sync(state: &OpState) -> Option<Arc<VirtualFS>> {
    state.borrow::<HostOpState>().vfs.clone()
}

/// Resolve a path using VFS.
///
/// All file paths MUST be virtual paths starting with /user, /cache, /code, or /tmp.
/// This ensures game isolation and prevents unauthorized file access.
#[inline]
fn resolve_path_vfs(
    vfs: Option<&VirtualFS>,
    path: &str,
    op: FileOp,
) -> Result<String, IOError> {
    let vfs = vfs.ok_or_else(|| ioerr("File system not initialized"))?;

    vfs.resolve(path, op)
        .map(|p| p.to_string_lossy().into_owned())
        .map_err(|e| match e {
            VfsError::PathNotAllowed => {
                ioerr(format!("Path not allowed: {}. Use /user, /cache, /code, or /tmp", path))
            }
            VfsError::PermissionDenied => {
                ioerr(format!("Permission denied: {}", path))
            }
            VfsError::PathTraversal => {
                ioerr(format!("Path traversal detected: {}", path))
            }
            VfsError::SymlinkEscape => {
                ioerr(format!("Symlink resolves outside sandbox: {}", path))
            }
            VfsError::SymlinkNotAllowed => {
                ioerr(format!("Symlinks not allowed in this directory: {}", path))
            }
            VfsError::InvalidPath => {
                ioerr(format!("Invalid path: {}", path))
            }
        })
}

#[inline]
fn parse_open_flag(flag: &str) -> Result<OpenFlag, IOError> {
    match flag {
        "r" => Ok(OpenFlag::Read),
        "r+" => Ok(OpenFlag::ReadWrite),
        "w" => Ok(OpenFlag::WriteTruncateCreate),
        "w+" => Ok(OpenFlag::ReadWriteTruncateCreate),
        "a" => Ok(OpenFlag::AppendCreate),
        "a+" => Ok(OpenFlag::ReadAppendCreate),
        _ => Err(ioerr(format!("Invalid open flag: {flag}"))),
    }
}

/// Convert OpenFlag to VFS FileOp for permission checking.
#[inline]
fn open_flag_to_vfs_op(flag: &OpenFlag) -> FileOp {
    match flag {
        OpenFlag::Read => FileOp::Read,
        OpenFlag::ReadWrite | OpenFlag::ReadWriteTruncateCreate | OpenFlag::ReadAppendCreate => {
            FileOp::Write // Write includes read
        }
        OpenFlag::WriteTruncateCreate | OpenFlag::AppendCreate => FileOp::Create,
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
    let vfs = get_vfs_async(&state);
    let full_path = resolve_path_vfs(vfs.as_deref(), &path, FileOp::Read)?;
    let tx = get_io_tx_async(state);

    let r = protocol::send_fs_with_resp_async(&tx, |resp_tx| IOCmd::Access {
        path: full_path,
        resp: IOCmdResp::Async(resp_tx),
    })
    .await;

    r.map(|(is_file, is_dir, _size)| is_file || is_dir)
        .map_err(IOError::from)
}

#[op2(fast)]
pub fn op_access_sync(state: &mut OpState, #[string] path: String) -> Result<bool, IOError> {
    let vfs = get_vfs_sync(state);
    let full_path = resolve_path_vfs(vfs.as_deref(), &path, FileOp::Read)?;
    let tx = get_io_tx_sync(state);

    let r = protocol::send_fs_with_resp_sync(tx, |resp_tx| IOCmd::Access {
        path: full_path,
        resp: IOCmdResp::Sync(resp_tx),
    });

    r.map(|(is_file, is_dir, _size)| is_file || is_dir)
        .map_err(IOError::from)
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
    let vfs = get_vfs_async(&state);
    let full_path = resolve_path_vfs(vfs.as_deref(), &path, FileOp::Write)?;
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
    let vfs = get_vfs_sync(state);
    let full_path = resolve_path_vfs(vfs.as_deref(), &path, FileOp::Write)?;
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
    let vfs = get_vfs_async(&state);
    let open_flag = parse_open_flag(&flag)?;
    let vfs_op = open_flag_to_vfs_op(&open_flag);
    let full_path = resolve_path_vfs(vfs.as_deref(), &path, vfs_op)?;
    let tx = get_io_tx_async(state);

    protocol::send_fs_with_resp_async(&tx, move |resp_tx| IOCmd::Open {
        path: full_path,
        flag: open_flag,
        resp: IOCmdResp::Async(resp_tx),
    })
    .await
    .map_err(IOError::from)
}

#[op2(fast)]
pub fn op_open_file_sync(
    state: &mut OpState,
    #[string] path: String,
    #[string] flag: String,
) -> Result<u32, IOError> {
    let vfs = get_vfs_sync(state);
    let open_flag = parse_open_flag(&flag)?;
    let vfs_op = open_flag_to_vfs_op(&open_flag);
    let full_path = resolve_path_vfs(vfs.as_deref(), &path, vfs_op)?;
    let tx = get_io_tx_sync(state);

    protocol::send_fs_with_resp_sync(tx, move |resp_tx| IOCmd::Open {
        path: full_path,
        flag: open_flag,
        resp: IOCmdResp::Sync(resp_tx),
    })
    .map_err(IOError::from)
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
    let vfs = get_vfs_async(&state);
    let src_full = resolve_path_vfs(vfs.as_deref(), &src_path, FileOp::Read)?;
    let dest_full = resolve_path_vfs(vfs.as_deref(), &dest_path, FileOp::Create)?;
    let tx = get_io_tx_async(state);

    protocol::send_fs_with_resp_async(&tx, move |resp_tx| IOCmd::Copy {
        src_path: src_full,
        dest_path: dest_full,
        resp: IOCmdResp::Async(resp_tx),
    })
    .await
    .map_err(IOError::from)
}

#[op2(fast)]
pub fn op_copy_file_sync(
    state: &mut OpState,
    #[string] src_path: String,
    #[string] dest_path: String,
) -> Result<(), IOError> {
    let vfs = get_vfs_sync(state);
    let src_full = resolve_path_vfs(vfs.as_deref(), &src_path, FileOp::Read)?;
    let dest_full = resolve_path_vfs(vfs.as_deref(), &dest_path, FileOp::Create)?;
    let tx = get_io_tx_sync(state);

    protocol::send_fs_with_resp_sync(tx, move |resp_tx| IOCmd::Copy {
        src_path: src_full,
        dest_path: dest_full,
        resp: IOCmdResp::Sync(resp_tx),
    })
    .map_err(IOError::from)
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
    let vfs = get_vfs_async(&state);
    let full_path = resolve_path_vfs(vfs.as_deref(), &dir_path, FileOp::Create)?;
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
    let vfs = get_vfs_sync(state);
    let full_path = resolve_path_vfs(vfs.as_deref(), &dir_path, FileOp::Create)?;
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
    let vfs = get_vfs_async(&state);
    let full_path = resolve_path_vfs(vfs.as_deref(), &dir_path, FileOp::Read)?;
    let tx = get_io_tx_async(state);

    protocol::send_fs_with_resp_async(&tx, move |resp_tx| IOCmd::Readdir {
        dir_path: full_path,
        resp: IOCmdResp::Async(resp_tx),
    })
    .await
    .map_err(IOError::from)
}

#[op2]
#[serde]
pub fn op_readdir_sync(
    state: &mut OpState,
    #[string] dir_path: String,
) -> Result<Vec<String>, IOError> {
    let vfs = get_vfs_sync(state);
    let full_path = resolve_path_vfs(vfs.as_deref(), &dir_path, FileOp::Read)?;
    let tx = get_io_tx_sync(state);

    protocol::send_fs_with_resp_sync(tx, move |resp_tx| IOCmd::Readdir {
        dir_path: full_path,
        resp: IOCmdResp::Sync(resp_tx),
    })
    .map_err(IOError::from)
}

//
// unlink / rename / rmdir - uses VFS with Delete/Write permissions
//
#[op2(async(lazy), fast)]
pub async fn op_unlink(
    state: Rc<RefCell<OpState>>,
    #[string] file_path: String,
) -> Result<(), IOError> {
    let vfs = get_vfs_async(&state);
    let full_path = resolve_path_vfs(vfs.as_deref(), &file_path, FileOp::Delete)?;
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
    let vfs = get_vfs_sync(state);
    let full_path = resolve_path_vfs(vfs.as_deref(), &file_path, FileOp::Delete)?;
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
    let vfs = get_vfs_async(&state);
    // Rename needs delete on source and create on destination
    let old_full = resolve_path_vfs(vfs.as_deref(), &old_path, FileOp::Delete)?;
    let new_full = resolve_path_vfs(vfs.as_deref(), &new_path, FileOp::Create)?;
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
    let vfs = get_vfs_sync(state);
    let old_full = resolve_path_vfs(vfs.as_deref(), &old_path, FileOp::Delete)?;
    let new_full = resolve_path_vfs(vfs.as_deref(), &new_path, FileOp::Create)?;
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
    let vfs = get_vfs_async(&state);
    let full_path = resolve_path_vfs(vfs.as_deref(), &dir_path, FileOp::Delete)?;
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
    let vfs = get_vfs_sync(state);
    let full_path = resolve_path_vfs(vfs.as_deref(), &dir_path, FileOp::Delete)?;
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
) -> Result<serde_json::Value, IOError> {
    let vfs = get_vfs_async(&state);
    let full_path = resolve_path_vfs(vfs.as_deref(), &path, FileOp::Read)?;
    let tx = get_io_tx_async(state);

    protocol::send_fs_with_resp_async(&tx, move |resp_tx| IOCmd::Stat {
        path: full_path,
        recursive,
        resp: IOCmdResp::Async(resp_tx),
    })
    .await
    .map_err(IOError::from)
}

#[op2]
#[serde]
pub fn op_stat_sync(
    state: &mut OpState,
    #[string] path: String,
    recursive: bool,
) -> Result<serde_json::Value, IOError> {
    let vfs = get_vfs_sync(state);
    let full_path = resolve_path_vfs(vfs.as_deref(), &path, FileOp::Read)?;
    let tx = get_io_tx_sync(state);

    protocol::send_fs_with_resp_sync(tx, move |resp_tx| IOCmd::Stat {
        path: full_path,
        recursive,
        resp: IOCmdResp::Sync(resp_tx),
    })
    .map_err(IOError::from)
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
    let vfs = get_vfs_async(&state);
    let full_path = resolve_path_vfs(vfs.as_deref(), &path, FileOp::Read)?;
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

#[op2]
#[serde]
pub fn op_read_file_sync(
    state: &mut OpState,
    #[string] path: String,
    #[bigint] position: Option<u64>,
    #[bigint] length: Option<u64>,
) -> Result<ToJsBuffer, IOError> {
    let vfs = get_vfs_sync(state);
    let full_path = resolve_path_vfs(vfs.as_deref(), &path, FileOp::Read)?;
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

// ============================ Unzip ============================

/// Default unzip operation using IOCmd::Unzip (Rust `zip` crate on IO thread).
///
/// Platforms can override this by providing their own `op_unzip_<platform>` op
/// and overriding `BaseFileManager.unzip()` in JS. For example, Android uses
/// `op_unzip_android` backed by `java.util.zip` via JNI.
#[op2(async(lazy), fast)]
pub async fn op_unzip(
    state: Rc<RefCell<OpState>>,
    #[string] zip_file_path: String,
    #[string] target_path: String,
) -> Result<(), IOError> {
    let (io_tx, vfs) = {
        let st = state.borrow();
        let host = st.borrow::<HostOpState>();
        (host.io_tx.clone(), host.vfs.clone())
    };

    let full_zip_path = resolve_path_vfs(vfs.as_deref(), &zip_file_path, FileOp::Read)?;
    let full_dest_dir = resolve_path_vfs(vfs.as_deref(), &target_path, FileOp::Write)?;

    protocol::send_fs_with_resp_async(&io_tx, move |resp_tx| IOCmd::Unzip {
        zip_path: full_zip_path,
        dest_dir: full_dest_dir,
        resp: IOCmdResp::Async(resp_tx),
    })
    .await
    .map(|_| ()) // Discard file count, JS doesn't need it
    .map_err(IOError::from)
}
