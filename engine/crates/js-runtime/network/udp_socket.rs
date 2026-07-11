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
use std::sync::Arc;
use std::sync::atomic::Ordering;

use deno_core::AsyncRefCell;
use deno_core::CancelHandle;
use deno_core::CancelTryFuture;
use deno_core::JsBuffer;
use deno_core::OpState;
use deno_core::RcRef;
use deno_core::Resource;
use deno_core::ResourceId;
use deno_core::ToJsBuffer;
use deno_core::op2;
use deno_error::JsErrorBox;
use serde::Serialize;
use shared::op_state::HostOpState;
use tokio::net::UdpSocket;
use tracing::debug;

use super::common::{
    AddrMeta, BACKGROUND_THROTTLE, ReceiveScratch, addr_family, checked_port, join_host_port,
    resolve_first,
};

// -- Resource --

/// Max single UDP recv (bytes). A datagram is delivered in one `recv_from` and
/// bytes past the buffer end are silently dropped (no truncation flag), so this
/// must hold the max UDP payload (65507 IPv4 / 65527 IPv6).
const UDP_RECV_CAPACITY: usize = 65536;

/// Receive-side state kept in its own `AsyncRefCell` so `op_udp_next_event`
/// serializes on it (no two concurrent recvs race the scratch) while the
/// `socket` stays shared for send/connect/set_ttl.
struct UdpReceiveState {
    scratch: ReceiveScratch,
    /// Bounded single-entry last-peer cache: the remote-address `Arc<str>` is
    /// reused only when the datagram's peer matches the cached one; a different
    /// peer rebuilds and replaces it, so `remoteAddress` always reflects the
    /// actual peer (never a stale value).
    last_peer: Option<(SocketAddr, Arc<str>)>,
}

/// Internal state for a bound UDP socket.
///
/// `socket` stays shared (immutable `borrow`) for `recv_from`/`send_to`/
/// `connect`/`set_ttl`; `rx` is a separate `AsyncRefCell` acquired only by
/// `next_event`. Lock order is `rx → socket`, so there is no deadlock.
pub struct UdpSocketResource {
    socket: AsyncRefCell<UdpSocket>,
    rx: AsyncRefCell<UdpReceiveState>,
    cancel: CancelHandle,
    local: AddrMeta,
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
        /// Raw bytes received (external Uint8Array backing, exact length).
        data: ToJsBuffer,
        remote_address: Arc<str>,
        remote_family: &'static str,
        remote_port: u16,
        size: usize,
        local_address: Arc<str>,
        local_family: &'static str,
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
    pub address: Arc<str>,
    pub family: &'static str,
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
    let port = checked_port(port)?;
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

    // Cache local address metadata once; events clone the `Arc<str>` per event.
    let local = AddrMeta::new(&local_addr);

    let resource = UdpSocketResource {
        socket: AsyncRefCell::new(socket),
        rx: AsyncRefCell::new(UdpReceiveState {
            scratch: ReceiveScratch::new(UDP_RECV_CAPACITY),
            last_peer: None,
        }),
        cancel: CancelHandle::default(),
        local: local.clone(),
    };

    let rid = state.borrow_mut().resource_table.add(resource);

    debug!("UDP bound, rid={}, local={}", rid, local_addr);

