use deno_core::{JsBuffer, OpState, op2, serde_json, v8};
use shared::{
    codec,
    error::{EngineError, ErrorCode},
    op_state::HostOpState,
    protocol::{
        self,
        io_cmd::{FileId, FileStat, IOCmd, IOCmdResp, OpenFlag, WriteMode},
    },
};
use std::{cell::RefCell, rc::Rc};
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

#[inline]
fn get_io_tx_async(state: Rc<RefCell<OpState>>) -> UnboundedSender<IOCmd> {
    let st = state.borrow();
    st.borrow::<HostOpState>().io_tx.clone()
}

#[inline]
fn get_io_tx_sync(state: &mut OpState) -> &UnboundedSender<IOCmd> {
    &state.borrow::<HostOpState>().io_tx
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
// Access
//
#[op2(async)]
pub async fn op_access(
    state: Rc<RefCell<OpState>>,
    #[string] path: String,
) -> Result<bool, IOError> {
    let tx = get_io_tx_async(state);

    let r = protocol::send_fs_with_resp_async(&tx, |resp_tx| IOCmd::Access {
        path,
        resp: IOCmdResp::Async(resp_tx),
    })
    .await;

    r.map(|(is_file, is_dir, _size)| is_file || is_dir)
        .map_err(IOError::from)
}

#[op2(fast)]
pub fn op_access_sync(state: &mut OpState, #[string] path: String) -> Result<bool, IOError> {
    let tx = get_io_tx_sync(state);

    let r = protocol::send_fs_with_resp_sync(tx, |resp_tx| IOCmd::Access {
        path,
        resp: IOCmdResp::Sync(resp_tx),
    });

    r.map(|(is_file, is_dir, _size)| is_file || is_dir)
        .map_err(IOError::from)
}

//
// Write / append (path)
//
#[op2(async)]
pub async fn op_write_or_append_file(
    state: Rc<RefCell<OpState>>,
    #[string] path: String,
    #[buffer] data_buf: Option<JsBuffer>,
    #[string] data_str: Option<String>,
    #[string] encoding: Option<String>,
    append: bool,
) -> Result<bool, IOError> {
    let tx = get_io_tx_async(state);
    let mode = mode_from_append(append);
    let (buf_opt, data_opt) = prepare_data(data_buf, data_str, encoding)?;

    if let Some((store, range)) = buf_opt {
        let r = protocol::send_fs_with_resp_async(&tx, move |resp_tx| IOCmd::WriteShared {
            path,
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
            path,
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
    let tx = get_io_tx_sync(state);
    let mode = mode_from_append(append);
    let (buf_opt, data_opt) = prepare_data(data_buf, data_str, encoding)?;

    if let Some((store, range)) = buf_opt {
        let r = protocol::send_fs_with_resp_sync(tx, move |resp_tx| IOCmd::WriteShared {
            path,
            store,
            range,
            mode,
            resp: IOCmdResp::Sync(resp_tx),
        });
        return r.map_err(IOError::from);
    }

    if let Some(data) = data_opt {
        let r = protocol::send_fs_with_resp_sync(tx, move |resp_tx| IOCmd::Write {
            path,
            data,
            mode,
            resp: IOCmdResp::Sync(resp_tx),
        });
        return r.map_err(IOError::from);
    }

    Err(ioerr("No data provided"))
}

//
// Open / close
//
#[op2(async)]
pub async fn op_open_file(
    state: Rc<RefCell<OpState>>,
    #[string] path: String,
    #[string] flag: String,
) -> Result<u32, IOError> {
    let tx = get_io_tx_async(state);
    let flag = parse_open_flag(&flag)?;

    protocol::send_fs_with_resp_async(&tx, move |resp_tx| IOCmd::Open {
        path,
        flag,
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
    let tx = get_io_tx_sync(state);
    let flag = parse_open_flag(&flag)?;

    protocol::send_fs_with_resp_sync(tx, move |resp_tx| IOCmd::Open {
        path,
        flag,
        resp: IOCmdResp::Sync(resp_tx),
    })
    .map_err(IOError::from)
}

#[op2(async)]
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
// Copy
//
#[op2(async)]
pub async fn op_copy_file(
    state: Rc<RefCell<OpState>>,
    #[string] src_path: String,
    #[string] dest_path: String,
) -> Result<(), IOError> {
    let tx = get_io_tx_async(state);

    protocol::send_fs_with_resp_async(&tx, move |resp_tx| IOCmd::Copy {
        src_path,
        dest_path,
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
    let tx = get_io_tx_sync(state);

    protocol::send_fs_with_resp_sync(tx, move |resp_tx| IOCmd::Copy {
        src_path,
        dest_path,
        resp: IOCmdResp::Sync(resp_tx),
    })
    .map_err(IOError::from)
}

//
// fstat / ftruncate
//
#[op2(async)]
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

#[op2(async)]
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
// mkdir / readdir
//
#[op2(async)]
pub async fn op_mkdir(
    state: Rc<RefCell<OpState>>,
    #[string] dir_path: String,
    recursive: bool,
) -> Result<(), IOError> {
    let tx = get_io_tx_async(state);

    protocol::send_fs_with_resp_async(&tx, move |resp_tx| IOCmd::Mkdir {
        dir_path,
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
    let tx = get_io_tx_sync(state);

    protocol::send_fs_with_resp_sync(tx, move |resp_tx| IOCmd::Mkdir {
        dir_path,
        recursive,
        resp: IOCmdResp::Sync(resp_tx),
    })
    .map_err(IOError::from)
}

#[op2(async)]
#[serde]
pub async fn op_readdir(
    state: Rc<RefCell<OpState>>,
    #[string] dir_path: String,
) -> Result<Vec<String>, IOError> {
    let tx = get_io_tx_async(state);

    protocol::send_fs_with_resp_async(&tx, move |resp_tx| IOCmd::Readdir {
        dir_path,
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
    let tx = get_io_tx_sync(state);

    protocol::send_fs_with_resp_sync(tx, move |resp_tx| IOCmd::Readdir {
        dir_path,
        resp: IOCmdResp::Sync(resp_tx),
    })
    .map_err(IOError::from)
}

//
// unlink / rename / rmdir
//
#[op2(async)]
pub async fn op_unlink(
    state: Rc<RefCell<OpState>>,
    #[string] file_path: String,
) -> Result<(), IOError> {
    let tx = get_io_tx_async(state);

    protocol::send_fs_with_resp_async(&tx, move |resp_tx| IOCmd::Unlink {
        file_path,
        resp: IOCmdResp::Async(resp_tx),
    })
    .await
    .map_err(IOError::from)
}

#[op2(fast)]
pub fn op_unlink_sync(state: &mut OpState, #[string] file_path: String) -> Result<(), IOError> {
    let tx = get_io_tx_sync(state);

    protocol::send_fs_with_resp_sync(tx, move |resp_tx| IOCmd::Unlink {
        file_path,
        resp: IOCmdResp::Sync(resp_tx),
    })
    .map_err(IOError::from)
}

#[op2(async)]
pub async fn op_rename(
    state: Rc<RefCell<OpState>>,
    #[string] old_path: String,
    #[string] new_path: String,
) -> Result<(), IOError> {
    let tx = get_io_tx_async(state);

    protocol::send_fs_with_resp_async(&tx, move |resp_tx| IOCmd::Rename {
        old_path,
        new_path,
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
    let tx = get_io_tx_sync(state);

    protocol::send_fs_with_resp_sync(tx, move |resp_tx| IOCmd::Rename {
        old_path,
        new_path,
        resp: IOCmdResp::Sync(resp_tx),
    })
    .map_err(IOError::from)
}

#[op2(async)]
pub async fn op_rmdir(
    state: Rc<RefCell<OpState>>,
    #[string] dir_path: String,
    recursive: bool,
) -> Result<(), IOError> {
    let tx = get_io_tx_async(state);

    protocol::send_fs_with_resp_async(&tx, move |resp_tx| IOCmd::Rmdir {
        dir_path,
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
    let tx = get_io_tx_sync(state);

    protocol::send_fs_with_resp_sync(tx, move |resp_tx| IOCmd::Rmdir {
        dir_path,
        recursive,
        resp: IOCmdResp::Sync(resp_tx),
    })
    .map_err(IOError::from)
}

//
// stat
//
#[op2(async)]
#[serde]
pub async fn op_stat(
    state: Rc<RefCell<OpState>>,
    #[string] path: String,
    recursive: bool,
) -> Result<serde_json::Value, IOError> {
    let tx = get_io_tx_async(state);

    protocol::send_fs_with_resp_async(&tx, move |resp_tx| IOCmd::Stat {
        path,
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
    let tx = get_io_tx_sync(state);

    protocol::send_fs_with_resp_sync(tx, move |resp_tx| IOCmd::Stat {
        path,
        recursive,
        resp: IOCmdResp::Sync(resp_tx),
    })
    .map_err(IOError::from)
}

//
// write(fd)
//
#[op2(async)]
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
