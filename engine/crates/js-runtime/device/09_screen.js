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
    const { value } = options;
    if (typeof value !== 'number') {
        const res = { errMsg: 'setScreenBrightness:fail value is required and must be a number' };
        if (typeof options.fail === 'function') {
            queueMicrotask(function () { options.fail(res); });
        }
        if (typeof options.complete === 'function') {
            queueMicrotask(function () { options.complete(res); });
        }
        return Promise.reject(res);
    }
    return wrapAsync('setScreenBrightness', function () {
        op_set_screen_brightness(value);
    }, options);
}

function setKeepScreenOn(options = {}) {
    const { keepScreenOn } = options;
    if (typeof keepScreenOn !== 'boolean') {
        const res = { errMsg: 'setKeepScreenOn:fail keepScreenOn is required and must be a boolean' };
        if (typeof options.fail === 'function') {
            queueMicrotask(function () { options.fail(res); });
        }
        if (typeof options.complete === 'function') {
            queueMicrotask(function () { options.complete(res); });
        }
        return Promise.reject(res);
    }
    return wrapAsync('setKeepScreenOn', function () {
        op_set_keep_screen_on(keepScreenOn);
    }, options);
}

function setDeviceOrientation(options = {}) {
    const { value } = options;
    if (value !== 'landscape' && value !== 'portrait') {
        const res = { errMsg: 'setDeviceOrientation:fail value must be "landscape" or "portrait"' };
        if (typeof options.fail === 'function') {
            queueMicrotask(function () { options.fail(res); });
        }
        if (typeof options.complete === 'function') {
            queueMicrotask(function () { options.complete(res); });
        }
        return Promise.reject(res);
    }
    return wrapAsync('setDeviceOrientation', function () {
        op_set_device_orientation(value);
    }, options);
}

// ==================== Debug ====================

function setEnableDebug(options = {}) {
    const { enableDebug } = options;
    if (typeof enableDebug !== 'boolean') {
        var res = { errMsg: 'setEnableDebug:fail enableDebug is required and must be a boolean' };
        if (typeof options.fail === 'function') {
            queueMicrotask(function () { options.fail(res); });
        }
        if (typeof options.complete === 'function') {
            queueMicrotask(function () { options.complete(res); });
        }
        return Promise.reject(res);
    }
    return wrapAsync('setEnableDebug', function () {
        op_set_enable_debug(enableDebug);
    }, options);
}

// ==================== User Capture Screen ====================

let _captureScreenListener = null;

function onUserCaptureScreen(listener) {
    if (typeof listener === 'function') {
        const hadListener = _captureScreenListener !== null;
        _captureScreenListener = listener;
        if (!hadListener) {
            try { op_start_capture_screen(); } catch (_) {}
        }
    }
}

function offUserCaptureScreen() {
    if (_captureScreenListener !== null) {
        _captureScreenListener = null;
        try { op_stop_capture_screen(); } catch (_) {}
    }
}

function _internalTriggerUserCaptureScreen() {
    if (_captureScreenListener) {
        try {
            _captureScreenListener({});
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