    Ok(UdpBindResult {
        rid,
        port: local.port,
        address: local.address,
        family: local.family,
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
    let port = checked_port(port)?;
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

    let addr_str = join_host_port(&address, port);
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
    let port = checked_port(port)?;
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

    let addr_str = join_host_port(&address, port);
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
        let (start, end) = udp_send_range(buf.len(), offset as usize, length as usize)
            .map_err(|e| JsErrorBox::generic(format!("send:fail {}", e)))?;
        buf[start..end].to_vec()
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

    let event = async {
        // Throttle polling when the app is in the background to save
        // CPU and battery.
        if backgrounded.load(Ordering::Relaxed) {
            tokio::time::sleep(BACKGROUND_THROTTLE).await;
        }

        // Lock order `rx → socket`: take the receive state exclusively (so two
        // concurrent `next_event` calls can't race the scratch), then the
        // shared socket borrow. `rx` is only ever acquired here, so this never
        // deadlocks with `send`/`connect`/`set_ttl` (which take only `socket`).
        let mut rx = RcRef::map(&resource, |r| &r.rx).borrow_mut().await;
        let rx = &mut *rx;

        // A single recv must hold a whole datagram: the kernel delivers a UDP
        // datagram in one recv and silently discards any bytes past the buffer
        // end. The scratch is sized to 64 KiB (>= max UDP payload) so no data
        // is lost, and is reused across recvs instead of reallocated per event.
        let recv = {
            let socket = RcRef::map(&resource, |r| &r.socket).borrow().await;
            socket.recv_from(rx.scratch.as_mut_slice()).await
        };

        match recv {
            Ok((n, peer)) => {
                // Copy exactly the filled prefix; datagram boundaries preserved.
                let data = ToJsBuffer::from(rx.scratch.copy_filled(n));
                let remote_address = udp_peer_address(&mut rx.last_peer, peer);
                Ok::<UdpEvent, JsErrorBox>(UdpEvent::Message {
                    size: n,
                    data,
                    remote_address,
                    remote_family: addr_family(&peer),
                    remote_port: peer.port(),
                    local_address: resource.local.address.clone(),
                    local_family: resource.local.family,
                    local_port: resource.local.port,
                })
            }
            Err(e) => Ok(UdpEvent::Error {
                err_msg: e.to_string(),
            }),
        }
    };

    event.try_or_cancel(cancel).await
}

/// Return the remote-address string for `peer`, reusing the cached `Arc<str>`
/// when the peer is unchanged and rebuilding (replacing) it when the peer
/// differs. Single entry and bounded, so `remoteAddress` always reflects the
/// actual datagram peer — never a stale value.
fn udp_peer_address(cache: &mut Option<(SocketAddr, Arc<str>)>, peer: SocketAddr) -> Arc<str> {
    if let Some((cached_peer, cached_addr)) = cache.as_ref() {
        if *cached_peer == peer {
            return cached_addr.clone();
        }
    }
    let addr: Arc<str> = Arc::from(peer.ip().to_string());
    *cache = Some((peer, addr.clone()));
    addr
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

/// Resolve the `[start, end)` slice of an outbound UDP payload for the
/// given `offset` / `length`.
///
/// Rejects out-of-bounds requests instead of silently clamping (the old
/// `(off + len).min(buf.len())` quietly sent fewer bytes than the caller
/// asked for, and `off + len` could overflow on 32-bit targets).
/// `length == 0` means "from `offset` to the end of the buffer".
fn udp_send_range(
    buf_len: usize,
    offset: usize,
    length: usize,
) -> Result<(usize, usize), &'static str> {
    if offset > buf_len {
        return Err("offset out of bounds");
    }
    let end = if length == 0 {
        buf_len
    } else {
        let end = offset.checked_add(length).ok_or("length overflow")?;
        if end > buf_len {
            return Err("offset+length out of bounds");
        }
        end
    };
    Ok((offset, end))
}

#[cfg(test)]
mod tests {
    use super::{udp_peer_address, udp_send_range};
    use std::net::SocketAddr;
    use std::sync::Arc;

    #[test]
    fn length_zero_means_offset_to_end() {
        assert_eq!(udp_send_range(10, 0, 0), Ok((0, 10)));
        assert_eq!(udp_send_range(10, 3, 0), Ok((3, 10)));
    }

    #[test]
    fn explicit_length_within_bounds() {
        assert_eq!(udp_send_range(10, 0, 10), Ok((0, 10)));
        assert_eq!(udp_send_range(10, 2, 5), Ok((2, 7)));
    }

    #[test]
    fn offset_at_end_sends_empty_slice() {
        assert_eq!(udp_send_range(10, 10, 0), Ok((10, 10)));
        assert_eq!(udp_send_range(0, 0, 0), Ok((0, 0)));
    }

    #[test]
    fn rejects_offset_past_buffer() {
        assert!(udp_send_range(10, 11, 0).is_err());
    }

    #[test]
    fn rejects_offset_plus_length_past_buffer_instead_of_clamping() {
        // Regression: this used to clamp to buf.len() and silently send
        // fewer bytes than requested.
        assert!(udp_send_range(10, 5, 10).is_err());
        assert!(udp_send_range(10, 0, 11).is_err());
    }

    #[test]
    fn rejects_overflowing_length_without_panicking() {
        assert_eq!(udp_send_range(10, 4, usize::MAX), Err("length overflow"));
    }

    #[test]
    fn udp_peer_cache_reuses_same_peer_and_rebuilds_on_change() {
        let a: SocketAddr = "1.2.3.4:5000".parse().unwrap();
        let b: SocketAddr = "9.8.7.6:6000".parse().unwrap();
        let mut cache: Option<(SocketAddr, Arc<str>)> = None;

        // First datagram from peer A: builds and caches.
        let first = udp_peer_address(&mut cache, a);
        assert_eq!(&*first, "1.2.3.4");

        // Same peer A: reuses the identical Arc allocation (refcount bump).
        let again = udp_peer_address(&mut cache, a);
        assert_eq!(&*again, "1.2.3.4");
        assert!(Arc::ptr_eq(&first, &again));

        // Different peer B: rebuilds, and the event reflects the ACTUAL peer,
        // never a stale cached value.
        let other = udp_peer_address(&mut cache, b);
        assert_eq!(&*other, "9.8.7.6");
        assert!(!Arc::ptr_eq(&first, &other));

        // Single-entry cache: A was evicted, so it rebuilds (new allocation).
        let a_third = udp_peer_address(&mut cache, a);
        assert_eq!(&*a_third, "1.2.3.4");
        assert!(!Arc::ptr_eq(&first, &a_third));
    }
}
