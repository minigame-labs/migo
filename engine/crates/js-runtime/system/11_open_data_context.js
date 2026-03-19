import { createOffscreenCanvas, getMainCanvas } from "ext:host_v8_web/03_canvas.js";
import { getWindowInfo } from "ext:host_v8_system/03_window_info.js";

const _messageListeners = [];

let _sharedCanvas = null;
let _openDataContext = null;

function _createSharedCanvas(mode) {
    var canvas;

    if (mode === "screenCanvas") {
        canvas = getMainCanvas();
    } else {
        canvas = createOffscreenCanvas(1, 1);
        try {
            const info = getWindowInfo();
            if (info && info.windowWidth > 0) {
                canvas.width = info.windowWidth;
            }
            if (info && info.windowHeight > 0) {
                canvas.height = info.windowHeight;
            }
        } catch (_) {
            // Ignore dimension sync failure; canvas remains usable.
        }
    }

    _sharedCanvas = canvas;

    try {
        globalThis.sharedCanvas = _sharedCanvas;
    } catch (_) {
        // Ignore global assignment failure.
    }

    return _sharedCanvas;
}

function _dispatchMessage(message) {
    for (let i = 0; i < _messageListeners.length; i++) {
        try {
            _messageListeners[i](message);
        } catch (e) {
            console.error("onMessage listener error:", e);
        }
    }
}

function onMessage(callback) {
    if (typeof callback === "function") {
        _messageListeners.push(callback);
    }
}

function offMessage(callback) {
    if (typeof callback === "function") {
        const i = _messageListeners.indexOf(callback);
        if (i !== -1) _messageListeners.splice(i, 1);
    } else {
        _messageListeners.length = 0;
    }
}

class OpenDataContext {
    constructor(canvas) {
        this.canvas = canvas;
    }

    postMessage(message) {
        queueMicrotask(function () {
            _dispatchMessage(message);
        });
    }
}

function getOpenDataContext(object) {
    if (_openDataContext === null) {
        var mode = "offscreenCanvas";
        if (object && object.sharedCanvasMode === "screenCanvas") {
            mode = "screenCanvas";
        }
        _openDataContext = new OpenDataContext(_createSharedCanvas(mode));
    }
    return _openDataContext;
}

function getSharedCanvas() {
    if (_sharedCanvas) {
        return _sharedCanvas;
    }
    return _createSharedCanvas("offscreenCanvas");
}

export {
    onMessage,
    offMessage,
    getOpenDataContext,
    getSharedCanvas,
};
