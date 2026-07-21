use std::{cell::RefCell, future::Future, rc::Rc, sync::Arc};

use deno_core::{
    Extension, ExtensionArguments, FsModuleLoader, JsRuntime, ModuleLoader, OpState,
    PollEventLoopOptions, RuntimeOptions, SharedArrayBufferStore, op2, resolve_path, v8,
};
use tokio::sync::mpsc;
use tracing::{error, info, warn};

use shared::op_state::HostOpState;

use crate::watchdog::{DeadlineWatchdog, DeadlineWatchdogConfig};

/// Maximum size for a single worker message payload (16 MB).
/// Prevents large messages from bypassing V8 heap limits.
const MAX_WORKER_MESSAGE_BYTES: usize = 16 * 1024 * 1024;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Messages flowing from the main thread to the worker thread.
pub(crate) enum WorkerMessage {
    /// A postMessage payload (JSON-serialized string).
    Message(String),
    /// A binary transfer (zero-copy ArrayBuffer ownership transfer).
    Binary(Vec<u8>),
    /// Terminate the worker.
    Terminate,
}

#[derive(Debug, Clone, Copy)]
struct WorkerTimerLifecycleTransition {
    backgrounded: bool,
    occurred_at: tokio::time::Instant,
}

impl WorkerTimerLifecycleTransition {
    fn now(backgrounded: bool) -> Self {
        Self {
            backgrounded,
            occurred_at: tokio::time::Instant::now(),
        }
    }
}

/// Typed events delivered only to the worker's internal message pump.
#[derive(Debug, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub(crate) enum WorkerInbound {
    Message {
        data: String,
    },
    Lifecycle {
        backgrounded: bool,
        #[serde(rename = "elapsedMicros")]
        elapsed_micros: u64,
    },
}

/// Stored in the **main** thread's `OpState` when a worker is active.
pub(crate) struct WorkerHandle {
    tx_to_worker: mpsc::UnboundedSender<WorkerMessage>,
    rx_from_worker: Arc<tokio::sync::Mutex<mpsc::UnboundedReceiver<String>>>,
    rx_errors: Arc<tokio::sync::Mutex<mpsc::UnboundedReceiver<String>>>,
    timer_backgrounded_tx: mpsc::UnboundedSender<WorkerTimerLifecycleTransition>,
    #[allow(dead_code)]
    join_handle: Option<std::thread::JoinHandle<()>>,
    terminated: bool,
    /// Thread-safe handle to the worker's V8 isolate, published by the worker
    /// thread right after it builds its `JsRuntime`. Used to *forcibly* stop a
    /// runaway worker: the cooperative `Terminate` message is only observed
    /// when the worker is awaiting `op_worker_inner_recv_message`, so a
    /// compute-bound `while (true) {}` would otherwise never exit. Wrapped in a
    /// `Mutex<Option<..>>` because it is filled asynchronously (the worker may
    /// not have created the isolate yet when this handle is stored).
    isolate_handle: Arc<std::sync::Mutex<Option<v8::IsolateHandle>>>,
}

impl WorkerHandle {
    /// Force the worker to stop: interrupt any executing JS via the isolate
    /// handle (breaks a runaway loop that ignores the cooperative `Terminate`)
    /// and signal the message pump to exit. Safe to call more than once and
    /// after the worker isolate has already been disposed.
    fn force_terminate(&self) {
        if let Ok(guard) = self.isolate_handle.lock() {
            if let Some(h) = guard.as_ref() {
                h.terminate_execution();
            }
        }
        let _ = self.tx_to_worker.send(WorkerMessage::Terminate);
    }
}

impl Drop for WorkerHandle {
    fn drop(&mut self) {
        // Guarantees the worker thread + its V8 isolate are torn down when the
        // main runtime is dropped (JS runtime restart / host shutdown), even if
        // the game never called `terminate()`. Without the isolate interrupt a
        // compute-bound worker would leak its OS thread and isolate for the rest
        // of the process lifetime.
        self.force_terminate();
    }
}

/// Stored in the **worker** thread's `OpState`.
pub(crate) struct WorkerCtx {
    tx_to_main: mpsc::UnboundedSender<String>,
    tx_errors: mpsc::UnboundedSender<String>,
    rx_from_main: Arc<tokio::sync::Mutex<mpsc::UnboundedReceiver<WorkerMessage>>>,
    timer_backgrounded_rx:
        Arc<tokio::sync::Mutex<mpsc::UnboundedReceiver<WorkerTimerLifecycleTransition>>>,
}

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error, deno_error::JsError)]
pub enum WorkerError {
    #[class("WorkerError")]
    #[error("{0}")]
    Message(String),
}

// ---------------------------------------------------------------------------
// Module loader (reuse the same logic from core::runtime::loader)
// ---------------------------------------------------------------------------

/// Lightweight module loader for the worker thread.
/// Mirrors `MyModuleLoader` from `core::runtime::loader` — auto-adds `.js`,
/// patches AMD `define` modules, and enforces the `/code` sandbox via
/// the mount table (same security boundary as the main thread loader).
struct WorkerModuleLoader {
    inner: FsModuleLoader,
    mount_table: Option<Arc<shared::vfs::MountTable>>,
}

impl WorkerModuleLoader {
    /// Validate that a resolved module URL is within the /code sandbox.
    fn validate_sandbox(
        &self,
        url: &deno_core::ModuleSpecifier,
    ) -> Result<(), deno_core::error::ModuleLoaderError> {
        // Fail-closed: a worker with no /code mount table has no sandbox to
        // enforce against, so refuse module loading rather than fall through to
        // the raw filesystem loader. `op_worker_create` also requires a mount
        // table, so reaching here with `None` should be impossible — this is
        // defense in depth against a future caller that skips that check.
        let Some(mt) = self.mount_table.as_ref() else {
            return Err(deno_core::error::ModuleLoaderError::generic(
                "Worker module load blocked: no /code mount table (sandbox unavailable)",
            ));
        };
        let Ok(path) = url.to_file_path() else {
            return Ok(());
        };
        if mt.is_allowed_path(&path) {
            return Ok(());
        }
        Err(deno_core::error::ModuleLoaderError::generic(format!(
            "Worker module import blocked: path escapes /code sandbox: {}",
            path.display()
        )))
    }
}

impl deno_core::ModuleLoader for WorkerModuleLoader {
    fn resolve(
        &self,
        specifier: &str,
        referrer: &str,
        kind: deno_core::ResolutionKind,
    ) -> Result<deno_core::ModuleSpecifier, deno_core::error::ModuleLoaderError> {
        let spec = normalize_specifier(specifier, &kind);
        let url = self.inner.resolve(spec.as_ref(), referrer, kind)?;
        self.validate_sandbox(&url)?;
        Ok(url)
    }

    fn load(
        &self,
        module_specifier: &deno_core::ModuleSpecifier,
        maybe_referrer: Option<&deno_core::ModuleLoadReferrer>,
        options: deno_core::ModuleLoadOptions,
    ) -> deno_core::ModuleLoadResponse {
        // Defense in depth: validate on load too.
        if let Err(e) = self.validate_sandbox(module_specifier) {
            return deno_core::ModuleLoadResponse::Sync(Err(e));
        }
        let resp = self.inner.load(module_specifier, maybe_referrer, options);
        match resp {
            deno_core::ModuleLoadResponse::Sync(result) => {
                deno_core::ModuleLoadResponse::Sync(result.and_then(patch_amd))
            }
            deno_core::ModuleLoadResponse::Async(fut) => {
                let fut = async move {
                    let source = fut.await?;
                    patch_amd(source)
                };
                deno_core::ModuleLoadResponse::Async(Box::pin(fut))
            }
        }
    }

    fn prepare_load(
        &self,
        module_specifier: &deno_core::ModuleSpecifier,
        maybe_referrer: Option<String>,
        maybe_content: Option<String>,
        options: deno_core::ModuleLoadOptions,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<(), deno_core::error::ModuleLoaderError>>>,
    > {
        self.inner
            .prepare_load(module_specifier, maybe_referrer, maybe_content, options)
    }

    fn finish_load(&self) {}

    fn code_cache_ready(
        &self,
        module_specifier: deno_core::ModuleSpecifier,
        hash: u64,
        code_cache: &[u8],
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()>>> {
        self.inner
            .code_cache_ready(module_specifier, hash, code_cache)
    }
}

fn normalize_specifier<'a>(
    specifier: &'a str,
    kind: &deno_core::ResolutionKind,
) -> std::borrow::Cow<'a, str> {
    use std::borrow::Cow;

