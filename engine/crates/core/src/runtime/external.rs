//! A session whose JavaScript runs somewhere else.
//!
//! On iOS the content's JavaScript and WebAssembly run inside WebKit's
//! WebContent process, because that is the only process Apple grants a JIT to.
//! What arrives here is a bounded, validated packet of drawing work per frame.
//! So this session owns everything a session normally owns -- the render
//! thread, the surface, the frame clock, lifecycle, generations -- and owns no
//! script runtime at all.
//!
//! That absence is the product claim `MigoApplePerformancePlus` rests on, and
//! it is structural rather than intended: this module compiles only under
//! `external-frames`, which `lib.rs` refuses to combine with `embedded-v8`, and
//! `scripts/test-apple-performance-rust-closure.sh` measures the resolved
//! dependency graph to prove no engine is reachable from here.
//!
//! # What is not here yet
//!
//! Frame submission. The ingress exists and its identity, sequence, generation
//! and resource rules are enforced, but nothing hands it bytes: doing that
//! correctly needs the pooled buffer a packet is copied into and the RAII token
//! that returns its credit when the renderer is finished, and those belong with
//! the renderer connection rather than ahead of it. An entry point that
//! accepted frames and dropped them would report credits nobody consumed, which
//! is worse than not having one -- an exported symbol that always succeeds is
//! how a Windows SDK once shipped able to load and unable to attach.
//!
//! What *is* wired is the half that has to be right before frames arrive: the
//! ingress learns about surface changes and context loss from the same events
//! the renderer does, so a producer's packet is measured against the timeline
//! the renderer is actually on.

use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicU64, Ordering},
};

use parking_lot::Mutex;
use tracing::{debug, error, info, warn};

use shared::{
    config::InitOptions,
    error::{EngineResult, ErrorCode},
    protocol::host_cmd::HostCommand,
    render_event::RenderEvent,
    surface::SurfaceRef,
};

use frame_wire::FrameIngress;

use crate::runtime::session_thread::{
    HostThread, SessionThreadContext, StartedHost, create_basic_runtime,
    create_runtime_before_ready, spawn_session_thread,
};
use crate::runtime::shell::SessionShell;
use crate::services::PlatformServices;

/// A running external-frame session.
///
/// The ingress is shared rather than owned by the thread because the transport
/// that will feed it runs on whichever thread the host's networking uses, and a
/// frame has to be validated and credited before it is queued. The lock is
/// taken once per frame, not once per drawing command -- at 120 Hz that is a
/// handful of microseconds a second, and the alternative is a channel hop on
/// the latency path this lane exists to shorten.
#[must_use = "a spawned session must be shut down and joined"]
pub struct ExternalFrameSession {
    host: HostThread,
    ingress: Arc<Mutex<FrameIngress>>,
    clock: Arc<ExternalFrameClock>,
    /// The lease for the public attachment handle, when the session was started
    /// with a Surface. Held for the session's life: it is what keeps the
    /// embedding host's generation from being reused underneath a renderer that
    /// is still drawing into it.
    _surface_resource: Option<shared::surface::SurfaceResourceLease>,
}

/// The frame clock, reachable from whichever thread the transport runs on.
///
/// A producer in another process asks for a frame; the host arms one vsync and
/// forwards the timestamp when it arrives. That is the same demand-driven shape
/// every other Migo platform already uses -- `requestAnimationFrame` here has
/// never been the browser's, it is `await op_await_next_frame()` fed by the
/// host -- so the cross-process version changes where the tick is delivered and
/// nothing about who drives it.
///
/// Deliberately not a channel. Arming a frame is a per-frame operation on the
/// latency path this lane exists to shorten, and putting it through the bounded
/// command queue would put it behind whatever else is queued.
#[derive(Default)]
pub struct ExternalFrameClock {
    /// Populated by the session thread once the renderer is up. A producer that
    /// asks before then is told no rather than silently ignored: a warm start
    /// has no clock yet, and a request that appears to succeed and produces no
    /// frame is indistinguishable from a hung renderer.
    inner: OnceLock<FrameClockParts>,
    ticks: AtomicU64,
    last_timestamp_millis: AtomicU64,
}

