//! Parallel GPU/V8 startup ordering, pinned against the source.

const HOST: &str = include_str!("../host.rs");
const SHELL: &str = include_str!("../shell.rs");
const SESSION_THREAD: &str = include_str!("../session_thread.rs");
const THREAD: &str = include_str!("../thread.rs");
const RENDER_SERVICE: &str = include_str!("../../services/render.rs");
const RENDER_THREAD: &str = include_str!("../../../../graphics/src/render_thread.rs");
const CANVAS_MANAGER: &str = include_str!("../../../../graphics/src/canvas/manager/mod.rs");
const GPU_CAPS: &str = include_str!("../../../../shared/src/device/gpu_caps.rs");
const CONTROL: &str = include_str!("../../../../shared/src/surface/control.rs");
const RENDER_CMD: &str = include_str!("../../../../shared/src/protocol/render_cmd.rs");
const EXTERNAL: &str = include_str!("../external.rs");
const REGISTRY: &str = include_str!("../registry.rs");
const CAPI_SURFACE: &str = include_str!("../../../../capi/src/surface.rs");

fn host_new_body() -> &'static str {
    let start = HOST
        .find("pub(crate) fn new(")
        .expect("Host::new must remain present");
    let end = HOST[start..]
        .find("pub(crate) async fn handle_command")
        .map(|offset| start + offset)
        .expect("Host::new must end before handle_command");
    &HOST[start..end]
}

#[test]
fn render_starts_before_v8_and_host_never_waits_for_the_gpu() {
    let body = host_new_body();
    // The render thread is launched by the session shell, which is engine-neutral
    // and shared with the external-frame execution. That makes "render before V8"
    // structural rather than an ordering someone has to preserve: the shell
    // returns before any line of `Host::new` can name a JavaScript runtime, and
    // the shell has no way to name one.
    assert!(
        SHELL.contains("RenderService::new("),
        "the session shell must be what starts RenderService"
    );
    assert!(
        !SHELL.contains("HostJsRuntime"),
        "the session shell must stay free of the embedded engine; it is compiled \
         into the external-frame product, which links none"
    );
    let render = body
        .find("SessionShell::build(")
        .expect("Host::new must build the session shell, which starts RenderService");
    let js = body
        .find("HostJsRuntime::new(")
        .expect("Host::new must construct V8 on the host thread");
    let watchdog = body
        .find("install_watchdog")
        .expect("watchdog installation must remain present");
    let assemble = body
        .find("let host = Self {")
        .expect("Host must be assembled before publication");

    assert!(
        render < js && js < watchdog && watchdog < assemble,
        "required startup order is render -> V8 -> watchdog -> Host"
    );

    // `Host::new` runs on the host thread while the caller that asked for a
    // session is blocked, so anything it waits for is time a host waits before
    // it can start a game. It waited for GPU readiness here once, at a measured
    // 30-44 ms; nothing between here and the first line of launch JS reads the
    // capabilities, so the wait belongs at that point instead. See
    // `gpu_readiness_is_joined_before_any_launch_js`.
    assert!(
        !body.contains("wait_ready("),
        "Host::new must not block on GPU readiness; the wait belongs in ensure_gpu_ready"
    );
}

#[test]
fn gpu_readiness_is_joined_before_any_launch_js() {
    // The property: no JS supplied for a launch -- prelude or module -- may run
    // against the provisional all-false capability snapshot, because image ops
    // read it to choose upload paths. `on_eval_script` is deliberately not
    // covered: it evaluates host-supplied source on a host-driven command, and
    // is not part of launching a game.
    let wait = HOST
        .find("fn ensure_gpu_ready(")
        .expect("the deferred GPU join must remain present");
    let wait_body = &HOST[wait..];
    let deadline = wait_body
        .find("wait_ready_until(self.gpu_init_started, GPU_INIT_TIMEOUT)")
        .expect("the join must use the budget that started when the render thread launched");
    assert!(
        deadline
            < wait_body
                .find("fn ")
                .map(|_| wait_body.len())
                .unwrap_or(wait_body.len()),
        "deadline lookup must stay inside ensure_gpu_ready"
    );

    let start = HOST
        .find("async fn on_evaluate_module(")
        .expect("the launch path must remain present");
    let end = HOST[start..]
        .find("\n    fn ")
        .map(|offset| start + offset)
        .expect("on_evaluate_module must end");
    let body = &HOST[start..end];

    let ready = body
        .find("self.ensure_gpu_ready()?")
        .expect("the launch path must join GPU readiness");
    let prelude = body
        .find("exec_script_owned(")
        .expect("prelude execution must remain present");
    let module = body
        .find(".evaluate_module(")
        .expect("module evaluation must remain present");
    assert!(
        ready < prelude && ready < module,
        "GPU capabilities must be published before any prelude or module JS runs"
    );
}

