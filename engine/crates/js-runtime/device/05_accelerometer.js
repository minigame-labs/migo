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

export { onAccelerometerChange, offAccelerometerChange, _internalTriggerAccelerometerChange };
