import {
    op_get_screen_brightness,
    op_set_screen_brightness,
    op_set_keep_screen_on,
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
        const code = op_set_screen_brightness(value);
        if (code !== 0) {
            throw new Error('failed to set screen brightness');
        }
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
        const code = op_set_keep_screen_on(keepScreenOn);
        if (code !== 0) {
            throw new Error('failed to set keep screen on');
        }
    }, options);
}

export { getScreenBrightness, setScreenBrightness, setKeepScreenOn };