    let mut s: Cow<'a, str> = if *kind != deno_core::ResolutionKind::MainModule {
        if specifier.starts_with("./") || specifier.starts_with("../") || specifier.contains(':') {
            Cow::Borrowed(specifier)
        } else {
            Cow::Owned(format!("./{specifier}"))
        }
    } else {
        Cow::Borrowed(specifier)
    };

    let (path_part, suffix_part) = match s.find(['?', '#']) {
        Some(i) => (&s.as_ref()[..i], &s.as_ref()[i..]),
        None => (s.as_ref(), ""),
    };

    let has_js_like_ext =
        path_part.ends_with(".js") || path_part.ends_with(".mjs") || path_part.ends_with(".cjs");

    if !has_js_like_ext {
        let new_path = format!("{path_part}.js{suffix_part}");
        s = Cow::Owned(new_path);
    }

    s
}

fn patch_amd(
    mut source: deno_core::ModuleSource,
) -> Result<deno_core::ModuleSource, deno_core::error::ModuleLoaderError> {
    let code = String::from_utf8_lossy(source.code.as_bytes());
    if code.contains("define.amd") || code.contains("typeof define") {
        let mut patched = code.into_owned();
        patched.push_str("\nexport default globalThis._lastDefinedModule;\n");
        source.code = deno_core::ModuleSourceCode::String(patched.into());
    } else if shared::cjs_compat::is_cjs(&code) {
        let patched = shared::cjs_compat::wrap_cjs(&code);
        source.code = deno_core::ModuleSourceCode::String(patched.into());
    }
    Ok(source)
}

// ---------------------------------------------------------------------------
// Main-thread ops (registered in `host_v8_worker`)
// ---------------------------------------------------------------------------

/// Create and spawn a worker thread. Only one worker can exist at a time.
#[op2(async(lazy), fast)]
async fn op_worker_create(
    state: Rc<RefCell<OpState>>,
    #[string] script_path: String,
) -> Result<(), WorkerError> {
    // Check existing worker. A previously terminated worker whose thread has
    // fully exited is reaped here so a new one can be created; but we refuse
    // while any worker is still alive OR still winding down after terminate()
    // (its thread not yet finished), so an old and new worker never coexist.
    {
        let needs_reap = {
            let st = state.borrow();
            match st.try_borrow::<WorkerHandle>() {
                None => false,
                Some(h) => {
                    let finished = h.join_handle.as_ref().map_or(true, |jh| jh.is_finished());
                    if h.terminated && finished {
                        true
                    } else {
                        return Err(WorkerError::Message(
                            "Only one worker can exist at a time. Call terminate() first.".into(),
                        ));
                    }
                }
            }
        };
        if needs_reap {
            // Drop the finished, terminated handle (its thread already exited,
            // so the detached JoinHandle leaks nothing).
            drop(state.borrow_mut().take::<WorkerHandle>());
        }
    }

    // Get the SharedArrayBufferStore from the main runtime (if set)
    let sab_store = {
        let st = state.borrow();
        st.try_borrow::<SharedArrayBufferStore>()
            .cloned()
            .unwrap_or_default()
    };

    // Read code_dir and clone a minimal HostOpState for the worker
    let (code_dir, worker_host_state) = {
        let st = state.borrow();
        let host = st.borrow::<HostOpState>();
        let code_dir = host.code_dir.clone().ok_or_else(|| {
            WorkerError::Message("No code directory set (game not loaded yet)".into())
        })?;

        // Fail-closed sandbox: refuse to spawn a worker before the /code mount
        // table exists, so the worker module loader always has a sandbox to
        // enforce (see WorkerModuleLoader::validate_sandbox).
        if host.mount_table.is_none() {
            return Err(WorkerError::Message(
                "No /code mount table set (game not fully loaded yet)".into(),
            ));
        }

        // Create dummy channels for services the worker does not use
        let (render_tx, _render_rx) = shared::render_command_sender::CommandSender::new();
        let (audio_raw_tx, _audio_rx) = mpsc::unbounded_channel();
        // Workers don't send audio commands in practice, but the type
        // system requires an AudioSender.  Use a no-op ThreadWakeup.
        let audio_tx =
            shared::op_state::AudioSender::new(audio_raw_tx, shared::channel::ThreadWakeup::new());

        let worker_state = HostOpState {
            id: host.id,
            app_cache_dir: host.app_cache_dir.clone(),
            app_files_dir: host.app_files_dir.clone(),
            code_dir: host.code_dir.clone(),
            game_paths: host.game_paths.clone(),
            vfs: host.vfs.clone(),
            mount_table: host.mount_table.clone(),
            render_tx,
            // Workers don't drive measureText (no Canvas2D context in
            // Web Worker yet), so the fast-path measurer is left `None`.
            text_measurer: None,
            audio_tx,
            host_tx: host.host_tx.clone(),
            device_services: None,
            raf_rx: None,
            raf_demand: std::sync::Arc::new(shared::raf_signal::RafDemand::new()),
            request_vsync: None,
            sub_packages: host.sub_packages.clone(),
            workers_path: host.workers_path.clone(),
            network_policy: host.network_policy.clone(),
            backgrounded: host.backgrounded.clone(),
            timer_backgrounded: host.timer_backgrounded.clone(),
            webgl_context_created: host.webgl_context_created.clone(),
            context_lost: host.context_lost.clone(),
            code_signing_enabled: host.code_signing_enabled,
            gpu_caps: host.gpu_caps.clone(),
        };

        (code_dir, worker_state)
    };

    // Create bidirectional channels
    let (tx_main_to_worker, rx_main_to_worker) = mpsc::unbounded_channel::<WorkerMessage>();
    let (tx_worker_to_main, rx_worker_to_main) = mpsc::unbounded_channel::<String>();
    let (tx_worker_errors, rx_worker_errors) = mpsc::unbounded_channel::<String>();
    let (timer_backgrounded_tx, timer_backgrounded_rx) = mpsc::unbounded_channel();

    let worker_ctx = WorkerCtx {
        tx_to_main: tx_worker_to_main,
        tx_errors: tx_worker_errors,
        rx_from_main: Arc::new(tokio::sync::Mutex::new(rx_main_to_worker)),
        timer_backgrounded_rx: Arc::new(tokio::sync::Mutex::new(timer_backgrounded_rx)),
    };

    // Shared slot the worker thread fills with its isolate handle once its
    // JsRuntime is built, so the main thread can forcibly terminate a runaway
    // worker (see WorkerHandle::force_terminate).
    let isolate_handle: Arc<std::sync::Mutex<Option<v8::IsolateHandle>>> =
        Arc::new(std::sync::Mutex::new(None));

    // Spawn worker thread
    info!(
        "[Worker] spawning worker thread for script: {}",
        script_path
    );
    let join_handle = spawn_worker_thread(
        script_path,
        code_dir,
        worker_ctx,
        worker_host_state,
        sab_store,
        isolate_handle.clone(),
    )?;
    info!("[Worker] worker thread spawned, storing handle");

    // Store handle in main OpState
    let handle = WorkerHandle {
        tx_to_worker: tx_main_to_worker,
        rx_from_worker: Arc::new(tokio::sync::Mutex::new(rx_worker_to_main)),
        rx_errors: Arc::new(tokio::sync::Mutex::new(rx_worker_errors)),
        timer_backgrounded_tx,
        join_handle: Some(join_handle),
        terminated: false,
        isolate_handle,
    };

    state.borrow_mut().put(handle);
    Ok(())
}