#[test]
fn host_drop_destroys_v8_before_stopping_render() {
    // V8 must be destroyed on the thread that owns its isolate, and that thread
    // is also the one that stops GL -- so the isolate has to go first. This used
    // to be pinned only on `Host::new`'s GPU-failure path, which was one of the
    // ways a host is torn down; `Drop` is all of them.
    let body = HOST
        .split("impl Drop for Host {")
        .nth(1)
        .expect("Host must implement Drop");
    let drop_js = body
        .find("self.js.take_and_drop();")
        .expect("Host teardown must explicitly drop V8");
    let shutdown = body
        .find("self.render.shutdown();")
        .expect("Host teardown must stop render");
    assert!(
        drop_js < shutdown,
        "V8 must be destroyed before render shutdown on every teardown path"
    );
}

#[test]
fn startup_registrations_are_guarded_until_host_publication() {
    let body = host_new_body();
    // The guard moved to the shell with the registrations it protects: the console
    // buffer is registered during the neutral bring-up, so a guard living beside
    // `Host` would have been guarding something it could no longer see.
    assert!(SHELL.contains("struct HostStartupGuard"));
    assert!(SHELL.contains("impl Drop for HostStartupGuard"));
    assert!(SHELL.contains("startup_guard.mark_console_registered()"));
    // The frame clock's sender is deliberately not one of them any more, and
    // needing a branch here was the symptom rather than the fix. It lived in a
    // registry of its own, so retiring it was somebody's job on each of spawn
    // failure, startup failure and ordinary exit -- and on the external-frame
    // product nobody held that job, so every session leaked an entry. It now sits
    // in the Host registry handle, retired by the `unregister_sender` that every
    // exit path already goes through.
    assert!(
        !SHELL.contains("mark_vsync_registered"),
        "the frame clock's sender must not need guarding: its lifetime is the Host \
         registry handle's"
    );
    assert!(
        !SHELL.contains("register_vsync_sender"),
        "the shell must not register a frame clock behind the ingress's back"
    );
    assert!(
        REGISTRY.contains("vsync_tx: Option<crossbeam_channel::Sender<f64>>,"),
        "the Host registry handle must own the frame clock's sender"
    );
    // And it is still handed back armed, so the embedded half's own failure
    // paths are covered by it rather than by nothing.
    assert!(
        body.contains("mut startup_guard,"),
        "Host::new must take the armed guard from the shell"
    );
    let disarm = body
        .find("startup_guard.disarm();")
        .expect("successful Host construction must transfer cleanup ownership");
    let assemble = body
        .find("let host = Self {")
        .expect("Host must be assembled while the guard is armed");
    let publish = body.find("Ok(host)").expect("Host publication must remain");
    assert!(
        assemble < disarm && disarm < publish,
        "guard must remain armed through assembly and disarm immediately before publication"
    );
}

#[test]
fn a_session_is_handed_its_own_ingress_rather_than_looking_it_up() {
    // The property: `attach` never asks the registry for the session it is
    // starting.
    //
    // It used to. The session thread removes its own entry on the way out, so a
    // renderer that failed to initialise took the entry away while `attach` was
    // still walking towards it -- measured on the iOS simulator, `attach` lost that
    // race about two runs in three and answered MIGO_ERROR_INTERNAL where the other
    // third answered MIGO_OK, for one input. Every part of the ingress exists before
    // any thread does, so the registration hands it back and there is nothing to
    // lose.
    assert!(
        REGISTRY.contains(") -> HostIngress {"),
        "registration must answer with the ingress it published"
    );
    assert!(
        SESSION_THREAD.contains("pub(crate) ingress: HostIngress,"),
        "the spawn must carry the session's own ingress out"
    );
    assert!(
        !CAPI_SURFACE.contains("host_ingress("),
        "attach must not look up the session it is installing; that lookup is for \
         callers who only have an id, and whose question is whether it is still alive"
    );
    assert!(
        CAPI_SURFACE.contains("Some(started.ingress)"),
        "attach must install the ingress the spawn handed it"
    );
}

