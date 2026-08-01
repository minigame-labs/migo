import { getWindowInfo } from "ext:host_v8_system/03_window_info.js";
import { adoptMainCanvasSurfaceSize } from "ext:host_v8_web/03_canvas.js";
import { createListenerGroup } from "ext:host_v8_base/02_async.js";

const _resize = createListenerGroup("onWindowResize");
let _lastWidth = -1;
let _lastHeight = -1;

function _readWindowSize() {
    const info = getWindowInfo();
    return {
        windowWidth: info.windowWidth,
        windowHeight: info.windowHeight,
    };
}

function onWindowResize(listener) {
    if (typeof listener === "function") {
        _resize.on(listener);
        try {
            listener(_readWindowSize());
        } catch (e) {
            console.error("onWindowResize listener error:", e);
        }
    }
}

function offWindowResize(listener) {
    _resize.off(listener);
}

function _internalTriggerWindowResize() {
    // First, and unconditionally.
    //
    // The main canvas is what the content draws into, so a listener that reads
    // `canvas.width` must not be handed the size the surface had a moment ago.
    // Ahead of the read below because a host with no window-info service still
    // has a surface -- gating this on `_readWindowSize()` succeeding would make
    // the canvas track the surface only on platforms that happen to report
    // their window geometry. Ahead of the de-duplication below for the same
    // reason: it compares the numbers the *platform* reports, which is not what
    // decides whether the surface moved.
    adoptMainCanvasSurfaceSize();

    let data;
    try {
        data = _readWindowSize();
    } catch (e) {
        console.error("onWindowResize read window info failed:", e);
        return;
    }

    if (data.windowWidth === _lastWidth && data.windowHeight === _lastHeight) {
        return;
    }
    _lastWidth = data.windowWidth;
    _lastHeight = data.windowHeight;

    _resize.trigger(data);
}

export {
    onWindowResize,
    offWindowResize,
    _internalTriggerWindowResize,
};
