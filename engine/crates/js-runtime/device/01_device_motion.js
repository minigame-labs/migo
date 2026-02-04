import { op_start_device_motion, op_stop_device_motion } from "ext:core/ops";
import { wrapAsync } from "ext:host_v8_base/02_async.js";

const _listeners = [];

function onDeviceMotionChange(listener) {
    if (typeof listener === 'function') {
        _listeners.push(listener);
    }
}

function offDeviceMotionChange(listener) {
    if (typeof listener === 'function') {
        const index = _listeners.indexOf(listener);
        if (index !== -1) {
            _listeners.splice(index, 1);
        }
    } else {
        _listeners.length = 0;
    }
}

function _internalTriggerDeviceMotionChange(alpha, beta, gamma) {
    const data = { alpha, beta, gamma };
    for (let i = 0; i < _listeners.length; i++) {
        try {
            _listeners[i](data);
        } catch (e) {
            console.error('onDeviceMotionChange listener error:', e);
        }
    }
}

function startDeviceMotionListening(options = {}) {
    const { interval = 'normal' } = options;
    return wrapAsync('startDeviceMotionListening', function () {
        op_start_device_motion(interval);
    }, options);
}

function stopDeviceMotionListening(options = {}) {
    return wrapAsync('stopDeviceMotionListening', function () {
        op_stop_device_motion();
    }, options);
}

export {
    onDeviceMotionChange,
    offDeviceMotionChange,
    _internalTriggerDeviceMotionChange,
    startDeviceMotionListening,
    stopDeviceMotionListening,
};
