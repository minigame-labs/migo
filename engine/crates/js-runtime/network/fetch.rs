use std::borrow::Cow;
use std::cell::RefCell;
use std::cmp::min;
use std::pin::Pin;
use std::rc::Rc;
use std::task::Context;
use std::task::Poll;
use std::time::Duration;

use bytes::Bytes;
use data_url::DataUrl;
use deno_core::AsyncRefCell;
use deno_core::AsyncResult;
use deno_core::BufView;
use deno_core::ByteString;
use deno_core::CancelFuture;
use deno_core::CancelHandle;
use deno_core::CancelTryFuture;
use deno_core::Canceled;
use deno_core::JsBuffer;
use deno_core::OpState;
use deno_core::RcRef;
use deno_core::Resource;
use deno_core::ResourceId;
use deno_core::error::AnyError;
use deno_core::futures::Future;
use deno_core::futures::FutureExt;
use deno_core::futures::Stream;
use deno_core::futures::StreamExt;
use deno_core::futures::stream::Peekable;
use deno_core::op2;
use deno_core::url::Url;
use deno_error::JsErrorBox;
use http::HeaderMap;
use http::HeaderName;
use http::HeaderValue;
use http::Uri;
use http::header::ACCEPT_ENCODING;
use http::header::CACHE_CONTROL;
use http::header::CONTENT_LENGTH;
use http::header::HOST;
use http::header::PRAGMA;
use http::header::RANGE;
use http::header::USER_AGENT;
use reqwest::Body;
use reqwest::Client;
use reqwest::Method;
use reqwest::Response;
use reqwest::redirect::Policy;
use serde::Serialize;
use tracing::debug;

use crate::network::Options;

// ---------------------------------------------------------------------------
// SSRF-preventing DNS resolver
// ---------------------------------------------------------------------------

/// Reject a URL whose host is an IP-literal pointing to a blocked range.
///
/// hyper-util skips the DNS resolver for IP-literal hosts, so the
/// `SsrfCheckingResolver` below does NOT cover `http://127.0.0.1/...`.
/// This function must be called **before** every `client.request()`
/// / `client.post()` to close that gap.
fn reject_blocked_ip_literal(url: &Url) -> Result<(), JsErrorBox> {
    if let Some(host) = url.host_str() {
        let port = url.port_or_known_default().unwrap_or(443);
        let addr_str = format!("{}:{}", host, port);
        if let Ok(sock_addr) = addr_str.parse::<std::net::SocketAddr>() {
            if super::address_filter::is_blocked_address(&sock_addr) {
                return Err(JsErrorBox::generic(format!(
                    "fetch: connection to {} is not allowed (private/loopback address)",
                    sock_addr.ip()
                )));
            }
        }
    }
    Ok(())
}

/// Check URL against the domain whitelist in NetworkPolicy.
///
/// **Security note:** If the whitelist is empty, all domains are allowed
/// (allow-all). The host app SHOULD populate `network_policy.domain_whitelist`
/// with the game's server domains to restrict outbound network access.
///
/// Matching supports exact match and subdomain match (e.g., "example.com"
/// allows "api.example.com").
fn check_domain_whitelist(url: &Url, state: &OpState) -> Result<(), JsErrorBox> {
    let host = state.borrow::<shared::op_state::HostOpState>();
    let wl = &host.network_policy.domain_whitelist;
    if wl.is_empty() {
        return Ok(());
    }
    if let Some(url_host) = url.host_str() {
        for allowed in wl {
            if url_host == allowed.as_str()
                || url_host.ends_with(&format!(".{}", allowed))
            {
                return Ok(());
            }
        }
        return Err(JsErrorBox::generic(format!(
            "fetch: domain '{}' is not in the allowed list",
            url_host
        )));
    }
    Ok(())
}

/// Check HTTPS enforcement policy.
fn check_https_policy(url: &Url, state: &OpState) -> Result<(), JsErrorBox> {
    let host = state.borrow::<shared::op_state::HostOpState>();
    if host.network_policy.enforce_https && url.scheme() == "http" {
        return Err(JsErrorBox::generic(
            "fetch:fail HTTP is not allowed, use HTTPS",
        ));
    }
    Ok(())
}

