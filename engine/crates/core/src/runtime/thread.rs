use std::{
    panic,
    sync::Arc,
    thread::{self, JoinHandle},
};

use tokio::runtime::{Builder, Runtime};
use tracing::{debug, error, info};

use shared::{
    config::InitOptions,
    error::{EngineError, EngineResult, ErrorCode},
    protocol::host_cmd::HostCommand,
    surface::{
        PublicSurfaceGeneration, SurfaceControl, SurfaceLease, SurfaceRef, SurfaceResourceLease,
    },
};

use crate::runtime::{HostId, host::Host, registry};
use crate::services::PlatformServices;

// Tokio still backs `tokio::fs` uploads/local-audio reads and resolver
// fallbacks. Keep a small lazy compatibility pool; bounded engine I/O uses the
// process-wide Migo-IO executor instead.
const HOST_BLOCKING_FALLBACK_THREADS: usize = 4;

/// Result of starting a Host whose initial Surface belongs to a public
/// embedding attachment.
pub struct SpawnedSurfaceHost {
    pub host: HostThread,
    pub resource: SurfaceResourceLease,
}

/// Owning handle for one Migo Host thread.
///
/// Command producers may copy [`HostId`], but a native host must retain this
/// value until it can request shutdown and join. Dropping it is a synchronous
/// fail-safe, not a detach operation.
#[must_use = "a spawned Host must be shut down and joined"]
#[derive(Debug)]
pub struct HostThread {
    host_id: HostId,
    join: Option<JoinHandle<()>>,
}

impl HostThread {
    fn new(host_id: HostId, join: JoinHandle<()>) -> Self {
        Self {
            host_id,
            join: Some(join),
        }
    }

    #[cfg(feature = "test-support")]
    #[doc(hidden)]
    pub fn from_join_handle_for_test(host_id: HostId, join: JoinHandle<()>) -> Self {
        Self::new(host_id, join)
    }

    #[inline]
    pub const fn id(&self) -> HostId {
        self.host_id
    }

    #[inline]
    pub fn is_current_thread(&self) -> bool {
        self.join
            .as_ref()
            .is_some_and(|join| join.thread().id() == thread::current().id())
    }

    pub fn request_shutdown(&self) -> Result<(), String> {
        registry::shutdown_host(self.host_id)
    }

    pub fn join(&mut self) -> EngineResult<()> {
        self.join_inner()
    }

    pub fn shutdown_and_join(&mut self) -> EngineResult<()> {
        if self.join.is_none() {
            return Ok(());
        }
        self.request_shutdown().map_err(|error| {
            EngineError::new(ErrorCode::Internal)
                .with_msg("failed to request Host shutdown")
                .with_detail(format!("Host {}: {error}", self.host_id))
        })?;
        self.join_inner()
    }

    fn join_inner(&mut self) -> EngineResult<()> {
        if self.join.is_none() {
            return Ok(());
        }
        if self.is_current_thread() {
            return Err(EngineError::new(ErrorCode::Internal)
                .with_msg("Host thread cannot join itself")
                .with_detail(format!("Host {}", self.host_id)));
        }

        let join = self.join.take().expect("join presence checked above");
        join.join().map_err(|payload| {
            EngineError::new(ErrorCode::Internal)
                .with_msg("Host thread panicked outside its panic barrier")
                .with_detail(format!(
                    "Host {}: {}",
                    self.host_id,
                    panic_payload_message(payload.as_ref())
                ))
        })
    }
}

impl Drop for HostThread {
    fn drop(&mut self) {
        if self.join.is_none() {
            return;
        }
        if self.is_current_thread() {
            error!(
                "[Host {}] owning HostThread was dropped on its own thread",
                self.host_id
            );
            std::process::abort();
        }

        if let Err(error) = self.request_shutdown() {
            error!(
                "[Host {}] shutdown request from HostThread::drop failed: {}",
                self.host_id, error
            );
        }
        let join = self.join.take().expect("join presence checked above");
        if let Err(payload) = join.join() {
            error!(
                "[Host {}] join from HostThread::drop observed panic: {}",
                self.host_id,
                panic_payload_message(payload.as_ref())
            );
        }
    }
}

