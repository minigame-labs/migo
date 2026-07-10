use std::borrow::Cow;
use std::cell::RefCell;
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
use futures::stream::{SplitSink, SplitStream};
use futures::{SinkExt, StreamExt};
use http::HeaderValue;
use http::header::HeaderName;
use serde::Serialize;
use shared::op_state::HostOpState;
use tokio_tungstenite::MaybeTlsStream;
use tokio_tungstenite::tungstenite::Message;
use tracing::debug;

use super::common::{BACKGROUND_THROTTLE, join_host_port};
use super::gate::{GateKind, enforce_from_state};

type WsStream = tokio_tungstenite::WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

/// Per-connection WebSocket limits. Kept module-private so callers go
/// through [`build_ws_config`] rather than ad-hoc configs.
pub const WS_MAX_MESSAGE_BYTES: usize = 8 * 1024 * 1024;
pub const WS_MAX_FRAME_BYTES: usize = 2 * 1024 * 1024;
pub const WS_WRITE_BUFFER_BYTES: usize = 128 * 1024;
pub const WS_MAX_WRITE_BUFFER_BYTES: usize = 8 * 1024 * 1024;

fn build_ws_config() -> tokio_tungstenite::tungstenite::protocol::WebSocketConfig {
    use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
    WebSocketConfig::default()
        .max_message_size(Some(WS_MAX_MESSAGE_BYTES))
        .max_frame_size(Some(WS_MAX_FRAME_BYTES))
        .write_buffer_size(WS_WRITE_BUFFER_BYTES)
        .max_write_buffer_size(WS_MAX_WRITE_BUFFER_BYTES)
}

/// Handshake-critical headers the client must control itself. If game JS
/// supplied these they'd corrupt the upgrade (or override the subprotocol
/// negotiated via the `protocols` argument), so they're dropped. Names
/// compare against `HeaderName::as_str()`, which is always lowercase.
fn is_reserved_ws_header(name: &HeaderName) -> bool {
    matches!(
        name.as_str(),
        "connection"
            | "upgrade"
            | "sec-websocket-key"
            | "sec-websocket-version"
            | "sec-websocket-accept"
            | "sec-websocket-protocol"
            | "sec-websocket-extensions"
    )
}

// ── Resource ──

pub struct WebSocketResource {
    tx: AsyncRefCell<SplitSink<WsStream, Message>>,
    rx: AsyncRefCell<SplitStream<WsStream>>,
    cancel: CancelHandle,
}

// Safety: the deno runtime is single-threaded; all access is via AsyncRefCell.
unsafe impl Send for WebSocketResource {}
unsafe impl Sync for WebSocketResource {}

impl Resource for WebSocketResource {
    fn name(&self) -> Cow<'_, str> {
        "webSocket".into()
    }

    fn close(self: Rc<Self>) {
        self.cancel.cancel();
    }
}

// ── Event types ──

#[derive(Serialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum WsEvent {
    #[serde(rename = "message")]
    Message {
        #[serde(skip_serializing_if = "Option::is_none")]
        data_str: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        data_bin: Option<Vec<u8>>,
        is_binary: bool,
    },
    #[serde(rename = "error")]
    Error { err_msg: String },
    #[serde(rename = "close")]
    Close { code: u16, reason: String },
}

// ── op_ws_create ──

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WsCreateResult {
    pub rid: ResourceId,
    pub protocol: String,
    pub extensions: String,
}

