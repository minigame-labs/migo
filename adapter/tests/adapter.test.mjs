// Behavioral simulation of the migo-adapter against a faked migo runtime.
// Run with `node tests/adapter.test.mjs` from this dir or via npm test.

import assert from "node:assert/strict";

// ---- Build a minimal fake migo runtime on globalThis BEFORE importing the
//      adapter. The adapter reads `migo.*` at module-eval time (bom.js et al).
const _resizeCbs = [];
const _storage = new Map();
const _innerAudios = [];
let _touchStart;

const fakeMigo = {
  // Window metrics
  getWindowInfo: () => ({ windowWidth: 390, windowHeight: 844, screenWidth: 390, screenHeight: 844, pixelRatio: 3 }),
  getSystemInfoSync: () => ({ platform: "android", system: "Android 14", language: "zh-CN", version: "1.2.3", screenWidth: 390, screenHeight: 844, pixelRatio: 3 }),
  onWindowResize: (cb) => _resizeCbs.push(cb),

  // Image / Canvas / Audio factories
  createImage: () => ({ src: null, onload: null, onerror: null, width: 0, height: 0 }),
  createCanvas: () => ({ width: 0, height: 0, getContext: () => ({}), toDataURL: () => "data:," }),
  createInnerAudioContext: () => {
    const ctx = {
      src: "", loop: false, volume: 1, autoplay: false, currentTime: 0, duration: 0, paused: true,
      _listeners: {},
      onCanplay(cb) { this._listeners.canplay = cb; },
      onPlay(cb) { this._listeners.play = cb; },
      onPause(cb) { this._listeners.pause = cb; },
      onEnded(cb) { this._listeners.ended = cb; },
      onError(cb) { this._listeners.error = cb; },
      play() { this.paused = false; this._listeners.play && this._listeners.play(); },
      pause() { this.paused = true; this._listeners.pause && this._listeners.pause(); },
      seek(t) { this.currentTime = t; },
    };
    _innerAudios.push(ctx);
    return ctx;
  },

  // Touch
  onTouchStart: (cb) => { _touchStart = cb; },
  onTouchMove: () => {}, onTouchEnd: () => {}, onTouchCancel: () => {},

  // Storage
  getStorageInfoSync: () => ({ keys: [..._storage.keys()] }),
  getStorageSync: (k) => _storage.get(k) ?? "",
  setStorageSync: (k, v) => { _storage.set(k, v); },
  removeStorageSync: (k) => { _storage.delete(k); },
  clearStorageSync: () => { _storage.clear(); },

  // Network
  request: (opts) => {
    queueMicrotask(() => {
      if (opts.url === "fail") opts.fail && opts.fail({ errMsg: "fail" });
      else opts.success && opts.success({ statusCode: 200, data: opts.responseType === "arraybuffer" ? new ArrayBuffer(4) : "ok" });
    });
    return { abort: () => {} };
  },
  connectSocket: () => {
    let onOpen, onMessage, onClose;
    const task = {
      onOpen: (cb) => { onOpen = cb; queueMicrotask(() => cb && cb({})); },
      onMessage: (cb) => { onMessage = cb; },
      onClose: (cb) => { onClose = cb; },
      onError: () => {},
      send: ({ data }) => { if (onMessage) queueMicrotask(() => onMessage({ data })); },
      close: () => { if (onClose) queueMicrotask(() => onClose({ code: 1000 })); },
    };
    return task;
  },
};
globalThis.migo = fakeMigo;
// Adapter reads `migo.*` only — no other host globals are required.

// ---- Now import the adapter (this triggers module-eval against fakeMigo)
await import("../src/index.js");

// ---- Assertions

// 1. BOM scalars
assert.equal(globalThis.innerWidth, 390, "innerWidth from getWindowInfo");
assert.equal(globalThis.innerHeight, 844);
assert.equal(globalThis.devicePixelRatio, 3);
assert.equal(globalThis.screen.width, 390);
assert.equal(globalThis.screen.availWidth, 390);

// 2. Writable BOM (engines do `window.innerWidth = ...`)
globalThis.innerWidth = 1080;
assert.equal(globalThis.innerWidth, 1080, "innerWidth must be writable");
globalThis.innerWidth = 390; // restore

// 3. Snapshot semantics (matches canonical adapter): user-set values persist,
//    host resize does NOT overwrite globalThis. Games wanting live metrics
//    call migo.getWindowInfo() directly.
globalThis.innerWidth = 1024;
_resizeCbs.forEach((cb) => cb({ windowWidth: 1080, windowHeight: 1920 }));
assert.equal(globalThis.innerWidth, 1024, "user assignment persists across host resize");

// 4. Reset for downstream tests that depend on innerWidth value.
globalThis.innerWidth = 390;
globalThis.innerHeight = 844;

// 4. navigator / location
assert.equal(globalThis.navigator.platform, "android");
assert.match(globalThis.navigator.userAgent, /Android/);
assert.equal(globalThis.location.href, "game.js");

