// BOM window-metrics globals: innerWidth/innerHeight/outerWidth/outerHeight,
// screenWidth/screenHeight, devicePixelRatio, and a screen object.
//
// Semantics match the canonical mini-game adapter (verified against the
// reference adapter source): one-shot snapshot at adapter load, exposed as
// writable data properties on globalThis. Assignment by game code persists
// (we do NOT subscribe to host resize events and overwrite). Games that need
// live window metrics call migo.getWindowInfo() / migo.getSystemInfoSync()
// directly -- the same expectation the canonical adapter sets.

const _info = (() => {
  // Prefer getWindowInfo (newer, returns windowWidth/Height); fall back to
  // getSystemInfoSync for older runtimes.
  try {
    if (typeof migo.getWindowInfo === "function") return migo.getWindowInfo();
  } catch {}
  try {
    if (typeof migo.getSystemInfoSync === "function") return migo.getSystemInfoSync();
  } catch {}
  return { windowWidth: 0, windowHeight: 0, screenWidth: 0, screenHeight: 0, pixelRatio: 1 };
})();

// Report window metrics in PHYSICAL (device) pixels with devicePixelRatio = 1.
//
// migo (correctly, matching wx semantics) returns windowWidth/Height in
// LOGICAL/CSS pixels plus a real pixelRatio (e.g. 360x780 @ dpr 3). In a real
// browser a game that renders to a logical-sized backing store is CSS-scaled up
// to fill the display; migo's onscreen canvas has no separate CSS display size —
// its backing store IS the physical surface. So a DPR-naive engine (Pixi/Phaser
// created with resolution = 1) that sizes its renderer to innerWidth would set a
// logical-sized (360) GL viewport on the physical (1080) surface and render into
// the bottom-left corner (or, for engines that render to their own FBO first,
// present nothing). Collapsing to physical dimensions + dpr = 1 makes such
// engines render at native resolution, full-screen, whether or not they are
// DPR-aware (a DPR-aware engine multiplies by our dpr = 1, i.e. no change).
// This matches the "physical pixels, dpr 1" convention used by other minigame
// browser-shim adapters. Games that need true logical metrics/dpr can still call
// migo.getWindowInfo() / migo.getSystemInfoSync() directly.
const _dpr = _info.pixelRatio || 1;
export const devicePixelRatio = 1;
export const innerWidth = Math.round((_info.windowWidth || _info.screenWidth || 0) * _dpr);
export const innerHeight = Math.round((_info.windowHeight || _info.screenHeight || 0) * _dpr);
export const outerWidth = innerWidth;
export const outerHeight = innerHeight;
export const screenWidth = Math.round((_info.screenWidth || _info.windowWidth || 0) * _dpr);
export const screenHeight = Math.round((_info.screenHeight || _info.windowHeight || 0) * _dpr);

export const screen = {
  width: screenWidth,
  height: screenHeight,
  availWidth: innerWidth,
  availHeight: innerHeight,
  availLeft: 0,
  availTop: 0,
  orientation: { angle: 0, type: innerWidth > innerHeight ? "landscape-primary" : "portrait-primary" },
};