/// Headers that game JS must not inject — these can amplify SSRF,
/// bypass reverse proxies, or leak internal routing information.
fn is_blocked_header(name: &HeaderName) -> bool {
    matches!(
        name.as_str(),
        "x-forwarded-for"
            | "x-forwarded-host"
            | "x-forwarded-proto"
            | "x-original-url"
            | "x-rewrite-url"
            | "x-real-ip"
            | "forwarded"
            | "proxy-authorization"
    )
}

/// Custom DNS resolver that checks ALL resolved addresses against the
/// blocked-address list before returning them to reqwest.  This is injected
/// into every `reqwest::Client` via `ClientBuilder::dns_resolver()`, so
/// reqwest connects **only** to addresses we have verified — no separate
/// pre-flight check needed, no double-resolution TOCTOU window.
///
/// Note: hyper-util bypasses the resolver for IP-literal hosts, so callers
/// must also call `reject_blocked_ip_literal()` before sending requests.
struct SsrfCheckingResolver;

impl reqwest::dns::Resolve for SsrfCheckingResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        Box::pin(async move {
            let host = name.as_str();
            let addr_str = format!("{}:0", host);
            let addrs: Vec<std::net::SocketAddr> = tokio::net::lookup_host(&addr_str)
                .await
                .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { Box::new(e) })?
                .collect();

            for addr in &addrs {
                if super::address_filter::is_blocked_address(addr) {
                    return Err(Box::new(std::io::Error::new(
                        std::io::ErrorKind::PermissionDenied,
                        format!(
                            "fetch: connection to {} is not allowed (private/loopback address)",
                            addr.ip()
                        ),
                    )) as Box<dyn std::error::Error + Send + Sync>);
                }
            }

            let addrs: reqwest::dns::Addrs = Box::new(addrs.into_iter());
            Ok(addrs)
        })
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FetchReturn {
    pub request_rid: ResourceId,
    pub cancel_handle_rid: Option<ResourceId>,
}

pub struct HttpClientResource {
    pub client: Client,
    pub allow_host: bool,
}

