// IME composition: forward migo.onComposition{Start,Update,End} to DOM
// compositionstart/compositionupdate/compositionend. Run with
// `node tests/composition-events.test.mjs`.
//
// CJK text input in HTML5 content listens on compositionstart/update/end and
// reads `event.data` (the current preedit). The runtime publishes
// migo.onComposition* with `{type, data}`, but the adapter did not forward them
// to DOM, so such content saw no preedit. Device-free, node-testable (same
// class as the mouse/wheel/keyboard forwarding).

import assert from "node:assert/strict";

let _start, _update, _end;
globalThis.migo = {
  getWindowInfo: () => ({ windowWidth: 390, windowHeight: 844, screenWidth: 390, screenHeight: 844, pixelRatio: 3 }),
  getSystemInfoSync: () => ({ platform: "android", system: "x", language: "zh-CN", version: "1", screenWidth: 390, screenHeight: 844, pixelRatio: 3 }),
  onWindowResize: () => {},
  createImage: () => ({}), createCanvas: () => ({ getContext: () => ({}) }),
  createInnerAudioContext: () => ({ onCanplay(){}, onPlay(){}, onPause(){}, onEnded(){}, onError(){}, play(){}, pause(){}, seek(){} }),
  getStorageInfoSync: () => ({ keys: [] }), getStorageSync: () => "", setStorageSync: () => {}, removeStorageSync: () => {}, clearStorageSync: () => {},
  request: () => ({ abort() {} }), connectSocket: () => ({ onOpen(){}, onMessage(){}, onClose(){}, onError(){}, send(){}, close(){} }),
  onCompositionStart: (cb) => { _start = cb; },
  onCompositionUpdate: (cb) => { _update = cb; },
  onCompositionEnd: (cb) => { _end = cb; },
};

await import("../src/index.js");

const _tracked = [];
function on(type, fn) { document.addEventListener(type, fn); _tracked.push([type, fn]); return fn; }
function untrackAll() { for (const [t, f] of _tracked.splice(0)) document.removeEventListener(t, f); }

let failures = 0;
function check(name, fn) {
  untrackAll();
  try { fn(); console.log("ok -", name); }
  catch (e) { failures++; console.error("FAIL -", name, "\n   ", e.message); }
}

check("compositionstart forwards to DOM with data", () => {
  let seen = null;
  on("compositionstart", (e) => { seen = e; });
  _start({ type: "compositionstart", data: "" });
  assert.ok(seen, "compositionstart must dispatch");
  assert.equal(seen.type, "compositionstart");
  assert.equal(seen.data, "");
});

check("compositionupdate carries the running preedit", () => {
  let data = null;
  on("compositionupdate", (e) => { data = e.data; });
  _update({ type: "compositionupdate", data: "nihao" });
  assert.equal(data, "nihao");
});

check("compositionend carries the committed multibyte text", () => {
  let seen = null;
  on("compositionend", (e) => { seen = e; });
  _end({ type: "compositionend", data: "你好" });
  assert.ok(seen, "compositionend must dispatch");
  assert.equal(seen.data, "你好");
});

check("composition event is a real Event (preventDefault available)", () => {
  let ok = false;
  on("compositionstart", (e) => { e.preventDefault(); ok = e.defaultPrevented; });
  _start({ type: "compositionstart", data: "" });
  assert.equal(ok, true);
});

if (failures > 0) { console.error(`\n${failures} failing`); process.exit(1); }
console.log("\nall composition-events checks passed");
