use std::{cell::RefCell, rc::Rc, sync::Arc};

use deno_core::{
    Extension, FsModuleLoader, JsRuntime, ModuleLoader, OpState, PollEventLoopOptions,
    RuntimeOptions, op2, resolve_path, v8,
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
/// patches AMD `define` modules.
struct WorkerModuleLoader(FsModuleLoader);

impl deno_core::ModuleLoader for WorkerModuleLoader {
    fn resolve(
        &self,
        specifier: &str,
        referrer: &str,
        kind: deno_core::ResolutionKind,
    ) -> Result<deno_core::ModuleSpecifier, deno_core::error::ModuleLoaderError> {
        let spec = normalize_specifier(specifier, &kind);
        self.0.resolve(spec.as_ref(), referrer, kind)
    }

    fn load(
        &self,
        module_specifier: &deno_core::ModuleSpecifier,
        maybe_referrer: Option<&deno_core::ModuleLoadReferrer>,
        options: deno_core::ModuleLoadOptions,
    ) -> deno_core::ModuleLoadResponse {
        let resp = self.0.load(module_specifier, maybe_referrer, options);
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
        self.0
            .prepare_load(module_specifier, maybe_referrer, maybe_content, options)
    }

    fn finish_load(&self) {}

    fn code_cache_ready(
        &self,
        module_specifier: deno_core::ModuleSpecifier,
        hash: u64,
        code_cache: &[u8],
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()>>> {
        self.0.code_cache_ready(module_specifier, hash, code_cache)
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
    // Check existing worker
    {
        let st = state.borrow();
        if st.try_borrow::<WorkerHandle>().is_some() {
            return Err(WorkerError::Message(
                "Only one worker can exist at a time. Call terminate() first.".into(),
            ));
        }
    }

    // Read code_dir and clone a minimal HostOpState for the worker
    let (code_dir, worker_host_state) = {
        let st = state.borrow();
        let host = st.borrow::<HostOpState>();
        let code_dir = host.code_dir.clone().ok_or_else(|| {
            WorkerError::Message("No code directory set (game not loaded yet)".into())
        })?;

        // Create dummy channels for services the worker does not use
        let (render_tx, _render_rx) = crossbeam_channel::bounded(1);
        let (io_tx, _io_rx) = mpsc::unbounded_channel();
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
            render_tx,
            io_tx,
            audio_tx,
            host_tx: host.host_tx.clone(),
            device_services: None,
            raf_rx: None,
            network_policy: host.network_policy.clone(),
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

    // Spawn worker thread
    info!(
        "[Worker] spawning worker thread for script: {}",
        script_path
    );
    let join_handle = spawn_worker_thread(script_path, code_dir, worker_ctx, worker_host_state)?;
    info!("[Worker] worker thread spawned, storing handle");

    // Store handle in main OpState
    let handle = WorkerHandle {
        tx_to_worker: tx_main_to_worker,
        rx_from_worker: Arc::new(tokio::sync::Mutex::new(rx_worker_to_main)),
        rx_errors: Arc::new(tokio::sync::Mutex::new(rx_worker_errors)),
        join_handle: Some(join_handle),
        terminated: false,
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
        let _ = handle.tx_to_worker.send(WorkerMessage::Terminate);
    }

    // Remove handle from state so a new worker can be created
    state.take::<WorkerHandle>();
    Ok(())
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
// Extension declarations
// ---------------------------------------------------------------------------

deno_core::extension!(
    host_v8_worker,
    deps = [host_v8_base],
    ops = [
        op_worker_create,
        op_worker_post_message,
        op_worker_recv_message,
        op_worker_recv_error,
        op_worker_terminate,
    ],
    esm = [
        dir "worker",
        "01_worker.js",
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
        audio, base, console, env, event, file, network, rendering, url, utility, web,
        worker_runtime,
    };

    let runtime_exts = vec![worker_runtime::init()];

    base::base_extensions(host_state)
        .into_iter()
        .chain(console::console_extensions())
        .chain(event::event_extensions())
        .chain(utility::utility_extensions())
        .chain(file::file_extensions())
        .chain(rendering::rendering_extensions())
        .chain(web::web_extensions())
        .chain(url::url_extensions())
        .chain(network::network_extensions())
        .chain(audio::audio_extensions())
        .chain(env::env_extensions())
        .chain(worker_inner_extensions(ctx))
        .chain(runtime_exts)
        .collect()
}

// ---------------------------------------------------------------------------
// Worker thread spawn
// ---------------------------------------------------------------------------

fn spawn_worker_thread(
    script_path: String,
    code_dir: String,
    ctx: WorkerCtx,
    host_state: HostOpState,
) -> Result<std::thread::JoinHandle<()>, WorkerError> {
    let tx_errors = ctx.tx_errors.clone();

    std::thread::Builder::new()
        .name("Migo-Worker".into())
        .spawn(move || {
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
                    let exts = create_worker_runtime_extensions(ctx, host_state);

                    let module_loader: Option<Rc<dyn ModuleLoader>> =
                        Some(Rc::new(WorkerModuleLoader(FsModuleLoader)));

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
                        ..Default::default()
                    });
                    info!("[Worker] JsRuntime created successfully");

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
        })
        .map_err(|e| WorkerError::Message(format!("Failed to spawn worker thread: {e}")))
}
