//! Uses Tokio's async UdpSocket, managed as a deno_core Resource.
//! The JS layer creates instances via `op_udp_bind`, and each socket
//! receives messages through an async polling loop (`op_udp_next_event`).
//!
//! ## Event model
//!
//! JS polls events via `op_udp_next_event` (async loop), matching
//! the TCP/WebSocket pattern. Send is fire-and-forget via `op_udp_send`.

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
use tokio::net::UdpSocket;
use tracing::debug;

use super::common::{BACKGROUND_THROTTLE, addr_family, resolve_first};

// -- Resource --

/// Internal state for a bound UDP socket.
pub struct UdpSocketResource {
    socket: AsyncRefCell<UdpSocket>,
    cancel: CancelHandle,
    local_addr: SocketAddr,
}

// Safety: the deno runtime is single-threaded; all access is via AsyncRefCell.
unsafe impl Send for UdpSocketResource {}
unsafe impl Sync for UdpSocketResource {}

impl Resource for UdpSocketResource {
    fn name(&self) -> Cow<'_, str> {
        "udpSocket".into()
    }

    fn close(self: Rc<Self>) {
        self.cancel.cancel();
    }
}

// -- Event types --

/// Events returned by `op_udp_next_event` to JS.
#[derive(Serialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum UdpEvent {
    /// Data received from a remote peer.
    #[serde(rename = "message")]
    Message {
        data: Vec<u8>,
        remote_address: String,
        remote_family: String,
        remote_port: u16,
        size: usize,
        local_address: String,
        local_family: String,
        local_port: u16,
    },
    /// An error occurred on the socket.
    #[serde(rename = "error")]
    Error { err_msg: String },
    /// The socket has been closed.
    #[serde(rename = "close")]
    #[allow(dead_code)]
    Close,
}

// -- op_udp_bind --

/// Bind result returned to JS.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UdpBindResult {
    pub rid: ResourceId,
    pub port: u16,
    pub address: String,
    pub family: String,
}

/// Bind a UDP socket to a local port (synchronous).
///
/// # Arguments
/// - `port`: Port to bind to (0 = system-assigned random port)
/// - `socket_type`: "udp4" or "udp6"
///
/// # Returns
/// A `UdpBindResult` with the resource ID and bound port.
#[op2]
#[serde]
pub fn op_udp_bind(
    state: Rc<RefCell<OpState>>,
    #[smi] port: u32,
    #[string] socket_type: String,
) -> Result<UdpBindResult, JsErrorBox> {
    let port = port as u16;
    let is_v6 = socket_type == "udp6";

    let bind_addr: SocketAddr = if is_v6 {
        format!("[::]:{}", port)
            .parse()
            .map_err(|e| JsErrorBox::generic(format!("bind:fail {}", e)))?
    } else {
        format!("0.0.0.0:{}", port)
            .parse()
            .map_err(|e| JsErrorBox::generic(format!("bind:fail {}", e)))?
    };

    debug!("UDP bind: {} (type={})", bind_addr, socket_type);

    let std_socket = std::net::UdpSocket::bind(bind_addr)
        .map_err(|e| JsErrorBox::generic(format!("bind:fail {}", e)))?;

    std_socket
        .set_nonblocking(true)
        .map_err(|e| JsErrorBox::generic(format!("bind:fail set_nonblocking: {}", e)))?;

    let socket = UdpSocket::from_std(std_socket)
        .map_err(|e| JsErrorBox::generic(format!("bind:fail from_std: {}", e)))?;

    let local_addr = socket
        .local_addr()
        .map_err(|e| JsErrorBox::generic(format!("bind:fail {}", e)))?;

    let resource = UdpSocketResource {
        socket: AsyncRefCell::new(socket),
        cancel: CancelHandle::default(),
        local_addr,
    };

    let rid = state.borrow_mut().resource_table.add(resource);

    debug!("UDP bound, rid={}, local={}", rid, local_addr);

    Ok(UdpBindResult {
        rid,
        port: local_addr.port(),
        address: local_addr.ip().to_string(),
        family: addr_family(&local_addr),
    })
}

