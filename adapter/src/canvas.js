// Canvas — engines either call `document.createElement('canvas')` or
// `new Canvas()`. Both must return a host canvas (with getContext, width,
// height, toDataURL). migo.createCanvas() already does that, so we just
// expose a constructor wrapper for the `new Canvas()` form.

export default function Canvas() {
  if (typeof migo.createCanvas !== "function") {
    throw new Error("[migo-adapter] migo.createCanvas is not available");
  }
  const c = migo.createCanvas();
  // Engines may want addEventListener on the canvas (touch input). Forward
  // those to the global touch event source set up by index.js.
  if (typeof c.addEventListener !== "function") {
    c.addEventListener = () => {};
    c.removeEventListener = () => {};
    c.dispatchEvent = () => {};
  }
  return c;
}
