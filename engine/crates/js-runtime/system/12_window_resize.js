import { getWindowInfo } from "ext:host_v8_system/03_window_info.js";

const _resizeListeners = [];
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
        _resizeListeners.push(listener);
    }
}

function offWindowResize(listener) {
    if (typeof listener === "function") {
        const idx = _resizeListeners.indexOf(listener);
        if (idx !== -1) {
            _resizeListeners.splice(idx, 1);
        }
    } else {
        _resizeListeners.length = 0;
    }
}

function _internalTriggerWindowResize() {
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

    for (let i = 0; i < _resizeListeners.length; i++) {
        try {
            _resizeListeners[i](data);
        } catch (e) {
            console.error("onWindowResize listener error:", e);
        }
    }
}

export {
    onWindowResize,
    offWindowResize,
    _internalTriggerWindowResize,
};
