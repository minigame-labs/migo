//! Parallel GPU/V8 startup ordering, pinned against the source.

const HOST: &str = include_str!("../host.rs");
const THREAD: &str = include_str!("../thread.rs");
const RENDER_SERVICE: &str = include_str!("../../services/render.rs");
const RENDER_THREAD: &str = include_str!("../../../../graphics/src/render_thread.rs");
const CANVAS_MANAGER: &str = include_str!("../../../../graphics/src/canvas/manager/mod.rs");
const GPU_CAPS: &str = include_str!("../../../../shared/src/device/gpu_caps.rs");

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
    let render = body
        .find("RenderService::new(")
        .expect("Host::new must start RenderService");
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
    assert!(HOST.contains("struct HostStartupGuard"));
    assert!(HOST.contains("impl Drop for HostStartupGuard"));
    assert!(body.contains("startup_guard.mark_console_registered()"));
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
        .find("if let Some(lease) = initial_surface")
        .expect("render initialization must resolve the initial SurfaceLease");
    let guarded_publish = render_init
        .find("if !startup_failed")
        .expect("failed surface setup must not publish successful caps");
    let publish = render_init
        .find("cm.publish_gpu_caps()")
        .expect("successful render initialization must publish caps");
    assert!(surface < guarded_publish && guarded_publish < publish);
}

#[test]
fn render_init_panic_wakes_gpu_join_as_failure() {
    let panic_handler = RENDER_THREAD
        .split("if let Err(panic_info) = result")
        .nth(1)
        .expect("render panic handler must remain present");
    assert!(panic_handler.contains("if !gpu_caps.is_ready()"));
    assert!(panic_handler.contains("gpu_caps.set_failed"));
}

#[test]
fn caller_ready_signal_stays_after_host_construction() {
    let construct = THREAD
        .find("Host::new(")
        .expect("host thread must construct Host");
    let ready = THREAD
        .find("ready_tx.send(())")
        .expect("host thread must publish readiness");
    assert!(construct < ready);
}