impl Resource for HttpClientResource {
    fn name(&'_ self) -> Cow<'_, str> {
        "httpClient".into()
    }
}

impl HttpClientResource {}

type CancelableResponseResult = Result<Result<Response, AnyError>, Canceled>;

pub struct FetchRequestResource(pub Pin<Box<dyn Future<Output = CancelableResponseResult>>>);

impl Resource for FetchRequestResource {
    fn name(&'_ self) -> Cow<'_, str> {
        "fetchRequest".into()
    }
}

#[allow(clippy::type_complexity)]
pub struct ResourceToBodyAdapter(
    Rc<dyn Resource>,
    Option<Pin<Box<dyn Future<Output = Result<BufView, JsErrorBox>>>>>,
);

impl ResourceToBodyAdapter {
    pub fn new(resource: Rc<dyn Resource>) -> Self {
        let future = resource.clone().read(64 * 1024);
        Self(resource, Some(future))
    }
}

unsafe impl Send for ResourceToBodyAdapter {}
unsafe impl Sync for ResourceToBodyAdapter {}

impl Stream for ResourceToBodyAdapter {
    type Item = Result<Bytes, JsErrorBox>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        if let Some(mut fut) = this.1.take() {
            match fut.poll_unpin(cx) {
                Poll::Pending => {
                    this.1 = Some(fut);
                    Poll::Pending
                }
                Poll::Ready(res) => match res {
                    Ok(buf) if buf.is_empty() => Poll::Ready(None),
                    Ok(buf) => {
                        this.1 = Some(this.0.clone().read(64 * 1024));
                        // Use copy_from_slice for clearer intent (single allocation)
                        Poll::Ready(Some(Ok(Bytes::copy_from_slice(&buf))))
                    }
                    Err(e) => Poll::Ready(Some(Err(e))),
                },
            }
        } else {
            Poll::Ready(None)
        }
    }
}

impl Drop for ResourceToBodyAdapter {
    fn drop(&mut self) {
        self.0.clone().close()
    }
}

pub struct FetchCancelHandle(pub Rc<CancelHandle>);

impl Resource for FetchCancelHandle {
    fn name(&'_ self) -> Cow<'_, str> {
        "fetchCancelHandle".into()
    }

    fn close(self: Rc<Self>) {
        self.0.cancel()
    }
}

type BytesStream = Pin<Box<dyn Stream<Item = Result<bytes::Bytes, std::io::Error>> + Unpin>>;

pub enum FetchResponseReader {
    Start(Response),
    BodyReader(Peekable<BytesStream>),
}

impl Default for FetchResponseReader {
    fn default() -> Self {
        let stream: BytesStream = Box::pin(deno_core::futures::stream::empty());
        Self::BodyReader(stream.peekable())
    }
}

/// Cached HTTP/1.1-only client
struct Http1Client(Client);

/// Cached HTTP/2-capable client
struct Http2Client(Client);

pub fn get_or_create_client_from_state(
    state: &mut OpState,
    enable_http2: bool,
) -> Result<reqwest::Client, AnyError> {
    if enable_http2 {
        if let Some(client) = state.try_borrow::<Http2Client>() {
            Ok(client.0.clone())
        } else {
            let options = state.borrow::<Options>();
            let user_agent = options.user_agent.clone();
            let policy = state.borrow::<shared::op_state::HostOpState>().network_policy.clone();
            let client = create_http_client(&user_agent, true, &policy)?;
            state.put::<Http2Client>(Http2Client(client.clone()));
            Ok(client)
        }
    } else {
        if let Some(client) = state.try_borrow::<Http1Client>() {
            Ok(client.0.clone())
        } else {
            let options = state.borrow::<Options>();
            let user_agent = options.user_agent.clone();
            let policy = state.borrow::<shared::op_state::HostOpState>().network_policy.clone();
            let client = create_http_client(&user_agent, false, &policy)?;
            state.put::<Http1Client>(Http1Client(client.clone()));
            Ok(client)
        }
    }
}

#[op2]
#[serde]
#[allow(clippy::too_many_arguments)]
pub fn op_fetch(
    state: &mut OpState,
    #[serde] method: ByteString,
    #[string] url: String,
    #[serde] headers: Vec<(ByteString, ByteString)>,
    #[smi] client_rid: Option<u32>,
    has_body: bool,
    #[buffer] data: Option<JsBuffer>,
    #[smi] resource: Option<ResourceId>,
    #[smi] timeout: u32,
    enable_http2: bool,
    enable_cache: bool,
) -> Result<FetchReturn, JsErrorBox> {
    let (client, allow_host) = if let Some(rid) = client_rid {
        let r = state
            .resource_table
            .get::<HttpClientResource>(rid)
            .map_err(|_e| JsErrorBox::generic("Failed to get HTTP client"))?;
        (r.client.clone(), r.allow_host)
    } else {
        (
            get_or_create_client_from_state(state, enable_http2)
                .map_err(|_e| JsErrorBox::generic("Failed to create HTTP client"))?,
            false,
        )
    };

    let method =
        Method::from_bytes(&method).map_err(|_| JsErrorBox::type_error("Invalid HTTP method"))?;
    let url = Url::parse(&url).map_err(|_| JsErrorBox::type_error("Invalid URL"))?;

    debug!("Fetch request: {} {}", method, url);

    // Check scheme before asking for net permission
    let scheme = url.scheme();
    let (request_rid, cancel_handle_rid) = match scheme {
        "file" => {
            let _path = url.to_file_path().map_err(|_| {
                JsErrorBox::type_error("NetworkError when attempting to fetch resource.")
            })?;

            if method != Method::GET {
                return Err(JsErrorBox::not_supported());
            }
            return Err(JsErrorBox::not_supported());
        }
        "http" | "https" => {
            // Make sure that we have a valid URI early, as reqwest's `RequestBuilder::send`
            // internally uses `expect_uri`, which panics instead of returning a usable `Result`.
            if url.as_str().parse::<Uri>().is_err() {
                return Err(JsErrorBox::type_error("Invalid URL"));
            }

            // Security: SSRF + domain whitelist + HTTPS enforcement.
            reject_blocked_ip_literal(&url)?;
            check_domain_whitelist(&url, state)?;
            check_https_policy(&url, state)?;

            let mut request = client
                .request(method.clone(), url)
                .timeout(Duration::from_millis(timeout as u64));

            if has_body {
                match (data, resource) {
                    (Some(data), _) => {
                        // If a body is passed, we use it, and don't return a body for streaming.
                        request = request.body(data.to_vec());
                    }
                    (_, Some(resource)) => {
                        let resource = state
                            .resource_table
                            .take_any(resource)
                            .map_err(|_| JsErrorBox::generic("Failed to take resource"))?;
                        match resource.size_hint() {
                            (body_size, Some(n)) if body_size == n && body_size > 0 => {
                                request =
                                    request.header(CONTENT_LENGTH, HeaderValue::from(body_size));
                            }
                            _ => {}
                        }
                        request =
                            request.body(Body::wrap_stream(ResourceToBodyAdapter::new(resource)))
                    }
                    (None, None) => unreachable!(),
                }
            } else {
                if matches!(method, Method::POST | Method::PUT) {
                    request = request.header(CONTENT_LENGTH, HeaderValue::from(0));
                }
            };

            let mut header_map = HeaderMap::new();
            for (key, value) in headers {
                let name = HeaderName::from_bytes(&key)
                    .map_err(|_| JsErrorBox::type_error("Invalid Header"))?;
                let v = HeaderValue::from_bytes(&value)
                    .map_err(|_| JsErrorBox::type_error("Invalid Header Value"))?;

                if (name != HOST || allow_host)
                    && name != CONTENT_LENGTH
                    && !is_blocked_header(&name)
                {
                    header_map.append(name, v);
                }
            }

            if header_map.contains_key(RANGE) {
                header_map.insert(ACCEPT_ENCODING, HeaderValue::from_static("identity"));
            }

            if !enable_cache {
                if !header_map.contains_key(CACHE_CONTROL) {
                    header_map.insert(CACHE_CONTROL, HeaderValue::from_static("no-cache"));
                }
                if !header_map.contains_key(PRAGMA) {
                    header_map.insert(PRAGMA, HeaderValue::from_static("no-cache"));
                }
            }

            request = request.headers(header_map);

            let cancel_handle = CancelHandle::new_rc();
            let cancel_handle_ = cancel_handle.clone();

            let fut = async move {
                // DNS resolution and SSRF check happen inside
                // SsrfCheckingResolver when reqwest opens the connection.
                request
                    .send()
                    .or_cancel(cancel_handle_)
                    .await
                    .map(|res| res.map_err(|err| err.into()))
            };

            let request_rid = state
                .resource_table
                .add(FetchRequestResource(Box::pin(fut)));

            let cancel_handle_rid = state.resource_table.add(FetchCancelHandle(cancel_handle));

            (request_rid, Some(cancel_handle_rid))
        }
        "data" => {
            let data_url = DataUrl::process(url.as_str())
                .map_err(|_| JsErrorBox::type_error("Invalid Data URL"))?;

            let (body, _) = data_url
                .decode_to_vec()
                .map_err(|_| JsErrorBox::type_error("Invalid Base64"))?;

            let response = http::Response::builder()
                .status(http::StatusCode::OK)
                .header(http::header::CONTENT_TYPE, data_url.mime_type().to_string())
                .body(reqwest::Body::from(body))
                .map_err(|e| JsErrorBox::generic(e.to_string()))?;

            let fut = async move { Ok(Ok(Response::from(response))) };

            let request_rid = state
                .resource_table
                .add(FetchRequestResource(Box::pin(fut)));

            (request_rid, None)
        }
        "blob" => {
            return Err(JsErrorBox::type_error("BlobNotFound"));
        }
        _ => return Err(JsErrorBox::type_error("SchemeNotSupported")),
    };

    Ok(FetchReturn {
        request_rid,
        cancel_handle_rid,
    })
}

#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FetchResponse {
    pub status: u16,
    pub status_text: String,
    pub headers: Vec<(ByteString, ByteString)>,
    pub url: String,
    pub response_rid: ResourceId,
    pub content_length: Option<u64>,
    pub remote_addr_ip: Option<String>,
    pub remote_addr_port: Option<u16>,
    pub error: Option<String>,
}

#[derive(Debug)]
pub struct FetchResponseResource {
    pub response_reader: AsyncRefCell<FetchResponseReader>,
    pub cancel: CancelHandle,
    pub size: Option<u64>,
}

impl FetchResponseResource {
    pub fn new(response: Response, size: Option<u64>) -> Self {
        Self {
            response_reader: AsyncRefCell::new(FetchResponseReader::Start(response)),
            cancel: CancelHandle::default(),
            size,
        }
    }
}

impl Resource for FetchResponseResource {
    fn name(&'_ self) -> Cow<'_, str> {
        "fetchResponse".into()
    }

    fn read(self: Rc<Self>, limit: usize) -> AsyncResult<BufView> {
        Box::pin(async move {
            let mut reader = RcRef::map(&self, |r| &r.response_reader).borrow_mut().await;

            let body = loop {
                match &mut *reader {
                    FetchResponseReader::BodyReader(reader) => break reader,
                    FetchResponseReader::Start(_) => {}
                }

                match std::mem::take(&mut *reader) {
                    FetchResponseReader::Start(resp) => {
                        let stream: BytesStream = Box::pin(resp.bytes_stream().map(|r| {
                            r.map_err(|err| std::io::Error::new(std::io::ErrorKind::Other, err))
                        }));
                        *reader = FetchResponseReader::BodyReader(stream.peekable());
                    }
                    FetchResponseReader::BodyReader(_) => unreachable!(),
                }
            };
            let fut = async move {
                let mut reader = Pin::new(body);
                loop {
                    match reader.as_mut().peek_mut().await {
                        Some(Ok(chunk)) if !chunk.is_empty() => {
                            let len = min(limit, chunk.len());
                            let chunk = chunk.split_to(len);
                            break Ok(chunk.into());
                        }
                        Some(_) => match reader.as_mut().next().await.unwrap() {
                            Ok(chunk) => assert!(chunk.is_empty()),
                            Err(err) => break Err(JsErrorBox::generic(err.to_string())),
                        },
                        None => break Ok(BufView::empty()),
                    }
                }
            };

            let cancel_handle = RcRef::map(self, |r| &r.cancel);
            fut.try_or_cancel(cancel_handle).await
        })
    }

    fn size_hint(&self) -> (u64, Option<u64>) {
        (self.size.unwrap_or(0), self.size)
    }

    fn close(self: Rc<Self>) {
        self.cancel.cancel()
    }
}

#[op2(async(lazy), fast)]
#[serde]
pub async fn op_fetch_send(
    state: Rc<RefCell<OpState>>,
    #[smi] rid: ResourceId,
) -> Result<FetchResponse, JsErrorBox> {
    let request = state
        .borrow_mut()
        .resource_table
        .take::<FetchRequestResource>(rid)
        .map_err(|_| JsErrorBox::generic("Failed to take fetch request resource"))?;

    let request = Rc::try_unwrap(request)
        .ok()
        .expect("multiple op_fetch_send ongoing");

    let res = match request.0.await {
        Ok(Ok(res)) => res,
        Ok(Err(err)) => {
            let mut err_ref: &dyn std::error::Error = err.as_ref();
            while let Some(err) = std::error::Error::source(err_ref) {
                if let Some(err) = err.downcast_ref::<reqwest::Error>() {
                    if err.is_body() {
                        // Extracts the next error cause and uses that for the message
                        if let Some(err) = std::error::Error::source(err) {
                            return Ok(FetchResponse {
                                error: Some(err.to_string()),
                                ..Default::default()
                            });
                        }
                    }
                }
                err_ref = err;
            }

            return Err(JsErrorBox::type_error(err_ref.to_string()));
        }
        Err(_) => return Err(JsErrorBox::type_error("request was cancelled")),
    };

    let status = res.status();
    let url = res.url().to_string();
    let mut res_headers = Vec::new();
    for (key, val) in res.headers().iter() {
        res_headers.push((key.as_str().into(), val.as_bytes().into()));
    }

    let content_length = res.content_length();
    let remote_addr = res.remote_addr();

    // SSRF prevention: check the *actual* resolved address after reqwest
    // performed DNS resolution internally.  The pre-flight check in op_fetch
    // only catches IP-literal URLs; this covers the domain-name path.
    if let Some(addr) = remote_addr {
        if super::address_filter::is_blocked_address(&addr) {
            return Err(JsErrorBox::generic(format!(
                "fetch: connection to {} is not allowed (private/loopback address)",
                addr.ip()
            )));
        }
    }

    let (remote_addr_ip, remote_addr_port) = if let Some(addr) = remote_addr {
        (Some(addr.ip().to_string()), Some(addr.port()))
    } else {
        (None, None)
    };

    let response_rid = state
        .borrow_mut()
        .resource_table
        .add(FetchResponseResource::new(res, content_length));

    Ok(FetchResponse {
        status: status.as_u16(),
        status_text: status.canonical_reason().unwrap_or("").to_string(),
        headers: res_headers,
        url,
        response_rid,
        content_length,
        remote_addr_ip,
        remote_addr_port,
        error: None,
    })
}

pub fn create_http_client(
    user_agent: &str,
    enable_http2: bool,
    net_policy: &shared::op_state::NetworkPolicy,
) -> Result<Client, AnyError> {
    let mut headers = HeaderMap::new();
    headers.insert(USER_AGENT, user_agent.parse().unwrap());

    // Capture network policy for the redirect closure.
    let whitelist = net_policy.domain_whitelist.clone();
    let enforce_https = net_policy.enforce_https;

    // Custom redirect policy: checks IP-block, domain whitelist, and HTTPS
    // enforcement on every redirect target.  This prevents bypasses like
    // "allowed.com → 302 → blocked.com" or "https → 302 → http".
    let ssrf_redirect_policy = Policy::custom(move |attempt| {
        if attempt.previous().len() >= 10 {
            return attempt.stop();
        }

        // Extract URL data before consuming `attempt` (which takes ownership).
        let scheme = attempt.url().scheme().to_string();
        let host_str = attempt.url().host_str().map(|s| s.to_string());
        let port = attempt.url().port_or_known_default().unwrap_or(443);

        if let Some(ref host) = host_str {
            // 1. Block redirect to private/loopback IP-literals
            let addr_str = format!("{}:{}", host, port);
            if let Ok(sock_addr) = addr_str.parse::<std::net::SocketAddr>() {
                if super::address_filter::is_blocked_address(&sock_addr) {
                    return attempt.error(std::io::Error::new(
                        std::io::ErrorKind::PermissionDenied,
                        format!(
                            "fetch: redirect to {} is not allowed (private/loopback address)",
                            sock_addr.ip()
                        ),
                    ));
                }
            }

            // 2. Domain whitelist check on redirect target
            if !whitelist.is_empty() {
                let allowed = whitelist.iter().any(|d| {
                    host.as_str() == d.as_str() || host.ends_with(&format!(".{}", d))
                });
                if !allowed {
                    return attempt.error(std::io::Error::new(
                        std::io::ErrorKind::PermissionDenied,
                        format!("fetch: redirect to '{}' is not in the allowed domain list", host),
                    ));
                }
            }
        }

        // 3. HTTPS enforcement on redirect target
        if enforce_https && scheme == "http" {
            return attempt.error(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "fetch: redirect to HTTP is not allowed (HTTPS enforced)",
            ));
        }

        attempt.follow()
    });

