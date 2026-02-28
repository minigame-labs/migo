use std::{panic, sync::Arc};

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

    let (js_tx, mut js_rx) = tokio::sync::mpsc::channel::<HostCommand>(256);
    let (ready_tx, ready_rx) = crossbeam_channel::bounded::<()>(1);

    registry::register_sender(id, js_tx.clone());

    // Clone the platform Arc so we can use it in the catch_unwind path
    // to notify Java about errors from any context (host loop, panic, etc.).
    let platform_for_error = platform.clone();

    let spawn_result = std::thread::Builder::new()
        .name(format!("Migo-Main-{}", id))
        .spawn(move || {
            let run = || {
                let host = match Host::new(id, js_tx, surface, platform, opt) {
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

                let runtime = create_basic_runtime();

                // Keep a reference to platform_for_error for use in the event loop
                let platform_ref = &platform_for_error;

                runtime.block_on(async move {
                    let poll = PollEventLoopOptions::default();
                    let mut host = host;

                    // P1-2: Spawn IO handler as a cooperative task on the Host
                    // runtime.  This eliminates the separate IO tokio runtime
                    // (one fewer epoll fd, timer wheel, and blocking pool).
                    host.io.spawn_handler();

                    // Heartbeat interval: run_event_loop may block indefinitely
                    // when long-lived async ops are pending (e.g., the RAF
                    // loop's op_await_next_frame).  This sleep ensures the
                    // outer loop re-iterates to tick the watchdog heartbeat,
                    // preventing false ANR reports.  The watchdog checks every
                    // 2s with a 10s timeout, so 3s gives comfortable margin.
                    let heartbeat_sleep = tokio::time::sleep(std::time::Duration::from_secs(3));
                    tokio::pin!(heartbeat_sleep);

                    'outer: loop {
                        // Tick the watchdog heartbeat before each iteration
                        #[cfg(feature = "v8-limits")]
                        if let Some(ref wd) = host.watchdog {
                            wd.state.tick();
                        }

                        // P1-5: Lazy audio — check if buffered audio commands
                        // require spawning the AudioThread.  Cost when thread
                        // is already running: one branch (is_some check).
                        if let Err(e) = host.audio.check_and_start() {
                            error!("[Host {}] failed to start audio thread: {}", host.id, e);
                            // Non-fatal: game continues without audio.
                        }

                        tokio::select! {
                            biased;

                            maybe_msg = js_rx.recv() => {
                                match maybe_msg {
                                    Some(HostCommand::Shutdown) => break 'outer,
                                    Some(msg) => host.handle_command(msg).await,
                                    None => break 'outer,
                                }
                            }

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
                                    break 'outer;
                                }
                                // Event loop returned Ok — no pending ops/timers/promises.
                                // With op-based RAF this normally shouldn't happen (the pending
                                // op_await_next_frame keeps the loop alive).  Safety fallback:
                                // park on the command channel to avoid busy spinning.
                                warn!("[Host {}] event loop idle, parking on command channel", id);
                                loop {
                                    match js_rx.recv().await {
                                        Some(HostCommand::Shutdown) => break 'outer,
                                        Some(msg) => {
                                            host.handle_command(msg).await;
                                            break; // back to outer select to re-poll event loop
                                        }
                                        None => break 'outer,
                                    }
                                }
                            }

                            _ = &mut heartbeat_sleep => {
                                // Reset the timer for the next cycle
                                heartbeat_sleep.as_mut().reset(
                                    tokio::time::Instant::now() + std::time::Duration::from_secs(3)
                                );
                                continue;
                            }
                        }
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

            registry::unregister_sender(id);
        });

    if let Err(e) = spawn_result {
        error!("[Host {}] failed to spawn thread: {}", id, e);
        registry::unregister_sender(id);
        return Err(EngineError::new(ErrorCode::Internal)
            .with_msg("failed to spawn host thread")
            .with_detail(e.to_string()));
    }

    if ready_rx.recv().is_err() {
        error!("[Host {}] failed to start (init panic / early exit)", id);
        registry::unregister_sender(id);
        return Err(EngineError::new(ErrorCode::Internal)
            .with_msg("host thread failed to start")
            .with_detail("init panic / early exit".to_string()));
    }

    Ok(id)
}

fn create_basic_runtime() -> Runtime {
    let (event_interval, global_queue_interval, max_io_events_per_tick) = (61, 31, 1024);

    Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .event_interval(event_interval)
        .global_queue_interval(global_queue_interval)
        .max_io_events_per_tick(max_io_events_per_tick)
        // P1-2: Raised from 4 to 8 — the Host runtime now serves both
        // JS (timer ops, module loading) and IO (file system, image decode,
        // zip extract) blocking work that was previously split across two
        // independent tokio runtimes.
        .max_blocking_threads(8)
        .build()
        .unwrap()
}

/// Classify a V8 "execution terminated" error into a more specific error code
/// by checking the watchdog state and OOM flag.
///
/// Returns `Some(EngineError)` if this is a recognized termination, `None` otherwise.
#[cfg(feature = "v8-limits")]
fn classify_termination_error(
    host: &Host,
    original: &EngineError,
) -> Option<EngineError> {
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
                TerminationReason::Timeout => {
                    EngineError::new(ErrorCode::JsExecutionTimeout)
                        .with_msg("JS execution exceeded watchdog timeout")
                        .with_detail("watchdog detected unresponsive JS execution and terminated isolate")
                }
                TerminationReason::OutOfMemory => {
                    EngineError::new(ErrorCode::OutOfMemory)
                        .with_msg("V8 heap limit exceeded")
                        .with_detail("watchdog OOM termination")
                }
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