#[op2(async(lazy))]
#[serde]
pub async fn op_ws_create(
    state: Rc<RefCell<OpState>>,
    #[string] url: String,
    #[serde] protocols: Vec<String>,
    #[serde] headers: Vec<(String, String)>,
    #[smi] timeout_ms: Option<u32>,
) -> Result<WsCreateResult, JsErrorBox> {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;

    let connect_started = std::time::Instant::now();
    debug!("WebSocket connect: {}", url);

    // Single deadline for the whole connect (DNS + TCP + TLS/WS
    // handshake). Previously only the handshake was bounded (30s) while
    // DNS and TCP connect could hang until the OS SYN timeout, and the
    // JS-supplied `timeout` was dropped entirely.
    //
    // `Option` (not a bare u32) is deliberate: a stale V8 snapshot whose
    // baked JS still calls the old 3-arg op_ws_create passes no 4th arg.
    // deno_core coerces a missing `Option` smi to `None`, whereas a
    // missing *required* smi throws (Smi::from_v8 -> to_i32_option ->
    // None -> BadType). So `Option` lets an un-regenerated snapshot fall
    // back to the default timeout instead of breaking WebSocket connect.
    // None or 0 => 60s.
    let connect_timeout = std::time::Duration::from_millis(match timeout_ms {
        Some(ms) if ms > 0 => ms as u64,
        _ => 60_000,
    });
    let deadline = tokio::time::Instant::now() + connect_timeout;

    // Parse once and run the shared network-policy gate BEFORE we
    // do anything network-visible (DNS, TLS, TCP connect). The gate
    // covers scheme whitelist, IP-literal block, domain whitelist,
    // and HTTPS enforcement (wss required when enforce_https=true).
    let parsed = deno_core::url::Url::parse(&url)
        .map_err(|e| JsErrorBox::type_error(format!("Invalid WebSocket URL: {}", e)))?;
    {
        let st = state.borrow();
        enforce_from_state(&parsed, &st, GateKind::WebSocket)?;
    }

    // Build request (post-gate; `into_client_request` only re-validates
    // the URL shape, not the policy).
    let mut request = url
        .into_client_request()
        .map_err(|e| JsErrorBox::generic(format!("Invalid WebSocket URL: {}", e)))?;
    let scheme = request.uri().scheme_str().unwrap_or("");

    // SSRF prevention: resolve DNS, check ALL addresses, then connect
    // to a verified address directly.  This eliminates the double-
    // resolution TOCTOU window — we connect the TcpStream ourselves
    // and hand it to the WebSocket handshake layer.
    let connect_addrs: Vec<std::net::SocketAddr> = if let Some(host) = request.uri().host() {
        let port = request
            .uri()
            .port_u16()
            .unwrap_or(if scheme == "wss" { 443 } else { 80 });
        let addr_str = join_host_port(host, port);
        let addrs: Vec<std::net::SocketAddr> =
            tokio::time::timeout_at(deadline, tokio::net::lookup_host(&addr_str))
                .await
                .map_err(|_| JsErrorBox::generic("WebSocket DNS resolve timeout"))?
                .map_err(|e| JsErrorBox::generic(format!("WebSocket DNS resolve failed: {}", e)))?
                .collect();
        if addrs.is_empty() {
            return Err(JsErrorBox::generic(
                "WebSocket DNS resolve returned no addresses",
            ));
        }
        for addr in &addrs {
            if super::address_filter::is_blocked_address(addr) {
                return Err(JsErrorBox::generic(format!(
                    "WebSocket connection to {} is not allowed (private/loopback address)",
                    addr.ip()
                )));
            }
        }
        addrs
    } else {
        return Err(JsErrorBox::generic("WebSocket URL has no host"));
    };

    // Add custom headers — same filtering as fetch so a game can't
    // inject Host (vhost/routing bypass), proxy/forwarding headers
    // (SSRF amplification, credential leak) or the handshake-critical
    // WebSocket headers (which would corrupt the upgrade). Blocked
    // headers are dropped silently, matching fetch's behaviour.
    let req_headers = request.headers_mut();
    for (key, value) in &headers {
        if let (Ok(name), Ok(val)) = (
            HeaderName::from_bytes(key.as_bytes()),
            HeaderValue::from_str(value),
        ) {
            if name.as_str() == "host"
                || crate::network::fetch::is_blocked_header(&name)
                || is_reserved_ws_header(&name)
            {
                continue;
            }
            req_headers.insert(name, val);
        }
    }

    // Add subprotocols
    if !protocols.is_empty() {
        let protocol_header = protocols.join(", ");
        if let Ok(val) = HeaderValue::from_str(&protocol_header) {
            req_headers.insert("Sec-WebSocket-Protocol", val);
        }
    }

    // Connect TCP, trying each verified address in turn within the
    // shared deadline. A dual-stack / multi-A host may list a dead
    // address first (e.g. an unreachable IPv6); connecting only to
    // addrs[0] would fail even when a later address works.
    let mut tcp_stream = None;
    let mut last_err = String::from("no address");
    for addr in &connect_addrs {
        match tokio::time::timeout_at(deadline, tokio::net::TcpStream::connect(*addr)).await {
            Ok(Ok(s)) => {
                tcp_stream = Some(s);
                break;
            }
            Ok(Err(e)) => last_err = e.to_string(),
            Err(_) => {
                // Deadline hit: stop trying further addresses.
                last_err = "timeout".to_string();
                break;
            }
        }
    }
    let tcp_stream = tcp_stream
        .ok_or_else(|| JsErrorBox::generic(format!("WebSocket TCP connect failed: {}", last_err)))?;
    let _ = tcp_stream.set_nodelay(true);

    // Explicit WebSocket limits: tungstenite's defaults are 64 MiB
    // message / 16 MiB frame, which are tuned for desktop servers. A
    // mobile game runtime cannot reserve that much per connection, so
    // we cap far lower. Apps that really need large frames should
    // fragment in-app rather than ship a multi-megabyte blob.
    let ws_cfg = build_ws_config();

    // Handshake over the pre-connected stream.
    // client_async_tls_with_config handles TLS upgrade for wss:// using
    // the hostname from the request URI for SNI — no second DNS lookup.
    let handshake_fut =
        tokio_tungstenite::client_async_tls_with_config(request, tcp_stream, Some(ws_cfg), None);
    let (ws_stream, response) = tokio::time::timeout_at(deadline, handshake_fut)
        .await
        .map_err(|_| JsErrorBox::generic("WebSocket handshake timeout"))?
        .map_err(|e| JsErrorBox::generic(format!("WebSocket handshake failed: {}", e)))?;

    let protocol = response
        .headers()
        .get("Sec-WebSocket-Protocol")
        .and_then(|v: &HeaderValue| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    let extensions = response
        .headers()
        .get("Sec-WebSocket-Extensions")
        .and_then(|v: &HeaderValue| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    // Split into read/write halves
    let (tx, rx) = futures::StreamExt::split(ws_stream);

    let resource = WebSocketResource {
        tx: AsyncRefCell::new(tx),
        rx: AsyncRefCell::new(rx),
        cancel: CancelHandle::default(),
    };

    let rid = state.borrow_mut().resource_table.add(resource);

    shared::stats::io_metrics_global()
        .record_op(shared::stats::OpClass::WsConnect, connect_started.elapsed());

    debug!("WebSocket connected, rid={}", rid);

    Ok(WsCreateResult {
        rid,
        protocol,
        extensions,
    })
}

// ── op_ws_next_event ──

#[op2(async(lazy), fast)]
#[serde]
pub async fn op_ws_next_event(
    state: Rc<RefCell<OpState>>,
    #[smi] rid: ResourceId,
) -> Result<WsEvent, JsErrorBox> {
    let (resource, backgrounded) = {
        let st = state.borrow();
        let res = st
            .resource_table
            .get::<WebSocketResource>(rid)
            .map_err(|_| JsErrorBox::generic("WebSocket not found"))?;
        let bg = st.borrow::<HostOpState>().backgrounded.clone();
        (res, bg)
    };

    let cancel = RcRef::map(&resource, |r| &r.cancel);

    let event = async {
        let mut rx = RcRef::map(&resource, |r| &r.rx).borrow_mut().await;
        loop {
            // Throttle polling when the app is in the background to save
            // CPU and battery.  The delay is inserted *before* the read so
            // the socket stays connected but data delivery is deferred.
            if backgrounded.load(Ordering::Relaxed) {
                tokio::time::sleep(BACKGROUND_THROTTLE).await;
            }

            match rx.next().await {
                Some(Ok(Message::Text(text))) => {
                    return Ok::<WsEvent, JsErrorBox>(WsEvent::Message {
                        data_str: Some(text.to_string()),
                        data_bin: None,
                        is_binary: false,
                    });
                }
                Some(Ok(Message::Binary(data))) => {
                    return Ok(WsEvent::Message {
                        data_str: None,
                        data_bin: Some(data.to_vec()),
                        is_binary: true,
                    });
                }
                Some(Ok(Message::Ping(_))) | Some(Ok(Message::Pong(_))) => {
                    continue;
                }
                Some(Ok(Message::Close(frame))) => {
                    let (code, reason) = frame
                        .map(|f| (f.code.into(), f.reason.to_string()))
                        .unwrap_or((1005, String::new()));
                    return Ok(WsEvent::Close { code, reason });
                }
                Some(Ok(Message::Frame(_))) => {
                    continue;
                }
                Some(Err(e)) => {
                    return Ok(WsEvent::Error {
                        err_msg: e.to_string(),
                    });
                }
                None => {
                    return Ok(WsEvent::Close {
                        code: 1006,
                        reason: String::new(),
                    });
                }
            }
        }
    };

    let result: Result<WsEvent, JsErrorBox> = event.try_or_cancel(cancel).await;
    result
}

// ── op_ws_send ──

#[op2(async(lazy))]
pub async fn op_ws_send(
    state: Rc<RefCell<OpState>>,
    #[smi] rid: ResourceId,
    #[string] data_str: Option<String>,
    #[buffer] data_buf: Option<JsBuffer>,
) -> Result<(), JsErrorBox> {
    let resource = state
        .borrow()
        .resource_table
        .get::<WebSocketResource>(rid)
        .map_err(|_| JsErrorBox::generic("WebSocket not found"))?;

    let message = if let Some(text) = data_str {
        Message::Text(text.into())
    } else if let Some(buf) = data_buf {
        Message::Binary(buf.to_vec().into())
    } else {
        return Err(JsErrorBox::type_error("No data provided"));
    };

    let mut tx = RcRef::map(&resource, |r| &r.tx).borrow_mut().await;
    tokio::time::timeout(std::time::Duration::from_secs(10), tx.send(message))
        .await
        .map_err(|_| JsErrorBox::generic("WebSocket send timeout"))?
        .map_err(|e| JsErrorBox::generic(format!("WebSocket send failed: {}", e)))
}

// ── op_ws_close ──

#[op2(async(lazy), fast)]
pub async fn op_ws_close(
    state: Rc<RefCell<OpState>>,
    #[smi] rid: ResourceId,
    #[smi] code: u16,
    #[string] reason: String,
) -> Result<(), JsErrorBox> {
    let resource = state
        .borrow()
        .resource_table
        .get::<WebSocketResource>(rid)
        .map_err(|_| JsErrorBox::generic("WebSocket not found"))?;

    let close_frame = tokio_tungstenite::tungstenite::protocol::CloseFrame {
        code: code.into(),
        reason: reason.into(),
    };

    let mut tx = RcRef::map(&resource, |r| &r.tx).borrow_mut().await;
    tx.send(Message::Close(Some(close_frame)))
        .await
        .map_err(|e| JsErrorBox::generic(format!("WebSocket close failed: {}", e)))
}