// -- op_udp_connect --

/// Pre-connect to a remote address (for use with write).
/// Sets the default destination for the socket.
#[op2(async(lazy), fast)]
pub async fn op_udp_connect(
    state: Rc<RefCell<OpState>>,
    #[smi] rid: ResourceId,
    #[string] address: String,
    #[smi] port: u32,
) -> Result<(), JsErrorBox> {
    let port = port as u16;
    {
        let st = state.borrow();
        super::gate::enforce_host_from_state(
            &address,
            port,
            &st,
            super::gate::GateKind::UdpSocket,
        )?;
    }
    let resource = state
        .borrow()
        .resource_table
        .get::<UdpSocketResource>(rid)
        .map_err(|_| JsErrorBox::generic("UDPSocket not found"))?;

    let addr_str = format!("{}:{}", address, port);
    debug!("UDP connect: rid={}, target={}", rid, addr_str);
    let sock_addr = resolve_first(&addr_str)
        .await
        .map_err(|e| JsErrorBox::generic(format!("connect:fail {}", e)))?;

    let socket = RcRef::map(&resource, |r| &r.socket).borrow().await;
    socket
        .connect(sock_addr)
        .await
        .map_err(|e| JsErrorBox::generic(format!("connect:fail {}", e)))?;

    debug!("UDP connect: success, rid={}, addr={}", rid, sock_addr);
    Ok(())
}

// -- op_udp_send --

/// Send a UDP datagram to a specified address and port.
///
/// Accepts either a string or binary buffer, with optional offset/length
/// for binary data and optional broadcast flag.
#[op2(async(lazy))]
pub async fn op_udp_send(
    state: Rc<RefCell<OpState>>,
    #[smi] rid: ResourceId,
    #[string] address: String,
    #[smi] port: u32,
    #[string] data_str: Option<String>,
    #[buffer] data_buf: Option<JsBuffer>,
    #[smi] offset: u32,
    #[smi] length: u32,
    set_broadcast: bool,
) -> Result<(), JsErrorBox> {
    let port = port as u16;
    // Broadcast is refused outright: the destination address filter
    // already blocks 255.255.255.255 and every multicast/link-local
    // range, so a `set_broadcast` flag has no legitimate target and
    // would only serve to escape those checks via a misconfigured
    // kernel option. Game scripts that need fan-out must go through
    // server-mediated delivery.
    if set_broadcast {
        return Err(JsErrorBox::generic(
            "send:fail broadcast is not permitted by the runtime policy",
        ));
    }
    {
        let st = state.borrow();
        super::gate::enforce_host_from_state(
            &address,
            port,
            &st,
            super::gate::GateKind::UdpSocket,
        )?;
    }
    let resource = state
        .borrow()
        .resource_table
        .get::<UdpSocketResource>(rid)
        .map_err(|_| JsErrorBox::generic("UDPSocket not found"))?;

    let addr_str = format!("{}:{}", address, port);
    debug!("UDP send: rid={}, target={}", rid, addr_str);
    // `resolve_first` runs the shared `address_filter` on every DNS
    // answer, which now covers multicast, documentation, benchmarking
    // and future-reserved ranges in addition to the classic private /
    // link-local / loopback set. So a multicast destination (whether
    // supplied as hostname or IP literal) is rejected here before any
    // `send_to` hits the kernel.
    let sock_addr = resolve_first(&addr_str)
        .await
        .map_err(|e| JsErrorBox::generic(format!("send:fail resolve error: {}", e)))?;

    let socket = RcRef::map(&resource, |r| &r.socket).borrow().await;

    let bytes: Vec<u8> = if let Some(ref text) = data_str {
        text.as_bytes().to_vec()
    } else if let Some(ref buf) = data_buf {
        let off = offset as usize;
        let len = if length == 0 {
            buf.len().saturating_sub(off)
        } else {
            length as usize
        };
        let end = (off + len).min(buf.len());
        if off >= buf.len() {
            return Err(JsErrorBox::generic("send:fail offset out of bounds"));
        }
        buf[off..end].to_vec()
    } else {
        return Err(JsErrorBox::type_error("send:fail no data provided"));
    };

    debug!("UDP send: {} bytes to {}", bytes.len(), sock_addr);
    socket
        .send_to(&bytes, sock_addr)
        .await
        .map_err(|e| JsErrorBox::generic(format!("send:fail {}", e)))?;

    debug!("UDP send: success");
    Ok(())
}

