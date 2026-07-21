
function createImageData(a, b) {
    // two overloads:
    // createImageData(width, height)
    // createImageData(imageData)
    const isNumber = (v) => typeof v === "number" && Number.isFinite(v);
    if (isNumber(a)) {
        const width = Math.trunc(a);
        const height = Math.trunc(b);
        if (!isNumber(height) || width < 0 || height < 0) {
            throw new TypeError("createImageData: width and height must be non-negative integers");
        }
        if (typeof ImageData !== "undefined") {
            return new ImageData(width, height);
        }
        const data = new Uint8ClampedArray(width * height * 4);
        return { data, width, height };
    }

    // assume image-like object (has width, height, data)
    if (a && typeof a === "object" && "width" in a && "height" in a && "data" in a) {
        const width = Math.trunc(Number(a.width));
        const height = Math.trunc(Number(a.height));
        if (width < 0 || height < 0) {
            throw new TypeError("createImageData: source width/height must be non-negative");
        }
        // clone data into a Uint8ClampedArray
        const srcData = a.data;
        const cloned = srcData instanceof Uint8ClampedArray
            ? new Uint8ClampedArray(srcData)
            : new Uint8ClampedArray(srcData instanceof ArrayBuffer ? srcData : Array.from(srcData));
        if (typeof ImageData !== "undefined") {
            return new ImageData(cloned, width, height);
        }
        return { data: cloned, width, height };
    }

    throw new TypeError("createImageData: invalid arguments");
}

export { createImageData }