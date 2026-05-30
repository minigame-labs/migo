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

export const innerWidth = _info.windowWidth || _info.screenWidth || 0;
export const innerHeight = _info.windowHeight || _info.screenHeight || 0;
export const outerWidth = innerWidth;
export const outerHeight = innerHeight;
export const screenWidth = _info.screenWidth || innerWidth;
export const screenHeight = _info.screenHeight || innerHeight;
export const devicePixelRatio = _info.pixelRatio || 1;

export const screen = {
  width: screenWidth,
  height: screenHeight,
  availWidth: innerWidth,
  availHeight: innerHeight,
  availLeft: 0,
  availTop: 0,
  orientation: { angle: 0, type: innerWidth > innerHeight ? "landscape-primary" : "portrait-primary" },
};
