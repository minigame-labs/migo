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
});
