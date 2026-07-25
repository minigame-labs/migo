// Physical keyboard: forward migo.onKeyDown/onKeyUp to DOM keydown/keyup.
// Run with `node tests/keyboard-events.test.mjs`.
//
// HTML5 games listen with `addEventListener('keydown', ...)` (WASD/arrows).
// The migo runtime already publishes migo.onKeyDown/onKeyUp with DOM key/code
// fields, but the adapter did not forward them to DOM, so such content received
// nothing. This is device-free and node-testable (same class as mouse/wheel).

import assert from "node:assert/strict";

let _keyDown, _keyUp;
globalThis.migo = {
  getWindowInfo: () => ({ windowWidth: 390, windowHeight: 844, screenWidth: 390, screenHeight: 844, pixelRatio: 3 }),
  getSystemInfoSync: () => ({ platform: "android", system: "x", language: "en", version: "1", screenWidth: 390, screenHeight: 844, pixelRatio: 3 }),
  onWindowResize: () => {},
  createImage: () => ({}), createCanvas: () => ({ getContext: () => ({}) }),
  createInnerAudioContext: () => ({ onCanplay(){}, onPlay(){}, onPause(){}, onEnded(){}, onError(){}, play(){}, pause(){}, seek(){} }),
  getStorageInfoSync: () => ({ keys: [] }), getStorageSync: () => "", setStorageSync: () => {}, removeStorageSync: () => {}, clearStorageSync: () => {},
  request: () => ({ abort() {} }), connectSocket: () => ({ onOpen(){}, onMessage(){}, onClose(){}, onError(){}, send(){}, close(){} }),
  onKeyDown: (cb) => { _keyDown = cb; }, onKeyUp: (cb) => { _keyUp = cb; },
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

const detail = (over = {}) => ({
  key: "a", code: "KeyA", timeStamp: 7,
  ctrlKey: false, shiftKey: false, altKey: false, metaKey: false, repeat: false, ...over,
});

check("keydown forwards to DOM with key/code and modifiers", () => {
  let seen = null;
  on("keydown", (e) => { seen = e; });
  _keyDown(detail({ code: "KeyW", key: "w", shiftKey: true, repeat: true }));
  assert.ok(seen, "a DOM keydown must be dispatched");
  assert.equal(seen.type, "keydown");
  assert.equal(seen.key, "w");
  assert.equal(seen.code, "KeyW");
  assert.equal(seen.shiftKey, true);
  assert.equal(seen.repeat, true);
});

check("keyup forwards to DOM", () => {
  let seen = null;
  on("keyup", (e) => { seen = e; });
  _keyUp(detail());
  assert.ok(seen, "a DOM keyup must be dispatched");
  assert.equal(seen.type, "keyup");
  assert.equal(seen.code, "KeyA");
});

check("keyboard event supports preventDefault", () => {
  let prevented = null;
  on("keydown", (e) => { e.preventDefault(); prevented = e.defaultPrevented; });
  _keyDown(detail());
  assert.equal(prevented, true, "KeyboardEvent must be a real cancelable Event");
});

if (failures > 0) { console.error(`\n${failures} failing`); process.exit(1); }
console.log("\nall keyboard-events checks passed");