    let mut builder = Client::builder()
        .dns_resolver(std::sync::Arc::new(SsrfCheckingResolver))
        .redirect(ssrf_redirect_policy)
        .default_headers(headers)
        .pool_max_idle_per_host(5)
        .pool_idle_timeout(Duration::from_secs(90));

    if enable_http2 {
        builder = builder.http2_adaptive_window(true);
    } else {
        builder = builder.http1_only();
    }

    builder.build().map_err(|e| e.into())
}

// ── Upload ──

#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FetchUploadResult {
    pub data: String,
    pub status_code: u16,
    pub headers: Vec<(ByteString, ByteString)>,
    pub total_bytes_sent: u64,
    pub error: Option<String>,
}

#[op2(async(lazy))]
#[serde]
#[allow(clippy::too_many_arguments)]
pub async fn op_fetch_upload(
    state: Rc<RefCell<OpState>>,
    #[string] url: String,
    #[buffer] file_data: JsBuffer,
    #[string] name: String,
    #[string] filename: String,
    #[serde] headers: Vec<(ByteString, ByteString)>,
    #[serde] form_data: Vec<(String, String)>,
    #[smi] timeout: u32,
    enable_http2: bool,
) -> Result<FetchUploadResult, JsErrorBox> {
    let client = {
        let mut st = state.borrow_mut();
        get_or_create_client_from_state(&mut st, enable_http2)
            .map_err(|e| JsErrorBox::generic(e.to_string()))?
    };

    let file_bytes = file_data.to_vec();
    let file_size = file_bytes.len() as u64;

    // Guess MIME type from filename extension
    let mime = match filename.rsplit('.').next().map(|e| e.to_lowercase()) {
        Some(ext) => match ext.as_str() {
            "jpg" | "jpeg" => "image/jpeg",
            "png" => "image/png",
            "gif" => "image/gif",
            "webp" => "image/webp",
            "mp3" => "audio/mpeg",
            "mp4" => "video/mp4",
            "wav" => "audio/wav",
            "ogg" => "audio/ogg",
            "json" => "application/json",
            "xml" => "application/xml",
            "txt" => "text/plain",
            "zip" => "application/zip",
            _ => "application/octet-stream",
        },
        None => "application/octet-stream",
    };

    // Build multipart form
    let file_part = reqwest::multipart::Part::bytes(file_bytes)
        .file_name(filename)
        .mime_str(mime)
        .map_err(|e| JsErrorBox::generic(e.to_string()))?;

    let mut form = reqwest::multipart::Form::new().part(name, file_part);

    for (key, value) in form_data {
        form = form.text(key, value);
    }

    // Parse URL
    let parsed_url = Url::parse(&url).map_err(|_| JsErrorBox::type_error("Invalid URL"))?;

    // Security: SSRF + domain whitelist + HTTPS enforcement.
    reject_blocked_ip_literal(&parsed_url)?;
    {
        let st = state.borrow();
        check_domain_whitelist(&parsed_url, &*st)?;
        check_https_policy(&parsed_url, &*st)?;
    }

    debug!("Upload request: POST {}", parsed_url);

    // Build request
    let mut request = client
        .post(parsed_url)
        .timeout(Duration::from_millis(timeout as u64))
        .multipart(form);

    // Apply custom headers — same security filtering as op_fetch.
    let mut header_map = HeaderMap::new();
    for (key, value) in headers {
        let hname =
            HeaderName::from_bytes(&key).map_err(|_| JsErrorBox::type_error("Invalid Header"))?;
        let hval = HeaderValue::from_bytes(&value)
            .map_err(|_| JsErrorBox::type_error("Invalid Header Value"))?;
        // Skip Content-Type and Content-Length (reqwest manages these for multipart),
        // HOST (prevent host-header attacks), and proxy-related headers (SSRF hardening).
        if hname != http::header::CONTENT_TYPE
            && hname != CONTENT_LENGTH
            && hname != HOST
            && !is_blocked_header(&hname)
        {
            header_map.append(hname, hval);
        }
    }
    request = request.headers(header_map);

    // Send request
    let res = match request.send().await {
        Ok(res) => res,
        Err(err) => {
            return Ok(FetchUploadResult {
                error: Some(err.to_string()),
                ..Default::default()
            });
        }
    };

    // Extract response
    let status = res.status().as_u16();
    let mut res_headers = Vec::new();
    for (key, val) in res.headers().iter() {
        res_headers.push((key.as_str().into(), val.as_bytes().into()));
    }

    let body = res
        .text()
        .await
        .map_err(|e| JsErrorBox::generic(e.to_string()))?;

    Ok(FetchUploadResult {
        data: body,
        status_code: status,
        headers: res_headers,
        total_bytes_sent: file_size,
        error: None,
    })
}