/// Start a Host, with or without the window Surface it will render into.
///
/// `surface` is `None` for a warm start: the host and render threads come up
/// and do every part of GPU bring-up that does not name a window -- EGL display
/// and config, the pbuffer resource context, the 709-entry GLES dispatch table,
/// capability detection, Skia, the system font scan -- and then park. The
/// Surface arrives later through the ordinary `UpdateSurface` path, which
/// installs it exactly as an initial Surface would have been installed.
///
/// The point is not to make bring-up cheaper but to move it off a critical
/// path. An Android Activity spends ~150 ms between `onCreate` and
/// `surfaceCreated` -- more when it rotates for a landscape game -- and until
/// this existed the engine could not start until the far end of it.
pub fn spawn_host_thread(
    surface: Option<SurfaceRef>,
    graphics_platform: graphics::egl_platform::GraphicsPlatform,
    platform: Arc<dyn PlatformServices>,
    opt: InitOptions,
) -> EngineResult<HostThread> {
    spawn_host_thread_inner(surface, graphics_platform, platform, opt, None)
        .map(|started| started.host)
}

/// Start a Host while preserving the embedding host's generation and a
/// resource lease for its unique public attachment handle.
pub fn spawn_host_thread_tracked(
    surface: SurfaceRef,
    public_generation: PublicSurfaceGeneration,
    graphics_platform: graphics::egl_platform::GraphicsPlatform,
    platform: Arc<dyn PlatformServices>,
    opt: InitOptions,
) -> EngineResult<SpawnedSurfaceHost> {
    spawn_host_thread_inner(
        Some(surface),
        graphics_platform,
        platform,
        opt,
        Some(public_generation),
    )
    .map(|started| SpawnedSurfaceHost {
        host: started.host,
        // Infallible by construction: this entry point always hands the inner
        // one a Surface, and the inner one mints a resource lease for exactly
        // the Surfaces it is given.
        resource: started
            .resource
            .expect("tracked spawn passes a Surface, so a resource lease exists"),
    })
}

