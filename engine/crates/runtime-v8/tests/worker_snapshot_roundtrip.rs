//! Dedicated-process create/restore guard for the Worker startup snapshot.
//!
//! V8 snapshot mode is process-wide, so this must not share the normal unit
//! test binary. The generated heap must contain deno_core restore callbacks and
//! the deferred Worker pump hook, but no pending receive op.

#![cfg(feature = "api-system")]

#[test]
fn worker_snapshot_roundtrip_keeps_an_unstarted_bootstrap() {
    let output = deno_core::snapshot::create_snapshot(
        deno_core::snapshot::CreateSnapshotOptions {
            cargo_manifest_dir: env!("CARGO_MANIFEST_DIR"),
            startup_snapshot: None,
            skip_op_registration: false,
            extensions: runtime_v8::snapshot::worker_lazy_extensions(),
            extension_transpiler: None,
            with_runtime_cb: Some(Box::new(|rt| {
                rt.execute_script(
                    "worker-snapshot-creation-check",
                    "if (typeof Deno.core.eventLoopTick !== 'function') throw new Error('Deno.core callbacks missing');\n\
                     if (typeof globalThis.__migoStartWorkerMessagePump !== 'function') throw new Error('deferred Worker pump hook missing');",
                )
                .expect("Worker bootstrap must be serializable before runtime state exists");
                let mut context =
                    std::task::Context::from_waker(std::task::Waker::noop());
                assert!(
                    matches!(
                        rt.poll_event_loop(
                            &mut context,
                            deno_core::PollEventLoopOptions::default()
                        ),
                        std::task::Poll::Ready(Ok(()))
                    ),
                    "snapshot generation must not capture a pending Worker receive"
                );
            })),
        },
        None,
    )
    .expect("create Worker snapshot");

    let snapshot_bytes: &'static [u8] = Box::leak(output.output);
    let mut rt = deno_core::JsRuntime::new(deno_core::RuntimeOptions {
        extensions: runtime_v8::snapshot::worker_lazy_extensions(),
        startup_snapshot: Some(snapshot_bytes),
        skip_op_registration: true,
        ..Default::default()
    });

    rt.execute_script(
        "worker-snapshot-restore-check",
        "if (typeof Deno.core.eventLoopTick !== 'function') throw new Error('restored Deno.core callbacks missing');\n\
         if (typeof globalThis.__migoStartWorkerMessagePump !== 'function') throw new Error('restored Worker pump hook missing');\n\
         delete globalThis.__migoStartWorkerMessagePump;\n\
         delete globalThis.Deno;\n\
         delete globalThis.__bootstrap;\n\
         if ('__migoStartWorkerMessagePump' in globalThis || 'Deno' in globalThis || '__bootstrap' in globalThis) throw new Error('Worker hardening failed');",
    )
    .expect("restored Worker heap can be hardened before game code");
    let mut context = std::task::Context::from_waker(std::task::Waker::noop());
    assert!(
        matches!(
            rt.poll_event_loop(&mut context, deno_core::PollEventLoopOptions::default()),
            std::task::Poll::Ready(Ok(()))
        ),
        "restoring an unstarted Worker must not materialize a pending receive"
    );
}