#[test]
fn render_service_drop_is_a_detached_shutdown_fallback() {
    assert!(RENDER_SERVICE.contains("impl Drop for RenderService"));
    let drop_impl = RENDER_SERVICE
        .split("impl Drop for RenderService")
        .nth(1)
        .expect("RenderService Drop implementation");
    assert!(drop_impl.contains("self.shutdown_detached();"));
}

#[test]
fn detached_shutdown_is_a_noop_after_joined_shutdown() {
    let detached = RENDER_THREAD
        .split("pub fn shutdown_detached(&mut self)")
        .nth(1)
        .expect("detached render shutdown must remain present");
    let handle = detached
        .find("let Some(h) = self.handle.take() else")
        .expect("detached shutdown must return when the join handle is gone");
    let send = detached
        .find("self.cmd_tx.send(RenderCommand::Shutdown)")
        .expect("live render shutdown must send the shutdown command");
    assert!(
        handle < send,
        "the handle must be claimed before sending so repeated shutdown is a no-op"
    );
}

#[test]
fn shutdown_request_cannot_be_lost_when_the_command_queue_is_full() {
    assert!(
        RENDER_THREAD.contains("shutdown_requested: Arc<std::sync::atomic::AtomicBool>"),
        "RenderThread must retain an out-of-band shutdown level"
    );

    for function in [
        "pub fn shutdown(&mut self)",
        "pub fn shutdown_detached(&mut self)",
    ] {
        let body = RENDER_THREAD
            .split(function)
            .nth(1)
            .unwrap_or_else(|| panic!("{function} must remain present"));
        let handle = body
            .find("self.handle.take()")
            .expect("shutdown must claim the join handle");
        let request = body
            .find("self.shutdown_requested.store(true, Ordering::Release)")
            .expect("shutdown must publish the out-of-band exit level");
        let send = body
            .find("self.cmd_tx.send(RenderCommand::Shutdown)")
            .expect("shutdown command must still wake an idle receiver");
        assert!(
            handle < request && request < send,
            "claim handle, publish exit level, then send the best-effort wake"
        );
    }

    let recovery = RENDER_THREAD
        .find("// --- Deferred EGL context recovery ---")
        .expect("render loop recovery marker must remain present");
    let loop_start = RENDER_THREAD[..recovery]
        .rfind("loop {")
        .expect("main render loop must remain present");
    let loop_prefix = &RENDER_THREAD[loop_start..recovery];
    assert!(loop_prefix.contains("shutdown_requested.load(Ordering::Acquire)"));
}

#[test]
fn gpu_failure_detail_is_initialized_before_ready_publication() {
    let body = GPU_CAPS
        .split("pub fn set_failed")
        .nth(1)
        .expect("GpuCaps::set_failed must remain present");
    let detail = body
        .find("*failure = Some(detail)")
        .expect("failure detail must be stored");
    let failed = body
        .find("self.failed.store(true, Ordering::Release)")
        .expect("failure level must be published");
    let ready = body
        .find("self.ready.store(true, Ordering::Release)")
        .expect("ready level must be published");
    assert!(
        detail < failed && failed < ready,
        "failure detail must happen-before the ready publication"
    );
}

#[test]
fn gpu_caps_publish_only_after_initial_surface_outcome() {
    let manager_start = CANVAS_MANAGER
        .find("pub(crate) fn new_with_resource(")
        .expect("CanvasManager constructor must remain present");
    let manager_end = CANVAS_MANAGER[manager_start..]
        .find("fn new_canvas_id")
        .map(|offset| manager_start + offset)
        .expect("CanvasManager constructor must end before new_canvas_id");
    assert!(
        !CANVAS_MANAGER[manager_start..manager_end].contains("gpu_caps.set("),
        "CanvasManager construction must not publish caps before the initial surface"
    );
    assert!(CANVAS_MANAGER.contains("pub(crate) fn publish_gpu_caps(&self)"));

    let render_init = RENDER_THREAD
        .split("CanvasManager::new_with_resource")
        .nth(1)
        .expect("render initialization must construct CanvasManager");
    let surface = render_init
        .find("surface_control.live_candidate()")
        .expect("render initialization must claim the initial Surface from the control plane");
    let guarded_publish = render_init
        .find("if !startup_failed")
        .expect("failed surface setup must not publish successful caps");
    let publish = render_init
        .find("cm.publish_gpu_caps()")
        .expect("successful render initialization must publish caps");
    assert!(surface < guarded_publish && guarded_publish < publish);
}

