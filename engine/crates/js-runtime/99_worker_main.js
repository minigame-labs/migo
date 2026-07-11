import { primordials } from "ext:core/mod.js";
import { windowOrWorkerGlobalScope } from "ext:worker_runtime/98_global_scope_shared.js";
import "ext:worker_runtime/98_global_scope_worker.js";
import { initializeEventHandlers } from "ext:host_v8_event/01_event.js";
import { worker, _startMessagePump } from "ext:host_v8_worker_inner/02_worker_inner.js";

const { ObjectDefineProperties, ObjectDefineProperty } = primordials;

// Apply the shared global scope (console, timers, encode/decode, etc.)
ObjectDefineProperties(globalThis, windowOrWorkerGlobalScope);

// Image-decode APIs require a render surface + a live GPU upload channel, which
// a Worker does not have (its render channel is a disconnected stub). The shared
// global scope exposes createImage/createImageBitmap for the main thread; in a
// Worker they would decode a bitmap on the worker heap only to have the GPU
// upload silently fail. Replace them with fast, explicit "not supported" errors
// so worker code fails immediately instead of wasting a full decode. (A real
// OffscreenCanvas / worker-render pipeline would replace this stub.)
ObjectDefineProperty(globalThis, "createImage", {
    value: () => {
        throw new Error("createImage is not supported inside a Worker (no render surface)");
    },
    writable: true,
    enumerable: true,
    configurable: true,
});
ObjectDefineProperty(globalThis, "createImageBitmap", {
    // Async factory: reject rather than throw synchronously so callers using
    // `.then(...)` observe a normal rejection.
    value: () =>
        Promise.reject(
            new Error("createImageBitmap is not supported inside a Worker (no render surface)"),
        ),
    writable: true,
    enumerable: true,
    configurable: true,
});

// Set the global `worker` object (read-only)
ObjectDefineProperty(globalThis, "worker", {
    value: worker,
    writable: false,
    enumerable: true,
    configurable: false,
});

// Initialize error handlers
initializeEventHandlers();

// Start the message pump (async, keeps event loop alive)
_startMessagePump();

// Remove deno_core's internal namespaces from the worker's game-visible global
// (same rationale as 99_main.js). Workers have no _internal* host bridge, so
// only Deno/__bootstrap need removal here.
delete globalThis.Deno;
delete globalThis.__bootstrap;
