//! `migo.getUpdateManager()` must not invent an update.
//!
//! This runtime has no update channel. Reporting that -- "no update" -- is a
//! complete and correct answer, and `checkUpdate()` has always given it.
//! `getUpdateManager()`, the class-style API for the same question, did not: a
//! `Math.random() < 0.3` at construction decided whether content was told an
//! update was waiting, and a second flip a random 2-5 s later fired
//! `onUpdateReady` (90%) or `onUpdateFailed` (10%). Roughly a quarter of
//! launches showed the game's own "new version -- restart?" prompt over an
//! update that did not exist; `applyUpdate()` then logged "Application restarted
//! with new version" and restarted nothing.
//!
//! The damage of that shape is not the absent feature, it is that the failure
//! moved: content met it on some launches and not others, in a different place
//! each time, so it survives testing and arrives in the field. There was no test
//! here at all, and no `@stub` marker, so the customer-facing prescreen report
//! counted the API as supported.
//!
//! The assertion is deliberately over the whole surface rather than over the one
//! path that was wrong: call every method the manager publishes, and require
//! that `onUpdateReady` and `onUpdateFailed` never fire. A future method that
//! invents an update fails here without anyone remembering to extend a list.
//!
//! `scripts/test-runtime-answers-are-not-invented.sh` is the source-level
//! companion: it refuses an unjustified `Math.random` anywhere in the embedded
//! JS, which is the mechanism this one asserts the absence of.

#[cfg(test)]
mod update_manager_reports_no_update_tests {
    use std::{
        path::PathBuf,
        sync::{Arc, atomic::AtomicBool},
        time::Duration,
    };

    use deno_core::{FastString, JsRuntime, PollEventLoopOptions, RuntimeOptions};
    use futures::future::poll_fn;
    use shared::{
        channel::ThreadWakeup,
        device::gpu_caps::GpuCaps,
        op_state::{AudioSender, HostOpState, NetworkPolicy},
        render_command_sender::CommandSender,
    };

    fn host_state() -> HostOpState {
        let (render_tx, _render_rx) = CommandSender::new();
        let (host_tx, _critical_host_tx, _host_rx) = shared::host_channel::channel(1);

        HostOpState {
            callback_ids: std::sync::Arc::new(shared::callback_id::CallbackIdAllocator::default()),
            runtime_generation: 1,
            id: 1,
            app_cache_dir: PathBuf::from("/tmp/cache"),
            app_files_dir: PathBuf::from("/tmp/files"),
            code_dir: None,
            game_paths: None,
            vfs: None,
            mount_table: None,
            render_tx,
            text_measurer: None,
            audio_tx: AudioSender::new(shared::audio_channel::disconnected(), ThreadWakeup::new()),
            host_tx,
            device_services: None,
            raf_rx: None,
            raf_demand: Arc::new(shared::raf_signal::RafDemand::new()),
            request_vsync: None,
            sub_packages: Vec::new(),
            workers_path: None,
            network_policy: NetworkPolicy::default(),
            backgrounded: Arc::new(AtomicBool::new(false)),
            timer_backgrounded: Arc::new(AtomicBool::new(false)),
            webgl_context_created: Arc::new(AtomicBool::new(false)),
            context_lost: Arc::new(shared::op_state::ContextLostState::default()),
            code_signing_enabled: false,
            gpu_caps: GpuCaps::new(),
        }
    }

    fn boot() -> JsRuntime {
        let mut rt = JsRuntime::new(RuntimeOptions {
            extensions: crate::main_extensions(host_state()),
            ..Default::default()
        });
        crate::harden_global_scope(&mut rt);
        rt
    }

    async fn poll_once(rt: &mut JsRuntime) {
        poll_fn(|cx| {
            let _ = rt.poll_event_loop(cx, PollEventLoopOptions::default());
            std::task::Poll::Ready(())
        })
        .await;
    }

