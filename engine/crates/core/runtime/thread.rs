use std::{
    panic,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use deno_core::PollEventLoopOptions;
use tokio::runtime::{Builder, Runtime};
use tracing::{error, info, warn};

use shared::{
    config::InitOptions,
    error::{EngineError, EngineResult, ErrorCode},
    protocol::host_cmd::HostCommand,
    surface::SurfaceRef,
};

use crate::runtime::{HostId, host::Host, registry};
use crate::services::PlatformServices;

#[cfg(feature = "v8-limits")]
use crate::runtime::watchdog::TerminationReason;

pub fn spawn_host_thread(
    surface: SurfaceRef,
    platform: Arc<dyn PlatformServices>,
    opt: InitOptions,
) -> EngineResult<HostId> {
    let id = registry::alloc_host_id();

    // Bound all normal/game-controlled traffic while allowing the four trusted
    // lifecycle/surface callbacks to share the same FIFO without consuming
    // that quota. This preserves the old 512 pending-normal-command limit.
    const HOST_NORMAL_COMMAND_CAPACITY: usize = 512;
    let (host_tx, critical_host_tx, mut host_rx) =
        shared::host_channel::channel(HOST_NORMAL_COMMAND_CAPACITY);
    let (ready_tx, ready_rx) = crossbeam_channel::bounded::<()>(1);

    // Authoritative shutdown signal, independent of the normal-command budget:
    // `shutdown_host` sets this even when the budget is full (where its normal
    // Shutdown nudge is dropped) and the host loop polls it every iteration.
    let shutdown = Arc::new(AtomicBool::new(false));
    registry::register_sender(id, host_tx.clone(), critical_host_tx, shutdown.clone());

    // Clone the platform Arc so we can use it in the catch_unwind path
    // to notify Java about errors from any context (host loop, panic, etc.).
    let platform_for_error = platform.clone();

    let spawn_result = std::thread::Builder::new()
        .name(format!("Migo-Main-{}", id))
        .spawn(move || {
            let run = || {
                let host = match Host::new(id, host_tx, surface, platform, opt) {
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

                // Signal that we successfully created the Host and are about to enter runtime.
                if ready_tx.send(()).is_err() {
                    error!("[Host {}] ready signal send failed (receiver dropped)", id);
                    // receiver dropped -> caller likely already returned; still proceed to run loop.
                }

                let runtime = match create_basic_runtime() {
                    Ok(rt) => rt,
                    Err(e) => {
                        error!("[Host {}] failed to create tokio runtime: {}", id, e);
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

                // Pre-warm the blocking thread pool to avoid cold-start
                // latency on the first real spawn_blocking call.
                // Creates 4 threads eagerly (covers most concurrent IO).
                runtime.block_on(async {
                    let mut handles = Vec::with_capacity(4);
                    for _ in 0..4 {
                        handles.push(tokio::task::spawn_blocking(|| {}));
                    }
                    for h in handles {
                        let _ = h.await;
                    }
                });

                // Owned handle to the shutdown flag so the `async move` loop
                // below can poll it (the thread closure only lends it to us).
                let shutdown = shutdown.clone();
                runtime.block_on(async move {
                    let poll = PollEventLoopOptions::default();
                    let mut host = host;

                    // Heartbeat interval: run_event_loop may block indefinitely
                    // when long-lived async ops are pending (e.g., the RAF
                    // loop's op_await_next_frame).  This sleep ensures the
                    // outer loop re-iterates to tick the watchdog heartbeat,
                    // preventing false ANR reports.  The watchdog checks every
                    // 2s with a 10s timeout, so 3s gives comfortable margin.
                    let heartbeat_sleep = tokio::time::sleep(std::time::Duration::from_secs(3));
                    tokio::pin!(heartbeat_sleep);

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
                        // ANR watchdog, not this flag.
                        if shutdown.load(Ordering::Acquire) {
                            break 'outer;
                        }

                        // Tick the watchdog heartbeat before each iteration
                        #[cfg(feature = "v8-limits")]
                        if let Some(ref wd) = host.watchdog {
                            wd.state.tick();
                        }

                        // Lazy audio — check if buffered audio commands
                        // require spawning the AudioThread.  Cost when thread
                        // is already running: one branch (is_some check).
                        if let Err(e) = host.audio.check_and_start() {
                            error!("[Host {}] failed to start audio thread: {}", host.id, e);
                            // Non-fatal: game continues without audio.
                        }

                        tokio::select! {
                            biased;

                            host_event = host.js.run_event_loop(poll) => {
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
                                // With op-based RAF this normally shouldn't happen (the pending
                                // op_await_next_frame keeps the loop alive).  Safety fallback:
                                // park on the command channel to avoid busy spinning.
                                warn!("[Host {}] event loop idle, parking on command channel", id);
                                loop {
                                    // Park on the command channel, but keep
                                    // the heartbeat live so render-thread
                                    // events (e.g. ContextLost) are still
                                    // drained while idle — otherwise the
                                    // bounded "<=3s" drain latency would not
                                    // hold during long command-less stretches.
                                    tokio::select! {
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
                                        _ = &mut heartbeat_sleep => {
                                            host.drain_render_events();
                                            heartbeat_sleep.as_mut().reset(
                                                tokio::time::Instant::now()
                                                    + std::time::Duration::from_secs(3),
                                            );
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

                            _ = &mut heartbeat_sleep => {
                                // Drain render-thread events even when no host
                                // commands arrive, so a ContextLost during an
                                // idle stretch still reaches Host.context_lost
                                // (backing gl.isContextLost()) within one tick.
                                host.drain_render_events();
                                // Reset the timer for the next cycle
                                heartbeat_sleep.as_mut().reset(
                                    tokio::time::Instant::now() + std::time::Duration::from_secs(3)
                                );
                                continue;
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
            registry::unregister_destroy_epoch(id);
        });

    if let Err(e) = spawn_result {
        error!("[Host {}] failed to spawn thread: {}", id, e);
        registry::unregister_sender(id);
        registry::unregister_destroy_epoch(id);
        return Err(EngineError::new(ErrorCode::Internal)
            .with_msg("failed to spawn host thread")
            .with_detail(e.to_string()));
    }

    if ready_rx.recv().is_err() {
        error!("[Host {}] failed to start (init panic / early exit)", id);
        registry::unregister_sender(id);
        registry::unregister_destroy_epoch(id);
        return Err(EngineError::new(ErrorCode::Internal)
            .with_msg("host thread failed to start")
            .with_detail("init panic / early exit".to_string()));
    }

    Ok(id)
}

fn create_basic_runtime() -> EngineResult<Runtime> {
    let (event_interval, global_queue_interval, max_io_events_per_tick) = (61, 31, 1024);

    Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .event_interval(event_interval)
        .global_queue_interval(global_queue_interval)
        .max_io_events_per_tick(max_io_events_per_tick)
        // All blocking work (file IO, image decode, zip extract) runs on
        // this runtime's blocking pool via spawn_blocking.
        .max_blocking_threads(32)
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

    // Check watchdog timeout
    if let Some(ref wd) = host.watchdog {
        if let Some(reason) = wd.state.termination_reason() {
            return Some(match reason {
                TerminationReason::Timeout => EngineError::new(ErrorCode::JsExecutionTimeout)
                    .with_msg("JS execution exceeded watchdog timeout")
                    .with_detail(
                        "watchdog detected unresponsive JS execution and terminated isolate",
                    ),
                TerminationReason::OutOfMemory => EngineError::new(ErrorCode::OutOfMemory)
                    .with_msg("V8 heap limit exceeded")
                    .with_detail("watchdog OOM termination"),
            });
        }
    }

    // Generic termination (unknown source)
    Some(
        EngineError::new(ErrorCode::JsException)
            .with_msg("execution terminated")
            .with_detail("V8 isolate was terminated by unknown source"),
    )
}
