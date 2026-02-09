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

pub fn spawn_host_thread(
    surface: SurfaceRef,
    platform: Arc<dyn PlatformServices>,
    opt: InitOptions,
) -> EngineResult<HostId> {
    let id = registry::alloc_host_id();

    let (js_tx, mut js_rx) = tokio::sync::mpsc::channel::<HostCommand>(256);
    let (ready_tx, ready_rx) = crossbeam_channel::bounded::<()>(1);

    registry::register_sender(id, js_tx.clone());

    let spawn_result = std::thread::Builder::new()
        .name(format!("Migo-Main-{}", id))
        .spawn(move || {
            let run = || {
                let host = match Host::new(id, js_tx, surface, platform, opt) {
                    Ok(h) => h,
                    Err(e) => {
                        error!("[Host {}] failed to create host: {}", id, e);
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

                runtime.block_on(async move {
                    let poll = PollEventLoopOptions::default();
                    let mut host = host;

                    'outer: loop {
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
                                    error!(
                                        "[Host {}] event loop error: code={:?}, msg={}, detail={:?}",
                                        id, e.code, e.msg, e.detail
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
        .max_blocking_threads(4)
        .build()
        .unwrap()
}
