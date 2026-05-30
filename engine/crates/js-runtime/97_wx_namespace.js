// wx namespace mirror.
//
// wx mini-game code is hard-coded against the global `wx` object
// (`wx.createCanvas`, `wx.getSystemInfoSync`, `wx.onTouchStart`, ...). On real
// wx Android, `wx` is a distinct object that holds ~463 APIs (verified
// against wx-android.json), separate from GameGlobal.
//
// migo currently registers all wx-style APIs directly on globalThis. To make
// wx games run unmodified we expose a `wx` (and `migo`) object that mirrors
// those APIs while keeping the existing globalThis registrations in place
// for backward compatibility. A future pass will move APIs off globalThis
// onto `wx` natively, matching wx Android exactly.
//
// Rules:
//   - wx, migo are the SAME object (alias) -- both yield the same function
//     references, additions on either are visible on both.
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

// Self-references and runtime internals not part of the wx surface.
const _NON_WX = new Set([
    "GameGlobal", "global", "migo", "wx",     // self-references (added below or by 99_main.js)
    "_perf",                                   // host-only profiler hook
    "console",                                 // standard JS, not wx-namespaced
    "_CCSettings",                             // engine-compat shim (Cocos)
]);

function _shouldMirror(key) {
    if (_JS_BUILTINS.has(key)) return false;
    if (_NON_WX.has(key)) return false;
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

function installWxNamespace() {
    const wx = {};
    const keys = Object.getOwnPropertyNames(globalThis);
    for (let i = 0; i < keys.length; i++) {
        const key = keys[i];
        if (!_shouldMirror(key)) continue;
        // Mirror the descriptor as-is so getters/setters and writability are
        // preserved.  The wx object holds the same function references as
        // globalThis -- equality holds, identity holds.
        try {
            const desc = Object.getOwnPropertyDescriptor(globalThis, key);
            if (desc) ObjectDefineProperty(wx, key, desc);
        } catch (_) {
            // ignore property that cannot be mirrored
        }
    }

    // wx and migo: same object, two names. Future-added APIs assigned to
    // either show up on both (because they ARE both). Enumerable to match
    // wx Android, where `wx` appears in Object.getOwnPropertyNames(GameGlobal).
    ObjectDefineProperty(globalThis, "wx", {
        value: wx, writable: true, enumerable: true, configurable: true,
    });
    ObjectDefineProperty(globalThis, "migo", {
        value: wx, writable: true, enumerable: true, configurable: true,
    });
}

export { installWxNamespace };
