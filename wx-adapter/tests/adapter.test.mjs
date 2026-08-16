// Behavioral test of the wx-adapter against a faked migo runtime.
// Run with `node tests/adapter.test.mjs` from this dir or via npm test.

import assert from "node:assert/strict";

// ---- Fake migo runtime, installed on globalThis BEFORE importing the
//      adapter. The adapter reads `globalThis.migo` at module-eval time.
const fakeMigo = {
  createCanvas: () => ({ width: 0, height: 0 }),
  getSystemInfoSync: () => ({ platform: "android" }),
  onTouchStart: (cb) => cb,
  clearStorage: () => {},
  // migo-only, no wx equivalent -- must NOT appear on wx.
  getGamepads: () => [],
  onGamepadConnected: () => {},
  offGamepadConnected: () => {},
  onGamepadDisconnected: () => {},
  offGamepadDisconnected: () => {},
};
Object.defineProperty(globalThis, "migo", {
  value: fakeMigo, writable: true, configurable: true, enumerable: true,
});

const { default: wx } = await import("../src/index.js");

// 1. Shared capabilities are the SAME function reference as migo's --
//    aliasing/copying, not reimplementation.
assert.equal(wx.createCanvas, fakeMigo.createCanvas, "wx.createCanvas === migo.createCanvas (same reference)");
assert.equal(wx.getSystemInfoSync, fakeMigo.getSystemInfoSync);
assert.equal(wx.clearStorage, fakeMigo.clearStorage);

// 2. Gamepad (migo-only, no real-wx equivalent) is excluded from wx.
assert.equal(typeof wx.getGamepads, "undefined", "getGamepads must not leak onto wx");
assert.equal(typeof wx.onGamepadConnected, "undefined");
assert.equal(typeof wx.offGamepadConnected, "undefined");
assert.equal(typeof wx.onGamepadDisconnected, "undefined");
assert.equal(typeof wx.offGamepadDisconnected, "undefined");

// 3. Gamepad stays reachable on migo directly -- this adapter narrows wx,
//    it doesn't remove anything from migo.
assert.equal(typeof globalThis.migo.getGamepads, "function");

// 4. globalThis.wx is the published object.
assert.equal(globalThis.wx, wx, "globalThis.wx is set");
assert.equal(globalThis.__migoWxAdapterInjected, true);

// Idempotency on re-entry is exercised in bundle.test.mjs, which runs the
// raw script twice via vm.runInContext -- ESM import caching makes a second
// `import()` of this same module a no-op regardless, so it isn't a
// meaningful test of the guard at this level.

console.log("ADAPTER TEST PASSED");
