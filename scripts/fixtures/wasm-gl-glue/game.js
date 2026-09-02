// The smallest host adapter an Emscripten export needs to run on Migo.
//
// This exists because of two findings, both of which come *before* any
// question about glue performance:
//
//   1. Emscripten loads its own `.wasm` with `fetch`/`XHR`. Migo has neither,
//      so a stock export aborts at startup with "both async and sync fetching
//      of the wasm failed". Worked around here with `-sSINGLE_FILE=1`, which
//      inlines the module as a data URI. That is fine for measuring per-call
//      cost and wrong for measuring startup -- base64 decode replaces
//      streaming compile.
//   2. Emscripten resolves its render target through the DOM
//      (`document.querySelector('#canvas')`). Migo has no DOM. But a Migo
//      canvas does expose `getContext`, which is all Emscripten's GL layer
//      actually calls, so a two-method `document` stand-in is enough.
//
// Neither is a performance problem, and neither is solved by MigoGLX. They are
// the compatibility floor any Unity/Emscripten export would hit first.
const __migoCanvas = migo.createCanvas();
globalThis.document = globalThis.document || {
    querySelector: () => __migoCanvas,
    getElementById: () => __migoCanvas,
};
globalThis.window = globalThis.window || globalThis;
var Module = {
    canvas: __migoCanvas,
    print: (t) => console.error(t),
    printErr: (t) => console.error(t),
};
