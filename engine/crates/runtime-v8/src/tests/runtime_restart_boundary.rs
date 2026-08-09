//! Host callback ids as seen from JavaScript, across a runtime replacement.
//!
//! Two properties, both about a result arriving for a request that is not the
//! one it names:
//!
//! * ids come from the Host's allocator, so a replacement isolate never reissues
//!   an id the retired one already handed to the platform;
//! * a result whose `requestId` is present but is not an id settles nothing,
//!   rather than falling through to whichever request happens to be oldest.
//!
//! The runtimes are built one after another over **one** allocator, which is
//! what a restart looks like from this layer: the isolate is new, the id space
//! is not.

#[cfg(test)]
mod runtime_restart_boundary_tests {
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    use deno_core::{FastString, JsRuntime, RuntimeOptions};
    use shared::callback_id::CallbackIdAllocator;
    use shared::channel::ThreadWakeup;
    use shared::device::gpu_caps::GpuCaps;
    use shared::op_state::{AudioSender, HostOpState, NetworkPolicy};
    use shared::render_command_sender::CommandSender;
    use shared::services::{
        AdService, CommerceServices, ConnectivityServices, DeviceServices, MediaServices,
        SensorServices, SystemUtilServices,
    };

    /// A host that only claims to do advertising, refusing every command.
    ///
    /// Present so ads take the *hosted* construction path, which is not a
    /// convenience: an unhosted ad schedules its fallback `load` through
    /// `setTimeout`, this harness runs without a Tokio reactor, and the panic
    /// that follows cannot unwind — it aborts the process instead of failing a
    /// test. Every command failing is fine here; the id is allocated before any
    /// of them, which is the property under test.
    struct HostedAdService;
    impl AdService for HostedAdService {}

    struct AdOnlyServices;
    impl SensorServices for AdOnlyServices {}
    impl MediaServices for AdOnlyServices {}
    impl ConnectivityServices for AdOnlyServices {}
    impl SystemUtilServices for AdOnlyServices {}
    impl CommerceServices for AdOnlyServices {
        fn ad(&self) -> Option<Arc<dyn AdService>> {
            Some(Arc::new(HostedAdService))
        }
    }

    // Exposes the real factory rather than a copy of it: these tests are about
    // `createDeferredApi`'s own correlation, so a reimplementation here would
    // assert nothing about the code that ships.
    deno_core::extension!(
        deferred_test_bridge,
        deps = [host_v8_base],
        esm_entry_point = "ext:deferred_test_bridge/bridge.js",
        esm = ["ext:deferred_test_bridge/bridge.js" = {
            source = r#"
                import { allocateHostCallbackId, createDeferredApi }
                    from "ext:host_v8_base/02_async.js";
                globalThis.__createDeferredApi = createDeferredApi;
                globalThis.__allocId = allocateHostCallbackId;
            "#
        },],
    );