#[test]
fn the_initial_surface_is_claimed_after_gpu_init_and_by_one_route_only() {
    // The property: no lease waits anywhere the host cannot hurry.
    //
    // A `SurfaceLease` pins the native Surface, and RELEASED is published by the
    // last one going away, so wherever a lease waits, `migo_surface_begin_detach`
    // waits with it. It waited in two places: handed to the worker at spawn it was
    // owned across GPU bring-up -- measured at 33 ms on macOS and 5.7-41 s on the
    // iOS simulator, where ANGLE compiles its Metal shaders cold -- and carried
    // inside `RecreateOnscreen` it sat in a queue behind the same phase. Neither
    // pin bought anything: `CanvasManager` construction takes an `EglProvider` and
    // never names a window, and the candidate's liveness is re-checked at install
    // regardless.
    //
    // Three halves, and the ordering one alone is not enough -- either delivery
    // route reappearing would restore the pin while this file still looked right.
    let claim = RENDER_THREAD
        .find("surface_control.live_candidate()")
        .expect("the render thread must claim its candidate from the control plane");
    let init = RENDER_THREAD
        .find("CanvasManager::new_with_resource(")
        .expect("render initialization must construct CanvasManager");
    assert!(
        init < claim,
        "the candidate must be claimed after GPU initialization, not carried through it"
    );

    let spawn = RENDER_THREAD
        .split("pub fn spawn(")
        .nth(1)
        .expect("RenderThread::spawn must remain present");
    let signature = spawn
        .split(") -> EngineResult<Self> {")
        .next()
        .expect("RenderThread::spawn must have a signature");
    assert!(
        !signature.contains("SurfaceLease"),
        "RenderThread::spawn must take no Surface: the control plane is the only \
         route, so there is nowhere else for a pin to reappear"
    );

    // The second route that used to exist, and the reason the level is read rather
    // than consumed. `RecreateOnscreen` carried an owning lease, so a pre-ready
    // re-attach left the host's Surface pinned in a bounded queue behind the same
    // initialization -- unbounded, because `RenderService` gives up on the reply
    // after 500 ms and the lease stays queued regardless.
    let command = RENDER_CMD
        .split("    RecreateOnscreen {")
        .nth(1)
        .expect("the RecreateOnscreen command must remain present")
        .split("    },")
        .next()
        .expect("the RecreateOnscreen command must end");
    assert!(
        !command.contains("SurfaceLease"),
        "RecreateOnscreen must carry no Surface: it is the wake and the reply \
         channel for a level, not the delivery of a lease"
    );
    assert!(
        RENDER_SERVICE.contains("self.surface_control.publish_candidate(lease.clone());"),
        "an update must publish through the control plane"
    );

    // And the control plane must actually be able to revoke what it parked.
    assert!(
        CONTROL.contains("candidate: Mutex<Option<SurfaceLease>>"),
        "SurfaceControl must own the published candidate"
    );
    for retire in [
        "pub fn retire_current_and_request(&self)",
        "pub fn retire_generation_and_request(&self, expected: SurfaceGeneration)",
    ] {
        let body = CONTROL
            .split(retire)
            .nth(1)
            .unwrap_or_else(|| panic!("{retire} must remain present"))
            .split("\n    pub fn ")
            .next()
            .expect("the retirement body must end");
        assert!(
            body.contains("self.release_dead_candidate();"),
            "{retire} must revoke a published candidate it just made unusable"
        );
    }
}

#[test]
fn a_surface_the_host_took_back_during_gpu_init_is_not_a_startup_failure() {
    // The property: "the host retired it while the GPU was coming up" is
    // cancellation, not a render failure.
    //
    // `gpu_caps.set_failed` is what `ensure_gpu_ready` turns into
    // `Render2DInitError` before the first line of launch JS, so routing this
    // through it aborts a launch over a Surface the host itself took back -- and
    // names the GPU as the culprit while the GPU is fine. The truthful state is
    // the one a warm start begins in: caps Ready, no Surface, waiting for the
    // `UpdateSurface` that brings the next one.
    let install = RENDER_THREAD
        .split("if let Some(lease) = claimed {")
        .nth(1)
        .expect("the initial install must remain present");
    let arm = install
        .find("Err(SurfaceRecreateError::Binding(SurfaceBindingError::StaleGeneration))")
        .expect("a retired candidate must be matched separately from a real failure");
    let blanket = install
        .find("gpu_caps.set_failed(")
        .expect("a real install failure must still publish Failed");
    assert!(
        arm < blanket,
        "the cancellation arm must be matched before the arm that publishes Failed"
    );
    let cancelled = &install[arm..blanket];
    assert!(
        !cancelled.contains("set_failed"),
        "a retired candidate must not publish a GPU failure"
    );
    assert!(
        !cancelled.contains("startup_failed = true"),
        "a retired candidate must not suppress the caps publication the GPU earned"
    );
}

