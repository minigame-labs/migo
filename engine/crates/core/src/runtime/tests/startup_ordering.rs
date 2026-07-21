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
fn render_starts_before_v8_and_gpu_join_follows_v8() {
    let body = host_new_body();
    let render = body
        .find("RenderService::new(")
        .expect("Host::new must start RenderService");
    let js = body
        .find("HostJsRuntime::new(")
        .expect("Host::new must construct V8 on the host thread");
    let join = body
        .find("wait_ready_until(")
        .expect("Host::new must join GPU init with the original deadline");
    let watchdog = body
        .find("install_watchdog")
        .expect("watchdog installation must remain present");
    let assemble = body
        .find("let host = Self {")
        .expect("Host must be assembled before publication");

    assert!(
        render < js && js < join && join < watchdog && watchdog < assemble,
        "required startup order is render -> V8 -> GPU join -> watchdog -> Host"
    );
    assert!(
        !body[render..js].contains("wait_ready("),
        "the host must not block on GPU readiness before V8 construction"
    );
}

#[test]
fn gpu_join_failure_drops_v8_before_render_shutdown() {
    let body = host_new_body();
    let join = body
        .find("wait_ready_until(")
        .expect("deadline-aware GPU join must remain present");
    let tail = &body[join..];
    let drop_js = tail
        .find("drop(js);")
        .expect("GPU init failure must explicitly drop V8");
    let shutdown = tail
        .find("render.shutdown_detached();")
        .expect("GPU init failure must stop render");
    assert!(
        drop_js < shutdown,
        "V8 must be destroyed before render shutdown on startup failure"
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
