//! Host thread/watchdog wiring, pinned against the source.

const THREAD: &str = include_str!("../thread.rs");
const HOST: &str = include_str!("../host.rs");
const MOD: &str = include_str!("../mod.rs");
const HOST_CMD: &str = include_str!("../../../../shared/src/protocol/host_cmd.rs");

#[test]
fn thread_has_no_three_second_heartbeat() {
    assert!(
        !THREAD.contains("heartbeat_sleep"),
        "the 3-second heartbeat sleep must be removed from thread.rs"
    );
    assert!(
        !THREAD.contains("from_secs(3)"),
        "no three-second timer may remain in thread.rs"
    );
}

#[test]
fn thread_selects_audio_and_render_notify_branches() {
    assert!(
        THREAD.contains("check_and_start"),
        "the host loop must select an audio start-signal branch calling check_and_start()"
    );
    assert!(
        THREAD.contains("drain_render_events"),
        "the host loop must select a render branch calling drain_render_events()"
    );
    assert!(
        THREAD.contains("notified()"),
        "audio and render branches must be Notify-driven (no replacement timer)"
    );
}

#[test]
fn drain_render_events_command_and_send_sites_are_gone() {
    let needle = "DrainRenderEvents";
    assert!(
        !HOST_CMD.contains(needle),
        "the DrainRenderEvents command variant must be removed"
    );
    assert!(
        !HOST.contains(needle),
        "all DrainRenderEvents send/handle sites must be removed from host.rs"
    );
    assert!(
        !THREAD.contains(needle),
        "no DrainRenderEvents reference may remain in thread.rs"
    );
}

#[test]
fn old_per_host_watchdog_module_is_removed() {
    assert!(
        !MOD.contains("pub mod watchdog"),
        "the old per-host core watchdog module declaration must be removed"
    );
}

#[test]
fn host_installs_the_process_watchdog_on_new_and_restart() {
    assert!(
        HOST.contains("install_watchdog"),
        "Host::new must install the process deadline watchdog"
    );
    assert!(
        HOST.contains("take_and_drop"),
        "restart must drop the old runtime before creating the new isolate"
    );
    // The old per-host WatchdogHandle field must be gone.
    assert!(
        !HOST.contains("WatchdogHandle"),
        "the old WatchdogHandle field must be removed from Host"
    );
}

#[test]
fn restart_awaits_audio_cleanup_before_dropping_or_creating_an_isolate() {
    let restart_start = HOST
        .find("async fn on_restart")
        .expect("Host::on_restart exists");
    let restart = &HOST[restart_start..];
    let cleanup = restart
        .find("self.audio.release_all_contexts().await")
        .expect("restart must await the host-owned native audio cleanup barrier");
    let old_drop = restart
        .find("self.js.take_and_drop()")
        .expect("restart drops the old isolate");
    let reopen = restart
        .find("self.audio.finish_release_all_contexts()")
        .expect("audio data remains fenced until the retired isolate is gone");
    let new_runtime = restart
        .find("HostJsRuntime::new")
        .expect("restart creates the replacement isolate");
    let runtime_install = restart
        .find("self.js.set(new_js)")
        .expect("restart installs the replacement isolate");
    let module_evaluate = restart
        .find("self.on_evaluate_module")
        .expect("restart may evaluate the replacement module");

    assert!(
        cleanup < old_drop,
        "audio ack must precede old isolate drop"
    );
    assert!(
        old_drop < reopen,
        "old isolate must be gone before reopening audio data"
    );
    assert!(
        new_runtime < runtime_install,
        "replacement is built before installation"
    );
    assert!(
        runtime_install < reopen,
        "replacement must be installed before reopening audio"
    );
    assert!(
        reopen < module_evaluate,
        "audio reopens before replacement module code runs"
    );
}

#[test]
fn restart_fences_and_reclaims_js_audio_backing_at_the_isolate_boundary() {
    let restart_start = HOST
        .find("async fn on_restart")
        .expect("Host::on_restart exists");
    let restart = &HOST[restart_start..];
    let generation = restart
        .find("let retired_generation = self.restart_boundary.current()")
        .expect("restart captures the retiring generation");
    let retire = restart
        .find("self.audio.begin_retire(retired_generation)")
        .expect("restart fences JS-owned audio backing");
    let native_barrier = restart
        .find("self.audio.release_all_contexts().await")
        .expect("restart awaits native WebAudio cleanup");
    let old_drop = restart
        .find("self.js.take_and_drop()")
        .expect("restart destroys the old isolate");
    let backing_drop = restart
        .find("self.audio.finish_runtime_drop(retired_generation)")
        .expect("restart releases backing permits after isolate destruction");
    let new_runtime = restart
        .find("HostJsRuntime::new")
        .expect("restart constructs the replacement isolate");

    assert!(generation < retire);
    assert!(
        retire < native_barrier,
        "resource admission closes before cleanup"
    );
    assert!(
        native_barrier < old_drop,
        "native cleanup is acknowledged first"
    );
    assert!(
        old_drop < backing_drop,
        "V8 owns the backing until isolate drop"
    );
    assert!(
        backing_drop < new_runtime,
        "old accounting is gone before replacement"
    );
}

#[test]
fn host_drop_reclaims_audio_backing_only_after_dropping_the_isolate() {
    let drop_start = HOST
        .find("impl Drop for Host")
        .expect("Host has deterministic teardown");
    let drop_body = &HOST[drop_start..HOST.find("impl Host {").unwrap()];
    let generation = drop_body
        .find("self.restart_boundary.current()")
        .expect("teardown captures the live runtime generation");
    let js_drop = drop_body
        .find("self.js.take_and_drop()")
        .expect("teardown destroys V8");
    let backing_drop = drop_body
        .find("self.audio.finish_runtime_drop(")
        .expect("teardown releases JS audio backing");

    assert!(generation < js_drop);
    assert!(
        js_drop < backing_drop,
        "backing permits outlive the isolate"
    );
}

#[test]
fn oom_classification_wins_before_watchdog_timeout() {
    assert!(
        THREAD.contains("was_oom_terminated"),
        "OOM classification must remain"
    );
    assert!(
        THREAD.contains("watchdog_timed_out"),
        "the new watchdog timeout classification must be present"
    );
    let oom = THREAD
        .find("was_oom_terminated")
        .expect("oom check present");
    let wd = THREAD
        .find("watchdog_timed_out")
        .expect("watchdog check present");
    assert!(
        oom < wd,
        "OOM classification must be ordered before the watchdog timeout classification"
    );
}

#[test]
fn shutdown_and_command_queue_semantics_are_preserved() {
    // The authoritative shutdown flag and command-queue receive must survive
    // the heartbeat removal (no regression of shutdown/full-queue progress).
    //
    // The flag moved into `SurfaceControl` when the surface state machine
    // took ownership of shutdown; what this pins is that the check is still
    // read *outside* the command-queue select, so a full queue cannot
    // swallow the request. The literal is the current authoritative
    // expression, not the historical `shutdown.load`.
    assert!(
        THREAD.contains("surface_control.is_shutting_down()"),
        "the authoritative shutdown flag check must remain"
    );
    assert!(
        THREAD.contains("host_rx.recv()"),
        "the command-queue receive branch must remain"
    );
}
