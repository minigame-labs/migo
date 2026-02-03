import { op_vibrate_short, op_vibrate_long } from "ext:core/ops";
import { wrapAsync } from "ext:host_v8_base/02_async.js";

function vibrateShort(options = {}) {
    const { type } = options;
    if (!type || (type !== 'heavy' && type !== 'medium' && type !== 'light')) {
        const res = { errMsg: 'vibrateShort:fail type is required and must be heavy, medium, or light' };
        if (typeof options.fail === 'function') {
            queueMicrotask(function () { options.fail(res); });
        }
        if (typeof options.complete === 'function') {
            queueMicrotask(function () { options.complete(res); });
        }
        return Promise.reject(res);
    }
    return wrapAsync('vibrateShort', function () {
        const code = op_vibrate_short(type);
        if (code === -1) {
            throw new Error('vibrator unavailable');
        }
        if (code === -2) {
            throw { errMsg: 'style is not support' };
        }
    }, options);
}

function vibrateLong(options = {}) {
    return wrapAsync('vibrateLong', function () {
        const code = op_vibrate_long();
        if (code === -1) {
            throw new Error('vibrator unavailable');
        }
    }, options);
}

export { vibrateShort, vibrateLong };
