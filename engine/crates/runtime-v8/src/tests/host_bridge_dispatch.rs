//! Host callbacks delivered through a handle instead of a name.
//!
//! Callbacks travel as eval'd JavaScript that names
//! `globalThis[Symbol.for('Migo.hostBridge')]`. `Symbol.for` reads the *global*
//! symbol registry, so content asks for the same symbol and reaches every hook
//! on the holder -- 78 of them, measured against a real runtime.
//!
//! `_internalDispatch` is the replacement: the runtime resolves it once at
//! start-up and keeps a handle, and a handle needs no name. Once every call
//! site has moved, the symbol can be removed from globalThis and the host still
//! reaches everything, because the retained handle keeps the holder alive
//! through the dispatcher's closure.
//!
//! These tests pin the dispatcher against the four call shapes it has to
//! replace. They were established by reading every existing call site, not by
//! guessing at what a dispatcher might need: a shape it cannot carry is a
//! callback that would go silent after the migration, on the channel that
//! delivers every async result -- login, payment, location, camera, keyboard.

#[cfg(test)]
mod host_bridge_dispatch_tests {
    use std::{
        path::PathBuf,
        sync::{Arc, atomic::AtomicBool},
    };

    use deno_core::{FastString, JsRuntime, RuntimeOptions};
    use shared::{
        channel::ThreadWakeup,
        device::gpu_caps::GpuCaps,
        op_state::{AudioSender, HostOpState, NetworkPolicy},
        render_command_sender::CommandSender,
    };
    use tokio::sync::mpsc;

    fn boot() -> JsRuntime {
        let (render_tx, _render_rx) = CommandSender::new();
        let (audio_raw_tx, _audio_rx) = mpsc::unbounded_channel();
        let (host_tx, _critical_host_tx, _host_rx) = shared::host_channel::channel(1);
        let host = HostOpState {
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
        };
        let mut rt = JsRuntime::new(RuntimeOptions {
            extensions: crate::main_extensions(host),
            ..Default::default()
        });
        crate::harden_global_scope(&mut rt);
        rt
    }

    fn exec(rt: &mut JsRuntime, src: &str) {
        rt.execute_script("<test:dispatch>", FastString::from(src.to_string()))
            .expect("script");
    }

    fn assert_js(rt: &mut JsRuntime, src: &str) {
        let wrapped = format!(
            "(()=>{{ {src}; if (!__ok) throw new Error('assertion failed: ' + __msg); }})()"
        );
        rt.execute_script("<test:assert>", FastString::from(wrapped))
            .expect("assertion");
    }

    /// Install a probe hook on the holder and dispatch to it by name.
    fn dispatch(rt: &mut JsRuntime, hook: &str, args_json: &str) {
        exec(
            rt,
            &format!(
                "globalThis[Symbol.for('Migo.hostBridge')]._internalDispatch('{hook}', '{args}')",
                args = args_json.replace('\\', "\\\\").replace('\'', "\\'")
            ),
        );
    }

    const PROBE: &str = "\
        globalThis.__calls = []; \
        Object.defineProperty(globalThis[Symbol.for('Migo.hostBridge')], '_internalProbe', { \
            value: function () { globalThis.__calls.push(Array.prototype.slice.call(arguments)); }, \
            configurable: true, \
        });";

    /// Shape 1: no arguments -- `_internalTriggerOnHide()` and friends.
    #[test]
    fn dispatches_a_hook_with_no_arguments() {
        let mut rt = boot();
        exec(&mut rt, PROBE);
        dispatch(&mut rt, "_internalProbe", "[]");
        assert_js(
            &mut rt,
            "let __ok = globalThis.__calls.length === 1 && globalThis.__calls[0].length === 0; \
             let __msg = JSON.stringify(globalThis.__calls)",
        );
    }

