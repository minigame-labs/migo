// Behavioral check that dist/migo-wx-adapter.bundle.js, when evaluated as a
// plain script (no module loader, no `import`), publishes the same `wx`
// surface as the ESM entry. This is the contract the migo runtime relies on
// when injecting the bundle as a boot prelude.
//
// Run inside a fresh `vm.Context` so the test process's own globals don't
// mask whatever the adapter sets, and so re-running the raw script twice
// (below) is a clean test of the idempotency guard.

import assert from "node:assert/strict";
import vm from "node:vm";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

const __dirname = dirname(fileURLToPath(import.meta.url));
const bundlePath = resolve(__dirname, "../dist/migo-wx-adapter.bundle.js");
const bundleSrc = readFileSync(bundlePath, "utf8");

const fakeMigo = {
  createCanvas: () => ({ width: 0, height: 0 }),
  getSystemInfoSync: () => ({ platform: "ios" }),
  getGamepads: () => [],
  onGamepadConnected: () => {},
};

const sandbox = { console, migo: fakeMigo };

// Crucially, do NOT pre-populate wx — the bundle must set it itself.
vm.createContext(sandbox);
vm.runInContext(bundleSrc, sandbox, { filename: bundlePath });

// 1. wx published, aliasing migo's own capabilities (same reference).
assert.equal(typeof sandbox.wx, "object", "wx is published");
assert.equal(sandbox.wx.createCanvas, fakeMigo.createCanvas, "wx.createCanvas === migo.createCanvas");
assert.equal(sandbox.wx.getSystemInfoSync, fakeMigo.getSystemInfoSync);

// 2. migo-only capabilities excluded from wx.
assert.equal(typeof sandbox.wx.getGamepads, "undefined", "getGamepads must not leak onto wx");
assert.equal(typeof sandbox.wx.onGamepadConnected, "undefined");
assert.equal(typeof sandbox.migo.getGamepads, "function", "still reachable on migo directly");

// 3. Idempotent: re-evaluating the bundle doesn't rebuild wx or throw.
const wxBefore = sandbox.wx;
vm.runInContext(bundleSrc, sandbox, { filename: bundlePath });
assert.equal(sandbox.wx, wxBefore, "second eval is a no-op (sentinel honored)");
assert.equal(sandbox.__migoWxAdapterInjected, true);

// 4. Fails loudly if migo isn't present -- confirms the adapter can't
//    silently publish an empty/broken wx.
const emptySandbox = { console };
vm.createContext(emptySandbox);
assert.throws(
  () => vm.runInContext(bundleSrc, emptySandbox, { filename: bundlePath }),
  /globalThis\.migo is not present/,
  "throws when migo hasn't booted yet"
);

console.log("BUNDLE TEST PASSED");
