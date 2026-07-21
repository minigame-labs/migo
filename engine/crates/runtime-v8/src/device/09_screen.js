import {
    op_get_screen_brightness,
    op_set_screen_brightness,
    op_set_keep_screen_on,
    op_set_device_orientation,
    op_start_capture_screen,
    op_stop_capture_screen,
    op_set_enable_debug,
} from "ext:core/ops";
import { wrapAsync, createListenerGroup } from "ext:host_v8_base/02_async.js";

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

var _captureScreenListeners = createListenerGroup('onUserCaptureScreen');

function onUserCaptureScreen(listener) {
    if (typeof listener === 'function') {
        var hadListeners = _captureScreenListeners.size() > 0;
        _captureScreenListeners.on(listener);
        if (!hadListeners) {
            try { op_start_capture_screen(); } catch (_) {}
        }
    }
}

function offUserCaptureScreen(listener) {
    _captureScreenListeners.off(listener);
    if (_captureScreenListeners.size() === 0) {
        try { op_stop_capture_screen(); } catch (_) {}
    }
}

function _internalTriggerUserCaptureScreen() {
    _captureScreenListeners.trigger({});
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