    /// Shape 2: one JSON string -- every `_internalOn*Result` callback.
    ///
    /// The hook receives the string, exactly as it does today, and parses it
    /// itself. Decoding here would change what every one of those hooks is
    /// handed.
    #[test]
    fn dispatches_a_hook_with_one_json_string() {
        let mut rt = boot();
        exec(&mut rt, PROBE);
        dispatch(&mut rt, "_internalProbe", r#"["{\"code\":\"abc\"}"]"#);
        assert_js(
            &mut rt,
            "const a = globalThis.__calls[0]; \
             let __ok = a.length === 1 && typeof a[0] === 'string' \
                 && JSON.parse(a[0]).code === 'abc'; \
             let __msg = JSON.stringify(globalThis.__calls)",
        );
    }

    /// Shape 3: one parsed object -- `_internalTriggerOnShow(JSON.parse(...))`.
    #[test]
    fn dispatches_a_hook_with_a_parsed_object() {
        let mut rt = boot();
        exec(&mut rt, PROBE);
        dispatch(&mut rt, "_internalProbe", r#"[{"scene":1001,"query":{}}]"#);
        assert_js(
            &mut rt,
            "const a = globalThis.__calls[0]; \
             let __ok = a.length === 1 && typeof a[0] === 'object' && a[0].scene === 1001; \
             let __msg = JSON.stringify(globalThis.__calls)",
        );
    }

    /// Shape 4: plain numbers -- `_internalOnModalResult(confirm, cancel)`.
    #[test]
    fn dispatches_a_hook_with_numeric_arguments() {
        let mut rt = boot();
        exec(&mut rt, PROBE);
        dispatch(&mut rt, "_internalProbe", "[1,0]");
        assert_js(
            &mut rt,
            "const a = globalThis.__calls[0]; \
             let __ok = a.length === 2 && a[0] === 1 && a[1] === 0; \
             let __msg = JSON.stringify(globalThis.__calls)",
        );
    }

    /// A payload that does not decode is dropped, not guessed at.
    ///
    /// Calling a host hook with the wrong arguments is worse than not calling
    /// it: `_internalOnMidasPaymentResult` with a mangled argument settles a
    /// payment promise with nonsense.
    #[test]
    fn a_malformed_payload_calls_nothing() {
        let mut rt = boot();
        exec(&mut rt, PROBE);
        for payload in ["", "not json", "{\"not\":\"an array\"}", "42"] {
            dispatch(&mut rt, "_internalProbe", payload);
        }
        assert_js(
            &mut rt,
            "let __ok = globalThis.__calls.length === 0; \
             let __msg = 'dispatched ' + globalThis.__calls.length + ' malformed payload(s)'",
        );
    }

    /// An unknown hook name is a no-op rather than a throw: the host names
    /// hooks from Rust, and a typo there should not take down the game.
    #[test]
    fn an_unknown_hook_is_ignored() {
        let mut rt = boot();
        exec(&mut rt, PROBE);
        dispatch(&mut rt, "_internalNoSuchHook", "[1]");
        assert_js(
            &mut rt,
            "let __ok = globalThis.__calls.length === 0; \
             let __msg = 'unknown hook reached something'",
        );
    }

    /// The dispatcher reaches the same hooks the eval channel does, so the
    /// migration is a change of route rather than of behaviour.
    #[test]
    fn the_dispatcher_and_the_eval_channel_reach_the_same_hook() {
        let mut rt = boot();
        exec(&mut rt, PROBE);

        // The shape the host builds today.
        exec(
            &mut rt,
            "globalThis[Symbol.for('Migo.hostBridge')]._internalProbe('viaEval');",
        );
        dispatch(&mut rt, "_internalProbe", r#"["viaDispatch"]"#);

        assert_js(
            &mut rt,
            "const c = globalThis.__calls; \
             let __ok = c.length === 2 && c[0][0] === 'viaEval' && c[1][0] === 'viaDispatch'; \
             let __msg = JSON.stringify(c)",
        );
    }

    /// The holder must be removable once every call site has moved.
    ///
    /// It was installed non-configurable, which would have left it permanently
    /// reachable however much else changed -- the migration's last step would
    /// have had nothing to delete.
    #[test]
    fn the_holder_can_be_deleted_once_nothing_needs_its_name() {
        let mut rt = boot();
        assert_js(
            &mut rt,
            "const d = Object.getOwnPropertyDescriptor(globalThis, Symbol.for('Migo.hostBridge')); \
             let __ok = d !== undefined && d.configurable === true; \
             let __msg = 'holder descriptor: ' + JSON.stringify(d)",
        );
    }
}