// 5. document
// The DOM lifecycle is deferred to a macrotask, so synchronously (before any
// await) readyState is still "loading" and load/DOMContentLoaded have not
// fired. Register listeners now, then let the macrotask run (section 5b).
assert.equal(globalThis.document.readyState, "loading", "readyState starts 'loading' until lifecycle fires");
let _domContentLoadedFired = false;
let _windowLoadFired = false;
let _readyStateAtLoad = null;
globalThis.document.addEventListener("DOMContentLoaded", () => { _domContentLoadedFired = true; });
globalThis.addEventListener("load", () => { _windowLoadFired = true; _readyStateAtLoad = globalThis.document.readyState; });
const c = globalThis.document.createElement("canvas");
assert.ok(c.getContext, "createElement(canvas) returns a host canvas");
const i = globalThis.document.createElement("img");
assert.ok("src" in i, "createElement(img) returns image-shaped object");
assert.equal(globalThis.document.getElementById("GameCanvas"), globalThis.canvas);
assert.equal(globalThis.document.getElementsByTagName("div").length, 0);

// 5b. DOM lifecycle: after a macrotask the adapter fires DOMContentLoaded then
// window 'load', walking readyState to "complete". Browser engines (Phaser,
// Egret, …) gate their boot on these events, so they must fire.
await new Promise((r) => setTimeout(r, 0));
assert.equal(_domContentLoadedFired, true, "DOMContentLoaded fires after a macrotask");
assert.equal(_windowLoadFired, true, "window 'load' fires after a macrotask");
assert.equal(_readyStateAtLoad, "complete", "readyState is 'complete' when load fires");
assert.equal(globalThis.document.readyState, "complete", "readyState settles at 'complete'");

// 5c. Display-canvas routing: after load, the first createElement('canvas')
// (while the onscreen canvas is unclaimed) is backed by the onscreen canvas so
// an engine that creates its own render canvas (Phaser) draws to the screen,
// which Migo presents. Subsequent canvases are offscreen.
const _routed = globalThis.document.createElement("canvas");
assert.equal(_routed, globalThis.canvas, "first post-load canvas routes to the onscreen canvas");
const _offscreen2 = globalThis.document.createElement("canvas");
assert.notEqual(_offscreen2, globalThis.canvas, "subsequent post-load canvases stay offscreen");

// 6. HTMLElement basics
const div = globalThis.document.createElement("div");
assert.ok(div instanceof globalThis.HTMLElement, "div instanceof HTMLElement");
assert.equal(div.tagName, "DIV");
div.setAttribute("data-foo", 1);
assert.equal(div.getAttribute("data-foo"), 1);
const rect = div.getBoundingClientRect();
assert.equal(rect.width, 390); // matches current innerWidth

// 7. addEventListener on document
let docHits = 0;
globalThis.document.addEventListener("touchstart", () => docHits++);
_touchStart && _touchStart({ touches: [{ clientX: 1, clientY: 2 }] });
assert.equal(docHits, 1, "host touchstart routes to document listeners");
assert.ok(globalThis.canvas, "global canvas is set");

// 8. localStorage
globalThis.localStorage.setItem("k", "v");
assert.equal(globalThis.localStorage.getItem("k"), "v");
assert.equal(globalThis.localStorage.length, 1);
globalThis.localStorage.removeItem("k");
assert.equal(globalThis.localStorage.getItem("k"), null);

// 9. XMLHttpRequest (async)
await new Promise((resolve, reject) => {
  const xhr = new globalThis.XMLHttpRequest();
  xhr.open("GET", "https://example.com");
  xhr.onload = () => {
    try {
      assert.equal(xhr.status, 200);
      assert.equal(xhr.responseText, "ok");
      resolve();
    } catch (e) { reject(e); }
  };
  xhr.onerror = reject;
  xhr.send();
});

// 10. WebSocket roundtrip
await new Promise((resolve, reject) => {
  const ws = new globalThis.WebSocket("wss://example");
  ws.onopen = () => {
    try {
      assert.equal(ws.readyState, globalThis.WebSocket.OPEN);
      ws.onmessage = (m) => {
        try { assert.equal(m.data, "ping"); resolve(); } catch (e) { reject(e); }
      };
      ws.send("ping");
    } catch (e) { reject(e); }
  };
  ws.onerror = reject;
});

// 11. Audio
const a = new globalThis.Audio("res/click.mp3");
assert.equal(a.src, "res/click.mp3");
let audioPlayed = false;
a.addEventListener("play", () => { audioPlayed = true; });
a.play();
assert.equal(audioPlayed, true);

// 12. window self-references
assert.equal(globalThis.window, globalThis);
assert.equal(globalThis.self, globalThis);
assert.equal(globalThis.parent, globalThis);
assert.equal(globalThis.top, globalThis);

// 13. Idempotent re-import
await import("../src/index.js");
assert.equal(globalThis.__migoAdapterInjected, true, "adapter is idempotent");

console.log("ALL ASSERTIONS PASSED (13 sections)");