struct FrameClockParts {
    demand: shared::raf_signal::RafDemandRef,
    arm: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl ExternalFrameClock {
    /// Ask for one frame. Returns `false` if the session is not yet rendering.
    pub fn request_frame(&self) -> bool {
        let Some(parts) = self.inner.get() else {
            return false;
        };
        parts.demand.mark_waiting();
        if let Some(arm) = &parts.arm {
            arm();
        }
        true
    }

    /// How many frame signals the renderer has delivered to this session.
    pub fn ticks(&self) -> u64 {
        self.ticks.load(Ordering::Relaxed)
    }

    /// The most recent frame timestamp, in whole milliseconds.
    ///
    /// Whole milliseconds because it crosses an atomic, and the sub-millisecond
    /// part belongs in the packet the producer builds rather than in a counter
    /// anyone can read. The exact `f64` is what gets forwarded.
    pub fn last_timestamp_millis(&self) -> u64 {
        self.last_timestamp_millis.load(Ordering::Relaxed)
    }

    fn record(&self, timestamp_millis: f64) {
        self.ticks.fetch_add(1, Ordering::Relaxed);
        self.last_timestamp_millis
            .store(timestamp_millis as u64, Ordering::Relaxed);
    }
}

impl ExternalFrameSession {
    #[inline]
    pub fn id(&self) -> crate::runtime::HostId {
        self.host.id()
    }

    /// Credits currently available to the producer.
    pub fn remaining_credits(&self) -> u32 {
        self.ingress.lock().remaining_credits()
    }

    /// The surface generation packets must be addressed to.
    pub fn surface_generation(&self) -> u64 {
        self.ingress.lock().surface_generation()
    }

    /// The resource epoch packets must name ids from.
    pub fn resource_epoch(&self) -> u64 {
        self.ingress.lock().resource_epoch()
    }

    /// Whether the host has declared this epoch's resources verified.
    pub fn resources_ready(&self) -> bool {
        self.ingress.lock().resources_ready()
    }

    /// The host has verified every resource this epoch admits.
    pub fn mark_resources_ready(&self) {
        self.ingress.lock().mark_resources_ready();
    }

    /// The frame clock, for the transport that drives the producer.
    pub fn clock(&self) -> &Arc<ExternalFrameClock> {
        &self.clock
    }

    pub fn request_shutdown(&self) -> Result<(), String> {
        self.host.request_shutdown()
    }

