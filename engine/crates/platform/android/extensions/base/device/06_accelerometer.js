import { op_start_accelerometer, op_stop_accelerometer } from "ext:core/ops";
import { wrapAsync } from "ext:host_v8_base/02_async.js";

function startAccelerometer(options = {}) {
    const { interval = 'normal' } = options;
    return wrapAsync('startAccelerometer', function () {
        op_start_accelerometer(interval);
    }, options);
}

function stopAccelerometer(options) {
    return wrapAsync('stopAccelerometer', function () {
        op_stop_accelerometer();
    }, options);
}

export { startAccelerometer, stopAccelerometer };
