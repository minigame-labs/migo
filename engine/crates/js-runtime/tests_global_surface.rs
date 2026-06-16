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
        let (host_tx, _host_rx) = mpsc::channel(1);

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
            webgl_context_created: Arc::new(AtomicBool::new(false)),
            code_signing_enabled: false,
            gpu_caps: GpuCaps::new(),
        }
    }

    /// Boot a runtime with the full main extension chain so `99_main.js` runs.
    fn boot_runtime() -> JsRuntime {
        JsRuntime::new(RuntimeOptions {
            extensions: crate::main_extensions(test_host_state()),
            ..Default::default()
        })
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
}