    fn test_host_state(callback_ids: Arc<CallbackIdAllocator>) -> HostOpState {
        let (render_tx, _render_rx) = CommandSender::new();
        let (host_tx, _critical_host_tx, _host_rx) = shared::host_channel::channel(1);

        HostOpState {
            callback_ids,
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
            device_services: Some(Arc::new(AdOnlyServices)),
            raf_rx: None,
            raf_demand: Arc::new(shared::raf_signal::RafDemand::new()),
            request_vsync: None,
            // One configured subpackage, so `loadSubpackage` resolves and reaches
            // the allocation instead of failing validation first -- an unreached
            // allocation site looks exactly like one that does not allocate.
            sub_packages: vec![("sub".to_owned(), "sub/".to_owned())],
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

    fn boot(callback_ids: Arc<CallbackIdAllocator>) -> JsRuntime {
        let mut extensions = crate::main_extensions(test_host_state(callback_ids));
        extensions.push(deferred_test_bridge::init());
        let mut rt = JsRuntime::new(RuntimeOptions {
            extensions,
            ..Default::default()
        });
        crate::harden_global_scope(&mut rt);
        // One probe API per runtime, timeouts disabled so nothing settles on a
        // clock instead of on a result.
        exec(
            &mut rt,
            r#"
            globalThis.__ids = [];
            globalThis.__settled = [];
            globalThis.__api = globalThis.__createDeferredApi('probe', 0);
            globalThis.__call = function (tag) {
                globalThis.__api.invoke(
                    { success: function () { globalThis.__settled.push(tag); } },
                    function (_opts, id) { globalThis.__ids.push(id); },
                ).catch(function () {});
            };
            "#,
        );
        rt
    }

    fn exec(rt: &mut JsRuntime, source: impl Into<String>) {
        rt.execute_script("<test:deferred>", FastString::from(source.into()))
            .expect("deferred probe script");
    }

    // The expression is never interpolated into a JavaScript string literal:
    // several of them contain quotes, and one that closed the literal produced a
    // SyntaxError that read like a failure of the code under test.
    fn assert_js(rt: &mut JsRuntime, expression: &str) {
        let script = format!(
            "if (!({expression})) throw new Error('ids=' + JSON.stringify(globalThis.__ids) \
             + ' settled=' + JSON.stringify(globalThis.__settled));"
        );
        if let Err(error) = rt.execute_script("<test:deferred>", FastString::from(script)) {
            panic!("assertion failed: {expression}\n{error}");
        }
    }

    #[test]
    fn a_replacement_runtime_never_reissues_a_retired_runtimes_id() {
        let callback_ids = Arc::new(CallbackIdAllocator::default());

        {
            let mut retired = boot(Arc::clone(&callback_ids));
            exec(&mut retired, "__call('a'); __call('b');");
            // Exact, not "increasing": if anything else in a booted runtime
            // allocated an id, this is where it would show.
            assert_js(&mut retired, "__ids.join(',') === '1,2'");
        }

        // Read the space from where the platform stands. Three proves the
        // retired runtime consumed exactly two and that dropping it released
        // nothing back.
        assert_eq!(callback_ids.allocate(), Ok(3));

        let mut replacement = boot(Arc::clone(&callback_ids));
        exec(&mut replacement, "__call('c');");
        assert_js(&mut replacement, "__ids.join(',') === '4'");

        // The retired runtime's first id is meaningless here. With a per-runtime
        // counter both would be 1 and this would settle 'c'.
        exec(
            &mut replacement,
            "__api.settle(JSON.stringify({ requestId: 1 }));",
        );
        assert_js(&mut replacement, "__settled.length === 0");

        exec(
            &mut replacement,
            "__api.settle(JSON.stringify({ requestId: 4 }));",
        );
        assert_js(&mut replacement, "__settled.join(',') === 'c'");
    }

    #[test]
    fn a_requestid_that_is_not_an_id_settles_nothing() {
        let mut rt = boot(Arc::new(CallbackIdAllocator::default()));
        exec(&mut rt, "__call('only');");

        // Every one of these is *present* and not an id. Before this, `Number()`
        // turned the non-numeric ones into `NaN`, which reached the fallback and
        // settled the oldest pending request -- here, the wrong one and the only
        // one.
        for value in [
            "null",
            "0",
            "-1",
            "1.5",
            "NaN",
            "Infinity",
            "-Infinity",
            "2147483648",
            "\"abc\"",
            "\"\"",
            "true",
            "{}",
            "[]",
        ] {
            exec(
                &mut rt,
                &format!("__api.settle(JSON.stringify({{ requestId: {value} }}));"),
            );
            assert_js(&mut rt, "__settled.length === 0");
        }

        // The real id still works afterwards: rejecting the impostors must not
        // have consumed or corrupted the pending entry.
        exec(
            &mut rt,
            "__api.settle(JSON.stringify({ requestId: __ids[0] }));",
        );
        assert_js(&mut rt, "__settled.join(',') === 'only'");
    }

    #[test]
    fn an_id_serialised_as_a_string_still_correlates() {
        // Tolerated on purpose, and pinned so it is a decision rather than an
        // accident: a platform that echoes the id as text is still echoing the
        // exact id. `true` and `[]` are not, which is why the parser checks the
        // type before it converts -- `Number(true)` is 1, and 1 is an id.
        let mut rt = boot(Arc::new(CallbackIdAllocator::default()));
        exec(&mut rt, "__call('only');");

        exec(
            &mut rt,
            "__api.settle(JSON.stringify({ requestId: String(__ids[0]) }));",
        );

        assert_js(&mut rt, "__settled.join(',') === 'only'");
    }

    #[test]
    fn an_id_settles_exactly_its_own_request_and_only_once() {
        let mut rt = boot(Arc::new(CallbackIdAllocator::default()));
        exec(&mut rt, "__call('first'); __call('second');");

        exec(
            &mut rt,
            "__api.settle(JSON.stringify({ requestId: __ids[1] }));",
        );
        assert_js(&mut rt, "__settled.join(',') === 'second'");

        // A duplicate of a settled id must not reach the still-pending request.
        exec(
            &mut rt,
            "__api.settle(JSON.stringify({ requestId: __ids[1] }));",
        );
        assert_js(&mut rt, "__settled.join(',') === 'second'");

        exec(
            &mut rt,
            "__api.settle(JSON.stringify({ requestId: __ids[0] }));",
        );
        assert_js(&mut rt, "__settled.join(',') === 'second,first'");
    }

    #[test]
    fn modules_with_their_own_pending_maps_draw_from_the_same_space() {
        // login, payment and subpackage each kept a module-local counter that
        // restarted at 1, so two of them handed the platform the same id for
        // different requests and a restart reissued ids it had already used.
        //
        // Bracketed with two allocations instead of counted from Rust: the
        // difference is exact regardless of how many APIs the profile ships, and
        // Slim drops commerce, so a fixed total would assert the product
        // configuration rather than the property. Absent APIs are skipped
        // out loud rather than silently making the sum work out.
        let mut rt = boot(Arc::new(CallbackIdAllocator::default()));
        exec(
            &mut rt,
            r#"
            globalThis.__checked = [];
            globalThis.__assertTakesOneId = function (name, fn) {
                if (typeof fn !== 'function') return;
                const before = globalThis.__allocId();
                try {
                    const out = fn();
                    if (out && typeof out.catch === 'function') out.catch(function () {});
                } catch (e) {
                    // The platform boundary throws in this harness -- no host is
                    // listening. Allocation happens first, which is the property.
                }
                const after = globalThis.__allocId();
                if (after - before !== 2) {
                    throw new Error(
                        name + ' took ' + (after - before - 1) + ' ids from the Host space, want 1'
                    );
                }
                globalThis.__checked.push(name);
            };

            __assertTakesOneId('wx.login', wx.login && function () { return wx.login({}); });
            __assertTakesOneId(
                'wx.requestMidasPayment',
                wx.requestMidasPayment && function () { return wx.requestMidasPayment({ offerId: 'x' }); },
            );
            __assertTakesOneId('wx.loadSubpackage', function () { return wx.loadSubpackage({ name: 'sub' }); });
            // Not a deferred call: the id names a player the platform keeps in
            // a map that outlives this isolate, and it is what every later
            // `onVideoEvent` routes by. A module counter restarting at 1 aims
            // the retired runtime's events at the replacement's objects.
            __assertTakesOneId('wx.createVideo', wx.createVideo && function () { return wx.createVideo({}); });
            // The same property for the callback-routed resources: each id names
            // something the host owns in a table that outlives this isolate, so
            // a module counter restarting at 1 points the retired runtime's
            // events -- camera frames, audio playback, an ad's reward verdict --
            // at the replacement runtime's objects.
            __assertTakesOneId('wx.createCamera', wx.createCamera && function () { return wx.createCamera({}); });
            __assertTakesOneId(
                'wx.createInnerAudioContext',
                wx.createInnerAudioContext && function () { return wx.createInnerAudioContext(); },
            );
            __assertTakesOneId(
                'wx.createRewardedVideoAd',
                wx.createRewardedVideoAd && function () { return wx.createRewardedVideoAd({ adUnitId: 'x' }); },
            );
            "#,
        );

        // At least one had to be present, or the test proved nothing at all.
        assert_js(&mut rt, "__checked.length > 0");
    }

    #[test]
    fn two_deferred_apis_in_one_runtime_do_not_share_a_numbering() {
        // Each `createDeferredApi` used to count from 1 independently, so two of
        // them handed the platform the same id for different requests. They now
        // draw from the Host's one space.
        let mut rt = boot(Arc::new(CallbackIdAllocator::default()));
        exec(
            &mut rt,
            r#"
            globalThis.__other = globalThis.__createDeferredApi('other', 0);
            globalThis.__otherIds = [];
            globalThis.__other.invoke({}, function (_o, id) { __otherIds.push(id); })
                .catch(function () {});
            __call('mine');
            "#,
        );

        assert_js(&mut rt, "__otherIds[0] !== __ids[0]");
    }
}