fn spawn_host_thread_inner(
    surface: Option<SurfaceRef>,
    graphics_platform: graphics::egl_platform::GraphicsPlatform,
    platform: Arc<dyn PlatformServices>,
    opt: InitOptions,
    public_generation: Option<PublicSurfaceGeneration>,
) -> EngineResult<StartedHost> {
    let id = registry::alloc_host_id();

    // Issue generation 1 before publishing the Host. A fresh gate cannot be
    // exhausted, but keep the failure path explicit so generation wrap always
    // fails closed rather than silently creating an untracked Surface.
    let surface_control = Arc::new(SurfaceControl::new());
    // Deliberately not minted for a warm start. `attach_or_update` mints the
    // *next* generation when the gate is detached and is idempotent when it is
    // live, so a session that starts without a Surface has its first one issued
    // generation 1 by the `UpdateSurface` that delivers it -- the same number,
    // issued at the same point in the Surface's life, as if it had been passed
    // here. Minting one now for a Surface that does not exist would spend a
    // generation on nothing and put every later one off by one.
    let initial_surface = match surface {
        Some(surface) => {
            let initial_token = surface_control.attach_or_update().map_err(|_| {
                EngineError::new(ErrorCode::InvalidOperation)
                    .with_msg("initial Surface generation exhausted")
            })?;
            Some(match public_generation {
                Some(public_generation) => {
                    SurfaceLease::new_tracked(surface, initial_token, public_generation)
                }
                None => SurfaceLease::new(surface, initial_token),
            })
        }
        None => None,
    };
    let initial_resource = initial_surface.as_ref().map(|lease| lease.resource_lease());
    // Having a Surface and being on the caller's critical path are the same
    // question asked twice: a caller that already holds the Surface is
    // necessarily starting the engine at the point it needs to render.
    let wait_for_ready = initial_surface.is_some();

    // Bound all normal/game-controlled traffic while allowing the four trusted
    // lifecycle/surface callbacks to share the same FIFO without consuming
    // that quota. This preserves the old 512 pending-normal-command limit.
    let (host_tx, critical_host_tx, mut host_rx) = shared::host_channel::channel_with_reserve(
        registry::HOST_NORMAL_COMMAND_CAPACITY,
        registry::HOST_RELIABLE_INPUT_RESERVE,
    );
    let (ready_tx, ready_rx) = crossbeam_channel::bounded::<()>(1);

    // Authoritative shutdown signal, independent of the normal-command budget:
    // `shutdown_host` sets this even when the budget is full (where its normal
    // Shutdown nudge is dropped) and the host loop polls it every iteration.
    // Constructed before registration so no sender can ever be reachable
    // without a generation to stamp with. The boundary itself moves into the
    // Host thread; the registry keeps only a reader.
    let restart_boundary = crate::runtime::restart_boundary::RestartBoundary::new();
    let log_level = opt.log_level();
    registry::register_sender(
        id,
        host_tx.clone(),
        critical_host_tx.clone(),
        Arc::clone(&surface_control),
        restart_boundary.reader(),
        log_level,
    );

    // Clone the platform Arc so we can use it in the catch_unwind path
    // to notify Java about errors from any context (host loop, panic, etc.).
    let platform_for_error = platform.clone();

    let spawn_result = thread::Builder::new()
        .name(format!("Migo-Main-{}", id))
        .spawn(move || {
            // Bound before any of this session's work, so its own records are
            // filtered by its own level rather than by the most verbose level some
            // other live session asked for. The thread ends with the session, so
            // there is nothing to unbind.
            shared::log_level::bind_thread_level(log_level);
            let run = || {
                let host = match Host::new(
                    id,
                    host_tx,
                    critical_host_tx,
                    initial_surface,
                    graphics_platform,
                    platform,
                    opt,
                    Arc::clone(&surface_control),
                    restart_boundary,
                ) {
                    Ok(h) => h,
                    Err(e) => {
                        error!("[Host {}] failed to create host: {}", id, e);
                        // Notify Java of the initialization failure
                        platform_for_error.notify_error(
                            id,
                            e.code.as_u16(),
                            &e.msg,
                            e.detail.as_deref().unwrap_or(""),
                        );
                        // ready_tx will be dropped without sending, triggering error in caller
                        return;
                    }
                };

                let runtime = match create_runtime_before_ready(ready_tx, create_basic_runtime) {
                    Ok(rt) => rt,
                    Err(e) => {
                        error!("[Host {}] failed to enter tokio runtime: {}", id, e);
                        platform_for_error.notify_error(
                            id,
                            e.code.as_u16(),
                            &e.msg,
                            e.detail.as_deref().unwrap_or(""),
                        );
                        return;
                    }
                };

                // Keep a reference to platform_for_error for use in the event loop
                let platform_ref = &platform_for_error;

                // Owned handle to the shutdown flag so the `async move` loop
                // below can poll it (the thread closure only lends it to us).
                let surface_control = Arc::clone(&surface_control);
                runtime.block_on(async move {
                    let mut host = host;

                    // Coalescing wake signals replace the deleted 3-second
                    // heartbeat. The render event channel calls `notify_one()` on
                    // every successfully enqueued event; the lazy-audio start
                    // signal fires on the first pre-start command. Both are Tokio
                    // `Notify`s that latch one permit even with no waiter
                    // registered, so a signal emitted while the host is inside a
                    // `run_event_loop` poll is delivered on the next select
                    // iteration rather than lost. Cloned once, outside the loop.
                    let render_notify = host.render_notify();
                    let audio_signal = host.audio.start_signal();

                    // Track whether to notify Java on exit.
                    // Error paths set this to false (they already call notify_error).
                    // Normal Shutdown keeps it true so Java learns the host exited
                    // (needed when JS calls exitMiniProgram — Java didn't initiate
                    // the shutdown and must finish the Activity).
                    let mut notify_exit = true;

                    'outer: loop {
                        // Shutdown check, decoupled from the command queue so a
                        // full queue can't swallow the request (see shutdown_host).
                        // Stops the loop the next time control returns here; a
                        // runaway JS section that never yields is handled by the
                        // deadline watchdog, not this flag.
                        if surface_control.is_shutting_down() {
                            break 'outer;
                        }

                        tokio::select! {
                            biased;

                            host_event = host.js.pump_event_loop() => {
                                if let Err(e) = host_event {
                                    // Check if this was a watchdog/OOM termination
                                    #[cfg(feature = "v8-limits")]
                                    {
                                        let classified = classify_termination_error(&host, &e);
                                        if let Some(engine_err) = classified {
                                            // Report fatal error via DebugStats for Java layer polling
                                            if let Some(stats) = shared::stats::get_stats(id) {
                                                stats.fatal_error_code.store(
                                                    engine_err.code.as_u16() as u32,
                                                    std::sync::atomic::Ordering::SeqCst,
                                                );
                                            }
                                            error!(
                                                "[Host {}] V8 terminated: code={:?}, msg={}, detail={:?}",
                                                id, engine_err.code, engine_err.msg, engine_err.detail
                                            );
                                            // Notify Java via JNI callback
                                            platform_ref.notify_error(
                                                id,
                                                engine_err.code.as_u16(),
                                                &engine_err.msg,
                                                engine_err.detail.as_deref().unwrap_or(""),
                                            );
                                            notify_exit = false;
                                            break 'outer;
                                        }
                                    }

                                    error!(
                                        "[Host {}] event loop error: code={:?}, msg={}, detail={:?}",
                                        id, e.code, e.msg, e.detail
                                    );
                                    // Notify Java about unclassified JS errors too
                                    platform_ref.notify_error(
                                        id,
                                        e.code.as_u16(),
                                        &e.msg,
                                        e.detail.as_deref().unwrap_or(""),
                                    );
                                    notify_exit = false;
                                    break 'outer;
                                }
                                // Event loop returned Ok — no pending ops/timers/promises.
                                // Ordinary once the game is running, because the pending
                                // op_await_next_frame keeps the loop alive; ordinary *before* it
                                // is running too, which is what this used to claim was abnormal.
                                // It fires three times on every single launch, in the window
                                // between the host thread coming up and the game registering its
                                // first rAF -- a warning on the guaranteed path is how a log
                                // teaches its reader to skip warnings.
                                //
                                // Park on the command channel + render/audio signals (no polling
                                // timer) so a ContextLost or a first audio command during an idle
                                // stretch is still handled promptly.
                                debug!("[Host {}] event loop idle, parking on command/render/audio signals", id);
                                loop {
                                    tokio::select! {
                                        biased;
                                        maybe_msg = host_rx.recv() => {
                                            match maybe_msg {
                                                Some(HostCommand::Shutdown) => break 'outer,
                                                Some(msg) => {
                                                    host.handle_command(msg).await;
                                                    break; // back to outer select to re-poll event loop
                                                }
                                                None => break 'outer,
                                            }
                                        }
                                        _ = render_notify.notified() => {
                                            host.drain_render_events();
                                        }
                                        _ = audio_signal.notified() => {
                                            if let Err(e) = host.audio.check_and_start() {
                                                error!("[Host {}] failed to start audio thread: {}", host.id, e);
                                            }
                                        }
                                    }
                                }
                            }

                            maybe_msg = host_rx.recv() => {
                                match maybe_msg {
                                    Some(HostCommand::Shutdown) => break 'outer,
                                    Some(msg) => host.handle_command(msg).await,
                                    None => break 'outer,
                                }
                            }

                            _ = render_notify.notified() => {
                                // A render-thread event was enqueued: drain +
                                // reconcile (notably ContextLost/Recovered) now,
                                // instead of waiting for the next command.
                                host.drain_render_events();
                            }

                            _ = audio_signal.notified() => {
                                // First pre-start audio command: lazily spawn the
                                // AudioThread. The signal disables itself
                                // (mark_started) once the thread is installed, so
                                // this branch stops firing afterwards.
                                if let Err(e) = host.audio.check_and_start() {
                                    error!("[Host {}] failed to start audio thread: {}", host.id, e);
                                    // Non-fatal: game continues without audio.
                                }
                            }
                        }
                    }

                    // Notify Java that the host is exiting.  When JS calls
                    // exitMiniProgram, this is the only way Java learns about it
                    // so it can finish() the Activity.  When Java itself called
                    // shutdown_host(), it can simply ignore this notification.
                    if notify_exit {
                        platform_ref.notify_exit(id);
                    }
                });

                info!("[Host {}] host thread exited", id);
            };

            let r = panic::catch_unwind(panic::AssertUnwindSafe(run));
            if let Err(panic_info) = r {
                let panic_msg = panic_info
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| panic_info.downcast_ref::<&str>().map(|s| s.to_string()))
                    .unwrap_or_else(|| "Unknown panic".to_string());

                error!("[Host {}] panicked: {}", id, panic_msg);

                // Report panic to DebugStats for Java layer polling
                if let Some(stats) = shared::stats::get_stats(id) {
                    stats.fatal_error_code.store(
                        ErrorCode::HostPanic.as_u16() as u32,
                        std::sync::atomic::Ordering::SeqCst,
                    );
                }

                // Notify Java via JNI callback.
                //
                // Thread attach/detach: `platform.notify_error` uses `with_env`
                // which calls `jvm.attach_current_thread()` if the host thread
                // isn't already attached.  On Android, AttachCurrentThread is
                // safe to call from any native thread and the AttachGuard will
                // auto-detach when the thread-local drops.
                platform_for_error.notify_error(
                    id,
                    ErrorCode::HostPanic.as_u16(),
                    "host thread panic",
                    &panic_msg,
                );
            }

            // NOTE: Shutdown-unregister race window
            //
            // Between the host thread exiting and this unregister call, JNI
            // callbacks (onVsync, touch events, etc.) may still call
            // `send_command_to_host(id, ...)`.  Those calls will get a
            // "Cannot find host_id=N sender" error from the registry.
            //
            // This is benign: the host is already shutting down, so dropping
            // late-arriving commands is the correct behavior.  The JNI callers
            // already ignore send failures (they use `let _ = send_command_to_host(...)`).
            registry::unregister_sender(id);
        });

    let join = match spawn_result {
        Ok(join) => join,
        Err(error) => {
            error!("[Host {}] failed to spawn thread: {}", id, error);
            registry::unregister_sender(id);
            return Err(EngineError::new(ErrorCode::Internal)
                .with_msg("failed to spawn host thread")
                .with_detail(error.to_string()));
        }
    };
    let host = HostThread::new(id, join);

    // A warm start does not wait for the Host to finish constructing.
    //
    // The handshake below exists to turn a failed construction into a
    // synchronous error for the caller, and for a cold start that is worth what
    // it costs: the caller is on the path to first frame anyway. A warm start's
    // entire purpose is to be off that path, and waiting here would put ~20 ms
    // of V8 isolate construction back on the caller's thread -- which on
    // Android is the main thread, mid-`onCreate`, with layout still to do.
    // Measured on a Mate 30 Pro, waiting cost ~60 ms of `Displayed`: more than
    // the warm start saved.
    //
    // Nothing is lost but the *timing* of the report. The id and its command
    // senders are registered before the thread is spawned, so commands sent
    // meanwhile queue rather than fail; and the thread unregisters itself and
    // calls `notify_error` on every exit path, construction failure included,
    // so a Host that dies on its way up still tells the embedder so.
    if !wait_for_ready {
        return Ok(StartedHost {
            host,
            resource: initial_resource,
        });
    }

    if ready_rx.recv().is_err() {
        error!("[Host {}] failed to start (init panic / early exit)", id);
        registry::unregister_sender(id);
        let error = EngineError::new(ErrorCode::Internal)
            .with_msg("host thread failed to start")
            .with_detail("init panic / early exit".to_string());
        return Err(join_failed_start(host, error));
    }

    Ok(StartedHost {
        host,
        resource: initial_resource,
    })
}

