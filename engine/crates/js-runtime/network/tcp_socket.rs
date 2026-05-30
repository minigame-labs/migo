//! Uses Tokio's async TcpStream, managed as a deno_core Resource.
//! The JS layer creates instances, and each socket has its own read/write
//! halves protected by AsyncRefCell for single-threaded V8 access.
//!
//! ## Event model
//!
//! JS polls events via `op_tcp_next_event` (async loop), similar to WebSocket.
//! Connect, message, error, and close events are returned as tagged enum values.

use std::borrow::Cow;
use std::cell::RefCell;
use std::net::SocketAddr;
use std::rc::Rc;
use std::sync::atomic::Ordering;

use deno_core::AsyncRefCell;
use deno_core::CancelHandle;
use deno_core::CancelTryFuture;
use deno_core::JsBuffer;
use deno_core::OpState;
use deno_core::RcRef;
use deno_core::Resource;
use deno_core::ResourceId;
use deno_core::op2;
use deno_error::JsErrorBox;
use serde::Serialize;
use shared::op_state::HostOpState;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tracing::debug;

use super::common::{BACKGROUND_THROTTLE, addr_family, resolve_first};

// ── Resource ──

/// Internal state for a connected TCP socket.
///
/// Read and write halves are split via `tokio::io::split()` and wrapped
/// in `AsyncRefCell` to satisfy deno_core's single-threaded async model.
pub struct TcpSocketResource {
    reader: AsyncRefCell<tokio::io::ReadHalf<TcpStream>>,
    writer: AsyncRefCell<tokio::io::WriteHalf<TcpStream>>,
    cancel: CancelHandle,
    local_addr: SocketAddr,
    remote_addr: SocketAddr,
}

// Safety: the deno runtime is single-threaded; all access is via AsyncRefCell.
unsafe impl Send for TcpSocketResource {}
unsafe impl Sync for TcpSocketResource {}

impl Resource for TcpSocketResource {
    fn name(&self) -> Cow<'_, str> {
        "tcpSocket".into()
    }

    fn close(self: Rc<Self>) {
        self.cancel.cancel();
    }
}

// ── Event types ──

/// Events returned by `op_tcp_next_event` to JS.
///
/// The JS polling loop maps these to TCPSocket event callbacks.
#[derive(Serialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum TcpEvent {
    /// Data received from the remote end.
    #[serde(rename = "message")]
    Message {
        /// Raw bytes received.
        data: Vec<u8>,
        /// Remote address info.
        remote_address: String,
        remote_family: String,
        remote_port: u16,
        /// Local address info.
        local_address: String,
        local_family: String,
        local_port: u16,
    },
    /// An error occurred on the socket.
    #[serde(rename = "error")]
    Error { err_msg: String },
    /// The socket has been closed.
    #[serde(rename = "close")]
    Close,
}

// ── op_tcp_connect ──

/// Result of a successful TCP connect operation.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TcpConnectResult {
    pub rid: ResourceId,
    pub remote_address: String,
    pub remote_family: String,
    pub remote_port: u16,
    pub local_address: String,
    pub local_family: String,
    pub local_port: u16,
}

/// Connect to a TCP endpoint.
///
/// # Arguments
/// - `address`: IP or hostname to connect to
/// - `port`: Port number
/// - `timeout_secs`: Connection timeout in seconds (0 = default 2s)
///
/// # Returns
/// A `TcpConnectResult` with the resource ID and address info.
#[op2(async(lazy), fast)]
#[serde]
pub async fn op_tcp_connect(
    state: Rc<RefCell<OpState>>,
    #[string] address: String,
    #[smi] port: u32,
    #[smi] timeout_secs: u32,
) -> Result<TcpConnectResult, JsErrorBox> {
    let port = port as u16;
    let timeout = if timeout_secs == 0 {
        2
    } else {
        timeout_secs.min(300)
    };

    debug!("TCP connect: {}:{} (timeout={}s)", address, port, timeout);

    // Shared network-policy gate: raw TCP must obey the same domain
    // whitelist / IP-literal block as `fetch` and `WebSocket`. Without
    // this, a game that can't reach `evil.example` via `fetch()` could
    // still reach it via `createTCPSocket().connect(...)`.
    {
        let st = state.borrow();
        super::gate::enforce_host_from_state(
            &address,
            port,
            &st,
            super::gate::GateKind::TcpSocket,
        )?;
    }

    // Resolve address — supports both IP and hostname.
    // tokio::net::lookup_host handles DNS resolution asynchronously.
    let addr_str = format!("{}:{}", address, port);
    let sock_addr = tokio::time::timeout(
        std::time::Duration::from_secs(timeout as u64),
        resolve_first(&addr_str),
    )
    .await
    .map_err(|_| JsErrorBox::generic(format!("connect:fail timeout after {}s", timeout)))?
    .map_err(|e| JsErrorBox::generic(format!("connect:fail resolve error: {}", e)))?;

    // Connect with timeout.
    let stream = tokio::time::timeout(
        std::time::Duration::from_secs(timeout as u64),
        TcpStream::connect(sock_addr),
    )
    .await
    .map_err(|_| JsErrorBox::generic(format!("connect:fail timeout after {}s", timeout)))?
    .map_err(|e| JsErrorBox::generic(format!("connect:fail {}", e)))?;

    // Enable TCP_NODELAY for lower latency (common for game sockets).
    let _ = stream.set_nodelay(true);

    let local_addr = stream
        .local_addr()
        .unwrap_or_else(|_| SocketAddr::from(([0, 0, 0, 0], 0)));
    let remote_addr = stream.peer_addr().unwrap_or(sock_addr);

    let (reader, writer) = tokio::io::split(stream);

    let resource = TcpSocketResource {
        reader: AsyncRefCell::new(reader),
        writer: AsyncRefCell::new(writer),
        cancel: CancelHandle::default(),
        local_addr,
        remote_addr,
    };

    let rid = state.borrow_mut().resource_table.add(resource);

    debug!(
        "TCP connected, rid={}, local={}, remote={}",
        rid, local_addr, remote_addr
    );

    Ok(TcpConnectResult {
        rid,
        remote_address: remote_addr.ip().to_string(),
        remote_family: addr_family(&remote_addr),
        remote_port: remote_addr.port(),
        local_address: local_addr.ip().to_string(),
        local_family: addr_family(&local_addr),
        local_port: local_addr.port(),
    })
}