// -- op_udp_set_ttl --

/// Set the IP_TTL socket option.
#[op2(fast)]
pub fn op_udp_set_ttl(
    state: &mut OpState,
    #[smi] rid: ResourceId,
    #[smi] ttl: u32,
) -> Result<(), JsErrorBox> {
    let resource = state
        .resource_table
        .get::<UdpSocketResource>(rid)
        .map_err(|_| JsErrorBox::generic("UDPSocket not found"))?;

    // UdpSocket::set_ttl is sync on the underlying std socket
    // We need to access it through the AsyncRefCell, but set_ttl is on the
    // outer UdpSocket type. Since we're in a sync op we use try_borrow.
    let socket = RcRef::map(&resource, |r| &r.socket);
    let guard = socket.try_borrow();
    match guard {
        Some(s) => {
            s.set_ttl(ttl)
                .map_err(|e| JsErrorBox::generic(format!("setTTL:fail {}", e)))?;
        }
        None => {
            return Err(JsErrorBox::generic("setTTL:fail socket busy"));
        }
    }
    Ok(())
}

// -- op_udp_next_event --

/// Poll for the next UDP event (blocking async).
///
/// Returns Message (with data + remote info) or Close/Error.
/// The JS layer calls this in a loop to receive datagrams.
#[op2(async(lazy), fast)]
#[serde]
pub async fn op_udp_next_event(
    state: Rc<RefCell<OpState>>,
    #[smi] rid: ResourceId,
) -> Result<UdpEvent, JsErrorBox> {
    let (resource, backgrounded) = {
        let st = state.borrow();
        let res = st
            .resource_table
            .get::<UdpSocketResource>(rid)
            .map_err(|_| JsErrorBox::generic("UDPSocket not found"))?;
        let bg = st.borrow::<HostOpState>().backgrounded.clone();
        (res, bg)
    };

    let cancel = RcRef::map(&resource, |r| &r.cancel);
    let local_addr = resource.local_addr;

    let event = async {
        // Throttle polling when the app is in the background to save
        // CPU and battery.
        if backgrounded.load(Ordering::Relaxed) {
            tokio::time::sleep(BACKGROUND_THROTTLE).await;
        }

        let socket = RcRef::map(&resource, |r| &r.socket).borrow().await;

        // 4096 bytes max per UDP message
        let mut buf = vec![0u8; 4096];

        match socket.recv_from(&mut buf).await {
            Ok((n, peer)) => {
                buf.truncate(n);
                Ok::<UdpEvent, JsErrorBox>(UdpEvent::Message {
                    size: n,
                    data: buf,
                    remote_address: peer.ip().to_string(),
                    remote_family: addr_family(&peer),
                    remote_port: peer.port(),
                    local_address: local_addr.ip().to_string(),
                    local_family: addr_family(&local_addr),
                    local_port: local_addr.port(),
                })
            }
            Err(e) => Ok(UdpEvent::Error {
                err_msg: e.to_string(),
            }),
        }
    };

    event.try_or_cancel(cancel).await
}

// -- op_udp_close --

/// Close the UDP socket, releasing the resource.
#[op2(fast)]
pub fn op_udp_close(state: &mut OpState, #[smi] rid: ResourceId) -> Result<(), JsErrorBox> {
    let resource = state
        .resource_table
        .take::<UdpSocketResource>(rid)
        .map_err(|_| JsErrorBox::generic("UDPSocket not found"))?;
    resource.close();
    Ok(())
}
