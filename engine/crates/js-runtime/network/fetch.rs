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
use http::header::CONTENT_LENGTH;
use http::header::HOST;
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

impl HttpClientResource {
}

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

pub fn get_or_create_client_from_state(state: &mut OpState) -> Result<reqwest::Client, AnyError> {
    if let Some(client) = state.try_borrow::<reqwest::Client>() {
        Ok(client.clone())
    } else {
        let options = state.borrow::<Options>();
        let client = create_client_from_options(options)?;
        state.put::<reqwest::Client>(client.clone());
        Ok(client)
    }
}

pub fn create_client_from_options(options: &Options) -> Result<reqwest::Client, AnyError> {
    create_http_client(
        &options.user_agent,
        CreateHttpClientOptions {
            pool_max_idle_per_host: None,
            pool_idle_timeout: None,
        },
    )
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
) -> Result<FetchReturn, JsErrorBox> {
    let (client, allow_host) = if let Some(rid) = client_rid {
        let r = state
            .resource_table
            .get::<HttpClientResource>(rid)
            .map_err(|_e| JsErrorBox::generic("Failed to get HTTP client"))?;
        (r.client.clone(), r.allow_host)
    } else {
        (
            get_or_create_client_from_state(state)
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

                if (name != HOST || allow_host) && name != CONTENT_LENGTH {
                    header_map.append(name, v);
                }
            }

            if header_map.contains_key(RANGE) {
                header_map.insert(ACCEPT_ENCODING, HeaderValue::from_static("identity"));
            }
            request = request.headers(header_map);

            let cancel_handle = CancelHandle::new_rc();
            let cancel_handle_ = cancel_handle.clone();

            let fut = async move {
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

#[op2(async)]
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

#[derive(Debug, Clone)]
pub struct CreateHttpClientOptions {
    pub pool_max_idle_per_host: Option<usize>,
    pub pool_idle_timeout: Option<Option<u64>>,
}

impl Default for CreateHttpClientOptions {
    fn default() -> Self {
        CreateHttpClientOptions {
            pool_max_idle_per_host: None,
            pool_idle_timeout: None,
        }
    }
}

pub fn create_http_client(
    user_agent: &str,
    options: CreateHttpClientOptions,
) -> Result<Client, AnyError> {
    let mut headers = HeaderMap::new();
    headers.insert(USER_AGENT, user_agent.parse().unwrap());
    let mut builder = Client::builder()
        .redirect(Policy::none())
        .default_headers(headers);

    if let Some(pool_max_idle_per_host) = options.pool_max_idle_per_host {
        builder = builder.pool_max_idle_per_host(pool_max_idle_per_host);
    }

    if let Some(pool_idle_timeout) = options.pool_idle_timeout {
        builder =
            builder.pool_idle_timeout(pool_idle_timeout.map(std::time::Duration::from_millis));
    }

    builder = builder.http2_adaptive_window(true);
    builder.build().map_err(|e| e.into())
}
