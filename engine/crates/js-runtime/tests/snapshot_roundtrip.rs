//! Self-contained V8 startup-snapshot create→restore round-trip on the HOST.
//!
//! This is a regression guard for the deno_core snapshot integration: creating a
//! snapshot from `lazy_extensions()` and restoring it must keep `Deno.core`
//! (ops + `eventLoopTick`/`buildCustomError`/`runImmediateCallbacks`) intact in
//! the snapshotted heap, because deno_core's restore path re-runs
//! `store_js_callbacks` unconditionally and reads them back out.
//!
//! The historical bug: `99_main.js` did `delete globalThis.Deno` during
//! bootstrap, which is baked into the snapshot, so restore panicked with
//! `bindings.rs: "unable to convert"` ("Deno is not defined"). The fix moved
//! that hardening to runtime (`HostJsRuntime::new` -> `harden_global_scope`),
//! out of the snapshot. This test reproduces the create→restore round-trip and
//! asserts the builtins survive.
//!
//! Why a dedicated integration-test binary (not a `#[cfg(test)]` unit test):
//! `create_snapshot` puts V8 into "predictable"/snapshot mode, which is a
//! PROCESS-WIDE flag set on first init. Mixing it with the many normal-runtime
//! unit tests in the same binary (run multithreaded) causes a SIGILL. Its own
//! binary runs in its own process, so the mode is consistent.

/// Mirrors `migo-snapshot-gen` (creation) + `HostJsRuntime::new` up to and
/// including `JsRuntime::new` (restore), which is where `store_js_callbacks`
/// runs — the exact point the bug used to panic. `lazy_init_extensions` (which
/// needs per-host state) is intentionally not exercised here; it runs after the
/// failure point and is covered by the on-device flow and the webgl/font tests
/// that build a runtime via `HostJsRuntime::new`.
#[test]
fn snapshot_roundtrip_restores_deno_core() {
    // ---- CREATION SIDE (mirror snapshot-gen/src/main.rs) ----
    let output = deno_core::snapshot::create_snapshot(
        deno_core::snapshot::CreateSnapshotOptions {
            cargo_manifest_dir: env!("CARGO_MANIFEST_DIR"),
            startup_snapshot: None,
            skip_op_registration: false,
            extensions: js_runtime::snapshot::lazy_extensions(),
            extension_transpiler: None,
            // Verify the deno_core builtins are present in the heap right before
            // serialization. If this fires, the snapshot is broken at CREATION
            // (e.g. `Deno` was deleted by a bootstrap module); if it passes but
            // restore fails below, the problem is on the RESTORE side.
            with_runtime_cb: Some(Box::new(|rt| {
                rt.execute_script(
                    "creation-check",
                    "if (typeof Deno.core.ops !== 'object') throw new Error('CREATION: Deno.core.ops is ' + typeof Deno.core.ops);\n\
                     if (typeof Deno.core.eventLoopTick !== 'function') throw new Error('CREATION: Deno.core.eventLoopTick is ' + typeof Deno.core.eventLoopTick);\n\
                     'ok'",
                )
                .expect("deno_core builtins must exist at snapshot creation");
            })),
        },
        None, // no warmup script
    )
    .expect("create snapshot");

    let snapshot_bytes: &'static [u8] = Box::leak(output.output);

    // ---- RESTORE SIDE (mirror HostJsRuntime::new) ----
    // store_js_callbacks runs inside JsRuntime::new; if the snapshot lacks
    // Deno.core.* this construction panics ("unable to convert").
    let mut rt = deno_core::JsRuntime::new(deno_core::RuntimeOptions {
        extensions: js_runtime::snapshot::lazy_extensions(),
        startup_snapshot: Some(snapshot_bytes),
        skip_op_registration: true,
        ..Default::default()
    });

    // The snapshot must still expose `Deno.core` so deno_core could re-capture
    // its callbacks on restore.
    rt.execute_script(
        "restore-check",
        "if (typeof Deno === 'undefined') throw new Error('RESTORE: Deno missing from snapshot');\n\
         if (typeof Deno.core.eventLoopTick !== 'function') throw new Error('RESTORE: eventLoopTick missing');\n\
         globalThis.__migo_ok = 1 + 2;",
    )
    .expect("execute_script after restore");

    // The game-visible-global hardening (delete Deno/__bootstrap) is applied at
    // RUNTIME, not baked into the snapshot. Verify it works post-restore — this
    // is exactly what `js_runtime::harden_global_scope` does.
    rt.execute_script(
        "harden-check",
        "delete globalThis.Deno; delete globalThis.__bootstrap;\n\
         if (typeof globalThis.Deno !== 'undefined') throw new Error('hardening failed');",
    )
    .expect("hardening after restore");
}
