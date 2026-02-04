import { op_start_compass, op_stop_compass } from "ext:core/ops";
import { wrapAsync } from "ext:host_v8_base/02_async.js";

const _listeners = [];

function onCompassChange(listener) {
    if (typeof listener === 'function') {
        _listeners.push(listener);
    }
}

function offCompassChange(listener) {
    if (listener === undefined) {
        _listeners.length = 0;
        return;
    }
    const idx = _listeners.indexOf(listener);
    if (idx !== -1) {
        _listeners.splice(idx, 1);
    }
}

function _internalTriggerCompassChange(direction, accuracy) {
    const data = { direction: direction, accuracy: accuracy };
    for (let i = 0; i < _listeners.length; i++) {
        try {
            _listeners[i](data);
        } catch (e) {
            console.error('onCompassChange listener error:', e);
        }
    }
}

function startCompass(options = {}) {
    return wrapAsync('startCompass', function () {
        op_start_compass();
    }, options);
}

function stopCompass(options = {}) {
    return wrapAsync('stopCompass', function () {
        op_stop_compass();
    }, options);
}

export {
    onCompassChange,
    offCompassChange,
    _internalTriggerCompassChange,
    startCompass,
    stopCompass,
};
