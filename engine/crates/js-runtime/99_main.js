import { primordials } from "ext:core/mod.js";
import { windowOrWorkerGlobalScope } from "ext:runtime/98_global_scope_shared.js";
import { WindowGlobalScope } from "ext:runtime/98_global_scope_window.js";
import { installWxNamespace } from "ext:runtime/97_wx_namespace.js";
import { initializeEventHandlers } from "ext:host_v8_event/01_event.js";
import { _perf, _perfEnable, _perfDisable } from "ext:host_v8_base/05_perf.js";

const { ObjectDefineProperties } = primordials;

ObjectDefineProperties(globalThis, windowOrWorkerGlobalScope);
ObjectDefineProperties(globalThis, WindowGlobalScope);

globalThis.GameGlobal = globalThis;
globalThis.global = globalThis;

// BOM (window.innerWidth, window.screen, devicePixelRatio, document, navigator,
// location, ...) and DOM are intentionally NOT exposed by the runtime, matching
// wx Android's GameGlobal: it ships only `wx.*` + standard JS, with no BOM/DOM.
// Games that need browser-style globals must layer a weapp-style adapter on top
// (driving them from `migo.getWindowInfo()` / `migo.getSystemInfoSync()` etc.).
//
// Engine compatibility shims (e.g. Cocos Creator's `_CCSettings` orientation
// hook) likewise belong in the adapter, not in the runtime.

initializeEventHandlers();

// Perf profiler: only accessible via evaluateJavaScript from host app.
// Not enumerable, not visible to game code via migo.* or globalThis iteration.
Object.defineProperty(globalThis, '_perf', {
    value: Object.freeze({ enable: _perfEnable, disable: _perfDisable }),
    configurable: false,
    enumerable: false,
    writable: false,
});

// Install `wx` and `migo` namespace objects mirroring the wx-style API surface
// from globalThis. Must run after every feature's 99_global_scope.js has
// registered its APIs (the runtime extension is loaded last in lib.rs, so by
// here all registrations are done) and after the BOM/_perf installs above
// (which use the _NON_WX exclusion list to stay off the wx namespace).
installWxNamespace();
