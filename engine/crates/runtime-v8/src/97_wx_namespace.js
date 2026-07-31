// wx namespace mirror.
//
// wx mini-game code is hard-coded against the global `wx` object
// (`wx.createCanvas`, `wx.getSystemInfoSync`, `wx.onTouchStart`, ...). On real
// wx Android, `wx` is a distinct object that holds ~463 APIs (verified
// against wx-android.json), separate from GameGlobal.
//
// migo currently registers its low-level APIs directly on globalThis. Keep
// those registrations for backward compatibility, then build two deliberate
// projections: `migo` is the engine capability namespace consumed by adapters,
// while `wx` contains only APIs that belong to the wx compatibility surface.
//
// Rules:
//   - Shared APIs hold the same function references on both namespaces.
//   - Engine/Web transport primitives that wx does not define stay on `migo`.
//   - GameGlobal, global remain aliases of globalThis (matches wx behavior).
//   - JS built-ins (Object/Array/Promise/...), self-references
//     (globalThis/wx/migo/GameGlobal/global), and runtime internals
//     (_perf, console) are NOT mirrored onto `wx`.

import { primordials } from "ext:core/mod.js";
const { ObjectDefineProperty } = primordials;

// Anything in this set is intentionally NOT mirrored onto the `wx` object.
// JS built-ins -- present on every globalThis. Sourced from V8's standard
// realm; we list them statically because they don't change between sessions.
const _JS_BUILTINS = new Set([
    // Value properties
    "globalThis", "Infinity", "NaN", "undefined",
    // Function properties
    "eval", "isFinite", "isNaN", "parseFloat", "parseInt",
    "decodeURI", "decodeURIComponent", "encodeURI", "encodeURIComponent",
    "escape", "unescape",
    // Fundamental objects
    "Object", "Function", "Boolean", "Symbol",
    // Error objects
    "Error", "AggregateError", "EvalError", "RangeError", "ReferenceError",
    "SyntaxError", "TypeError", "URIError", "SuppressedError",
    // Numbers, dates, text
    "Number", "BigInt", "Math", "Date", "String", "RegExp",
    // Indexed collections
    "Array", "Int8Array", "Uint8Array", "Uint8ClampedArray",
    "Int16Array", "Uint16Array", "Int32Array", "Uint32Array",
    "BigInt64Array", "BigUint64Array", "Float32Array", "Float64Array",
    // Keyed collections
    "Map", "Set", "WeakMap", "WeakSet",
    // Structured data
    "ArrayBuffer", "SharedArrayBuffer", "Atomics", "DataView", "JSON",
    // Control abstraction
    "Promise", "Iterator", "AsyncIterator", "Generator", "GeneratorFunction",
    "AsyncFunction", "AsyncGenerator", "AsyncGeneratorFunction",
    "DisposableStack", "AsyncDisposableStack",
    // Reflection / proxies
    "Reflect", "Proxy",
    // Internationalization
    "Intl",
    // Resource management
    "FinalizationRegistry", "WeakRef",
    // WebAssembly
    "WebAssembly",
]);

// Self-references and runtime internals are not part of either API namespace.
//
// ORDERING HAZARD -- read before adding anything to globalThis.
//
// These namespaces are built here, during bootstrap. `harden_global_scope`
// (Rust, `lib.rs`) deletes the deno_core internals from globalThis *after*
// that, because deleting them from JS breaks deno_core's snapshot restore
// path. So a mirror built first captures whatever hardening deletes later, and
// deleting the global afterwards does nothing to the copy.
//
// That is not hypothetical: `Deno` was mirrored onto both namespaces and
// `wx.Deno.core.ops` handed content 616 invocable ops -- the whole native
// surface, past every JS-level API and policy. `__bootstrap` escaped only
// because it happens to start with an underscore.
//
// So anything hardening removes must be listed here too, and the two lists are
// kept in agreement by `scripts/test-runtime-internals-not-published.sh`. The
// behavioural backstop is `tests/published_namespace_isolation.rs`, which
// searches the published namespaces for an op table by shape rather than by
// name -- a new internal that leaks fails there even if nobody updates a list.
const _RUNTIME_INTERNALS = new Set([
    "Deno",             // deno_core: `.core.ops` is the entire native surface
    "__bootstrap",      // deno_core: internal module registry
]);

const _NON_API = new Set([
    "GameGlobal", "global", "migo", "wx",     // self-references (added below or by 99_main.js)
    "_perf",                                   // host-only profiler hook
    "console",                                 // standard JS, not wx-namespaced
    "_CCSettings",                             // engine-compat shim (Cocos)
    ..._RUNTIME_INTERNALS,
]);

// Browser content capabilities implemented by the native runtime and surfaced
// by the HTML5 adapter. wx has no corresponding public names.
const _NON_WX = new Set([
    "getGamepads",
    "onGamepadConnected", "offGamepadConnected",
    "onGamepadDisconnected", "offGamepadDisconnected",
]);

function _shouldMirrorApi(key) {
    if (_JS_BUILTINS.has(key)) return false;
    if (_NON_API.has(key)) return false;
    // Exclude any underscore-prefixed name. Two classes:
    //   - V8 / engine internals exposed on globalThis (e.g. wx's __wxConfig,
    //     __WAGameSubContextEndTime__).
    //   - migo's _internal* event-pump hooks intended only for the host's
    //     evaluateJavaScript channel.
    // The real wx.* namespace has zero underscore-prefixed APIs (verified
    // against wx-android.json), so this rule is the right alignment.
    if (key.charCodeAt(0) === 95 /* '_' */) return false;
    return true;
}

function _copyApiNamespace(excluded) {
    const namespace = {};
    const keys = Object.getOwnPropertyNames(globalThis);
    for (let i = 0; i < keys.length; i++) {
        const key = keys[i];
        if (!_shouldMirrorApi(key) || excluded.has(key)) continue;
        // Mirror the descriptor as-is so getters/setters and writability are
        // preserved.  The wx object holds the same function references as
        // globalThis -- equality holds, identity holds.
        try {
            const desc = Object.getOwnPropertyDescriptor(globalThis, key);
            if (desc) ObjectDefineProperty(namespace, key, desc);
        } catch (_) {
            // ignore property that cannot be mirrored
        }
    }
    return namespace;
}

function installApiNamespaces() {
    const migo = _copyApiNamespace(new Set());
    const wx = _copyApiNamespace(_NON_WX);

    // Enumerable to match wx Android, where `wx` appears in
    // Object.getOwnPropertyNames(GameGlobal). Namespace objects are distinct:
    // app/adapters may extend migo without silently mutating the wx contract.
    ObjectDefineProperty(globalThis, "wx", {
        value: wx, writable: true, enumerable: true, configurable: true,
    });
    ObjectDefineProperty(globalThis, "migo", {
        value: migo, writable: true, enumerable: true, configurable: true,
    });
}

export { installApiNamespaces };
