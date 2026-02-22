import { primordials } from "ext:core/mod.js";
import { windowOrWorkerGlobalScope } from "ext:worker_runtime/98_global_scope_shared.js";
import { initializeEventHandlers } from "ext:host_v8_event/01_event.js";
import { worker, _startMessagePump } from "ext:host_v8_worker_inner/02_worker_inner.js";

const { ObjectDefineProperties, ObjectDefineProperty } = primordials;

// Apply the shared global scope (console, timers, network, file, encode/decode, etc.)
ObjectDefineProperties(globalThis, windowOrWorkerGlobalScope);

// Set the global `worker` object (read-only)
ObjectDefineProperty(globalThis, "worker", {
    value: worker,
    writable: false,
    enumerable: true,
    configurable: false,
});

globalThis.GameGlobal = globalThis;
globalThis.migo = globalThis;

// Initialize error handlers
initializeEventHandlers();

// Start the message pump (async, keeps event loop alive)
_startMessagePump();