#[test]
fn render_init_panic_wakes_gpu_join_as_failure() {
    let panic_handler = RENDER_THREAD
        .split("Err(panic_info) => {")
        .nth(1)
        .expect("render panic handler must remain present");
    assert!(panic_handler.contains("if !gpu_caps.is_ready()"));
    assert!(panic_handler.contains("gpu_caps.set_failed"));
}

#[test]
fn the_render_worker_names_every_way_it_stops() {
    // The property: a session that observes its renderer gone can say why.
    //
    // It could not before. The worker logged its reason and dropped its frame
    // clock sender; the external session logged "frame clock closed" and exited;
    // and the host, whose `on_error` exists for this, heard nothing. What it heard
    // instead was `attach` losing a race for a registry entry the exiting session
    // was removing -- about two runs in three on the iOS simulator, arriving as a
    // bare MIGO_ERROR_INTERNAL with no reason attached.
    //
    // Pinned in three parts, because each one alone is satisfiable while the
    // property is false.
    let body = RENDER_THREAD
        .split("std::panic::catch_unwind(std::panic::AssertUnwindSafe(")
        .nth(1)
        .expect("the render body must remain inside the panic barrier");

    // One: the body is typed, so an exit cannot decline to say which it was.
    assert!(
        body.contains("|| -> Result<(), EngineError> {"),
        "the render body must return a Result, so every exit names itself rather \
         than relying on whoever wrote it to remember"
    );

    // Two: the reason is published in exactly one place, at the tail. Two publish
    // sites is how a later exit comes to be reported by neither.
    let publishes = RENDER_THREAD
        .matches("render_exit.publish_failure(")
        .count();
    assert_eq!(
        publishes, 2,
        "the tail must be the only publisher: one site for a failed body and one \
         for a panic, and nothing anywhere else"
    );
    // Anchored on the barrier's own closing marker, not on `match result {`:
    // there are two of those in this file and `nth(1)` picked the wrong one, which
    // is the extraction-by-name-pattern mistake this suite exists to avoid.
    let tail = RENDER_THREAD
        .split("})); // end catch_unwind")
        .nth(1)
        .expect("the tail must classify the body's outcome");
    assert!(
        tail.contains("Ok(Ok(())) => {}"),
        "a body that ran and was asked to stop must record nothing"
    );
    assert!(
        tail.contains("Ok(Err(failure)) => render_exit.publish_failure(failure)"),
        "a body that failed must publish the failure it returned"
    );

    // Three: a panic publishes whatever `gpu_caps` says. A panic after the first
    // frame leaves that level reporting Ready, so gating the terminal reason on it
    // -- the way the caps write beside it is gated, correctly -- would lose exactly
    // the panics that happen once a session is running.
    let panic_arm = tail
        .split("Err(panic_info) => {")
        .nth(1)
        .expect("the panic arm must remain present");
    let caps_guard = panic_arm
        .find("if !gpu_caps.is_ready()")
        .expect("the caps write must stay guarded");
    let publish = panic_arm
        .find("render_exit.publish_failure(")
        .expect("a panic must publish a terminal reason");
    assert!(
        caps_guard < publish,
        "the guarded caps write comes first; the unguarded publish follows it"
    );
    let guarded = &panic_arm[caps_guard..publish];
    assert_eq!(
        guarded.matches('{').count(),
        guarded.matches('}').count(),
        "the publish must sit outside the readiness guard, not inside it"
    );
}

#[test]
fn the_session_reports_the_reason_its_renderer_stopped() {
    // The other half: publishing is worth nothing if nobody reads it. The external
    // execution is the one with a `select!` arm that can see the frame clock close,
    // and that arm is where the host has to be told.
    let arm = EXTERNAL
        .split("timestamp = raf_rx.recv(raf_demand.session_ticket())")
        .nth(1)
        .expect("the external session must select on its frame clock")
        .split("\n            }")
        .next()
        .expect("the frame-clock arm must end");
    let closed = arm
        .find("None => {")
        .expect("a closed frame clock must still be handled");
    let closed = &arm[closed..];
    assert!(
        closed.contains("render_exit.failure()"),
        "the closed-clock branch must read why the worker stopped"
    );
    assert!(
        closed.contains("platform_for_error.notify_error("),
        "and must tell the host, which is the whole point of recording it"
    );
    assert!(
        closed.contains("\"the renderer stopped\""),
        "a worker that stopped without recording a reason must still be reported: \
         this branch is unreachable on a requested shutdown, so silence here would \
         only ever hide a real failure"
    );
}

