import {
    op_get_screen_brightness,
    op_set_screen_brightness,
    op_set_keep_screen_on,
    op_set_device_orientation,
    op_start_capture_screen,
    op_stop_capture_screen,
    op_set_enable_debug,
} from "ext:core/ops";
import { wrapAsync } from "ext:host_v8_base/02_async.js";

function getScreenBrightness(options) {
    return wrapAsync('getScreenBrightness', function () {
        const value = op_get_screen_brightness();
        return { value: value };
    }, options);
}

function setScreenBrightness(options = {}) {
    return wrapAsync('setScreenBrightness', function () {
        var value = options.value;
        if (typeof value !== 'number') {
            throw new Error('value is required and must be a number');
        }
        op_set_screen_brightness(value);
    }, options);
}

function setKeepScreenOn(options = {}) {
    return wrapAsync('setKeepScreenOn', function () {
        var keepScreenOn = options.keepScreenOn;
        if (typeof keepScreenOn !== 'boolean') {
            throw new Error('keepScreenOn is required and must be a boolean');
        }
        op_set_keep_screen_on(keepScreenOn);
    }, options);
}

function setDeviceOrientation(options = {}) {
    return wrapAsync('setDeviceOrientation', function () {
        var value = options.value;
        if (value !== 'landscape' && value !== 'portrait') {
            throw new Error('value must be "landscape" or "portrait"');
        }
        op_set_device_orientation(value);
    }, options);
}

// ==================== Debug ====================

function setEnableDebug(options = {}) {
    return wrapAsync('setEnableDebug', function () {
        var enableDebug = options.enableDebug;
        if (typeof enableDebug !== 'boolean') {
            throw new Error('enableDebug is required and must be a boolean');
        }
        op_set_enable_debug(enableDebug);
    }, options);
}

// ==================== User Capture Screen ====================

var _captureScreenListeners = [];

function onUserCaptureScreen(listener) {
    if (typeof listener === 'function') {
        var hadListeners = _captureScreenListeners.length > 0;
        _captureScreenListeners.push(listener);
        if (!hadListeners) {
            try { op_start_capture_screen(); } catch (_) {}
        }
    }
}

function offUserCaptureScreen(listener) {
    if (typeof listener === 'function') {
        var i = _captureScreenListeners.indexOf(listener);
        if (i !== -1) _captureScreenListeners.splice(i, 1);
    } else {
        _captureScreenListeners.length = 0;
    }
    if (_captureScreenListeners.length === 0) {
        try { op_stop_capture_screen(); } catch (_) {}
    }
}

function _internalTriggerUserCaptureScreen() {
    for (var i = 0; i < _captureScreenListeners.length; i++) {
        try {
            _captureScreenListeners[i]({});
        } catch (e) {
            console.error('onUserCaptureScreen listener error:', e);
        }
    }
}

export {
    getScreenBrightness,
    setScreenBrightness,
    setKeepScreenOn,
    setDeviceOrientation,
    setEnableDebug,
    onUserCaptureScreen,
    offUserCaptureScreen,
    _internalTriggerUserCaptureScreen,
};
