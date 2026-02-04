import { op_start_accelerometer, op_stop_accelerometer } from "ext:core/ops";
import { wrapAsync } from "ext:host_v8_base/02_async.js";

const _listeners = [];

function onAccelerometerChange(listener) {
    if (typeof listener === 'function') {
        _listeners.push(listener);
    }
}

function offAccelerometerChange(listener) {
    if (listener === undefined) {
        _listeners.length = 0;
        return;
    }
    const idx = _listeners.indexOf(listener);
    if (idx !== -1) {
        _listeners.splice(idx, 1);
    }
}

function _internalTriggerAccelerometerChange(x, y, z) {
    const data = { x: x, y: y, z: z };
    for (let i = 0; i < _listeners.length; i++) {
        try {
            _listeners[i](data);
        } catch (e) {
            console.error('onAccelerometerChange listener error:', e);
        }
    }
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