    pub fn shutdown_and_join(&mut self) -> EngineResult<()> {
        self.host.shutdown_and_join()
    }
}

/// The generation a session's first ingress accepts.
///
/// One, because a fresh `RestartBoundary` issues one, and the ingress is built
/// before the session thread exists so the two have to agree rather than one
/// telling the other. Named rather than written twice: a test asserts this
/// against the boundary, and a second copy of the literal would let the call
/// site drift away from the thing that checks it -- which is the failure where
/// every packet is rejected as belonging to a dead generation, on a device,
/// with nothing in the log saying why.
const INITIAL_RUNTIME_GENERATION: u64 = 1;

/// Start a session that renders frames produced by an external agent.
///
/// `launch_nonce` is the 128-bit identity this session will accept packets
/// under. It is generated once per app launch by the host, paired with the
/// producer out of band, and never appears in a URL, a query string or a log --
/// it is the value that decides whether bytes arriving from another process
/// belong to this session at all.
pub fn spawn_external_frame_session(
    launch_nonce: u128,
    surface: Option<SurfaceRef>,
    graphics_platform: graphics::egl_platform::GraphicsPlatform,
    platform: Arc<dyn PlatformServices>,
    opt: InitOptions,
) -> EngineResult<ExternalFrameSession> {
    // Built here rather than on the session thread so the caller has the handle
    // the moment the spawn returns: a transport that connected before the
    // thread finished bringing up the renderer would otherwise have nowhere to
    // put the first packet, and "wait a bit" is not a protocol.
    let ingress = Arc::new(Mutex::new(FrameIngress::new(
        launch_nonce,
        INITIAL_RUNTIME_GENERATION,
    )));
    let thread_ingress = Arc::clone(&ingress);
    let clock = Arc::new(ExternalFrameClock::default());
    let thread_clock = Arc::clone(&clock);

    let started: StartedHost = spawn_session_thread(
        surface,
        graphics_platform,
        platform,
        opt,
        None,
        move |ctx| run_external_session(ctx, thread_ingress, thread_clock),
    )?;

    Ok(ExternalFrameSession {
        host: started.host,
        ingress,
        clock,
        _surface_resource: started.resource,
    })
}

/// The external session, start to finish, on its own thread.
fn run_external_session(
    ctx: SessionThreadContext,
    ingress: Arc<Mutex<FrameIngress>>,
    clock: Arc<ExternalFrameClock>,
) {
    let SessionThreadContext {
        id,
        host_tx,
        critical_host_tx,
        mut host_rx,
        initial_surface,
        graphics_platform,
        platform,
        platform_for_error,
        opt,
        surface_control,
        restart_boundary,
        ready_tx,
    } = ctx;

    let shell = match SessionShell::build(
        id,
        &host_tx,
        critical_host_tx,
        initial_surface,
        graphics_platform,
        &platform,
        &opt,
        surface_control,
    ) {
        Ok(shell) => shell,
        Err(error) => {
            error!("[Host {id}] failed to build the session shell: {error}");
            platform_for_error.notify_error(
                id,
                error.code.as_u16(),
                &error.msg,
                error.detail.as_deref().unwrap_or(""),
            );
            // `ready_tx` drops unsent, which is what turns this into a
            // synchronous error for the caller rather than a hang.
            return;
        }
    };

    let SessionShell {
        mut startup_guard,
        mut render,
        render_events,
        render_notify,
        backgrounded,
        raf_rx,
        raf_demand,
        request_vsync,
        // Host-side audio. In this lane the WebView never touches audio at all:
        // PCM does not cross the process boundary, only low-frequency control
        // messages do, so the service belongs here exactly as it does for an
        // embedded session.
        mut audio,
        // The network policy and capability snapshot are what the control
        // channel answers a producer's synchronous queries from; both land
        // with it.
        network_policy: _network_policy,
        gpu_caps: _gpu_caps,
        context_lost: _context_lost,
        timer_backgrounded: _timer_backgrounded,
        gpu_init_started: _gpu_init_started,
        t_start: _t_start,
    } = shell;

    // Publish the clock only once the renderer is up. Before this point
    // `request_frame` answers no, which is the truthful answer for a session
    // that cannot yet produce a frame.
    let _ = clock.inner.set(FrameClockParts {
        demand: Arc::clone(&raf_demand),
        arm: request_vsync.clone(),
    });

    // The generation the producer must stamp every packet with. The handle was
    // built before this thread existed, so the two have to agree rather than one
    // telling the other -- and they agree because a fresh `RestartBoundary`
    // issues generation 1 and the handle is constructed with 1. Asserted rather
    // than assumed: if that ever stops being true, every packet is rejected as
    // belonging to a dead generation, and the symptom is a black screen with no
    // error anywhere.
    {
        // The engine counts generations as `i64` and the wire carries `u64`.
        // The boundary starts at 1 and only ever increments, so the conversion
        // is total in practice; it is still checked, because the failure of an
        // unchecked cast here is a negative generation reappearing as a very
        // large positive one that happens to match nothing, which reads as
        // "the producer is broken".
        let live = u64::try_from(restart_boundary.current()).unwrap_or(u64::MAX);
        let expected = ingress.lock().runtime_generation();
        if live != expected {
            error!(
                "[Host {id}] the ingress accepts generation {expected} but the session is on \
                 {live}; every packet would be rejected as a dead generation"
            );
            platform_for_error.notify_error(
                id,
                ErrorCode::Internal.as_u16(),
                "external-frame session generation mismatch",
                "",
            );
            return;
        }
    }

    let runtime = match create_runtime_before_ready(ready_tx, create_basic_runtime) {
        Ok(runtime) => runtime,
        Err(error) => {
            error!("[Host {id}] failed to enter tokio runtime: {error}");
            platform_for_error.notify_error(
                id,
                error.code.as_u16(),
                &error.msg,
                error.detail.as_deref().unwrap_or(""),
            );
            return;
        }
    };
    startup_guard.disarm();

    let mut last_context_epoch = 0u64;
    runtime.block_on(async move {
        loop {
            tokio::select! {
                command = host_rx.recv() => {
                    let Some(command) = command else {
                        info!("[Host {id}] command channel closed");
                        break;
                    };
                    if !handle_command(
                        id, command, &mut render, &mut audio, &backgrounded, &ingress,
                    ) {
                        break;
                    }
                }
                () = render_notify.notified() => {
                    drain_render_events(id, &render_events, &ingress, &mut last_context_epoch);
                }
                timestamp = raf_rx.recv(raf_demand.session_ticket()) => {
                    match timestamp {
                        Some(timestamp) => clock.record(timestamp),
                        None => {
                            // The render thread is gone. Nothing else will
                            // arrive on this channel, and continuing to select
                            // on it would spin.
                            info!("[Host {id}] frame clock closed");
                            break;
                        }
                    }
                }
            }
        }
        // Audio first, then render: the audio thread can still be holding a
        // decoded buffer whose lifetime is tied to this session, and stopping
        // the renderer first would leave it writing into a context that is
        // going away.
        audio.shutdown();
        render.shutdown();
        info!("[Host {id}] external-frame session exited");
    });
}

/// Returns `false` when the session should stop.
fn handle_command(
    id: crate::runtime::HostId,
    command: HostCommand,
    render: &mut crate::services::RenderService,
    audio: &mut crate::services::AudioService,
    backgrounded: &Arc<std::sync::atomic::AtomicBool>,
    ingress: &Arc<Mutex<FrameIngress>>,
) -> bool {
    use std::sync::atomic::Ordering;

    match command {
        HostCommand::Shutdown => return false,

        HostCommand::UpdateSurface { lease, pixel_ratio } => {
            // The surface generation advances *before* the renderer is told, so
            // a packet built against the previous surface cannot be accepted in
            // the window between the two. The producer is in another process
            // and does not stop when the screen rotates.
            let generation = lease.generation().get();
            if !ingress.lock().set_surface_generation(generation) {
                error!(
                    "[Host {id}] refused a surface generation that moves backwards: {generation}"
                );
            }
            if let Err(error) = render.update_surface(lease, pixel_ratio) {
                error!("[Host {id}] update_surface failed: {error:?}");
            }
        }

        HostCommand::SurfaceDestroyed { generation } => {
            render.on_surface_destroyed(generation);
        }

        HostCommand::SurfaceLost {
            public_generation,
            reason,
        } => {
            warn!("[Host {id}] surface {public_generation:?} lost: {reason:?}");
            render.pause();
        }

        HostCommand::OnShow { .. } => {
            backgrounded.store(false, Ordering::Relaxed);
            // Only resume against a surface that is actually live. Android
            // fires `onResume` before `surfaceCreated`, so on that path the old
            // surface is already gone and the resume belongs to the
            // `UpdateSurface` that follows; resuming here would run a renderer
            // with nothing to present into.
            if render.has_live_surface() {
                render.resume();
                audio.resume();
            } else {
                debug!("[Host {id}] OnShow with no live surface; resume waits for UpdateSurface");
            }
        }

        HostCommand::OnHide => {
            backgrounded.store(true, Ordering::Relaxed);
            render.pause();
            audio.pause();
        }

        HostCommand::OnAudioInterruptionBegin => audio.pause(),
        HostCommand::OnAudioInterruptionEnd => audio.resume(),

        // Everything else is addressed to a script runtime this session does
        // not have. Logged rather than silently dropped: the two that will
        // arrive in production -- input and the frame clock -- belong to the
        // control channel that carries them to the producer, and until that
        // exists a host sending them is a host expecting something to happen.
        other => {
            debug!(
                "[Host {id}] {other:?} has no consumer in an external-frame session; \
                 input and clock delivery arrive with the control channel"
            );
        }
    }
    true
}

/// Drain what the renderer has to say, and keep the ingress on the same
/// timeline it is.
fn drain_render_events(
    id: crate::runtime::HostId,
    events: &shared::render_event::RenderEventReceiver,
    ingress: &Arc<Mutex<FrameIngress>>,
    last_context_epoch: &mut u64,
) {
    while let Ok(event) = events.try_recv() {
        match event {
            RenderEvent::ContextLost => {
                // Every resource id the producer holds now names nothing, or
                // worse, names whatever the rebuilt table put in its place. The
                // epoch advance is what makes those ids fail loudly, and it
                // withdraws readiness in the same call so a frame cannot name a
                // resource between the loss and the host re-verifying the table.
                *last_context_epoch += 1;
                let epoch = *last_context_epoch;
                if !ingress.lock().set_resource_epoch(epoch) {
                    error!("[Host {id}] refused a resource epoch that moves backwards: {epoch}");
                }
                warn!("[Host {id}] GL context lost; resource epoch is now {epoch}");
            }
            RenderEvent::ContextRecovered { success } => {
                info!("[Host {id}] GL context recovered: success={success}");
            }
            RenderEvent::SwapFailed { message } => {
                warn!("[Host {id}] swap failed: {message}");
            }
            other => {
                debug!("[Host {id}] render event: {other:?}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The source of this module, for the invariants that are about what it
    /// does *not* contain. A dependency-closure gate proves no engine is
    /// reachable from the built product; this proves the module was written
    /// that way rather than happening to resolve that way today.
    const SOURCE_WITH_TESTS: &str = include_str!("external.rs");

    /// Everything above this test module. Scanning the whole file would find
    /// the forbidden names in the list below and fail on its own words -- the
    /// self-reference this repository has already been caught by once, in an
    /// audit that came up four short because the auditing file counted itself.
    fn source() -> &'static str {
        let end = SOURCE_WITH_TESTS
            .find("#[cfg(test)]")
            .expect("this module has a test section");
        &SOURCE_WITH_TESTS[..end]
    }

    #[test]
    fn the_external_session_never_names_a_script_runtime() {
        for forbidden in [
            "HostJsRuntime",
            "JsRuntimeSlot",
            "runtime_v8",
            "invoke_host_hook",
            "EvaluateModule",
        ] {
            assert!(
                !source().contains(forbidden),
                "external.rs names {forbidden}; this session exists because there is no \
                 script runtime in this process"
            );
        }
    }

    /// A producer that asks for a frame before the renderer is up is told no.
    ///
    /// The alternative -- returning success and arming nothing -- is
    /// indistinguishable at the far end from a renderer that has hung, and the
    /// far end is in another process with no way to tell them apart.
    #[test]
    fn the_clock_refuses_before_the_renderer_is_up() {
        let clock = ExternalFrameClock::default();
        assert!(!clock.request_frame());
        assert_eq!(clock.ticks(), 0);
        assert_eq!(clock.last_timestamp_millis(), 0);
    }

    #[test]
    fn recorded_ticks_accumulate_and_keep_the_latest_timestamp() {
        let clock = ExternalFrameClock::default();
        clock.record(16.7);
        clock.record(33.4);
        clock.record(50.1);
        assert_eq!(clock.ticks(), 3);
        assert_eq!(
            clock.last_timestamp_millis(),
            50,
            "the counter carries whole milliseconds; the exact value is what gets forwarded"
        );
    }

    /// Every packet is measured against the generation the session is on, and
    /// the handle is built before the session thread exists. The two agree
    /// because a fresh boundary issues generation 1; if that ever changes, the
    /// session refuses to start rather than rejecting every frame.
    #[test]
    fn a_fresh_boundary_and_a_fresh_ingress_agree_on_the_generation() {
        let boundary = crate::runtime::restart_boundary::RestartBoundary::new();
        assert_eq!(
            u64::try_from(boundary.current()).expect("generations are positive"),
            INITIAL_RUNTIME_GENERATION,
            "spawn_external_frame_session builds the ingress with 1 because a fresh \
             RestartBoundary issues 1"
        );
    }

    #[test]
    fn a_new_ingress_admits_no_resources_and_starts_at_the_full_credit_window() {
        let ingress = FrameIngress::new(1, 1);
        assert!(!ingress.resources_ready());
        assert_eq!(ingress.resource_epoch(), 0);
        assert_eq!(ingress.surface_generation(), 0);
        assert_eq!(
            ingress.remaining_credits(),
            frame_wire::ingress::MAX_CREDITS
        );
    }
}