/// Send a message from the main thread to the worker.
#[op2(fast)]
fn op_worker_post_message(
    state: &mut OpState,
    #[string] json_message: String,
) -> Result<(), WorkerError> {
    if json_message.len() > MAX_WORKER_MESSAGE_BYTES {
        return Err(WorkerError::Message(format!(
            "Worker message too large: {} bytes (max {} bytes)",
            json_message.len(),
            MAX_WORKER_MESSAGE_BYTES
        )));
    }

    let handle = state
        .try_borrow::<WorkerHandle>()
        .ok_or_else(|| WorkerError::Message("No active worker".into()))?;

    if handle.terminated {
        return Err(WorkerError::Message("Worker has been terminated".into()));
    }

    info!(
        "[Worker] main->worker postMessage: {} bytes",
        json_message.len()
    );
    handle
        .tx_to_worker
        .send(WorkerMessage::Message(json_message))
        .map_err(|_| WorkerError::Message("Worker channel closed".into()))
}

/// Async op: wait for a message from the worker. Returns null when worker exits.
#[op2(async(lazy), fast)]
#[string]
async fn op_worker_recv_message(
    state: Rc<RefCell<OpState>>,
) -> Result<Option<String>, WorkerError> {
    let rx = {
        let st = state.borrow();
        let handle = st
            .try_borrow::<WorkerHandle>()
            .ok_or_else(|| WorkerError::Message("No active worker".into()))?;
        handle.rx_from_worker.clone()
    };

    info!("[Worker] main waiting for worker message...");
    let mut guard = rx.lock().await;
    let msg = guard.recv().await;
    info!(
        "[Worker] main received from worker: {:?}",
        msg.as_ref().map(|s| s.len())
    );
    Ok(msg)
}

/// Async op: wait for an error from the worker. Returns null when worker exits.
#[op2(async(lazy), fast)]
#[string]
async fn op_worker_recv_error(state: Rc<RefCell<OpState>>) -> Result<Option<String>, WorkerError> {
    let rx = {
        let st = state.borrow();
        let handle = st
            .try_borrow::<WorkerHandle>()
            .ok_or_else(|| WorkerError::Message("No active worker".into()))?;
        handle.rx_errors.clone()
    };

    let mut guard = rx.lock().await;
    Ok(guard.recv().await)
}

/// Terminate the active worker.
#[op2(fast)]
fn op_worker_terminate(state: &mut OpState) -> Result<(), WorkerError> {
    let handle = state
        .try_borrow_mut::<WorkerHandle>()
        .ok_or_else(|| WorkerError::Message("No active worker".into()))?;
    if !handle.terminated {
        handle.terminated = true;
        // Interrupt any executing JS (breaks a runaway `while (true) {}`) and
        // signal the message pump to exit.
        handle.force_terminate();
    }
    // The handle is intentionally KEPT in OpState (not taken) until the worker
    // thread has actually exited. `op_worker_create` reaps it once
    // `join_handle.is_finished()`, so a freshly created worker can never coexist
    // with an old one that is still winding down (e.g. briefly stuck in a native
    // op after the JS-loop interrupt).
    Ok(())
}

/// Send binary data from the main thread to the worker.
///
/// NOTE: This currently copies the buffer (deno_core limitation — true zero-copy
/// transfer requires V8 ArrayBuffer::Detach + BackingStore sharing which is not
/// yet wired). The JS-side ArrayBuffer is NOT detached/neutered.
/// Still faster than JSON-serializing large typed arrays.
#[op2(fast)]
fn op_worker_transfer_buffer(
    state: &mut OpState,
    #[buffer(copy)] data: Vec<u8>,
) -> Result<(), WorkerError> {
    if data.len() > MAX_WORKER_MESSAGE_BYTES {
        return Err(WorkerError::Message(format!(
            "Transfer buffer too large: {} bytes (max {} bytes)",
            data.len(),
            MAX_WORKER_MESSAGE_BYTES
        )));
    }

    let handle = state
        .try_borrow::<WorkerHandle>()
        .ok_or_else(|| WorkerError::Message("No active worker".into()))?;

    if handle.terminated {
        return Err(WorkerError::Message("Worker has been terminated".into()));
    }

    handle
        .tx_to_worker
        .send(WorkerMessage::Binary(data))
        .map_err(|_| WorkerError::Message("Worker channel closed".into()))
}

// ---------------------------------------------------------------------------
// Worker-thread ops (registered in `host_v8_worker_inner`)
// ---------------------------------------------------------------------------

/// Send a message from the worker to the main thread.
#[op2(fast)]
fn op_worker_inner_post_message(
    state: &mut OpState,
    #[string] json_message: String,
) -> Result<(), WorkerError> {
    if json_message.len() > MAX_WORKER_MESSAGE_BYTES {
        return Err(WorkerError::Message(format!(
            "Worker message too large: {} bytes (max {} bytes)",
            json_message.len(),
            MAX_WORKER_MESSAGE_BYTES
        )));
    }

    info!(
        "[Worker] worker->main postMessage: {} bytes",
        json_message.len()
    );
    let ctx = state.borrow::<WorkerCtx>();
    ctx.tx_to_main
        .send(json_message)
        .map_err(|_| WorkerError::Message("Main thread channel closed".into()))
}

fn worker_message_to_inbound(message: Option<WorkerMessage>) -> Option<WorkerInbound> {
    match message {
        Some(WorkerMessage::Message(json)) => {
            info!("[Worker] worker received from main: {} bytes", json.len());
            Some(WorkerInbound::Message { data: json })
        }
        Some(WorkerMessage::Binary(data)) => {
            // Encode binary as JSON with base64 payload so JS can reconstruct
            info!("[Worker] worker received binary: {} bytes", data.len());
            let encoded = deno_core::serde_json::json!({
                "__binary": true,
                "base64": base64_encode(&data),
                "byteLength": data.len()
            });
            Some(WorkerInbound::Message {
                data: encoded.to_string(),
            })
        }
        Some(WorkerMessage::Terminate) => {
            info!("[Worker] worker received Terminate signal");
            None
        }
        None => {
            info!("[Worker] worker channel closed (None)");
            None
        }
    }
}

async fn recv_worker_inbound(ctx: &WorkerCtx) -> Result<Option<WorkerInbound>, WorkerError> {
    let mut lifecycle = ctx.timer_backgrounded_rx.lock().await;
    let mut messages = ctx.rx_from_main.lock().await;

    if let Ok(transition) = lifecycle.try_recv() {
        return Ok(Some(worker_lifecycle_to_inbound(transition)));
    }

    tokio::select! {
        biased;
        transition = lifecycle.recv() => {
            match transition {
                Some(transition) => Ok(Some(worker_lifecycle_to_inbound(transition))),
                None => Ok(worker_message_to_inbound(messages.recv().await)),
            }
        }
        message = messages.recv() => Ok(worker_message_to_inbound(message)),
    }
}

fn worker_lifecycle_to_inbound(transition: WorkerTimerLifecycleTransition) -> WorkerInbound {
    let elapsed = tokio::time::Instant::now().saturating_duration_since(transition.occurred_at);
    WorkerInbound::Lifecycle {
        backgrounded: transition.backgrounded,
        elapsed_micros: elapsed.as_micros().min(u64::MAX as u128) as u64,
    }
}

