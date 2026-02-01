import { primordials } from "ext:core/mod.js";
import { windowOrWorkerGlobalScope } from "ext:runtime/98_global_scope_shared.js";
import { WindowGlobalScope } from "ext:runtime/98_global_scope_window.js";
import { initializeEventHandlers } from "ext:host_v8_event/01_event.js";

const { ObjectDefineProperties } = primordials;

ObjectDefineProperties(globalThis, windowOrWorkerGlobalScope);
ObjectDefineProperties(globalThis, WindowGlobalScope);

globalThis.GameGlobal = globalThis;
globalThis.migo = globalThis;

// Initialize event handlers
initializeEventHandlers();