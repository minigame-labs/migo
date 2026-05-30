import { op_vibrate_short, op_vibrate_long } from "ext:core/ops";
import { promisify, wrapAsync } from "ext:host_v8_base/02_async.js";

function vibrateShort(options = {}) {
    const { type = 'medium' } = options;
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
        op_vibrate_short(type);
    }, options);
}

const vibrateLong = promisify('vibrateLong', () => op_vibrate_long());

export { vibrateShort, vibrateLong };
