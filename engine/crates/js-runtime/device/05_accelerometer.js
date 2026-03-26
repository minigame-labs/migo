import { op_start_accelerometer, op_stop_accelerometer } from "ext:core/ops";
import { wrapAsync, createListenerGroup } from "ext:host_v8_base/02_async.js";

const _grp = createListenerGroup('onAccelerometerChange');

function onAccelerometerChange(listener) { _grp.on(listener); }
function offAccelerometerChange(listener) { _grp.off(listener); }

function _internalTriggerAccelerometerChange(x, y, z) {
    _grp.trigger({ x: x, y: y, z: z });
}

function startAccelerometer(options = {}) {
    const { interval = 'normal' } = options;
    return wrapAsync('startAccelerometer', function () {
        op_start_accelerometer(interval);
    }, options);
}

function stopAccelerometer(options = {}) {
    return wrapAsync('stopAccelerometer', function () {
        op_stop_accelerometer();
    }, options);
}

export {
    onAccelerometerChange,
    offAccelerometerChange,
    _internalTriggerAccelerometerChange,
    startAccelerometer,
    stopAccelerometer,
};
