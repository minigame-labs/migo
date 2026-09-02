// Injected with `--pre-js`, so it runs *inside* the module scope.
//
// Three things an Emscripten export needs that Migo does not provide, each
// found by hitting it:
//
//   1. `fetch`/`XHR` to load its own `.wasm` -- worked around with
//      `-sSINGLE_FILE=1` at build time, not here.
//   2. a DOM to resolve `#canvas`.
//   3. `specialHTMLTargets`, Emscripten's own target table, which is consulted
//      *before* the DOM and is module-scoped -- an outer shim cannot reach it,
//      which is why this file is a `--pre-js` and not a prelude.
var __migoCanvas = migo.createCanvas();
globalThis.document = globalThis.document || {
    querySelector: () => __migoCanvas,
    getElementById: () => __migoCanvas,
};
globalThis.window = globalThis.window || globalThis;
Module["canvas"] = __migoCanvas;
Module["print"] = (t) => console.error(t);
Module["printErr"] = (t) => console.error(t);
Module["preRun"] = Module["preRun"] || [];
Module["preRun"].push(function () {
    specialHTMLTargets["#canvas"] = __migoCanvas;
    specialHTMLTargets["canvas"] = __migoCanvas;
    console.error("[pre] preRun ran; canvas=" + typeof __migoCanvas
        + " getContext=" + typeof __migoCanvas.getContext);
    try {
        const probe = __migoCanvas.getContext("webgl2");
        console.error("[pre] direct getContext('webgl2') => " + typeof probe
            + (probe ? " ok" : " NULL"));
    } catch (e) {
        console.error("[pre] getContext threw: " + e);
    }
    // Ask GL.createContext itself why it refuses: findCanvasEventTarget
    // succeeds (the document stand-in returns the canvas), so the failure is
    // downstream of target resolution.
    // Emscripten wraps `getContext` to work around a Safari bug, and the
    // wrapper decides which version it got with
    // `gl instanceof WebGLRenderingContext`. Migo exposes no such global
    // constructor, so the check is false for a perfectly good context and
    // `createContext` returns 0. Give it a constructor whose prototype the
    // WebGL1 context is not on, so `instanceof` answers correctly for both.
    {
        const g2 = __migoCanvas.getContext("webgl2");
        console.error("[pre] WebGLRenderingContext global? "
            + (typeof globalThis.WebGLRenderingContext)
            + "; WebGL2RenderingContext? " + (typeof globalThis.WebGL2RenderingContext)
            + "; webgl2 ctx instanceof WebGLRenderingContext = "
            + (typeof globalThis.WebGLRenderingContext !== "undefined"
                ? (g2 instanceof globalThis.WebGLRenderingContext) : "n/a")
            + "; ctor=" + (g2 && g2.constructor && g2.constructor.name));
    }
    if (typeof globalThis.WebGLRenderingContext === "undefined") {
        globalThis.WebGLRenderingContext = function WebGLRenderingContext() {};
        console.error("[pre] installed a WebGLRenderingContext stand-in");
    }
    if (typeof GL !== "undefined" && GL.createContext) {
        const h = GL.createContext(__migoCanvas, { majorVersion: 2, minorVersion: 0 });
        console.error("[pre] GL.createContext => " + h);
    } else {
        console.error("[pre] GL not visible from preRun");
    }
});
