// VideoDecoder
//
// @stub All methods are no-op. Video decoding is not yet supported.
// Requires a native video codec integration to be functional.

class VideoDecoder {
    start(options) {}
    stop() {}
    seek(position) {}
    remove() {}
    getFrameData() { return null; }
    on(type, listener) {}
    off(type, listener) {}
}

function createVideoDecoder() {
    return new VideoDecoder();
}

export { createVideoDecoder };
