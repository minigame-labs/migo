//! R4 core wiring/contract guards.
//!
//! Pure `include_str!` string checks with no dependency on the rest of the
//! `core` crate, so they compile+run standalone via
//! `rustc --edition 2024 --test crates/core/runtime/tests_r4_contract.rs`
//! — bypassing the host EGL/Skia link that blocks the full `core` test binary —
//! while still being compiled by `cargo check -p core --tests`.

#[cfg(test)]
mod r4_contract {
    const THREAD: &str = include_str!("thread.rs");
    const HOST: &str = include_str!("host.rs");
    const MOD: &str = include_str!("mod.rs");
    const HOST_CMD: &str = include_str!("../../shared/protocol/host_cmd.rs");

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
        assert!(
            THREAD.contains("shutdown.load"),
            "the authoritative shutdown flag check must remain"
        );
        assert!(
            THREAD.contains("host_rx.recv()"),
            "the command-queue receive branch must remain"
        );
    }
}
