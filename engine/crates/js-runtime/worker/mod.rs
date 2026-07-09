use std::{cell::RefCell, rc::Rc, sync::Arc};

use deno_core::{
    Extension, FsModuleLoader, JsRuntime, ModuleLoader, OpState, PollEventLoopOptions,
    RuntimeOptions, SharedArrayBufferStore, op2, resolve_path, v8,
};
use tokio::sync::mpsc;
use tracing::{error, info, warn};

use shared::op_state::HostOpState;

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

/// Stored in the **main** thread's `OpState` when a worker is active.
pub(crate) struct WorkerHandle {
    tx_to_worker: mpsc::UnboundedSender<WorkerMessage>,
    rx_from_worker: Arc<tokio::sync::Mutex<mpsc::UnboundedReceiver<String>>>,
    rx_errors: Arc<tokio::sync::Mutex<mpsc::UnboundedReceiver<String>>>,
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
                    let finished =
                        h.join_handle.as_ref().map_or(true, |jh| jh.is_finished());
                    if h.terminated && finished {
                        true
                    } else {
                        return Err(WorkerError::Message(
                            "Only one worker can exist at a time. Call terminate() first."
                                .into(),
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
            sub_packages: host.sub_packages.clone(),
            workers_path: host.workers_path.clone(),
            network_policy: host.network_policy.clone(),
            backgrounded: host.backgrounded.clone(),
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

    let worker_ctx = WorkerCtx {
        tx_to_main: tx_worker_to_main,
        tx_errors: tx_worker_errors,
        rx_from_main: Arc::new(tokio::sync::Mutex::new(rx_main_to_worker)),
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

/// Async op: wait for a message from the main thread.
/// Returns `None` when a Terminate signal is received.
#[op2(async(lazy), fast)]
#[string]
async fn op_worker_inner_recv_message(
    state: Rc<RefCell<OpState>>,
) -> Result<Option<String>, WorkerError> {
    let rx = {
        let st = state.borrow();
        st.borrow::<WorkerCtx>().rx_from_main.clone()
    };

    info!("[Worker] worker waiting for main message...");
    let mut guard = rx.lock().await;
    match guard.recv().await {
        Some(WorkerMessage::Message(json)) => {
            info!("[Worker] worker received from main: {} bytes", json.len());
            Ok(Some(json))
        }
        Some(WorkerMessage::Binary(data)) => {
            // Encode binary as JSON with base64 payload so JS can reconstruct
            info!("[Worker] worker received binary: {} bytes", data.len());
            let encoded = deno_core::serde_json::json!({
                "__binary": true,
                "base64": base64_encode(&data),
                "byteLength": data.len()
            });
            Ok(Some(encoded.to_string()))
        }
        Some(WorkerMessage::Terminate) => {
            info!("[Worker] worker received Terminate signal");
            Ok(None)
        }
        None => {
            info!("[Worker] worker channel closed (None)");
            Ok(None)
        }
    }
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
        dir "worker",
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
        dir "worker",
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

/// Create the full extension set for a worker JsRuntime.
///
/// Includes all extensions needed by `98_global_scope_shared.js`:
/// base, console, event, utility, file, rendering (webgl/image), web, url, network.
/// This gives workers the same shared APIs as the main thread.
pub fn create_worker_runtime_extensions(ctx: WorkerCtx, host_state: HostOpState) -> Vec<Extension> {
    use crate::{
        base, console, env, event, file, io_state, network, rendering, url, utility, web,
        worker_runtime,
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
    exts.extend(crate::audio::audio_extensions());

    exts.extend(env::env_extensions());
    exts.extend(worker_inner_extensions(ctx));
    exts.push(worker_runtime::init());

    exts
}

// ---------------------------------------------------------------------------
// Worker thread spawn
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Runaway watchdog
// ---------------------------------------------------------------------------

/// How often the monitor thread samples the worker heartbeat.
const WORKER_WATCHDOG_CHECK_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);
/// Max time a worker may run without yielding before it is force-terminated.
/// Matches the host ANR timeout: generous enough to cover module compilation on
/// low-end devices, tight enough to catch a `while(true)` runaway.
const WORKER_WATCHDOG_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Liveness heartbeat for a worker isolate. The worker runs on a
/// **single-threaded** tokio runtime, so a runaway JS loop that never yields
/// monopolises the thread and starves the ticker task; the heartbeat then goes
/// stale and the monitor thread force-terminates the isolate. Self-contained
/// mirror of the host ANR watchdog (`js-runtime` cannot depend on `core`).
struct WorkerWatchdog {
    epoch: std::time::Instant,
    heartbeat_ms: std::sync::atomic::AtomicU64,
}

impl WorkerWatchdog {
    #[inline]
    fn mono_millis(&self) -> u64 {
        self.epoch.elapsed().as_millis() as u64
    }
    #[inline]
    fn tick(&self) {
        self.heartbeat_ms
            .store(self.mono_millis(), std::sync::atomic::Ordering::Release);
    }
}

/// Arm the runaway watchdog for the current worker. Spawns two helpers:
///   * a ticker task on the worker's single-threaded runtime that refreshes the
///     heartbeat once a second — it can only run when JS yields, so a runaway
///     loop stops it (and is cancelled automatically when the runtime is
///     dropped on worker exit);
///   * a monitor OS thread (unaffected by a blocked runtime) that terminates
///     the isolate via the published handle once the heartbeat is older than
///     [`WORKER_WATCHDOG_TIMEOUT`], and exits on its own once the worker clears
///     the handle slot on any exit path.
fn spawn_worker_watchdog(
    isolate_handle_slot: Arc<std::sync::Mutex<Option<v8::IsolateHandle>>>,
    tx_errors: mpsc::UnboundedSender<String>,
) {
    use std::sync::atomic::Ordering;

    let wd = Arc::new(WorkerWatchdog {
        epoch: std::time::Instant::now(),
        heartbeat_ms: std::sync::atomic::AtomicU64::new(0),
    });
    wd.tick();

    let wd_tick = Arc::clone(&wd);
    tokio::task::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
        loop {
            interval.tick().await;
            wd_tick.tick();
        }
    });

    let spawned = std::thread::Builder::new()
        .name("Migo-WorkerWatchdog".into())
        .spawn(move || {
            let timeout_ms = WORKER_WATCHDOG_TIMEOUT.as_millis() as u64;
            let mut reported = false;
            loop {
                std::thread::sleep(WORKER_WATCHDOG_CHECK_INTERVAL);
                let elapsed = wd
                    .mono_millis()
                    .saturating_sub(wd.heartbeat_ms.load(Ordering::Acquire));
                // Hold the slot lock only briefly. The worker never holds it
                // while executing JS, so a runaway loop can still be reached.
                let slot = match isolate_handle_slot.lock() {
                    Ok(s) => s,
                    Err(_) => break,
                };
                match slot.as_ref() {
                    // Handle cleared => worker exited (all exit paths clear it)
                    // => the isolate is gone, stop the monitor.
                    None => break,
                    Some(handle) => {
                        if elapsed > timeout_ms {
                            // Re-arm termination every cycle until the worker
                            // actually exits (slot clears). One
                            // terminate_execution() is enough for a JS runaway,
                            // but a worker wedged in an uninterruptible native op
                            // won't drop until that op returns; keep the request
                            // live rather than giving up after one shot. Report
                            // once to avoid spamming the error channel.
                            if !reported {
                                warn!(
                                    "[Worker] watchdog: runaway detected ({}ms unresponsive > {}ms), terminating isolate",
                                    elapsed, timeout_ms
                                );
                                let _ = tx_errors.send(
                                    r#"{"message":"Worker terminated: unresponsive (watchdog timeout)"}"#
                                        .to_string(),
                                );
                                reported = true;
                            }
                            handle.terminate_execution();
                        }
                    }
                }
            }
        });
    if let Err(e) = spawned {
        // Non-fatal: without the monitor the worker simply lacks auto-kill (the
        // pre-existing state), so log and continue rather than fail the spawn.
        warn!("[Worker] failed to spawn watchdog monitor thread: {e}");
    }
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
                    let exts = create_worker_runtime_extensions(ctx, host_state);

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

                    info!("[Worker] creating JsRuntime with {} extensions", exts.len());
                    let mut rt = JsRuntime::new(RuntimeOptions {
                        module_loader,
                        extensions: exts,
                        create_params,
                        shared_array_buffer_store: Some(sab_store),
                        ..Default::default()
                    });
                    info!("[Worker] JsRuntime created successfully");

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

                    // Arm the runaway watchdog now that the isolate handle is
                    // published: covers module evaluation and the event loop. A
                    // worker that stops yielding (infinite loop) starves the
                    // ticker on this single-threaded runtime and gets killed.
                    spawn_worker_watchdog(isolate_handle_slot.clone(), tx_errors.clone());

                    // Resolve and load worker script
                    let code_path = std::path::PathBuf::from(&code_dir);
                    let resolved = match resolve_path(&script_path, &code_path) {
                        Ok(r) => r,
                        Err(e) => {
                            error!(
                                "[Worker] failed to resolve worker script '{}' in '{}': {}",
                                script_path, code_dir, e
                            );
                            let _ = tx_errors.send(format!(
                                r#"{{"message":"Failed to resolve worker script: {}"}}"#,
                                e
                            ));
                            return;
                        }
                    };

                    info!("[Worker] loading main module: {}", resolved);
                    let module_id = match rt.load_main_es_module(&resolved).await {
                        Ok(id) => id,
                        Err(e) => {
                            error!("[Worker] failed to load worker script: {}", e);
                            let _ = tx_errors.send(format!(
                                r#"{{"message":"Failed to load worker script: {}"}}"#,
                                e
                            ));
                            return;
                        }
                    };

                    info!("[Worker] module loaded (id={}), evaluating...", module_id);
                    if let Err(e) = rt.mod_evaluate(module_id).await {
                        error!("[Worker] worker script evaluation error: {}", e);
                        let _ = tx_errors.send(format!(
                            r#"{{"message":"Worker script evaluation error: {}"}}"#,
                            e
                        ));
                        return;
                    }

                    info!("[Worker] module evaluated, running event loop");
                    // Run event loop until it completes (message pump op keeps it alive)
                    let poll = PollEventLoopOptions::default();
                    if let Err(e) = rt.run_event_loop(poll).await {
                        error!("[Worker] event loop error: {}", e);
                        let _ = tx_errors
                            .send(format!(r#"{{"message":"Worker event loop error: {}"}}"#, e));
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
