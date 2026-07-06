// Behavioral check that dist/migo-adapter.bundle.js, when evaluated as a
// plain script (no module loader, no `import`), wires up the same BOM/DOM
// surface as the ESM entry. This is the contract the migo runtime relies on
// when injecting the bundle as a boot prelude.
//
// We run the bundle inside a fresh `vm.Context` so the test process's own
// globals (e.g. real `XMLHttpRequest`) don't mask whatever the adapter sets.

import assert from "node:assert/strict";
import vm from "node:vm";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

const __dirname = dirname(fileURLToPath(import.meta.url));
const bundlePath = resolve(__dirname, "../dist/migo-adapter.bundle.js");
const bundleSrc = readFileSync(bundlePath, "utf8");

// Build a minimal fake migo on a fresh context. Mirrors the runtime contract
// the ESM tests assume; the bundle reads `migo.*` at evaluation time.
const _resizeCbs = [];
const _storage = new Map();

const sandbox = {
  console,
  queueMicrotask,
  setTimeout,
  clearTimeout,
  migo: {
    getWindowInfo: () => ({ windowWidth: 750, windowHeight: 1334, screenWidth: 750, screenHeight: 1334, pixelRatio: 2 }),
    getSystemInfoSync: () => ({ platform: "ios", system: "iOS 17", language: "en-US", version: "1.0.0", screenWidth: 750, screenHeight: 1334, pixelRatio: 2 }),
    onWindowResize: (cb) => _resizeCbs.push(cb),
    createImage: () => ({ src: null, onload: null, onerror: null, width: 0, height: 0 }),
    createCanvas: () => ({ width: 0, height: 0, getContext: () => ({}), toDataURL: () => "data:," }),
    createInnerAudioContext: () => ({
      src: "", paused: true, _l: {},
      onCanplay(cb) { this._l.canplay = cb; },
      onPlay(cb) { this._l.play = cb; },
      onPause(cb) { this._l.pause = cb; },
      onEnded(cb) { this._l.ended = cb; },
      onError(cb) { this._l.error = cb; },
      play() { this.paused = false; this._l.play && this._l.play(); },
      pause() { this.paused = true; this._l.pause && this._l.pause(); },
    }),
    onTouchStart: () => {}, onTouchMove: () => {}, onTouchEnd: () => {}, onTouchCancel: () => {},
    getStorageInfoSync: () => ({ keys: [..._storage.keys()] }),
    getStorageSync: (k) => _storage.get(k) ?? "",
    setStorageSync: (k, v) => { _storage.set(k, v); },
    removeStorageSync: (k) => { _storage.delete(k); },
    clearStorageSync: () => { _storage.clear(); },
    request: (opts) => {
      queueMicrotask(() => opts.success && opts.success({ statusCode: 200, data: "ok" }));
      return { abort: () => {} };
    },
    connectSocket: () => {
      let onMsg, onClose;
      return {
        onOpen: (cb) => queueMicrotask(() => cb && cb({})),
        onMessage: (cb) => { onMsg = cb; },
        onClose: (cb) => { onClose = cb; },
        onError: () => {},
        send: ({ data }) => onMsg && queueMicrotask(() => onMsg({ data })),
        close: () => onClose && queueMicrotask(() => onClose({ code: 1000 })),
      };
    },
  },
};

// Crucially, do NOT pre-populate window/document/etc. — the bundle must
// set those itself. After evaluation, sandbox.window should self-reference.
vm.createContext(sandbox);
vm.runInContext(bundleSrc, sandbox, { filename: bundlePath });

// 1. BOM scalars wired from migo.getWindowInfo
assert.equal(sandbox.innerWidth, 750, "innerWidth from getWindowInfo");
assert.equal(sandbox.innerHeight, 1334);
assert.equal(sandbox.devicePixelRatio, 2);
assert.equal(sandbox.screen.width, 750);

// 2. BOM is writable (game code does window.innerWidth = ...)
sandbox.innerWidth = 1024;
assert.equal(sandbox.innerWidth, 1024);

// 3. Constructors / classes
assert.equal(typeof sandbox.Image, "function");
assert.equal(typeof sandbox.XMLHttpRequest, "function");
assert.equal(typeof sandbox.WebSocket, "function");
assert.equal(typeof sandbox.HTMLElement, "function");

// 4. document basics
const div = sandbox.document.createElement("div");
assert.ok(div instanceof sandbox.HTMLElement);
assert.equal(div.tagName, "DIV");
assert.equal(sandbox.document.getElementById("GameCanvas"), sandbox.canvas);

// 5. window self-references
// Inside vm.runInContext the sandbox object is `globalThis`, so `window`,
// `self`, `parent`, `top` should all alias each other.
assert.equal(sandbox.window, sandbox.self);
assert.equal(sandbox.parent, sandbox.window);
assert.equal(sandbox.top, sandbox.window);

// 6. localStorage roundtrip
sandbox.localStorage.setItem("k", "v");
assert.equal(sandbox.localStorage.getItem("k"), "v");

// 6b. DOM lifecycle fires from the built bundle too (deferred to a macrotask):
// DOMContentLoaded on document, then window 'load', readyState → "complete".
assert.equal(sandbox.document.readyState, "loading", "bundle readyState starts 'loading'");
let bundleDCLFired = false, bundleLoadFired = false;
sandbox.document.addEventListener("DOMContentLoaded", () => { bundleDCLFired = true; });
sandbox.addEventListener("load", () => { bundleLoadFired = true; });
await new Promise((r) => setTimeout(r, 0));
assert.equal(bundleDCLFired, true, "bundle fires DOMContentLoaded after a macrotask");
assert.equal(bundleLoadFired, true, "bundle fires window 'load' after a macrotask");
assert.equal(sandbox.document.readyState, "complete", "bundle readyState settles at 'complete'");

// 7. Idempotent: re-evaluating the bundle doesn't double-inject.
sandbox.innerWidth = 9999; // user override before re-run
vm.runInContext(bundleSrc, sandbox, { filename: bundlePath });
assert.equal(sandbox.innerWidth, 9999, "second eval is a no-op (sentinel honored)");
assert.equal(sandbox.__migoAdapterInjected, true);

console.log("BUNDLE TEST PASSED");
