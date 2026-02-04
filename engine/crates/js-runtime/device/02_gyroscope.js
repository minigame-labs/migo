import { op_start_gyroscope, op_stop_gyroscope } from "ext:core/ops";
import { wrapAsync } from "ext:host_v8_base/02_async.js";

const _listeners = [];

function onGyroscopeChange(listener) {
    if (typeof listener === 'function') {
        _listeners.push(listener);
    }
}

function offGyroscopeChange(listener) {
    if (typeof listener === 'function') {
        const index = _listeners.indexOf(listener);
        if (index !== -1) {
            _listeners.splice(index, 1);
        }
    } else {
        _listeners.length = 0;
    }
}

function _internalTriggerGyroscopeChange(x, y, z) {
    const data = { x, y, z };
    for (let i = 0; i < _listeners.length; i++) {
        try {
            _listeners[i](data);
        } catch (e) {
            console.error('onGyroscopeChange listener error:', e);
        }
    }
}

function startGyroscope(options = {}) {
    const { interval = 'normal' } = options;
    return wrapAsync('startGyroscope', function () {
        op_start_gyroscope(interval);
    }, options);
}

function stopGyroscope(options = {}) {
    return wrapAsync('stopGyroscope', function () {
        op_stop_gyroscope();
    }, options);
}

export {
    onGyroscopeChange,
    offGyroscopeChange,
    _internalTriggerGyroscopeChange,
    startGyroscope,
    stopGyroscope,
};