/// What `spawn_host_thread_inner` returns: a Host, and a resource lease for the
/// Surface it was given, if it was given one.
struct StartedHost {
    host: HostThread,
    resource: Option<SurfaceResourceLease>,
}

fn join_failed_start(mut host: HostThread, startup_error: EngineError) -> EngineError {
    match host.join() {
        Ok(()) => startup_error,
        Err(join_error) => EngineError::new(ErrorCode::Internal)
            .with_msg("Host startup failed and its thread could not be joined")
            .with_detail(format!("startup={startup_error}; join={join_error}")),
    }
}

fn panic_payload_message(payload: &(dyn std::any::Any + Send)) -> &str {
    payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("non-string panic payload")
}

/// Build the tokio runtime, and only then announce that startup succeeded.
///
/// The ordering is the point: a runtime that failed to build must never be
/// reported ready. Whether anyone is *listening* is a separate question, and
/// not this function's business -- a warm start deliberately stops listening,
/// because the whole reason it exists is to get its caller's thread back. A
/// dropped receiver used to abort the Host here, which turned every warm start
/// into a Host that came up and immediately died.
fn create_runtime_before_ready(
    ready_tx: crossbeam_channel::Sender<()>,
    create_runtime: impl FnOnce() -> EngineResult<Runtime>,
) -> EngineResult<Runtime> {
    let runtime = create_runtime()?;
    let _ = ready_tx.send(());
    Ok(runtime)
}

