//! Integration tests for the game-visible global surface (P0 audit hardening).
//!
//! These tests boot a full runtime via `main_extensions()` so the runtime
//! ESM entry point (`99_main.js`) executes and registers the real global
//! scope. They pin the contract that a security audit dumping
//! `Object.getOwnPropertyNames(globalThis)` sees neither deno_core internals
//! (`Deno`, `__bootstrap`) nor the host-bridge hooks (`_internal*`), while the
//! host's two delivery channels can still reach those hooks via the
//! `Symbol.for('Migo.hostBridge')` holder.

#[cfg(test)]
mod global_surface_tests {
    use std::{path::PathBuf, sync::Arc, sync::atomic::AtomicBool};

    use deno_core::{FastString, JsRuntime, RuntimeOptions};
    use shared::{
        channel::ThreadWakeup,
        device::gpu_caps::GpuCaps,
        op_state::{AudioSender, HostOpState, NetworkPolicy},
        render_command_sender::CommandSender,
    };
    use tokio::sync::mpsc;

    fn test_host_state() -> HostOpState {
        let (render_tx, _render_rx) = CommandSender::new();
        let (audio_raw_tx, _audio_rx) = mpsc::unbounded_channel();
        let (host_tx, _critical_host_tx, _host_rx) = shared::host_channel::channel(1);

        HostOpState {
            id: 1,
            app_cache_dir: PathBuf::from("/tmp/cache"),
            app_files_dir: PathBuf::from("/tmp/files"),
            code_dir: None,
            game_paths: None,
            vfs: None,
            mount_table: None,
            render_tx,
            text_measurer: None,
            audio_tx: AudioSender::new(audio_raw_tx, ThreadWakeup::new()),
            host_tx,
            device_services: None,
            raf_rx: None,
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

    /// Boot a runtime with the full main extension chain so `99_main.js` runs.
    ///
    /// Mirrors `HostJsRuntime::new`: the `Deno`/`__bootstrap` hardening is now
    /// applied at runtime via `crate::harden_global_scope` (not in 99_main.js, so
    /// it stays out of the V8 startup snapshot), so the test helper must apply it
    /// too to represent the real game-visible global surface.
    fn boot_runtime() -> JsRuntime {
        let mut rt = JsRuntime::new(RuntimeOptions {
            extensions: crate::main_extensions(test_host_state()),
            ..Default::default()
        });
        crate::harden_global_scope(&mut rt);
        rt
    }

    /// Run an assertion script that should `throw` on failure, surfacing the
    /// message as a test failure (same pattern as tests_prelude).
    fn assert_js(rt: &mut JsRuntime, src: &str) {
        let wrapped = format!(
            "(()=>{{ {src}; if (!__ok) throw new Error('assertion failed: ' + __msg); }})()"
        );
        rt.execute_script("<test:assert>", FastString::from(wrapped))
            .expect("assertion script");
    }

    /// A security audit dumps own property names of the global. After P0
    /// cleanup, none of the deno_core internals or host-bridge hooks may be
    /// reachable by string key.
    #[test]
    fn audit_dump_hides_internals() {
        let mut rt = boot_runtime();
        assert_js(
            &mut rt,
            "const keys = Object.getOwnPropertyNames(globalThis); \
             const bad = keys.filter(k => k === 'Deno' || k === '__bootstrap' \
                 || k.startsWith('_internal')); \
             let __ok = bad.length === 0; \
             let __msg = 'leaked global keys: ' + JSON.stringify(bad)",
        );
    }

    /// The host bridge holder must still expose the hooks under its Symbol key
    /// so both delivery channels (js_bindings lookup + eval) can reach them.
    #[test]
    fn host_bridge_holder_reachable() {
        let mut rt = boot_runtime();
        assert_js(
            &mut rt,
            "const b = globalThis[Symbol.for('Migo.hostBridge')]; \
             let __ok = !!b \
                 && typeof b._internalOnLoginResult === 'function' \
                 && typeof b._internalOnMidasPaymentResult === 'function' \
                 && typeof b._internalEnqueueRawTouchEvent === 'function'; \
             let __msg = 'holder=' + (b ? Object.getOwnPropertyNames(b).length + ' keys' : 'missing')",
        );
    }

    /// The eval-channel script shape the host builds must resolve to the holder
    /// hook and deliver its payload. We install a probe on the holder, run a
    /// script identical to what `build_eval_script` produces, and verify it ran.
    #[test]
    fn eval_channel_reaches_holder() {
        let mut rt = boot_runtime();
        assert_js(
            &mut rt,
            "globalThis[Symbol.for('Migo.hostBridge')].__probe = (s) => { globalThis.__got = s; }; \
             let __ok = true; let __msg = 'setup'",
        );
        // Mirror build_eval_script's output shape (holder-qualified call).
        rt.execute_script(
            "eval-script",
            FastString::from_static(
                "globalThis[Symbol.for('Migo.hostBridge')].__probe('{\"code\":\"ok\"}');",
            ),
        )
        .expect("eval-channel script runs");
        assert_js(
            &mut rt,
            "let __ok = globalThis.__got === '{\"code\":\"ok\"}'; \
             let __msg = 'got=' + globalThis.__got",
        );
    }

    /// GC APIs (03_gc.js) must work after switching off global `Deno.core`.
    #[test]
    fn gc_apis_work_without_global_deno() {
        let mut rt = boot_runtime();
        assert_js(
            &mut rt,
            "let __ok = typeof globalThis.triggerGC === 'function' \
                 && typeof globalThis.getHeapStatistics === 'function' \
                 && typeof globalThis.Deno === 'undefined'; \
             let __msg = 'triggerGC=' + typeof globalThis.triggerGC",
        );
    }

    /// A `touchend` must report only the pointers still on the surface in
    /// `touches`; the lifted finger belongs in `changedTouches` only. Native
    /// marks the lifted pointer with FLAG_REMOVED (bit 1) alongside FLAG_CHANGED
    /// (bit 0). Regression guard for games that detect "all fingers up" via
    /// `event.touches.length === 0`.
    #[tokio::test]
    async fn touchend_excludes_lifted_pointer_from_touches() {
        use deno_core::PollEventLoopOptions;
        use std::time::Duration;

        let mut rt = boot_runtime();

        rt.execute_script(
            "<test:setup>",
            FastString::from_static(
                "globalThis.__ev = null; \
                 globalThis.onTouchEnd((e) => { globalThis.__ev = e; });",
            ),
        )
        .expect("register touchend listener");

        // Two pointers in the raw buffer (stride 20): id 0 stays down (flags 0);
        // id 1 is the lifting finger (FLAG_CHANGED|FLAG_REMOVED = 3).
        rt.execute_script(
            "<test:enqueue>",
            FastString::from_static(
                "const b = globalThis[Symbol.for('Migo.hostBridge')]; \
                 const S = 20; const buf = new ArrayBuffer(2 * S); const dv = new DataView(buf); \
                 dv.setUint32(0, 0, true); dv.setFloat32(4, 10, true); dv.setFloat32(8, 20, true); dv.setFloat32(12, 1, true); dv.setUint32(16, 0, true); \
                 dv.setUint32(S, 1, true); dv.setFloat32(S + 4, 30, true); dv.setFloat32(S + 8, 40, true); dv.setFloat32(S + 12, 1, true); dv.setUint32(S + 16, 3, true); \
                 b._internalEnqueueRawTouchEvent(2, buf, 2, 123);",
            ),
        )
        .expect("enqueue raw touch");

        // Drain the microtask that runs `_drain`. No timers are pending, so one
        // brief poll delivers the event; the timeout guards against the loop
        // staying alive on open channels held by the op state.
        let _ = tokio::time::timeout(
            Duration::from_secs(5),
            rt.run_event_loop(PollEventLoopOptions::default()),
        )
        .await;

        assert_js(
            &mut rt,
            "const e = globalThis.__ev; \
             let __ok = !!e \
                 && e.touches.length === 1 && e.touches[0].identifier === 0 \
                 && e.changedTouches.length === 1 && e.changedTouches[0].identifier === 1; \
             let __msg = e ? ('touches=' + e.touches.length + ' changed=' + e.changedTouches.length) : 'no event delivered'",
        );
    }

    /// Touch dispatch must snapshot its listener set: a listener that registers
    /// another listener mid-dispatch must not cause the new one to run for the
    /// same event (standard event-dispatch semantics, avoids re-entrancy surprises).
    #[tokio::test]
    async fn touch_listener_set_is_snapshotted_during_dispatch() {
        use deno_core::PollEventLoopOptions;
        use std::time::Duration;

        let mut rt = boot_runtime();

        rt.execute_script(
            "<test:setup>",
            FastString::from_static(
                "globalThis.__calls = []; \
                 globalThis.onTouchEnd(function A() { \
                     globalThis.__calls.push('A'); \
                     globalThis.onTouchEnd(function B() { globalThis.__calls.push('B'); }); \
                 });",
            ),
        )
        .expect("register listener A");

        rt.execute_script(
            "<test:enqueue>",
            FastString::from_static(
                "const b = globalThis[Symbol.for('Migo.hostBridge')]; \
                 const S = 20; const buf = new ArrayBuffer(S); const dv = new DataView(buf); \
                 dv.setUint32(0, 7, true); dv.setFloat32(4, 1, true); dv.setFloat32(8, 2, true); dv.setFloat32(12, 1, true); dv.setUint32(16, 3, true); \
                 b._internalEnqueueRawTouchEvent(2, buf, 1, 1);",
            ),
        )
        .expect("enqueue raw touch");

        let _ = tokio::time::timeout(
            Duration::from_secs(5),
            rt.run_event_loop(PollEventLoopOptions::default()),
        )
        .await;

        assert_js(
            &mut rt,
            "let __ok = globalThis.__calls.length === 1 && globalThis.__calls[0] === 'A'; \
             let __msg = 'calls=' + JSON.stringify(globalThis.__calls)",
        );
    }
}