/// Async op: wait for an internal lifecycle event or a user message.
/// Returns `None` when a Terminate signal is received.
#[op2(async(lazy), fast)]
#[serde]
async fn op_worker_inner_recv_message(
    state: Rc<RefCell<OpState>>,
) -> Result<Option<WorkerInbound>, WorkerError> {
    let ctx = {
        let st = state.borrow();
        let ctx = st.borrow::<WorkerCtx>();
        WorkerCtx {
            tx_to_main: ctx.tx_to_main.clone(),
            tx_errors: ctx.tx_errors.clone(),
            rx_from_main: ctx.rx_from_main.clone(),
            timer_backgrounded_rx: ctx.timer_backgrounded_rx.clone(),
        }
    };

    info!("[Worker] worker waiting for main message or lifecycle...");
    recv_worker_inbound(&ctx).await
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Simple base64 encoder (no external dependency).
fn base64_encode(data: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(CHARS[((triple >> 18) & 0x3F) as usize] as char);
        out.push(CHARS[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            out.push(CHARS[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(CHARS[(triple & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Extension declarations
// ---------------------------------------------------------------------------

deno_core::extension!(
    host_v8_worker,
    deps = [host_v8_base],
    ops = [
        op_worker_create,
        op_worker_post_message,
        op_worker_transfer_buffer,
        op_worker_recv_message,
        op_worker_recv_error,
        op_worker_terminate,
    ],
    esm_entry_point = "ext:host_v8_worker/99_global_scope.js",
    esm = [
        dir "src/worker",
        "01_worker.js",
        "99_global_scope.js",
    ],
);

deno_core::extension!(
    host_v8_worker_inner,
    ops = [
        op_worker_inner_post_message,
        op_worker_inner_recv_message,
    ],
    esm = [
        dir "src/worker",
        "02_worker_inner.js",
    ],
    options = {
        ctx: WorkerCtx,
    },
    state = |state, options| {
        state.put::<WorkerCtx>(options.ctx);
    },
);

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

pub fn worker_extensions() -> Vec<Extension> {
    vec![host_v8_worker::init()]
}

pub fn worker_lazy_extensions() -> Vec<Extension> {
    vec![host_v8_worker::lazy_init()]
}

pub fn worker_inner_extensions(ctx: WorkerCtx) -> Vec<Extension> {
    vec![host_v8_worker_inner::init(ctx)]
}

pub(crate) fn set_timer_backgrounded(state: &mut OpState, backgrounded: bool) {
    let Some(handle) = state.try_borrow::<WorkerHandle>() else {
        return;
    };
    if handle.terminated {
        return;
    }
    let _ = handle
        .timer_backgrounded_tx
        .send(WorkerTimerLifecycleTransition::now(backgrounded));
}

/// Create the full extension set for a worker JsRuntime.
///
/// Includes all extensions needed by `98_global_scope_shared.js`:
/// base, console, event, utility, file, rendering (webgl/image), web, url, network.
/// This gives workers the same shared APIs as the main thread.
pub fn create_worker_runtime_extensions(ctx: WorkerCtx, host_state: HostOpState) -> Vec<Extension> {
    use crate::{
        base, console, env, event, file, io_state, lifecycle, network, rendering, url, utility,
        web, worker_runtime,
    };

    let mut exts: Vec<Extension> = Vec::new();

    exts.extend(base::base_extensions(host_state));
    exts.extend(io_state::io_state_extensions());
    exts.extend(console::console_extensions());
    exts.extend(event::event_extensions());
    exts.extend(utility::utility_extensions());
    exts.extend(file::file_extensions());
    exts.extend(rendering::rendering_extensions());
    exts.extend(web::web_extensions());
    exts.extend(url::url_extensions());
    exts.extend(network::network_extensions());

    #[cfg(feature = "api-media")]
    {
        // host_v8_audio depends on host_v8_lifecycle, so it must be registered
        // first (its esm imports onHide/onShow from the lifecycle module).
        exts.extend(lifecycle::lifecycle_extensions());
        exts.extend(crate::audio::audio_extensions());
    }

    exts.extend(env::env_extensions());
    exts.extend(worker_inner_extensions(ctx));
    exts.push(worker_runtime::init());

    exts
}

/// Create the exact Worker extension chain in lazy-init mode for snapshot
/// creation/restoration. Keep this order byte-identical to
/// [`create_worker_runtime_extensions`] and [`create_worker_runtime_extension_args`].
pub(crate) fn create_worker_runtime_lazy_extensions() -> Vec<Extension> {
    use crate::{
        base, console, env, event, file, io_state, lifecycle, network, rendering, url, utility,
        web, worker_runtime,
    };

    let mut exts = vec![
        base::host_v8_base::lazy_init(),
        io_state::host_v8_io_state::lazy_init(),
    ];
    exts.extend(console::console_lazy_extensions());
    exts.extend(event::event_lazy_extensions());
    exts.extend(utility::utility_lazy_extensions());
    exts.extend(file::file_lazy_extensions());
    exts.extend(rendering::rendering_lazy_extensions());
    exts.extend(web::web_lazy_extensions());
    exts.extend(url::url_lazy_extensions());
    exts.extend(network::network_lazy_extensions());

    #[cfg(feature = "api-media")]
    {
        exts.push(lifecycle::host_v8_lifecycle::lazy_init());
        exts.extend(crate::audio::audio_lazy_extensions());
    }

    exts.push(env::host_v8_env::lazy_init());
    exts.push(host_v8_worker_inner::lazy_init());
    exts.push(worker_runtime::lazy_init());
    exts
}

/// Runtime-only state callbacks for a restored Worker snapshot.
pub(crate) fn create_worker_runtime_extension_args(
    ctx: WorkerCtx,
    host_state: HostOpState,
) -> Vec<ExtensionArguments> {
    use crate::{
        base, console, env, event, file, io_state, lifecycle, network, rendering, url, utility,
        web, worker_runtime,
    };

    let mut args = vec![
        base::host_v8_base::args(host_state),
        io_state::host_v8_io_state::args(),
        console::host_v8_console::args(),
        event::host_v8_event::args(),
        utility::host_v8_utility::args(),
        file::host_v8_file::args(),
        rendering::image::host_v8_image::args(),
        rendering::webgl::host_v8_webgl::args(),
        web::host_v8_web::args(),
        url::host_v8_url::args(),
        network::network_extension_args(),
    ];

    #[cfg(feature = "api-media")]
    {
        args.push(lifecycle::host_v8_lifecycle::args());
        args.push(crate::audio::host_v8_audio::args());
    }

    args.push(env::host_v8_env::args());
    args.push(host_v8_worker_inner::args(ctx));
    args.push(worker_runtime::args());
    args
}

// ---------------------------------------------------------------------------
// Worker thread spawn
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Runaway protection via the process deadline watchdog
// ---------------------------------------------------------------------------

/// Max time a Worker may run untrusted JS without yielding before it is
/// force-terminated. Matches the host ANR budget: generous enough for module
/// compilation on low-end devices, tight enough to catch a `while (true)`.
const WORKER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// The single JSON error reported (exactly once) when the deadline watchdog
/// terminates a runaway Worker.
const WORKER_TIMEOUT_MSG: &str =
    r#"{"message":"Worker terminated: unresponsive (watchdog timeout)"}"#;

/// Report a Worker error unless the deadline watchdog already fired. On a
/// timeout the watchdog observer sends [`WORKER_TIMEOUT_MSG`] exactly once, so
/// the load/eval/event-loop error paths must not additionally report the
/// resulting "execution terminated" error.
fn report_worker_error(
    watchdog: Option<&DeadlineWatchdog>,
    tx_errors: &mpsc::UnboundedSender<String>,
    message: String,
) {
    if watchdog.is_some_and(DeadlineWatchdog::timed_out) {
        return;
    }
    let _ = tx_errors.send(message);
}

fn worker_initialization_error(stage: &str, error: &dyn std::fmt::Display) -> String {
    deno_core::serde_json::json!({
        "message": format!("Worker {stage} failed: {error}")
    })
    .to_string()
}

/// Guard every poll of module loading (V8 parse/compile of untrusted code counts
/// against the budget).
async fn worker_load_module(
    rt: &mut JsRuntime,
    watchdog: Option<&DeadlineWatchdog>,
    resolved: &deno_core::ModuleSpecifier,
) -> Result<deno_core::ModuleId, String> {
    let mut load = std::pin::pin!(rt.load_main_es_module(resolved));
    std::future::poll_fn(|cx| crate::watchdog::poll_guarded(watchdog, load.as_mut(), cx))
        .await
        .map_err(|e| e.to_string())
}

/// Guard the SYNCHRONOUS `mod_evaluate` constructor — deno_core 0.385 enters V8
/// to run the module top-level while building the returned future, so a
/// top-level `while (true) {}` must be covered here, not only in later polls —
/// plus every poll of the evaluation future and the event loop.
async fn worker_evaluate_module(
    rt: &mut JsRuntime,
    watchdog: Option<&DeadlineWatchdog>,
    module_id: deno_core::ModuleId,
) -> Result<(), String> {
    let evaluation = {
        let _scope = watchdog.map(DeadlineWatchdog::enter);
        rt.mod_evaluate(module_id)
    };
    let mut evaluation = std::pin::pin!(evaluation);
    std::future::poll_fn(move |cx| -> std::task::Poll<Result<(), String>> {
        let _scope = watchdog.map(DeadlineWatchdog::enter);
        if let std::task::Poll::Ready(res) = evaluation.as_mut().poll(cx) {
            return std::task::Poll::Ready(res.map_err(|e| e.to_string()));
        }
        match rt.poll_event_loop(cx, PollEventLoopOptions::default()) {
            std::task::Poll::Ready(Err(e)) => std::task::Poll::Ready(Err(e.to_string())),
            _ => std::task::Poll::Pending,
        }
    })
    .await
}

/// Guard every poll of the Worker event loop. The Worker message pump runs here,
/// so a runaway message handler is force-terminated on the next poll boundary.
async fn worker_run_event_loop(
    rt: &mut JsRuntime,
    watchdog: Option<&DeadlineWatchdog>,
) -> Result<(), String> {
    std::future::poll_fn(move |cx| {
        let _scope = watchdog.map(DeadlineWatchdog::enter);
        rt.poll_event_loop(cx, PollEventLoopOptions::default())
    })
    .await
    .map_err(|e| e.to_string())
}

/// Start the Worker receive loop and remove snapshot/bootstrap internals before
/// the isolate becomes reachable from another thread or game code is loaded.
fn initialize_worker_runtime(rt: &mut JsRuntime) -> Result<(), String> {
    rt.execute_script(
        "ext:worker_runtime/initialize_runtime.js",
        deno_core::FastString::from_static(
            r#"(() => {
                const start = globalThis.__migoStartWorkerMessagePump;
                if (typeof start !== "function") {
                    throw new Error("Worker message-pump bootstrap hook is missing");
                }
                start();
                if (!delete globalThis.__migoStartWorkerMessagePump ||
                    !delete globalThis.Deno ||
                    !delete globalThis.__bootstrap) {
                    throw new Error("Worker bootstrap globals could not be removed");
                }
                if ("__migoStartWorkerMessagePump" in globalThis ||
                    "Deno" in globalThis || "__bootstrap" in globalThis) {
                    throw new Error("Worker bootstrap globals remain visible");
                }
            })();"#,
        ),
    )
    .map(|_| ())
    .map_err(|error| error.to_string())
}

fn spawn_worker_thread(
    script_path: String,
    code_dir: String,
    ctx: WorkerCtx,
    host_state: HostOpState,
    sab_store: SharedArrayBufferStore,
    isolate_handle_slot: Arc<std::sync::Mutex<Option<v8::IsolateHandle>>>,
) -> Result<std::thread::JoinHandle<()>, WorkerError> {
    let tx_errors = ctx.tx_errors.clone();

    std::thread::Builder::new()
        .name("Migo-Worker".into())
        .spawn(move || {
            // Clone kept in the outer scope so we can clear the published isolate
            // handle once the thread exits (any path). `run` moves the original
            // clone into its async block. Clearing the slot makes a post-exit
            // `force_terminate` an observable no-op and aids state inspection.
            let slot_for_cleanup = isolate_handle_slot.clone();
            let run = || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_io()
                    .enable_time()
                    .event_interval(61)
                    .global_queue_interval(31)
                    .max_io_events_per_tick(1024)
                    .max_blocking_threads(2)
                    .build()
                    .expect("Failed to create worker tokio runtime");

                runtime.block_on(async move {
                    let worker_mount_table = host_state.mount_table.clone();
                    let snapshot_bytes = crate::snapshot::WORKER_SNAPSHOT_BYTES;
                    let use_snapshot = snapshot_bytes.is_some();
                    let (exts, extension_args) = if use_snapshot {
                        (
                            create_worker_runtime_lazy_extensions(),
                            Some(create_worker_runtime_extension_args(ctx, host_state)),
                        )
                    } else {
                        (
                            create_worker_runtime_extensions(ctx, host_state),
                            None,
                        )
                    };

                    let module_loader: Option<Rc<dyn ModuleLoader>> =
                        Some(Rc::new(WorkerModuleLoader {
                            inner: FsModuleLoader,
                            mount_table: worker_mount_table,
                        }));

                    // Apply the same V8 heap limits as the main thread to prevent
                    // worker code from OOM-ing the entire process.
                    let v8_limits = crate::V8LimitsConfig::default();
                    let create_params = Some(
                        v8::Isolate::create_params()
                            .heap_limits(v8_limits.initial_heap_size, v8_limits.max_heap_size),
                    );

                    info!(
                        "[Worker] creating JsRuntime with {} extensions (snapshot={})",
                        exts.len(),
                        use_snapshot
                    );
                    let mut rt = JsRuntime::new(RuntimeOptions {
                        module_loader,
                        extensions: exts,
                        create_params,
                        startup_snapshot: snapshot_bytes,
                        shared_array_buffer_store: Some(sab_store),
                        skip_op_registration: use_snapshot,
                        ..Default::default()
                    });
                    info!("[Worker] JsRuntime created successfully");

                    if let Some(extension_args) = extension_args {
                        if let Err(error) = rt.lazy_init_extensions(extension_args) {
                            error!("[Worker] snapshot state initialization failed: {error}");
                            let _ = tx_errors.send(worker_initialization_error(
                                "snapshot state initialization",
                                &error,
                            ));
                            return;
                        }
                    }

                    if let Err(error) = initialize_worker_runtime(&mut rt) {
                        error!("[Worker] runtime bootstrap failed: {error}");
                        let _ = tx_errors
                            .send(worker_initialization_error("runtime bootstrap", &error));
                        return;
                    }

                    // Publish the isolate handle so the main thread can forcibly
                    // terminate this worker (WorkerHandle::force_terminate) even
                    // if it's stuck in a runaway JS loop that never awaits the
                    // cooperative Terminate message.
                    if let Ok(mut slot) = isolate_handle_slot.lock() {
                        *slot = Some(rt.v8_isolate().thread_safe_handle());
                    }

                    // Register near-heap-limit callback for OOM protection
                    {
                        let hard_cap = v8_limits.max_heap_size.saturating_add(8 * 1024 * 1024);
                        let oom_handle = rt.v8_isolate().thread_safe_handle();
                        let oom_fired = Arc::new(std::sync::atomic::AtomicBool::new(false));
                        let cb_fired = Arc::clone(&oom_fired);
                        let cb_tx = tx_errors.clone();

                        rt.add_near_heap_limit_callback(move |current_limit, _initial_limit| {
                            let first = cb_fired
                                .compare_exchange(
                                    false,
                                    true,
                                    std::sync::atomic::Ordering::SeqCst,
                                    std::sync::atomic::Ordering::SeqCst,
                                )
                                .is_ok();
                            if first {
                                warn!("[Worker] V8 heap limit reached, terminating");
                                oom_handle.terminate_execution();
                                let _ = cb_tx.send(
                                    r#"{"message":"Worker terminated: V8 heap limit exceeded"}"#
                                        .to_string(),
                                );
                            }
                            current_limit.saturating_add(1024 * 1024).min(hard_cap)
                        });
                    }

                    // Register with the ONE process deadline watchdog. This
                    // replaces the per-Worker one-second ticker + two-second
                    // monitor OS thread; the observer reports the timeout exactly
                    // once on `tx_errors`. Unconditional (not `v8-limits`-gated):
                    // Worker runaway protection exists even under
                    // `--no-default-features`. Declared after `rt` so it disarms +
                    // unregisters (RAII) before `rt`/the isolate drops on every
                    // exit path (success, error, panic).
                    let watchdog = {
                        let handle = rt.v8_isolate().thread_safe_handle();
                        let tx = tx_errors.clone();
                        let config = DeadlineWatchdogConfig::new(WORKER_TIMEOUT, "worker")
                            .with_observer(Arc::new(move |_| {
                                let _ = tx.send(WORKER_TIMEOUT_MSG.to_string());
                            }));
                        match DeadlineWatchdog::register_isolate(handle, config) {
                            Ok(w) => Some(w),
                            Err(e) => {
                                warn!(
                                    "[Worker] deadline watchdog unavailable, continuing without runaway protection: {e}"
                                );
                                None
                            }
                        }
                    };

                    // Resolve and load worker script
                    let code_path = std::path::PathBuf::from(&code_dir);
                    let resolved = match resolve_path(&script_path, &code_path) {
                        Ok(r) => r,
                        Err(e) => {
                            error!(
                                "[Worker] failed to resolve worker script '{}' in '{}': {}",
                                script_path, code_dir, e
                            );
                            report_worker_error(
                                watchdog.as_ref(),
                                &tx_errors,
                                format!(r#"{{"message":"Failed to resolve worker script: {}"}}"#, e),
                            );
                            return;
                        }
                    };

                    info!("[Worker] loading main module: {}", resolved);
                    let module_id =
                        match worker_load_module(&mut rt, watchdog.as_ref(), &resolved).await {
                            Ok(id) => id,
                            Err(e) => {
                                error!("[Worker] failed to load worker script: {}", e);
                                report_worker_error(
                                    watchdog.as_ref(),
                                    &tx_errors,
                                    format!(r#"{{"message":"Failed to load worker script: {}"}}"#, e),
                                );
                                return;
                            }
                        };

                    info!("[Worker] module loaded (id={}), evaluating...", module_id);
                    if let Err(e) = worker_evaluate_module(&mut rt, watchdog.as_ref(), module_id).await
                    {
                        error!("[Worker] worker script evaluation error: {}", e);
                        report_worker_error(
                            watchdog.as_ref(),
                            &tx_errors,
                            format!(r#"{{"message":"Worker script evaluation error: {}"}}"#, e),
                        );
                        return;
                    }

                    info!("[Worker] module evaluated, running event loop");
                    // Run event loop until it completes (message pump op keeps it alive)
                    if let Err(e) = worker_run_event_loop(&mut rt, watchdog.as_ref()).await {
                        error!("[Worker] event loop error: {}", e);
                        report_worker_error(
                            watchdog.as_ref(),
                            &tx_errors,
                            format!(r#"{{"message":"Worker event loop error: {}"}}"#, e),
                        );
                    }

                    info!("[Worker] thread exiting cleanly");
                });
            };

            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(run));
            if let Err(panic_info) = result {
                let panic_msg = panic_info
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| panic_info.downcast_ref::<&str>().map(|s| s.to_string()))
                    .unwrap_or_else(|| "Unknown panic".to_string());

                error!("[Worker] panicked: {}", panic_msg);
            }

            // Thread is exiting (clean, error, or panic): drop the published
            // isolate handle so the main thread stops holding a handle to a dead
            // isolate.
            if let Ok(mut slot) = slot_for_cleanup.lock() {
                *slot = None;
            }
        })
        .map_err(|e| WorkerError::Message(format!("Failed to spawn worker thread: {e}")))
}

#[cfg(test)]
mod timer_lifecycle_tests {
    use super::*;
    use std::{path::PathBuf, sync::atomic::AtomicBool, time::Duration};

    use deno_core::{FastString, PollEventLoopOptions, RuntimeOptions};
    use futures::future::poll_fn;
    use shared::{
        channel::ThreadWakeup,
        device::gpu_caps::GpuCaps,
        op_state::{AudioSender, NetworkPolicy},
        render_command_sender::CommandSender,
    };
    fn test_host_state(timer_backgrounded: Arc<AtomicBool>) -> HostOpState {
        let (render_tx, _render_rx) = CommandSender::new();
        let (audio_raw_tx, _audio_rx) = mpsc::unbounded_channel();
        let (host_tx, _critical_host_tx, _host_rx) = shared::host_channel::channel(1);

        HostOpState {
            id: 1,
            app_cache_dir: PathBuf::from("/tmp/cache"),
            app_files_dir: PathBuf::from("/tmp/files"),
            code_dir: None,
            game_paths: None,
            vfs: None,
            mount_table: None,
            render_tx,
            text_measurer: None,
            audio_tx: AudioSender::new(audio_raw_tx, ThreadWakeup::new()),
            host_tx,
            device_services: None,
            raf_rx: None,
            raf_demand: std::sync::Arc::new(shared::raf_signal::RafDemand::new()),
            request_vsync: None,
            sub_packages: Vec::new(),
            workers_path: None,
            network_policy: NetworkPolicy::default(),
            backgrounded: Arc::new(AtomicBool::new(false)),
            timer_backgrounded,
            webgl_context_created: Arc::new(AtomicBool::new(false)),
            context_lost: Arc::new(shared::op_state::ContextLostState::default()),
            code_signing_enabled: false,
            gpu_caps: GpuCaps::new(),
        }
    }

    fn test_worker_ctx(
        rx_from_main: mpsc::UnboundedReceiver<WorkerMessage>,
        timer_backgrounded_rx: mpsc::UnboundedReceiver<WorkerTimerLifecycleTransition>,
    ) -> WorkerCtx {
        let (tx_to_main, _rx_to_main) = mpsc::unbounded_channel();
        let (tx_errors, _rx_errors) = mpsc::unbounded_channel();
        WorkerCtx {
            tx_to_main,
            tx_errors,
            rx_from_main: Arc::new(tokio::sync::Mutex::new(rx_from_main)),
            timer_backgrounded_rx: Arc::new(tokio::sync::Mutex::new(timer_backgrounded_rx)),
        }
    }

    fn exec(rt: &mut JsRuntime, source: impl Into<String>) {
        rt.execute_script("<test:worker-timer>", FastString::from(source.into()))
            .expect("worker timer script");
    }

    fn assert_js(rt: &mut JsRuntime, expression: &str) {
        exec(
            rt,
            format!(
                "if (!({expression})) throw new Error('worker timer assertion failed: ' + ({expression}));"
            ),
        );
    }

    async fn poll_once(rt: &mut JsRuntime) {
        poll_fn(|cx| {
            let _ = rt.poll_event_loop(cx, PollEventLoopOptions::default());
            std::task::Poll::Ready(())
        })
        .await;
    }

    async fn drain_ready(rt: &mut JsRuntime) {
        for _ in 0..6 {
            poll_once(rt).await;
            tokio::task::yield_now().await;
        }
    }

    async fn advance_and_drain(rt: &mut JsRuntime, duration: Duration) {
        poll_once(rt).await;
        tokio::time::advance(duration).await;
        tokio::time::advance(Duration::from_nanos(1)).await;
        tokio::task::yield_now().await;
        drain_ready(rt).await;
    }

    #[tokio::test(start_paused = true)]
    async fn lifecycle_change_preempts_a_queued_user_message() {
        let (message_tx, message_rx) = mpsc::unbounded_channel();
        message_tx
            .send(WorkerMessage::Message("user".into()))
            .unwrap();
        let (lifecycle_tx, lifecycle_rx) = mpsc::unbounded_channel();
        lifecycle_tx
            .send(WorkerTimerLifecycleTransition::now(true))
            .unwrap();
        let ctx = test_worker_ctx(message_rx, lifecycle_rx);

        assert_eq!(
            recv_worker_inbound(&ctx).await.unwrap(),
            Some(WorkerInbound::Lifecycle {
                backgrounded: true,
                elapsed_micros: 0,
            })
        );
        assert_eq!(
            recv_worker_inbound(&ctx).await.unwrap(),
            Some(WorkerInbound::Message {
                data: "user".into(),
            })
        );
    }

    #[tokio::test(start_paused = true)]
    async fn lifecycle_changes_preserve_every_edge() {
        let (_message_tx, message_rx) = mpsc::unbounded_channel();
        let (lifecycle_tx, lifecycle_rx) = mpsc::unbounded_channel();
        lifecycle_tx
            .send(WorkerTimerLifecycleTransition::now(true))
            .unwrap();
        lifecycle_tx
            .send(WorkerTimerLifecycleTransition::now(false))
            .unwrap();
        lifecycle_tx
            .send(WorkerTimerLifecycleTransition::now(true))
            .unwrap();
        let ctx = test_worker_ctx(message_rx, lifecycle_rx);

        assert_eq!(
            recv_worker_inbound(&ctx).await.unwrap(),
            Some(WorkerInbound::Lifecycle {
                backgrounded: true,
                elapsed_micros: 0,
            })
        );
        assert_eq!(
            tokio::time::timeout(Duration::from_millis(10), recv_worker_inbound(&ctx))
                .await
                .expect("the foreground edge must not be coalesced")
                .unwrap(),
            Some(WorkerInbound::Lifecycle {
                backgrounded: false,
                elapsed_micros: 0,
            })
        );
        assert_eq!(
            tokio::time::timeout(Duration::from_millis(10), recv_worker_inbound(&ctx))
                .await
                .expect("the second background edge must not be coalesced")
                .unwrap(),
            Some(WorkerInbound::Lifecycle {
                backgrounded: true,
                elapsed_micros: 0,
            })
        );
    }

    #[test]
    fn inbound_events_have_non_user_control_shapes() {
        let lifecycle = deno_core::serde_json::to_value(WorkerInbound::Lifecycle {
            backgrounded: true,
            elapsed_micros: 2500,
        })
        .unwrap();
        assert_eq!(lifecycle["type"], "lifecycle");
        assert_eq!(lifecycle["backgrounded"], true);
        assert_eq!(lifecycle["elapsedMicros"], 2500);
        assert!(lifecycle.get("data").is_none());

        let message = deno_core::serde_json::to_value(WorkerInbound::Message {
            data: "payload".into(),
        })
        .unwrap();
        assert_eq!(message["type"], "message");
        assert_eq!(message["data"], "payload");
        assert!(message.get("backgrounded").is_none());
    }

    #[test]
    fn worker_initialization_errors_are_valid_json() {
        let encoded = worker_initialization_error("runtime bootstrap", &"bad \"hook\"");
        let decoded: deno_core::serde_json::Value =
            deno_core::serde_json::from_str(&encoded).expect("valid Worker error JSON");
        assert_eq!(
            decoded["message"],
            "Worker runtime bootstrap failed: bad \"hook\""
        );
    }

    #[test]
    fn worker_snapshot_extensions_match_eager_and_argument_order() {
        let (_eager_message_tx, eager_message_rx) = mpsc::unbounded_channel();
        let (_eager_lifecycle_tx, eager_lifecycle_rx) = mpsc::unbounded_channel();
        let eager_names: Vec<_> = create_worker_runtime_extensions(
            test_worker_ctx(eager_message_rx, eager_lifecycle_rx),
            test_host_state(Arc::new(AtomicBool::new(false))),
        )
        .into_iter()
        .map(|extension| extension.name)
        .collect();

        let lazy_names: Vec<_> = create_worker_runtime_lazy_extensions()
            .into_iter()
            .map(|extension| extension.name)
            .collect();

        let (_args_message_tx, args_message_rx) = mpsc::unbounded_channel();
        let (_args_lifecycle_tx, args_lifecycle_rx) = mpsc::unbounded_channel();
        let argument_names: Vec<_> = create_worker_runtime_extension_args(
            test_worker_ctx(args_message_rx, args_lifecycle_rx),
            test_host_state(Arc::new(AtomicBool::new(false))),
        )
        .into_iter()
        .map(|arguments| arguments.name)
        .collect();

        assert_eq!(lazy_names, eager_names);
        assert_eq!(argument_names, eager_names);
    }

    #[tokio::test(start_paused = true)]
    async fn worker_pump_consumes_lifecycle_and_freezes_timer_remainder() {
        let (message_tx, message_rx) = mpsc::unbounded_channel();
        let (lifecycle_tx, lifecycle_rx) = mpsc::unbounded_channel();
        let timer_backgrounded = Arc::new(AtomicBool::new(false));
        let ctx = test_worker_ctx(message_rx, lifecycle_rx);
        let mut rt = JsRuntime::new(RuntimeOptions {
            extensions: create_worker_runtime_extensions(
                ctx,
                test_host_state(timer_backgrounded.clone()),
            ),
            ..Default::default()
        });
        initialize_worker_runtime(&mut rt).expect("worker runtime bootstrap");

        exec(
            &mut rt,
            "globalThis.__workerMessages = 0; \
             globalThis.__workerTimer = 0; \
             worker.onMessage(() => __workerMessages++); \
             setTimeout(() => __workerTimer++, 100)",
        );
        advance_and_drain(&mut rt, Duration::from_millis(30)).await;

        timer_backgrounded.store(true, std::sync::atomic::Ordering::Release);
        lifecycle_tx
            .send(WorkerTimerLifecycleTransition::now(true))
            .unwrap();
        drain_ready(&mut rt).await;
        advance_and_drain(&mut rt, Duration::from_secs(10)).await;
        assert_js(&mut rt, "__workerTimer === 0 && __workerMessages === 0");

        timer_backgrounded.store(false, std::sync::atomic::Ordering::Release);
        lifecycle_tx
            .send(WorkerTimerLifecycleTransition::now(false))
            .unwrap();
        drain_ready(&mut rt).await;
        advance_and_drain(&mut rt, Duration::from_millis(69)).await;
        assert_js(&mut rt, "__workerTimer === 0 && __workerMessages === 0");
        advance_and_drain(&mut rt, Duration::from_millis(1)).await;
        assert_js(&mut rt, "__workerTimer === 1 && __workerMessages === 0");

        drop(message_tx);
    }

    #[tokio::test(start_paused = true)]
    async fn worker_pump_uses_transition_time_when_lifecycle_delivery_is_delayed() {
        let (message_tx, message_rx) = mpsc::unbounded_channel();
        let (lifecycle_tx, lifecycle_rx) = mpsc::unbounded_channel();
        let timer_backgrounded = Arc::new(AtomicBool::new(false));
        let ctx = test_worker_ctx(message_rx, lifecycle_rx);
        let mut rt = JsRuntime::new(RuntimeOptions {
            extensions: create_worker_runtime_extensions(
                ctx,
                test_host_state(timer_backgrounded.clone()),
            ),
            ..Default::default()
        });
        initialize_worker_runtime(&mut rt).expect("worker runtime bootstrap");

        exec(
            &mut rt,
            "globalThis.__workerMessages = 0; \
             globalThis.__workerTimer = 0; \
             worker.onMessage(() => __workerMessages++); \
             setTimeout(() => __workerTimer++, 100)",
        );
        advance_and_drain(&mut rt, Duration::from_millis(30)).await;

        timer_backgrounded.store(true, std::sync::atomic::Ordering::Release);
        lifecycle_tx
            .send(WorkerTimerLifecycleTransition::now(true))
            .unwrap();
        tokio::time::advance(Duration::from_secs(10)).await;
        timer_backgrounded.store(false, std::sync::atomic::Ordering::Release);
        lifecycle_tx
            .send(WorkerTimerLifecycleTransition::now(false))
            .unwrap();
        drain_ready(&mut rt).await;

        assert_js(&mut rt, "__workerTimer === 0 && __workerMessages === 0");
        advance_and_drain(&mut rt, Duration::from_millis(69)).await;
        assert_js(&mut rt, "__workerTimer === 0 && __workerMessages === 0");
        advance_and_drain(&mut rt, Duration::from_millis(1)).await;
        assert_js(&mut rt, "__workerTimer === 1 && __workerMessages === 0");

        drop(message_tx);
    }

    #[tokio::test(start_paused = true)]
    async fn worker_created_hidden_keeps_its_first_timer_logical_until_show() {
        let (message_tx, message_rx) = mpsc::unbounded_channel();
        let (lifecycle_tx, lifecycle_rx) = mpsc::unbounded_channel();
        let timer_backgrounded = Arc::new(AtomicBool::new(true));
        let ctx = test_worker_ctx(message_rx, lifecycle_rx);
        let mut rt = JsRuntime::new(RuntimeOptions {
            extensions: create_worker_runtime_extensions(
                ctx,
                test_host_state(timer_backgrounded.clone()),
            ),
            ..Default::default()
        });
        initialize_worker_runtime(&mut rt).expect("worker runtime bootstrap");

        exec(
            &mut rt,
            "globalThis.__workerTimer = 0; setTimeout(() => __workerTimer++, 100)",
        );
        advance_and_drain(&mut rt, Duration::from_secs(10)).await;
        assert_js(&mut rt, "__workerTimer === 0");

        // A Worker created after the host entered the background has no prior
        // hide edge in its per-worker queue. The shared level initializes its
        // timer registry; the first queued edge is therefore show.
        timer_backgrounded.store(false, std::sync::atomic::Ordering::Release);
        lifecycle_tx
            .send(WorkerTimerLifecycleTransition::now(false))
            .unwrap();
        drain_ready(&mut rt).await;
        advance_and_drain(&mut rt, Duration::from_millis(99)).await;
        assert_js(&mut rt, "__workerTimer === 0");
        advance_and_drain(&mut rt, Duration::from_millis(1)).await;
        assert_js(&mut rt, "__workerTimer === 1");

        drop(message_tx);
    }
}

/// R4: Worker runaway protection via the one process deadline watchdog.
#[cfg(all(test, feature = "v8-limits"))]
mod watchdog_worker_tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::AtomicBool;
    use std::time::Duration;

    use deno_core::{FastString, JsRuntime, RuntimeOptions};
    use shared::{
        channel::ThreadWakeup,
        device::gpu_caps::GpuCaps,
        op_state::{AudioSender, HostOpState, NetworkPolicy},
        render_command_sender::CommandSender,
    };

    use crate::watchdog::{DeadlineWatchdog, DeadlineWatchdogConfig, Scheduler};

    fn wt_host_state() -> HostOpState {
        let (render_tx, _render_rx) = CommandSender::new();
        let (audio_raw_tx, _audio_rx) = mpsc::unbounded_channel();
        let (host_tx, _critical_host_tx, _host_rx) = shared::host_channel::channel(1);
        HostOpState {
            id: 1,
            app_cache_dir: PathBuf::from("/tmp/cache"),
            app_files_dir: PathBuf::from("/tmp/files"),
            code_dir: None,
            game_paths: None,
            vfs: None,
            mount_table: None,
            render_tx,
            text_measurer: None,
            audio_tx: AudioSender::new(audio_raw_tx, ThreadWakeup::new()),
            host_tx,
            device_services: None,
            raf_rx: None,
            raf_demand: std::sync::Arc::new(shared::raf_signal::RafDemand::new()),
            request_vsync: None,
            sub_packages: Vec::new(),
            workers_path: None,
            network_policy: NetworkPolicy::default(),
            backgrounded: Arc::new(AtomicBool::new(false)),
            timer_backgrounded: Arc::new(AtomicBool::new(false)),
            webgl_context_created: Arc::new(AtomicBool::new(false)),
            context_lost: Arc::new(shared::op_state::ContextLostState::default()),
            code_signing_enabled: false,
            gpu_caps: GpuCaps::new(),
        }
    }

    fn build_worker_rt(loader: Option<Rc<dyn ModuleLoader>>) -> JsRuntime {
        let (_tx, rx_from_main) = mpsc::unbounded_channel::<WorkerMessage>();
        let (_ltx, lifecycle_rx) = mpsc::unbounded_channel::<WorkerTimerLifecycleTransition>();
        let (tx_to_main, _rx_to_main) = mpsc::unbounded_channel();
        let (tx_errors, _rx_errors) = mpsc::unbounded_channel();
        let ctx = WorkerCtx {
            tx_to_main,
            tx_errors,
            rx_from_main: Arc::new(tokio::sync::Mutex::new(rx_from_main)),
            timer_backgrounded_rx: Arc::new(tokio::sync::Mutex::new(lifecycle_rx)),
        };
        let mut rt = JsRuntime::new(RuntimeOptions {
            module_loader: loader,
            extensions: create_worker_runtime_extensions(ctx, wt_host_state()),
            ..Default::default()
        });
        initialize_worker_runtime(&mut rt).expect("worker runtime bootstrap");
        rt
    }

    fn unique_dir(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("migo-worker-wd-{tag}-{nanos}"))
    }

    #[tokio::test]
    async fn worker_top_level_infinite_loop_is_terminated_and_reports_once() {
        let dir = unique_dir("toplevel");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("main.js"), "while (true) {}").unwrap();

        let mut rt = build_worker_rt(Some(Rc::new(FsModuleLoader)));
        let sched = Scheduler::new_test();
        let (err_tx, mut err_rx) = mpsc::unbounded_channel::<String>();
        let obs_tx = err_tx.clone();
        let config = DeadlineWatchdogConfig::new(Duration::from_millis(200), "worker-test")
            .with_observer(Arc::new(move |_| {
                let _ = obs_tx.send("watchdog-timeout".to_string());
            }));
        let wd = DeadlineWatchdog::register_isolate_on(
            sched,
            rt.v8_isolate().thread_safe_handle(),
            config,
        );

        let resolved = resolve_path("main.js", &dir).unwrap();
        let module_id = worker_load_module(&mut rt, Some(&wd), &resolved)
            .await
            .expect("module load (compile) succeeds; top-level runs during evaluate");
        let eval = worker_evaluate_module(&mut rt, Some(&wd), module_id).await;
        assert!(
            eval.is_err(),
            "a top-level infinite loop must be terminated"
        );
        assert!(wd.timed_out());

        // Mirror the run block: the eval error is suppressed because the observer
        // already reported the timeout exactly once.
        if let Err(e) = eval {
            report_worker_error(Some(&wd), &err_tx, format!("eval error: {e}"));
        }
        let first = tokio::time::timeout(Duration::from_secs(2), err_rx.recv())
            .await
            .expect("the observer must report the timeout")
            .expect("message present");
        assert_eq!(first, "watchdog-timeout");
        assert!(
            err_rx.try_recv().is_err(),
            "exactly one timeout error; the eval error must be suppressed"
        );
    }

    #[tokio::test]
    async fn worker_message_handler_infinite_loop_is_terminated() {
        let mut rt = build_worker_rt(None);
        let sched = Scheduler::new_test();
        let config = DeadlineWatchdogConfig::new(Duration::from_millis(200), "worker-test");
        let wd = DeadlineWatchdog::register_isolate_on(
            sched,
            rt.v8_isolate().thread_safe_handle(),
            config,
        );

        // A macrotask that never yields; it runs inside the guarded event loop,
        // not during the (unguarded) setup script.
        rt.execute_script(
            "<setup>",
            FastString::from_static("setTimeout(() => { while (true) {} }, 0);"),
        )
        .unwrap();

        let result = worker_run_event_loop(&mut rt, Some(&wd)).await;
        assert!(
            result.is_err(),
            "a runaway handler running in the event loop must be terminated"
        );
        assert!(wd.timed_out());
    }

    #[tokio::test]
    async fn worker_watchdog_unregisters_on_drop() {
        let mut rt = build_worker_rt(None);
        let sched = Scheduler::new_test();
        let wd = DeadlineWatchdog::register_isolate_on(
            sched,
            rt.v8_isolate().thread_safe_handle(),
            DeadlineWatchdogConfig::new(Duration::from_secs(10), "worker-test"),
        );
        assert_eq!(sched.registered_len(), 1);
        drop(wd);
        assert_eq!(
            sched.registered_len(),
            0,
            "dropping the watchdog must unregister the target (no leak on any exit path)"
        );
    }

    #[test]
    fn worker_uses_shared_scheduler_not_a_local_monitor() {
        let src = include_str!("mod.rs");
        // Forbidden symbols (split via concat! so this test's own text does not
        // match the needle).
        assert!(
            !src.contains(concat!("Migo-", "WorkerWatchdog")),
            "the per-Worker monitor thread must be gone"
        );
        assert!(
            !src.contains(concat!("WORKER_WATCHDOG_", "CHECK_INTERVAL")),
            "the periodic check interval constant must be gone"
        );
        assert!(
            !src.contains(concat!("WORKER_WATCHDOG_", "TIMEOUT")),
            "the old timeout constant must be gone"
        );
        assert!(
            !src.contains(concat!("spawn_worker_", "watchdog")),
            "the per-Worker watchdog spawner must be gone"
        );
        assert!(
            !src.contains(concat!("struct Worker", "Watchdog")),
            "the per-Worker heartbeat struct must be gone"
        );
        assert!(
            !src.contains(concat!("tokio::time::", "interval")),
            "the one-second ticker task must be gone"
        );
        // Required new wiring.
        assert!(
            src.contains(concat!("DeadlineWatchdog::register_", "isolate")),
            "the Worker must register with the shared process scheduler"
        );
        assert!(
            src.contains(concat!("poll_", "guarded")),
            "the Worker must guard its module-load/event-loop polls"
        );
        // Main-thread force-terminate stays independent.
        assert!(
            src.contains(concat!("fn force_", "terminate")),
            "WorkerHandle::force_terminate must remain"
        );
    }
}