fn create_basic_runtime() -> EngineResult<Runtime> {
    let (event_interval, global_queue_interval, max_io_events_per_tick) = (61, 31, 1024);

    Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .event_interval(event_interval)
        .global_queue_interval(global_queue_interval)
        .max_io_events_per_tick(max_io_events_per_tick)
        // Bounded engine I/O runs on the process Migo-IO executor. This lazy
        // fallback remains for tokio::fs and resolver internals only.
        .max_blocking_threads(HOST_BLOCKING_FALLBACK_THREADS)
        .build()
        .map_err(|e| {
            EngineError::from_detail(
                ErrorCode::Internal,
                format!("failed to create tokio runtime: {}", e),
            )
        })
}

/// Classify a V8 "execution terminated" error into a more specific error code
/// by checking the watchdog state and OOM flag.
///
/// Returns `Some(EngineError)` if this is a recognized termination, `None` otherwise.
#[cfg(feature = "v8-limits")]
fn classify_termination_error(host: &Host, original: &EngineError) -> Option<EngineError> {
    // Check if this is a termination error (V8 sends "execution terminated")
    let is_termination = original.msg.contains("execution terminated")
        || original
            .detail
            .as_ref()
            .is_some_and(|d| d.contains("execution terminated"));

    if !is_termination {
        return None;
    }

    // Check OOM first (the near-heap-limit callback sets this flag)
    if host.js.was_oom_terminated() {
        return Some(
            EngineError::new(ErrorCode::OutOfMemory)
                .with_msg("V8 heap limit exceeded")
                .with_detail("near_heap_limit_callback triggered terminate_execution"),
        );
    }

    // Then the process deadline watchdog. OOM stays higher priority than a
    // timeout; the timeout is sticky per isolate.
    if host.js.watchdog_timed_out() {
        return Some(
            EngineError::new(ErrorCode::JsExecutionTimeout)
                .with_msg("JS execution exceeded watchdog timeout")
                .with_detail("watchdog detected unresponsive JS execution and terminated isolate"),
        );
    }

    // Generic termination (unknown source)
    Some(
        EngineError::new(ErrorCode::JsException)
            .with_msg("execution terminated")
            .with_detail("V8 isolate was terminated by unknown source"),
    )
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
            mpsc,
        },
        thread,
    };

    use shared::error::{EngineError, ErrorCode};

    use super::{HostThread, create_runtime_before_ready, join_failed_start};

    struct DropSentinel(Arc<AtomicBool>);

    impl Drop for DropSentinel {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Release);
        }
    }

    #[test]
    fn join_waits_for_named_host_and_observes_sentinel_drop() {
        let dropped = Arc::new(AtomicBool::new(false));
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let sentinel = DropSentinel(Arc::clone(&dropped));
        let join = thread::Builder::new()
            .name("Migo-Main-test-join".to_owned())
            .spawn(move || {
                let _sentinel = sentinel;
                started_tx
                    .send(thread::current().name().map(str::to_owned))
                    .expect("publish thread name");
                release_rx.recv().expect("release test host");
            })
            .expect("spawn test host");
        let mut host = HostThread::new(41, join);
        let (joined_tx, joined_rx) = mpsc::channel();

        let joiner = thread::spawn(move || {
            joined_tx.send(host.join()).expect("publish join result");
        });

        assert_eq!(
            started_rx.recv().expect("test host started").as_deref(),
            Some("Migo-Main-test-join")
        );
        assert!(
            joined_rx.try_recv().is_err(),
            "join returned while the named Host was still blocked"
        );
        assert!(!dropped.load(Ordering::Acquire));

        release_tx.send(()).expect("release test host");
        joined_rx
            .recv()
            .expect("join result")
            .expect("Host join succeeds");
        joiner.join().expect("join caller");
        assert!(dropped.load(Ordering::Acquire));
    }

    #[test]
    fn failed_start_joins_the_spawned_thread() {
        let dropped = Arc::new(AtomicBool::new(false));
        let sentinel = DropSentinel(Arc::clone(&dropped));
        let join = thread::Builder::new()
            .name("Migo-Main-test-failed-start".to_owned())
            .spawn(move || drop(sentinel))
            .expect("spawn failed-start test host");
        let original =
            EngineError::new(ErrorCode::Internal).with_msg("host thread failed to start");

        let error = join_failed_start(HostThread::new(42, join), original);

        assert_eq!(error.msg, "host thread failed to start");
        assert!(dropped.load(Ordering::Acquire));
    }

    #[test]
    fn runtime_creation_failure_is_not_published_as_ready() {
        let (ready_tx, ready_rx) = crossbeam_channel::bounded(1);
        let original =
            EngineError::new(ErrorCode::Internal).with_msg("injected runtime construction failure");

        let error = create_runtime_before_ready(ready_tx, || Err(original))
            .expect_err("runtime construction must fail");

        assert_eq!(error.msg, "injected runtime construction failure");
        assert!(
            ready_rx.recv().is_err(),
            "startup readiness was published before runtime construction"
        );
    }

    /// A warm start returns before the Host is ready and drops the receiver as
    /// it goes. That must leave the Host alive: the first cut of the warm start
    /// treated the dropped receiver as a startup failure, so every warm-started
    /// session built its runtime, failed to announce it, and exited -- the game
    /// never ran, and the only symptom was a missing `Fully drawn`.
    #[test]
    fn nobody_waiting_for_readiness_is_not_a_startup_failure() {
        let (ready_tx, ready_rx) = crossbeam_channel::bounded::<()>(1);
        drop(ready_rx);

        let runtime = create_runtime_before_ready(ready_tx, create_basic_runtime)
            .expect("a dropped readiness receiver must not fail Host startup");

        drop(runtime);
    }

    #[test]
    fn host_thread_id_is_stable() {
        let join = thread::Builder::new()
            .name("Migo-Main-test-id".to_owned())
            .spawn(|| {})
            .expect("spawn ID test host");
        let mut host = HostThread::new(73, join);

        assert_eq!(host.id(), 73);
        host.join().expect("Host join succeeds");
    }

    #[test]
    fn self_join_rejection_preserves_owner_for_another_thread() {
        let (owner_tx, owner_rx) = mpsc::channel();
        let (error_tx, error_rx) = mpsc::channel();
        let (returned_tx, returned_rx) = mpsc::channel();
        let join = thread::Builder::new()
            .name("Migo-Main-test-self-join".to_owned())
            .spawn(move || {
                let mut host: HostThread = owner_rx.recv().expect("receive own Host owner");
                let error = host.join().expect_err("self-join must be rejected");
                error_tx.send(error).expect("publish self-join error");
                returned_tx.send(host).expect("return Host owner");
            })
            .expect("spawn self-join test host");
        owner_tx
            .send(HostThread::new(74, join))
            .expect("transfer owner to its Host");

        let error = error_rx.recv().expect("self-join result");
        assert_eq!(error.code, ErrorCode::Internal);
        let mut host = returned_rx.recv().expect("returned Host owner");
        host.join().expect("another thread joins the Host");
    }
}
