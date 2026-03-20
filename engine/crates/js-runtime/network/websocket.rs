use std::borrow::Cow;
use std::cell::RefCell;
use std::rc::Rc;

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
use tokio_tungstenite::MaybeTlsStream;
use tokio_tungstenite::tungstenite::Message;
use tracing::debug;

type WsStream = tokio_tungstenite::WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

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
) -> Result<WsCreateResult, JsErrorBox> {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;

    debug!("WebSocket connect: {}", url);

    // Build request
    let mut request = url
        .into_client_request()
        .map_err(|e| JsErrorBox::generic(format!("Invalid WebSocket URL: {}", e)))?;

    // Validate scheme
    let scheme = request.uri().scheme_str().unwrap_or("");
    if scheme != "ws" && scheme != "wss" {
        return Err(JsErrorBox::type_error("URL scheme must be ws:// or wss://"));
    }

    // SSRF prevention: resolve DNS, check ALL addresses, then connect
    // to the verified address directly.  This eliminates the double-
    // resolution TOCTOU window — we connect the TcpStream ourselves
    // and hand it to the WebSocket handshake layer.
    let connect_addr = if let Some(host) = request.uri().host() {
        let port = request.uri().port_u16().unwrap_or(if scheme == "wss" { 443 } else { 80 });
        let addr_str = format!("{}:{}", host, port);
        let addrs: Vec<std::net::SocketAddr> = tokio::net::lookup_host(&addr_str)
            .await
            .map_err(|e| JsErrorBox::generic(format!("WebSocket DNS resolve failed: {}", e)))?
            .collect();
        if addrs.is_empty() {
            return Err(JsErrorBox::generic("WebSocket DNS resolve returned no addresses"));
        }
        for addr in &addrs {
            if super::address_filter::is_blocked_address(addr) {
                return Err(JsErrorBox::generic(format!(
                    "WebSocket connection to {} is not allowed (private/loopback address)",
                    addr.ip()
                )));
            }
        }
        addrs[0]
    } else {
        return Err(JsErrorBox::generic("WebSocket URL has no host"));
    };

    // Add custom headers
    let req_headers = request.headers_mut();
    for (key, value) in &headers {
        if let (Ok(name), Ok(val)) = (
            HeaderName::from_bytes(key.as_bytes()),
            HeaderValue::from_str(value),
        ) {
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

    // Connect TCP to the verified address (no second DNS resolution).
    let tcp_stream = tokio::net::TcpStream::connect(connect_addr)
        .await
        .map_err(|e| JsErrorBox::generic(format!("WebSocket TCP connect failed: {}", e)))?;
    let _ = tcp_stream.set_nodelay(true);

    // Handshake over the pre-connected stream.
    // client_async_tls_with_config handles TLS upgrade for wss:// using
    // the hostname from the request URI for SNI — no second DNS lookup.
    let (ws_stream, response) =
        tokio_tungstenite::client_async_tls_with_config(request, tcp_stream, None, None)
            .await
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
    let resource = state
        .borrow()
        .resource_table
        .get::<WebSocketResource>(rid)
        .map_err(|_| JsErrorBox::generic("WebSocket not found"))?;

    let cancel = RcRef::map(&resource, |r| &r.cancel);

    let event = async {
        let mut rx = RcRef::map(&resource, |r| &r.rx).borrow_mut().await;
        loop {
            match rx.next().await {
                Some(Ok(Message::Text(text))) => {
                    return Ok::<WsEvent, JsErrorBox>(WsEvent::Message {
                        data_str: Some(text),
                        data_bin: None,
                        is_binary: false,
                    });
                }
                Some(Ok(Message::Binary(data))) => {
                    return Ok(WsEvent::Message {
                        data_str: None,
                        data_bin: Some(data),
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
        Message::Text(text)
    } else if let Some(buf) = data_buf {
        Message::Binary(buf.to_vec())
    } else {
        return Err(JsErrorBox::type_error("No data provided"));
    };

    let mut tx = RcRef::map(&resource, |r| &r.tx).borrow_mut().await;
    tx.send(message)
        .await
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