    async fn drain_ready(rt: &mut JsRuntime) {
        for _ in 0..4 {
            poll_once(rt).await;
            tokio::task::yield_now().await;
        }
    }

    /// Time is paused in these tests, so the manager's one-second launch check
    /// is crossed instantly rather than waited on.
    async fn advance_and_drain(rt: &mut JsRuntime, duration: Duration) {
        poll_once(rt).await;
        tokio::time::advance(duration).await;
        tokio::time::advance(Duration::from_nanos(1)).await;
        tokio::task::yield_now().await;
        drain_ready(rt).await;
    }

    /// Assertions are JS `throw`s: deno_core does not hand out a handle scope
    /// here, so a failed assertion surfaces as `execute_script` returning `Err`.
    fn assert_js(rt: &mut JsRuntime, src: &str) {
        let wrapped = format!(
            "(()=>{{ {src}; if (!__ok) throw new Error('assertion failed: ' + __msg); }})()"
        );
        rt.execute_script("<test:update-manager>", FastString::from(wrapped))
            .expect("update manager assertion script");
    }

    /// The launch-time check reports no update, and nothing the manager
    /// publishes can say otherwise.
    #[tokio::test(start_paused = true)]
    async fn no_published_method_announces_an_update() {
        let mut rt = boot();

        // Listeners first, then cross the deferral the constructor scheduled --
        // this is the path that used to flip a coin.
        rt.execute_script(
            "<test:update-manager:arm>",
            FastString::from_static(
                r#"
                globalThis.__mgr = globalThis.migo.getUpdateManager();
                globalThis.__ready = 0;
                globalThis.__failed = 0;
                globalThis.__checks = [];
                __mgr.onCheckForUpdate((res) => { __checks.push(res); });
                __mgr.onUpdateReady(() => { __ready++; });
                __mgr.onUpdateFailed(() => { __failed++; });
                "#,
            ),
        )
        .expect("arming script");

        advance_and_drain(&mut rt, Duration::from_secs(30)).await;

        assert_js(
            &mut rt,
            r#"
            const mgr = globalThis.__mgr;
            let ready = globalThis.__ready, failed = globalThis.__failed;
            const checks = globalThis.__checks;

            // Every own method, discovered rather than listed, so a new one that
            // fabricates is covered without this test being edited.
            const proto = Object.getPrototypeOf(mgr);
            const called = [];
            for (const key of Object.getOwnPropertyNames(proto)) {
                if (key === 'constructor') continue;
                const fn = proto[key];
                if (typeof fn !== 'function') continue;
                if (fn.length > 0) continue;  // listener registrars take an argument
                try { fn.call(mgr); called.push(key); } catch (_) { /* still must not fire */ }
            }

            let __ok = ready === 0
                    && failed === 0
                    && mgr.hasUpdate === false
                    && mgr.isReady === false
                    && checks.length > 0
                    && checks.every((c) => c && c.hasUpdate === false)
                    && called.length > 0;
            let __msg = JSON.stringify({ ready, failed, called, checks,
                                         hasUpdate: mgr.hasUpdate, isReady: mgr.isReady });
            "#,
        );
    }

    /// The callback-style API and the class-style API answer the same question,
    /// so they must agree. They did not: this is the disagreement that made the
    /// fabricating one visible.
    #[tokio::test(start_paused = true)]
    async fn check_update_and_the_manager_agree() {
        let mut rt = boot();
        advance_and_drain(&mut rt, Duration::from_secs(30)).await;
        assert_js(
            &mut rt,
            r#"
            let seen = null;
            globalThis.migo.checkUpdate({ success: (r) => { seen = r; } });
            const mgr = globalThis.migo.getUpdateManager();
            let __ok = seen !== null
                    && seen.hasUpdate === false
                    && mgr.hasUpdate === seen.hasUpdate;
            let __msg = JSON.stringify({ seen, hasUpdate: mgr.hasUpdate });
            "#,
        );
    }
}
