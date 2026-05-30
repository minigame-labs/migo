// Image() — engines use `new Image(); img.src = '...';  img.onload = ...`.
// migo.createImage() already returns a host Image with `src` setter and
// onload/onerror, so we just hand that back from a constructor.

export default function Image() {
  if (typeof migo.createImage !== "function") {
    throw new Error("[migo-adapter] migo.createImage is not available");
  }
  return migo.createImage();
}