// ── op_tcp_next_event ──

/// Poll for the next TCP event (blocking async).
///
/// Returns one of: Message (with data), Error, or Close.
/// The JS layer calls this in a loop to receive data.
#[op2(async(lazy), fast)]
#[serde]
pub async fn op_tcp_next_event(
    state: Rc<RefCell<OpState>>,
    #[smi] rid: ResourceId,
) -> Result<TcpEvent, JsErrorBox> {
    let (resource, backgrounded) = {
        let st = state.borrow();
        let res = st
            .resource_table
            .get::<TcpSocketResource>(rid)
            .map_err(|_| JsErrorBox::generic("TCPSocket not found"))?;
        let bg = st.borrow::<HostOpState>().backgrounded.clone();
        (res, bg)
    };

    let cancel = RcRef::map(&resource, |r| &r.cancel);
    let local_addr = resource.local_addr;
    let remote_addr = resource.remote_addr;

    let event = async {
        // Throttle polling when the app is in the background to save
        // CPU and battery.
        if backgrounded.load(Ordering::Relaxed) {
            tokio::time::sleep(BACKGROUND_THROTTLE).await;
        }

        let mut reader = RcRef::map(&resource, |r| &r.reader).borrow_mut().await;

        // 64 KB read buffer — balances memory usage vs syscall overhead.
        let mut buf = vec![0u8; 65536];

        match reader.read(&mut buf).await {
            Ok(0) => Ok::<TcpEvent, JsErrorBox>(TcpEvent::Close),
            Ok(n) => {
                buf.truncate(n);
                Ok(TcpEvent::Message {
                    data: buf,
                    remote_address: remote_addr.ip().to_string(),
                    remote_family: addr_family(&remote_addr),
                    remote_port: remote_addr.port(),
                    local_address: local_addr.ip().to_string(),
                    local_family: addr_family(&local_addr),
                    local_port: local_addr.port(),
                })
            }
            Err(e) => Ok(TcpEvent::Error {
                err_msg: e.to_string(),
            }),
        }
    };

    event.try_or_cancel(cancel).await
}

// ── op_tcp_write ──

/// Write data to the TCP socket.
///
/// Accepts either a string or binary buffer.
#[op2(async(lazy))]
pub async fn op_tcp_write(
    state: Rc<RefCell<OpState>>,
    #[smi] rid: ResourceId,
    #[string] data_str: Option<String>,
    #[buffer] data_buf: Option<JsBuffer>,
) -> Result<(), JsErrorBox> {
    let resource = state
        .borrow()
        .resource_table
        .get::<TcpSocketResource>(rid)
        .map_err(|_| JsErrorBox::generic("TCPSocket not found"))?;

    let bytes: &[u8] = if let Some(ref text) = data_str {
        text.as_bytes()
    } else if let Some(ref buf) = data_buf {
        buf
    } else {
        return Err(JsErrorBox::type_error("write:fail no data provided"));
    };

    let mut writer = RcRef::map(&resource, |r| &r.writer).borrow_mut().await;
    writer
        .write_all(bytes)
        .await
        .map_err(|e| JsErrorBox::generic(format!("write:fail {}", e)))
}

// ── op_tcp_close ──

/// Close the TCP socket, releasing the resource.
#[op2(fast)]
pub fn op_tcp_close(state: &mut OpState, #[smi] rid: ResourceId) -> Result<(), JsErrorBox> {
    // Taking the resource from the table and dropping it triggers
    // Resource::close(), which cancels the CancelHandle and drops
    // reader/writer halves, causing the underlying TcpStream to shut down.
    let resource = state
        .resource_table
        .take::<TcpSocketResource>(rid)
        .map_err(|_| JsErrorBox::generic("TCPSocket not found"))?;
    resource.close();
    Ok(())
}
