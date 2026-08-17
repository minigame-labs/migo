// VideoDecoder
//
// @stub All methods are no-op. Video decoding is not yet supported.
// Requires a native video codec integration to be functional.
//
// Unlike the other no-op stubs in this runtime, this one cannot fail quietly
// and be harmless. Analytics and RealtimeLogManager are fire-and-forget --
// content never waits on them, so a no-op costs nothing. Content *does* wait on
// a decoder: it calls start(), then polls getFrameData() or waits for an event.
// With no frames and no events, that wait never ends, and the game looks frozen
// with nothing in the log to explain it.
//
// So this warns once, on the same pattern as the analytics stub. It does not
// make the API work, and it deliberately does not invent a failure event:
// `on(type, ...)` takes a type string whose vocabulary belongs to the API
// contract, and emitting one nobody listens for would trade a silent hang for a
// silent hang plus a lie. Reporting through the real event set, or not
// publishing `createVideoDecoder` at all so `typeof` feature detection can
// succeed, are both live options -- and both are decisions about the published
// surface rather than about this file.

let _videoDecoderWarned = false;
function _warnOnce() {
    if (!_videoDecoderWarned) {
        _videoDecoderWarned = true;
        console.error(
            'VideoDecoder is not implemented in this build: every method is a no-op, ' +
            'getFrameData() always returns null, and no listener registered through ' +
            'on() will ever fire. Content that waits for decoded frames will wait ' +
            'forever. Feature-detect before use, or decode elsewhere.'
        );
    }
}

class VideoDecoder {
    start(options) { _warnOnce(); }
    stop() {}
    seek(position) {}
    remove() {}
    getFrameData() { _warnOnce(); return null; }
    on(type, listener) { _warnOnce(); }
    off(type, listener) {}
}

function createVideoDecoder() {
    _warnOnce();
    return new VideoDecoder();
}

export { createVideoDecoder };