#[test]
fn caller_ready_signal_stays_after_host_construction() {
    // Readiness is published by the shared runtime helper, which the embedded
    // body calls only after `Host::new` has returned. Keeping the order pinned
    // across the two files is the point: the helper moved, and a caller that
    // signalled ready before construction finished would turn a construction
    // failure into a hang rather than into a synchronous error.
    let construct = THREAD
        .find("Host::new(")
        .expect("host thread must construct Host");
    let publish = THREAD
        .find("create_runtime_before_ready(")
        .expect("host thread must publish readiness through the shared helper");
    assert!(
        construct < publish,
        "readiness must be published after Host construction, not before"
    );
    assert!(
        SESSION_THREAD.contains("ready_tx.send(())"),
        "the shared helper is what actually signals readiness"
    );
}

#[test]
fn every_startup_error_the_caller_sees_was_announced_exactly_once() {
    // The property: `spawn_session_thread` returns no `Err` the host has not
    // already heard about, and none it has heard about twice.
    //
    // It needs pinning because the two halves look interchangeable from either
    // end. A failure inside the thread announces its own specific reason and
    // *then* drops `ready_tx` -- and dropping `ready_tx` unsent is the only
    // thing the caller's handshake can observe. So a caller that reports what
    // the handshake told it delivers a second, vaguer callback for a failure
    // that already spoke, and the host cannot collapse the two: the notifier
    // posts every event independently. That is not hypothetical; the duplicate
    // existed, at the C boundary, and the two causes are distinguishable there
    // only by matching on the error message.
    let start = SESSION_THREAD
        .find("pub(crate) fn spawn_session_thread<Body>(")
        .expect("spawn_session_thread must remain present");
    let body = &SESSION_THREAD[start..];
    let end = body
        .find("pub(crate) struct StartedHost")
        .expect("spawn_session_thread must end before StartedHost");
    let body = &body[..end];

    let spawn_at = body
        .find("let spawn_result = thread::Builder::new()")
        .expect("the thread spawn must remain present");
    let join_at = body
        .find("let join = match spawn_result {")
        .expect("the spawn outcome must still be matched");
    let host_at = body
        .find("let host = HostThread::new(id, join);")
        .expect("the spawn-failure arm must end at the HostThread it could not build");
    let handshake_at = body
        .find("if ready_rx.recv().is_err() {")
        .expect("the cold-start handshake must remain present");

    // The two regions where no thread can be holding `ready_tx`: before the
    // spawn statement, and the arm where the spawn itself failed and dropped the
    // closure without running it. Derived by pairing counts rather than by
    // listing the exits, so an exit added without a report moves one count and
    // not the other.
    for (region, what) in [
        (&body[..spawn_at], "before the thread is spawned"),
        (&body[join_at..host_at], "when the spawn itself fails"),
    ] {
        let built = region.matches("EngineError::new(").count();
        let announced = region.matches("report_unspawned_failure(").count();
        assert!(
            built > 0,
            "the region {what} must still have a failure to announce"
        );
        assert_eq!(
            built, announced,
            "every error built {what} must be announced, because no thread exists \
             to announce it: {built} built, {announced} announced"
        );
    }

    // And the mirror. Past the handshake the thread has already spoken for every
    // way it can fail, including a panic, which the barrier reports.
    let after_handshake = &body[handshake_at..];
    assert!(
        !after_handshake.contains("report_unspawned_failure("),
        "the handshake observes a failure that already reported; announcing it \
         again is the duplicate this test exists to prevent"
    );
    assert!(
        !after_handshake.contains("notify_error("),
        "the handshake must not report through any route"
    );

    // The pairing above is only meaningful if the helper is the sole reporter in
    // those two regions -- a bare `notify_error` there would satisfy no count.
    assert!(
        !body[..spawn_at].contains("notify_error("),
        "pre-spawn reporting must go through report_unspawned_failure"
    );
    assert!(
        !body[join_at..host_at].contains("notify_error("),
        "spawn-failure reporting must go through report_unspawned_failure"
    );
}
