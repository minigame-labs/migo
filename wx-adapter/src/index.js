// migo-wx-adapter entry point. Publishes `globalThis.wx`, aliased to the
// migo runtime's own `migo.*` capabilities, so mini-game content written
// against the `wx` global (unmodified WeChat mini-game source, or content
// from a similarly-shaped mini-game platform) runs on migo unchanged.
//
// Usage (game side, before content that references `wx` runs):
//
//   import "@minigame-labs/migo-wx-adapter";       // ESM
//   // or, in CommonJS / require-style:
//   require("./wx-adapter/src/index.js");
//
// One-shot: idempotent on re-entry, and safe to load alongside
// @minigame-labs/migo-adapter (the BOM/DOM adapter) -- they touch disjoint
// globals (`wx` here, `window`/`document`/etc. there).
//
// What this is NOT: a reimplementation of wx. `migo`'s capabilities for
// every name wx also defines are the *same functions* wx would call --
// this adapter is a naming/shaping layer, not new behavior. If migo does
// not implement a given wx API, `wx.thatApi` is `undefined` here exactly
// as it is on `migo` directly; this adapter cannot manufacture capabilities
// the runtime doesn't have.

// Capabilities migo exposes beyond the common mini-game surface -- present
// on real engines for browser-content interop (Web Gamepad), but with no
// wx equivalent. Kept off `wx` so feature-detection code ported from real
// wx content sees the same absence it would see there. Mirrors
// `_NON_MINIGAME_API` in the engine's own `97_migo_namespace.js`; update
// both together if that set ever changes.
const NON_WX = new Set([
  "getGamepads",
  "onGamepadConnected", "offGamepadConnected",
  "onGamepadDisconnected", "offGamepadDisconnected",
]);

if (!globalThis.__migoWxAdapterInjected) {
  globalThis.__migoWxAdapterInjected = true;

  if (typeof globalThis.migo !== "object" || globalThis.migo === null) {
    throw new Error(
      "@minigame-labs/migo-wx-adapter: globalThis.migo is not present. " +
      "This adapter must load after the migo runtime has booted (migo " +
      "installs its namespace during bootstrap, before any content runs)."
    );
  }

  const wx = {};
  const keys = Object.getOwnPropertyNames(globalThis.migo);
  for (let i = 0; i < keys.length; i++) {
    const key = keys[i];
    if (NON_WX.has(key)) continue;
    // Mirror the descriptor as-is, the same way the engine's own namespace
    // projection does: preserves getters/setters and writability rather
    // than flattening everything to a plain value copy.
    const desc = Object.getOwnPropertyDescriptor(globalThis.migo, key);
    if (desc) Object.defineProperty(wx, key, desc);
  }

  Object.defineProperty(globalThis, "wx", {
    value: wx, writable: true, enumerable: true, configurable: true,
  });
}

export default globalThis.wx;
