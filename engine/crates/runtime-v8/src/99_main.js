import { primordials } from "ext:core/mod.js";
import { windowOrWorkerGlobalScope } from "ext:runtime/98_global_scope_shared.js";
import { WindowGlobalScope } from "ext:runtime/98_global_scope_window.js";
import { installApiNamespaces } from "ext:runtime/97_migo_namespace.js";
import { initializeEventHandlers } from "ext:host_v8_event/01_event.js";
import { _perf, _perfEnable, _perfDisable } from "ext:host_v8_base/05_perf.js";

const {
    ArrayIsArray,
    JSONParse,
    ObjectDefineProperties,
    ReflectApply,
} = primordials;

ObjectDefineProperties(globalThis, windowOrWorkerGlobalScope);
ObjectDefineProperties(globalThis, WindowGlobalScope);

globalThis.GameGlobal = globalThis;
globalThis.global = globalThis;

// BOM (window.innerWidth, window.screen, devicePixelRatio, document, navigator,
// location, ...) and DOM are intentionally NOT exposed by the runtime, matching
// the common mini-game platform's GameGlobal: it ships only the platform
// namespace + standard JS, with no BOM/DOM. Games that need browser-style
// globals must layer an adapter on top (driving them from
// `migo.getWindowInfo()` / `migo.getSystemInfoSync()` etc.).
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

// Install the engine's `migo` API projection from globalThis. Must run after
// every feature's 99_global_scope.js has registered its APIs (the runtime
// extension is loaded last in lib.rs, so by here all registrations are done)
// and after the _perf install above (which is excluded from the projection).
installApiNamespaces();

// NOTE: `delete globalThis.Deno` / `delete globalThis.__bootstrap` are NOT done
// here. This module is baked into the V8 startup snapshot, and deno_core's
// snapshot RESTORE path re-runs `store_js_callbacks` unconditionally, reading
// `Deno.core.{ops,eventLoopTick,buildCustomError,runImmediateCallbacks}` back
// out of the snapshotted heap. Deleting `Deno` here removes those from the
// snapshot, so restore would panic ("Deno is not defined" / "unable to
// convert"). Following deno's own pattern, this game-visible-global hardening
// runs at RUNTIME after the runtime is constructed (snapshot-restored or not),
// in `HostJsRuntime::new` -> `harden_global_scope()`, so it is never captured
// in the snapshot. Game code still never sees `Deno`/`__bootstrap`.
// (Engine JS uses `import { core } from "ext:core/mod.js"`, never global Deno.)

// Move host-bridge hooks (_internal*) off the game-visible global onto a
// holder. These are the engine's event-pump entry points the HOST calls --
// input/sensor/keyboard dispatch, and result callbacks for
// login/payment/modal/etc. Games must never see them: an audit dump of
// `Object.getOwnPropertyNames(globalThis)` would otherwise reveal payment and
// login callback entry points.
//
// `getOwnPropertyNames` does not enumerate Symbol keys, so relocating the
// hooks behind `Symbol.for('Migo.hostBridge')` keeps the audit surface clean.
// That alone was never a forgery defense: the Symbol is in the *global*
// registry, so content asking for the same one got the holder and every hook
// on it. What closes that is below -- the name exists only long enough for
// js_bindings.rs to resolve it, and is deleted once it holds a handle.
const _hostBridge = { __proto__: null };
const _globalKeys = Object.getOwnPropertyNames(globalThis);
for (let i = 0; i < _globalKeys.length; i++) {
    const key = _globalKeys[i];
    // fast prefix check: charCode 95 === '_'
    if (key.charCodeAt(0) === 95 && key.startsWith("_internal")) {
        const desc = Object.getOwnPropertyDescriptor(globalThis, key);
        if (desc) {
            Object.defineProperty(_hostBridge, key, desc);
            delete globalThis[key];
        }
    }
}
// One entry point the host holds a handle to, so its callbacks need no name
// content could also use.
//
// `js_bindings` resolves this once at start-up and keeps a handle; the handle
// keeps the holder alive, and `_hostBridge` above is a module-scope binding
// nothing looks up by name. That is why deleting the global symbol below costs
// nothing on this side.
//
// Uniform shape on purpose. The call sites it replaces take four forms -- no
// arguments, one JSON string, one parsed object, and plain numbers -- and
// `apply` over a decoded array covers all of them without the caller having to
// say which it is.
Object.defineProperty(_hostBridge, "_internalDispatch", {
    value: function (name, argsJson) {
        const hook = _hostBridge[name];
        if (typeof hook !== "function") return;
        let args;
        try {
            args = JSONParse(argsJson);
        } catch (_) {
            // A malformed payload is dropped rather than guessed at: calling a
            // host hook with the wrong arguments is worse than not calling it.
            return;
        }
        if (!ArrayIsArray(args)) return;
        ReflectApply(hook, _hostBridge, args);
    },
    enumerable: false,
    configurable: false,
    writable: false,
});

// The one thing this property is for: letting `js_bindings` find the holder
// once. It removes it immediately afterwards (`retire_bridge_name`), so by the
// time content runs there is nothing here to retrieve.
Object.defineProperty(globalThis, Symbol.for("Migo.hostBridge"), {
    value: _hostBridge,
    enumerable: false,
    // Configurable because a non-configurable property cannot be deleted, and
    // `delete` on one is a silent no-op -- the holder would stay reachable with
    // nothing reporting a failure. Guarded by
    // scripts/test-host-bridge-channel-contract.sh.
    //
    // Nothing runs between this line and the removal except runtime start-up,
    // so there is no window in which content could redefine it.
    configurable: true,
    writable: false,
});
